# What moves, and how

Tell the operator this before any change, counts filled from `summary.json`.

## Moved by the setup wizard's import (step d)

- Titles with files on disk: library scan finds them. Import passes arr's monitored state and scan hints so the scan matches them to the right title.
- Monitored state: applied to titles the scan finds.
- Root folders: each arr root folder (and each folder titles actually live in) maps to a Scryer library root. Paths may differ when Scryer runs in a container.
- Libraries: created per facet where none exists, each with the quality profile and scoring persona chosen in the wizard's Quality step. Existing library only gains roots.
- Library settings: rename on or off, NFO and plexmatch writing, Linux permissions (chmod, chown group), specials monitoring, and the default quality profile when it matches one Scryer profile without ambiguity.
- Download clients: created in Scryer for the ones the operator selects. Arr masks secrets as `********`, so a client whose password or API key is needed asks for it again (`requiresPasswordOverride`, `apiKeyPresent: false`).
- Indexers: same as download clients. With Prowlarr connected, its indexers are offered instead.

## Handled by this skill (each step asks first)

- Custom formats and their scores: server translator into one rule set per decision (`custom-formats.md`)
- Titles with no files: `AddTitle`, in a confirmed batch (`manual-areas.md`)
- Quality profiles: copy a Scryer profile, set tiers, cutoff and score floors (`manual-areas.md`)
- Delay profiles: `UpsertDelayProfile`
- Naming templates: `UpdateMediaSettings`, rewritten in Scryer's template syntax
- Tags: `CreateTitleTagDefinition`, then `UpdateTitleTags`
- Notifications: `CreateNotificationChannel` plus one subscription per event type
- Sonarr release profiles: rewritten as a rule set with `scryer-write-rules`
- Remote path mappings: download client's `remote_path_mappings`, through `scryer-configure-acquisition`

## Manual or unsupported

- Import lists: no arr equivalent to map to automatically. Operator sets up Scryer lists in the web UI.
- Import list exclusions (Radarr): no API to import them.
- Auto-tagging rules: no equivalent.
- History and blocklist: not imported. Arr keeps its own.
- Quality definitions (size limits per quality): Scryer scores sizes itself. Use a rule (`scryer-write-rules`) for hard size limits.
- Arr naming tokens: not translated by the import. This skill rewrites them by hand.

## Where the import runs

Operator skips first-run setup and creates an API key in the browser. Agent
then re-enters setup through the API, as **Settings → General → Run setup
wizard** does, and runs the wizard's import in wizard order, ending with
`completeSetup` for the import run, which erases the copy of client and
indexer secrets stored for that run. This skill only adds roots, never removes
one.
