# Scryer 0.19.12 release notes

## Highlights

- Anime released under the community's per-cour season layout now matches a catalog that follows TVDB's official order. Scryer translates release and file numbering on every lane that reads a parsed name — search and RSS scoring, parked-release adoption, single-file and pack import, manual-import preview, and the library scan — so a release named `S04E20` can satisfy a wanted `S01E56`. A release with several equally good readings is held for review rather than guessed at, and wanted episodes in a later cour also gain community-numbered search queries. Non-anime titles and anime without a stored numbering bridge are unaffected.
- Movies whose filename year disagrees with their folder year now import instead of stalling as unresolved. The scan retries the metadata search with the folder year, and then without a year, and records each attempt on the pending item.
- The Search & Match dialog shows the year it is searching with as a clearable input, reports a failed search inline instead of looking empty, and lists candidates that already exist in your library so you can attach the pending item to the existing title.
- Enabling login no longer sends you to Profile and back. The Security page now opens a set-password dialog, sets the account password, and turns on form login in one pass.
- Log timestamps follow the host's local time zone, honouring `TZ`, and always carry a UTC offset. Operators running in Docker no longer see timestamps that disagree with their wall clock.
- The routing category picker is driven by each indexer's own capabilities: known category codes gain the indexer's label, an "Also on this indexer" section lists the categories specific to that indexer for the current scope, and a "Custom codes" section routes to categories outside the standard tree. Prowlarr-managed indexers populate their picker immediately.
- Automatic search explains itself when it queues nothing. Instead of "no auto-eligible release found", Scryer names the quality rules that blocked the candidates, or points at routing categories and indexer health when nothing was returned at all.
- Post-processing runs with script output capture enabled now keep the last 32 KiB of stdout and stderr, up from 4 KB, stored compressed so the larger tails cost roughly what the old ones did.
- The Activity tab shows downloads again: imported-but-still-seeding entries are capped at five visible rows with the rest behind a disclosure row, the same way active imports already work.
- Several slow database queries were removed from hot paths — image proxy caching, single-key settings reads, activity stream startup, and background acquisition cycles that searched indexers already under a backoff.

## Included fixes

- Backups failed outright on builds carrying the anime numbering bridge, with "backup catalog is missing classifications". Manual and scheduled backups work again, and the bridge is rebuilt on restore like other derived title data.
- A series movie with no linked episode is no longer filed as an episode that does not exist. Scryer resolves the special by exact film title or release date when exactly one season-zero episode matches, and otherwise names the file without an episode token.
- A release parsed as a season-one episode can no longer satisfy a wanted special.
- Refreshing a catalog list no longer blanks fields on the title panel already open. Most visibly, the "Watch in" media server control and the root folder path stopped disappearing depending on how a refresh raced with the panel loading, and an episode's playback links survive a series overview refresh.
- The title action bar no longer offers Monitor, Search, Interactive, Refresh, History, Edit and Delete to users who lack manage rights on that title's library, where every one of those buttons failed.
- Saving a quality profile twice in a row now shows the confirmation both times.
- The header's "Enable login" entry works on a repeat click while you are already on the Security page.
- TRaSH Guides data is refreshed to 830 active release-group rules. The `sqp-4-ma-hybrid` score envelope moves its maximum from 3070 to 3970; every score set keeps its existing minimum and veto values, so vetoes are still treated as vetoes.

## Upgrading

Two schema migrations run automatically on first start: one adds per-title storage for anime numbering bridges, the other converts stored post-processing output tails to their compressed form. Existing rows are converted in place. Take your usual backup before upgrading.
