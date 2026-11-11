---
name: scryer-request-feature
description: Drafts a Scryer feature request for the operator to review and file themselves, after checking whether Scryer can already do it. Use when an operator asks for something Scryer does not seem to do, wants to suggest an improvement, or says another tool handles a case better.
---

# Request a Scryer feature

Turn the operator's need into a short request that matches the
repository's feature form. The operator files it. The agent does not
submit or post anything.

## Prerequisites

- None required. `scryer-connect` helps for step 1 when the request touches
  indexers, download clients or profiles.

## Rules

- Do not read the GraphQL schema or Scryer's source code.
- Keep the request about the operator's problem and use case, not a design.
- Never submit or post anything on the operator's behalf unless they
  explicitly ask in this conversation.

## Procedure

1. **Check whether it is already possible.** Spend at most a few reads:
   - Indexers, download clients, routing, quality profiles, delay
     profiles: the field tables in
     `../scryer-configure-acquisition/references/` list what can be set.
   - Release scoring and custom conditions: `scryer-write-rules`.
   - Moving from another media manager: `scryer-migrate-from-arr`.
   - If it exists, show the operator the setting or UI path and ask whether
     it solves the problem. If yes, stop. If partly, note what is missing
     for field 3.

2. **Route elsewhere when it is not a feature request.**
   - Something that should already work but does not: `scryer-report-bug`.
   - Open-ended ideas, design discussion or roadmap questions: point to
     `https://github.com/scryer-media/scryer/discussions`.
   - Help with setup: Discord, `https://discord.gg/SQmtZTanqm`.

3. **Ask for the problem**, in this order, one short question at a time,
   stopping when you have enough:
   - What are you trying to do, and what gets in the way?
   - How often does it happen, and what does it cost you?
   - What would you see if it worked?
   - What have you tried?

4. **Write the draft.** Load `references/feature-form.md` and write
   `./scryer-feature-request.md` with the four form fields, in order,
   nothing more. Show the operator the full draft.

5. **Revise** until the operator is satisfied.

6. **Hand over.** Tell the operator:
   - the draft path
   - the form: `https://github.com/scryer-media/scryer/issues/new?template=feature_request.yml`
     (title prefix `[Feature]: `)
   - that nothing has been posted

## Stop and ask

- After step 1, if an existing setting may cover the need.
- Before including any detail the operator did not give.
- If the operator asks the agent to file it: show exactly what will be
  posted and where, and wait for an explicit yes.
