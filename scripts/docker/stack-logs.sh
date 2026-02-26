#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
COMPOSE_FILE="${SCRYER_DOCKER_COMPOSE_FILE:-$REPO_DIR/docker-compose.dev.yml}"
COMPOSE_ORCHESTRATION_SERVICE="${SCRYER_DOCKER_STACK_NAME:-scryer-dev}"
DEFAULT_SERVICE="${SCRYER_DOCKER_LOG_SERVICE:-scryer}"
SCRYER_STACK_LINES="${SCRYER_STACK_LINES:-200}"

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

service="${1:-$DEFAULT_SERVICE}"

if ! [[ "$SCRYER_STACK_LINES" =~ ^[0-9]+$ ]]; then
  echo "SCRYER_STACK_LINES must be numeric." >&2
  exit 1
fi

docker compose -p "$COMPOSE_ORCHESTRATION_SERVICE" -f "$COMPOSE_FILE" logs "$service" -n "$SCRYER_STACK_LINES" -f
