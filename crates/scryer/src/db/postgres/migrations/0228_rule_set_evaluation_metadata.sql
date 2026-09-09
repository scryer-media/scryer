ALTER TABLE rule_sets ADD COLUMN evaluation_phase text NOT NULL DEFAULT 'additional';
ALTER TABLE rule_sets ADD COLUMN exclusive_group text;
ALTER TABLE rule_sets ADD COLUMN disabled_reason text;
