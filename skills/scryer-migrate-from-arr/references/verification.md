# Verification checklist

All read-only. Compare with `summary.json`; one line per check: expected,
found, gap. First seven rows check the setup wizard's import (`SKILL.md`, step
e); run them right after it, whether agent or operator ran the import. Scan
rows need scan summaries; if operator ran the import in the web UI, use title
counts instead.

| Check | Scryer side | Arr side (`summary.json`) |
|---|---|---|
| Libraries and roots | `Libraries.graphql`: every arr root has a Scryer root | `rootFolders` |
| Titles | `TitleCounts.graphql` per facet | `titles.withFiles` right after import, `titles.total` once titles without files are added (Sonarr split by `anime`) |
| Titles with files matched | scan `summary.matched` + `summary.imported` | `titles.withFiles` |
| Unmatched files | scan `summary.unmatched` | 0 expected; list a sample to operator |
| Monitored | `TitlesByExternalIds` on a sample of 50 ids, compare `monitored` | same ids in `series.json` / `movie.json` |
| Download clients | `DownloadClientConfigs.graphql`: count and `isEnabled` | `downloadClients` |
| Indexers | `Indexers.graphql`: count and enabled | `indexers` |
| Quality profiles | `QualityProfileSettings.graphql`: every library has a profile | `qualityProfiles.names` |
| Custom formats | rule set exists, replay mismatches explained | `customFormats` |
| Delay profiles | `DelayProfiles.graphql` | `delayProfiles` |
| Naming | `MediaSettings.graphql` per scope | `config_naming.json` |
| Remote path mappings | `scryer-configure-acquisition`'s `ListDownloadClients.graphql`: each client's `remote_path_mappings` | `remotePathMappings` |
| Missing titles | count of titles added without files | `titles.withoutFiles` |

Monitored-state sample check:

```sh
jq -r '[.[] | .tvdbId | tostring][:50]' snap/sonarr/series.json > ids.json
jq -n --slurpfile v ids.json '{source: "tvdb", values: $v[0]}' > tv.json
gql "$O/TitlesByExternalIds.graphql" tv.json > found.json
jq -n --slurpfile s snap/sonarr/series.json --slurpfile f found.json '
  ($f[0].data.titlesByExternalIds | map({key: (.externalIds[] | select(.source == "tvdb") | .value), value: .monitored}) | from_entries) as $m
  | [$s[0][:50][] | select(($m[.tvdbId | tostring]) != null and ($m[.tvdbId | tostring]) != .monitored) | .title] | {mismatched: length, sample: .[:5]}'
```

Finish with a short end-to-end test the operator approves: one manual search
for a title in Scryer, one grab, one import, operator watching in Scryer's web
UI. Side effect (it downloads): ask first, and only after arr's download
handling for that client is off (`cutover.md`).
