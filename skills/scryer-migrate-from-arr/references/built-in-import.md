# The setup wizard's import, driven through the API

These are the calls the web UI's setup wizard makes on its **Import from
Sonarr / Radarr** path, in the wizard's order: **Connect**, **Libraries**,
**Quality**, **Sources**, **Summary**. Do them in this order, with the same
inputs. Do not add your own creation steps: the wizard's import creates the
download clients, indexers and library roots itself.

`O=path/to/skills/scryer-migrate-from-arr/operations`; `gql` and `poll` are
the helpers from `SKILL.md`. Every variables file below holds arr API keys or
other secrets, so write it with `umask 077`, never print it, and remove
nothing but your own temp files.

## What differs from the web wizard

An API key only exists once setup has been skipped (`SKILL.md`, step c), so
this flow runs as the web UI's re-entry does (**Settings → General → Run
setup wizard**): setup stays complete on the server throughout, and the flow
ends with `completeSetup(sourceImportRunId)` as the wizard does. That call
needs `ManageSystemSettings`, accepts an instance whose setup is already
complete, and erases the copy of the selected client and indexer settings,
secrets included, that the Sources step stored for the run. The run must have
been started with the same key's user. Two differences remain:

- The wizard saves typed secrets to a server-side draft so a page reload
  keeps them, and clears it at the end. The agent keeps secrets in its
  environment, never writes the draft, so has nothing to clear.
- On an existing library the agent omits `settings` from `UpdateLibrary`
  (step 5), because the wizard's call would reset the library's other saved
  overrides.

## 1. Connect: validate, then start the warm-up (read-only for the library)

Per arr app, and for Prowlarr if the operator uses it:

```sh
umask 077
jq -n --arg u "$SONARR_URL" --arg k "$SONARR_API_KEY" \
  '{input: {kind: "SONARR", connection: {baseUrl: $u, apiKey: $k}}}' > sonarr-conn.json
gql "$O/ValidateArrConnection.graphql" sonarr-conn.json | jq -c '.data.validateExternalImportConnection | {connected, version}'
gql "$O/StartArrSourceWarmup.graphql" sonarr-conn.json > w.json
jq -r '.data.startExternalImportArrSourceWarmup.sessionId' w.json   # keep as SONARR_SESSION
```

`baseUrl` is the address **Scryer** uses to reach the arr app (`SKILL.md`,
step b). `connected: false` comes with a human `error`: show it, do not parse
it. Refusal reasons: `EXTERNAL_IMPORT_INVALID_HOST`,
`EXTERNAL_IMPORT_SOURCE_URL_INVALID`, `EXTERNAL_IMPORT_SOURCE_URL_QUERY_OR_FRAGMENT`
(drop `?…` and `#…`), `EXTERNAL_IMPORT_SOURCE_APPLICATION_MISMATCH` (the URL
belongs to the other app), `EXTERNAL_IMPORT_VERSION_EXPECTED_MAJOR_NEWER_UNSUPPORTED`,
`EXTERNAL_IMPORT_API_VERSION_EXPECTED_ONE_UNSUPPORTED`.

Prowlarr: validate with `kind: "PROWLARR"`, then `StartProwlarrWarmup` with
`{input: {connection: {baseUrl, apiKey}}}`, and poll its session with
`ExternalImportWarmupStatus` until `status` is `COMPLETED`, `FAILED` or
`CANCELED`. Keep its connection in `prowlarr-conn.json` as `{baseUrl, apiKey}`.

## 2. Libraries: preview, map every root, check the paths

Poll the preview until every arr source has settled, as the wizard does:

```sh
jq -n --arg a "$SONARR_SESSION" --arg b "$RADARR_SESSION" --arg p "$PROWLARR_SESSION" \
  '{input: {sourceWarmupSessionIds: [$a, $b] | map(select(. != "")),
    prowlarrWarmupSessionId: (if $p == "" then null else $p end)}}' > p.json
poll "$O/PreviewExternalImport.graphql" p.json \
  '[.data.previewExternalImport.arrSources[].status] | length > 0 and all(IN("COMPLETED","FAILED","CANCELED"))' 5 > preview.json
jq -c '.data.previewExternalImport | {sources: [.arrSources[] | {kind, status, error}],
  roots: [.rootFolders[] | [.kind, .arrRootPath]]}' preview.json
```

