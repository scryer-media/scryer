CREATE TABLE IF NOT EXISTS oauth_client_registrations (
    client_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_oauth_client_registrations_enabled
    ON oauth_client_registrations(enabled);

CREATE TABLE IF NOT EXISTS oauth_client_redirect_uris (
    client_id TEXT NOT NULL REFERENCES oauth_client_registrations(client_id) ON DELETE CASCADE,
    redirect_uri TEXT NOT NULL,
    PRIMARY KEY (client_id, redirect_uri)
);
