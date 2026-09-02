# Tasks: Indexer Search

**Input**: [spec.md](./spec.md), [plan.md](./plan.md). Constitution applies (C8 targeted tests,
web lint before handoff; C9 isolated worktree, gitflow branch, signed commits by the reviewer).
Format: `[ID] [WP] Description`. Work packages run sequentially, one agent at a time.

## WP1b — Fold the query subject into the interactive-search job

- [ ] T111 [WP1b] Delete the sibling module (`catalog/indexer_search.rs`), its tests, registry
      field and hash-domain variant; keep the search-client facet-less dispatch.
- [ ] T112 [WP1b] `InteractiveReleaseSearchRequest` gains `query`/`kind`/`indexer_ids`/`categories`;
      exactly one of title/query; `indexer_ids` restricts dispatch for both subjects.
- [ ] T113 [WP1b] Job context subject enum; query branch calls the indexer client with the
      single-indexer routing plan; results get parsed metadata + default-profile block codes (D6).
- [ ] T114 [WP1b] Indexer view: `elapsed_ms`, `priority` (D15).
- [ ] T115 [WP1b] `issue_interactive_release_candidate_token` (D4) reusing the start path's
      subject resolution and `attach_candidate_tokens`.
- [ ] T116 [WP1b] GraphQL additive fields + `issueInteractiveReleaseCandidateToken`; schema regen;
      graphql-compat.
- [ ] T117 [WP1b] Tests extended in the existing interactive-search modules (unit, integration,
      integration_graphql).

## WP3 — Grab

- [ ] T301 [WP3] Unlinked grab (D8): smallest add-request change that reaches every client,
      orphan submission row, `ReleaseGrabbed` event, category from routing, permission.
- [ ] T302 [WP3] GraphQL `queueUnlinkedRelease(input: {searchId, downloadUrl, downloadClientId})`;
      schema regen.
- [ ] T303 [WP3] Tests: orphan submission + event written; tracked-download poller surfaces it
      for manual import; unknown url ⇒ NotFound; permission denial.

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

## WP7 — Download to browser (D17, FR-028)

- [ ] T701 [WP7] `DownloadClient::fetch_download_artifact(&DownloadClientAddRequest) ->
      ResolvedDownloadArtifact` (default: unsupported); router implementation reuses
      `prepare_download_request` with a host-side fetch policy for NZB URLs; router test with a
      stubbed NZB endpoint.
- [ ] T702 [WP7] `download_interactive_search_artifacts(actor, search_id, download_urls)` in
      `interactive_release_search.rs`: gate, per-url lookup (D4), sequential fetch, single file
      or `tar.gz` bundle with deduped names; one `ReleaseGrabbed` history event per release on
      success; unit tests over naming, dedupe, archive contents, and the emitted events.
- [ ] T703 [WP7] `POST /api/indexer-search/artifacts` route (new `indexer_search_routes.rs`,
      mounted in `main.rs` beside the avatar proxy); unauthorized / bad body / app-error mapping
      tests in the middleware style.
- [ ] T704 [WP7] Web: row "Download" + footer "Download selected" (magnet rows excluded),
      `saveDownloadResponse` lifted to a shared util, filename from `Content-Disposition`;
      i18n ×10; selector ids; lint/test/react-compiler.
- [ ] T705 [WP7] e2e: extend `indexer-search.spec.ts` with a single-row `.nzb` download and a
      two-row `.tar.gz` download (Playwright download events); do not run the flow.

## Capstone

- [ ] T901 One `cargo clippy` pass over touched crates; targeted suites; web lint/test.
- [ ] T902 Journal closed; decisions D1–D16 recorded as final; PR description drafted.
