# Built-in import, step by step

`O=path/to/skills/scryer-migrate-from-arr/operations` and `gql`, `poll` are the
helpers from `SKILL.md`. Every variables file holds arr API keys, so write it
with `umask 077`, never print it, and remove nothing but your own temp files.

Build connection variables without echoing the key:

```sh
umask 077
jq -n --arg u "$SONARR_URL" --arg k "$SONARR_API_KEY" \
  '{input: {kind: "SONARR", connection: {baseUrl: $u, apiKey: $k}}}' > sonarr-conn.json
```

`baseUrl` must be the address **Scryer** uses to reach the arr app. When
Scryer runs in a container, `localhost` means the container. Ask the operator
if the URLs differ.

## 1. Validate (read-only)

```sh
gql "$O/ValidateArrConnection.graphql" sonarr-conn.json | jq -c '.data.validateExternalImportConnection | {connected, version}'
```

`connected: false` comes with a human `error`. Show it to the operator. Do
not parse it. Refusals with a reason: `EXTERNAL_IMPORT_INVALID_HOST`,
`EXTERNAL_IMPORT_SOURCE_URL_INVALID`, `EXTERNAL_IMPORT_SOURCE_URL_QUERY_OR_FRAGMENT`
(drop `?…` and `#…`), `EXTERNAL_IMPORT_SOURCE_APPLICATION_MISMATCH` (the URL
belongs to the other app), `EXTERNAL_IMPORT_VERSION_EXPECTED_MAJOR_NEWER_UNSUPPORTED`,
`EXTERNAL_IMPORT_API_VERSION_EXPECTED_ONE_UNSUPPORTED`.

## 2. Warm up (reads arr, changes nothing in the library)

```sh
gql "$O/StartArrSourceWarmup.graphql" sonarr-conn.json > w.json
jq -n --arg s "$(jq -r '.data.startExternalImportArrSourceWarmup.sessionId' w.json)" '{sessionId: $s}' > ws.json
PROGRESS_JQ='.data.externalImportWarmupStatus | "\(.status) \(.phase)"' \
  poll "$O/ExternalImportWarmupStatus.graphql" ws.json \
  '.data.externalImportWarmupStatus.status | IN("COMPLETED","FAILED","CANCELED")' 10 > wdone.json
jq -c '.data.externalImportWarmupStatus | {status, matchedSeriesCount, unmatchedSeriesCount, ambiguousSeriesCount, matchedMovieCount, unmatchedMovieCount, ambiguousMovieCount}' wdone.json
```

Repeat per app. Keep every `sessionId`. `EXTERNAL_IMPORT_SESSION_GONE` means
the session expired: warm up again. Optional Prowlarr: `StartProwlarrWarmup`
with `{input: {connection: {baseUrl, apiKey}}}`, polled the same way, and its
session id passed as `prowlarrWarmupSessionId` in step 3.

## 3. Preview (read-only)

```sh
jq -n --arg a "$SONARR_SESSION" --arg b "$RADARR_SESSION" \
  '{input: {sourceWarmupSessionIds: [$a, $b] | map(select(. != "")), prowlarrWarmupSessionId: null}}' > p.json
gql "$O/PreviewExternalImport.graphql" p.json > preview.json
jq -c '.data.previewExternalImport | {roots: [.rootFolders[] | [.kind, .arrRootPath]],
  clients: [.downloadClients[] | {dedupKey, name, supported, requiresPasswordOverride, apiKeyPresent}],
  indexers: [.indexers[] | {dedupKey, name, supported, requiresApiKeyOverride, apiKeyPresent}]}' preview.json
```

## 4. Create download clients and indexers (side effect: confirm)

Show the operator the supported clients and indexers. Ask which to create,
and collect the passwords or API keys the preview flags as missing. Ask the
operator to put each secret in an environment variable or a file, and read it
from there. Never paste it into the conversation.

```sh
jq -n --slurpfile p p.json --argjson clients '["<dedupKey>"]' --argjson idx '["<dedupKey>"]' \
  '{requestKey: "migrate-1", input: {sourceWarmupSessionIds: $p[0].input.sourceWarmupSessionIds,
    prowlarr: null, selectedDownloadClientDedupKeys: $clients, selectedIndexerDedupKeys: $idx,
    downloadClientApiKeyOverrides: [], downloadClientPasswordOverrides: [], indexerApiKeyOverrides: []}}' > s.json
gql "$O/StartExternalImportSources.graphql" s.json > run.json
jq -n --arg r "$(jq -r '.data.startExternalImportSources.runId' run.json)" '{runId: $r}' > r.json
poll "$O/ExternalImportSourceStatus.graphql" r.json '.data.externalImportSourceStatus.active == false' 5 > rdone.json
jq -c '.data.externalImportSourceStatus | {ready, items: [.items[] | {key, name, state, reason}]}' rdone.json
```

