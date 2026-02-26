#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="${SCRYER_REPO_DIR:-$(cd "$SCRIPT_DIR/.." && pwd)}"

NZBGET_BIN="${NZBGET_BIN:-/opt/homebrew/bin/nzbget}"
NZBGET_WEB_DIR="${NZBGET_WEB_DIR:-/opt/homebrew/opt/nzbget/share/nzbget/webui}"
NZBGET_CONFIG_TEMPLATE="${NZBGET_CONFIG_TEMPLATE:-/opt/homebrew/opt/nzbget/share/nzbget/nzbget.conf}"
NZBGET_CERT_STORE="${NZBGET_CERT_STORE:-/opt/homebrew/opt/ca-certificates/share/ca-certificates/cacert.pem}"
NZBGET_CONF="${NZBGET_CONF:-$REPO_DIR/tmp/nzbget/config/nzbget.conf}"
NZBGET_DOWNLOAD_DIR="${NZBGET_DOWNLOAD_DIR:-$REPO_DIR/tmp/nzbget/downloads}"

if ! [ -x "$NZBGET_BIN" ] && ! command -v "$NZBGET_BIN" >/dev/null 2>&1; then
  echo "NZBGet binary not found or not executable: $NZBGET_BIN" >&2
  exit 1
fi

if [ ! -f "$NZBGET_CONF" ]; then
  echo "NZBGet config file not found at $NZBGET_CONF" >&2
  echo "Copy your nzbget.conf there before starting." >&2
  exit 1
fi

mkdir -p "$NZBGET_DOWNLOAD_DIR"
echo "Starting NZBGet with $NZBGET_CONF"

command=(
  "$NZBGET_BIN"
  -D
  -o OutputMode=loggable
  -o "WebDir=$NZBGET_WEB_DIR"
  -o "ConfigTemplate=$NZBGET_CONFIG_TEMPLATE"
)

if [ -n "$NZBGET_CERT_STORE" ]; then
  command+=(-o "CertStore=$NZBGET_CERT_STORE")
fi

command+=(-c "$NZBGET_CONF")
"${command[@]}"

