CREATE TABLE IF NOT EXISTS download_clients(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    client_type TEXT NOT NULL,
    base_url TEXT,
    config_json TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'idle',
    last_error TEXT,
    last_seen_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workflow_operations(
    id TEXT PRIMARY KEY,
    operation_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    actor_user_id TEXT,
    title_id TEXT,
    collection_id TEXT,
    episode_id TEXT,
    release_id TEXT,
    media_file_id TEXT,
    external_reference TEXT,
    progress_json TEXT,
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL,
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL,
    FOREIGN KEY (release_id) REFERENCES releases(id) ON DELETE SET NULL,
    FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS download_jobs(
    id TEXT PRIMARY KEY,
    workflow_operation_id TEXT NOT NULL,
    download_client_id TEXT NOT NULL,
    release_id TEXT,
    source_hint TEXT,
    payload_json TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workflow_operation_id) REFERENCES workflow_operations(id) ON DELETE CASCADE,
    FOREIGN KEY (download_client_id) REFERENCES download_clients(id) ON DELETE RESTRICT,
    FOREIGN KEY (release_id) REFERENCES releases(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS imports(
    id TEXT PRIMARY KEY,
    source_system TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    import_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    payload_json TEXT NOT NULL,
    result_json TEXT,
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS upgrades(
    id TEXT PRIMARY KEY,
    component TEXT NOT NULL,
    from_version TEXT,
    to_version TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    workflow_operation_id TEXT,
    actor_user_id TEXT,
    started_at TEXT,
    finished_at TEXT,
    error_message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workflow_operation_id) REFERENCES workflow_operations(id) ON DELETE SET NULL,
    FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS quarantine_items(
    id TEXT PRIMARY KEY,
    media_file_id TEXT,
    file_path TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    reason_json TEXT,
    quarantined_by TEXT,
    quarantined_at TEXT NOT NULL,
    release_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE SET NULL,
    FOREIGN KEY (quarantined_by) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (release_id) REFERENCES releases(id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_download_clients_name
    ON download_clients (name);

CREATE INDEX IF NOT EXISTS idx_workflow_operations_status_started
    ON workflow_operations (status, started_at);

CREATE INDEX IF NOT EXISTS idx_download_jobs_workflow
    ON download_jobs (workflow_operation_id);

CREATE INDEX IF NOT EXISTS idx_download_jobs_client
    ON download_jobs (download_client_id, status);

CREATE UNIQUE INDEX IF NOT EXISTS idx_imports_source_ref
    ON imports (source_system, source_ref, import_type);

CREATE INDEX IF NOT EXISTS idx_upgrades_status
    ON upgrades (status);

CREATE UNIQUE INDEX IF NOT EXISTS idx_quarantine_items_file
    ON quarantine_items (file_path);
