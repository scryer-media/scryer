use async_graphql::{Enum, ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};

// ── Enums ──────────────────────────────────────────────────────────────────

/// How a maintenance rule set is evaluated.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceEvaluationMode {
    /// Stored but never evaluated. The only mode this release accepts.
    Disabled,
    /// Evaluate and record candidates, never act.
    Shadow,
    /// Evaluate, record, and surface candidates for manual approval.
    Observe,
}

/// Granularity a maintenance rule set is scoped to.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceRuleSubjectKind {
    /// The rule evaluates whole titles, both movies and shows.
    Title,
}

/// The closed catalog of actions a maintenance rule revision can authorize.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceActionKind {
    /// Track membership only, and mutate no media.
    DoNothing,
    /// Unmonitor the matched scope and keep its files.
    UnmonitorScopeKeepFiles,
    /// Delete the title and its files.
    DeleteTitleAndFiles,
    /// Unmonitor the title and delete its files while preserving the title.
    UnmonitorTitleDeleteAllFiles,
    /// Unmonitor the show and its seasons, and delete existing episode files.
    UnmonitorShowDeleteExistingFiles,
    /// Unmonitor the season or episode scope and delete its files.
    UnmonitorScopeDeleteFiles,
    /// Unmonitor the season and delete its files, then delete the show when a
    /// fresh parent check proves it is empty.
    UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty,
    /// Unmonitor the season, then unmonitor the show when a fresh parent check
    /// proves it is empty.
    UnmonitorSeasonThenUnmonitorShowIfEmpty,
    /// Change the quality profile and search only when the current profile
    /// differs from the target.
    ChangeQualityProfileAndSearchIfChanged,
}

/// A media subject a maintenance action can be configured against.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceActionSubject {
    /// A movie title.
    Movie,
    /// A show title.
    Show,
    /// One season of a show.
    Season,
    /// One episode of a show.
    Episode,
}

/// How destructive a maintenance action is, and therefore the minimum control
/// its execution requires.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceRiskClass {
    /// Mutates no managed media or catalog state.
    None,
    /// Reversible Scryer state, a notification, or a bounded refresh.
    Low,
    /// Changes acquisition intent, starts external work, or reorganizes files.
    Medium,
    /// Deletes, blocklists, or can otherwise make media unavailable.
    High,
}

/// What a matcher decided for one subject.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceOutcome {
    /// The rule matched the subject.
    Match,
    /// The rule did not match the subject.
    NoMatch,
    /// A fact the rule needs could not be observed, so the rule is held.
    Unknown,
}

// ── Read models ────────────────────────────────────────────────────────────

/// A maintenance rule set: identity, scope, and evaluation state. The matcher
/// and the action it authorizes live on the revision, never here.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceRuleSet {
    /// Rule-set ID.
    pub id: ID,
    /// Rule-set name.
    pub name: String,
    /// Rule-set description; empty when the author supplied none.
    pub description: String,
    /// Whether the rule set is enabled. Always false while maintenance rules
    /// ship dark.
    pub enabled: bool,
    /// Evaluation mode currently stored for the rule set.
    pub evaluation_mode: MaintenanceEvaluationMode,
    /// Libraries the rule is confined to. Empty means every library.
    pub library_ids: Vec<String>,
    /// Granularity the rule is scoped to.
    pub subject_kind: MaintenanceRuleSubjectKind,
    /// Revision number of the matcher currently in force.
    pub current_revision_number: i32,
    /// Grace period of the revision currently in force, in days.
    pub grace_days: i32,
    /// Action the revision currently in force authorizes.
    pub action_spec: MaintenanceActionSpec,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

/// One immutable revision of a maintenance rule set's matcher and action.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceRuleRevision {
    /// Revision ID.
    pub id: ID,
    /// ID of the rule set this revision belongs to.
    pub rule_set_id: ID,
    /// Revision number, starting at one and incremented on every matcher edit.
    pub revision_number: i32,
    /// Rego source as the editor should show it, with the package declaration
    /// and the rego.v1 import stripped.
    pub rego_source: String,
    /// Days a subject must match continuously before the action becomes due.
    pub grace_days: i32,
    /// Hash of the exact stored source, used to attribute a decision to the
    /// revision that produced it.
    pub matcher_content_hash: String,
    /// ID of the user who wrote the revision, or null when unattributed.
    pub created_by: Option<ID>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
}

/// The closed, schema-versioned action configuration a rule revision stores.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceActionSpec {
    /// Catalog action the revision authorizes.
    pub kind: MaintenanceActionKind,
    /// Schema version the stored parameters were validated against.
    pub schema_version: i32,
    /// Target quality profile, set only for the quality-profile action and null
    /// for every other kind.
    pub target_quality_profile_id: Option<String>,
}

/// A rule set together with the revision currently in force.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceRuleSetDetail {
    /// The rule set itself.
    pub rule_set: MaintenanceRuleSet,
    /// The revision currently in force.
    pub revision: MaintenanceRuleRevision,
    /// The action that revision authorizes, already decoded.
    pub action_spec: MaintenanceActionSpec,
}

