#!/bin/sh
# Download the Scryer diagnostics bundle to a file, unpack it into a new empty
# directory and print a short manifest summary.
#
# Usage: fetch-bundle.sh [output-dir]
#   output-dir defaults to ./scryer-diagnostics-<local timestamp>. It must not
#   exist yet, or must be empty. Nothing outside it is written or removed.
#
# Needs SCRYER_URL and SCRYER_API_KEY in the environment, plus curl, jq, zstd
# and tar. The API key is passed to curl on stdin, never on the command line,
# and is never printed.
#
# Exit codes: 0 ok, 2 usage or environment, 3 refused by Scryer, 4 bad download.
set -eu

: "${SCRYER_URL:?SCRYER_URL is not set; complete scryer-connect first}"
: "${SCRYER_API_KEY:?SCRYER_API_KEY is not set; complete scryer-connect first}"

for tool in curl jq zstd tar od; do
  command -v "$tool" >/dev/null 2>&1 || { echo "missing tool: $tool" >&2; exit 2; }
done

out_dir=${1:-./scryer-diagnostics-$(date '+%Y%m%d-%H%M%S')}
if [ -e "$out_dir" ]; then
  if [ -n "$(ls -A "$out_dir" 2>/dev/null)" ]; then
    echo "refusing: $out_dir exists and is not empty; pick a new directory" >&2
    exit 2
  fi
else
  mkdir -p "$out_dir"
fi

archive="$out_dir/bundle.tar.zst"
unpacked="$out_dir/bundle"

status=$(printf 'Authorization: Bearer %s\n' "$SCRYER_API_KEY" |
  curl --silent --show-error --location \
    --header @- \
    --output "$archive" \
    --write-out '%{http_code}' \
    "$SCRYER_URL/api/diagnostics/bundle") || {
  echo "request failed before Scryer answered (network, TLS or URL problem)" >&2
  exit 4
}

if [ "$status" != "200" ]; then
  reason=$(jq -r '.reason // "none"' "$archive" 2>/dev/null || echo "unreadable")
  echo "refused: http=$status reason=$reason" >&2
  case "$reason" in
    HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED)
      echo "the key or session was not accepted; re-run scryer-connect" >&2 ;;
    HTTP_DIAGNOSTICS_PERMISSION_DENIED)
      echo "the key's user lacks the Manage system settings permission" >&2 ;;
    HTTP_DIAGNOSTICS_BUNDLE_FAILED)
      echo "Scryer could not build the bundle; see the service log" >&2 ;;
  esac
  exit 3
fi

magic=$(od -An -tx1 -N4 "$archive" | tr -d ' \n')
if [ "$magic" != "28b52ffd" ]; then
  echo "download is not a zstd archive (got $magic); check SCRYER_URL and any reverse proxy" >&2
  exit 4
fi

mkdir -p "$unpacked"
zstd -dc "$archive" | tar -xf - -C "$unpacked"

echo "archive: $archive"
echo "unpacked: $unpacked"
jq -r '
  "format_version: \(.format_version)",
  "scryer_version: \(.scryer_version)",
  "generated_at: \(.generated_at)",
  "files:", (.files[] | "  \(.name) \(.bytes)"),
  "omissions:", ((.omissions // [])[] | "  \(.item): \(.reason)"),
  "truncations:", ((.truncations // [])[] | "  \(.item): \(.reason) dropped_lines=\(.dropped_lines)"),
  "sanitization: best_effort=\(.sanitization.best_effort) path_policy=\(.sanitization.path_policy) known_values=\(.sanitization.known_values)"
' "$unpacked/manifest.json"
