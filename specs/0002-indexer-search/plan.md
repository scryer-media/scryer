# Implementation Plan: Indexer Search

**Spec**: [spec.md](./spec.md) · **Tasks**: [tasks.md](./tasks.md)
**Status**: Draft — 2026-09-02. Binding once the operator signs off on D1–D16.
**Worktree**: `.worktrees/indexer-search`, branch `feature/indexer-search` off
`origin/release-NEXT` @ `5951ae5be`.
**Pipeline**: Claude (Fable) plans, briefs, reviews, commits (SSH-signed). Opus agents implement
one work package at a time (cap: 1 concurrent), never commit, never bump versions.

## Summary

Add an aggregate, title-less indexer search to the Indexers page and a standalone grab dialog
that assigns a release to a catalog title (or grabs it unlinked) at the moment of grabbing.
Reuse the interactive-search job machinery (server job, poll, cancel, per-indexer fan-out) and
the operator-queued submission path; add only what is genuinely new: a title-less search
request, server-derived facets and context-free rejections, job-scoped release ids that stand
in for candidate tokens, a title-candidate query, and an orphan-scope unlinked submission.

## Technical context

Rust workspace (`crates/`), GraphQL (`api/graphql/schema.graphql` is the source of truth;
resolvers in `crates/scryer-interface*`), React/TS web app (`apps/scryer-web`, urql, Tailwind,
`--scry-*` tokens, 10 locales under `lib/i18n/locales`). E2E harness is the sibling repo
the sibling `e2e` checkout next to this repo (Playwright, Tier-S/F seeding policy).

### Verified anchors (2026-09-02, release-NEXT)

