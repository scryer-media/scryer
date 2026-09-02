# Tasks: Library Location, Folder Ownership, and Cross-Library Movement

**Input**: [spec.md](./spec.md), [plan.md](./plan.md)
**Prerequisites**: plan approved; relocation-prototype coordination note read
(plan.md "Prior & In-Flight Work").

**Format**: `[ID] [P?] [Story] Description` — `[P]` = parallelizable with its
neighbors (different files, no ordering dependency). File paths are the expected
primary touch points; see plan.md "Project Structure".

**Conventions for every task**: the project constitution
([specs/constitution.md](../constitution.md)) applies — notably C1 (new migrations
only), C8 (targeted tests per work package; web lint before handoff), and C9
(isolated worktrees, gitflow branches, signed commits).

## Implementation status (2026-09-01, branch tip `bfd553f09`)

A task is checked only where a landed commit names it or the code plainly carries
it; the evidence is cited on the task line. `[~]` marks a task that is partly
landed and names what is still open. Phases 6, 7 and 8 are **not built** and stay
unchecked. T095 (operator-run e2e) and T096 (final acceptance) stay unchecked:
both are execution steps this pass does not perform.

| Phase | State | Commits |
|---|---|---|
| 1 Setup | done | `0874f57d5` |
| 2 Foundational | done | `74cf5c6c9`, `911513e48`, `f5a3fc7db`, `0f6ba5034`, `4f35cc766` |
| 3 US1 folder match | done | `d044283f8`, `d67be5e3f`, `098788cf8` |
| 4 US2 root move | done | `6b6e22b45`, `5243362a1`, `082783251`, `6a89b7021`, `c42fcea57` |
| 5 US9 verification + backfill | done | `d5a6882ad`, `c42fcea57` |
| 6 US3 adoption | **not built** | — `location/adoption.rs` holds accounting value types only; no `FilesAlreadyThere` execution path exists; the web mode radio is disabled ("Not available yet") |
| 7 US4 change root | **not built — gated** | — T060's coordination gate (the in-flight `feature/library-root-relocation` prototype) never opened; `LocationOperationType::RootChange` has no producer |
| 8 US5 consolidate root | **not built** | — depends on Phase 7; `LocationOperationType::RootConsolidation` has no producer |
| 9 US6/US7 transfer + merge | done | `c98adde09`…`477f278b9`, `9136c7992` |
| 10 US8 + polish | T090–T094 done | `0f0c16012`, `a302cfb57`, `bfd553f09`, `5f0450c47`, `d4ffd16cd` |

## Phase 1: Setup

- [x] T001 Add `crc-fast` workspace dependency; benchmark available algorithms on
      representative hardware and record the chosen default (expected CRC-64/NVME)
      in `crates/scryer-application/src/location/verify.rs` module docs.
- [x] T002 [P] Add verification-depth user preference (full | quick) to settings
      runtime + typed settings definitions + GraphQL settings surface
      (`crates/scryer-application/src/settings/runtime/`, schema).
      *Evidence*: `0874f57d5` — `verificationSettings` query +
      `updateVerificationSettings` mutation. **No web settings UI**: the
      preference is API-only in this branch.
- [x] T003 [P] Scaffold `crates/scryer-application/src/location/` module tree
      (model, preview, classify, executor, verify, collisions, merge, adoption,
      ownership_guard) with types compiling end-to-end.

## Phase 2: Foundational (blocking all user stories)

**Schema.** Sequential — shared migration registry:

- [x] T010 Forward migration `synthetic_root_ids` (SQLite + PostgreSQL): synthetic
      root id column/registry, backfill from path-derived ids, transactional remap
      of `titles.root_folder_id` and all other referents; retain legacy id for
      diagnostics. Dual-datastore migration tests with seeded catalogs. (D1, FR-078)
- [x] T011 Forward migration `media_file_full_hashes`: nullable `full_blake3`,
      `move_crc` (algorithm-tagged), `hash_computed_at` on media files. (FR-041)
