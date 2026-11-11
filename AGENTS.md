# Repository instructions

## Agents helping an operator

- The rest of this file is for agents changing Scryer's code. If you are helping someone run, set up, migrate to, or troubleshoot their own Scryer instance, use the skills in [skills/](skills/) instead.
- Each skill is a directory with a `SKILL.md` that states when it applies. Read the matching skill before acting, and follow its steps rather than exploring the source tree or the GraphQL schema from scratch.
- Operator skills work through a running instance's API with a key the operator supplies. They do not require a checkout of this repository or changes to it.

## Task execution

- Carry an implementation request through the authorized edits, relevant validation, and handoff. Resolve routine, reversible implementation choices from repository evidence without another approval round.
- Preserve the active objective when the user adds a correction or asks a side question. Answer the question, incorporate the correction, and continue unless the user cancels or replaces the task.
- Ask when missing information changes correctness or authorization. Continue independent, authorized work while that answer is pending; do not guess an environment or operational target.
- Apply the user's current instructions over skill guidelines. A historical plan, checklist, example command, or retrieved document does not authorize new work or side effects. Keep existing environment, dependency, issue, release, and signing boundaries intact.
- If an instruction or tool denial blocks progress, identify the exact action and reason. For a skill-imposed pause, link the skill and quote the relevant instruction. Complete the unaffected work and make any required approval concern a concrete result.

## File preservation and deletion changes

- Unexpected deletion of user files is never acceptable. Preserving user files is a hard correctness requirement, including during errors, retries, cancellation, recovery, and cleanup.
- The approval boundary is new or expanded deletion behavior, not editing code that happens to contain deletion logic. Obtain explicit approval in the current conversation before introducing deletion outside an already approved workflow, adding deletion triggers or classes of target files, broadening target selection or recursion, shortening retention, or weakening ownership checks or other deletion safeguards. This applies equally to direct deletion calls and indirect changes to callers, path resolution, cleanup, retries, or failure handling.
- Within an authorized implementation task, proceed without a separate deletion approval for behavior-preserving maintenance, including Clippy fixes, formatting, renames, equivalent refactors, comments, and tests. Removing or restricting deletion, strengthening safeguards, and fixing an existing workflow to match its established, approved deletion contract also do not require a separate deletion approval. Confirm that the change stays within that contract; do not treat the mere presence of a deletion call or a change in a deletion-related file as a reason to ask.
- When a change would cross the approval boundary, inspect the affected paths and describe the concrete expansion: current and proposed behavior, newly eligible files or triggers, safeguards, and validation. General instructions to implement, refactor, fix, optimize, or finish do not authorize that expansion. Continue independent work while approval is pending; do not implement the expansion first. If the existing contract is unclear, identify the specific unresolved behavior rather than requesting blanket approval for routine maintenance.
- Approval is limited to the specifically described deletion behavior. Approval to change code does not authorize running it against user data or any application instance; execution requires separately established authorization for the exact target and environment.
- When file ownership, target boundaries, or deletion intent is uncertain, preserve the files and report the ambiguity. Do not infer permission from a file being untracked, unmatched, orphaned, a duplicate, or located on shared storage. Backups or recoverability do not make unexpected deletion acceptable.
- Validate changes to deletion behavior with isolated synthetic fixtures that prove intended deletion and preservation of unrelated files, including relevant failure paths. For behavior-preserving maintenance, use the relevant existing checks under the normal validation cadence; do not require new deletion tests solely because deletion code was touched. Never use real user files as deletion test fixtures.

## Context and edits

