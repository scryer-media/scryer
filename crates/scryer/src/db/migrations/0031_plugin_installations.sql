CREATE TABLE IF NOT EXISTS plugin_installations (
    id            TEXT PRIMARY KEY,
    plugin_id     TEXT NOT NULL UNIQUE,
    name          TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    version       TEXT NOT NULL,
    plugin_type   TEXT NOT NULL DEFAULT 'indexer',
    provider_type TEXT NOT NULL,
    is_enabled    INTEGER NOT NULL DEFAULT 1,
    is_builtin    INTEGER NOT NULL DEFAULT 0,
    wasm_bytes    BLOB,
    wasm_sha256   TEXT,
    source_url    TEXT,
    installed_at  TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