- [x] T012 Forward migration `location_operations`: operation rows, per-title
      checkpoints, per-file verification records, owned-entity rows. (D5)
- [x] T013 Replace `root_folder_id_for_path` call sites with synthetic-id lookups;
      path change no longer implies identity change
      (`crates/scryer-domain/src/lib.rs` + call sites). Depends on T010.
      *Evidence*: `74cf5c6c9` — new roots allocate `root_<uuid>`;
      `root_folder_id_for_path` survives as legacy migration/lookup only
      (`scryer-domain/src/lib.rs:184-188`) and its remaining callers are test
      fixtures.

**Engines** (parallel after T011/T012):

- [x] T014 [P] Verified streaming copy in `location/verify.rs`: copy loop feeding
      crc-fast + blake3 hashers; fsync; depth-governed read-back (full w/ per-platform
      cache bypass: `F_NOCACHE` macOS, `O_DIRECT`/fadvise Linux; quick = existing
      sampled proof via `fs_integrity`); persist hashes + verification record;
      quick-floor fallback recorded. Unit tests incl. injected corruption. (FR-040–044)
- [x] T015 [P] Operation runner in `location/executor.rs`: state machine
      (queued→…→completed variants), per-title checkpointing, safe-cancel points,
      restart resume, stale-scope rule (expected partials resumable; foreign changes
      to unprocessed inputs → stale error). (FR-030–033, FR-089, FR-092)
- [x] T016 [P] Ownership guard in `location/ownership_guard.rs`: persisted +
      in-process (title, root) ownership; choke-point helper; wire into scan,
      import, rename, title-delete, media-file mutation, root-config, policy
      automation/maintenance entry points; audit test enumerating guarded entry
      points. (FR-084, D7)
- [x] T017 [P] Shared preview core in `location/preview.rs`: plan builder,
      full-plan fingerprint, complete counts + sampled items (rename-plan pattern),
      free-space estimation incl. recycle-volume cost, depth statement, typed
      confirmation hook. (FR-080–082)
- [x] T018 [P] Collision/dedup engine in `location/collisions.rs`: destination-wins
      naming, source-library suffix + numeric disambiguation, sidecar/companion
      grouping and follow-renames, canonical-sidecar preservation, per-platform
      case-sensitivity rules, dedup gate on full BLAKE3 with size+sampled prefilter,
      recycle-unavailable preserve+rename+warn path. (FR-072–075, FR-090, D4)
- [x] T019 [P] Hardlink detection (link count > 1) surfaced into previews with
      seeding/disk warnings. (FR-085)

**Checkpoint**: foundational engines unit-tested; no user-facing surface yet.

## Phase 3: User Story 1 — folder-match correction (P1) 🎯 MVP

- [x] T020 Backend: change-folder preview + apply on `folder_ownership.rs` seams
      (detach old-folder associations, claim, scan, rebuild, release old folder to
      unmatched discovery); explicit no-op for currently owned folder.
      (FR-001–005, FR-014)
- [x] T021 Backend: swap + takeover flows, atomic commit-or-nothing, displaced-title
      repair reason "Folder ownership changed by user". (FR-006–008)
- [x] T022 GraphQL: `changeTitleFolderPreview` / `applyTitleFolderChange` (+ swap /
      takeover variants) in `crates/scryer-interface*`; schema regeneration;
      integration tests in `crates/scryer/tests/integration_graphql/`.
- [x] T023 [P] Web: **Change folder** dialog (folder browser scoped to current
      library roots, ownership states, media counts, no-files-moved statement),
      owned-folder resolution UI, i18n (10 locales), `npm run lint`.
- [x] T024 Story tests: US1 acceptance scenarios 1–6 as lib/integration tests;
      byte-for-byte no-file-change assertion (SC-001).

**Checkpoint**: US1 independently shippable.

## Phase 4: User Story 2 — move to another root (P1)

