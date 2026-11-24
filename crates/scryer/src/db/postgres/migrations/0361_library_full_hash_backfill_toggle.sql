-- Whether the background full-hash backfill reads this library's files. Off
-- keeps the job away from the library's disks entirely; a file there is
-- hashed only when something needs its hash. Existing libraries keep the job.
ALTER TABLE libraries ADD COLUMN full_hash_backfill_enabled BOOLEAN NOT NULL DEFAULT TRUE;
