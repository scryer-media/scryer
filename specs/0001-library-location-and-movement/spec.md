# Feature Specification: Library Location, Folder Ownership, and Cross-Library Movement

**Feature Branch**: `feature/library-location-spec`
**Created**: 2026-08-30
**Status**: Implemented (2026-09-01) — all nine user stories built on
`feature/library-location-movement`. US3 (adoption), US4 (change root) and US5
(consolidate root) landed `cf9f92bcd`..`b51f30973`; the relocation-prototype
gate on US4 dissolved and the phase was built fresh against this spec. Final
acceptance (tasks.md T096) and the operator-run e2e gate (T095) are still
pending; known deltas are recorded in
[checklists/requirements.md](./checklists/requirements.md) and the tasks-file
phase notes.
**Input**: Operator product plan (2026-08-30) + plan review amendments + operator decisions recorded in the Clarifications section below.

## Summary

Unify how Scryer manages where titles and media live. Four user intentions that look
similar today have materially different consequences, and each gets its own explicit
workflow:

1. **Change folder match** — correct which existing folder belongs to a title.
   Changes catalog ownership and rescans. Never moves files.
2. **Move to another root** — move one or more title folders between roots in the
   same library.
3. **Change or consolidate a library root** — relocate all managed content from one
   root to a new path, or into another root in the same library.
4. **Move to another library** — transfer title ownership, media, and related catalog
   data into a compatible destination library, merging with an existing destination
   title when necessary.

Every location-changing workflow follows the same interaction pattern: the user
chooses **Move with Scryer** (Scryer performs and verifies the filesystem work) or
**Files are already there** (the user moved files externally; Scryer verifies and
adopts the destination). Every workflow produces a complete fingerprinted preview,
requires explicit confirmation, runs as a resumable Activity-visible operation, and
never overwrites or permanently deletes colliding content.

Direct path changes stop behaving like ordinary metadata edits. Anything that changes
a title folder, root, or library enters the appropriate preview-and-confirm workflow.

## Product Language

Use this terminology consistently across UI, API, and documentation:

| Term | Meaning |
|---|---|
| **Folder match** | The directory currently owned by one title. |
| **Root** | A configured storage location belonging to one library. |
| **Library** | The policy and ownership boundary containing titles and roots. |
| **Move with Scryer** | Scryer performs and verifies the filesystem operation. |
| **Files are already there** | The user moved files externally; Scryer verifies and adopts the destination. |
| **Merge** | The destination already contains the same canonical title; records are combined. |
| **Consolidate root** | Moving one root's managed contents into another existing root in the same library. |
| **Quick check** | Verification using the sampled head+tail content proof plus size. |
| **Full verification** | Verification using a streaming CRC compared against a full destination read-back. |

## Boundaries

- Folder-match correction never moves filesystem content.
- Root changes never cross library boundaries.
- Cross-library movement is always an explicit title or bulk-title operation.
- Movies move only between movie libraries.
- Series and anime move within their own facet or between series and anime libraries.
- Series↔anime moves automatically convert the title facet and apply
  destination-library policies.
- Movie↔episodic transfers are not supported, with one carve-out: titles participating
  in **series-movie links** and **media kinds** (movie-kind titles inside episodic
  libraries) follow the rules in FR-060–FR-062.
- Library scans always use the quick hash. Full hashing happens only in the streaming
  copy pass and the background backfill job (FR-042, FR-047).

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Correct a wrong folder match (Priority: P1)

A user notices Scryer matched a title to the wrong folder after a scan. They open
title editing, choose **Change folder**, pick the correct folder from the title's
current library roots, and Scryer reassigns ownership and rescans — without moving
a single file.

**Why this priority**: This is the highest-frequency repair action and requires no
filesystem mutation, no verification engine, and no operation runner. It is
independently shippable on existing folder-ownership seams.

**Independent test**: Seed a library with two folders and one title matched to the
wrong one; correct the match; verify catalog ownership, media associations, and the
old folder returning to unmatched discovery — with filesystem contents untouched.

**Acceptance scenarios**:

1. **Given** a movie matched to an unrelated folder, **When** the user selects an
   unowned folder and confirms, **Then** old-folder media associations are detached,
   the selected folder is assigned and scanned, and the old folder appears in
   unmatched discovery.
2. **Given** a series matched to the wrong folder, **When** the match is corrected,
   **Then** episode media associations are rebuilt from the new folder and the
   title's identity, monitoring, quality settings, tags, history, and requests are
   unchanged.
3. **Given** the user selects the title's existing folder, **When** they confirm,
   **Then** the UI explains the title already owns it and submits nothing.
4. **Given** the selected folder is owned by another title, **When** the user chooses
   **Swap folders**, **Then** each title owns the other's former folder and both are
   rescanned atomically.
5. **Given** the selected folder is owned by another title, **When** the user chooses
   **Take over folder**, **Then** the edited title owns it, and the displaced title
   surfaces in the repair experience with reason "Folder ownership changed by user".
