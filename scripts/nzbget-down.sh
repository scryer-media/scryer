#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="${SCRYER_REPO_DIR:-$(cd "$SCRIPT_DIR/.." && pwd)}"

NZBGET_BIN="${NZBGET_BIN:-/opt/homebrew/bin/nzbget}"
NZBGET_CONF="${NZBGET_CONF:-$REPO_DIR/tmp/nzbget/config/nzbget.conf}"

MATCH_PATTERN="$NZBGET_BIN .* -c $NZBGET_CONF"

if pgrep -f "$MATCH_PATTERN" >/dev/null 2>&1; then
  pkill -f "$MATCH_PATTERN"
  echo "NZBGet stopped."
else
  echo "NZBGet is not running."
fi

