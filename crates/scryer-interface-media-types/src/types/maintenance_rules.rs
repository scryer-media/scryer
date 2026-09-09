use async_graphql::{Enum, ID, InputObject, MaybeUndefined, SimpleObject};
use chrono::{DateTime, Utc};

// ── Enums ──────────────────────────────────────────────────────────────────

/// How a maintenance rule set is evaluated.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceEvaluationMode {
    /// Stored but never evaluated. A disabled rule opens no candidates and runs
    /// no actions, and it is the mode every new rule set is created in.
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
    /// The rule evaluates each season independently, including specials.
    Season,
    /// The rule evaluates each episode independently.
    Episode,
}

/// How far a rule set's effects are armed. Arming is per rule and independent
/// of the instance effect gates; an action executes only when both permit it.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceEffectArming {
    /// Evaluate and track candidates, never act.
    None,
    /// Low- and medium-risk actions may execute.
    Reversible,
    /// High-risk actions may additionally execute. Requires acknowledging the
    /// rule's current candidate count when set.
    Destructive,
}

/// The closed catalog of actions a maintenance rule revision can authorize.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceActionKind {
    /// This revision contains an ordered action sequence. Read
    /// `actionSequence` for the authoritative steps; it is never a legacy
    /// action input value.
    ActionSequence,
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
    /// Add the configured user tags to the matched title.
    AddTags,
    /// Remove the configured user tags from the matched title.
    RemoveTags,
}

/// Lifecycle state of one maintenance candidate.
///
/// Two writers move a candidate through these states. The scheduled evaluator
/// opens and closes membership, writing only `OBSERVING`, `CANCELED`, and
/// `EXCLUDED`. The action handler owns the rest: it leases a due candidate into
/// `EXECUTING` and writes the outcome, and it only ever selects candidates of a
/// rule that is in `OBSERVE` mode, is armed, and whose matching instance effect
/// gate is on.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceCandidateState {
    /// Matching, with the grace clock running.
    Observing,
    /// Grace elapsed and effect arming permits the action; awaiting the handler.
    PendingAction,
    /// Eligible to execute now.
    Due,
    /// An action lease is held for this candidate.
    Executing,
    /// The action completed.
    Succeeded,
    /// The action failed terminally.
    Failed,
    /// The subject stopped matching, or its rule revision was superseded.
    Canceled,
    /// An exclusion covers the subject.
    Excluded,
    /// A safety precondition refused the action.
    Blocked,
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
    /// Whether the rule set is enabled. Derived from `evaluationMode` rather
    /// than set independently: false for `DISABLED`, true for `SHADOW` and
    /// `OBSERVE`.
    pub enabled: bool,
    /// Evaluation mode currently stored for the rule set.
    pub evaluation_mode: MaintenanceEvaluationMode,
    /// How far this rule's effects are armed, independent of its mode.
    pub effect_arming: MaintenanceEffectArming,
    /// This rule was disarmed because new show facts can change its destructive
    /// title matches. Review the matcher and arm it again to acknowledge them.
    pub destructive_rearm_required: bool,
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
    /// Ordered schema-2 actions for the revision currently in force. Legacy
    /// rows expose their unchanged action spec and leave this null.
    pub action_sequence: Option<MaintenanceActionSequence>,
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
    /// Configured library root whose capacity and owned files this revision
    /// uses, or null when it is not storage-scoped.
    pub storage_root_id: Option<String>,
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
    /// Tags the action writes, set only for the tag actions and empty for every
    /// other kind. Labels are registry-defined and stored lowercase.
    pub tags: Vec<String>,
}

/// One typed parameter set for a sequence step. The action catalog decides
/// which fields are meaningful for each `kind`; the application rejects every
/// other shape before the sequence can be stored.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceActionStepParameters {
    /// Whether a title-level unmonitor step also unmonitors its seasons and
    /// episodes. This is required before a title file-deletion step can cover
    /// a show completely.
    pub include_descendants: Option<bool>,
    /// Target profile for a quality-profile step.
    pub target_quality_profile_id: Option<String>,
    /// Dispatch condition for a search step.
    pub search_condition: Option<MaintenanceSearchCondition>,
    /// Tags written by an add-tags or remove-tags step.
    pub tags: Vec<String>,
}

