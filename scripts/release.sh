#!/usr/bin/env bash
#
# release.sh — pre-release validation and tagging script
#
# Validates: web audit/lint/build and Rust fmt/audit/nextest/clippy in parallel
# Then:      bumps crates/scryer version · signed tag · push
#
# Usage:
#   ./scripts/release.sh              # auto-increment patch (0.0.2 → 0.0.3)
#   ./scripts/release.sh --minor      # increment minor
#   ./scripts/release.sh --major      # increment major
#   ./scripts/release.sh 0.1.0        # explicit version
#   ./scripts/release.sh --dry-run    # validate only, no commit/tag/push
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRYER_CRATE_TOML="$REPO_ROOT/crates/scryer/Cargo.toml"
WEB_DIR="$REPO_ROOT/apps/scryer-web"
KEEP_RELEASES=4  # keep this many old releases; after new release → 5 total

# ── Colors ─────────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; BOLD='\033[1m'; RESET='\033[0m'

step() { echo -e "\n${BLUE}${BOLD}▶  $*${RESET}"; }
ok()   { echo -e "   ${GREEN}✓  $*${RESET}"; }
warn() { echo -e "   ${YELLOW}⚠  $*${RESET}"; }
die()  { echo -e "\n${RED}${BOLD}✗  $*${RESET}" >&2; exit 1; }

# ── Argument parsing ───────────────────────────────────────────────────────────
BUMP="patch"
EXPLICIT_VERSION=""
DRY_RUN=false

for arg in "$@"; do
    case "$arg" in
        --major)   BUMP="major" ;;
        --minor)   BUMP="minor" ;;
        --patch)   BUMP="patch" ;;
        --dry-run) DRY_RUN=true ;;
        v[0-9]*.[0-9]*.[0-9]*) EXPLICIT_VERSION="${arg#v}" ;;
        [0-9]*.[0-9]*.[0-9]*)  EXPLICIT_VERSION="$arg" ;;
        *) die "Unknown argument: $arg" ;;
    esac
done

# ── Determine next version ─────────────────────────────────────────────────────
step "Determining next version"

cd "$REPO_ROOT"

LATEST_TAG="$(git tag --sort=-version:refname | grep '^scryer-v' | head -1 || true)"
CURRENT_VERSION="${LATEST_TAG#scryer-v}"
CURRENT_VERSION="${CURRENT_VERSION:-0.0.0}"

if [[ -n "$EXPLICIT_VERSION" ]]; then
    NEXT_VERSION="$EXPLICIT_VERSION"
else
    IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT_VERSION"
    case "$BUMP" in
        major) NEXT_VERSION="$((MAJOR + 1)).0.0" ;;
        minor) NEXT_VERSION="${MAJOR}.$((MINOR + 1)).0" ;;
        patch) NEXT_VERSION="${MAJOR}.${MINOR}.$((PATCH + 1))" ;;
    esac
fi

TAG_NAME="scryer-v${NEXT_VERSION}"

echo "   Latest tag : ${LATEST_TAG:-none}"
echo "   Next tag   : ${TAG_NAME}"
$DRY_RUN && echo -e "   ${YELLOW}(dry run — no commits, tags, or pushes)${RESET}"

# ── Pre-flight checks ──────────────────────────────────────────────────────────
step "Pre-flight checks"

if git tag | grep -qx "$TAG_NAME"; then
    die "Tag $TAG_NAME already exists"
fi

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
echo "   Branch : $BRANCH"

if [[ -n "$(git status --porcelain)" ]]; then
    warn "Working tree has uncommitted changes:"
    git status --short | sed 's/^/     /'
    echo ""
    read -r -p "   Continue anyway? [y/N] " REPLY
    [[ "$REPLY" =~ ^[Yy]$ ]] || die "Aborted"
fi

ok "Pre-flight OK"

run_web_validation() {
    step "Running npm audit fix"

    cd "$WEB_DIR"
    npm audit fix 2>&1
    ok "npm audit fix complete"

    step "Running TypeScript type check"

    npm run lint 2>&1 || die "TypeScript type check failed — fix before releasing"

    ok "TypeScript type check passed"

    step "Running web build"

    SCRYER_GRAPHQL_URL=/graphql \
    SCRYER_METADATA_GATEWAY_GRAPHQL_URL=https://smg.scryer.media/graphql \
        npm run build 2>&1 || die "Web build failed — fix before releasing"

    ok "Web build passed"
}

