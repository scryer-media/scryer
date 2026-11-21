-- A dashboard panel may carry its own settings, a JSON object the web owns.
-- Panels saved before this column existed, and panels without settings, hold
-- NULL.
ALTER TABLE user_ui_dashboard_layout_items ADD COLUMN settings_json TEXT;
