ALTER TABLE series_movie_links
    ADD COLUMN monitoring_override INTEGER;
ALTER TABLE series_movie_links
    ADD COLUMN metadata_active INTEGER NOT NULL DEFAULT 1;

-- Existing derived links were created by policy, not by an operator toggle.
-- Disable them once; the next title-policy reconciliation selectively enables
-- canonical links for All and Missing monitoring modes.
UPDATE series_movie_links
SET monitored = 0,
    monitoring_override = NULL,
    metadata_active = 1
WHERE COALESCE(source, '') = 'anibridge';
