# Tasks: Indexer Search

**Input**: [spec.md](./spec.md), [plan.md](./plan.md). Constitution applies (C8 targeted tests,
web lint before handoff; C9 isolated worktree, gitflow branch, signed commits by the reviewer).
Format: `[ID] [WP] Description`. Work packages run sequentially, one agent at a time.

## WP1 — Application search job

- [ ] T101 [WP1] `catalog/indexer_search.rs`: request model (`IndexerSearchRequest` {query, kind,
      indexer_ids, categories, limits}), kind→facet/id-facet/default-categories mapping (D2),
      input validation (non-empty query, limit cap 250, size/age sanity).
- [ ] T102 [WP1] Registry (`runtime.acquisition.indexer_searches`): entries with actor, cancel
      token, TTLs (completed 30 min, running 10 min), per-actor cap 8, eviction on every access.
- [ ] T103 [WP1] Dispatch resolution: enabled + interactive-enabled indexers, minus backoff
      (`disabled_until`), intersected with requested ids; skipped ones recorded with reason.
- [ ] T104 [WP1] Runner: JoinSet, one task per indexer through the multi-indexer client with a
      single-indexer restriction, `SearchMode::Interactive`, per-indexer started/elapsed, outcome
      mapping (ok / failed + short error word / timed out on deadline), overall deadline.
- [ ] T105 [WP1] Merge: parse each result (`parse_release_metadata`), apply advanced limits,
      derive facet values + flags + file summary + category label + protocol, assign job-scoped
      release id (D4), dedupe within indexer on id, cap 5 000 total, recompute facet counts.
- [ ] T106 [WP1] Context-free rejections (D6): facet default profile block codes + user rules with
      empty title context + minimum-seeders floor. Raw kind: seeders only.
- [ ] T107 [WP1] `retry_indexer_search(actor, id)` (D9), `cancel_indexer_search`,
      `indexer_search(actor, id)` snapshot read (actor-scoped).
- [ ] T108 [WP1] Unit tests in `lib_tests/indexer_search.rs` with the test indexer client:
      kind mapping, restriction, merge/facets/ids, rejections, retry-only-failed, TTL eviction,
      actor scoping, cap.
- [ ] T109 [WP1] Integration test `crates/scryer/tests/integration_indexer_search.rs` (copy the
      interactive one's bootstrap): two wiremock newznab indexers, one healthy one 500 → health
      line states; retry heals; raw text query reaches the plugin as `TextQuery`.

## WP2 — GraphQL search surface

- [ ] T201 [WP2] Payload/input types in `scryer-interface-media-types` (plan contract), enums.
- [ ] T202 [WP2] Mutations `startIndexerSearch`, `retryIndexerSearch`, `cancelIndexerSearch`;
      query `indexerSearch(id)`; permission gate D13.
- [ ] T203 [WP2] Regenerate `api/graphql/schema.graphql`; `npm run test:graphql-compat`.
- [ ] T204 [WP2] `integration_graphql` tests: start→poll→complete, cancel, retry, actor isolation,
      permission denial, expired id → null.

## WP3 — Grab

- [ ] T301 [WP3] Linked grab: resolve snapshot release → `QueuedReleaseSelection` (mirror
      `attach_candidate_tokens`), target → `SubmissionScope` (movie Title; episode → episode id;
      season → the scope the season-pack path uses today; series → Title), purpose (D7), client
      override (D16), batch loop with per-release outcomes, expired-job typed error.
- [ ] T302 [WP3] Unlinked grab (D8): add-request title optionality, orphan submission row,
      history event, category resolution, permission.
- [ ] T303 [WP3] `indexerSearchGrabTargets` (D12): ranking, gap label, profile name, root path,
      default client id, seasons with missing counts.
- [ ] T304 [WP3] GraphQL: `grabIndexerSearchReleases`, `indexerSearchGrabTargets`; schema regen.
- [ ] T305 [WP3] Tests: unit (scope resolution, purpose mapping, ranking), integration
      (linked grab writes the same submission/history as interactive queue; unlinked grab writes an
      orphan submission and a history event; override flag semantics documented and tested).

## WP4 — Web search pane

- [ ] T401 [WP4] `IndexerSettingsTab` += `search`; routing + `routing.test.ts`; subnav entry
      (icon `ScanSearch`); pane switch in the section; breadcrumb label.
- [ ] T402 [WP4] `lib/graphql/indexer-search.ts`: documents + poll loop (pattern of
      `runIterativeReleaseSearch`), types in `lib/types/indexer-search.ts`.
- [ ] T403 [WP4] Components per handoff §12 (QueryCard + ScopeChips + AdvancedLimitsGrid,
      IndexerHealthLine, RefineRail + SizeRange, ResultsCard + ReleaseRow + ReleaseDetail +
      SelectionFooter). Tokens only; scroll ownership per §2; breakpoints per §2.
- [ ] T404 [WP4] Client-side filter/sort/facet state; health-line click filters; Retry failed;
      saved searches in localStorage (D10); "Search with this indexer" preset from the Providers
      table.
- [ ] T405 [WP4] i18n keys in all 10 locales; selector ids (`indexer-search-*`) on every control.
- [ ] T406 [WP4] `npm run lint`, `npm test`, `npm run check:react-compiler`; unit tests for
      pure helpers (facet filtering, sort, size/age formatting, coverage inference).

## WP5 — Web grab dialog

- [ ] T501 [WP5] `GrabDialog` (standalone export; props allow a pre-resolved target): release
      summary (single / MIX), title picker (query prefilled, candidates, unlinked row),
      episodic scope (season + coverage, inferred per D11), destination (client select, derived
      path), options (D7 label; override only when rejected), footer consequence line + CTA states.
- [ ] T502 [WP5] Wiring from row grab and Grab selected; mutation call; per-release outcome
      toasts; conflict → offer replace-in-progress retry.
- [ ] T503 [WP5] i18n ×10; selector ids; lint/test/react-compiler.

## WP6 — E2E, notes, docs

- [ ] T601 [WP6] e2e `indexer-search.spec.ts` (Tier-F): seed indexers (newznab + torrent) and
      clients; search; assert health entries; toggle a facet; grab linked to a seeded title →
      tracked download appears; grab unlinked → Activity shows manual-import candidate. Force one
      indexer failure (toxiproxy or bad key) → Retry failed heals.
- [ ] T602 [WP6] Release-notes draft entry; docs mention under Indexers.
- [ ] T603 [WP6] Hand the operator the exact run command; do not run the gate.

## Capstone

- [ ] T901 One `cargo clippy` pass over touched crates; targeted suites; web lint/test.
- [ ] T902 Journal closed; decisions D1–D16 recorded as final; PR description drafted.
