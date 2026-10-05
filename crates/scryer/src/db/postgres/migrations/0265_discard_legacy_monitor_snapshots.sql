-- Legacy snapshots may already have overwritten later monitoring choices.
-- Discard pending, source-session and consumed chunks before the first scan;
-- only snapshots created by a new migration attempt may apply after upgrade.
DELETE FROM external_import_monitor_snapshot_chunks;
