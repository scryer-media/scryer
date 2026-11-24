---
name: scryer-configure-acquisition
description: Reads and changes the settings that decide where Scryer gets releases and which it takes, through the API after the setup wizard - indexers, download clients, per-scope routing, quality profiles and delay profiles - showing every change before it is made. Use when an operator wants to add, edit, enable, disable, reprioritise or test an indexer or download client, set categories or remote path mappings, tune quality or upgrade rules, or change delays.
---

# Configure indexers, clients and profiles

Exact, reviewed writes. Reads are safe. Every write and every connection test
is shown to the operator first and waits for a yes.

## Prerequisites

- Complete `scryer-connect` first (sets `SCRYER_URL`, `SCRYER_API_KEY`;
  provides `../scryer-connect/scripts/gql.sh`, below
  `G="$S"/../scryer-connect/scripts/gql.sh`).
- `S` = this skill's directory (absolute). Work in a separate directory the
  operator agrees to, so output never lands inside the skill. Secret-bearing
  files go in a private subdirectory of it.
- Needs **Manage system settings** for indexers and download clients, and
  **Manage catalog settings** for routing, quality and delay profiles.
- Local tools: `jq`.

## Rules

- No schema or source reads. Files in `operations/` are complete for this
  skill.
- Never hard-code a plugin's config keys; discover them
  (`references/plugin-config-fields.md`).
- Secrets only from a file or environment variable the operator set, via
  `scripts/add-secret.sh`. Never ask for a secret in the conversation, never
  print one, never put one on a command line.
- Every response to a file; read with jq.
- Branch on `extensions.reason` / `extensions.code`, never the error sentence
  (`references/refusal-codes.md`).
- Full-replacement inputs: indexer and client `config` (non-secret keys),
  indexer routing, download-client routing. Always send the complete set.
- Quality profiles: always `replaceExisting: false`.
- Never removes indexers, clients, profiles or proxies. Point operator to the
  UI for removals.

## Procedure

1. **Pick the area**; load only its reference:
   - indexers, indexer routing, proxies in use: `references/indexers.md`
   - download clients, categories, path mappings, client routing:
     `references/download-clients.md`
   - quality profiles, cutoff, upgrades: `references/quality-profiles.md`
   - delay profiles: `references/delay-profiles.md`

   Adding an indexer or client also needs `references/plugin-config-fields.md`.

2. **Read current state** with the area's read operation, save it:
   `sh "$G" "$S"/operations/<Read>.graphql [vars.json] > current.json`. Show a
   one-line-per-item summary, not raw JSON.

3. **Agree on the change** in plain terms. Ask only for values not given. For
   secrets, operator writes the value to a file (or exports a variable) and
   gives you the path or name.

4. **Build the variables file** from stored state, changing only requested
   fields (`scripts/config-to-input.sh` for config arrays,
   `scripts/add-secret.sh` for secrets, jq for the rest).

5. **Test before saving** where supported (indexers, download clients). A
   test contacts the remote service; ask first. Testing a saved indexer by id
   also updates its stored capabilities and last error.

6. **Show the change**: `sh "$S"/scripts/show-change.sh before.json vars.json`.
   Present its lines as before/after. Add a plain note for anything with wider
   effect:
   - an entry shown as `(removed)` in config or routing
   - `removeCompleted` or `removeFailed` turned on (Scryer removes items from
     the download client); needs its own explicit yes
   - creating an indexer (Scryer contacts it to validate)
   Wait for a yes.

7. **Write**: `sh "$G" "$S"/operations/<Write>.graphql vars.json > result.json`.
   Non-zero exit: look up the reason in `references/refusal-codes.md`,
   explain, back to step 3.

8. **Confirm**: re-run the read; show changed fields only.

9. **Hand back secret files.** Tell the operator which local files hold a
   secret (their secret file and any `*.json` built by `add-secret.sh`) so
   they can dispose of them. Do not remove them yourself.

## Stop and ask

- Before every write and every connection test (steps 5 and 6).
- When `config-to-input.sh` warns keys would be dropped.
- When a plugin needs a login challenge answered (manual in the UI).
- When the request needs something not covered: removals, proxy setup,
  seeding profiles, indexer-manager sync, movie availability, release scoring
  rules (`scryer-write-rules`). Give the UI area instead.
- Acquisition worker settings are internal, not operator-configurable; if
  they look wrong, route to the maintainer via `scryer-report-bug`.
