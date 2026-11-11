# Areas the import leaves out

Each section reads one snapshot file, shows the operator a short plan, and
writes only after a yes. Read the snapshot with jq projections, never whole.

## Titles with no files

The scan only finds titles on disk. Arr titles with no files (`withoutFiles`
in `summary.json`) must be added.

```sh
jq -c '[.[] | select((.statistics.episodeFileCount // 0) == 0) | {title, tvdbId, monitored, seriesType}]' snap/sonarr/series.json > to-add.json
jq -c '[.[] | select(.hasFile != true) | {title, tmdbId, imdbId, monitored}]' snap/radarr/movie.json > to-add.json
```

Drop the ones Scryer already has (`TitlesByExternalIds.graphql`, `source`
`tvdb` or `tmdb`, 100 values per call). Show the count and the first ten names.
After a yes, call `AddTitle.graphql` once per title, with `facet` `SERIES`
(`ANIME` when `seriesType` is `anime`) or `MOVIE`, `libraryId`, `monitored`,
`tags: []`, and `tvdbId` (string) or `tmdbId` (integer). Write one line per
result to a file and report counts. `reusedExistingTitle: true` is not an
error.

## Quality profiles

| Arr | Scryer `criteria` field |
|---|---|
| allowed qualities (`items[].allowed`) | `qualityTiers`: resolution tiers in preference order: `4320P 2160P 1440P 1080P 1080I 720P 576P 480P 360P` |
| allowed sources inside those qualities | `sourceAllowlist` / `sourceBlocklist`: `WEB-DL WEBRip BluRay BRDISK DVD HDTV CAM TELESYNC TELECINE DVDSCR WORKPRINT` |
| `cutoff` | `cutoffTier` (a tier from `qualityTiers`) |
| `upgradeAllowed` | `allowUpgrades` |
| `minFormatScore` | `minScoreToGrab`, see below |
| `cutoffFormatScore` | `cutoffScore`, see below |
| `minUpgradeFormatScore` | `minUpgradeScoreDelta` |

Scryer compares these score fields with its **whole** release score: the
built-in score plus every rule set. Arr compares them with the custom format
sum only. Copying the numbers is therefore a starting point, not an
equivalent. Leave `minScoreToGrab` unset unless arr's was non-zero, then
check the totals for a few names with `scryer-write-rules`'s test step.

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

Show `qp-vars.json`'s profile to the operator, then `SaveQualityProfileSettings.graphql`.
Always `replaceExisting: false`. Assign it per library with `UpdateLibrary`
`settings.qualityProfileId` (send no `roots` in that call). `settings` replaces
every saved override on that library: any field left out, such as
`defaultMonitorType`, the languages or the routing, is reset to inherit. Send it
only for a library this migration created and has not otherwise configured; for
any other library, ask the operator to pick the profile in that library's
settings instead.

## Delay profiles

| Arr | Scryer `DelayProfileInput` |
|---|---|
| `usenetDelay`, `torrentDelay` (minutes) | `usenetDelayMinutes`, `torrentDelayMinutes` |
| `enableUsenet`, `enableTorrent` | same names |
| `preferredProtocol` (`usenet`/`torrent`) | `USENET` / `TORRENT` |
| `bypassIfHighestQuality` | same name |
| `bypassIfAboveCustomFormatScore` + `minimumCustomFormatScore` | `bypassScoreThreshold` (whole-score scale, as above) |
| `tags` (ids) | `tags` (labels, from `tag.json`) |
| `order` | `priority` |

Also required: `id` (any stable string, e.g. `sonarr-delay-1`), `name`,
`minAgeMinutes` (0 unless the operator wants one), `appliesToFacets`
(`SERIES`, `ANIME` for Sonarr; `MOVIE` for Radarr), `enabled`. List existing
ones with `DelayProfiles.graphql` first; upsert replaces the profile with that
`id`.

## Naming

