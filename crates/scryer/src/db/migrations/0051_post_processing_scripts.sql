CREATE TABLE post_processing_scripts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT DEFAULT '',
    script_type TEXT NOT NULL DEFAULT 'inline',   -- 'inline' | 'file'
    script_content TEXT NOT NULL DEFAULT '',       -- shell command (inline) or file path
    applied_facets TEXT NOT NULL DEFAULT '[]',     -- JSON: ["movie","tv","anime"]
    execution_mode TEXT NOT NULL DEFAULT 'blocking', -- 'blocking' | 'fire_and_forget'
    timeout_secs INTEGER DEFAULT 300,
    priority INTEGER NOT NULL DEFAULT 0,          -- lower = runs first
    enabled INTEGER NOT NULL DEFAULT 1,
    debug INTEGER NOT NULL DEFAULT 0,             -- capture stdout/stderr when enabled
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

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
    stdout_tail TEXT,                             -- last 4KB
    stderr_tail TEXT,                             -- last 4KB
    duration_ms INTEGER,
    env_payload_json TEXT,                        -- the JSON payload passed to the script
    started_at TEXT NOT NULL,
    completed_at TEXT,
    FOREIGN KEY (script_id) REFERENCES post_processing_scripts(id) ON DELETE CASCADE
);

CREATE INDEX idx_pp_script_runs_script_id ON post_processing_script_runs(script_id, started_at DESC);
CREATE INDEX idx_pp_script_runs_title_id ON post_processing_script_runs(title_id, started_at DESC);