6. **Given** a swap or takeover where the selected folder cannot be scanned or
   ownership cannot commit, **When** the operation fails, **Then** neither title is
   left with a partially applied ownership change.

### User Story 2 — Move titles to another root in the same library (Priority: P1)

A user selects one or more titles and picks a different destination root. Scryer
previews the calculated destination folders (using the destination naming policy),
the user picks **Move with Scryer**, and the files move with verification.

**Why this priority**: The most common physical-move need (rebalancing storage), and
it exercises the whole core machinery — preview, verified copy, checkpointed
operation, Activity — inside one library where merge complexity is absent.

**Independent test**: Move a title between two roots on the same filesystem (rename
path) and across filesystems (copy+verify path); verify catalog, folder naming
repair, and Activity reporting.

**Acceptance scenarios**:

1. **Given** a title on root A, **When** the user selects root B and **Move with
   Scryer**, **Then** the preview shows current and destination folders, file count,
   and total size; on confirm the operation completes and catalog ownership updates
   only after verification.
2. **Given** the title's folder name is stale versus the naming policy, **When** it
   moves roots, **Then** the calculated destination folder repairs the name and the
   preview showed the change before confirmation.
3. **Given** a bulk selection mixing titles on root A and root B with B as
   destination, **When** previewed, **Then** A-titles classify as moves, B-titles as
   no-ops, and no title is silently omitted.
4. **Given** a monitored title with **no files on disk**, **When** its root is
   changed, **Then** the change classifies as a catalog-only reassignment and
   completes without entering move-mode selection (FR-076).
5. **Given** a cross-filesystem move, **When** files copy, **Then** each file is
   verified at the configured verification depth before the source copy is recycled
   or removed, and the applied depth is recorded per file.

### User Story 3 — Adopt files the user already moved (Priority: P2)

A user moved a title (or a whole root) with Finder/rsync/another host, then tells
Scryer **Files are already there**. Scryer scans the destination, accounts for every
tracked file, and adopts the new location.

**Independent test**: Move a title's folder externally, run adoption, verify
accounting states (accounted-for / missing / additional / ambiguous) and that
confirmation is blocked while required media is missing.

**Acceptance scenarios**:

1. **Given** an externally moved title, **When** adoption runs, **Then** tracked
   media are matched using stored identity, size, and stored content signatures, and
   the same preview model as a managed move is shown.
2. **Given** a tracked file is missing or ambiguous at the destination, **When** the
   user attempts to confirm, **Then** confirmation is blocked with a clear unresolved
   state — never a guess.
3. **Given** the source mount is stale or unavailable, **When** the destination can
   be proven from stored catalog information, **Then** adoption proceeds and source
   cleanup is left to the user.
4. **Given** adoption completes, **Then** catalog ownership updates only after
   verification, and Scryer recycles a redundant source copy only when it can prove
   redundancy.

### User Story 4 — Change a root to a new path (Priority: P2)

A user replaces a root's path (new disk, new mount) via **Change root** in library
settings, choosing managed move or external adoption.

**Why this priority**: An in-flight prototype already implements the managed-move
path (see plan.md, "Prior & In-Flight Work"); this story absorbs and finishes it
under the unified model.

**Independent test**: Change a root to a new empty path on the same filesystem and
across filesystems; verify every assigned title relocates, the root keeps its
identity/role/default status, and source-root retirement rules hold.

**Acceptance scenarios**:

1. **Given** a root with N titles, **When** the root is changed to a new path with
   **Move with Scryer**, **Then** all N titles are accounted for in the preview;
   blocked titles must be repaired before the source root can be retired — titles
   cannot be excluded from a root change.
2. **Given** the root is the library default, **When** its path changes, **Then** it
   remains the default and keeps its logical identity (synthetic id, FR-078).
3. **Given** unmanaged content exists at the source root, **When** the operation is
   previewed, **Then** unknown files/directories are listed separately, are never
   silently deleted or abandoned, and root removal is blocked until the user resolves
   them.
4. **Given** a completed root change, **Then** only empty source directories were
   removed automatically, and removal happened only after full verification.
5. **Given** a root-wide operation, **When** the user confirms, **Then** the stronger
   typed confirmation is required.

### User Story 5 — Consolidate a root into another root (Priority: P2)

A user folds root A into existing (non-empty) root B in the same library.

**Independent test**: Consolidate a root into a destination containing overlapping
titles, colliding folder names for unrelated titles, identical files, and colliding
sidecars; verify each classification and outcome.

**Acceptance scenarios**:

1. **Given** consolidation, **When** previewed, **Then** the preview identifies:
   titles moving into unused destination folders; titles merging with an existing
   destination title; folder-name collisions between unrelated titles; media
   collisions; identical files eligible for dedup; sidecar collisions requiring
   rename; untracked content blocking retirement.
2. **Given** two unrelated titles calculating the same destination folder, **When**
   previewed, **Then** the incoming folder receives a unique previewed destination
   name or the operation stays blocked — unrelated titles never merge over a name.
