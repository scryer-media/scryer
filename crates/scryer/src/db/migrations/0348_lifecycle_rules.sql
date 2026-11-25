-- Maintenance rules become lifecycle rules: rules that declare their own
-- actions in their source, run on their own schedule and store one state.
--
-- A rule written before rules declared their actions cannot be read as one,
-- so every user-owned maintenance rule is removed together with its history.
-- The system-owned rule stays; startup brings its source up to date. Only
-- database rows are removed: no file on disk is touched.
--
-- Everything else keeps its rows and takes the new names: the rule tables
-- and their indexes, the instance gate settings, the evaluation job's run
-- history, stored reason codes, and the Rego package a stored rule source
-- declares. Not renamed: domain event payloads (compressed, and plugins
-- subscribe to their tags), lock or journal keys inside stored JSON, and the
-- `maintenance_sequence_history` key.

-- Every table deleted from here is reached from maintenance_rule_sets through
-- ON DELETE CASCADE. The rows are deleted child first so the result is the
-- same whether or not foreign-key enforcement is on for this connection.
DELETE FROM maintenance_action_step_attempts
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN maintenance_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.system_owned = 0
);

DELETE FROM maintenance_action_job_receipts
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN maintenance_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.system_owned = 0
);

DELETE FROM maintenance_action_steps
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN maintenance_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.system_owned = 0
);

DELETE FROM maintenance_sequence_terminal_memberships
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN maintenance_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.system_owned = 0
);

DELETE FROM lifecycle_action_runs
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN maintenance_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.system_owned = 0
);

DELETE FROM lifecycle_candidates
WHERE rule_set_id IN (SELECT id FROM maintenance_rule_sets WHERE system_owned = 0);

DELETE FROM maintenance_evaluation_runs
WHERE rule_set_id IN (SELECT id FROM maintenance_rule_sets WHERE system_owned = 0);

DELETE FROM maintenance_rule_exclusions
WHERE rule_set_id IN (SELECT id FROM maintenance_rule_sets WHERE system_owned = 0);

DELETE FROM maintenance_rule_set_libraries
WHERE rule_set_id IN (SELECT id FROM maintenance_rule_sets WHERE system_owned = 0);

DELETE FROM maintenance_rule_revisions
WHERE rule_set_id IN (SELECT id FROM maintenance_rule_sets WHERE system_owned = 0);

DELETE FROM maintenance_rule_sets WHERE system_owned = 0;

ALTER TABLE maintenance_rule_sets RENAME TO lifecycle_rule_sets;
ALTER TABLE maintenance_rule_revisions RENAME TO lifecycle_rule_revisions;
ALTER TABLE maintenance_rule_set_libraries RENAME TO lifecycle_rule_set_libraries;
ALTER TABLE maintenance_rule_exclusions RENAME TO lifecycle_rule_exclusions;
ALTER TABLE maintenance_evaluation_runs RENAME TO lifecycle_evaluation_runs;
ALTER TABLE maintenance_action_steps RENAME TO lifecycle_action_steps;
ALTER TABLE maintenance_action_step_attempts RENAME TO lifecycle_action_step_attempts;
ALTER TABLE maintenance_action_job_receipts RENAME TO lifecycle_action_job_receipts;
ALTER TABLE maintenance_sequence_terminal_memberships
    RENAME TO lifecycle_sequence_terminal_memberships;

-- SQLite cannot rename an index, so each is recreated under its new name with
-- the same definition.
DROP INDEX IF EXISTS idx_maintenance_rule_set_libraries_library;
CREATE INDEX idx_lifecycle_rule_set_libraries_library
    ON lifecycle_rule_set_libraries(library_id);

DROP INDEX IF EXISTS idx_maintenance_rule_revisions_rule_set;
CREATE INDEX idx_lifecycle_rule_revisions_rule_set
    ON lifecycle_rule_revisions(rule_set_id, revision_number DESC);

DROP INDEX IF EXISTS idx_maintenance_rule_exclusions_title;
CREATE INDEX idx_lifecycle_rule_exclusions_title
    ON lifecycle_rule_exclusions(title_id);

DROP INDEX IF EXISTS idx_maintenance_rule_exclusions_rule_subject;
CREATE UNIQUE INDEX idx_lifecycle_rule_exclusions_rule_subject
    ON lifecycle_rule_exclusions(rule_set_id, subject_kind, subject_id)
    WHERE rule_set_id IS NOT NULL;

