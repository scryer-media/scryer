# scryer web

The React SPA shell for scryer, built with Vite, React 19, react-router, urql,
and Tailwind v4 on shadcn/Radix primitives. It is a projection client: product
rules live in the backend, and the UI reaches them over GraphQL.

## Run

```bash
cd apps/scryer-web
npm ci
npm run dev
```

The Vite dev server proxies `/graphql` (including its WebSocket transport),
`/authless-client`, the OAuth routes (`/.well-known/oauth-authorization-server`,
`/oauth/authorize/decision`, `/oauth/token`, `/oauth/revoke`), `/health`,
`/admin`, `/backups`, and `/images` to `http://127.0.0.1:8080` by default, so
subscriptions and the authenticated routes work when the Rust backend is running
locally. Override the proxy target with `SCRYER_DEV_PROXY_TARGET` if your
backend is elsewhere.

## Environment

- `SCRYER_BASE_PATH` (optional): path prefix the UI is served under. Defaults to `/`.
- `SCRYER_GRAPHQL_URL` (optional): GraphQL URL used by the UI.
  - Defaults to `<base path>/graphql`.

The production build is a static bundle in `dist/`, built with a relative asset
base so it can be served from any path prefix. The serving binary fills the
`__SCRYER_BASE_PATH__` and `__SCRYER_GRAPHQL_URL__` placeholders, or provides
`window.__SCRYER_RUNTIME_CONFIG__`, so one build works under any prefix.

## Checks

- `npm run lint` — typecheck, translation-key check, ESLint
- `npm run test` — unit tests
- `npm run check:react-compiler` — React Compiler compilation of the large title tables
- `npm run test:graphql-compat` — GraphQL schema compatibility
- `npm run build` — production bundle

## UI

- Left gutter navigation for Movies/Series/Anime/Activity/Settings/System
- Top header search bar and status area
- shadcn-style client components (`button`, `input`, `card`, and table primitives)
- Dark theme by default
