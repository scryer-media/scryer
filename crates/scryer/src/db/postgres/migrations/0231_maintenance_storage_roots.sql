-- A storage-scoped maintenance revision names the configured root whose
-- capacity it reads and whose owned files it may delete. Legacy revisions
-- remain root-agnostic. The rule-set flag records the one-time safety review
-- needed when title-level show facts become executable.
ALTER TABLE maintenance_rule_revisions
    ADD COLUMN storage_root_id text;

ALTER TABLE maintenance_rule_sets
    ADD COLUMN destructive_rearm_required boolean NOT NULL DEFAULT false;
