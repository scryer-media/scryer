# Scryer 0.23.0 release notes

These changes are relative to v0.22.5 and cover `scryer-v0.22.5..41df4aa8e`.

## Highlights

- Maintenance rules are now **lifecycle rules**. A rule is one Rego source whose heads say what it does: unmonitor, delete files, delete the title, search, change the quality profile, add or remove tags, move to another library or root, fetch subtitles or notify, at the title, season or episode level. Each rule runs on its own schedule and can read watchlists, play counts, Jellyfin and Emby favorites, title details, series shape and free space.
- **Request rules shape a request** as well as approve or deny it: they can set the quality profile, lease, monitoring, audio languages and tags, and send a notification.
- **One Simple builder** writes scoring, lifecycle and request rules from dropdowns, and Advanced shows the Rego it wrote. The rules pages are out of the experimental switch and marked beta.
- **Guided tours**: new accounts get an onboarding tour of the interface, and each release can show a tour of what changed. Tours are recorded per account.
- **Monthly indexer limits**: an indexer can carry a monthly request limit and a monthly grab limit. An indexer at its limit is passed over without counting as a failure.
- **Safer file handling**: one disk service now carries out every filesystem operation. Moves, imports, copies and the recycle bin never replace a different file, never remove the only copy, and take back only what they placed. A dead mount no longer ties up threads.
- **Archives are extracted once per download**, into a workspace of their own, instead of again on every import retry.
- **Pack thresholds per quality profile** replace the fixed rule for when a season or multi-season pack may be taken.
- **Import rules from Sonarr, Radarr and Maintainerr**: connect to a Sonarr or Radarr instance to bring its custom formats in as scoring rules, or to Maintainerr to bring its rule groups in as lifecycle rules. Every imported rule is saved disabled, with a report of what translated exactly and what needs review.
- **Content ratings and awards** are stored per title. The overview shows a certification badge, and request and lifecycle rules can read both.
- **Per-library full hashing**: a library can keep the nightly full-hash job off its disks.
- **Security hardening**: sessions are signed with a key derived from the instance's master key, open admin mode and Skip for local IPs can be narrowed to listed host names and trusted proxies, and replies from every outside service are read within a size limit.
- **Dolby Vision is detected** in real Matroska and MP4 files.
- **Grabs and searches run in the background**: a grab button no longer holds a request open, and episode, series-movie and cutoff searches wait in line instead of running twice.
- **Built-in MCP endpoint**: Scryer serves `/mcp` so an MCP client can run the operator skills' operations with an API key. A mutation with lasting effects runs only after the client confirms it.

## Upgrade notes

- **Every lifecycle (maintenance) rule you created is removed on upgrade**, with its revisions, library scopes, exclusions, evaluation runs, candidates and action history. Rules written before rules declared their own actions cannot be read in the new form. Only database rows are removed; no file on disk is touched. The built-in expired request leases rule is kept, and its source is updated on first start.
- **Every request rule set is removed on upgrade**, with its revisions and library scopes. Requests, request decisions and their history, leases, titles and tags stay. A stored decision keeps the name of the rule set that took it.
- Lifecycle instance gate settings and the evaluation job's run history are kept under their new names.
- Request rule sets have one state: enabled or disabled. The evaluation mode is gone, and every decision is enforced.
- A lifecycle rule that deletes files or titles must carry a maximum number of titles per run. The built-in per-run limits are gone. Rules that do not delete may leave the maximum empty for no limit.
- New lifecycle rules run every 8 hours by default.
- The pack thresholds default to the values Scryer used before: a pack needs at least 2 missing episodes, and a multi-season pack needs more than 30% of its monitored, aired episodes missing. Existing profiles read these defaults until they are changed.
- **Every user signs in once more after the upgrade.** Sessions are now signed with a key derived from the instance's encryption master key, so sessions issued by 0.22 are no longer accepted. An instance that sets `SCRYER_JWT_SIGNING_SECRET` keeps its sessions. API keys are unaffected.
- **Lifecycle rules need Manage system settings**, where they needed Manage catalog settings before. This covers reading, editing, previewing, running and importing them. Members with only catalog-settings management no longer see the Lifecycle page. Scheduled runs are unchanged.
- First start queues a metadata refresh for existing titles so their content ratings and awards are stored. Until a title's refresh lands, rules read its ratings and awards as unknown, so a lifecycle rule that needs them holds that title.
- A quality profile with no cutoff now searches for upgrades up to its top tier. Earlier, searches never looked for upgrades for such a profile.
- A download whose import passes keep ending the same way without importing anything new stops after six such passes and waits for a retry.
- The background full-hash job stays on for every library. A library where it is turned off asks for confirmation first.
- The MCP endpoint is on by default. It can be turned off under Settings > Security & Access, and `SCRYER_MCP_ENABLED` forces it on or off while set.
- The two local trust limits are off by default and change nothing until set.
- Media analysis revision 3 analyzes again the files that were analyzed before Dolby Vision records were read.
- Extraction workspaces left inside title folders by 0.22 are noted by the next library scan and removed by housekeeping once their download has settled for a day.
- Working files and folders take short `.scry-` names. Names written by earlier releases are still recognised.
- Database indexes that no query used are dropped, and the indexes that common lookups were missing are added.
- Guided tour progress and monthly indexer usage are kept in backups.

## Lifecycle rules

