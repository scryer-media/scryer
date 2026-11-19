# Cutover

Skill never writes to Sonarr or Radarr. Every arr change below is made by the
operator in arr's own web UI. Arr data, files and settings stay in place until
the operator decides otherwise, after Scryer is verified. Never suggest
deleting arr, its database, its config or any media.

## 1. Side by side

After import and checklist both apps know the same library. Only one may grab
and import at a time, else both download the same releases and both try to
import them.

Until cutover keep Scryer from grabbing: ask operator to turn off automatic search
on Scryer's indexers in Scryer's web UI; check with `Indexers.graphql`
(`enableAutoSearch: false`). Manual Scryer searches still work for testing.

## 2. Stop arr first (operator, in arr's UI)

Per arr app, in order:

1. Settings → Indexers: for every indexer turn off **Enable RSS** and
   **Enable Automatic Search**. Arr stops grabbing.
2. Wait until arr's queue holds nothing of its own still downloading, or agree
   with operator to let Scryer pick those up.
3. Settings → Download Clients: turn off **Completed Download Handling**. Arr
   stops importing and stops removing finished downloads from the client.

Confirm from a fresh snapshot (read-only):

```sh
"$M/scripts/snapshot.sh" snap-after
jq -c 'map_values(.failedEndpoints)' snap-after/summary.json
jq -c '{cdh: .enableCompletedDownloadHandling}' snap-after/*/config_downloadclient.json
jq -c '[.[] | select(.enableRss or .enableAutomaticSearch) | .name]' snap-after/*/indexer.json
```

Last command must print `[]` for every app before going on.

## 3. Start Scryer (side effect: confirm)

Ask operator to turn automatic search back on for Scryer's indexers; check each
download client is enabled (`DownloadClientConfigs.graphql`). Then run the
end-to-end test from `verification.md`.

Download client shared with arr: ask operator to check the category or label
Scryer's client uses, so each app sees only its own downloads.

## 4. Tools that talk to arr

Tools that push releases to arr's API (`/api/v3/release/push`) or read its
title list can point at Scryer instead:

- Sonarr → `$SCRYER_URL/compat/sonarr`
- Radarr → `$SCRYER_URL/compat/radarr`
- one library only → `$SCRYER_URL/compat/sonarr/library/<library id>` (or `radarr`)

Auth: Scryer API key in `X-Api-Key`. Only `api/v3/system/status`,
`api/v3/release/push`, `api/v3/tag` and `api/v3/series` (Sonarr) or
`api/v3/movie` (Radarr) exist. Anything else the tool needs is unsupported;
tell operator which tool and call.

## 5. Rolling back

Reverse: turn Scryer's automatic search off, then arr's indexers and Completed
Download Handling back on. Nothing was removed from arr; it resumes where it
stopped.
