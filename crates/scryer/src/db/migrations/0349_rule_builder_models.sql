-- The simple rule builder keeps the document it writes a rule from beside
-- the rule, so the builder can reopen it. Nullable and never backfilled: a
-- rule without one opens as Rego, exactly as before.
--
-- A scoring rule has one row, so the document sits on it. A maintenance
-- rule's source lives on immutable revisions, so the document sits on the
-- revision it wrote.

ALTER TABLE rule_sets ADD COLUMN builder_model_json TEXT;

ALTER TABLE maintenance_rule_revisions ADD COLUMN builder_model_json TEXT;
