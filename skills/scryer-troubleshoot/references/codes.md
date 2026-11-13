# Codes to branch on

Branch on these codes. Never decide anything from an error sentence or a
log message; use those only as context for the operator.

## GraphQL errors

Every GraphQL error has `extensions.code`; many also carry
`extensions.reason`, a stable area code. Common `code` values:
`VALIDATION_ERROR`, `NOT_FOUND`, `UNAUTHORIZED`, `TEMPORARY_UNAVAILABLE`
(may carry `retryAfterSeconds`), `PLUGIN_FAILED`, `MFA_STEP_UP_REQUIRED`,
`REAUTHENTICATION_REQUIRED`, `PASSWORD_CHANGE_REQUIRED`,
`LOCATION_PLAN_REFUSED`, `PUBLIC_URL_REJECTED`, `DOWNLOAD_SUBMIT_UNAVAILABLE`.

`UNAUTHORIZED` on a read query usually means the key's user lacks a
permission. Diagnostics, indexers, download clients and service logs need
**Manage system settings**; quality, routing and delay
settings need **Manage catalog settings**.

## Release decision codes (`decisionCode`, `summary.release_decisions.by_code`)

| code | meaning | usual next look |
|---|---|---|
| `eligible` | automatic search would grab it | if nothing grabbed: download client, queue |
| `parse_ambiguous`, `parse_unparseable` | the release name could not be read reliably | likely naming issue; candidate for `scryer-report-bug` with the release name |
| `numbering_pack_unresolved` | the pack's episode range could not be mapped | anime or absolute numbering |
| `title_mismatch` | release title does not match the title | correct match? alias? |
| `episode_mismatch` | numbering does not match the wanted episode | numbering scheme |
| `episode_not_monitored` | covers an episode the library does not monitor | monitoring |
| `acquisition_paused` | acquisition is paused for covered content | title or library pause |
| `acquisition_state_unavailable` | state check failed; transient | retry later |
| `category_mismatch` | indexer category contradicts the title (movie vs series) | indexer categories |
| `ambiguous_identity` | title name is ambiguous and no year or id disambiguates | |
| `title_year_mismatch` | year is more than a year off | |
| `episodic_release_for_movie` | a pack or episode release for a movie | |
| `pack_for_single_episode` | automatic single-episode search skips packs | expected |
| `anime_numbering_ambiguous` | several equally good episode readings | |
| `quality_blocked` | quality profile blocked it | quality profile items, blocked codecs or sources |
| `negative_score` | historical rows only; no longer emitted | |
| `upgrade_rejected` | upgrade policy rejected it | `allowUpgrades`, `minUpgradeScoreDelta` |
| `cutoff_reached` | the existing file meets the cutoff | expected |
| `proper_for_old_file` | existing file too old to replace with a PROPER | expected |
| `already_active` | same release already in the queue | queue |
| `queued_better_or_equal` | a better or equal release is downloading | queue |
| `db_blocklisted` | blocklisted after an earlier failure | title history `BLOCKLISTED` |
| `pending_delay` | eligible but held by a delay profile | pending releases, delay profiles |
| `minimum_age` | under the usenet minimum age | delay profile `minAgeMinutes` |
| `release_age_unknown` | indexer gave no publish time and an age gate applies | indexer; health check `ReleaseAgeUnknown` |
| `protocol_disabled` | delay profile disables this protocol | delay profile enableUsenet/enableTorrent |
| `download_client_unavailable` | no usable client for the protocol | download clients |
| `repack_group_mismatch` | repack is from a different group than the file | expected |
| `minimum_seeders` | too few seeders for the seeding profile | seeding profile |
| `movie_availability_blocked` | movie has not reached its availability setting | media settings |
| `pack_below_missing_threshold` | pack does not cover enough missing episodes | expected |
| `subtitles_only` | subtitles, no video | expected |

Stored rows can also show `manual_override`, `initial` and `upgrade`, which
record how a grab was decided rather than a refusal.

## Queue and import

`displayState`: QUEUED, DOWNLOADING, PAUSED, POST_PROCESSING, COMPLETED,
IMPORTED_SEEDING, FAILED, WARNING, IMPORTING, IMPORT_PENDING, IMPORT_BLOCKED,
IMPORT_FAILED, IGNORED, REMOVING, REMOVE_FAILED.

`trackedState`: MISSING, DOWNLOADING, IMPORT_PENDING, IMPORTING, IMPORTED,
IMPORTED_SEEDING, IMPORT_BLOCKED, FAILED_PENDING, FAILED, IGNORED.

`attentionReasonCode`: `IMPORT_<importErrorCode>`, `IMPORT_FAILED`, or
`download_password_retry`. `passwordFailureCode`:
ARCHIVE_PASSWORD_REQUIRED, ARCHIVE_PASSWORD_OR_CORRUPTION.

`importErrorCode`:

| code | likely cause |
|---|---|
| FILE_NOT_FOUND | Scryer cannot see the client's output path: remote path mapping or container volume |
| PERMISSION_DENIED | file ownership or mount permissions |
| DISK_FULL | target filesystem full |
| IO_FAILED | filesystem or network share error |
| EPISODE_NOT_FOUND, EPISODE_LOOKUP_FAILED | metadata does not have the episode yet |
| POLICY_MISMATCH | file does not match the profile or rules |
| SOURCE_JOB_FAILED | the client reported failure |
| SCENE_SUBTITLE_PENDING | waiting for a subtitle file |
| UNKNOWN | read the log lines around the import |

Import `decision`: IMPORTED, REJECTED, SKIPPED, CONFLICT, UNMATCHED, FAILED.
`skipReason`: ALREADY_IMPORTED, DUPLICATE_FILE, POST_DOWNLOAD_RULE_BLOCKED,
POLICY_MISMATCH, UNRESOLVED_IDENTITY, UNPARSEABLE_EPISODE, NO_VIDEO_FILES,
UNWANTED_EXECUTABLES, DOWNLOAD_IN_PROGRESS, SOURCE_CHANGED,
SOURCE_UNAVAILABLE, SCENE_SUBTITLE_PENDING, DISK_FULL, PERMISSION_DENIED,
PASSWORD_REQUIRED, ARCHIVE_EXTRACTION_PLUGIN_REQUIRED,
ARCHIVE_EXTRACTION_TIMED_OUT, TITLE_FOLDER_CONFLICT.

## Indexer errors

`classification`:

| group | values | likely cause |
|---|---|---|
| key or account | NEWZNAB_INVALID_API_KEY, NEWZNAB_ACCOUNT_SUSPENDED, NEWZNAB_INSUFFICIENT_PRIVILEGES, NEWZNAB_API_DISABLED, HTTP_UNAUTHORIZED, HTTP_FORBIDDEN | wrong or expired key, account state; operator checks with the indexer |
| limits | NEWZNAB_REQUEST_LIMIT_REACHED, NEWZNAB_DOWNLOAD_LIMIT_REACHED, HTTP_RATE_LIMITED | daily quota or rate; lower `maxQueriesPerMinute` or wait |
| request shape | NEWZNAB_MISSING_PARAMETER, NEWZNAB_INCORRECT_PARAMETER, NEWZNAB_NO_SUCH_FUNCTION, NEWZNAB_FUNCTION_NOT_AVAILABLE, HTTP_BAD_REQUEST, HTTP_NOT_FOUND | wrong base URL or API path, or the indexer lacks a search type |
| server | HTTP_SERVER_ERROR, HTTP_REQUEST_TIMEOUT, NEWZNAB_UNKNOWN_ERROR, UNKNOWN | indexer outage, proxy, DNS or network |
| other | NEWZNAB_NO_SUCH_ITEM, NEWZNAB_REGISTRATION_DENIED, NEWZNAB_REGISTRATIONS_CLOSED, NEWZNAB_INVALID_REGISTRATION, NEWZNAB_INVALID_REGISTRATION_EMAIL, NEWZNAB_REGISTRATION_FAILED | item gone, or registration flows |

`operation`: CONNECTION_TEST, INTERACTIVE_SEARCH, AUTOMATIC_SEARCH, RSS_SYNC,
INDEXER_ACTION, MANAGEMENT_SYNC, CAPS_REFRESH.

## Health checks

From `HealthChecks.graphql`: `summaryJson.checks[] = {source, status, message}`.
There is no per-check code; branch on `source` and `status` only.

| source | status | condition |
|---|---|---|
| DownloadClient | error | no download client configured |
| DownloadClient | warning | all clients disabled; clients reporting errors; output path unresolvable after remote path mapping; output roots overlap library roots; invalid remote path mappings |
| Indexer | warning | no indexer configured, or all disabled |
| Indexer | error | all indexers failing |
| RootFolder | error / warning | root folder missing / read-only |
| RecycleBin | error | recycle bin cleanup disabled |
| DiskSpace | error / warning | under 100 MB / under 500 MB free |
| ReleaseAgeUnknown | warning | an indexer returns releases with no publish time |

## Upgrade

`installationKind`: PORTABLE, DIRECT_MSI, MACOS_APP_BUNDLE, DOCKER, HOMEBREW,
WINGET, WINDOWS_SUPERVISED, DISABLED, UNSUPPORTED. `managementOwner`:
IN_APP (Scryer upgrades itself) or OPERATOR (image, package manager or
installer).

`eligibilityReason`:

| value | meaning |
|---|---|
| eligible | in-app upgrade is available |
| disabled_by_operator | self-upgrade turned off by environment |
| managed_by_docker | pull a new image and recreate the container |
| managed_by_homebrew | `brew upgrade` |
| managed_by_winget | `winget upgrade` |
| windows_supervised | the Windows installer or tray manages it |
| unsupported_layout | install layout not recognised |
| install_dir_not_writable | the process cannot write its install directory |
| app_bundle_translocated | macOS runs the app from a quarantine copy; move it to Applications |
| app_bundle_read_only_volume | the app bundle is on a read-only volume |
| app_bundle_not_tray_supervised | the macOS app was not started by its tray supervisor |
