# Library location and movement — release-notes draft

<!--
  DRAFT — input for the operator's release tooling, not a release file.

  This is not `release-notes/scryer-vX.Y.Z.md`. The release script owns that file
  and the version it carries. Lift the sections below into whichever release
  actually ships this feature line, trim to that release's scope, and delete this
  comment.

  Written 2026-09-01 against `feature/library-location-movement`; updated the
  same day at `b51f30973`, when US3 (adoption), US4 (change root), and US5
  (consolidate root) landed. It now describes the whole of spec 0001.
-->

## Highlights

Scryer now treats *where* a title lives as its own deliberate operation rather
than a metadata edit. Correcting a wrong folder match, moving titles to another
root, adopting files you already moved yourself, changing or consolidating a
library root, and moving titles into another library are each their own
workflow, each with a full preview you confirm before anything happens,
verified copies, and a resumable job you can watch, cancel, and resume from
Activity.

Every copy Scryer performs — including cross-device copies from your download
client — is now checksummed as it is written and verified before the source is
touched. You choose how thorough that verification is, and a background job
gradually fills in full-file hashes for content that was already in your library.

## What changed

### Correct a wrong folder match without moving anything

- **Movie, series, and anime titles have a "Change folder" action.** It browses
  the folders under that title's own library roots, shows which title (if any)
  already owns each folder and how much tracked media sits in it, and states
  plainly that no files will be moved.
- Correcting a match reassigns folder ownership and rescans. Identity,
  monitoring, quality settings, tags, history, and requests are untouched — only
  folder ownership and the media associations derived from it change. The old
  folder returns to unmatched discovery.
- Picking the folder the title already owns is explained as a no-op instead of
  being submitted as work.
- **A folder owned by another title is never silently taken.** Scryer names the
  current owner and offers **Swap folders** or **Take over folder**. A swap
  leaves both titles owning the other's former folder; a takeover surfaces the
  displaced title in the repair experience with the reason "Folder ownership
  changed by user". Either way, a failure part-way through leaves neither title
  partially changed.

### Move titles to another root, with a preview and verification

- **Single-title and bulk title editing gained destination library and
  destination root controls.** Changing either one opens the move workflow
  instead of quietly saving the title.
- The preview is complete before you confirm: current and destination
  library/root/folder, the destination folder name calculated from the
  destination library's naming policy, file counts, total size, collisions,
  expected media-role changes, hardlink warnings, estimated free space, and the
  verification depth that will apply. Large plans show complete counts with a
  sampled item list.
- **Bulk selections classify every title and omit none.** Each is grouped as a
  cross-library transfer, a same-library root move, a no-op, a catalog-only
  reassignment, incompatible, or needing resolution — with counts per group, and
  an explanation on any destination that is unavailable. The job cannot start
  while unresolved or incompatible titles are still included.
- Titles with an active download or import are held out of the move until that
  work finishes, and the preview lets you drop them from the selection.
- **A monitored title with no files on disk takes a catalog-only fast path**: no
  move-mode question, no filesystem work, just the reassignment.
- Moves within one filesystem use an atomic rename. Moves across filesystems copy,
  verify, then clean up the source through the recycle path. Catalog ownership
  flips only after that title's destination content has been verified, and only
  empty source directories are ever removed automatically.
- The plan is fingerprinted. If the filesystem, catalog, selection, or
  destination changes between preview and confirm, the confirmation is refused
  and you are asked to review a fresh plan.

### Tell Scryer the files are already there

- **The move workflow's "Files are already there" mode is live.** If you already
  moved a title's files yourself — Finder, rsync, a download client writing
  straight to the destination — pick the same destination in the move dialog
  and choose this mode. Scryer proves the files instead of copying them: a file
  with a stored full hash is read back completely and re-verified; otherwise
  its size and head-and-tail content are checked against what the catalog
  knows, and the result records which proof applied.
- **Nothing is adopted on faith.** Every tracked file must be accounted for at
  the destination. A title with tracked media that cannot be matched is named
  and blocked, and the preview lets you drop it from the selection rather than
  guessing. Disagreeing content excludes a match; mere absence of a hash does
  not.
