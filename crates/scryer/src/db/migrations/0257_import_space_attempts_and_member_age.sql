-- When each blocked member was last observed, so reconciliation can age out
-- members nothing else will ever retire. Rows from 0256 start at '' (oldest).
ALTER TABLE import_space_members ADD COLUMN observed_at TEXT NOT NULL DEFAULT '';

-- Failed delivery attempts per disk-space notification event, so one broken
-- channel cannot stall every later notification forever.
CREATE TABLE import_space_notification_attempts (
    event_id TEXT NOT NULL PRIMARY KEY,
    attempts INTEGER NOT NULL,
    first_attempt_at TEXT NOT NULL
);
