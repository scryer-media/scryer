-- A library graph's nodes, edges and clusters move from one row per item to
-- one compressed blob per run. Every read of a graph loads it whole, and the
-- per-item rows repeated the run id and text keys in every row and index.
--
-- The graph is derived from the library and rebuilt by the next discovery
-- sync, so stored graphs are dropped rather than converted. Failed runs stay:
-- they carry the gateway's retry wait, which decides when a facet is tried
-- again. Complete runs go with their graphs, so a facet without a graph is
-- built on the next sync.
DROP TABLE discovery_library_graph_edges;

DROP TABLE discovery_library_graph_nodes;

DROP TABLE discovery_library_graph_clusters;

DELETE FROM discovery_library_graph_runs WHERE status <> 'failed';

-- payload is a zstd frame holding the run's nodes, edges and clusters; the
-- layout is documented beside the code that writes it.
CREATE TABLE discovery_library_graph_payloads (
    run_id text PRIMARY KEY NOT NULL REFERENCES discovery_library_graph_runs(id) ON DELETE CASCADE,
    payload bytea NOT NULL
);
