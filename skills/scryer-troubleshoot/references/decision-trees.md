# Decision trees

`$S` = this skill's directory, `$B` = unpacked bundle. Operations below are
in `$S/operations/`; run each with
`sh "$G" "$S"/operations/<Name>.graphql [vars.json] > out.json`, read output
with jq. `log-search.sh` is `$S/scripts/log-search.sh`. Codes explained in
`codes.md`.

Any fix that changes Scryer's settings is a write: show the exact change and
wait for a yes, or hand over the UI path. Restarts, container changes, file
permission changes and queue removals are the operator's; describe them, do
not run them.

## Nothing downloads

1. `jq '.settings.acquisition' $B/environment.json` is evidence only. Values
   are internal, not operator-configurable; never suggest changing them. If
   `enabled` is false or a value looks abnormal, record it for a maintainer
   report (`scryer-report-bug`) and keep going.
2. `HealthChecks.graphql`: any `DownloadClient` or `Indexer` check at `error`
   explains it. No client configured, or all indexers failing, stops
   everything.
3. `jq '.indexers' $B/summary.json`: no enabled indexer, or
   `auto_search_enabled` 0 = automatic search has nothing to query.
4. `jq '.release_decisions.by_code' $B/summary.json`, top codes:
   - mostly `eligible` but no grabs: check queue and download clients
     (step 5).
   - `pending_delay`, `minimum_age`: releases waiting on a delay profile.
     `PendingReleases.graphql` shows `delayUntil` (convert to operator's local
     time).
   - `quality_blocked`, `upgrade_rejected`, `cutoff_reached`: profile is
     refusing; go to **Wrong release chosen**.
   - `download_client_unavailable`, `protocol_disabled`: no client accepts
     that protocol.
   - `acquisition_paused`, `episode_not_monitored`: monitoring or pause.
   - few rows at all: search not finding releases; go to **Search finds
     nothing**.
5. `DownloadClientStatus.graphql`: `status` not healthy, `lastError` set,
   `disabledUntil` in the future or `escalationLevel` above 0 = Scryer backed
   off that client. Verify client reachable from the Scryer host or
   container; connection test is in `scryer-configure-acquisition`.
6. `LatestJobRuns.graphql`: `RSS_SYNC`, `ACQUISITION_SEARCH` and
   `PENDING_RELEASE_PROCESSING` should have recent `COMPLETED` runs. `FAILED`
   run: read `JobRuns.graphql` for that key and its `errorText`.
7. Single title: `FindTitle.graphql`, then `TitleDecisions.graphql`.

## Downloads do not import

1. `DownloadQueuePage.graphql`, then
   `jq '[.data.downloadQueuePage.items[] | select(.attentionRequired)] | group_by(.importErrorCode) | map({code: .[0].importErrorCode, n: length})'`.
2. By code:
   - `FILE_NOT_FOUND`: client's output path not visible to Scryer. Check
     `jq '.container, .filesystems' $B/environment.json` and
     `with_remote_path_mappings` in `summary.download_clients`. In containers
     both services need the same host path mounted, or a remote path mapping
     (`REMOTE => LOCAL`) on the client. Health check "output path
     unresolvable" confirms it.
   - `PERMISSION_DENIED`: ownership or mount mode. Operator fixes it on the
     host.
   - `DISK_FULL`: `jq '.filesystems.roots' $B/environment.json`.
   - `EPISODE_NOT_FOUND`, `EPISODE_LOOKUP_FAILED`: metadata does not list the
     episode yet.
   - `POLICY_MISMATCH` or skip `POST_DOWNLOAD_RULE_BLOCKED`: a rule or the
     profile refused the file. Rules: `scryer-write-rules`.
   - `passwordFailureCode` set: archive needs a password.
   - `ARCHIVE_EXTRACTION_PLUGIN_REQUIRED`: no extraction plugin installed.
   - `ARCHIVE_EXTRACTION_WAITING`: an extraction failed; the next of its
     three attempts is not due yet. `ARCHIVE_EXTRACTION_BLOCKED`: all three
     failed. Operator retries the import from the queue item once the archive
     is fixed.
   - `attentionReasonCode` `IMPORT_RETRY_LIMIT_REACHED`: six passes in a row
     made no progress. Read the import's last `skipReason` and error (step 3)
     for the cause; operator retries from the queue item once it is fixed.
3. `ImportHistory.graphql` with limit 25, group by `decision` and
   `skipReason`. `UNRESOLVED_IDENTITY`, `UNPARSEABLE_EPISODE`: file name did
   not map to a title or episode. Operator resolves it from the queue item's
   import actions in the web UI.
