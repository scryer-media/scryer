# Scoring conventions

## How a rule set scores

A rule set is one Rego module. Scryer reads only one thing from it, the
`score_entry` object: each key is a short code and each value is a score delta.

```rego
package scryer.rules.user.example
import rego.v1

score_entry["prefer_web"] := 50 if {
	input.release.source == "WEB-DL"
}
```

- Write any `package` line. Scryer rewrites it to the rule set's own package
  on save and validate, and adds `import rego.v1` if it is missing.
- Values must be integers in the 32-bit signed range. A float is rejected, so
  wrap any arithmetic in `round()`.
- Each code appears once per evaluation. Two rules that can both produce the
  same key with different values make a conflict at runtime.
- Helper rules, functions and constants are allowed, but only `score_entry`
  counts.
- Rules have no I/O. Packages are isolated, so a rule cannot read another rule
  set's output.

## How totals become decisions

1. Scryer computes its built-in score (`input.builtin_score.total`), which
   covers quality tier, source, codec, the persona and so on.
2. Every enabled rule set whose `appliedFacets` include the title's facet
   adds its `score_entry` deltas. Empty `appliedFacets` means every facet.
3. A **completed total at or below -9000** rejects the release.
4. The total must also reach the quality profile's `minScoreToGrab`. The
   profile's `cutoffScore` governs when upgrades stop.

`testRuleSet` reports all of this: `score` (the total), `allowed`, `blocked`,
`minimumScoreMet`, and per rule set `entries[{code, delta}]`.

## The block score

`scryer.block_score()` returns **-10000**. Use it for "never take this":

```rego
score_entry["no_cam"] := scryer.block_score() if {
	input.release.source == "CAM"
}
```

It is a strong penalty, not a hard veto. Large bonuses elsewhere (built-in or
from other rule sets) can pull the total back above -9000. Keep ordinary
preferences in the tens to low hundreds so that the block stays decisive.

## Suggested magnitudes

| Intent | Delta |
|---|---|
| Tie-breaker preference | 5 to 25 |
| Clear preference | 50 to 200 |
| Strong preference | 300 to 1000 |
| Strong dislike | -300 to -1000 |
| Never | `scryer.block_score()` |

Arr custom-format scores translate as they are. An arr score of 1500 stays
1500 in Scryer and can outweigh the built-in quality ordering. Review large
values with the operator.

## Facets and profiles

- `appliedFacets` takes `movie`, `series` and `anime`. Save a TV-only rule
  with `["series", "anime"]`.
- A rule set applies to every quality profile. To score per profile, branch
  on `input.profile.id` (or `input.profile.name`) inside the rule:

```rego
score_entry["remux_bonus"] := 300 if {
	input.profile.name == "Example Archive"
	input.release.is_remux
}
```

## Null-safe patterns

| Field may be null | Pattern |
|---|---|
| `size_bytes` | `is_number(input.release.size_bytes)` before `scryer.size_gib()` |
| `release_group` | `g := input.release.release_group; is_string(g); lower(g) == "x"` |
| `input.file` | `input.file != null` first |
| `extra.<key>` | `object.get(input.release.extra, "key", "")` |
