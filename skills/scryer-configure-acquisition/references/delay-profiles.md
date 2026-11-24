# Delay profiles

Needs **Manage catalog settings**. UI: **Settings > Delay profiles**
(`/settings/delay-profiles`).

```sh
sh "$G" "$S"/operations/DelayProfiles.graphql > delay.json
jq -r '.data.delayProfiles[] | "\(.id)\t\(.name)\tenabled=\(.enabled)\tpriority=\(.priority)\tusenet=\(.usenetDelayMinutes)m\ttorrent=\(.torrentDelayMinutes)m\tprefer=\(.preferredProtocol)"' delay.json
```

Fields:

- `id` (string): required; caller picks it. New profile: name lowercased,
  runs of other characters turned into `-` (as the UI does), adding `-2`, `-3`
  if taken.
- `name` (string), `enabled` (bool).
- `priority` (int): order among delay profiles; lower first.
- `usenetDelayMinutes`, `torrentDelayMinutes` (int): wait this long after a
  release appears before grabbing.
- `enableUsenet`, `enableTorrent` (bool): disabled protocol gives decision
  `protocol_disabled`.
- `preferredProtocol` (USENET or TORRENT): preferred when both delayed
  sources qualify.
- `minAgeMinutes` (int): usenet minimum age; gives `minimum_age`.
- `bypassScoreThreshold` (int or null): score that bypasses the delay.
- `bypassIfHighestQuality` (bool): highest-quality release bypasses its delay.
- `appliesToFacets` ([MOVIE, SERIES, ANIME]): which media types.
- `tags` ([string]): limit to titles with these tags.
- `libraryIds` ([id] or null): limit to libraries; `[]` = every library,
  `null` keeps the stored list.

`upsertDelayProfile` replaces the profile with that `id`. Send every field:

```sh
jq --arg id "$DID" '.data.delayProfiles[] | select(.id == $id)' delay.json > before.json
jq '.usenetDelayMinutes = 60' before.json > after.json
sh "$S"/scripts/show-change.sh before.json after.json
jq -n --slurpfile p after.json '{input: $p[0]}' > upsert.json
```

Yes first, then `sh "$G" "$S"/operations/UpsertDelayProfile.graphql upsert.json`.
Removing a delay profile: not covered; use the UI.
