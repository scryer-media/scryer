# Custom formats to rule sets

Primary path: Scryer connects to Sonarr or Radarr itself, reads every custom
format and every quality profile's scores, and saves **one disabled scoring
rule set per custom format**. Runs as a background job (job key
`EXTERNAL_RULE_IMPORT`). Needs `MANAGE_CATALOG_SETTINGS`. Once per arr app
(`SONARR` or `RADARR`). Use the paste-JSON fallback at the end only when
Scryer cannot reach the arr instance.

## 1. Start the import (side effect: confirm first)

Saves rules (all disabled). Tell the operator how many custom formats the
snapshot holds (`summary.json`) and that each becomes a disabled rule; wait
for a yes. `requestKey` is any unique string you keep: resending the same key
and input after a lost response returns the same run instead of starting a
second one. The API key is read from the environment, never printed.

```sh
jq -n --arg key "cf-sonarr-$(date +%s)" --arg url "$SONARR_URL" \
  --arg api "$SONARR_API_KEY" \
  '{requestKey: $key, input: {source: "SONARR", baseUrl: $url, apiKey: $api}}' > rule-import-vars.json
chmod 600 rule-import-vars.json
gql "$O/StartExternalRuleImport.graphql" rule-import-vars.json > rule-import-run.json
jq -c .data.startExternalRuleImport rule-import-run.json
```

`baseUrl` is the arr URL as Scryer reaches it (step b of `SKILL.md`); pass
that URL instead of `$SONARR_URL` when the two differ. Scryer
keeps the key only until the run ends; the report never contains it. Delete
`rule-import-vars.json` once the run has settled.

## 2. Wait for the run to settle (read-only)

```sh
jq '{runId: .data.startExternalRuleImport.id}' rule-import-run.json > rule-import-status-vars.json
PROGRESS_JQ='.data.externalRuleImportStatus | "\(.phase) \(.counts.total)"' \
  poll "$O/ExternalRuleImportStatus.graphql" rule-import-status-vars.json \
  '.data.externalRuleImportStatus.active == false' 5 > rule-import.json
jq -c '.data.externalRuleImportStatus | {status, reason, counts}' rule-import.json
```

Phases: `QUEUED`, `FETCHING`, `PLANNING`, `SAVING`, `FINISHED`. Run `status`
is `COMPLETED` when every item is `SAVED` or `SKIPPED_EXISTS`, `WARNING` when
some item needs review or failed, `FAILED` when the run stopped short. A
non-null run `reason` means nothing more was saved; look it up in
`scryer-connect/references/errors.md` (for example
`EXTERNAL_IMPORT_SOURCE_UNAUTHORIZED`: wrong API key;
`EXTERNAL_IMPORT_SOURCE_UNREACHABLE`: Scryer cannot reach that URL). Only
the account that started the run can read it; any other gets
`RULE_IMPORT_NOT_OWNED`.

## 3. Act on each item (read-only report)

```sh
jq -r '.data.externalRuleImportStatus.items[] | [.state, (.savedName // .sourceName), (.reason // "")] | @tsv' rule-import.json
```

Names: `(Sonarr) <format name>` or `(Radarr) <format name>`; a format that
needs review is `(Sonarr) REVIEW - <format name>` /
`(Radarr) REVIEW - <format name>`. Facets: Radarr rules apply to `movie`,
Sonarr rules to `series` and `anime`. Description says which app and version
they came from. Every saved rule is **disabled**.

Score, from the arr quality profiles (a profile that does not list the format
scores it 0):

- every profile gives the same score (or there is one profile): that literal
  score;
- profiles disagree and every arr profile name exactly matches a Scryer
  quality profile name (case and spaces count): the rule scores by
  `input.profile.name` from a per-profile table; a Scryer profile not in the
  table, or one the arr profiles scored 0, gets no score entry at all;
- profiles disagree and some arr profile has no Scryer profile of that name:
  a REVIEW rule.

By `state`:

- `SAVED`: translated exactly. Facts may add `RULE_IMPORT_PROFILE_SCORES_APPLIED`
  (scores by profile name; `details.scores` lists `{profile, score}`) or
  `RULE_IMPORT_FORMAT_UNSCORED` (every profile scored 0, so the rule scores 0;
  it still matches, harmlessly).
