# Areas the import leaves out

Each section reads one snapshot file (jq projections, never whole), shows the
operator a short plan, writes only after a yes.

## Titles with no files

Scan only finds titles on disk. Arr titles with no files (`withoutFiles` in
`summary.json`) must be added.

```sh
jq -c '[.[] | select((.statistics.episodeFileCount // 0) == 0) | {title, tvdbId, monitored, seriesType}]' snap/sonarr/series.json > to-add.json
jq -c '[.[] | select(.hasFile != true) | {title, tmdbId, imdbId, monitored}]' snap/radarr/movie.json > to-add.json
```

Drop ones Scryer already has (`TitlesByExternalIds.graphql`, `source` `tvdb`
or `tmdb`, 100 values per call). Show count and first ten names. After a yes,
`AddTitle.graphql` once per title: `facet` `SERIES` (`ANIME` when
`seriesType` is `anime`) or `MOVIE`, `libraryId`, `monitored`, `tags: []`, and
`tvdbId` (string) or `tmdbId` (integer). One line per result to a file; report
counts. `reusedExistingTitle: true` is not an error.

## Quality profiles

Arr → Scryer `criteria` field:

- allowed qualities (`items[].allowed`) → `qualityTiers`: resolution tiers in preference order: `4320P 2160P 1440P 1080P 1080I 720P 576P 480P 360P`
- allowed sources inside those qualities → `sourceAllowlist` / `sourceBlocklist`: `WEB-DL WEBRip BluRay BRDISK DVD HDTV CAM TELESYNC TELECINE DVDSCR WORKPRINT`
- `cutoff` → `cutoffTier` (a tier from `qualityTiers`)
- `upgradeAllowed` → `allowUpgrades`
- `minFormatScore` → `minScoreToGrab`, see below
- `cutoffFormatScore` → `cutoffScore`, see below
- `minUpgradeFormatScore` → `minUpgradeScoreDelta`

Scryer compares these score fields with its **whole** release score (built-in
score plus every rule set); arr compares with the custom format sum only.
Copied numbers are a starting point, not an equivalent. Leave
`minScoreToGrab` unset unless arr's was non-zero; then check totals for a few
names with `scryer-write-rules`'s test step.

Create a profile by copying a Scryer one:

```sh
gql "$O/QualityProfileSettings.graphql" > qp.json
jq -c '.data.qualityProfileSettings.profiles[] | {id, name, tiers: .criteria.qualityTiers}' qp.json
jq --arg from "<scryer profile id>" --arg id "hd-from-sonarr" --arg name "HD (from Sonarr)" '
  .data.qualityProfileSettings.profiles[] | select(.id == $from)
  | {input: {profiles: [{id: $id, name: $name, criteria: (.criteria
      | .qualityTiers = ["1080P","720P"] | .cutoffTier = "1080P" | .allowUpgrades = true)}],
    categorySelections: [], categoryPersonaSelections: [], replaceExisting: false}}' qp.json > qp-vars.json
```

Show `qp-vars.json`'s profile to operator, then `SaveQualityProfileSettings.graphql`.
Always `replaceExisting: false`. To use it for a library, operator picks it in
that library's settings in the web UI (**Quality**). Do not send
`UpdateLibrary` `settings` for this: it replaces every saved override on the
library, including those the import applied (NFO and plexmatch writing,
permissions, scoring persona); the web UI instead sends the library's full
settings back with the one change.

## Delay profiles

Arr → Scryer `DelayProfileInput`:

- `usenetDelay`, `torrentDelay` (minutes) → `usenetDelayMinutes`, `torrentDelayMinutes`
- `enableUsenet`, `enableTorrent` → same names
- `preferredProtocol` (`usenet`/`torrent`) → `USENET` / `TORRENT`
- `bypassIfHighestQuality` → same name
- `bypassIfAboveCustomFormatScore` + `minimumCustomFormatScore` → `bypassScoreThreshold` (whole-score scale, as above)
- `tags` (ids) → `tags` (labels, from `tag.json`)
- `order` → `priority`

Also required: `id` (any stable string, e.g. `sonarr-delay-1`), `name`,
`minAgeMinutes` (0 unless operator wants one), `appliesToFacets` (`SERIES`,
`ANIME` for Sonarr; `MOVIE` for Radarr), `enabled`. List existing ones with
`DelayProfiles.graphql` first; upsert replaces the profile with that `id`.