- A rule's source is the whole rule. Its heads declare its actions, and each head names the level it acts at: title heads such as `unmonitor`, `delete_files`, `delete_title`, `search`, `set_quality_profile`, `add_tags`, `remove_tags`, `move_to_library`, `move_to_root`, `fetch_subtitles` and `notify`, with season and episode forms such as `unmonitor_season`, `delete_season_files`, `unmonitor_episode` and `delete_episode_files`.
- A `days_since` helper is available to rules.
- Each rule has its own schedule: manual, every interval, daily, weekly or a cron expression. The rules list shows when each rule runs next. Cron schedules are evaluated by Scryer itself.
- Rules can read:
  - members' watchlists
  - title details, genres and ratings
  - file streams and total plays
  - the shape of a series
  - Jellyfin and Emby favorites, including series and season favorites and unplayed favorites, read for every media-server account. Plex favorites read as unknown.
  - storage facts for every library root, with free space sampled by the storage capacity module
  - when a title was first requested (`requested_at`), and who watched it, by name (`watched_by_usernames`) and with each user's plays (`watched_by_users`)
  - content ratings (`age_rating`, `certifications`, `certification_label`, `certification_rank`) and awards (`award_count`, `awards`), in the same shapes request rules read
- A rule loads only the fact groups it names.
- A title whose ratings or awards are not stored yet reads them as unknown (`metadata_not_stored`), never as empty.
- A rule that deletes files to free space checks the free space again after each deletion, and stops once the space it freed satisfies it.
- A title deleted by a lifecycle rule is excluded from every public list that added it, so the list does not add it back.
- A rule holds a title when a fact it needs cannot be known. `max_video_height` is unknown until every file of the title has been scanned. A new series or season favorite holds an action that would remove it.
- Conflicting actions across a title, its seasons and its episodes hold each other, and the Simple builder refuses to save them.
- Act now runs one selected rule and evaluates it first. Only a rule's own run acts on its candidates.
- A held title waits for the rule's next evaluation.
- Titles past a rule's maximum per run wait for the next run.
- A rule whose only action is a season or episode notification counts as notify-only, so its notification is sent.
- Validation reports the undefined tags, quality profiles and move targets that a save would refuse.
- Every hold and state reason shows translated text.
- The rule builder no longer offers facts the server never collects.
- The Rules page lists Scoring, Lifecycle and Request Rules under the sidebar's Rules entry. Lifecycle's Rules, Leaving soon and History are page tabs. The command palette still finds the page when searched as "maintenance".
- The rule table sorts by any column and pages after 15 rules.
- Leaving soon and History lay out like Lists, with a filter bar, bordered tables and expandable candidate detail.
- Starter templates and the input reference open as slide-out panels.

## Request rules

- A request rule can approve, deny or send a request to manual review, and can shape it: quality profile, lease days, monitoring, preferred and required audio languages, tags and a notification.
- When rules disagree on a shaped value, the request goes to manual review.
- Rules can read series shape, release dates, other requesters of the same title, the requester's watchlist, list membership, account age and free space.
- Rules can read a title's `awards`: every award win, with `award`, `award_id`, `year`, `recipient`, `recipient_id` and `given_to`.
- The requester's watchlist is read only when a loaded rule can use it. Preview always reads it.
- A request whose rules cannot be consulted waits for review.
- A denial stands when the tag registry cannot be read.
- Preflight and the rule tester read the monitor type and selection the way a submitted request does, so a rule sees the same value in all three.
- Request dialogs put lease, tag and release help behind info icons.

## Rule builder

- One schema-driven Simple builder writes scoring, lifecycle and request rules. Its outputs come from the server, so the builder offers exactly what the engine accepts.
- Advanced shows the rule's Rego and can be edited. Before the first edit of a builder rule, a warning says the rule will no longer open in Simple.
- 21 of the 36 starter templates open in Simple.
- Conditions on users, quality profiles and lists pick the value by name.
- Facts are sorted, filterable and explained beside the highlighted row.
- An action's own conditions sit behind one button, apart from the rule's.
- A save held for unfinished choices points at them.
- The builder shows only failures, and help text sits behind info icons.
- Scoring and request rule libraries and references open as slide-out panels, and reference tables fit the panel.
- Rule input references match what each engine provides. `input.file.audio_streams[].name` is in the scoring reference, and the 4K x264 template matches the lower-case quality the parser emits.
- Rule refusals read as sentences in every area.

## Rule imports

- The scoring rules import dialog gains a Connect tab beside paste: give a Sonarr or Radarr URL and API key, and each custom format is saved as a disabled scoring rule set.
- Profiles that agree on a format's score give a literal score. Profiles that disagree score by Scryer quality profile name when every name matches. Otherwise the rule is saved for review, scoring 0, with the score table commented out.
- Lifecycle rules import Maintainerr rule groups from its URL, one disabled lifecycle rule per group. Properties, sections and AND/OR follow Maintainerr's meaning, and actions map onto lifecycle actions.
- A Maintainerr comparison that Scryer's facts could make match more titles than Maintainerr would fails its group instead of translating loosely. A group tied to one library warns that it now applies to every library.
- A group or format with no faithful translation is saved as a failed placeholder or not at all, and the report lists each finding.
- Imports run as background jobs. The report survives closing the dialog and switching tabs, and a run can be retried when its report fails to load.
- A rule whose name already exists is skipped. The connection details are stored encrypted for the run and removed after the fetch.
- Sonarr and Radarr imports need Manage catalog settings; Maintainerr imports need Manage system settings.

## Guided tours

- New accounts get an onboarding tour covering search, libraries, discovery and requests, lists, activity, indexers and download clients, quality profiles, rules, media servers, notifications and plugins.
- Each release can show a tour of its highlights.
- Steps that change page point at the navigation first, and each spotlight has a hand-drawn arrow.
- A finished or skipped tour is recorded for the account on the server.
- The onboarding tour opens right after the first-run wizard, without a reload.

## Indexers

- An indexer can carry a monthly request limit and a monthly grab limit.
- An indexer at its limit is passed over: the search moves on to other indexers, and the release is neither blocklisted nor recorded as failed.
- A grab is refused before it is counted when the request limit is reached.
- A replacement grab is checked against the limit before anything is cancelled to make room for it, so a refused replacement keeps the current download.
- A retry refused by the limit keeps the failed submission it would replace.
- An RSS release passed over only for the limit is seen again on the first poll after the limit lifts.
- An instance with no limits set reads nothing extra.
- A release missing from every indexer no longer holds every indexer out of a facet.

