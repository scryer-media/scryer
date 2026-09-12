# scryer-vNEXT — request rules

Notes for the request-rules track (spec `specs/0003-request-rules/`). Fold this
section into the next release's notes file when the version is cut.

Scryer can now decide media requests with rules you write, tell a requester what
will happen before they submit, and keep a requested title for a fixed period
that maintenance rules must respect.

## Highlights
- **Request rules.** A new Rules pane (Automation → Rules → Request) where an
  administrator writes rules that vote approve, needs-approval, or deny on a
  request, using facts about the requester, the target library, the requested
  quality profile and lease, and the title's own metadata (certification,
  genres, ratings, popularity, adult flag, request history). A deny always
  beats an approve, an unevaluable rule never approves, and a request nobody's
  rule matched takes the path it took before.
- **Know before you submit.** The request dialog now shows what will happen to
  the request as you change library, profile, monitoring or lease: approved
  automatically, needs approval, or would be denied and why. It is a courtesy
  rather than a gate — a request the rules would deny can still be submitted, so
  the decision is recorded and the requester can read the reason afterwards.
- **Leases.** A requester can ask to keep a title forever or for a number of
  days. The clock starts when the title first imports, not when the request is
  approved, and the approver can override the period when they approve.
- **Maintenance respects a lease.** While any lease or keep claim on a title is
  live, maintenance rules will not delete it: the attempt is recorded as held,
  with the reason, instead of removing the media. A shipped maintenance
  template, "Expired request leases", cleans up titles whose leases have all
  expired, under the existing destructive gates, arming and preview.
- **Automatic tags.** A rule can stamp tags (for example `family`) on the title
  an approved request creates. Approvers see the pending tags before they
  approve and can edit them in the approve dialog.
- **Explainable decisions.** Every evaluated request keeps a decision trace.
  Approvers see which rules voted what, with reasons; a requester sees the
  outcome and the reason codes for their own request, and never the rule
  internals.
- **Retention actions.** A manager can extend a title's retention, make it
  permanent, or release it with a recorded reason.

## What an administrator has to turn on
Nothing here is armed by default, and there are three separate switches:

1. **Experimental features** (Settings → General) — the Rules → Request pane is
   hidden until this instance-wide switch is on.
2. **The rule's own mode** — every rule is created disabled. *Shadow* records
   what it would decide and changes nothing; *Enforce* lets its verdict resolve
   the request. Changing a rule's mode needs catalog-settings management.
3. **The instance-wide request-rule gate** — until this is on, rules still
   evaluate and still record their verdicts, but requests resolve exactly as
   they did before. This gate lives under system-settings management, which is
   deliberately a different permission from the one the authoring pane needs.

Writing a rule that reads facts about *people* (`input.requester.*`) also needs
permission-management authority, because it uses the instance's identity
records. Rules about content only need catalog-settings management. The rule
editor ships five starter templates and a reference for every fact a rule can
read.

**Tags a rule emits have to exist first.** A title only ever carries labels an
administrator defined in Settings → Tags, and a request rule is held to the same
bar: a label that is not in the registry is dropped when the request is decided,
so it never reaches the title, the pending request's tag chip, or the approve
dialog. The decision trace still lists it, which is where to look when a tag you
expected did not appear, and the rule preview names the undefined labels while
you are still writing the rule. This matters for the shipped
`named-requesters-family-rated` template, which emits `family`: define that tag
before arming the rule. Renaming a tag follows pending requests but never
rewrites a rule's source, so the rule keeps emitting the old label until you
edit it; the rename dialog counts how many request rules that affects.

Tags also compose with maintenance rules, on purpose. A request rule that emits
`remove` plus the shipped `tagged-for-removal` maintenance template means the
titles that rule approves are deleted once the template's grace period elapses.
That is a supported arrangement rather than an accident, but it is a destructive
one: pick the label deliberately, and check what else already carries it.

## Upgrade notes
- This release adds database migrations for request rule sets and revisions,
  decision traces, and lifecycle retention claims, plus additive columns on
  media requests for the requested and approved lease.
- The GraphQL API change is additive: new queries, mutations and types, plus
  optional input fields and new fields on existing request payloads. No existing
  field changed shape or was removed.
- Existing requests are unaffected. A request submitted before this release has
  no decision trace and no lease, and renders as it always did.