`rootFolders` holds every arr root folder and every folder titles actually
live in. Each one needs a Scryer library and a Scryer-side path (the same
path, or where Scryer sees it). Agree them with the operator. Anime is its
own facet: Sonarr series with `seriesType: "anime"` usually go to an `ANIME`
library. One path belongs to one library only.

Check that Scryer can open each Scryer-side path, as the wizard does before
it lets the operator continue:

```sh
jq -n --arg p "/media/series" '{path: $p}' > bp.json
gql "$O/BrowsePath.graphql" bp.json > /dev/null && echo ok
```

An error means Scryer cannot read that path; fix the path with the operator.

## 3. Quality: one profile and persona per mapped library

The wizard requires a quality profile for every mapped library and offers a
scoring persona (`BALANCED` by default; `AUDIOPHILE`, `EFFICIENT`,
`COMPATIBLE`). Read the choices:

```sh
gql "$O/QualityProfileSettings.graphql" | jq -c '.data.qualityProfileSettings | {globalProfileId, profiles: [.profiles[] | {id, name}]}'
```

The wizard preselects the first profile. Ask the operator per library. They
are sent in step 5 when a library is created.

## 4. Sources: create download clients and indexers (side effect: confirm)

The wizard selects every `supported` client and indexer by default:

```sh
jq -c '.data.previewExternalImport | {clients: [.downloadClients[] | {dedupKey, name, supported, requiresPasswordOverride, apiKeyPresent}],
  indexers: [.indexers[] | {dedupKey, name, supported, requiresApiKeyOverride, apiKeyPresent}]}' preview.json
```

Show the list, say which need a secret again (`requiresPasswordOverride`,
`requiresApiKeyOverride`, `apiKeyPresent: false`). Scryer keeps an encrypted
copy of the selected settings for this run until `completeSetup` in step 5
erases it. After a yes, have the operator
put each secret in an environment variable or a file and build the
variables from there. Put override secrets in the three arrays as
`{dedupKey, password}` or `{dedupKey, apiKey}`; `prowlarr` is the Prowlarr
connection or `null`.

```sh
RK=migrate-$(date +%Y%m%d%H%M%S)   # new per run; reuse it only to resend this run
jq -n --arg rk "$RK" --slurpfile p p.json --argjson clients '["<dedupKey>"]' --argjson idx '["<dedupKey>"]' \
  '{requestKey: $rk, input: {sourceWarmupSessionIds: $p[0].input.sourceWarmupSessionIds,
    prowlarr: null, selectedDownloadClientDedupKeys: $clients, selectedIndexerDedupKeys: $idx,
    downloadClientApiKeyOverrides: [], downloadClientPasswordOverrides: [], indexerApiKeyOverrides: []}}' > s.json
gql "$O/StartExternalImportSources.graphql" s.json > run.json
jq -n --arg r "$(jq -r '.data.startExternalImportSources.runId' run.json)" '{runId: $r}' > r.json
poll "$O/ExternalImportSourceStatus.graphql" r.json '.data.externalImportSourceStatus.active == false' 5 > rdone.json
jq -c '.data.externalImportSourceStatus | {ready, items: [.items[] | {key, name, state, reason}]}' rdone.json
```

The wizard uses a random `requestKey` per run; a resend with the same key
returns the same run. Item
`state` is `queued`, `installing`, `checking`, `saving`, `succeeded`,
`failed`, `interrupted` or `skipped`. For a failed item, retry with
`RetryExternalImportSources` (`runId`, `corrections: [{dedupKey, apiKey}]`)
or skip it with `SkipExternalImportSource` (`runId`, `key`); both are side
effects. Go on only when `ready` is true: finalize refuses a run with an
unresolved item (`SOURCE_IMPORT_UNRESOLVED`). Other reasons:
`SOURCE_IMPORT_CREDENTIAL_REQUIRED`, `SOURCE_IMPORT_CONFIG_CONFLICT`,
`SOURCE_IMPORT_ITEM_FAILED`, `SOURCE_IMPORT_ACTIVE`,
`SOURCE_IMPORT_ITEM_NOT_RETRYABLE`, and `SOURCE_IMPORT_INVALID_REQUEST` or
`SOURCE_IMPORT_SOURCE_NOT_READY` (fix the input or wait, then start again).

