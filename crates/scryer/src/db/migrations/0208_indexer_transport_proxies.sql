-- Indexer proxies stop being solver-only. `provider_type` now also names
-- plain transport hops (`http`, `socks5`) that Scryer's own HTTP client dials
-- through instead of asking a service to fetch the page for it.
--
-- Two schema consequences:
--
-- * `protocol` names the challenge-solver wire contract. A transport proxy
--   speaks no such protocol, so the column has to admit NULL. SQLite cannot
--   drop a NOT NULL in place, so this follows the table-rebuild pattern
--   established by migrations/0186_download_identity_states_token_optional.sql.
--   Existing solver rows carry their protocol across unchanged.
-- * Transport proxies frequently require credentials. They are stored
--   encrypted at rest under the same `*_encrypted` convention as
--   `indexers.api_key_encrypted`, and `remote_dns` records the SOCKS5
--   `socks5h` choice (resolve the destination at the proxy) as a flag rather
--   than as a second provider type.
CREATE TABLE indexer_proxy_configs_0208 (
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
    last_health_status TEXT,
    last_error_message TEXT,
    last_error_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO indexer_proxy_configs_0208 (
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

DROP TABLE indexer_proxy_configs;
ALTER TABLE indexer_proxy_configs_0208 RENAME TO indexer_proxy_configs;

CREATE INDEX idx_indexer_proxy_configs_provider_type
    ON indexer_proxy_configs(provider_type);