- [x] T030 Bulk/single classification in `location/classify.rs`: cross-library /
      root-move / no-op / catalog-only / incompatible / needs-resolution; grouped
      counts; zero omissions. (FR-015–017, FR-076)
- [x] T031 Root-move planner: destination folder calculation from destination
      naming policy (folder-name repair), per-title plan items into shared preview.
      (FR-012–013)
- [x] T032 Root-move executor path: rename on same filesystem; verified copy via
      T014 otherwise; catalog ownership flip at per-title checkpoint; recycle /
      empty-dir cleanup ordering. (FR-031–032, FR-044)
- [x] T033 Fileless catalog-only fast path (no move-mode selection). (FR-076)
- [x] T034 GraphQL: `locationOperationPreview` / `startLocationOperation` /
      `cancelLocationOperation` / `resumeLocationOperation` + operation query;
      integration tests.
- [x] T035 [P] Web: destination library/root controls in single + bulk title
      editing; move-mode chooser; shared preview experience; classification groups
      with no-op/catalog-only visibility; i18n; lint. (FR-010–011)
- [x] T036 [P] Activity: operation states, counters, per-title expansion, depth
      stamp surfaced (initial version). (FR-043, FR-091)
- [x] T037 Blocked-title rules: active download/import exclusion in preview with
      deselection. (FR-086)
- [x] T038 Story tests: US2 scenarios 1–5 incl. cross-filesystem verify + restart
      resume (SC-002) and corruption detection at both depths (SC-006).

**Checkpoint**: core value shipped — verified physical moves inside a library.

## Phase 5: User Story 9 — verification preference, backfill, client copies (P3 but unblocks catalog convergence early)

- [x] T040 Depth preference honored end-to-end; preview/Activity/per-file stamping
      "verified (full|quick)" incl. fallback flag. (FR-042–043)
- [x] T041 `FullHashBackfill` job on existing job infra: single-threaded, throttled
      reads, resumable cursor, skip unavailable mounts + operation-owned files +
      already-hashed; Activity/jobs visibility. (FR-047, D9)
      *Evidence*: `d5a6882ad` — job key `full_hash_backfill`, "Full Hash
      Backfill", Maintenance category, 30-minute interval
      (`jobs/definitions.rs:187,250,344,402`). The spec states no cadence (see
      checklist CHK006).
- [x] T042 Scan-side invalidation: changed quick hash clears stored full hashes and
      re-queues backfill; scans never full-hash. (FR-046)
- [x] T043 Download-client completed-download copy path adopts the streaming
      CRC/BLAKE3 machinery + depth preference
      (`crates/scryer-infrastructure-workflow/src/workflow/file_importer.rs`,
      import workflow copy sites). (FR-045)
- [x] T044 Story tests: US9 scenarios 1–5; backfill non-interference (SC-007).

## Phase 6: User Story 3 — files are already there (P2)

**Built (2026-09-01).** Matcher, verifier, preview accounting, and executor
branch landed in `cf9f92bcd`; the web mode radio is enabled with the accounting
panel in `18ec0b542`; a vanished source folder blocks the title instead of
failing the preview (`f5dc6a71b`); refused titles get
the deselect control the copy promises (`09bb114c9`).

- [x] T050 Adoption matcher in `location/adoption.rs`: stored identity + size +
      sampled proof (+ persisted full BLAKE3 where present); accounted-for /
      missing / additional / ambiguous accounting. (FR-050–051) — `cf9f92bcd`;
      matching is exclusion-first, tiers FullHash > SampledProof > IdentityOnly.
- [x] T051 Adoption preview + blocked-confirmation rules; stale-source-mount
      allowance; user-owned source cleanup with provable-redundancy recycle
      exception. (FR-052–053) — `cf9f92bcd`; unaccounted media are Blocked plan
      items and the shared confirm refuses; FR-053 recycle requires a
      full-hash-proven verification record.
