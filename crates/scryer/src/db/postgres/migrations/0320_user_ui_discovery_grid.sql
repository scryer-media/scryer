-- Discovery section grids remember whether to leave out titles already in a
-- library (on by default, as the home rails do) and titles with a pending
-- request (off by default).
ALTER TABLE user_ui_settings ADD COLUMN discovery_grid_hide_owned BOOLEAN NOT NULL DEFAULT TRUE;
ALTER TABLE user_ui_settings ADD COLUMN discovery_grid_hide_requested BOOLEAN NOT NULL DEFAULT FALSE;
