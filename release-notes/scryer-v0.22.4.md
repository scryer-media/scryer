# Scryer 0.22.4 release notes

These changes are relative to v0.22.3.

## Highlights

- One unreadable folder no longer stops every import. A title folder or library root on an unplugged drive, a share that is down or a directory Scryer may not read used to refuse imports, renames, scans and deletes of every other title.
- Title search, catalog sorting, title history, the requests list and the calendar read indexes instead of scanning their tables.
- NZB and torrent file downloads from an `http://` link try HTTPS first.
- A failed automatic import shows in import history.

## Upgrade notes

- The title search index is rebuilt once on first start.
- First start adds indexes for catalog sorting, requests, the calendar and history, and records the request on each stored request submission.
- The requests page loads requests in pages instead of all at once.
- Title history shows the range of the page without a total.

### API changes

- `titleHistory` no longer returns `totalCount`. Use `hasMore` to page.
- `mediaRequests` and `myMediaRequests` take `limit` and `offset`. An omitted `limit` returns 200 requests, where earlier releases returned every request; the maximum is 1,000.
- `mediaRequests` takes `order`, a `MediaRequestOrderValue`: `RECENTLY_UPDATED` (the default) or `OLDEST_CREATED`.
- The refusal `CANNOT_INSPECT_FOLDER_OWNED_ANOTHER_TITLE` stays defined and is no longer raised.

## Libraries and folders

- Another title's folder, or a library root, that cannot be read no longer refuses an import, rename, scan or delete. Only the folder the action itself uses must resolve. An unreadable folder is compared by its spelling, without regard to case, and a folder that holds it or lies inside it is still refused as an overlap.
- A title folder on a missing drive or volume is refused with a message that names the folder and the drive.
- A folder that shares a library root's name in another letter case is checked for links like any other folder.
- The Add dialog starts Monitor type on the library's default monitoring for new titles. It started on the built-in default whatever the library had saved.

## Indexers and downloads

- A download from an `http://` link tries HTTPS first and falls back to the advertised link when HTTPS is unavailable, including through the indexer's assigned proxy.
- An HTTPS attempt that the server redirects back to the same host over HTTP falls back to the advertised link instead of ending as a redirect loop.
- The HTTPS attempt counts toward the indexer's request total only when a connection was made.

## Imports and activity

- An automatic import that ends early records its title, so the failed attempt is listed in import history.
- The log line for an automatic import retry carries the error.

## Performance

- Title search finds names that contain the search text through the search index instead of scanning every stored name, and a title is searchable as soon as it is written.
- Catalog sorts by year, runtime, popularity, status and external rating read indexes.
- Title history reads a bounded page for each event type. A single title's history reads only the request submissions that can belong to that title.
- The requests list pages in the database, and the dashboard asks for the longest-waiting pending requests instead of sorting a page.
- The calendar's movie release-date window reads an index instead of scanning every title.
- The Missing view, activity pages, request policy checks, dashboard import evidence and calendar availability read their rows in batches instead of one row at a time.
- The Upgrades view reuses its list until wanted items change.
- Poster polling pauses while the tab is hidden.
