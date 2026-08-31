-- Synthetic stable root ids (FR-078, plan D1). PostgreSQL half of 0204.
--
-- See the SQLite file for the rationale. Only the column types differ; the
-- `migrate_synthetic_root_ids` Rust hook performs the identical remap on both
-- engines.

ALTER TABLE library_roots ADD COLUMN legacy_path_derived_id TEXT;

CREATE TABLE library_root_id_remaps (
    legacy_root_id text PRIMARY KEY NOT NULL,
    root_id text NOT NULL,
    normalized_path text NOT NULL,
    remapped boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_library_root_id_remaps_root
    ON library_root_id_remaps(root_id);
