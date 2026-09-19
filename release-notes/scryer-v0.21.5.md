# Scryer 0.21.5 release notes

These notes cover what's changed since **0.21.4**.

## Highlights

- **Library scans no longer lose files without saying so.** Several problems could leave files out of a scan while the scan still reported success.
  - **Anime named the way Sonarr names it** is now placed on the right episode. Sonarr names anime files like `Example Show (2001) - S02E01 - 027 - Episode Title`, with the absolute episode number in its own slot. Scryer read that as a range running from episode 1 to episode 27, could not match it to anything and skipped the file. Season 1 was unaffected, because there the two numbers are the same. On one test library about 1,900 anime files were being skipped this way.
  - **One bad file no longer stops the rest of a series.** Before, a temporary error reading one episode abandoned every file still waiting to be checked for that series. Now only that file is marked as failed, with the actual error, and the scan carries on with the rest.
  - **Network shares that briefly report an empty folder** no longer make a series look empty. Under heavy load some shared-folder mounts answer with an empty listing instead of an error, which could add a series with no files and no warning. Scryer now checks an empty folder again before believing it, and warns if a series folder that exists still has no media files.
  - A file the scan cannot place is now counted as **failed** in the scan's progress and logged as a warning with its path and reason. Before, it was counted as completed and logged only at debug level, so a scan could finish reporting no failures while files were missing.

- **Scryer is much faster on very large libraries.** This release removes a set of slow spots found by testing a library of more than 100,000 files.
  - Scanning a large library no longer re-reads the whole library for every title it adds. On the test library that step alone had cost more than five minutes of database time per scan.
  - Scans write far less to the database: fewer transactions per file, fewer progress records, and much less frequent database checkpointing.
  - Sorting the catalog by episodes, size or quality, refreshing artwork, and refreshing a title's recommendations are all faster.
  - An idle Scryer no longer writes a record every time a background job checks in with nothing to do, or stores the same release decision again on every RSS sync. Together these added several megabytes of history an hour that said nothing new.
  - Automatic search reads the state of your download clients once per cycle instead of once per title, and records a rejected release once rather than every time it sees it again.
  - RSS sync and the search for missing episodes now check that an indexer and a download client are set up before loading your whole catalog.
  - Media server playback tracking no longer does extra work for every change Scryer makes during a scan.

## Included fixes

- **Import:** a finished movie download that included a sample video could be imported again on every check, adding another numbered copy of the movie to your library each time. One download produced 189 copies. A movie download now counts as imported once its main video is in the library, and a repeat import of the same file is recognised and skipped. Extra copies already made are left in place for you to remove.
- **Discovery:** the discovery page, public catalog sections and the home page's top-rated list no longer fail with a "too many SQL variables" error on libraries with more than about 16,000 titles.
- **Metadata:** a movie and a series that share the same number at a metadata provider are no longer treated as the same title. Before, this made loading a title's metadata fail, and all of the metadata fetched for it was thrown away. When a title does claim an id that another title already owns, Scryer now skips only that id, logs which title owns it, and keeps the rest of the metadata.
- **Metadata:** for anime that has a movie or special listed as season 0, Scryer now links the series to its regular seasons instead of the special, so it no longer claims an id that belongs to another title.
- **Metadata:** a title whose metadata fails the same way every time is no longer retried a dozen times over half an hour. Scryer logs a warning once and stops.
- **External accounts:** **Settings → External account invites** now has a button on each row to cancel a pending invite or unlink an account that has already been claimed.

## API changes

These affect scripts and tools that call Scryer's GraphQL API. The Scryer web app is already updated.

- **Added:** `ExternalIdPayload` has a `kind` field that says what kind of thing the id names at its source, such as `movie`, `series`, `anime` or `title`. It is empty when the id does not say.
- **Added:** `ExternalIdInput` accepts an optional `kind`. Leave it out when you do not know the kind, and the id matches a stored id of any kind, as before.

## Upgrading

Scryer updates its database automatically the first time the new version starts. No configuration changes are required. On a large library the first start can take a little longer than usual while new indexes are built.

As part of the update, repeated copies of the same release decision for the same wanted item are removed, keeping the newest one of each.
