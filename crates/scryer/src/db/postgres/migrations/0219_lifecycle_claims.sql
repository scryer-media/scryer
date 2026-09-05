-- PostgreSQL twin of migrations/0219_lifecycle_claims.sql.
-- Title leases and keep claims (spec 0003 FR-041..FR-044, plan 0003 section 5).
--
-- No foreign key to `titles`: claims are history, release is an explicit state
-- transition rather than a cascade, and a dormant claim legitimately names a
-- title that does not exist yet at the instant the approval writes it.
CREATE TABLE lifecycle_claims (
    id text PRIMARY KEY NOT NULL,
    title_id text NOT NULL,
    library_id text NOT NULL DEFAULT '',
    producer text NOT NULL,
    producer_ref text,
    kind text NOT NULL DEFAULT 'retain_until',
    state text NOT NULL DEFAULT 'dormant',
    duration_days bigint,
    starts_at timestamptz,
    expires_at timestamptz,
    created_by text,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW(),
    released_reason text
);

-- One live claim per producing request; the live states are listed literally so
-- the predicate matches the SQLite twin and `LIFECYCLE_CLAIM_LIVE_STATES`.
CREATE UNIQUE INDEX idx_lifecycle_claims_live_producer
    ON lifecycle_claims(producer, producer_ref)
    WHERE state IN ('dormant', 'active') AND producer_ref IS NOT NULL;

CREATE INDEX idx_lifecycle_claims_title_state
    ON lifecycle_claims(title_id, state);