DROP INDEX IF EXISTS idx_maintenance_rule_exclusions_global_subject;
CREATE UNIQUE INDEX idx_lifecycle_rule_exclusions_global_subject
    ON lifecycle_rule_exclusions(subject_kind, subject_id)
    WHERE rule_set_id IS NULL;

DROP INDEX IF EXISTS idx_maintenance_evaluation_runs_rule_set;
CREATE INDEX idx_lifecycle_evaluation_runs_rule_set
    ON lifecycle_evaluation_runs(rule_set_id, started_at DESC);

DROP INDEX IF EXISTS idx_maintenance_evaluation_runs_started;
CREATE INDEX idx_lifecycle_evaluation_runs_started
    ON lifecycle_evaluation_runs(started_at DESC, id);

DROP INDEX IF EXISTS idx_maintenance_action_step_attempts_step;
CREATE INDEX idx_lifecycle_action_step_attempts_step
    ON lifecycle_action_step_attempts(candidate_id, match_generation, revision_number, step_id, attempt, started_at DESC);

DROP INDEX IF EXISTS idx_maintenance_action_job_receipts_step;
CREATE INDEX idx_lifecycle_action_job_receipts_step
    ON lifecycle_action_job_receipts(candidate_id, match_generation, revision_number, step_id, dispatch_attempt DESC);

DROP INDEX IF EXISTS idx_maintenance_action_job_receipts_accepted_request;
CREATE UNIQUE INDEX idx_lifecycle_action_job_receipts_accepted_request
    ON lifecycle_action_job_receipts(candidate_id, match_generation, revision_number, step_id, request_hash)
    WHERE state IN ('accepted', 'completed');

DROP INDEX IF EXISTS idx_maintenance_sequence_terminal_memberships_active;
CREATE UNIQUE INDEX idx_lifecycle_sequence_terminal_memberships_active
    ON lifecycle_sequence_terminal_memberships(rule_set_id, revision_number, subject_kind, subject_id)
    WHERE released_at IS NULL;

DROP INDEX IF EXISTS idx_maintenance_sequence_terminal_memberships_active_page;
CREATE INDEX idx_lifecycle_sequence_terminal_memberships_active_page
    ON lifecycle_sequence_terminal_memberships(rule_set_id, revision_number, id)
    WHERE released_at IS NULL;

-- The monitoring-conflict check reads which rules recently changed which
-- titles from the step attempts started inside a recent window, so the read
-- goes by start time.
CREATE INDEX idx_lifecycle_action_step_attempts_started
    ON lifecycle_action_step_attempts(started_at);

-- A fire walks a rule's due candidates in pages ordered by (due_at, id).
CREATE INDEX idx_lifecycle_candidates_rule_due
    ON lifecycle_candidates(rule_set_id, due_at, id);

-- No producer writes an operator keep claim any more.
DROP INDEX IF EXISTS idx_lifecycle_claims_live_operator_keep;

-- What a rule decided for one candidate, as a JSON document. NULL on
-- candidates written before rules declared their actions.
ALTER TABLE lifecycle_candidates ADD COLUMN decided_actions_json TEXT;

-- What the rule decided, in the same evaluation, for a subject enclosing or
-- inside this candidate's subject that cannot be applied together with this
-- candidate's own actions. While it is set the candidate is held with
-- actions_conflict. NULL when there is none.
ALTER TABLE lifecycle_candidates ADD COLUMN actions_conflict_json TEXT;

-- The rule builder's document for the revision it wrote. NULL when the
-- revision was not written with the builder; such a rule opens as Rego.
ALTER TABLE lifecycle_rule_revisions ADD COLUMN builder_model_json TEXT;

-- The content hash is taken over the whole rule source, and the actions are
-- the ones the source declares.
ALTER TABLE lifecycle_rule_revisions RENAME COLUMN matcher_content_hash TO source_content_hash;
ALTER TABLE lifecycle_rule_revisions RENAME COLUMN action_spec TO declared_actions_json;
ALTER TABLE lifecycle_candidates RENAME COLUMN matcher_content_hash TO source_content_hash;
ALTER TABLE lifecycle_evaluation_runs RENAME COLUMN matcher_content_hash TO source_content_hash;
ALTER TABLE lifecycle_sequence_terminal_memberships
    RENAME COLUMN matcher_content_hash TO source_content_hash;

