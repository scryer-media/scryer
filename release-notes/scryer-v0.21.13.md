# Scryer 0.21.13 release notes

These notes cover what's changed since **0.21.12**.

## Highlights

- **Raw indexer searches behave like Prowlarr's manual search.** Typing a query on the Indexers page with no category selected no longer sends a hidden movie category to Prowlarr-proxied indexers, which made TV, anime and general torrent trackers return nothing. Only the categories you pick are sent, indexers that only accept anime or series text queries are asked too, and results are no longer dropped when their parsed title differs from what you typed. Indexers that could not be searched (no text search, capabilities unknown, or in backoff) are now marked as skipped with the reason shown on hover instead of appearing as "0 results". Duplicate detection only removes exact duplicates (same GUID or download URL) rather than merging distinct releases that happen to share a group and quality, per-indexer counts reflect what is listed, and raw searches default to 200 results shown newest first.
- **Grab logging is quiet again at the default level.** The per-stage lines that 0.21.12 added to trace a stalled grab submission are now logged at debug rather than info, so ordinary operation no longer writes a dozen lines for every grab. They remain available by raising the log filter for the `scryer_application::acquisition::submission`, `scryer_application::catalog::workflow` and `scryer_infrastructure_acquisition::downloads::clients` modules.

## Included fixes

- **Dashboard:** free space below 1 TB is shown in gigabytes instead of a fraction of a terabyte.
- **Release validation:** a test fixture for outbound HTTP cooldown handling closed its connections without saying so, which could make a later request in the same test land on a stale pooled connection and fail intermittently. The fixture now marks each response `Connection: close`.
