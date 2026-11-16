use async_graphql::{Enum, ID, InputObject, Json, MaybeUndefined, SimpleObject};
use chrono::{DateTime, Utc};

use super::{ScriptScheduleInput, ScriptSchedulePayload};

// ── Enums ──────────────────────────────────────────────────────────────────

/// The one control of a lifecycle rule set.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleRuleState {
    /// Stored but never evaluated. A disabled rule opens no candidates and runs
    /// no actions, and it is the state every new rule set is created in.
    Disabled,
    /// Evaluate and record candidates, never act.
    DryRun,
    /// Evaluate and act on due candidates with the effects the rule's own
    /// action can have, while the instance does not pause actions.
    Enabled,
}

/// Granularity a lifecycle rule set is scoped to.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleRuleSubjectKind {
    /// The rule evaluates whole titles, both movies and shows.
    Title,
    /// The rule evaluates each season independently, including specials.
    Season,
    /// The rule evaluates each episode independently.
    Episode,
}

/// The class of effects a rule's action can have.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleEffectArming {
    /// Evaluate and track candidates, never act.
    None,
    /// Low- and medium-risk actions may execute.
    Reversible,
    /// High-risk actions may additionally execute. Requires acknowledging the
    /// rule's current candidate count when set.
    Destructive,
}

/// How a lifecycle candidate or action run records its actions.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleActionKind {
    /// The actions run as an ordered step sequence. Read `actionSequence` for
    /// the steps.
    ActionSequence,
}

/// Lifecycle state of one lifecycle candidate.
///
/// Two writers move a candidate through these states. The scheduled evaluator
/// opens and closes membership, writing only `OBSERVING`, `CANCELED`, and
/// `EXCLUDED`. The action handler owns the rest: it leases a due candidate into
/// `EXECUTING` and writes the outcome, and it only ever selects candidates of a
/// rule that is in `OBSERVE` mode, is armed, and whose matching instance effect
/// gate is on.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleCandidateState {
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

/// An action a lifecycle rule's source can define.
///
/// Each value is a Rego rule name: `unmonitor if { … }`,
/// `set_quality_profile := "Name" if { … }`, and so on.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleActionHead {
    /// `unmonitor`: stop monitoring the subject and everything under it.
    Unmonitor,
    /// `delete_files`: unmonitor the subject, then delete its files.
    DeleteFiles,
    /// `delete_title`: delete the title and its files. Title rules only.
    DeleteTitle,
    /// `search`: search for the title.
    Search,
    /// `set_quality_profile`: change the title's quality profile, by name.
    SetQualityProfile,
    /// `add_tags`: add tags to the title.
    AddTags,
    /// `remove_tags`: remove tags from the title.
    RemoveTags,
    /// `monitor`: monitor the subject.
    Monitor,
    /// `move_to_library`: move the title into another library, by name.
    MoveToLibrary,
    /// `move_to_root`: move the title to another root folder, by path.
    MoveToRoot,
    /// `notify`: send a notification naming the rule and the subject.
    Notify,
    /// `fetch_subtitles`: search for subtitles.
    FetchSubtitles,
    /// `monitor_season`: monitor one season.
    MonitorSeason,
    /// `unmonitor_season`: stop monitoring one season and its episodes.
    UnmonitorSeason,
    /// `delete_season_files`: unmonitor one season, then delete its files.
    DeleteSeasonFiles,
    /// `fetch_season_subtitles`: search for one season's subtitles.
    FetchSeasonSubtitles,
    /// `notify_season`: send a notification naming the rule and the season.
    NotifySeason,
    /// `monitor_episode`: monitor one episode.
    MonitorEpisode,
    /// `unmonitor_episode`: stop monitoring one episode.
    UnmonitorEpisode,
    /// `delete_episode_files`: unmonitor one episode, then delete its files.
    DeleteEpisodeFiles,
    /// `fetch_episode_subtitles`: search for one episode's subtitles.
    FetchEpisodeSubtitles,
    /// `notify_episode`: send a notification naming the rule and the episode.
    NotifyEpisode,
}