## Naming

Arr tokens are not translated. Read arr formats:

```sh
jq -c '{standardEpisodeFormat, animeEpisodeFormat, seriesFolderFormat, seasonFolderFormat, standardMovieFormat, movieFolderFormat, renameEpisodes, renameMovies} | with_entries(select(.value != null))' snap/*/config_naming.json
```

Scryer default `renameTemplate` per scope (often close enough to keep):

- MOVIE: `{title} ({year}) - {quality}.{ext}`
- SERIES: `{title} - S{season:2}E{episode:2} - {quality}.{ext}`
- ANIME: `{title} - S{season_order:2}E{episode:2}{?absolute_episode: ({absolute_episode})}{?episode_title: - {episode_title|truncate:64}} - {quality}.{ext}`

Folders: `folderTemplate` `{title} ({year})`, `seasonFolderTemplate`
`Season {season}`, `specialsFolderTemplate` `Specials`.

Tokens: `title`, `title_with_year`, `title_without_year`, `title_with_article`,
`title_first_character`, `year`, `quality`, `edition`, `source`,
`dynamic_range`, `revision`, `is_3d`, `video_codec`, `audio_codec`,
`audio_channels`, `group`, `ext`, `season`, `season_order`, `episode`,
`episode_title`, `absolute_episode`, and `imdb_id`, `tmdb_id`, `tvdb_id`,
`anidb_id`, `mal_id`, `anilist_id`. Some are facet-specific; server refuses a
token the scope cannot use. Syntax: `{season:2}` zero-pads to 2 digits,
`{?token:text}` emits `text` only when token has a value, `{token|truncate:N}`
shortens, `{{` and `}}` are literal braces.

Rewrite the arr format with these, show old and new side by side, save with
`UpdateMediaSettings.graphql` (`scope` plus only changed fields). Arr tokens
with no Scryer equivalent are dropped; name them to the operator.

## Remote path mappings

Scryer keeps them per download client, in its `remote_path_mappings` config
key, as the web UI's download client editor does. Show operator the arr list:

```sh
jq -r '.[] | "\(.host): \(.remotePath) => \(.localPath)"' snap/sonarr/remotepathmapping.json
```

Arr keys a mapping by client host. Agree which Scryer client each belongs to
and where Scryer sees the local side (absolute path), then follow
`scryer-configure-acquisition`'s `references/download-clients.md`, **Edit a
client**: one line per mapping, `REMOTE => LOCAL`, added to the client's
existing `remote_path_mappings`.

## Tags

`tag.json` holds `{id, label}`. Read Scryer labels with
`TitleTagDefinitions.graphql`; leave out existing ones. After showing the
list, create each remaining label used by titles or delay profiles with
`CreateTitleTagDefinition.graphql`. Then tag titles in batches: group arr
titles by tag, look up Scryer ids with `TitlesByExternalIds`, call
`UpdateTitleTags.graphql` with `{input: {titleIds, add: [label]}}`.

## Notifications

`NotificationProviderTypes.graphql` lists supported channel types, their
config fields and `supportedEvents`. Match each arr notification by
`implementation`. Arr masks secrets as `********`: ask operator for each
secret via env variable or file, pass as `secretValue`; other fields go in
`stringValue`, `boolValue`, `intValue` or `floatValue` by `fieldType`. Then
subscribe the channel with `CreateNotificationSubscription.graphql`
(`{input: {channelId, eventType, scope: "global", isEnabled: true}}`), one
call per event the operator wants, from that type's `supportedEvents`
(`NotificationEventTypes.graphql` lists every event). Arr type with no Scryer
match is manual.

## Sonarr release profiles

Each has `required` and `ignored` terms (plain words or `/regex/`) and
optional `tags`. `ignored` becomes a `scryer.block_score()` entry when a term
matches `input.release.raw_title`; `required` becomes a block when no term
matches. Write them with `scryer-write-rules`. A profile limited to tags
applies only to tagged titles, which a rule cannot see; tell the operator.

## Import lists, exclusions, auto-tagging, history (manual)

No API maps these. List them to operator by name and implementation; leave
them in arr.
