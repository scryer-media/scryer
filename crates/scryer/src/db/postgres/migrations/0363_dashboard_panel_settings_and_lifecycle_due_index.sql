-- A dashboard panel may carry its own settings, a JSON object the web owns.
-- Panels saved before this column existed, and panels without settings, hold
-- NULL.
ALTER TABLE user_ui_dashboard_layout_items ADD COLUMN settings_json TEXT;

-- The upcoming lifecycle actions view lists live candidates of every rule by
-- state, soonest due first. The rule-scoped due index leads with the rule set,
-- so it cannot serve that read across rules; this index serves the state filter
-- and the due order together.
CREATE INDEX IF NOT EXISTS idx_lifecycle_candidates_state_due
    ON lifecycle_candidates (state, due_at, id);
