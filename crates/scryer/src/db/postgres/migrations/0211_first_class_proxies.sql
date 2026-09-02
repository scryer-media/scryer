-- PostgreSQL twin of migrations/0211_first_class_proxies.sql.
--
-- Postgres can do in place everything SQLite needed a table rebuild for, so
-- this is the same change expressed as ALTERs. Renaming the table carries the
-- primary key, the foreign key from `indexers` and every row with it. Only the
-- secondary index has to be renamed by hand, because its name spells out the
-- old table and column.
ALTER TABLE indexer_proxy_configs RENAME TO proxy_configs;

ALTER INDEX idx_indexer_proxy_configs_provider_type
    RENAME TO idx_proxy_configs_provider_type;

ALTER TABLE proxy_configs
    ALTER COLUMN protocol DROP NOT NULL;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS username_encrypted text;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS password_encrypted text;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS remote_dns boolean DEFAULT false NOT NULL;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS private_key_encrypted text;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS private_key_passphrase_encrypted text;

-- Public by nature: a host key fingerprint is what the operator compares
-- against their own server, so it is stored and shown in the clear.
ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS host_key_fingerprint text;

ALTER TABLE proxy_configs
    ADD COLUMN IF NOT EXISTS host_key_pinned_at timestamp with time zone;

ALTER TABLE indexers RENAME COLUMN indexer_proxy_config_id TO proxy_config_id;

ALTER INDEX idx_indexers_indexer_proxy_config_id
    RENAME TO idx_indexers_proxy_config_id;

-- The second consumer family. A download client may be assigned any kind of
-- proxy, including a challenge solver, so this is a plain nullable reference
-- with no kind constraint.
ALTER TABLE download_clients
    ADD COLUMN IF NOT EXISTS proxy_config_id text;
