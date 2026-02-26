# scryer service

This crate provides a first-version executable that exposes:

- `GET /` backend notice page (points to Next.js app)
- `POST /graphql` GraphQL endpoint
- `GET /graphiql` GraphQL playground

## Run

```bash
cd crates/scryer
cargo run
```

Run the SPA separately:

```bash
cd apps/scryer-web
npm install
npm run dev
```

The backend can expose the SPA URL in the root notice via:

```bash
SCRYER_WEB_UI_URL=http://127.0.0.1:3000
```
`SCRYER_WEB_DIST_DIR` controls where the service serves embedded static UI assets from.
When set (for example `${SCRYER_DIST=...}` by the build pipeline), the service serves the built SPA from that path at `/`.

Copy and edit the config template:

```bash
cp crates/scryer/.env.example .env
```

Load values before running (choose a preferred loader):

```bash
set -a
. .env
set +a
cargo run
```

Runtime bootstrap environment values (optional, used when DB settings are empty):

- `SCRYER_DB_PATH` (default `sqlite://file::memory:?mode=memory&cache=shared`)
- `SCRYER_BIND` (default `127.0.0.1:8080`)

Application configuration now lives in `settings_definitions` and `settings_values`.

Configuration reads and writes are exposed through GraphQL on `POST /graphql`:

- `adminSettings(scope: ..., scopeId: ..., category: ...)`
- `saveAdminSettings(input: AdminSettingsUpdateInput!)`

Common managed keys:

- `system.service.nzbget.url`
- `system.service.nzbget.username`
- `system.service.nzbget.password` (sensitive)
- `system.service.nzbget.dupe_mode`
- `system.service.nzbgeek.api_key` (sensitive)
- `system.service.nzbgeek.api_base_url`
- `system.service.nzbgeek.user_agent`
- `system.service.nzbgeek.min_request_interval_ms`
- `system.service.nzbgeek.base_backoff_seconds`
- `system.service.nzbgeek.max_backoff_seconds`
- `media.media.movies.path`
- `media.media.series.path`

Legacy bootstrap settings (still supported as fallback):

- `SCRYER_NZBGET_URL` (default `http://127.0.0.1:6789`)
- `SCRYER_NZBGET_DUPE_MODE` / `SCRYER_NZBGET_DUPEMODE` (optional, defaults to `SCORE`)
- `SCRYER_NZBGET_USERNAME`
- `SCRYER_NZBGET_PASSWORD`
- `SCRYER_NZBGEEK_API_KEY`
- `SCRYER_NZBGEEK_NAME` (optional, default `NZBGeek`)
- `SCRYER_NZBGEEK_PROVIDER_TYPE` (optional, default `nzbgeek`)
- `SCRYER_NZBGEEK_RATE_LIMIT_SECONDS` (optional, indexer policy hint)
- `SCRYER_NZBGEEK_RATE_LIMIT_BURST` (optional, indexer policy hint)
- `SCRYER_NZBGEEK_ENABLED` (optional, default `true`)
- `SCRYER_NZBGEEK_API_BASE_URL` (optional, defaults to `https://api.nzbgeek.info`)
- `SCRYER_NZBGEEK_USER_AGENT` (optional, defaults to `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36`)
- `SCRYER_NZBGEEK_MIN_REQUEST_INTERVAL_MS` (optional, default `1100`)
- `SCRYER_NZBGEEK_BASE_BACKOFF_SECONDS` (optional, default `10`)
- `SCRYER_NZBGEEK_MAX_BACKOFF_SECONDS` (optional, default `900`)
- `SCRYER_MOVIES_PATH`
- `SCRYER_SERIES_PATH`

- `SCRYER_WEB_UI_URL` (optional, default `http://127.0.0.1:3000`)
- `SCRYER_WEB_DIST_DIR` (optional, default `./crates/scryer/ui`)

MVP workflow: open the SPA on `http://127.0.0.1:3000` and use the nav/search experience for title add/queue actions.
`addTitleAndQueueDownload` should return success only when NZBGet accepts the exact NZB URL.

### NZBGet category routing

When queueing titles, Scryer now submits an NZBGet category derived from the title facet (`movie`, `tv`, `anime`, or `other`).

For a standard completed-directory workflow:
- Configure NZBGet with matching category definitions for `movie`, `tv`, `anime`, and `other`.
- Set category-specific `DestDir` under a common completed root (for example, `/media/completed/movie`, `/media/completed/tv`, etc.).
- Configure your Servarr clients to monitor the completed directories and move final assets into your library destinations.
- Keep this scryer category on queued items as the routing key; NZBGet category should remain your integration point for mover semantics.

Data storage:
- SQLite is used with SQLx and runs through the bundled SQLite library from `libsqlite3-sys`.
- No system SQLite package is required at runtime for basic DB access.
