-- Request rules explicitly distinguish acquisition requests from media
-- issues. Existing rows and all pre-feature rule sets remain acquisition-only.
ALTER TABLE request_rule_sets
    ADD COLUMN IF NOT EXISTS applicability text NOT NULL DEFAULT 'acquisition';

ALTER TABLE media_issues
    ADD COLUMN IF NOT EXISTS authorized_remediation text
        CHECK (authorized_remediation IN ('mark_bad', 'recycle_and_search'));
ALTER TABLE media_issues
    ADD COLUMN IF NOT EXISTS rule_input_schema_version bigint;
ALTER TABLE media_issues
    ADD COLUMN IF NOT EXISTS rule_input_hash text;
ALTER TABLE media_issues
    ADD COLUMN IF NOT EXISTS rule_snapshot_json text;
ALTER TABLE media_issues
    ADD COLUMN IF NOT EXISTS rule_votes_json text NOT NULL DEFAULT '[]';
ALTER TABLE media_issues
    ADD COLUMN IF NOT EXISTS rule_effective_outcome text;
ALTER TABLE media_issues
    ADD COLUMN IF NOT EXISTS rule_policy_outcome text;
ALTER TABLE media_issues
    ADD COLUMN IF NOT EXISTS rule_hold_reasons_json text NOT NULL DEFAULT '[]';
