#!/usr/bin/env python3
"""Sanitize JSONL payload shapes before dictionary-corpus export.

Input and output are JSONL. Errors identify only line numbers; raw values are
never included in diagnostics. Database selection and read-only enforcement
belong to the caller so this utility remains engine-independent.
"""

import json
import re
import sys

ALLOW = {
    "movie", "series", "anime", "torrent", "usenet", "completed", "failed",
    "skipped", "queued", "running", "eligible", "blocked", "import_rejected",
    "import_completed", "release_grabbed", "release_blocklisted", "download_failed",
    "media_file_deleted", "media_file_upgraded", "upgrade_cleanup", "deleted",
    "missing_on_disk", "recycle_bin_purged", "system", "global", "title", "job_run",
    "library_scan", "download_queue_item", "WEBDL-1080p", "WEBDL-2160p",
    "Bluray-1080p", "HDTV-720p", "Parsed", "NeedsReview", "Episode", "Movie",
    "Web", "BluRay", "pending_delay", "minimum_age", "protocol_disabled",
    "quality_blocked", "episode_mismatch", "title_mismatch", "category_mismatch",
    "ambiguous_identity", "download_client_unavailable", "quality_tier",
    "preferred_protocol", "revision",
}


def number_bucket(value):
    value = abs(int(value))
    if value == 0:
        return 0
    if value < 10:
        return 1
    if value < 100:
        return 10
    if value < 1_000:
        return 100
    if value < 1_000_000:
        return 1_000
    return 1_000_000


def length_bucket(value):
    length = len(value)
    if length < 16:
        return "short"
    if length < 64:
        return "medium"
    if length < 256:
        return "long"
    return "very-long"


def sanitize(value, key="value"):
    if value is None or isinstance(value, bool):
        return value
    if isinstance(value, (int, float)):
        return number_bucket(value)
    if isinstance(value, str):
        if value in ALLOW:
            return value
        field = re.sub(r"[^a-z0-9]+", "-", key.lower()).strip("-") or "value"
        return f"<{field}:{length_bucket(value)}>"
    if isinstance(value, list):
        return [sanitize(item, key) for item in value[:8]]
    if isinstance(value, dict):
        return {str(name): sanitize(item, str(name)) for name, item in sorted(value.items())}
    return "<unsupported>"


def main():
    seen = set()
    for line_number, line in enumerate(sys.stdin, 1):
        if not line.strip():
            continue
        try:
            compact = json.dumps(
                sanitize(json.loads(line)), separators=(",", ":"), sort_keys=True
            )
        except Exception:
            print(f"invalid JSON on input line {line_number}", file=sys.stderr)
            return 1
        if compact not in seen:
            seen.add(compact)
            print(compact)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
