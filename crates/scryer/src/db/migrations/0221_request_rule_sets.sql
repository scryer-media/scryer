-- Request rules (spec 0003 section 6): rule sets, their append-only matcher
-- revisions, and the durable trace of every evaluation.
--
-- Shaped after `maintenance_rule_sets` deliberately: the two families ride the
-- same policy core, so a reader who knows one table knows the other. What the
-- request family does not have is an action spec, a grace period, a subject
-- kind, or effect arming -- a request rule votes and nothing else, so there is
-- no blast radius to acknowledge.
CREATE TABLE request_rule_sets (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 0,
    evaluation_mode TEXT NOT NULL DEFAULT 'disabled',
    library_ids TEXT NOT NULL DEFAULT '[]',
    current_revision_number INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE request_rule_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    rule_set_id TEXT NOT NULL REFERENCES request_rule_sets(id) ON DELETE CASCADE,
    revision_number INTEGER NOT NULL,
    rego_source TEXT NOT NULL,
    matcher_content_hash TEXT NOT NULL,
    created_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (rule_set_id, revision_number)
);

CREATE INDEX idx_request_rule_revisions_rule_set
    ON request_rule_revisions(rule_set_id, revision_number DESC);

-- Every evaluation is recorded, shadow and enforce alike, which is why the
-- policy verdict and the verdict the instance acted on are separate columns:
-- in shadow they disagree on purpose.
--
-- `request_id` carries no foreign key to `media_requests`. Pre-flight evaluates
-- a draft the requester has not submitted yet (spec 0003 FR-020), so a decision
-- can legitimately precede -- or never acquire -- a request row. A constraint
-- here would force the pre-flight path to either invent a request or drop the
-- trace, and losing the trace is the one thing FR-016 forbids.
CREATE TABLE request_rule_decisions (
    id TEXT PRIMARY KEY NOT NULL,
    request_id TEXT NOT NULL,
    evaluated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    mode TEXT NOT NULL DEFAULT 'disabled',
    effective_outcome TEXT NOT NULL DEFAULT 'manual_review',
    policy_outcome TEXT NOT NULL DEFAULT 'manual_review',
    fallback_reason TEXT,
    votes_json TEXT NOT NULL DEFAULT '[]',
    tags_json TEXT NOT NULL DEFAULT '[]',
    input_hash TEXT NOT NULL DEFAULT '',
    input_schema_version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX idx_request_rule_decisions_request
    ON request_rule_decisions(request_id, evaluated_at DESC);
