#!/bin/sh
# Replay grabbed release names from an arr snapshot through a draft rule set
# and compare Scryer's score with the arr custom format score.
#
#   replay-history.sh <snapshot-app-dir> <rule.rego> <facets> [arr-profile-id] > replay.tsv
#
# <snapshot-app-dir>  e.g. snap/sonarr, written by snapshot.sh
# <facets>            comma-separated appliedFacets of the draft, e.g. series,anime
# [arr-profile-id]    only replay titles that use this arr quality profile
#
# Read-only on both sides: arr data comes from the snapshot files and Scryer
# only runs testRuleSet, which saves nothing. LIMIT (default 200) caps the
# number of history records replayed.
#
# Output columns: status (match|mismatch|skipped), name, arr_score,
# draft_score, arr_formats, draft_entries, note. Prints one summary line to
# stderr. Needs SCRYER_URL and SCRYER_API_KEY; finds gql.sh in the sibling
# scryer-connect skill (override with GQL=/path/gql.sh).

set -eu

[ $# -ge 3 ] && [ $# -le 4 ] || {
  echo "usage: replay-history.sh <snapshot-app-dir> <rule.rego> <facets> [arr-profile-id] > replay.tsv" >&2
  exit 64
}
snap=$1 rule_file=$2 facets=$3 profile=${4:-}
limit=${LIMIT:-200}

here=$(cd "$(dirname "$0")" && pwd)
gql=${GQL:-$here/../../scryer-connect/scripts/gql.sh}
ops=$here/../operations
[ -x "$gql" ] || { echo "replay-history.sh: gql.sh not found at $gql" >&2; exit 64; }
[ -r "$rule_file" ] || { echo "replay-history.sh: cannot read $rule_file" >&2; exit 64; }
[ -r "$snap/history_grabbed.json" ] || { echo "replay-history.sh: no $snap/history_grabbed.json" >&2; exit 64; }
if [ -r "$snap/series.json" ]; then
  app=sonarr titles=$snap/series.json source=tvdb
elif [ -r "$snap/movie.json" ]; then
  app=radarr titles=$snap/movie.json source=tmdb
else
  echo "replay-history.sh: no series.json or movie.json in $snap" >&2
  exit 64
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/scryer-replay.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
tab=$(printf '\t')