/// The closed catalog of sequence step kinds. These kinds are intentionally
/// separate from the legacy maintenance-action catalog: a sequence never
/// serializes as a synthetic legacy action.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceActionStepKind {
    /// Stop monitoring the selected scope and, when configured, its descendants.
    Unmonitor,
    /// Delete only the selected scope's authorized media files.
    DeleteFiles,
    /// Change the selected title's quality profile.
    ChangeQualityProfile,
    /// Request an acquisition search for the selected title.
    Search,
    /// Add configured tags to the selected title.
    AddTags,
    /// Remove configured tags from the selected title.
    RemoveTags,
    /// Delete the selected title record and its authorized files.
    DeleteTitleAndFiles,
}

/// The explicit condition a search step applies before requesting work.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MaintenanceSearchCondition {
    /// Always request the search when the step executes.
    Unconditional,
    /// Request the search only when the preceding profile step changed a profile.
    PreviousProfileChanged,
}

/// One ordered action in a revision-bound sequence.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceActionStep {
    /// Stable identifier used by durable execution receipts and history.
    pub id: ID,
    /// Catalog primitive this ordered step executes.
    pub kind: MaintenanceActionStepKind,
    /// Typed parameters accepted by this step's catalog primitive.
    pub parameters: MaintenanceActionStepParameters,
}

/// The authoritative, schema-versioned sequence a revision authorizes.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceActionSequence {
    /// Schema version that defines the accepted step representation.
    pub schema_version: i32,
    /// Ordered steps that make up the revision's action sequence.
    pub steps: Vec<MaintenanceActionStep>,
}

/// Backend-owned capabilities for one schema-2 primitive.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceActionStepDescriptor {
    /// Stable catalog identifier for this primitive.
    pub id: String,
    /// Primitive represented by this descriptor.
    pub kind: MaintenanceActionStepKind,
    /// Operator-facing label for the primitive.
    pub label: String,
    /// Subjects that may use this primitive.
    pub supported_subjects: Vec<MaintenanceActionSubject>,
    /// Name of the typed parameter shape accepted by this primitive.
    pub parameter_schema: String,
    /// Effects used when arbitrating conflicts with other steps.
    pub effect_classes: Vec<String>,
    /// Risk class that determines the required arming level.
    pub risk_class: MaintenanceRiskClass,
    /// Preconditions this primitive requires before it can execute.
    pub requires: Vec<String>,
    /// Whether this primitive ends the sequence when it succeeds.
    pub terminal: bool,
    /// How completion for this primitive is determined.
    pub completion_policy: String,
    /// Whether this primitive is permitted for a storage-root-scoped rule.
    pub storage_root_allowed: bool,
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
    /// Ordered actions for a schema-2 revision. Legacy revisions keep this
    /// null and continue to expose their original `actionSpec` unchanged.
    pub action_sequence: Option<MaintenanceActionSequence>,
}

/// Static registry entry describing one maintenance action to the rule builder.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceActionDescriptor {
    /// Scopes for which this build implements the action.
    pub supported_rule_scopes: Vec<MaintenanceRuleSubjectKind>,
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
    /// Whether configuring the action requires at least one title tag.
    pub requires_tags: bool,
    /// Whether this action remains valid when a configured storage root limits
    /// the files it may affect.
    pub supports_storage_scope: bool,
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

