---
name: scryer-migrate-from-arr
description: Migrates a Sonarr and/or Radarr setup into Scryer. It runs the setup wizard's own Sonarr/Radarr import through Scryer's GraphQL API (the same calls, in the same order, as the web wizard), then verifies the result and carries over what the import leaves out, keeping bulk data in files. Use it when an operator wants to "move from Sonarr", "switch from Radarr", "import my arr library", bring over custom formats, quality profiles, delay profiles, naming, tags or notifications, compare Scryer scoring with arr history, or plan the cutover from arr to Scryer.
---

# Migrate from Sonarr / Radarr

Set the arr variables (URLs without a trailing slash, including any base
path; keys from arr's Settings → General). Steps a and b need only these;
`scryer-connect` comes in step c.

```sh
export SONARR_URL=http://sonarr.example.lan:8989 SONARR_API_KEY=...   # and/or
export RADARR_URL=http://radarr.example.lan:7878 RADARR_API_KEY=...
C=path/to/skills/scryer-connect; M=path/to/skills/scryer-migrate-from-arr; O=$M/operations
gql()  { "$C/scripts/gql.sh" "$@"; }
poll() { "$M/scripts/poll.sh" "$@"; }
```

Ask the operator to set the keys in their shell. Never print, echo or paste a
key. Work in a directory the operator chooses; snapshot files hold arr
configuration and are written mode 600.

## Rules for this skill

- Sonarr and Radarr are only read (GET). Any arr change is made by the
  operator in arr's UI.
- Read-only steps run directly. Every Scryer write shows the operator
  a short summary (counts, names, the first few items) and waits for a yes.
- Never delete anything: not arr data, not media, not Scryer titles, roots or
  rule sets. Never send `removedRootIds`.
- Read summaries and jq projections only (`references/token-budget.md`).
- Branch on `extensions.code` and `extensions.reason`, never on message text.

## a. Snapshot (read-only)

```sh
"$M/scripts/snapshot.sh" snap
jq . snap/summary.json
```

`failedEndpoints` above 0 names a missing file under `snap/<app>/` with
`_error`. An older arr may lack an endpoint; carry on and mark that area
manual. `HISTORY_LIMIT=1000` widens the history sample.

## b. Plan and prepare (read-only)

Present `references/what-migrates.md` filled in with the summary's counts and
ask which areas the operator wants. Then settle what the import will need:

- **The arr URL as Scryer reaches it.** Scryer connects to arr itself. When
  Scryer runs in a container, `localhost` is the container. Agree the URL
  Scryer must use for each app, and for Prowlarr if they use it.
- **A Scryer-side path for every root.** List `rootFolders` from the summary.
  For each, ask where Scryer sees that folder and which library it belongs to
  (anime is its own library facet).
- **Secrets to enter again.** Arr hides some secrets from its API, so the
  import cannot always copy them. The import's preview names each client and
  indexer that needs one (`built-in-import.md`, step 4). Ask the operator to
  have download client passwords and indexer API keys at hand, to put in
  environment variables or files when that step asks.

## c. Setup state and the Scryer connection

Only a signed-in browser session can create an API key, and until first-run
setup is complete the web UI sends every page to the setup wizard. If Scryer
still opens its wizard, ask the operator to choose **Skip setup entirely** on
its first screen and then create the key. That is the only browser step; the
agent then re-enters setup through the API (step d), as **Settings → General
→ Run setup wizard** does in the web UI.

Then complete `scryer-connect` and read the state:

```sh
gql "$O/SetupStatus.graphql" | jq -c .data.setupStatus
```

`setupComplete` is `true` once a key works. `hasDownloadClients` or
`hasIndexers` `true` means some exist already: name them before importing.

## d. The setup wizard's import (agent, through the API)

Follow `references/built-in-import.md`. It is the wizard's Connect,
Libraries, Quality, Sources and Summary steps in the wizard's order, ending
with `completeSetup` for the import run and the hinted scans, as the wizard
does. The import itself creates the download clients, indexers and library
roots.

## e. Check what the import produced (read-only)

Run the first seven rows of `references/verification.md` (libraries and
roots, titles, scan results, monitored state, download clients, indexers)
against the snapshot. Report gaps before going on.

## f. Custom formats

Follow `references/custom-formats.md`:

1. Translate in review mode and list formats that cannot be translated.
2. Settle the score decision with the operator explicitly: arr scores are
   per quality profile, the translated rule set applies to every profile. Use
   `qualityProfiles.distinctScoreSets` from the summary.
3. Translate with the agreed scores and validate.
4. Replay arr's grab history and report mismatches only:

```sh
"$M/scripts/replay-history.sh" snap/sonarr arr-formats.rego series,anime <arr-profile-id> > replay.tsv
```

5. Save disabled, then enable after the operator agrees.

## g. The rest

`references/manual-areas.md` has one section per area with arr → Scryer field
tables and the exact operations: titles with no files, quality profiles,
delay profiles, naming, remote path mappings, tags, notifications and Sonarr
release profiles. Import lists, list exclusions, auto-tagging and history
are manual or unsupported; that file says what to tell the operator.

## h. Verify

Work through the rest of `references/verification.md` and report one line per
check with the gap.

## i. Cutover

Follow `references/cutover.md`: run both side by side with Scryer's automatic
search off, have the operator turn off arr's RSS, automatic search and
Completed Download Handling first, then turn Scryer on. Keep all arr data
until the operator has verified Scryer. Rollback is the reverse.

## Scripts

| Script | Does |
|---|---|
| `scripts/snapshot.sh <dir>` | GETs arr config and history into files, writes counts to `summary.json` |
| `scripts/poll.sh <op> <vars> '<jq cond>' [interval] [max]` | Polls an operation until the condition holds (exit 0), times out (3) or fails |
| `scripts/replay-history.sh <snap/app> <rule.rego> <facets> [arr-profile-id]` | Scores arr's grabbed releases with a draft rule set; TSV of match, mismatch, skipped |

All three need `curl` and `jq`, and find `gql.sh` in the sibling
`scryer-connect` skill (override with `GQL=`).

## When something fails

- `gql.sh` exit 1 prints `code [reason]`. Look the reason up in
  `scryer-connect/references/errors.md` or in the reference for that phase.
- `MFA_STEP_UP_REQUIRED`: config writes need the security setting described
  in `scryer-connect`. Ask the operator; do not work around it.
