-- A lifecycle rule's action heads now name the level they act at
-- (`delete_season_files`, `unmonitor_episode`), and storage facts cover every
-- configured root a subject's files are on. A rule saved with a season or
-- episode scope, or confined to one storage root, would act at a different
-- level or on more files under the new reading, so it is removed together
-- with its candidates, checkpoints and history. Such rules exist only on
-- development builds: the upgrade to rules that declare their own actions
-- already removed every rule written before them. The system-owned rule
-- acts on whole titles with no root and stays.
--
-- The rule-set `subject_kind` and revision `storage_root_id` columns are left
-- in place and no longer read; new rows take their defaults.
--
-- Every table deleted from here is reached from lifecycle_rule_sets through
-- ON DELETE CASCADE. The rows are deleted child first so the result is the
-- same whether or not foreign-key enforcement is on for this connection. Only
-- database rows are removed: no file on disk is touched.

-- Mark the retired rules first, so every delete below names the same set
-- after the revisions that identify a root-confined rule are gone.
UPDATE lifecycle_rule_sets
SET subject_kind = 'retired'
WHERE system_owned = 0
  AND (subject_kind <> 'title'
       OR id IN (
           SELECT rule_set_id FROM lifecycle_rule_revisions
           WHERE storage_root_id IS NOT NULL));

DELETE FROM lifecycle_action_step_attempts
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN lifecycle_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.subject_kind = 'retired'
);

DELETE FROM lifecycle_action_job_receipts
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN lifecycle_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.subject_kind = 'retired'
);

DELETE FROM lifecycle_action_steps
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN lifecycle_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.subject_kind = 'retired'
);

DELETE FROM lifecycle_sequence_terminal_memberships
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN lifecycle_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.subject_kind = 'retired'
);

DELETE FROM lifecycle_action_runs
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN lifecycle_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.subject_kind = 'retired'
);

DELETE FROM lifecycle_candidates
WHERE rule_set_id IN (SELECT id FROM lifecycle_rule_sets WHERE subject_kind = 'retired');

DELETE FROM lifecycle_evaluation_runs
WHERE rule_set_id IN (SELECT id FROM lifecycle_rule_sets WHERE subject_kind = 'retired');

DELETE FROM lifecycle_rule_exclusions
WHERE rule_set_id IN (SELECT id FROM lifecycle_rule_sets WHERE subject_kind = 'retired');

DELETE FROM lifecycle_rule_set_libraries
WHERE rule_set_id IN (SELECT id FROM lifecycle_rule_sets WHERE subject_kind = 'retired');

DELETE FROM lifecycle_rule_revisions
WHERE rule_set_id IN (SELECT id FROM lifecycle_rule_sets WHERE subject_kind = 'retired');

DELETE FROM lifecycle_rule_sets WHERE subject_kind = 'retired';