/// One subject's preview outcome. `outcome` is null exactly when `error` is set:
/// a rule that failed produced no decision, and a failure is never rendered as
/// a no-match.
#[derive(SimpleObject, Clone)]
pub struct MaintenancePreviewTitle {
    /// Whether a direct or inherited exclusion protects this subject.
    pub excluded: bool,
    /// Existing grace deadline, or an estimated deadline for a new match.
    pub due_at: Option<DateTime<Utc>>,
    /// True when the deadline is estimated rather than persisted on a candidate.
    pub due_at_is_estimate: bool,
    /// Display label for the exact title, season, or episode being evaluated.
    pub subject_label: String,
    /// Number of current files associated with this subject.
    pub file_count: i32,
    /// Combined size in bytes of this subject's current files.
    pub total_size_bytes: i64,
    /// Number of files eligible for deletion on the selected storage root, or
    /// null when this matcher does not select a resolvable root.
    pub storage_root_file_count: Option<i32>,
    /// Combined bytes eligible for deletion on the selected storage root, or
    /// null when this matcher does not select a resolvable root.
    pub storage_root_total_size_bytes: Option<i64>,
    /// Subject scope: title, season, or episode.
    pub subject_kind: String,
    /// Scryer title, collection, or episode ID for the selected scope.
    pub subject_id: ID,
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
    /// Why evaluation failed for this subject, or null when it succeeded.
    pub error: Option<String>,
}

/// Outcome of running one matcher against a bounded subject selection. Preview
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
    /// One entry per evaluated subject, including its owning title.
    pub titles: Vec<MaintenancePreviewTitle>,
}

/// One subject's durable membership in one maintenance rule set.
///
/// A candidate records that a rule matched the subject, how long its grace
/// period still has to run, and, once the handler has acted, what happened.
/// `state` is the authority on which of those a given row is: opening a
/// candidate never acts on anything, and only `EXECUTING`, `SUCCEEDED`, and
/// `FAILED` mean an action was attempted.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceCandidate {
    /// Number of current subject files, or null if the subject is unavailable.
    pub file_count: Option<i32>,
    /// Current subject file size in bytes, or null if the subject is unavailable.
    pub total_size_bytes: Option<i64>,
    /// Number of unambiguously owned current subject files on the root selected
    /// by this candidate's pinned revision, or null when no root is selected or
    /// exact root ownership cannot be established.
    pub storage_root_file_count: Option<i32>,
    /// Total bytes of unambiguously owned current subject files on the root
    /// selected by this candidate's pinned revision. It is null under the same
    /// conditions as `storageRootFileCount`.
    pub storage_root_total_size_bytes: Option<i64>,
    /// Exact subject display label, falling back to its ID if unavailable.
    pub subject_label: String,
    /// Subject scope: title, season, or episode.
    pub subject_kind: String,
    /// Scryer title, collection, or episode ID identifying this candidate's subject.
    pub subject_id: ID,
    /// Candidate ID.
    pub id: ID,
    /// Rule set that produced the candidate.
    pub rule_set_id: ID,
    /// Name of that rule set.
    pub rule_name: String,
    /// Rule revision in force when the candidate was opened.
    pub revision_number: i32,
    /// Subject title ID.
    pub title_id: ID,
    /// Subject title name, or the stored title ID when the title is gone.
    pub title_name: String,
    /// Library the subject belonged to when the candidate was opened.
    pub library_id: String,
    /// Media facet of the subject.
    pub facet: String,
    /// Current lifecycle state.
    pub state: MaintenanceCandidateState,
    /// Why the candidate is in that state.
    pub state_reason: String,
    /// Reason codes the matcher emitted on the most recent match.
    pub reason_codes: Vec<String>,
    /// Action the rule revision authorizes, and the one the handler will run if
    /// this candidate becomes due while the rule is in `OBSERVE` mode, armed for
    /// the action's risk class, and its instance effect gate is on.
    pub action_kind: MaintenanceActionKind,
    /// The immutable schema-2 sequence that opened this candidate. Legacy
    /// candidates retain their original `actionKind` and leave this null.
    pub action_sequence: Option<MaintenanceActionSequence>,
    /// Ordered latest evidence for each schema-2 step. An empty sequence is an
    /// observe-only candidate rather than a missing action.
    pub sequence_steps: Vec<MaintenanceActionStepProgress>,
    /// Days the subject must match continuously before the action is due.
    pub grace_days: i32,
    /// Increments each time a fresh candidate is opened for this subject, so a
    /// cancel-then-rematch is distinguishable from a continuing membership.
    pub match_generation: i32,
    /// When the subject started matching. Never reset while the candidate lives.
    pub first_matched_at: DateTime<Utc>,
    /// When the subject most recently matched.
    pub last_matched_at: DateTime<Utc>,
    /// When the grace period elapses, equal to `firstMatchedAt` for a zero-day
    /// grace period.
    pub due_at: DateTime<Utc>,
    /// Set while the latest evaluation could not decide, and null otherwise.
    pub held_since: Option<DateTime<Utc>>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

