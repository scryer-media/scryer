-- One archive extraction per tracked download.
--
-- A download's archives are extracted once, into a workspace whose path is
-- derived from the download client and the client's item id, and every
-- import pass of that download reads the same output. The row records where
-- the workspace is and what state it is in, so a retry or a restart never
-- extracts the same download again. It is deleted when the workspace is
-- removed, so the table holds only downloads with scratch on disk.
CREATE TABLE IF NOT EXISTS archive_extractions (
    client_id TEXT NOT NULL,
    client_item_id TEXT NOT NULL,
    state TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    archive_digest TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_reason_code TEXT,
    output_count INTEGER NOT NULL DEFAULT 0,
    output_bytes INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT,
    settled_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (client_id, client_item_id)
);

CREATE INDEX IF NOT EXISTS idx_archive_extractions_state_next_attempt
    ON archive_extractions(state, next_attempt_at);

CREATE INDEX IF NOT EXISTS idx_archive_extractions_settled_at
    ON archive_extractions(settled_at);
