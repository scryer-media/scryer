# Scryer Project Constitution

**Version**: 1.0.0
**Ratified**: 2026-08-30

This document states the non-negotiable principles every feature specification,
implementation plan, and task list in `specs/` is written against. Each spec's
`plan.md` carries a **Constitution Check** that cites this document and records any
deviation with its justification; an unjustified deviation is a defect in the plan,
not a judgment call for the implementer. Principles are numbered for stable
reference (e.g., "C4").

## C1 — Migrations are immutable

A shipped datastore migration is never edited. Schema and data corrections happen in
new forward migrations, provided for both SQLite and PostgreSQL, with
dual-datastore tests. Read-side fixes are preferred when a migration's output can be
tolerated rather than rewritten.

*Rationale*: user databases have already run the shipped bytes; editing them forks
reality between installs.

## C2 — Preview before mutate

Any operation that changes user data at scale — files, catalog ownership, library
membership, bulk edits — presents a complete preview before executing. Previews are
fingerprinted; a change to the underlying state invalidates the confirmation and
requires regeneration. High-impact operations require an explicit confirmation
step; operations that can retire whole roots or libraries require typed
confirmation. Large previews return complete counts with sampled item lists, with
the fingerprint covering the full plan.

*Rationale*: users approve outcomes, not intentions; a preview that can drift from
execution is a lie.

## C3 — Nothing silent, nothing destroyed

No operation silently overwrites, deletes, merges, or omits. Destination content
wins pathname conflicts; incoming content is renamed or deduplicated, never
clobbered. The configured recycle bin is the only automatic destination for removed
user data; when recycling is unavailable, the fallback is preservation with a
visible warning — never permanent deletion. Content an operation cannot classify is
surfaced to the user, not skipped or discarded.

*Rationale*: media libraries are years of accumulated, often irreplaceable state;
every loss path must be explicit and user-chosen.

## C4 — Verified moves

Any operation that removes or recycles a source after copying it verifies the
destination first. Verification strength is auditable: the depth applied is
recorded with the operation and its per-file results. Identity claims that justify
deleting data (deduplication) require full-content hashing; sampled proofs are
sufficient only for copy-integrity checks and are the universal floor that
verification never drops below.

*Rationale*: the moment the source is gone, the destination's integrity is
unprovable in hindsight; prove it while both exist.

## C5 — Long-running work is asynchronous, observable, and resumable

Operations that outlive a request run as persisted jobs: accepted immediately,
visible in Activity with real progress, cancel-safe at defined checkpoints, and
resumable across process restarts without repeating verified work. Background jobs
that touch the whole catalog run single-threaded at low priority and yield to
user-facing work.

*Rationale*: users close browsers and restart servers; correctness cannot depend on
a session staying open.

## C6 — External compatibility is a contract

Integrations with download clients, indexers, media servers, and API consumers must
not regress against real-world implementations. Behavioral claims about an external
system are validated against the real system or a pinned oracle, not assumptions.
API surface changes are additive; retiring a behavior produces a typed, actionable
error, not silent reinterpretation. The GraphQL schema is the contract's source of
truth.

*Rationale*: Scryer sits in the middle of an ecosystem it does not control;
breaking a peer breaks the user.

## C7 — Platform differences are handled explicitly

Path normalization, case sensitivity, filesystem semantics (rename atomicity,
cross-device behavior, cache effects on read-back), and permission models differ
across Linux, macOS, and Windows. Code that touches the filesystem states its
per-platform behavior and is tested where the platforms diverge; previews must
match what the destination filesystem will actually do.

*Rationale*: "works on Linux" is where silent data loss on the other two platforms
comes from.

## C8 — Validation is targeted, gates are respected

Work packages run targeted tests for what they touch; one full pass happens at
final acceptance. Web changes pass the lint gate (typecheck and eslint) before
handoff. CI is the verification authority; local runs exist to keep CI green, not
to replace it.

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
