# Scryer 0.21.22 release notes

These changes are relative to v0.21.21.

## Highlights

- Share your Scryer indexers with ancillary tools such as Lidarr and ebook managers through new Newznab and Torznab endpoints, with per-client keys and indexer grants. Indexer clients are in beta.
- Arrange the dashboard with a per-user grid: move, resize, add, and remove panels, with a saved layout for each screen size.
- New instances now require an explicit authentication choice on first run, with an optional one-time setup code for instances exposed before that choice is made.
- Backup restores now run as supervised background jobs with exclusive maintenance access, and only encrypted `.enc` bundles can be restored.
- Background searches leave out indexers that never answer for a media type, share pages between searches that read the same listing, and stop within their time budget.
- RSS sync takes priority over background searches and now checks every due indexer each cycle.
- Add Czech as an interface and metadata language.

## Upgrade notes

- Existing instances are not asked to make the new authentication choice. It applies only to newly created databases.
- Automated deployments of a new instance must complete first-run authentication before Scryer becomes usable, or set `SCRYER_ADMIN_PASSWORD` or `SCRYER_ADMIN_PASSWORD_FILE`.
- Until first-run setup is complete, anyone who can reach a new instance can make the authentication choice or restore a backup. If a new instance is reachable by others before you finish setup, set `SCRYER_REQUIRE_SETUP_CODE=true`. The setup code is then written to the server console or container output; desktop installs write it to `logs/console.log` in Scryer's data folder, and restarting Scryer replaces it.
- Restore accepts only encrypted `.enc` backups. An older unencrypted bundle can no longer be restored, so keep the Scryer version that can read it if you still depend on one. The bundle file itself is left untouched.
- RSS sync now wakes every 15 minutes by default instead of every 5, and each cycle considers all due indexers. `SCRYER_RSS_TICK_INDEXER_LIMIT` is removed; use `SCRYER_RSS_SYNC_TICK_SECS` to change how often the worker wakes.
- The **Glow** preference is removed, along with every saved glow choice. The interface now always uses the look without glow.
- On first startup, shows without a TMDB ID are queued once for a background metadata refresh. Startup does not wait for it, and existing metadata and retry schedules are preserved.
- Imports that were already blocked with an ambiguous older reason are not reopened automatically and need one manual retry.
- Full-file hash backfill now starts at 2:00 AM host local time instead of midnight, and still ends by 6:00 AM.
- Release parsing no longer accepts version 0 as a release revision, so `02v0` is not read as a versioned absolute episode.

## Indexer clients

- Add **Client management** under Indexers, offered as a beta feature with no experimental switch. Create a client, choose Newznab, Torznab, or both, and grant it specific indexers and categories. Nothing is granted by default.
- Clients connect to `/newznab/api` and `/torznab/api`. The page shows copyable endpoint URLs built from your public URL and base path.
- A client's key is shown once and is not stored by Scryer. Rotating or revoking a key immediately stops the old key and its retrieval links. Revoking also disables the client, and a client without a key cannot be enabled.
- Creating, changing, rotating, or revoking a client uses the same verification step as personal API keys, so users with multi-factor authentication must verify first.
- Each client shows when it was last used and how its latest request ended. Tables become cards on narrow screens.
- Feeds carry typed release facts even when an indexer does not return a Nab feed, including seeders, peers, info hash, volume factors, seed requirements, languages, and external IDs. Upstream attributes pass through without loss.
- Every feed item has a `pubDate`, so one release without a parseable date no longer makes a client reject the whole feed. Magnet links stay out of feeds, and download links point to Scryer.
- Upstream credentials are redacted from feed attributes, including configured indexer credentials. Archive passwords remain in the feed because they belong to the release.

## Dashboard

- **Edit dashboard** adds move and resize handles, panel removal, and a tray for adding panels. **Reset to default** restores the original layout.
- Each user keeps a separate layout per screen size. The default layouts match the previous fixed dashboard, and breakpoints follow the window width.
- Panels adapt to their size. Indexer and download client tables drop optional columns one at a time as space runs out.
- While arranging, the grid shows crosshairs and shaded landing areas, and it extends below the window. A dragged panel follows the cursor and its outline snaps to the nearest free cells, so a drop lands where the outline shows.
- Leaving with unsaved changes asks before discarding them.
- Saved dashboard layouts are included in backups.