-- A rule acts at the levels its action heads name, on every configured root,
-- so neither a rule-set subject kind nor a revision storage root is stored.
ALTER TABLE lifecycle_rule_revisions DROP COLUMN storage_root_id;
ALTER TABLE lifecycle_rule_sets DROP COLUMN subject_kind;

-- Each rule fires on its own schedule, the same JSON shape a job schedule
-- uses. A rule without one runs every eight hours.
ALTER TABLE lifecycle_rule_sets ADD COLUMN schedule_json TEXT;

-- The most titles a rule may act on in one fire. Required for a rule that
-- deletes files or titles and optional otherwise; NULL means no limit. The
-- system expired-leases rule acts on ten.
ALTER TABLE lifecycle_rule_sets ADD COLUMN max_titles_per_run INTEGER;
UPDATE lifecycle_rule_sets SET max_titles_per_run = 10 WHERE system_owned = 1;

-- A rule stores one state, disabled, dry_run or enabled, in place of the
-- enabled flag, evaluation mode, effect arming and re-arm flag. Each row keeps
-- exactly what it may do. A rule that was not observing is disabled. An
-- observing rule armed for effects and not flagged for review is enabled;
-- every other observing rule is a dry run, which is how the rule read before.
-- A rule armed for deletion carries its current revision as the confirmed
-- one, and only while it is enabled. The review flag carries over unchanged.
ALTER TABLE lifecycle_rule_sets ADD COLUMN state TEXT NOT NULL DEFAULT 'disabled';
ALTER TABLE lifecycle_rule_sets ADD COLUMN edited_since_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE lifecycle_rule_sets ADD COLUMN destructive_confirmed_revision INTEGER;

UPDATE lifecycle_rule_sets
SET state = CASE
        WHEN enabled = 0 OR evaluation_mode <> 'observe' THEN 'disabled'
        WHEN effect_arming IN ('reversible', 'destructive')
             AND destructive_rearm_required = 0 THEN 'enabled'
        ELSE 'dry_run'
    END,
    edited_since_enabled = destructive_rearm_required,
    destructive_confirmed_revision = CASE
        WHEN enabled = 1
             AND evaluation_mode = 'observe'
             AND effect_arming = 'destructive'
             AND destructive_rearm_required = 0 THEN current_revision_number
        ELSE NULL
    END;

ALTER TABLE lifecycle_rule_sets DROP COLUMN enabled;
ALTER TABLE lifecycle_rule_sets DROP COLUMN evaluation_mode;
ALTER TABLE lifecycle_rule_sets DROP COLUMN effect_arming;
ALTER TABLE lifecycle_rule_sets DROP COLUMN destructive_rearm_required;

-- A stored source declares the lifecycle package, which is what the engine
-- loads it under and what startup compares the shipped source against.
UPDATE lifecycle_rule_revisions
SET rego_source = replace(replace(rego_source,
        'scryer.maintenance.user', 'scryer.lifecycle.user'),
        'scryer.maintenance.wrapper', 'scryer.lifecycle.wrapper')
WHERE instr(rego_source, 'scryer.maintenance.') > 0;

-- The instance gates keep their definition rows, and so their stored values.
UPDATE settings_definitions
SET key_name = 'lifecycle.gate.' || substr(key_name, length('maintenance.gate.') + 1)
WHERE key_name IN (
    'maintenance.gate.evaluation',
    'maintenance.gate.result_display',
    'maintenance.gate.reversible_effects',
    'maintenance.gate.destructive_effects'
)
AND NOT EXISTS (
    SELECT 1 FROM settings_definitions renamed
    WHERE renamed.category = settings_definitions.category
      AND renamed.scope = settings_definitions.scope
      AND renamed.key_name =
          'lifecycle.gate.' || substr(settings_definitions.key_name, length('maintenance.gate.') + 1)
);

-- Job history for the evaluation job. A job run's operation type is its key.
UPDATE workflow_operations
SET job_key = 'lifecycle_rule_evaluation'
WHERE job_key = 'maintenance_rule_evaluation';
UPDATE workflow_operations
SET operation_type = 'lifecycle_rule_evaluation'
WHERE operation_type = 'maintenance_rule_evaluation';

