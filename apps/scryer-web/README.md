# scryer web

This is the Next.js (React) SPA shell for scryer.

## Run

```bash
cd apps/scryer-web
npm install
npm run dev
```

The Vite dev server proxies `/graphql`, `/health`, `/admin`, and `/images`
to `http://127.0.0.1:8080` by default so subscriptions work when the Rust
backend is running locally. Override the proxy target with
`SCRYER_DEV_PROXY_TARGET` if your backend is elsewhere.

## Environment

- `SCRYER_BASE_PATH` (optional): Router basename for dev/proxy testing. Defaults to `/`.
- `SCRYER_GRAPHQL_URL` (optional): GraphQL URL used by the UI.
  - Defaults to `/graphql`.
- `SCRYER_METADATA_GATEWAY_GRAPHQL_URL` (optional): GraphQL URL for metadata proxy operations.
  - Defaults to `http://127.0.0.1:8090/graphql` when not set and should be overridden for production.

Metadata lookups used for TVDB title discovery/details should use this endpoint.

The app is built for static export and calls the backend directly via the configured GraphQL URL.

## UI

- Left gutter navigation for Movies/Series/Anime/Activity/Settings/System
- Top header search bar and status area
- shadcn-style client components (`button`, `input`, `card`, and table primitives)
- Dark theme by default
