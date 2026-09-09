# Scryer 0.19.15 release notes

## Highlights

- Upgrading from 0.19.13 or earlier no longer floods the log. The durable-cleanup backfill introduced in 0.19.14 seeds a cleanup row for every historical terminal download, and each legacy row spent three attempts failing the same way before it settled, logging a warning per attempt plus a settle line. A large library produced thousands of warn and info lines in the first minutes after upgrade. Legacy rows now settle on their first pass at debug level, and each reconcile pass logs one summary line with the settled outcomes by kind and the number still pending.
- Cleanup settles what a retry cannot change. When the download client reports a job absent and there is no payload cleanup left to verify, the row settles immediately rather than after the retry budget. A row whose host cleanup was actually planned still logs one warning that the payload may remain on disk. Rows that can never be attributed to a title are abandoned on their first attempt; manually importing such a download attributes it and reopens its cleanup so the client entry and payload are handled.
- A download bound to a client without an exact job lookup can now be proven absent behind the first history page. The absence check follows the client's history cursor for up to eight pages before releasing the binding, so a stale grab on an older Weaver or a SABnzbd-compatible backend that ignores `nzo_ids` no longer blocks a replacement download indefinitely.

## Included fixes

- Weaver cleanup no longer stalls when the API key is refused file deletion. Current Weaver builds answer a `deleteFiles` request from an integration-scoped key with `admin scope required to delete completed files`, which left the history entry and payload in place and logged a warning on every retry. Scryer now deletes the payload itself from the shared download directory, using the same verified per-file deletion it applies for NZBGet, and then removes the history entry without data. The refusal is remembered on the cleanup row, so later attempts go straight to host deletion instead of asking Weaver again.
- Policy-retained, recovered-absence and unresolved-client cleanup outcomes log at debug level. A cleanup settled because its download client was removed remains at info.
- A cleanup that keeps failing warns on its first failure and then once per hour of backoff instead of on every attempt.

## Upgrading

No migration. Installs already on 0.19.14 have finished the backfill and see only the quieter steady-state logging; installs upgrading from earlier versions complete the backfill in a few minutes with one summary line per pass.
