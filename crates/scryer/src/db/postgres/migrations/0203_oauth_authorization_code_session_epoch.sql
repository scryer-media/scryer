-- Authorization codes are short lived and cannot be safely assigned a past
-- session epoch. Discard them before making the binding mandatory.
DELETE FROM oauth_authorization_codes;

ALTER TABLE oauth_authorization_codes
    ADD COLUMN auth_session_version TEXT NOT NULL DEFAULT '';

-- The orphan cleanup probes this child table by source_id.
CREATE INDEX IF NOT EXISTS idx_indexer_search_run_sources_source
    ON indexer_search_run_candidate_sources(source_id);