With Prowlarr, set `prowlarr` to its `{baseUrl, apiKey}` connection instead
of `null`. Put override secrets in the three override arrays as
`{dedupKey, password}` or `{dedupKey, apiKey}`.

Item `state` is one of `queued`, `installing`, `checking`, `saving`,
`succeeded`, `failed`, `interrupted`, `skipped`. For a `failed` or
`interrupted` indexer, retry with `RetryExternalImportSources` (`runId` plus
`corrections: [{dedupKey, apiKey}]`), or skip with `SkipExternalImportSource`
(`runId`, `key`). Both are side effects. Reasons include
`SOURCE_IMPORT_CREDENTIAL_REQUIRED`, `SOURCE_IMPORT_CONFIG_CONFLICT`,
`SOURCE_IMPORT_ITEM_FAILED`, `SOURCE_IMPORT_ACTIVE` and
`SOURCE_IMPORT_ITEM_NOT_RETRYABLE`.

## 5. Libraries for every root (side effect: confirm)

Finalize needs a mapping for **every** root in the preview, including folders
where titles live outside a configured root folder. For each root, agree with
the operator the Scryer-side path (as Scryer sees it) and the library:

- No library for that facet yet: `CreateLibrary` with the root.
- A library exists: `Libraries.graphql`, then `UpdateLibrary` with every
  existing root (with its `id`) plus the new path. Never leave a root out and
  never send `removedRootIds`.

One path may belong to one library only. Anime is its own facet: Sonarr series
with `seriesType: "anime"` usually belong in an `ANIME` library.

## 6. Finalize (side effect: confirm)

```sh
jq -n --arg run "$RUN_ID" --slurpfile p p.json --slurpfile m mappings.json \
  '{input: {sourceImportRunId: $run, sourceWarmupSessionIds: $p[0].input.sourceWarmupSessionIds, mappings: $m[0]}}' > fin.json
gql "$O/FinalizeExternalImport.graphql" fin.json > fin-out.json
jq -n --arg s "$(jq -r '.data.finalizeExternalImport.finalizeSessionId' fin-out.json)" '{sessionId: $s}' > fs.json
poll "$O/ExternalImportWarmupStatus.graphql" fs.json \
  '.data.externalImportWarmupStatus.status | IN("COMPLETED","FAILED","CANCELED")' 10 > fdone.json
```

`mappings.json` is an array of `{sourceWarmupSessionId, sourceKey, kind,
arrRootPath, scryerRootPath, libraryId, facet}`, one per preview root. Copy
`sourceWarmupSessionId`, `sourceKey`, `kind` and `arrRootPath` from
`preview.json`. Use `null` for `sourceImportRunId` when step 4 was skipped.
Refusals: `LOCATION_PLAN_REFUSED` or `LOCATION_ROOT_REFUSED` with
`extensions.refusalCode` (the path is unusable), or reason
`INTERFACE_IMPORT_MAPPING_INCOMPLETE` for an unmapped root. On `FAILED`, show `errorMessage` and run finalize again; it
redoes the snapshot.

## 7. Scan each mapped library (side effect: confirm)

Only after finalize is `COMPLETED`:

```sh
jq -n --arg l "$LIB" '{input: {libraryId: $l, importWarmupSessionId: "external-import-monitor-apply"}}' > sc.json
gql "$O/ScanLibrary.graphql" sc.json > scan.json
jq -n --arg s "$(jq -r '.data.scanLibrary.sessionId' scan.json)" '{sessionId: $s}' > ss.json
poll "$O/LibraryScanSession.graphql" ss.json \
  '.data.libraryScanSession.status | IN("COMPLETED","WARNING","FAILED","CANCELED")' 15 > sdone.json
jq -c '.data.libraryScanSession | {status, summary}' sdone.json
```

The `importWarmupSessionId` value is a fixed name, not the finalize session.
It tells the scan to apply the staged monitored states and hints.
