# Lessons Learned

## Environment & User Trust
- **Never question the user's environment state.** When they say something is broken, the code is freshly built and the environment is clean. Go straight to debugging the code.
- **Never suggest restarts/rebuilds as a fix.** The user runs `stack-restart.sh` on every Rust change. Assume this is done.
- **Don't assume you know better than the user about their environment.** If they say the container was rebuilt, it was rebuilt.
- **Treat Cargo concurrency limits as repo-scoped, not workspace-shell-scoped, when this shell contains multiple independent repos.** Avoid concurrent Cargo work inside the same Rust repo, but do not block unrelated repo work when the user confirms the active jobs are in a different project.

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

## UI Organization
- **Per-facet/per-category settings belong in the per-category section**, not in a general settings section. When a feature varies by media type (movie/series/anime), put its UI alongside the existing per-category controls (e.g. "Default category profiles"), not inside the profile editor's general scoring section.
- **Don't mix preset selection with fine-tuning knobs** in the same visible section. If a user picks a persona preset, showing override toggles right next to it is confusing — collapse advanced overrides behind a sub-`<details>`.

## Search Scoring
- **Validate persona promises against real release examples before shipping scoring tweaks.** If the UI says remux is an Audiophile concern, Balanced/Efficient/Compatible should not still reward remux-heavy anime releases through hidden defaults or oversized file bonuses.

## Tracked Downloads
- **When reconstructing tracked-download state, preserve a durable import-history fallback until terminal tracked state persistence is proven end-to-end.** Otherwise a completed import can be resurrected after restart and re-enter the workflow incorrectly.
- **In import verification, completion should be based on expected logical units being satisfied, not on the mere presence of rejected artifacts.** Extra rejected files must not block a fully satisfied download.

## Planning & Type Design
- **When converting string workflow states, follow the existing serde-enum pattern already used in the codebase instead of inventing new constant-only patterns.** Keep text serialization at the persistence boundary and make the enum authoritative inside Rust.
- **When moving conditional persistence logic out of SQL and into Rust, preserve the original atomicity guarantees.** If the old query did read/modify/write in one statement, replace it with a transaction or a single repository command, and keep low-level upserts defensive against accidental bypasses.

## Indexer Search Contracts
- **Do not bake alias language or script policy into core search query construction.** Core should pass canonical title plus tagged alias context through to indexer plugins, and plugins should decide whether to prefer romanized Japanese, Korean aliases, or other provider-specific naming conventions.
- **Pass an explicit normalized facet through the plugin search contract.** Plugins should get `movie` / `series` / `anime` directly instead of reverse-engineering semantics from category strings or host-side ID heuristics.
- **Pass IDs through the host/plugin boundary as a filtered map, not fixed `imdb_id` / `tvdb_id` / `anidb_id` slots.** The host may filter to supported IDs for a strategy, but provider-specific query shaping from those IDs belongs inside the plugin.
- **When the user asks for logging via `RUST_LOG`, do not add plugin config fields or descriptor changes.** Prefer existing runtime log filtering and keep observability changes out of the plugin contract unless the user explicitly asks for new config surface.

## urql / Frontend Caching
- `cacheExchange` was removed from all urql clients — the network layer handles caching naturally.
- Don't add per-query `requestPolicy` overrides; the exchange-level removal is the correct fix.
