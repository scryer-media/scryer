//! The simple builder's stored document.
//!
//! The web builder writes a rule's Rego from a small document of conditions
//! and outputs. The server keeps that document beside the rule so the builder
//! can reopen it, and never reads it: the stored Rego is what evaluates. The
//! client checks on read that the document still writes the stored source, so
//! a hand-edited rule simply stops opening in the builder.
//!
//! The server only checks the document's envelope: an object that names the
//! rule's own family and a document version, within a size bound.

use crate::{AppError, AppResult};

/// The largest builder document a rule stores, as canonical JSON.
pub const MAX_BUILDER_MODEL_BYTES: usize = 64 * 1024;

pub use scryer_rules::rule_builder_schema::{
    RULE_BUILDER_FAMILIES, RuleBuilderOutputDescriptor, RuleBuilderOutputRule,
    RuleBuilderOutputRuleKind, RuleBuilderSchema, RuleBuilderValueKind,
};

/// What the rule builder offers for one rule family: its outputs, how each is
/// entered and bounded, and which exclude each other. A family the server has
/// no schema for is refused with a code.
pub fn rule_builder_schema(family: &str) -> AppResult<RuleBuilderSchema> {
    scryer_rules::rule_builder_schema::rule_builder_schema(family).ok_or_else(|| {
        AppError::validation_refused(
            crate::error_reasons::rules::BUILDER_FAMILY_UNKNOWN,
            format!("the rule builder has no schema for the family \"{family}\""),
        )
    })
}

/// The rule family a builder document belongs to, as the document spells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuilderFamily {
    Scoring,
    Maintenance,
    Request,
}

impl BuilderFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scoring => "scoring",
            Self::Maintenance => "maintenance",
            Self::Request => "request",
        }
    }
}

/// What an edit does to a rule's stored builder document.
///
/// GraphQL carries three states for the field: `null` clears the stored
/// document and a value replaces it. Left out keeps it only while the source
/// stays the same; see [`BuilderModelChange::after_source_edit`].
#[derive(Clone, Debug, Default, PartialEq)]
pub enum BuilderModelChange {
    #[default]
    Keep,
    Clear,
    Set(serde_json::Value),
}

impl BuilderModelChange {
    /// The change a create or update input field asks for.
    pub fn from_input(field: Option<Option<serde_json::Value>>) -> Self {
        match field {
            None => Self::Keep,
            Some(None) | Some(Some(serde_json::Value::Null)) => Self::Clear,
            Some(Some(value)) => Self::Set(value),
        }
    }

    /// The change once the edit's source is known. A stored document describes
    /// the source it generated, so an omitted field keeps it only when the
    /// source is unchanged; after a hand edit it is cleared, or the next builder
    /// save would regenerate from it and silently revert that edit.
    pub fn after_source_edit(self, previous_source: &str, new_source: &str) -> Self {
        match self {
            Self::Keep
                if scryer_rules::strip_editor_source(previous_source)
                    != scryer_rules::strip_editor_source(new_source) =>
            {
                Self::Clear
            }
            change => change,
        }
    }
}

/// Check a builder document's envelope and return its canonical JSON text.
pub fn checked_builder_model(
    value: &serde_json::Value,
    family: BuilderFamily,
) -> AppResult<String> {
    let invalid = |message: String| {
        AppError::validation_refused(crate::error_reasons::rules::BUILDER_MODEL_INVALID, message)
    };
    let Some(object) = value.as_object() else {
        return Err(invalid("the builder document must be a JSON object".into()));
    };
    match object.get("family").and_then(serde_json::Value::as_str) {
        Some(name) if name == family.as_str() => {}
        _ => {
            return Err(invalid(format!(
                "the builder document must name the {} family",
                family.as_str()
            )));
        }
    }
    if !object
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|version| version >= 1)
    {
        return Err(invalid(
            "the builder document must carry a positive integer version".into(),
        ));
    }
    let text = serde_json::to_string(value).map_err(|error| {
        AppError::validation_refused(
            crate::error_reasons::rules::BUILDER_MODEL_INVALID,
            format!("the builder document could not be stored: {error}"),
        )
    })?;
    if text.len() > MAX_BUILDER_MODEL_BYTES {
        return Err(AppError::validation_refused(
            crate::error_reasons::rules::BUILDER_MODEL_TOO_LARGE,
            format!("the builder document is larger than {MAX_BUILDER_MODEL_BYTES} bytes"),
        ));
    }
    Ok(text)
}

