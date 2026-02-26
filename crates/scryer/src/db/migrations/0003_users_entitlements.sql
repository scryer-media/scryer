CREATE TABLE IF NOT EXISTS users(
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    entitlements TEXT NOT NULL,
    password_hash TEXT,
    passkey_public_key TEXT,
    locale TEXT,
    created_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    last_login_at TEXT
);

CREATE TABLE IF NOT EXISTS entitlements(
    code TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    category TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS user_entitlements(
    user_id TEXT NOT NULL,
    entitlement_code TEXT NOT NULL,
    granted_by_user_id TEXT,
    granted_at TEXT NOT NULL,
    expires_at TEXT,
    PRIMARY KEY (user_id, entitlement_code),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (entitlement_code) REFERENCES entitlements(code) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS integration_tokens(
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    token_name TEXT,
    token_hash TEXT NOT NULL UNIQUE,
    scopes_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    created_by_user_id TEXT,
    expires_at TEXT,
    revoked_at TEXT,
    last_used_at TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_user_entitlements_user
    ON user_entitlements (user_id);

CREATE INDEX IF NOT EXISTS idx_integration_tokens_user
    ON integration_tokens (user_id);
