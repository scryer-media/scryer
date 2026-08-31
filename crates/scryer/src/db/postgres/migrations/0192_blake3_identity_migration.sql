-- Plan 149 / WP0 + WP6: retire the legacy v1 password format and sweep
-- convergence coverage rows whose scope key embedded a SHA-256 digest.

-- WP0. `v1$<salt>$<sha256(salt+password)>` is no longer accepted. The online
-- re-hash on login is gone with it, so any surviving row is cleared and flagged
-- for reset: the account fails closed with an explicit reason and an
-- administrator re-issues a password. Expected to affect zero rows in practice —
-- the online migration upgraded every account that logged in since it shipped.
UPDATE users
   SET password_hash = NULL,
       password_change_required = true
 WHERE password_hash LIKE 'v1$%';

-- WP6. `episode_set:` and `series_pack_set:` convergence scope keys embed their
-- digest, so the BLAKE3 switch produces a different primary key rather than a
-- stale value the upsert would overwrite. The old rows are unreachable and the
-- table has no TTL, so they are removed here. New-form keys carry `:b3:` and are
-- left alone, which makes this idempotent and safe to re-run.
DELETE FROM scope_indexer_coverage
 WHERE (scope_key LIKE 'episode_set:%' OR scope_key LIKE 'series_pack_set:%')
   AND scope_key NOT LIKE 'episode_set:b3:%'
   AND scope_key NOT LIKE 'series_pack_set:b3:%';
