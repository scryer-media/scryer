-- Two large JSON payloads leave the middle of wide tables for keyed tables of
-- their own: a location operation's confirmed plan and a media file's
-- analysis. New values are zstd frames. SQL cannot compress, so existing
-- values are copied as their plain UTF-8 bytes; readers accept both forms. A
-- payload row goes with its owner through the foreign key.
--
-- A completed operation is never resumed, retried or reopened, so its plan is
-- not carried over, the same as completing one drops it from now on. One that
-- names no single source library keeps it: its permission check reads the
-- libraries from the plan.

CREATE TABLE location_operation_plans (
    operation_id text PRIMARY KEY NOT NULL REFERENCES location_operations(id) ON DELETE CASCADE,
    plan bytea NOT NULL
);

INSERT INTO location_operation_plans (operation_id, plan)
SELECT id, convert_to(plan_json, 'UTF8')
  FROM location_operations
 WHERE plan_json IS NOT NULL
   AND NOT (state IN ('completed', 'completed_with_warnings')
            AND source_library_id IS NOT NULL);

ALTER TABLE location_operations DROP COLUMN plan_json;

CREATE TABLE media_file_analyses (
    media_file_id text PRIMARY KEY NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    analysis bytea NOT NULL
);

-- A blank analysis already read as the empty default, which is what an
-- absent row reads as.
INSERT INTO media_file_analyses (media_file_id, analysis)
SELECT id, convert_to(analysis_json, 'UTF8')
  FROM media_files
 WHERE analysis_json IS NOT NULL
   AND btrim(analysis_json) <> '';

ALTER TABLE media_files DROP COLUMN analysis_json;
