#!/bin/sh
# Count occurrences of operator-named sensitive terms in files about to be
# shared. Prints term numbers and per-file counts only; never the terms or
# the matching text.
#
# Usage: scan-bundle.sh <terms-file> <file-or-dir>...
#   <terms-file> holds one term per line; blank lines are ignored. Matching
#   is case-insensitive and literal (not a regex).
#
# Also counts these built-in patterns, reported as builtin:<name>:
#   api_key_prefix  ska_ followed by key characters
#   bearer_token    "Bearer " followed by a token
#   email           something@something.tld
#   ipv4            dotted quad
#
# Exit codes: 0 no matches, 1 at least one match, 2 usage error.
set -eu

terms=${1:?usage: scan-bundle.sh <terms-file> <file-or-dir>...}
shift
[ "$#" -gt 0 ] || { echo "usage: scan-bundle.sh <terms-file> <file-or-dir>..." >&2; exit 2; }
[ -f "$terms" ] || { echo "terms file not found" >&2; exit 2; }

list=$(find "$@" -type f 2>/dev/null | sort)
[ -n "$list" ] || { echo "no files to scan" >&2; exit 2; }

found=0

report() {
  # $1 label, then reads per-file counts from stdin as "file count"
  line=""
  while IFS=' ' read -r file count; do
    [ "$count" -gt 0 ] || continue
    line="$line $file=$count"
  done
  if [ -n "$line" ]; then
    echo "$1:$line"
    return 1
  fi
  return 0
}

n=0
while IFS= read -r term || [ -n "$term" ]; do
  [ -n "$term" ] || continue
  n=$((n + 1))
  printf '%s\n' "$list" | while IFS= read -r f; do
    c=$(printf '%s\n' "$term" | grep -F -i -c -f - "$f" 2>/dev/null || true)
    echo "$f ${c:-0}"
  done | report "term $n" || found=1
done < "$terms"

scan_builtin() {
  name=$1
  re=$2
  printf '%s\n' "$list" | while IFS= read -r f; do
    c=$(grep -E -c -- "$re" "$f" 2>/dev/null || true)
    echo "$f ${c:-0}"
  done | report "builtin:$name" || found=1
}

scan_builtin api_key_prefix 'ska_[A-Za-z0-9_-]{8,}'
scan_builtin bearer_token '[Bb]earer [A-Za-z0-9._~+/=-]{12,}'
scan_builtin email '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}'
scan_builtin ipv4 '(^|[^0-9.])([0-9]{1,3}\.){3}[0-9]{1,3}([^0-9.]|$)'

echo "scanned $(printf '%s\n' "$list" | wc -l | tr -d ' ') files, $n terms"
if [ "$found" -eq 0 ]; then
  echo "no matches"
  exit 0
fi
exit 1