## Proxies

- A challenge solver can be reached through a transport proxy or tunnel.
- The solver's capability fetch, preflight and solved session stay on that route, and the route fails closed when it cannot be used.
- A route a solver depends on cannot be disabled or deleted.

## Searches and grabs

- Episode, series-movie and cutoff searches run in the background and wait in line. One item is never searched twice at once.
- Grab buttons run as background jobs instead of holding a request open.
- Grabs and searches always end their job run.
- A user follows the searches they started without needing to manage system settings.
- Slow requests share two connections, so the rest of the page keeps working.
- Busy and progress states clear without the event stream.
- Failure cleanup is offered only for downloads Scryer grabbed.
- Mismatch recovery queues every eligible scope of a title.
- Editing a custom search alias reopens the title's searches.
- A profile with no cutoff uses its top tier as the cutoff. Files below it are upgrade targets, and past it only same-tier releases that clear the minimum score delta replace them.
- A pack or upgrade that overlaps a download the client is still working waits until that download leaves the queue, instead of being refused by submission on every pass.
- A grabbed file that loses to the library copy blocks its release on every indexer, except when it lost to a file the same download placed. A pack is blocked for losing only when it imported nothing.
- A download holding only unwanted executables is blocked on every indexer. A password failure stays with its indexer.
- The search-time blocklist check looks up the candidates' names and hashes exactly, instead of reading the title's newest thousand rows.

## Quality profiles

- Each profile sets the minimum number of missing episodes for a pack (default 2) and the percentage of a multi-season pack's monitored, aired episodes that must be missing (default 30, strictly more than).
- At a minimum of 1, an automatic single-episode search may take a pack.
- The season-pack gate, the series-pack check and the parked series-pack check follow the profile's thresholds.
- Out-of-range values are refused with their own reasons.
- Changing a threshold does not reopen searches already covered.

## Imports and file safety

All filesystem work now goes through one disk service.

- Each library root has its own timeout, and reads on a dead mount no longer collect blocking threads.
- A volume that did not answer never overwrites an answer another read wrote meanwhile.
- A move or import:
  - never replaces a different file
  - never removes the only copy
  - takes back only the copy, link or hard link it placed, and keeps one that another writer replaced
  - never takes back a destination that has become the last copy of the source
  - keeps a copy whose source now holds another file
- A source set aside during an operation is put back, retried for thirty seconds, and set-aside leftovers are cleared.
- A failed placement removes the empty file it made to reserve a name, and only that file.
- A failed move or copy says where it left each file.
- An import whose destination is its source's own file completes on that file.
- A destination already in place is compared with the source at the verification depth you chose: Quick samples both files, Full compares whole-file hashes.
- Where file numbers cannot tell two names apart (two mounts, a share or a link), the source is set aside briefly to learn whether the destination is the same file.
- A download's file is never handed back to its client for removal while it may be the library file. When this cannot be told, the import is retried instead of removing anything.
- A download blocked for reconciliation is never removed from its client and shows a translated reason. The block and its reason survive a queue rebuild after a client refresh or a restart.
- A download's file is never renamed while its torrent may be seeding. Removing a title leaves its torrents in their client. Ignore leaves the client's entry alone.
- An overwrite restore replaces the name, never the file behind it.
- A link that cannot be proven, and could not be removed, is reported with where it was left. Links on FAT, exFAT, ReFS and shares without file numbers are proven by their content.
- Whole-file hashes are cached and stored with a mark of the file they were read from, so a file is hashed again only when its length or modification time changes.
- A move that fails while its storage is gone waits for the storage to return.
- A move refused for a full or denied destination, a missing source or an unproven link says so with its own reason.
- A failed title in a location move keeps its reason and facts, and the Activity panel explains it in its own words.
- A folder or file that names an external id binds an existing title only through that id, never by name. A movie conflict row offers Fix Match.
- An import whose source cleanup could not finish after its file was cataloged waits as a retry instead of being recorded as a failed import. A queued manual import does the same.
- A history retry whose source cleanup could not finish is never taken as imported.
- A repeat success for the same source no longer resets the retry backoff, so an unverifiable download stops retrying at the first-step delay.
- Identical bytes are never copied or replaced again on a retry: an additional file is recognised by its size and sampled content, every superseded library file is compared, and a library file that could not be compared is left in place.
- A source whose size could not be read, or a library listing that failed, is a retryable failure.
- A film linked to a series special imports when its download also holds a bonus clip or trailer.
- A series-movie import completes its scope row whatever its status.
- A cancelled import clears its staged file.
- NZBs with Latin-1 text, a byte order mark or comment before the root, or HTML entities in a subject are accepted. A subject with no quoted name takes its last dotted word, as the download client does.
- A download whose passes keep ending the same way stops after six such passes and waits for a retry. Transient outcomes never count, and the count survives a restart.
- An unparseable extra video beside a grabbed episode is recorded as ignored, so the download settles once its episodes land.
- A pass never replaces a library file the same download placed, and never adds another copy of it.
- A failed upgrade keeps the existing file instead of placing a second one beside it.
- Retry import accepts the same evidence the automatic import does.
- A film extracted from archives settles when a loose trailer or extra sits beside the archives.
- Manual import settles an identical linked film as already present.

## Archives

- A download's archives are extracted once, into a workspace under the library root, and later import passes read from it.
- The workspace lives until the download settles, and is reused only for the same archives.
- Extraction stops after three attempts. A failed or interrupted extraction's partial workspace is removed at once.
- A download whose archives have no installed extractor creates nothing and is reopened when an extractor plugin is installed.
- Leftover workspaces are swept, and a health check reports them.
- Manual imports and downloads without a client item id extract per pass, as before.
- Re-extraction is held to the same attempt limit.
- The leftover-folder sweep continues from where it stopped and wraps around, so folders that sort late are reached. A download's workspace is never swept by age.
- A settled workspace is removed only after its download is checked again under a removal hold, so a download that became live meanwhile keeps its workspace.
- When the free-space check for an extraction fails, the volume's last known free space is used.

