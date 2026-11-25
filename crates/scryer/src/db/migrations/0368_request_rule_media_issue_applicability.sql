-- Request rules explicitly distinguish acquisition requests from media
-- issues. Existing rows and all pre-feature rule sets remain acquisition-only.
ALTER TABLE request_rule_sets
    ADD COLUMN applicability TEXT NOT NULL DEFAULT 'acquisition';

ALTER TABLE media_issues
    ADD COLUMN authorized_remediation TEXT
        CHECK (authorized_remediation IN ('mark_bad', 'recycle_and_search'));
ALTER TABLE media_issues
    ADD COLUMN rule_input_schema_version INTEGER;
ALTER TABLE media_issues
    ADD COLUMN rule_input_hash TEXT;
ALTER TABLE media_issues
    ADD COLUMN rule_snapshot_json TEXT;
ALTER TABLE media_issues
    ADD COLUMN rule_votes_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE media_issues
    ADD COLUMN rule_effective_outcome TEXT;
ALTER TABLE media_issues
    ADD COLUMN rule_policy_outcome TEXT;
ALTER TABLE media_issues
    ADD COLUMN rule_hold_reasons_json TEXT NOT NULL DEFAULT '[]';
