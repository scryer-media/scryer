DROP INDEX IF EXISTS idx_operations_status_time;
DROP INDEX IF EXISTS idx_workflow_operations_job_key_status;
DROP INDEX IF EXISTS idx_domain_events_facet_sequence;

DROP INDEX IF EXISTS idx_workflow_operations_actor_job_started;
CREATE INDEX idx_workflow_operations_actor_job_started
    ON workflow_operations (actor_user_id, job_key, started_at DESC)
    WHERE job_key IS NOT NULL
      AND actor_user_id IS NOT NULL;

DROP INDEX IF EXISTS idx_workflow_operations_actor_recent_started;
CREATE INDEX idx_workflow_operations_actor_recent_started
    ON workflow_operations (actor_user_id, started_at DESC)
    WHERE job_key IS NOT NULL
      AND actor_user_id IS NOT NULL;

DROP INDEX IF EXISTS idx_domain_events_title_sequence;
CREATE INDEX idx_domain_events_title_sequence
    ON domain_events (title_id, sequence DESC)
    WHERE title_id IS NOT NULL;
