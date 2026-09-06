-- Post-processing script output tails become zstd frames (see
-- scryer_infrastructure_sql::script_output). SQLite cannot retype a column in
-- place, so the table is rebuilt: the legacy table is renamed, the new one is
-- created with BLOB tails, the Rust hook `compress_post_processing_output`
-- copies every row across while compressing, and the _post half drops the
-- legacy table and restores the indexes.
DROP INDEX IF EXISTS idx_pp_script_runs_script_id;
DROP INDEX IF EXISTS idx_pp_script_runs_title_id;

ALTER TABLE post_processing_script_runs RENAME TO post_processing_script_runs_legacy_0210;

CREATE TABLE post_processing_script_runs (
    id TEXT PRIMARY KEY,
    script_id TEXT NOT NULL,
    script_name TEXT NOT NULL,                    -- denormalized for history
    title_id TEXT,
    title_name TEXT,
    facet TEXT,
    file_path TEXT,
    status TEXT NOT NULL,                         -- 'success' | 'failed' | 'timeout' | 'running'
    exit_code INTEGER,
    stdout_tail BLOB,                             -- last 32 KiB, zstd frame
    stderr_tail BLOB,                             -- last 32 KiB, zstd frame
    duration_ms INTEGER,
    env_payload_json TEXT,                        -- the JSON payload passed to the script
    started_at TEXT NOT NULL,
    completed_at TEXT,
    FOREIGN KEY (script_id) REFERENCES post_processing_scripts(id) ON DELETE CASCADE
);