## Libraries

- Each library has a setting, on by default, for the background full-hash job. Off, the job never reads that library's files, and a file's full hash is read only when something needs it.
- Turning it off asks for confirmation, since it reduces how far Scryer can prove some file moves safe.
- In a library with hashing off, a title folder whose source file is gone is recovered by finding the file in the destination by its name and catalog size, and checked again the same way before the recovery commits. The original stays in place; only the catalog row is re-pointed.
- A `tvshow.nfo` larger than 32 MiB is ignored.

## Recycle bin

- A recycle-bin copy, its restore and the relocation of bin entries are proven at your verification depth instead of always at Quick.
- An entry replaced by an upgrade is purged once any later library file of the same title covers what it held, even after a second upgrade or a rename. Earlier entries were kept forever.
- Each purge is decided on the library file as it is at that moment.
- A batch delete lists each bin once.

## Media analysis

- Dolby Vision is detected in Matroska, MP4 `dvwC` boxes and 4-byte configuration records. The earlier check never matched real files.
- Dolby Vision records are attached to their track.

## Titles and metadata

- Content ratings are stored per title, and the overview shows a certification badge that matches the `certification_label` request fact.
- Award wins are stored per title beside its content ratings.
- A rematch clears a title's stored content ratings and awards.
- Title merge clears orphaned monitor selections and manual-import candidates.
- Deleting a user clears the monitor selections of their requests.
- Library graph posters no longer pile onto each other in small libraries.

## Release parsing and matching

- Separate season and episode markers such as `S03.E05` are one episode.
- Zero-padded season ranges are multi-season packs.
- Repeated episode markers keep every episode.
- More daily date forms are read.
- Hyphenated `WEB-Rip` is read as WEBRip.
- Localized season words (Saison, Stagione, Temporada, Sezon) count as seasons only with their own number.
- `WIDTHxHEIGHT` is a resolution, never a season and episode.
- A spaced episode label with no season, such as `EP 04`, is one episode.
- A season-only scene file name no longer replaces the release's numbering.
- Dotted episode file names keep their episode.
- Bilingual torrent release names are kept.
- Latin spellings reach Cyrillic catalog names, and imports accept romanized names of the identified series.
- A number in a show's name, such as `The.140.Lanterns.S02E01`, stays in the guessed title when the release numbers its episode later. Such releases were refused as title mismatches by automatic search and RSS.

## Lists, plugins and accounts

- A member whose lists are not allowed no longer sees the Lists page, unless they manage lists.
- Restore warnings on the Plugins page can be dismissed.
- WebSockets whose sign-in is stale are closed. A sign-out heard by no connection is stored, so later connections are not closed at once.
- A user's live connections end when their permissions change or they are deleted.

## Security

- Sessions are signed with a salt derived from the instance's encryption master key, unless `SCRYER_JWT_SIGNING_SECRET` is set. Stored API key, OAuth token and indexer client key hashes still verify.
- Settings > Security & Access gains a Local network trust panel:
  - **Allowed host names**: when the list is not empty, a request served by open admin mode or Skip for local IPs must name an IP address, `localhost`, the public URL host or a listed name. Sign-ins and API keys are unaffected, and a save that would lock out the browser making it is refused.
  - **Trust forwarded addresses only from trusted proxies**: only a listed trusted proxy may vouch for a local client, and only when the whole forwarded chain is local. It cannot be on while no trusted proxy is listed.
- Skip for local IPs and the disable-login dialog warn that a reverse proxy that does not forward client addresses makes every visitor local.
- Replies are read within a size limit from indexers (caps), SAB-compatible download clients, media servers (with a page ceiling on catalog scans), plugin downloads, Sonarr and Radarr, and title images. Image dimensions are checked before decoding.
- A restore unpacks its payload entry by entry, and refuses too many entries, entries that are not files or folders, and a payload that would leave too little scratch space.
- An indexer or download client whose field definitions are unavailable has every stored text value treated as a secret.
- Aliased GraphQL searches are charged one search per field.
- `scryer init --compose` creates its files only when the name is free and never through a link, and the key file is owner-only from creation.
- A plugin keeps at most 64 HTTP dispatchers per host.
- The bundled SQLite and the plugin runtime (wasmtime 50.0.3) are updated past published vulnerabilities.
- Backups leave out the session signing secret when it is only the SMG registration secret.

## MCP endpoint

- Scryer serves a Model Context Protocol endpoint at `/mcp`: stateless JSON over HTTP, admitted by API key only, behind the same origin policy, setup gate, restore admission and rate limits as the GraphQL API.
- Its tools are `whoami`, `list_operations`, `describe_operation`, `run_query`, `run_mutation`, `read_diagnostics_section` and `search_service_logs`.
- The run tools execute only the operations shipped with the operator skills, as the key's account, with each operation's own permission checks.
- Each call is charged to the rate limit the GraphQL API gives the same operation. A result over 32 KiB is refused with its size.
- A mutation that changes the library or settings runs nothing on its first call. It returns a summary with secret values redacted and a single-use confirmation token, bound to the API key and the exact call and valid for five minutes. The same call with the token runs it. `describe_operation` marks these operations.
- The endpoint can be turned off under Settings > Security & Access, and `SCRYER_MCP_ENABLED` forces it on or off while set. The switch shows a forced value, locked.
- The Sonarr and Radarr import reads used by the migrate skill fit the result limit on large libraries.
- The diagnostics bundle's release decision tally is filled in, and it shows `SCRYER_MCP_ENABLED`.

## Web interface

- Acquisition is listed under Settings at `/settings/acquisition`. The old address redirects.
- A dropped connection while a page loads no longer leaves "Failed to fetch dynamically imported module" on screen. The page retries with back-off, waits while offline, and later pages load from the cache.
- Back to list and Escape return to the poster grid at the same scroll position, without reloading the list.
- The diagnostics bundle download says the bundle is sanitized on a best-effort basis: safe to share in a Discord ticket, but not recommended on GitHub issues or public Discord channels.