/// What a matcher decided for one subject.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleOutcome {
    /// The rule matched the subject.
    Match,
    /// The rule did not match the subject.
    NoMatch,
    /// A fact the rule needs could not be observed, so the rule is held.
    Unknown,
}

// ── Read models ────────────────────────────────────────────────────────────

/// A lifecycle rule set: identity, scope, and evaluation state. The source,
/// which declares both when the rule matches and what it does, lives on the
/// revision.
#[derive(SimpleObject, Clone)]
pub struct LifecycleRuleSet {
    /// Rule-set ID.
    pub id: ID,
    /// Rule-set name.
    pub name: String,
    /// Rule-set description; empty when the author supplied none.
    pub description: String,
    /// Disabled, dry run, or enabled.
    pub state: LifecycleRuleState,
    /// What enabling this rule arms. `DESTRUCTIVE` means enabling it requires
    /// the candidate-count confirmation.
    pub enabled_arming: LifecycleEffectArming,
    /// The rule can delete media and was enabled when its source or scope last
    /// changed, so it fell back to dry run. Review its candidates and enable it
    /// again.
    pub edited_since_enabled: bool,
    /// Shipped with Scryer: name and source are fixed and the rule
    /// cannot be deleted.
    pub system_owned: bool,
    /// Libraries the rule is confined to. Empty means every library.
    pub library_ids: Vec<String>,
    /// Revision number of the matcher currently in force.
    pub current_revision_number: i32,
    /// Grace period of the revision currently in force, in days.
    pub grace_days: i32,
    /// The actions the source of the revision currently in force declares.
    pub declared_actions: LifecycleDeclaredActions,
    /// Stable reason code when the revision in force cannot be evaluated or
    /// run, such as `LIFECYCLE_RULE_STORED_ACTIONS_NOT_DECLARED` for a rule
    /// saved before rules declared their own actions. The rule matches nothing
    /// and cannot be enabled until its source is saved again. An armed rule
    /// that deletes with no most titles per run shows
    /// `LIFECYCLE_RULE_ARMED_WITHOUT_MAX_TITLES_PER_RUN`: it still evaluates
    /// and previews, but its actions do not run until a maximum is set. Null
    /// when the rule is usable.
    pub load_refusal: Option<String>,
    /// When the rule evaluates and then acts on what it decided. A rule saved
    /// without one runs every eight hours.
    pub schedule: ScriptSchedulePayload,
    /// The most titles the rule acts on in one run. Due titles beyond it wait
    /// for the next run. Null means no limit.
    pub max_titles_per_run: Option<i32>,
    /// UTC time of the next scheduled run. Null while the rule is disabled,
    /// when its schedule is manual, or before the scheduler has loaded it.
    pub next_run_at: Option<DateTime<Utc>>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
    /// UTC last-update time.
    pub updated_at: DateTime<Utc>,
}

/// One immutable revision of a lifecycle rule set's matcher and action.
#[derive(SimpleObject, Clone)]
pub struct LifecycleRuleRevision {
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
    pub source_content_hash: String,
    /// ID of the user who wrote the revision, or null when unattributed.
    pub created_by: Option<ID>,
    /// UTC creation time.
    pub created_at: DateTime<Utc>,
}

/// Everything a rule's source declares it can do, read from the source
/// without running it. Every list holds each value once, in source order.
#[derive(SimpleObject, Clone)]
pub struct LifecycleDeclaredActions {
    /// Every action head the source defines.
    pub heads: Vec<LifecycleActionHead>,
    /// Whether any defined head deletes media. Enabling such a rule needs the
    /// candidate-count confirmation.
    pub destructive: bool,
    /// Every quality profile name written for `set_quality_profile`.
    pub quality_profiles: Vec<String>,
    /// Every tag written for `add_tags`.
    pub add_tags: Vec<String>,
    /// Every tag written for `remove_tags`.
    pub remove_tags: Vec<String>,
    /// Every library name written for `move_to_library`.
    pub libraries: Vec<String>,
    /// Every root folder path written for `move_to_root`.
    pub roots: Vec<String>,
    /// Every notification text written for `notify`.
    pub notifications: Vec<String>,
}

