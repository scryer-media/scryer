# Download-client lifecycle reliability — #204 and #205

Local implementation on `release-0.19.14`, based on `main` at
`af2a598ae870389c89db549f37cec4744d37b7a8` (0.19.13).
The worktree is `.worktrees/release-0.19.14` under the application repository.
No dependency, public GraphQL, plugin ABI, or settings changes are included.

## Behavior

- Cleanup follows verified import or the existing handled-failure policy.
  Completion in the client alone does not authorize deletion.
- SQLite and PostgreSQL migration 211 add cleanup intents separate from import
  history. Terminal state writes enqueue intent in the same transaction.
  Backfill discovers older handled downloads independently of recent client
  history. Pending work retains identity, payload checkpoints, attempts, retry
  time, and errors across restart.
- The runtime owner processes at most 100 candidates per pass with four
  concurrent workers. Selection interleaves clients; leases prevent duplicate
  claims and expire after five minutes. Retries back off from 30 seconds to
  five minutes. Settings and torrent retention policy are read again on retry.
- Weaver requires `deleteFiles: true` and does not silently downgrade when the
  schema rejects it. Plugin native data removal follows the reported capability.
  Host-managed cleanup resolves and checkpoints authoritative source paths
  before removing entries. Mapping, output-root, library, recycle-root, symlink,
  and job-directory guards remain enforced. SAB-compatible backend deletion
  flags are preserved after host payload cleanup.
- Missing entries do not prove payload deletion. Unverifiable payloads, reused
  locators, client failures, and unsupported required deletion remain pending.
  Keep, seeding hold, pause, and handoff retain their separate policy outcomes.
- Acquisition distinguishes presence, authoritative absence, and uncertainty
  for the original configured client. Recent-history omissions and partial
  failures preserve unresolved claims. Canonical submission admission refreshes
  evidence even after a healthy cached snapshot, including imported downloads
  with outstanding cleanup and legacy imports awaiting intent backfill.
- Weaver uses exact history lookup where supported; NZBGet uses complete
  listings; SAB uses exact job filtering and falls back to bounded history
  traversal. Completed-payload lookup advances through backend-capped pages
  and rejects repeated pages when a backend ignores offsets.
  qBittorrent, Transmission, and rTorrent plugin observations use complete
  retained-job listings. Other plugins can recover positive sightings from
  their full history export; missing items remain unknown when completeness
  cannot be established through the existing plugin contract.

The baseline is Sonarr's local checkout at
`5352e16d95a785aee1b760a7593857350c6a94ec`: verified import before cleanup,
repeatable terminal cleanup, client-native data deletion where supported,
host payload deletion before entry deletion elsewhere, and explicit torrent
removal eligibility. Durable retry discovery and acquisition retention during
client uncertainty intentionally strengthen that baseline.

## Validation

Independent peer review found and resolved these issues:

- P1: completed host payload deletion could repeat after an entry-removal
  failure. Retry now honors its checkpoint and suppresses both filesystem
  deletion and backend data-deletion flags, preserving recreated content.
- P2: a failed stop-seeding request could become completed cleanup. Failed
  pause requests now remain retryable until pause succeeds.
- P2: capped SAB history pages could strand completed-payload lookup. Lookup
  now advances to an empty page and rejects ignored pagination offsets.
- P2: SAB's bounded recent-history scan could prevent absence resolution
  forever with a large backlog. Validated exact job filtering now resolves
  real SAB absence independently of retained history size.
- P2: a recycle-path setting read failure could silently weaken deletion
  protection. The custom-path read now propagates errors and protects custom
  roots even when no library roots exist.
- P2: a total snapshot failure could escape as a nonretryable submission
  error. Admission now classifies that failure as unavailable and preserves
  the candidate for retry.

The independent final review found no remaining concrete P1/P2 in its reviewed
scope. Two existing claim tests were updated to the approved rule that omission
from recent history cannot retire an unresolved submission.

Rust tests used escalated local Nextest execution for fixture port binding.
The following batches overlap; counts must not be added as unique coverage.

| Batch | Result |
| --- | --- |
| Affected application lifecycle/acquisition, adapters, workflow store, and plugin adapter regressions | 449 passed |
| Follow-up seeding, queueing, store, and exact-observation regressions | 149 passed |
| Final manual import, recovered cleanup, host deletion, and outage admission regressions | 7 passed |
| Plugin adapter regressions after complete-listing observation changes | 33 passed |
| Imported-cleanup admission and persistence follow-up | 2 passed |
| Final formatted tree: outage admission, durable retry, and pre-backfill protection | 3 passed |
| Final peer-reviewed tree: affected lifecycle, acquisition, adapters, and persistence | 512 passed |

The main focused command was:

```sh
rtk proxy cargo nextest run -p scryer-application -p scryer-infrastructure-acquisition -p scryer-infrastructure-workflow -p scryer-plugins --no-fail-fast -E 'test(lib_tests::seeding_gate) | test(lib_tests::queueing) | test(lib_tests::acquisition_recovery) | test(task_runner_tests) | test(client_snapshot_tests) | test(canonical_context) | test(coverage::) | test(download_submission_store) | test(download_client_adapter) | test(weaver::tests::) | test(sabnzbd::tests::) | test(nzbget::tests::) | test(terminal) | test(restart_ghost)'
```

The final 512-test run above was
`bcac9a97-a1ee-411e-885a-9bbc87d5f8a6`; all 512 passed, 3685 were filtered out.

Formatting passed with `rtk proxy cargo fmt --all -- --check`, plus direct
Rustfmt of changed Rust include fragments. `rtk proxy git diff --check` passed.
Focused compilation checks passed for affected packages. The existing unused
imports warning in `import/workflow/tests.rs` remains unrelated to this change.

PostgreSQL execution is unverified: no local test server was available.
The PostgreSQL migration mirrors SQLite with native timestamp types, but this
does not substitute for executing the PostgreSQL fixture suite. Full workspace
Nextest and Clippy are deferred to the completed integration checkpoint under
the repository validation policy. No production, release, publication, or
GitHub issue operations were performed.

Implementation fingerprint, excluding this report: SHA-256 over sorted changed
paths and their contents, each separated with a NUL byte:
`f5efa6e92372520a2bfa9118ac4b113ffa0340e520fe7b3bc091e13867f28989`.