- [x] T052 [P] Web: adoption mode in the move workflow; accounting UI; i18n;
      lint. — `18ec0b542`, `09bb114c9`.
- [x] T053 Story tests: US3 scenarios 1–4, incl. rejection when tracked media is
      unaccounted for. — `cf9f92bcd` (`lib_tests`), plus the vanished-source
      regression test in `f5dc6a71b`; e2e flow `ui-location-adoption` landed in
      the e2e repo (`e1a3805`).

## Phase 7: User Story 4 — change root (P2, absorbs prototype)

**Built (2026-09-01).** The gating prototype never landed; the phase was built
fresh against the spec (planner `17cd37374`, executor + `OperationEpilogue` +
traveling recycle bin `627b93377`, GraphQL + web `b51f30973`), with the
prototype's uncommitted worktree used read-only as reference. Its divergences
were catalogued and the spec won every one — notably the `RELOCATE` phrase was
dropped for the shared `MOVE` typed confirmation (Clarifications 2026-09-01).

- [x] T060 Root-change path onto the shared operation model: one root-change
      operation type, shared preview payload, shared executor; typed
      confirmation via T017 hook (shared `MOVE`, not the prototype's
      `RELOCATION_CONFIRMATION`). (FR-020–021, FR-029) — `17cd37374`,
      `627b93377`, `b51f30973`; the prototype-landing precondition dissolved.
- [x] T061 Every-title accounting + blocked-title repair gate; no exclusions;
      default/role retention over synthetic ids. (FR-021–023) — `17cd37374`;
      same root id on both sides via `LibraryRepository::set_root_path`
      (`627b93377`).
- [x] T062 Unmanaged-content classification (managed / companion / unknown) and
      retirement block; empty-dir-only cleanup. (FR-027–028) — `17cd37374`;
      classification fails closed on an unreadable entry.
- [x] T063 Recycle/retirement ordering: retire root config only after recycling
      completes; resume treats in-retirement root as allowlisted
      (`recycle_bin.rs` allowlist interaction). (FR-087) — `627b93377`; the
      recycle bin travels with the root (rename, EXDEV copy fallback, merge on
      resume) per Clarifications 2026-09-01.
- [x] T064 [P] Web: **Change root** action on root rows; typed confirmation UI;
      i18n; lint. — `b51f30973`; the adoption variant is deliberately refused
      on this branch (a root change's destination must be empty, so files can
      never already be there).
- [x] T065 Story tests: US4 scenarios 1–5 incl. cross-filesystem root move and
      restart resume. — `17cd37374` / `627b93377` (`lib_tests::root_move`),
      GraphQL round trip in `b51f30973`; e2e flow in flight in the e2e repo.

## Phase 8: User Story 5 — consolidate root (P2)

**Built (2026-09-01).** Planner and executor landed in `85fe8bbb1`
(`ConsolidationTail` rides inside the root-change epilogue, so both FR-020
branches share one settle point); GraphQL + web in `b51f30973`. Consolidation
offers **Move with Scryer only** — `FILES_ALREADY_THERE` is refused by name
(`root_consolidation_mode_not_supported`); see the CHK003 note.

