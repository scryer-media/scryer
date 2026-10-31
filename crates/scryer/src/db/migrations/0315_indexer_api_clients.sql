CREATE TABLE indexer_api_clients (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    protocols_json TEXT NOT NULL,
    lookup_id TEXT UNIQUE,
    secret_hash TEXT,
    key_generation INTEGER NOT NULL CHECK (key_generation > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    last_used_at TEXT,
    last_outcome_code TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK ((lookup_id IS NULL) = (secret_hash IS NULL))
);

CREATE TABLE indexer_api_client_grants (
    client_id TEXT NOT NULL REFERENCES indexer_api_clients(id) ON DELETE CASCADE,
    indexer_id TEXT NOT NULL REFERENCES indexers(id) ON DELETE CASCADE,
    categories_json TEXT,
    PRIMARY KEY (client_id, indexer_id)
);

CREATE INDEX indexer_api_client_grants_source ON indexer_api_client_grants(indexer_id);