- A source that has since vanished — an unplugged drive, an already-deleted
  folder — does not fail the preview or block the adoption of titles whose
  files are present at the destination.
- Leftover source files are yours: Scryer only recycles a source copy it has
  **proven** redundant by full hash, and otherwise leaves the source alone.

### Change a root's path, moving everything on it

- **Every saved library root has a "Change root" action.** Give it a new path
  and Scryer moves the entire root: the root keeps its identity, its
  library-default status, and every title assignment — only the path changes.
  Connected clients, requests, and settings that reference the root never
  notice.
- The preview is a complete accounting: **every title on the root is listed as
  moving, catalog-only, or blocked — there is no way to leave one behind.** A
  blocked title stops the operation until it is repaired.
- Everything found under the root is classified as tracked media, companion
  files, or unexplained content. **Unexplained content is never moved and never
  deleted**; it is listed by name in the preview and it keeps the old location
  standing — only empty directories are ever removed automatically.
- A recycle bin living inside the root travels with it: recycled entries remain
  restorable after the move, onto the new path.
- The old path's configuration is retired only after every title has settled,
  verification has passed, and recycling has finished.

### Consolidate one root into another

- **The same "Change root" action can fold a root into another root of the same
  library.** Point it at a configured root instead of a new path (either way —
  if you type a path that turns out to be a configured root, or pick a root
  that turns out not to be one, the dialog switches branches for you).
- The preview classifies every title seven ways before you confirm: moving into
  unused folders, merging with a destination title, folder-name collisions,
  media-name collisions, proven-identical files eligible for dedup,
  companion-name collisions, and untracked source entries — all seven shown,
  zeros included.
- **Two unrelated titles with the same folder name are never merged.** The
  incoming folder gets a readable unique name derived from its source root, and
  every changed folder name is shown in the preview by name. Titles that truly
  are the same canonical title go through the full merge engine, with the same
  destination-wins rules as a cross-library merge.
- If the source root was the library default, **the destination root becomes
  the default** — the preview says so out loud, since it changes where new
  content lands.
- The source root's configuration is retired at the end; unexplained content
  keeps the old path standing (still configured) without blocking the
  consolidation itself.

### Copies are verified — moves always fully, imports at your chosen depth

- **Every copy computes a checksum over the bytes as they are written**, in the
  same single read of the source that also produces a full-file BLAKE3 hash.
  Both are stored with the media file.
- **Library and root moves always verify fully.** The destination is read back
  completely — bypassing the OS cache where the platform allows it — and
  compared against the checksum computed during the copy, before the source is
  ever touched. This is not configurable: content already in your library is
  never put at a selectable level of risk by a move.
- **For download-client import copies, verification depth is your preference.**
  *Full* (the default) reads the imported copy back completely; *quick check*
  compares size plus the head-and-tail content proof. Quick is the floor in
  both contexts: full verification falls back to it when a full read-back
  cannot run, and verification never drops below it.
- **The depth applied is always stated.** The preview says what will apply
  before you confirm; Activity and each file's result record "verified (full)"
  or "verified (quick)", including files that fell back.
- A source file is recycled or removed only after the verification that applies
  to it has passed.

### Full-file hashes converge in the background

- A low-priority **Full Hash Backfill** maintenance job gradually hashes media
  files that have no stored full-file hash yet. It is single-threaded and
  throttled, yields to real work, resumes across restarts, and skips unavailable
  mounts, files already hashed, and any file owned by a running location
  operation.
- **Library scans are unchanged and stay cheap.** A scan still uses only the
  head-and-tail proof; it never computes a full hash. When a scan sees that a
  file's quick signature changed, it clears that file's stored full hash and
  re-queues it for the backfill job.

### Collisions and duplicates are never resolved by overwriting

- **Destination content always keeps its name.** Incoming content is renamed or
  deduplicated — never overwritten, never permanently deleted.