3. **Given** a default source root consolidated into another root, **Then** the
   destination becomes the default; consolidating a non-default root leaves the
   default unchanged.
4. **Given** consolidation completes, **Then** the source root's relative folder
   layout was preserved where practical, and every changed folder name was shown
   before confirmation.

### User Story 6 — Move titles to another library (Priority: P2)

A user moves titles (single or bulk, possibly spanning several source libraries)
into a compatible destination library, including series↔anime facet conversion.

**Independent test**: Move a movie between movie libraries; move a series to an anime
library and verify facet conversion, destination policy inheritance, and preserved
history/requests/monitoring; verify a movie→series attempt is rejected with a clear
explanation.

**Acceptance scenarios**:

1. **Given** no matching title in the destination, **When** the transfer runs,
   **Then** the title transfers with valid title-specific settings and tags; inherited
   source-library behavior is replaced by destination defaults; root assignment and
   folder naming come from the destination; history, requests, monitored state, and
   media associations are preserved; the source title is removed only after the
   destination is complete and verified.
2. **Given** a series→anime move, **When** previewed, **Then** the facet conversion
   is shown along with any setting that becomes invalid, resets, or changes meaning.
3. **Given** a bulk selection from libraries A, B, C with A as destination, **When**
   previewed, **Then** every title independently classifies as cross-library
   transfer, same-library root move, no-op, incompatible, or needs-resolution — and
   the job cannot start while unresolved or incompatible titles remain included.
4. **Given** a destination option no selected title can use, **When** shown, **Then**
   it is disabled with an explanation naming the incompatible source libraries or
   facets.
5. **Given** a movie→series or movie→anime attempt, **Then** it is rejected with a
   clear explanation (subject to the series-movie-link rules, FR-060–FR-062).

### User Story 7 — Merge into an existing destination title (Priority: P3)

The destination library already contains the same canonical title; the move combines
their records under explicit, destination-wins rules.

**Independent test**: Merge two copies of one series with overlapping episodes,
divergent settings, tags, history, and media files; verify every rule in FR-063–FR-071
via the preview and the post-merge catalog.

**Acceptance scenarios**:

1. **Given** a unique canonical identity match, **When** previewed, **Then** a merge
   preview is produced; a same-name title without matching identity is never
   auto-merged; conflicting or ambiguous identities require user resolution before
   the job starts.
2. **Given** a merge, **Then** the destination title id, metadata identity,
   monitoring, explicit settings, quality configuration, naming behavior, and
   library inheritance win; additive data (tags, history, requests, import records,
   acquisition history, compatible title-linked records) is unioned; the preview
   summarizes which settings win, what carries forward, what is unioned, and what is
   dropped or converted.
3. **Given** both titles have a primary file for the same logical slot, **Then** the
   destination primary remains primary and the incoming primary becomes additional;
   an incoming primary fills a slot with no destination primary; the preview shows
   every role change.
4. **Given** episode or special identities that cannot be mapped unambiguously,
   **Then** the operation blocks rather than attaching files or episode-scoped
   records to guessed identities (FR-066).
5. **Given** the merge completes, **Then** the source title is removed only when
   every required relationship has been transferred or intentionally resolved.

### User Story 8 — Monitor, cancel, and resume operations (Priority: P3)

Every location operation is visible in Activity with progress, warnings, per-title
results, safe cancellation, retry, and restart-surviving resume.

**Independent test**: Start a large cross-filesystem move, restart Scryer mid-copy,
verify resume from the last verified checkpoint; cancel another run and verify it
stops at the next safe title checkpoint with completed titles consistent.

**Acceptance scenarios**:

1. **Given** any location operation, **Then** Activity shows queued / preparing /
   moving / verifying / reconciling / cleaning-up / completed /
   completed-with-warnings / canceled / failed; titles, files, and bytes processed
   versus totals; current title and file; counts of merges, dedups, renames, no-ops,
   unresolved items; source and destination; initiating user; and a concise failure
   or warning explanation, with expandable per-title results.
2. **Given** cancellation, **Then** the operation stops at the next safe title
   checkpoint; completed titles remain consistent and visible; the operation can be
   retried or resumed without repeating verified work.
3. **Given** a process restart mid-operation, **Then** the persisted operation
   resumes from its last verified checkpoint; expected partial destination state is
   resumable, while foreign changes to not-yet-processed inputs stop the job with a
   stale-state error requiring a new preview (FR-089).
4. **Given** an operation completes, **Then** the final summary lists renamed and
   deduplicated assets separately from media files, and states the verification
   depth applied.

### User Story 9 — Operator controls import verification depth; catalog hashes converge (Priority: P3)

An operator chooses between full verification (default) and quick check for
download-client import copies, and a background job slowly backfills full-file
hashes across the catalog. Location operations are not governed by the
preference: a move over existing library content always verifies full (FR-042).

