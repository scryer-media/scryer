CREATE TABLE IF NOT EXISTS history_events(
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    actor_user_id TEXT,
    title_id TEXT,
    message TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    source TEXT,
    created_at TEXT NOT NULL,
    metadata_json TEXT,
    FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS event_outboxes(
    id TEXT PRIMARY KEY,
    history_event_id TEXT NOT NULL,
    channel_key TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    dispatched_at TEXT,
    FOREIGN KEY (history_event_id) REFERENCES history_events(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS scheduler_jobs(
    id TEXT PRIMARY KEY,
    job_name TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    schedule_cron TEXT,
    next_run_at TEXT,
    status TEXT NOT NULL DEFAULT 'enabled',
    last_run_at TEXT,
    last_result TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS notification_channels(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    config_json TEXT NOT NULL,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS notification_subscriptions(
    id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (channel_id) REFERENCES notification_channels(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_history_events_title_time
    ON history_events (title_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_history_events_type_time
    ON history_events (event_type, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_event_outboxes_status
    ON event_outboxes (status, updated_at);

CREATE INDEX IF NOT EXISTS idx_event_outboxes_channel
    ON event_outboxes (channel_key);

CREATE INDEX IF NOT EXISTS idx_scheduler_jobs_name
    ON scheduler_jobs (job_name);

CREATE INDEX IF NOT EXISTS idx_scheduler_jobs_status_next_run
    ON scheduler_jobs (status, next_run_at);

CREATE UNIQUE INDEX IF NOT EXISTS idx_notification_channels_name_type
    ON notification_channels (name, channel_type);

CREATE UNIQUE INDEX IF NOT EXISTS idx_notification_subscriptions_channel_scope
    ON notification_subscriptions (channel_id, event_type, COALESCE(scope, ''), COALESCE(scope_id, ''));
