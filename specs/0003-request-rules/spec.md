# Feature Specification: Request Rules, Title Leases, and the Shared Policy Core

**Status**: Approved by the operator 2026-09-04. Implementation in progress on `feature/request-rules`.
**Plan**: [plan.md](./plan.md) · **Tasks**: [tasks.md](./tasks.md)
**Design authority**: RFC 137 (`scryer-docs/plans/137-policy-automation-maintenance-and-request-rules-rfc.md`) §4.3, §6.1, §9.13, §13, §17 future track. There is no external product oracle; RFC 137 and the maintenance family are the references.

## Problem

Media requests are either auto-approved by one per-library permission or wait for a human. Operators cannot express who may have what approved automatically, requesters learn the outcome only after submitting, nothing records why a request was approved or denied, and there is no way to ask for media for a limited time. Meanwhile the policy runtime that could carry this already exists twice (release scoring, maintenance rules) with duplicated engine code.

## User stories

- **US1 — Author request rules.** As a catalog administrator I write Rego rules that vote approve / manual / deny on a request, using facts about the requester, the target library, the requested profile and lease, and the title's metadata (certification, genres, ratings, popularity, adult flag, history), and I can preview a rule against a sample requester and title before enabling it.
- **US2 — Know before I submit.** As a requester I see in the request dialog whether my draft will be auto-approved, needs approval, or would be denied and why, and the banner updates as I change library, profile, or lease.
- **US3 — Lease a title.** As a requester I can ask for a title forever or for N days. An approved finite lease starts when the title first imports, is visible on the request, and the title stays protected from maintenance deletion while any lease or keep claim is live.
- **US4 — Clean up expired leases.** As an operator I enable a shipped maintenance template that removes titles whose leases have all expired, under the existing destructive gates, arming, and preview.
- **US5 — Tag automatically.** As an administrator my rules can stamp tags (for example `family`) on the title a request creates, and approvers see and can edit pending tags.
- **US6 — Explain decisions.** As an approver I see which rules voted what, with reasons, for every request, and the requester sees the deny reason.
- **US7 — One engine.** As a maintainer, release, maintenance, and request rules share one policy core; adding a family means writing an input document, a wrapper, and a decoder.

## Functional requirements

Policy core
- FR-001 A shared core owns engine construction, limits, wrapper loading, per-rule evaluation with error isolation, observation envelopes, host-derived holds, closed-output decoding, reason and tag bounds, content hashing, package rewriting, and contract-driven input-path validation.
- FR-002 Release scoring produces identical decisions after moving onto the core; `UserRulesEngine`/`UserRulesEvaluator` signatures, `rule_identity()`, and scoring fingerprints are unchanged.
- FR-003 Maintenance keeps its `unknown if` hold head; request uses `manual if`; the head name is a per-family parameter.

Request rules
- FR-010 Heads: `approve if`, `deny if`, `manual if`, `reasons contains`, `tags contains`. Per rule: manual > deny > approve; neither ⇒ abstain.
- FR-011 Arbitration across rules: deny > manual (incl. held, error) > approve. The existing per-library Auto-Approve permission is an approve vote and appears in the trace. No matching rule ⇒ manual review.
- FR-012 Engine error, timeout, unknown fact, or partial metadata never approves and never denies.
- FR-013 Rule sets have modes disabled / shadow / enforce and an instance gate defaulting off; shadow records the trace but changes nothing.
- FR-014 Rules reading `input.requester.*` require permission-management authority to author or preview.
- FR-015 The input surface is the one documented in plan.md §3 and `crates/scryer-rules/request-input-contract.json`; unknown paths fail validation with the Rules Context Reference wording.
- FR-016 Every evaluation (shadow or enforce, preview or submit) is explainable: outcome, fallback reason, per-rule votes, reasons, tags, input hash and schema version.

Pre-flight
- FR-020 A requester can evaluate a draft request without persisting it, seeing outcome, reasons, tags, and whether metadata was partial. Rule bodies are never returned.
- FR-021 Pre-flight and submit share one input builder and one metadata enrichment, so identical drafts yield identical decisions.

Snapshot
- FR-030 Requests store a versioned metadata snapshot of every field the rule surface can read, captured at submit; enrichment failures are recorded as partial rather than silently empty.

Leases and claims
- FR-040 A request carries a requested lease (forever or N days); an approver may change it; the approved lease is stored.
- FR-041 Approval creates a lifecycle claim: forever ⇒ permanent keep; finite ⇒ dormant retention that activates at the title's first import and expires `duration_days` later.
- FR-042 Any dormant or active retention or keep claim holds every destructive maintenance action on the title.
- FR-043 Maintenance facts expose lease state, latest expiry, active claim count, and keep-claim presence; a shipped template removes titles whose leases have all expired and that carry no keep claim.
- FR-044 Claims are released when the request is canceled or rejected before availability and when the title is deleted; administrators can extend, convert to permanent, or release a claim.

Tags
- FR-050 Tags emitted by rules are bounded (count, length, charset; `scryer:` prefix rejected), recorded on the request, shown in pre-flight and trace, and merged onto the created title at approval.

## Non-goals (v1)

No-code rule builder; rules altering the lease; per-library lease presets; requester-initiated extensions; joiner lease terms; Plex-watchlist origin; season/episode-scoped leases; root free-space facts; a maintenance-side tag action.

## Constitution check

See plan.md "Constitution Check".
