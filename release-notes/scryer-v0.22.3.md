# Scryer 0.22.3 release notes

These changes are relative to v0.22.2.

## Highlights

- Rank releases by preferred and optional audio languages, set per facet, per library or per title, and pick a title's audio languages when adding, requesting, approving or routing it from a list.
- Each quality profile has a **Decision order**: the steps that decide between two releases before the score does. Reorder or remove steps by dragging or with buttons.
- Download a sanitized diagnostics bundle from the server logs page.
- Choose each library's default monitoring for new titles.
- Request-only users can search for titles and see which of their libraries already hold a result.
- Dismiss an import warning while a torrent keeps seeding, and resume it later.
- Large libraries scan, import, rename, move and delete with far fewer database round trips, and idle background work no longer writes to the database.

## Upgrade notes

- Every title folder is now written through one ownership check. A title cannot take a folder another title holds, a folder nested inside another title's or holding one, or a library root. On first start a one-time repair clears the recorded folder of each title that breaks these rules, keeping the folder for the title with the most primary files in it, and records the reason on the cleared title. No files on disk and no file records change.
- Folder comparisons ignore case for Windows-shaped paths and respect case for POSIX paths.
- Deleting a library now runs as a background job. The library stays listed until the job finishes.
- A download Scryer did not submit that scores at or below the file already in the library is now held for manual import instead of being skipped as already imported.
- A multi-audio release that names no language now counts as carrying the title's original language and English. A release that names its dub is taken at its word, and an indexer's audio track list still outranks the marker.
- Scheduled maintenance jobs, such as housekeeping, health checks, list sync and Prowlarr sync, keep running when background acquisition is turned off.

### API changes

- `DeleteLibraryPayload` returns `jobRun` for the background delete. Its `recyclePurged`, `recyclePreserved`, `recycleBusy` and `recycleFailed` fields are removed.
- Quality profile criteria carry `decisionPipeline`, a list of `QualityDecisionGateValue` (`QUALITY_TIER`, `REVISION`, `PREFERRED_LANGUAGE`, `OPTIONAL_LANGUAGES`). A repeated step is refused with `INTERFACE_QUALITY_PROFILE_DECISION_PIPELINE_DUPLICATE_GATE`. `ignoreProperRepack` remains readable and accepted and adds or removes the `REVISION` step. A profile that sets no order uses `QUALITY_TIER`, `REVISION`.
- Quality profile decisions, recorded release decisions and grab history carry a nullable `decidedBy` naming the step that decided.
- A null `cutoffScore` on save now clears the stored cutoff score.
- Media settings and library settings carry preferred audio languages, optional audio languages and whether missing optional languages keep a title wanted, with override and effective values on libraries. Titles, requests, approvals and list routes carry required and preferred audio language overrides. An unknown language code is refused and the refusal names the code.
- `translateArrCustomFormats` translates pasted Sonarr and Radarr custom formats to a Rego rule on the server.
- Search results gain `presentInLibraryIds`, the caller's libraries of that facet that already hold the result.
- `dismissImportWarning` and `resumeImportTracking` dismiss and restore an import warning. They accept the queue's download id, a history row's id or the client's own id, such as a torrent info hash.
- Pending imports can be ignored in bulk; a request naming more than 1,000 distinct ids is refused with `LIBRARY_PENDING_IMPORT_IGNORE_BATCH_TOO_LARGE`.
- `libraryGraph` accepts the caller's known view key and answers `unchanged` when it still matches.
- `GET /api/diagnostics/bundle` returns the diagnostics bundle. It needs a session or `ska_` API key with full session scope and the system settings permission, and refusals carry `HTTP_DIAGNOSTICS_*` reason codes.

## Quality profiles and audio languages

- Set an ordered list of preferred audio languages and a list of optional ones for each facet, override them per library, and override the preferred list per title. A title resolves its languages from the title, then the library, then the facet. Add, request, approval and list route dialogs offer required and preferred audio pickers that inherit by default and show the library's effective languages.
- Requests store the languages the requester asked for. Approvers can grant, clear or replace them, and request rules can read both.
- The quality profile **Decision order** lists the steps that compare two releases before the score: quality tier, PROPER/REPACK, preferred language and optional languages. A step left out has no effect anywhere, including admission, cutoff and upgrades. Profiles that never chose an order compare by quality tier, then PROPER/REPACK, as before; the language steps take part only once added. The PROPER/REPACK switch is replaced by the presence of that step.
- Drag steps and quality tiers to reorder them, with a placeholder showing where the row will land. Drag a row between the used list and the available list to add it at a chosen position or remove it. Every move is also available through the row buttons.
- In a profile that uses the language steps, a file still missing a preferred language, or every optional language when optional languages keep a title wanted, is never at cutoff, and only a release that improves the language passes the cutoff for it.
- Release search results, history events and the release decisions list show which step decided between two releases.
- Quality profiles gain an optional cutoff score next to the cutoff tier.
- A change to a title's audio language preferences reopens its search.

