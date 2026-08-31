# scryer-v0.19.3

AI generated release notes

## Highlights

This release is focused on build and release reliability rather than application behavior.

- Improved Linux ARM release reliability by addressing stalled CI builds.
- Added build heartbeat monitoring to keep long-running ARM jobs active and easier to observe during release automation.

## Details

- Fixed stalled Linux ARM builds in CI.
- Updated the release workflow to better handle long-running ARM build steps.
- Bumped project version metadata to `0.19.3`.