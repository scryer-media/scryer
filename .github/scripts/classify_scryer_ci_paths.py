"""Classify changed paths for the Scryer GitHub Actions workflow."""

from __future__ import annotations

import argparse
import sys
from collections.abc import Iterable


NONE = "none"
WEB = "web"
FULL = "full"

DOCUMENTATION_PREFIXES = ("docs/", "release-notes/")
WEB_PREFIX = "apps/scryer-web/"
XTASK_PREFIXES = (
    "xtask/",
    "xtask-release/",
    "xtask-migrations/",
    "xtask-support/",
    "xtask-trash-guides/",
)
XTASK_SHARED_INPUTS = {
    "Cargo.lock",
    "Cargo.toml",
    "rust-toolchain.toml",
}


def is_documentation_path(path: str) -> bool:
    return path.endswith(".md") or path.startswith(DOCUMENTATION_PREFIXES)


def classify_paths(paths: Iterable[str]) -> str:
    """Return the minimum safe validation scope for the supplied changed paths."""
    normalized_paths = [path.strip() for path in paths if path.strip()]
    if not normalized_paths:
        return FULL

    non_documentation_paths = [
        path for path in normalized_paths if not is_documentation_path(path)
    ]
    if not non_documentation_paths:
        return NONE

    if all(path.startswith(WEB_PREFIX) for path in non_documentation_paths):
        return WEB

    # Unknown paths intentionally require the complete pipeline. This covers
    # Rust, Cargo/toolchain, packaging, embedded built-ins, and CI changes
    # without relying on an allowlist that can become stale.
    return FULL


def requires_xtask_validation(paths: Iterable[str]) -> bool:
    """Return whether a change can alter the xtask test targets or inputs."""
    return any(
        path.startswith(XTASK_PREFIXES) or path in XTASK_SHARED_INPUTS
        for path in (path.strip() for path in paths)
    )


def read_paths() -> list[str]:
    raw_paths = sys.stdin.buffer.read()
    if b"\0" in raw_paths:
        return [path.decode("utf-8", "surrogateescape") for path in raw_paths.split(b"\0")]
    return raw_paths.decode("utf-8", "surrogateescape").splitlines()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--paths-file",
        type=argparse.FileType("rb"),
        help="Read NUL-delimited paths from this file instead of standard input.",
    )
    parser.add_argument(
        "--output",
        choices=("scope", "xtask"),
        default="scope",
        help="Select the requested CI classification result.",
    )
    args = parser.parse_args()

    if args.paths_file is None:
        paths = read_paths()
    else:
        raw_paths = args.paths_file.read()
        paths = [
            path.decode("utf-8", "surrogateescape")
            for path in raw_paths.split(b"\0")
        ]

    if args.output == "scope":
        print(classify_paths(paths))
    else:
        print(str(requires_xtask_validation(paths)).lower())


if __name__ == "__main__":
    main()
