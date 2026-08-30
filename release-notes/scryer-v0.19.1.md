# scryer-v0.19.1

AI generated release notes

This patch release focuses on acquisition correctness, leaner housekeeping, and safer upgrades.

## User-Facing Changes

- Acquisition scope claims now stay active through verification, repair, extraction, and import-pending states, which reduces duplicate grabs while a completed download is still being imported.
- Absolute-numbered releases that contradict the wanted episode are now rejected instead of being attached to the wrong episode scope.
- Subtitle markers such as `[ENG-Sub]` are now treated as subtitle language rather than audio language, avoiding false English-audio matches on subbed releases.
- Housekeeping now prunes more stale workflow and telemetry records, fixes retention ordering, and improves database maintenance so long-running installs stay leaner over time.
- Automatic backup retention now keeps a previous-version rollback backup across upgrades, reducing the chance of losing a recovery point during update cycles.
- WinGet release tooling has been repaired, improving Windows package delivery for this release.

## Maintenance

- CI validation was streamlined by moving benchmark dependencies out of the main dev dependency path, speeding up `nextest`-based builds.