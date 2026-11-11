# Refusal codes

A refused write comes back as a GraphQL error. Branch on
`extensions.reason` (and `extensions.code`); show the operator the
sentence only as context. `code` is usually `VALIDATION_ERROR`; `NOT_FOUND`
and `UNAUTHORIZED` also occur. `UNAUTHORIZED` means the key's user lacks the
permission named in the area's reference.

## Config values (any provider)

| reason | fix |
|---|---|
| `INTERFACE_CONFIG_KEY_REQUIRED` | an entry has an empty `key` |
| `INTERFACE_CONFIG_VALUE_AMBIGUOUS` | an entry carries more than one value field |
| `INTERFACE_CONFIG_JSON_INVALID` | stored or sent config is not valid; rebuild from `config-to-input.sh` |
| `INTEGRATION_API_KEY_APPEARS_MASKED_PLACEHOLDER_ENTER_REAL_KEY` | a masked placeholder was sent as a secret; ask the operator for the real value via a file |
| `INTEGRATION_BASE_URL_INVALID`, `INTEGRATION_BASE_URL_HOST_REQUIRED` | fix the URL |
| `INTEGRATION_BASE_URL_CREDENTIALS_FORBIDDEN` | the URL contains `user:pass@`; move credentials to their own fields |

## Indexers

| reason | fix |
|---|---|
| `INTEGRATION_INDEXER_NAME_REQUIRED`, `INTEGRATION_PROVIDER_TYPE_REQUIRED` | supply it |
| `INTEGRATION_INDEXER_FIELD_REQUIRED` | a required provider field is missing; re-read the fields |
| `INTEGRATION_INDEXER_CONNECTION_URL_REQUIRED` | the CONNECTION_URL field is empty |
| `INTEGRATION_INDEXER_CONFIG_ID_REQUIRED`, `INTEGRATION_INDEXER_ID_REQUIRED` | the input lacks the indexer id |
| `INTEGRATION_INDEXER_CONFIG_NOT_FOUND` | stale id; re-read the list |
| `INTEGRATION_INDEXER_CONFIG_INVALID`, `INTEGRATION_INDEXER_CONFIG_JSON_INVALID` | config rejected; compare with the provider's fields |
| `INTEGRATION_NO_INDEXER_PROVIDER_AVAILABLE_PROVIDER_TYPE`, `INTEGRATION_INDEXER_PROVIDER_UNAVAILABLE` | the provider plugin is not installed or enabled |
| `INTEGRATION_INDEXER_CONNECTION_TEST_FAILED` | the indexer answered with a failure; check key and URL with the operator |
| `INTEGRATION_INDEXER_CONNECTION_UNREACHABLE` | network, DNS, TLS or proxy; check from the Scryer host |
| `INTEGRATION_INDEXER_REQUEST_REJECTED` | the indexer refused the request (key, account, limits) |
| `INTEGRATION_INDEXER_CONNECTION_TEST_DID_NOT_RETURN_VALID_NEWZNAB_CAPS_DOCUMENT` | the URL does not point at a newznab API; check the path |
| `INTEGRATION_INDEXER_SETTINGS_CHANGED_DURING_VALIDATION` | someone else saved meanwhile; re-read and retry |
| `INTEGRATION_MANAGED_CHILD_INDEXERS_CONTROLLED_THEIR_PARENT_SYNC_CANNOT_EDITED_DIRECTLY` | edit it in the indexer manager instead |
| `INTEGRATION_MANAGED_INDEXERS_CANNOT_USE_PROXY` | remove the proxy from the input |
| `INTEGRATION_PROWLARR_INDEXERS_CANNOT_USE_CHALLENGE_SOLVERS` | not supported for that indexer |
| `INDEXER_CATEGORY_MISMATCH` | categories do not fit the indexer's capabilities |
| `INDEXER_CAPS_REFRESH_FAILED`, `INDEXER_CAPS_REFRESH_DEFERRED` | capabilities could not be refreshed now; retry later |
| `TEMPORARY_UNAVAILABLE` code (with `retryAfterSeconds`) | wait that long; tell the operator the time in their local zone |

## Proxies (when referenced)

