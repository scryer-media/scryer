use super::{Long, MediaFacetValue};
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
    /// Baseline rules establish the subtotal read by additional rules.
    pub evaluation_phase: String,
    /// Why a retained rule was disabled during a context migration.
    pub disabled_reason: Option<String>,
    /// Other rules with this group cannot be enabled simultaneously.
    pub exclusive_group: Option<String>,
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
/// Unsaved rule-set fields evaluated by a scoring preview.
pub struct RuleSetTestDraftInput {
    /// Rule name used in the preview output.
    pub name: String,
    /// Rule description retained for the editor draft.
    pub description: String,
    /// Complete Rego source for the unsaved draft.
    pub rego_source: String,
    /// Whether the draft participates in the preview policy set.
    pub enabled: bool,
    /// Evaluation priority for the draft.
    pub priority: i32,
    /// Media facets to which the draft applies.
    pub applied_facets: Vec<String>,
}

#[derive(InputObject)]
/// Input for an explicit, read-only title-aware rule-set scoring preview.
pub struct TestRuleSetInput {
    /// Current unsaved editor draft. Omit when testing a saved rule set.
    pub draft: Option<RuleSetTestDraftInput>,
    /// Saved rule-set identity to test without supplying a draft.
    pub test_rule_set_id: Option<ID>,
    /// Existing rule-set identity being edited, or null when creating a rule.
    pub edit_rule_set_id: Option<ID>,
    /// Source identity when the editor was opened by copying a rule, or null.
    pub copy_source_rule_set_id: Option<ID>,
    /// Whether copying disables the source rule in the preview policy set.
    #[graphql(default = false)]
    pub copy_disables_source: bool,
    /// Library title used to construct title-aware scoring facts.
    pub title_id: ID,
    /// Episode belonging to the selected title, or null for a whole-title preview.
    pub episode_id: Option<ID>,
    /// Release name to parse and score.
    pub release_name: String,
    /// Release size in bytes, or null when unknown.
    pub size_bytes: Option<Long>,
}

#[cfg(test)]
mod saved_rule_preview_input_tests {
    use super::TestRuleSetInput;
    use async_graphql::{InputType, value};

    #[test]
    fn saved_rule_preview_input_needs_no_draft_or_copy_flag() {
        let input = TestRuleSetInput::parse(Some(value!({
            "testRuleSetId": "installed-rule",
            "titleId": "library-title",
            "releaseName": "A.Release.1080p"
        })))
        .unwrap_or_else(|_| panic!("minimal saved-rule request must parse"));
        assert_eq!(
            input.test_rule_set_id.as_ref().map(|id| id.as_str()),
            Some("installed-rule")
        );
        assert!(input.draft.is_none());
        assert!(input.edit_rule_set_id.is_none());
        assert!(input.copy_source_rule_set_id.is_none());
        assert!(!input.copy_disables_source);
        assert!(input.size_bytes.is_none());
    }
}

