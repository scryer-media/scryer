use super::{
    Args, Effect, Kind, OFFSET_PARAM, Param, ToolContext, ToolFuture, ToolSpec, field,
    free_text_value, graphql, limit_param, list_result, offset_page, pick,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value, json};

/// Every release rule set, without its source.
pub(super) const LIST_OPERATION: &str =
    include_str!("../../../../../skills/scryer-write-rules/operations/RuleSets.graphql");

/// One rule set with its source.
pub(super) const GET_OPERATION: &str =
    include_str!("../../../../../skills/scryer-write-rules/operations/RuleSet.graphql");

/// Characters of Rego source these tools read and write. The server sets no
/// limit of its own; this keeps one rule body, sent twice in a result (as
/// structured content and as its text block), under a 25,000-token client
/// limit.
pub(super) const REGO_SOURCE_MAX_CHARS: usize = 30_000;

/// Characters of a rule set's description these tools read and write.
pub(super) const DESCRIPTION_MAX_CHARS: usize = 2_000;

const MAX_ITEMS: i64 = 50;

pub(super) static LIST_TOOL: ToolSpec = ToolSpec {
    name: "list_rule_sets",
    title: "Release rule sets",
    description: "List release-scoring rule sets without their source: id, name, enabled, \
priority, applied facets, whether a trusted pack manages it, why it was disabled and when it \
changed. Counts come first. get_rule_set returns one with its source.",
    params: &[limit_param("Rule sets to return.", MAX_ITEMS), OFFSET_PARAM],
    effect: Effect::Read,
    run: run_list,
};

pub(super) static GET_TOOL: ToolSpec = ToolSpec {
    name: "get_rule_set",
    title: "Release rule set",
    description: "One release-scoring rule set with its complete Rego source, name and \
description exactly as saved, never cut. A source longer than 30,000 characters, or a \
description longer than 2,000, is left out instead (regoSource or description null, with \
rego_source_omitted or description_omitted true and the length), because these tools cannot \
write it back; edit such a rule in the web UI.",
    params: &[Param {
        name: "id",
        description: "The rule set's id, from list_rule_sets.",
        required: true,
        kind: Kind::Str { max_chars: 100 },
    }],
    effect: Effect::Read,
    run: run_get,
};

fn run_list(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let data = match graphql(&context, LIST_OPERATION, Value::Null).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let rule_sets = field(&data, "ruleSets")
            .as_array()
            .cloned()
            .unwrap_or_default();
        let count = |name: &str| {
            rule_sets
                .iter()
                .filter(|rule_set| field(rule_set, name).as_bool() == Some(true))
                .count()
        };
        let mut head = Map::new();
        head.insert("total".into(), json!(rule_sets.len()));
        head.insert("enabled".into(), json!(count("enabled")));
        head.insert("managed".into(), json!(count("isManaged")));
        let total = rule_sets.len();
        let rows = rule_sets.iter().map(|rule_set| {
            let mut row = pick(
                rule_set,
                &[
                    "id",
                    "name",
                    "enabled",
                    "priority",
                    "appliedFacets",
                    "isManaged",
                    "disabledReason",
                    "updatedAt",
                ],
            );
            row.insert(
                "disabledReason".into(),
                free_text_value(field(rule_set, "disabledReason")),
            );
            Value::Object(row)
        });
        let (items, next) = offset_page(rows, total, args.count("offset"), args.count("limit"));
        Ok(list_result(head, items, "next_offset", next))
    })
}

fn run_get(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let data = match graphql(
            &context,
            GET_OPERATION,
            json!({"id": args.str("id").unwrap_or_default()}),
        )
        .await
        {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let rule_set = field(&data, "ruleSet");
        if rule_set.is_null() {
            return Ok(CallToolResult::structured(json!({"ruleSet": null})));
        }
        let mut row = pick(
            rule_set,
            &[
                "id",
                "name",
                "description",
                "enabled",
                "priority",
                "appliedFacets",
                "isManaged",
                "disabledReason",
                "regoSource",
            ],
        );
        row.insert(
            "disabledReason".into(),
            free_text_value(field(rule_set, "disabledReason")),
        );
        whole_or_omitted(
            &mut row,
            "description",
            "description_omitted",
            "description_chars",
            DESCRIPTION_MAX_CHARS,
        );
        whole_or_omitted(
            &mut row,
            "regoSource",
            "rego_source_omitted",
            "rego_source_chars",
            REGO_SOURCE_MAX_CHARS,
        );
        Ok(CallToolResult::structured(json!({"ruleSet": row})))
    })
}

/// Keeps a text field whole, or replaces it with null when it is longer than
/// `max_chars`, so a reader never edits a cut copy. The flag and the length
/// are always present.
fn whole_or_omitted(
    row: &mut Map<String, Value>,
    name: &str,
    flag: &str,
    length: &str,
    max_chars: usize,
) {
    let chars = row
        .get(name)
        .and_then(Value::as_str)
        .map_or(0, |text| text.chars().count());
    let omitted = chars > max_chars;
    if omitted {
        row.insert(name.into(), Value::Null);
    }
    row.insert(flag.into(), json!(omitted));
    row.insert(length.into(), json!(chars));
}
