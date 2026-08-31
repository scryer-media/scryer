ALTER TABLE media_server_connections ADD COLUMN external_url text;

CREATE TABLE media_server_playback_items (
    connection_id text NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    entity_kind text NOT NULL CHECK (entity_kind IN ('title', 'episode')),
    entity_id text NOT NULL,
    provider_item_id text NOT NULL,
    last_seen_at timestamp with time zone NOT NULL,
    PRIMARY KEY (connection_id, entity_kind, entity_id)
);

CREATE INDEX idx_media_server_playback_items_entity
    ON media_server_playback_items (entity_kind, entity_id);
