-- An HTTP-family release (provider page, direct file, HLS or DASH resources)
-- is described by the indexer as a structured object, not by a download URL.
-- A delayed grab must re-resolve and submit that same description, so the
-- park freezes it as opaque JSON next to the listing snapshot.
-- Nullable, no backfill: rows parked before this column, and every release of
-- another protocol family, carry no HTTP release.
ALTER TABLE pending_releases
    ADD COLUMN IF NOT EXISTS http_release_json TEXT;
