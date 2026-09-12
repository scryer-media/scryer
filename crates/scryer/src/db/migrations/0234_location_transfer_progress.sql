CREATE TABLE location_transfer_runtime (
    id BIGINT PRIMARY KEY CHECK (id = 1),
    generation BIGINT NOT NULL
);
INSERT INTO location_transfer_runtime (id, generation) VALUES (1, 0);
CREATE TABLE location_transfer_progress (
    operation_id TEXT PRIMARY KEY REFERENCES location_operations(id) ON DELETE CASCADE,
    progress_basis_points BIGINT NOT NULL DEFAULT 0,
    titles_initialized BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE TABLE location_transfer_titles (
    operation_id TEXT NOT NULL REFERENCES location_operations(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL,
    sequence BIGINT NOT NULL,
    hot_rank BIGINT NOT NULL,
    summary_json TEXT NOT NULL,
    PRIMARY KEY (operation_id, title_id)
);
CREATE INDEX idx_location_transfer_titles_page ON location_transfer_titles(operation_id, hot_rank, sequence, title_id);
