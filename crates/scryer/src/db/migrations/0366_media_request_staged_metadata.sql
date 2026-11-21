-- The metadata staged for a request subject: what one SMG read returned when
-- a requester picked a search result, reduced to the request shape. A preview
-- and the submit that follows read this record instead of asking SMG again.
-- The key is the facet and the subject's normalized external identifiers.
-- A submitted request copies what it needs onto its own row, so these rows
-- are a staging area: housekeeping removes the ones staged more than 7 days
-- ago, found through the staged_at index.
CREATE TABLE media_request_staged_metadata (
    staging_key TEXT PRIMARY KEY NOT NULL,
    payload_json TEXT NOT NULL,
    staged_at TEXT NOT NULL
);

CREATE INDEX idx_media_request_staged_metadata_staged_at
    ON media_request_staged_metadata (staged_at);
