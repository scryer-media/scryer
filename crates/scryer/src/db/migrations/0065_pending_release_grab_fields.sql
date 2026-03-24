-- Store fields needed by try_grab_pending_release that were previously lost
-- when a release was held by a delay profile.  Without these, the pending
-- grab path produces an incomplete DownloadClientAddRequest (no password,
-- no info hash, wrong queue priority).
ALTER TABLE pending_releases ADD COLUMN source_password TEXT;
ALTER TABLE pending_releases ADD COLUMN published_at TEXT;
ALTER TABLE pending_releases ADD COLUMN info_hash TEXT;