run_rust_validation() {
    step "Running cargo fmt --all --check"

    cd "$REPO_ROOT"
    cargo fmt --all --check 2>&1 || die "Rust formatting drift detected — fix before releasing"

    ok "cargo fmt passed"

    step "Updating Cargo.lock (cargo update)"

    cargo update 2>&1
    ok "Cargo.lock updated"

    step "Running cargo audit"

    if ! command -v cargo-audit &>/dev/null; then
        warn "cargo-audit not installed — installing"
        cargo install --locked cargo-audit 2>&1 || die "failed to install cargo-audit"
    fi

    CARGO_AUDIT_IGNORES=(
        # sqlx macros still pull the MySQL backend into Cargo.lock even though Scryer only uses SQLite.
        "RUSTSEC-2023-0071"
        # extism 1.13.0 still pins wasmtime 37.x upstream; no fixed extism release is available yet.
        "RUSTSEC-2026-0006"
        "RUSTSEC-2026-0020"
        "RUSTSEC-2026-0021"
    )

    if [[ ${#CARGO_AUDIT_IGNORES[@]} -gt 0 ]]; then
        warn "Ignoring advisories pending upstream fixes: ${CARGO_AUDIT_IGNORES[*]}"
    fi

    CARGO_AUDIT_ARGS=()
    for advisory in "${CARGO_AUDIT_IGNORES[@]}"; do
        CARGO_AUDIT_ARGS+=(--ignore "$advisory")
    done

    cargo audit "${CARGO_AUDIT_ARGS[@]}" 2>&1 || die "cargo audit found vulnerabilities — fix before releasing"
    ok "cargo audit passed"

    step "Running Rust tests (cargo nextest run --workspace --locked)"

    if ! command -v cargo-nextest &>/dev/null; then
        warn "cargo-nextest not installed — installing"
        cargo install --locked cargo-nextest 2>&1 || die "failed to install cargo-nextest"
    fi

    cargo nextest run --workspace --locked 2>&1 || die "Rust tests failed — fix before releasing"

    ok "Rust tests passed"

    step "Running cargo clippy (linux ci target)"

    "$REPO_ROOT/scripts/clippy-ci.sh" --linux-only 2>&1 || die "Clippy errors — fix before releasing"

    ok "Clippy passed"
}

# ── Release group database validation (AI-assisted, monthly) ─────────────────
step "Validating release group database"

CLAUDE_VALIDATION_STAMP="$REPO_ROOT/.claude/release-validation-timestamp"
CLAUDE_VALIDATION_INTERVAL=$((30 * 86400))  # 30 days in seconds
CLAUDE_VALIDATION_DUE=true

if [[ -f "$CLAUDE_VALIDATION_STAMP" ]]; then
    LAST_RUN="$(cat "$CLAUDE_VALIDATION_STAMP")"
    LAST_EPOCH="$(date -j -f "%Y-%m-%dT%H:%M:%S" "${LAST_RUN%%[-+]*}" "+%s" 2>/dev/null || echo 0)"
    NOW_EPOCH="$(date "+%s")"
    ELAPSED=$(( NOW_EPOCH - LAST_EPOCH ))
    DAYS_AGO=$(( ELAPSED / 86400 ))
    if [[ $ELAPSED -lt $CLAUDE_VALIDATION_INTERVAL ]]; then
        CLAUDE_VALIDATION_DUE=false
        ok "Last validated ${DAYS_AGO}d ago — skipping (runs monthly)"
    else
        echo "   Last validated ${DAYS_AGO}d ago — due for re-validation"
    fi
else
    echo "   No previous validation found — running"
fi

PROMPT_FILE="$REPO_ROOT/scripts/prompts/validate-release-data.md"
if $CLAUDE_VALIDATION_DUE; then
    if [[ -f "$PROMPT_FILE" ]] && command -v claude &>/dev/null; then
        echo "   Spawning Claude to validate release group data..."
        if CLAUDECODE= claude -p "$(cat "$PROMPT_FILE")" \
            --model claude-opus-4-6 \
            --max-turns 30 \
            --allowedTools "Read,Edit,Write,Glob,Grep,Bash(cargo nextest*),Bash(ls*),WebFetch,WebSearch" \
            2>&1; then
            mkdir -p "$(dirname "$CLAUDE_VALIDATION_STAMP")"
            date -u "+%Y-%m-%dT%H:%M:%S%z" > "$CLAUDE_VALIDATION_STAMP"
            ok "Release group validation complete"
        else
            warn "Release group validation encountered errors — review changes manually"
        fi
    else
        if ! command -v claude &>/dev/null; then
            warn "claude CLI not found — skipping release group validation"
        else
            warn "Prompt file not found at $PROMPT_FILE — skipping"
        fi
    fi
fi

step "Running web and Rust validation in parallel"

(
    exec > >(sed 's/^/[web] /') 2>&1
    run_web_validation
) &
WEB_VALIDATION_PID=$!

(
    exec > >(sed 's/^/[rust] /') 2>&1
    run_rust_validation
) &
RUST_VALIDATION_PID=$!

VALIDATION_FAILED=false

if ! wait "$WEB_VALIDATION_PID"; then
    VALIDATION_FAILED=true
    warn "Web validation failed"
fi

if ! wait "$RUST_VALIDATION_PID"; then
    VALIDATION_FAILED=true
    warn "Rust validation failed"
fi

if [[ "$VALIDATION_FAILED" == true ]]; then
    die "Validation failed — fix before releasing"
fi

ok "Parallel validation passed"

# ── Bump all workspace crate versions ──────────────────────────────────────────
step "Updating all workspace crate versions to $NEXT_VERSION"

cd "$REPO_ROOT"

# Collect all member Cargo.toml files from the workspace
WORKSPACE_TOMLS=()
while IFS= read -r member; do
    toml="$REPO_ROOT/$member/Cargo.toml"
    [[ -f "$toml" ]] && WORKSPACE_TOMLS+=("$toml")
done < <(grep '^\s*"' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')

[[ ${#WORKSPACE_TOMLS[@]} -eq 0 ]] && die "No workspace member Cargo.toml files found"

for toml in "${WORKSPACE_TOMLS[@]}"; do
    sed -i '' 's/^version = "[^"]*"/version = "'"$NEXT_VERSION"'"/' "$toml"
    name="$(basename "$(dirname "$toml")")"
    echo "   bumped: $name → $NEXT_VERSION"
done

# Verify the main binary got updated
WRITTEN_VERSION="$(grep -m1 '^version = ' "$SCRYER_CRATE_TOML" | sed 's/.*"\(.*\)".*/\1/')"
[[ "$WRITTEN_VERSION" == "$NEXT_VERSION" ]] \
    || die "Version write failed — $SCRYER_CRATE_TOML shows: $WRITTEN_VERSION"

ok "${#WORKSPACE_TOMLS[@]} crates updated to $NEXT_VERSION"

# ── Verify build after bump ────────────────────────────────────────────────────
step "Running cargo check after version bump"

cargo check 2>&1 || die "cargo check failed after version bump"

ok "cargo check passed"

# ── From here on nothing destructive happens in dry-run mode ──────────────────
if $DRY_RUN; then
    echo ""
    echo -e "${YELLOW}${BOLD}Dry run complete — stopping before commit/tag/push.${RESET}"
    echo -e "  Version $NEXT_VERSION validated OK."
    # Restore any changes so the working tree is clean
    git checkout -- "${WORKSPACE_TOMLS[@]}"
    git checkout -- "$REPO_ROOT/Cargo.lock" 2>/dev/null || true
    git checkout -- "$WEB_DIR/package-lock.json" 2>/dev/null || true
    exit 0
fi

# ── Commit version bump ────────────────────────────────────────────────────────
step "Committing version bump"

CHANGED_FILES=()
for toml in "${WORKSPACE_TOMLS[@]}"; do
    [[ -n "$(git diff --name-only "$toml")" ]] && CHANGED_FILES+=("$toml")
done
CARGO_LOCK="$REPO_ROOT/Cargo.lock"
[[ -n "$(git diff --name-only "$CARGO_LOCK")" ]] && CHANGED_FILES+=("$CARGO_LOCK")
NPM_LOCK="$WEB_DIR/package-lock.json"
[[ -n "$(git diff --name-only "$NPM_LOCK")" ]] && CHANGED_FILES+=("$NPM_LOCK")

if [[ ${#CHANGED_FILES[@]} -gt 0 ]]; then
    git add "${CHANGED_FILES[@]}"
    git commit -m "release: bump scryer to $NEXT_VERSION"
    ok "Committed: ${CHANGED_FILES[*]##*/}"
else
    ok "Nothing to commit"
fi

# ── Prune old releases and artifacts ──────────────────────────────────────────
step "Pruning old releases and artifacts (keeping $KEEP_RELEASES most recent)"

RELEASES_TO_DELETE=()
while IFS=$'\t' read -r TAG_COL _REST; do
    RELEASES_TO_DELETE+=("$TAG_COL")
done < <(gh release list --limit 100 --json tagName,publishedAt --jq '
    sort_by(.publishedAt) | reverse | .['"$KEEP_RELEASES"':] | .[] |
    [.tagName] | @tsv
')

if [[ ${#RELEASES_TO_DELETE[@]} -gt 0 ]]; then
    for rel_tag in "${RELEASES_TO_DELETE[@]}"; do
        echo "   deleting release: $rel_tag"
        gh release delete "$rel_tag" --yes 2>&1 || warn "failed to delete release $rel_tag"
    done
    ok "Deleted ${#RELEASES_TO_DELETE[@]} old release(s)"
else
    ok "No old releases to prune"
fi

KEEP_TAGS=()
while IFS= read -r kept_tag; do
    KEEP_TAGS+=("$kept_tag")
done < <(gh release list --limit 100 --json tagName,publishedAt --jq '
    sort_by(.publishedAt) | reverse | .[:'$KEEP_RELEASES'] | .[].tagName
')

ARTIFACTS_DELETED=0
while IFS=$'\t' read -r ART_ID ART_BRANCH; do
    KEEP=false
    for kept_tag in "${KEEP_TAGS[@]}"; do
        if [[ "$ART_BRANCH" == "$kept_tag" ]]; then
            KEEP=true
            break
        fi
    done
    if ! $KEEP; then
        gh api -X DELETE "repos/{owner}/{repo}/actions/artifacts/$ART_ID" 2>/dev/null || true
        ARTIFACTS_DELETED=$((ARTIFACTS_DELETED + 1))
    fi
done < <(gh api repos/{owner}/{repo}/actions/artifacts --paginate --jq '
    .artifacts[] | [(.id | tostring), .workflow_run.head_branch] | @tsv
')

if [[ $ARTIFACTS_DELETED -gt 0 ]]; then
    ok "Deleted $ARTIFACTS_DELETED old artifact(s)"
else
    ok "No old artifacts to prune"
fi

# Prune old GHCR container images (keep versions matching kept releases)
GHCR_PACKAGE="scryer"
IMAGES_DELETED=0
while IFS=$'\t' read -r VID VTAGS; do
    KEEP=false
    for kept_tag in "${KEEP_TAGS[@]}"; do
        # Scryer tags are "scryer-v0.8.3" → GHCR version tag is "0.8.3"
        STRIPPED="${kept_tag#scryer-v}"
        if echo "$VTAGS" | grep -qF "$STRIPPED"; then
            KEEP=true
            break
        fi
        if echo "$VTAGS" | grep -qw "latest"; then
            KEEP=true
            break
        fi
    done
    if ! $KEEP; then
        gh api -X DELETE "/orgs/scryer-media/packages/container/$GHCR_PACKAGE/versions/$VID" 2>/dev/null || true
        IMAGES_DELETED=$((IMAGES_DELETED + 1))
    fi
done < <(gh api "/orgs/scryer-media/packages/container/$GHCR_PACKAGE/versions?per_page=100" --paginate --jq '
    .[] | [(.id | tostring), (.metadata.container.tags | join(","))] | @tsv
' 2>/dev/null || true)

if [[ $IMAGES_DELETED -gt 0 ]]; then
    ok "Deleted $IMAGES_DELETED old container image(s) from ghcr.io/scryer-media/$GHCR_PACKAGE"
else
    ok "No old container images to prune"
fi

# ── Create signed tag ──────────────────────────────────────────────────────────
step "Creating signed tag $TAG_NAME"

git tag -s "$TAG_NAME" -m "Release $TAG_NAME"
ok "Tag $TAG_NAME created"

# ── Push ───────────────────────────────────────────────────────────────────────
step "Pushing to origin"

git push origin "$BRANCH"
git push origin "$TAG_NAME"
ok "Pushed $BRANCH and tag $TAG_NAME"

# ── Done ───────────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}${BOLD}🚀  Released $TAG_NAME${RESET}"
