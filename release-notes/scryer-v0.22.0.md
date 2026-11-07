# Scryer 0.22.0 release notes

These changes are relative to v0.21.24.

## Highlights

- Browse all matching Discovery candidates by genre or theme, open shelves as paginated grids, and explore related titles through the library graph.
- Queue saved Wanted standby releases directly, with options to keep or cancel conflicting downloads.
- Search for missing subtitles across selected episodes in one background batch using your configured search and synchronization settings.
- Move titles between libraries and resolve conflicting folders with previews and tracked background operations. Move is now a bulk selection action. File moves are marked **Beta**.
- Import selected indexers and download clients in background jobs with individual retry and skip controls.
- Read Plex and Emby watch history, show media-server sync health, and make watch-based maintenance decisions more conservative when history is incomplete.
- Set the same-tier upgrade threshold per quality profile, and choose how much time the background acquisition walk takes.
- Choose Scryer's listening address from setup or **Security & Access**.

## Upgrade notes

- The library graph requires the experimental features switch.
- File moves and folder-conflict resolution are marked **Beta**. Review the preview, including any recycling or permanent deletion, before applying a resolution.
- **Copy verification** now applies to library and root moves as well as download-client imports, and moves no longer always verify in full. Quick is the default; choose **Full** before moving media off a disk you plan to erase.
- Automatic imports now hold a file for review instead of importing it when Scryer cannot read its media, or when the media is malformed or encrypted. An upgrade is held when the new file's video codec or runtime cannot be verified, so an existing library file is never replaced by unverified media. Disc companion files and playlists that point at local files are held as well. A manual import still imports a file whose media could not be read, but encrypted or malformed media stays held.
- Library scans and imports now recognize the full set of video extensions Sonarr accepts, including DivX, XviD, RealMedia, WTV, 3GP, ASF, QuickTime and disc images, so a rescan can pick up files that were previously ignored.
- Removing a root from a library now removes the titles under it from the catalog. Files on disk and the recycle bin are not touched. The interface asks for confirmation and shows how many titles will be removed; the removal is refused while any of those titles is part of a running location operation or deletion.
- The minimum score a same-tier release must beat the current file by is now part of each quality profile (default 120). If you had changed the old instance-wide acquisition setting, upgrading copies your value into profiles that have none of their own; instances on the default are unchanged and nothing is re-searched.
- Library users with only **View** access no longer receive full file paths, acquisition details, Wanted items, or download and import history. These surfaces require the appropriate management permissions. Subtitle managers retain their subtitle-specific actions.
- `SCRYER_BIND` continues to override the saved listening address. Changes made through the network settings require a restart; the interface shows the effective address and any pending change.
- The first start after upgrading builds catalog ordering keys for existing titles, so it can take longer on large libraries.

### API changes

- Library roots are set only through `updateLibrary`, by stored root id. `updateLibraryPaths`, `UpdateLibraryPathsInput`, `RootFolderInput`, and the `libraryPath` and `rootFolders` fields of `UpdateMediaSettingsInput` are removed. The read-side `libraryPaths` and `rootFolders` queries remain.
- `updateLibrary` requires every stored root to be sent back with its `id`. A root without an id is a new root; a root without an id at a stored root's path is refused with `LIBRARY_ROOT_ID_REQUIRED`. Changing a root's path under the same id keeps the root and its titles. `LibraryPayload.removedTitleCount` reports titles removed with dropped roots, and a busy title refuses the removal with `LIBRARY_ROOT_TITLES_BUSY`.
- The acquisition settings' `sameTierMinDelta` is deprecated: the field reports 0, the input is accepted but ignored, and both are removed in the next minor release. Set `minUpgradeScoreDelta` on the quality profile instead.
- `TitlePayload.rootFolderPath` is now nullable and returns null when the caller cannot manage the title's library.
- The synchronous `executeExternalImport` GraphQL mutation and `ExternalImportResultPayload` are removed. API clients must use `startExternalImportSources` and follow progress through `externalImportSourceStatus`.

## Discovery and library graph

