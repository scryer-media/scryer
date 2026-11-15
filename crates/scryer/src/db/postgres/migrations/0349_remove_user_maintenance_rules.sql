-- Rules now declare their own actions in their source. A rule written before
-- that cannot be read as one, so every user-owned maintenance rule is removed
-- together with its history. The system-owned rule stays; startup brings its
-- source up to date.
--
-- Every table deleted from here is reached from maintenance_rule_sets through
-- ON DELETE CASCADE. The rows are deleted child first so the result is the
-- same whether or not foreign-key enforcement is on for this connection. Only
-- database rows are removed: no file on disk is touched.

DELETE FROM maintenance_action_step_attempts
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN maintenance_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.system_owned = false
);

DELETE FROM maintenance_action_job_receipts
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN maintenance_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.system_owned = false
);

DELETE FROM maintenance_action_steps
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN maintenance_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.system_owned = false
);

DELETE FROM maintenance_sequence_terminal_memberships
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN maintenance_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.system_owned = false
);

DELETE FROM lifecycle_action_runs
WHERE candidate_id IN (
    SELECT candidate.id
    FROM lifecycle_candidates AS candidate
    JOIN maintenance_rule_sets AS rule_set ON rule_set.id = candidate.rule_set_id
    WHERE rule_set.system_owned = false
);

DELETE FROM lifecycle_candidates
WHERE rule_set_id IN (SELECT id FROM maintenance_rule_sets WHERE system_owned = false);

DELETE FROM maintenance_evaluation_runs
WHERE rule_set_id IN (SELECT id FROM maintenance_rule_sets WHERE system_owned = false);

DELETE FROM maintenance_rule_exclusions
WHERE rule_set_id IN (SELECT id FROM maintenance_rule_sets WHERE system_owned = false);

DELETE FROM maintenance_rule_set_libraries
WHERE rule_set_id IN (SELECT id FROM maintenance_rule_sets WHERE system_owned = false);

DELETE FROM maintenance_rule_revisions
WHERE rule_set_id IN (SELECT id FROM maintenance_rule_sets WHERE system_owned = false);

DELETE FROM maintenance_rule_sets WHERE system_owned = false;
