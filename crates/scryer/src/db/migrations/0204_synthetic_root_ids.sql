-- Synthetic stable root ids (FR-078, plan D1).
--
-- Root identity used to be a pure function of the root path:
-- `root_folder_id_for_path` hashed the platform-normalized path, and every root
-- write recomputed it. Changing a root's path therefore changed its identity and
-- orphaned every title that referenced it. This migration breaks that functional
-- dependency: the path-derived value is recorded for diagnostics and legacy
-- lookup, and the id itself becomes an opaque, frozen value.
--
-- The schema half only adds the retention surfaces. The remap itself is the
-- `migrate_synthetic_root_ids` Rust hook in the same migration, because it has to
-- rewrite `library_roots.id` and every referent of it in one transaction.

ALTER TABLE library_roots ADD COLUMN legacy_path_derived_id TEXT;

-- Legacy path-derived id -> current synthetic root id.
--
-- Populated for every root, including roots that already carried a non
-- path-derived id (the seeded `canonical_root_for_*` rows), so any caller still
-- holding a path-derived id can resolve the real root. Deliberately without a
-- foreign key onto `library_roots`: the library update path deletes and
-- reinserts root rows, and the mapping is a durable audit record of what the
-- migration did, not a live association.
CREATE TABLE library_root_id_remaps (
    legacy_root_id TEXT PRIMARY KEY NOT NULL,
    root_id TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    remapped INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_library_root_id_remaps_root
    ON library_root_id_remaps(root_id);
