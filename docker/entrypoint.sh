#!/bin/sh
set -e

# If not running as root (e.g. --user flag), skip privilege setup
# and just exec the binary directly.
if [ "$(id -u)" -ne 0 ]; then
    exec /usr/local/bin/scryer "$@"
fi

PUID=${PUID:-1000}
PGID=${PGID:-1000}

# ── Migrate from /data to /config ────────────────────────────────────────────
# Previous images stored the database in /data. If the user hasn't overridden
# SCRYER_DB_PATH and the old database exists but the new location doesn't,
# move it automatically so existing installs upgrade seamlessly.
if [ "${SCRYER_DB_PATH}" = "/config/scryer.db" ] || [ -z "${SCRYER_DB_PATH}" ]; then
    if [ -f /data/scryer.db ] && [ ! -f /config/scryer.db ]; then
        echo "  Migrating database from /data/scryer.db to /config/scryer.db"
        mkdir -p /config
        cp /data/scryer.db /config/scryer.db
        # Copy WAL/SHM if present
        [ -f /data/scryer.db-wal ] && cp /data/scryer.db-wal /config/scryer.db-wal
        [ -f /data/scryer.db-shm ] && cp /data/scryer.db-shm /config/scryer.db-shm
        echo "  Migration complete. You can remove /data/scryer.db after verifying."
    fi
fi

# Derive the database directory from SCRYER_DB_PATH so we chown the right
# location regardless of whether the user overrides the default path.
DB_PATH="${SCRYER_DB_PATH:-/config/scryer.db}"
DB_PATH="${DB_PATH#sqlite://}"   # strip scheme prefix
DB_PATH="${DB_PATH%%\?*}"        # strip query params
DB_DIR="$(dirname "$DB_PATH")"

# Ensure /config and the database directory are owned by the requested user.
mkdir -p /config
chown -R "$PUID":"$PGID" /config
if [ -d "$DB_DIR" ] && [ "$DB_DIR" != "/config" ]; then
    chown -R "$PUID":"$PGID" "$DB_DIR"
fi

echo "
───────────────────────────────────
  scryer
  User UID:  $PUID
  User GID:  $PGID
  Config:    /config
───────────────────────────────────
"

exec su-exec "$PUID":"$PGID" /usr/local/bin/scryer "$@"
