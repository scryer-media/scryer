-- Plan 150: a blocklist row blocks a release for a title.
--
-- It matched on `source_hint` — the release's download URL or magnet — a
-- locator used as an identity. That stored indexer API keys at rest, and once
-- `normalize_release_attempt_hint` stripped them on read it collapsed distinct
-- releases onto one key: two URLs differing only in `?token=` normalize to the
-- same hint, so blocklisting one release silently blocked every release from
-- that indexer for the title. The release name is always present (acquisition's
-- title guard guarantees it) and always sufficient.
--
-- Episode attribution goes with it. `download_id` and `data_json` exist only to
-- pin an entry to episodes; nothing has ever gated on that, and the web app
-- already derives the association from the release name. `quality` had no
-- reader at all.
--
-- The table also had no uniqueness, so one failure recorded by two writers
-- produced two rows and a sweep on the clear path compensated. The sweep is
-- deleted; the collapse below runs once and the unique indexes keep it true.

DROP INDEX IF EXISTS idx_blocklist_source_title;

DELETE FROM blocklist WHERE source_title IS NULL;

ALTER TABLE blocklist RENAME COLUMN source_title TO release_name;
ALTER TABLE blocklist ADD COLUMN normalized_release_name TEXT NOT NULL DEFAULT '';
ALTER TABLE blocklist ADD COLUMN indexer_id TEXT NOT NULL DEFAULT '';
ALTER TABLE blocklist ADD COLUMN info_hash TEXT;
ALTER TABLE blocklist DROP COLUMN source_hint;
ALTER TABLE blocklist DROP COLUMN quality;
ALTER TABLE blocklist DROP COLUMN download_id;
ALTER TABLE blocklist DROP COLUMN data_json;

-- The only place SQL normalizes a release name. Every later write normalizes in
-- Rust, so the two engines cannot drift on non-ASCII names; a pre-migration row
-- that differs here is re-normalized the next time its release is blocked.
UPDATE blocklist SET normalized_release_name = LOWER(TRIM(release_name));

-- Every legacy row carries the empty indexer, so this collapses the duplicate
-- rows the missing constraint allowed. Newest wins.
WITH ranked AS (
    SELECT id, ROW_NUMBER() OVER (
        PARTITION BY title_id, indexer_id, normalized_release_name
        ORDER BY created_at DESC, id DESC
    ) AS rank
    FROM blocklist
)
DELETE FROM blocklist WHERE id IN (SELECT id FROM ranked WHERE rank > 1);

-- Both columns are NOT NULL because a unique index treats NULLs as distinct,
-- which would silently defeat the constraint on exactly the rows that need it.
CREATE UNIQUE INDEX idx_blocklist_release_unique
    ON blocklist (title_id, indexer_id, normalized_release_name)
    WHERE info_hash IS NULL;

CREATE UNIQUE INDEX idx_blocklist_info_hash_unique
    ON blocklist (title_id, info_hash)
    WHERE info_hash IS NOT NULL;