## 5. Summary and finish (side effects: confirm once for the whole finish)

Wait for the title warm-up across all arr sessions:

```sh
jq '{input: {sourceWarmupSessionIds: .input.sourceWarmupSessionIds}}' p.json > ag.json
poll "$O/ExternalImportAggregateWarmupProgress.graphql" ag.json \
  '.data.externalImportAggregateWarmupProgress.status | IN("COMPLETED","FAILED","CANCELED")' 10 > agdone.json
jq -c '.data.externalImportAggregateWarmupProgress | {status, titlesFetched, titlesTotal}' agdone.json
```

On `FAILED` or `CANCELED` show `errorMessage` and restart the warm-ups
(step 1). `EXTERNAL_IMPORT_SESSION_GONE` at any step means a session expired:
start again from step 1.

Show the operator the libraries, roots and counts, then run these four
steps back to back. Setup is already complete, as on a web re-entry, so
scheduled library scans are not held back; one that reaches a new root before
the hinted scan matches titles without arr's monitored states.

**Libraries.** Read `Libraries.graphql`. For each facet the wizard uses the
default library when one exists and creates it otherwise; extra libraries are
created.

- New library: `CreateLibrary` with `facet`, `name`, its roots (the first
  `isDefault: true`) and `settings: {qualityProfileId, scoringPersona}` from
  step 3.
- Existing library: `UpdateLibrary` with every stored root (with its `id`
  and `isDefault`) plus each new path. Never leave a root out and never send
  `removedRootIds`. Omit `settings`: it replaces every saved override on the
  library. The wizard sends it, but on a configured library that resets the
  operator's settings. Name the chosen profile to the operator instead.

**Finalize** with the run id from step 4 (`null` if step 4 was skipped):

```sh
jq -n --arg run "$RUN_ID" --slurpfile p p.json --slurpfile m mappings.json \
  '{input: {sourceImportRunId: $run, sourceWarmupSessionIds: $p[0].input.sourceWarmupSessionIds, mappings: $m[0]}}' > fin.json
gql "$O/FinalizeExternalImport.graphql" fin.json > fin-out.json
jq -n --arg s "$(jq -r '.data.finalizeExternalImport.finalizeSessionId' fin-out.json)" '{sessionId: $s}' > fs.json
poll "$O/ExternalImportWarmupStatus.graphql" fs.json \
  '.data.externalImportWarmupStatus.status | IN("COMPLETED","FAILED","CANCELED")' 10 > fdone.json
```

`mappings.json` is an array of `{sourceWarmupSessionId, sourceKey, kind,
arrRootPath, scryerRootPath, libraryId, facet}`, one per preview root, with
the first four copied from `preview.json`. Refusals: `LOCATION_PLAN_REFUSED`
or `LOCATION_ROOT_REFUSED` with `extensions.refusalCode` (the path is
unusable), `INTERFACE_IMPORT_MAPPING_INCOMPLETE` (a root has no mapping). On
`FAILED`, show `errorMessage` and finalize again.

**Complete setup** once finalize is `COMPLETED`, with the same run id (omit
it if step 4 was skipped). Stop unless `completed` is `true`;
`SOURCE_IMPORT_UNRESOLVED` means a source item is still unresolved (step 4).

```sh
jq -n --arg run "$RUN_ID" '{sourceImportRunId: (if $run == "" then null else $run end)}' > cs.json
gql "$O/CompleteSetup.graphql" cs.json | jq -c .data.completeSetup
```

**Scan** each library you created or updated:

```sh
jq -n --arg l "$LIB" '{input: {libraryId: $l, importWarmupSessionId: "external-import-monitor-apply"}}' > sc.json
gql "$O/ScanLibrary.graphql" sc.json > scan.json
jq -n --arg s "$(jq -r '.data.scanLibrary.sessionId' scan.json)" '{sessionId: $s}' > ss.json
poll "$O/LibraryScanSession.graphql" ss.json \
  '.data.libraryScanSession.status | IN("COMPLETED","WARNING","FAILED","CANCELED")' 15 > sdone.json
jq -c '.data.libraryScanSession | {status, summary}' sdone.json
```

`external-import-monitor-apply` is a fixed name, not the finalize session. It
tells the scan to apply arr's monitored states and the scan hints that
finalize staged.
