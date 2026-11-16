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

-- Constraints and indexes on the renamed tables take the new prefix. A
-- constraint's backing index is renamed with it. Names are looked up rather
-- than listed because older Postgres installs did not name NOT NULL
-- constraints.
DO $$
DECLARE
    item record;
BEGIN
    FOR item IN
        SELECT c.conname, t.relname
        FROM pg_constraint c
        JOIN pg_class t ON t.oid = c.conrelid
        JOIN pg_namespace n ON n.oid = t.relnamespace
        WHERE n.nspname = current_schema()
          AND t.relname IN (
              'lifecycle_rule_sets', 'lifecycle_rule_revisions',
              'lifecycle_rule_set_libraries', 'lifecycle_rule_exclusions',
              'lifecycle_evaluation_runs', 'lifecycle_action_steps',
              'lifecycle_action_step_attempts', 'lifecycle_action_job_receipts',
              'lifecycle_sequence_terminal_memberships')
          AND c.conname LIKE 'maintenance\_%'
    LOOP
        EXECUTE format(
            'ALTER TABLE %I RENAME CONSTRAINT %I TO %I',
            item.relname,
            item.conname,
            'lifecycle_' || substr(item.conname, length('maintenance_') + 1));
    END LOOP;

    FOR item IN
        SELECT i.relname
        FROM pg_index x
        JOIN pg_class i ON i.oid = x.indexrelid
        JOIN pg_class t ON t.oid = x.indrelid
        JOIN pg_namespace n ON n.oid = i.relnamespace
        WHERE n.nspname = current_schema()
          AND t.relname IN (
              'lifecycle_rule_sets', 'lifecycle_rule_revisions',
              'lifecycle_rule_set_libraries', 'lifecycle_rule_exclusions',
              'lifecycle_evaluation_runs', 'lifecycle_action_steps',
              'lifecycle_action_step_attempts', 'lifecycle_action_job_receipts',
              'lifecycle_sequence_terminal_memberships')
          AND i.relname LIKE 'idx\_maintenance\_%'
    LOOP
        EXECUTE format(
            'ALTER INDEX %I RENAME TO %I',
            item.relname,
            'idx_lifecycle_' || substr(item.relname, length('idx_maintenance_') + 1));
    END LOOP;
END
$$;

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
WHERE strpos(builder_model_json, '"family":"maintenance"') > 0;

-- Reason codes stored on rule history. A column that holds one code is
-- rewritten by prefix, after the one code that repeats the word; LIKE is case-sensitive, so lower-case reasons such as
-- `playback_hold` are never touched. Codes inside JSON are rewritten where
-- they appear as a quoted value.
UPDATE lifecycle_action_runs
SET hold_reason = CASE
        WHEN hold_reason LIKE 'CONFIGURATION\_MAINTENANCE\_%'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(hold_reason, 27)
        WHEN hold_reason LIKE 'LIBRARY\_MAINTENANCE\_DELETION\_%'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(hold_reason, 30)
        WHEN hold_reason LIKE 'ACQUISITION\_MAINTENANCE\_SEARCH\_%'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(hold_reason, 32)
        WHEN hold_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE'
            THEN 'LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE'
        WHEN hold_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN hold_reason LIKE 'MAINTENANCE\_%'
            THEN 'LIFECYCLE_' || substr(hold_reason, 13)
        ELSE hold_reason
    END
WHERE hold_reason LIKE '%MAINTENANCE\_%';

UPDATE lifecycle_action_steps
SET hold_reason = CASE
        WHEN hold_reason LIKE 'CONFIGURATION\_MAINTENANCE\_%'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(hold_reason, 27)
        WHEN hold_reason LIKE 'LIBRARY\_MAINTENANCE\_DELETION\_%'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(hold_reason, 30)
        WHEN hold_reason LIKE 'ACQUISITION\_MAINTENANCE\_SEARCH\_%'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(hold_reason, 32)
        WHEN hold_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE'
            THEN 'LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE'
        WHEN hold_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN hold_reason LIKE 'MAINTENANCE\_%'
            THEN 'LIFECYCLE_' || substr(hold_reason, 13)
        ELSE hold_reason
    END
WHERE hold_reason LIKE '%MAINTENANCE\_%';

UPDATE lifecycle_action_step_attempts
SET hold_reason = CASE
        WHEN hold_reason LIKE 'CONFIGURATION\_MAINTENANCE\_%'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(hold_reason, 27)
        WHEN hold_reason LIKE 'LIBRARY\_MAINTENANCE\_DELETION\_%'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(hold_reason, 30)
        WHEN hold_reason LIKE 'ACQUISITION\_MAINTENANCE\_SEARCH\_%'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(hold_reason, 32)
        WHEN hold_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE'
            THEN 'LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE'
        WHEN hold_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN hold_reason LIKE 'MAINTENANCE\_%'
            THEN 'LIFECYCLE_' || substr(hold_reason, 13)
        ELSE hold_reason
    END
WHERE hold_reason LIKE '%MAINTENANCE\_%';

UPDATE lifecycle_candidates
SET state_reason = CASE
        WHEN state_reason LIKE 'CONFIGURATION\_MAINTENANCE\_%'
            THEN 'CONFIGURATION_LIFECYCLE_' || substr(state_reason, 27)
        WHEN state_reason LIKE 'LIBRARY\_MAINTENANCE\_DELETION\_%'
            THEN 'LIBRARY_LIFECYCLE_DELETION_' || substr(state_reason, 30)
        WHEN state_reason LIKE 'ACQUISITION\_MAINTENANCE\_SEARCH\_%'
            THEN 'ACQUISITION_LIFECYCLE_SEARCH_' || substr(state_reason, 32)
        WHEN state_reason = 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE'
            THEN 'LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE'
        WHEN state_reason = 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND'
            THEN 'INTERFACE_LIFECYCLE_RULE_NOT_FOUND'
        WHEN state_reason LIKE 'MAINTENANCE\_%'
            THEN 'LIFECYCLE_' || substr(state_reason, 13)
        ELSE state_reason
    END
WHERE state_reason LIKE '%MAINTENANCE\_%';

UPDATE lifecycle_action_runs
SET detail = replace(replace(replace(replace(replace(replace(detail,
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE strpos(detail, 'MAINTENANCE_') > 0;

UPDATE lifecycle_action_steps
SET provenance_json = replace(replace(replace(replace(replace(replace(provenance_json,
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE strpos(provenance_json, 'MAINTENANCE_') > 0;

UPDATE lifecycle_action_step_attempts
SET provenance_json = replace(replace(replace(replace(replace(replace(provenance_json,
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE strpos(provenance_json, 'MAINTENANCE_') > 0;

UPDATE lifecycle_action_job_receipts
SET reconciliation_evidence_json = replace(replace(replace(replace(replace(replace(reconciliation_evidence_json,
        '"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE"', '"LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE"'),
        '"CONFIGURATION_MAINTENANCE_', '"CONFIGURATION_LIFECYCLE_'),
        '"LIBRARY_MAINTENANCE_DELETION_', '"LIBRARY_LIFECYCLE_DELETION_'),
        '"ACQUISITION_MAINTENANCE_SEARCH_', '"ACQUISITION_LIFECYCLE_SEARCH_'),
        '"INTERFACE_MAINTENANCE_RULE_NOT_FOUND"', '"INTERFACE_LIFECYCLE_RULE_NOT_FOUND"'),
        '"MAINTENANCE_', '"LIFECYCLE_')
WHERE strpos(reconciliation_evidence_json, 'MAINTENANCE_') > 0;
