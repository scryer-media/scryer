# Scryer 0.21.14 release notes

These notes cover what's changed since **0.21.12**.

## Highlights

- **The Indexers page search is now a plain query to your indexers, like Prowlarr's manual search.** The query is sent to every enabled indexer exactly as typed, with only the categories you pick; no hidden movie category, no quality profile, no indexer routing or priority, and no title filter decide what you see. That fixes TV, anime and general torrent trackers behind Prowlarr returning nothing, and results no longer disappear because their parsed title differed from what you typed. Indexers that could not be asked (no text search, disabled, or in backoff) are marked as skipped with the reason shown on hover instead of appearing as "0 results". The limit is now a per-indexer page size (default 100, up to 500) rather than a cap on the merged list, exact duplicates (same GUID or download URL) are removed, and results are listed newest first. Release-name parsing and the result filters are unchanged.
- **Grab logging is quiet again at the default level.** The per-stage lines that 0.21.12 added to trace a stalled grab submission are now logged at debug rather than info, so ordinary operation no longer writes a dozen lines for every grab. They remain available by raising the log filter for the `scryer_application::acquisition::submission`, `scryer_application::catalog::workflow` and `scryer_infrastructure_acquisition::downloads::clients` modules.

## Included fixes

- **Imports on sshfs, FUSE, CIFS and mergerfs path-hash volumes:** an import no longer fails with "import destination is not linked to the expected source" on filesystems that report a different inode for each path. When a hard link succeeds but its identity cannot be verified, Scryer removes the link and copies the file instead, logs one warning naming the volume, and skips the link attempt for later imports to that volume until restart. Files the archive extractor wrote into its own workspace are now renamed into place in every import mode, so they need no link and no copy at all. The strict check itself is unchanged: a verified mismatch of the source is still an error.
- **Dashboard:** free space below 1 TB is shown in gigabytes instead of a fraction of a terabyte.
- **Release validation:** a test fixture for outbound HTTP cooldown handling closed its connections without saying so, which could make a later request in the same test land on a stale pooled connection and fail intermittently. The fixture now marks each response `Connection: close`.
