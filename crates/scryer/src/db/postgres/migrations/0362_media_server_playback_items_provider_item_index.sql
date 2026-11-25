-- The lifecycle playback hold places a live session on a Scryer title by
-- the provider's item id. The primary key leads with the connection and then
-- the Scryer entity, so it cannot serve that lookup; this index serves it.
CREATE INDEX IF NOT EXISTS idx_media_server_playback_items_provider_item
    ON media_server_playback_items (connection_id, provider_item_id);