/// The latest durable checkpoint for one ordered schema-2 step.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceActionStepRun {
    /// Stable identifier of the sequence step this checkpoint belongs to.
    pub step_id: ID,
    /// Stored catalog kind for the step checkpoint.
    pub step_kind: String,
    /// Latest durable state for this step.
    pub state: String,
    /// Number of durable attempts made for this step.
    pub attempt: i32,
    /// Reason execution is waiting, when the step is held.
    pub hold_reason: Option<String>,
    /// Failure message for the latest step attempt, when any.
    pub error: Option<String>,
    /// UTC time the step checkpoint was created.
    pub created_at: DateTime<Utc>,
    /// UTC time the step checkpoint was last updated.
    pub updated_at: DateTime<Utc>,
    /// UTC time the latest step attempt finished, or null while active.
    pub finished_at: Option<DateTime<Utc>>,
}

/// Durable receipt for an external job requested by a schema-2 action step.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceActionJobReceipt {
    /// Dispatch ordinal for this durable external-job request.
    pub dispatch_attempt: i32,
    /// Idempotent key identifying the requested external job.
    pub logical_request_key: String,
    /// External job-run identifier after the request is accepted.
    pub job_run_id: Option<ID>,
    /// Latest reconciliation state of the external job request.
    pub state: String,
    /// UTC time this receipt was created.
    pub created_at: DateTime<Utc>,
    /// UTC time this receipt was last reconciled.
    pub updated_at: DateTime<Utc>,
}

/// One configured step with the latest outcome and any durable job receipts.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceActionStepProgress {
    /// Immutable step definition from the sequence revision.
    pub step: MaintenanceActionStep,
    /// Latest durable checkpoint for this step, or null before execution starts.
    pub run: Option<MaintenanceActionStepRun>,
    /// Durable external-job receipts associated with this step.
    pub receipts: Vec<MaintenanceActionJobReceipt>,
}

/// One rule set's pass through one evaluation run.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceEvaluationRun {
    /// Evaluation-run ID.
    pub id: ID,
    /// Rule set that was evaluated.
    pub rule_set_id: ID,
    /// Rule revision that was evaluated.
    pub revision_number: i32,
    /// One of running, succeeded, or failed. A run left running is the trace of
    /// an interrupted pass.
    pub status: String,
    /// When the pass started.
    pub started_at: DateTime<Utc>,
    /// When the pass finished, or null while it is still running.
    pub finished_at: Option<DateTime<Utc>>,
    /// Subjects the matcher was run against.
    pub evaluated_count: i32,
    /// Subjects the matcher matched.
    pub matched_count: i32,
    /// Subjects the matcher did not match.
    pub no_match_count: i32,
    /// Subjects the matcher could not decide, which are held.
    pub unknown_count: i32,
    /// Subjects whose evaluation failed, which are also held.
    pub error_count: i32,
    /// How long the pass took, or null while it is still running.
    pub duration_ms: Option<i32>,
    /// Why the pass failed, or null when it did not.
    pub error: Option<String>,
}