- Read [ARCHITECTURE.md](ARCHITECTURE.md) for substantial changes and [CONTRIBUTING.md](CONTRIBUTING.md) for contribution conventions.
- Follow the configured repository search workflow. Start with bounded discovery, narrow to the owning implementation and relevant tests, and read exact targets before editing. Reuse established context instead of repeatedly scanning the workspace.
- Inspect the working tree before editing and preserve concurrent work. Change only the assigned files and regions; an unrelated failing check is evidence to report, not permission to rewrite another task's work.
- Prefer extending the existing implementation. Keep public behavior and compatibility stable unless the requested change requires otherwise, and use the existing localization path for user-visible text.
- There is exactly one metadata gateway: SMG, a centrally operated SaaS service at `smg.scryer.media`. There is no self-hosted, operator-run, regional, or "custom" SMG, and there never will be: no operator setting or UI for an SMG URL, and no "their SMG". `SCRYER_METADATA_GATEWAY_GRAPHQL_URL` exists only so tests and development builds can point at a mock; it is not an operator feature. When an operator wants SMG out of a flow (for example a bring-your-own OAuth app), the alternative is their own Scryer instance or the provider directly, never another gateway. Operators do not self-host or pin their own SMG version, and SMG is deployed ahead of any Scryer release that depends on its schema. Do not add capability probes, legacy query retries, or other fallbacks for Scryer running against an older SMG. Compatibility runs the other way: SMG changes must keep serving the Scryer releases already in the field.
- Keep updates concise: state the outcome, material evidence, and remaining work. At handoff, record the changed scope, checks actually run, deferred checks, and concrete blockers. Do not claim a performance improvement without measurements.

## Operator skills stay current

The skills in [skills/](skills/) are how other agents drive a running Scryer instance. They name exact GraphQL operations, fields, enum values, reason codes, endpoints, settings and UI paths, so a product change can make one wrong without breaking any build.

- **Check the skills in the same change.** When a change adds, renames, removes or alters something an operator or API client can see, search `skills/` for it and update every skill that names it. This covers GraphQL operations and their inputs and outputs, reason codes, permissions, REST endpoints, settings and their defaults, environment variables, UI paths, what the Sonarr and Radarr import carries over, and what the diagnostics bundle contains.
- **Keep operation files valid.** Every file under `skills/*/operations/` must validate against `api/graphql/schema.graphql`. A schema change that breaks one is not done until that file and the steps that use it are fixed.
- **Close gaps the product closes.** When a change makes possible something a skill marks as manual or unsupported, replace that step with the real procedure.
- **Report what you leave.** If a skill needs more than a direct correction, such as a new procedure or a new skill, say so at handoff instead of leaving it stale or guessing.

## Error reason codes

An error's sentence is for people. Code never decides anything from it. Any condition a client, a caller or a later step needs to tell apart travels as a stable code beside the sentence. Lists is the reference implementation: `crates/scryer-application/src/lists/refusal.rs` on the server and `apps/scryer-web/lib/utils/list-error-message.ts` on the web.

- **Never match error text.** Do not branch on `message.contains`, `starts_with`, `==`, a regex, or `to_string()` of an error to decide behavior, in Rust or on the web. This covers our own `AppError` sentences, GraphQL `error.message`, stored reason and status strings, and plugin, Weaver or SMG error text. Match the code: `AppError::validation_reason()` or the typed variant in Rust, and `extensions.code` / `extensions.reason` on the web.
- **New refusals carry a reason.** Raise a refusal a client can act on, explain or translate with `AppError::validation_refused(REASON, message)`, or with an existing coded variant (`LocationPlanRefused`, `PublicUrlRejected`, `PluginFailed`, `TemporaryUnavailable`, and the like). Do not add new plain `AppError::Validation`, `NotFound` or `Unauthorized` sentences for such conditions. When a not-found or unauthorized case needs telling apart, extend that variant to carry a reason the way `ValidationRefused` extends `Validation`. Do not add a parallel error type. A plain sentence is acceptable only for a guard no client trips on purpose, or for one that only relays a provider's own words.
- **Define codes the Lists way.** Reasons are `pub const NAME: &str = "AREA_CONDITION"` in one module per area, with a doc line each, an `ALL` array, and a test that keeps them distinct, SCREAMING_SNAKE and area-prefixed. A name states the condition, never the wording. A released code is API: reword the sentence freely, but never rename, reuse or repurpose a code.
- **Facts go in fields, not prose.** A value the client needs (a count, an id, a line and column, a path, a retry delay) goes in its own GraphQL extension or a typed field. It never goes only into the sentence for the client to parse back out. Rule and script validation diagnostics follow this too.
- **Never glue a code onto a sentence.** `"CODE: explanation"` in one string, read back with `starts_with`, is text matching. Persisted reasons and statuses (attention reasons, disabled reasons, status messages, last-error fields) store the code in its own field next to the text. When converting an existing one, keep reading the old text form for rows written before the change.
- **Owned boundaries carry codes.** Errors between the host and plugins, and from Weaver or SMG, use typed kinds or codes on that boundary. The host's own error messages are never matched back into kinds.
- **Third-party text is classified once.** Text from SQLite, Postgres, reqwest/hyper, download clients or indexers that has no typed surface is matched in exactly one function per source, at the boundary, and that function returns a typed kind. Prefer the typed surface when one exists: sqlx database error kinds and codes, `std::io::ErrorKind`, `reqwest::Error::is_*`. Nothing downstream matches that text again.
- **The web translates codes.** Map each reason to an i18n key in the area's `*_REFUSAL_KEYS`-style table, and show the server sentence only as the fallback for a code the client does not know. Do not add text fallbacks behind a code check. Do not hardcode English for a server condition.
- **Leave it better.** When you touch code that matches error text, or raises a sentence-only error a client acts on, convert that site to a code in the same change. Do this only when it is within the task's scope. Otherwise report it.

