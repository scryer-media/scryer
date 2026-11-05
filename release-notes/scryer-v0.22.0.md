# Scryer 0.22.0 release notes

These changes are relative to v0.21.23.

## Highlights

- Browse all matching Discovery candidates by genre or theme, open shelves as paginated grids, and explore related titles through the library graph.
- Queue saved Wanted standby releases directly, with options to keep or cancel conflicting downloads.
- Search for missing subtitles across selected episodes in one background batch using your configured search and synchronization settings.
- Move titles between libraries and resolve conflicting folders with previews and tracked background operations. File moves are marked **Beta**.
- Import selected indexers and download clients in background jobs with individual retry and skip controls.
- Read Plex and Emby watch history, show media-server sync health, and make watch-based maintenance decisions more conservative when history is incomplete.
- Choose Scryer's listening address from setup or **Security & Access**, and support Prowlarr 3.x.

## Upgrade notes

- The library graph requires the experimental features switch.
- File moves and folder-conflict resolution are marked **Beta**. Review the preview, including any recycling or permanent deletion, before applying a resolution.
- Library users with only **View** access no longer receive full file paths, acquisition details, Wanted items, or download and import history. These surfaces require the appropriate management permissions. Subtitle managers retain their subtitle-specific actions.
- `SCRYER_BIND` continues to override the saved listening address. Changes made through the network settings require a restart; the interface shows the effective address and any pending change.
- The synchronous `executeExternalImport` GraphQL mutation and `ExternalImportResultPayload` are removed. API clients must use `startExternalImportSources` and follow progress through `externalImportSourceStatus`.

## Discovery and library graph

- Open recommendation shelves as paginated section grids, with existing sort and filter controls. Add browse-by-genre and browse-by-theme cards and a per-viewer **From Your Watchlist** shelf.
- Selecting a genre or theme in the sidebar opens a complete browse grid combining the locally stored personalized and public pools. Titles no longer need recommendation affinity, corroboration, popularity, or library media-mix eligibility to appear in explicit browsing. Recommendation shelves keep their existing eligibility rules.
- Deduplicate cross-pool titles before counting and pagination. Browse-card counts use the same eligibility rules as their grids, including labels with only one eligible match. Browsing does not launch new searches or fetch additional recommendations.
- Add an interactive library graph with title posters, named neighborhoods, highlighted connections, and a focused view explaining related titles. Follow connections, open titles with Ctrl-click, and close the focused panel with Escape.
- Store library graphs as compressed payloads and support up to 10,000 subjects per media facet. Recover graph reads interrupted by replacement and retry syncs whose completed generation is missing its graph.

## Wanted, acquisition, and download clients

- Add **Download** to saved standby candidates. Queue the saved release using configured routing as an operator override, without reopening the grab dialog or running another search.
- When the title already has in-flight downloads, show the affected downloads and offer **Queue** alongside them or **Cancel and queue**. Cancellation is best effort: the replacement still queues if a cancellation fails, and the interface reports the failures. Dismissing the dialog makes no changes.
- Add acquisition-search cancellation from Activity for users with the required system-management permission.
- Surface unavailable download-client plugins, indexers that cannot be initialized, and releases dropped by grab limits. Show download-client notices in movie and season interactive search, and retain client status on password-retry queue rows.
- Support Prowlarr 2.x and 3.x. Newer major versions attempt the current API generation with a compatibility warning; versions older than 2.0.0 are refused.
- Allow an explicit zero ratio in seeding profiles. Improve searches using untagged and romanized aliases, and preserve release groups after dot-separated tags.

## Bulk subtitle search

- Add **Search for subtitles** to selected-episode actions for series and anime. Submit up to 1,000 episodes as one tracked background job and return immediately with confirmation.
- Apply configured languages, forced and hearing-impaired preferences, score thresholds, translation filters, provider settings, blocklists, and synchronization policy. The batch starts immediately; it does not wait for or reset the periodic search interval.
- Allow explicitly selected unmonitored episodes and searches when automatic search on import is disabled. The global subtitle-enabled setting still applies.
- Process shared multi-episode files once, skip missing media and already-satisfied requirements, and coordinate with periodic and on-import searches to avoid duplicate automatic downloads.
- Let subtitle managers select episodes without gaining deletion permission. Report progress, partial failures, provider deferrals, and synchronization warnings, then refresh subtitle details when the batch finishes.

## Library moves, imports, and media files

- Support library moves and folder-conflict resolution through previewed, tracked location operations. Choose **Adopt all** to consolidate media or **Keep one** to retain the best media and recycle inferior files. Remaining files are preserved by default; permanent deletions require explicit confirmation when no recycle bin is configured.
- Strengthen folder ownership, changed-file, hardlink, and native-path handling. Report blocked renames and preserve additional movie files and recycle payloads during conflict handling.
- Run selected source imports as durable background jobs. Show per-source progress and failure details, reconnect to progress, and retry failed or interrupted sources with corrected credentials or explicitly skip them. One source missing credentials no longer blocks its siblings.
- Use a shared media-format registry across scanning, imports, and the web, including OGM support. Keep probe-only formats distinct from standalone import candidates and cleanup targets.
- Preserve primary audio metadata, re-analyze stored audio summaries, allow partially analyzed MKV inventories to upgrade, and report unreadable media runtimes and extensionless files.
- Improve numbered and seeding-pack episode matching, including explicit manual mappings. Add rename tokens for dynamic range, release revision, and 3D, and derive Dolby Vision base-layer naming from analyzed files.
- Skip scheduled library jobs when their only storage root is unavailable.

## Watch history, maintenance, and recycle bin

- Add Plex and Emby watch-history reads alongside Jellyfin and retain Jellyfin/Emby favorites. Show the last complete watch-history sync and the reason for an incomplete read in media-server settings.
- Treat incomplete or unreadable watch history as unknown rather than unwatched. Recheck relevant playback before watch-based maintenance deletion, including episode-linked files.
- Add audience watch facts so maintenance rules can distinguish whether anyone or all requesters have watched a title. Expose the fact document in rule preview for authorized managers.
- Add an optional pending-deletion dashboard panel. Link recycle-bin items to expanded acquisition history and improve disclosures on narrow screens.

## Network, permissions, and interface

- Choose **Only this computer** or access from other computers during setup and in **Security & Access**, with an advanced listening-address option. Report unusable configured addresses and fall back to loopback on the same port.
- Group listening address, public URL, and trusted proxies under Security. Move external-account invites to Users and require setup verification when exposed beyond loopback or through a trusted proxy unless explicitly configured otherwise.
- Restrict root-folder listings to visible libraries and redact privileged acquisition fields for view-only users. Fix interactive search initialization for restricted title managers.
- Improve phone layouts for title headers, season actions, settings, search with the keyboard open, notices, and toasts. Keep catalog counts aligned with active filters and make episode availability clearer.
- Restore rule-editor error underlines, separate inline scripts from script-file selection, and support copy buttons when the browser's asynchronous clipboard API is unavailable.
- Expand Russian localization and translate standby queue actions across the supported locales. Load runtime plugins once at startup and apply consistent loading rules during reloads.
