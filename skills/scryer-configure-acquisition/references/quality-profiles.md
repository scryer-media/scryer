# Quality profiles

Permission: **Manage catalog settings**. UI: **Settings > Quality profiles**
(`/settings/quality-profiles`); per-scope selection is under each media
area's settings, **Quality**.

## Read

```sh
sh "$G" "$S"/operations/QualityProfileSettings.graphql > qp.json
jq '.data.qualityProfileSettings | {globalProfileId, globalScoringPersona,
  profiles: [.profiles[] | {id, name, tiers: .criteria.qualityTiers, cutoff: .criteria.cutoffTier, upgrades: .criteria.allowUpgrades}],
  categorySelections, categoryPersonaSelections}' qp.json
```

## Criteria fields

| field | type | notes |
|---|---|---|
| `qualityTiers` | [string] | ordered, best first; only listed tiers are wanted |
| `cutoffTier` | string | stop upgrading once a file reaches this tier |
| `allowUpgrades` | bool | |
| `minUpgradeScoreDelta` | int ≥ 0 | a same-tier release must beat the current file by this much (default 120); leaving it out keeps the stored value |
| `cutoffScore` | int | stop same-tier upgrades past this score; leaving it out keeps the stored value |
| `minScoreToGrab` | int | refuse releases below this score |
| `archivalQuality` | string | |
| `allowUnknownQuality` | bool | |
| `sourceAllowlist`, `sourceBlocklist` | [string] | |
| `videoCodecAllowlist`, `videoCodecBlocklist` | [string] | |
| `audioCodecAllowlist`, `audioCodecBlocklist` | [string] | |
| `dolbyVisionAllowed`, `detectedHdrAllowed` | bool | |
| `preferRemux`, `allowBdDisk` | bool | |
| `ignoreProperRepack` | bool | |
| `scoringOverrides` | object | `allowX265Non4K`, `blockDvWithoutFallback`, `preferCompactEncodes`, `preferLosslessAudio`, `blockUpscaled` (each bool or null) |
| `decisionPipeline` | [enum] | order of QUALITY_TIER, REVISION, PREFERRED_LANGUAGE, OPTIONAL_LANGUAGES |

Use the tier, source and codec strings already present in the stored
profiles. Do not invent new ones; if the operator wants one that does not
appear, ask them to pick it in the UI.

Scoring personas: BALANCED, AUDIOPHILE, EFFICIENT, COMPATIBLE.
Release scoring beyond these fields is done with rules; see
`scryer-write-rules`.

## Save

`saveQualityProfileSettings` input:

| field | rule for this skill |
|---|---|
| `profiles` | only the profiles being created or changed, each with its full `criteria` |
| `replaceExisting` | **always `false`**: profiles not listed are kept. `true` removes every profile not listed; never send it |
| `globalProfileId` | `null` to keep the current global default |
| `globalScoringPersona` | `null` to keep |
| `categorySelections` | `[]` to keep; or `[{scope, profileId, inheritGlobal}]` for the scopes to change |
| `categoryPersonaSelections` | `[]` to keep; or `[{scope, persona, inheritGlobal}]` |

The stored `criteria` object has the same shape as the input, so edit a copy:

```sh
jq --arg id "$PID" '.data.qualityProfileSettings.profiles[] | select(.id == $id)' qp.json > before.json
jq '.criteria.cutoffTier = "<tier from qualityTiers>"' before.json > profile.json
jq -n --slurpfile p profile.json '{input: {profiles: [$p[0]], replaceExisting: false,
  globalProfileId: null, globalScoringPersona: null, categorySelections: [], categoryPersonaSelections: []}}' > save.json
sh "$S"/scripts/show-change.sh before.json profile.json
```

Show the change, wait for a yes, then
`sh "$G" "$S"/operations/SaveQualityProfileSettings.graphql save.json`.

A new profile needs a new `id` the operator agrees to (lowercase words and
hyphens, for example `example-4k`) and a full `criteria`; start from a copy
of an existing profile's criteria.

Removing a profile is not covered by this skill. Point the operator to the
UI; it refuses while a library, title, request or the global default still
uses the profile.
