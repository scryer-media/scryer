use super::rule_sets::{GET_OPERATION, REGO_SOURCE_MAX_CHARS};
use super::rule_writes::{PRIORITY_MAX, PRIORITY_MIN, facets};
use super::{
    Args, Effect, Kind, Param, ToolContext, ToolFuture, ToolSpec, field, fit_budget,
    free_text_value, graphql, invalid_argument,
};
use rmcp::model::CallToolResult;
use serde_json::{Map, Value, json};

/// Scores one release name for one title, with a draft or a saved rule set.
pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-write-rules/operations/TestRuleSet.graphql");

const FACET_PARAM_DESCRIPTION_FOR_TEST: &str = "The media facets the draft applies to: movie, \
series, anime. Omitted, the saved rule's with rule_set_id, else every facet.";

/// Characters of a release name; the server takes at most 4 KiB of UTF-8.
const RELEASE_NAME_MAX_CHARS: usize = 1_000;

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "test_rule_set",
    title: "Test a rule set",
    description: "Score one release name for one library title with a draft or saved release \
rule set, saving nothing and contacting no indexer. With rego_source, the draft is scored as if \
enabled. With rego_source and rule_set_id, the draft replaces that saved rule in the preview, \
and omitted applied_facets and priority are taken from the saved rule, as update_rule_set would \
leave them. With rego_source alone, omitted applied_facets mean every facet and priority \
defaults to 0. Without rego_source, rule_set_id names a saved rule to test as saved. Returns \
the score, allowed, blocked, minimum score met, what was parsed from the name, the draft's \
contribution, each rule set's score entries with their codes, and errors with codes. For series \
and anime, episode_id comes from list_title_episodes.",
    params: &[
        Param {
            name: "title_id",
            description: "The library title to score against, from find_titles.",
            required: true,
            kind: Kind::Str { max_chars: 100 },
        },
        Param {
            name: "release_name",
            description: "The release name to parse and score.",
            required: true,
            kind: Kind::Str {
                max_chars: RELEASE_NAME_MAX_CHARS,
            },
        },
        Param {
            name: "size_bytes",
            description: "The release size in bytes, when known.",
            required: false,
            kind: Kind::Int {
                default: None,
                min: 0,
                max: 1 << 50,
            },
        },
        Param {
            name: "episode_id",
            description: "An episode of the title, for series and anime.",
            required: false,
            kind: Kind::Str { max_chars: 100 },
        },
        Param {
            name: "rule_set_id",
            description: "Without rego_source: the saved rule set to test. With it: the saved \
rule set the draft would replace.",
            required: false,
            kind: Kind::Str { max_chars: 100 },
        },
        Param {
            name: "rego_source",
            description: "A draft's complete Rego module, at most 30,000 characters.",
            required: false,
            kind: Kind::Str {
                max_chars: REGO_SOURCE_MAX_CHARS,
            },
        },
        Param {
            name: "name",
            description: "The draft's name in the preview (default \"Draft\").",
            required: false,
            kind: Kind::Str { max_chars: 200 },
        },
        Param {
            name: "applied_facets",
            description: FACET_PARAM_DESCRIPTION_FOR_TEST,
            required: false,
            kind: Kind::EnumList {
                values: facets,
                min_items: 0,
                max_items: 3,
            },
        },
        Param {
            name: "priority",
            description: "The draft's evaluation priority; omitted, the saved rule's with \
rule_set_id, else 0.",
            required: false,
            kind: Kind::Int {
                default: None,
                min: PRIORITY_MIN,
                max: PRIORITY_MAX,
            },
        },
    ],
    effect: Effect::DryRun,
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let mut input = Map::new();
        input.insert("titleId".into(), args.value("title_id"));
        input.insert("releaseName".into(), args.value("release_name"));
        input.insert("sizeBytes".into(), args.value("size_bytes"));
        input.insert("episodeId".into(), args.value("episode_id"));
        input.insert("copyDisablesSource".into(), json!(false));
        if args.has("rego_source") {
            let mut priority = args.value("priority");
            let mut facets = args.value("applied_facets");
            if args.has("rule_set_id") && (priority.is_null() || facets.is_null()) {
                // The saved rule's values, so the preview matches what an
                // update leaving them out would save.
                let saved = match graphql(
                    &context,
                    GET_OPERATION,
                    json!({"id": args.value("rule_set_id")}),
                )
                .await
                {
                    Ok(data) => data,
                    Err(error) => return Ok(error),
                };
                let saved = field(&saved, "ruleSet");
                if priority.is_null() {
                    priority = field(saved, "priority").clone();
                }
                if facets.is_null() {
                    facets = field(saved, "appliedFacets").clone();
                }
            }
            if priority.is_null() {
                priority = json!(0);
            }
            if facets.is_null() {
                facets = json!([]);
            }
            input.insert(
                "draft".into(),
                json!({
                    "name": args.str("name").unwrap_or("Draft"),
                    "description": "",
                    "regoSource": args.value("rego_source"),
                    "enabled": true,
                    "priority": priority,
                    "appliedFacets": facets,
                }),
            );
            input.insert("editRuleSetId".into(), args.value("rule_set_id"));
        } else if args.has("rule_set_id") {
            input.insert("testRuleSetId".into(), args.value("rule_set_id"));
        } else {
            return Err(invalid_argument(
                "rego_source",
                "give rego_source for a draft, or rule_set_id for a saved rule set",
            ));
        }
        let data = match graphql(&context, OPERATION, json!({"input": input})).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        Ok(CallToolResult::structured(shape(field(
            &data,
            "testRuleSet",
        ))))
    })
}

fn shape(result: &Value) -> Value {
    let mut contribution = field(result, "draftContribution").clone();
    if let Some(object) = contribution.as_object_mut() {
        object.insert(
            "message".into(),
            free_text_value(field(field(result, "draftContribution"), "message")),
        );
    }
    let errors = field(result, "errors")
        .as_array()
        .map(|errors| {
            errors
                .iter()
                .map(|error| {
                    json!({
                        "code": field(error, "code"),
                        "ruleSetId": field(error, "ruleSetId"),
                        "message": free_text_value(field(error, "message")),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let rule_sets = field(result, "ruleSets")
        .as_array()
        .cloned()
        .unwrap_or_default();
    let rule_set_count = rule_sets.len();
    let rule_sets = fit_budget(rule_sets);
    let mut shaped = Map::new();
    for name in [
        "releaseName",
        "score",
        "allowed",
        "blocked",
        "minimumScoreMet",
        "profileName",
        "context",
        "parsed",
    ] {
        shaped.insert(name.into(), field(result, name).clone());
    }
    shaped.insert("draftContribution".into(), contribution);
    shaped.insert("errors".into(), Value::Array(errors));
    shaped.insert("rule_set_count".into(), json!(rule_set_count));
    shaped.insert("rule_sets_returned".into(), json!(rule_sets.len()));
    shaped.insert("ruleSets".into(), Value::Array(rule_sets));
    Value::Object(shaped)
}
