-- The interface glow becomes a per-user preference that is off by default.
-- Instances already in use keep the glow they had: every stored preference
-- row keeps it through the column default, and each user without a row gets
-- one that keeps it. A fresh install has finished no setup and holds no
-- titles, so its users start without the glow.
ALTER TABLE user_ui_settings ADD COLUMN no_glow BOOLEAN NOT NULL DEFAULT FALSE;

INSERT INTO user_ui_settings (user_id)
SELECT u.id
FROM users u
WHERE NOT EXISTS (SELECT 1 FROM user_ui_settings s WHERE s.user_id = u.id)
  AND (
      EXISTS (
          SELECT 1
          FROM settings_values v
          JOIN settings_definitions d ON d.id = v.setting_definition_id
          WHERE d.key_name = 'setup.complete'
            AND v.scope_id IS NULL
            AND lower(trim(both '" ' from v.value_json::text)) IN ('true', '1', 'yes', 'on')
      )
      OR EXISTS (SELECT 1 FROM titles)
  );
