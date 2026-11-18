use super::{
    Args, Kind, Param, ToolContext, ToolFuture, ToolSpec, field, fit_budget, free_text_value,
    graphql, list_result, tally,
};
use serde_json::{Map, Value, json};

/// Recent import attempts, without paths.
pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-troubleshoot/operations/ImportHistory.graphql");

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "list_import_history",
    title: "Import history",
    description: "List recent import attempts newest first with their type, status, decision, \
skip reason and error, tallied by decision and by skip reason. Only imports of titles in \
libraries the key can see are listed. Branch on decision and skipReason.",
    params: &[Param {
        name: "limit",
        description: "Imports to read.",
        required: false,
        kind: Kind::Int {
            default: Some(25),
            min: 1,
            max: 100,
        },
    }],
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let data = match graphql(&context, OPERATION, json!({"limit": args.value("limit")})).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let imports = field(&data, "importHistory")
            .as_array()
            .cloned()
            .unwrap_or_default();
        let read = imports.len();
        let mut head = Map::new();
        head.insert(
            "by_decision".into(),
            tally(imports.iter().map(|import| {
                field(import, "decision")
                    .as_str()
                    .unwrap_or("none")
                    .to_string()
            })),
        );
        head.insert(
            "by_skip_reason".into(),
            tally(
                imports
                    .iter()
                    .filter_map(|import| field(import, "skipReason").as_str().map(str::to_string)),
            ),
        );
        let rows = imports
            .into_iter()
            .map(|mut import| {
                import["errorMessage"] = free_text_value(field(&import, "errorMessage"));
                import
            })
            .collect::<Vec<_>>();
        let items = fit_budget(rows);
        if items.len() < read {
            head.insert("omitted_for_size".into(), json!(read - items.len()));
        }
        Ok(list_result(head, items, "next_offset", Value::Null))
    })
}
