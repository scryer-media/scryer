-- Maintenance rules are now lifecycle rules. Their tables, indexes, settings
-- keys, job key, builder family and stored reason codes take the new name.
-- Rows are kept as they are; only names change.
--
-- Not renamed: domain event payloads (compressed, and plugins subscribe to
-- the `maintenance_rule_notified` tag), Rego package and policy paths inside
-- stored rule sources, and lock or journal keys inside stored JSON.

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

DROP INDEX IF EXISTS idx_maintenance_action_step_attempts_started;
CREATE INDEX idx_lifecycle_action_step_attempts_started
    ON lifecycle_action_step_attempts(started_at);

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

-- Builder documents name their rule family. They are stored canonically, so
-- the family is spelled exactly this way.
UPDATE lifecycle_rule_revisions
SET builder_model_json = replace(builder_model_json, '"family":"maintenance"', '"family":"lifecycle"')
WHERE instr(builder_model_json, '"family":"maintenance"') > 0;

-- Reason codes stored on rule history. A column that holds one code is
-- rewritten by prefix, after the one code that repeats the word; GLOB is case-sensitive, so lower-case reasons such as
-- `playback_hold` are never touched. Codes inside JSON are rewritten where
-- they appear as a quoted value.
UPDATE lifecycle_action_runs
SET hold_reason = CASE
        WHEN hold_reason GLOB 'CONFIGURATION_MAINTENANCE_*'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(hold_reason, 27)
        WHEN hold_reason GLOB 'LIBRARY_MAINTENANCE_DELETION_*'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(hold_reason, 30)
        WHEN hold_reason GLOB 'ACQUISITION_MAINTENANCE_SEARCH_*'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(hold_reason, 32)
        WHEN hold_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE'
            THEN 'LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE'
        WHEN hold_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN hold_reason GLOB 'MAINTENANCE_*'
            THEN 'LIFECYCLE_' || substr(hold_reason, 13)
        ELSE hold_reason
    END
WHERE hold_reason GLOB '*MAINTENANCE_*';

UPDATE lifecycle_action_steps
SET hold_reason = CASE
        WHEN hold_reason GLOB 'CONFIGURATION_MAINTENANCE_*'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(hold_reason, 27)
        WHEN hold_reason GLOB 'LIBRARY_MAINTENANCE_DELETION_*'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(hold_reason, 30)
        WHEN hold_reason GLOB 'ACQUISITION_MAINTENANCE_SEARCH_*'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(hold_reason, 32)
        WHEN hold_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE'
            THEN 'LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE'
        WHEN hold_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN hold_reason GLOB 'MAINTENANCE_*'
            THEN 'LIFECYCLE_' || substr(hold_reason, 13)
        ELSE hold_reason
    END
WHERE hold_reason GLOB '*MAINTENANCE_*';

UPDATE lifecycle_action_step_attempts
SET hold_reason = CASE
        WHEN hold_reason GLOB 'CONFIGURATION_MAINTENANCE_*'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(hold_reason, 27)
        WHEN hold_reason GLOB 'LIBRARY_MAINTENANCE_DELETION_*'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(hold_reason, 30)
        WHEN hold_reason GLOB 'ACQUISITION_MAINTENANCE_SEARCH_*'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(hold_reason, 32)
        WHEN hold_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE'
            THEN 'LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE'
        WHEN hold_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN hold_reason GLOB 'MAINTENANCE_*'
            THEN 'LIFECYCLE_' || substr(hold_reason, 13)
        ELSE hold_reason
    END
WHERE hold_reason GLOB '*MAINTENANCE_*';

UPDATE lifecycle_candidates
SET state_reason = CASE
        WHEN state_reason GLOB 'CONFIGURATION_MAINTENANCE_*'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(state_reason, 27)
        WHEN state_reason GLOB 'LIBRARY_MAINTENANCE_DELETION_*'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(state_reason, 30)
        WHEN state_reason GLOB 'ACQUISITION_MAINTENANCE_SEARCH_*'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(state_reason, 32)
        WHEN state_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE'
            THEN 'LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE'
        WHEN state_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN state_reason GLOB 'MAINTENANCE_*'
            THEN 'LIFECYCLE_' || substr(state_reason, 13)
        ELSE state_reason
    END
WHERE state_reason GLOB '*MAINTENANCE_*';

UPDATE lifecycle_action_runs
SET detail = replace(replace(replace(replace(replace(replace(detail,
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE instr(detail, 'MAINTENANCE_') > 0;

UPDATE lifecycle_action_steps
SET provenance_json = replace(replace(replace(replace(replace(replace(provenance_json,
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE instr(provenance_json, 'MAINTENANCE_') > 0;

UPDATE lifecycle_action_step_attempts
SET provenance_json = replace(replace(replace(replace(replace(replace(provenance_json,
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE instr(provenance_json, 'MAINTENANCE_') > 0;

UPDATE lifecycle_action_job_receipts
SET reconciliation_evidence_json = replace(replace(replace(replace(replace(replace(reconciliation_evidence_json,
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE instr(reconciliation_evidence_json, 'MAINTENANCE_') > 0;
