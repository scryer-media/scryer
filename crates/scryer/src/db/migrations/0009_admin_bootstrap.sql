INSERT OR IGNORE INTO entitlements (code, description, category)
VALUES
    ('view_catalog', 'Read access to title and media catalog', 'media'),
    ('monitor_title', 'Change monitor state', 'media'),
    ('manage_title', 'Create and edit catalog entities', 'media'),
    ('trigger_actions', 'Queue and re-queue actions', 'operations'),
    ('manage_config', 'Manage instance configuration', 'system'),
    ('view_history', 'Read activity and event history', 'operations');

INSERT INTO users (
    id,
    username,
    status,
    entitlements,
    password_hash,
    updated_at
)
SELECT
    hex(randomblob(16)),
    'admin',
    'active',
    '["view_catalog","monitor_title","manage_title","trigger_actions","manage_config","view_history"]',
    NULL,
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE NOT EXISTS (SELECT 1 FROM users);

UPDATE users
SET
    status = 'active',
    entitlements = '["view_catalog","monitor_title","manage_title","trigger_actions","manage_config","view_history"]',
    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE username = 'admin';

INSERT OR IGNORE INTO user_entitlements (
    user_id,
    entitlement_code,
    granted_by_user_id,
    granted_at,
    expires_at
)
SELECT
    u.id,
    e.code,
    NULL,
    strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
    NULL
FROM users u
CROSS JOIN entitlements e
WHERE u.username = 'admin';
