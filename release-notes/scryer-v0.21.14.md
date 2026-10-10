# Scryer 0.21.14 release notes

These notes cover what's changed since **0.21.13**.

## Highlights

- **Library moves and renames protect the library root.** A title can no longer claim a library root, a folder containing one, or a folder outside its library as its own. Moves, imports and title scans refuse an unsafe recorded folder instead of traversing other titles' media.
- **Incorrect title folders are repaired on upgrade.** When a title was recorded at or above a library root, Scryer uses its tracked media files to identify the correct title folder where that can be done unambiguously. The repair changes database records only; it does not move or delete media files.

## Included fixes

- **Partial renames:** the recorded title folder follows files that moved successfully, so a later scan does not detach those files because the record still points to the old folder. Files left behind remain visible for follow-up.
- **Library scans and pending imports:** scans can correct a root-recorded title to a folder already containing its tracked media, while imports and title scans refuse to use a library root as a title folder.

## Upgrading

Scryer applies a title-folder repair migration automatically on startup for SQLite and PostgreSQL. If the tracked files do not identify one safe folder, the migration leaves that title's folder record unchanged rather than guessing. Operations that would use an unsafe title folder remain blocked until the record can be corrected.
