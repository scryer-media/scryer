-- A lifecycle rule runs on its own schedule: each fire evaluates that rule
-- and then acts on what it decided. The schedule is the same JSON shape a
-- scheduled script stores. Nullable and never backfilled: a rule without one
-- runs every eight hours, the cadence the shared evaluation pass had.

ALTER TABLE maintenance_rule_sets ADD COLUMN schedule_json TEXT;
