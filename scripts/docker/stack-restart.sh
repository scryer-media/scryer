#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
COMPOSE_FILE="${SCRYER_DOCKER_COMPOSE_FILE:-$REPO_DIR/docker-compose.dev.yml}"
COMPOSE_ORCHESTRATION_SERVICE="${SCRYER_DOCKER_STACK_NAME:-scryer-dev}"
SCRYER_DOCKER_RESTART_SERVICES="${SCRYER_DOCKER_RESTART_SERVICES:-nzbget sabnzbd weaver scryer nodejs proxy prometheus grafana}"
SCRYER_DOCKER_SCRYER_READY_TIMEOUT_SECONDS="${SCRYER_DOCKER_SCRYER_READY_TIMEOUT_SECONDS:-300}"
SCRYER_DOCKER_NODEJS_READY_TIMEOUT_SECONDS="${SCRYER_DOCKER_NODEJS_READY_TIMEOUT_SECONDS:-120}"
SCRYER_DOCKER_READY_POLL_SECONDS="${SCRYER_DOCKER_READY_POLL_SECONDS:-2}"
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

start_services() {
  local no_deps="$1"
  shift
  local -a selected_services=("$@")
  if [ "${#selected_services[@]}" -eq 0 ]; then
    return 0
  fi

  local -a up_args=("${compose_cmd[@]}" up -d --build)
  if [ "$no_deps" = "1" ]; then
    up_args+=(--no-deps)
  fi
  up_args+=("${selected_services[@]}")
  "${up_args[@]}"
}

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

mkdir -p "$REPO_DIR/tmp/scryer-config"
mkdir -p "$REPO_DIR/tmp/scryer-data"
mkdir -p "$REPO_DIR/tmp/scryer-media/movies"
mkdir -p "$REPO_DIR/tmp/scryer-media/series"

proxy_requested=false
non_proxy_services=()
for service in "${services[@]}"; do
  if [ "$service" = "proxy" ]; then
    proxy_requested=true
  else
    non_proxy_services+=("$service")
  fi
done

start_services 1 "${non_proxy_services[@]}"

if contains_service scryer "${services[@]}" || [ "$proxy_requested" = true ]; then
  wait_for_scryer
fi

if contains_service nodejs "${services[@]}" || [ "$proxy_requested" = true ]; then
  wait_for_nodejs
fi

if [ "$proxy_requested" = true ]; then
  start_services 1 proxy
fi

# Run the seed sidecar after scryer is healthy (unless --no-seed)
if [ "$NO_SEED" = false ] && [ -f "$REPO_DIR/dev-seed.json" ]; then
  wait_for_scryer
  "${compose_cmd[@]}" --profile seed rm -f seed 2>/dev/null || true
  "${compose_cmd[@]}" --profile seed up -d seed
fi
