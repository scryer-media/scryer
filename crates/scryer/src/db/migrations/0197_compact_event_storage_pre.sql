DELETE FROM domain_events
WHERE event_type = 'discovery_search_completed'
   OR (
       event_type IN ('acquisition_search_completed', 'acquisition_candidate_rejected')
       AND julianday(occurred_at) <= julianday('now', '-1 day')
   );

DROP INDEX IF EXISTS idx_domain_events_stream_sequence;

ALTER TABLE domain_events RENAME TO domain_events_legacy_0197;
CREATE TABLE domain_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    occurred_at TEXT NOT NULL,
    actor_user_id TEXT,
    title_id TEXT,
    facet TEXT,
    correlation_id TEXT,
    causation_id TEXT,
    schema_version INTEGER NOT NULL,
    stream_kind TEXT NOT NULL,
    stream_id TEXT,
    event_type TEXT NOT NULL,
    payload_json BLOB NOT NULL,
    actor_kind TEXT NOT NULL DEFAULT 'system',
    actor_display_name TEXT NOT NULL DEFAULT 'System',
    import_status TEXT,
    media_file_delete_reason TEXT,
    download_id TEXT
);

ALTER TABLE release_decisions RENAME TO release_decisions_legacy_0197;
CREATE TABLE release_decisions (
    id TEXT PRIMARY KEY,
    wanted_item_id TEXT NOT NULL REFERENCES wanted_items(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL,
    release_title TEXT NOT NULL,
    release_url TEXT,
    release_size_bytes INTEGER,
    decision_code TEXT NOT NULL,
    candidate_score INTEGER NOT NULL,
    current_score INTEGER,
    score_delta INTEGER,
    explanation_json BLOB,
    created_at TEXT NOT NULL
);
