DELETE FROM domain_events
WHERE event_type = 'discovery_search_completed'
   OR (
       event_type IN ('acquisition_search_completed', 'acquisition_candidate_rejected')
       AND occurred_at <= now() - interval '24 hours'
   );

DROP INDEX IF EXISTS idx_domain_events_stream_sequence;

ALTER TABLE domain_events
    ADD COLUMN import_status text,
    ADD COLUMN media_file_delete_reason text,
    ADD COLUMN download_id text;

ALTER TABLE domain_events
    ALTER COLUMN payload_json TYPE bytea
    USING convert_to(payload_json::text, 'UTF8');

ALTER TABLE release_decisions
    ALTER COLUMN explanation_json TYPE bytea
    USING CASE
        WHEN explanation_json IS NULL THEN NULL
        ELSE convert_to(explanation_json::text, 'UTF8')
    END;