/// The five independent instance-wide maintenance gates. Every one of them
/// defaults off, an unconfigured instance therefore evaluates nothing and acts
/// on nothing, and each gate is read at the start of every scheduled pass so a
/// change takes effect on the next run without a restart.
///
/// A gate is only ever the instance half of a permission. Executing an action
/// additionally requires the individual rule to be in `OBSERVE` mode and armed
/// for that action's risk class, so opening a gate cannot by itself start
/// anything.
#[derive(SimpleObject, Clone, Copy)]
pub struct MaintenanceInstanceGates {
    /// Whether the scheduled evaluator may run at all. While this is off no rule
    /// is evaluated, no candidate is opened or closed, and no evaluation run is
    /// recorded.
    pub evaluation_enabled: bool,
    /// Whether candidate results are returned to clients.
    pub result_display_enabled: bool,
    /// Reserved for provider collection projection and lifecycle notifications.
    /// Nothing reads it yet.
    pub presentation_effects_enabled: bool,
    /// Whether low and medium risk actions may execute, such as unmonitoring a
    /// title or changing its quality profile. A rule armed as `REVERSIBLE` or
    /// `DESTRUCTIVE` starts running those actions on its due candidates while
    /// this is on.
    pub reversible_effects_enabled: bool,
    /// Whether high risk actions may execute, meaning the ones that delete media
    /// files. Turning this on lets every rule already armed as `DESTRUCTIVE`
    /// begin deleting its due candidates on the next handler pass, without any
    /// further confirmation.
    pub destructive_effects_enabled: bool,
}

/// A subject a maintenance rule must never act on.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceExclusion {
    /// Exact excluded subject label, falling back to its ID if unavailable.
    pub subject_label: String,
    /// Excluded scope: title, season, or episode. Parent exclusions cover descendants.
    pub subject_kind: String,
    /// Scryer title, collection, or episode ID of the excluded subject.
    pub subject_id: ID,
    /// Exclusion ID.
    pub id: ID,
    /// Rule the exclusion is confined to, or null when it is global.
    pub rule_set_id: Option<ID>,
    /// Excluded title ID.
    pub title_id: ID,
    /// Excluded title name, or the stored title ID when the title is gone.
    pub title_name: String,
    /// Why the subject was excluded; empty when the author gave no reason.
    pub reason: String,
    /// ID of the user who added the exclusion, or null when unattributed.
    pub created_by: Option<ID>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
}

/// Identifier returned after removing a maintenance exclusion.
#[derive(SimpleObject, Clone)]
pub struct DeleteMaintenanceExclusionPayload {
    /// Removed exclusion ID.
    pub id: ID,
}

/// One recorded action-handler attempt on one candidate, holds included.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceActionRun {
    /// Exact subject display label from execution evidence or the current catalog.
    pub subject_label: String,
    /// Persisted per-file outcomes and policy provenance.
    pub detail: String,
    /// Persisted subject scope: title, season, or episode.
    pub subject_kind: String,
    /// Persisted Scryer title, collection, or episode ID targeted by this attempt.
    pub subject_id: ID,
    /// Action-run ID.
    pub id: ID,
    /// Rule set the attempt belongs to.
    pub rule_set_id: ID,
    /// Candidate the attempt acted on.
    pub candidate_id: ID,
    /// Title the attempt acted on.
    pub title_id: ID,
    /// Display name of the title, or its stored ID when the title is gone.
    pub title_name: String,
    /// Action the attempt performed or refused.
    pub action_kind: MaintenanceActionKind,
    /// The immutable schema-2 sequence this attempt executed. Legacy attempts
    /// retain their original `actionKind` and leave this null.
    pub action_sequence: Option<MaintenanceActionSequence>,
    /// Ordered latest evidence for every step in the immutable sequence.
    pub sequence_steps: Vec<MaintenanceActionStepProgress>,
    /// Match generation of the candidate at attempt time.
    pub match_generation: i32,
    /// Attempt number for this candidate generation, starting at one.
    pub attempt: i32,
    /// Outcome: running, succeeded, already_satisfied, failed, or held.
    pub status: String,
    /// Why a held attempt was refused; null otherwise.
    pub hold_reason: Option<String>,
    /// Error text of a failed attempt; null otherwise.
    pub error: Option<String>,
    /// UTC start time of the attempt.
    pub started_at: DateTime<Utc>,
    /// UTC finish time; null while the attempt is running.
    pub finished_at: Option<DateTime<Utc>>,
}