-- The action handling job is gone: each rule acts right after its own
-- evaluation. Its runs recorded no work, and a run whose job key this build
-- does not know cannot be read back into the job history, so they are
-- removed.
DELETE FROM workflow_operations WHERE job_key = 'lifecycle_action_handling';

-- Reason codes stored on rule history take their current names. A code
-- renamed outright is matched whole first; every other code takes the new
-- prefix. GLOB is case-sensitive, so lower-case reasons such as
-- `playback_hold` are never touched. Codes inside JSON are rewritten where
-- they appear as a quoted value.
UPDATE lifecycle_action_runs
SET hold_reason = CASE
        WHEN hold_reason = 'MAINTENANCE_RULE_FAILED_COMPILE' THEN 'LIFECYCLE_RULE_COMPILE_FAILED'
        WHEN hold_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE' THEN 'LIFECYCLE_FILE_COVERAGE_AMBIGUOUS'
        WHEN hold_reason = 'MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON' THEN 'LIFECYCLE_CHILD_DELETION_SUBJECT_REQUIRED'
        WHEN hold_reason = 'MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE' THEN 'LIFECYCLE_CANDIDATE_TITLE_MISMATCH'
        WHEN hold_reason = 'MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED' THEN 'LIFECYCLE_FILE_TARGETS_CHANGED'
        WHEN hold_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER' THEN 'LIFECYCLE_SYSTEM_RULE_READ_ONLY'
        WHEN hold_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED' THEN 'LIFECYCLE_SYSTEM_RULE_RENAME_REFUSED'
        WHEN hold_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED' THEN 'LIFECYCLE_SYSTEM_RULE_DELETE_REFUSED'
        WHEN hold_reason = 'MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID' THEN 'LIFECYCLE_RULE_DELETE_HAS_ACTION_HISTORY'
        WHEN hold_reason = 'MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE' THEN 'LIFECYCLE_RULE_DELETE_HAS_ACTIVE_CANDIDATES'
        WHEN hold_reason = 'MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS' THEN 'LIFECYCLE_PREVIEW_SUBJECT_COUNT_OUT_OF_RANGE'
        WHEN hold_reason = 'MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE' THEN 'LIFECYCLE_PREVIEW_SUBJECT_OUTSIDE_LIBRARIES'
        WHEN hold_reason = 'MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED' THEN 'LIFECYCLE_PREVIEW_TITLE_LIMIT_EXCEEDED'
        WHEN hold_reason = 'MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY' THEN 'LIFECYCLE_MOVIE_HAS_NO_EPISODES'
        WHEN hold_reason = 'MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND' THEN 'LIFECYCLE_SUBJECT_NOT_FOUND'
        WHEN hold_reason = 'MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN' THEN 'LIFECYCLE_RULE_HAS_NO_EFFECT'
        WHEN hold_reason = 'MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP' THEN 'LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISSING'
        WHEN hold_reason = 'MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE' THEN 'LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISMATCH'
        WHEN hold_reason = 'MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST' THEN 'LIFECYCLE_QUALITY_PROFILE_NOT_FOUND'
        WHEN hold_reason GLOB 'CONFIGURATION_MAINTENANCE_*'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(hold_reason, 27)
        WHEN hold_reason GLOB 'LIBRARY_MAINTENANCE_DELETION_*'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(hold_reason, 30)
        WHEN hold_reason GLOB 'ACQUISITION_MAINTENANCE_SEARCH_*'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(hold_reason, 32)
        WHEN hold_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN hold_reason GLOB 'MAINTENANCE_*'
            THEN 'LIFECYCLE_' || substr(hold_reason, 13)
        ELSE hold_reason
    END
WHERE hold_reason GLOB '*MAINTENANCE_*';