/// Static registry entry describing one maintenance action to the rule builder.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceActionDescriptor {
    /// Catalog action this descriptor describes.
    pub kind: MaintenanceActionKind,
    /// Subjects the action may be configured against.
    pub supported_subjects: Vec<MaintenanceActionSubject>,
    /// Risk class governing the controls the action requires.
    pub risk_class: MaintenanceRiskClass,
    /// Effect classes the action declares for conflict arbitration.
    pub effect_classes: Vec<String>,
    /// When a due action becomes eligible to execute.
    pub timing_mode: String,
    /// Repeat semantics the registry allows for the action.
    pub allowed_repeat_modes: Vec<String>,
    /// Whether configuring the action requires a target quality profile.
    pub requires_target_quality_profile: bool,
}

/// Identifier returned after deleting a maintenance rule set.
#[derive(SimpleObject, Clone)]
pub struct DeleteMaintenanceRuleSetPayload {
    /// Deleted rule-set ID.
    pub id: ID,
}

/// Result of validating maintenance rule source without saving it.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceRuleValidationPayload {
    /// Whether the source is valid.
    pub valid: bool,
    /// Validation errors; empty when valid.
    pub errors: Vec<String>,
}

/// One title's preview outcome. `outcome` is null exactly when `error` is set:
/// a rule that failed produced no decision, and a failure is never rendered as
/// a no-match.
#[derive(SimpleObject, Clone)]
pub struct MaintenancePreviewTitle {
    /// Evaluated title ID.
    pub title_id: ID,
    /// Evaluated title name.
    pub title_name: String,
    /// Media facet of the evaluated title.
    pub facet: String,
    /// Library containing the evaluated title.
    pub library_id: String,
    /// Decision the matcher reached, or null when evaluation failed.
    pub outcome: Option<MaintenanceOutcome>,
    /// Reason codes the matcher emitted; empty when it emitted none.
    pub reason_codes: Vec<String>,
    /// Why evaluation failed for this title, or null when it succeeded.
    pub error: Option<String>,
}

/// Outcome of running one matcher against a bounded title selection. Preview
/// persists nothing.
#[derive(SimpleObject, Clone)]
pub struct MaintenancePreviewPayload {
    /// Stored rule set the preview ran, or the throwaway ID an unsaved draft
    /// compiled under.
    pub rule_set_id: String,
    /// Hash of the exact source that produced these outcomes.
    pub matcher_content_hash: String,
    /// UTC time the preview evaluated the selection.
    pub evaluated_at: DateTime<Utc>,
    /// One entry per evaluated title, in selection order.
    pub titles: Vec<MaintenancePreviewTitle>,
}

// ── Inputs ─────────────────────────────────────────────────────────────────

/// Selects one catalog action and its parameters.
#[derive(InputObject, Clone)]
pub struct MaintenanceActionInput {
    /// Catalog action to authorize.
    pub kind: MaintenanceActionKind,
    /// Target quality profile. Required by the quality-profile action and
    /// rejected for every other kind.
    pub target_quality_profile_id: Option<String>,
}

/// Creates a maintenance rule set together with its first matcher revision.
#[derive(InputObject)]
pub struct CreateMaintenanceRuleSetInput {
    /// Rule-set name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Matcher source as written in the editor. The package declaration is
    /// applied by the server.
    pub rego_source: String,
    /// Action the first revision authorizes.
    pub action: MaintenanceActionInput,
    /// Days a subject must match continuously before the action becomes due;
    /// defaults to zero.
    pub grace_days: Option<i32>,
    /// Libraries to confine the rule to. Omitted or empty means every library.
    pub library_ids: Option<Vec<String>>,
}

/// Replaces the matcher of an existing rule set, appending a revision.
#[derive(InputObject)]
pub struct UpdateMaintenanceRuleMatcherInput {
    /// Rule-set ID.
    pub id: ID,
    /// Replacement matcher source as written in the editor.
    pub rego_source: String,
    /// Action the new revision authorizes.
    pub action: MaintenanceActionInput,
    /// Days a subject must match continuously before the action becomes due;
    /// defaults to zero.
    pub grace_days: Option<i32>,
}

/// Renames and re-scopes a rule set without touching its matcher.
#[derive(InputObject)]
pub struct UpdateMaintenanceRuleMetadataInput {
    /// Rule-set ID.
    pub id: ID,
    /// Replacement name.
    pub name: String,
    /// Replacement description; omitted clears it.
    pub description: Option<String>,
    /// Replacement library scope. Omitted or empty means every library.
    pub library_ids: Option<Vec<String>>,
}

/// Validates maintenance rule source without saving it.
#[derive(InputObject)]
pub struct ValidateMaintenanceRuleInput {
    /// Matcher source to validate, as written in the editor.
    pub rego_source: String,
}

/// Runs one matcher against a bounded title selection without saving anything.
///
/// Supply either `ruleSetId` to preview a stored rule set, or `regoSource` with
/// `action` to preview an unsaved draft, never both. Select titles with either
/// `titleIds` or `libraryId`, never both.
#[derive(InputObject)]
pub struct PreviewMaintenanceRuleInput {
    /// Stored rule set to preview at its current revision.
    pub rule_set_id: Option<ID>,
    /// Unsaved matcher source to preview, as written in the editor.
    pub rego_source: Option<String>,
    /// Action for the unsaved draft; required with `regoSource`.
    pub action: Option<MaintenanceActionInput>,
    /// Grace period for the unsaved draft; defaults to zero.
    pub grace_days: Option<i32>,
    /// Evaluate exactly these titles.
    pub title_ids: Option<Vec<ID>>,
    /// Evaluate the first titles of this library.
    pub library_id: Option<ID>,
    /// How many titles to evaluate for a library selection; defaults to twenty
    /// and is clamped to the preview cap.
    pub limit: Option<i32>,
}