4. Logs: `log-search.sh $B 'import' WARN`.
5. Hardlinks: `same_filesystem_pairs` empty while operator expects hardlinks
   = download and library roots on different filesystems. For files already
   imported, `TitleHistory.graphql` with `eventTypes: ["IMPORTED"]` shows
   `filePlacements` per file. `placement` `COPY` with `linkFallback.cause`
   `FAILED` = the hard link call failed and the file was copied; `reason` and
   `osErrorCode` carry the error, and the log has a WARN
   `hard link failed; handing import to copy lane` for it. `NOT_POSSIBLE` =
   the destination cannot hold a link (another volume or filesystem).
   Imports recorded before placements were kept have no `filePlacements`.

## Wrong release chosen

1. `FindTitle.graphql`, then `TitleDecisions.graphql`. Compare
   `candidateScore`, `currentScore` and `decidedBy` for the release the
   operator wanted against the one grabbed.
2. `jq '.quality_profiles' $B/summary.json`: global default profile, scoring
   persona, `allowUpgrades`, `cutoffTier`, `minUpgradeScoreDelta`, blocked
   sources and codecs.
3. `jq '.rule_sets' $B/summary.json`: enabled user rules can add or remove
   score. Hand rule changes to `scryer-write-rules`.
4. `jq '.delay_profiles' $B/summary.json` and `PendingReleases.graphql`: a
   delay profile with `preferredProtocol` or bypass settings can change which
   release wins.
5. Same release from several indexers: source preference wins first, then
   grab quota, then indexer priority (lower number wins).
6. Scores contradicting the profile as configured: likely bug; switch to
   `scryer-report-bug`.

## Search finds nothing

1. `IndexerStatus.graphql`: disabled, `enableAutoSearch` false,
   `disabledUntil` or `rateLimitedUntil` in the future, or
   `lastHealthStatus` failing.
2. `IndexerErrors.graphql` with `{"first": 25}`, then
   `jq '.data.indexerErrors.items | group_by(.classification) | map({c: .[0].classification, n: length, op: (map(.operation) | unique)})'`.
   Read the groups in `codes.md`.
3. `SystemHealth.graphql`: `indexerStats` with `failedLast24H` close to
   `queriesLast24H`, or `apiCurrent` at `apiMax`.
4. Category routing: a title's scope (movie, series, anime) searches only
   indexers enabled for that scope with matching categories. Read
   `scryer-configure-acquisition` → indexer routing.
5. Release decisions `title_mismatch` or `category_mismatch`: results
   arrived but were refused.
6. Indexer behind a proxy: `with_proxy` in `summary.indexers`.

## UI or login problem

1. `jq '.auth, .network, .network_configuration' $B/environment.json`.
2. `auth.environment_override_active` true: an environment variable decides
   the auth mode, so UI changes may not stick.
3. Reverse proxy: `base_path_configured`, `public_url_configured`,
   `trusted_proxies_configured`. Base path not matching the proxy gives blank
   pages or 404s for assets. Behind a proxy, check the proxy is listed as
   trusted before relying on client addresses.
4. Passkeys need `webauthn_configured` and a public URL matching the browser
   origin.
5. Logs: `log-search.sh $B 'auth|session|login|cors|webauthn' WARN`.
6. Locked out with no working login: reset path is a deployment change the
   operator makes; say so and stop.

## Performance

1. `jq '.os, .process, .datastore_files, .datastore' $B/environment.json`:
   memory, uptime, peak resident size, database and WAL size.
2. `jq '.titles, .download_queue' $B/summary.json` for scale.
3. Metrics (when present):
   `grep -E '^scryer_(http_request_duration_seconds|datastore_writer_gate_wait_seconds)_(sum|count)' $B/metrics.prom | head -n 40`.
   Large writer-gate wait = database contention.
4. `LatestJobRuns.graphql`: a job stuck in `RUNNING` or `DISCOVERING` for a
   long time, such as a library scan.
5. WAL far larger than the database, or database on a network share: common
   cause; storage moves are the operator's.

## Upgrade problem

1. `UpgradeStatus.graphql`. `managementOwner: OPERATOR` = Scryer will not
   upgrade itself; `eligibilityReason` names who does (list in `codes.md`).
2. `latestRun.status` FAILED: `JobRuns.graphql` with
   `{"jobKey": "APPLICATION_UPGRADE", "limit": 3}`, read `errorText`.
3. `jq '.smg_compatibility_notice' $B/environment.json`: `present` true with
   `minimum_version` above `your_version` = metadata gateway needs a newer
   Scryer; upgrade is the fix.
4. After an upgrade: `jq '.datastore' $B/environment.json` and
   `log-search.sh $B 'migrat' ERROR` for a failed migration. Failed migration
   is a bug report.
