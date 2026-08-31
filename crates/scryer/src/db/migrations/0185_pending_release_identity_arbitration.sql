ALTER TABLE pending_releases
    ADD COLUMN release_identity TEXT;
ALTER TABLE pending_releases
    ADD COLUMN last_observed_at TEXT NOT NULL DEFAULT '';
ALTER TABLE pending_releases
    ADD COLUMN coverage_identity TEXT;
ALTER TABLE pending_releases
    ADD COLUMN role TEXT NOT NULL DEFAULT 'primary';
ALTER TABLE pending_releases
    ADD COLUMN last_decision_code TEXT;
ALTER TABLE pending_releases
    ADD COLUMN release_age_unknown INTEGER NOT NULL DEFAULT 0;

UPDATE pending_releases
SET release_identity = CASE
        WHEN length(trim(COALESCE(release_guid, ''))) > 0 THEN
            'guid:' || lower(trim(COALESCE(indexer_id, indexer_source, 'unknown'))) || ':' || lower(trim(release_guid))
        WHEN length(trim(COALESCE(info_hash, ''))) > 0 THEN
            'hash:' || lower(trim(info_hash))
        WHEN length(trim(COALESCE(release_url, ''))) > 0 THEN
            'source:' || trim(release_url)
        ELSE
            'listing:' || lower(trim(COALESCE(indexer_id, indexer_source, 'unknown'))) || ':' ||
            lower(trim(release_title)) || ':' || lower(trim(COALESCE(published_at, 'unknown')))
    END,
    last_observed_at = added_at,
    coverage_identity = 'scope:' || lower(trim(wanted_item_id)),
    role = CASE WHEN status = 'standby' THEN 'fallback' ELSE 'primary' END,
    release_age_unknown = CASE
        WHEN published_at IS NULL AND julianday(delay_until) > julianday(added_at) THEN 1
        ELSE 0
    END;

WITH duplicate_active AS (
    SELECT id,
           row_number() OVER (
               PARTITION BY release_identity
               ORDER BY added_at ASC, id ASC
           ) AS duplicate_rank
    FROM pending_releases
    WHERE status IN ('waiting', 'standby', 'processing', 'needs_review')
)
UPDATE pending_releases
SET status = 'superseded'
WHERE id IN (
    SELECT id FROM duplicate_active WHERE duplicate_rank > 1
);

CREATE UNIQUE INDEX idx_pending_releases_active_release_identity
    ON pending_releases(release_identity)
    WHERE status IN ('waiting', 'standby', 'processing', 'needs_review');
CREATE INDEX idx_pending_releases_active_coverage
    ON pending_releases(coverage_identity, status, published_at, added_at);
CREATE INDEX idx_pending_releases_active_unknown_age
    ON pending_releases(release_age_unknown, status, indexer_id, added_at)
    WHERE release_age_unknown = 1;
