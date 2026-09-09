ALTER TABLE rule_sets ADD COLUMN evaluation_phase TEXT NOT NULL DEFAULT 'additional';
ALTER TABLE rule_sets ADD COLUMN exclusive_group TEXT;
ALTER TABLE rule_sets ADD COLUMN disabled_reason TEXT;
