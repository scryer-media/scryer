-- What a lifecycle rule decided, in the same evaluation, for a subject
-- enclosing or inside this candidate's subject that cannot be applied
-- together with this candidate's own actions, as a JSON document. While it
-- is set the candidate is held with actions_conflict. NULL when there is none.
ALTER TABLE lifecycle_candidates ADD COLUMN actions_conflict_json TEXT;