## Backups

- Postgres backup exports are ordered correctly.
- Title awards are kept in backups.

## Windows

- Moves on one volume rename instead of copying, using a rename that never replaces.
- The volume serial and file index are read from the file's own handle.
- Deduplicated, compressed and cloud-placeholder files can be read again; only links and junctions are refused.
- Removals leave the read-only flag alone unless asked.

## Performance

- Hydration writes for episodes, ids and recommendation cards hold the database writer briefly.
- A recorded search response is a handful of statements.
- A search's decisions are prepared off the async runtime, and a decision's explanation is serialized once.
- The wanted page reads its titles' scopes without decisions and loads the latest decision only for the rows shown. A title's wanted page is not counted.
- Wanted items and title diagnostics parse explanations only when a query reads them.
- Title diagnostics count every scope per status in one read.
- The failed-download check reads only the failed items' submissions and their titles' grabbed scopes.
- Backups page by key, and their file work and restore run off the runtime.
- Media file analyses are decoded once per read.
- Newest imports are read through a creation-time index.
- Passes that find nothing to do stop repeating reads, writes and hashing: hash backfill refuses a size mismatch before reading the file, a download's pending attempt is settled in place, pending-release and RSS passes ask existence questions, and library refresh writes only rows that changed.
- Each activity feed read is bounded to twenty source pages and resumes from the last event it scanned.
- A failed origin lookup keeps a download's state instead of treating it as unowned.

## Translations

- Czech gains the monthly indexer limit text.
- Lifecycle, request rule and watch-sync text is complete in English, Czech, Russian and Traditional Chinese (Taiwan and Hong Kong).
- Error reasons read as sentences in every area.
- The library hashing setting is translated in every locale. Local trust and MCP settings text is in English and Russian.

## API changes

This release contains breaking GraphQL changes.

**Maintenance is renamed to lifecycle.** Every type, enum, input, field and operation named `Maintenance…` is now named `Lifecycle…`, for example `maintenanceRuleSets` → `lifecycleRuleSets`, `createMaintenanceRuleSet` → `createLifecycleRuleSet` and `MaintenanceCandidate` → `LifecycleCandidate`. Reason codes that began with `MAINTENANCE_` begin with `LIFECYCLE_`. The domain event `MAINTENANCE_RULE_NOTIFIED` is `LIFECYCLE_RULE_NOTIFIED`, and the plugin SDK event is `LifecycleRuleNotified` (`lifecycle_rule_notified`). Instance gate setting keys moved from `maintenance.gate.*` to `lifecycle.gate.*`.

**Lifecycle rules**

- `updateMaintenanceRuleMatcher` is replaced by `updateLifecycleRuleSource`, which takes `UpdateLifecycleRuleSourceInput` (`id`, `regoSource`, `graceDays`, `maxTitlesPerRun`, `builderModel`).
- The action model is removed: `maintenanceActionDescriptors`, `maintenanceActionStepDescriptors`, `LifecycleActionDescriptor`, `LifecycleActionStepDescriptor`, `LifecycleActionInput`, `LifecycleActionSequenceInput`, `LifecycleActionStepInput`, `LifecycleActionStepParametersInput`, `LifecycleActionSpec`, `LifecycleActionKind` values, `LifecycleActionSubject`, `LifecycleEffectArming` and `LifecycleRiskClass`.
- `CreateLifecycleRuleSetInput` and `PreviewLifecycleRuleInput` lose `action`, `actionSequence`, `storageRootId` and `subjectKind`.
- `LifecycleRuleSet` loses `actionSpec`, `actionSequence`, `enabledArming` and `subjectKind`. `LifecycleRuleSetDetail` loses `actionSpec` and `actionSequence`. `LifecycleRuleRevision` loses `storageRootId`.
- `LifecycleCandidate` and `LifecyclePreviewTitle` lose `storageRootFileCount` and `storageRootTotalSizeBytes`.
- `matcherContentHash` is `sourceContentHash` on `LifecyclePreviewPayload` and `LifecycleRuleRevision`.
- `runLifecycleActionHandlerNow` takes `ruleSetId`. Omitting it is refused with `LIFECYCLE_ACTION_RUN_RULE_REQUIRED`.
- The job key `MAINTENANCE_ACTION_HANDLING` is removed.
- Added: `LifecycleActionHead` (`ADD_TAGS`, `DELETE_EPISODE_FILES`, `DELETE_FILES`, `DELETE_SEASON_FILES`, `DELETE_TITLE`, `FETCH_EPISODE_SUBTITLES`, `FETCH_SEASON_SUBTITLES`, `FETCH_SUBTITLES`, `MONITOR`, `MONITOR_EPISODE`, `MONITOR_SEASON`, `MOVE_TO_LIBRARY`, `MOVE_TO_ROOT`, `NOTIFY`, `NOTIFY_EPISODE`, `NOTIFY_SEASON`, `REMOVE_TAGS`, `SEARCH`, `SET_QUALITY_PROFILE`, `UNMONITOR`, `UNMONITOR_EPISODE`, `UNMONITOR_SEASON`); `LifecycleDeclaredActions` on `LifecycleRuleSet`, `LifecycleRuleSetDetail` and `LifecycleRuleValidationPayload`; `LifecycleDecidedActions` on `LifecycleCandidate` and `LifecyclePreviewTitle`; `LifecycleCandidate.actionsConflict`; `LifecyclePreviewTitle.errorCode`; `LifecycleRuleSet.schedule`, `nextRunAt`, `maxTitlesPerRun` and `loadRefusal`; `schedule` and `maxTitlesPerRun` on `CreateLifecycleRuleSetInput` and `UpdateLifecycleRuleMetadataInput`; `LifecycleRuleValidationPayload.warnings`; and `LifecycleActionStepKind` values `FETCH_SUBTITLES`, `MONITOR`, `MOVE_TO_LIBRARY` and `MOVE_TO_ROOT`.
- Lifecycle reason codes renamed to name their condition (0.22.5 name → 0.23.0 name):
  - `MAINTENANCE_RULE_FAILED_COMPILE` → `LIFECYCLE_RULE_COMPILE_FAILED`
  - `MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE` → `LIFECYCLE_FILE_COVERAGE_AMBIGUOUS`
  - `MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON` → `LIFECYCLE_CHILD_DELETION_SUBJECT_REQUIRED`
  - `MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE` → `LIFECYCLE_CANDIDATE_TITLE_MISMATCH`
  - `MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED` → `LIFECYCLE_FILE_TARGETS_CHANGED`
  - `MAINTENANCE_RULE_SHIPS_SCRYER` → `LIFECYCLE_SYSTEM_RULE_READ_ONLY`
  - `MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED` → `LIFECYCLE_SYSTEM_RULE_RENAME_REFUSED`
  - `MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED` → `LIFECYCLE_SYSTEM_RULE_DELETE_REFUSED`
  - `MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID` → `LIFECYCLE_RULE_DELETE_HAS_ACTION_HISTORY`
  - `MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE` → `LIFECYCLE_RULE_DELETE_HAS_ACTIVE_CANDIDATES`
  - `MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS` → `LIFECYCLE_PREVIEW_SUBJECT_COUNT_OUT_OF_RANGE`
  - `MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE` → `LIFECYCLE_PREVIEW_SUBJECT_OUTSIDE_LIBRARIES`
  - `MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED` → `LIFECYCLE_PREVIEW_TITLE_LIMIT_EXCEEDED`
  - `MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY` → `LIFECYCLE_MOVIE_HAS_NO_EPISODES`
  - `MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND` → `LIFECYCLE_SUBJECT_NOT_FOUND`
  - `MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN` → `LIFECYCLE_RULE_HAS_NO_EFFECT`
  - `MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP` → `LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISSING`
  - `MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE` → `LIFECYCLE_DELETE_FILES_UNMONITOR_STEP_MISMATCH`
  - `MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST` → `LIFECYCLE_QUALITY_PROFILE_NOT_FOUND`
  - `MAINTENANCE_RULE_CONFIGURATION_INVALID`, raised for a stale arm confirmation, → `LIFECYCLE_ARM_CONFIRMATION_STALE`
