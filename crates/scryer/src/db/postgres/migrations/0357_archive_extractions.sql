-- One archive extraction per tracked download. See the SQLite twin for what
-- the columns mean.
CREATE TABLE IF NOT EXISTS archive_extractions (
    client_id TEXT NOT NULL,
    client_item_id TEXT NOT NULL,
    state TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    archive_digest TEXT,
    attempts BIGINT NOT NULL DEFAULT 0,
    last_reason_code TEXT,
    output_count BIGINT NOT NULL DEFAULT 0,
    output_bytes BIGINT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ,
    settled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (client_id, client_item_id)
);

CREATE INDEX IF NOT EXISTS idx_archive_extractions_settled_at
    ON archive_extractions(settled_at, client_id, client_item_id);

CREATE INDEX IF NOT EXISTS idx_archive_extractions_workspace_name
    ON archive_extractions((substr(workspace_path, length(workspace_path) - 15)));
