# scryer-v0.19.4

AI generated release notes

Patch release focused on metadata recovery, managed indexer stability, download cleanup, and stricter signature verification.

## User-facing changes
- Movie metadata hydration is more reliable for movies whose primary match is not TVDB. Scryer now schedules background hydration for movie titles backed by SMG, TMDB, IMDb, or TVDB IDs, and retries existing idle movie titles after upgrade.
- Managed indexer routing now preserves entries you have disabled locally during managed syncs, preventing those routes from being re-enabled unexpectedly.
- Legacy download deletions now converge more safely, reducing stale download bindings after queue or history items are removed from connected download clients.
- Fixed missing description copy in the movie **Fix Match** flow in the web UI.

## Security and reliability
- Hardened Sigstore verification for signed plugin catalogs and application upgrades.
- Upgrade manifest verification is now pinned to the exact expected release tag, and mismatched workflow or tag identities are rejected.

## Maintenance
- Reduced the runtime dependency surface and refreshed release-signing plumbing and test coverage.