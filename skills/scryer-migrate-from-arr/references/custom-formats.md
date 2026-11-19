# Custom formats to a rule set

Server translates arr custom formats into one Rego rule set. Nothing saved
until `CreateRuleSet`. Once per arr app (`SONARR` or `RADARR`). `snap` is the
snapshot directory.

## 1. Review pass (read-only)

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

## 2. Score decision (ask the operator)

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

## 3. Build score list

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

## 4. Translate with scores (read-only)

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

## 5. Replay arr history (read-only)

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
- format is a disabled stub (step 1);
- format used a field arr knows only after download (size, indexer flags) and
  the history record lacks it.

`skipped` rows: titles not yet in Scryer (run library scan first) or episodes
Scryer numbers differently.

## 6. Save disabled, then enable (side effects: confirm each)

Save with `CreateRuleSet.graphql`: translation's `name`, `description`,
`regoSource` and `appliedFacets` (web dialog fills its draft the same way;
rename only if operator asks), `enabled: false`. If anything changed, replay
again against the saved draft; then enable with `ToggleRuleSet.graphql` once
operator agrees.

Arr scores and Scryer's built-in score add up. A format scored -10000 in arr
rejects in Scryer too (block line: total of -9000 or lower). Large positive
arr scores can outweigh Scryer's own quality preferences; mention it when
scores above 1000 appear.
