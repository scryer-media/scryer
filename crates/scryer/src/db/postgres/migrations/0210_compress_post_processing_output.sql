-- PostgreSQL twin of migrations/0210_compress_post_processing_output_{pre,post}.sql.
-- The tails retype to bytea holding the legacy UTF-8 text verbatim; the Rust
-- hook `compress_post_processing_output` then rewrites each populated value as
-- a zstd frame (see scryer_infrastructure_sql::script_output).
ALTER TABLE post_processing_script_runs
    ALTER COLUMN stdout_tail TYPE bytea
    USING convert_to(stdout_tail, 'UTF8'),
    ALTER COLUMN stderr_tail TYPE bytea
    USING convert_to(stderr_tail, 'UTF8');
