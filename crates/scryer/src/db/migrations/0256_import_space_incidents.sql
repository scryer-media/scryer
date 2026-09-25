CREATE TABLE import_space_incident_lock (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL
);
INSERT INTO import_space_incident_lock (id, revision) VALUES (1, 0);

CREATE TABLE import_space_incidents (
    destination_key TEXT NOT NULL PRIMARY KEY,
    incident_id TEXT NOT NULL
);
CREATE TABLE import_space_members (
    member_key TEXT NOT NULL PRIMARY KEY,
    job_key TEXT NOT NULL,
    destination_key TEXT NOT NULL,
    measurement_json TEXT NOT NULL,
    download_id TEXT NOT NULL
);
CREATE INDEX idx_import_space_members_job ON import_space_members(job_key);
CREATE INDEX idx_import_space_members_destination ON import_space_members(destination_key);
CREATE TABLE import_space_notification_receipts (
    event_id TEXT NOT NULL,
    target_key TEXT NOT NULL,
    PRIMARY KEY (event_id, target_key)
);
