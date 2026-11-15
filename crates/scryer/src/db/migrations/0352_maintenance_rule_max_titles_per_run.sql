-- A lifecycle rule states the most titles it may act on in one fire. It is
-- required for a rule that deletes files or titles and optional otherwise;
-- null means no limit. The system expired-leases rule acts on ten.
--
-- A fire walks a rule's due candidates in pages ordered by (due_at, id), so
-- the walk is served by its own index.

ALTER TABLE maintenance_rule_sets ADD COLUMN max_titles_per_run INTEGER;

UPDATE maintenance_rule_sets SET max_titles_per_run = 10 WHERE system_owned = 1;

CREATE INDEX IF NOT EXISTS idx_lifecycle_candidates_rule_due
    ON lifecycle_candidates(rule_set_id, due_at, id);