/// What a rule decided for one subject, in the order the actions run.
/// Deleting files unmonitors first, so `DELETE_FILES` comes with `UNMONITOR`.
#[derive(SimpleObject, Clone)]
pub struct LifecycleDecidedActions {
    /// The decided action heads, in execution order.
    pub heads: Vec<LifecycleActionHead>,
    /// Quality profile name to set, or null when the profile is not changed.
    pub quality_profile: Option<String>,
    /// Tags to add.
    pub add_tags: Vec<String>,
    /// Tags to remove.
    pub remove_tags: Vec<String>,
    /// Library name to move the title into, or null.
    pub library: Option<String>,
    /// Root folder path to move the title to, or null.
    pub root: Option<String>,
    /// Notification text to send, or null.
    pub notification: Option<String>,
}

/// One typed parameter set for a sequence step. The action catalog decides
/// which fields are meaningful for each `kind`; the application rejects every
/// other shape before the sequence can be stored.
#[derive(SimpleObject, Clone, Default)]
pub struct LifecycleActionStepParameters {
    /// Whether a title-level unmonitor step also unmonitors its seasons and
    /// episodes. This is required before a title file-deletion step can cover
    /// a show completely.
    pub include_descendants: Option<bool>,
    /// Target profile for a quality-profile step.
    pub target_quality_profile_id: Option<String>,
    /// Dispatch condition for a search step.
    pub search_condition: Option<LifecycleSearchCondition>,
    /// Tags written by an add-tags or remove-tags step.
    pub tags: Vec<String>,
    /// Library name a move-to-library step sends the title to.
    pub target_library: Option<String>,
    /// Root folder path a move-to-root step sends the title to.
    pub target_root: Option<String>,
}

/// The closed catalog of sequence step kinds. These kinds are intentionally
/// separate from the legacy lifecycle-action catalog: a sequence never
/// serializes as a synthetic legacy action.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleActionStepKind {
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
    /// Monitor the selected scope.
    Monitor,
    /// Search for subtitles for the selected scope's files.
    FetchSubtitles,
    /// Move the selected title into another library.
    MoveToLibrary,
    /// Move the selected title to another root folder of its library.
    MoveToRoot,
}

/// The explicit condition a search step applies before requesting work.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleSearchCondition {
    /// Always request the search when the step executes.
    Unconditional,
    /// Request the search only when the preceding profile step changed a profile.
    PreviousProfileChanged,
}

/// One ordered action in a revision-bound sequence.
#[derive(SimpleObject, Clone)]
pub struct LifecycleActionStep {
    /// Stable identifier used by durable execution receipts and history.
    pub id: ID,
    /// Catalog primitive this ordered step executes.
    pub kind: LifecycleActionStepKind,
    /// Typed parameters accepted by this step's catalog primitive.
    pub parameters: LifecycleActionStepParameters,
}

/// The authoritative, schema-versioned sequence a revision authorizes.
#[derive(SimpleObject, Clone)]
pub struct LifecycleActionSequence {
    /// Schema version that defines the accepted step representation.
    pub schema_version: i32,
    /// Ordered steps that make up the revision's action sequence.
    pub steps: Vec<LifecycleActionStep>,
}

/// A rule set together with the revision currently in force.
#[derive(SimpleObject, Clone)]
pub struct LifecycleRuleSetDetail {
    /// The rule set itself.
    pub rule_set: LifecycleRuleSet,
    /// The revision currently in force.
    pub revision: LifecycleRuleRevision,
    /// The actions that revision's source declares.
    pub declared_actions: LifecycleDeclaredActions,
    /// The simple builder's document for the revision's source, or null when
    /// the source was written by hand.
    pub builder_model: Option<Json<serde_json::Value>>,
}

/// Identifier returned after deleting a lifecycle rule set.
#[derive(SimpleObject, Clone)]
pub struct DeleteLifecycleRuleSetPayload {
    /// Deleted rule-set ID.
    pub id: ID,
}

