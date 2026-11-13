# Token budget

Rough tokens the agent reads per phase when it follows the jq projections in
this skill. Library size barely matters, because bulk data stays in files.
If a phase runs far over, something is being read whole: stop and project.

| Phase | Reads | About |
|---|---|---|
| a. Snapshot | `summary.json` | 1–3k |
| b. Plan and prepare | `what-migrates.md` + summary + masked-secret list | 2–3k |
| c. Setup state | one `SetupStatus` line | under 1k |
| d. Wizard import | five wizard steps, one-line jq outputs, preview projection | 4–8k (more with many clients and indexers) |
| e. Import check | one line per check | 1–2k |
| f. Custom formats | review problems, score count, translate summary, replay summary + up to 20 mismatches | 3–6k per app |
| g. Other areas | one projection per area, plans shown to the operator | 1–2k per area |
| h. Verify | one line per check | 2–4k |
| i. Cutover | three short jq checks | 1k |

The whole migration of one Sonarr and one Radarr typically stays under 40k
tokens of tool output.

Never read: `series.json`, `movie.json`, `history_grabbed.json`,
`customformat.json`, any translated `.rego` file, `preview.json` or any
GraphQL response in full. Never read the schema or Scryer's source.
