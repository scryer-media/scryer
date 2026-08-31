ALTER TABLE series_movie_links
    ADD COLUMN monitoring_override boolean;
ALTER TABLE series_movie_links
    ADD COLUMN metadata_active boolean NOT NULL DEFAULT true;

-- Existing derived links were created by policy, not by an operator toggle.
-- Disable them once; the next title-policy reconciliation selectively enables
-- canonical links for All and Missing monitoring modes.
UPDATE series_movie_links
SET monitored = false,
    monitoring_override = NULL,
    metadata_active = true
WHERE COALESCE(source, '') = 'anibridge';
