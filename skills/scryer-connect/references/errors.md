# Error codes

Shape:

```json
{"message": "validation: ...", "extensions": {"code": "VALIDATION_ERROR", "reason": "RULE_SOURCE_INVALID", "diagnostics": [ ... ]}}
```

- `extensions.code`: class of failure. Always present on Scryer's own errors.
- `extensions.reason`: specific condition, SCREAMING_SNAKE with area prefix
  (e.g. `INTERFACE_IMPORT_MAPPING_REQUIRED`). Codes stable; sentences not.
- Extra fields carry facts: `diagnostics`, `currentCount`,
  `retryAfterSeconds`, `errorId`, `refusalCode` and others.

Decide on `code` and `reason` only. Quote the message to the operator when
useful; never parse it.

## Classes

| `extensions.code` | Meaning | Do |
|---|---|---|
| `AUTHENTICATION_REQUIRED` | HTTP 401: no key, malformed, expired or revoked | Ask operator for a new key. No retry. |
| `UNAUTHORIZED` | Account lacks a permission or library grant; `reason` may say which | Report missing permission (`permissions.md`). Stop. |
| `MFA_STEP_UP_REQUIRED` | Config change needs fresh MFA check; API key cannot provide one | Explain step-up setting blocks automation. Operator decides. |
| `TOTP_ENROLLMENT_REQUIRED` | Same setting; account has no authenticator | Same |
| `MFA_ENROLLMENT_REQUIRED`, `PASSWORD_CHANGE_REQUIRED`, `REAUTHENTICATION_REQUIRED` | Account must finish a sign-in step in the web UI | Ask operator to sign in once in the browser |
| `VALIDATION_ERROR` | Input refused; `reason` names the condition when classified | Fix input for that reason. No `reason`: show message, ask. |
| `NOT_FOUND` | Id does not exist or not visible | Re-read ids from a list query. Never guess ids. |
| `TEMPORARY_UNAVAILABLE` | Dependency busy or rate limited; `retryAfterSeconds` may be set | Wait that long (or about 30 s), retry once. |
| `PLUGIN_FAILED` | Provider plugin failed; `reason` gives kind, `detail` may be set | Report. Provider config usually needs attention. |
| `LOCATION_PLAN_REFUSED`, `LOCATION_ROOT_REFUSED` | File-location plan refused; `refusalCode` gives cause | Report with the code. |
| `LOCATION_OPERATION_BUSY` | Another file operation holds the location | Wait for it to finish |
| `PUBLIC_URL_REJECTED` | URL failed safety check | Ask operator for a reachable URL |
| `CANCELED` | Operation cancelled | Ask before restarting |
| `INTERNAL_ERROR` | Server fault, with `errorId` | Report `errorId`. Do not loop. |

## HTTP-level refusals

`gql.sh` exits 2 when the body is not a GraphQL response:

- **HTTP 429**: rate limiter answered `{"error": "..."}`; `Retry-After`
  header may be set. Slow down. No parallel calls.
- **HTTP 403 "web client proof"**: no key reached Scryer in authless mode.
  Check `SCRYER_API_KEY` is exported.
- **HTTP 403 `HTTP_LOCAL_TRUST_HOST_NOT_ALLOWED`**: no key reached Scryer,
  and the host name in `SCRYER_URL` is not on the instance's allowed host
  names list. Check `SCRYER_API_KEY` is exported; requests with a key skip
  this check.
- **HTML instead of JSON**: `SCRYER_URL` points at a proxy page or wrong base
  path.

## Common reasons

