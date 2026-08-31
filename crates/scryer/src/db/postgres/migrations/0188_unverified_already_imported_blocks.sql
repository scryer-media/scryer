-- PostgreSQL twin of migrations/0188_unverified_already_imported_blocks.sql.
-- Classify legacy status-only blocks once so runtime control flow reads the
-- durable reason instead of user-facing text.
UPDATE download_identity_states
SET reason = 'unverified_already_imported'
WHERE tracked_state = 'import_blocked'
  AND (reason IS NULL OR btrim(reason) = '')
  AND lower(btrim(COALESCE(detail, ''))) = 'import blocked: already_imported';