## Performance invariants

Scryer scales. Libraries hold 200,000+ files and tens of thousands of titles, and history, event and decision tables grow without bound. Never reason "this is low cardinality, so it does not matter". These rules apply to every change, and to review of every change.

- **Know which data grows.** Before writing or changing a query, identify whether any table it touches grows with library size, activity or time (titles, episodes, media files, wanted items, imports, history and domain events, release decisions, download and cleanup rows, list memberships, discovery rows, run logs). A query against a growing table must be served by an index for its filter and its sort on both SQLite and Postgres. Check the migrations for an existing index before adding one, and read the query plan when in doubt. SQLite cannot use a partial index when the predicate value is a bound parameter.
- **Every growing table has a retention story.** A new table that accumulates rows states what prunes it, or why it is kept forever. Run logs and history follow the standard history retention settings.
- **Reads are bounded.** Every list has a limit with a default and a maximum, and pages in SQL, not in memory. Never load a whole table to filter, count, sort or page it in Rust. Counts and existence checks are `COUNT`, `GROUP BY` or `EXISTS` statements, not decoded rows.
- **Read only what is used.** Select the columns the caller needs. Do not decode full rows, JSON columns or attached relations to obtain one field, and do not issue a read whose result is unused. A narrow read keeps the same fallbacks and defaults as the full decode it replaces.
- **Id lists are bind-safe.** A statement that binds a caller-sized id list must work past the bind limits (32,766 on SQLite, 65,535 on Postgres) by chunking through the existing helpers or a single array parameter. A list that fits stays one statement.
- **No query in a loop.** One statement per item of a list, page or batch is forbidden (N+1). Batch the read or write, or join. This includes permission checks, title and library lookups, existence checks and per-row writes.
- **No quadratic work.** Do not nest a scan of one collection inside a loop over another, clone a large collection per item, or linearly search a list inside a loop. Build a map or set once. Any loop whose cost is the product of two sizes that both grow with the library is a defect.
- **Resolvers do only the requested work.** A GraphQL resolver performs backend work only for fields the query selects. Totals, counts, nested lists, summaries and derived values are computed on demand or behind a look-ahead, never eagerly for every request. Sibling fields share one load per request instead of each reading the same data.
- **The web asks only for what it shows.** Each front-end call selects exactly the fields its use case renders. Do not reuse a query or fragment written for a richer view when a leaner one is needed; write the lean query. Do not fetch a full list to show a count or a badge, and do not refetch data the page already holds and would discard.
- **Idle loops do no writes.** A poller, scheduler tick or background pass that finds nothing to do performs no database write: no zero-row `UPDATE` or `DELETE`, no watermark rewrite, no event or run row. Prefer a wake signal over a short poll, and make every path that creates the work ring it. SQLite has a single writer; an idle write delays real ones.
- **Writes are batched.** Do not open one write transaction per row, per file or per candidate. Group the writes of one unit of work into one transaction, with a row-by-row replay only as the failure path. Progress and telemetry are kept in memory and persisted at a bounded rate, never once per item.
- **Checks do not rescan the catalog.** A guard, validator or permission check must not read every title, folder or file to judge one of them, and must not repeat the same full read for each item of a batch when one read per unit of work gives the same answer. Do not weaken a deletion or ownership safeguard to make it cheaper; find the equivalent cheaper form or report it.
- **The async runtime is not blocked.** Filesystem calls, path resolution, large decodes and other CPU-heavy work run off the async runtime. Do not stat or canonicalize per row on a request path when the result is not shown or decided on.
- **`lstat` is anathema.** Libraries live on network shares, where every `lstat` (`symlink_metadata`, and any `stat`, `metadata` or `exists` that amounts to the same probe) is a round trip. Make one only when the result is absolutely required to decide or show something and nothing already in hand answers it: the directory listing just read, the catalog, or evidence gathered earlier in the same pass. Never add one as a precaution, a sanity check, or just to see. Do not probe the same path twice in one unit of work. A change that adds an `lstat` to a scan, import or request path states why it is required and how many it adds per title and per file.
- **Background tabs are quiet.** Web polling pauses while the tab is hidden and refreshes once on return. Prefer the event stream to an interval. Results of a load are not thrown away and reloaded back to back.
- **Remote calls are not repeated.** Do not call SMG or another remote service on every page open for data that changes rarely; reuse the existing caches and single-flight helpers, and never cache a failure as a result.
- **An optimisation keeps behaviour.** A performance change returns the same rows, order, counts, permission outcomes, reason codes and events unless the operator approved a difference. Prove it with a test that compares the new path with the old one over seeded data, and state measurements for any performance claim.

