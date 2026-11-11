# What moves, and how

Tell the operator this table before any change. Fill the counts in from
`summary.json`.

## Moved by Scryer's built-in import

| Item | Notes |
|---|---|
| Titles that have files on disk | The library scan finds them. The import passes arr's monitored state and scan hints so the scan matches them to the right title. |
| Monitored state | Applied to titles the scan finds. |
| Root folders | Each arr root folder (and each folder titles actually live in) is mapped to a Scryer library root. Paths may differ when Scryer runs in a container. |
| Library settings | Rename on or off, NFO and plexmatch writing, Linux permissions (chmod, chown group), specials monitoring, and the default quality profile when it matches one Scryer profile without ambiguity. |
| Download clients | Created in Scryer for the ones the operator selects. Arr masks secrets as `********`, so a client whose password or API key is needed asks for it again (`requiresPasswordOverride`, `apiKeyPresent: false`). |
| Indexers | Same as download clients. With Prowlarr connected, its indexers are offered instead. |

## Handled by this skill (each step asks first)

| Item | How |
|---|---|
| Custom formats and their scores | Server translator into one rule set per decision (`custom-formats.md`) |
| Titles with no files | `AddTitle`, in a confirmed batch (`manual-areas.md`) |
| Quality profiles | Copy a Scryer profile and set tiers, cutoff and score floors (`manual-areas.md`) |
| Delay profiles | `UpsertDelayProfile` |
| Naming templates | `UpdateMediaSettings`, rewritten in Scryer's template syntax |
| Tags | `CreateTitleTagDefinition`, then `UpdateTitleTags` |
| Notifications | `CreateNotificationChannel` plus one subscription per event type |
| Sonarr release profiles | Rewritten as a rule set with `scryer-write-rules` |

## Manual or unsupported

| Item | Why |
|---|---|
| Remote path mappings | No dedicated API. The operator enters them in the download client's settings in the web UI (`manual-areas.md`). |
| Import lists | No arr equivalent to map to automatically. The operator sets up Scryer lists in the web UI. |
| Import list exclusions (Radarr) | No API to import them. |
| Auto-tagging rules | No equivalent. |
| History and blocklist | Not imported. Arr keeps its own. |
| Quality definitions (size limits per quality) | Scryer scores sizes itself. Use a rule (`scryer-write-rules`) for hard size limits. |
| Arr naming tokens | Not translated by the import. This skill rewrites them by hand. |

## Running the import after setup

The import API works on an instance that has finished first-run setup. The web
UI offers it only inside the setup wizard. Do not call `completeSetup`: it is
for first run only. Finalize maps arr roots onto libraries that already exist,
so create the libraries or add the roots first (`built-in-import.md`, step 5).
This skill only adds roots. It never removes one.