- A duplicate is only ever a duplicate when full-file hashes match. Proven
  duplicates keep the destination copy and recycle the redundant incoming one,
  and the operation summary records the dedup. **If the recycle bin is disabled,
  unavailable, or refuses the file, Scryer preserves the file and renames it with
  a visible warning** — it never falls back to deleting.
- Non-identical media collisions keep the destination filename and rename the
  incoming file with a readable source-library suffix, adding a number only if
  needed. Both media records are preserved, and the generated name is shown in
  the preview.
- Sidecars and companion assets — NFOs, subtitles, artwork, trickplay,
  thumbnails, related directories — follow the same rule and move as a group, so
  a renamed media file keeps its companions. A renamed `movie.nfo` or
  `tvshow.nfo` is preserved as an incoming artifact while the destination's
  canonical file stays authoritative.
- Collision detection follows the destination filesystem's own case rules, so the
  preview matches what macOS and Windows will actually do. A "collision" that is
  really the moving title's own folder under a different case is treated as a
  rename.
- The preview warns when a source file has more than one hardlink — a
  cross-device move breaks the link, doubles the space used, and recycling one of
  the links frees nothing.

### Move titles into another library

- **Titles can move into a compatible destination library**, singly or in bulk,
  including bulk selections that span several source libraries.
- Movies move between movie libraries. Series and anime move within their facet
  or between series and anime libraries. A movie-to-episodic attempt is refused
  with an explanation rather than half-applied.
- **A series moving to an anime library converts its facet automatically**, and
  the preview shows every setting that becomes invalid, resets, or changes
  meaning under the destination library's policies.
- Facet conversion recalculates **folder** names only. Files keep their names,
  and the preview says so — aligning file names with the destination policy is a
  follow-up through the existing rename feature.
- Transfers preserve valid title settings, tags, history, requests, monitored
  state, and media associations; behavior inherited from the source library is
  replaced by the destination's defaults; the destination assigns the root and
  the folder name. The source title is removed only after the destination is
  complete and verified.
- Series-movie links, movie-kind titles inside episodic libraries, and
  collections that span the boundary are given an explicit disposition in the
  preview instead of being silently orphaned.

### Merging into a title the destination already has

- **When the destination library already holds the same canonical title, the move
  becomes a merge.** Matching is done on stable metadata identities and their
  redirects, never on the title text: a same-named title with no shared identity
  is never auto-merged, and two plausible candidates are handed back to you to
  resolve before anything starts.
- **The destination wins.** Its title identity, monitoring, explicit settings,
  quality configuration, naming behavior, and library inheritance are kept.
  Additive data — tags, history, requests, import records, acquisition history,
  and other compatible title-linked records — is carried over alongside the
  destination's own.
- **The merge preview says what will happen to each kind of data** before you
  confirm: which destination settings win, what carries forward, what is combined,
  what is not carried over, and which settings disagreed.
- Media roles are resolved per logical slot rather than per filename. A
  destination primary stays primary and the incoming primary becomes an
  additional file; an incoming primary fills a slot that has none. A file covering
  several episodes can be primary for the uncovered ones and additional for the
  rest. **No destination primary is ever silently demoted**, every role change
  appears in the preview, and you can re-promote later with the existing
  media-file controls.
- **If an episode or special cannot be mapped unambiguously, the merge is blocked
  rather than attaching records to a guess.** The source title is removed only
  once every required relationship has been carried over or deliberately
  resolved.

### Follow, cancel, and resume a move from Activity

- **Every location operation appears in Activity** with its state (queued,
  preparing, moving, verifying, reconciling, cleaning up, completed, completed
  with warnings, canceled, failed), titles/files/bytes done versus total, the
  current title and file, counts of merges, dedups, renames, no-ops and
  unresolved items, source and destination, who started it, the verification
  depth applied, and expandable per-title results.
- **You do not have to keep a browser open.** The operation is persisted; if
  Scryer restarts mid-copy it resumes from the last verified checkpoint and
  repeats no verified work. A partly written destination file left by a crash is
  recognized as resumable, not stale.
