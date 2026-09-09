-- The settings value is included in logical backups, so interrupted renames
-- retain their original labels across restart and restore.
INSERT INTO settings_definitions
    (id, category, scope, key_name, data_type, default_value_json, is_sensitive, created_at, updated_at)
VALUES
    ('title-tags-pending-renames', 'system', 'system', 'title_tags.pending_renames', 'json', '{}'::jsonb, false,
     CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
ON CONFLICT(category, scope, key_name) DO NOTHING;
