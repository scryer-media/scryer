# Lessons Learned

## Environment & User Trust
- **Never question the user's environment state.** When they say something is broken, the code is freshly built and the environment is clean. Go straight to debugging the code.
- **Never suggest restarts/rebuilds as a fix.** The user runs `stack-restart.sh` on every Rust change. Assume this is done.
- **Don't assume you know better than the user about their environment.** If they say the container was rebuilt, it was rebuilt.

## Destructive Actions
- **NEVER execute destructive actions on live services without explicit permission.** This includes deleting downloads, killing processes, dropping data, etc.
- Only suggest commands for destructive operations. Let the user decide and execute.
- Even if something looks like "test data" or "leftover" — it might be a valid in-progress item the user cares about.

## NZBGet Integration
- NZBGet v21 `append` API expects post-processing parameters as `[{"Name": "key", "Value": "val"}]`, NOT `[{"key": "val"}]`. The latter is silently ignored.
- The `extract_nzbget_parameters` function correctly parses `Name`/`Value` pairs — both sides must agree on the format.

## SQL / SQLite
- When adding JOINs to existing queries, prefix ALL column references in WHERE and ORDER BY with table aliases to avoid ambiguous column errors.
- Count queries without JOINs don't need prefixes — only fix the queries that actually have the JOIN.

## Full-Stack Schema Changes
- When adding fields to a GraphQL type, update ALL layers: migration → application types → infrastructure queries → interface types/mappers → **frontend TypeScript types** → frontend GraphQL queries.
- Frontend types are manually defined (no codegen). Check `TitleMediaFile` in movie-overview-container, `EpisodeMediaFile` in series-overview-container, and `MediaInfoFile` in media-info-badges for media file schema changes.
- Don't declare a schema change "done" until the frontend builds clean with the new fields flowing end-to-end.

## urql / Frontend Caching
- `cacheExchange` was removed from both `backendClient` and `smgClient` — the network layer handles caching naturally.
- Don't add per-query `requestPolicy` overrides; the exchange-level removal is the correct fix.
