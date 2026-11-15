-- Request rules gain action heads: shaping heads that change what an approval
-- writes, add_tags and notify. Rule sources written for the earlier shape (a
-- `tags` rule, no shaping) are not carried over: every request rule set is
-- removed with its revisions and library scopes. Every request rule set is
-- operator-authored; none is seeded by Scryer.
--
-- Nothing else is touched. Media requests, request decisions and their
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

-- The rule builder's own description of a rule set, stored as the client
-- sent it. NULL when the rule set was not written with the builder.
ALTER TABLE request_rule_sets ADD COLUMN builder_model_json TEXT;

DELETE FROM request_rule_set_libraries;
DELETE FROM request_rule_revisions;
DELETE FROM request_rule_sets;
