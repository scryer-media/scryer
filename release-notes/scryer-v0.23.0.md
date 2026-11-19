# Scryer 0.23.0 release notes

These changes are relative to v0.22.5 and cover `scryer-v0.22.5..59d4677d4`.

## Highlights

- Maintenance rules are now **lifecycle rules**. A rule is one Rego source whose heads say what it does: unmonitor, delete files, delete the title, search, change the quality profile, add or remove tags, move to another library or root, fetch subtitles or notify, at the title, season or episode level. Each rule runs on its own schedule and can read watchlists, play counts, Jellyfin and Emby favorites, title details, series shape and free space.
- **Request rules shape a request** as well as approve or deny it: they can set the quality profile, lease, monitoring, audio languages and tags, and send a notification.
- **One Simple builder** writes scoring, lifecycle and request rules from dropdowns, and Advanced shows the Rego it wrote. The rules pages are out of the experimental switch and marked beta.
- **Guided tours**: new accounts get an onboarding tour of the interface, and each release can show a tour of what changed. Tours are recorded per account.
- **Monthly indexer limits**: an indexer can carry a monthly request limit and a monthly grab limit. An indexer at its limit is passed over without counting as a failure.
- **Safer file handling**: one disk service now carries out every filesystem operation. Moves, imports, copies and the recycle bin never replace a different file, never remove the only copy, and take back only what they placed. A dead mount no longer ties up threads.
- **Archives are extracted once per download**, into a workspace of their own, instead of again on every import retry.
- **Pack thresholds per quality profile** replace the fixed rule for when a season or multi-season pack may be taken.
- **Content ratings** are stored per title and shown as a certification badge on the overview.
- **Dolby Vision is detected** in real Matroska and MP4 files.
- **Grabs and searches run in the background**: a grab button no longer holds a request open, and episode, series-movie and cutoff searches wait in line instead of running twice.
- **Built-in MCP endpoint**: Scryer serves `/mcp` so an MCP client can run the operator skills' operations with an API key.

## Upgrade notes

- **Every lifecycle (maintenance) rule you created is removed on upgrade**, with its revisions, library scopes, exclusions, evaluation runs, candidates and action history. Rules written before rules declared their own actions cannot be read in the new form. Only database rows are removed; no file on disk is touched. The built-in expired request leases rule is kept, and its source is updated on first start.
- **Every request rule set is removed on upgrade**, with its revisions and library scopes. Requests, request decisions and their history, leases, titles and tags stay. A stored decision keeps the name of the rule set that took it.
- Lifecycle instance gate settings and the evaluation job's run history are kept under their new names.
- Request rule sets have one state: enabled or disabled. The evaluation mode is gone, and every decision is enforced.
- A lifecycle rule that deletes files or titles must carry a maximum number of titles per run. The built-in per-run limits are gone. Rules that do not delete may leave the maximum empty for no limit.
- New lifecycle rules run every 8 hours by default.
- The pack thresholds default to the values Scryer used before: a pack needs at least 2 missing episodes, and a multi-season pack needs more than 30% of its monitored, aired episodes missing. Existing profiles read these defaults until they are changed.
- First start queues a metadata refresh for existing titles so their content ratings are stored.
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
- A rule loads only the fact groups it names.
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

## Archives

- A download's archives are extracted once, into a workspace under the library root, and later import passes read from it.
- The workspace lives until the download settles, and is reused only for the same archives.
- Extraction stops after three attempts. A failed or interrupted extraction's partial workspace is removed at once.
- A download whose archives have no installed extractor creates nothing and is reopened when an extractor plugin is installed.
- Leftover workspaces are swept, and a health check reports them.
- Manual imports and downloads without a client item id extract per pass, as before.

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
- A rematch clears a title's stored content ratings.
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

## Lists, plugins and accounts

- A member whose lists are not allowed no longer sees the Lists page, unless they manage lists.
- Restore warnings on the Plugins page can be dismissed.
- WebSockets whose sign-in is stale are closed. A sign-out heard by no connection is stored, so later connections are not closed at once.

## MCP endpoint

- Scryer serves a Model Context Protocol endpoint at `/mcp`: stateless JSON over HTTP, admitted by API key only, behind the same origin policy, setup gate, restore admission and rate limits as the GraphQL API.
- Its tools are `whoami`, `list_operations`, `describe_operation`, `run_query`, `run_mutation`, `read_diagnostics_section` and `search_service_logs`.
- The run tools execute only the operations shipped with the operator skills, as the key's account, with each operation's own permission checks.
- Each call is charged to the rate limit the GraphQL API gives the same operation. A result over 32 KiB is refused with its size.
- The diagnostics bundle's release decision tally is filled in.

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

## Translations

- Czech gains the monthly indexer limit text.
- Lifecycle, request rule and watch-sync text is complete in English, Czech, Russian and Traditional Chinese (Taiwan and Hong Kong).
- Error reasons read as sentences in every area.

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

**Grabs**

- `startGrab` starts one grab as a background job and returns its `jobRun`; `grabJobResult(id:)` returns its outcome. The existing grab mutations are unchanged.
- The job key `RELEASE_GRAB` is added.

**Indexers and proxies**

- `maxRequestsPerMonth` and `maxGrabsPerMonth` on `CreateIndexerConfigInput`, `UpdateIndexerConfigInput` and `IndexerConfigPayload`.
- `InteractiveReleaseSearchSkipReasonValue.MONTHLY_LIMIT`.
- Reasons `INDEXER_MONTHLY_QUOTA_REQUEST_LIMIT_REACHED`, `INDEXER_MONTHLY_QUOTA_GRAB_LIMIT_REACHED`, `INDEXER_MONTHLY_QUOTA_CONFIGURATION_INVALID` and `INDEXER_MONTHLY_QUOTA_ACCOUNTING_UNAVAILABLE`.
- `reachThroughProxyConfigId` on `CreateProxyConfigInput`, `UpdateProxyConfigInput` and `ProxyConfigPayload`, with the reasons `INTEGRATION_PROXY_REACH_THROUGH_NOT_FOUND`, `INTEGRATION_PROXY_REACH_THROUGH_DISABLED`, `INTEGRATION_PROXY_REACH_THROUGH_SELF` and `INTEGRATION_PROXY_REACH_THROUGH_IS_SOLVER`.

**Other additions**

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
