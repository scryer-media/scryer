CREATE TABLE IF NOT EXISTS domain_events(
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
    payload_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_domain_events_occurred_at
    ON domain_events (occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_domain_events_event_type_sequence
    ON domain_events (event_type, sequence DESC);

CREATE INDEX IF NOT EXISTS idx_domain_events_title_sequence
    ON domain_events (title_id, sequence DESC);

CREATE INDEX IF NOT EXISTS idx_domain_events_facet_sequence
    ON domain_events (facet, sequence DESC);

CREATE INDEX IF NOT EXISTS idx_domain_events_stream_sequence
    ON domain_events (stream_kind, stream_id, sequence DESC);

CREATE TABLE IF NOT EXISTS event_subscriber_offsets(
    subscriber_name TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
