# TRaSH scoring migration checkpoint

Worktree branch: `feature/trash-builtin-scoring`, based on release commit
`1911cb38b40c6a7b0ae10c4f3dfb06d673549426`. This includes the release's
recoverable-scoring contract. This checkpoint is not ready for integration.

## Open contract decisions

- The old mandatory extreme-size guard depends on persona-specific size
  expectations. Its replacement is pending a decision between retaining a
  small Rust admission-only helper and making that penalty recoverable in Rego.
  The current checkpoint does not yet retain that guard.
- Numeric changes from customizable rules cannot establish that a file was
  misadvertised. Two existing size-drift contradiction tests need a factual
  replacement or an explicitly informational variance contract. A direct size
  comparison also needs comparable coverage: aggregate pack announcements
  cannot be compared with a single member file.

## Artifact

The identical host bundle and `scryer-plugins/rule_packs/trash-scoring.json`
contain 14 templates (9 core, 5 optional locales), 610,175 bytes. SHA-256:
`6d6d573226605692e7878393eadce02806bd4b93cd924b7dce2be72d3edc733c`.
Upstream revision: `31a2716d03a3f554a5a2a6bd76456109d900af05`.
The normalized source includes 830 reputation rows. Production Rust retains
only a lexical group-prefix index for existing title anchoring; reputation
tiers and numeric weights are test-only migration oracles.

## Validation evidence

- 7,392 exact entry-level numeric golden fixtures passed against the release
  oracle; run `a85eee20-957a-4ad0-97bb-9e02e1833ff3`.
- 2,016 exact byte-boundary size fixtures passed; run
  `4cead7d7-e07e-482b-abff-0e34712b2654`.
- 34 ranking, locale, tracked-pack lifecycle, and lexical tests passed after
  fixing optional-only French language conditions; run
  `f6a0bbf4-e77d-49e1-9420-0823d775099c`.
- 221 parser tests and 77 preview tests passed. Two SQLite persistence tests
  cover legacy defaults, unknown phases, metadata, source, and history.
- Converter Go tests (32), refresh helper tests, deterministic generation, and
  signed-tag retry tests passed. Signature fixtures use ephemeral local keys
  and temporary Git repositories; no publication was performed.
- Web type checking, 15 affected utility tests, scoped lint, and production
  build passed. Rust formatting passed. Full workspace Nextest and Clippy are
  deferred until the contract decisions and integration are complete.

## Resource evidence

Warm debug-harness incremental RSS was 8.63 MiB for core, 13.47 MiB for core
plus French/German/Asian, and 11.36 MiB when adding those policies alongside
SeaDex. Build times were approximately 49 ms, 80 ms, and 250 ms respectively;
warm evaluations were approximately 3.8 ms, 7.4 ms, and 17.3 ms. These are
process RSS measurements, not a compilation peak-memory bound.

Twelve temporary-engine build/evaluate/drop cycles reached an allocator
plateau: 35,667,968 bytes after cycle 6 and 35,700,736 after cycle 12. Immediate
RSS did not drop on destruction, so the evidence supports bounded retained
allocator pages, not return of every page to the OS. Resource figures above
precede the small French-language fix and need the final measurement refresh.

No deployment, release, tag, push, workflow activation, or running instance was
changed. Daily workflow activation and credential provisioning remain separate
operator actions.
