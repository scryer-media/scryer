ALTER TABLE indexers ADD COLUMN last_error_code TEXT;
ALTER TABLE rule_sets ADD COLUMN disabled_reason_code TEXT;

-- The sole compatibility conversion for values persisted before reason codes.
UPDATE indexers SET last_error_code = 'INDEXER_CAPS_REFRESH_FAILED'
WHERE substr(last_error_message, 1, 20) = 'caps refresh failed:';
UPDATE rule_sets SET disabled_reason_code = 'RULE_INPUT_RETIRED'
WHERE replace(disabled_reason, 'uses retired input.release.guide_facts; update the rule source before enabling it.', '') <> disabled_reason;
