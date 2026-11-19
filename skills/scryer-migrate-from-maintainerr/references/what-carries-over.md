# What carries over from Maintainerr

One Maintainerr rule group becomes one lifecycle rule. Tell the operator this
before the import, and the "does not carry over" list after it.

## Carries over

- **Name**: `(Maintainerr) <group name>`, or `(Maintainerr) FAILED - <group name>`
  for a group that did not translate. Control characters in the name are
  replaced by spaces; names over 200 characters are cut.
- **Description**: one sentence on where the rule came from (for a FAILED
  rule, what did not translate), then the group's own description.
- **Conditions**: the group's rules and their AND/OR sections, as Rego. Each
  condition reads the Scryer fact that matches the Maintainerr property.
- **Grace period**: the collection's "delete after days" becomes the rule's
  grace days.
- **Action**, by media type:

  | Maintainerr action | Movies, shows | Seasons | Episodes |
  |---|---|---|---|
  | Delete | delete title | unmonitor season, delete season files | unmonitor episode, delete episode files |
  | Unmonitor and delete all | unmonitor, delete files | not supported | not supported |
  | Unmonitor and delete existing | movie: delete title; show: unmonitor, delete files (warning: Scryer unmonitors the whole show) | unmonitor season, delete season files | not supported |
  | Unmonitor | unmonitor | unmonitor season | unmonitor episode |
  | Do nothing | notify (warning) | notify season | notify episode |
  | Change quality profile | set quality profile (Scryer profile of the same name), search | not supported | not supported |
  | Delete or unmonitor show if empty | not supported | not supported | not supported |

- **Safety cap**: a rule that deletes saves with at most 10 titles per run.

## Properties Scryer reads

From Plex, Jellyfin, Emby, Tautulli and Tracearr: who watched (only accounts
linked to a Scryer user, by Scryer username), last watched, view counts, genre,
date added (Scryer's first import). Per user (Tautulli, Tracearr,
Streamystats): a user's play count and last play. Jellyfin and Emby: who
favourited. From Radarr: date added, tags, monitored, in cinemas, file size,
release date (earliest digital or physical), file date (Scryer's newest file),
IMDb rating and votes. From Sonarr: date added, tags, monitored, first aired,
show size, missing episodes, status, ended, episode file rank, season and
episode number. Plex IMDb and TMDB ratings. From Seerr, Overseerr, Jellyseerr
and Ombi: request date, who requested, whether requested.

Each binding that differs from Maintainerr in a stated way adds a
`maintainerr.fact.approximate` warning with the difference. Such a binding
translates only where the difference can make the rule match fewer titles,
never more; elsewhere the group fails:

- **Lists Scryer holds only part of** (who watched, who favourited, who
  requested; Radarr and Sonarr tags, which are Scryer's own tags because the
  Sonarr and Radarr import does not carry tags over): contains, contains
  partially, contains all and count bigger translate. Not contains, equals,
  not equals, count equals, count not equals and count smaller fail
  (`maintainerr.comparator.partial_list_widens`).
- **Whether requested** (Scryer knows only requests made in Scryer): "is
  requested" translates; "is not requested" fails
  (`maintainerr.comparator.absence_widens`).
- **Dates Scryer records later**: Radarr and Sonarr date added (when the
  title was added to Scryer), the media servers' date added (Scryer's first
  import) and Radarr's file date. Before, smaller and exists translate; after, bigger,
  in the last and in the next fail (`maintainerr.comparator.date_drift_widens`).
- **Dates that can differ either way**: Radarr release date and the request
  date. Only "exists" translates.
- **Not exists** on any binding that differs fails
  (`maintainerr.comparator.absence_widens`).
- **A user's play count** for a name no Scryer user has: Scryer reads zero
  plays, so a comparison zero plays satisfy fails
  (`maintainerr.username.unknown_widens`). Others translate with
  `maintainerr.username.unknown` and match nothing for that user.

## Does not carry over

- **Library scope**: every imported rule applies to every library, not to the
  group's media-server library; such a group carries a
  `maintainerr.scope.all_libraries` warning.
- **Schedule**: rules run on Scryer's default schedule, not Maintainerr's.
- **Active state**: every rule is disabled, whether or not the group was active.
- **Collections, playlists and labels** of the media server
  (`maintainerr.property.collection_unsupported`).
- **Other apps' quality profiles**, runtime, original language, season counts,
  unaired episodes, series type, Sonarr ratings, Trakt and Rotten Tomatoes
  ratings, Plex critic, audience and user ratings, Jellyfin and Emby ratings
  (`maintainerr.property.unmapped`).
- **Exclusions**, the Maintainerr collection's members and its media-server
  collection itself.
- A group with no rules, rules turned off, or a media type Scryer does not act
  on: saved as a FAILED starter rule.
