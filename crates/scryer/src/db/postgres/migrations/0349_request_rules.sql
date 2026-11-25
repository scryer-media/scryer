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
-- a JSON array, and a vote with no rule set id, add nothing. The whole table
-- is read in one statement; only if a stored document cannot be read as
-- jsonb is it read again one decision at a time, skipping that decision. A
-- text value fails the cast as invalid_text_representation when it is not
-- JSON, untranslatable_character when it holds a \u0000 escape or one the
-- database encoding cannot hold, numeric_value_out_of_range when a number is
-- too large for numeric, statement_too_complex when it nests too deeply, and
-- program_limit_exceeded when it is larger than jsonb can store.
DO $$
DECLARE
    stored record;
BEGIN
    BEGIN
        INSERT INTO request_rule_decision_rule_sets (decision_id, rule_set_id)
        SELECT decision.id, vote.value ->> 'rule_set_id'
        FROM request_rule_decisions AS decision
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(decision.votes_json::jsonb) = 'array'
                    THEN decision.votes_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS vote(value)
        WHERE jsonb_typeof(vote.value -> 'rule_set_id') = 'string'
          AND vote.value ->> 'rule_set_id' <> ''
        ON CONFLICT DO NOTHING;
    EXCEPTION
        WHEN invalid_text_representation OR untranslatable_character
            OR numeric_value_out_of_range OR statement_too_complex
            OR program_limit_exceeded THEN
        FOR stored IN SELECT id, votes_json FROM request_rule_decisions LOOP
            BEGIN
                INSERT INTO request_rule_decision_rule_sets (decision_id, rule_set_id)
                SELECT stored.id, vote.value ->> 'rule_set_id'
                FROM jsonb_array_elements(
                    CASE
                        WHEN jsonb_typeof(stored.votes_json::jsonb) = 'array'
                            THEN stored.votes_json::jsonb
                        ELSE '[]'::jsonb
                    END
                ) AS vote(value)
                WHERE jsonb_typeof(vote.value -> 'rule_set_id') = 'string'
                  AND vote.value ->> 'rule_set_id' <> ''
                ON CONFLICT DO NOTHING;
            EXCEPTION
                WHEN invalid_text_representation OR untranslatable_character
                    OR numeric_value_out_of_range OR statement_too_complex
                    OR program_limit_exceeded THEN
                NULL;
            END;
        END LOOP;
    END;
END
$$;