/// Outcome of asking for an immediate evaluation pass.
#[derive(SimpleObject, Clone)]
pub struct MaintenanceEvaluationTriggerPayload {
    /// Whether a pass was started. False when a gate, a disabled rule, or a
    /// run already in progress stopped it.
    pub started: bool,
    /// What happened, suitable for showing to an operator.
    pub message: Option<String>,
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
    /// Tags to add or remove. Required by the tag actions, ignored by every
    /// other kind, and rejected unless every label is already defined in the
    /// title-tag registry.
    pub tags: Option<Vec<String>>,
}

/// Parameters for one input sequence step. The selected `kind` determines the
/// accepted subset. Keeping this input closed and validated by the server
/// prevents Rego or the web client from inventing effects.
#[derive(InputObject, Clone)]
pub struct MaintenanceActionStepParametersInput {
    /// Include monitored descendants when unmonitoring a whole title.
    pub include_descendants: Option<bool>,
    /// Quality-profile identifier for a change-quality-profile step.
    pub target_quality_profile_id: Option<String>,
    /// Condition that controls whether a search step requests work.
    pub search_condition: Option<MaintenanceSearchCondition>,
    /// Tags to add or remove for a tag step.
    pub tags: Option<Vec<String>>,
}

/// One ordered input action with a caller-stable identifier.
#[derive(InputObject, Clone)]
pub struct MaintenanceActionStepInput {
    /// Caller-stable identifier used for durable step receipts and history.
    pub id: ID,
    /// Catalog primitive this input step configures.
    pub kind: MaintenanceActionStepKind,
    /// Typed parameters accepted by the selected primitive.
    pub parameters: MaintenanceActionStepParametersInput,
}

/// A versioned ordered sequence. An empty sequence is a valid observe-only
/// rule and is not represented with a placeholder action.
#[derive(InputObject, Clone)]
pub struct MaintenanceActionSequenceInput {
    /// Schema version that defines this sequence input representation.
    pub schema_version: i32,
    /// Ordered steps to store in the new immutable revision.
    pub steps: Vec<MaintenanceActionStepInput>,
}

/// Creates a maintenance rule set together with its first matcher revision.
#[derive(InputObject)]
pub struct CreateMaintenanceRuleSetInput {
    /// Defaults to title scope when omitted.
    pub subject_kind: Option<MaintenanceRuleSubjectKind>,
    /// Rule-set name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Matcher source as written in the editor. The package declaration is
    /// applied by the server.
    pub rego_source: String,
    /// Legacy action the first revision authorizes. Supply this or
    /// `actionSequence`, never both.
    pub action: Option<MaintenanceActionInput>,
    /// Ordered actions the first revision authorizes. Supply this or `action`,
    /// never both.
    pub action_sequence: Option<MaintenanceActionSequenceInput>,
    /// Days a subject must match continuously before the action becomes due;
    /// defaults to zero.
    pub grace_days: Option<i32>,
    /// Libraries to confine the rule to. Omitted or empty means every library.
    pub library_ids: Option<Vec<String>>,
    /// Configured library root whose capacity and owned files this revision
    /// uses. Required when the matcher reads a storage fact.
    pub storage_root_id: Option<String>,
}

