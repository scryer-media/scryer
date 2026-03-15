#!/bin/sh
set -e

# If not running as root (e.g. --user flag), skip privilege setup
# and just exec the binary directly.
if [ "$(id -u)" -ne 0 ]; then
    exec /usr/local/bin/scryer "$@"
fi

PUID=${PUID:-1000}
PGID=${PGID:-1000}

# Derive the database directory from SCRYER_DB_PATH so we chown the right
# location regardless of whether the user overrides the default /data path.
DB_PATH="${SCRYER_DB_PATH:-/data/scryer.db}"
DB_PATH="${DB_PATH#sqlite://}"   # strip scheme prefix
DB_PATH="${DB_PATH%%\?*}"        # strip query params
DB_DIR="$(dirname "$DB_PATH")"

# Ensure the database directory (and any existing files) are owned by the
# requested user.  Covers upgrades from older images that used a different uid.
if [ -d "$DB_DIR" ]; then
    chown -R "$PUID":"$PGID" "$DB_DIR"
fi

echo "
───────────────────────────────────
  scryer
  User UID:  $PUID
  User GID:  $PGID
───────────────────────────────────
"

exec su-exec "$PUID":"$PGID" /usr/local/bin/scryer "$@"
