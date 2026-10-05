# Scryer 0.21.14 release notes

These notes cover what's changed since **0.21.13**.

## Highlights

- **Library moves and renames protect the library root.** A title can no longer claim a library root, a folder containing one, or a folder outside its library as its own. Moves, imports and title scans refuse an unsafe recorded folder instead of traversing other titles' media.
- **Incorrect title folders are repaired on upgrade.** When a title was recorded at or above a library root, or at one of its season folders, Scryer uses its tracked media files to identify the correct title folder where that can be done unambiguously. This repair changes database records only; it does not move or delete media files.
- **Empty folders left by older renames are cleaned up once.** On upgrade from a version before 0.21.14, Scryer can remove an obsolete title folder that contains only empty season folders after confirming the title has a separate, existing folder. Folders containing files, unrelated subfolders, or uncertain ownership are left alone.

## Included fixes

- **Partial renames:** the recorded title folder follows files that moved successfully, so a later scan does not detach those files because the record still points to the old folder. Files left behind remain visible for follow-up.
- **Library scans and pending imports:** a scan can correct a root-recorded title only when one folder holds all its tracked media; it does not guess when the files are spread across folders. Title scans, scoped reconciliation and pending imports refuse an unsafe root record. Folder matching no longer offers a swap that would assign a library root to another title.

## Upgrading

Scryer applies a title-folder repair migration automatically on startup for SQLite and PostgreSQL. If the tracked files do not identify one safe folder, the migration leaves that title's folder record unchanged rather than guessing. Operations that would use an unsafe title folder remain blocked until the record can be corrected.

The separate empty-folder cleanup runs in the background once when upgrading from a version before 0.21.14; a fresh installation or later restart does not run it. It removes only empty season directories and their now-empty obsolete title directory, never files. An interrupted or failed cleanup is logged but not retried automatically.
