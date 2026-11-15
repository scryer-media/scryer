-- The actions a maintenance rule decided for one candidate, as a JSON
-- document, for rules whose source declares its own actions. NULL on
-- candidates written before rules declared their actions.
ALTER TABLE lifecycle_candidates ADD COLUMN decided_actions_json TEXT;
