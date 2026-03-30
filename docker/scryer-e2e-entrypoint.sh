#!/bin/sh
set -eu

SCRYER_UID="${SCRYER_UID:-1000}"
SCRYER_GID="${SCRYER_GID:-1000}"

ensure_dir() {
  mkdir -p "$1"
}

if [ "$(id -u)" = "0" ]; then
  ensure_dir /data
  ensure_dir /media
  ensure_dir /media/movies
  ensure_dir /media/series
  ensure_dir /media/anime
  ensure_dir /weaver-downloads
  ensure_dir /nzbget-downloads
  ensure_dir /sabnzbd-downloads
  ensure_dir /home/scryer

  chown -R "${SCRYER_UID}:${SCRYER_GID}" \
    /data \
    /media \
    /weaver-downloads \
    /nzbget-downloads \
    /sabnzbd-downloads \
    /home/scryer

  exec setpriv \
    --reuid="${SCRYER_UID}" \
    --regid="${SCRYER_GID}" \
    --init-groups \
    /usr/local/bin/scryer "$@"
fi

exec /usr/local/bin/scryer "$@"