- Open recommendation shelves as paginated section grids, sorted by relevance, popularity, rating, release date, or title, with remembered **Hide owned** and **Hide requested** filters. Add browse-by-genre and browse-by-theme cards, named "Because you like…" shelves, and a per-viewer **From Your Watchlist** shelf.
- Selecting a genre or theme in the sidebar opens a complete browse grid combining the locally stored personalized and public pools. Titles no longer need recommendation affinity, corroboration, popularity, or library media-mix eligibility to appear in explicit browsing. Recommendation shelves keep their existing eligibility rules.
- Deduplicate cross-pool titles before counting and pagination. Browse-card counts use the same eligibility rules as their grids, including labels with only one eligible match. Browsing does not launch new searches or fetch additional recommendations.
- Add an interactive library graph with title posters, named neighborhoods, highlighted connections, and a focused view explaining related titles. Follow connections, open titles with Ctrl-click, and close the focused panel with Escape. Long navigation trails are compacted.
- Store library graphs as compressed payloads and support up to 10,000 subjects per media facet. Load large graphs progressively and keep zooming smooth in large libraries. Recover graph reads interrupted by replacement and retry syncs whose completed generation is missing its graph.
- Speed up Discovery on large catalogs by reading only the candidate pool and reusing the library profile until its inputs change. During bulk changes such as a library move, the profile is rebuilt at most once every five minutes. Discovery refreshes after catalog changes and ignores responses for a scope you have left.

## Wanted, acquisition, and download clients

- Add **Download** to saved standby candidates. Queue the saved release using configured routing as an operator override, without reopening the grab dialog or running another search.
- When the title already has in-flight downloads, show the affected downloads and offer **Queue** alongside them or **Cancel and queue**. The saved release is submitted first, and conflicting downloads are cancelled only once the download client accepts it, so a refused submission leaves the existing downloads in place. Cancellation failures are reported. Dismissing the dialog makes no changes.
- Add a **Search walk intensity** setting (quiet, normal, or eager; default normal) that sets the share of time the background walk spends searching, along with how many titles and indexer pages it reads at once. Recently active titles are walked first, and the rest of the library always receives a share. Very large libraries walk as quiet on the normal setting.
- Leave indexers whose plugin is not loaded, or whose proxy is missing or disabled, out of routing so they no longer keep their scopes in every walk.
- Score and prune search results without blocking other work.
- Surface unavailable download-client plugins, indexers that cannot be initialized, and releases dropped by grab limits. Interactive search reports how many releases it hid because no enabled download client accepts their protocol, or because that client's plugin is not loaded. Show download-client notices in movie and season interactive search, and retain client status on password-retry queue rows.
- Report Weaver, SABnzbd, and NZBGet failures by cause, such as a rejected API key or an unreachable client, instead of passing through raw client text.
- Show indexer routing as cards.
- Treat `-1` and other archive-protection flags reported by indexers as markers rather than passwords, so download clients use the real password from the NZB.
- Remove imported torrents from hash-based clients such as qBittorrent even when the client's torrent name differs from the release title.
- Stop sending Scryer's reserved `scryer:` title tags to download clients as client tags.
- Allow an explicit zero ratio in seeding profiles, and improve searches using untagged and romanized aliases.

## Bulk subtitle search

- Add **Search for subtitles** to selected-episode actions for series and anime. Submit up to 1,000 episodes as one tracked background job and return immediately with confirmation.
- Apply configured languages, forced and hearing-impaired preferences, score thresholds, translation filters, provider settings, blocklists, and synchronization policy. The batch starts immediately; it does not wait for or reset the periodic search interval.
- Allow explicitly selected unmonitored episodes and searches when automatic search on import is disabled. The global subtitle-enabled setting still applies.
- Process shared multi-episode files once, skip missing media and already-satisfied requirements, and coordinate with periodic and on-import searches to avoid duplicate automatic downloads.
- Let subtitle managers select episodes without gaining deletion permission. Report progress, partial failures, provider deferrals, and synchronization warnings, then refresh subtitle details when the batch finishes.

## Library moves, imports, and media files

