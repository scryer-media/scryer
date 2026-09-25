# Scryer 0.21.10 release notes

These notes cover what's changed since **0.21.9**.

## Highlights

- **Scryer no longer re-scores your whole library every minute when nothing has changed.** The background pass that decides which titles still need a better copy used to re-read and re-score every title on every cycle, which kept CPU busy on an idle server. Scryer now remembers each answer and reuses it until something that feeds into it changes: the file, the quality profile, your scoring rules, the title's tags or runtime, or the library it sits in. Rules that look at the current time are never cached. Settings, quality profiles and anime numbering are also cached, and each cycle reads far less from the database. Nothing needs configuring.

- **Downloads that are waiting for disk space are tracked and reported.** When an import cannot finish because the destination drive is full, Scryer now records it as an ongoing incident for that destination, keeps retrying on its own, and shows the download on the Activity page as **Waiting for disk space** with the reason underneath. A **health issue** notification goes out when the problem starts and a **health restored** notification once the import goes through; if a notification cannot be delivered, Scryer retries it for up to 24 hours before giving up. Sizes and counts in these notifications are written out in words rather than raw numbers and unit codes.

- **External tools can hand Scryer a release.** Feed watchers and announce bots such as autobrr can push a release to Scryer through a new API call. Scryer matches it to a monitored title, applies your delay profiles and holds it if needed, refuses it when the title is not monitored or the caller may not manage that library, and reports **duplicate** when the same release is already downloading.

- **Sonarr- and Radarr-compatible endpoints.** Tools that only know how to talk to Sonarr or Radarr, such as Prowlarr, can now push releases to Scryer with an API key at `/compat/sonarr/api/v3/...` and `/compat/radarr/api/v3/...`. If more than one library could take a release, Scryer refuses rather than guess; add `/library/<id>` after `/compat/sonarr` or `/compat/radarr` to name the library.

- **Re-searching fully searched titles is now off by default.** Once every indexer has been searched for a title, Scryer used to search again every 30 days. It now leaves those titles to RSS unless you set **Re-converge after (days)** in Settings ▸ Acquisition. A value you had set yourself still applies.

- **Subtitle providers understand anime community numbering.** Providers now receive the per-season episode numbers that fan communities use, alongside the official numbering, so a provider such as Jimaku can find the right entry. When picking a file out of a subtitle archive, a file named with the community number now counts as good a match as one named with the official number.

## Included fixes