**Independent test**: Flip the preference and verify import copies honor it while
location operations ignore it and always verify full, stamping the applied depth
either way; run the backfill job against a catalog with unhashed files and verify
convergence, throttling, and skip rules.

**Acceptance scenarios**:

1. **Given** any preference, **When** a location operation's cross-filesystem
   copy completes, **Then** the destination is fully read back and compared
   against the streaming CRC before the source is touched — the quick-check
   preference does not apply to moves.
2. **Given** the quick-check preference, **When** a download-client import copy
   completes, **Then** verification uses the sampled head+tail proof plus size,
   and the result records "verified (quick)".
3. **Given** full verification cannot run for a file, **Then** verification falls
   back to the quick check (never below it) and the fallback is recorded.
4. **Given** the backfill job runs, **Then** it hashes only files missing a persisted
   full hash, single-threaded and low-priority, skipping unavailable mounts and files
   owned by an active location operation, and resumes across restarts.
5. **Given** a library scan detects a changed quick hash for a file, **Then** stored
   full hashes for that file are invalidated and the file re-enters the backfill
   queue — the scan itself never computes a full hash.

### Edge Cases

- A title with no files on disk changes root or library → catalog-only fast path
  (FR-076); no move-mode selection.
- Source file has hardlink count > 1 (e.g., seeding) → preview warns; cross-device
  move breaks the link and doubles disk; recycling one link frees nothing (FR-085).
- Case-insensitive filesystems (macOS/Windows) → collision detection uses
  per-platform case rules; previews match what the filesystem will actually do.
- Multi-episode file spanning covered and uncovered episode slots → primary for
  uncovered slots, additional for covered ones (FR-069).
- Recycle bin disabled, unavailable, or rejecting a file → preserve + collision
  rename + visible warning; never permanent deletion (FR-073).
- Crash mid-copy → partial destination state is expected and resumable; not stale.
- Stale source mount during adoption → proceed when the destination is provable from
  stored catalog data; otherwise a clear unresolved state (US3.3).
- Series with series-movie-linked titles moves libraries → linked titles follow
  FR-060–FR-062; no silent orphaning.
- Renamed canonical sidecar (`movie.nfo`, `tvshow.nfo`) → preserved incoming
  artifact; destination canonical file stays authoritative.
- Active download or import on a selected title → title blocked from the move until
  the work finishes; bulk preview identifies it and allows deselection.
- Destination folder collision where the "colliding" path is the moving title's own
  source folder under a different case → treated as a rename, not a collision.

## Requirements *(mandatory)*

### Folder-match correction (US1)

- **FR-001**: Movie, series, and anime title editing MUST offer **Change folder**,
  restricted to folders under the title's current library roots.
- **FR-002**: The dialog MUST show: title and current folder; current root and
  library; candidate-folder ownership state (unowned / owned by this title / owned
  by another title); tracked-media counts for old and selected folders; and an
  explicit statement that no files will be moved.
- **FR-003**: For an unowned destination: preview the ownership change; detach media
  associations originating from the old folder; assign the selected folder; scan it
  and rebuild associations; return the old folder to unmatched discovery.
- **FR-004**: Folder-match correction MUST NOT change metadata identity, monitoring,
  quality settings, tags, history, or requests — only folder ownership and the media
  associations derived from it.
- **FR-005**: Selecting the currently owned folder MUST be an explicit no-op with an
  explanation, not a submitted job.
- **FR-006**: An owned folder MUST NOT be silently stolen. The UI MUST show the
  current owner and offer **Swap folders**, **Take over folder**, or **Cancel**.
- **FR-007**: Takeover MUST explain that the former owner becomes unmatched and needs
  repair; the displaced title MUST surface in the repair/unmatched experience with
  reason "Folder ownership changed by user".
- **FR-008**: Swap and takeover MUST be atomic from the user's perspective: scan or
  commit failure leaves neither title partially changed.

### Title move workflows (US2, US6)

- **FR-010**: Single-title and bulk title editing MUST gain **Destination library**
  and **Destination root** controls.
- **FR-011**: Changing either field MUST open a move workflow (Move with Scryer /
  Files are already there / Cancel) instead of saving the title — except the
  fileless fast path (FR-076).
- **FR-012**: The move preview MUST show: current library/root/folder; destination
  library/root/calculated folder; facet changes; naming changes; conflicts; file
  counts; total size; and expected media-role changes.
- **FR-013**: Destination folders for title moves MUST be calculated from the
  destination library's active folder-naming policy and shown before confirmation;
  root moves MAY thereby repair stale folder names.
- **FR-014**: Folder-match correction (FR-001–FR-008) adopts a chosen existing folder
  and MUST NOT recalculate or move it.
- **FR-015**: Bulk selections MAY span multiple source libraries. Every selected
  title MUST classify independently as: cross-library transfer; same-library root
  move; no-op; incompatible; or needs-resolution. The preview MUST group these with
  counts and omit no title.
- **FR-016**: A bulk job MUST NOT start while included titles are unresolved or
  incompatible; the user resolves them or removes them from the operation.
