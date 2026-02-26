CREATE TABLE IF NOT EXISTS settings_definitions(
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    scope TEXT NOT NULL,
    key_name TEXT NOT NULL,
    data_type TEXT NOT NULL,
    default_value_json TEXT,
    is_sensitive INTEGER NOT NULL DEFAULT 0,
    validation_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(category, scope, key_name)
);

CREATE TABLE IF NOT EXISTS settings_values(
    id TEXT PRIMARY KEY,
    setting_definition_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    value_json TEXT NOT NULL,
    source TEXT NOT NULL,
    effective_from TEXT,
    effective_to TEXT,
    updated_by_user_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (setting_definition_id) REFERENCES settings_definitions(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_setting_values_scope_name
    ON settings_values(setting_definition_id, scope, COALESCE(scope_id, ''));

CREATE INDEX IF NOT EXISTS idx_settings_values_definition
    ON settings_values(setting_definition_id);