## Search and indexers

- Indexer search facets count the results left under every other active filter, and gain video codec, audio codec, audio channel and language groups. A release counts under the languages its indexer reported and those its title names. The two size sliders are replaced by one range slider, and the results count follows the filters.
- Global search keeps the gateway's result order instead of re-sorting by popularity, so the title typed is no longer pushed below a more popular near-match.
- A release whose name drops an apostrophe from the title, such as "Cartographers.Key" for "Cartographer's Key", now matches.
- Automatic grabs wait for an episode's air date: RSS and automatic search refuse a release until twelve hours before the episode airs. Manual grabs and episodes with no known air date are unaffected.
- Interactive search stops at its deadline instead of spinning when a request hangs, keeps partial results, and reports cancelled or missing sessions.
- Background paged indexer searches no longer restart from the first page when stored pages cannot be replayed.
- Library release search languages are saved again.

## Parsing

- MULTi and Multi-Audio markers are read as the original audio plus any dubs the name gives. A marker no longer fills in the original language when the indexer listed the audio tracks without it.
- A range written as a complete season ("01-12 of 12") is read as a season pack only in Sonarr's multi-title form; elsewhere the range keeps its listed episodes.
- Spaced and fused episode ranges, numeric titles next to ranges, Unicode hyphens and compact ranges followed by hyphenated metadata are parsed more reliably. A single episode followed by a spaced bare number is no longer reported as a confident parse.

## Imports, downloads and activity

- Dismiss an import warning while a torrent keeps seeding, and resume tracking later with the original block restored.
- A completed download whose import the title folder check refuses waits in Import with a title folder conflict reason instead of retrying forever.
- A manual import of part of a seeding pack finishes even when the automatic pass rejected some of the pack's files first.
- Skipped and failed imports leave Activity > Import and show in history as completed or failed. **Ignore** works on rows whose download has already finished.
- Imports honor search aliases when matching pack members and title names.
- Moves, location copies and imports finish on filesystems that cannot sync, such as some macOS SMB mounts; the copy is still verified before the source is removed.
- The pending imports list pages in the database, and ignoring a selection sends one request per 500 items.
- The pending import badge updates as soon as a library or root changes.
- Finished location operations keep their file list in the Activity detail.

## Libraries, folders and deletion

- Choose each library's default monitoring for new titles beside its quality settings.
- Scans and folder recovery check folder ownership before creating a title or copying files, and root moves, conflict resolution, rename and scan-created movies check it before moving any file.
- A person can delete a title's individual files even when its folder overlaps another title's; only that file, its sidecar and its subtitles are removed.
- Bulk edit and bulk delete work across libraries. Bulk move is offered only when every selected title is in one library.
- Deleting titles and libraries runs in chunks, and recycle cleanup moves to housekeeping.

## Rules

- Sonarr and Radarr custom formats translate on the server.
- Translated custom formats with a negative lookahead or lookbehind now match.
- Regex nesting depth in pasted custom formats is bounded.

## Media servers

- Jellyfin and Emby link forms offer the connection's accounts to users who manage media-server connections.
- A new media-server link starts a watch-history sweep at once.

## System and diagnostics

- **Download diagnostics bundle** on the server logs page builds a compressed archive of a settings summary, environment facts, recent logs and metrics. URLs, credentials, emails, paths, host names, IP addresses and instance identifiers are replaced or removed before download.

## Performance

- Scans, imports, renames, moves, deletes and housekeeping batch their database reads and writes, and per-item loops are gone from these paths.
- Idle background passes and housekeeping ticks no longer write to the database, and retention deletes run in bounded batches.
- Library scans read each folder once and hold folder ownership in memory for the length of the scan.
- The calendar filters episodes and movies in the database, and stops refetching the same range, which reset its scroll position.
- Discovery's affinity read takes a fixed number of statements at any library size.
- SQLite uses a larger page cache, and new indexes cover the queries these paths run.
- Web polling pauses in hidden tabs.

## Interface

- The library graph uses the packed neighbourhood overview for libraries of 500 titles or more, so typical anime and series libraries no longer collapse into one overlapping pile.
- With episodes selected, the Seasons and Episodes header docks to the top while scrolling and follows the active palette.
- Browser back and forward move between folders in the folder browser.
- Hover cards have a visible shadow on dark backgrounds.
- Changing a library's quality profile and saving acquisition settings no longer fail with a validation error.
- Built-in job toasts are translated.