- **FR-017**: Disabled destination options MUST explain why, naming the incompatible
  source libraries or facets.

### Root management (US4, US5)

- **FR-020**: Each configured root in library settings MUST offer **Change root**
  (to a new unconfigured path, or to another existing root in the same library —
  the latter being consolidation).
- **FR-021**: A root whose path is replaced MUST retain its logical identity, role,
  and default status (see FR-078, synthetic root ids).
- **FR-022**: Consolidating a default source root makes the destination the default;
  consolidating a non-default root leaves the default unchanged.
- **FR-023**: A root change MUST account for every title assigned to the source root;
  titles cannot be excluded. Blocked titles MUST be repaired before the source root
  is retired.
- **FR-024**: The consolidation preview MUST classify: titles moving into unused
  destination folders; titles merging with existing destination titles; folder-name
  collisions between unrelated titles; media collisions; dedup-eligible identical
  files; sidecar/non-media collisions requiring rename; and untracked/unsupported
  content that prevents safe source-root retirement.
- **FR-025**: Unrelated titles MUST never merge because they calculate the same
  destination folder; the incoming folder gets a unique previewed name or the
  operation remains blocked.
- **FR-026**: Root replacement SHOULD preserve the source root's relative folder
  layout where practical; consolidation MAY apply destination naming rules to avoid
  collisions, with every changed folder name previewed.
- **FR-027**: Scryer MUST distinguish managed title content, recognized companion
  assets (NFO, subtitles, artwork, trickplay, etc. — which move with their title),
  and unrelated root-level content. Unknown content MUST appear separately in the
  preview and MUST NOT be silently deleted or abandoned.
- **FR-028**: A root MUST NOT be removed while unexplained content remains at the
  source; only empty source directories may be removed automatically, and only after
  successful verification.
- **FR-029**: Root-wide operations MUST require the stronger typed confirmation.

### Managed move execution — "Move with Scryer" (US2, US4, US5, US6)

- **FR-030**: A managed move is asynchronous: accepted immediately, returning an
  operation identifier, monitored through Activity.
- **FR-031**: Execution order per operation: validate (paths, ownership, permissions,
  free-space expectations, active-operation conflicts) → build a stable fingerprinted
  preview → require explicit confirmation → move one title at a time → verify each
  copy at the configured depth → apply configured file/folder permissions at the
  destination → update catalog ownership only after that title's destination content
  is verified → recycle or preserve redundant source files per collision rules →
  remove only empty source directories → finalize root or source-title removal only
  after the complete operation succeeds.
- **FR-032**: Same-filesystem moves MUST use an atomic rename when safe (no
  verification pass needed); cross-filesystem moves use copy + verification + source
  cleanup through the approved recycle or removal path.
- **FR-033**: The user MUST NOT need to keep a browser open; restart resumes the
  persisted operation from its last verified checkpoint.

### Verification and integrity (US9; applies to all copies)

- **FR-040**: Every copy performed by a location operation MUST compute a streaming
  CRC over the bytes as they are copied (read-once at the source), using the fastest
  algorithm the `crc-fast` crate provides.
- **FR-041**: The same streaming pass MUST also compute the full-file BLAKE3; both
  values are persisted with the media file, separately from the sampled head+tail
  proof.
- **FR-042**: Two verification depths exist: **full** reads the destination back
  in full (cache-bypassed where the platform allows) and compares against the
  streaming CRC; **quick check** uses the sampled head+tail proof plus size. The
  depth is a user preference **for download-client import copies only** (full by
  default). **Location operations always verify full**: existing library content
  is never put at a user-selectable level of risk by a move (operator decision,
  2026-09-01). The quick check remains the universal floor in both contexts: full
  verification falls back to it when a full read-back cannot run, and
  verification never drops below it.
- **FR-043**: The applied depth MUST be stamped on the operation: the preview states
  the depth that will apply; Activity and per-file results record
  "verified (full)" or "verified (quick)" (including fallback cases).
- **FR-044**: Source deletion or recycling MUST be gated on the applicable
  verification passing for that file.
- **FR-045**: Download-client completed-download moves that copy (cross-device) MUST
  use the same streaming CRC machinery and honor the same depth preference.
- **FR-046**: Library scans MUST continue to use only the sampled head+tail proof.
  A scan that detects a changed quick hash MUST invalidate that file's persisted
  full hashes and re-queue it for backfill; scans never compute full hashes.
- **FR-047**: A background backfill job MUST slowly hash media files missing a
  persisted full BLAKE3: single-threaded, low I/O priority, yielding to real work,
  resumable, skipping unavailable mounts and files owned by an active location
  operation, and skipping files that already have a current full hash.

### External adoption — "Files are already there" (US3)

- **FR-050**: Adoption MUST NOT simply replace stored path prefixes. It scans the
  destination and matches tracked media using stored identity information, size,
  media characteristics, and stored content signatures (sampled proof always; full
  BLAKE3 where already persisted).
