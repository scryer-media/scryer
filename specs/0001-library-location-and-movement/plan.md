# Implementation Plan: Library Location, Folder Ownership, and Cross-Library Movement

**Branch**: `feature/library-location-spec` (spec authoring) — implementation lands on
work-package branches per repo convention (`feature/…` off the current release tip,
in isolated worktrees).
**Spec**: [spec.md](./spec.md)
**Status**: Draft — plan approved for task generation; no implementation started.

## Summary

Build a unified location-operation subsystem in `scryer-application` that powers four
workflows (folder-match correction, root moves, root change/consolidation,
cross-library transfer) over shared machinery: a fingerprinted preview model, a
checkpointed/resumable operation runner, a verified streaming-copy engine
(CRC + full BLAKE3 in one pass), destination-wins collision/dedup rules, and a
merge engine. Two schema changes underpin it: synthetic stable root ids and
persisted full-file hashes on media files. An in-flight prototype already implements
the root-change-to-new-path slice and is absorbed, not duplicated.

## Technical Context

**Language/runtime**: Rust workspace (async, tokio) + React/TypeScript web app
(`apps/scryer-web`).
**API**: GraphQL — schema source of truth at `api/graphql/schema.graphql`, resolvers
in `crates/scryer-interface*`.
**Storage**: SQLite and PostgreSQL behind `scryer-infrastructure-datastore`;
migrations are immutable once shipped — new forward migrations only.
**Key existing crates/modules** (verified anchors, 2026-08-30):

| Concern | Anchor |
|---|---|
| Folder ownership seams | `crates/scryer-application/src/folder_ownership.rs` (`title_owns_folder`, `find_other_folder_owner`, `ensure_folder_move_available_to_title`, `claim_title_folder_if_missing`, `unlink_title_media_in_folder`) |
| Safe file moves | `crates/scryer-application/src/fs_safety.rs` (`move_file_exclusive`, `claim_destination`, cross-device detection) |
| Sampled content proof | `crates/scryer-application/src/fs_integrity.rs` (size + BLAKE3 of first/last 1 MiB; explicitly not a full-file hash) |
| Recycle bin | `crates/scryer-application/src/library/recycle_bin.rs` (manifests with `source_operation_id`, pending/committed/quarantined, fail-closed source-root allowlist, configured `base_path`) |
| Fingerprinted plan/apply pattern | `crates/scryer-application/src/library/rename.rs` + `LibraryRenamePlan` (`crates/scryer-interface-media-types/src/types/library.rs`) — sampled items with complete counts |
| Typed confirmation pattern | `crates/scryer-application/src/library/user_delete.rs` (`requires_typed_confirmation`) |
| Operation persistence | `crates/scryer-infrastructure-workflow/src/workflow/stores/workflow_operation_store.rs`; job infra (`JobRun`, `JobRunStatus`, `crates/scryer-application/src/jobs/`) |
| Root identity (to be replaced) | `root_folder_id_for_path` in `crates/scryer-domain/src/lib.rs` — path-derived, platform-normalized |
| Facets | `MediaFacet { Movie, Series, Anime }` in `crates/scryer-domain/src/lib.rs` |
| Title API seam | `TitleOptionsInput.rootFolderId` (replace-on-write today) in `api/graphql/schema.graphql` |
| Media file store | `crates/scryer-infrastructure-library/src/media/search/media_file_store.rs` |
| Full BLAKE3 precedent | `crates/scryer-application/src/application_upgrade/engine.rs` (artifact verification only, today) |

**New dependency**: `crc-fast` (workspace-level), fastest available algorithm
(expected CRC-64/NVME; confirm benchmark at implementation time).
**Testing**: unit + `lib_tests` in `scryer-application`, GraphQL integration tests in
`crates/scryer/tests/integration_graphql/`, web `npm run lint` (typecheck + eslint)
before any web handoff, e2e flows added under the existing e2e harness (operator
runs the gate).
**Platforms**: Linux, macOS, Windows — case-insensitivity and cache-bypass read-back
(`F_NOCACHE` / `O_DIRECT` / `posix_fadvise`) are per-platform concerns in the
verification engine.

## Constitution Check

No `constitution.md` exists yet (this is the first spec-kit spec). Standing repo
rules act as gates:

- Shipped migrations are immutable — all schema change is new forward migrations
  (SQLite + PostgreSQL variants), including the migration-numbering registry.
- CI `web` lane = typecheck AND eslint; run `npm run lint` locally before handing
  off web changes.
- Commits are SSH-signed; release procedure runs through repo release tooling only.
- Implementation happens in isolated worktrees on `feature/`-prefixed branches;
  never directly in shared checkouts.
- No security-audit scope in this workstream (owned elsewhere); functional
  correctness only.

