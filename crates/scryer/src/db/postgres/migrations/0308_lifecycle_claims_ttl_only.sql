-- A lifecycle claim is only a time to delete. A request approved forever, or
-- with a zero-day lease, has no time to delete, so it has no claim: its keep
-- rows, and lease rows that never carried an expiry, are removed. A lease an
-- administrator extended to an explicit date keeps that date.
DELETE FROM lifecycle_claims
WHERE producer = 'request_permanent'
   OR kind = 'keep'
   OR (kind = 'retain_until'
       AND (duration_days IS NULL OR duration_days <= 0)
       AND expires_at IS NULL);