UPDATE lifecycle_action_steps
SET hold_reason = CASE
        WHEN hold_reason = 'MAINTENANCE_RULE_FAILED_COMPILE' THEN 'LIFECYCLE_RULE_COMPILE_FAILED'
        WHEN hold_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE' THEN 'LIFECYCLE_FILE_COVERAGE_AMBIGUOUS'
        WHEN hold_reason = 'MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON' THEN 'LIFECYCLE_CHILD_DELETION_SUBJECT_REQUIRED'
        WHEN hold_reason = 'MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE' THEN 'LIFECYCLE_CANDIDATE_TITLE_MISMATCH'
        WHEN hold_reason = 'MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED' THEN 'LIFECYCLE_FILE_TARGETS_CHANGED'
        WHEN hold_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER' THEN 'LIFECYCLE_SYSTEM_RULE_READ_ONLY'
        WHEN hold_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED' THEN 'LIFECYCLE_SYSTEM_RULE_RENAME_REFUSED'
        WHEN hold_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED' THEN 'LIFECYCLE_SYSTEM_RULE_DELETE_REFUSED'
        WHEN hold_reason = 'MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID' THEN 'LIFECYCLE_RULE_DELETE_HAS_ACTION_HISTORY'
        WHEN hold_reason = 'MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE' THEN 'LIFECYCLE_RULE_DELETE_HAS_ACTIVE_CANDIDATES'
        WHEN hold_reason = 'MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS' THEN 'LIFECYCLE_PREVIEW_SUBJECT_COUNT_OUT_OF_RANGE'
        WHEN hold_reason = 'MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE' THEN 'LIFECYCLE_PREVIEW_SUBJECT_OUTSIDE_LIBRARIES'
        WHEN hold_reason = 'MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED' THEN 'LIFECYCLE_PREVIEW_TITLE_LIMIT_EXCEEDED'
        WHEN hold_reason = 'MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY' THEN 'LIFECYCLE_MOVIE_HAS_NO_EPISODES'
        WHEN hold_reason = 'MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND' THEN 'LIFECYCLE_SUBJECT_NOT_FOUND'
        WHEN hold_reason = 'MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN' THEN 'LIFECYCLE_RULE_HAS_NO_EFFECT'
        WHEN hold_reason = 'MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP' THEN 'LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISSING'
        WHEN hold_reason = 'MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE' THEN 'LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISMATCH'
        WHEN hold_reason = 'MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST' THEN 'LIFECYCLE_QUALITY_PROFILE_NOT_FOUND'
        WHEN hold_reason GLOB 'CONFIGURATION_MAINTENANCE_*'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(hold_reason, 27)
        WHEN hold_reason GLOB 'LIBRARY_MAINTENANCE_DELETION_*'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(hold_reason, 30)
        WHEN hold_reason GLOB 'ACQUISITION_MAINTENANCE_SEARCH_*'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(hold_reason, 32)
        WHEN hold_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN hold_reason GLOB 'MAINTENANCE_*'
            THEN 'LIFECYCLE_' || substr(hold_reason, 13)
        ELSE hold_reason
    END
WHERE hold_reason GLOB '*MAINTENANCE_*';

UPDATE lifecycle_action_step_attempts
SET hold_reason = CASE
        WHEN hold_reason = 'MAINTENANCE_RULE_FAILED_COMPILE' THEN 'LIFECYCLE_RULE_COMPILE_FAILED'
        WHEN hold_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE' THEN 'LIFECYCLE_FILE_COVERAGE_AMBIGUOUS'
        WHEN hold_reason = 'MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON' THEN 'LIFECYCLE_CHILD_DELETION_SUBJECT_REQUIRED'
        WHEN hold_reason = 'MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE' THEN 'LIFECYCLE_CANDIDATE_TITLE_MISMATCH'
        WHEN hold_reason = 'MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED' THEN 'LIFECYCLE_FILE_TARGETS_CHANGED'
        WHEN hold_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER' THEN 'LIFECYCLE_SYSTEM_RULE_READ_ONLY'
        WHEN hold_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED' THEN 'LIFECYCLE_SYSTEM_RULE_RENAME_REFUSED'
        WHEN hold_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED' THEN 'LIFECYCLE_SYSTEM_RULE_DELETE_REFUSED'
        WHEN hold_reason = 'MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID' THEN 'LIFECYCLE_RULE_DELETE_HAS_ACTION_HISTORY'
        WHEN hold_reason = 'MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE' THEN 'LIFECYCLE_RULE_DELETE_HAS_ACTIVE_CANDIDATES'
        WHEN hold_reason = 'MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS' THEN 'LIFECYCLE_PREVIEW_SUBJECT_COUNT_OUT_OF_RANGE'
        WHEN hold_reason = 'MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE' THEN 'LIFECYCLE_PREVIEW_SUBJECT_OUTSIDE_LIBRARIES'
        WHEN hold_reason = 'MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED' THEN 'LIFECYCLE_PREVIEW_TITLE_LIMIT_EXCEEDED'
        WHEN hold_reason = 'MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY' THEN 'LIFECYCLE_MOVIE_HAS_NO_EPISODES'
        WHEN hold_reason = 'MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND' THEN 'LIFECYCLE_SUBJECT_NOT_FOUND'
        WHEN hold_reason = 'MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN' THEN 'LIFECYCLE_RULE_HAS_NO_EFFECT'
        WHEN hold_reason = 'MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP' THEN 'LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISSING'
        WHEN hold_reason = 'MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE' THEN 'LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISMATCH'
        WHEN hold_reason = 'MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST' THEN 'LIFECYCLE_QUALITY_PROFILE_NOT_FOUND'
        WHEN hold_reason GLOB 'CONFIGURATION_MAINTENANCE_*'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(hold_reason, 27)
        WHEN hold_reason GLOB 'LIBRARY_MAINTENANCE_DELETION_*'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(hold_reason, 30)
        WHEN hold_reason GLOB 'ACQUISITION_MAINTENANCE_SEARCH_*'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(hold_reason, 32)
        WHEN hold_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN hold_reason GLOB 'MAINTENANCE_*'
            THEN 'LIFECYCLE_' || substr(hold_reason, 13)
        ELSE hold_reason
    END
