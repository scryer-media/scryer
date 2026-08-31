-- Historical status-only blocks never ran import verification, but their
-- only durable discriminator was the user-facing detail text. Classify that
-- exact legacy state once so runtime control flow reads `reason` exclusively.
UPDATE download_identity_states
SET reason = 'unverified_already_imported'
WHERE tracked_state = 'import_blocked'
  AND (reason IS NULL OR trim(reason) = '')
  AND lower(trim(COALESCE(detail, ''))) = 'import blocked: already_imported';
