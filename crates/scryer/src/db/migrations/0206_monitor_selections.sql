-- Advanced monitoring stores an explicit set of seasons and canon series
-- movies per owner (a title, or a still-pending media request). Everything the
-- owner does not list stays unmonitored.
CREATE TABLE monitor_selections (
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    entry_kind TEXT NOT NULL,
    entry_key TEXT NOT NULL,
    label TEXT,
    external_ids_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (owner_kind, owner_id, entry_kind, entry_key),
    CHECK (owner_kind IN ('title', 'media_request')),
    CHECK (entry_kind IN ('season', 'series_movie'))
);

CREATE INDEX idx_monitor_selections_owner
    ON monitor_selections (owner_kind, owner_id);

-- `media_requests.requested_monitor_type` carries a CHECK that enumerates the
-- pre-advanced monitor types, so `advanced` cannot be stored until the table is
-- rebuilt without it. Validation already lives in the application layer
-- (`normalize_requested_monitor_type`), so the rebuilt table drops the CHECK
-- rather than extending it.
--
-- Migrations run inside a transaction, where `PRAGMA foreign_keys` is a no-op,
-- so `DROP TABLE media_requests` fires the ON DELETE CASCADE on
-- `media_request_external_ids` and `media_request_requesters`. Those rows are
-- copied aside first and restored afterwards, which produces the same result
-- whether or not foreign-key enforcement happens to be on.
CREATE TABLE media_request_external_ids_0206_backup AS
    SELECT request_id, library_id, source, external_id, created_at
      FROM media_request_external_ids;

CREATE TABLE media_request_requesters_0206_backup AS
    SELECT request_id, user_id, requested_at
      FROM media_request_requesters;

CREATE TABLE media_requests_0206 (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    status TEXT NOT NULL,
    identity_fingerprint TEXT NOT NULL,
    title TEXT NOT NULL,
    sort_title TEXT,
    slug TEXT,
    poster_url TEXT,
    year INTEGER,
    overview TEXT,
    runtime_minutes INTEGER,
    language TEXT,
    content_status TEXT,
    requested_quality_profile_id TEXT,
    requested_quality_profile_name TEXT,
    requested_monitor_type TEXT,
    resolved_by_user_id TEXT,
    resolved_at TEXT,
    created_title_id TEXT,
    approved_quality_profile_id TEXT,
    approved_quality_profile_name TEXT,
    created_by_user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    rating_summary_json TEXT NOT NULL DEFAULT '{"rating":null,"rating_sources":[],"external_ratings":[]}',
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE,
    FOREIGN KEY (created_by_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (resolved_by_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (created_title_id) REFERENCES titles(id) ON DELETE SET NULL,
    CHECK (facet IN ('movie', 'series', 'anime')),
    CHECK (status IN ('pending', 'approved', 'rejected', 'canceled'))
);

INSERT INTO media_requests_0206 (
    id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
    poster_url, year, overview, runtime_minutes, language, content_status,
    requested_quality_profile_id, requested_quality_profile_name, requested_monitor_type,
    resolved_by_user_id, resolved_at, created_title_id,
    approved_quality_profile_id, approved_quality_profile_name,
    created_by_user_id, created_at, updated_at, rating_summary_json
)
SELECT
    id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
    poster_url, year, overview, runtime_minutes, language, content_status,
    requested_quality_profile_id, requested_quality_profile_name, requested_monitor_type,
    resolved_by_user_id, resolved_at, created_title_id,
    approved_quality_profile_id, approved_quality_profile_name,
    created_by_user_id, created_at, updated_at, rating_summary_json
FROM media_requests;

DROP TABLE media_requests;

ALTER TABLE media_requests_0206 RENAME TO media_requests;

CREATE INDEX idx_media_requests_library_facet_status
    ON media_requests (library_id, facet, status);

CREATE INDEX idx_media_requests_status_updated
    ON media_requests (status, updated_at);

CREATE INDEX idx_media_requests_created_title
    ON media_requests (created_title_id);

DELETE FROM media_request_external_ids;
INSERT INTO media_request_external_ids (request_id, library_id, source, external_id, created_at)
    SELECT request_id, library_id, source, external_id, created_at
      FROM media_request_external_ids_0206_backup;

DELETE FROM media_request_requesters;
INSERT INTO media_request_requesters (request_id, user_id, requested_at)
    SELECT request_id, user_id, requested_at
      FROM media_request_requesters_0206_backup;

DROP TABLE media_request_external_ids_0206_backup;
DROP TABLE media_request_requesters_0206_backup;