WHERE hold_reason GLOB '*MAINTENANCE_*';

UPDATE lifecycle_candidates
SET state_reason = CASE
        WHEN state_reason = 'MAINTENANCE_RULE_FAILED_COMPILE' THEN 'LIFECYCLE_RULE_COMPILE_FAILED'
        WHEN state_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE' THEN 'LIFECYCLE_FILE_COVERAGE_AMBIGUOUS'
        WHEN state_reason = 'MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON' THEN 'LIFECYCLE_CHILD_DELETION_SUBJECT_REQUIRED'
        WHEN state_reason = 'MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE' THEN 'LIFECYCLE_CANDIDATE_TITLE_MISMATCH'
        WHEN state_reason = 'MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED' THEN 'LIFECYCLE_FILE_TARGETS_CHANGED'
        WHEN state_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER' THEN 'LIFECYCLE_SYSTEM_RULE_READ_ONLY'
        WHEN state_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED' THEN 'LIFECYCLE_SYSTEM_RULE_RENAME_REFUSED'
        WHEN state_reason = 'MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED' THEN 'LIFECYCLE_SYSTEM_RULE_DELETE_REFUSED'
        WHEN state_reason = 'MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID' THEN 'LIFECYCLE_RULE_DELETE_HAS_ACTION_HISTORY'
        WHEN state_reason = 'MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE' THEN 'LIFECYCLE_RULE_DELETE_HAS_ACTIVE_CANDIDATES'
        WHEN state_reason = 'MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS' THEN 'LIFECYCLE_PREVIEW_SUBJECT_COUNT_OUT_OF_RANGE'
        WHEN state_reason = 'MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE' THEN 'LIFECYCLE_PREVIEW_SUBJECT_OUTSIDE_LIBRARIES'
        WHEN state_reason = 'MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED' THEN 'LIFECYCLE_PREVIEW_TITLE_LIMIT_EXCEEDED'
        WHEN state_reason = 'MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY' THEN 'LIFECYCLE_MOVIE_HAS_NO_EPISODES'
        WHEN state_reason = 'MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND' THEN 'LIFECYCLE_SUBJECT_NOT_FOUND'
        WHEN state_reason = 'MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN' THEN 'LIFECYCLE_RULE_HAS_NO_EFFECT'
        WHEN state_reason = 'MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP' THEN 'LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISSING'
        WHEN state_reason = 'MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE' THEN 'LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISMATCH'
        WHEN state_reason = 'MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST' THEN 'LIFECYCLE_QUALITY_PROFILE_NOT_FOUND'
        WHEN state_reason GLOB 'CONFIGURATION_MAINTENANCE_*'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(state_reason, 27)
        WHEN state_reason GLOB 'LIBRARY_MAINTENANCE_DELETION_*'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(state_reason, 30)
        WHEN state_reason GLOB 'ACQUISITION_MAINTENANCE_SEARCH_*'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(state_reason, 32)
        WHEN state_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN state_reason GLOB 'MAINTENANCE_*'
            THEN 'LIFECYCLE_' || substr(state_reason, 13)
        ELSE state_reason
    END
