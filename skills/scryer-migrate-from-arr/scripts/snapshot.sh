#!/bin/sh
# Snapshot Sonarr and/or Radarr configuration into files, then write counts.
#
#   snapshot.sh <out-dir>
#
# Reads SONARR_URL + SONARR_API_KEY and/or RADARR_URL + RADARR_API_KEY (URLs
# without a trailing slash, including any base path). Only GET requests are
# made; nothing in Sonarr or Radarr changes.
#
# Writes <out-dir>/<app>/<endpoint>.json (mode 600, dir mode 700) and
# <out-dir>/summary.json with counts only. Read summary.json, not the raw files.
# HISTORY_LIMIT (default 500) caps the grabbed-release history sample.

set -eu

[ $# -eq 1 ] || { echo "usage: snapshot.sh <out-dir>" >&2; exit 64; }
out=$1
command -v curl >/dev/null 2>&1 || { echo "snapshot.sh: curl is required" >&2; exit 64; }
command -v jq >/dev/null 2>&1 || { echo "snapshot.sh: jq is required" >&2; exit 64; }
history_limit=${HISTORY_LIMIT:-500}

umask 077
mkdir -p "$out"

# fetch <base-url> <api-key> <path-and-query> <file>
fetch() {
  set +e
  status=$(printf 'header = "X-Api-Key: %s"\n' "$2" \
    | curl --silent --show-error --config - \
        --header 'Accept: application/json' \
        --output "$4" --write-out '%{http_code}' \
        "$1/api/v3/$3")
  rc=$?
  set -e
  if [ "$rc" -ne 0 ] || [ "$status" -ge 400 ] || ! jq -e . "$4" >/dev/null 2>&1; then
    printf '{"_error": "HTTP %s curl %s"}\n' "$status" "$rc" > "$4"
    echo "snapshot.sh: $3 -> HTTP $status (curl $rc)" >&2
    return 1
  fi
  return 0
}

snapshot_app() {
  app=$1 url=$2 key=$3
  dir=$out/$app
  mkdir -p "$dir"
  common="system/status rootfolder qualityprofile customformat delayprofile
          downloadclient indexer notification importlist tag remotepathmapping
          autotagging config/naming config/mediamanagement config/downloadclient"
  if [ "$app" = sonarr ]; then
    specific="series releaseprofile"
    history_query="history?page=1&pageSize=$history_limit&sortKey=date&sortDirection=descending&eventType=1&includeEpisode=true"
  else
    specific="movie exclusions"
    history_query="history?page=1&pageSize=$history_limit&sortKey=date&sortDirection=descending&eventType=1"
  fi
  failures=0
  for endpoint in $common $specific; do
    file=$dir/$(printf '%s' "$endpoint" | tr '/' '_').json
    fetch "$url" "$key" "$endpoint" "$file" || failures=$((failures + 1))
  done
  fetch "$url" "$key" "$history_query" "$dir/history_grabbed.json" || failures=$((failures + 1))
  echo "snapshot.sh: $app done, $failures endpoint(s) failed" >&2
}

apps=""
if [ -n "${SONARR_URL:-}" ]; then
  [ -n "${SONARR_API_KEY:-}" ] || { echo "snapshot.sh: SONARR_API_KEY is not set" >&2; exit 64; }
  snapshot_app sonarr "${SONARR_URL%/}" "$SONARR_API_KEY"
  apps="$apps sonarr"
fi
if [ -n "${RADARR_URL:-}" ]; then
  [ -n "${RADARR_API_KEY:-}" ] || { echo "snapshot.sh: RADARR_API_KEY is not set" >&2; exit 64; }
  snapshot_app radarr "${RADARR_URL%/}" "$RADARR_API_KEY"
  apps="$apps radarr"
fi
[ -n "$apps" ] || { echo "snapshot.sh: set SONARR_URL and/or RADARR_URL" >&2; exit 64; }

# Counts only. Arrays of small labels (root paths, profile names) are kept
# because they drive decisions; nothing secret is copied.
summarize() {
  dir=$out/$1
  jq -n --arg app "$1" \
    --slurpfile status "$dir/system_status.json" \
    --slurpfile roots "$dir/rootfolder.json" \
    --slurpfile profiles "$dir/qualityprofile.json" \
    --slurpfile formats "$dir/customformat.json" \
    --slurpfile delays "$dir/delayprofile.json" \
    --slurpfile clients "$dir/downloadclient.json" \
    --slurpfile indexers "$dir/indexer.json" \
    --slurpfile notifications "$dir/notification.json" \
    --slurpfile lists "$dir/importlist.json" \
    --slurpfile tags "$dir/tag.json" \
    --slurpfile rpm "$dir/remotepathmapping.json" \
    --slurpfile autotag "$dir/autotagging.json" \
    --slurpfile naming "$dir/config_naming.json" \
    --slurpfile history "$dir/history_grabbed.json" \
    --slurpfile titles "$dir/$( [ "$1" = sonarr ] && echo series || echo movie ).json" \
    --slurpfile extra "$dir/$( [ "$1" = sonarr ] && echo releaseprofile || echo exclusions ).json" '
    def arr(x): if (x | type) == "array" then x else null end;
    def n(x): if (x | type) == "array" then (x | length) else null end;
    ($titles[0]) as $t
    | ($profiles[0]) as $p
    | {
        app: $app,
        version: ($status[0].version // null),
        titles: (if arr($t) then {
            total: ($t | length),
            monitored: ([$t[] | select(.monitored)] | length),
            withFiles: ([$t[] | select(.hasFile == true or ((.statistics.episodeFileCount // 0) > 0))] | length),
            withoutFiles: ([$t[] | select(.hasFile != true and ((.statistics.episodeFileCount // 0) == 0))] | length),
            anime: (if $app == "sonarr" then ([$t[] | select(.seriesType == "anime")] | length) else null end),
            byQualityProfileId: ($t | group_by(.qualityProfileId) | map({key: (.[0].qualityProfileId | tostring), value: length}) | from_entries)
          } else null end),
        rootFolders: (arr($roots[0]) | if . then map(.path) else null end),
        qualityProfiles: (if arr($p) then {
            count: ($p | length),
            names: ($p | map({id, name})),
            scoringFormats: ([$p[] | select([.formatItems[]? | select(.score != 0)] | length > 0)] | length),
            distinctScoreSets: ([$p[] | [.formatItems[]? | select(.score != 0) | [.format, .score]] | sort | select(length > 0)] | unique | length)
          } else null end),
        customFormats: n($formats[0]),
        delayProfiles: n($delays[0]),
        releaseProfiles: (if $app == "sonarr" then n($extra[0]) else null end),
        importListExclusions: (if $app == "radarr" then n($extra[0]) else null end),
        downloadClients: (arr($clients[0]) | if . then {count: length, enabled: ([.[] | select(.enable)] | length), implementations: (map(.implementation) | unique)} else null end),
        indexers: (arr($indexers[0]) | if . then {count: length, implementations: (map(.implementation) | unique)} else null end),
        notifications: (arr($notifications[0]) | if . then {count: length, implementations: (map(.implementation) | unique)} else null end),
        importLists: (arr($lists[0]) | if . then {count: length, implementations: (map(.implementation) | unique)} else null end),
        tags: n($tags[0]),
        remotePathMappings: n($rpm[0]),
        autoTagging: n($autotag[0]),
        renameEnabled: ($naming[0].renameEpisodes // $naming[0].renameMovies // null),
        historyGrabbedSample: (if ($history[0] | type) == "object" then ($history[0].records // [] | length) else null end),
        failedEndpoints: ([$status, $roots, $profiles, $formats, $delays, $clients, $indexers, $notifications, $lists, $tags, $rpm, $autotag, $naming, $history, $titles, $extra] | map(.[0] | select(type == "object" and has("_error"))) | length)
      }' > "$dir/summary.json"
}

set --
for app in $apps; do
  summarize "$app"
  set -- "$@" "$out/$app/summary.json"
done
jq -s 'map({key: .app, value: .}) | from_entries' "$@" > "$out/summary.json"
echo "snapshot.sh: wrote $out/summary.json" >&2