- **Canceling stops at the next safe title boundary.** Titles that already
  finished stay finished and consistent, and the operation can be resumed or
  retried afterward.
- If something outside the operation changed the inputs it has not processed yet,
  the job stops with a stale-plan error and asks for a fresh preview instead of
  acting on an outdated plan.
- While an operation owns a title or a root, Scryer holds off conflicting work on
  exactly those entities — scans, imports, renames, title and media-file
  deletion, root reconfiguration, and other location operations. Unrelated titles
  and libraries carry on normally.

### Connected media servers are refreshed when a move finishes

- **When an operation completes, Scryer asks each enabled Plex, Jellyfin, or Emby
  connection to refresh just the folders that actually changed**, translated
  through that connection's configured path mappings. This is the first feature
  to use those path mappings, so check them if a refresh does not land where you
  expect. A media server that is unreachable is logged and skipped; it never
  fails or delays the move.

## Upgrade notes

- **No manual database work is required.** Startup applies three new migrations
  on both SQLite and PostgreSQL: stable internal identities for library roots,
  full-hash and checksum columns on media files, and the tables behind location
  operations, their per-title checkpoints, and their per-file verification
  records.
- **Library roots now keep a stable identity that does not depend on their path.**
  Existing roots are migrated in place and every reference to them is remapped in
  one transaction; the previous identity is retained for diagnostics. Nothing in
  your configuration needs to be re-entered.
- Existing media files have no stored full-file hash until the backfill job
  reaches them or a move or import rewrites them. Until then, deduplication
  simply does not fire for those files — Scryer will not delete anything on
  weaker evidence.
- The verification-depth preference applies to **download-client import copies
  only** and defaults to **full**. If completed downloads land on a slow network
  mount and a full read-back after every import is too expensive, switch it to
  quick check. Library and root moves ignore the preference and always verify
  fully. The preference lives in **Settings → General**, under *Import copy
  verification*, and is also available through the API
  (`verificationSettings` / `updateVerificationSettings`).

## API and integration compatibility

New GraphQL surface: `locationOperationPreview`, `startLocationOperation`,
`cancelLocationOperation`, `resumeLocationOperation`, and a `locationOperation`
query for Activity, with `locationOperationAssets` for the per-file listing of
a finished operation; `locationRootScopePreview` for **Change root**, whose
one input takes either a `destinationPath` or a `destinationRootId` and whose
one payload covers both destinations (it confirms through
`startLocationOperation`, whose input gained a `rootScope` target of the same
shape — `titleIds` and `destination` are now nullable, which is
backwards-compatible for existing callers); a refused
root-scoped request carries `extensions.code = "LOCATION_ROOT_REFUSED"` with a
machine-readable `extensions.refusalCode`; `changeTitleFolderPreview` and
`applyTitleFolderChange` (including the swap and take-over variants);
`verificationSettings` and `updateVerificationSettings`. Regenerate typed
clients against the included schema.

**Client migration note — `TitleOptionsInput.rootFolderId`:**

> **API change — `TitleOptionsInput.rootFolderId` is deprecated for existing
> titles.** Writing `rootFolderId` through `addTitle` (title creation) is
> unchanged, and titles with no tracked files on disk may still be reassigned
> directly — that stays a catalog-only pointer change. For a title that already
> has tracked files, `updateTitle` no longer rewrites the root in place: the
> mutation fails with `extensions.code = "DIRECT_ROOT_WRITE_RETIRED"` (and
> `extensions.titleId` naming the title), because relocating content on disk is
> now the move workflow's job. Migrate such calls to `locationOperationPreview`
> followed by `startLocationOperation`. The input field remains in the schema,
> marked `@deprecated`, so it is hidden from default introspection — pass
> `includeDeprecated: true` if you introspect it.
>
> This also applies when `addTitle` reuses a title that already exists: re-adding
> a title with tracked files under a different `rootFolderId` is refused with the
> same `DIRECT_ROOT_WRITE_RETIRED` code. Creating a genuinely new title with a
> `rootFolderId`, and re-adding a title on the root it already sits on, are
> unaffected.

