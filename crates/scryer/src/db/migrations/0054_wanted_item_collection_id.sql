ALTER TABLE wanted_items ADD COLUMN collection_id TEXT REFERENCES collections(id);
CREATE UNIQUE INDEX idx_wanted_items_collection_id ON wanted_items(collection_id) WHERE collection_id IS NOT NULL;
