ALTER TABLE rule_sets ADD COLUMN is_managed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE rule_sets ADD COLUMN managed_key TEXT;
CREATE UNIQUE INDEX idx_rule_sets_managed_key ON rule_sets(managed_key) WHERE managed_key IS NOT NULL;
