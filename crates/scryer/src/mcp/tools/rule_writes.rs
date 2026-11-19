use super::rule_sets::{DESCRIPTION_MAX_CHARS, REGO_SOURCE_MAX_CHARS};
use super::{
    Args, Effect, Kind, Param, ToolContext, ToolFuture, ToolSpec, invalid_argument, mutation_result,
};
use serde_json::{Map, Value, json};

/// Saves a new rule set.
pub(super) const CREATE_OPERATION: &str =
    include_str!("../../../../../skills/scryer-write-rules/operations/CreateRuleSet.graphql");

/// Changes a saved rule set; omitted fields stay as they are.
pub(super) const UPDATE_OPERATION: &str =
    include_str!("../../../../../skills/scryer-write-rules/operations/UpdateRuleSet.graphql");

/// Enables or disables a rule set.
pub(super) const TOGGLE_OPERATION: &str =
    include_str!("../../../../../skills/scryer-write-rules/operations/ToggleRuleSet.graphql");

pub(super) const FACET_PARAM_DESCRIPTION: &str =
    "The media facets the rule applies to: movie, series, anime.";

pub(super) fn facets() -> Vec<&'static str> {
    [
        scryer_domain::MediaFacet::Movie,
        scryer_domain::MediaFacet::Series,
        scryer_domain::MediaFacet::Anime,
    ]
    .into_iter()
    .map(|facet| facet.as_str())
    .collect()
}

const NAME_PARAM: Param = Param {
    name: "name",
    description: "The rule set's name.",
    required: false,
    kind: Kind::Str { max_chars: 200 },
};

const DESCRIPTION_PARAM: Param = Param {
    name: "description",
    description: "What the rule does, at most 2,000 characters.",
    required: false,
    kind: Kind::Str {
        max_chars: DESCRIPTION_MAX_CHARS,
    },
};

const SOURCE_PARAM: Param = Param {
    name: "rego_source",
    description: "The complete Rego module, at most 30,000 characters. Check it with \
validate_rule_set first.",
    required: false,
    kind: Kind::Str {
        max_chars: REGO_SOURCE_MAX_CHARS,
    },
};

const FACETS_PARAM: Param = Param {
    name: "applied_facets",
    description: FACET_PARAM_DESCRIPTION,
    required: false,
    kind: Kind::EnumList {
        values: facets,
        min_items: 0,
        max_items: 3,
    },
};

/// The priority range the rule, delay-profile and routing mutations accept:
/// a GraphQL `Int`.
pub(super) const PRIORITY_MIN: i64 = i32::MIN as i64;
pub(super) const PRIORITY_MAX: i64 = i32::MAX as i64;

const PRIORITY_PARAM: Param = Param {
    name: "priority",
    description: "Evaluation priority; higher runs first.",
    required: false,
    kind: Kind::Int {
        default: None,
        min: PRIORITY_MIN,
        max: PRIORITY_MAX,
    },
};

const ID_PARAM: Param = Param {
    name: "id",
    description: "The rule set's id, from list_rule_sets.",
    required: true,
    kind: Kind::Str { max_chars: 100 },
};

const fn required(param: Param) -> Param {
    Param {
        required: true,
        ..param
    }
}

pub(super) static CREATE_TOOL: ToolSpec = ToolSpec {
    name: "create_rule_set",
    title: "Create a rule set",
    description: "Save a new release-scoring rule set through the createRuleSet mutation, with \
the same permission check, MFA step-up and audit trail as the web UI. It is saved disabled \
unless enabled is true. Returns its id, name, enabled, priority, facets and disabled reason. \
Each call saves another rule set.",
    params: &[
        required(NAME_PARAM),
        required(SOURCE_PARAM),
        DESCRIPTION_PARAM,
        FACETS_PARAM,
        PRIORITY_PARAM,
        Param {
            name: "enabled",
            description: "Whether the rule takes part in scoring as soon as it is saved \
(default false).",
            required: false,
            kind: Kind::Bool {
                default: Some(false),
            },
        },
    ],
    effect: Effect::Write {
        destructive: false,
        idempotent: false,
    },
    run: run_create,
};

pub(super) static UPDATE_TOOL: ToolSpec = ToolSpec {
    name: "update_rule_set",
    title: "Update a rule set",
    description: "Change a saved release-scoring rule set through the updateRuleSet mutation. \
Only the fields given change; each replaces the saved value, rego_source the whole module. Give \
at least one. Read the current source with get_rule_set first. A changed rego_source drops the \
rule's visual-builder layout, so the web UI then shows it as source only. A source that reads \
retired release inputs is saved disabled, with disabledReason in the result; any other invalid \
source is refused with its diagnostics.",
    params: &[
        ID_PARAM,
        NAME_PARAM,
        DESCRIPTION_PARAM,
        SOURCE_PARAM,
        FACETS_PARAM,
        PRIORITY_PARAM,
    ],
    effect: Effect::Write {
        destructive: true,
        idempotent: true,
    },
    run: run_update,
};

pub(super) static TOGGLE_TOOL: ToolSpec = ToolSpec {
    name: "set_rule_set_enabled",
    title: "Enable or disable a rule set",
    description: "Enable or disable a saved release-scoring rule set through the toggleRuleSet \
mutation. Returns its id, name, enabled state and disabled reason.",
    params: &[
        ID_PARAM,
        Param {
            name: "enabled",
            description: "The state to set.",
            required: true,
            kind: Kind::Bool { default: None },
        },
    ],
    effect: Effect::Write {
        destructive: true,
        idempotent: true,
    },
    run: run_toggle,
};

/// The arguments given, renamed to their GraphQL input fields.
fn input(args: &Args, names: &[(&str, &str)]) -> Map<String, Value> {
    names
        .iter()
        .filter(|(argument, _)| args.has(argument))
        .map(|(argument, field)| ((*field).to_string(), args.value(argument)))
        .collect()
}

const FIELDS: &[(&str, &str)] = &[
    ("name", "name"),
    ("description", "description"),
    ("rego_source", "regoSource"),
    ("applied_facets", "appliedFacets"),
    ("priority", "priority"),
];

fn run_create(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let mut input = input(&args, FIELDS);
        input.insert("enabled".into(), args.value("enabled"));
        mutation_result(
            &context,
            CREATE_OPERATION,
            json!({"input": input}),
            "createRuleSet",
        )
        .await
    })
}

fn run_update(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let mut input = input(&args, FIELDS);
        if input.is_empty() {
            return Err(invalid_argument(
                "rego_source",
                "give at least one of name, description, rego_source, applied_facets or priority",
            ));
        }
        input.insert("id".into(), args.value("id"));
        mutation_result(
            &context,
            UPDATE_OPERATION,
            json!({"input": input}),
            "updateRuleSet",
        )
        .await
    })
}

fn run_toggle(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        mutation_result(
            &context,
            TOGGLE_OPERATION,
            json!({"input": {"id": args.value("id"), "enabled": args.value("enabled")}}),
            "toggleRuleSet",
        )
        .await
    })
}
