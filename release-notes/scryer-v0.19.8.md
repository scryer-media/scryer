# scryer-v0.19.8

AI generated release notes

## Highlights

- Jellyfin plugin OAuth setup is now smoother and more reliable. The web UI can guide standalone plugin client creation, the authorization screen validates the callback and requested scopes before approval, and Jellyfin account-link grants are bound more tightly to the approved caller.
- Media requests now persist a metadata snapshot at submission time, including overview text, combined ratings, and provider-specific ratings. The requests view now surfaces those captured ratings directly.
- Web updates are more resilient after upgrades. The UI now recovers better from stale cached Vite chunks and language bundles, and outdated update reminders no longer linger after the app version changes.
- Search routing is more predictable in routed setups. Per-search indexer restrictions no longer interfere with scope routing, improving interactive release search compatibility.
- rTorrent cleanup is safer and more reliable. Scryer now blocks symlink escapes, respects safe cleanup roots, removes payloads before deleting client entries, and keeps cleanup failures retryable instead of leaving them in a broken final state.

## Additional fixes

- Recovered Jellyfin OAuth setup when client creation results are uncertain.
- Applied follow-up polish to the OAuth authorization page and related web flows.
- Includes the schema and database changes required for Jellyfin OAuth link binding and persisted media-request metadata snapshots.