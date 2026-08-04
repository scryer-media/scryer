CREATE TABLE IF NOT EXISTS manual_import_selections (
    id TEXT PRIMARY KEY,
    actor_user_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    source_client_id TEXT NOT NULL DEFAULT '',
    source_system TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_manual_import_selections_source
    ON manual_import_selections (source_client_id, source_system, source_ref);
CREATE INDEX IF NOT EXISTS idx_manual_import_selections_owner
    ON manual_import_selections (actor_user_id, title_id, source_client_id, source_system, source_ref);

CREATE TABLE IF NOT EXISTS manual_import_selection_candidates (
    id TEXT PRIMARY KEY,
    selection_id TEXT NOT NULL,
    canonical_path TEXT NOT NULL,
    quality TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (selection_id, canonical_path)
);

CREATE INDEX IF NOT EXISTS idx_manual_import_selection_candidates_selection
    ON manual_import_selection_candidates (selection_id);
