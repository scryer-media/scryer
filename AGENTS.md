# Repository instructions

## Validation cadence

The Rust workspace has more than 8,000 tests. Repeated workspace sweeps and Clippy runs consume substantial shared machine resources. Validation must follow the stages below, including when using a general validation or hygiene skill.

### During implementation and peer review

- Use the smallest relevant checks: formatting, focused regression tests, and narrowly scoped compilation checks where needed. Use `cargo nextest run -p <package> <test-filter>` for Rust tests; do not use `cargo test`.
- Do not run full workspace tests or other full workspace validation during routine implementation, review, fixes, commits, or worktree handoffs. Do not approximate a full sweep by running every package separately.
- Do not run Clippy during this stage, including package-scoped Clippy. Defer it until integration into a release branch is complete.
- Rerun focused checks only when changes or a failure warrant it. Reuse existing results for unchanged code instead of rerunning checks at every turn.
- Report which focused checks passed and which workspace checks are deferred. Deferral under this policy is expected and is not a blocker to review or integration.

### After integration is complete

- Run full workspace validation once after the planned integration batch is complete and integration conflicts and fixes are resolved. Do not run it after each individual merge in the batch. Merely working on a release branch does not mean integration is complete.
- Run Clippy only at the completed release-branch integration checkpoint. Follow the repository's supported feature configuration when selecting Clippy flags.
- Assign expensive validation to one coordinating agent. Agents and worktrees sharing the machine must not launch duplicate or overlapping full sweeps or Clippy runs.
- Use `cargo nextest run --workspace --no-fail-fast` for the full Rust test sweep, or the repository-prescribed equivalent that reports the complete failure set. Request escalated execution on the first local Nextest run when fixture servers require port binding.
- Record the validated commit or tree, commands, results, skips, and blockers so later turns can reuse that evidence. A merge that preserves the already validated tree does not require another full sweep.
- If validation finds failures, reproduce and fix them with focused checks first. Repeat broad validation only when subsequent changes invalidate the previous result; do not repeatedly run the full suite while diagnosing a failure.

An explicit user request for an earlier or additional validation run overrides this cadence. Do not infer such a request from ordinary instructions to implement, review, fix, finish, or merge work.

## Code Review Rules

- Before performing any code review, read the repository-root `SECURITY.md` completely.
- Apply the security posture and threat model documented there when deciding whether behavior is a security finding and when assigning severity.
- Do not report behavior that `SECURITY.md` defines as expected merely because it would be risky under a different deployment model. Require a concrete violation of this project's stated security boundaries.
