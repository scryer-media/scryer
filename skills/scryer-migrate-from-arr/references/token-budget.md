# Token budget

Rough tokens read per phase following this skill's jq projections. Library
size barely matters; bulk data stays in files. Phase far over: something is
read whole; stop and project.

- a. Snapshot: `summary.json`, 1–3k
- b. Plan and prepare: `what-migrates.md` + summary + masked-secret list, 2–3k
- c. Setup state: one `SetupStatus` line, under 1k
- d. Wizard import: five wizard steps, one-line jq outputs, lean preview reads, 4–8k (more with many clients and indexers); over MCP the roots read adds about 1k per 25 roots
- e. Import check: one line per check, 1–2k
- f. Custom formats: review problems, score count, translate summary, replay summary + up to 20 mismatches, 3–6k per app
- g. Other areas: one projection per area, plans shown to operator, 1–2k per area
- h. Verify: one line per check, 2–4k
- i. Cutover: three short jq checks, 1k

Whole migration of one Sonarr and one Radarr typically stays under 40k tokens
of tool output.

Never read: `series.json`, `movie.json`, `history_grabbed.json`,
`customformat.json`, any translated `.rego` file, `roots.json` or any
GraphQL response in full. Never read the schema or Scryer's source.
