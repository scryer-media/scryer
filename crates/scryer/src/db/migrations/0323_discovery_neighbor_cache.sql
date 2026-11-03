-- The related titles the gateway last returned for one discovery target key,
-- in one metadata language, so expanding the same anchor again is a local
-- read. Entries expire on the same schedule as a title's more-like-this rows
-- and are dropped when a newer write finds them stale.
CREATE TABLE discovery_neighbor_cache (
    target_key TEXT NOT NULL,
    language TEXT NOT NULL,
    payload_version INTEGER NOT NULL,
    payload_blob BLOB NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (target_key, language)
);

CREATE INDEX idx_discovery_neighbor_cache_fetched_at
    ON discovery_neighbor_cache (fetched_at);