## First-run setup and restore

- Choose between an administrator password and running without authentication on first run. Running without authentication requires acknowledging that anyone who can reach the instance can act as administrator.
- When `SCRYER_REQUIRE_SETUP_CODE` is set, the one-time setup code is needed for the first-run choice and for restoring a backup during first-run setup. Failed code attempts are rate limited.
- Restores own their work independently of the browser, so a disconnected page does not stop them. The restore screen shows the current phase, import progress, and translated failure reasons.
- A restore keeps its original deadline across restarts and survives a failed restart handoff. Once restored files are promoted, later startup problems are treated as ordinary startups rather than a recovery lock.
- Scheduled and background work stops during a restore. A failed Apply before promotion releases the application and restarts it to bring that work back.
- Serve startup requests through a splash screen while the instance is not ready, and close remaining gaps in startup admission.

## Search and acquisition

- After five different titles in a row come back empty from an indexer for a media type, background searches leave that indexer out of that media type for 24 hours. One search then retries it, and any usable result clears the hold. Errors, timeouts, and partial answers do not count, and interactive searches are never skipped.
- Indexers that ignore season and episode parameters are recognized by their first page. Scopes reading the same listing share one page chain, and a repeated page ends the read.
- Background walks stop inside a title when their time budget runs out and resume at the first scope they did not reach. Each title has a budget of 120 searches per walk, and a title always completes at least one scope.
- Walks rest between runs and yield to RSS sync. Adding a title or changing a profile or settings still wakes a walk immediately.
- Operator title searches no longer trigger a background walk that searches every missing season of that title.
- The RSS carry-over list survives restarts, and fresh titles take turns with carried-over ones. RSS reads only the data it needs and reuses title context across episodes and packs.
- Each search reads learning data once and writes one batch per indexer, reducing database pressure.
- Keep the protection on accepted grabs that a successful client listing no longer shows. They appear in history as **Not visible in client**, and treat an unconfirmed submission as ambiguous rather than blocklisting a release the client may have accepted.
- Keep separate regrab attempts for the same title independent, and stop retries after an unresolved adoption.
- Back off unanswered capability requests for managed indexers. Searches waiting on capabilities refresh them at interactive priority.
- Strip leading hyphens from title words in indexer queries, so they are not read as exclusion operators.

## Interactive search

- Indexer Search results are paged across all indexers, with a configurable merged page size and a time bound on each page.
- Results show more release detail: leechers, upload factor, seed requirements, flags, languages, subtitles, file count, year, and poster. Peers fall back to seeders plus leechers, and a zero download factor counts as freeleech.
- Publish dates are read from many more formats, so most Newznab results now show an age. Each indexer's count includes everything it has delivered, not only the rows on the current page.
- Each indexer shows how its capabilities were obtained during a search.
- Movie, season, and episode searches share one progress display.
- Scoring rules can read the frozen release protocol, and the rule tester accepts it.

## Parsing and matching

- Parse ordinal season labels followed by episode numbers, and four-digit episode ranges such as `S01E999-1001`. Year-sized suffixes no longer become range endpoints.
- Retain dual-name releases such as `Name (Other Name)` under both titles, and resume title recognition after bracketed prefix metadata.
- Parenthetical qualifiers stay part of the title, so `Name (US)` does not match a plain `Name`.
- Recognize `EP` and `EPISODE` labels during context inference, and read revisions consistently, including ordinal-season `vN` releases.
- Explain anime packs whose episode coverage cannot be resolved rather than reporting them as unparseable.

## Imports and downloads

