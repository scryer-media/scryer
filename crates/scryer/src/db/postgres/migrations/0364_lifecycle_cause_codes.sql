-- The underlying code behind a lifecycle hold or failure, kept beside the
-- reason it explains: the location planner's refusal behind `move_refused`,
-- or the coded reason a failed step's error carried. Null when there is none.
ALTER TABLE lifecycle_candidates ADD COLUMN state_cause_code TEXT;
ALTER TABLE lifecycle_action_runs ADD COLUMN cause_code TEXT;
ALTER TABLE lifecycle_action_steps ADD COLUMN cause_code TEXT;
ALTER TABLE lifecycle_action_step_attempts ADD COLUMN cause_code TEXT;
