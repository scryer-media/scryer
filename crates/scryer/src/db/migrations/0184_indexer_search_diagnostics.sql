CREATE TABLE IF NOT EXISTS indexer_search_runs (
    id TEXT PRIMARY KEY,
    indexer_id TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    search_session_id TEXT NOT NULL,
    scope_key TEXT NOT NULL,
    query_signature TEXT NOT NULL,
    branch TEXT NOT NULL,
    page INTEGER,
    -- Reserved for the per-strategy search corpus (plan 151): the provider
    -- offset this run requested and the next offset it advertised. Nothing
    -- reads or writes them yet.
    provider_offset INTEGER,
    next_provider_offset INTEGER,
    range_min_size INTEGER,
    range_max_size INTEGER,
    result_count INTEGER NOT NULL,
    completion_state TEXT NOT NULL,
    retry_at TEXT,
    error_summary TEXT,
    indexer_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_indexer_search_runs_scope_created
    ON indexer_search_runs(scope_key, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_indexer_search_runs_indexer_created
    ON indexer_search_runs(indexer_id, created_at DESC);

CREATE TABLE IF NOT EXISTS indexer_search_candidates (
    id TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    normalized_title TEXT NOT NULL,
    size_bytes INTEGER,
    source_kind TEXT,
    info_hash TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    reusable_until TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS indexer_search_candidate_sources (
    id TEXT PRIMARY KEY,
    candidate_id TEXT NOT NULL REFERENCES indexer_search_candidates(id) ON DELETE CASCADE,
    indexer_id TEXT NOT NULL,
    source_identity TEXT NOT NULL,
    provider_ref TEXT,
    source TEXT NOT NULL,
    encrypted_download_url TEXT,
    encrypted_link_url TEXT,
    published_at TEXT,
    thumbs_up INTEGER,
    thumbs_down INTEGER,
    grabs INTEGER,
    grab_current INTEGER,
    grab_max INTEGER,
    response_tvdb_id TEXT,
    response_tmdb_id TEXT,
    response_imdb_id TEXT,
    season INTEGER,
    episode INTEGER,
    absolute_episode INTEGER,
    release_group TEXT,
    provider_source TEXT,
    seeders INTEGER,
    peers INTEGER,
    download_volume_factor REAL,
    upload_volume_factor REAL,
    protected INTEGER,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    reusable_until TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    UNIQUE(candidate_id, indexer_id, source_identity)
);

CREATE TABLE IF NOT EXISTS indexer_search_run_candidate_sources (
    run_id TEXT NOT NULL REFERENCES indexer_search_runs(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES indexer_search_candidate_sources(id) ON DELETE CASCADE,
    search_session_id TEXT NOT NULL,
    PRIMARY KEY(run_id, source_id)
);

CREATE TABLE IF NOT EXISTS indexer_search_candidate_source_values (
    source_id TEXT NOT NULL REFERENCES indexer_search_candidate_sources(id) ON DELETE CASCADE,
    value_kind TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY(source_id, value_kind, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_indexer_search_candidates_expiry
    ON indexer_search_candidates(expires_at);
CREATE INDEX IF NOT EXISTS idx_indexer_search_sources_indexer_reusable
    ON indexer_search_candidate_sources(indexer_id, reusable_until);
CREATE INDEX IF NOT EXISTS idx_indexer_search_run_sources_run
    ON indexer_search_run_candidate_sources(run_id);
CREATE INDEX IF NOT EXISTS idx_indexer_search_run_sources_session
    ON indexer_search_run_candidate_sources(search_session_id);
