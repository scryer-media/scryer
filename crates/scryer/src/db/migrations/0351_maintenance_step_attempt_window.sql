-- The monitoring-conflict check reads which rules recently changed which
-- titles from the step attempts started inside a recent window, so the
-- read goes by start time.

CREATE INDEX IF NOT EXISTS idx_maintenance_action_step_attempts_started
    ON maintenance_action_step_attempts(started_at);
