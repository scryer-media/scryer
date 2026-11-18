use std::collections::HashMap;

use super::{
    Args, Kind, Param, ToolContext, ToolFuture, ToolSpec, field, free_text_value, graphql,
    list_result, offset_page, pick, tally,
};
use serde_json::{Map, Value, json};

/// Indexer state without config values or secrets.
pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-troubleshoot/operations/IndexerStatus.graphql");

/// The 24-hour statistics, joined to the indexers by id.
pub(super) const STATS_OPERATION: &str = include_str!("../operations/IndexerStats.graphql");

const MAX_ITEMS: i64 = 50;

const INDEXER_FIELDS: &[&str] = &[
    "id",
    "name",
    "providerType",
    "isEnabled",
    "isManaged",
    "enableInteractiveSearch",
    "enableAutoSearch",
    "proxyConfigId",
    "downloadClientId",
    "lastHealthStatus",
    "lastErrorMessage",
    "lastErrorAt",
    "lastQueryAt",
    "disabledUntil",
    "rateLimitedUntil",
    "maxQueriesPerMinute",
    "maxRequestsPerMonth",
    "maxGrabsPerMonth",
];

const STATS_FIELDS: &[&str] = &[
    "queriesLast24H",
    "successfulLast24H",
    "failedLast24H",
    "grabsLast24H",
    "apiCurrent",
    "apiMax",
    "grabCurrent",
    "grabMax",
];

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "list_indexers",
    title: "Indexers",
    description: "List indexers with their provider, search flags, health, last error, rate-limit \
and disable windows and configured limits, and by default their 24-hour query, failure and grab \
counts. No configuration values; nothing contacts the indexers.",
    params: &[
        Param {
            name: "include_stats",
            description: "Join each indexer's 24-hour statistics.",
            required: false,
            kind: Kind::Bool { default: true },
        },
        Param {
            name: "limit",
            description: "Indexers to return.",
            required: false,
            kind: Kind::Int {
                default: Some(MAX_ITEMS),
                min: 1,
                max: MAX_ITEMS,
            },
        },
        Param {
            name: "offset",
            description: "Indexers to skip, from next_offset.",
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
        let data = match graphql(&context, OPERATION, Value::Null).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let indexers = field(&data, "indexers")
            .as_array()
            .cloned()
            .unwrap_or_default();

        let mut head = Map::new();
        let mut stats_by_indexer: Option<HashMap<String, Value>> = None;
        if args.bool("include_stats") {
            match graphql(&context, STATS_OPERATION, Value::Null).await {
                Ok(stats) => {
                    stats_by_indexer = Some(
                        field(field(&stats, "systemHealth"), "indexerStats")
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|row| {
                                let id = field(row, "indexerId").as_str()?.to_string();
                                Some((id, row.clone()))
                            })
                            .collect(),
                    );
                }
                Err(refused) => match permission_refusal(&refused) {
                    // The list stands on its own; say why the counts are absent.
                    Some(reason) => {
                        head.insert("stats_omitted".into(), json!(reason));
                    }
                    None => return Ok(refused),
                },
            }
        }

        head.insert("total".into(), json!(indexers.len()));
        head.insert(
            "enabled".into(),
            json!(
                indexers
                    .iter()
                    .filter(|indexer| field(indexer, "isEnabled").as_bool() == Some(true))
                    .count()
            ),
        );
        head.insert(
            "by_health".into(),
            tally(indexers.iter().map(|indexer| {
                field(indexer, "lastHealthStatus")
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string()
            })),
        );
        let total = indexers.len();
        let rows = indexers.iter().map(|indexer| {
            let mut row = pick(indexer, INDEXER_FIELDS);
            row.insert(
                "lastErrorMessage".into(),
                free_text_value(field(indexer, "lastErrorMessage")),
            );
            if let Some(stats) = &stats_by_indexer {
                let id = field(indexer, "id").as_str().unwrap_or_default();
                row.insert(
                    "stats".into(),
                    stats
                        .get(id)
                        .map(|stats| Value::Object(pick(stats, STATS_FIELDS)))
                        .unwrap_or(Value::Null),
                );
            }
            Value::Object(row)
        });
        let (items, next) = offset_page(rows, total, args.count("offset"), args.count("limit"));
        Ok(list_result(head, items, "next_offset", next))
    })
}

/// The reason, or the code, of a permission refusal, decided on the error's
/// code and never on its message.
fn permission_refusal(result: &rmcp::model::CallToolResult) -> Option<String> {
    let errors = result
        .structured_content
        .as_ref()?
        .get("errors")?
        .as_array()?;
    let refusal = errors.iter().find(|error| {
        matches!(
            field(error, "code").as_str(),
            Some("UNAUTHORIZED" | "FORBIDDEN")
        )
    })?;
    field(refusal, "reason")
        .as_str()
        .or_else(|| field(refusal, "code").as_str())
        .map(str::to_string)
}
