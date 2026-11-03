-- A library graph is the gateway's similarity graph over one facet's owned
-- titles. Each build is a run; its nodes, edges and clusters hang off the run
-- id so a new build is written beside the previous one and the previous one
-- is deleted after the new one lands. Runs that failed keep their error code
-- and text and carry no children.
CREATE TABLE discovery_library_graph_runs (
    id text PRIMARY KEY NOT NULL,
    facet text NOT NULL,
    axis text NOT NULL,
    status text NOT NULL,
    subject_count bigint NOT NULL DEFAULT 0,
    submitted_count bigint NOT NULL DEFAULT 0,
    generated_at text,
    error_code text,
    error_text text,
    started_at timestamp with time zone NOT NULL,
    completed_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE INDEX idx_discovery_library_graph_runs_facet_status
    ON discovery_library_graph_runs (facet, status, completed_at);

-- title_ids_json lists the owned titles that share this subject, so removing
-- one of them only drops the node when no other title still stands for it.
CREATE TABLE discovery_library_graph_nodes (
    run_id text NOT NULL REFERENCES discovery_library_graph_runs(id) ON DELETE CASCADE,
    subject_key text NOT NULL,
    target_key text NOT NULL,
    cluster_id bigint NOT NULL,
    title_ids_json text NOT NULL DEFAULT '[]',
    PRIMARY KEY (run_id, subject_key)
);

CREATE INDEX idx_discovery_library_graph_nodes_target
    ON discovery_library_graph_nodes (run_id, target_key);

-- Edges are undirected and stored once, with from_key < to_key; both are
-- target keys.
CREATE TABLE discovery_library_graph_edges (
    run_id text NOT NULL REFERENCES discovery_library_graph_runs(id) ON DELETE CASCADE,
    from_key text NOT NULL,
    to_key text NOT NULL,
    weight double precision NOT NULL,
    kinds_json text NOT NULL DEFAULT '[]',
    PRIMARY KEY (run_id, from_key, to_key)
);

CREATE INDEX idx_discovery_library_graph_edges_to
    ON discovery_library_graph_edges (run_id, to_key);

CREATE TABLE discovery_library_graph_clusters (
    run_id text NOT NULL REFERENCES discovery_library_graph_runs(id) ON DELETE CASCADE,
    cluster_id bigint NOT NULL,
    label_key text,
    label text NOT NULL,
    medoid text NOT NULL,
    size bigint NOT NULL,
    PRIMARY KEY (run_id, cluster_id)
);
