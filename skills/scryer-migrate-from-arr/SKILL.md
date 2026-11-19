---
name: scryer-migrate-from-arr
description: Migrates a Sonarr and/or Radarr setup into Scryer. Runs the setup wizard's own Sonarr/Radarr import through Scryer's GraphQL API (same calls, same order as the web wizard), verifies the result and carries over what the import leaves out, keeping bulk data in files. Use when an operator wants to "move from Sonarr", "switch from Radarr", "import my arr library", bring over custom formats, quality profiles, delay profiles, naming, tags or notifications, compare Scryer scoring with arr history, or plan the cutover from arr to Scryer.
---

# Migrate from Sonarr / Radarr

Set arr variables (URLs without trailing slash, including any base path; keys
from arr's Settings → General). Steps a and b need only these; `scryer-connect`
comes in step c.

```sh
export SONARR_URL=http://sonarr.example.lan:8989 SONARR_API_KEY=...   # and/or
export RADARR_URL=http://radarr.example.lan:7878 RADARR_API_KEY=...
C=path/to/skills/scryer-connect; M=path/to/skills/scryer-migrate-from-arr; O=$M/operations
gql()  { "$C/scripts/gql.sh" "$@"; }
poll() { "$M/scripts/poll.sh" "$@"; }
```

Operator sets the keys in their shell. Never print, echo or paste a key. Work
in a directory the operator chooses; snapshot files hold arr config and are
written mode 600.

## Rules for this skill

- Sonarr and Radarr are only read (GET). Any arr change is made by the operator
  in arr's UI.
- Read-only steps run directly. Every Scryer write: show a short summary
  (counts, names, first few items), wait for a yes.
- Never delete anything: not arr data, media, Scryer titles, roots or rule
  sets. Never send `removedRootIds`.
- Read summaries and jq projections only (`references/token-budget.md`).
- Branch on `extensions.code` and `extensions.reason`, never on message text.

## a. Snapshot (read-only)

```sh
"$M/scripts/snapshot.sh" snap
jq . snap/summary.json
```

`failedEndpoints` above 0: the missing file under `snap/<app>/` has `_error`.
Older arr may lack an endpoint; carry on, mark that area manual.
`HISTORY_LIMIT=1000` widens the history sample.

## b. Plan and prepare (read-only)

Present `references/what-migrates.md` filled with the summary's counts; ask
which areas the operator wants. Then settle:

- **Arr URL as Scryer reaches it.** Scryer connects to arr itself. In a
  container, `localhost` is the container. Agree the URL Scryer must use per
  app, and for Prowlarr if used.
- **Scryer-side path for every root.** List `rootFolders` from the summary.
  For each: where Scryer sees it, which library it belongs to (anime is its
  own library facet).
- **Secrets to re-enter.** Arr hides some secrets from its API, so the import
  cannot always copy them. Preview names each client and indexer needing one
  (`built-in-import.md`, step 4). Operator keeps download client passwords and
  indexer API keys at hand, to put in env variables or files when that step
  asks.

## c. Setup state and Scryer connection

Only a signed-in browser session can create an API key, and until first-run
setup is complete the web UI sends every page to the setup wizard. If Scryer
still opens its wizard, operator chooses **Skip setup entirely** on its first
screen, then creates the key. Only browser step; agent then re-enters setup
through the API (step d), as **Settings → General → Run setup wizard** does in
the web UI.

Then complete `scryer-connect` and read state:

```sh
gql "$O/SetupStatus.graphql" | jq -c .data.setupStatus
```

`setupComplete` is `true` once a key works. `hasDownloadClients` or
`hasIndexers` `true`: some exist already; name them before importing.

## d. Setup wizard's import (agent, through the API)

Follow `references/built-in-import.md`: wizard's Connect, Libraries, Quality,
Sources, Summary steps in wizard order, ending with `completeSetup` for the
import run and the hinted scans. The import itself creates download clients,
indexers and library roots.

## e. Check import result (read-only)

Run first seven rows of `references/verification.md` (libraries and roots,
titles, scan results, monitored state, download clients, indexers) against the
snapshot. Report gaps before going on.

## f. Custom formats

Follow `references/custom-formats.md`:

1. Translate in review mode; list formats that cannot be translated.
2. Settle score decision with operator explicitly: arr scores are per quality
   profile, translated rule set applies to every profile. Use
   `qualityProfiles.distinctScoreSets` from the summary.
3. Translate with agreed scores, validate.
4. Replay arr grab history, report mismatches only:

```sh
"$M/scripts/replay-history.sh" snap/sonarr arr-formats.rego series,anime <arr-profile-id> > replay.tsv
```

5. Save disabled; enable after operator agrees.

## g. The rest

`references/manual-areas.md`: one section per area with arr → Scryer field
maps and exact operations: titles with no files, quality profiles, delay
profiles, naming, remote path mappings, tags, notifications, Sonarr release
profiles. Import lists, list exclusions, auto-tagging and history are manual or
unsupported; that file says what to tell the operator.

## h. Verify

Rest of `references/verification.md`; one line per check with the gap.

## i. Cutover

Follow `references/cutover.md`: run side by side with Scryer's automatic
search off; operator turns off arr's RSS, automatic search and Completed
Download Handling first, then Scryer goes on. Keep all arr data until operator
has verified Scryer. Rollback is the reverse.

## Scripts

- `scripts/snapshot.sh <dir>`: GETs arr config and history into files, counts to `summary.json`.
- `scripts/poll.sh <op> <vars> '<jq cond>' [interval] [max]`: polls an operation until condition holds (exit 0), times out (3) or fails.
- `scripts/replay-history.sh <snap/app> <rule.rego> <facets> [arr-profile-id]`: scores arr's grabbed releases with a draft rule set; TSV of match, mismatch, skipped.

All need `curl` and `jq`, and find `gql.sh` in sibling `scryer-connect` skill
(override with `GQL=`).

## When something fails

- `gql.sh` exit 1 prints `code [reason]`. Look the reason up in
  `scryer-connect/references/errors.md` or the reference for that phase.
- `MFA_STEP_UP_REQUIRED`: config writes need the security setting described in
  `scryer-connect`. Ask the operator; do not work around it.
