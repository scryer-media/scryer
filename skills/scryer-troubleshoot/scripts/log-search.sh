#!/bin/sh
# Search the bundle's log files and print a capped, compact result.
#
# Usage:
#   log-search.sh <bundle-dir> --summary
#       Line counts by level, then the 15 most frequent targets of WARN and
#       ERROR lines.
#   log-search.sh <bundle-dir> <regex> [level] [max-lines]
#       Lines whose message or target matches <regex> (case-insensitive,
#       extended regex). level filters to TRACE|DEBUG|INFO|WARN|ERROR.
#       max-lines defaults to 40.
#
# <bundle-dir> is the unpacked directory printed by fetch-bundle.sh. Both
# logs/buffer.log and logs/file.log are read when present. JSON lines (the
# default SCRYER_LOG_FORMAT) are projected to "time level target message";
# other lines are matched as plain text.
set -eu

dir=${1:?usage: log-search.sh <bundle-dir> --summary | <regex> [level] [max-lines]}
mode=${2:?usage: log-search.sh <bundle-dir> --summary | <regex> [level] [max-lines]}
level=${3:-}
max=${4:-40}

files=""
for f in "$dir/logs/buffer.log" "$dir/logs/file.log"; do
  [ -f "$f" ] && files="$files $f"
done
if [ -z "$files" ]; then
  echo "no log files in $dir/logs (see manifest.json omissions)" >&2
  exit 1
fi

# shellcheck disable=SC2086
if [ "$mode" = "--summary" ]; then
  cat $files | jq -R -r 'fromjson? | .level // empty' | sort | uniq -c | sort -rn
  echo "top WARN/ERROR targets:"
  cat $files | jq -R -r 'fromjson? | select(.level == "WARN" or .level == "ERROR") | "\(.level) \(.target)"' |
    sort | uniq -c | sort -rn | head -n 15
  plain=$(cat $files | jq -R -r 'select(((fromjson? // null) | type) != "object")' | grep -c . || true)
  echo "non-JSON lines: $plain"
  exit 0
fi

# shellcheck disable=SC2086
cat $files | jq -R -r --arg re "$mode" --arg lvl "$level" '
  (fromjson? // null) as $j
  | if ($j | type) == "object" then
      select($lvl == "" or $j.level == $lvl)
      | select((($j.fields.message // "") + " " + ($j.target // "")) | test($re; "i"))
      | "\($j.timestamp) \($j.level) \($j.target) \($j.fields.message // "" | .[0:400])"
    else
      select(test($re; "i")) | select($lvl == "" or test($lvl)) | .[0:500]
    end
' | head -n "$max"
