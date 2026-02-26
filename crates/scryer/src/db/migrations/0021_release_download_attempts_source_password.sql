-- Add source password retention for release download attempts.
-- This allows password protected releases to be retried without re-scraping metadata.

ALTER TABLE release_download_attempts
    ADD COLUMN source_password TEXT;
