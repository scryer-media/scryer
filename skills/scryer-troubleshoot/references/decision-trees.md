# Decision trees

`$S` is this skill's directory and `$B` the unpacked bundle. Operations
named below are in `$S/operations/`; run each with
`sh "$G" "$S"/operations/<Name>.graphql [vars.json] > out.json` and read
the output with jq. `log-search.sh` is `$S/scripts/log-search.sh`. Codes
are explained in `codes.md`.

Any fix below that changes Scryer's settings is a write: show the operator
the exact change and wait for a yes, or hand them the UI path. Restarts,
container changes, file permission changes and queue removals are the
operator's to do; describe them, do not run them.

## Nothing downloads

1. `jq '.settings.acquisition' $B/environment.json` is evidence only. These
   values are internal and not operator-configurable; never suggest changing
   them. If `enabled` is false or a value looks abnormal, record it for a
   maintainer report (`scryer-report-bug`) and keep going.
2. `HealthChecks.graphql`: any `DownloadClient` or `Indexer` check at
   `error` explains it. No client configured, or all indexers failing, stops
   everything.
3. `jq '.indexers' $B/summary.json`: no enabled indexer, or
   `auto_search_enabled` is 0, means automatic search has nothing to query.
4. `jq '.release_decisions.by_code' $B/summary.json`, top codes:
   - mostly `eligible` but no grabs: check the queue and download clients
     (step 5).
   - `pending_delay`, `minimum_age`: releases are waiting on a delay
     profile. `PendingReleases.graphql` shows `delayUntil` (convert to the
     operator's local time).
   - `quality_blocked`, `upgrade_rejected`, `cutoff_reached`: the profile is
     refusing; go to **Wrong release chosen**.
   - `download_client_unavailable`, `protocol_disabled`: no client accepts
     that protocol.
   - `acquisition_paused`, `episode_not_monitored`: monitoring or pause.
   - few rows at all: search is not finding releases; go to **Search finds
     nothing**.
5. `DownloadClientStatus.graphql`: `status` not healthy, `lastError` set,
   `disabledUntil` in the future or `escalationLevel` above 0 means Scryer
   backed off that client. Verify the client is reachable from the Scryer
   host or container; a connection test is in `scryer-configure-acquisition`.
6. `LatestJobRuns.graphql`: `RSS_SYNC`, `ACQUISITION_SEARCH` and
   `PENDING_RELEASE_PROCESSING` should have recent `COMPLETED` runs. A
   `FAILED` run: read `JobRuns.graphql` for that key and its `errorText`.
7. For a single title: `FindTitle.graphql`, then `TitleDecisions.graphql`.

## Downloads do not import

1. `DownloadQueuePage.graphql`, then
   `jq '[.data.downloadQueuePage.items[] | select(.attentionRequired)] | group_by(.importErrorCode) | map({code: .[0].importErrorCode, n: length})'`.
2. By code:
   - `FILE_NOT_FOUND`: the client's output path is not visible to Scryer.
     Check `jq '.container, .filesystems' $B/environment.json` and
     `with_remote_path_mappings` in `summary.download_clients`. In
     containers both services need the same host path mounted, or a remote
     path mapping (`REMOTE => LOCAL`) on the client. The health check
     "output path unresolvable" confirms it.
   - `PERMISSION_DENIED`: ownership or mount mode. The operator fixes it on
     the host.
   - `DISK_FULL`: `jq '.filesystems.roots' $B/environment.json`.
   - `EPISODE_NOT_FOUND`, `EPISODE_LOOKUP_FAILED`: metadata does not list
     the episode yet.
   - `POLICY_MISMATCH` or skip `POST_DOWNLOAD_RULE_BLOCKED`: a rule or the
     profile refused the file. Rules are covered by `scryer-write-rules`.
   - `passwordFailureCode` set: the archive needs a password.
   - `ARCHIVE_EXTRACTION_PLUGIN_REQUIRED`: no extraction plugin installed.
3. `ImportHistory.graphql` with limit 25, then group by `decision` and
   `skipReason`. `UNRESOLVED_IDENTITY`, `UNPARSEABLE_EPISODE`: the file name
   did not map to a title or episode. The operator resolves it from the
   queue item's import actions in the web UI.
4. Logs: `log-search.sh $B 'import' WARN`.
5. Hardlinks: `same_filesystem_pairs` empty while the operator expects
   hardlinks means download and library roots are on different filesystems.

## Wrong release chosen

1. `FindTitle.graphql`, then `TitleDecisions.graphql`. Compare
   `candidateScore`, `currentScore` and `decidedBy` for the release the
   operator wanted against the one grabbed.
2. `jq '.quality_profiles' $B/summary.json`: which profile is the global
   default, the scoring persona, `allowUpgrades`, `cutoffTier`,
   `minUpgradeScoreDelta`, blocked sources and codecs.
3. `jq '.rule_sets' $B/summary.json`: enabled user rules can add or remove
   score. Hand rule changes to `scryer-write-rules`.
4. `jq '.delay_profiles' $B/summary.json` and `PendingReleases.graphql`: a
   delay profile with `preferredProtocol` or bypass settings can change which
   release wins.
5. Several indexers returning the same release: the source preference wins
   first, then the grab quota, then the indexer priority (lower number wins).
6. If the scores contradict the profile as configured, treat it as a likely
   bug and switch to `scryer-report-bug`.

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
5. Release decisions of `title_mismatch` or `category_mismatch` mean results
   arrived but were refused.
6. Indexer behind a proxy: `with_proxy` in `summary.indexers`.

## UI or login problem

1. `jq '.auth, .network, .network_configuration' $B/environment.json`.
2. `auth.environment_override_active` true: an environment variable decides
   the auth mode, so UI changes may not stick.
3. Reverse proxy: `base_path_configured`, `public_url_configured`,
   `trusted_proxies_configured`. A base path that does not match the proxy
   gives blank pages or 404s for assets. Behind a proxy, check whether the
   proxy is listed as trusted before relying on client addresses.
4. Passkeys need `webauthn_configured` and a public URL matching the browser
   origin.
5. Logs: `log-search.sh $B 'auth|session|login|cors|webauthn' WARN`.
6. Locked out with no working login: the reset path is a deployment change
   the operator makes; say so and stop.

## Performance

1. `jq '.os, .process, .datastore_files, .datastore' $B/environment.json`:
   memory, uptime, peak resident size, database and WAL size.
2. `jq '.titles, .download_queue' $B/summary.json` for scale.
3. Metrics (when present):
   `grep -E '^scryer_(http_request_duration_seconds|datastore_writer_gate_wait_seconds)_(sum|count)' $B/metrics.prom | head -n 40`.
   A large writer-gate wait means database contention.
4. `LatestJobRuns.graphql`: a job stuck in `RUNNING` or `DISCOVERING` for a
   long time, such as a library scan.
5. A WAL far larger than the database, or a database on a network share,
   is a common cause; storage moves are the operator's to do.

## Upgrade problem

1. `UpgradeStatus.graphql`. `managementOwner: OPERATOR` means Scryer will not
   upgrade itself; `eligibilityReason` names who does (table in
   `codes.md`).
2. `latestRun.status` FAILED: `JobRuns.graphql` with
   `{"jobKey": "APPLICATION_UPGRADE", "limit": 3}` and read `errorText`.
3. `jq '.smg_compatibility_notice' $B/environment.json`: `present` true with
   a `minimum_version` above `your_version` means the metadata gateway needs
   a newer Scryer; upgrade is the fix.
4. After an upgrade: `jq '.datastore' $B/environment.json` and
   `log-search.sh $B 'migrat' ERROR` for a failed migration. A failed
   migration is a bug report.
