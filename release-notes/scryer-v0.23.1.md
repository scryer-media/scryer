# Scryer 0.23.1 release notes

These changes are relative to v0.23.0.

## Highlights

- Imports that copy a file into a library on a Linux CIFS or SMB mount work again. In 0.23.0 such an import failed with a permission error and left a 0-byte file at the library path.
- The recycle bin again purges files that an upgrade replaced, including entries an earlier version had set aside as quarantined.

## Upgrade notes

- A 0-byte file that a failed 0.23.0 import left in a library is not removed by this release.
- Quarantined recycle bin entries for upgrade-replaced files are purged by housekeeping once their retention period has passed and the library holds the file that replaced them. Entries already past retention are purged on the first housekeeping run after the upgrade.

## Imports

- A copied import gives the finished file its library name without holding that name open: first by a rename that refuses to replace anything, then by a hard link, then by a reserved name that is closed before the rename. This applies on every platform.
- If another file takes the destination name while the copy is in progress, the import fails and that file is left untouched.
- A reservation that cannot be removed after a failed rename is logged as a warning with its path.

## Recycle bin

- Housekeeping re-checks quarantined entries for upgrade-replaced files after their retention period. An entry is purged when the library holds the replacement; otherwise it is left exactly as it was.
- Entries quarantined for any other reason, and entries whose layout cannot be trusted, are kept.
