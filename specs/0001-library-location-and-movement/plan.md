# Implementation Plan: Library Location, Folder Ownership, and Cross-Library Movement

**Spec**: [spec.md](./spec.md)
**Status**: Draft — plan approved for task generation; no implementation started.

## Summary

Build a unified location-operation subsystem in `scryer-application` powering four
workflows (folder-match correction, root moves, root change/consolidation,
cross-library transfer) over shared machinery: a fingerprinted preview model, a
checkpointed/resumable operation runner, a verified streaming-copy engine
(CRC + full BLAKE3 in one pass), destination-wins collision/dedup rules, and a
merge engine. Two schema changes underpin it: synthetic stable root ids and
persisted full-file hashes on media files. An in-flight prototype already
implements the root-change-to-new-path slice and is absorbed, not duplicated.

## Technical Context

**Language/runtime**: Rust workspace (async, tokio) + React/TypeScript web app
(`apps/scryer-web`).
**API**: GraphQL — schema source of truth at `api/graphql/schema.graphql`,
resolvers in `crates/scryer-interface*`.
**Storage**: SQLite and PostgreSQL behind `scryer-infrastructure-datastore`.
**New dependency**: `crc-fast` (workspace-level), fastest available algorithm
(expected CRC-64/NVME; confirm by benchmark at implementation time).
**Test surfaces**: `lib_tests` in `scryer-application`; GraphQL integration tests
in `crates/scryer/tests/integration_graphql/`; e2e flows under the existing
release-gate harness.

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

## Constitution Check

Gated by [specs/constitution.md](../constitution.md) v1.1.0. Principal exercise of
C2 (preview before mutate), C3 (nothing silent), C4 (destruction requires proof),
C5 (async/resumable), and C7 (platform differences); C1 governs the three new
migrations. **No deviations.** Complexity justification (C10): one new subsystem
(`location/`), one new dependency (`crc-fast`), three migrations — each defended in
D1–D10 below. Deliberate scope cut: file renaming to destination naming policy is
out of scope (spec "Out of Scope"); folder names only. Security auditing is owned
by a separate review track; this plan is functional correctness only.

## Prior & In-Flight Work

An in-flight prototype (branch `feature/library-root-relocation`, unpublished as of
2026-08-30) implements the root-change-to-new-path slice:
`crates/scryer-application/src/library/relocation.rs` (fingerprinted preview,
typed confirmation, interrupted-job resume), a `LibraryRootRelocation` job type,
and Activity/web wiring.

**Stance**: absorb, don't duplicate. The US4 phase rebases this prototype onto the
shared operation model rather than building a second root-change path. Coordinate
with that branch's owner before building on it; treat its API names as provisional
until it lands.

## Key Design Decisions

- **D1 — Synthetic root ids** (FR-078). Forward migration adds generated stable
  ids, backfills from path-derived ids, and transactionally remaps all referents.
  Path changes stop being identity changes. *Rejected*: remapping on every path
  change, forever.
- **D2 — One streaming pass, two hashers** (FR-040/041). The cross-device copy
  loop feeds each buffer to `crc-fast` and `blake3` while writing; both persisted.
  Read-once at source; every move/import backfills the dedup hash for free.
- **D3 — User-decided verification depth** (FR-042/043). Full (default) = post-copy
  destination read-back with platform cache bypass, compared against the streamed
  CRC; quick = sampled proof + size, and the universal floor/fallback. Applied
  depth stamped per file and per operation.
- **D4 — Dedup requires full BLAKE3** (FR-073). Candidacy pre-filters on
  size + sampled proof; the deciding comparison is always full-hash vs full-hash.
- **D5 — One operation model**. A `location_operations` persistence layer with
  per-title checkpoints, per-file verification records, and safe-cancel points.
  All operation types share it, so Activity, resume, and concurrency guards are
  written once.
- **D6 — Preview = plan + fingerprint**, reusing the rename-plan pattern (complete
  counts, sampled items, fingerprint over the full plan; staleness scope per
  FR-089). Typed confirmation reuses the established pattern.
