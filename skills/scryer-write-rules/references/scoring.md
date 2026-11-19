# Scoring conventions

## How a rule set scores

One Rego module. Scryer reads only the `score_entry` object: key = short
code, value = score delta.

```rego
package scryer.rules.user.example
import rego.v1

score_entry["prefer_web"] := 50 if {
	input.release.source == "WEB-DL"
}
```

- Any `package` line works. Scryer rewrites it to the rule set's own package
  on save and validate, and adds `import rego.v1` if missing.
- Values must be integers in 32-bit signed range. Float rejected; wrap
  arithmetic in `round()`.
- Each code once per evaluation. Two rules that can both produce the same key
  with different values conflict at runtime.
- Helper rules, functions, constants allowed; only `score_entry` counts.
- No I/O. Packages isolated: a rule cannot read another rule set's output.

## How totals become decisions

1. Scryer computes its built-in score (`input.builtin_score.total`): quality
   tier, source, codec, persona and so on.
2. Every enabled rule set whose `appliedFacets` include the title's facet
   adds its `score_entry` deltas. Empty `appliedFacets` = every facet.
3. **Completed total at or below -9000** rejects the release.
4. Total must also reach the quality profile's `minScoreToGrab`. Profile's
   `cutoffScore` governs when upgrades stop.

`testRuleSet` reports all of it: `score` (total), `allowed`, `blocked`,
`minimumScoreMet`, per rule set `entries[{code, delta}]`.

## The block score

`scryer.block_score()` returns **-10000**. Use for "never take this":

```rego
score_entry["no_cam"] := scryer.block_score() if {
	input.release.source == "CAM"
}
```

Strong penalty, not a hard veto: large bonuses elsewhere (built-in or other
rule sets) can pull the total back above -9000. Keep ordinary preferences in
the tens to low hundreds so the block stays decisive.

## Suggested magnitudes

- Tie-breaker preference: 5 to 25
- Clear preference: 50 to 200
- Strong preference: 300 to 1000
- Strong dislike: -300 to -1000
- Never: `scryer.block_score()`

Arr custom-format scores translate as is: arr 1500 stays 1500 in Scryer and
can outweigh the built-in quality ordering. Review large values with the
operator.

## Facets and profiles

- `appliedFacets` takes `movie`, `series`, `anime`. TV-only:
  `["series", "anime"]`.
- A rule set applies to every quality profile. Per-profile scores: branch on
  `input.profile.id` (or `input.profile.name`) inside the rule:

```rego
score_entry["remux_bonus"] := 300 if {
	input.profile.name == "Example Archive"
	input.release.is_remux
}
```

## Null-safe patterns

- `size_bytes`: `is_number(input.release.size_bytes)` before
  `scryer.size_gib()`
- `release_group`: `g := input.release.release_group; is_string(g); lower(g) == "x"`
- `input.file`: `input.file != null` first
- `extra.<key>`: `object.get(input.release.extra, "key", "")`
