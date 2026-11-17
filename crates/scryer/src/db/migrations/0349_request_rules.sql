-- Request rules gain action heads: shaping heads that change what an approval
-- writes, add_tags and notify. Rule sources written for the earlier shape (a
-- `tags` rule, no shaping) are not carried over: every request rule set is
-- removed with its revisions and library scopes. Every request rule set is
-- operator-authored; none is seeded by Scryer.
--
-- Nothing else is removed. Media requests, request decisions and their
-- history, leases, lifecycle claims, titles, tags, files, events and run logs
-- stay. Only request_rule_revisions and request_rule_set_libraries hold a
-- foreign key to a request rule set. A decision keeps the rule set's id and
-- name inside votes_json, and a media request keeps the deciding ids in
-- decided_by_rule_set_ids, both as text, exactly as deleting a rule set by
-- hand leaves them. Children go first, so the result is the same with
-- foreign keys off.

-- The shaping the rules set on one decision, with the rule set behind each
-- value. NULL when no rule set any.
ALTER TABLE request_rule_decisions ADD COLUMN shaping_json TEXT;

-- The rule builder's own description of a rule, stored as the client sent
-- it, on a request rule set and on a scoring rule. NULL when the rule was not
-- written with the builder; such a rule opens as Rego.
ALTER TABLE request_rule_sets ADD COLUMN builder_model_json TEXT;
ALTER TABLE rule_sets ADD COLUMN builder_model_json TEXT;

DELETE FROM request_rule_set_libraries;
DELETE FROM request_rule_revisions;
DELETE FROM request_rule_sets;

-- The content hash is taken over the whole rule source.
ALTER TABLE request_rule_revisions RENAME COLUMN matcher_content_hash TO source_content_hash;

-- A request rule set is on or off, and a decision is always enforced, so
-- neither an evaluation mode nor a decision mode is stored.
ALTER TABLE request_rule_sets DROP COLUMN evaluation_mode;
ALTER TABLE request_rule_decisions DROP COLUMN mode;

-- Which rule sets voted on each decision, so the decisions a rule set took
-- part in are counted from an index rather than by scanning every stored
-- vote. A row goes with its decision.
CREATE TABLE request_rule_decision_rule_sets (
    decision_id TEXT NOT NULL REFERENCES request_rule_decisions(id) ON DELETE CASCADE,
    rule_set_id TEXT NOT NULL,
    PRIMARY KEY (decision_id, rule_set_id)
);

CREATE INDEX idx_request_rule_decision_rule_sets_rule_set
    ON request_rule_decision_rule_sets(rule_set_id, decision_id);

-- Filled from every stored decision's votes. A decision whose votes are not
-- a JSON array, and a vote with no rule set id, add nothing.
INSERT OR IGNORE INTO request_rule_decision_rule_sets (decision_id, rule_set_id)
SELECT decision.id, json_extract(vote.value, '$.rule_set_id')
FROM request_rule_decisions AS decision
JOIN json_each(
    CASE
        WHEN json_valid(decision.votes_json) AND json_type(decision.votes_json) = 'array'
            THEN decision.votes_json
        ELSE '[]'
    END
) AS vote
WHERE vote.type = 'object'
  AND json_type(vote.value, '$.rule_set_id') = 'text'
  AND json_extract(vote.value, '$.rule_set_id') <> '';
