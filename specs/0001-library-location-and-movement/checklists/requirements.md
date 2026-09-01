# Specification Quality Checklist: Library Location, Folder Ownership, and Cross-Library Movement

**Purpose**: Validate [spec.md](../spec.md) for completeness, clarity, and internal
consistency before implementation planning is treated as final. Each item is a
yes/no check an author or reviewing agent can answer from the spec alone.
**Created**: 2026-08-30
**Spec**: `specs/0001-library-location-and-movement/spec.md`

## Requirement Completeness

- [ ] CHK001 — Are all four user intentions (folder match, root move, root
      change/consolidation, library move) defined with non-overlapping boundaries?
      [Completeness, Summary + Boundaries]
- [ ] CHK002 — Does every location-changing surface route through a
      preview-and-confirm workflow, with the fileless catalog-only fast path as the
      sole stated exception? [Completeness, FR-011, FR-076]
- [ ] CHK003 — Are both execution modes (Move with Scryer / Files are already
      there) specified for every workflow that moves content? [Completeness,
      US2–US6]
- [ ] CHK004 — Is the legacy `rootFolderId` API behavior explicitly retired for
      titles with tracked files, with the creation path preserved? [Completeness,
      FR-077, SC-009]
- [ ] CHK005 — Are requirements present for all cross-cutting engines the stories
      depend on: verification (FR-040–047), preview/fingerprint (FR-080–082),
      concurrency (FR-083–086), collisions/dedup (FR-072–075)? [Completeness]
- [ ] CHK006 — Is the background backfill job fully specified (trigger set,
      scheduling posture, skip rules, invalidation)? [Completeness, FR-046–047]
- [ ] CHK007 — Is download-client copy verification included, not just library
      moves? [Completeness, FR-045]
- [ ] CHK008 — Are media kinds, series-movie links, and collections addressed at
      the movie/episodic boundary rather than left implicit? [Completeness,
      FR-060–062]

## Requirement Clarity & Testability

- [ ] CHK009 — Is every FR phrased so a test can pass or fail it without
      interpretation (single behavior, defined trigger, defined outcome)?
      [Clarity, Requirements]
- [ ] CHK010 — Are "quick check" and "full verification" defined once in Product
      Language and used consistently (no unlabeled "verify")? [Clarity, Product
      Language, FR-042]
- [ ] CHK011 — Is the dedup identity bar unambiguous (full-file BLAKE3 only; the
      sampled proof can never justify deletion)? [Clarity, FR-073, D4 in plan]
- [ ] CHK012 — Is "atomic from the user's perspective" for swap/takeover given a
      concrete failure contract (neither title partially changed)? [Clarity,
      FR-008]
- [ ] CHK013 — Are the five bulk classification classes exhaustive and mutually
      exclusive, including the catalog-only class? [Clarity, FR-015, FR-076]
- [ ] CHK014 — Does every preview-related FR state what invalidates it
      (fingerprint scope, stale rules, resume carve-out)? [Clarity, FR-081,
      FR-089]

## Consistency & Conflict Checks

- [ ] CHK015 — Do the verification requirements never contradict the scan
      requirements (scans quick-hash only; invalidation without computation)?
      [Consistency, FR-042 vs FR-046]
- [ ] CHK016 — Does the recycle-ordering rule (FR-087) reconcile with the
      fail-closed recycle allowlist and root retirement (FR-028)? [Consistency]
- [ ] CHK017 — Do merge rules and collision rules stay independent as stated
      (role resolution never driven by filename outcomes)? [Consistency, FR-068,
      FR-074]
- [ ] CHK018 — Is "destination wins" applied uniformly across settings, media
      roles, pathnames, and canonical sidecars, with any exception called out?
      [Consistency, FR-063, FR-070, FR-072, FR-075]
- [ ] CHK019 — Does the folder-match workflow nowhere acquire move semantics
      (adopts existing folder; never recalculates or moves)? [Consistency, FR-014
      vs FR-013]
