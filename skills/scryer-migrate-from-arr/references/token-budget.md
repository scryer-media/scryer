# Token budget

Rough tokens the agent reads per phase when it follows the jq projections in
this skill. Library size barely matters, because bulk data stays in files.
If a phase runs far over, something is being read whole: stop and project.

| Phase | Reads | About |
|---|---|---|
| a. Snapshot | `summary.json` | 1–3k |
| b. Plan | `what-migrates.md` + summary | 2k |
| c. Built-in import | 7 steps, one-line jq outputs, preview projection | 4–8k (more with many clients and indexers) |
| d. Custom formats | review problems, score count, translate summary, replay summary + up to 20 mismatches | 3–6k per app |
| e. Other areas | one projection per area, plans shown to the operator | 1–2k per area |
| f. Scan and checklist | scan summary per library, one line per check | 2–4k |
| g. Cutover | three short jq checks | 1k |

The whole migration of one Sonarr and one Radarr typically stays under 40k
tokens of tool output.

Never read: `series.json`, `movie.json`, `history_grabbed.json`,
`customformat.json`, any translated `.rego` file, `preview.json` or any
GraphQL response in full. Never read the schema or Scryer's source.
