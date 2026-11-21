# Reading rule diagnostics

## From validateRuleSet

```json
{"valid": false, "errors": ["..."],
 "diagnostics": [{"code": "RULE_UNKNOWN_INPUT_PATH", "message": "...", "line": 12, "column": 5, "path": "input.release.group"}]}
```

Use `code`, `line`, `column`, `path`. Do not parse `message` or `errors[]`
(human text). Lines count from the top of the source sent; keep the validated
source on disk:

```sh
jq -r '.data.validateRuleSet.diagnostics[] | "\(.line // "-"):\(.column // "-") \(.code) \(.path // "")"' v.json
```

`line` and `column` can be null, mainly for dry-run runtime errors; location
is then only in the message. Show the message to the operator; do not parse
it.

- `RULE_COMPILATION_ERROR`: Rego syntax or type error. Fix the line shown.
  Common: missing `if`, `=` where `:=` is meant, unbalanced braces.
- `RULE_UNKNOWN_INPUT_PATH`: `path` not a documented input field. Use a field
  from `input-fields.md`; watch for typos such as `release_grp`.
- `RULE_DYNAMIC_INPUT_PATH`: input indexed with a variable
  (`input.release[k]`). Name the field directly.
- `RULE_INPUT_IMPORT_UNSUPPORTED`: `import input.release as r` or similar.
  Write `input.` paths in full.
- `RULE_PACKAGE_REQUIRED`: no `package` line. Add
  `package scryer.rules.user.example`.
- `RULE_OUTPUT_INVALID`: `score_entry` not an object of integer values. Use
  integer deltas; wrap arithmetic in `round()`.
- `RULE_RUNTIME_ERROR`: evaluation failed on the synthetic input (invalid
  regex, conflicting `score_entry` values, a type error). Read the message,
  fix that expression.
- `RULE_TARGETS_PERSON`: rule reads person-specific facts it may not use.
  Remove those references.
- `RULE_VALIDATION_FAILED`: generic failure. Read the message.

## From createRuleSet and updateRuleSet

Saving runs the same validation. Failure is a GraphQL error:

- `extensions.code = "VALIDATION_ERROR"`, `extensions.reason = "RULE_SOURCE_INVALID"`,
  and `extensions.diagnostics[]` with the same `{code, message, line, column, path}`
  shape. `gql.sh` prints them as `line L col C <code>: <message>`.
- `reason = "RULE_SOURCE_USES_RETIRED_FIELDS"` or `"RULE_INPUT_RETIRED"`:
  source uses a removed field. Rewrite with current fields.

Nothing is saved when validation fails.

## From testRuleSet

- `errors[{code, message, ruleSetId}]`: evaluation errors per rule set,
  including the draft (`ruleSetId` null).
- `draftContribution.applies = false`: draft's `appliedFacets` exclude this
  title's facet.
- `draftContribution.matched = false` with `score = 0`: no `score_entry` rule
  fired.
- Request-level refusals come back as GraphQL errors with a `reason`, e.g.
  `RULE_EPISODE_REQUIRED_WHEN_TESTING_SERIES_ANIME_RULE`.