/// Result of validating lifecycle rule source without saving it.
#[derive(SimpleObject, Clone)]
pub struct LifecycleRuleValidationPayload {
    /// Structured conditions and source locations.
    pub diagnostics: Vec<super::rules::RuleValidationDiagnosticPayload>,
    /// Whether the source is valid.
    pub valid: bool,
    /// Validation errors; empty when valid.
    pub errors: Vec<String>,
    /// Conditions worth fixing that do not stop the source from saving.
    pub warnings: Vec<super::rules::RuleValidationDiagnosticPayload>,
    /// The actions the source declares, or null when it does not parse.
    pub declared_actions: Option<LifecycleDeclaredActions>,
}

/// One subject's preview outcome. `outcome` is null exactly when `error` is set:
/// a rule that failed produced no decision, and a failure is never rendered as
/// a no-match.
#[derive(SimpleObject, Clone)]
pub struct LifecyclePreviewTitle {
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
    pub outcome: Option<LifecycleOutcome>,
    /// Reason codes the matcher emitted; empty when it emitted none.
    pub reason_codes: Vec<String>,
    /// Why evaluation failed for this subject, or null when it succeeded.
    pub error: Option<String>,
    /// Stable code for `error`, such as `RULE_RUNTIME_ERROR` or
    /// `RULE_OUTPUT_INVALID`; null exactly when `error` is null.
    pub error_code: Option<String>,
    /// What the rule decided for this subject, or null unless it matched.
    pub actions: Option<LifecycleDecidedActions>,
    /// The input document the rule evaluated for this subject: its `facts`
    /// and their `observations`, with the subject and library. Null unless
    /// the caller may manage permissions, because the document names who
    /// watched and who requested the subject.
    pub facts: Option<Json<serde_json::Value>>,
}

/// Outcome of running one matcher against a bounded subject selection. Preview
/// persists nothing.
#[derive(SimpleObject, Clone)]
pub struct LifecyclePreviewPayload {
    /// Stored rule set the preview ran, or the throwaway ID an unsaved draft
    /// compiled under.
    pub rule_set_id: String,
    /// Hash of the exact source that produced these outcomes.
    pub source_content_hash: String,
    /// UTC time the preview evaluated the selection.
    pub evaluated_at: DateTime<Utc>,
    /// One entry per evaluated subject, including its owning title.
    pub titles: Vec<LifecyclePreviewTitle>,
}

/// A conflicting action one rule decided for a title and a season or episode
/// of it, as seen from one side of the pair.
#[derive(SimpleObject, Clone)]
pub struct LifecycleActionsConflict {
    /// This candidate's head in the pair, such as `delete_season_files`.
    pub head: String,
    /// The other subject's scope: title, season, or episode.
    pub other_subject_kind: String,
    /// Scryer title, collection, or episode ID of the other subject.
    pub other_subject_id: ID,
    /// The other subject's head in the pair, such as `monitor`.
    pub other_head: String,
}

