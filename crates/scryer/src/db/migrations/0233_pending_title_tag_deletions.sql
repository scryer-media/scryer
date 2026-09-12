-- The settings value is included in logical backups, so interrupted deletes
-- retain the label and acting user across restart and restore.
INSERT INTO settings_definitions
    (id, category, scope, key_name, data_type, default_value_json, is_sensitive, created_at, updated_at)
VALUES
    ('title-tags-pending-deletions', 'system', 'system', 'title_tags.pending_deletions', 'json', '{}', 0,
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
ON CONFLICT(category, scope, key_name) DO NOTHING;