## Prior & In-Flight Work

An in-flight prototype (branch `feature/library-root-relocation`, unpublished as of
2026-08-30) already implements the root-change-to-new-path slice:

- `crates/scryer-application/src/library/relocation.rs` (~1,340 lines):
  `LibraryRootRelocationPreview { fingerprint, … }`,
  `preview_library_root_relocation`, `start_library_root_relocation_job` with typed
  confirmation (`RELOCATION_CONFIRMATION`) and fingerprint check,
  `resume_interrupted_library_root_relocations`.
- Job type `LibraryRootRelocation` in `jobs/definitions.rs`; Activity/web wiring in
  `activity-view.tsx`, `media-library-settings-panel.tsx`, mutations, i18n.

**Plan stance**: absorb, don't duplicate. Phase F below rebases this prototype onto
the shared operation model (synthetic root ids, unified preview payload, shared
verification engine) rather than building a second root-change path. Coordinate
with that branch's owner before building on it; until it lands, treat its
API names as provisional.

## Key Design Decisions

- **D1 — Synthetic root ids** (FR-078). Replace path-derived
  `root_folder_id_for_path` with generated stable ids via a forward migration that
  (a) adds the id column/registry, (b) backfills from existing path-derived ids,
  (c) remaps `titles.root_folder_id` and all other referents transactionally.
  *Rationale*: change-root, consolidation, and resume all get simpler; path changes
  stop being identity changes. *Alternative rejected*: transactional remap on every
  path change (touches more rows on every operation, forever).
- **D2 — One streaming pass, two hashers** (FR-040/041). The cross-device copy loop
  feeds each buffer to both a `crc-fast` hasher and a `blake3` hasher while writing.
  Persist both. *Rationale*: read-once at source; every move/import backfills the
  dedup hash for free. *Note*: this forgoes kernel copy offload
  (`copy_file_range`/reflink), which only applies to same-filesystem copies —
  same-filesystem moves are renames here, so nothing is lost in practice.
- **D3 — User-decided verification depth** (FR-042/043). Setting
  (full default / quick check) governs the post-copy destination read-back; sampled
  head+tail proof is the floor and fallback. Full read-back uses cache bypass
  (`F_NOCACHE` on macOS, `O_DIRECT` or fadvise on Linux) after fsync so it verifies
  media, not page cache. Applied depth is stamped per file and per operation.
- **D4 — Dedup requires full BLAKE3** (FR-073). Identity claims that delete data
  never rest on the sampled proof. Dedup candidacy pre-filters on size + sampled
  proof; the deciding comparison is full-hash vs full-hash (from persisted values or
  an on-demand hash of the specific candidate pair, surfaced as preview cost).
- **D5 — One operation model**. A `location_operations` persistence layer (extending
  the workflow-operation/job-run infra) with per-title checkpoints, per-file
  verification records, safe-cancel points, and resume. All six operation types are
  rows of one model so Activity, resume, and concurrency guards are written once.
- **D6 — Preview = plan + fingerprint, reusing the rename-plan pattern**. Complete
  counts, sampled item lists, fingerprint over the full plan; staleness scope per
  FR-089 (catalog inputs + not-yet-processed items; expected partials are
  resumable). Typed confirmation for root-wide ops reuses the `user_delete` /
  relocation-prototype pattern.
- **D7 — Concurrency via an operation-ownership registry**. New in-process +
  persisted ownership of (title ids, root ids) per active operation; scan, import,
  rename, delete, policy-automation, and maintenance entry points consult it
  (FR-084). No global locks; unrelated work proceeds.
- **D8 — Merge engine maps identities before touching anything**. Build the full
  source→destination identity map (title, episodes, specials, series-movie links)
  up front; any unmappable episode-scoped record blocks the plan (FR-066). Unions
  execute as id-rewrites in one transaction per table group at the title checkpoint.
- **D9 — Backfill as a standard job**. `FullHashBackfill` job on the existing job
  infra: single-threaded, throttled reads, resumable cursor, skip rules per FR-047.
  Scan-side invalidation hooks into the existing quick-hash comparison.
- **D10 — API compatibility**. `TitleOptionsInput.rootFolderId` remains for
  creation; on update of a title with tracked files it returns a typed error
  pointing at the new move mutations (FR-077). Fileless titles take the
  catalog-only fast path (FR-076). New GraphQL surface is additive.

## Project Structure (implementation)