- Every lifecycle query and mutation an actor makes now requires system-settings management instead of catalog-settings management.
- Lifecycle rule facts that can read unknown with the reasons `metadata_not_stored`, `no_age_rating`, `no_certifications`, `no_us_certification` and `unrankable_certification`.
- Lifecycle reason codes removed with the action model: `MAINTENANCE_ACTION_CONFIGURATION_INVALID`, `MAINTENANCE_ACTION_INVALID`, `MAINTENANCE_ACTION_NOT_STORABLE`, `MAINTENANCE_ACTION_UNSUPPORTED`, `MAINTENANCE_MATCHER_READS_STORAGE_FACTS_MUST_SELECT_CONFIGURED_LIBRARY_ROOT`, `MAINTENANCE_QUALITY_PROFILE_ACTION_NO_TARGET_PROFILE`, `MAINTENANCE_SEQUENCE_ACTIONS_INVALID`, `MAINTENANCE_STORAGE_ROOT_NOT_CONFIGURED` and `MAINTENANCE_TAG_ACTION_NO_TAGS_CONFIGURED`.

**Request rules**

- `updateRequestRuleMatcher` is replaced by `updateRequestRuleSource`, which takes `UpdateRequestRuleSourceInput` (`ruleSetId`, `regoSource`, `builderModel`).
- `RequestRuleEvaluationModeValue`, `RequestRuleDecision.mode` and `RequestPreflightPayload.evaluationMode` are removed.
- `matcherContentHash` is `sourceContentHash` on `RequestRulePreviewPayload` and `RequestRuleRevision`.
- Added: `RequestActionHead` (`APPROVE`, `DENY`, `MANUAL`, `SET_QUALITY_PROFILE`, `SET_LEASE_DAYS`, `SET_MONITOR`, `SET_PREFERRED_AUDIO_LANGUAGES`, `SET_REQUIRED_AUDIO_LANGUAGES`, `ADD_TAGS`, `NOTIFY`); `RequestShaping` on `RequestRuleDecision` and `RequestPreflightPayload`; `RequestDeclaredActions` on `RequestRuleSet` and `RequestRuleValidationPayload`; `RequestRuleValidationPayload.warnings`; and `ValidateRequestRuleInput.libraryIds`.
- Request rule reason codes renamed:
  - `REQUEST_RULE_RULE_FAILED_COMPILE` → `REQUEST_RULE_COMPILE_FAILED`
  - `REQUEST_RULE_REQUEST_MAY_CARRY_MOST_TAGS_SUPPLIED` → `REQUEST_RULE_TAG_LIMIT_EXCEEDED`
  - `REQUEST_RULE_TAG_CHARACTERS_MOST_ALLOWED` → `REQUEST_RULE_TAG_TOO_LONG`
  - `REQUEST_RULE_LIBRARY_FACET_DOES_NOT_MATCH_REQUESTED_MEDIA_FACET` → `REQUEST_RULE_LIBRARY_FACET_MISMATCH`
  - `REQUEST_RULE_MAY_NOT_READ_REQUEST_S_DECISION` → `REQUEST_RULE_DECISION_READ_FORBIDDEN`
  - `REQUEST_RULE_FAILED_BUILD_REQUEST_RULES_ENGINE` → `REQUEST_RULE_ENGINE_BUILD_FAILED`

**Rule builder**

