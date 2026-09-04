-- Proxies stop being an indexer-only concept.
--
-- Three things happen here, and they belong in one migration because they are
-- one change: the table stops being named after a single consumer, the rows
-- learn to describe families other than challenge solvers, and the second
-- consumer family (download clients) gets its assignment column.
--
-- * `indexer_proxy_configs` becomes `proxy_configs`, and
--   `indexers.indexer_proxy_config_id` becomes `indexers.proxy_config_id`, so
--   indexers and download clients name the same thing the same way.
-- * `protocol` names the challenge-solver wire contract. A transport hop or a
--   tunnel speaks no such protocol, so the column has to admit NULL. SQLite
--   cannot drop a NOT NULL in place, so this follows the table-rebuild pattern
--   established by migrations/0186_download_identity_states_token_optional.sql.
--   Existing solver rows carry their protocol across unchanged.
-- * Transport proxies frequently require credentials and tunnels always do.
--   Both are stored encrypted at rest under the same `*_encrypted` convention
--   as `indexers.api_key_encrypted`. `remote_dns` records the SOCKS
--   `socks5h`/`socks4a` choice (resolve the destination at the proxy) as a flag
--   rather than as a second provider type. `host_key_fingerprint` is
--   deliberately NOT encrypted: a host key is public, and the operator has to
--   be able to read the pinned value to compare it against their server.
CREATE TABLE proxy_configs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    protocol TEXT,
    base_url TEXT NOT NULL,
    request_timeout_seconds INTEGER NOT NULL DEFAULT 60,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    username_encrypted TEXT,
    password_encrypted TEXT,
    remote_dns INTEGER NOT NULL DEFAULT 0,
    private_key_encrypted TEXT,
    private_key_passphrase_encrypted TEXT,
    host_key_fingerprint TEXT,
    host_key_pinned_at TEXT,
    last_health_status TEXT,
    last_error_message TEXT,
    last_error_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO proxy_configs (
    id,
    name,
    provider_type,
    protocol,
    base_url,
    request_timeout_seconds,
    is_enabled,
    last_health_status,
    last_error_message,
    last_error_at,
    created_at,
    updated_at
)
SELECT
    id,
    name,
    provider_type,
    protocol,
    base_url,
    request_timeout_seconds,
    is_enabled,
    last_health_status,
    last_error_message,
    last_error_at,
    created_at,
    updated_at
FROM indexer_proxy_configs;

-- Dropping the table takes its provider_type index with it. The replacement is
-- created below under the new name.
DROP TABLE indexer_proxy_configs;

CREATE INDEX idx_proxy_configs_provider_type
    ON proxy_configs(provider_type);

DROP INDEX IF EXISTS idx_indexers_indexer_proxy_config_id;

ALTER TABLE indexers RENAME COLUMN indexer_proxy_config_id TO proxy_config_id;

CREATE INDEX idx_indexers_proxy_config_id
    ON indexers(proxy_config_id);

-- The second consumer family. A download client may be assigned any kind of
-- proxy, including a challenge solver, so this is a plain nullable reference
-- with no kind constraint.
ALTER TABLE download_clients ADD COLUMN proxy_config_id TEXT;
