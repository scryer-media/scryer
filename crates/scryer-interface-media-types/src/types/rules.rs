use super::{Long, MediaFacetValue};
use async_graphql::{Enum, ID, InputObject, Json, SimpleObject};
use chrono::{DateTime, Utc};
use scryer_application::arr_custom_format::{
    ArrCustomFormatTranslation, ArrDiagnostic, ArrDiagnosticSeverity, ArrFormatStatus,
    ArrSourceApplication,
};

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

/// Structured rule diagnostic. Coordinates refer to the compiled Rego source.
#[derive(SimpleObject, Clone)]
pub struct RuleValidationDiagnosticPayload {
    /// Stable condition code.
    pub code: String,
    /// Human-readable diagnostic for unknown-code fallback.
    pub message: String,
    /// One-based source line when the compiler supplies it.
    pub line: Option<i32>,
    /// One-based source column when the compiler supplies it.
    pub column: Option<i32>,
    /// Input path involved in the diagnostic.
    pub path: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Result of validating Rego source.
pub struct RuleValidationResultPayload {
    /// Structured conditions and source locations.
    pub diagnostics: Vec<RuleValidationDiagnosticPayload>,
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
    /// Release name to parse and score. Exactly one of this and
    /// `mediaFileId` is required.
    pub release_name: Option<String>,
    /// Release size in bytes, or null when unknown. Only with `releaseName`.
    pub size_bytes: Option<Long>,
    /// Indexer listing facts for `releaseName`, as a live search result would
    /// report them. Omitted facts are unknown (null to rules). Only with
    /// `releaseName`.
    pub listing: Option<RuleSetTestListingInput>,
    /// Stored media file of the selected title to score from its own row and
    /// the listing facts frozen when it was grabbed, instead of a release name.
    pub media_file_id: Option<ID>,
}

#[derive(InputObject, Default)]
/// Indexer listing facts a tested release name is scored with. The tester
/// treats the release as a fresh, never-grabbed search result.
pub struct RuleSetTestListingInput {
    /// Download protocol: usenet, torrent, or http. Other values are unknown.
    pub protocol: Option<String>,
    /// Publish time as an RFC 2822 (newznab `pubDate`) or RFC 3339
    /// timestamp, kept as written; `input.release.age_days` is measured from
    /// it to now.
    pub published_at: Option<String>,
    /// Indexer up-votes; never negative.
    pub thumbs_up: Option<i32>,
    /// Indexer down-votes; never negative.
    pub thumbs_down: Option<i32>,
    /// Whether the indexer reports the release as password protected.
    pub is_password_protected: Option<bool>,
    /// Audio languages the indexer reports.
    pub indexer_languages: Option<Vec<String>>,
    /// Indexer-specific attributes as a flat JSON object. Nested values are
    /// dropped and the map is size-bounded, as for a live listing.
    pub extra: Option<Json<serde_json::Value>>,
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
/// Indexer listing facts a scoring preview read, as rules saw them.
pub struct RuleSetTestListingPayload {
    /// Frozen download protocol: usenet, torrent, http, or null when unknown.
    pub protocol: Option<String>,
    /// Publish time the listing reported, or null when unknown.
    pub published_at: Option<String>,
    /// `input.release.age_days`: whole days from publish to `capturedAt`, or
    /// null when the publish time is unknown.
    pub age_days: Option<i32>,
    /// Indexer up-votes, or null when unknown.
    pub thumbs_up: Option<i32>,
    /// Indexer down-votes, or null when unknown.
    pub thumbs_down: Option<i32>,
    /// Indexer password-protection flag, or null when unknown.
    pub is_password_protected: Option<bool>,
    /// Audio languages the indexer reported.
    pub indexer_languages: Vec<String>,
    /// Bounded indexer-specific attributes rules read as `input.release.extra`.
    pub extra: Json<serde_json::Value>,
    /// When the facts were captured: the grab for a stored file, now for a
    /// tested release name. Release age is measured at this instant.
    pub captured_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Explicit, read-only scoring preview for the current rule-editor draft.
pub struct TestRuleSetPayload {
    /// Release name that was scored: the supplied one, or a stored file's
    /// grabbed release title (its file name when it has none).
    pub release_name: String,
    /// Stored media file that was scored, or null for a release name.
    pub media_file_id: Option<ID>,
    /// Listing facts the rules read. Null for a stored file that has no
    /// listing snapshot (scanned in, adopted, or imported before snapshots
    /// were kept): every listing fact was unknown.
    pub listing: Option<RuleSetTestListingPayload>,
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

// ── Arr custom-format import ───────────────────────────────────────────────

/// The application a custom-format export came from.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ArrSourceApplication", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ArrSourceApplicationValue {
    /// Sonarr custom formats.
    Sonarr,
    /// Radarr custom formats.
    Radarr,
}

impl ArrSourceApplicationValue {
    pub fn into_domain(self) -> ArrSourceApplication {
        match self {
            Self::Sonarr => ArrSourceApplication::Sonarr,
            Self::Radarr => ArrSourceApplication::Radarr,
        }
    }
}

/// How much a custom-format import diagnostic matters.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ArrDiagnosticSeverity", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ArrDiagnosticSeverityValue {
    /// The format is disabled in the generated rule.
    Error,
    /// The format translates, but needs the operator's attention.
    Warning,
    /// Informational only.
    Info,
}

/// Whether a pasted custom format became an active condition.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ArrCustomFormatStatus", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ArrCustomFormatStatusValue {
    /// The format scores matching releases.
    Translated,
    /// The format is kept as a disabled stub that never matches.
    Disabled,
}

#[derive(InputObject)]
/// The score the operator chose for one pasted custom format.
pub struct ArrCustomFormatScoreInput {
    /// Format ID from a previous translation of the same paste.
    pub format_id: String,
    /// Score added when the format matches.
    pub score: i32,
}

#[derive(InputObject)]
/// Pasted Sonarr or Radarr custom-format JSON and the scores to apply.
pub struct TranslateArrCustomFormatsInput {
    /// The application the JSON was exported from.
    pub source_application: ArrSourceApplicationValue,
    /// One custom format object, or an array of them, as exported.
    pub custom_format_json: String,
    /// Scores per format ID. A format without a score translates as disabled,
    /// so an empty list reviews the paste.
    pub scores: Vec<ArrCustomFormatScoreInput>,
}

/// One finding about a pasted custom format.
#[derive(SimpleObject, Clone)]
pub struct ArrCustomFormatDiagnosticPayload {
    /// Stable condition code, such as `regex.parse_failed`.
    pub code: String,
    /// Diagnostic severity.
    pub severity: ArrDiagnosticSeverityValue,
    /// Human-readable explanation for display.
    pub message: String,
    /// JSON-path-like location in the pasted document, when known.
    pub location: Option<String>,
    /// Format the diagnostic belongs to, when it belongs to one.
    pub format_id: Option<String>,
    /// Zero-based specification index within the format, when known.
    pub specification_index: Option<i32>,
}

impl From<ArrDiagnostic> for ArrCustomFormatDiagnosticPayload {
    fn from(diagnostic: ArrDiagnostic) -> Self {
        Self {
            code: diagnostic.code.to_string(),
            severity: match diagnostic.severity {
                ArrDiagnosticSeverity::Error => ArrDiagnosticSeverityValue::Error,
                ArrDiagnosticSeverity::Warning => ArrDiagnosticSeverityValue::Warning,
                ArrDiagnosticSeverity::Info => ArrDiagnosticSeverityValue::Info,
            },
            message: diagnostic.message,
            location: diagnostic.location,
            format_id: diagnostic.format_id,
            specification_index: diagnostic
                .specification_index
                .and_then(|index| i32::try_from(index).ok()),
        }
    }
}

/// A score the export suggests for a format.
#[derive(SimpleObject, Clone)]
pub struct ArrCustomFormatSuggestedScorePayload {
    /// Score label from the export, such as `default`.
    pub label: String,
    /// Suggested score; exports may carry non-integer values.
    pub score: f64,
}

/// One specification the import read from a format.
#[derive(SimpleObject, Clone)]
pub struct ArrCustomFormatSpecificationPayload {
    /// Zero-based index in the pasted specifications array.
    pub index: i32,
    /// Arr implementation name.
    pub implementation: String,
    /// Specification name.
    pub name: String,
    /// Whether the condition is negated.
    pub negate: bool,
    /// Whether the condition is required.
    pub required: bool,
}

/// One pasted custom format and how it translated.
#[derive(SimpleObject, Clone)]
pub struct ArrCustomFormatPayload {
    /// Format ID; pass it back with a score to translate the format.
    pub id: String,
    /// Format name.
    pub name: String,
    /// Format description, when the export has one.
    pub description: Option<String>,
    /// Translation status.
    pub status: ArrCustomFormatStatusValue,
    /// Why a disabled format was disabled. Empty for translated formats.
    pub diagnostics: Vec<ArrCustomFormatDiagnosticPayload>,
    /// Scores the export suggests, in export order.
    pub suggested_scores: Vec<ArrCustomFormatSuggestedScorePayload>,
    /// Specifications the import read.
    pub specifications: Vec<ArrCustomFormatSpecificationPayload>,
    /// What inspection found in this format.
    pub inspection_diagnostics: Vec<ArrCustomFormatDiagnosticPayload>,
}

/// Rego source generated from pasted custom formats. Nothing is saved.
#[derive(SimpleObject, Clone)]
pub struct ArrCustomFormatTranslationPayload {
    /// Generated Rego source for the rule editor.
    pub rego_source: String,
    /// Suggested rule name.
    pub name: String,
    /// Suggested rule description.
    pub description: String,
    /// Media facets the rule should apply to.
    pub applied_facets: Vec<String>,
    /// Every format inspection could read, in paste order.
    pub formats: Vec<ArrCustomFormatPayload>,
    /// Diagnostics of the disabled formats.
    pub diagnostics: Vec<ArrCustomFormatDiagnosticPayload>,
    /// Everything inspection found, including formats it could not read.
    pub inspection_diagnostics: Vec<ArrCustomFormatDiagnosticPayload>,
}

impl From<ArrCustomFormatTranslation> for ArrCustomFormatTranslationPayload {
    fn from(translation: ArrCustomFormatTranslation) -> Self {
        let diagnostics = |items: Vec<ArrDiagnostic>| items.into_iter().map(Into::into).collect();
        Self {
            rego_source: translation.rego_source,
            name: translation.name,
            description: translation.description,
            applied_facets: translation.applied_facets,
            formats: translation
                .formats
                .into_iter()
                .map(|format| ArrCustomFormatPayload {
                    id: format.id,
                    name: format.name,
                    description: format.description,
                    status: match format.status {
                        ArrFormatStatus::Translated => ArrCustomFormatStatusValue::Translated,
                        ArrFormatStatus::Disabled => ArrCustomFormatStatusValue::Disabled,
                    },
                    diagnostics: diagnostics(format.diagnostics),
                    suggested_scores: format
                        .suggested_scores
                        .into_iter()
                        .map(|(label, score)| ArrCustomFormatSuggestedScorePayload { label, score })
                        .collect(),
                    specifications: format
                        .specifications
                        .into_iter()
                        .map(|spec| ArrCustomFormatSpecificationPayload {
                            index: i32::try_from(spec.index).unwrap_or(i32::MAX),
                            implementation: spec.implementation,
                            name: spec.name,
                            negate: spec.negate,
                            required: spec.required,
                        })
                        .collect(),
                    inspection_diagnostics: diagnostics(format.inspection_diagnostics),
                })
                .collect(),
            diagnostics: diagnostics(translation.diagnostics),
            inspection_diagnostics: diagnostics(translation.inspection_diagnostics),
        }
    }
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
