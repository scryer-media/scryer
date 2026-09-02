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

Gated by [specs/constitution.md](../constitution.md) v1.1.0. C2: the grab dialog is the preview
and the signed candidate token binds the grab to the shown release. C3: failed/skipped indexers
and per-release outcomes are always surfaced. C5: the search is the existing in-memory interactive
job (accepted, polled, cancellable; ephemeral by design, as today). C6: additive GraphQL only;
no plugin/SDK contract change. C1: no migrations. **No deviations.**

## Guiding rule (operator, 2026-09-02)

**Reuse the interactive-search machinery. No parallel subsystem.** The first cut of WP1 built a
sibling job module; it is being folded back. Every piece below names the existing thing it
extends. If a step cannot be expressed as an extension of existing code, it is out of scope.

## Decisions (final unless the operator objects)

- **D1 Placement.** Fourth pane of the Indexers page, `/integrations/indexers/search`, on the
  existing subnav rail. An indexer row gets a "Search with this indexer" action that opens the
  pane with that indexer preset.
- **D2 Search kinds.** Movie / Series / Anime / Raw. Kind ⇒ facet + `id_search_facet` + default
  categories from `default_indexer_routing_categories_for_scope`; Raw ⇒ no facet ⇒ plain
  `TextQuery` (the search client now dispatches that; landed in 485ae0a01).
- **D3 One job, two subjects.** `interactive_release_search.rs` gains a query subject:
  `InteractiveReleaseSearchRequest` grows `query`, `kind`, `indexer_ids`, `categories` (title
  fields become optional; exactly one of `title_id` / `query` is required). The job context
  holds an enum `{Title{…existing…}, Query{query, facet, categories}}`; the per-indexer task
  branches once: Title ⇒ the existing scoring call + tokens; Query ⇒
  `services.integrations.indexer_client.search(...)` with the single-indexer routing plan the
  interactive path already builds (`catalog/discovery.rs` ~L1179). Merge, dedupe, status,
  registry, TTLs, cancel, poll: unchanged. `indexer_ids` restricts dispatch for both subjects.
- **D4 Release identity and grabs.** No job-scoped ids. A grab names the release by
  `(searchId, downloadUrl)` — the value the existing payload already hands the browser — and the
  server verifies it exists in that actor's job snapshot before minting a candidate token with
  the existing `attach_candidate_tokens` for the chosen title + season/episode subject. The web
  then calls the existing `queueDownload` / `queueReplacementRelease` mutations. Coverage
  (episode / season pack) comes from the existing `resolve_release_coverage`, exactly as the
  interactive queue does, so the dialog has no manual coverage control.
- **D5 Facets.** Derived in the browser from `parsedRelease` (quality/source/remux/atmos/DV/HDR/
  proper), `sourceKind`, `freeleech`, `seeders`, `source` (indexer name). Counts are computed over
  the full result set the browser already holds, so toggling never changes them. No server facet
  code.
- **D6 Rejections.** For Movie/Series/Anime kinds the server fills the existing
  `quality_profile_decision` on each query-subject result with `evaluate_against_profile_for_category`
  against the facet's default profile (`resolve_quality_profile` with an empty lookup), nothing
  else — so `blockCodes` is what the web shows as "rejected by …". Raw kind: none. Rules-engine and
  title-relative checks are not evaluated (they need a title). The grab path re-evaluates as it does
  today; the dialog's override tick is an acknowledgement (operator grabs already bypass scoring).
- **D7 "Count as an upgrade".** Dropped from phase 1. The dialog offers "Replace the existing
  file" only when the target scope holds a file, wired to the existing `queueReplacementRelease`.
- **D8 Unlinked grab.** The one genuinely new path: submit to a chosen enabled client with no
  title, record the submission like an adopted foreign item (`title_id: ""`, `SubmissionScope::Orphan`,
  `OperatorQueued`, source fields filled), emit `ReleaseGrabbed` without a title. Requires
  `ManageSystemSettings`. Smallest change that reaches every client implementation.
- **D9 Retry failed.** The web starts a new job with `indexerIds` = the failed set and merges
  rows (by existing row identity) and health entries into the current view. No server retry.
- **D10 Save search.** Per-browser `localStorage` bookmark, max 20.
- **D11 Coverage inference.** Server-side via `resolve_release_coverage` at token time (D4).
- **D12 Title picker.** Web-composed from existing `titles(query, facet, limit)` and each
  candidate's `wantedItems` (gap label: missing count / cutoff unmet / complete), plus
  `rootFolderPath` and quality-profile name from the title payload. No new server query.