# 1. Join history with the title list. Empty fields become "-" so that
#    `read` keeps the columns aligned.
jq -r --arg app "$app" --arg profile "$profile" --argjson limit "$limit" \
  --slurpfile titles "$titles" '
  ($titles[0] | map({key: (.id | tostring), value: .}) | from_entries) as $byId
  | .records[:$limit][]
  | (if $app == "sonarr" then .seriesId else .movieId end | tostring) as $tid
  | ($byId[$tid]) as $t
  | select($profile == "" or (($t.qualityProfileId // "") | tostring) == $profile)
  | [ (.sourceTitle // "-"),
      ((.data.size // .data.Size // "-") | tostring),
      (if $t == null then "-" elif $app == "sonarr" then ($t.tvdbId // "-") else ($t.tmdbId // "-") end | tostring),
      ((.episode.seasonNumber // "-") | tostring),
      ((.episode.episodeNumber // "-") | tostring),
      ((.customFormatScore // 0) | tostring),
      ([.customFormats[]?.name] | if length == 0 then "-" else join(";") end) ]
  | map(if . == "" then "-" else gsub("\t"; " ") end)
  | @tsv' "$snap/history_grabbed.json" > "$tmp/rows.tsv"

# 2. Resolve provider ids to Scryer title ids, 100 at a time.
cut -f3 "$tmp/rows.tsv" | grep -v '^-$' | sort -u > "$tmp/ids.txt" || true
echo '{}' > "$tmp/map.json"
split -l 100 "$tmp/ids.txt" "$tmp/chunk." 2>/dev/null || true
for chunk in "$tmp"/chunk.*; do
  [ -s "$chunk" ] || continue
  jq -R . "$chunk" | jq -s --arg source "$source" '{source: $source, values: .}' > "$tmp/lookup-vars.json"
  "$gql" "$ops/TitlesByExternalIds.graphql" "$tmp/lookup-vars.json" > "$tmp/lookup.json"
  jq --arg source "$source" --slurpfile found "$tmp/lookup.json" '
    . + ($found[0].data.titlesByExternalIds
         | map(. as $t | .externalIds[] | select(.source == $source) | {key: .value, value: $t.id})
         | from_entries)' "$tmp/map.json" > "$tmp/map.next" && mv "$tmp/map.next" "$tmp/map.json"
done

# 3. Score each row.
n_match=0 n_mismatch=0 n_skipped=0
printf 'status\tname\tarr_score\tdraft_score\tarr_formats\tdraft_entries\tnote\n'
skip() {
  n_skipped=$((n_skipped + 1))
  printf 'skipped\t%s\t%s\t\t%s\t\t%s\n' "$name" "$arr_score" "$arr_formats" "$1"
}
while IFS="$tab" read -r name size ext season episode arr_score arr_formats; do
  [ "$arr_formats" = "-" ] && arr_formats=""
  if [ "$ext" = "-" ]; then skip "title not in arr snapshot"; continue; fi
  title_id=$(jq -r --arg k "$ext" '.[$k] // empty' "$tmp/map.json")
  if [ -z "$title_id" ]; then skip "$source $ext not in Scryer"; continue; fi
  episode_id=""
  if [ "$app" = sonarr ]; then
    if [ "$season" = "-" ] || [ "$episode" = "-" ]; then skip "no episode in history record"; continue; fi
    eps=$tmp/eps-$title_id.json
    if [ ! -f "$eps" ]; then
      jq -n --arg id "$title_id" '{id: $id}' > "$tmp/eps-vars.json"
      "$gql" "$ops/TitleEpisodes.graphql" "$tmp/eps-vars.json" > "$eps" || { rm -f "$eps"; skip "episode lookup failed"; continue; }
    fi
    episode_id=$(jq -r --arg s "$season" --arg e "$episode" '
      def num: tostring | ltrimstr("0") | if . == "" then "0" else . end;
      [.data.title.collections[]?.episodes[]?
       | select((.seasonNumber | num) == ($s | num) and (.episodeNumber | num) == ($e | num))
       | .id][0] // empty' "$eps")
    if [ -z "$episode_id" ]; then skip "S${season}E${episode} not in Scryer"; continue; fi
  fi
  jq -n --rawfile src "$rule_file" --arg facets "$facets" --arg title "$title_id" \
    --arg episode "$episode_id" --arg name "$name" --arg size "$size" '
    {input: ({
      draft: {name: "replay", description: "", regoSource: $src, enabled: true,
              priority: 0, appliedFacets: ($facets | split(",") | map(select(. != "")))},
      copyDisablesSource: false, titleId: $title, releaseName: $name}
      + (if $episode != "" then {episodeId: $episode} else {} end)
      + (if ($size | test("^[0-9]+$")) then {sizeBytes: ($size | tonumber)} else {} end))}
  ' > "$tmp/vars.json"
  if ! "$gql" "$ops/TestRuleSetScore.graphql" "$tmp/vars.json" > "$tmp/out.json" 2> "$tmp/err.txt"; then
    skip "$(head -n 1 "$tmp/err.txt" | tr '\t' ' ')"
    continue
  fi
  line=$(jq -r --arg arr "$arr_score" '
    .data.testRuleSet as $r
    | ($r.draftContribution.score // 0) as $d
    | [ (if ($d | tostring) == $arr then "match" else "mismatch" end),
        ($d | tostring),
        ([$r.ruleSets[] | select(.isDraft) | .entries[] | "\(.code):\(.delta)"] | join(";")),
        ([$r.errors[] | .code] | join(";")) ]
    | @tsv' "$tmp/out.json")
  # Split the four @tsv fields; jq escapes tabs inside a field.
  status=${line%%"$tab"*}
  rest=${line#*"$tab"}
  draft_score=${rest%%"$tab"*}
  rest=${rest#*"$tab"}
  entries=${rest%%"$tab"*}
  errors=${rest#*"$tab"}
  if [ "$status" = match ]; then n_match=$((n_match + 1)); else n_mismatch=$((n_mismatch + 1)); fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$status" "$name" "$arr_score" "$draft_score" \
    "$arr_formats" "$entries" "$errors"
done < "$tmp/rows.tsv"

echo "replay-history.sh: $n_match match, $n_mismatch mismatch, $n_skipped skipped" >&2
