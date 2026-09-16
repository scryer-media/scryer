-- Per-download-client failure status with an escalating backoff.
--
-- Indexers have had this since 0139 (`indexer_system_backoffs`); download
-- clients had nothing, so an unreachable client was rediscovered by probing it
-- again on the next grab instead of being routed around. The shape is Sonarr's
-- `DownloadClientStatus`: when the current failure run started, when it was
-- last seen, which rung of the escalation ladder the client is on, and until
-- when it is disabled. A successful listing deletes the row, so an absent row
-- means healthy and the table only ever holds currently-failing clients.
CREATE TABLE IF NOT EXISTS download_client_status (
    client_config_id TEXT PRIMARY KEY NOT NULL,
    initial_failure_at TEXT,
    most_recent_failure_at TEXT,
    escalation_level INTEGER NOT NULL DEFAULT 0,
    disabled_until TEXT,
    FOREIGN KEY(client_config_id) REFERENCES download_clients(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_download_client_status_disabled_until
    ON download_client_status(disabled_until);
