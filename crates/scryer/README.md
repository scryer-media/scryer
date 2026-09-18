# `scryer` (service binary)

The single deployable: HTTP server, GraphQL endpoint (`POST /graphql`),
embedded web UI, database migrations, background jobs, and the desktop/tray
entry points. Everything user-facing is documented on the website linked from
the [repository README](../../README.md); this file only covers running the
crate from a checkout.

## Run from source

```bash
cargo run -p scryer
```

The web app is served from the embedded build when present, or from
`SCRYER_WEB_DIST_DIR` when set. During UI development run the Vite dev server
from `apps/scryer-web` and point the backend notice at it with
`SCRYER_WEB_UI_URL`.

`crates/scryer/.env.example` lists the bootstrap environment; the ones you are
most likely to touch:

- `SCRYER_BIND` — listen address (default `127.0.0.1:8080`)
- `SCRYER_DB_PATH` — database location override
- `SCRYER_BASE_PATH` — serve UI, GraphQL, health, and WebSocket endpoints
  under a path prefix when hosted behind a reverse proxy
- `SCRYER_WEB_DIST_DIR`, `SCRYER_WEB_UI_URL` — see above

Application settings themselves live in the database and are managed through
the UI/GraphQL, not environment variables.

## Layout

- `src/main.rs`, `src/init.rs`: startup, runtime composition, job wiring
- `src/db/`: migration manifest, SQLite and PostgreSQL migrations, baselines
- `src/middleware.rs`, `src/http_error.rs`, `src/rate_limit.rs`: HTTP surface
- `src/ui_assets.rs`, `src/base_path.rs`: embedded UI and prefix handling
- `src/tray.rs`, `src/splash.rs`: desktop entry points
- `tests/`: integration suites (GraphQL, import, download, search, …)

Use the workspace release tooling (`xtask-release`) for builds and releases;
see [CONTRIBUTING.md](../../CONTRIBUTING.md).
