CREATE TABLE IF NOT EXISTS settings_values_new(
    id TEXT PRIMARY KEY,
    setting_definition_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    value_json TEXT NOT NULL,
    source TEXT NOT NULL,
    updated_by_user_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (setting_definition_id) REFERENCES settings_definitions(id) ON DELETE CASCADE
);

INSERT INTO settings_values_new
    (id, setting_definition_id, scope, scope_id, value_json, source, updated_by_user_id, created_at, updated_at)
SELECT id, setting_definition_id, scope, scope_id, value_json, source, updated_by_user_id, created_at, updated_at
FROM settings_values;

DROP TABLE settings_values;

ALTER TABLE settings_values_new RENAME TO settings_values;

CREATE UNIQUE INDEX IF NOT EXISTS idx_setting_values_scope_name
    ON settings_values(setting_definition_id, scope, COALESCE(scope_id, ''));

CREATE INDEX IF NOT EXISTS idx_settings_values_definition
    ON settings_values(setting_definition_id);