- **Indexers:** an indexer that cannot search for a kind of title at all, such as an anime-only indexer asked about a movie, was asked again every cycle. Scryer now records that and stops asking until you edit the indexer, refresh its capabilities, or its plugin changes what it supports.
- **Indexers:** rate-limit penalties are now kept separately per indexer and apart from the metadata service and download clients. A "slow down" answer from one indexer holds only that indexer; it no longer pauses requests to the others.
- **Indexers:** re-enabling a disabled indexer now tests the connection first. If the test fails, the indexer stays disabled and you see the error. If it passes, any leftover wait from earlier failures is cleared.
- **Indexers:** saving indexer routing while a provider filter was active dropped the routing of indexers hidden by the filter. Hidden indexers now keep their routing.
- **Grabbing:** assigning a search result to a title now needs only the manage-titles permission on that title's library. It used to also need the system-settings permission. Grabbing a result without assigning it to a title still needs system-settings.
- **Grabbing:** in the grab dialog, a download client routed under two categories was shown as one entry and the second category was lost; it now appears as two labelled choices. Plain **Grab** now also asks you to acknowledge a release's rejection reasons, as **Grab & Assign** already did.
- **Imports:** retrying an import that had no matching title now checks whether you may manage the library the title turns out to belong to. If not, the retry is recorded as failed instead of silently doing nothing.
- **Imports:** manually importing a series no longer fails its final check because of leftover video files in the download folder that Scryer never recorded. Automatic imports are unchanged.
- **Windows:** an existing title folder whose name differs from the expected one only in the upper or lower case of accented or non-Latin letters is now found instead of a second folder being created.
- **Users:** a pending invitation to a media-server account could not be cancelled while the invited user had no local password; the delete silently did nothing. It can now be cancelled from the Users page. After deleting a user the status line now says the user was deleted instead of repeating the confirmation question, and the password-changed message names the user.
- **Discover (PostgreSQL):** the genre filters on the Discover page now work.
- **Activity:** reading the queue or import list now gives up after 30 seconds and shows **Refresh timed out** with a **Retry** button instead of hanging. When a refresh fails, the page keeps the last good rows with a warning and offers **Retry** and **Load more** rather than showing an empty list.
- **Activity:** right after you act on a download, the server can briefly answer with older data. The page now keeps the current rows and asks again; only after five stale answers in a row does it report that the refresh did not reach the latest update.
- **Activity:** each import row's title is now a link to that title's page.
- **Activity:** the **Import** sub-page is now always listed in the sidebar; it used to disappear whenever nothing needed attention. The badge shows "…" while the count is loading and a star after the number if the last count refresh failed. The page no longer sends you back to Activity when the attention count reaches zero, and the badge re-reads the real count about two seconds after a live update instead of guessing.
- **Activity:** the client filter set to "all clients" now stays "all", so a download client that appears later is included automatically, and it no longer collapses to "no clients" while the client list is briefly empty during a refresh.
- **Sessions:** logging out or switching accounts in another browser tab now clears the Activity data and sidebar badges in this tab instead of leaving the old account's rows on screen.
- **Library:** changing scope or filter keeps the title counts and storage totals on screen until the new ones arrive, instead of dropping them to zero. If loading them fails, a status message is shown.
- **Dashboard:** the download-clients panel waits for both of its data sources before drawing, so it no longer shows an empty panel that fills in a moment later.
- **Backups:** the tables that record disk-space incidents are included in backups.

## API changes

These affect scripts and tools that call Scryer's GraphQL API or write plugins. The Scryer web app is already updated.

- **Added:** `submitExternalRelease(input: SubmitExternalReleaseInput!)` evaluates an externally discovered release against your catalog. The input takes `name`, `downloadUrl`, `protocol` (`USENET` or `TORRENT`), `source`, and optionally `sizeBytes`, `publishedAt`, `tvdbId`, `tmdbId`, `imdbId` and `flags`. The `ExternalReleasePayload` result carries `status` (`QUEUED`, `HELD`, `REJECTED` or `DUPLICATE`), `reasons`, and the `titleId`, `downloadId` and `pendingId` involved. The caller needs manage-titles on the matched library.
- **Added:** Sonarr- and Radarr-shaped HTTP endpoints under `/compat/sonarr` and `/compat/radarr`, optionally scoped with `/library/{id}`: `GET api/v3/system/status`, `GET api/v3/tag`, `GET api/v3/series` or `api/v3/movie`, and `POST api/v3/release/push`. Authenticate with an API key in `X-Api-Key` or as a bearer token; user sessions are not accepted.
- **Plugin SDK:** subtitle search requests may now carry a `community_entry` with the anime community season and episode numbers, AniList, AniDB and MyAnimeList ids, and the community's titles. Plugins built against the previous schema keep working.
- **Plugin host:** archive plugins can request checksums from the host through the new `archive` world 1.1.0 (`crc(algorithm, seed, data)`, covering the common CRC variants). The 1.0.0 world remains available.

## Upgrading

Scryer updates its database automatically the first time the new version starts. No configuration changes are required.

Two migrations add the tables that track disk-space incidents and their notifications. They create new tables only and complete quickly on SQLite and PostgreSQL alike.

If you relied on converged titles being searched again every 30 days, set **Re-converge after (days)** in Settings ▸ Acquisition; the default is now 0 (off).
