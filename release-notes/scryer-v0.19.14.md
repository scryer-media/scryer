# Scryer 0.19.14 release notes

## Highlights

- A download that Scryer cannot see is no longer treated as gone. Every "is the previous grab still there?" check now fails closed: an unreadable, disabled or ambiguous download client keeps the existing claim, and only the client's own exact observation of the job can retire it. This closes the duplicate-grab report in issue #204, where a momentary client outage let a second copy of the same release be queued.
- Post-import and post-failure cleanup is now durable. Each terminal download gets a persisted cleanup row with a lease, retry backoff and a recorded outcome, so a client entry that could not be removed at the time (client offline, delete rejected, Scryer restarted mid-cleanup) is retried until it settles instead of being forgotten. This closes issue #205, where imported Usenet jobs stayed in the client's history forever.
- Host-side payload deletion for clients that cannot delete their own data (NZBGet, SABnzbd, rTorrent, Download Station, aria2) no longer needs an operator-configured remote path mapping to run, and no longer performs a recursive delete. Scryer inventories the job directory first, records what it expects to remove, then deletes only files that still match their recorded identity and content sample, and only when the directory is named for the release Scryer grabbed. Unknown files, files that changed since the inventory, symlinks and anything reachable through a symlinked directory are left in place along with their directory. A directory that is not provably the job's own (a shared completed folder or a client-renamed duplicate) loses only the files Scryer's own import record proves it imported. Library and recycle roots are refused outright.
- Cleanup can no longer retry forever. After three failed host deletions the client entry is removed anyway with the payload left on disk and the outcome recorded as unverified, matching Sonarr's best-effort data deletion. Rows whose client entry vanished before the payload was verified, whose client configuration was deleted, or that can never be attributed to a title settle with an explicit outcome after the same budget, so the scope they held is released and the log carries one line instead of one every five minutes.
- Jobs bound to a download client that has since been deleted are reconciled as absent instead of holding their scope indefinitely, both in the background search planner and at grab admission.

## Included fixes

- Legacy submissions recorded before per-download identity states existed still expose their tracked state, so a settled legacy job that is absent from the client no longer freezes its scope.
- An imported job that has not yet reached the catalog holds an empty scope only while its client is unreadable; once the client answers and the entry is simply gone, the scope is free to search again.
- SABnzbd cleanup deletes the job through the client API with `del_files` as before; Scryer's own verified deletion runs first and is never repeated on a retry. NZBGet history is read in full on each poll rather than a recent window, so an old imported entry is not missed.
- Failed torrents are retained unless the client reports they can be removed, and torrent import in Move mode degrades to a copy when the client is unreachable rather than failing the import.
- Weaver versions before 0.1.8 reject `deleteFiles`; the error is surfaced rather than treated as a successful cleanup.
- The SABnzbd queue guard on `noofslots_total` is unchanged. SAB-compatible backends that omit the field (nzbdav) are unaffected, and Scryer does not send `limit=0`, which nzbdav interprets as an empty queue.

## Upgrading

Migration 0211 adds the `download_cleanup` table on SQLite and PostgreSQL. Existing terminal downloads are seeded into it in bounded batches on startup and retried from there; no manual action is required. Payloads that cleanup leaves behind as partially retained or unverified are reported in the log with the client, job and retained paths so they can be swept by hand.
