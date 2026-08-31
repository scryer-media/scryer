-- Persisted full-file content hashes on media files (FR-041, FR-046, plan D2/D4).
-- PostgreSQL half of 0205; see the SQLite file for the rationale.

ALTER TABLE media_files ADD COLUMN full_blake3 text;
ALTER TABLE media_files ADD COLUMN move_crc text;
ALTER TABLE media_files ADD COLUMN move_crc_algorithm text;
ALTER TABLE media_files ADD COLUMN hash_computed_at timestamptz;

CREATE INDEX idx_media_files_full_blake3
    ON media_files(full_blake3)
    WHERE full_blake3 IS NOT NULL;

CREATE INDEX idx_media_files_full_hash_missing
    ON media_files(id)
    WHERE full_blake3 IS NULL;
