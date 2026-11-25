# Regex in rules

`regex.match(pattern, value)` uses Rust's `regex` engine (linear time), not
PCRE or .NET.

## Not supported

- Lookahead / lookbehind (`x(?=y)`, `(?<!x)y`): two conditions,
  `regex.match("xy", t)` plus `not regex.match(...)`.
- Backreferences (`(a)\1`): spell out the repetition, or test in two steps.
- Possessive / atomic groups (`a++`, `(?>a)`): plain groups.
- Conditionals (`(?(1)a|b)`): separate rules.

Unsupported pattern = evaluation error (`RULE_RUNTIME_ERROR`, "invalid
regex"), not compile error. `validateRuleSet` catches it only when its dry run
against one synthetic release reaches that `regex.match` call; a call after a
condition false for the synthetic input goes unchecked until a real release
reaches it. So:

- put `regex.match` lines first in a rule body, or
- run `testRuleSet` with a name that satisfies the earlier conditions.

## Supported and useful

- Case-insensitive flag `(?i)`
- Word boundary `\b`, classes `\d`, `\w`, `\s`, POSIX classes `[[:alpha:]]`
- Non-capturing groups `(?:...)`, alternation, `{m,n}` counts, anchors `^`,
  `$` (and `\A`, `\z`)

## Writing patterns in Rego

Raw string in backticks; backslashes need no doubling:

```rego
regex.match(`(?i)\bremux\b`, input.release.raw_title)
```

In a double-quoted string double every backslash: `"(?i)\\bremux\\b"`.

## Habits

- Match parsed fields (`source`, `quality`, `release_group`, `is_remux`) when
  one exists. Fall back to `raw_title` only for what the parser does not
  expose.
- Separators are `.`, `-`, `_` and spaces. `\b` treats `.` and `-` as
  boundaries but not `_`; use `[._ -]` when underscores matter.
- Anchor group names to the end: `-ExampleGrp$`, not `ExampleGrp`.
- Keep patterns short. The arr importer refuses regexes over its size limit,
  and long alternations are hard to review.