- **D7 — Concurrency via an operation-ownership registry** (FR-084). Persisted +
  in-process ownership of (title, root) per active operation, consulted through a
  single choke-point helper by every conflicting entry point. No global locks.
- **D8 — Merge maps identities first**. Full source→destination identity map
  (title, episodes, specials, series-movie links) built up front; unmappable
  episode-scoped records block the plan (FR-066); unions execute as transactional
  id-rewrites at the title checkpoint.
- **D9 — Backfill as a standard job** (FR-047). Single-threaded, throttled,
  resumable cursor; scan-side invalidation hooks the existing quick-hash
  comparison.
- **D10 — API compatibility** (FR-076/077). `rootFolderId` stays for creation;
  updates on titles with tracked files return a typed error pointing at the move
  mutations; fileless titles take the catalog-only fast path. New surface is
  additive.

## Project Structure (implementation)

```
crates/scryer-domain/src/lib.rs                     # root id type change (D1)
crates/scryer-infrastructure-datastore/src/migrations/
    synthetic_root_ids.rs                            # D1 forward migration (sqlite+pg)
    media_file_full_hashes.rs                        # full_blake3, crc, verified_depth columns
    location_operations.rs                           # operation/checkpoint/verification tables
crates/scryer-application/src/
    location/                                        # NEW subsystem
        model.rs / preview.rs / classify.rs / executor.rs / verify.rs
        collisions.rs / merge.rs / adoption.rs / ownership_guard.rs
    library/relocation.rs                            # absorbed prototype (US4 phase)
    jobs/                                            # FullHashBackfill registration
    settings/runtime/                                # verification-depth preference
crates/scryer-interface*/ + api/graphql/schema.graphql   # GraphQL surface
apps/scryer-web/                                     # dialogs, move workflow, conflict
                                                     # resolver, activity detail, i18n
```

## Data Model Changes

1. **Roots**: synthetic id primary key; path becomes a mutable attribute; legacy
   path-derived ids backfilled and all referents remapped in one migration.
2. **Media files**: nullable `full_blake3`, `move_crc` (algorithm-tagged),
   `hash_computed_at`; invalidated on scan signature change (FR-046).
3. **Location operations**: operation row (type, mode, state, user, refs, plan
   fingerprint, depth), per-title checkpoints, per-file verification records,
   owned-entity rows for the concurrency registry.

## GraphQL Surface (sketch — names finalized in contracts task)

- `locationOperationPreview` / `startLocationOperation` (fingerprint, mode,
  optional typed confirmation) / `cancelLocationOperation` /
  `resumeLocationOperation`; `locationOperation(id)` query + Activity payloads
- `changeTitleFolderPreview` / `applyTitleFolderChange` (swap/takeover variants)
- `verificationDepth` settings read/write; typed error for legacy `rootFolderId`
  edits (FR-077)

## Delivery

Phasing, ordering, and the MVP line live in [tasks.md](./tasks.md): foundations
(migrations, verify engine, executor, guard) first; US1 is the MVP slice and needs
only ownership-guard stubs; US2 is the first end-to-end consumer; remaining
stories land independently per phase.

## Risks & Mitigations

- **Prototype drift**: the US4 phase starts by diffing the landed prototype;
  shared-model types are shaped so its preview/job maps on with renames.
- **Migration blast radius** (root-id remap): dual-datastore tests with seeded
  catalogs; one transaction; legacy id retained for diagnostics.
- **Full read-back cost**: user-chosen depth, stated in the preview; quick check
  is one setting away.
- **Merge-union table sprawl**: an explicit inventory task enumerates every
  title/episode-id-bearing table before the engine is written; FR-066 fails
  closed.
- **Guard misses an entry point**: single choke-point helper plus an audit test
  enumerating mutating entry points.
- **Case-insensitive collisions**: detection parameterized by destination
  filesystem case rule; Windows CI lane + targeted fixtures.
