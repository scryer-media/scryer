---
name: scryer-troubleshoot
description: Diagnoses problems on an operator's own Scryer instance from the diagnostics bundle and read-only API queries. Use when an operator says Scryer is not downloading, downloads are not importing, the wrong release was grabbed, searches return nothing, the UI or login misbehaves, Scryer is slow, or an upgrade failed or is not offered.
---

# Troubleshoot Scryer

Find the cause of a Scryer problem while reading as little as possible.
Diagnosis is read-only. A settings change or a connection test happens
only after the operator says yes. Bulk data goes to files; read only
counts, jq extracts and capped searches.

## Prerequisites

- Complete `scryer-connect` first. It sets `SCRYER_URL` and
  `SCRYER_API_KEY` and provides `../scryer-connect/scripts/gql.sh`.
- Set `S` to this skill's directory (absolute path) and work in a separate
  directory the operator agrees to, so output files never land inside the
  skill.
- The key's user needs **Manage system settings** for the bundle, indexers,
  download clients and logs.
- Local tools: `curl`, `jq`, `zstd`, `tar`.

## Rules

- Do not read the GraphQL schema or Scryer's source code. The operations in
  `operations/` are complete for this skill.
- Never print, log or paste the API key. Never `cat` a whole log,
  `metrics.prom` or a large JSON response.
- Branch on codes (`extensions.code`, `extensions.reason`, decision and
  import codes), never on error or log sentences.
- Do not restart services, delete data, clear or remove queue items, change
  file permissions or edit containers. Describe the action and let the
  operator do it.
- Any change to Scryer's settings is a write: show the exact change and
  wait for a yes. Most fixes belong to `scryer-configure-acquisition` or
  `scryer-write-rules`.
- State times in the operator's local time zone. Bundle and API timestamps
  are UTC; convert them.

## Procedure

1. **Classify the symptom** from the operator's words into one class:
   nothing downloads / downloads do not import / wrong release chosen /
   search finds nothing / UI or login / performance / upgrade. Ask one
   short question only if the class is unclear. Ask for a title name when
   the problem is about one title.

2. **Fetch the bundle** (read-only, nothing leaves the machine):

   ```sh
   sh "$S"/scripts/fetch-bundle.sh ./scryer-diag-1
   ```

   It writes the archive and an unpacked `bundle/` directory and prints the
   manifest summary. On a non-zero exit, read the printed `reason`:
   `HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED` → redo `scryer-connect`;
   `HTTP_DIAGNOSTICS_PERMISSION_DENIED` → the key's user needs Manage system
   settings; `HTTP_DIAGNOSTICS_BUNDLE_FAILED` → continue with steps 4 and 5
   only. If the UI path is easier for the operator, the same bundle is under
   **System > Service Logs**.

3. **Read the manifest summary** that the script printed. Note omissions
   and truncations; they limit what the bundle can show (meanings in
   `references/bundle.md`).

4. **Read only the sections for this class.** Load
   `references/bundle.md` → "Which sections matter per symptom" and run
   those jq extracts. Start with:

   ```sh
   B=./scryer-diag-1/bundle
   jq '{v: .build.version, install: .installation.kind, os: .os.family, container: .container}' "$B/environment.json"
   sh "$S"/scripts/log-search.sh "$B" --summary
   ```

5. **Query live state** with the operations for this class. Write each
   response to a file and extract with jq:

   ```sh
   G="$S"/../scryer-connect/scripts/gql.sh
   sh "$G" "$S"/operations/HealthChecks.graphql > health.json
   jq -r '.data.jobRuns[0].summaryJson.checks[] | select(.status != "ok") | "\(.status) \(.source): \(.message)"' health.json
   ```

   For operations with variables, write the variables to a small JSON file
   and pass it as the second argument.

6. **Walk the decision tree.** Load `references/decision-trees.md` and
   follow only the section for this class. Load `references/codes.md` when a
   code needs explaining.

7. **Search logs narrowly** when a step asks for it:
   `sh "$S"/scripts/log-search.sh "$B" '<regex>' WARN 40`. Widen the level or
   the pattern only if nothing matches.

8. **Report to the operator**: the likely cause, the evidence (codes and
   counts, not raw dumps) and the fix. For each fix say who does it: a
   setting change you can make after a yes, a UI path, or a host-side action
   the operator takes.

## Stop and ask

- Before any settings change: show the field, the current value and the
  proposed value, and wait for a yes.
- Before testing a connection to an indexer or download client: that
  contacts the remote service; say so and wait for a yes.
- When the evidence points to a defect in Scryer (scores that contradict
  the profile, a crash, a failed migration, an unparseable ordinary release
  name, a code path that contradicts its own health check): stop
  diagnosing and offer `scryer-report-bug`, passing along the bundle
  directory and the evidence.
- When the cause is outside Scryer (indexer account, client host, disk,
  network, reverse proxy): say what to check and stop.

## Token budget

A typical run should stay around 15k to 30k tokens of tool output:

| step | rough size |
|---|---|
| manifest summary | 0.5k |
| 3 to 5 jq extracts | 2k to 5k |
| log summary plus 1 to 3 searches of 40 lines | 3k to 8k |
| 2 to 4 live queries, filtered | 2k to 6k |
| one reference file | 2k to 4k |

If a single read would exceed about 5k tokens, filter it further with jq,
`head` or a narrower regex instead of reading it.
