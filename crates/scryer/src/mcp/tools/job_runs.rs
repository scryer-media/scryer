use super::{
    Args, Kind, PATTERN_SANITIZER, Param, ToolContext, ToolFuture, ToolSpec, field, fit_budget,
    free_text_value, graphql, graphql_enum_names, list_result, offset_page, pick, tally,
};
use scryer_interface::types::JobKeyValue;
use serde_json::{Map, Value, json};

/// The latest run of every job, without summaries.
pub(super) const LATEST_OPERATION: &str =
    include_str!("../../../../../skills/scryer-troubleshoot/operations/LatestJobRuns.graphql");

/// Recent runs of one job.
pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-troubleshoot/operations/JobRuns.graphql");

const MAX_LATEST: i64 = 40;
const MAX_RUNS: i64 = 20;
/// Characters of a run's serialized summary kept.
const SUMMARY_MAX_CHARS: usize = 2_000;

const CUSTOM_JOB: &str = "CUSTOM_JOB";

fn job_keys() -> Vec<&'static str> {
    graphql_enum_names::<JobKeyValue>()
        .into_iter()
        .filter(|name| *name != CUSTOM_JOB)
        .collect()
}

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "list_job_runs",
    title: "Job runs",
    description: "Without job_key: the latest run of every background job with its status, \
trigger, times and error, tallied by status. With job_key: that job's recent runs, newest \
first; include_summary adds each run's summary.",
    params: &[
        Param {
            name: "job_key",
            description: "One job, for its recent runs.",
            required: false,
            kind: Kind::Enum {
                values: job_keys,
                default: None,
            },
        },
        Param {
            name: "limit",
            description: "Runs to return: up to 40 latest runs, or up to 20 runs of one job \
(default 5).",
            required: false,
            kind: Kind::Int {
                default: None,
                min: 1,
                max: MAX_LATEST,
            },
        },
        Param {
            name: "offset",
            description: "Latest runs to skip, from next_offset (default 0). Only without \
job_key; refused with it.",
            required: false,
            kind: Kind::Int {
                default: None,
                min: 0,
                max: 10_000,
            },
        },
        Param {
            name: "include_summary",
            description: "Add each run's summary text and JSON. Only with job_key; true is \
refused without it.",
            required: false,
            kind: Kind::Bool { default: false },
        },
    ],
    run,
};

fn invalid(argument: &str, message: &str) -> rmcp::ErrorData {
    rmcp::ErrorData::invalid_params(
        message.to_string(),
        Some(json!({
            "reason": crate::error_reasons::HTTP_MCP_ARGUMENTS_INVALID,
            "argument": argument,
        })),
    )
}

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let has_job_key = args.str("job_key").is_some();
        if has_job_key && args.int("offset").is_some() {
            return Err(invalid(
                "offset",
                "argument `offset` is only accepted without job_key",
            ));
        }
        if !has_job_key && args.bool("include_summary") {
            return Err(invalid(
                "include_summary",
                "argument `include_summary` is only accepted with job_key",
            ));
        }
        match args.str("job_key") {
            None => latest(context, args).await,
            Some(job_key) => {
                let job_key = job_key.to_string();
                one_job(context, args, job_key).await
            }
        }
    })
}

async fn latest(
    context: ToolContext,
    args: Args,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    let data = match graphql(&context, LATEST_OPERATION, Value::Null).await {
        Ok(data) => data,
        Err(error) => return Ok(error),
    };
    let runs = field(&data, "latestJobRuns")
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut head = Map::new();
    head.insert("jobs".into(), json!(runs.len()));
    head.insert(
        "by_status".into(),
        tally(runs.iter().map(|run| {
            field(run, "status")
                .as_str()
                .unwrap_or("unknown")
                .to_string()
        })),
    );
    let total = runs.len();
    let rows = runs.iter().map(|run| {
        let mut row = pick(
            run,
            &[
                "jobKey",
                "status",
                "triggerSource",
                "startedAt",
                "completedAt",
                "errorText",
            ],
        );
        row.insert("errorText".into(), free_text_value(field(run, "errorText")));
        Value::Object(row)
    });
    let limit = args
        .int("limit")
        .map_or(MAX_LATEST as usize, |limit| limit as usize);
    let (items, next) = offset_page(rows, total, args.count("offset"), limit);
    Ok(list_result(head, items, "next_offset", next))
}

async fn one_job(
    context: ToolContext,
    args: Args,
    job_key: String,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    let limit = args.int("limit").unwrap_or(5);
    if limit > MAX_RUNS {
        return Err(invalid(
            "limit",
            "argument `limit` must be an integer from 1 to 20 with job_key",
        ));
    }
    let data = match graphql(
        &context,
        OPERATION,
        json!({"jobKey": job_key, "limit": limit}),
    )
    .await
    {
        Ok(data) => data,
        Err(error) => return Ok(error),
    };
    let runs = field(&data, "jobRuns")
        .as_array()
        .cloned()
        .unwrap_or_default();
    let include_summary = args.bool("include_summary");
    let mut head = Map::new();
    head.insert("job_key".into(), json!(job_key));
    head.insert(
        "by_status".into(),
        tally(runs.iter().map(|run| {
            field(run, "status")
                .as_str()
                .unwrap_or("unknown")
                .to_string()
        })),
    );
    let rows = runs
        .iter()
        .map(|run| {
            let mut row = pick(
                run,
                &[
                    "status",
                    "triggerSource",
                    "startedAt",
                    "completedAt",
                    "errorText",
                ],
            );
            row.insert("errorText".into(), free_text_value(field(run, "errorText")));
            if include_summary {
                row.insert(
                    "summaryText".into(),
                    free_text_value(field(run, "summaryText")),
                );
                let summary = field(run, "summaryJson");
                row.insert(
                    "summaryJson".into(),
                    if summary.is_null() {
                        Value::Null
                    } else {
                        let text = PATTERN_SANITIZER.text(&summary.to_string());
                        json!(scryer_application::diagnostics::cut_chars(
                            &text,
                            SUMMARY_MAX_CHARS
                        ))
                    },
                );
            }
            Value::Object(row)
        })
        .collect::<Vec<_>>();
    let read = rows.len();
    let items = fit_budget(rows);
    if items.len() < read {
        // Older runs are not paged: say how many did not fit, so a smaller
        // limit or no summaries can be asked for.
        head.insert("omitted_for_size".into(), json!(read - items.len()));
    }
    Ok(list_result(head, items, "next_offset", Value::Null))
}