/// Replaces the matcher of an existing rule set, appending a revision.
#[derive(InputObject)]
pub struct UpdateMaintenanceRuleMatcherInput {
    /// Rule-set ID.
    pub id: ID,
    /// Replacement matcher source as written in the editor.
    pub rego_source: String,
    /// Legacy action the new revision authorizes. Supply this or
    /// `actionSequence`, never both.
    pub action: Option<MaintenanceActionInput>,
    /// Ordered actions the new revision authorizes. Supply this or `action`,
    /// never both.
    pub action_sequence: Option<MaintenanceActionSequenceInput>,
    /// Days a subject must match continuously before the action becomes due;
    /// defaults to zero.
    pub grace_days: Option<i32>,
    /// Configured library root. Omission preserves the current root, null
    /// clears it, and a value replaces it.
    pub storage_root_id: MaybeUndefined<ID>,
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

/// Moves a rule set between evaluation modes.
#[derive(InputObject)]
pub struct SetMaintenanceRuleModeInput {
    /// Rule-set ID.
    pub id: ID,
    /// Mode to store. Anything other than `DISABLED` also enables the rule.
    pub mode: MaintenanceEvaluationMode,
}

/// Sets how far one rule set's effects are armed.
#[derive(InputObject)]
pub struct SetMaintenanceRuleArmingInput {
    /// Rule-set ID.
    pub id: ID,
    /// Arming level to store. `DESTRUCTIVE` requires acknowledging the rule's
    /// current candidate count and applies only to high-risk actions.
    pub arming: MaintenanceEffectArming,
    /// The candidate count the operator saw and acknowledged; required for
    /// `DESTRUCTIVE` and must equal the rule's current non-terminal count.
    pub acknowledged_candidate_count: Option<i32>,
}

/// Arms or disarms the instance-wide maintenance gates. An omitted field leaves
/// that gate exactly as stored.
#[derive(InputObject)]
pub struct SetMaintenanceInstanceGatesInput {
    /// Whether the scheduled evaluator may run.
    pub evaluation_enabled: Option<bool>,
    /// Whether candidate results are returned to clients.
    pub result_display_enabled: Option<bool>,
    /// Reserved for provider collection projection and lifecycle notifications.
    /// Nothing reads it yet.
    pub presentation_effects_enabled: Option<bool>,
    /// Whether low and medium risk actions may execute for rules that are armed
    /// and in `OBSERVE` mode.
    pub reversible_effects_enabled: Option<bool>,
    /// Whether high risk actions may execute. Setting this to true lets every
    /// rule already armed as `DESTRUCTIVE` begin deleting media files on the
    /// next handler pass.
    pub destructive_effects_enabled: Option<bool>,
}

/// Excludes one subject from maintenance rules.
#[derive(InputObject)]
pub struct ExcludeMaintenanceSubjectInput {
    /// Required for episode or season scope.
    pub subject_id: Option<ID>,
    /// Defaults to title scope when omitted.
    pub subject_kind: Option<MaintenanceRuleSubjectKind>,
    /// Title to exclude.
    pub title_id: ID,
    /// Rule to confine the exclusion to. Omitted means every rule.
    pub rule_set_id: Option<ID>,
    /// Why the subject is being excluded.
    pub reason: Option<String>,
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
/// exactly one action definition (`action` or `actionSequence`) to preview an
/// unsaved draft, never both. Select titles with either `titleIds` or
/// `libraryId`, never both.
#[derive(InputObject)]
pub struct PreviewMaintenanceRuleInput {
    /// Selected libraries for an unsaved matcher; empty means all libraries.
    pub library_ids: Option<Vec<String>>,
    /// Defaults to title scope when omitted.
    pub subject_kind: Option<MaintenanceRuleSubjectKind>,
    /// Stored rule set to preview at its current revision.
    pub rule_set_id: Option<ID>,
    /// Unsaved matcher source to preview, as written in the editor.
    pub rego_source: Option<String>,
    /// Legacy action for the unsaved draft. Supply this or `actionSequence`,
    /// never both.
    pub action: Option<MaintenanceActionInput>,
    /// Ordered actions for the unsaved draft. Supply this or `action`, never
    /// both.
    pub action_sequence: Option<MaintenanceActionSequenceInput>,
    /// Grace period for the unsaved draft; defaults to zero.
    pub grace_days: Option<i32>,
    /// Configured library root for an unsaved matcher. Required when its
    /// matcher reads a storage fact.
    pub storage_root_id: Option<String>,
    /// Optional exact subjects, requiring their owning `titleIds`.
    pub subject_ids: Option<Vec<ID>>,
    /// Evaluate exactly these titles.
    pub title_ids: Option<Vec<ID>>,
    /// Evaluate the first titles of this library.
    pub library_id: Option<ID>,
    /// How many titles to evaluate for a library selection; defaults to twenty
    /// and is clamped to the preview cap.
    pub limit: Option<i32>,
}
