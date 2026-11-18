use super::{
    Args, Kind, Param, ToolContext, ToolFuture, ToolSpec, field, fit_budget, graphql, list_result,
};
use serde_json::{Map, Value, json};

/// Find a title id by name.
pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-troubleshoot/operations/FindTitle.graphql");

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "find_titles",
    title: "Find titles",
    description: "Find titles by name and return their ids, kind, year, monitoring, library and \
quality profile. Use the id with the title tools.",
    params: &[
        Param {
            name: "query",
            description: "Part of the title's name.",
            required: true,
            kind: Kind::Str { max_chars: 200 },
        },
        Param {
            name: "limit",
            description: "Titles to return.",
            required: false,
            kind: Kind::Int {
                default: Some(5),
                min: 1,
                max: 10,
            },
        },
    ],
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let variables = json!({"query": args.value("query"), "limit": args.value("limit")});
        let data = match graphql(&context, OPERATION, variables).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let titles = field(&data, "titles");
        let items = fit_budget(
            field(titles, "items")
                .as_array()
                .cloned()
                .unwrap_or_default(),
        );
        let mut head = Map::new();
        head.insert("total_count".into(), field(titles, "totalCount").clone());
        // A narrower query, not a page, finds the rest.
        Ok(list_result(head, items, "next_offset", Value::Null))
    })
}
