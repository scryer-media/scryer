-- A library graph is the gateway's similarity graph over one facet's owned
-- titles. Each build is a run; its nodes, edges and clusters hang off the run
-- id so a new build is written beside the previous one and the previous one
-- is deleted after the new one lands. Runs that failed keep their error code
-- and text and carry no children.
CREATE TABLE discovery_library_graph_runs (
    id TEXT PRIMARY KEY NOT NULL,
    facet TEXT NOT NULL,
    axis TEXT NOT NULL,
    status TEXT NOT NULL,
    subject_count INTEGER NOT NULL DEFAULT 0,
    submitted_count INTEGER NOT NULL DEFAULT 0,
    generated_at TEXT,
    error_code TEXT,
    error_text TEXT,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_discovery_library_graph_runs_facet_status
    ON discovery_library_graph_runs (facet, status, completed_at);

-- title_ids_json lists the owned titles that share this subject, so removing
-- one of them only drops the node when no other title still stands for it.
CREATE TABLE discovery_library_graph_nodes (
    run_id TEXT NOT NULL REFERENCES discovery_library_graph_runs(id) ON DELETE CASCADE,
    subject_key TEXT NOT NULL,
    target_key TEXT NOT NULL,
    cluster_id INTEGER NOT NULL,
    title_ids_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (run_id, subject_key)
);

CREATE INDEX idx_discovery_library_graph_nodes_target
    ON discovery_library_graph_nodes (run_id, target_key);

-- Edges are undirected and stored once, with from_key < to_key; both are
-- target keys.
CREATE TABLE discovery_library_graph_edges (
    run_id TEXT NOT NULL REFERENCES discovery_library_graph_runs(id) ON DELETE CASCADE,
    from_key TEXT NOT NULL,
    to_key TEXT NOT NULL,
    weight REAL NOT NULL,
    kinds_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (run_id, from_key, to_key)
);

CREATE INDEX idx_discovery_library_graph_edges_to
    ON discovery_library_graph_edges (run_id, to_key);

CREATE TABLE discovery_library_graph_clusters (
    run_id TEXT NOT NULL REFERENCES discovery_library_graph_runs(id) ON DELETE CASCADE,
    cluster_id INTEGER NOT NULL,
    label_key TEXT,
    label TEXT NOT NULL,
    medoid TEXT NOT NULL,
    size INTEGER NOT NULL,
    PRIMARY KEY (run_id, cluster_id)
);
