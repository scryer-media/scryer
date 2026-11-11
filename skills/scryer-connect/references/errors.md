# Error codes

A GraphQL error looks like this:

```json
{"message": "validation: ...", "extensions": {"code": "VALIDATION_ERROR", "reason": "RULE_SOURCE_INVALID", "diagnostics": [ ... ]}}
```

- `extensions.code` gives the class of failure. It is always present on
  Scryer's own errors.
- `extensions.reason` gives the specific condition, in SCREAMING_SNAKE with
  an area prefix (for example `INTERFACE_IMPORT_MAPPING_REQUIRED`). Codes are
  stable. The sentences are not.
- Extra fields carry facts: `diagnostics`, `currentCount`,
  `retryAfterSeconds`, `errorId`, `refusalCode` and others.

Decide on `code` and `reason` only. Quote the message to the operator when it
helps them, but never parse it.

## Classes

| `extensions.code` | Meaning | What to do |
|---|---|---|
| `AUTHENTICATION_REQUIRED` | HTTP 401: no key, a malformed key, or an expired or revoked key | Ask the operator for a new key. Do not retry. |
| `UNAUTHORIZED` | The account behind the key lacks a permission or library grant. `reason` may say which. | Report the missing permission (see `permissions.md`). Stop. |
| `MFA_STEP_UP_REQUIRED` | Config change needs a fresh MFA check. An API key cannot provide one. | Explain that the step-up setting blocks automation. The operator decides. |
| `TOTP_ENROLLMENT_REQUIRED` | Same setting, and the account has no authenticator | Same as above |
| `MFA_ENROLLMENT_REQUIRED`, `PASSWORD_CHANGE_REQUIRED`, `REAUTHENTICATION_REQUIRED` | The account must finish a sign-in step in the web UI | Ask the operator to sign in once in the browser |
| `VALIDATION_ERROR` | Input refused. `reason` names the condition when the server classified it. | Fix the input for that reason. If `reason` is absent, show the message and ask. |
| `NOT_FOUND` | An id does not exist or is not visible | Re-read the ids from a list query. Do not guess ids. |
| `TEMPORARY_UNAVAILABLE` | A dependency is busy or rate limited. `retryAfterSeconds` may be set. | Wait that long (or about 30 s) and retry once. |
| `PLUGIN_FAILED` | A provider plugin failed. `reason` gives the kind and `detail` may be set. | Report it. The provider config usually needs attention. |
| `LOCATION_PLAN_REFUSED`, `LOCATION_ROOT_REFUSED` | File-location plan refused. `refusalCode` gives the cause. | Report it with the code. |
| `LOCATION_OPERATION_BUSY` | Another file operation holds the location | Wait for it to finish |
| `PUBLIC_URL_REJECTED` | A URL failed the safety check | Ask the operator for a reachable URL |
| `CANCELED` | The operation was cancelled | Ask before restarting it |
| `INTERNAL_ERROR` | Server fault, with `errorId` | Report the `errorId` to the operator. Do not loop. |

## HTTP-level refusals

`gql.sh` exits 2 when the body is not a GraphQL response:

- **HTTP 429**: Scryer's rate limiter answered `{"error": "..."}`, and a
  `Retry-After` header may be set. Slow down. Do not run calls in parallel.
- **HTTP 403 "web client proof"**: no key reached Scryer in authless mode.
  Check that `SCRYER_API_KEY` is exported.
- **HTML instead of JSON**: `SCRYER_URL` points at a proxy page or the wrong
  base path.

## Reasons you will meet often

| reason | Where | Fix |
|---|---|---|
| `RULE_SOURCE_INVALID` | `createRuleSet`, `updateRuleSet` | Read `extensions.diagnostics`. Fix those lines. |
| `RULE_EPISODE_REQUIRED_WHEN_TESTING_SERIES_ANIME_RULE` | `testRuleSet` | Pass an `episodeId` for a series or anime title |
| `RULE_EPISODE_CANNOT_SELECTED_WHEN_TESTING_MOVIE_RULE` | `testRuleSet` | Drop `episodeId` for a movie |
| `ARR_CUSTOM_FORMAT_JSON_INVALID`, `ARR_CUSTOM_FORMAT_FORMATS_EMPTY`, `ARR_CUSTOM_FORMAT_FORMATS_TOO_MANY`, `ARR_CUSTOM_FORMAT_JSON_TOO_LARGE` | `translateArrCustomFormats` | Send a JSON array of at most 1000 formats, under 16 MiB |
| `INTERFACE_IMPORT_MAPPING_REQUIRED`, `INTERFACE_IMPORT_MAPPING_ROOT_UNKNOWN`, `INTERFACE_IMPORT_ROOT_LIBRARY_MISMATCH`, `INTERFACE_IMPORT_LIBRARY_FACET_MISMATCH` | `finalizeExternalImport` | See the migrate skill's import reference |
| `INTERFACE_INTERACTIVE_SESSION_REQUIRED` | key management | Done in the web UI only |