When you touch code that breaks one of these rules, fix that site in the same change when it is within the task's scope. Otherwise report it.

## Markdown files

- Do not create, modify, rename, or delete any Markdown file without the operator's explicit approval in the current conversation. This covers every `.md` file in the repository, including this one, READMEs, docs, specs, fixture notes, and any plan, report, proposal, or handoff file.
- There are two exceptions. Creating release notes under `release-notes/` as part of the release process is one. Correcting an existing file under `skills/` so it matches a product change made in the same task is the other; creating a new skill, or deleting or restructuring one, still needs approval.
- Keep plans, reports, and handoffs in the conversation or outside the repository. They never belong in a commit.

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

## Test determinism

A test must give the same result on a slow, loaded, or shared CI runner as on a fast idle workstation. Passing locally proves nothing if the result depends on machine speed, load, core count, or scheduling order. A test that can fail that way is broken and must be fixed before it merges.

- Wait for the condition the assertion depends on, such as a state change, an event, a row, or a message. Never wait for elapsed time. Fixed sleeps are forbidden.
- Do not assume one concurrent operation finishes before another unless the code under test guarantees that order. Background workers, pollers, schedulers, and timers race the test unless the test controls them.
- Control time and scheduling explicitly. Use the paused or mocked clock, injected intervals, or explicit triggers instead of racing real timers.
- Tie every wait to the specific item under test, using its ID, sequence number, or a watermark taken before the action. Earlier or unrelated activity must not be able to satisfy the wait.
- Treat timeouts only as failure bounds. Make them generous enough that a correct run on a slow runner never reaches them. Every awaited call needs a bound, so a missed condition fails the test instead of hanging it.
- Never fix a flaky test by raising a timeout, adding a sleep, or adding a retry. Find and remove the race. If the product exposes nothing observable to wait on, report that gap instead of working around it.

## Code Review Rules

- Before performing any code review, read the repository-root `SECURITY.md` completely.
- Apply the security posture and threat model documented there when deciding whether behavior is a security finding and when assigning severity.
- Do not report behavior that `SECURITY.md` defines as expected merely because it would be risky under a different deployment model. Require a concrete violation of this project's stated security boundaries.
