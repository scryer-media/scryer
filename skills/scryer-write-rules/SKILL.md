---
name: scryer-write-rules
description: Writes, validates, tests and saves Scryer release rules (Rego rule sets that add score deltas to release candidates) through the GraphQL API. Use it when an operator wants Scryer to prefer, penalise or block releases by group, source, codec, size, language, name pattern or any other release field, asks to "write a rule", "block X", "prefer Y", or wants to debug why a rule did or did not fire.
---

# Write Scryer release rules

Complete `scryer-connect` first. All calls use its `gql.sh`:

```sh
C=path/to/skills/scryer-connect; R=path/to/skills/scryer-write-rules
gql() { "$C/scripts/gql.sh" "$@"; }
```

## Rule model

- **Rule set** = one Rego module plus name, `appliedFacets` (`movie`,
  `series`, `anime`; empty = all), `priority`, `enabled` flag.
- Outputs `score_entry["<code>"] := <integer delta>`. Nothing else is read.
- Deltas from every enabled rule set add to Scryer's built-in score. Total at
  or below **-9000** rejects the release. `scryer.block_score()` is -10000.
- Rules see `input.release`, `input.profile`, `input.context`,
  `input.builtin_score`, `input.file` (null before download).
- Rule sets are global: apply to every quality profile. Branch on
  `input.profile.id` for per-profile scores.

Read `references/scoring.md` before choosing numbers,
`references/input-fields.md` for field names. These two files are the
contract; do not read the schema or source.

## Workflow

### 1. Agree the intent

Restate in one sentence with concrete numbers, e.g. "+150 for group
ExampleGrp, block hardcoded subs, TV only". Ask only if a number or the facet
is unclear.

### 2. Draft the source

Write to a file such as `rule.rego`. Start from the closest example:

- `examples/trusted-group-and-dub-tag.rego`: null-safe string field, a flag,
  `regex.match` with a raw string
- `examples/block-hardcoded-and-ai.rego`: `scryer.block_score()`
- `examples/episode-size-window.rego`: helper value, `scryer.size_gib()`, a
  null guard, TV-only facets

Regex is Rust syntax: no lookaround, no backreferences (`references/regex.md`).

### 3. Validate (read-only, runs directly)

```sh
jq -n --rawfile s rule.rego '{input: {regoSource: $s}}' > v-vars.json
gql "$R/operations/ValidateRuleSet.graphql" v-vars.json > v.json
jq -c '.data.validateRuleSet | {valid, d: [.diagnostics[] | [.line, .column, .code, .path]]}' v.json
```

Fix every diagnostic by `code` and `line` (`references/diagnostics.md`);
re-validate until `valid` is true.

### 4. Test against sample names (read-only)

`testRuleSet` needs a real title; series or anime also needs an episode.

```sh
jq -n '{facet: "SERIES", query: "Example Show", limit: 3}' > f.json
gql "$R/operations/FindTitles.graphql" f.json > t.json
jq -c '.data.titles.items[] | {id, name, year}' t.json
# series/anime only:
jq -n --arg id "$TITLE_ID" '{id: $id}' > e.json
gql "$R/operations/TitleEpisodes.graphql" e.json > eps.json
jq -r '[.data.title.collections[].episodes[]][0].id' eps.json
```

Put 5 to 20 names in `names.txt`: should match, should not, edge cases.
Invented names in scene format, e.g.
`Example.Show.S01E02.1080p.WEB-DL.DDP5.1.H.264-ExampleGrp`. Optional size in
bytes after a TAB. Then:

```sh
"$R/scripts/test-names.sh" rule.rego series,anime "$TITLE_ID" "$EPISODE_ID" \
  < names.txt > results.tsv
column -t -s "$(printf '\t')" results.tsv | head -25
```

Columns: `draft_score` (this rule's contribution), `total_score`, `allowed`,
`blocked`, draft's `code:delta` entries, `errors`. Show the operator the
table. Explain any row where the rule fired or stayed silent against
expectation. When a field did not hold what you expected, check `parsed` in a
single `TestRuleSet.graphql` call.

### 5. Save (side effect: confirm first)

Show name, facets, enabled state and codes with deltas; wait for a yes. New
rule sets start **disabled** unless the operator says otherwise.

```sh
jq -n --rawfile s rule.rego --arg name "Prefer ExampleGrp" \
  '{input: {name: $name, description: "", regoSource: $s,
            appliedFacets: ["series","anime"], priority: 0, enabled: false}}' > c.json
gql "$R/operations/CreateRuleSet.graphql" c.json > created.json
jq -c '.data.createRuleSet' created.json
```

Refusal `RULE_SOURCE_INVALID` carries `extensions.diagnostics`. Fix source,
repeat from step 3.

### 6. Enable, disable, edit

- Enable/disable: `ToggleRuleSet.graphql` with `{input: {id, enabled}}`.
  Side effect; confirm first.
- Edit: fetch source with `RuleSet.graphql`, change the file, validate, test,
  then `UpdateRuleSet.graphql` with `{input: {id, regoSource}}`. Omitted
  fields unchanged. Confirm first.
- List: `RuleSets.graphql` returns every rule set without sources. Managed
  rule sets (`isManaged: true`) come from installed rule packs; leave them
  alone unless the operator asks.
- Test an edit of a saved rule against the saved version: send
  `editRuleSetId: <id>` with the draft; saved copy is then left out of the
  totals.

Deleting a rule set is not part of this skill; disable it instead. Operator
can delete it in the web UI.

## Debugging "why didn't my rule fire?"

1. Run `TestRuleSet.graphql` once with the exact release name; read
   `parsed`. Field may not hold what you assumed (`source` is `WEB-DL`, not
   `WEB`).
2. `draftContribution.applies: false`: facets exclude this title.
3. Nullable field compared without a guard fails silently. See null-safe
   patterns in `references/scoring.md`.
4. `input.file` rules never fire during search, only after download.
