use super::rule_sets::REGO_SOURCE_MAX_CHARS;
use super::{
    Args, Effect, Kind, Param, ToolContext, ToolFuture, ToolSpec, field, fit_budget,
    free_text_value, graphql, pick,
};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};

/// Compiles and checks a source without saving it.
pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-write-rules/operations/ValidateRuleSet.graphql");

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "validate_rule_set",
    title: "Validate rule source",
    description: "Compile and statically check a Rego source for a release rule set without \
saving anything. Returns valid and each diagnostic's code, line, column, path and message. \
Decide on code, line and column; the message is for people. Run it before create_rule_set or \
update_rule_set.",
    params: &[
        Param {
            name: "rego_source",
            description: "The complete Rego module, at most 30,000 characters.",
            required: true,
            kind: Kind::Str {
                max_chars: REGO_SOURCE_MAX_CHARS,
            },
        },
        Param {
            name: "rule_set_id",
            description: "The saved rule set this source would replace, for checks that \
depend on it; omit for a new rule.",
            required: false,
            kind: Kind::Str { max_chars: 100 },
        },
    ],
    effect: Effect::DryRun,
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let data = match graphql(
            &context,
            OPERATION,
            json!({"input": {
                "regoSource": args.value("rego_source"),
                "ruleSetId": args.value("rule_set_id"),
            }}),
        )
        .await
        {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let result = field(&data, "validateRuleSet");
        let diagnostics = field(result, "diagnostics")
            .as_array()
            .cloned()
            .unwrap_or_default();
        let total = diagnostics.len();
        let rows = diagnostics
            .iter()
            .map(|diagnostic| {
                let mut row = pick(diagnostic, &["code", "line", "column", "path", "message"]);
                row.insert(
                    "message".into(),
                    free_text_value(field(diagnostic, "message")),
                );
                Value::Object(row)
            })
            .collect();
        let items = fit_budget(rows);
        Ok(CallToolResult::structured(json!({
            "valid": field(result, "valid"),
            "diagnostic_count": total,
            "returned": items.len(),
            "diagnostics": items,
        })))
    })
}
