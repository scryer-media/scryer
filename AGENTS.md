# AGENTS Instructions

Architecture documentation lives in a separate repo: `github.com/scryer-media/scryer-docs`.

For code discovery in this repo, use the `agent-context` MCP first if your
environment provides it. Start with `list_scopes`, select the repo scope that
corresponds to this repository, and use a workspace/group scope for cross-repo
searches when available. Use `search_symbols` for exact definitions and
`search_code` for broader semantic/hybrid discovery. Treat shell search as
follow-up refinement only after MCP has identified the relevant files, unless
you are doing a narrow exact-string confirmation. If MCP is unavailable in your
environment, fall back to `rg` or other local search tools.

Use this order when making architectural decisions:

1. Confirm the requested change matches existing module boundaries.
2. Validate route/view and component boundaries in the frontend before touching UI files.
3. Validate layer boundaries in the backend before touching crates.
4. Keep changes consistent with existing ownership and naming conventions.

Only update README files when the user explicitly asks for README changes.

If runtime interfaces change (GraphQL, subscription payloads, gateway contract), update API contract documentation in the scryer-docs repo.

## Release requests

- External release authority is per action, never standing. Do not infer it from
  earlier “approved” messages, signer unlocks, prior release requests, existing
  credentials, or an API reporting that the current user *can* approve.
- Before any irreversible external action—including pushing a release branch or
  tag, approving a protected GitHub Actions deployment, publishing to a
  registry, or triggering a catalog release—state the exact action and target
  and obtain an explicit approval in the current turn. A reply only authorizes
  the immediately described action; it does not authorize follow-up patches,
  additional versions, retries that publish, or related releases.
- Treat a source commit, a signed tag, a GitHub Actions deployment approval,
  and registry publication as distinct actions. Approval to create or sign one
  does not authorize any of the others. If a validation failure requires a new
  patch or version, stop and obtain fresh approval before publishing it.
- Release automation goes through `cargo xtask`.
- The root `xtask` binary is intentionally thin: `cargo xtask release ...`,
  `cargo xtask builtins sync`, `cargo xtask sdk release ...`, and
  `cargo xtask migrations rebaseline ...` delegate to the heavier
  `xtask-release` / `xtask-migrations` packages behind the same CLI.
- Direct aliases `cargo xtask-release -- ...` and
  `cargo xtask-migrations -- ...` exist for advanced debugging, but the
  canonical operator interface stays `cargo xtask ...`.
- For app releases, run `cargo xtask release --dry-run` first. Treat it as a
  mutating rehearsal: it may apply and commit release-prep fixes, but it must
  still stop before the Cargo version bump, signed tag, and push. Only follow
  with the real `cargo xtask release` if the dry run succeeds and the release
  should proceed.
- If the user explicitly asks for a release or tag, treat that as approval to
  commit the in-scope dirty tree for that release with a sensible commit
  message before invoking the release flow.
- Do not fold unrelated dirty files into that prep commit. If the tree contains
  unrelated user changes, stop and report the blocking paths.

## Runtime troubleshooting quickstart (Docker Compose logs)

For local stack checks, use this sequence:

1. `docker compose -f docker-compose.dev.yml ps`
   Confirm service status and current container names.

2. `docker compose -f docker-compose.dev.yml logs --tail=200 scryer`
   Inspect service startup/runtime logs.

3. `docker compose -f docker-compose.dev.yml logs --tail=200 nodejs`
   Inspect frontend logs.

4. `docker compose -f docker-compose.dev.yml logs --tail=200 nzbget`
   Check backing service health when app errors indicate upstream failures.