| Concern | Anchor |
|---|---|
| Interactive search job (pattern to mirror) | `crates/scryer-application/src/catalog/interactive_release_search.rs` — registry types L23–135, `start_…` L138, poll L402, cancel L422, runner `run_interactive_release_search_job` L452 (JoinSet fan-out, per-indexer status, deadline), merge L636 |
| Per-indexer restriction | `restrict_to_indexer_ids` → `crate::contracts::indexer_search_eligibility` (`contracts.rs` L1108); `IndexerRoutingPlan` L1096 |
| Multi-indexer client | `crates/scryer-infrastructure-acquisition/src/indexers/search_client.rs` `MultiIndexerSearchClient::search` L2299; port `IndexerClient::search` `ports.rs` L5578 (query, ids, category, facet, id_search_facet, newznab_categories, routing, mode, season/episode, tagged_aliases, learning ctx, cancel) |
| Raw text query kind | `crates/scryer-plugins/src/indexer_adapter.rs` L1390–1400: no ids + non-empty query + no facet ⇒ `PluginSearchQueryKind::Text`; plugin SDK `IndexerSearchInput::{TextQuery, Category, Limit}` `crates/scryer-plugin-sdk/src/indexer.rs` L32 |
| Search result model | `IndexerSearchResult` `crates/scryer-application/src/types.rs` L1716 (indexer_id, source, title, link, download_url, source_kind, size, published_at, indexer_grabs, parsed_release_metadata, extra, response_attributes, guid, info_url, …); `IndexerSearchResponse` L2123 (`indexer_outcomes`) |
| Release parsing | `crate::parse_release_metadata` (`quality/release_parser.rs` L12) → `ParsedReleaseMetadata` (quality/source/codec/audio/atmos/DV/HDR/proper/remux/season/episode) |
| Indexer priority | `build_indexer_priority_by_name(indexer_routing)` (used at `interactive_release_search.rs` L234); routing entry priority in `settings/runtime/routing.rs` |
| Default categories per kind | `settings/keys.rs` `default_indexer_routing_categories_for_scope("movie"\|"series"\|"anime")` L74 |
| Minimum seeders (context-free check) | `acquisition/seed_goals::meets_minimum_seeders` (called at `release_search.rs` ~L1488) |
| Rules engine | `crates/scryer-rules/src/lib.rs` `UserRulesEngine` L368, `UserRulesEvaluator::evaluate` L465+; input builder `rules/user_rule_input.rs` `build_rule_input(parsed, profile, decision, ReleaseRuntimeInfo, RuleContextInfo, file)` L29; engine handle `services.customization.user_rules` (`quality/canonical_context.rs` L329) |
| Manual grab path (linked) | `catalog/workflow/queueing.rs` `queue_manual_release_for_title(actor, &Title, QueuedReleaseSelection, SubmissionScope, SubmissionConflictPolicy, DownloadSubmissionPurpose)` L329; selection built exactly as `attach_candidate_tokens` does (`catalog/discovery.rs` L1742–1830: canonical_download_source, password_hint, info_hash from extra, seeders_from_extra) |
| Scopes / purposes | `contracts.rs` `SubmissionScope::{Episode, EpisodeSet, SeriesMovie, Collection, Title, Orphan}` L5; `DownloadSubmissionPurpose::{Standard, AdditionalFile, OperatorQueued, ManualReplacement}` L15 |
| Coverage resolution | `crate::acquisition_coverage::resolve_release_coverage(parsed, episodes, collections, requested_episode)` (used in `attach_candidate_tokens`) |
| Download client add request | `contracts.rs` `DownloadClientAddRequest` L1125 (`title: Title` required today); port `DownloadClient::submit_download` `ports.rs` L6491; plugin adapter builds `title_id: Some(..)` at `crates/scryer-plugins/src/download_client_adapter.rs` L895 |
| Orphan submission precedent | `integration/tracked_downloads.rs` L1030–1050 (adopted foreign item: `title_id: String::new()`, `scope: SubmissionScope::Orphan`); `import/parameters.rs` L9 treats Orphan as not importable |
| Submission pipeline | `acquisition/submission.rs` (`submission_for_grab` L44, `AppUseCase` impl L109) |
| Client routing | `settings/runtime/routing.rs` `get_download_client_routing(actor, scope_id)` L543; `IndexerConfig.download_client_id` (indexer→client mapping) `crates/scryer-domain/src/lib.rs` L972 |
| Wanted gap data | `TitlePayload.wantedItems(status, limit, offset)` (schema L~18700); `WantedItemPayload` (status, mediaType, seasonNumber, currentScore, …); app `acquisition/wanted_views.rs` |
| Title search | GraphQL `titles(facet, libraryIds, query, limit)`; web `catalogSearchTitlesQuery` `apps/scryer-web/lib/graphql/queries.ts` L1614 |
| GraphQL mutation/query wiring | `crates/scryer-interface/src/mutation/interactive_search.rs` (pattern), `mutation/mod.rs` `MutationRoot`; query root in `crates/scryer-interface-query/src/lib.rs`; payload types in `crates/scryer-interface-media-types/src/types/acquisition.rs` (`IndexerSearchResultPayload` L25) |
| Existing job GraphQL shape | schema `InteractiveReleaseSearchPayload` L7530, `InteractiveReleaseSearchIndexerPayload` L7478, `SearchReleasesInput` L16395 |
| Web poll loop (pattern) | `apps/scryer-web/lib/graphql/release-search.ts` `runIterativeReleaseSearch` (start → poll 1 s → cancel on abort; never rejects after start) |
| Indexers page panes | `apps/scryer-web/components/root/types.ts` `IndexerSettingsTab` L39; `lib/utils/routing.ts` `INDEXER_TAB_PATH`/`indexerSettingsTabFromPath` L56–84 (+ `routing.test.ts`); subnav `components/containers/settings/settings-container.tsx` L150–200 (`INDEXER_SETTINGS_TABS`, `IndexerSettingsSubnav`); pane switch `components/views/settings/settings-indexers-section.tsx` L1017 |
| Existing results row (reuse pieces) | `components/common/release-search-results.tsx` (`SearchResultRow` L164, badges, size/age formatting); selector ids `lib/utils/dom-ids.ts` L260–300 |
| UI primitives | `components/ui/{dialog,tabs,checkbox,select,table,badge,button,icon-button,popover,multi-select-dropdown}.tsx` |
| Tokens | `apps/scryer-web/app/globals.css` — every token the prototype uses exists (`--scry-surf/surfD/surfF/border2/border3/rowHover/inset/card2/faint*/ink*/muted*/soft3/accent-text/accent-ring/page3/hover`) |
| i18n | `lib/i18n/locales/{en,de,es,fr,it,ja,ko,pt_BR,ru,zh_CN}.ts`, `LocaleDictionary` keys; parity enforced by typecheck |
| Web checks | `npm run lint` (typecheck + eslint), `npm test`, `npm run check:react-compiler`, `npm run test:graphql-compat` |
| Integration test harness | `crates/scryer/tests/integration_interactive_release_search.rs` (real WASM newznab plugin + wiremock + real upstream scheduler) — copy its bootstrap |
| E2E fixtures | e2e `services/newznab` (supports `t=search&q=`), `services/torrent-indexer`, seed module `playwright/tests/seed/index.ts` (Tier-F seeding ledger), interactive-search helpers `playwright/tests/minimum-seeders-flow.ts` |

