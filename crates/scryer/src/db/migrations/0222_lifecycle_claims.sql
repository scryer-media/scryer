-- Title leases and keep claims (spec 0003 FR-041..FR-044, plan 0003 section 5).
--
-- A claim is a hold on a title's lifecycle: an approved finite lease
-- ('retain_until'), a "forever" request or an operator pin ('keep'). Any claim
-- in a live state -- 'dormant' or 'active' -- blocks every destructive
-- maintenance action on its title.
--
-- No foreign key to `titles`. Claims are history: a released claim survives the
-- title it protected, and release is an explicit state transition rather than a
-- cascade, so nothing is ever destroyed silently (constitution C3). A dormant
-- claim also legitimately names a title that does not exist yet at the instant
-- the approval writes it.
CREATE TABLE lifecycle_claims (
    id TEXT PRIMARY KEY NOT NULL,
    title_id TEXT NOT NULL,
    library_id TEXT NOT NULL DEFAULT '',
    producer TEXT NOT NULL,
    producer_ref TEXT,
    kind TEXT NOT NULL DEFAULT 'retain_until',
    state TEXT NOT NULL DEFAULT 'dormant',
    duration_days INTEGER,
    starts_at TEXT,
    expires_at TEXT,
    created_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    released_reason TEXT
);

-- One live claim per producing request. The live states are listed rather than
-- negated through the terminal ones so the index predicate stays immutable,
-- which is what SQLite requires of a partial index; `LIFECYCLE_CLAIM_LIVE_STATES`
-- in `scryer-domain` is the same set in code. `producer_ref IS NOT NULL` keeps
-- operator pins -- which have nothing upstream -- out of the constraint, since
-- SQLite treats NULLs as distinct anyway and a partial index says so plainly.
CREATE UNIQUE INDEX idx_lifecycle_claims_live_producer
    ON lifecycle_claims(producer, producer_ref)
    WHERE state IN ('dormant', 'active') AND producer_ref IS NOT NULL;

CREATE INDEX idx_lifecycle_claims_title_state
    ON lifecycle_claims(title_id, state);
