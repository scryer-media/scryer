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