| reason | fix |
|---|---|
| `INTEGRATION_PROXY_CONFIGURATION_NOT_FOUND`, `INTEGRATION_PROXY_CONFIG_NOT_FOUND` | stale proxy id |
| `INTEGRATION_PROXY_DISABLED_INDEXER`, `INTEGRATION_PROXY_DISABLED_DOWNLOAD_CLIENT` | the proxy is disabled; enable it in the UI or pick another |

## Download clients

| reason | fix |
|---|---|
| `INTEGRATION_DOWNLOAD_CLIENT_NAME_REQUIRED`, `INTEGRATION_CLIENT_NAME_REQUIRED` | supply a name |
| `INTEGRATION_CLIENT_TYPE_REQUIRED` | supply `clientType` |
| `INTEGRATION_DOWNLOAD_CLIENT_TYPE_UNSUPPORTED` | provider not installed or enabled |
| `INTEGRATION_DOWNLOAD_CLIENT_FIELD_REQUIRED` | a required field is missing |
| `INTEGRATION_DOWNLOAD_CLIENT_CONFIG_INVALID` | config rejected |
| `INTEGRATION_DOWNLOAD_CLIENT_CONFIG_NOT_FOUND`, `INTERFACE_DOWNLOAD_CLIENT_NOT_FOUND` | stale id |
| `INTERFACE_DOWNLOAD_CLIENT_PROVIDER_MISMATCH` | the test's `clientType` differs from the saved client's |
| `INTEGRATION_DOWNLOAD_CLIENT_CONNECTION_TEST_FAILED` | client unreachable or credentials wrong; check from the Scryer host or container |
| `INTEGRATION_ACTION_PERMISSION_DENIED` | the user lacks the permission |

Remote path mappings (`remote_path_mappings`), all prefixed
`DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_`:

| suffix | fix |
|---|---|
| `INVALID`, `LINE_INVALID` | each line must be `REMOTE => LOCAL` |
| `LINE_REQUIRES_REMOTE_PATH`, `LINE_REQUIRES_LOCAL_PATH` | a side is empty |
| `LINE_LOCAL_PATH_INVALID` | the local side must be absolute |
| `LINE_DUPLICATES_EXISTING_REMOTE_PATH_ROOT` | two lines share a remote root |
| `LINE_REMOTE_PATH_CANNOT_START_WHITESPACE`, `LINE_REMOTE_PATH_CANNOT_END_WHITESPACE` | trim the remote path |

`DOWNLOAD_PATH_MAPPING_DOWNLOAD_CLIENT_CONFIG_JSON_INVALID`: the stored
config could not be read; rebuild it.

## Routing

| reason | fix |
|---|---|
| `SETTINGS_INDEXER_ROUTING_ENTRY_REQUIRES_INDEXER_ID` | an entry lacks `indexerId` |
| `SETTINGS_DOWNLOAD_CLIENT_ROUTING_ENTRY_REQUIRES_CLIENT_ID` | an entry lacks `clientId` |

## Quality profiles

| reason | fix |
|---|---|
| `SETTINGS_QUALITY_PROFILE_ID_REQUIRED` | a profile lacks `id` |
| `SETTINGS_QUALITY_PROFILE_UNKNOWN`, `SETTINGS_QUALITY_PROFILE_NOT_FOUND` | the id does not exist |
| `SETTINGS_QUALITY_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE`, `SETTINGS_QUALITY_PROFILE_ID_AMBIGUOUS_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE` | two ids differ only by case; pick a distinct id |
| `SETTINGS_PROFILE_ID_REQUIRED_WHEN_INHERIT_GLOBAL_FALSE` | a selection with `inheritGlobal: false` needs `profileId` |
| `SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_*` | a profile is still in use (global default, library, requests, titles); this skill never removes profiles |

## Delay profiles

| reason | fix |
|---|---|
| `SETTINGS_DELAY_PROFILE_ID_REQUIRED` | supply `id` |
| `SETTINGS_DELAY_PROFILE_NOT_FOUND` | stale id |
| `SETTINGS_DELAY_PROFILE_NAMES_UNKNOWN_LIBRARY` | a `libraryIds` entry does not exist |
| `SETTINGS_DELAY_PROFILE_CATALOG_INVALID` | the stored set of profiles is invalid; stop and report it |
