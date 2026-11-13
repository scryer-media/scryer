-- Indexes for the retention pass over maintenance action runs. The pass
-- selects held runs by finish time and keeps the newest run of each
-- candidate generation, which it finds by looking for a newer run in the same
-- generation. Adding an index removes and changes no rows.

-- Held runs past the retention cutoff.
CREATE INDEX IF NOT EXISTS idx_lifecycle_action_runs_status_finished
    ON lifecycle_action_runs(status, finished_at);

-- Runs of one candidate generation in start order.
CREATE INDEX IF NOT EXISTS idx_lifecycle_action_runs_generation_started
    ON lifecycle_action_runs(candidate_id, match_generation, started_at, id);