- [ ] CHK020 — Are Boundaries statements and FRs free of contradiction regarding
      series↔anime conversion and movie↔episodic rejection (including the
      series-movie carve-out)? [Consistency, Boundaries, FR-057, FR-060]

## Acceptance & Scenario Coverage

- [ ] CHK021 — Does every user story carry an independent test description and
      Given/When/Then scenarios covering its happy path and at least one failure
      or conflict path? [Coverage, User Scenarios]
- [ ] CHK022 — Does every acceptance scenario from the original product plan map
      to at least one story scenario, FR, or edge case (no silent drops)?
      [Coverage, traceability]
- [ ] CHK023 — Are cancel, resume-after-restart, and stale-plan behaviors each
      covered by a scenario, not only by FRs? [Coverage, US8]
- [ ] CHK024 — Is corruption detection tested at both depths, including the
      auditable reduced guarantee of quick mode? [Coverage, US9, SC-006]
- [ ] CHK025 — Are merge scenarios sufficient to exercise every FR-063–071 rule
      (settings wins, unions, episode blocking, role matrix, multi-episode
      split)? [Coverage, US7]

## Edge Cases & Failure Modes

- [ ] CHK026 — Hardlinked/seeding sources: detection, warning, and consequences
      specified? [Edge cases, FR-085]
- [ ] CHK027 — Recycle bin disabled/unavailable/rejecting: preserve+rename+warn
      path specified with no deletion fallback? [Edge cases, FR-073]
- [ ] CHK028 — Stale/unavailable source mount during adoption: proceed and
      unresolved paths both specified? [Edge cases, FR-053, US3]
- [ ] CHK029 — Case-insensitive filesystems: collision behavior specified,
      including self-collision-as-rename? [Edge cases, FR-090]
- [ ] CHK030 — Crash mid-copy: partial destination state explicitly resumable
      rather than stale? [Edge cases, FR-089]
- [ ] CHK031 — Unmanaged/unknown root content: listed, never deleted or abandoned,
      blocks retirement? [Edge cases, FR-027–028]
- [ ] CHK032 — Unrelated same-folder-name titles: never merged; unique previewed
      name or blocked? [Edge cases, FR-025]

## Non-Functional & Measurability

- [ ] CHK033 — Are all success criteria measurable without naming implementation
      technology, and does each map to at least one story or FR? [Measurability,
      Success Criteria]
- [ ] CHK034 — Is the backfill job's non-interference requirement measurable
      (SC-007) and bounded (single-threaded, low priority, skip rules)?
      [Non-functional, FR-047]
- [ ] CHK035 — Is preview scale addressed (complete counts, sampled items,
      fingerprint over full plan)? [Non-functional, FR-081]
- [ ] CHK036 — Is the verification-depth trade-off user-visible end to end
      (setting → preview statement → per-file stamp)? [Non-functional,
      FR-042–043]

## Dependencies & Assumptions

- [ ] CHK037 — Is the in-flight relocation prototype acknowledged with a defined
      absorption stance instead of being duplicated or assumed shipped?
      [Dependencies, plan.md Prior & In-Flight Work, tasks T060]
- [ ] CHK038 — Are the two schema prerequisites (synthetic root ids, persisted
      full hashes) stated as requirements with migration-immutability respected?
      [Dependencies, FR-041, FR-078]
- [ ] CHK039 — Are all Assumptions genuinely assumptions (not disguised
      requirements), and is each Out of Scope item matched by a requirement or
      preview statement preventing silent scope creep (e.g., files-keep-names)?
      [Assumptions, FR-058]
- [ ] CHK040 — Are permission requirements stated for every operation type,
      including bulk selections spanning multiple source libraries? [Dependencies,
      FR-083]

## Notes

- Run this checklist after any material spec edit; record failures as spec issues,
  not implementation issues.
- Traceability: FR ↔ story scenarios ↔ SC ↔ tasks (tasks.md references FRs per
  task; T081 produces the merge-inventory appendix this spec depends on for
  FR-064/FR-066 completeness).