WHERE state_reason GLOB '*MAINTENANCE_*';

UPDATE lifecycle_action_runs
SET detail = replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(detail,
        '"MAINTENANCE_RULE_FAILED_COMPILE"', '"LIFECYCLE_RULE_COMPILE_FAILED"'),
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_FILE_COVERAGE_AMBIGUOUS"'),
        '"MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON"', '"LIFECYCLE_CHILD_DELETION_SUBJECT_REQUIRED"'),
        '"MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE"', '"LIFECYCLE_CANDIDATE_TITLE_MISMATCH"'),
        '"MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED"', '"LIFECYCLE_FILE_TARGETS_CHANGED"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER"', '"LIFECYCLE_SYSTEM_RULE_READ_ONLY"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED"', '"LIFECYCLE_SYSTEM_RULE_RENAME_REFUSED"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED"', '"LIFECYCLE_SYSTEM_RULE_DELETE_REFUSED"'),
        '"MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID"', '"LIFECYCLE_RULE_DELETE_HAS_ACTION_HISTORY"'),
        '"MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE"', '"LIFECYCLE_RULE_DELETE_HAS_ACTIVE_CANDIDATES"'),
        '"MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS"', '"LIFECYCLE_PREVIEW_SUBJECT_COUNT_OUT_OF_RANGE"'),
        '"MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE"', '"LIFECYCLE_PREVIEW_SUBJECT_OUTSIDE_LIBRARIES"'),
        '"MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED"', '"LIFECYCLE_PREVIEW_TITLE_LIMIT_EXCEEDED"'),
        '"MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY"', '"LIFECYCLE_MOVIE_HAS_NO_EPISODES"'),
        '"MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND"', '"LIFECYCLE_SUBJECT_NOT_FOUND"'),
        '"MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN"', '"LIFECYCLE_RULE_HAS_NO_EFFECT"'),
        '"MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP"', '"LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISSING"'),
        '"MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE"', '"LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISMATCH"'),
        '"MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST"', '"LIFECYCLE_QUALITY_PROFILE_NOT_FOUND"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE instr(detail, 'MAINTENANCE_') > 0;

UPDATE lifecycle_action_steps
SET provenance_json = replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(provenance_json,
        '"MAINTENANCE_RULE_FAILED_COMPILE"', '"LIFECYCLE_RULE_COMPILE_FAILED"'),
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_FILE_COVERAGE_AMBIGUOUS"'),
        '"MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON"', '"LIFECYCLE_CHILD_DELETION_SUBJECT_REQUIRED"'),
        '"MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE"', '"LIFECYCLE_CANDIDATE_TITLE_MISMATCH"'),
        '"MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED"', '"LIFECYCLE_FILE_TARGETS_CHANGED"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER"', '"LIFECYCLE_SYSTEM_RULE_READ_ONLY"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED"', '"LIFECYCLE_SYSTEM_RULE_RENAME_REFUSED"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED"', '"LIFECYCLE_SYSTEM_RULE_DELETE_REFUSED"'),
        '"MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID"', '"LIFECYCLE_RULE_DELETE_HAS_ACTION_HISTORY"'),
        '"MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE"', '"LIFECYCLE_RULE_DELETE_HAS_ACTIVE_CANDIDATES"'),
        '"MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS"', '"LIFECYCLE_PREVIEW_SUBJECT_COUNT_OUT_OF_RANGE"'),
        '"MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE"', '"LIFECYCLE_PREVIEW_SUBJECT_OUTSIDE_LIBRARIES"'),
        '"MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED"', '"LIFECYCLE_PREVIEW_TITLE_LIMIT_EXCEEDED"'),
        '"MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY"', '"LIFECYCLE_MOVIE_HAS_NO_EPISODES"'),
        '"MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND"', '"LIFECYCLE_SUBJECT_NOT_FOUND"'),
        '"MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN"', '"LIFECYCLE_RULE_HAS_NO_EFFECT"'),
        '"MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP"', '"LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISSING"'),
        '"MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE"', '"LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISMATCH"'),
        '"MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST"', '"LIFECYCLE_QUALITY_PROFILE_NOT_FOUND"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE instr(provenance_json, 'MAINTENANCE_') > 0;

