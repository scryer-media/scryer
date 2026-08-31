ALTER TABLE media_server_connections ADD COLUMN external_url TEXT;

CREATE TABLE media_server_playback_items (
    connection_id TEXT NOT NULL,
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    provider_item_id TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (connection_id, entity_kind, entity_id),
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE,
    CHECK (entity_kind IN ('title', 'episode'))
);

CREATE INDEX idx_media_server_playback_items_entity
    ON media_server_playback_items (entity_kind, entity_id);
