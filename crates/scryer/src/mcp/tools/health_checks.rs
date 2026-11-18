use super::{
    Args, Kind, Param, ToolContext, ToolFuture, ToolSpec, app_error, free_text, graphql,
    list_result, offset_page, tally,
};
use scryer_application::HealthCheckStatus;
use serde_json::{Map, Value, json};

/// When the latest run started and finished; the results come from the
/// cached checks.
pub(super) const OPERATION: &str = include_str!("../operations/HealthCheckRunTimes.graphql");

const MAX_ITEMS: i64 = 20;

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "get_health_checks",
    title: "Health checks",
    description: "Show the health checks that are failing (warning or error) with their source \
and message, the totals by status, and when the checks last ran. Sources: DownloadClient, \
Indexer, RootFolder, RecycleBin, DiskSpace, ReleaseAgeUnknown.",
    params: &[
        Param {
            name: "limit",
            description: "Failing checks to return.",
            required: false,
            kind: Kind::Int {
                default: Some(MAX_ITEMS),
                min: 1,
                max: MAX_ITEMS,
            },
        },
        Param {
            name: "offset",
            description: "Failing checks to skip, from next_offset.",
            required: false,
            kind: Kind::Int {
                default: Some(0),
                min: 0,
                max: 10_000,
            },
        },
    ],
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let results = match context
            .auth
            .app
            .cached_health_check_results(context.caller.user())
            .await
        {
            Ok(results) => results,
            Err(error) => return Ok(app_error(error)),
        };
        let mut data = match graphql(&context, OPERATION, Value::Null).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let run = data
            .get_mut("jobRuns")
            .and_then(Value::as_array_mut)
            .and_then(|runs| runs.first_mut())
            .map(Value::take)
            .unwrap_or(Value::Null);

        let failing = results
            .iter()
            .filter(|check| check.status != HealthCheckStatus::Ok)
            .collect::<Vec<_>>();
        let mut head = Map::new();
        head.insert("total".into(), json!(results.len()));
        head.insert("failing".into(), json!(failing.len()));
        head.insert(
            "by_status".into(),
            tally(results.iter().map(|check| check.status.as_str())),
        );
        head.insert("latest_run".into(), run);
        let rows = failing.iter().map(|check| {
            json!({
                "source": check.source,
                "status": check.status.as_str(),
                "message": free_text(&check.message),
            })
        });
        let (items, next) = offset_page(
            rows,
            failing.len(),
            args.count("offset"),
            args.count("limit"),
        );
        Ok(list_result(head, items, "next_offset", next))
    })
}