- `SAVED_NEEDS_REVIEW`, reason `RULE_IMPORT_PROFILE_SCORES_UNMATCHED`: the
  active rule scores 0. Below a `REVIEW:` comment the rule's source carries the
  per-profile table and lookup rule commented out (`# ...` lines). The fact's
  `details.unmatchedProfiles` names the arr profiles with no Scryer match and
  `details.scores` every profile's score. Show both to the operator. Ask how
  those profiles map to Scryer profiles (rename one side, or edit the table
  names); then, with a yes, edit the rule with `scryer-write-rules`: uncomment
  the block, fix its names, and delete the `:= 0` rule it replaces.
- `SKIPPED_EXISTS`, reason `RULE_IMPORT_ALREADY_EXISTS`: a rule with that
  exact name already exists (a re-run, or the same name twice in one source).
  Nothing was written; the existing rule was not touched. To re-import one
  format, the operator renames or deletes the old rule themselves.
- `FAILED`: nothing was saved, no placeholder rule. `RULE_IMPORT_FORMAT_UNTRANSLATABLE`:
  a specification Scryer cannot translate (facts carry the translator's
  diagnostic `code`, `message`, `details.location` and `details.severity`, for
  example `arr.year.unmapped` or `regex.backreference_unsupported`).
  `RULE_IMPORT_FORMAT_UNREADABLE`: the format itself could not be read.
  List these formats to the operator by name; a hand-written rule
  (`scryer-write-rules`) is the only way to carry them over.

`SAVED_FAILED_TRANSLATION` is never produced by this import.

```sh
jq -c '.data.externalRuleImportStatus.items[] | select(.state == "SAVED_NEEDS_REVIEW")
  | {savedName, unmatched: (.facts[] | select(.code == "RULE_IMPORT_PROFILE_SCORES_UNMATCHED") | .details.unmatchedProfiles)}' rule-import.json
jq -c '.data.externalRuleImportStatus.items[] | select(.state == "FAILED")
  | {sourceName, reason, codes: [.facts[].code]}' rule-import.json
```

## 4. Replay, review, then enable (side effects: confirm each)

Per saved rule: read its source with `scryer-write-rules`'s
`RuleSet.graphql` (`ruleSetId` from the report), write it to a file and replay arr history against it as in step F5
(`replay-history.sh`), passing the facets above. Report mismatches only.
Enable a rule with `ToggleRuleSet.graphql` (`{"input": {"id": "<ruleSetId>",
"enabled": true}}`) only
after the operator agrees, one confirmed batch at a time; never enable a REVIEW
rule before its table is fixed.

Arr scores and Scryer's built-in score add up. A format scored -10000 in arr
rejects in Scryer too (block line: total of -9000 or lower). Large positive
arr scores can outweigh Scryer's own quality preferences; mention it when
scores above 1000 appear.

## Fallback: paste JSON into one rule set

Only when Scryer cannot reach arr. The server translates the snapshot's custom
formats into one Rego rule set; nothing saved until `CreateRuleSet`. `snap`
is the snapshot directory. The web UI's **Paste JSON** tab of the import
dialog does the same.

### F1. Review pass (read-only)

```sh
jq -n --arg app SONARR --rawfile cf snap/sonarr/customformat.json \
  '{input: {sourceApplication: $app, customFormatJson: $cf, scores: []}}' > cf-review-vars.json
gql "$O/TranslateArrCustomFormatsReview.graphql" cf-review-vars.json > cf-review.json
jq -c '.data.translateArrCustomFormats | {facets: .appliedFacets,
  problems: [.formats[] | {name, d: [.diagnostics[] | select(.code != "arr.score.invalid") | .code]} | select(.d | length > 0)]}' cf-review.json
```

With no scores every format shows `arr.score.invalid`; ignore it here. Any
other diagnostic (e.g. `regex.backreference_unsupported`,
`regex.re2_incompatible`) means a specification could not be translated; that
format becomes a disabled stub that never matches. List those formats to the
operator by name.

