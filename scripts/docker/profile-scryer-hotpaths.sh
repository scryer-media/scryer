#!/usr/bin/env bash
set -euo pipefail

container="${SCRYER_PROFILE_CONTAINER:-scryer}"
duration_seconds="${1:-20}"
interval_seconds="${2:-0.5}"
sample_depth="${SCRYER_PROFILE_SAMPLE_DEPTH:-12}"
out_dir="${SCRYER_PROFILE_OUT_DIR:-/tmp/scryer-hotpaths}"
timestamp="$(date +%Y%m%d-%H%M%S)"
run_dir="${out_dir}/${timestamp}"
raw_dir="${run_dir}/raw"
mkdir -p "${raw_dir}"

log() {
  printf '%s\n' "$*"
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require_running_container() {
  local status
  status="$(docker inspect "${container}" --format '{{.State.Status}}' 2>/dev/null || true)"
  [ "${status}" = "running" ] || fail "container ${container} is not running"
}

container_cmd() {
  docker exec "${container}" sh -lc "$1"
}

find_scryer_pid() {
  local pid
  pid="$(container_cmd "ps -eo pid,args | awk '/\\/cargo-target\\/debug\\/scryer|target\\/debug\\/scryer/ && !/awk/ { print \$1; exit }'")"
  if [ -n "${pid}" ]; then
    printf '%s\n' "${pid}"
    return
  fi

  container_cmd "ps -eo pid,args | awk '/cargo run --locked -p scryer/ && !/awk/ { print \$1; exit }'"
}

validate_tools() {
  container_cmd 'command -v gdb >/dev/null && command -v ps >/dev/null' >/dev/null \
    || fail "container is missing required tools (need gdb and ps)"
}

validate_attach() {
  local pid="$1"
  local validate_out="${run_dir}/validate-attach.txt"
  if ! container_cmd "gdb -batch -ex 'set pagination off' -ex 'thread apply all bt 2' -ex detach -ex quit -p ${pid}" >"${validate_out}" 2>&1; then
    cat "${validate_out}" >&2 || true
    fail "gdb could not attach to pid ${pid}; check SYS_PTRACE/seccomp settings"
  fi
  grep -Eq '^Thread|^#0|^#1' "${validate_out}" \
    || fail "gdb attach succeeded but did not return a usable backtrace"
}

take_sample() {
  local pid="$1"
  local sample_idx="$2"
  local sample_file="${raw_dir}/sample-$(printf '%04d' "${sample_idx}").txt"
  container_cmd "gdb -batch -ex 'set pagination off' -ex 'thread apply all bt ${sample_depth}' -ex detach -ex quit -p ${pid}" \
    >"${sample_file}" 2>&1 || true
}

summarize_samples() {
  local summary_file="${run_dir}/summary.txt"
  {
    echo "Container: ${container}"
    echo "Duration: ${duration_seconds}s"
    echo "Interval: ${interval_seconds}s"
    echo "Sample depth: ${sample_depth}"
    echo
    echo "Top sampled application frames:"
    awk '
      /^Thread / { in_thread=1; next }
      /^#([0-9]+)/ {
        line=$0
        gsub(/^#([0-9]+)[[:space:]]+/, "", line)
        if (line ~ /futex|epoll|poll|__lll_lock_wait|clone|start_thread|__GI___/ ) next
        if (line ~ /libpthread|libc\.so|linux-vdso|ld-linux|tokio::runtime::park|std::thread::/) next
        counts[line]++
      }
      END {
        for (line in counts) {
          printf "%7d  %s\n", counts[line], line
        }
      }
    ' "${raw_dir}"/sample-*.txt | sort -nr | head -n 40
    echo
    echo "Raw samples: ${raw_dir}"
  } >"${summary_file}"
}

main() {
  require_running_container
  validate_tools

  local pid
  pid="$(find_scryer_pid)"
  [ -n "${pid}" ] || fail "could not find a running scryer pid inside ${container}"

  log "Validating debugger attach to ${container} pid ${pid}..."
  validate_attach "${pid}"

  log "Sampling ${container} pid ${pid} every ${interval_seconds}s for ${duration_seconds}s..."
  local deadline
  deadline="$(python3 - <<'PY'
import time
print(time.time())
PY
)"
  deadline="$(python3 - <<PY
import time
print(${deadline} + float(${duration_seconds}))
PY
)"

  local sample_idx=0
  while python3 - <<PY
import time
raise SystemExit(0 if time.time() < float(${deadline}) else 1)
PY
  do
    sample_idx=$((sample_idx + 1))
    take_sample "${pid}" "${sample_idx}"
    sleep "${interval_seconds}"
  done

  summarize_samples
  log "Profile complete."
  log "Summary: ${run_dir}/summary.txt"
  log "Raw samples: ${raw_dir}"
}

main "$@"
