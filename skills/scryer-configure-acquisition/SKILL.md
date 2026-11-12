---
name: scryer-configure-acquisition
description: Reads and changes the settings that decide where Scryer gets releases and which it takes, through the API after the setup wizard - indexers, download clients, per-scope routing, quality profiles and delay profiles - showing every change before it is made. Use when an operator wants to add, edit, enable, disable, reprioritise or test an indexer or download client, set categories or remote path mappings, tune quality or upgrade rules, or change delays.
---

# Configure indexers, clients and profiles

Change these settings with exact, reviewed writes. Every read is
safe. Every write and every connection test is shown to the operator
first and waits for a yes.

## Prerequisites

- Complete `scryer-connect` first. It sets `SCRYER_URL` and
  `SCRYER_API_KEY` and provides `../scryer-connect/scripts/gql.sh`
  (below, `G="$S"/../scryer-connect/scripts/gql.sh`).
- Set `S` to this skill's directory (absolute path) and work in a separate
  directory the operator agrees to, so output files never land inside the
  skill. Secret-bearing files go in a private subdirectory of it.
- Needs the **Manage system settings** permission for indexers and
  download clients, and the **Manage catalog settings** permission for
  routing, quality and delay profiles.
- Local tools: `jq`.

## Rules

- Do not read the GraphQL schema or Scryer's source code. Use the files in
  `operations/`; they are complete for this skill.
- Never hard-code a plugin's config keys. Discover them
  (`references/plugin-config-fields.md`).
- Secrets come only from a file or an environment variable the operator
  set, through `scripts/add-secret.sh`. Never ask for a secret in the
  conversation, never print one, never put one on a command line.
- Write every response to a file; read it with jq.
- Branch on `extensions.reason` / `extensions.code`, never on the error
  sentence (`references/refusal-codes.md`).
- Full-replacement inputs: indexer and client `config` (non-secret keys),
  indexer routing, download-client routing. Always send the complete set.
- Quality profiles: always `replaceExisting: false`.
- This skill never removes indexers, clients, profiles or proxies. Point
  the operator to the UI for removals.

## Procedure

1. **Pick the area** from the request and load only its reference:

   | area | reference |
   |---|---|
   | indexers, indexer routing, proxies in use | `references/indexers.md` |
   | download clients, categories, path mappings, client routing | `references/download-clients.md` |
   | quality profiles, cutoff, upgrades | `references/quality-profiles.md` |
   | delay profiles | `references/delay-profiles.md` |

   Adding an indexer or client also needs
   `references/plugin-config-fields.md`.

2. **Read the current state** with the area's read operation and save it:
   `sh "$G" "$S"/operations/<Read>.graphql [vars.json] > current.json`. Show the
   operator a one-line-per-item summary, not the raw JSON.

3. **Agree on the change** in plain terms. Ask only for values the
   operator has not given. For secrets, ask the operator to write the value
   to a file (or export a variable) and give you the path or name.

4. **Build the variables file** from the stored state, changing only the
   requested fields (`scripts/config-to-input.sh` for config arrays,
   `scripts/add-secret.sh` for secrets, jq for the rest).

5. **Test before saving** where the area supports it (indexers and download
   clients). A test contacts the remote service; ask first. A test of a
   saved indexer by id also updates its stored capabilities and last error.

6. **Show the change**: `sh "$S"/scripts/show-change.sh before.json vars.json`.
   Present its lines as the before/after. Add a plain note for anything
   with wider effect:
   - an entry shown as `(removed)` in config or routing
   - `removeCompleted` or `removeFailed` turned on (Scryer removes items
     from the download client); this needs its own explicit yes
   - creating an indexer (Scryer contacts it to validate)
   Wait for a yes.

7. **Write**: `sh "$G" "$S"/operations/<Write>.graphql vars.json > result.json`.
   On a non-zero exit, look up the reason in
   `references/refusal-codes.md`, explain it, and go back to step 3.

8. **Confirm**: re-run the read and show the changed fields only.

9. **Hand back secret files.** Tell the operator which local files hold a
   secret (the secret file they wrote and any `*.json` built by
   `add-secret.sh`) so they can dispose of them. Do not remove them
   yourself.

## Stop and ask

- Before every write and every connection test (steps 5 and 6).
- When `config-to-input.sh` warns that keys would be dropped.
- When a plugin needs a login challenge answered (manual in the UI).
- When the request needs something this skill does not cover: removals,
  proxy setup, seeding profiles, indexer-manager sync, movie availability,
  release scoring rules (`scryer-write-rules`). Give the UI area instead.
- Acquisition worker settings are internal and not operator-configurable;
  if they look wrong, that goes to the maintainer via `scryer-report-bug`.
