ALTER TABLE workflow_operations ADD COLUMN job_key TEXT;
ALTER TABLE workflow_operations ADD COLUMN trigger_source TEXT;
ALTER TABLE workflow_operations ADD COLUMN summary_json TEXT;
ALTER TABLE workflow_operations ADD COLUMN summary_text TEXT;
ALTER TABLE workflow_operations ADD COLUMN error_text TEXT;

CREATE INDEX IF NOT EXISTS idx_workflow_operations_job_key_started
    ON workflow_operations (job_key, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_workflow_operations_job_key_status
    ON workflow_operations (job_key, status, started_at DESC);

CREATE TABLE IF NOT EXISTS library_probe_signatures(
    title_id TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    probe_signature_scheme TEXT,
    probe_signature_value TEXT,
    last_probed_at TEXT,
    last_changed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_library_probe_signatures_last_probed
    ON library_probe_signatures (last_probed_at DESC);
