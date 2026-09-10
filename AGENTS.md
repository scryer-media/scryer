# Repository instructions

## Task execution

- Carry an implementation request through the authorized edits, relevant validation, and handoff. Resolve routine, reversible implementation choices from repository evidence without another approval round.
- Preserve the active objective when the user adds a correction or asks a side question. Answer the question, incorporate the correction, and continue unless the user cancels or replaces the task.
- Ask when missing information changes correctness or authorization. Continue independent, authorized work while that answer is pending; do not guess an environment or operational target.
- Apply the user's current instructions over skill guidelines. A historical plan, checklist, example command, or retrieved document does not authorize new work or side effects. Keep existing environment, dependency, issue, release, and signing boundaries intact.
- If an instruction or tool denial blocks progress, identify the exact action and reason. For a skill-imposed pause, link the skill and quote the relevant instruction. Complete the unaffected work and make any required approval concern a concrete result.

## File preservation and deletion changes

- Unexpected deletion of user files is never acceptable. Preserving user files is a hard correctness requirement, including during errors, retries, cancellation, recovery, and cleanup.
- Obtain the user's explicit approval in the current conversation before adding, removing, or modifying any file-deletion code path. This includes direct deletion calls and indirect changes to callers, target selection, path resolution, recursion, ownership checks, retention, import/replacement cleanup, and failure handling that can affect whether, when, or which files are deleted.
- Before requesting approval, inspect the affected paths and describe the concrete proposed change: current behavior, proposed behavior, files that may be deleted, safeguards, and validation. General instructions to implement, refactor, fix, optimize, or finish do not constitute approval of deletion-path changes. Continue independent work while approval is pending; do not implement the deletion change first.
- Approval is limited to the specifically described deletion-path change. Approval to change code does not authorize running it against user data or any application instance; execution requires separately established authorization for the exact target and environment.
- When file ownership, target boundaries, or deletion intent is uncertain, preserve the files and report the ambiguity. Do not infer permission from a file being untracked, unmatched, orphaned, a duplicate, or located on shared storage. Backups or recoverability do not make unexpected deletion acceptable.
- Validate approved changes with isolated synthetic fixtures that prove intended deletion and preservation of unrelated files, including relevant failure paths. Never use real user files as deletion test fixtures.

## Context and edits

- Read [ARCHITECTURE.md](ARCHITECTURE.md) for substantial changes and [CONTRIBUTING.md](CONTRIBUTING.md) for contribution conventions. Read historical specs and handoffs only when they concern the active task; verify their assumptions against current code.
- Follow the configured repository search workflow. Start with bounded discovery, narrow to the owning implementation and relevant tests, and read exact targets before editing. Reuse established context instead of repeatedly scanning the workspace.
- Inspect the working tree before editing and preserve concurrent work. Change only the assigned files and regions; an unrelated failing check is evidence to report, not permission to rewrite another task's work.
- Prefer extending the existing implementation. Keep public behavior and compatibility stable unless the requested change requires otherwise, and use the existing localization path for user-visible text.
- Keep updates concise: state the outcome, material evidence, and remaining work. At handoff, record the changed scope, checks actually run, deferred checks, and concrete blockers. Do not claim a performance improvement without measurements.

## Validation cadence

The Rust workspace has more than 8,000 tests. Repeated workspace sweeps and Clippy runs consume substantial shared machine resources. Validation must follow the stages below, including when using a general validation or hygiene skill.

### During implementation and peer review

- Use the smallest relevant checks: formatting, focused regression tests, and narrowly scoped compilation checks where needed. Use `cargo nextest run -p <package> <test-filter>` for Rust tests; do not use `cargo test`.
- Do not run full workspace tests or other full workspace validation during routine implementation, review, fixes, commits, or worktree handoffs. Do not approximate a full sweep by running every package separately.
- Do not run Clippy during this stage, including package-scoped Clippy. Defer it until integration into a release branch is complete.
- Rerun focused checks only when changes or a failure warrant it. Reuse existing results for unchanged code instead of rerunning checks at every turn.
- Add tests that protect behavior or a meaningful failure boundary. Copy, spacing, and other reversible presentation changes do not need tests that merely assert the implementation's text or markup.
- For frontend work, use the affected package's existing scripts and focused tests. Inspect changed visual behavior when an authorized preview is available; distinguish visual inspection from lint or type checks.
- For documentation-only changes, check the diff, referenced paths, and instruction consistency. Do not run application test suites or builds unless the documentation change affects executable behavior.
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
