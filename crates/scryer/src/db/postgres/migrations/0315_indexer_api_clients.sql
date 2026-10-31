CREATE TABLE indexer_api_clients (
    id text PRIMARY KEY NOT NULL,
    name text NOT NULL,
    enabled boolean NOT NULL,
    protocols_json text NOT NULL,
    lookup_id text UNIQUE,
    secret_hash text,
    key_generation bigint NOT NULL CHECK (key_generation > 0),
    revision bigint NOT NULL CHECK (revision > 0),
    last_used_at timestamp with time zone,
    last_outcome_code text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CHECK ((lookup_id IS NULL) = (secret_hash IS NULL))
);

CREATE TABLE indexer_api_client_grants (
    client_id text NOT NULL REFERENCES indexer_api_clients(id) ON DELETE CASCADE,
    indexer_id text NOT NULL REFERENCES indexers(id) ON DELETE CASCADE,
    categories_json text,
    PRIMARY KEY (client_id, indexer_id)
);

CREATE INDEX indexer_api_client_grants_source ON indexer_api_client_grants(indexer_id);
