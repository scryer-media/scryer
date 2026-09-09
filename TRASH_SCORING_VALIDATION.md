# TRaSH scoring migration validation

Worktree branch: `feature/trash-builtin-scoring`, based on release commit
`1911cb38b40c6a7b0ae10c4f3dfb06d673549426`. This includes the release's
recoverable-scoring contract. Implementation remains in isolated host and
plugin worktrees; release integration and publication are separate actions.

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
contain 14 templates (9 core, 5 optional locales), 610,145 bytes. SHA-256:
`3eb2d9287ae6e783e36d2c912f78d15ef5078b793ed8b79b8cea24adbb3bc72b`.
Upstream revision: `31a2716d03a3f554a5a2a6bd76456109d900af05`.
The normalized source includes 830 reputation rows. Production Rust retains
only a lexical group-prefix index for existing title anchoring; reputation
tiers and numeric weights are test-only migration oracles.

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

Latest empty baseline: 10,944,512 bytes; TRaSH plus three locales: 25,395,200
bytes; SeaDex baseline: 28,786,688 bytes; combined: 40,861,696 bytes. The combined
run is `22b4a377-d963-4958-b790-523deb5142f3`. Core-only measurements precede the
locale fix; the core rules were unchanged.

Twelve temporary-engine build/evaluate/drop cycles with distinct policy
revisions reached an allocator plateau: 35,635,200 bytes after cycle 6 and
35,651,584 after cycle 12 (run `770744c4-653e-4a8d-90b8-2168535d53a0`). Immediate
RSS did not drop on destruction, so the evidence supports bounded retained
allocator pages, not return of every page to the OS.

No deployment, release, tag, push, workflow activation, or running instance was
changed. Daily workflow activation and credential provisioning remain separate
operator actions.