#[derive(SimpleObject, Clone)]
/// Title and library facts resolved for a rule-set scoring preview.
pub struct RuleSetTestContextPayload {
    /// Resolved title name.
    pub title_name: String,
    /// Resolved library name, or null when unavailable.
    pub library_name: Option<String>,
    /// Resolved media facet.
    pub facet: String,
    /// Resolved original language, or null when unavailable.
    pub language: Option<String>,
    /// Resolved title tags.
    pub tags: Vec<String>,
    /// Resolved episode label, or null for whole-title previews.
    pub episode_label: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Release facts parsed for a rule-set scoring preview.
pub struct RuleSetTestParsedPayload {
    /// Parsed release group, or null when unknown.
    pub release_group: Option<String>,
    /// Parsed quality, or null when unknown.
    pub quality: Option<String>,
    /// Parsed release source, or null when unknown.
    pub source: Option<String>,
    /// Season parsed from the release name, or null when absent.
    pub season: Option<String>,
    /// Episode parsed from the release name, or null when absent.
    pub episode: Option<String>,
    /// Parsed edition, or null when unknown.
    pub edition: Option<String>,
    /// Parsed video codec, or null when unknown.
    pub video_codec: Option<String>,
    /// Parsed audio codec, or null when unknown.
    pub audio: Option<String>,
    /// Parsed release year, or null when unknown.
    pub year: Option<i32>,
    /// Parsed audio-language codes.
    pub audio_languages: Vec<String>,
    /// Release size in bytes, or null when the caller omitted it.
    pub size_bytes: Option<Long>,
}

#[derive(SimpleObject, Clone)]
/// One rule-set's contribution to a title-aware scoring preview.
pub struct RuleSetTestRuleSetPayload {
    /// Stable rule-set identity.
    pub rule_set_id: Option<String>,
    /// Rule-set display name.
    pub rule_set_name: String,
    /// Existing policy origin.
    pub origin: String,
    /// Signed score contribution.
    pub score: i32,
    /// Whether this rule set matched the candidate facts.
    pub matched: bool,
    /// Whether this group contains an explicit rejection. Numeric penalties are recoverable.
    pub blocked: bool,
    /// Whether this entry represents the current editor draft.
    pub is_draft: bool,
    /// Individual numeric contributions and explicit rejection reasons.
    pub entries: Vec<RuleSetTestEntryPayload>,
    /// Evaluation messages and diagnostics for this rule set.
    pub messages: Vec<String>,
}

#[derive(SimpleObject, Clone)]
/// One numeric contribution or explicit rejection in a scoring preview.
pub struct RuleSetTestEntryPayload {
    /// Stable explanation code; the code does not determine rejection type.
    pub code: String,
    /// Signed score delta associated with the entry.
    pub delta: i32,
    /// Whether this is an explicit rejection, as identified by kind.
    pub blocked: bool,
    /// score_contribution, mandatory_rejection, or final_score_rejection.
    pub kind: String,
}

#[derive(SimpleObject, Clone)]
/// Current editor draft's contribution to a scoring preview.
pub struct RuleSetTestDraftContributionPayload {
    /// Signed score contribution from the draft.
    pub score: i32,
    /// Whether the draft matched the candidate facts.
    pub matched: bool,
    /// Compatibility field, always false: drafts emit recoverable numeric contributions.
    pub blocked: bool,
    /// Whether the draft applies to the selected title facet.
    pub applies: bool,
    /// Whether the draft is enabled.
    pub enabled: bool,
    /// Explanation for a disabled or non-applicable draft, or null when none.
    pub message: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// A validation or evaluation error retained by a scoring preview.
pub struct RuleSetTestErrorPayload {
    /// Stable error classification.
    pub code: String,
    /// Human-readable error detail.
    pub message: String,
    /// Rule-set identity associated with the error, or null when global.
    pub rule_set_id: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Explicit, read-only scoring preview for the current rule-editor draft.
pub struct TestRuleSetPayload {
    /// Aggregate score after normal policy aggregation.
    pub score: i32,
    /// Whether mandatory requirements and both final score gates pass.
    pub allowed: bool,
    /// Whether the completed decision is rejected by a requirement or score gate.
    pub blocked: bool,
    /// Whether the configured minimum-score gate passed.
    pub minimum_score_met: bool,
    /// Resolved quality-profile name.
    pub profile_name: String,
    /// Resolved title, library, and episode context.
    pub context: RuleSetTestContextPayload,
    /// Parsed release facts preserved from the supplied release name.
    pub parsed: RuleSetTestParsedPayload,
    /// Contributions grouped by policy rule set.
    pub rule_sets: Vec<RuleSetTestRuleSetPayload>,
    /// Dedicated explanation of the current draft's contribution.
    pub draft_contribution: RuleSetTestDraftContributionPayload,
    /// Validation and evaluation errors retained by the preview.
    pub errors: Vec<RuleSetTestErrorPayload>,
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
