-- Discovery section grids remember whether to leave out titles already in a
-- library (on by default, as the home rails do) and titles with a pending
-- request (off by default).
ALTER TABLE user_ui_settings ADD COLUMN discovery_grid_hide_owned INTEGER NOT NULL DEFAULT 1;
ALTER TABLE user_ui_settings ADD COLUMN discovery_grid_hide_requested INTEGER NOT NULL DEFAULT 0;
