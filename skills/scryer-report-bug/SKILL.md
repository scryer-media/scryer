---
name: scryer-report-bug
description: Drafts a Scryer bug report for the operator to review and file themselves, using facts from the diagnostics bundle and a privacy check before anything is shared. Use when an operator wants to report a Scryer bug, when troubleshooting points to a defect in Scryer, or when Scryer crashes, fails a migration or behaves against its own settings.
---

# Report a Scryer bug

Produce a local draft that matches the repository's bug form, check it for
sensitive data with the operator, and hand it over. The operator files the
issue. The agent does not submit, post or upload anything.

## Prerequisites

- Complete `scryer-connect` first if a bundle is still needed.
- A diagnostics bundle, unpacked. If `scryer-troubleshoot` already fetched
  one, reuse its directory. Otherwise run
  `sh "$S"/../scryer-troubleshoot/scripts/fetch-bundle.sh ./scryer-diag-1`.
- Local tools: `jq`, `grep`, `find`.
- Set `S` to this skill's directory (absolute path) and work in a separate
  directory the operator agrees to.

## Rules

- Do not read the GraphQL schema or Scryer's source code.
- Never print, log or paste the API key or any other secret.
- Never ask the operator to paste sensitive values into the conversation;
  use a file (step 6).
- Never submit, post, upload or attach anything on the operator's behalf
  unless they explicitly ask in this conversation. Even then, never attach
  the bundle to a public issue by default.
- Do not edit or delete files inside the bundle. Edit only the draft.

## Procedure

1. **Rule out what is not a bug.**
   - Suspected security vulnerability (auth bypass, exposure of secrets or
     data, code execution, privilege escalation): stop. It must never go in
     a public issue. Point the operator to the private advisory form at
     `https://github.com/scryer-media/scryer/security/advisories/new` and
     list what to include: Scryer version, steps to reproduce, impact and a
     proof of concept if available. Draft nothing public.
   - A usage, setup or installation question: point to the Discord support
     link `https://discord.gg/SQmtZTanqm` (the repository's issue-form
     contact link for support and installation help), or offer
     `scryer-troubleshoot`.
   - An idea or a design discussion: point to
     `https://github.com/scryer-media/scryer/discussions`, or use
     `scryer-request-feature` for a concrete request.
   - Otherwise continue.

2. **Collect version and deployment facts from the bundle.** Load
   `references/bug-form.md` → "Facts from the bundle" and run that jq
   extract. Do not ask the operator for anything it answers.

3. **Ask only for what the bundle cannot give:**
   - what happened (the bug) and what they expected
   - steps to reproduce, which you draft from the conversation and the
     operator confirms
   - for UI bugs only: browser and browser version

4. **Gather a short evidence summary.** Reuse codes and counts from
   troubleshooting if available. Otherwise take the top codes from
   `summary.json` sections that match the symptom and at most 20 filtered
   log lines (`sh "$S"/../scryer-troubleshoot/scripts/log-search.sh "$B" '<regex>' WARN 20`).

5. **Write the draft** to `./scryer-bug-report.md` in the layout from
   `references/bug-form.md`: the ten form fields, in order, nothing more.
   Show the operator the full draft.

6. **Run the sharing checklist.** Load `references/sharing-checklist.md`
   and go through every item: best-effort sanitizing, release and file names
   are kept, the manifest's `sanitization` section, the term scan with
   `scripts/scan-bundle.sh`, and where the bundle should and should not go.

7. **Revise** the draft for any matches or corrections the operator asks
   for, then show the final draft again.

8. **Hand over.** Tell the operator:
   - the draft path
   - the bug form: `https://github.com/scryer-media/scryer/issues/new?template=bug_report.yml`
     (title prefix `[Bug]: `)
   - that the bundle stays on their machine unless a maintainer asks for
     it, and that sending it is their choice
   - that nothing has been sent, posted or uploaded

## Stop and ask

- Before writing any value the operator did not supply or the bundle did
  not show.
- If the scan finds matches: before deciding what to do with them.
- If the operator asks the agent to file the issue or upload anything:
  show exactly what will be sent and where, confirm the destination, and
  wait for an explicit yes. Never include the bundle in a public issue
  without a separate, explicit yes for that.
