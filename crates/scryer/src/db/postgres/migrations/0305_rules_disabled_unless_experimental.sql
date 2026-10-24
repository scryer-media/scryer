-- Maintenance rules and request rules are experimental again. 0.21.19 showed
-- both panes to every instance and shipped its built-in rule enabled, so an
-- instance that never opted into experimental features may hold rules that
-- were enabled without that opt-in. Unless experimental features are on, every
-- maintenance rule goes back to Disabled, with its arming cleared the way the
-- Disabled control does, and every request rule is disabled. Candidates,
-- exclusions and recorded decisions are kept.
UPDATE maintenance_rule_sets
SET enabled = false,
    evaluation_mode = 'disabled',
    effect_arming = 'none',
    destructive_rearm_required = false
WHERE NOT EXISTS (
      SELECT 1
      FROM settings_values v
      JOIN settings_definitions d ON d.id = v.setting_definition_id
      WHERE d.key_name = 'ui.experimental_features_enabled'
        AND v.scope_id IS NULL
        AND lower(trim(both '" ' from v.value_json::text)) IN ('true', '1', 'yes', 'on')
  );

UPDATE request_rule_sets
SET enabled = false,
    evaluation_mode = 'disabled'
WHERE NOT EXISTS (
      SELECT 1
      FROM settings_values v
      JOIN settings_definitions d ON d.id = v.setting_definition_id
      WHERE d.key_name = 'ui.experimental_features_enabled'
        AND v.scope_id IS NULL
        AND lower(trim(both '" ' from v.value_json::text)) IN ('true', '1', 'yes', 'on')
  );
