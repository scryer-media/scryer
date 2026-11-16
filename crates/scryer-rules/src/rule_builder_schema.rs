//! What the rule builder offers for each rule family.
//!
//! The builder writes a rule's source from a form. Which outputs it can offer,
//! how each one's value is entered and bounded, and which outputs exclude each
//! other belong to the server, so the form never offers what a save would
//! refuse. [`rule_builder_schema`] serves one family's table; each family's
//! table is also checked in as JSON beside this crate, and a test keeps the
//! file and the table equal.

use serde::Serialize;

use crate::lifecycle_actions::{LifecycleActionHead, MAX_NOTIFY_LEN};
use crate::policy::decode::{MAX_TAG_LEN, MAX_TAGS};

/// The family name of lifecycle rules.
pub const LIFECYCLE_RULE_FAMILY: &str = "lifecycle";
/// The family name of scoring rules.
pub const SCORING_RULE_FAMILY: &str = "scoring";
/// The family name of request rules.
pub use crate::request_actions::REQUEST_RULE_FAMILY;

/// Every family [`rule_builder_schema`] serves.
pub const RULE_BUILDER_FAMILIES: [&str; 3] = [
    LIFECYCLE_RULE_FAMILY,
    SCORING_RULE_FAMILY,
    REQUEST_RULE_FAMILY,
];

/// How the builder enters one output's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuleBuilderValueKind {
    /// No value: the output is on or off.
    Flag,
    /// Free text.
    Text,
    /// One entry from a catalog the client loads.
    Choice,
    /// A whole number.
    Number,
    /// A list of short text entries.
    TextList,
}

/// One output the rule builder offers, as a client reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleBuilderOutputDescriptor {
    /// Stable id of the output.
    pub id: &'static str,
    /// The Rego rule name the output writes.
    pub head: &'static str,
    pub label_key: String,
    pub help_key: Option<String>,
    pub value_kind: RuleBuilderValueKind,
    /// The catalog a choice or list draws from.
    pub options: Option<&'static str>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub max_items: Option<usize>,
    pub max_bytes: Option<usize>,
    /// Characters every entry must match, as a regular expression.
    pub pattern: Option<&'static str>,
    /// How the output is written into the source.
    pub template: &'static str,
    pub per_item: bool,
    /// Whether the output removes media.
    pub destructive: bool,
    /// The rule variants that may use the output; `None` means every one.
    pub variants: Option<&'static [&'static str]>,
    /// Outputs that cannot be chosen with this one. Each pair is listed once.
    pub conflicts_with: Vec<&'static str>,
}

/// Which bound an output rule sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuleBuilderOutputRuleKind {
    AtLeastOne,
    AtMostOne,
    ExactlyOne,
}

/// A rule about which outputs one rule chooses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleBuilderOutputRule {
    pub rule: RuleBuilderOutputRuleKind,
    /// The output ids the rule is about; `None` means every output.
    pub of: Option<Vec<&'static str>>,
}

/// What the rule builder needs to offer a family's outputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleBuilderSchema {
    pub family: &'static str,
    pub outputs: Vec<RuleBuilderOutputDescriptor>,
    pub output_rules: Vec<RuleBuilderOutputRule>,
}

/// The builder schema of one rule family, or `None` for a family the server
/// does not know. A new family adds its arm here.
pub fn rule_builder_schema(family: &str) -> Option<RuleBuilderSchema> {
    match family {
        LIFECYCLE_RULE_FAMILY => Some(lifecycle_builder_schema()),
        SCORING_RULE_FAMILY => Some(scoring_builder_schema()),
        REQUEST_RULE_FAMILY => Some(crate::request_actions::request_builder_schema()),
        _ => None,
    }
}

/// A builder schema as its checked-in JSON file holds it.
pub fn rule_builder_schema_json(schema: &RuleBuilderSchema) -> String {
    let mut json = serde_json::to_string_pretty(schema).expect("the builder schema serializes");
    json.push('\n');
    json
}

/// Scoring rules decide a score, not a list of outputs, so the builder offers
/// none.
pub fn scoring_builder_schema() -> RuleBuilderSchema {
    RuleBuilderSchema {
        family: SCORING_RULE_FAMILY,
        outputs: Vec::new(),
        output_rules: Vec::new(),
    }
}

const TITLE_VARIANT_ONLY: &[&str] = &["TITLE"];

/// The id a client reads for a lifecycle head.
pub fn lifecycle_output_id(head: LifecycleActionHead) -> &'static str {
    match head {
        LifecycleActionHead::Unmonitor => "UNMONITOR",
        LifecycleActionHead::DeleteFiles => "DELETE_FILES",
        LifecycleActionHead::DeleteTitle => "DELETE_TITLE",
        LifecycleActionHead::Search => "SEARCH",
        LifecycleActionHead::SetQualityProfile => "SET_QUALITY_PROFILE",
        LifecycleActionHead::AddTags => "ADD_TAGS",
        LifecycleActionHead::RemoveTags => "REMOVE_TAGS",
        LifecycleActionHead::Monitor => "MONITOR",
        LifecycleActionHead::FetchSubtitles => "FETCH_SUBTITLES",
        LifecycleActionHead::MoveToLibrary => "MOVE_TO_LIBRARY",
        LifecycleActionHead::MoveToRoot => "MOVE_TO_ROOT",
        LifecycleActionHead::Notify => "NOTIFY",
    }
}

