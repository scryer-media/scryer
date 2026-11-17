-- A lifecycle rule stores one state, disabled, dry_run or enabled, in place of
-- the enabled flag, evaluation mode, effect arming and re-arm flag. Whether
-- the rule evaluates, acts or may run a step that deletes is read from the
-- state and from the revision the operator confirmed deletion for.
--
-- Each existing row keeps exactly what it may do. A rule that was not
-- observing is disabled. An observing rule armed for effects and not flagged
-- for review is enabled; every other observing rule is a dry run, which is how
-- the rule read before. A rule armed for deletion carries its current
-- revision as the confirmed one, and only while it is enabled. The review flag
-- carries over unchanged.

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