- **FR-051**: Adoption MUST present accounted-for, missing, additional, and ambiguous
  files, and apply the same title/folder/library/merge preview as a managed move.
- **FR-052**: Confirmation MUST be blocked while required tracked media is missing or
  ambiguous. Insufficient proof produces a clear unresolved state, never a guess.
- **FR-053**: Catalog ownership updates only after verification. Source cleanup is
  left to the user unless Scryer can prove a redundant source copy is safe to
  recycle. A stale or unavailable source mount MUST NOT block adoption when the
  destination is provable from stored catalog information.

### Cross-library transfer (US6)

- **FR-055**: Destination-title detection MUST use stable metadata identities and
  redirects, never title text alone. Unique canonical match → merge preview; no
  match → new destination title; conflicting/ambiguous identities → user resolution
  before start; same-name-without-identity → never auto-merged.
- **FR-056**: Transfer without a destination match preserves valid title-specific
  settings, tags, history, requests, monitored state, and media associations;
  replaces inherited source-library behavior with destination defaults; assigns the
  selected destination root; applies destination folder naming; and removes the
  source title only after the destination is complete and verified.
- **FR-057**: Series↔anime moves convert the facet automatically and show every
  setting that becomes invalid, resets, or changes meaning.
- **FR-058**: Facet conversion recalculates **folder** names only. Files keep their
  names; aligning file names with the destination policy is a follow-up via the
  existing rename feature, and the preview says so.
- **FR-060**: The spec's movie/episodic boundary MUST define behavior for
  series-movie links: when a series moves libraries, its linked movie titles are
  listed in the preview with an explicit disposition (move together, keep linked in
  place, or block pending user choice) — never silently orphaned.
- **FR-061**: Movie-kind titles residing in episodic libraries (media kinds) move
  under the rules of their containing library's facet, with their kind preserved.
- **FR-062**: Collections spanning the movement boundary MUST NOT block a move;
  collection membership is preserved or remapped, and the preview notes any
  cross-library collection consequences.

### Merge rules (US7)

- **FR-063**: On merge, the destination wins: title id, metadata identity,
  monitoring, explicit settings, quality configuration, naming behavior, and
  destination-library inheritance are kept.
- **FR-064**: Additive data is unioned: tags, history, requests, import records,
  acquisition history, and other compatible title-linked records. Source-only
  compatible records are retained when they can be mapped safely.
- **FR-065**: Source media MUST be mapped onto destination movie, episode, or
  series-movie identities. Destination episode and collection metadata wins for
  duplicate records.
- **FR-066**: Episode-identity mapping applies to **every episode-scoped record
  being unioned** (media files, history rows, import records, and any other record
  referencing source episode ids), not only media. Ambiguous episode or special
  identities block the operation rather than attaching records to guessed
  identities.
- **FR-067**: The source title is removed only when every required relationship has
  been transferred or intentionally resolved.
- **FR-068**: Media roles resolve per logical slot (movie title / linked series
  movie / mapped episode), not per filename: destination primary stays primary;
  incoming primary becomes additional where a destination primary exists, and stays
  or becomes primary where none exists; incoming and existing additionals stay
  additional.
- **FR-069**: A multi-episode file may be primary for uncovered episodes and
  additional for covered ones.
- **FR-070**: No destination primary is ever silently demoted by a library move;
  role changes appear in the preview; users may re-promote later via existing
  media-file controls.
- **FR-071**: The merge preview summarizes which destination settings win, which
  source values carry forward, which data is unioned, and which values are dropped
  or converted.

### Filesystem collision and deduplication rules

- **FR-072**: Destination content always wins the pathname; incoming content is
  deduplicated or renamed, never overwritten.
- **FR-073**: Identical files (proven by matching **full-file BLAKE3** — never the
  sampled proof): keep the destination copy; recycle the redundant source copy;
  retain or merge catalog associations onto the survivor; record the dedup in the
  operation summary. If the recycle bin is disabled, unavailable, or rejects the
  file: preserve the incoming copy, rename it per FR-074, complete with a visible
  warning, and never fall back to permanent deletion.
- **FR-074**: Non-identical media collisions: keep the destination filename; rename
  the incoming file with a readable source-library suffix plus numeric
  disambiguation if needed; preserve both media records; apply role rules
  independently of the filename decision; show the generated name in the preview.
- **FR-075**: Sidecars and companion assets (NFO, subtitles, artwork, trickplay,
  thumbnails, related directories): destination keeps its name; incoming renamed
  with the same suffix scheme; related asset groups move together; companion names
  follow a renamed media file to preserve the relationship; renamed canonical items
  (`movie.nfo`, `tvshow.nfo`) are preserved incoming artifacts while the
  destination's canonical file stays authoritative; BLAKE3-identical assets
  deduplicate via the recycle rule. The final summary lists renamed and deduplicated
  assets separately from media files.

### API and compatibility

