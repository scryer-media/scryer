# Specification Quality Checklist: Library Location, Folder Ownership, and Cross-Library Movement

**Purpose**: Validate [spec.md](../spec.md) for completeness, clarity, and internal
consistency before implementation planning is treated as final. Each item is a
yes/no check an author or reviewing agent can answer from the spec alone.
**Created**: 2026-08-30
**Spec**: `specs/0001-library-location-and-movement/spec.md`

## Re-run: 2026-09-01 (T094)

Walked against `spec.md` at tip `bfd553f09` and against the landed implementation.

**Marking convention.** A box is checked only when the item is answerable **yes**
from the spec, with a citation. Every checked item carries an evidence line naming
either a `spec.md` line anchor (spec-quality evidence, the item's own currency) or
a commit / test name / `file:line` (implementation evidence, added where the
implementation confirms or contradicts the spec). An item that is only partly true
stays **unchecked** with a gap note; per this checklist's own note, those are
recorded as **spec issues**, not implementation issues. Evidence classes used
below: `[spec]` line anchor, `[commit]` sha, `[test]` test name, `[code]`
`file:line`.

**Result: 32 checked, 8 gap-noted** (CHK003, CHK006, CHK009, CHK013, CHK022,
CHK025, CHK039, CHK040). None of the eight blocks a shipped behavior; six are
wording or coverage defects in the spec, one (CHK022) is unverifiable from the
repo, one (CHK006) records an unspecified scheduling decision the implementation
had to make.

### Addendum: 2026-09-01, later the same day (US3/US4/US5 landed)

The re-run above was walked while US3, US4, and US5 were unbuilt. All three have
since landed (see the coverage table, updated in place with the new evidence).
Two of the eight gap notes moved:

- **CHK003** — the spec question ("does US5 offer both modes?") is now answered:
  a Clarifications entry (Session 2026-09-01) records that consolidation offers
  **Move with Scryer only** and refuses adoption by name
  (`root_consolidation_mode_not_supported`, pinned by
  `graphql_a_consolidation_refuses_the_files_already_there_mode`). The box stays
  unchecked as a record that the original spec text was silent; the gap is
  closed by clarification, not by rewording US5.
- **CHK025 (related US8.4 note)** — the per-file asset listing landed
  (`d4ffd16cd`, `locationOperationAssets`) and T090 is checked; the persisted
  counters still merge media and asset dedups/renames, which the listing, not
  the counters, now disambiguates. The scenario-coverage defect in the main
  note (FR-069 multi-episode split) is unchanged.

### Implementation coverage at this re-run

Recorded here because several items below cite it. The checklist itself remains a
spec-quality instrument.

| Story | State | Evidence |
|---|---|---|
| US1 folder-match correction | shipped | `d044283f8`, `d67be5e3f`, `098788cf8` |
| US2 root move (same library) | shipped | `6b6e22b45`, `5243362a1`, `082783251`, `c42fcea57` |
| US3 adoption ("Files are already there") | shipped *(addendum)* | `cf9f92bcd` (matcher/verifier/executor branch), `18ec0b542` (web mode + accounting), `f5dc6a71b`, `09bb114c9` |
| US4 change root | shipped *(addendum)* | `17cd37374` (planner), `627b93377` (executor, epilogue seam, traveling bin), `b51f30973` (GraphQL + web) |
| US5 consolidate root | shipped *(addendum)* | `85fe8bbb1` (planner + executor, real merge handoff), `b51f30973` (GraphQL + web) |
| US6 cross-library transfer | shipped | `8d0b7020b`, `7ff14ca81`, `1c1a05cd9` |
| US7 merge | shipped | `fbfa942f9`, `3b117cc1b`, `913c72d97`, `477f278b9` |
| US8 monitor / cancel / resume | shipped | `6a89b7021`, `28bff508a`, `bfd553f09`; the asset-listing split landed after the re-run (`d4ffd16cd`, see the addendum and the CHK025 note) |
| US9 verification depth + backfill | shipped | `0874f57d5`, `911513e48`, `d5a6882ad`, `c42fcea57` |

Open product calls recorded against the items they touch: US6.3 mixed-source
end-to-end, ambiguous pick-a-candidate input, invalid-settings-kept-on-conversion,
live-vs-frozen merged-into name, FR-084 multi-source-root claims, source-folder
media-server refresh, PostgreSQL migration run pending, resume attribution
`SystemInternal`, resume-after-cancel fresh-preview.

## Requirement Completeness

- [x] CHK001 — Are all four user intentions (folder match, root move, root
      change/consolidation, library move) defined with non-overlapping boundaries?
      [Completeness, Summary + Boundaries]
      - **Pass.** `[spec]` spec.md:14–22 enumerates the four; spec.md:50–63
        draws the separating boundaries (folder match never moves files; root
        changes never cross libraries; cross-library is always title-scoped).
        `[code]` the six persisted operation types mirror the split
        (`location/model.rs:15-28`).

- [x] CHK002 — Does every location-changing surface route through a
      preview-and-confirm workflow, with the fileless catalog-only fast path as the
      sole stated exception? [Completeness, FR-011, FR-076]
      - **Pass.** `[spec]` FR-011 (spec.md:386) names the fileless fast path as
        the only carve-out; FR-076 (spec.md:582) defines it. `[commit]`
        `a302cfb57` closes the last non-preview door (`rootFolderId`), including
        the `addTitle` reuse branch. `[test]`
        `title_catalog::graphql_update_title_root_folder_id_is_refused_for_tracked_file_titles`.

- [ ] CHK003 — Are both execution modes (Move with Scryer / Files are already
      there) specified for every workflow that moves content? [Completeness,
      US2–US6]
      - **Gap (spec).** US2 (spec.md:108), US3 (spec.md:137–141) and US4
        (spec.md:164–166, "choosing managed move or external adoption") state
        their modes. **US5 consolidation never does**: neither the story
        (spec.md:192–215) nor FR-020–FR-028 mentions either mode, and FR-024's
        preview list has no mode language. The Summary's blanket claim
        (spec.md:24–27) is the only thing covering it. FR-020/US5 should state
        whether consolidation offers adoption at all.

- [x] CHK004 — Is the legacy `rootFolderId` API behavior explicitly retired for
      titles with tracked files, with the creation path preserved? [Completeness,
      FR-077, SC-009]
      - **Pass.** `[spec]` FR-077 (spec.md:585–588), SC-009 (spec.md:697–699).
        `[commit]` `a302cfb57`; the field is `@deprecated` in
        `api/graphql/schema.graphql:20256` and refusals carry
        `extensions.code = "DIRECT_ROOT_WRITE_RETIRED"`. `[test]`
        `lib_tests::title_updates::creating_a_title_still_assigns_the_requested_root_directly`
        pins the preserved creation path.

- [x] CHK005 — Are requirements present for all cross-cutting engines the stories
      depend on: verification (FR-040–047), preview/fingerprint (FR-080–082),
      concurrency (FR-083–086), collisions/dedup (FR-072–075)? [Completeness]
      - **Pass.** `[spec]` all four blocks present and populated: spec.md:458–482,
        596–611, 615–628, 559–578. `[commit]` each has a landed engine —
        `911513e48` (verify), `0f6ba5034` (preview), `4f35cc766` (ownership
        guard), `f5a3fc7db` (collisions/dedup).

- [ ] CHK006 — Is the background backfill job fully specified (trigger set,
      scheduling posture, skip rules, invalidation)? [Completeness, FR-046–047]
      - **Gap (spec).** Posture, skip rules and invalidation are specified
        (FR-047 spec.md:479–482; FR-046 spec.md:476–478). The **trigger set is
        not**: nothing in the spec says what starts the job (boot, interval, or
        on-demand), so the cadence was an implementation choice — a 30-minute
        Maintenance-category interval job
        (`jobs/definitions.rs:402`, `:344`). Record the cadence in FR-047 or
        state explicitly that it is an operator/implementation decision.

- [x] CHK007 — Is download-client copy verification included, not just library
      moves? [Completeness, FR-045]
      - **Pass.** `[spec]` FR-045 (spec.md:474–475). `[commit]` `d5a6882ad`
        routes the completed-download copy path through the same streaming
        CRC/BLAKE3 machinery and depth preference.

- [x] CHK008 — Are media kinds, series-movie links, and collections addressed at
      the movie/episodic boundary rather than left implicit? [Completeness,
      FR-060–062]
      - **Pass.** `[spec]` FR-060–FR-062 (spec.md:515–523) plus the Boundaries
        carve-out (spec.md:59–61). `[commit]` `7ff14ca81` lands the structural
        dispositions (`location/transfer_effects.rs`).

## Requirement Clarity & Testability

- [ ] CHK009 — Is every FR phrased so a test can pass or fail it without
      interpretation (single behavior, defined trigger, defined outcome)?
      [Clarity, Requirements]
      - **Gap (spec).** The large majority are. Two are not pass/fail testable as
        written: FR-026 (spec.md:426–428) — "SHOULD preserve the source root's
        relative folder layout **where practical**" has no defined outcome, and
        FR-013's "root moves **MAY** thereby repair stale folder names"
        (spec.md:392–394) leaves the repair optional while US2 scenario 2
        (spec.md:124–126) asserts it happens. Both belong to the unbuilt US4/US5
        phases, so no shipped behavior is ambiguous today.

- [x] CHK010 — Are "quick check" and "full verification" defined once in Product
      Language and used consistently (no unlabeled "verify")? [Clarity, Product
      Language, FR-042]
      - **Pass.** `[spec]` defined once at spec.md:47–48; FR-042 (spec.md:464–468)
        is the single behavioral definition and every downstream use is
        depth-qualified ("at the configured depth", FR-031/FR-044/SC-002).
        `[code]` the persisted stamp carries the depth and the fallback flag
        (`location/model.rs`, verification records).

- [x] CHK011 — Is the dedup identity bar unambiguous (full-file BLAKE3 only; the
      sampled proof can never justify deletion)? [Clarity, FR-073, D4 in plan]
      - **Pass.** `[spec]` FR-073 (spec.md:561–566) says "proven by matching
        **full-file BLAKE3** — never the sampled proof"; plan D4 (plan.md:84–85)
        makes the sampled proof a pre-filter only. `[commit]` `f5a3fc7db`.

- [x] CHK012 — Is "atomic from the user's perspective" for swap/takeover given a
      concrete failure contract (neither title partially changed)? [Clarity,
      FR-008]
      - **Pass.** `[spec]` FR-008 (spec.md:379–380) plus US1 scenario 6
        (spec.md:100–102). `[commit]` `d044283f8`; `[test]` the three US1
        acceptance gaps closed in `098788cf8`.

- [ ] CHK013 — Are the five bulk classification classes exhaustive and mutually
      exclusive, including the catalog-only class? [Clarity, FR-015, FR-076]
      - **Gap (spec) — internal inconsistency.** FR-015 (spec.md:397–400)
        enumerates exactly five classes and **omits catalog-only**, while FR-076
        (spec.md:582–584) requires catalog-only to be "classified distinctly …
        in bulk previews". The canonical list and the requirement disagree. The
        implementation ships six and is right (`location/classify.rs:29-46`,
        `TitleLocationClass::{CrossLibraryTransfer, RootMove, NoOp, CatalogOnly,
        Incompatible, NeedsResolution}`); FR-015 should be amended to six. US2
        scenario 4 (spec.md:130–132) and US6 scenario 3 (spec.md:237–240) inherit
        the same five-way wording.

- [x] CHK014 — Does every preview-related FR state what invalidates it
      (fingerprint scope, stale rules, resume carve-out)? [Clarity, FR-081,
      FR-089]
      - **Pass.** `[spec]` FR-081 (spec.md:605–608) names the invalidating
        inputs and puts the fingerprint over the full plan, not the sample;
        FR-089 (spec.md:634–636) carves out expected partial destination state.
        `[test]`
        `execution/tests.rs` `a_partial_left_by_a_crashed_copy_does_not_block_the_resumed_run`.

## Consistency & Conflict Checks

- [x] CHK015 — Do the verification requirements never contradict the scan
      requirements (scans quick-hash only; invalidation without computation)?
      [Consistency, FR-042 vs FR-046]
      - **Pass.** `[spec]` Boundaries (spec.md:62–63), FR-046 (spec.md:476–478,
        "scans never compute full hashes"), FR-042 (spec.md:464). No conflict.
        `[commit]` `d5a6882ad` implements scan-side invalidation without hashing.

- [x] CHK016 — Does the recycle-ordering rule (FR-087) reconcile with the
      fail-closed recycle allowlist and root retirement (FR-028)? [Consistency]
      - **Pass (spec).** FR-087 (spec.md:629–631) retires the root config only
        after all recycling completes and keeps an in-retirement root allowlisted
        on resume, which is exactly what FR-028 (spec.md:433–435) needs.
        *Unexercised*: root retirement belongs to the unbuilt US4/US5 phases, so
        this reconciliation has no implementation evidence yet.

- [x] CHK017 — Do merge rules and collision rules stay independent as stated
      (role resolution never driven by filename outcomes)? [Consistency, FR-068,
      FR-074]
      - **Pass.** `[spec]` FR-068 (spec.md:543–547, "per logical slot … not per
        filename") and FR-074 (spec.md:567–570, "apply role rules independently
        of the filename decision"). `[commit]` `fbfa942f9` keeps roles in
        `location/merge/roles.rs`, separate from `location/collisions.rs`.

- [x] CHK018 — Is "destination wins" applied uniformly across settings, media
      roles, pathnames, and canonical sidecars, with any exception called out?
      [Consistency, FR-063, FR-070, FR-072, FR-075]
      - **Pass.** `[spec]` settings FR-063 (spec.md:527), roles FR-070
        (spec.md:550), pathnames FR-072 (spec.md:559), episode/collection
        metadata FR-065 (spec.md:534). The one exception is stated: FR-075
        (spec.md:571–578) preserves the renamed incoming canonical sidecar while
        the destination's canonical file stays authoritative.

- [x] CHK019 — Does the folder-match workflow nowhere acquire move semantics
      (adopts existing folder; never recalculates or moves)? [Consistency, FR-014
      vs FR-013]
      - **Pass.** `[spec]` FR-014 (spec.md:395–396) explicitly exempts
        folder-match from FR-013's calculated destinations. `[code]` the US1 path
        persists no operation row and moves no bytes
        (`location/folder_match.rs`); `[test]` SC-001's byte-for-byte assertion
        in the T024 story tests (`098788cf8`).

- [x] CHK020 — Are Boundaries statements and FRs free of contradiction regarding
      series↔anime conversion and movie↔episodic rejection (including the
      series-movie carve-out)? [Consistency, Boundaries, FR-057, FR-060]
      - **Pass.** `[spec]` Boundaries spec.md:55–61 and FR-057 (spec.md:510),
        FR-060–FR-062 (spec.md:515–523) agree, and the Out of Scope entry
        (spec.md:751) matches. `[commit]` `7ff14ca81`.

## Acceptance & Scenario Coverage

- [x] CHK021 — Does every user story carry an independent test description and
      Given/When/Then scenarios covering its happy path and at least one failure
      or conflict path? [Coverage, User Scenarios]
      - **Pass.** All nine stories carry an "Independent test" line and at least
        one failure/conflict scenario: US1.6, US2.3, US3.2, US4.3, US5.2, US6.5,
        US7.4, US8.3, US9.3. *Nit (not a failure)*: US1, US2 and US4 carry a
        "Why this priority" rubric; US3, US5, US7, US8 and US9 omit it.

- [ ] CHK022 — Does every acceptance scenario from the original product plan map
      to at least one story scenario, FR, or edge case (no silent drops)?
      [Coverage, traceability]
      - **Not verifiable from the repository.** The source artifact — the
        "Operator product plan (2026-08-30)" named in spec.md:6 — is not in
        version control, so this mapping cannot be re-checked at re-run time.
        Either check the product plan (or an extract of its scenario list) into
        this spec directory, or accept that CHK022 is answerable only at
        authoring time by whoever held the plan. Left unchecked rather than
        marked on faith.

- [x] CHK023 — Are cancel, resume-after-restart, and stale-plan behaviors each
      covered by a scenario, not only by FRs? [Coverage, US8]
      - **Pass.** `[spec]` US8 scenario 2 (cancel, spec.md:295–297) and scenario
        3 (restart resume + stale-state error, spec.md:298–301). `[test]`
        `lib_tests/root_move.rs` `a_resume_is_refused_while_the_operation_still_has_a_live_runner`,
        `executor.rs` `a_cancel_stops_at_the_next_title_boundary_and_leaves_finished_titles_alone`
        (`28bff508a`, `bfd553f09`).

- [x] CHK024 — Is corruption detection tested at both depths, including the
      auditable reduced guarantee of quick mode? [Coverage, US9, SC-006]
      - **Pass.** `[spec]` US9 scenarios 1–3 (spec.md:317–324) and SC-006
        (spec.md:688–690) require the quick-mode stamp so the reduced guarantee
        is auditable. `[commit]` `911513e48` (injected-corruption unit tests),
        `c42fcea57` (T038/T044 story tests at both depths).

- [ ] CHK025 — Are merge scenarios sufficient to exercise every FR-063–071 rule
      (settings wins, unions, episode blocking, role matrix, multi-episode
      split)? [Coverage, US7]
      - **Gap (spec).** Four of the five named rules have a US7 scenario:
        settings-wins and unions (US7.2, spec.md:262–267), episode blocking
        (US7.4, spec.md:272–274), role matrix (US7.3, spec.md:268–271). The
        **multi-episode split (FR-069, spec.md:548–549) has no US7 acceptance
        scenario** — it appears only as an Edge Cases bullet (spec.md:340–341).
        The implementation does cover it (`location/merge/roles.rs`, `fbfa942f9`
        / `T086`), so this is a scenario-coverage defect in the spec, not a
        behavior gap.
      - **Related open item, US8.4 side:** FR-075 and US8 scenario 4
        (spec.md:302–304) require the final summary to list renamed and
        deduplicated **assets separately from media files**. The persisted
        counters still merge both into single `dedups` / `renames` fields
        (`location/model.rs`, `LocationOperationCounters`); the per-file listing
        that answers "which ones" is landing as
        `location/asset_listing.rs` — uncommitted and in flight at this re-run
        (T090). Mark T090 complete only once that lands.

## Edge Cases & Failure Modes

- [x] CHK026 — Hardlinked/seeding sources: detection, warning, and consequences
      specified? [Edge cases, FR-085]
      - **Pass.** `[spec]` FR-085 (spec.md:623–625) names all three consequences
        (broken link, doubled disk, recycling frees nothing); edge case at
        spec.md:336–337. `[commit]` `f5a3fc7db` (`location/hardlinks.rs`).

- [x] CHK027 — Recycle bin disabled/unavailable/rejecting: preserve+rename+warn
      path specified with no deletion fallback? [Edge cases, FR-073]
      - **Pass.** `[spec]` FR-073 (spec.md:563–566) — "never fall back to
        permanent deletion"; edge case spec.md:342–343; SC-003 (spec.md:680–682)
        makes it a success criterion. `[commit]` `f5a3fc7db`.

- [x] CHK028 — Stale/unavailable source mount during adoption: proceed and
      unresolved paths both specified? [Edge cases, FR-053, US3]
      - **Pass (spec).** `[spec]` FR-053 (spec.md:494–497) and US3 scenario 3
        (spec.md:155–157) specify the proceed path; US3 scenario 2
        (spec.md:152–154) and FR-052 specify the blocked/unresolved path; edge
        case spec.md:345–346. *Unexercised*: US3 is not built, so there is no
        implementation evidence.

- [x] CHK029 — Case-insensitive filesystems: collision behavior specified,
      including self-collision-as-rename? [Edge cases, FR-090]
      - **Pass.** `[spec]` FR-090 (spec.md:637–638) plus the explicit
        self-collision edge case (spec.md:353–354). `[commit]` `f5a3fc7db`
        (per-platform case rules in `location/collisions.rs`).

- [x] CHK030 — Crash mid-copy: partial destination state explicitly resumable
      rather than stale? [Edge cases, FR-089]
      - **Pass.** `[spec]` FR-089 (spec.md:634–636) and the edge case at
        spec.md:344. `[test]` `execution/tests.rs`
        `a_partial_left_by_a_crashed_copy_does_not_block_the_resumed_run` and
        `a_destination_left_unrecorded_by_a_crash_is_proven_and_the_move_continues`.

- [x] CHK031 — Unmanaged/unknown root content: listed, never deleted or abandoned,
      blocks retirement? [Edge cases, FR-027–028]
      - **Pass (spec).** `[spec]` FR-027 (spec.md:429–432) requires separate
        listing and forbids silent deletion; FR-028 (spec.md:433–435) blocks
        removal and limits automatic cleanup to empty directories after
        verification; US4 scenario 3 (spec.md:183–186) restates it.
        *Unexercised*: root retirement belongs to the unbuilt US4/US5 phases.

- [x] CHK032 — Unrelated same-folder-name titles: never merged; unique previewed
      name or blocked? [Edge cases, FR-025]
      - **Pass (spec).** `[spec]` FR-025 (spec.md:423–425) and US5 scenario 2
        (spec.md:207–209); FR-055 (spec.md:501–504) independently forbids
        same-name-without-identity auto-merge, and that half **is** implemented
        (`c98adde09`, `DestinationIdentityMatch::SameNameNoIdentity`). The
        folder-name uniquing half belongs to the unbuilt US5 phase.

## Non-Functional & Measurability

- [x] CHK033 — Are all success criteria measurable without naming implementation
      technology, and does each map to at least one story or FR? [Measurability,
      Success Criteria]
      - **Pass.** `[spec]` SC-001…SC-009 (spec.md:675–699) each carry an
        observable threshold, and each maps: SC-001→US1, SC-002→US2/US8,
        SC-003→FR-073/FR-075, SC-004→FR-081, SC-005→US6.3/FR-015, SC-006→US9,
        SC-007→FR-047, SC-008→US1.5/FR-007, SC-009→FR-077. *Nit*: SC-009 names
        the `rootFolderId` API field — acceptable, since that field **is** the
        user-visible contract being retired, but it is the only SC naming a
        surface.

- [x] CHK034 — Is the backfill job's non-interference requirement measurable
      (SC-007) and bounded (single-threaded, low priority, skip rules)?
      [Non-functional, FR-047]
      - **Pass.** `[spec]` SC-007 (spec.md:691–693) is stated as observable
        non-impact on scan/import/playback plus a never-touch rule for
        operation-owned files; FR-047 (spec.md:479–482) bounds it.
        `[commit]` `d5a6882ad`; `[test]` `lib_tests/full_hash_backfill.rs`.

- [x] CHK035 — Is preview scale addressed (complete counts, sampled items,
      fingerprint over full plan)? [Non-functional, FR-081]
      - **Pass.** `[spec]` FR-081 (spec.md:607–608) states all three explicitly.
        `[commit]` `0f6ba5034` reuses the rename-plan pattern
        (`location/preview.rs`).

- [x] CHK036 — Is the verification-depth trade-off user-visible end to end
      (setting → preview statement → per-file stamp)? [Non-functional,
      FR-042–043]
      - **Pass.** `[spec]` FR-042 (setting) and FR-043 (spec.md:469–471: preview
        statement, Activity, per-file record, fallback cases). `[commit]`
        `0874f57d5` (setting + GraphQL `verificationDepth`), `d5a6882ad`/`c42fcea57`
        (T040 end-to-end stamping).

## Dependencies & Assumptions

- [x] CHK037 — Is the in-flight relocation prototype acknowledged with a defined
      absorption stance instead of being duplicated or assumed shipped?
      [Dependencies, plan.md Prior & In-Flight Work, tasks T060]
      - **Pass.** `[spec]` Clarifications (spec.md:722–724); `[plan]` plan.md:58–69
        states the "absorb, don't duplicate" stance and the coordination
        requirement; `[tasks]` T060 restates the gate. **Still open in reality**:
        the prototype has not landed, so Phase 7 was deliberately not built and
        `LocationOperationType::RootChange` has no producer.

- [x] CHK038 — Are the two schema prerequisites (synthetic root ids, persisted
      full hashes) stated as requirements with migration-immutability respected?
      [Dependencies, FR-041, FR-078]
      - **Pass.** `[spec]` FR-078 (spec.md:589–592) says "via a new forward
        migration (shipped migrations are immutable)"; FR-041 (spec.md:461–463)
        requires persisted CRC + full BLAKE3 separate from the sampled proof.
        `[commit]` `74cf5c6c9` adds three new forward migrations
        (`0204_synthetic_root_ids`, `0205_media_file_full_hashes`,
        `0206_location_operations`) in both the SQLite and PostgreSQL sets; no
        shipped migration was edited. **Open**: the PostgreSQL migration run is
        still pending operator execution.

- [ ] CHK039 — Are all Assumptions genuinely assumptions (not disguised
      requirements), and is each Out of Scope item matched by a requirement or
      preview statement preventing silent scope creep (e.g., files-keep-names)?
      [Assumptions, FR-058]
      - **Gap (spec), first half.** Four of the nine Assumptions restate
        requirements rather than assume anything: spec.md:730–731 ≡ FR-063;
        spec.md:732–733 ≡ FR-068; spec.md:736–738 ≡ FR-073; spec.md:739–740 ≡
        FR-023 + FR-028. Duplicating a requirement in Assumptions creates two
        places to change one rule. Either delete them or convert them to
        cross-references.
      - **Second half passes.** Out of Scope items are matched: file renaming →
        FR-058's files-keep-names preview statement (spec.md:512–514); scrub
        feature → FR-047's narrow backfill scope; client checksums → FR-040's
        source-computed CRC; movie↔episodic → Boundaries + FR-060–062.

- [ ] CHK040 — Are permission requirements stated for every operation type,
      including bulk selections spanning multiple source libraries? [Dependencies,
      FR-083]
      - **Gap (spec) — wording narrower than the feature.** FR-083 (spec.md:615–616)
        says "management permission for **the source library** and every
        destination library" — singular — while FR-015 (spec.md:397–398)
        explicitly permits bulk selections spanning **several** source libraries.
        The implementation is broader than the spec and correct: every source
        library in the selection is checked
        (`location/operations.rs:1269-1272`, plus
        `require_location_operation_permission` at `:1004-1019` for
        cancel/resume). FR-083 should read "every source library and every
        destination library involved".

## Notes

- Run this checklist after any material spec edit; record failures as spec issues,
  not implementation issues.
- Traceability: FR ↔ story scenarios ↔ SC ↔ tasks (tasks.md references FRs per
  task; T081 produces the merge-inventory appendix this spec depends on for
  FR-064/FR-066 completeness — landed as `merge-inventory.md`, commit `9136c7992`).
- Re-run 2026-09-01 raised six spec-edit candidates: FR-015 six-class list
  (CHK013), FR-083 plural source libraries (CHK040), FR-069 US7 scenario
  (CHK025), US5 execution modes (CHK003), FR-047 trigger (CHK006), and the four
  duplicated Assumptions (CHK039). None were applied in this pass — this task
  records findings; amending the spec's requirement text is an operator decision.
