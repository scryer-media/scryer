#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
COMPOSE_FILE="${SCRYER_DOCKER_COMPOSE_FILE:-$REPO_DIR/docker-compose.dev.yml}"
COMPOSE_ORCHESTRATION_SERVICE="${SCRYER_DOCKER_STACK_NAME:-scryer-dev}"
SCRYER_DOCKER_RESTART_SERVICES="${SCRYER_DOCKER_RESTART_SERVICES:-nzbget sabnzbd weaver scryer nodejs proxy prometheus grafana}"
NO_SEED=false

while [ "$#" -gt 0 ]; do
  case "$1" in
    --no-seed) NO_SEED=true; shift ;;
    *) break ;;
  esac
done

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
"${compose_cmd[@]}" pull --ignore-buildable "${services[@]}"

# Clear download and import directories so each restart begins clean
echo "Cleaning download and import directories..."
rm -rf "$REPO_DIR/tmp/nzbget-downloads/"*
rm -rf "$REPO_DIR/tmp/sabnzbd-downloads/"*
rm -rf "$REPO_DIR/tmp/weaver-downloads/"*
rm -rf "$REPO_DIR/tmp/weaver/data/intermediate/"*
rm -rf "$REPO_DIR/tmp/weaver/data/complete/"*
rm -rf "$REPO_DIR/tmp/scryer-media/"*

up_args=("${compose_cmd[@]}" up -d --build --no-deps)
up_args+=("${services[@]}")

"${up_args[@]}"

# Run the seed sidecar after scryer is healthy (unless --no-seed)
if [ "$NO_SEED" = false ] && [ -f "$REPO_DIR/dev-seed.json" ]; then
  echo "Waiting for scryer to be ready..."
  for i in $(seq 1 60); do
    if curl -sf http://localhost:8080/health >/dev/null 2>&1; then
      break
    fi
    sleep 2
  done
  "${compose_cmd[@]}" --profile seed rm -f seed 2>/dev/null || true
  "${compose_cmd[@]}" --profile seed up -d seed
fi
