-- Policy provenance and the requested/approved lease on a media request
-- (spec 0003 FR-030, FR-040, FR-050).
--
-- `resolved_by_user_id` is deliberately not touched. The plan flagged it as a
-- possible NOT NULL that a policy denial would have to work around, but it is
-- already nullable in both dialects: the 0206 SQLite rebuild declares it
-- `resolved_by_user_id TEXT` with `ON DELETE SET NULL`, and the 0125 Postgres
-- rollup adds it as a plain nullable `text` column. A rule-driven denial can
-- therefore leave it NULL without inventing a system user, and rebuilding the
-- table to "relax" a column that is already relaxed would be churn.
ALTER TABLE media_requests
    ADD COLUMN requested_lease_days INTEGER;

ALTER TABLE media_requests
    ADD COLUMN approved_lease_days INTEGER;

ALTER TABLE media_requests
    ADD COLUMN decision_id TEXT;

ALTER TABLE media_requests
    ADD COLUMN decided_by_rule_set_ids TEXT NOT NULL DEFAULT '[]';

ALTER TABLE media_requests
    ADD COLUMN policy_tags_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE media_requests
    ADD COLUMN metadata_snapshot_json TEXT NOT NULL DEFAULT '{}';
