---
name: scryer-migrate-from-maintainerr
description: Migrates Maintainerr rule groups into Scryer lifecycle rules. Starts Scryer's own Maintainerr import through the GraphQL API, which reads a running Maintainerr and saves one disabled lifecycle rule per rule group, then reports each group and walks the operator through previewing the rules before any is armed. Use when an operator wants to "move from Maintainerr", "import my Maintainerr rules", bring over Maintainerr collections or cleanup rules, or replace Maintainerr with Scryer's lifecycle rules.
---

# Migrate from Maintainerr

Scryer connects to Maintainerr itself, reads every rule group, translates each
into Rego on the server and saves **one disabled lifecycle rule per group**.
Runs as a background job (job key `EXTERNAL_RULE_IMPORT`). The web UI's
**Migrate from Maintainerr** button on the lifecycle rules page runs the same
import.

```sh
export MAINTAINERR_URL=http://maintainerr.example.lan:6246
C=path/to/skills/scryer-connect; M=path/to/skills/scryer-migrate-from-maintainerr; O=$M/operations
gql() { "$C/scripts/gql.sh" "$@"; }
```

## Rules for this skill

- Maintainerr is only read. Any Maintainerr change is made by the operator in
  Maintainerr's UI.
- Read-only steps run directly. Every Scryer write: show a short summary
  (counts, names), wait for a yes.
- Never delete anything: not Maintainerr data, media, titles or rules. Never
  arm a rule that deletes; the operator does that, after review.
- Read jq projections only, never a whole response or a rule's source.
- Branch on `extensions.reason`, a run or item `reason` and fact `code`, never
  on message text.

## Prerequisites

- `scryer-connect` done: a working API key.
- The key's account has `MANAGE_CATALOG_SETTINGS` to start and read the run.
- A rule group that reads facts about specific people (who watched, who
  favourited, who requested, a user's plays) saves only for an account that
  also has `MANAGE_PERMISSIONS`. Without it that group is `FAILED` with
  `LIFECYCLE_PERSON_TARGETING_PERMISSION_REQUIRED`; the others still save.
- Maintainerr's URL **as Scryer reaches it** (in a container, `localhost` is
  the container), with any base path, no trailing slash. Maintainerr has no
  API key; send `apiKey: null`.

## 1. Start the import (side effect: confirm first)

Tell the operator: every rule group becomes a **disabled** lifecycle rule named
`(Maintainerr) <group name>`, on the default schedule, for every library;
nothing acts until they arm a rule. Wait for a yes. `requestKey` is any unique
string you keep: resending the same key and input after a lost response
returns the same run.

```sh
jq -n --arg key "maintainerr-$(date +%s)" --arg url "$MAINTAINERR_URL" \
  '{requestKey: $key, input: {source: "MAINTAINERR", baseUrl: $url, apiKey: null}}' > mt-vars.json
gql "$O/StartMaintainerrRuleImport.graphql" mt-vars.json > mt-run.json
jq -c .data.startExternalRuleImport mt-run.json
```

## 2. Wait for the run to settle (read-only)

```sh
jq '{runId: .data.startExternalRuleImport.id}' mt-run.json > mt-status-vars.json
until gql "$O/MaintainerrRuleImportStatus.graphql" mt-status-vars.json > mt.json &&
      jq -e '.data.externalRuleImportStatus.active == false' mt.json >/dev/null; do
  jq -c '.data.externalRuleImportStatus | {phase, counts}' mt.json; sleep 5
done
jq -c '.data.externalRuleImportStatus | {status, reason, counts, facts: [.facts[].code]}' mt.json
```

Phases: `QUEUED`, `FETCHING`, `PLANNING`, `SAVING`, `FINISHED`. Run `status`
is `COMPLETED` when every group is `SAVED` or `SKIPPED_EXISTS`, `WARNING` when
some group saved untranslated or failed, `FAILED` when the run stopped short.
A non-null run `reason` means the run stopped; see `references/codes.md`.
Only the account that started the run can read it (`RULE_IMPORT_NOT_OWNED`).

## 3. Report each group (read-only)

```sh
jq -r '.data.externalRuleImportStatus.items[] | [.state, (.savedName // .sourceName), (.reason // "")] | @tsv' mt.json
jq -c '.data.externalRuleImportStatus.items[] | {name: .sourceName, codes: [.facts[] | "\(.details.severity // "-") \(.code) \(.details.ruleIndex // "")"]}
  | select(.codes | length > 0)' mt.json
```

By `state`:

- `SAVED`: translated. Name `(Maintainerr) <group name>`. Warnings in the
  facts say where the rule differs from the group; tell the operator each one.
  `maintainerr.fact.approximate` and `maintainerr.action.approximate`: Scryer's
  fact or action differs from Maintainerr's, in a way that can only match
  fewer titles. `maintainerr.username.unknown`: no Scryer user has that name,
  so the rule matches nothing for that user until one does.
  `maintainerr.scope.all_libraries`: the group was tied to one media-server
  library, the rule applies to every library (step 4).
- `SAVED_FAILED_TRANSLATION`, reason `RULE_IMPORT_GROUP_UNTRANSLATABLE`: the
  group could not be translated. Saved as a disabled starter rule named
  `(Maintainerr) FAILED - <group name>` whose description lists what did not
  translate; its source is the editor's starter, not the group's conditions.
  Facts with `details.severity` `ERROR` say why, `details.ruleIndex` (from 0)
  which Maintainerr rule. A comparison that Scryer's facts could make match
  more titles than Maintainerr's fails the group this way too (the
  `*_widens` codes). Rewrite it with `scryer-write-rules`, or delete it in the
  web UI, as the operator chooses.
- `SKIPPED_EXISTS`, reason `RULE_IMPORT_ALREADY_EXISTS`: a rule named
  `(Maintainerr) <group name>` or `(Maintainerr) FAILED - <group name>` already
  exists. This covers a re-run, the same name twice in one source, and two
  imports running at once (one saves, the other skips). Nothing written; the
  existing rule untouched. To import that group again the operator renames or
  deletes the old rule first.
- `FAILED`: nothing saved. The item `reason` is the save's own refusal, such as
  `LIFECYCLE_PERSON_TARGETING_PERMISSION_REQUIRED` (prerequisites) or
  `LIFECYCLE_QUALITY_PROFILE_NOT_FOUND`; `RULE_IMPORT_INTERRUPTED` when Scryer
  stopped mid-run (start again: saved groups are skipped).

`SAVED_NEEDS_REVIEW` is never produced by this import.

What carries over and what does not: `references/what-carries-over.md`.
Every diagnostic code: `references/codes.md`.

## 4. Review before anything acts (side effects: confirm each)

Imported rules are disabled. Review each before it is armed, a rule that
deletes above all (`declaredActions.destructive`):

```sh
gql "$O/ImportedLifecycleRules.graphql" | jq -c '.data.lifecycleRuleSets[]
  | select(.name | startswith("(Maintainerr) ")) | {id, name, state, graceDays, destructive: .declaredActions.destructive, heads: .declaredActions.heads}'
gql "$O/Libraries.graphql" | jq -c '.data.libraries[] | {id, name, facet}'
```

1. **Scope.** Every imported rule applies to every library. A group tied to one
   media-server library carries a `maintainerr.scope.all_libraries` warning;
   if the operator wants that scope, they set the rule's libraries in the web
   UI (lifecycle rules page, edit the rule).
2. **Preview** (read-only, acts on nothing) per rule and library:

   ```sh
   jq -n --arg r "<ruleSetId>" --arg l "<library id>" '{input: {ruleSetId: $r, libraryId: $l, limit: 50}}' > pv.json
   gql "$O/PreviewImportedLifecycleRule.graphql" pv.json | jq -c '.data.previewLifecycleRule.titles[]
     | select(.outcome != "NO_MATCH") | {titleName, subjectLabel, outcome, dueAt, errorCode}'
   ```

   `MATCH` would be acted on after the grace period (`dueAt`); `UNKNOWN` is
   held because a fact could not be observed; `errorCode` set means the rule
   failed for that subject. Compare the matches with what Maintainerr's
   collection held; report differences only.
3. **Dry run**, with a yes: `SetImportedLifecycleRuleDryRun.graphql`
   (`{"ruleSetId": "<id>"}`). The rule records candidates on its schedule and
   never acts; the operator watches them on the lifecycle **Candidates** page.
4. **Arming** is the operator's step, in the web UI. A rule that deletes asks
   them to confirm its revision, library scope and current candidate count.
   Imported deleting rules carry a cap of 10 titles per run; never raise it on
   their behalf.

Turn off the matching Maintainerr rule group before arming its Scryer rule, so
the two never act on the same title.

## When something fails

- `gql.sh` exit 1 prints `code [reason]`; look the reason up in
  `references/codes.md`, then `scryer-connect/references/errors.md`.
- `MFA_STEP_UP_REQUIRED`: see `scryer-connect`. Ask the operator; do not work
  around it.
