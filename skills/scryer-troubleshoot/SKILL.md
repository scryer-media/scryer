---
name: scryer-troubleshoot
description: Diagnoses problems on an operator's own Scryer instance from the diagnostics bundle and read-only API queries. Use when an operator says Scryer is not downloading, downloads are not importing, the wrong release was grabbed, searches return nothing, the UI or login misbehaves, Scryer is slow, or an upgrade failed or is not offered.
---

# Troubleshoot Scryer

Find the cause reading as little as possible. Diagnosis is read-only. A
settings change or connection test happens only after the operator says yes.
Bulk data to files; read only counts, jq extracts and capped searches.

## Prerequisites

- Complete `scryer-connect` first (sets `SCRYER_URL`, `SCRYER_API_KEY`;
  provides `../scryer-connect/scripts/gql.sh`).
- `S` = this skill's directory (absolute). Work in a separate directory the
  operator agrees to, so output never lands inside the skill.
- Key's user needs **Manage system settings** for the bundle, indexers,
  download clients and logs.
- Local tools: `curl`, `jq`, `zstd`, `tar`.

## Rules

- No schema or source reads. Operations in `operations/` are complete for
  this skill.
- Never print, log or paste the API key. Never `cat` a whole log,
  `metrics.prom` or a large JSON response.
- Branch on codes (`extensions.code`, `extensions.reason`, decision and import
  codes), never on error or log sentences.
- Do not restart services, delete data, clear or remove queue items, change
  file permissions or edit containers. Describe the action; operator does it.
- Any Scryer settings change is a write: show the exact change, wait for a
  yes. Most fixes belong to `scryer-configure-acquisition` or
  `scryer-write-rules`.
- State times in the operator's local time zone. Bundle and API timestamps
  are UTC; convert them.

## Procedure

1. **Classify the symptom** into one class: nothing downloads / downloads do
   not import / wrong release chosen / search finds nothing / UI or login /
   performance / upgrade. Ask one short question only if unclear. Ask for a
   title name when the problem is about one title.

2. **Fetch the bundle** (read-only, nothing leaves the machine):

   ```sh
   sh "$S"/scripts/fetch-bundle.sh ./scryer-diag-1
   ```

   Writes the archive and an unpacked `bundle/` directory, prints the manifest
   summary. Non-zero exit: read the printed `reason`:
   `HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED` → redo `scryer-connect`;
   `HTTP_DIAGNOSTICS_PERMISSION_DENIED` → key's user needs Manage system
   settings; `HTTP_DIAGNOSTICS_BUNDLE_FAILED` → continue with steps 4 and 5
   only. If easier for the operator, the same bundle is in the UI under
   **System > Service Logs**.

3. **Read the printed manifest summary.** Note omissions and truncations;
   they limit what the bundle can show (meanings in `references/bundle.md`).

4. **Read only this class's sections.** Load `references/bundle.md` → "Which
   sections matter per symptom" and run those jq extracts. Start with:

   ```sh
   B=./scryer-diag-1/bundle
   jq '{v: .build.version, install: .installation.kind, os: .os.family, container: .container}' "$B/environment.json"
   sh "$S"/scripts/log-search.sh "$B" --summary
   ```

5. **Query live state** with this class's operations. Each response to a
   file, extract with jq:

   ```sh
   G="$S"/../scryer-connect/scripts/gql.sh
   sh "$G" "$S"/operations/HealthChecks.graphql > health.json
   jq -r '.data.jobRuns[0].summaryJson.checks[] | select(.status != "ok") | "\(.status) \(.source): \(.message)"' health.json
   ```

   Operations with variables: write them to a small JSON file, pass as second
   argument.

6. **Walk the decision tree.** Load `references/decision-trees.md`; follow
   only this class's section. Load `references/codes.md` when a code needs
   explaining.

7. **Search logs narrowly** when a step asks:
   `sh "$S"/scripts/log-search.sh "$B" '<regex>' WARN 40`. Widen level or
   pattern only if nothing matches.

8. **Report**: likely cause, evidence (codes and counts, not raw dumps), fix.
   For each fix say who does it: a setting change you can make after a yes, a
   UI path, or a host-side action the operator takes.

## Stop and ask

- Before any settings change: show field, current value, proposed value;
  wait for a yes.
- Before testing a connection to an indexer or download client: it contacts
  the remote service; say so, wait for a yes.
- Evidence points to a Scryer defect (scores contradicting the profile, a
  crash, a failed migration, an unparseable ordinary release name, a code
  path contradicting its own health check): stop diagnosing, offer
  `scryer-report-bug`, pass along the bundle directory and evidence.
- Cause outside Scryer (indexer account, client host, disk, network, reverse
  proxy): say what to check and stop.

## Token budget

Typical run: about 15k to 30k tokens of tool output.

- manifest summary: 0.5k
- 3 to 5 jq extracts: 2k to 5k
- log summary plus 1 to 3 searches of 40 lines: 3k to 8k
- 2 to 4 live queries, filtered: 2k to 6k
- one reference file: 2k to 4k

A single read over about 5k tokens: filter further with jq, `head` or a
narrower regex instead.