/// One subject's durable membership in one lifecycle rule set.
///
/// A candidate records that a rule matched the subject, how long its grace
/// period still has to run, and, once the handler has acted, what happened.
/// `state` is the authority on which of those a given row is: opening a
/// candidate never acts on anything, and only `EXECUTING`, `SUCCEEDED`, and
/// `FAILED` mean an action was attempted.
#[derive(SimpleObject, Clone)]
pub struct LifecycleCandidate {
    /// Number of current subject files, or null if the subject is unavailable.
    pub file_count: Option<i32>,
    /// Current subject file size in bytes, or null if the subject is unavailable.
    pub total_size_bytes: Option<i64>,
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
    pub state: LifecycleCandidateState,
    /// Why the candidate is in that state.
    pub state_reason: String,
    /// Reason codes the matcher emitted on the most recent match.
    pub reason_codes: Vec<String>,
    /// How the candidate records its actions.
    pub action_kind: LifecycleActionKind,
    /// What the rule decided for this subject on its latest match. Null only
    /// for candidates opened before rules declared their own actions.
    pub actions: Option<LifecycleDecidedActions>,
    /// The action the same rule decided for an enclosing or contained subject
    /// that conflicts with this candidate's, as the latest evaluation found
    /// it. While set, the candidate is held with `actions_conflict` and runs
    /// nothing. Null when there is none.
    pub actions_conflict: Option<LifecycleActionsConflict>,
    /// The immutable schema-2 sequence that opened this candidate. Legacy
    /// candidates retain their original `actionKind` and leave this null.
    pub action_sequence: Option<LifecycleActionSequence>,
    /// Ordered latest evidence for each schema-2 step. An empty sequence is an
    /// observe-only candidate rather than a missing action.
    pub sequence_steps: Vec<LifecycleActionStepProgress>,
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
pub struct LifecycleActionStepRun {
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
pub struct LifecycleActionJobReceipt {
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
pub struct LifecycleActionStepProgress {
    /// Immutable step definition from the sequence revision.
    pub step: LifecycleActionStep,
    /// Latest durable checkpoint for this step, or null before execution starts.
    pub run: Option<LifecycleActionStepRun>,
    /// Durable external-job receipts associated with this step.
    pub receipts: Vec<LifecycleActionJobReceipt>,
}

/// One rule set's pass through one evaluation run.
#[derive(SimpleObject, Clone)]
pub struct LifecycleEvaluationRun {
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

/// The two instance-wide lifecycle switches. An unconfigured instance has
/// lifecycle off and actions paused, and both are read at the start of every
/// scheduled pass so a change takes effect on the next run without a restart.
///
/// A switch is only ever the instance half of a permission. Executing an action
/// additionally requires the individual rule to be `ENABLED`, so unpausing
/// actions cannot by itself start anything.
#[derive(SimpleObject, Clone, Copy)]
pub struct LifecycleInstanceGates {
    /// Whether rules are evaluated and their candidates shown. While this is
    /// off no rule is evaluated and no candidate is opened or closed.
    pub enabled: bool,
    /// Whether every action is paused. While this is off, enabled rules act on
    /// their due candidates, including deleting files for rules that do so.
    pub actions_paused: bool,
}

/// A subject a lifecycle rule must never act on.
#[derive(SimpleObject, Clone)]
pub struct LifecycleExclusion {
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

/// Identifier returned after removing a lifecycle exclusion.
#[derive(SimpleObject, Clone)]
pub struct DeleteLifecycleExclusionPayload {
    /// Removed exclusion ID.
    pub id: ID,
}

/// One recorded action-handler attempt on one candidate, holds included.
#[derive(SimpleObject, Clone)]
pub struct LifecycleActionRun {
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
    /// How the attempt records its actions.
    pub action_kind: LifecycleActionKind,
    /// The immutable sequence this attempt executed.
    pub action_sequence: Option<LifecycleActionSequence>,
    /// Ordered latest evidence for every step in the immutable sequence.
    pub sequence_steps: Vec<LifecycleActionStepProgress>,
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
pub struct LifecycleEvaluationTriggerPayload {
    /// Whether a pass was started. False when a gate, a disabled rule, or a
    /// run already in progress stopped it.
    pub started: bool,
    /// What happened, suitable for showing to an operator.
    pub message: Option<String>,
}

// ── Inputs ─────────────────────────────────────────────────────────────────

/// Creates a lifecycle rule set together with its first matcher revision.
#[derive(InputObject)]
pub struct CreateLifecycleRuleSetInput {
    /// Rule-set name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Matcher source as written in the editor. The package declaration is
    /// applied by the server.
    pub rego_source: String,
    /// Days a subject must match continuously before the action becomes due;
    /// defaults to zero.
    pub grace_days: Option<i32>,
    /// Libraries to confine the rule to. Omitted or empty means every library.
    pub library_ids: Option<Vec<String>>,
    /// The simple builder's document for the source, or omitted for a rule
    /// written by hand.
    pub builder_model: MaybeUndefined<Json<serde_json::Value>>,
    /// When the rule runs. Omitted means every eight hours.
    pub schedule: Option<ScriptScheduleInput>,
    /// The most titles the rule may act on in one run, at least one.
    /// Required when the rule deletes files or titles; omitted means no
    /// limit otherwise.
    pub max_titles_per_run: Option<i32>,
}

/// Replaces the matcher of an existing rule set, appending a revision.
#[derive(InputObject)]
pub struct UpdateLifecycleRuleSourceInput {
    /// Rule-set ID.
    pub id: ID,
    /// Replacement matcher source as written in the editor.
    pub rego_source: String,
    /// Days a subject must match continuously before the action becomes due;
    /// defaults to zero.
    pub grace_days: Option<i32>,
    /// The simple builder's document. Omit to keep the current revision's
    /// document, pass null to clear it, or pass a document to replace it.
    pub builder_model: MaybeUndefined<Json<serde_json::Value>>,
    /// Replacement most titles the rule may act on in one run, checked with
    /// the new source so a delete head and its maximum can be saved
    /// together. Omitted keeps the current value, null clears it, and a
    /// value of at least one replaces it.
    pub max_titles_per_run: MaybeUndefined<i32>,
}

/// Renames and re-scopes a rule set without touching its matcher.
#[derive(InputObject)]
pub struct UpdateLifecycleRuleMetadataInput {
    /// Rule-set ID.
    pub id: ID,
    /// Replacement name.
    pub name: String,
    /// Replacement description; omitted clears it.
    pub description: Option<String>,
    /// Replacement library scope. Omitted or empty means every library.
    pub library_ids: Option<Vec<String>>,
    /// Replacement schedule. Omitted keeps the current one. Changing it
    /// leaves the rule's state and arming alone.
    pub schedule: Option<ScriptScheduleInput>,
    /// Replacement most titles the rule may act on in one run. Omitted keeps
    /// the current value, null clears it, and a value of at least one
    /// replaces it. A rule that deletes files or titles cannot be cleared.
    /// Changing it leaves the rule's state and arming alone.
    pub max_titles_per_run: MaybeUndefined<i32>,
}

/// What the operator reviewed before enabling a rule that can delete files.
#[derive(InputObject)]
pub struct LifecycleRuleArmingConfirmationInput {
    /// The reviewed matcher/action revision.
    pub revision_number: i32,
    /// The reviewed library scope. An empty list explicitly acknowledges all
    /// libraries.
    pub library_ids: Vec<ID>,
    /// The candidate count the operator saw; must equal the rule's current
    /// non-terminal count.
    pub candidate_count: i32,
}

/// Sets the instance-wide lifecycle switches. An omitted field leaves that
/// switch exactly as stored.
#[derive(InputObject)]
pub struct SetLifecycleInstanceGatesInput {
    /// Whether rules are evaluated and their candidates shown.
    pub enabled: Option<bool>,
    /// Whether every action is paused. Setting this to false lets every rule
    /// that is already `ENABLED` begin acting, including deleting media files,
    /// on the next handler pass.
    pub actions_paused: Option<bool>,
}

/// Excludes one subject from lifecycle rules.
#[derive(InputObject)]
pub struct ExcludeLifecycleSubjectInput {
    /// Required for episode or season scope.
    pub subject_id: Option<ID>,
    /// Defaults to title scope when omitted.
    pub subject_kind: Option<LifecycleRuleSubjectKind>,
    /// Title to exclude.
    pub title_id: ID,
    /// Rule to confine the exclusion to. Omitted means every rule.
    pub rule_set_id: Option<ID>,
    /// Why the subject is being excluded.
    pub reason: Option<String>,
}

/// Validates lifecycle rule source without saving it.
#[derive(InputObject)]
pub struct ValidateLifecycleRuleInput {
    /// Matcher source to validate, as written in the editor.
    pub rego_source: String,
}

/// Runs one matcher against a bounded title selection without saving anything.
///
/// Supply either `ruleSetId` to preview a stored rule set, or `regoSource` to
/// preview an unsaved draft, never both. A draft's source declares its own
/// actions. Select titles with either `titleIds` or
/// `libraryId`, never both.
#[derive(InputObject)]
pub struct PreviewLifecycleRuleInput {
    /// Selected libraries for an unsaved matcher; empty means all libraries.
    pub library_ids: Option<Vec<String>>,
    /// Stored rule set to preview at its current revision.
    pub rule_set_id: Option<ID>,
    /// Unsaved matcher source to preview, as written in the editor.
    pub rego_source: Option<String>,
    /// Grace period for the unsaved draft; defaults to zero.
    pub grace_days: Option<i32>,
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
