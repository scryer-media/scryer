-- Scripts can now be started by a schedule as well as by the import pipeline.
-- `language` picks the interpreter for inline content, `trigger` says what
-- starts the script, `schedule_json` holds the schedule for scheduled scripts
-- and `run_on_startup` also runs a scheduled script once at host start.
-- Rows written before this migration are inline shell scripts run after
-- import, which is what the defaults describe, so they keep working unchanged.
ALTER TABLE post_processing_scripts ADD COLUMN language text NOT NULL DEFAULT 'shell';
ALTER TABLE post_processing_scripts ADD COLUMN trigger text NOT NULL DEFAULT 'post_import';
ALTER TABLE post_processing_scripts ADD COLUMN schedule_json text;
ALTER TABLE post_processing_scripts ADD COLUMN run_on_startup boolean NOT NULL DEFAULT false;
