-- Facets a user hid from navigation, discovery and pickers, as a
-- comma-separated list of facet ids ("movies", "series", "anime"). Empty shows
-- every facet.
ALTER TABLE user_ui_settings ADD COLUMN hidden_facets TEXT NOT NULL DEFAULT '';