- **FR-076**: A monitored title with no tracked files on disk changing root or
  library is a catalog-only reassignment: no move-mode selection, no filesystem
  work, classified distinctly ("catalog-only") in bulk previews.
- **FR-077**: The existing `TitleOptionsInput.rootFolderId` replace-on-write
  semantics MUST be retired for existing titles with tracked files: such a change is
  rejected with guidance toward the move workflow (or the fileless fast path when it
  applies). Root selection at title creation remains a direct assignment.
- **FR-078**: Root identity moves to synthetic stable ids via a new forward
  migration (shipped migrations are immutable). Path changes never change a root's
  identity; titles keep valid root references across path changes and
  consolidations.

### Preview and confirmation (all workflows)

- **FR-080**: All location-changing workflows share one preview model including:
  operation type and execution mode; source and destination libraries, roots,
  folders; title/file counts and total bytes; same-volume vs cross-volume where
  known; naming changes; identity matches and proposed merges; media-role changes;
  dedups; collision renames; sidecar/trickplay renames; folder swaps/takeovers;
  facet conversions; no-op and catalog-only titles; blocked/unresolved titles;
  unmanaged root content; estimated free-space requirement (including recycle-copy
  cost when the recycle bin is on another volume); recycling availability; and the
  verification depth that will apply.
- **FR-081**: Previews are fingerprinted; a changed filesystem, catalog, selection,
  or destination invalidates the confirmation and requires regeneration. Large
  previews return complete counts with sampled item lists (fingerprint covers the
  full plan, not just the sample).
- **FR-082**: High-impact operations require an explicit confirmation step
  summarizing that files, title ownership, or library membership will change;
  root-wide operations use typed confirmation (FR-029).

### Concurrency, permissions, and coexistence

- **FR-083**: The initiating user MUST hold management permission for the source
  library and every destination library involved.
- **FR-084**: While an operation owns a title or root, Scryer MUST prevent
  conflicting: library scans; imports; renames; title deletion; media-file deletion
  or primary changes; other location operations on the same title; root
  removal/configuration changes affecting the operation; and policy-automation or
  maintenance jobs acting on the owned titles. Unrelated titles and libraries
  operate normally.
- **FR-085**: The preview MUST detect source files with hardlink count > 1 and warn:
  cross-device moves break the link (seeding copies orphaned, disk usage doubles);
  recycling one link frees no space.
- **FR-086**: Titles with active downloads or imports are blocked from entering a
  move until the work finishes; bulk preview identifies them and allows removal from
  the selection.
- **FR-087**: Recycle ordering during root retirement: the source root's
  configuration is retired only after all recycling for the operation completes;
  resume treats an in-retirement root as still allowlisted for recycling.
- **FR-088**: On completion, Scryer performs a targeted refresh/notification for
  connected media servers covering the affected titles.
- **FR-089**: Resume semantics: the plan fingerprint's staleness check covers
  catalog inputs and not-yet-processed items; expected partial destination state
  from an interrupted copy is resumable, not stale.
- **FR-090**: Collision detection uses per-platform case sensitivity rules so
  previews match actual filesystem behavior on macOS and Windows.

### Activity (US8)

- **FR-091**: Every managed move, adoption, root change, consolidation, folder
  reassignment, and cross-library transfer appears in Activity with the states,
  counters, per-title detail, initiating user, and explanations enumerated in
  US8 scenario 1.
- **FR-092**: Cancellation stops at the next safe title checkpoint; completed titles
  stay consistent and visible; retry/resume never repeats verified work.

## Key Entities

- **Library**: policy and ownership boundary; owns roots and titles; carries facet,
  naming policy, defaults, and per-user management grants.
- **Root**: configured storage path within one library; synthetic stable id
  (FR-078); role and default status independent of path.
- **Title**: catalog entity with facet (movie/series/anime), optional media kind,
  folder match, root reference, settings, tags, monitoring, history, requests.
- **Folder match**: the one directory a title owns; unowned folders live in
  unmatched discovery.
- **Media file**: tracked file with logical slot (title / linked series movie /
  episode), role (primary/additional), sampled head+tail proof, and — once
  computed — persisted full-file BLAKE3 and streaming CRC.
- **Location operation**: persisted, checkpointed, resumable unit of work; one of
  folder-reassignment, root-move (title-scoped), root-change, root-consolidation,
  cross-library transfer, adoption; owns titles/roots for its duration; visible in
  Activity.
- **Operation plan (preview)**: fingerprinted description of every expected move,
  merge, rename, dedup, and catalog change; complete counts with sampled items.
- **Recycle entry**: manifest-backed record of a recycled source copy, linked to the
  operation id.
- **Verification record**: per-file outcome with applied depth (full/quick) and
  fallback status.

## Success Criteria *(mandatory)*

- **SC-001**: A user can correct a wrong folder match end-to-end without any file
  content changing on disk (verified byte-for-byte before/after).
- **SC-002**: A cross-filesystem title move of at least 50 GB completes with every
  file verified at the configured depth, survives a process restart mid-copy, and
  repeats no verified work on resume.
