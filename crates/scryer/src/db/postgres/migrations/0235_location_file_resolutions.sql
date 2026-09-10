CREATE TABLE location_file_resolutions (
    operation_id TEXT NOT NULL REFERENCES location_operations(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL,
    source_path TEXT NOT NULL,
    resolution_json TEXT NOT NULL,
    PRIMARY KEY (operation_id, title_id, source_path)
);