```
crates/scryer-domain/src/lib.rs                     # root id type change (D1)
crates/scryer-infrastructure-datastore/src/migrations/
    synthetic_root_ids.rs                            # D1 forward migration (sqlite+pg)
    media_file_full_hashes.rs                        # full_blake3, crc, verified_depth columns
    location_operations.rs                           # operation/checkpoint/verification tables
crates/scryer-application/src/
    location/                                        # NEW subsystem
        mod.rs
        model.rs                                     # operation types, states, checkpoints
        preview.rs                                   # shared plan builder + fingerprint
        classify.rs                                  # bulk classification (FR-015)
        executor.rs                                  # per-title state machine, resume
        verify.rs                                    # streaming copy: CRC+BLAKE3, read-back tiers
        collisions.rs                                # FR-072..075 naming/dedup/sidecars
        merge.rs                                     # D8 identity map + unions
        adoption.rs                                  # files-already-there matching
        ownership_guard.rs                           # D7 registry
    library/relocation.rs                            # absorbed prototype (Phase F)
    jobs/                                            # FullHashBackfill job registration
    settings/runtime/                                # verification-depth preference
crates/scryer-interface*/                            # GraphQL types, mutations, queries
api/graphql/schema.graphql                           # generated surface
apps/scryer-web/
    components/…                                     # change-folder dialog, move workflow,
                                                     # conflict resolver, root actions,
                                                     # activity operation detail
    lib/graphql, lib/i18n                            # ops + 10 locales
```

## Data Model Changes

1. **Roots**: synthetic id primary key; path becomes a mutable attribute; backfill
   maps legacy path-derived ids; all referents (`titles.root_folder_id`, settings
   mirrors, import roots) remapped in the same migration.
2. **Media files**: nullable `full_blake3`, `move_crc` (algorithm-tagged),
   `hash_computed_at`; invalidated by scan signature change (FR-046).
3. **Location operations**: operation row (type, mode, state, initiating user,
   source/destination refs, plan fingerprint, depth setting at start), per-title
   checkpoint rows, per-file verification records (depth applied, fallback flag),
   owned-entity rows for the concurrency registry.
4. **Recycle manifests**: already carry `source_operation_id`; link rows to
   operations for the summary.

## GraphQL Surface (sketch — names finalized in contracts task)

- `locationOperationPreview(input)` → plan payload (all operation types)
- `startLocationOperation(input { fingerprint, mode, typedConfirmation? })` → id
- `cancelLocationOperation(id)` / `resumeLocationOperation(id)`
- `changeTitleFolderPreview` / `applyTitleFolderChange` (swap/takeover variants)
- `locationOperation(id)` query + Activity subscription payloads
- Settings: `verificationDepth` preference read/write
- Typed error for legacy `rootFolderId` edits (FR-077)

## Phased Delivery (maps to tasks.md)

- **A. Foundations**: crc-fast dep; verification engine (D2/D3); migrations (D1,
  media-file hashes, operation tables); depth preference; ownership guard (D7).
- **B. US1** folder-match correction (no dependency on A's copy engine; can run in
  parallel after ownership-guard stubs).
- **C. US2** root moves + bulk classification + Activity basics (first consumer of
  A end-to-end).
- **D. US9** backfill job + scan invalidation + download-client copy CRC (FR-045).
- **E. US3** adoption.
- **F. US4** absorb relocation prototype onto the shared model; typed confirmation;
  unmanaged-content rules; retirement ordering (FR-087).
- **G. US5** consolidation (collisions/dedup engine exercised fully).
- **H. US6+US7** cross-library transfer, facet conversion, merge engine.
- **I. US8** Activity completeness, cancel/resume hardening, media-server refresh
  (FR-088), API deprecation (FR-077), docs/release notes, e2e flows.

## Risks & Mitigations

- **Prototype drift** (the in-flight relocation prototype evolves or lands mid-build):
  Phase F starts by diffing the landed state; shared-model types are designed so the
  prototype's preview/job maps onto them with renames, not rewrites.
- **Migration blast radius** (root-id remap touches every title): dual-datastore
  migration tests (SQLite+PG) with seeded catalogs; the remap is one transaction;
  the legacy id remains recorded for diagnostics.
- **Full read-back cost surprises users**: depth is user-chosen with a stated
  default; preview states the depth; docs explain the trade-off; quick check is one
  setting away.
- **Merge-union table sprawl**: D8's explicit inventory task (tasks.md T-phase H)
  enumerates every title-id/episode-id-bearing table before the engine is written;
  the blocking rule (FR-066) fails closed on anything unmapped.
- **Concurrency-guard misses an entry point**: the registry is consulted via a
  single choke-point helper; a repo-wide audit task lists mutating entry points and
  asserts guard coverage in tests.
- **Windows/case-insensitive collisions**: collision detection parameterized by the
  destination filesystem's case rule; CI's Windows lane plus targeted unit fixtures.

## Complexity Tracking

No constitution deviations to record (no constitution yet). The one deliberate
scope-cut: file renaming to destination naming policy is out of scope (spec "Out of
Scope") — folder names only, existing rename feature covers files later.
