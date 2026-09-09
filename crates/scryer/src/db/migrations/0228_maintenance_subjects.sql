-- Preserve candidate clocks, generations, execution evidence, and rule arming.
ALTER TABLE lifecycle_candidates ADD COLUMN subject_id TEXT NOT NULL DEFAULT '';
UPDATE lifecycle_candidates SET subject_id = title_id;
DROP INDEX idx_lifecycle_candidates_active_subject;
CREATE UNIQUE INDEX idx_lifecycle_candidates_active_subject
    ON lifecycle_candidates(rule_set_id, subject_kind, subject_id)
    WHERE state NOT IN ('succeeded', 'failed', 'canceled', 'excluded');
CREATE INDEX idx_lifecycle_candidates_subject_generation
    ON lifecycle_candidates(rule_set_id, subject_kind, subject_id, match_generation);

ALTER TABLE maintenance_rule_exclusions ADD COLUMN subject_kind TEXT NOT NULL DEFAULT 'title';
ALTER TABLE maintenance_rule_exclusions ADD COLUMN subject_id TEXT NOT NULL DEFAULT '';
UPDATE maintenance_rule_exclusions SET subject_id = title_id;
DROP INDEX idx_maintenance_rule_exclusions_rule_title;
DROP INDEX idx_maintenance_rule_exclusions_global_title;
CREATE UNIQUE INDEX idx_maintenance_rule_exclusions_rule_subject
    ON maintenance_rule_exclusions(rule_set_id, subject_kind, subject_id)
    WHERE rule_set_id IS NOT NULL;
CREATE UNIQUE INDEX idx_maintenance_rule_exclusions_global_subject
    ON maintenance_rule_exclusions(subject_kind, subject_id)
    WHERE rule_set_id IS NULL;

ALTER TABLE lifecycle_action_runs ADD COLUMN subject_kind TEXT NOT NULL DEFAULT 'title';
ALTER TABLE lifecycle_action_runs ADD COLUMN subject_id TEXT NOT NULL DEFAULT '';
UPDATE lifecycle_action_runs SET subject_id = title_id;
