# Contributing to Scryer

This document covers the development setup, architecture, and workflows for contributors.

## Repo Layout

```
scryer/
  crates/
    scryer-domain/          # Domain models and traits
    scryer-application/     # Use cases and business logic
    scryer-infrastructure/  # External integrations (DB, HTTP, download clients)
    scryer-interface/       # GraphQL API and WebSocket layer
    scryer-release-parser/  # Release title parsing and scoring
    scryer-rules/           # OPA/Rego policy engine
    scryer-plugins/         # WASM plugin runtime and built-in plugins
    scryer-mediainfo/       # Media file analysis (ffprobe)
    scryer/                 # Binary entry point, CLI, server bootstrap
    scryer-mock-apis/       # Mock services for testing
  apps/
    scryer-web/             # Vite + React 19 + React Router 7 SPA
  docker/                   # Dockerfiles for build, dev, and release
  scripts/                  # Dev stack orchestration and release tooling
  docs/                     # Getting started guide and assets
```

## Design-First Workflow

Architecture documentation lives in the [scryer-docs](https://github.com/scryer-media/scryer-docs) sibling repo:

1. Update or add an intention in `intentions/`
2. Derive/adjust specs from that intention in `specs/`
3. Update `architecture/manifest.yaml` if structure changes
4. Record decisions in `adr/`
5. Implement code only after artifacts are aligned

## UI Architecture

The frontend is a **Vite + React 19 + React Router 7** single-page application with a unified media model behind the API, where media-type is a facet (`movie`, `series`, `anime`) filtered at UI boundaries.

- **UI primitives**: shadcn/ui components in `apps/scryer-web/components/ui/`
- **Theme**: Tailwind v4 with semantic color tokens in `apps/scryer-web/app/globals.css`
- **i18n**: All visible strings go through `t(...)` from `apps/scryer-web/lib/i18n/`
- **GraphQL**: urql with `network-only` policy (no client-side caching)

## Prerequisites

- **Rust** (stable toolchain) + Cargo
- **Node.js** 22+ and npm
- **Docker** and Docker Compose

## Development Stack

The dev stack is orchestrated via Docker Compose:

```bash
./scripts/stack-up.sh
```

This brings up:
- NZBGet container (download client for testing)
- Scryer Rust service (compiled and run inside the container)
- Vite dev server for the web UI
- Nginx reverse proxy combining both on port 3000

`./scripts/stack-up.sh` recreates the Rust service container each time, so local testing
starts from a fresh Linux build tree by default.

To stop:

```bash
./scripts/stack-down.sh
```

View logs:

```bash
./scripts/stack-logs.sh
```

## Running Services Individually

### Scryer (Rust backend)

```bash
cargo run -p scryer
```

Environment is loaded from `crates/scryer/.env`.

### Web UI (Vite dev server)

```bash
cd apps/scryer-web && npm run dev
```

## Build & Test

```bash
# Rust
cargo build --workspace --locked
cargo nextest run --workspace --locked

# Frontend
cd apps/scryer-web && npm ci && npm run build

# Lint
cargo clippy --workspace --locked -- -D warnings
cd apps/scryer-web && npm run lint
```

## Release

From the repo root:

```bash
./scripts/release.sh          # patch bump
./scripts/release.sh --minor  # minor bump
./scripts/release.sh --dry-run
```

The script handles: cargo update, audit, clippy, tests, npm audit fix, lint, version bumping all workspace crates, cargo check, signed tag, and push. CI builds and publishes the release on tag push.

## Reporting Issues

File bug reports and feature requests in the [GitHub Issues](https://github.com/scryer-media/scryer/issues) tab.