- **D13 Permissions.** Query-subject search: `ManageSystemSettings` (the Indexers page gate).
  Linked grab: existing `ManageTitles` check inside the queue path. Unlinked: `ManageSystemSettings`.
- **D14 Sorting, filtering, advanced limits** (size / seeders / age) are client-side over the
  full set. The server keeps the existing `limit` semantics.
- **D15 Health.** `InteractiveReleaseSearchIndexerView` gains `elapsed_ms` and `priority`
  (additive). "Slow" = `elapsedMs > 1000`, derived in the web. A 5xx arms the client's system
  backoff, so a retry of that indexer may report skipped/backed-off; the web shows the reason.
- **D16 Download client.** Linked grabs use the existing indexer mapping / facet routing (no
  per-grab override). The client select exists only in unlinked mode.
- **D17 Download to browser (operator addition, 2026-09-02).** A third grab mode with no
  dialog. The web `POST`s `{searchId, downloadUrls}` to `/api/indexer-search/artifacts`, an
  axum route mounted beside the avatar proxy and authenticated the same way (`resolve_actor`,
  full session scope); the response is the file with `Content-Disposition: attachment`, exactly
  like the backup download route. The server resolves each `(searchId, downloadUrl)` through
  `find_interactive_search_result` (D4) and fetches the bytes through the download router's
  existing artifact resolution (`prepare_download_request` → `classify_resolved_download_artifact`),
  exposed as one new `DownloadClient` port method that forces the host-side fetch the router
  otherwise skips for NZB URLs. One release ⇒ raw file; several ⇒ `tar.gz` built with the
  `tar`/`flate2` crates the application crate already uses. Magnets are refused. Each
  downloaded release emits the same `ReleaseGrabbed` event the unlinked grab emits (stand-in
  title, no `download_id`); no submission row, since there is no client item to track. Gate:
  `ManageSystemSettings` (D13).

## GraphQL (additive)

- `SearchReleasesInput`: `titleId` becomes nullable; add `query: String`, `kind:
  InteractiveSearchKindValue (MOVIE|SERIES|ANIME|RAW)`, `indexerIds: [ID!]`, `categories:
  [String!]`. Validation: exactly one of `titleId` / `query`; `kind` required with `query`.
  The one-shot `searchReleases` query rejects query-subject input with a Validation error.
- `InteractiveReleaseSearchIndexerPayload`: add `elapsedMs: Int`, `priority: Int`.
- `IndexerSearchResultPayload`: add `grabs: Int` (from `indexer_grabs`) and `indexerId: ID`.
- New mutation `issueInteractiveReleaseCandidateToken(input: {searchId, downloadUrl, titleId,
  season, episode}) : IndexerSearchResultPayload` — the same result, now carrying
  `candidateToken` + `queueScope`.
- WP3 adds `queueUnlinkedRelease(input: {searchId, downloadUrl, downloadClientId})`.
Regenerate `api/graphql/schema.graphql` through the repo's existing mechanism; never hand-edit.

## Work packages (sequential, one Opus agent each)

| WP | Scope |
|---|---|
| WP1b | Fold: delete `catalog/indexer_search.rs` + its tests/registry; extend the interactive job (D3, D6, D15), GraphQL additive fields + token mutation (D4); port the useful WP1 tests into the interactive test modules; keep the search-client change |
| WP3 | Unlinked grab (D8) + its mutation + tests |
| WP4 | Web search pane (reusing `runIterativeReleaseSearch`, `Release` type, dom-id helpers) with live per-indexer refinement, client-side facets/sort/filters, retry (D9), saved searches |
| WP5 | Web grab dialog: title picker (D12), token mutation → existing queue mutations, unlinked mode |
| WP6 | E2E flow, release notes, docs |
| WP7 | Download to browser (D17): port method + router fetch policy, app bundle builder, HTTP route, web action + save helper, e2e step |
| Capstone | Clippy once, targeted suites, web lint/test |

## Validation policy

Per WP: targeted `cargo test` filters and the touched integration test; web WPs run lint, test,
react-compiler check. No workspace sweeps, no clippy until the capstone, no e2e runs by agents.
Agents never commit; the reviewer commits signed.
