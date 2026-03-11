#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LINUX_TARGET="x86_64-unknown-linux-gnu"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
LINUX_IMAGE="${SCRYER_LINUX_CLIPPY_IMAGE:-rust:1.94-bookworm}"
LINUX_PLATFORM="${SCRYER_LINUX_CLIPPY_PLATFORM:-linux/amd64}"
LINUX_ONLY=false

for arg in "$@"; do
    case "$arg" in
        --linux-only) LINUX_ONLY=true ;;
        *)
            echo "error: unknown argument: $arg" >&2
            exit 1
            ;;
    esac
done

cd "$REPO_ROOT"

if ! $LINUX_ONLY; then
    echo "Running cargo clippy for host target: $HOST_TARGET"
    cargo clippy --workspace -- -D warnings
fi

if $LINUX_ONLY || [[ "$HOST_TARGET" != "$LINUX_TARGET" ]]; then
    if command -v docker >/dev/null 2>&1; then
        echo "Running cargo clippy in Linux container: $LINUX_IMAGE"
        docker run --rm \
            --platform "$LINUX_PLATFORM" \
            -v "$REPO_ROOT:/work" \
            -w /work \
            -e CARGO_HOME=/tmp/cargo \
            -e CARGO_TARGET_DIR=/tmp/target \
            -e CARGO_TERM_COLOR=always \
            "$LINUX_IMAGE" \
            bash -lc 'set -euo pipefail; /usr/local/cargo/bin/rustup component add clippy; toolchain="$("/usr/local/cargo/bin/rustup" show active-toolchain | cut -d" " -f1)"; toolchain_bin="/usr/local/rustup/toolchains/${toolchain}/bin"; export PATH="${toolchain_bin}:$PATH"; "${toolchain_bin}/cargo-clippy" clippy --workspace -- -D warnings'
    elif command -v x86_64-linux-gnu-gcc >/dev/null 2>&1; then
        echo "Ensuring Linux CI target is installed: $LINUX_TARGET"
        rustup target add "$LINUX_TARGET"

        echo "Running cargo clippy for Linux CI target: $LINUX_TARGET"
        cargo clippy --workspace --target "$LINUX_TARGET" -- -D warnings
    else
        echo "error: cannot run Linux CI clippy locally; install Docker or x86_64-linux-gnu-gcc" >&2
        exit 1
    fi
fi
