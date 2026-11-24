# Quality profiles

Needs **Manage catalog settings**. UI: **Settings > Quality profiles**
(`/settings/quality-profiles`); per-scope selection under each media area's
settings, **Quality**.

## Read

```sh
sh "$G" "$S"/operations/QualityProfileSettings.graphql > qp.json
jq '.data.qualityProfileSettings | {globalProfileId, globalScoringPersona,
  profiles: [.profiles[] | {id, name, tiers: .criteria.qualityTiers, cutoff: .criteria.cutoffTier, upgrades: .criteria.allowUpgrades}],
  categorySelections, categoryPersonaSelections}' qp.json
```

## Criteria fields

"Omit keeps" = leaving the field out keeps the stored value.

- `qualityTiers` ([string]): ordered, best first; only listed tiers wanted.
- `cutoffTier` (string): stop upgrading once a file reaches this tier. Null
  means the first tier in `qualityTiers`: searches upgrade a file until it
  reaches that tier, and after that only a same-tier release that clears
  `minUpgradeScoreDelta` replaces it.
- `allowUpgrades` (bool).
- `minUpgradeScoreDelta` (int ≥ 0): same-tier release must beat the current
  file by this much (default 120); omit keeps.
- `minMissingEpisodesForPack` (int ≥ 1): season pack searched only when at
  least this many of the season's episodes are missing, and the title's
  multi-season pack lookup only when at least this many of the series'
  episodes are (default 2); at 1 an automatic search for one missing episode
  may also take a pack; omit keeps.
- `minMissingPercentForSeriesPack` (int 0–100): multi-season pack taken only
  when more than this percent of the series is missing (default 30); omit
  keeps.
- `cutoffScore` (int): stop same-tier upgrades past this score; omit keeps.
- `minScoreToGrab` (int): refuse releases below this score.
- `archivalQuality` (string), `allowUnknownQuality` (bool).
- `sourceAllowlist`, `sourceBlocklist`, `videoCodecAllowlist`,
  `videoCodecBlocklist`, `audioCodecAllowlist`, `audioCodecBlocklist`
  ([string]).
- `dolbyVisionAllowed`, `detectedHdrAllowed`, `preferRemux`, `allowBdDisk`,
  `ignoreProperRepack` (bool).
- `scoringOverrides` (object): `allowX265Non4K`, `blockDvWithoutFallback`,
  `preferCompactEncodes`, `preferLosslessAudio`, `blockUpscaled` (each bool or
  null).
- `decisionPipeline` ([enum]): order of QUALITY_TIER, REVISION,
  PREFERRED_LANGUAGE, OPTIONAL_LANGUAGES.

Use tier, source and codec strings already present in stored profiles. Do
not invent new ones; if the operator wants one not present, ask them to pick
it in the UI.

Scoring personas: BALANCED, AUDIOPHILE, EFFICIENT, COMPATIBLE. Release
scoring beyond these fields: rules, `scryer-write-rules`.

## Save

`saveQualityProfileSettings` input, rules for this skill:

- `profiles`: only profiles being created or changed, each with full
  `criteria`.
- `replaceExisting`: **always `false`** (unlisted profiles kept). `true`
  removes every unlisted profile; never send it.
- `globalProfileId`: `null` keeps the current global default.
- `globalScoringPersona`: `null` keeps.
- `categorySelections`: `[]` keeps; or `[{scope, profileId, inheritGlobal}]`
  for scopes to change.
- `categoryPersonaSelections`: `[]` keeps; or `[{scope, persona, inheritGlobal}]`.

Stored `criteria` has the input's shape; edit a copy:

```sh
jq --arg id "$PID" '.data.qualityProfileSettings.profiles[] | select(.id == $id)' qp.json > before.json
jq '.criteria.cutoffTier = "<tier from qualityTiers>"' before.json > profile.json
jq -n --slurpfile p profile.json '{input: {profiles: [$p[0]], replaceExisting: false,
  globalProfileId: null, globalScoringPersona: null, categorySelections: [], categoryPersonaSelections: []}}' > save.json
sh "$S"/scripts/show-change.sh before.json profile.json
```

Show the change, wait for a yes, then
`sh "$G" "$S"/operations/SaveQualityProfileSettings.graphql save.json`.

New profile: new `id` the operator agrees to (lowercase words and hyphens,
e.g. `example-4k`) and full `criteria`; start from a copy of an existing
profile's criteria.

Removing a profile: not covered. Point the operator to the UI; it refuses
while a library, title, request or the global default still uses the
profile.
