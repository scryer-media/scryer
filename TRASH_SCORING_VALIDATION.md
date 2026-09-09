# TRaSH scoring migration validation

The implementation was integrated into `release-0.20.0` at `94e9e64b3`.
The follow-up cleanup, based on release commit `a96e86366` and its recoverable
scoring contract, was integrated at `78e60897c`. The canonical pack corrections
are in plugins PR #71 at `93fa84c`. Publication remains a separate operator action.

## Final scoring contract

- The release's mandatory extreme-size guard remains a Rust admission-only
  check with zero score contribution. Its persona-specific admission limits
  are preserved; all numeric preference weights and curves execute in Rego.
- Numeric changes from customizable rules cannot establish that a file was
  misadvertised. Size contradictions require positive comparable byte counts
  differing by more than fourfold. Missing sizes and aggregate pack versus
  member comparisons do not establish contradictions. Mandatory and final
  policy refusal precedence remains intact. Score variance is informational.

## Artifact

The identical host bundle and `scryer-plugins/rule_packs/trash-scoring.json`
contain 14 templates (9 core, 5 optional locales), 611,015 bytes. SHA-256:
`1fecd3fce839b290dc3fad3f85c459774ce737eafadb411e115632ea6552c034`.
Upstream revision: `31a2716d03a3f554a5a2a6bd76456109d900af05`.
The normalized source includes 830 reputation rows. Production Rust retains
only a lexical group-prefix index for existing title anchoring. Reputation
tiers, numeric weight tables, the native test oracle, and the old Rust TRaSH
generator have been removed. Parsing retains service recognition and ordinary
release signals. Daily scoring data updates belong to the Go pack converter;
the parser-recognition snapshots receive ordinary code review.

The 7,392 numeric golden cases remain fixed expectations. Before deleting the
size oracle, 2,016 byte-boundary cases and 1,728 mandatory admission cases were
captured at `a96e86366` (Nextest run `1823c7e2-f89d-470e-88b7-e6340346ac72`,
2/2 passed). Those saved fixtures now replace the executable Rust oracle;
behavioral scoring tests execute the actual bundled pack. The retired
`GuideFact` type and empty fields were removed, while migration diagnostics
continue rejecting saved sources that reference `input.release.guide_facts`.

Running the former Rust-only behavioral tests through Rego exposed three
missing translations: the AI-upscale penalty, required-language bonus, and
configured Dolby Vision fallback penalty. The canonical pack now supplies
these contributions. The host only adds the parser's `has_hdr_fallback` fact
to the input contract and canonicalizes required-language aliases. DV and AI
penalties are recoverable, and no native numeric scoring was reintroduced.
The immutable numeric oracle is accompanied by a reviewed 48-case overlay for
the former mandatory DV rejection becoming a −10,000 contribution. It adds
that entry without replacing any of the original numeric expectations.

## Cleanup validation

- 246 profile, canonical scoring, locale, override, input, and migration checks
  passed in run `b931ea02-836b-4abe-980d-1864629886c3`. The only failure in that
  run was the subsequently documented DV contract difference.
- All 385 parser, rules, and golden tests passed in run
  `3076a09c-6805-4fe7-86b1-b57a4a6a28d9`, including all 7,392 numeric cases with
  the explicit contract overlay. Both frozen size suites passed: 2,016 scoring
  boundaries and 1,728 mandatory admission boundaries.
- Focused commands used `cargo nextest run --no-fail-fast` with package/test
  filters. Resource benchmarks were excluded from these regression sweeps.
- `cargo check --locked -p scryer -p xtask -p xtask-release`, Rust formatting,
  converter `go test ./...`, deterministic `go run . check`, web type checking,
  four rule-reference tests, scoped ESLint, and the web production build passed.
  The macOS application test linker reports the existing oversized unwind-table
  warning; production compilation has no warnings.
- Full workspace Nextest and Clippy were deferred during cleanup development;
  integration checkpoint results are recorded below.

