-- Two large JSON payloads leave the middle of wide tables for keyed tables of
-- their own. Every update of an owner row rewrote the payload, and every read
-- of a column after it walked the payload's overflow pages: a location
-- operation's confirmed plan (megabytes for a large transfer, behind the
-- cancel flag and progress counters that are polled and updated constantly),
-- and a media file's analysis (a few kilobytes, so nearly every row
-- overflowed a page).
--
-- New values are zstd frames. SQL cannot compress, so existing values are
-- copied as their plain UTF-8 bytes; readers accept both forms. A payload row
-- goes with its owner through the foreign key.
--
-- A completed operation is never resumed, retried or reopened, so its plan is
-- not carried over, the same as completing one drops it from now on. One that
-- names no single source library keeps it: its permission check reads the
-- libraries from the plan.

CREATE TABLE location_operation_plans (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES location_operations(id) ON DELETE CASCADE,
    plan BLOB NOT NULL
);

INSERT INTO location_operation_plans (operation_id, plan)
SELECT id, CAST(plan_json AS BLOB)
  FROM location_operations
 WHERE plan_json IS NOT NULL
   AND NOT (state IN ('completed', 'completed_with_warnings')
            AND source_library_id IS NOT NULL);

ALTER TABLE location_operations DROP COLUMN plan_json;

CREATE TABLE media_file_analyses (
    media_file_id TEXT PRIMARY KEY NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    analysis BLOB NOT NULL
);

-- A blank analysis already read as the empty default, which is what an
-- absent row reads as.
INSERT INTO media_file_analyses (media_file_id, analysis)
SELECT id, CAST(analysis_json AS BLOB)
  FROM media_files
 WHERE analysis_json IS NOT NULL
   AND trim(analysis_json) <> '';

ALTER TABLE media_files DROP COLUMN analysis_json;