| reason | Where | Fix |
|---|---|---|
| `RULE_SOURCE_INVALID` | `createRuleSet`, `updateRuleSet` | Read `extensions.diagnostics`; fix those lines. |
| `RULE_EPISODE_REQUIRED_WHEN_TESTING_SERIES_ANIME_RULE` | `testRuleSet` | Pass `episodeId` for a series or anime title |
| `RULE_EPISODE_CANNOT_SELECTED_WHEN_TESTING_MOVIE_RULE` | `testRuleSet` | Drop `episodeId` for a movie |
| `ARR_CUSTOM_FORMAT_JSON_INVALID`, `ARR_CUSTOM_FORMAT_FORMATS_EMPTY`, `ARR_CUSTOM_FORMAT_FORMATS_TOO_MANY`, `ARR_CUSTOM_FORMAT_JSON_TOO_LARGE` | `translateArrCustomFormats` | Send a JSON array of at most 1000 formats, under 16 MiB |
| `INTERFACE_IMPORT_MAPPING_REQUIRED`, `INTERFACE_IMPORT_MAPPING_ROOT_UNKNOWN`, `INTERFACE_IMPORT_ROOT_LIBRARY_MISMATCH`, `INTERFACE_IMPORT_LIBRARY_FACET_MISMATCH` | `finalizeExternalImport` | See migrate skill's import reference |
| `INTERFACE_INTERACTIVE_SESSION_REQUIRED` | key management | Web UI only |

## Rule import from Sonarr, Radarr or Maintainerr

Codes come back either as `extensions.reason` on `startExternalRuleImport` /
`externalRuleImportStatus`, as the run's `reason` in the status report, or as
an item's `reason` or fact `code`. Facts never stop the run.