Arr tokens are not translated. Read the arr formats:

```sh
jq -c '{standardEpisodeFormat, animeEpisodeFormat, seriesFolderFormat, seasonFolderFormat, standardMovieFormat, movieFolderFormat, renameEpisodes, renameMovies} | with_entries(select(.value != null))' snap/*/config_naming.json
```

Scryer defaults, which are often close enough to keep:

| Scope | `renameTemplate` |
|---|---|
| MOVIE | `{title} ({year}) - {quality}.{ext}` |
| SERIES | `{title} - S{season:2}E{episode:2} - {quality}.{ext}` |
| ANIME | `{title} - S{season_order:2}E{episode:2}{?absolute_episode: ({absolute_episode})}{?episode_title: - {episode_title|truncate:64}} - {quality}.{ext}` |

Folders: `folderTemplate` `{title} ({year})`, `seasonFolderTemplate`
`Season {season}`, `specialsFolderTemplate` `Specials`.

Tokens: `title`, `title_with_year`, `title_without_year`, `title_with_article`,
`title_first_character`, `year`, `quality`, `edition`, `source`,
`dynamic_range`, `revision`, `is_3d`, `video_codec`, `audio_codec`,
`audio_channels`, `group`, `ext`, `season`, `season_order`, `episode`,
`episode_title`, `absolute_episode`, and `imdb_id`, `tmdb_id`, `tvdb_id`,
`anidb_id`, `mal_id`, `anilist_id`. Some are facet-specific; the server
refuses a token the scope cannot use. Syntax: `{season:2}` zero-pads to 2
digits, `{?token:text}` emits `text` only when the token has a value,
`{token|truncate:N}` shortens, `{{` and `}}` are literal braces.

Rewrite the arr format with these, show old and new side by side, and save
with `UpdateMediaSettings.graphql` (`scope` plus only the changed fields).
Arr tokens with no Scryer equivalent are dropped; name them to the operator.

## Remote path mappings (manual)

There is no API for them alone. Show the operator the arr list:

```sh
jq -r '.[] | "\(.host): \(.remotePath) => \(.localPath)"' snap/sonarr/remotepathmapping.json
```

The operator enters them in Scryer's web UI, in the matching download
client's settings, one line per mapping: `REMOTE => LOCAL`. The local path is
where Scryer sees the files and must be absolute.

## Tags

`tag.json` holds `{id, label}`. Create each label used by titles or delay
profiles with `CreateTitleTagDefinition.graphql`, after showing the list.
Then tag titles in batches: group arr titles by tag, look up their Scryer ids
with `TitlesByExternalIds`, and call `UpdateTitleTags.graphql` with
`{input: {titleIds, add: [label]}}`.

## Notifications

`NotificationProviderTypes.graphql` lists the channel types Scryer supports,
their config fields and `supportedEvents`. Match each arr notification by
`implementation`. Arr masks secrets as `********`, so ask the operator for
each secret through an environment variable or a file and pass it as
`secretValue`; other fields go in `stringValue`, `boolValue`, `intValue` or
`floatValue` by `fieldType`. Then subscribe the channel
with `CreateNotificationSubscription.graphql`
(`{input: {channelId, eventType, scope: "global", isEnabled: true}}`), one
call per event the operator wants, chosen from that type's `supportedEvents`
(`NotificationEventTypes.graphql` lists every event). An arr type with no
Scryer match is manual.

## Sonarr release profiles

Each profile has `required` and `ignored` terms (plain words or `/regex/`)
and optional `tags`. `ignored` becomes a `scryer.block_score()` entry when a
term matches `input.release.raw_title`; `required` becomes a block when no
term matches. Write them with `scryer-write-rules`. A profile limited to tags
applies only to tagged titles, which a rule cannot see; tell the operator.

## Import lists, exclusions, auto-tagging, history (manual)

No API maps these. List them to the operator by name and implementation and
leave them in arr.