/// The stored text a change leaves, given the text stored before it.
pub fn resolved_builder_model(
    change: &BuilderModelChange,
    stored: Option<String>,
    family: BuilderFamily,
) -> AppResult<Option<String>> {
    match change {
        BuilderModelChange::Keep => Ok(stored),
        BuilderModelChange::Clear => Ok(None),
        BuilderModelChange::Set(value) => checked_builder_model(value, family).map(Some),
    }
}

/// Parse stored text back into the document the client reads. A stored value
/// that no longer parses reads as no document, so the rule opens as Rego.
pub fn stored_builder_model(text: Option<&str>) -> Option<serde_json::Value> {
    text.and_then(|text| serde_json::from_str(text).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reason(error: AppError) -> String {
        error.validation_reason().unwrap_or_default().to_string()
    }

    #[test]
    fn the_builder_schema_is_served_per_family_and_an_unknown_family_is_refused_with_a_code() {
        for family in ["maintenance", "scoring"] {
            assert!(rule_builder_schema(family).is_ok(), "{family}");
        }
        assert_eq!(
            reason(rule_builder_schema("synthetic").expect_err("no such family")),
            crate::error_reasons::rules::BUILDER_FAMILY_UNKNOWN
        );
    }

    #[test]
    fn a_document_for_the_rule_family_is_stored_canonically() {
        let text = checked_builder_model(
            &json!({ "version": 2, "family": "maintenance", "join": "ALL" }),
            BuilderFamily::Maintenance,
        )
        .expect("valid document");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap(),
            json!({ "family": "maintenance", "join": "ALL", "version": 2 })
        );
    }

    #[test]
    fn the_envelope_is_checked() {
        for value in [
            json!([]),
            json!({ "version": 2 }),
            json!({ "version": 2, "family": "scoring" }),
            json!({ "family": "maintenance" }),
            json!({ "family": "maintenance", "version": 0 }),
            json!({ "family": "maintenance", "version": "2" }),
        ] {
            let error =
                checked_builder_model(&value, BuilderFamily::Maintenance).expect_err("refused");
            assert_eq!(
                reason(error),
                crate::error_reasons::rules::BUILDER_MODEL_INVALID,
                "{value}"
            );
        }
    }

    #[test]
    fn an_oversized_document_is_refused() {
        let value = json!({
            "family": "scoring",
            "version": 2,
            "padding": "x".repeat(MAX_BUILDER_MODEL_BYTES),
        });
        let error = checked_builder_model(&value, BuilderFamily::Scoring).expect_err("refused");
        assert_eq!(
            reason(error),
            crate::error_reasons::rules::BUILDER_MODEL_TOO_LARGE
        );
    }

    #[test]
    fn the_three_input_states_keep_clear_and_set() {
        let stored = Some("{\"family\":\"scoring\",\"version\":2}".to_string());
        assert_eq!(
            resolved_builder_model(
                &BuilderModelChange::from_input(None),
                stored.clone(),
                BuilderFamily::Scoring
            )
            .unwrap(),
            stored
        );
        assert_eq!(
            resolved_builder_model(
                &BuilderModelChange::from_input(Some(None)),
                stored.clone(),
                BuilderFamily::Scoring
            )
            .unwrap(),
            None
        );
        let set = BuilderModelChange::from_input(Some(Some(
            json!({ "family": "scoring", "version": 3 }),
        )));
        assert_eq!(
            stored_builder_model(
                resolved_builder_model(&set, stored, BuilderFamily::Scoring)
                    .unwrap()
                    .as_deref()
            ),
            Some(json!({ "family": "scoring", "version": 3 }))
        );
    }

    #[test]
    fn an_omitted_document_survives_only_an_unchanged_source() {
        let source =
            "package scryer.rules.user.r1\nimport rego.v1\n\nscore_entry[\"a\"] := 1 if { true }\n";
        let same_in_editor = "score_entry[\"a\"] := 1 if { true }\n";
        assert_eq!(
            BuilderModelChange::Keep.after_source_edit(source, same_in_editor),
            BuilderModelChange::Keep
        );
        assert_eq!(
            BuilderModelChange::Keep
                .after_source_edit(source, "score_entry[\"a\"] := 2 if { true }\n"),
            BuilderModelChange::Clear
        );
        let set = BuilderModelChange::Set(serde_json::json!({"family": "scoring", "version": 2}));
        assert_eq!(set.clone().after_source_edit(source, "x := 1\n"), set);
        assert_eq!(
            BuilderModelChange::Clear.after_source_edit(source, source),
            BuilderModelChange::Clear
        );
    }
}
