---
name: scryer-migrate-from-arr
description: Migrates a Sonarr and/or Radarr setup into Scryer through Scryer's GraphQL API and arr's v3 API, keeping bulk data in files. Use it when an operator wants to "move from Sonarr", "switch from Radarr", "import my arr library", bring over custom formats, quality profiles, delay profiles, naming, tags or notifications, compare Scryer scoring with arr history, or plan the cutover from arr to Scryer.
---

# Migrate from Sonarr / Radarr

Complete `scryer-connect` first. Then set the arr variables (URLs without a
trailing slash, including any base path; keys from arr's Settings → General):

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
- Read-only steps run without asking. Every Scryer write shows the operator
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

## b. Tell the operator the plan

Before any change, present `references/what-migrates.md` filled in with the
summary's counts: what the built-in import moves, what this skill moves step
by step, and what stays manual. Ask which areas they want.

## c. Built-in import

Follow `references/built-in-import.md`: validate the connection, warm up,
preview, create the selected download clients and indexers, make sure every
arr root has a Scryer library root, finalize, then scan each library. The
import works after first-run setup too. Do not call `completeSetup`.

## d. Custom formats

Follow `references/custom-formats.md`:

1. Translate in review mode and list formats that cannot be translated.
2. Settle the score decision with the operator explicitly: arr scores are
   per quality profile, Scryer rule sets are global. Use
   `qualityProfiles.distinctScoreSets` from the summary.
3. Translate with the agreed scores, add a profile guard if needed, validate.
4. Replay arr's grab history and report mismatches only:

```sh
"$M/scripts/replay-history.sh" snap/sonarr arr-formats.rego series,anime <arr-profile-id> > replay.tsv
```

5. Save disabled, then enable after the operator agrees.

## e. The rest

`references/manual-areas.md` has one section per area with arr → Scryer field
tables and the exact operations: titles with no files, quality profiles,
delay profiles, naming, tags, notifications and Sonarr release profiles.
Remote path mappings, import lists, list exclusions, auto-tagging and history
are manual or unsupported; that file says what to tell the operator.

## f. Scan and verify

The scan runs in step c. Then work through `references/verification.md` and
report one line per check with the gap.

## g. Cutover

Follow `references/cutover.md`: run both side by side with Scryer's automatic
search off, have the operator turn off arr's RSS, automatic search and
Completed Download Handling first, then turn Scryer on. Keep all arr data
until the operator has verified Scryer. Rollback is the reverse.

## Scripts

| Script | Does |
|---|---|
| `scripts/snapshot.sh <dir>` | GETs arr config and history into files, writes counts to `summary.json` |
| `scripts/poll.sh <op> <vars> '<jq cond>' [interval] [max]` | Polls a status operation until the condition holds (exit 0), times out (3) or fails |
| `scripts/replay-history.sh <snap/app> <rule.rego> <facets> [arr-profile-id]` | Scores arr's grabbed releases with a draft rule set; TSV of match, mismatch, skipped |

All three need `curl` and `jq`, and find `gql.sh` in the sibling
`scryer-connect` skill (override with `GQL=`).

## When something fails

- `gql.sh` exit 1 prints `code [reason]`. Look the reason up in
  `scryer-connect/references/errors.md` or in the reference for that phase.
- `MFA_STEP_UP_REQUIRED`: config writes need the security setting described
  in `scryer-connect`. Ask the operator; do not work around it.
- A warm-up or finalize that ends `FAILED`: show `errorMessage` to the
  operator and run that step again.
