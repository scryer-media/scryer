-- Older builds wrote a decision trace for every pre-flight preview, under a
-- request id that starts with 'preflight:'. Pre-flight no longer writes one,
-- and housekeeping removes the old rows in batches. This partial index lets
-- each batch find them without scanning the whole trace table, and lets a
-- pass with nothing left to remove finish on an empty index.
--
-- The predicate is a literal so SQLite can use the partial index; the
-- housekeeping query spells it identically.
CREATE INDEX IF NOT EXISTS idx_request_rule_decisions_preflight
    ON request_rule_decisions (id)
    WHERE request_id LIKE 'preflight:%';