Request refusals: `ARR_CUSTOM_FORMAT_JSON_INVALID`, `ARR_CUSTOM_FORMAT_JSON_TOO_LARGE`
(16 MiB), `ARR_CUSTOM_FORMAT_FORMATS_EMPTY`, `ARR_CUSTOM_FORMAT_FORMATS_TOO_MANY`
(1000).

### F2. Score decision (ask the operator)

Arr scores live on quality profiles. Translator, like the web UI's **Import
Arr custom formats** dialog in rules settings, takes one score per format and
returns one rule set applying to every Scryer profile. Show
`qualityProfiles.distinctScoreSets` from `summary.json`, then by its value:

- 0: nothing scores; skip custom formats.
- 1: one rule set with those scores.
- more: one rule set from the arr profile the operator names. Tell them other
  profiles' scores are not carried over: translator has no per-profile scores,
  and this skill does not edit the rules it returns.

Write down which arr profile id feeds the rule set.

### F3. Build score list

Use format ids from the review pass. Each starts with
`[<index in the pasted array>]`, pointing back at the arr format. Scores come
from the chosen arr profile (`pid` = its arr id). A format the profile does
not list scores 0.

```sh
jq --argjson pid 4 --slurpfile p snap/sonarr/qualityprofile.json \
   --slurpfile cf snap/sonarr/customformat.json '
  ($p[0][] | select(.id == $pid) | .formatItems
    | map({key: (.format | tostring), value: .score}) | from_entries) as $s
  | [.data.translateArrCustomFormats.formats[]
     | (.id | capture("^\\[(?<i>[0-9]+)\\]").i | tonumber) as $i
     | {formatId: .id, score: ($s[($cf[0][$i].id | tostring)] // 0)}]
' cf-review.json > scores.json
jq '[.[] | select(.score != 0)] | length' scores.json
```

### F4. Translate with scores (read-only)

```sh
jq -n --arg app SONARR --rawfile cf snap/sonarr/customformat.json --slurpfile s scores.json \
  '{input: {sourceApplication: $app, customFormatJson: $cf, scores: $s[0]}}' > cf-vars.json
gql "$O/TranslateArrCustomFormats.graphql" cf-vars.json > cf.json
jq -r '.data.translateArrCustomFormats.regoSource' cf.json > arr-formats.rego
jq -c '.data.translateArrCustomFormats | {appliedFacets, translated: [.formats[] | select(.status == "TRANSLATED")] | length,
  disabled: [.formats[] | select(.status == "DISABLED") | .name]}' cf.json
```

Validate with `ValidateRuleSet.graphql` (same variables as in
`scryer-write-rules`). Do not edit the translated rules.

### F5. Replay arr history (read-only)

```sh
"$M/scripts/replay-history.sh" snap/sonarr arr-formats.rego series,anime 4 > replay.tsv
awk -F '\t' '$1 == "mismatch"' replay.tsv | head -20
```

Last argument is the arr profile id: only titles that used those scores are
replayed. Report summary line and mismatches only. Expected mismatch causes:

- arr recomputes `customFormatScore` with today's formats and the title's
  current profile, so history can drift from what was grabbed;
- Scryer parses the name differently (check `parsed` with
  `scryer-write-rules`'s `TestRuleSet.graphql`);
- format is a disabled stub (step F1);
- format used a field arr knows only after download (size, indexer flags) and
  the history record lacks it.

`skipped` rows: titles not yet in Scryer (run library scan first) or episodes
Scryer numbers differently.

### F6. Save disabled, then enable (side effects: confirm each)

Save with `CreateRuleSet.graphql`: translation's `name`, `description`,
`regoSource` and `appliedFacets` (web dialog fills its draft the same way;
rename only if operator asks), `enabled: false`. If anything changed, replay
again against the saved draft; then enable with `ToggleRuleSet.graphql` once
operator agrees.

Arr scores and Scryer's built-in score add up. A format scored -10000 in arr
rejects in Scryer too (block line: total of -9000 or lower). Large positive
arr scores can outweigh Scryer's own quality preferences; mention it when
scores above 1000 appear.
