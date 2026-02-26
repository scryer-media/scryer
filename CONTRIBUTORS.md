# Contributing to Scryer

This document covers the development setup, architecture, and workflows for contributors.


## Repo layout

```
scryer/
  docs/
    architecture/
    intentions/
    specs/
    adr/
  crates/
    scryer-domain/
    scryer-application/
    scryer-infrastructure/
    scryer-interface/
    scryer/
  apps/
    scryer-web/            # Next.js SPA
    scryer-metadata-gateway/ # Go metadata gateway (SMG)
```

## Design-first loop

This repo is intentionally scaffolded around intent and architecture documentation before
implementation:

- `docs/intentions` defines the initial intentions and product direction.
- `docs/specs` captures expected behavior in bounded requirements.
- `docs/architecture` stores the declarative architecture declaration and derived notes.
- `docs/adr` tracks architectural decisions as they are made.

The workflow:

1. Update or add an intention.
2. Derive/adjust specs from that intention.
3. Update `docs/architecture/manifest.yaml` if structure or boundaries change.
4. Record the decision in an ADR.
5. Implement code only after the above artifacts are aligned.

## UI model

The first architectural principle is a **unified media model** behind the API with
media-type as a facet (`movie`, `tv`, `anime`) filtered at UI boundaries.

## Prerequisites

- **Rust** (stable toolchain) + Cargo
- **Node.js** 22+ and npm
- **Go** 1.24+
- **Docker** (for PostgreSQL)
- **Docker Compose** (for the full dev stack orchestration, optional)

## Development Run (Docker Compose)

For local development, use the script runner to orchestrate the Docker Compose stack:

```bash
./scripts/stack-up.sh
```

The orchestrator brings up:

- PostgreSQL container for metadata gateway cache
- Metadata gateway service task (`go run`) for GraphQL + TVDB normalization
- Rust service (`raw_exec`, separate Next.js dev server)
- Next.js SPA (`raw_exec`, separate process)

To stop the stack:

```bash
./scripts/stack-down.sh
```

The stack can be run in foreground (default, Ctrl+C to stop app services) or detached:

```bash
DETACHED=1 ./scripts/stack-up.sh
```

Docker Compose must already be running. Set `NOMAD_ADDR` for a reachable endpoint before running:

```bash
NOMAD_ADDR="http://127.0.0.1:4646" ./scripts/stack-up.sh
```

SMG TVDB credentials are loaded from `apps/scryer-metadata-gateway/.env`.

Job variables can be overridden by environment before stack startup (for example `SCRYER_WEB_PORT` / `SCRYER_BIND` / `NOMAD_GATEWAY_PORT` / `NOMAD_DATACENTER` / `NOMAD_NETWORK_MODE`).

## Running services individually

### Scryer (Rust service + Next.js UI)

```bash
cd crates/scryer && cargo run -p scryer
```

Use separate terminals for each process below. Environment is loaded from `.env` files at the repo root and in `crates/scryer/`.

Run only the backend:

```bash
cd crates/scryer && cargo run -p scryer
```

Run only the web UI dev server:

```bash
cd apps/scryer-web && NEXT_PUBLIC_SCRYER_GRAPHQL_URL="${NEXT_PUBLIC_SCRYER_GRAPHQL_URL:-http://127.0.0.1:8080/graphql}" npm run dev -- --port "${SCRYER_WEB_PORT:-3000}"
```

### Metadata Gateway (SMG)

```bash
cd apps/scryer-metadata-gateway && GOCACHE=/tmp/gocache go run .
```

SMG loads its environment from `apps/scryer-metadata-gateway/.env`. See `.env.example` in that directory for all available variables.

Regenerate GraphQL bindings after schema changes:

```bash
cd apps/scryer-metadata-gateway && GOCACHE=/tmp/gocache go run github.com/99designs/gqlgen@v0.17.86 generate
```

Run SMG tests:

```bash
cd apps/scryer-metadata-gateway && go test ./...
```

## Release pipeline (single-command binary + embedded UI)

From repo root:

```bash
cd apps/scryer-web && NEXT_PUBLIC_SCRYER_GRAPHQL_URL=/graphql npm run build
rm -rf crates/scryer/ui
mkdir -p crates/scryer/ui
cp -R apps/scryer-web/out/. crates/scryer/ui/
SCRYER_PROFILE=release SCRYER_WEB_DIST_DIR=crates/scryer/ui cargo run -p scryer
```

This command:

- Builds the Next.js SPA (static export)
- Copies the export into `crates/scryer/ui`
- Builds the Rust service (`--release` by default)
- Runs the Rust service serving GraphQL and embedded static UI

Set `SCRYER_PROFILE=debug` if you want a debug Rust binary instead:

```bash
SCRYER_PROFILE=debug SCRYER_WEB_DIST_DIR=crates/scryer/ui cargo run -p scryer
```

## Tests and checks

```bash
cd crates/scryer && cargo test -p scryer
cd crates/scryer && cargo check -p scryer
cd apps/scryer-web && npm run lint
cd apps/scryer-metadata-gateway && go test ./...
```

## NZBGet (local dev)

Start a local NZBGet daemon for development:

```bash
/opt/homebrew/bin/nzbget -D -o OutputMode=loggable \
	-o WebDir=/opt/homebrew/opt/nzbget/share/nzbget/webui \
	-o ConfigTemplate=/opt/homebrew/opt/nzbget/share/nzbget/nzbget.conf \
	-o CertStore=/opt/homebrew/opt/ca-certificates/share/ca-certificates/cacert.pem \
	-c tmp/nzbget/config/nzbget.conf
```

Stop it:

```bash
pkill -f "/opt/homebrew/bin/nzbget -D -o OutputMode=loggable -o WebDir=/opt/homebrew/opt/nzbget/share/nzbget/webui -o ConfigTemplate=/opt/homebrew/opt/nzbget/share/nzbget/nzbget.conf -o CertStore=/opt/homebrew/opt/ca-certificates/share/ca-certificates/cacert.pem -c tmp/nzbget/config/nzbget.conf"
```

Requires NZBGet installed at `/opt/homebrew/bin/nzbget` and a config file at `tmp/nzbget/config/nzbget.conf`.

## Command reference

Use the scripts and shell commands shown above for all common workflows.