- [x] T070 Consolidation planner: seven-way preview classification (FR-024);
      unrelated-title name-collision uniquing (FR-025, suffixed by the source
      root's last path segment); layout preservation vs destination naming
      (FR-026); default-root transfer rule (FR-022). — `85fe8bbb1`.
- [x] T071 Consolidation executor over collisions/dedup engine; merge handoff
      for overlapping titles. — `85fe8bbb1`; Phase 9's merge engine had already
      landed, so the handoff is real (`merge_target_title_id`), not the stub
      this task anticipated.
- [x] T072 [P] Web: **Consolidate root** action + preview groups; i18n; lint. —
      `b51f30973`; one FR-020 control with the refusal pair cross-routing
      between the two branches.
- [x] T073 Story tests: US5 scenarios 1–4; dedup-via-recycle and
      recycle-unavailable preserve+rename (SC-003). — `85fe8bbb1`
      (`lib_tests::consolidation`), GraphQL round trip in `b51f30973`; e2e flow
      in flight in the e2e repo.

## Phase 9: User Stories 6+7 — cross-library transfer and merge (P2/P3)

- [x] T080 Identity detection: canonical metadata identities + redirects; unique →
      merge preview; none → transfer; ambiguous → resolution-required;
      same-name-no-identity → never auto-merge. (FR-055)
- [x] T081 **Merge rule deliverable**: state, in
      `specs/0001-library-location-and-movement/merge-inventory.md`, the rule the
      engine implements — the destination wins everything except the merging
      title's media file records and its history, and everything else retires
      with it through the ordinary title-delete path. Superseded the per-table
      disposition appendix. (D8, FR-064, FR-066)
- [x] T082 Transfer-without-match: settings/tags carryover, destination inheritance,
      root + folder naming assignment, source removal after verify. (FR-056)
- [x] T083 Facet conversion series↔anime: automatic facet change, invalid/reset/
      meaning-change settings surfaced, folder-name-only recalculation with
      files-keep-names statement. (FR-057–058)
- [x] T084 Series-movie link + media-kind dispositions; collection preservation/
      remap notes in preview. (FR-060–062)
- [x] T085 Merge engine in `location/merge.rs`: full identity map first
      (episodes/specials/links), block on unmapped episode-scoped records,
      transactional unions per table group at title checkpoint, destination-wins
      settings, additive unions, source-only mapped retention. (FR-063–067, D8)
- [x] T086 Media-role resolution per logical slot incl. multi-episode split roles;
      preview shows every role change; no silent primary demotion. (FR-068–070)
- [x] T087 [P] Web: cross-library destinations, incompatibility explanations,
      merge preview summary (wins/carries/unions/drops), conflict resolver; i18n;
      lint. (FR-017, FR-071)
- [x] T088 Story tests: US6 scenarios 1–5, US7 scenarios 1–5; mixed A/B/C→A bulk
      classification (SC-005); merge preview-vs-outcome parity (SC-004).

## Phase 10: User Story 8 + polish

- [x] T090 Activity completeness: full state/counter/per-title contract, warnings,
      initiating user, dedup/rename asset listing split, depth stamp final form.
      (FR-091, US8.1/8.4)
      *Evidence*: states, counters, per-title expansion, warnings, initiating
      user and the depth stamp in `082783251` / `6a89b7021`; the per-file
      dedup/rename listing in `d4ffd16cd` — `locationOperationAssets(id)` reads
      the confirmed plan plus checkpoints (`location/asset_listing.rs`),
      done-vs-planned by the executor counters' settled rule, with the Activity
      per-title asset section on top.
- [x] T091 Cancel/resume hardening across all operation types: safe checkpoints,
      no repeated verified work, restart-mid-copy resume, stale-plan behavior.
      (FR-092, FR-089, SC-002)
      *Evidence*: `28bff508a`, `bfd553f09` — two code gaps closed (unreadable
      stored plan now declines instead of erroring; a settled operation's leftover
      ownership claims are released at boot) plus five new tests, incl.
      `a_finished_operations_leftover_ownership_claims_are_released_at_boot` and
      `a_transfer_whose_flip_committed_before_the_crash_cleans_up_on_resume`.
- [x] T092 Media-server targeted refresh on completion. (FR-088)
      *Evidence*: `0f0c16012` — `location/media_server_refresh.rs` (which folders)
      → `media_servers/refresh.rs` (which servers, path-mapped) →
      `ExternalIdentityVerifier::refresh_media_server_paths`. Failures are
      logged and swallowed; they cannot delay or fail an operation.
- [x] T093 API deprecation: `TitleOptionsInput.rootFolderId` typed error for
      tracked-file titles; creation path untouched; migration note for clients.
      (FR-077, SC-009)
      *Evidence*: `a302cfb57` — `DIRECT_ROOT_WRITE_RETIRED` on the options path
      **and** the `addTitle` reuse branch; field marked `@deprecated`; creation
      and fileless reassignment unchanged. Client migration note is in
      [release-notes-draft.md](./release-notes-draft.md).
- [x] T094 [P] Docs + release notes; spec `Status` flip; checklist re-run
      (`checklists/requirements.md`).
      *Evidence*: spec `Status` now records the shipped/unbuilt split;
      [release-notes-draft.md](./release-notes-draft.md) added (draft for the
      release tooling, not a `release-notes/` file); `checklists/requirements.md`
      re-run — 32 checked with citations, 8 gap-noted, six spec-edit candidates
      raised for operator decision.
- [ ] T095 [P] E2E flows for the release gate (folder correction; root move;
      adoption; root change; consolidation w/ dedup; cross-library merge;
      cancel/resume) — handed to the operator to execute. *Progress*: the
      adoption flow landed in the e2e repo (`e1a3805`, `ui-location-adoption`);
      root-change and consolidation flows are being authored; execution stays
      with the operator.
- [x] T096 Final acceptance: one full targeted-suite pass across touched crates +
      web lint; SC-001..SC-009 walked and evidenced.
      *Suite pass (2026-09-01, worktree tip `8a08e2853`)*: clippy zero warnings
      across the nine campaign-touched crates; `scryer-application`
      `lib_tests::` 880 + `location::` 344; `scryer` `integration_graphql` all
      335; `scryer-infrastructure-library` root tests 5; web `npm run lint`
      clean and `npm test` 535/535.
      *SC walk*: SC-001 folder correction leaves bytes untouched —
      `ui-location-folder-match` digest assertions + folder-change lib tests.
      SC-002 cross-fs mechanics + restart resume — EXDEV copy path, checkpoint
      and crash-window resume tests; the 50 GB scale claim itself is
      operator-gate evidence (T095), not unit-testable. SC-003 never
      overwrite/delete — collision engine preserve+rename tests incl.
      recycle-unavailable, consolidation dedup-via-recycle, FR-053
      full-hash-proven adoption recycle. SC-004 preview matches execution —
      stale-fingerprint refusals (incl. merge-state hashing) at the GraphQL
      boundary; web forces a fresh preview on `plan_changed`. SC-005 bulk
      classification 100% — classification counts follow draft downgrades
      (`f5dc6a71b`), web `accountingCloses` re-checks the arithmetic,
      `ui-location-bulk-mixed`. SC-006 corruption detected before source
      removal — verify-engine byte-flip tests; depth recorded, location ops
      forced full. SC-007 backfill — full-hash-backfill tests (yields, skips
      active-operation files, resumes). SC-008 displaced takeover title
      repairable — takeover tests + `ui-location-takeover`. SC-009
      `rootFolderId` retired — `DIRECT_ROOT_WRITE_RETIRED` tests, `@deprecated`
      schema field, preserved creation path pinned.

## Dependencies & Execution Order

- Phase 2 blocks everything except T020–T024 (US1 needs only ownership-guard stubs
  from T016 and no copy engine).
- T010 → T013; T011/T012 → T014/T015; T014+T015+T017 → Phase 4.
- Phase 5 (T041–T043) needs T011+T014; independent of Phases 6–9.
- Phase 7 (T060) is gated on coordination with the in-flight relocation branch
  (`feature/library-root-relocation`) — do not build on it directly; if it lands
  first, rebase; if abandoned, cherry-pick.
- Phase 8's overlapping-title consolidation completes only after T085 (merge).
- MVP line: end of Phase 3. Core-value line: end of Phase 4. Everything after is
  independently landable per phase.

## Parallel Example

After T012 lands, three work streams can proceed concurrently without file
overlap: (1) T014 verify engine, (2) T015 executor + T017 preview, (3) T020–T024
US1 slice. Web tasks marked [P] parallelize against their backend counterparts once
the GraphQL contract for the phase is merged.