/// Whether a lifecycle head may be used by a rule of a narrower subject
/// than a whole title (a season or an episode).
pub fn lifecycle_head_allows_narrow_subjects(head: LifecycleActionHead) -> bool {
    matches!(
        head,
        LifecycleActionHead::Unmonitor
            | LifecycleActionHead::DeleteFiles
            | LifecycleActionHead::Monitor
            | LifecycleActionHead::FetchSubtitles
            | LifecycleActionHead::Notify
    )
}

fn lifecycle_output(head: LifecycleActionHead) -> RuleBuilderOutputDescriptor {
    use LifecycleActionHead as H;
    let (value_kind, options) = match head {
        H::AddTags | H::RemoveTags => (RuleBuilderValueKind::TextList, Some("titleTags")),
        H::SetQualityProfile => (RuleBuilderValueKind::Choice, Some("qualityProfiles")),
        H::MoveToLibrary => (RuleBuilderValueKind::Choice, Some("libraries")),
        H::MoveToRoot => (RuleBuilderValueKind::Choice, Some("roots")),
        H::Notify => (RuleBuilderValueKind::Text, None),
        _ => (RuleBuilderValueKind::Flag, None),
    };
    let tags = value_kind == RuleBuilderValueKind::TextList;
    RuleBuilderOutputDescriptor {
        id: lifecycle_output_id(head),
        head: head.rule_name(),
        label_key: format!("ruleOutput.lifecycle.{}", head.rule_name()),
        help_key: None,
        value_kind,
        options,
        min: None,
        max: None,
        max_items: tags.then_some(MAX_TAGS),
        max_bytes: if tags {
            Some(MAX_TAG_LEN)
        } else if head == H::Notify {
            Some(MAX_NOTIFY_LEN)
        } else {
            None
        },
        pattern: tags.then_some("[A-Za-z0-9._ -]+"),
        template: match value_kind {
            RuleBuilderValueKind::Flag => "{head} if {helper}",
            RuleBuilderValueKind::TextList => "{head} := [{items}] if {helper}",
            _ => "{head} := {value} if {helper}",
        },
        per_item: false,
        destructive: head.is_destructive(),
        variants: (!lifecycle_head_allows_narrow_subjects(head)).then_some(TITLE_VARIANT_ONLY),
        conflicts_with: head
            .conflicts_with()
            .iter()
            .map(|other| lifecycle_output_id(*other))
            .collect(),
    }
}

/// The lifecycle rule builder schema, in the order the host applies the
/// actions.
pub fn lifecycle_builder_schema() -> RuleBuilderSchema {
    RuleBuilderSchema {
        family: LIFECYCLE_RULE_FAMILY,
        outputs: LifecycleActionHead::EXECUTION_ORDER
            .into_iter()
            .map(lifecycle_output)
            .collect(),
        // A lifecycle rule must define at least one action; the save
        // refuses one that defines none.
        output_rules: vec![RuleBuilderOutputRule {
            rule: RuleBuilderOutputRuleKind::AtLeastOne,
            of: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIFECYCLE_FILE: &str = include_str!("../lifecycle-builder-schema.json");
    const SCORING_FILE: &str = include_str!("../scoring-builder-schema.json");
    const REQUEST_FILE: &str = include_str!("../request-builder-schema.json");

    #[test]
    fn checked_in_builder_schemas_match_the_tables() {
        assert_eq!(
            LIFECYCLE_FILE,
            rule_builder_schema_json(&lifecycle_builder_schema()),
            "regenerate lifecycle-builder-schema.json from lifecycle_builder_schema()"
        );
        assert_eq!(
            SCORING_FILE,
            rule_builder_schema_json(&scoring_builder_schema()),
            "regenerate scoring-builder-schema.json from scoring_builder_schema()"
        );
        assert_eq!(
            REQUEST_FILE,
            rule_builder_schema_json(&crate::request_actions::request_builder_schema()),
            "regenerate request-builder-schema.json from request_builder_schema()"
        );
    }

    #[test]
    fn every_served_family_resolves_and_unknown_families_do_not() {
        for family in RULE_BUILDER_FAMILIES {
            assert_eq!(
                rule_builder_schema(family).map(|schema| schema.family),
                Some(family)
            );
        }
        assert!(rule_builder_schema("library").is_none());
    }

    #[test]
    fn lifecycle_outputs_cover_every_head_once_and_name_real_conflicts() {
        let schema = lifecycle_builder_schema();
        let ids: Vec<_> = schema.outputs.iter().map(|output| output.id).collect();
        assert_eq!(ids.len(), LifecycleActionHead::ALL.len());
        for head in LifecycleActionHead::ALL {
            assert_eq!(
                ids.iter()
                    .filter(|id| **id == lifecycle_output_id(head))
                    .count(),
                1
            );
        }
        for output in &schema.outputs {
            for other in &output.conflicts_with {
                assert!(
                    ids.contains(other),
                    "{} conflicts with unknown {other}",
                    output.id
                );
            }
        }
    }
}