- `ruleBuilderSchema(family:)` returns the subjects, outputs and output rules the Simple builder offers for a rule family.
- `builderModel` is stored on scoring rules (`CreateRuleSetInput`, `UpdateRuleSetInput`, `RuleSetPayload`), lifecycle rules and request rules. A rule without one opens as Rego.
- New validation reasons include `RULE_ACTION_REQUIRED`, `RULE_ACTION_VALUE_INVALID`, `RULE_ACTION_VALUE_NOT_LITERAL`, `RULE_ACTION_READS_OTHER_LEVEL`, `RULE_HEAD_DEFINED_AS_FUNCTION`, `RULE_QUALITY_PROFILE_UNKNOWN`, `RULE_QUALITY_PROFILE_AMBIGUOUS`, `RULE_AUDIO_LANGUAGE_UNKNOWN`, `RULE_MONITOR_UNSUPPORTED`, `RULE_UNREFERENCED_RULE`, `RULE_BUILDER_FAMILY_UNKNOWN`, `RULE_BUILDER_MODEL_INVALID` and `RULE_BUILDER_MODEL_TOO_LARGE`.

**Rule imports**

- `startExternalRuleImport(requestKey:, input: ExternalRuleImportInput)` starts an import and returns its `jobRun`; `externalRuleImportStatus(runId:)` returns `ExternalRuleImportStatusPayload` with its phase, counts and per-item report.
- Added types: `ExternalRuleImportInput` (`source`, `baseUrl`, `apiKey`), `ExternalRuleImportSource` (`SONARR`, `RADARR`, `MAINTAINERR`), `ExternalRuleImportPhase`, `ExternalRuleImportItemState`, `ExternalRuleImportItemPayload`, `ExternalRuleImportCountsPayload` and `ExternalRuleImportFactPayload`.
- The job key `EXTERNAL_RULE_IMPORT` is added.
- Reasons: `RULE_IMPORT_ALREADY_EXISTS`, `RULE_IMPORT_API_KEY_REQUIRED`, `RULE_IMPORT_FETCH_FAILED`, `RULE_IMPORT_FORMAT_UNREADABLE`, `RULE_IMPORT_FORMAT_UNSCORED`, `RULE_IMPORT_FORMAT_UNTRANSLATABLE`, `RULE_IMPORT_GROUP_UNTRANSLATABLE`, `RULE_IMPORT_INTERRUPTED`, `RULE_IMPORT_INVALID_REQUEST`, `RULE_IMPORT_ITEM_FAILED`, `RULE_IMPORT_NOT_OWNED`, `RULE_IMPORT_PLAN_FAILED`, `RULE_IMPORT_PROFILES_UNAVAILABLE`, `RULE_IMPORT_PROFILE_SCORES_APPLIED`, `RULE_IMPORT_PROFILE_SCORES_UNMATCHED`, `RULE_IMPORT_QUEUE_FULL`, `RULE_IMPORT_REQUEST_CHANGED`, `RULE_IMPORT_TOO_MANY_ITEMS`, `RULE_IMPORT_TOO_MANY_PROFILE_SOURCES` and `RULE_IMPORT_TRANSLATOR_UNAVAILABLE`.
- Source connection reasons: `EXTERNAL_IMPORT_SOURCE_UNREACHABLE`, `EXTERNAL_IMPORT_SOURCE_UNAUTHORIZED`, `EXTERNAL_IMPORT_SOURCE_ENDPOINT_MISSING`, `EXTERNAL_IMPORT_SOURCE_RATE_LIMITED`, `EXTERNAL_IMPORT_SOURCE_RESPONSE_TOO_LARGE` and `EXTERNAL_IMPORT_SOURCE_UNEXPECTED_RESPONSE`.
- Maintainerr source reasons: `MAINTAINERR_IMPORT_GROUPS_JSON_INVALID`, `MAINTAINERR_IMPORT_CONSTANTS_JSON_INVALID`, `MAINTAINERR_IMPORT_PROFILES_JSON_INVALID`, `MAINTAINERR_IMPORT_GROUPS_TOO_MANY` and `MAINTAINERR_IMPORT_JSON_TOO_LARGE`. Per-group translation findings carry `maintainerr.*` codes, such as `maintainerr.comparator.partial_list_widens`, `maintainerr.comparator.absence_widens`, `maintainerr.comparator.date_drift_widens`, `maintainerr.username.unknown_widens` and `maintainerr.scope.all_libraries`.

**Grabs**

- `startGrab` starts one grab as a background job and returns its `jobRun`; `grabJobResult(id:)` returns its outcome. The existing grab mutations are unchanged.
- The job key `RELEASE_GRAB` is added.

**Indexers and proxies**

- `maxRequestsPerMonth` and `maxGrabsPerMonth` on `CreateIndexerConfigInput`, `UpdateIndexerConfigInput` and `IndexerConfigPayload`.
- `InteractiveReleaseSearchSkipReasonValue.MONTHLY_LIMIT`.
- Reasons `INDEXER_MONTHLY_QUOTA_REQUEST_LIMIT_REACHED`, `INDEXER_MONTHLY_QUOTA_GRAB_LIMIT_REACHED`, `INDEXER_MONTHLY_QUOTA_CONFIGURATION_INVALID` and `INDEXER_MONTHLY_QUOTA_ACCOUNTING_UNAVAILABLE`.
- `reachThroughProxyConfigId` on `CreateProxyConfigInput`, `UpdateProxyConfigInput` and `ProxyConfigPayload`, with the reasons `INTEGRATION_PROXY_REACH_THROUGH_NOT_FOUND`, `INTEGRATION_PROXY_REACH_THROUGH_DISABLED`, `INTEGRATION_PROXY_REACH_THROUGH_SELF` and `INTEGRATION_PROXY_REACH_THROUGH_IS_SOLVER`.

**Security and MCP**

