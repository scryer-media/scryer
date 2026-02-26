#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
COMPOSE_FILE="${SCRYER_DOCKER_COMPOSE_FILE:-$REPO_DIR/docker-compose.dev.yml}"
COMPOSE_ORCHESTRATION_SERVICE="${SCRYER_DOCKER_STACK_NAME:-scryer-dev}"
SCRYER_DOCKER_RESTART_SERVICES="${SCRYER_DOCKER_RESTART_SERVICES:-scryer nodejs proxy}"
SCRYER_DOCKER_INFRA_SERVICES="${SCRYER_DOCKER_INFRA_SERVICES:-nzbget}"

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

export SCRYER_DOCKER_DEV_AUTO_LOGIN="true"
export SCRYER_DEV_AUTO_LOGIN="true"

mkdir -p "$REPO_DIR/tmp/scryer-data"
mkdir -p "$REPO_DIR/tmp/scryer-media/movies"
mkdir -p "$REPO_DIR/tmp/scryer-media/series"
mkdir -p "$REPO_DIR/tmp/scryer-media/completed"
mkdir -p "$REPO_DIR/tmp/nzbget/config"

if [ ! -f "$REPO_DIR/tmp/nzbget/config/nzbget.conf" ]; then
  echo "Expected NZBGet config at $REPO_DIR/tmp/nzbget/config/nzbget.conf." >&2
  echo "Copy a working nzbget.conf there before continuing." >&2
  exit 1
fi

compose_cmd=(
  docker compose
  -p "$COMPOSE_ORCHESTRATION_SERVICE"
  -f "$COMPOSE_FILE"
)

compose_up() {
  local no_deps="$1"
  shift
  local -a services=("$@")
  local -a args=("${compose_cmd[@]}" up -d --remove-orphans)

  if [ "$no_deps" = "1" ]; then
    args+=(--no-deps)
  fi
  args+=("${services[@]}")

  "${args[@]}"
}

if [ "${SCRYER_DOCKER_FORCE_INFRA_RESTART:-0}" = "1" ]; then
  compose_up 0 ${SCRYER_DOCKER_INFRA_SERVICES}
else
  running_services=$("${compose_cmd[@]}" ps --services --filter status=running)
  to_start=()
  for service in ${SCRYER_DOCKER_INFRA_SERVICES}; do
    if ! printf '%s\n' $running_services | grep -Fxq "$service"; then
      to_start+=("$service")
    fi
  done

  if [ "${#to_start[@]}" -gt 0 ]; then
    compose_up 0 "${to_start[@]}"
  fi
fi

compose_up 1 ${SCRYER_DOCKER_RESTART_SERVICES}