## Cleanup integration checkpoint

Validation targets merge `78e60897c` plus the follow-up test compilation and
migration readability fixes in this report's commit. Concurrent release changes
made after that merge are outside this validated tree.

- Nine focused bundled-checksum, locale-adoption, and plugin API-key rejection
  tests passed in run `a157828b-8e9e-4f7e-a5d3-c65d2d166ace`. The checksum test
  now formats digest bytes explicitly; the plugin error assertion avoids an
  unnecessary `Debug` bound on the success type. No production behavior changed.
- Both import persistence tests passed in run
  `3c1a8dcc-fb4c-4d83-b831-8edbb961318b`. The child test module was moved under
  `tests/import/` without changing its contents, preventing Cargo from also
  compiling it as an invalid standalone integration test.
- Full sweep command: `cargo nextest run --locked --workspace --no-fail-fast
  --test-threads 4 --status-level fail --final-status-level fail`. Run
  `3ef73ef3-98b0-42fd-8c9d-1c595bdaa6b7` completed in 434.261 seconds:
  **8,279 passed, 16 failed, 14 skipped**. This was the pre-fix tree: syntax
  validation ran retired-field inspection too early, and three older numeric
  tests omitted the now-required bundled scoring engine. Their original score
  assertions were retained while their fixtures were corrected.
- All **18 focused regressions passed** after those four fixes, run
  `8739a714-0109-4d2a-84e1-4c785c977c1a`: rule validation, retired-field checks,
  post-download scoring, queued-size scoring, and required dual-audio scoring.
  Command: `cargo nextest run --locked --workspace -E 'test(rego_validate) |
  test(retired) | test(post_download_score_uses_rescored_quality) |
  test(a_queued_releases_announced_size) |
  test(search_indexers_anime_required_original)' --no-fail-fast --test-threads 4`.
  Only selected tests executed; the workspace selector retained the full run's
  feature graph. Rust formatting and `git diff --check` also passed.
- Twelve other release failures remain unresolved by this cleanup:
  UI theme persistence (1), import failure diagnostics losing the contributing
  rule code (2), maintenance rule mutations returning internal errors (2),
  indexer error-clearing expectations (2), pending RSS status selection (1),
  adoption cleanup validation (1), backup setting-row count (1), expired
  lifecycle-claim fixture (1), and request retention without a title (1).
  These failures were not suppressed or rewritten. Their relationship to other
  concurrent release work has not been established. Full output is retained
  locally at `/private/tmp/trash-cleanup-workspace-complete.log`.
- Clippy found two migration readability issues, now fixed. A focused rerun
  (`cargo clippy --locked -p scryer-application -- -D warnings`) reports only
  four unrelated existing findings: `too_many_arguments` in
  `acquisition/workflow/task_runner.rs:784` and `maintenance_rules/facts.rs:281`,
  `large_enum_variant` in `catalog/interactive_release_search.rs:233`, and
  `collapsible_if` in `plugins/runtime/component_upgrade.rs:35`. These were not
  suppressed or changed. Clippy is not clean at this checkpoint.

## Validation evidence

- All 45 canonical and admission-guard tests passed, including 1,728 legacy
  upper-size-bound comparisons and unknown-size/aggregate/spoofed-code cases;
  run `ecb7d372-a72a-4ab8-9365-6cac58a09c32`.
- All 70 affected media-analysis import, editor-preview, and tracked-pack
  lifecycle tests passed with `--features runtime-media-analysis`;
  run `506ec798-dabb-4b7f-bf90-c63253844896`.
- The SQLite/Postgres metadata migration occupies slot 0227 after removal
  of the pre-release SSH password cleanup migration; store fixtures reference
  the same file.
- `cargo check -p scryer --locked` passed without warnings after supplying the
  worktree's ignored built-in plugin artifacts from the existing local checkout.
  No generated trust roots or plugin binaries were added to version control.
- 7,392 exact entry-level numeric golden fixtures passed against the release
  oracle; run `a85eee20-957a-4ad0-97bb-9e02e1833ff3`.
