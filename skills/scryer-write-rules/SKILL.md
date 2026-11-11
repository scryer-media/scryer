---
name: scryer-write-rules
description: Writes, validates, tests and saves Scryer release rules (Rego rule sets that add score deltas to release candidates) through the GraphQL API. Use it when an operator wants Scryer to prefer, penalise or block releases by group, source, codec, size, language, name pattern or any other release field, asks to "write a rule", "block X", "prefer Y", or wants to debug why a rule did or did not fire.
---

# Write Scryer release rules

Complete `scryer-connect` first. All calls below use its `gql.sh`:

```sh
C=path/to/skills/scryer-connect; R=path/to/skills/scryer-write-rules
gql() { "$C/scripts/gql.sh" "$@"; }
```

## The rule model in five lines

- A **rule set** is one Rego module plus a name, `appliedFacets`
  (`movie`, `series`, `anime`; empty means all), a `priority` and an
  `enabled` flag.
- It outputs `score_entry["<code>"] := <integer delta>`. Nothing else is read.
- Deltas from every enabled rule set add to Scryer's built-in score. A total
  at or below **-9000** rejects the release. `scryer.block_score()` is -10000.
- Rules see `input.release`, `input.profile`, `input.context`,
  `input.builtin_score` and `input.file` (null before download).
- Rule sets are global: they apply to every quality profile. Branch on
  `input.profile.id` when a score should differ per profile.

Read `references/scoring.md` before choosing numbers and
`references/input-fields.md` for field names. Do not read the schema or the
source. These two files are the contract.

## Workflow

### 1. Agree the intent

Restate the rule in one sentence with concrete numbers, for example "+150 for
group ExampleGrp, block hardcoded subs, TV only". Ask only if a number or the
facet is unclear.

### 2. Draft the source

Write it to a file such as `rule.rego`. Start from the closest example:

| Example | Shows |
|---|---|
| `examples/trusted-group-and-dub-tag.rego` | null-safe string field, a flag, `regex.match` with a raw string |
| `examples/block-hardcoded-and-ai.rego` | `scryer.block_score()` |
| `examples/episode-size-window.rego` | helper value, `scryer.size_gib()`, a null guard, TV-only facets |

Regex follows Rust syntax: no lookaround and no backreferences
(`references/regex.md`).

### 3. Validate (read-only, no confirmation needed)

```sh
jq -n --rawfile s rule.rego '{input: {regoSource: $s}}' > v-vars.json
gql "$R/operations/ValidateRuleSet.graphql" v-vars.json > v.json
jq -c '.data.validateRuleSet | {valid, d: [.diagnostics[] | [.line, .column, .code, .path]]}' v.json
```

Fix every diagnostic by `code` and `line` (`references/diagnostics.md`) and
validate again until `valid` is true.

### 4. Test against sample names (read-only)

`testRuleSet` needs a real title. A series or anime title also needs an
episode.

```sh
jq -n '{facet: "SERIES", query: "Example Show", limit: 3}' > f.json
gql "$R/operations/FindTitles.graphql" f.json > t.json
jq -c '.data.titles.items[] | {id, name, year}' t.json
# series/anime only:
jq -n --arg id "$TITLE_ID" '{id: $id}' > e.json
gql "$R/operations/TitleEpisodes.graphql" e.json > eps.json
jq -r '[.data.title.collections[].episodes[]][0].id' eps.json
```

Put 5 to 20 names in `names.txt`: ones that should match, ones that should
not, and edge cases. Use invented names in the scene format, for example
`Example.Show.S01E02.1080p.WEB-DL.DDP5.1.H.264-ExampleGrp`. A size in bytes may
follow each name after a TAB. Then:

```sh
"$R/scripts/test-names.sh" rule.rego series,anime "$TITLE_ID" "$EPISODE_ID" \
  < names.txt > results.tsv
column -t -s "$(printf '\t')" results.tsv | head -25
```

The columns are `draft_score` (this rule's contribution), `total_score`,
`allowed`, `blocked`, the draft's `code:delta` entries, and `errors`. Show the
operator the table. Explain any row where the rule fired or stayed silent
against expectation. Check the parse with `parsed` in a single
`TestRuleSet.graphql` call when a field did not hold what you expected.

### 5. Save (side effect: confirm first)

Show the operator the name, facets, enabled state and the codes with their
deltas, and wait for a yes. New rule sets should start **disabled** unless
the operator says otherwise.

```sh
jq -n --rawfile s rule.rego --arg name "Prefer ExampleGrp" \
  '{input: {name: $name, description: "", regoSource: $s,
            appliedFacets: ["series","anime"], priority: 0, enabled: false}}' > c.json
gql "$R/operations/CreateRuleSet.graphql" c.json > created.json
jq -c '.data.createRuleSet' created.json
```

A refusal with reason `RULE_SOURCE_INVALID` carries `extensions.diagnostics`.
Fix the source and repeat from step 3.

### 6. Enable, disable, edit

- Enable or disable: `ToggleRuleSet.graphql` with `{input: {id, enabled}}`.
  This is a side effect, so confirm first.
- Edit: fetch the source with `RuleSet.graphql`, change the file, validate,
  test, then `UpdateRuleSet.graphql` with `{input: {id, regoSource}}`.
  Omitted fields stay unchanged. Confirm first.
- List: `RuleSets.graphql` returns every rule set without sources. Managed
  rule sets (`isManaged: true`) come from installed rule packs. Leave them
  alone unless the operator asks.
- To test an edit of a saved rule against the saved version, send
  `editRuleSetId: <id>` with the draft. The saved copy is then left out of the
  totals.

Deleting a rule set is not part of this skill. Disable it instead. If the
operator wants it gone, they can delete it in the web UI.

## Debugging "why didn't my rule fire?"

1. Run `TestRuleSet.graphql` once with the exact release name and read
   `parsed`. The field may not hold what you assumed (`source` is `WEB-DL`,
   not `WEB`).
2. `draftContribution.applies: false` means the facets exclude this title.
3. A nullable field compared without a guard fails silently. See the
   null-safe patterns in `references/scoring.md`.
4. `input.file` rules never fire during search, only after download.
