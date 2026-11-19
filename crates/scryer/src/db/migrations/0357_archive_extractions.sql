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

-- Settled rows due for removal, oldest first; and unsettled rows (settled_at
-- IS NULL) in key order, for the pass that settles downloads no longer
-- tracked.
CREATE INDEX IF NOT EXISTS idx_archive_extractions_settled_at
    ON archive_extractions(settled_at, client_id, client_item_id);

-- Whether a workspace found on disk still has a row, by the workspace's
-- name: its last 16 characters, whichever spelling of the library root the
-- row was written with.
CREATE INDEX IF NOT EXISTS idx_archive_extractions_workspace_name
    ON archive_extractions(substr(workspace_path, length(workspace_path) - 15));