- `SecuritySettingsPayload` gains `mcpEnabled`, `mcpEnabledEnvOverride`, `localTrustAllowedHosts` and `restrictLocalTrustToTrustedProxies`; `UpdateSecuritySettingsInput` gains `mcpEnabled`, `localTrustAllowedHosts` and `restrictLocalTrustToTrustedProxies`. Omitting one keeps the saved value.
- Reasons `SETTINGS_LOCAL_TRUST_HOST_INVALID`, `SETTINGS_LOCAL_TRUST_HOSTS_WOULD_LOCK_OUT` and `SETTINGS_LOCAL_TRUST_PROXY_LIST_EMPTY`.
- HTTP refusals `HTTP_LOCAL_TRUST_HOST_NOT_ALLOWED` (403), `HTTP_MCP_DISABLED`, `HTTP_MCP_CONFIRMATION_MISMATCH` and `HTTP_MCP_CONFIRMATION_UNKNOWN`.
- MCP `run_mutation` takes a `confirm_token` for operations that need confirmation.

**Other additions**

- Libraries: `LibraryPayload.fullHashBackfillEnabled` and `setLibraryFullHashBackfill(input: SetLibraryFullHashBackfillInput)` (`libraryId`, `enabled`), with the recovery reasons `LOCATION_RECOVERY_DESTINATION_NAME_SIZE_NOT_FOUND` and `LOCATION_RECOVERY_DESTINATION_NAME_SIZE_AMBIGUOUS`.
- Import and acquisition reasons: `IMPORT_RETRY_LIMIT_REACHED`, `IMPORT_UNPARSEABLE_EPISODE_MEMBER`, `IMPORT_INCUMBENT_PLACED_BY_THIS_DOWNLOAD`, `IMPORT_UPGRADE_PREPARATION_FAILED`, `ADMISSION_QUEUED_ITEM_BLOCKS_UPGRADE`, `BLOCKLIST_IMPORT_NOT_AN_UPGRADE`, `DOWNLOAD_FAILURE_UNWANTED_EXECUTABLES`, `DOWNLOAD_ORIGIN_LOOKUP_FAILED` and `LOCATION_BACKFILL_SIZE_MISMATCH`.
- Backups and media: `SECURITY_BACKUP_BUNDLE_TOO_MANY_ENTRIES`, `SECURITY_BACKUP_BUNDLE_ENTRY_TYPE_UNSUPPORTED`, `SECURITY_BACKUP_BUNDLE_SCRATCH_SPACE_LOW` and `MEDIA_IMAGE_DIMENSIONS_TOO_LARGE`.
- Request rules: the `awards` fact.

- Quality profiles: `minMissingEpisodesForPack` and `minMissingPercentForSeriesPack` on `QualityProfileCriteriaInput` and `QualityProfileCriteriaPayload`.
- Titles: `TitlePayload.contentRatings` and `TitlePayload.certificationLabel`.
- Guided tours: `recordMyUiTours`, `UiSettingsPayload.tours`, `UiTourOutcomeValue` (`COMPLETED`, `SKIPPED`), with the reasons `SETTINGS_UI_TOUR_ID_INVALID` and `SETTINGS_UI_TOUR_COUNT_OUT_OF_RANGE`.
- Lists: `myListPolicy` returns the signed-in member's own list policy.
- Plugins: `dismissPluginRestoreWarnings`.
- Location moves: `locationTransferTitleOutcome(id:, titleId:)`, and `refusalReason`, `refusalFacts` and `detail` on a title's transfer outcome.
- Imports: `ImportSkipReasonValue.ARCHIVE_EXTRACTION_BLOCKED`.
- Import and location reasons: `IMPORT_DESTINATION_IS_SOURCE`, `IMPORT_DESTINATION_ALIAS_UNKNOWN`, `IMPORT_COPY_KEPT_SOURCE_GONE` (with a `keptPath` fact), `IMPORT_INCUMBENT_IDENTITY_UNDECIDED`, `LOCATION_UNPROVEN_LINK_NOT_REMOVED` (with a `leftBehindPath` fact), `LOCATION_DESTINATION_NO_SPACE`, `LOCATION_DESTINATION_PERMISSION_DENIED` and `LOCATION_SET_ASIDE_SOURCE_NOT_PUT_BACK`.
- Disk reasons, carried by refusals that come from the filesystem: `DISK_PATH_MISSING`, `DISK_PATH_UNREACHABLE`, `DISK_TIMED_OUT`, `DISK_PERMISSION_DENIED`, `DISK_NO_SPACE`, `DISK_DESTINATION_EXISTS`, `DISK_DIRECTORY_NOT_EMPTY`, `DISK_WRONG_KIND`, `DISK_MOVE_NEEDS_COPY`, `DISK_COPY_NOT_VERIFIED`, `DISK_COPY_CANCELLED`, `DISK_COPY_STAGING_OCCUPIED`, `DISK_LINK_DIFFERENT_VOLUME`, `DISK_LINK_NOT_FOLLOWED`, `DISK_LINK_NOT_SUPPORTED`, `DISK_LINK_UNVERIFIABLE`, `DISK_NOT_OBSERVED` and `DISK_OPERATION_FAILED`.
- Archive reasons: `ARCHIVE_EXTRACTION_WAITING`, `ARCHIVE_EXTRACTION_IN_PROGRESS`, `ARCHIVE_EXTRACTION_BLOCKED`, `ARCHIVE_EXTRACTION_INTERRUPTED`, `ARCHIVE_EXTRACTION_TIMED_OUT`, `ARCHIVE_EXTRACTION_PASSWORD_REQUIRED`, `ARCHIVE_EXTRACTION_WORKSPACE_OCCUPIED`, `ARCHIVE_WORKSPACE_EXTRACTIONS_BLOCKED`, `ARCHIVE_WORKSPACE_SCRATCH_IN_USE` and `ARCHIVE_WORKSPACE_UNMARKED_ENTRIES`.
- Downloads: `INTEGRATION_DOWNLOAD_NOT_GRABBED` refuses failure cleanup for a download Scryer did not grab, and `INTEGRATION_TRACKED_DOWNLOAD_NOT_IGNORABLE` refuses Ignore outside a blocked or failed import.
