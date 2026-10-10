-- Scheduled scripts share one job key and are told apart by operation type,
-- so their per-script run lists and latest-run lookups read by
-- (job_key, operation_type), newest first.
CREATE INDEX IF NOT EXISTS idx_workflow_operations_job_operation_started
    ON workflow_operations (job_key, operation_type, started_at DESC);