| reason | Where | Meaning, fix |
|---|---|---|
| `EXTERNAL_IMPORT_SOURCE_UNAUTHORIZED` | run | Arr rejected the API key. Operator checks the key in arr's Settings → General, then start again. |
| `EXTERNAL_IMPORT_SOURCE_UNREACHABLE` | run | Refused, timed out or name did not resolve, from where Scryer runs. Fix `baseUrl` (container networking, base path). |
| `EXTERNAL_IMPORT_SOURCE_RATE_LIMITED` | run | Arr kept rate limiting. Wait, start again. |
| `EXTERNAL_IMPORT_SOURCE_ENDPOINT_MISSING` | run | Arr answered 404 for an API path Scryer reads. Usually `baseUrl` lacks arr's URL base (Settings → General) or points at another service. |
| `EXTERNAL_IMPORT_SOURCE_UNEXPECTED_RESPONSE` | run | Error status or wrong JSON shape; often `baseUrl` points at a proxy page or the other app. |
| `EXTERNAL_IMPORT_SOURCE_RESPONSE_TOO_LARGE` | run | One arr response was over 16 MiB, more than Scryer reads. |
| `EXTERNAL_IMPORT_SOURCE_URL_INVALID`, `EXTERNAL_IMPORT_SOURCE_URL_QUERY_OR_FRAGMENT`, `EXTERNAL_IMPORT_INVALID_HOST` | start | Bad URL, a `?…`/`#…` part, or a host Scryer cannot use. Fix `baseUrl`. |
| `EXTERNAL_IMPORT_SOURCE_APPLICATION_MISMATCH` | run | The URL does not identify itself as the chosen app (for example Radarr's URL sent as `SONARR`). |
| `EXTERNAL_IMPORT_COULD_NOT_DETERMINE_VERSION` | run | Arr's status reported no version Scryer can read. |
| `EXTERNAL_IMPORT_VERSION_EXPECTED_MAJOR_NEWER_UNSUPPORTED` | run | Arr is too old: Sonarr 4 or newer, Radarr 6 or newer. Operator upgrades arr. |
| `EXTERNAL_IMPORT_DID_NOT_REPORT_API_VERSION` | run | Arr did not report an API version; `baseUrl` likely is not arr. |
| `EXTERNAL_IMPORT_API_VERSION_EXPECTED_ONE_UNSUPPORTED` | run | Arr's API version is not one Scryer reads (Sonarr `v3`, `v4`, `v5`; Radarr `v3`). |
| `RULE_IMPORT_PLAN_FAILED` | run | Scryer could not load its own quality profiles (or, for Maintainerr, user names) to plan the import; nothing saved. Check Scryer's logs, then start again. |
| `RULE_IMPORT_API_KEY_REQUIRED` | start | Sonarr and Radarr need `apiKey`. |
| `RULE_IMPORT_INVALID_REQUEST` | start | Empty or invalid `requestKey`, URL or other field. |
| `RULE_IMPORT_REQUEST_CHANGED` | start | `requestKey` reused with another source or URL. Use a new key. |
| `RULE_IMPORT_QUEUE_FULL` | start | Too many imports running. Wait for one to settle. |
| `RULE_IMPORT_NOT_OWNED` | status | Unknown run id, or another account started it. |
| `RULE_IMPORT_FETCH_FAILED` | run | Source could not be read; run `message` says why. |
| `RULE_IMPORT_TOO_MANY_ITEMS` | run | More than 1000 custom formats. |
| `RULE_IMPORT_TRANSLATOR_UNAVAILABLE` | run | Source had no translator; only runs from releases before the Maintainerr import carry it. |
| `RULE_IMPORT_INTERRUPTED` | run, item | Scryer stopped mid-run; items not saved are `FAILED`. Start again: saved ones are skipped. |
| `RULE_IMPORT_ALREADY_EXISTS` | item | A rule exists under the format's plain or `REVIEW - ` name (`savedName` is the one found); `SKIPPED_EXISTS`, nothing written. |
| `RULE_IMPORT_ITEM_FAILED` | item | Saving that rule failed; item `message` says why. |
| `RULE_IMPORT_FORMAT_UNTRANSLATABLE` | item | A specification cannot be translated; `FAILED`, nothing saved. Facts hold translator diagnostics (`code`, `details.location`, `details.specificationIndex`). |
| `RULE_IMPORT_FORMAT_UNREADABLE` | item | Format could not be read; `FAILED`, nothing saved. |
| `RULE_IMPORT_PROFILE_SCORES_UNMATCHED` | item, fact | Profiles disagree and some cannot be matched to one Scryer profile: `details.unmatchedProfiles` have no Scryer profile of that name, `details.duplicateProfiles` repeat in arr with different scores. `SAVED_NEEDS_REVIEW` rule scoring 0. |
| `RULE_IMPORT_PROFILE_SCORES_APPLIED` | fact | Scores follow `input.profile.name`; `details.scores` lists them. |
| `RULE_IMPORT_FORMAT_UNSCORED` | fact | Every profile scores the format 0; the rule scores 0. |
| `RULE_IMPORT_GROUP_UNTRANSLATABLE` | item | Maintainerr group could not be translated; saved as a disabled `(Maintainerr) FAILED - <name>` starter (`SAVED_FAILED_TRANSLATION`). Facts hold `maintainerr.*` diagnostics. |
| `MAINTAINERR_IMPORT_JSON_TOO_LARGE`, `MAINTAINERR_IMPORT_GROUPS_JSON_INVALID`, `MAINTAINERR_IMPORT_CONSTANTS_JSON_INVALID`, `MAINTAINERR_IMPORT_GROUPS_TOO_MANY`, `MAINTAINERR_IMPORT_PROFILES_JSON_INVALID` | run | Maintainerr's answer was too large, unreadable or held too many groups. See `scryer-migrate-from-maintainerr/references/codes.md`. |
| `RULE_IMPORT_PROFILES_UNAVAILABLE`, `RULE_IMPORT_TOO_MANY_PROFILE_SOURCES` | run fact | Maintainerr import went on without some Sonarr/Radarr profiles. |
| `LIFECYCLE_PERSON_TARGETING_PERMISSION_REQUIRED` | item | Maintainerr group reads facts about people; the account needs `MANAGE_PERMISSIONS`. `FAILED`, nothing saved. |