- **SC-003**: No location operation ever overwrites destination content or
  permanently deletes a collision or duplicate, in any acceptance scenario,
  including recycle-unavailable paths.
- **SC-004**: Every acceptance scenario's preview matches the executed outcome
  exactly (no unpreviewed rename, merge, dedup, or role change) or the operation
  stops with a stale-plan error.
- **SC-005**: A bulk move spanning three source libraries classifies 100% of
  selected titles into exactly one class, with zero silent omissions.
- **SC-006**: With the default preference, a corrupted destination copy (byte
  flipped after write) is detected before source removal; with quick check, the
  same operation records "verified (quick)" so the reduced guarantee is auditable.
- **SC-007**: The backfill job reaches full-hash coverage of an idle catalog without
  user-visible impact on scan, import, or playback-facing operations, and never
  touches a file owned by an active operation.
- **SC-008**: A displaced title from a takeover is discoverable in repair with the
  documented reason; zero titles end in an unrepresentable state across all
  scenarios (every title has a classified state at operation end).
- **SC-009**: Legacy `rootFolderId` direct edits on titles with tracked files are
  fully absent from the API surface's accepted inputs; existing clients receive an
  actionable error.

## Clarifications

### Session 2026-09-01 (operator decisions)

- Q: Should the verification-depth preference govern location operations as well
  as import copies → A: **No — moves are forced full.** The preference stays
  user-facing for download-client import copies; library/root moves and every
  other location operation always verify full. Rationale: losing banked library
  data on a move is far more damaging than losing a fresh import the user never
  had. Supersedes the 2026-08-30 depth-preference answer for the location-move
  scope; the quick floor and recorded fallback are unchanged.
- Q: What happens to a recycle bin that lives inside a root whose path is being
  replaced → A: **The bin's contents move with the operation** (always the
  intended design): after recycling completes and before the configuration
  flips, the in-root bin relocates to the destination path, and restores of
  entries recycled before the flip re-anchor onto the new root path.
- Q: Does US5 consolidation offer both execution modes (CHK003 found the spec
  silent) → A: **Move with Scryer only**, implementation-decided and accepted:
  a consolidation's destination is a configured root whose content already
  belongs to other titles, so "files are already there" has no coherent
  meaning there and is refused by name
  (`root_consolidation_mode_not_supported`). A root change refuses it too —
  its destination must be empty or absent, so files can never already be
  there. Adoption of externally-moved content remains the title-scoped US3
  workflow.

### Session 2026-08-30 (operator decisions)

- Q: Copy-verification hashing vs. the codebase's sampled-proof design → A:
  **Streaming CRC** for move-corruption detection, computed during the copy
  (read-once), fastest `crc-fast` algorithm; full-file **BLAKE3 reserved for dedup**
  identity and persisted separately from the sampled proof. Same machinery applies
  to download-client copy moves.
- Q: Full destination read-back on network mounts is costly → A: verification depth
  is **user-decided, not fstype-auto**: full by default, quick-check preference
  available, sampled head+tail proof is the universal floor and fallback.
- Q: Should scans compute full hashes → A: **No. Library scans stay on the quick
  hash unconditionally**; only the copy pass and the backfill job compute full
  BLAKE3; scans invalidate stale full hashes on signature change.
- Q: How do full hashes reach pre-existing files → A: **background backfill job**,
  single-threaded, nice, resumable.
- Q: Path-derived root ids conflict with "change root keeps identity" → A: migrate
  to **synthetic stable root ids** (new forward migration).
- Q: Verification transparency → A: **stamp the applied depth** on previews,
  Activity, and per-file results.
- Q: Prior work → A: an in-flight prototype for root relocation exists
  (see plan.md "Prior & In-Flight Work"); this spec absorbs it rather than
  duplicating it.

## Assumptions

- Root operations remain within one library.
- Folder-match correction never moves files.
- Destination title policy wins during a merge; additive title data is retained or
  unioned.
- Destination primary media always wins a role conflict; incoming conflicting media
  becomes additional.
- Stable metadata identity is required for automatic title merging.
- Series↔anime crossover is supported and converts the facet automatically.
- No operation permanently deletes a collision or duplicate; the configured recycle
  bin is the only automatic destination for redundant source copies; missing
  recycle support causes preservation and rename, not failure or deletion.
- Root-wide operations require every title and unmanaged source item to be resolved
  before the source root is retired.

## Out of Scope

- Automatic file renaming to match the destination library's file-naming policy
  during a move (folder names only; the existing rename feature handles files as a
  follow-up).
- A general catalog-integrity/scrub feature (the persisted CRC/BLAKE3 values enable
  it later; only the backfill job ships here).
- Comparing streamed CRCs against download-client-provided expected checksums
  (future hook; requires client support).
- Movie↔episodic facet conversion.
- Physical-device or cloud-storage migration tooling beyond mounted-filesystem
  moves and adoptions.
