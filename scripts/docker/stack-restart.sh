#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
COMPOSE_FILE="${SCRYER_DOCKER_COMPOSE_FILE:-$REPO_DIR/docker-compose.dev.yml}"
COMPOSE_ORCHESTRATION_SERVICE="${SCRYER_DOCKER_STACK_NAME:-scryer-dev}"
SCRYER_DOCKER_RESTART_SERVICES="${SCRYER_DOCKER_RESTART_SERVICES:-nzbget sabnzbd scryer nodejs proxy}"

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required to run this command." >&2
  exit 1
fi

if ! docker compose version >/dev/null 2>&1; then
  echo "docker compose is required to run this command." >&2
  exit 1
fi

if [ ! -f "$COMPOSE_FILE" ]; then
  echo "Compose file not found: $COMPOSE_FILE" >&2
  exit 1
fi

compose_cmd=(
  docker compose
  -p "$COMPOSE_ORCHESTRATION_SERVICE"
  -f "$COMPOSE_FILE"
)

if [ "$#" -gt 0 ]; then
  services=("$@")
else
  services=(${SCRYER_DOCKER_RESTART_SERVICES})
fi

if [ "${#services[@]}" -eq 0 ]; then
  echo "No services specified to restart." >&2
  exit 1
fi

"${compose_cmd[@]}" stop "${services[@]}"
"${compose_cmd[@]}" rm -f "${services[@]}"

up_args=("${compose_cmd[@]}" up -d --build --no-deps)
up_args+=("${services[@]}")

"${up_args[@]}"
