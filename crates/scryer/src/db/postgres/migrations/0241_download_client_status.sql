-- Per-download-client failure status with an escalating backoff. See the
-- SQLite twin for what the columns mean.
CREATE TABLE IF NOT EXISTS download_client_status (
    client_config_id TEXT PRIMARY KEY NOT NULL
        REFERENCES download_clients(id) ON DELETE CASCADE,
    initial_failure_at TIMESTAMPTZ,
    most_recent_failure_at TIMESTAMPTZ,
    escalation_level INTEGER NOT NULL DEFAULT 0,
    disabled_until TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_download_client_status_disabled_until
    ON download_client_status(disabled_until);
