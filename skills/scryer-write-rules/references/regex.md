# Regex in rules

`regex.match(pattern, value)` uses Rust's `regex` engine, not PCRE or .NET.
The engine guarantees linear-time matching, and in exchange some familiar
syntax is missing.

## Not supported

| Construct | Example | Use instead |
|---|---|---|
| Lookahead / lookbehind | `x(?=y)`, `(?<!x)y` | Two conditions: `regex.match("xy", t)` plus `not regex.match(...)` |
| Backreferences | `(a)\1` | Spell out the repetition, or test in two steps |
| Possessive / atomic groups | `a++`, `(?>a)` | Plain groups |
| Conditionals | `(?(1)a|b)` | Separate rules |

An unsupported pattern is an evaluation error (`RULE_RUNTIME_ERROR`, "invalid
regex"), not a compile error. `validateRuleSet` catches it only when its dry run
against one synthetic release reaches that `regex.match` call. A call placed
after a condition that is false for the synthetic input goes unchecked until a
real release reaches it. So:

- put `regex.match` lines first in a rule body, or
- run `testRuleSet` with a name that satisfies the earlier conditions.

## Supported and useful

- Case-insensitive flag: `(?i)`
- Word boundary `\b`, classes `\d`, `\w`, `\s`, POSIX classes `[[:alpha:]]`
- Non-capturing groups `(?:...)`, alternation, `{m,n}` counts, anchors `^`, `$`
  (and `\A`, `\z`)

## Writing patterns in Rego

Write a pattern as a raw string in backticks, so that backslashes need no
doubling:

```rego
regex.match(`(?i)\bremux\b`, input.release.raw_title)
```

In a double-quoted string every backslash must be doubled:
`"(?i)\\bremux\\b"`.

## Habits

- Match on the parsed fields (`source`, `quality`, `release_group`,
  `is_remux`) when one exists. Fall back to `raw_title` only for what the
  parser does not expose.
- Release names use `.`, `-`, `_` and spaces as separators. `\b` treats `.`
  and `-` as boundaries but not `_`, so use `[._ -]` when underscores matter.
- Anchor group names to the end: `-ExampleGrp$` rather than `ExampleGrp`.
- Keep patterns short. The arr importer refuses regexes over its size limit,
  and long alternations are hard to review.