- Support library moves and folder-conflict resolution through previewed, tracked location operations. Choose **Adopt all** to consolidate media or **Keep one** to retain the best media and recycle inferior files. Remaining files are preserved by default; permanent deletions require explicit confirmation when no recycle bin is configured.
- Add **Move** to the bulk selection panel and bar; it opens the move wizard to pick another root or library. Bulk conflict actions apply to the conflicting titles in a selection.
- Refuse conflict folders that are or contain a library root, or that hold other works. Refuse **Keep one** for media split into parts. Resolve folders under nested roots to the most specific root, and compare other owners case-insensitively.
- Make large root moves faster: plan reads are batched, previews sample large groups, and a confirmed move reuses its preview when nothing has changed. Transfers interrupted by a killed process resume as pending, a failed copy cleans up only its own partial files, and the transfer table sorts active, queued, then finished.
- Strengthen folder ownership, changed-file, hardlink, and native-path handling. Report blocked renames and preserve additional movie files and recycle payloads during conflict handling. Reuse scan evidence only when filesystem change stamps are reliable.
- Run selected source imports as durable background jobs. Show per-source progress and failure details, reconnect to progress, and retry failed or interrupted sources with corrected credentials or explicitly skip them. One source missing credentials no longer blocks its siblings, and NZBGet sources with authentication disabled are accepted.
- Select every pending import for the current filters, not just the visible page, and ignore a selection in bulk after confirmation. Imports held because no episode matched link to **Manual Import**.
- Use a shared media-format registry across scanning, imports, and the web, with native probing for Ogg/OGM and RealMedia and recognition of video files whose names are not valid UTF-8. Only standalone video files are import candidates or are removed during payload cleanup; probe-only formats and disc companions are not.
- Files on mount backends such as nzbdav, decypharr, and altmount that cannot be probed keep the quality from their release name when imported with an override.
- Preserve primary audio metadata, re-analyze stored audio summaries, allow partially analyzed MKV inventories to upgrade, and report unreadable media runtimes and extensionless files.
- Improve seeding-pack episode matching, including explicit manual mappings, and derive Dolby Vision base-layer naming from analyzed files.
- Skip scheduled library jobs when their only storage root is unavailable.

## Watch history, maintenance, and recycle bin

- Add Plex and Emby watch-history reads alongside Jellyfin and retain Jellyfin/Emby favorites. Show the last complete watch-history sync and the reason for an incomplete read in media-server settings.
- Treat incomplete or unreadable watch history as unknown rather than unwatched. Recheck relevant playback before watch-based maintenance deletion, including episode-linked files.
- Hold maintenance deletions while something is playing on a media server, when a server cannot be asked, or when the server shows a newer or uncounted play than the stored watch history. The hold reason is shown on the action.
- Add audience watch facts so maintenance rules can tell whether any requester, every requester, or anyone with an account on a connected media server has watched a title. Expose the facts a rule read in rule preview for authorized managers.
- Add an optional pending-deletion dashboard panel. Link recycle-bin items to expanded acquisition history and improve disclosures on narrow screens.

## Plugins and notifications

- Send provider artwork URLs in notifications instead of Scryer-local cache paths, fixing Discord import and upgrade notifications for titles with cached artwork.
- Pass error response bodies through to command plugins, so failures such as Discord's rejected-field errors are reported in full.
- Compile plugins in parallel at startup and install, and load runtime plugins once at startup with consistent loading rules during reloads.

## Network, permissions, and interface

- Choose **Only this computer** or access from other computers during setup and in **Security & Access**, with an advanced listening-address option. Report unusable configured addresses and fall back to loopback on the same port.
- Group listening address, public URL, and trusted proxies under Security, with **Restart now** to apply a changed address. In containers, the published ports decide reach and the choice is not offered. The desktop app no longer pins the address, so it can be changed there too.
- Move external-account invites to Users and require the setup code when Scryer is reachable beyond loopback or through a trusted proxy, unless it is set explicitly.
- Redact privileged acquisition fields for view-only users.
- Badge **Activity** in the navigation with the number of jobs running for 30 seconds or more, and simplify the location operation panel and move toast.
- Improve phone layouts for title headers, season actions, settings, search with the keyboard open, notices, and toasts. Keep season actions inline in the desktop side panel. Keep catalog counts aligned with active filters and make episode availability clearer.
- Restore rule-editor error underlines, separate inline scripts from script-file selection, and support copy buttons when the browser's asynchronous clipboard API is unavailable.
- Translate standby queue actions, bulk subtitle search, and other new 0.22 features across the supported locales, including Russian.
