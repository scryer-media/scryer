#!/bin/sh
set -e

# If not running as root (e.g. --user flag), skip privilege setup
# and just exec the binary directly.
if [ "$(id -u)" -ne 0 ]; then
    exec /usr/local/bin/scryer "$@"
fi

PUID=${PUID:-1000}
PGID=${PGID:-1000}

# Ensure data directory ownership matches the requested user
chown "$PUID":"$PGID" /data

echo "
───────────────────────────────────
  scryer
  User UID:  $PUID
  User GID:  $PGID
───────────────────────────────────
"

exec su-exec "$PUID":"$PGID" /usr/local/bin/scryer "$@"
