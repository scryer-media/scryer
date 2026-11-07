-- A library graph run remembers how long the gateway asked Scryer to wait
-- before trying a failed build again, when it said, and the subject keys the
-- build left out because the facet was over the per-call cap, so a reader can
-- tell whether any of its own titles are missing from the stored graph.
--
-- A node's title_ids_json lists the owned titles that share its subject, so a
-- reader shows the node when any one of them is visible to it.
ALTER TABLE discovery_library_graph_runs ADD COLUMN retry_after_seconds INTEGER;

ALTER TABLE discovery_library_graph_runs
    ADD COLUMN omitted_subject_keys_json TEXT NOT NULL DEFAULT '[]';

-- Retry and cleanup look runs up by when they started, including runs that
-- never completed.
DROP INDEX idx_discovery_library_graph_runs_facet_status;

CREATE INDEX idx_discovery_library_graph_runs_facet_status_started
    ON discovery_library_graph_runs (facet, status, started_at);
