#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
COMPOSE_FILE="${SCRYER_DOCKER_COMPOSE_FILE:-$REPO_DIR/docker-compose.dev.yml}"
COMPOSE_ORCHESTRATION_SERVICE="${SCRYER_DOCKER_STACK_NAME:-scryer-dev}"
SCRYER_DOCKER_RESTART_SERVICES="${SCRYER_DOCKER_RESTART_SERVICES:-scryer nodejs proxy}"
SCRYER_DOCKER_INFRA_SERVICES="${SCRYER_DOCKER_INFRA_SERVICES:-nzbget sabnzbd weaver prometheus grafana}"
SCRYER_DOCKER_SCRYER_READY_TIMEOUT_SECONDS="${SCRYER_DOCKER_SCRYER_READY_TIMEOUT_SECONDS:-300}"
SCRYER_DOCKER_NODEJS_READY_TIMEOUT_SECONDS="${SCRYER_DOCKER_NODEJS_READY_TIMEOUT_SECONDS:-120}"
SCRYER_DOCKER_READY_POLL_SECONDS="${SCRYER_DOCKER_READY_POLL_SECONDS:-2}"

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

export SCRYER_AUTH_ENABLED="false"

mkdir -p "$REPO_DIR/tmp/scryer-config"
mkdir -p "$REPO_DIR/tmp/scryer-data"
mkdir -p "$REPO_DIR/tmp/scryer-media/movies"
mkdir -p "$REPO_DIR/tmp/scryer-media/series"
mkdir -p "$REPO_DIR/tmp/nzbget/config"
mkdir -p "$REPO_DIR/tmp/nzbget-downloads"
mkdir -p "$REPO_DIR/tmp/sabnzbd/config"
mkdir -p "$REPO_DIR/tmp/sabnzbd-downloads"
mkdir -p "$REPO_DIR/tmp/weaver/data"
mkdir -p "$REPO_DIR/tmp/weaver-downloads"

compose_cmd=(
  docker compose
  -p "$COMPOSE_ORCHESTRATION_SERVICE"
  -f "$COMPOSE_FILE"
)

contains_service() {
  local target="$1"
  shift
  local service
  for service in "$@"; do
    if [ "$service" = "$target" ]; then
      return 0
    fi
  done
  return 1
}

log_container_failure() {
  local container_name="$1"
  echo "Recent logs for ${container_name}:" >&2
  docker logs --tail 200 "$container_name" >&2 || true
}

wait_for_scryer() {
  echo "Waiting for scryer to be ready..."
  local attempts=$((SCRYER_DOCKER_SCRYER_READY_TIMEOUT_SECONDS / SCRYER_DOCKER_READY_POLL_SECONDS))
  if [ "$attempts" -lt 1 ]; then
    attempts=1
  fi
  for _ in $(seq 1 "$attempts"); do
    if curl -sf http://localhost:8080/health >/dev/null 2>&1; then
      return 0
    fi

    case "$(docker inspect --format '{{.State.Status}}' scryer 2>/dev/null || true)" in
      exited|dead)
        echo "scryer exited before it became ready." >&2
        log_container_failure scryer
        return 1
        ;;
    esac
    sleep "$SCRYER_DOCKER_READY_POLL_SECONDS"
  done

  echo "Timed out waiting for scryer to become ready." >&2
  log_container_failure scryer
  return 1
}

wait_for_nodejs() {
  echo "Waiting for nodejs to be ready..."
  local attempts=$((SCRYER_DOCKER_NODEJS_READY_TIMEOUT_SECONDS / SCRYER_DOCKER_READY_POLL_SECONDS))
  if [ "$attempts" -lt 1 ]; then
    attempts=1
  fi
  for _ in $(seq 1 "$attempts"); do
    case "$(docker inspect --format '{{.State.Status}}' scryer-nodejs 2>/dev/null || true)" in
      running)
        if docker exec scryer-nodejs sh -lc \
          'wget -q -O /dev/null http://127.0.0.1:3000' >/dev/null 2>&1; then
          return 0
        fi
        ;;
      exited|dead)
        echo "nodejs exited before it became ready." >&2
        log_container_failure scryer-nodejs
        return 1
        ;;
    esac
    sleep "$SCRYER_DOCKER_READY_POLL_SECONDS"
  done

  echo "Timed out waiting for nodejs to become ready." >&2
  log_container_failure scryer-nodejs
  return 1
}

# Recreate the Rust container on every bring-up so local testing always
# starts from a fresh Linux build tree.
"${compose_cmd[@]}" rm -sf scryer >/dev/null 2>&1 || true

compose_up() {
  local no_deps="$1"
  shift
  local -a services=("$@")
  if [ "${#services[@]}" -eq 0 ]; then
    return 0
  fi
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

restart_services=(${SCRYER_DOCKER_RESTART_SERVICES})
proxy_requested=false
non_proxy_services=()
for service in "${restart_services[@]}"; do
  if [ "$service" = "proxy" ]; then
    proxy_requested=true
  else
    non_proxy_services+=("$service")
  fi
done

compose_up 1 "${non_proxy_services[@]}"

if contains_service scryer "${restart_services[@]}" || [ "$proxy_requested" = true ]; then
  wait_for_scryer
fi

if contains_service nodejs "${restart_services[@]}" || [ "$proxy_requested" = true ]; then
  wait_for_nodejs
fi

if [ "$proxy_requested" = true ]; then
  compose_up 1 proxy
fi
