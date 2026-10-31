INSERT INTO settings_definitions
    (id, category, scope, key_name, data_type, default_value_json, is_sensitive, created_at, updated_at)
VALUES ('initial-authentication-setup', 'system', 'system', 'auth.initial_setup', 'json', '{"stage":"choice"}', FALSE, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO settings_values
    (id, setting_definition_id, scope, value_json, source, created_at, updated_at)
VALUES ('initial-authentication-setup', 'initial-authentication-setup', 'system', '{"stage":"choice"}', 'initial-authentication', '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