- Late files stay automatically retryable. A missing mapped path is rechecked every five minutes after the grace period, with one notification. Scryer retries no-video results after 30 seconds and 2 minutes, then every 15 minutes, and sends unchanged results to manual review after 48 hours.
- Scryer-submitted downloads waiting for their completed path remain tracked after they age out of the client's activity window.
- Same-size destination files are compared by a content proof before being treated as duplicates. The check samples the start and end of each file rather than reading both in full.
- Active downloads stay available for unmatched import after their title is deleted.
- Fall back to ordinary `fsync` on macOS volumes that do not support `F_FULLFSYNC`, such as SMB mounts. Real sync errors still fail the import.
- External imports accept existing series folders with incomplete provider IDs, and refuse only a real ID conflict.
- Automatic library scans wait for setup to finish and for pending import monitoring to settle.
- Large queues no longer lose delete and import state past about 1,000 items.
- Saving a corrected download client or passing its connection test ends its failure backoff immediately.
- Weaver delete conflicts on jobs that have already left the queue wait and recheck in 20 minutes instead of warning. Other retryable cleanup failures back off to an hour and then six hours.
- The native HTTP client honors its per-host limit when starting jobs.
- Queue enrichment is cached and reused, reducing work on each queue refresh.

## Rename and metadata

- Include additional series and anime episode files in rename planning.
- Keep the on-disk folder spelling after a case-only rename, without scanning the root folder for every title in a bulk rename.
- Preserve external IDs from metadata, so naming tokens use the ID that fits the title's type.
- Explain metadata connection failures caused by a system clock that is more than 5 minutes off.

## Backups, subtitles, and plugins

- Automatic backups stay open when switched off and save as you change them. The time, key, and next run share one row. **Save** or **Clear** sits beside the key, and clearing asks first and turns off automatic backups if they are on.
- Backup location uses a compact card, and **Use default** confirms and then saves.
- Add **New folder** to the folder picker, with a translated reason when creation fails.
- Subtitle downloads that fail now count toward a provider's failure limit, and Scryer skips files a provider reports as gone.
- Plugin HTTP requests follow redirects with per-hop allowlist and DNS checks, fixing Plex RSS. Credentials are stripped on redirects, and Emby tokens are dropped on cross-origin hops.
- Fix archive, subtitle, and subtitle-sync paths on Windows.
- Update bundled Newznab and Torznab plugins to 3.2.0.
- Many more errors now have translated explanations across acquisition, imports, backups, and settings.

## Interface and performance

- Indexer page tabs move to a sidebar submenu, and promoted settings pages no longer expand the Settings menu.
- Naming a new indexer follows the selected provider until you type your own name.
- Saving a dashboard layout or catalog view no longer resets general UI preferences.
- Image cache maintenance runs ahead of hash backfill and one at a time. Backfill skips missed ticks rather than running them late.
- Fix a macOS startup failure in signed app bundles.

## API and plugin compatibility

- Add `completeInitialAuthentication` and the initial-authentication fields on `AuthRuntimeStatePayload`. Authentication state now includes `passwordMinLength`.
- Add `startInspectRestoreBundle`, `startApplyRestoreBundle`, and `restoreStatus`. `inspectRestoreBundle` and `applyRestoreBundle` remain for compatibility and accept `setupCode`. Restore passwords are now required.
- Add `indexerClients` and the create, update, rotate, and revoke mutations.
- Add `setMyDashboardLayout`, `resetMyDashboardLayout`, and `dashboardLayouts` on UI settings.
- Remove `noGlow` from UI settings and its input.
- Add `continueInteractiveReleaseSearch`, `aggregatePageSize`, `nextCursor`, `deadlineReached`, `failureReason`, and `failureReasonCode` to query searches. Add capability provenance fields to search indexer results, and the extra release facts to `IndexerSearchResultPayload`.
- Add `createFolder`, with `FOLDER_CREATE_*` refusal reasons.
- Add `DOWNLOAD_MISSING` and `TrackedDownloadStateValue.MISSING`, along with `attentionReasonCode` on queue items and `failureReasonCode` on title history.
- Add `SCENE_SUBTITLE_PENDING`, `SOURCE_CHANGED`, and `SOURCE_UNAVAILABLE` import values.
- Rule validation results add `diagnostics` with a stable code, line, column, and input path. Rule tester listings add `protocol`.
- Plugin SDK 3.20.0 adds optional fields for exact Nab operations, recent-feed paging, and host-resolved capabilities. The additions are compatible with existing plugins.
