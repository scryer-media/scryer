CREATE TABLE IF NOT EXISTS oauth_client_registrations (
    client_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (enabled IN (0, 1))
);

CREATE INDEX IF NOT EXISTS idx_oauth_client_registrations_enabled
    ON oauth_client_registrations(enabled);

CREATE TABLE IF NOT EXISTS oauth_client_redirect_uris (
    client_id TEXT NOT NULL REFERENCES oauth_client_registrations(client_id) ON DELETE CASCADE,
    redirect_uri TEXT NOT NULL,
    PRIMARY KEY (client_id, redirect_uri)
);
