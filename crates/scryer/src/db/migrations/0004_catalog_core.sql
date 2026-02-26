CREATE TABLE IF NOT EXISTS titles(
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    name_normalized TEXT NOT NULL DEFAULT '',
    facet TEXT NOT NULL,
    monitored INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'active',
    tags TEXT NOT NULL DEFAULT '[]',
    external_ids TEXT NOT NULL DEFAULT '[]',
    created_by TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT,
    deleted_at TEXT
);

CREATE TABLE IF NOT EXISTS title_aliases(
    id TEXT PRIMARY KEY NOT NULL,
    title_id TEXT NOT NULL,
    alias_type TEXT NOT NULL,
    alias_value TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS title_external_ids(
    id TEXT PRIMARY KEY NOT NULL,
    title_id TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS collections(
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    collection_type TEXT NOT NULL,
    collection_index TEXT NOT NULL,
    label TEXT,
    ordered_path TEXT,
    first_episode_number TEXT,
    last_episode_number TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS episodes(
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    collection_id TEXT,
    episode_type TEXT NOT NULL,
    episode_number TEXT,
    season_number TEXT,
    episode_label TEXT,
    title TEXT,
    air_date TEXT,
    duration_seconds INTEGER,
    has_multi_audio INTEGER DEFAULT 0,
    has_subtitle INTEGER DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE,
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_title_aliases_title_alias
    ON title_aliases(title_id, alias_type, alias_value);

CREATE UNIQUE INDEX IF NOT EXISTS idx_title_external_ids_lookup
    ON title_external_ids(title_id, source, external_id);

CREATE INDEX IF NOT EXISTS idx_collections_title
    ON collections (title_id, collection_type);

CREATE INDEX IF NOT EXISTS idx_episodes_title
    ON episodes (title_id, season_number);

CREATE INDEX IF NOT EXISTS idx_episodes_collection
    ON episodes (collection_id);
