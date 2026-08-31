-- Persisted full-file content hashes on media files (FR-041, FR-046, plan D2/D4).
--
-- `source_signature_scheme`/`source_signature_value` stay what they are: the
-- sampled head+tail proof scans compute. These columns are the separate,
-- expensive values produced by a location operation's single streaming copy pass
-- (or by the background backfill job), and they are what the dedup gate compares.
--
-- All nullable: a file only has them once something has actually read it end to
-- end, and a scan that sees the sampled proof change clears them again.
--
-- `move_crc` is algorithm-tagged rather than assuming one algorithm forever: the
-- crate picks the fastest CRC the host supports, and that choice is allowed to
-- change between releases. A row is only comparable to another row with the same
-- `move_crc_algorithm`.

ALTER TABLE media_files ADD COLUMN full_blake3 TEXT;
ALTER TABLE media_files ADD COLUMN move_crc TEXT;
ALTER TABLE media_files ADD COLUMN move_crc_algorithm TEXT;
ALTER TABLE media_files ADD COLUMN hash_computed_at TEXT;

-- Dedup candidacy pre-filters on size + sampled proof, but the deciding
-- comparison is always full hash against full hash, so that lookup gets an index.
CREATE INDEX idx_media_files_full_blake3
    ON media_files(full_blake3)
    WHERE full_blake3 IS NOT NULL;

-- The backfill job's work queue: files still missing a full hash.
CREATE INDEX idx_media_files_full_hash_missing
    ON media_files(id)
    WHERE full_blake3 IS NULL;