UPDATE lifecycle_action_step_attempts
SET provenance_json = replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(provenance_json,
        '"MAINTENANCE_RULE_FAILED_COMPILE"', '"LIFECYCLE_RULE_COMPILE_FAILED"'),
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_FILE_COVERAGE_AMBIGUOUS"'),
        '"MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON"', '"LIFECYCLE_CHILD_DELETION_SUBJECT_REQUIRED"'),
        '"MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE"', '"LIFECYCLE_CANDIDATE_TITLE_MISMATCH"'),
        '"MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED"', '"LIFECYCLE_FILE_TARGETS_CHANGED"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER"', '"LIFECYCLE_SYSTEM_RULE_READ_ONLY"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED"', '"LIFECYCLE_SYSTEM_RULE_RENAME_REFUSED"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED"', '"LIFECYCLE_SYSTEM_RULE_DELETE_REFUSED"'),
        '"MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID"', '"LIFECYCLE_RULE_DELETE_HAS_ACTION_HISTORY"'),
        '"MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE"', '"LIFECYCLE_RULE_DELETE_HAS_ACTIVE_CANDIDATES"'),
        '"MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS"', '"LIFECYCLE_PREVIEW_SUBJECT_COUNT_OUT_OF_RANGE"'),
        '"MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE"', '"LIFECYCLE_PREVIEW_SUBJECT_OUTSIDE_LIBRARIES"'),
        '"MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED"', '"LIFECYCLE_PREVIEW_TITLE_LIMIT_EXCEEDED"'),
        '"MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY"', '"LIFECYCLE_MOVIE_HAS_NO_EPISODES"'),
        '"MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND"', '"LIFECYCLE_SUBJECT_NOT_FOUND"'),
        '"MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN"', '"LIFECYCLE_RULE_HAS_NO_EFFECT"'),
        '"MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP"', '"LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISSING"'),
        '"MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE"', '"LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISMATCH"'),
        '"MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST"', '"LIFECYCLE_QUALITY_PROFILE_NOT_FOUND"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE instr(provenance_json, 'MAINTENANCE_') > 0;

UPDATE lifecycle_action_job_receipts
SET reconciliation_evidence_json = replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(replace(reconciliation_evidence_json,
        '"MAINTENANCE_RULE_FAILED_COMPILE"', '"LIFECYCLE_RULE_COMPILE_FAILED"'),
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_FILE_COVERAGE_AMBIGUOUS"'),
        '"MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON"', '"LIFECYCLE_CHILD_DELETION_SUBJECT_REQUIRED"'),
        '"MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE"', '"LIFECYCLE_CANDIDATE_TITLE_MISMATCH"'),
        '"MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED"', '"LIFECYCLE_FILE_TARGETS_CHANGED"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER"', '"LIFECYCLE_SYSTEM_RULE_READ_ONLY"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED"', '"LIFECYCLE_SYSTEM_RULE_RENAME_REFUSED"'),
        '"MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED"', '"LIFECYCLE_SYSTEM_RULE_DELETE_REFUSED"'),
        '"MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID"', '"LIFECYCLE_RULE_DELETE_HAS_ACTION_HISTORY"'),
        '"MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE"', '"LIFECYCLE_RULE_DELETE_HAS_ACTIVE_CANDIDATES"'),
        '"MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS"', '"LIFECYCLE_PREVIEW_SUBJECT_COUNT_OUT_OF_RANGE"'),
        '"MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE"', '"LIFECYCLE_PREVIEW_SUBJECT_OUTSIDE_LIBRARIES"'),
        '"MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED"', '"LIFECYCLE_PREVIEW_TITLE_LIMIT_EXCEEDED"'),
        '"MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY"', '"LIFECYCLE_MOVIE_HAS_NO_EPISODES"'),
        '"MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND"', '"LIFECYCLE_SUBJECT_NOT_FOUND"'),
        '"MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN"', '"LIFECYCLE_RULE_HAS_NO_EFFECT"'),
        '"MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP"', '"LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISSING"'),
        '"MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE"', '"LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISMATCH"'),
        '"MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST"', '"LIFECYCLE_QUALITY_PROFILE_NOT_FOUND"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE instr(reconciliation_evidence_json, 'MAINTENANCE_') > 0;
