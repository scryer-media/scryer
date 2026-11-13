#!/bin/sh
# Score a list of release names against a draft rule without saving it.
#
#   test-names.sh <rule.rego> <facets> <title-id> [episode-id] < names.txt > results.tsv
#
# <facets>    comma-separated appliedFacets for the draft, e.g. movie or series,anime
# names.txt   one release name per line; an optional TAB and size in bytes may follow
# results.tsv name, draft score, total score, allowed, blocked, draft entries, errors
#
# Prints one summary line to stderr. Needs SCRYER_URL and SCRYER_API_KEY, and
# finds gql.sh in the sibling scryer-connect skill (override with GQL=/path/gql.sh).

set -eu

[ $# -ge 3 ] && [ $# -le 4 ] || {
  echo "usage: test-names.sh <rule.rego> <facets> <title-id> [episode-id] < names.txt" >&2
  exit 64
}
rule_file=$1
facets=$2
title_id=$3
episode_id=${4:-}

here=$(cd "$(dirname "$0")" && pwd)
gql=${GQL:-$here/../../scryer-connect/scripts/gql.sh}
op=$here/../operations/TestRuleSet.graphql
[ -r "$rule_file" ] || { echo "test-names.sh: cannot read $rule_file" >&2; exit 64; }
[ -x "$gql" ] || { echo "test-names.sh: gql.sh not found at $gql" >&2; exit 64; }

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/scryer-test-names.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

total=0
failed=0
printf 'name\tdraft_score\ttotal_score\tallowed\tblocked\tdraft_entries\terrors\n'
while IFS= read -r line || [ -n "$line" ]; do
  [ -n "$line" ] || continue
  name=$(printf '%s' "$line" | cut -f1)
  size=$(printf '%s' "$line" | cut -s -f2)
  jq -n --rawfile src "$rule_file" --arg facets "$facets" --arg title "$title_id" \
    --arg episode "$episode_id" --arg name "$name" --arg size "$size" '
    {input: ({
      draft: {name: "draft", description: "", regoSource: $src, enabled: true,
              priority: 0, appliedFacets: ($facets | split(",") | map(select(. != "")))},
      copyDisablesSource: false,
      titleId: $title,
      releaseName: $name
    }
    + (if $episode != "" then {episodeId: $episode} else {} end)
    + (if $size != "" then {sizeBytes: ($size | tonumber)} else {} end))}
  ' > "$tmp_dir/vars.json"
  total=$((total + 1))
  if "$gql" "$op" "$tmp_dir/vars.json" > "$tmp_dir/out.json" 2> "$tmp_dir/err.txt"; then
    jq -r --arg name "$name" '
      .data.testRuleSet as $r
      | [$name,
         ($r.draftContribution.score | tostring),
         ($r.score | tostring),
         ($r.allowed | tostring),
         ($r.blocked | tostring),
         ([$r.ruleSets[] | select(.isDraft) | .entries[] | "\(.code):\(.delta)"] | join(";")),
         ([$r.errors[] | .code] | join(";"))]
      | @tsv' "$tmp_dir/out.json"
  else
    failed=$((failed + 1))
    printf '%s\t\t\t\t\t\t%s\n' "$name" "$(head -n 1 "$tmp_dir/err.txt" | tr '\t' ' ')"
  fi
done

echo "test-names.sh: $total names tested, $failed requests failed" >&2