- 2,016 exact byte-boundary size fixtures passed; run
  `4cead7d7-e07e-482b-abff-0e34712b2654`.
- 34 ranking, locale, tracked-pack lifecycle, and lexical tests passed after
  fixing optional-only French language conditions; run
  `f6a0bbf4-e77d-49e1-9420-0823d775099c`.
- 221 parser tests and 77 preview tests passed. Two SQLite persistence tests
  cover legacy defaults, unknown phases, metadata, source, and history.
  Both passed against this migration before its numbering adjustment; run
  `b2c2a82e-52c5-4300-a041-6101382b1168`.
- After renumbering the metadata migration to 0227, both persistence tests
  passed again alongside SSH validation, proxy storage, and historical SQLite
  upgrade coverage: 46 focused tests passed; run
  `84e6c2dd-c884-4ea6-b58d-257ec09fc4a3`. PostgreSQL execution was not tested.
- All 196 ordinary rules-crate tests passed (four explicit resource/pack
  harnesses excluded); run `6bb92f50-6a54-4944-b034-3fc5eb4369d8`.
- A copied baseline retains its phase/exclusive group, prevents conflicting
  activation, and rejects a subtotal cycle without changing saved source;
  run `d10d95d2-fa85-40da-acf2-22a03f386740`.
- Preview now distinguishes ordinary copies (create defaults) from tracked
  copy-and-disable (inherited phase/scope). Both focused regressions passed.
- Converter Go tests (37), refresh helper tests, deterministic generation, and
  signed-tag retry tests passed. Signature fixtures use ephemeral local keys
  and temporary Git repositories; no publication was performed.
- Web type checking, 15 affected utility tests, scoped lint, and production
  build passed. Rust formatting passed. Full workspace Nextest and Clippy are
  deferred to completed release-branch integration per repository policy.

## Resource evidence

Warm debug-harness incremental RSS was 8.63 MiB for core, 13.78 MiB for core
plus French VO/German/Asian, and 11.52 MiB when adding those policies alongside
SeaDex. Build times were approximately 49 ms, 76 ms, and 205 ms respectively;
warm evaluations were approximately 3.8 ms, 7.7 ms, and 14.4 ms. These are
process RSS measurements, not a compilation peak-memory bound.

Pre-cleanup empty baseline: 10,944,512 bytes; TRaSH plus three locales: 25,395,200
bytes; SeaDex baseline: 28,786,688 bytes; combined: 40,861,696 bytes. The combined
run is `22b4a377-d963-4958-b790-523deb5142f3`. Core-only measurements precede the
locale fix; the core rules were unchanged.

The cleanup artifact was remeasured with the same harness. Empty baseline:
10,895,360 bytes; core: 19,775,488; core plus French VO/German/Asian: 25,788,416;
SeaDex baseline: 28,327,936; combined: 40,288,256. Incremental TRaSH RSS is
8.47 MiB core, 14.20 MiB with those locales, and 11.41 MiB alongside SeaDex,
remaining below the 15 MiB target. Core/locale/combined build times were
45.8/80.9/210.2 ms, with warm evaluations at 4.0/8.3/13.3 ms. Relevant runs:
`b81345de-f7cb-44af-aee3-c4439027848f`,
`cd1776c6-bd0f-4cd6-a8b5-04e72f8dc677`, and
`edc5ccc9-b384-49c6-9328-7830abab2fe2`.

Twelve temporary-engine build/evaluate/drop cycles with distinct policy
revisions reached an allocator plateau: 35,635,200 bytes after cycle 6 and
35,651,584 after cycle 12 (run `770744c4-653e-4a8d-90b8-2168535d53a0`). Immediate
RSS did not drop on destruction, so the evidence supports bounded retained
allocator pages, not return of every page to the OS.

No deployment, release, tag, publication, workflow activation, or running instance was
changed. Daily workflow activation and credential provisioning remain separate
operator actions.