## Constitution check

Gated by [specs/constitution.md](../constitution.md) v1.1.0.
- C2 preview-before-mutate: the grab dialog *is* the preview (target, client, path, consequence
  line); the mutation executes exactly what was shown, and a job-scoped release id binds the
  grab to the server-held result.
- C3 nothing silent: failed/skipped indexers are surfaced with their reason; per-release grab
  outcomes are reported individually; rejections are shown, never hidden.
- C5 async: the search is a server job (accepted, polled, cancellable); it is in-memory and
  short-lived by design (search results are not durable state) — same deviation the existing
  interactive search already carries; not resumable across restart, and that is acceptable.
- C6 external compatibility: no plugin/SDK contract change; raw text search uses the existing
  `TextQuery` capability; unlinked grabs use the existing add-request shape with an absent title
  (plugin request already models `title_id: Option`).
- C1: **no migrations.** Saved searches are per-browser (D10); nothing new is persisted server-side
  except the ordinary submission row and history event a grab already writes.
- C8/C9 (validation, worktree, signed commits) apply per work package.
**No deviations requiring justification.** Security review is a separate track; this plan is
functional correctness only (the one security-relevant property, "download URLs never leave the
server / grabs reference server-held results", is preserved by construction).

## Decisions (recommendations; each is the operator's call)

- **D1 Placement.** Fourth pane of the Indexers page, `/integrations/indexers/search`, using the
  existing left subnav rail rather than the design's segmented control. *Impact:* same intent
  ("search is a tab of Indexers"), consistent with Proxies/Seeding profiles, deep-linkable.
  Arriving from an indexer row's new "Search with this indexer" action presets that indexer.
- **D2 Search kinds.** Movie / Series / Anime / Raw. Music and Book do not exist in Scryer and are
  dropped. Kind ⇒ facet + `id_search_facet` + default categories from routing defaults; Raw ⇒ no
  facet, no default categories ⇒ plain `TextQuery`.
- **D3 Job model.** New module `catalog/indexer_search.rs` with its own registry (completed TTL
  30 min, running TTL 10 min, per-actor cap 8), mirroring `interactive_release_search.rs`. One
  task per indexer through the multi-indexer client with a single-indexer restriction and
  `SearchMode::Interactive`. No title, no scoring, no candidate tokens. Per-indexer timing recorded.
  *Why a new module rather than an enum on the old one:* the old job's whole body is title-bound
  (subject resolution, scoring, tokens); sharing types would force `Option` everywhere.
- **D4 Release identity.** Job-scoped `release_id` = short hash of (indexer_id, guid ∥
  download_url ∥ title). Grabs send `(jobId, releaseIds)`; the server resolves the full result
  from the snapshot. Download URLs are never sent to the client. This replaces candidate tokens
  for this surface (same trust boundary: server-held, actor-scoped job).
- **D5 Facets.** Derived server-side per release from parsed metadata + indexer extras
  (protocol; resolution 2160p/1080p/720p/SD; source REMUX/BluRay/WEB-DL/WEBRip/HDTV/other;
  audio-hdr Atmos/Dolby Vision/HDR; flags Freeleech/Internal/Proper-Repack/Scene). Each release
  carries its facet values; the snapshot carries facet counts over the full merged set. The
  client filters and sorts locally; counts never change on toggle.
- **D6 Context-free rejections.** For Movie/Series/Anime kinds, evaluate each release against
  that facet's default quality profile (block codes) and the user rules engine with an empty
  title context (`title_id: None`, no library, `has_existing_file: false`,
  `search_mode: "raw"`), plus the indexer minimum-seeders floor. Reasons become
  `rejections: [String]` (first = badge text). Raw kind: only the seeders check. These are
  advisory: the grab path re-evaluates against the real target as it does today. *Impact:* the
  table shows "banned source" style rejections with no fake score; a rejection that only exists
  relative to a title (cutoff, upgrade) is never shown here.
- **D7 "Count as an upgrade and stop searching".** Scryer has no "mark cutoff satisfied" flag;
  cutoff-met is derived from the landed file's score. Recommendation: map the checkbox to
  submission purpose — checked ⇒ `ManualReplacement` when the target scope already holds a
  primary file (forces the replace, blocklists the displaced release, bypasses required-audio),
  otherwise `OperatorQueued` — and relabel it **"Replace the existing file with this release"**
  with helper text saying what it does. A real "stop searching" flag is a separate feature.
  **Needs the operator's call** (alternative: drop the option in phase 1).
- **D8 Unlinked grab.** `DownloadClientAddRequest.title` becomes `Option<Title>` (or a sibling
  request type if that ripple is too wide — implementer picks the smallest change that reaches
  every client implementation, and reports the site count). The submission row is written like an
  adopted foreign item (`title_id: ""`, `SubmissionScope::Orphan`, purpose `OperatorQueued`, all
  source fields populated) and a `ReleaseGrabbed` history event is emitted without a title. The
  tracked-download poller then surfaces it for manual import. Category: the chosen client's
  configured category for the kind's facet, else none ("download client default").
- **D9 Retry failed.** `retryIndexerSearch(id)` re-dispatches only indexers in `failed` state
  inside the same job; state returns to Running; merge dedupes on release id.
- **D10 Save search.** Per-browser bookmark in `localStorage` (query card state, max 20). No
  server persistence, no migration. *Impact:* works today, not synced across browsers; can be
  promoted later.
- **D11 Coverage inference.** Pre-select from the parsed release name: `SxxEyy` ⇒ Episode yy with
  season xx; season-only/season-pack ⇒ Season pack; else Episode. Multi-grab infers from the
  first release. Operator corrects.
- **D12 Title picker.** New query `indexerSearchGrabTargets(query, facet?, limit=5)` ranks:
  name match (existing `titles(query)` search) → titles with an open wanted gap → rest. Each
  candidate returns gap label/tone, profile name, root path, default download client id (title's
  routing), episodic seasons with missing counts. Server-side; the web renders what it gets.
- **D13 Permissions.** Page and search: `manageSystemSettings` (the Indexers page's gate).
  Linked grab: additionally `ManageTitles` on the target's library (existing check inside the
  manual queue path). Unlinked grab: `manageSystemSettings`.
- **D14 Sorting** is client-side (newest, size, age, seeders, indexer priority) over the full
  set. Server returns the full merged set; no pagination (bounded by per-indexer limit × indexers).
- **D15 Limits.** Per-indexer limit default 100, cap 250. *Amended after WP1:* the search port
  has no `limit` parameter, so the limit is applied by truncation of each indexer's batch; a real
  pass-through would touch the plugin SDK contract and is deferred. Advanced size/seeders/age
  filters are applied server-side before merge; `matched` counts survivors.
- **D16 Download client override.** The dialog's client select sends `downloadClientId`. If the
  manual queue path cannot take a per-grab client override today, thread a narrow
  `download_client_override: Option<String>` from `queue_manual_release_for_title` into the
  submission intent. The indexer→client mapping remains the default.

## Established by WP1 (2026-09-02, commit 485ae0a01)

- Application API: `AppUseCase::{start_indexer_search, indexer_search, retry_indexer_search,
  cancel_indexer_search}`; types `IndexerSearchRequest`, `IndexerSearchKind`,
  `IndexerSearchSnapshot` (request echo, totals, indexers, facets, releases),
  `IndexerSearchIndexerView` (status `Pending|Searching|Ok|Failed|Skipped`, `elapsed_ms`,
  `failure_reason`), `IndexerSearchRelease` (+ `facet_values`, `rejections`, parsed
  season/episode/is_season_pack, and the full server-held `result`), `IndexerSearchTotals`
  (`matched`, `indexers_queried`, `indexers_responded`, `truncated`). All re-exported from
  `scryer_application`.
- **No `Slow` status in the app.** "Slow" is a presentation threshold: the web derives it from
  `elapsedMs` (> 1 000 ms per the handoff). The GraphQL enum therefore has no `SLOW`.
- Facet keys: `protocol`, `indexer`, `resolution`, `source`, `audio_hdr`, `flags`; item labels
  are English values — the web keys i18n off `key`/`value`.
- A Raw search still passes the client's freetext title guard, so a query that is not a title
  (`2160p remux`) returns nothing even when the indexer answered. Kept on purpose for now; the
  web's empty state must say "no releases whose name matches this query". Lifting the guard for
  Raw is a separate decision.
- A 5xx arms the client's 5-minute system backoff, so "Retry failed" on such an indexer reports
  `Skipped · temporarily backed off` rather than healing. The web shows that reason honestly.
- Search and retry are gated on `ManageSystemSettings` in the application layer; the resolver
  gate is a consistent second check.

## GraphQL contract (additive)

```graphql
input StartIndexerSearchInput {
  query: String!
  kind: IndexerSearchKindValue!            # MOVIE | SERIES | ANIME | RAW
  indexerIds: [ID!]                        # null/empty = all enabled interactive indexers
  categories: [String!]                    # null = kind defaults
  minSizeBytes: Long  maxSizeBytes: Long  minSeeders: Int  maxAgeDays: Int
  perIndexerLimit: Int
}
type IndexerSearchPayload {
  id: ID!  state: IndexerSearchStateValue!  # RUNNING | COMPLETED | CANCELLED
  request: IndexerSearchRequestPayload!      # echo of the effective request
  totals: IndexerSearchTotalsPayload!        # matched, indexersQueried, indexersResponded, elapsedMs, ageSeconds
  indexers: [IndexerSearchIndexerPayload!]!  # id, name, priority, state(PENDING|SEARCHING|OK|FAILED|SKIPPED), count, elapsedMs, error
  facets: [IndexerSearchFacetPayload!]!      # key, label, items{value,label,count}
  releases: [IndexerSearchReleasePayload!]!  # id, title, protocol, indexer{id,name,priority}, sizeBytes,
                                             # publishedAt, categoryLabel, fileSummary, releaseGroup,
                                             # seeders, leechers, grabs, flags, facetValues{...},
                                             # rejections, infoUrl, parsed{season,episode,isSeasonPack}
}
type Mutation {
  startIndexerSearch(input: StartIndexerSearchInput!): IndexerSearchPayload!
  retryIndexerSearch(id: ID!): IndexerSearchPayload!
  cancelIndexerSearch(id: ID!): CancelIndexerSearchPayload!
  grabIndexerSearchReleases(input: GrabIndexerSearchReleasesInput!): GrabIndexerSearchReleasesPayload!
}
type Query {
  indexerSearch(id: ID!): IndexerSearchPayload
  indexerSearchGrabTargets(query: String!, facet: MediaFacetValue, limit: Int = 5): IndexerSearchGrabTargetsPayload!
}
input GrabIndexerSearchReleasesInput {
  searchId: ID!  releaseIds: [ID!]!
  target: IndexerSearchGrabTargetInput!      # { kind: TITLE|UNLINKED, titleId, seasonNumber, coverage: EPISODE|SEASON|SERIES, episodeNumber }
  downloadClientId: ID
  replaceExistingFile: Boolean!              # D7
  overrideRejections: Boolean!
  replaceInProgress: Boolean
}
type GrabIndexerSearchReleasesPayload { results: [IndexerSearchGrabResultPayload!]! }  # releaseId, status(QUEUED|CONFLICT|FAILED), jobId, titleId, message, conflict
```
Exact names may be refined by WP2/WP3 to match existing naming; the schema file is regenerated
from the resolvers (find the export path — `export_sdl`/schema test — and run it), then
`npm run test:graphql-compat`.

## Work packages (sequential, one Opus agent each)

| WP | Scope | Primary files |
|---|---|---|
| WP1 | Application: `IndexerSearchJob` (request, registry, fan-out, per-indexer timing/outcome, merge, facets, rejections, retry, cancel) + unit tests + integration test | `crates/scryer-application/src/catalog/indexer_search.rs`, `lib.rs` exports, `crates/scryer/tests/integration_indexer_search.rs` |
| WP2 | GraphQL search surface: types, start/poll/retry/cancel, permission gates, schema regen, integration_graphql tests | `crates/scryer-interface*/…`, `api/graphql/schema.graphql` |
| WP3 | Grab: linked (scope resolution, purpose mapping D7, client override D16, batch outcomes), unlinked (D8), `indexerSearchGrabTargets` (D12); GraphQL + tests | `catalog/indexer_search_grab.rs`, `contracts.rs`, submission/adapter touch points, interface |
| WP4 | Web search pane: route/pane, query card, health line, refine rail, results table, selection footer, poll loop, saved searches (D10), i18n ×10, selector ids | `apps/scryer-web/components/views/settings/indexer-search/…`, `lib/graphql/indexer-search.ts`, routing |
| WP5 | Web grab dialog (exported standalone), title picker, episodic scope, destination, options, outcomes; wiring from row/footer; i18n ×10 | `components/common/grab-dialog/…` |
| WP6 | E2E flow in the e2e repo (Tier-F spec: seeded newznab + torrent-indexer + SABnzbd/qBittorrent; search → health → facet → linked grab → tracked download; unlinked grab → Activity) + release-notes draft + docs | `e2e/playwright/tests/indexer-search.spec.ts` |
| Capstone | One clippy pass, targeted suites for every touched crate, web lint/test, journal, PR prep | — |

## Validation policy

Per WP: targeted `cargo test -p <crate> <filter>` and the WP's integration test only; web WPs
run `npm run lint`, `npm test`, `npm run check:react-compiler`. No workspace-wide sweeps, no
clippy until the capstone. E2E runs are launched by the operator only; WP6 hands him the command.
Agents never commit; the reviewer commits signed after review.

## Risks

- Per-indexer restriction plumbing may not be exposed on the port the raw job wants to call;
  WP1 verifies `search_restriction` reaches `MultiIndexerSearchClient` and otherwise adds it.
- `DownloadClientAddRequest.title` ripple (D8) may touch many constructors; WP3 reports the count
  before choosing the sibling-type route.
- Rules evaluation without a title may need a stub `QualityProfileDecision`; WP1 reuses the
  scoring path's decision builder rather than inventing one.
- Job memory: 250 × N indexers results per job × 8 jobs per actor — acceptable; WP1 caps total
  releases per job at 5 000 and reports truncation in totals.
