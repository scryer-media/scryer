# Scryer Project Constitution

**Version**: 1.1.0
**Ratified**: 2026-08-30 · **Amended**: 2026-08-30

This document states the non-negotiable principles every feature specification,
implementation plan, and task list in `specs/` is written against. Each spec's
`plan.md` carries a **Constitution Check** that cites this document and records any
deviation with its justification; an unjustified deviation is a defect in the plan,
not a judgment call for the implementer. Principles are numbered for stable
reference (e.g., "C4"). Principles state ends, not mechanisms — a spec chooses the
mechanism and defends it in its plan.

## C1 — Migrations are immutable

A shipped datastore migration is never edited. Schema and data corrections happen
in new forward migrations, provided for every supported datastore, with tests
across all of them. Read-side fixes are preferred when a migration's output can be
tolerated rather than rewritten.

*Rationale*: user databases have already run the shipped bytes; editing them forks
reality between installs.

## C2 — Preview before mutate

An operation that changes user data at scale — files, catalog state, settings with
broad reach, bulk edits — shows the user what will happen before it happens, and
executes only what was shown. A confirmation is bound to the state it was granted
against: if the underlying state changes, the confirmation is void and the preview
regenerates. Confirmation friction scales with blast radius — routine changes
confirm simply; operations that can eliminate large amounts of user state demand
deliberate, unambiguous consent.

*Rationale*: users approve outcomes, not intentions; a preview that can drift from
execution is a lie.

## C3 — Nothing silent, nothing destroyed

No operation silently overwrites, deletes, merges, or omits user data. Automatic
removal is permitted only into a recoverable holding area; when recovery is
unavailable, the fallback is preservation with a visible warning — never permanent
deletion. Whatever an operation cannot classify or account for is surfaced to the
user, not skipped or discarded. Every loss path is explicit and user-chosen.

*Rationale*: media libraries are years of accumulated, often irreplaceable state.

## C4 — Destruction requires proof

Before an operation discards one copy of user data on the claim that another copy
suffices, it proves the surviving copy first — while both still exist. The strength
of evidence scales with irreversibility, and the verification actually performed is
recorded so the guarantee given is auditable afterward.

*Rationale*: the moment the source is gone, the survivor's integrity is unprovable
in hindsight.

## C5 — Long-running work is asynchronous, observable, and resumable

Work that outlives a request runs as a persisted job: accepted immediately, visible
to the user with real progress, cancel-safe at defined checkpoints, and resumable
across process restarts without repeating completed, verified steps. Background
maintenance yields to user-facing work.

*Rationale*: users close browsers and restart servers; correctness cannot depend on
a session staying open.

## C6 — External compatibility is a contract

Integrations with external systems — download clients, indexers, media servers,
API consumers — must not regress against real-world implementations. Behavioral
claims about an external system are validated against the real system or a pinned
oracle, not assumptions. Public API changes are additive; retiring a behavior
produces a typed, actionable error, not silent reinterpretation.

*Rationale*: Scryer sits in the middle of an ecosystem it does not control;
breaking a peer breaks the user.

## C7 — Platform differences are handled explicitly

Filesystem and OS semantics differ across Linux, macOS, and Windows. Code whose
correctness depends on platform behavior states that behavior explicitly and is
tested where the platforms diverge; what the user is shown must match what their
platform will actually do.

*Rationale*: "works on Linux" is where silent data loss on the other platforms
comes from.

## C8 — Validation is targeted, gates are respected

Work packages run targeted tests for what they touch; one full pass happens at
final acceptance. Every change clears the repository's lint and typecheck gates
before handoff. CI is the verification authority; local runs exist to keep CI
green, not to replace it.

*Rationale*: broad local sweeps burn time without adding assurance; gates catch
what targeted runs miss.

## C9 — Version-control discipline

Implementation happens in isolated worktrees on gitflow-prefixed branches
(`feature/`, `bugfix/`, `hotfix/`) cut from the current release tip. Commits are
signed. Releases run exclusively through the repository's release tooling —
hand-performed release steps are reverted and redone through tooling.

*Rationale*: a shared history everyone can trust requires provenance and a single
release path.

## C10 — Specs govern

Feature work of consequence is specified before it is planned, and planned before
it is tasked, under `specs/NNNN-name/` (`spec.md`, `plan.md`, `tasks.md`,
`checklists/`). Specs state WHAT and WHY without implementation detail; plans state
HOW with verified code anchors; tasks are independently executable with explicit
dependencies. Added complexity — new subsystems, new dependencies, schema changes —
is justified in the plan or removed.

*Rationale*: the spec is the durable artifact; code and contributors churn around
it.

## Governance

- Amendments are commits to this file with a version bump: patch for wording,
  minor for a new or materially expanded principle, major for a removal or
  reversal.
- A spec may not weaken a principle; it may only record a justified, scoped
  deviation in its plan's Constitution Check.
- When a principle and an existing behavior conflict, the principle wins for new
  work; migrating the old behavior gets its own spec.
