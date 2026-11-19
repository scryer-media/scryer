use super::Effect;
use super::{
    Args, Kind, Param, ToolContext, ToolFuture, ToolSpec, field, fit_budget, graphql, list_result,
    tally,
};
use serde_json::{Map, Value, json};

/// Releases held back (delay, standby, review).
pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-troubleshoot/operations/PendingReleases.graphql");

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "list_pending_releases",
    title: "Pending releases",
    description: "List releases held back by a delay, standby or review, with their score, \
indexer, delay and last decision code, for one title or every title the key can see.",
    params: &[
        Param {
            name: "title_id",
            description: "Only this title's pending releases.",
            required: false,
            kind: Kind::Str { max_chars: 100 },
        },
        Param {
            name: "limit",
            description: "Releases to return.",
            required: false,
            kind: Kind::Int {
                default: Some(25),
                min: 1,
                max: 50,
            },
        },
        Param {
            name: "offset",
            description: "Releases to skip, from next_offset.",
            required: false,
            kind: Kind::Int {
                default: Some(0),
                min: 0,
                max: 100_000,
            },
        },
    ],
    effect: Effect::Read,
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let offset = args.count("offset");
        let variables = json!({
            "titleId": args.value("title_id"),
            "limit": args.value("limit"),
            "offset": offset,
        });
        let data = match graphql(&context, OPERATION, variables).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let page = field(&data, "pendingReleases");
        let releases = field(page, "items").as_array().cloned().unwrap_or_default();
        let read = releases.len();
        let mut head = Map::new();
        head.insert("total_count".into(), field(page, "totalCount").clone());
        head.insert(
            "page_by_status".into(),
            tally(releases.iter().map(|release| {
                field(release, "status")
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string()
            })),
        );
        let items = fit_budget(releases);
        let has_more = field(page, "hasMore").as_bool() == Some(true) || items.len() < read;
        let next = if has_more {
            json!(offset + items.len())
        } else {
            Value::Null
        };
        Ok(list_result(head, items, "next_offset", next))
    })
}
