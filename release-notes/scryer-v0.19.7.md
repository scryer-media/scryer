# scryer-v0.19.7

AI generated release notes

## User-facing changes

No user-facing application changes are included in this release. `0.19.7` focuses on release pipeline reliability and diagnostics.

## Release infrastructure updates

- Release builds now target dedicated release runners for Linux, macOS, and Windows where configured, instead of using the pull request runner setup.
- Build heartbeat logging now captures richer diagnostics during release builds, including Linux memory pressure and cgroup memory metrics.
- Heartbeat process snapshots now include parent process IDs and safely truncate very long argument lists to keep logs readable.
- The build wrapper now records received `SIGTERM` and `SIGINT` signals and emits an on-signal process snapshot to help diagnose interrupted or terminated builds.
- ARM Linux release builds now sample heartbeat data more frequently to improve visibility into long-running or unstable build jobs.

## Included change

- `684412253` `ci: instrument custom release runners`