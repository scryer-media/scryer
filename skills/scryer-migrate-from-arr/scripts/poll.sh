#!/bin/sh
# Poll a read-only status operation until a jq condition holds.
#
#   poll.sh <operation.graphql> <variables.json> '<jq done condition>' [interval-s] [max-polls] > last.json
#
# Example:
#   poll.sh ExternalImportWarmupStatus.graphql s.json \
#     '.data.externalImportWarmupStatus.status | IN("COMPLETED","FAILED","CANCELED")' > done.json
#
# PROGRESS_JQ, when set, is a jq expression printed to stderr after each poll
# (keep it to one short line). Exit 0 when the condition holds, 3 after
# max-polls (default 240), or gql.sh's own exit status on a request error.

set -eu

[ $# -ge 3 ] && [ $# -le 5 ] || {
  echo "usage: poll.sh <operation.graphql> <variables.json> '<jq condition>' [interval-s] [max-polls]" >&2
  exit 64
}
op=$1 vars=$2 cond=$3
interval=${4:-10}
max=${5:-240}

here=$(cd "$(dirname "$0")" && pwd)
gql=${GQL:-$here/../../scryer-connect/scripts/gql.sh}
[ -x "$gql" ] || { echo "poll.sh: gql.sh not found at $gql" >&2; exit 64; }

tmp=$(mktemp "${TMPDIR:-/tmp}/scryer-poll.XXXXXX")
trap 'rm -f "$tmp"' EXIT HUP INT TERM

i=0
while [ "$i" -lt "$max" ]; do
  i=$((i + 1))
  status=0
  "$gql" "$op" "$vars" > "$tmp" || status=$?
  if [ "$status" -ne 0 ]; then
    cat "$tmp"
    exit "$status"
  fi
  if [ -n "${PROGRESS_JQ:-}" ]; then
    printf 'poll %s: %s\n' "$i" "$(jq -c "$PROGRESS_JQ" "$tmp")" >&2
  fi
  if jq -e "$cond" "$tmp" >/dev/null; then
    cat "$tmp"
    exit 0
  fi
  sleep "$interval"
done

cat "$tmp"
echo "poll.sh: condition not met after $max polls" >&2
exit 3
