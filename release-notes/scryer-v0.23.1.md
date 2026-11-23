# Scryer 0.23.1 release notes

These changes are relative to v0.23.0.

## Highlights

- Imports that copy a file into a library on a Linux CIFS or SMB mount work again. In 0.23.0 such an import failed with a permission error and left a 0-byte file at the library path.
- The recycle bin again purges files that an upgrade replaced, including entries an earlier version had set aside as quarantined.

## Upgrade notes

- On the first start, a one-time background pass removes the 0-byte files that failed 0.23.0 imports left in libraries. It reads the destinations recorded with failed imports and scans no library. A path is removed only when it is under a library root and is a regular file of 0 bytes. When a scan had already recorded that 0-byte file for a movie or episode, the record is removed with it and the item shows as missing again.
- Only the last file that failed in a multi-file download has its destination recorded, and a later retry replaces the record. A 0-byte file the pass does not find is replaced when an import next lands on its name.
- Quarantined recycle bin entries for upgrade-replaced files are purged by housekeeping once their retention period has passed and the library holds the file that replaced them. Entries already past retention are purged on the first housekeeping run after the upgrade.

## Imports

- A copied import gives the finished file its library name without holding that name open: first by a rename that refuses to replace anything, then by a hard link, then by a reserved name that is closed before the rename. This applies on every platform.
- If another file takes the destination name while the copy is in progress, the import fails and that file is left untouched.
- An import whose destination holds a 0-byte regular file removes that file and takes its name. Before, the import stopped for manual reconciliation. A destination with any content, or a link, still stops the import.
- A reservation that cannot be removed after a failed rename is logged as a warning with its path.

## Recycle bin

- Housekeeping re-checks quarantined entries for upgrade-replaced files after their retention period. An entry is purged when the library holds the replacement; otherwise it is left exactly as it was.
- Entries quarantined for any other reason, and entries whose layout cannot be trusted, are kept.
