-- PostgreSQL twin of migrations/0221_media_request_policy.sql.
-- Policy provenance and the requested/approved lease on a media request
-- (spec 0003 FR-030, FR-040, FR-050).
--
-- `resolved_by_user_id` is deliberately not touched: the 0125 rollup already
-- adds it as a nullable `text REFERENCES users(id) ON DELETE SET NULL`, so a
-- rule-driven denial can leave it NULL without inventing a system user.
ALTER TABLE media_requests
    ADD COLUMN requested_lease_days bigint,
    ADD COLUMN approved_lease_days bigint,
    ADD COLUMN decision_id text,
    ADD COLUMN decided_by_rule_set_ids text NOT NULL DEFAULT '[]',
    ADD COLUMN policy_tags_json text NOT NULL DEFAULT '[]',
    ADD COLUMN metadata_snapshot_json text NOT NULL DEFAULT '{}';
