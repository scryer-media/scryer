use super::MediaFacetValue;
use async_graphql::{ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};

// ── Rule Sets ──────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// Rego rule set configuration and managed-pack metadata.
pub struct RuleSetPayload {
    /// Rule-set ID.
    pub id: ID,
    /// Rule-set name.
    pub name: String,
    /// Rule-set description.
    pub description: String,
    /// Rego source used for validation and evaluation.
    pub rego_source: String,
    /// Whether the rule set is enabled.
    pub enabled: bool,
    /// Evaluation priority.
    pub priority: i32,
    /// Media facets to which the rule set applies.
    pub applied_facets: Vec<String>,
    /// Whether the rule set is managed by a trusted pack.
    pub is_managed: bool,
    /// Managed-pack key, or null for user-authored rules.
    pub managed_key: Option<String>,
    /// Tags a managed pack is narrowed to. Null means it applies wherever its
    /// facts match. Always null for user-authored rule sets.
    pub managed_tag_filter: Option<Vec<String>>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after deleting a rule set.
pub struct DeleteRuleSetPayload {
    /// Deleted rule-set ID.
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
/// Result of validating Rego source.
pub struct RuleValidationResultPayload {
    /// Whether the source is valid.
    pub valid: bool,
    /// Validation errors; empty when valid.
    pub errors: Vec<String>,
}

#[derive(InputObject)]
/// Creates a user-authored Rego rule set.
pub struct CreateRuleSetInput {
    /// Rule-set name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Complete Rego module evaluated for this rule set.
    pub rego_source: String,
    /// Optional media facets.
    pub applied_facets: Option<Vec<String>>,
    /// Optional evaluation priority.
    pub priority: Option<i32>,
    /// Optional enabled state.
    pub enabled: Option<bool>,
}

#[derive(InputObject)]
/// Patches a rule set while preserving omitted values.
pub struct UpdateRuleSetInput {
    /// Rule-set ID.
    pub id: ID,
    /// Replacement name, or null to preserve.
    pub name: Option<String>,
    /// Replacement description, or null to preserve.
    pub description: Option<String>,
    /// Replacement Rego source, or null to preserve.
    pub rego_source: Option<String>,
    /// Replacement facet list, or null to preserve.
    pub applied_facets: Option<Vec<String>>,
    /// Replacement priority, or null to preserve.
    pub priority: Option<i32>,
    /// Narrow a managed locale pack to titles carrying one of these tags. An
    /// empty list clears the filter so the pack applies wherever its facts
    /// match. Rejected for user-authored rule sets.
    pub managed_tag_filter: Option<Vec<String>>,
}

#[derive(InputObject)]
/// Enables or disables one rule set.
pub struct ToggleRuleSetInput {
    /// Rule-set ID.
    pub id: ID,
    /// Desired enabled state.
    pub enabled: bool,
}

#[derive(InputObject)]
/// Validates Rego source, optionally in the context of an existing rule set.
pub struct ValidateRuleSetInput {
    /// Rego source to validate.
    pub rego_source: String,
    /// Existing rule-set ID for context, or null for standalone validation.
    pub rule_set_id: Option<ID>,
}

#[derive(InputObject)]
/// Sets a title-level required-audio-language override.
pub struct SetTitleRequiredAudioInput {
    /// Target title ID.
    pub title_id: ID,
    /// The facet of the title: "movie", "series", or "anime"
    pub facet: MediaFacetValue,
    /// `null` removes the override and inherits from the library or facet.
    /// `[]` stores an explicit "no required languages" override for the title.
    /// Use `original` to resolve the title's original language dynamically.
    pub languages: Option<Vec<String>>,
}

#[derive(SimpleObject, Clone)]
/// Result of setting a title's required-audio-language override.
pub struct SetTitleRequiredAudioPayload {
    /// Target title ID.
    pub title_id: ID,
    /// Title media facet.
    pub facet: MediaFacetValue,
    /// Effective override languages; null means inherited behavior.
    pub languages: Option<Vec<String>>,
    /// Whether the stored value changed.
    pub updated: bool,
}

#[derive(SimpleObject, Clone)]
/// Snapshot of the in-memory service log buffer.
pub struct ServiceLogsPayload {
    /// UTC time when the snapshot was generated.
    pub generated_at: DateTime<Utc>,
    /// Log lines returned, newest or oldest order as defined by the service buffer.
    pub lines: Vec<String>,
    /// Number of returned lines.
    pub count: i32,
}
