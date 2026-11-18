use super::{
    Args, Kind, Param, ToolContext, ToolFuture, ToolSpec, field, fit_budget, free_text_value,
    graphql, graphql_enum_names, list_result, tally,
};
use scryer_interface::types::TitleHistoryEventTypeValue;
use serde_json::{Map, Value, json};

/// Grab, download and import events for one title.
pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-troubleshoot/operations/TitleHistory.graphql");

fn event_types() -> Vec<&'static str> {
    graphql_enum_names::<TitleHistoryEventTypeValue>()
}

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "get_title_history",
    title: "Title history",
    description: "List one title's grab, download and import events newest first, with the \
release, client, quality and skip, failure and blocklist reasons, tallied by event type.",
    params: &[
        Param {
            name: "title_id",
            description: "The title, from find_titles.",
            required: true,
            kind: Kind::Str { max_chars: 100 },
        },
        Param {
            name: "event_types",
            description: "Only these event types.",
            required: false,
            kind: Kind::EnumList {
                values: event_types,
                max_items: 20,
            },
        },
        Param {
            name: "limit",
            description: "Events to return.",
            required: false,
            kind: Kind::Int {
                default: Some(25),
                min: 1,
                max: 50,
            },
        },
    ],
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let variables = json!({
            "titleIds": [args.value("title_id")],
            "eventTypes": args.value("event_types"),
            "limit": args.value("limit"),
        });
        let data = match graphql(&context, OPERATION, variables).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let page = field(&data, "titleHistory");
        let events = field(page, "items").as_array().cloned().unwrap_or_default();
        let read = events.len();
        let mut head = Map::new();
        head.insert(
            "by_event_type".into(),
            tally(events.iter().map(|event| {
                field(event, "eventType")
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string()
            })),
        );
        head.insert("has_more".into(), field(page, "hasMore").clone());
        let rows = events
            .into_iter()
            .map(|mut event| {
                for name in ["failureReason", "blocklistReason"] {
                    event[name] = free_text_value(field(&event, name));
                }
                event
            })
            .collect::<Vec<_>>();
        let items = fit_budget(rows);
        if items.len() < read {
            head.insert("omitted_for_size".into(), json!(read - items.len()));
        }
        // History is not paged here: narrow it with event_types.
        Ok(list_result(head, items, "next_offset", Value::Null))
    })
}
