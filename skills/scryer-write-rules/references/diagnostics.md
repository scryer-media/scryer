# Reading rule diagnostics

## From validateRuleSet

```json
{"valid": false, "errors": ["..."],
 "diagnostics": [{"code": "RULE_UNKNOWN_INPUT_PATH", "message": "...", "line": 12, "column": 5, "path": "input.release.group"}]}
```

Use `code`, `line`, `column` and `path`. Do not parse `message` or
`errors[]`, which are human text. Lines count from the top of the source you
sent, so keep the source you validated on disk:

```sh
jq -r '.data.validateRuleSet.diagnostics[] | "\(.line // "-"):\(.column // "-") \(.code) \(.path // "")"' v.json
```

`line` and `column` can be null, mainly for runtime errors from the dry run.
The location then appears only in the message. Show the message to the
operator rather than parsing it.

| code | Cause | Fix |
|---|---|---|
| `RULE_COMPILATION_ERROR` | Rego syntax or type error | Fix the line shown. Common causes: missing `if`, `=` where `:=` is meant, unbalanced braces. |
| `RULE_UNKNOWN_INPUT_PATH` | `path` is not a documented input field | Use a field from `input-fields.md`. Watch for typos such as `release_grp`. |
| `RULE_DYNAMIC_INPUT_PATH` | Input indexed with a variable (`input.release[k]`) | Name the field directly |
| `RULE_INPUT_IMPORT_UNSUPPORTED` | `import input.release as r` or similar | Refer to `input.` paths in full |
| `RULE_PACKAGE_REQUIRED` | No `package` line | Add `package scryer.rules.user.example` |
| `RULE_OUTPUT_INVALID` | `score_entry` is not an object of integer values | Use integer deltas. Wrap arithmetic in `round()`. |
| `RULE_RUNTIME_ERROR` | Evaluation failed on the synthetic input: invalid regex, conflicting `score_entry` values, a type error | Read the message and fix that expression |
| `RULE_TARGETS_PERSON` | The rule reads person-specific facts it may not use | Remove those references |
| `RULE_VALIDATION_FAILED` | Generic failure | Read the message |

## From createRuleSet and updateRuleSet

Saving runs the same validation. A failure is a GraphQL error:

- `extensions.code = "VALIDATION_ERROR"`, `extensions.reason = "RULE_SOURCE_INVALID"`,
  and `extensions.diagnostics[]` with the same `{code, message, line, column, path}`
  shape. `gql.sh` prints them as `line L col C <code>: <message>`.
- `reason = "RULE_SOURCE_USES_RETIRED_FIELDS"` or `"RULE_INPUT_RETIRED"`: the
  source uses a field that was removed. Rewrite with current fields.

Nothing is saved when validation fails.

## From testRuleSet

- `errors[{code, message, ruleSetId}]` lists evaluation errors per rule set,
  including the draft (whose `ruleSetId` is null).
- `draftContribution.applies = false` means the draft's `appliedFacets`
  exclude this title's facet.
- `draftContribution.matched = false` with `score = 0` means no
  `score_entry` rule fired.
- Request-level refusals come back as GraphQL errors with a `reason`, for
  example `RULE_EPISODE_REQUIRED_WHEN_TESTING_SERIES_ANIME_RULE`.
