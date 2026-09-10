# Contributing to Scryer

Read [ARCHITECTURE.md](ARCHITECTURE.md) before making a substantial change. It
defines the durable system boundaries; this guide covers contribution workflow
and validation.

## Before You Start

- File bugs and feature requests through
  [GitHub Issues](https://github.com/scryer-media/scryer/issues).
- Keep changes focused. Exclude unrelated cleanup, dependency churn,
  formatting, generated-file churn, and refactors.
- Follow existing ownership and naming patterns. Add an abstraction only when
  it removes real complexity or enforces an established boundary.
- Preserve public behavior unless changing it is the stated goal. Update tests
  and user-facing documentation for intentional behavior changes.
- Do not bump versions, release, deploy, tag, or alter release artifacts unless
  a maintainer explicitly requests it.

## Pull Request Process

1. Open the pull request against `main`.
2. A maintainer will review it and respond with feedback on the request and its
   implementation.
3. If the request is accepted for potential adoption, a maintainer will name
   the release branch and direct you to retarget the pull request to it. Do not
   choose or create a release branch yourself.

Keep the pull request description current. Explain the problem, the chosen
boundary, behavior changes, migrations or compatibility effects, and the exact
validation performed. Call out anything relevant that was not run.

## Local Validation

Follow the [validation cadence in AGENTS.md](AGENTS.md#validation-cadence).
It defines when focused checks and full integration checks run, including for
automated contributors. Use the repository-owned task interface and current CI
to select the supported commands and feature configuration.

During implementation and review, run formatting and the smallest relevant
tests or compilation checks. Use `cargo nextest run -p <package> <test-filter>
--locked` for focused Rust tests; do not use `cargo test`. Do not run Clippy or
full workspace sweeps at this stage, or approximate a sweep package by package.
Reuse passing results for unchanged code.

After the planned release-branch integration batch is complete, one coordinator
runs the full validation once:

```bash
cargo fmt --all --check
cargo xtask ci clippy
cargo nextest run --workspace --locked --no-fail-fast
```

For frontend changes, use the affected scripts and focused tests from
`apps/scryer-web`. Select checks for the changed behavior; the full frontend
validation belongs at the completed integration checkpoint when relevant:

```bash
npm run lint
npm run check:react-compiler
npm run test:graphql-compat
npm run build
```

Use the checked-in package lock when installing dependencies. Documentation-only
changes need diff, link, and consistency checks rather than application builds
or test sweeps. Report the validation performed and the checks deferred under
the cadence; expected deferral does not block review.

Platform, migration, plugin SDK, release, and end-to-end changes have additional
tasks discoverable through `cargo xtask --help` and current CI.

## Tests And Test Data

- Add regression coverage for bug fixes and relevant failure, cancellation,
  retry, restart, and authorization paths.
- Test at the narrowest boundary that proves the contract. Use integration or
  end-to-end coverage when behavior crosses persistence, public interfaces,
  plugins, or external systems.
- Datastore changes require logical parity coverage for every first-class
  engine unless the behavior is explicitly engine-local.
- Prefer small deterministic synthetic fixtures. Sourced fixtures require
  known provenance and redistribution rights.
- Do not commit private libraries, downloads, NZBs, databases, credentials,
  logs, personal paths, or media collections.
- Performance claims require reproducible workloads, equivalent work, verified
  output, and enough measurements to support the claim.

## Persistence And Migrations

- Keep domain and application behavior independent of datastore-specific
  handles, records, and SQL.
- Implement durable schema and repository changes for every supported engine in
  the same pull request.
- Use transactions for multi-step mutations and preserve equivalent logical
  behavior across engines.
- Released migrations are immutable. Add a new migration for every correction
  or evolution.
- Consider backup, restore, upgrade, rollback, and restart behavior whenever a
  durable format or secret changes.

## Interfaces And Frontend

- Public operations describe product intent, not storage or implementation
  details.
- Keep business rules in the backend. Resolvers and HTTP handlers authorize,
  map, delegate, and return results.
- Map domain enums and public enums explicitly and exhaustively. Treat wire
  values as compatibility contracts.
- Update public contract documentation when runtime interfaces change.
- Keep the frontend a projection client. All user-visible text must use the
  existing localization path.
- Preserve bounded loading, error, empty, unauthorized, and reconnect states in
  user-facing workflows.

## Plugins And External Systems

- Extend plugins through reviewed host capabilities and versioned contracts;
  do not bypass sandbox, permission, trust, or resource limits.
- Keep external calls behind the established transport and integration
  boundaries. Preserve timeout, rate-limit, retry, cancellation, and
  observability behavior.
- Never log or expose passwords, API keys, tokens, private plugin material, or
  decrypted secrets.
- Third-party code, fixtures, schemas, and generated material require clear
  origin and license compatibility.

## AI-Assisted Contributions

- The human submitter owns and must understand every change. AI cannot review
  itself or claim validation that the submitter did not run.
- Disclose substantive AI assistance and describe the human review and testing.
- Ground architecture, protocol, persistence, security, and GraphQL behavior in
  repository evidence and authoritative references, not model memory.
- Check the origin and license of generated or suggested material. Do not send
  secrets, production data, private NZBs, databases, logs, restricted fixtures,
  or unpublished security reports to AI services.
- AI tools must not bypass hooks, signing, branch protection, safety checks, or
  maintainer approval. Releases, deployments, real-data migrations, and
  destructive actions require explicit authorization and human supervision.

## Repository Policy

Enable the versioned hooks after cloning:

```bash
git config core.hooksPath .githooks
```

The hooks scan staged changes for secrets, local usernames, and personal paths.
Do not bypass them. Satisfy commit and tag signature requirements.

Contributions must be original or compatible with [LICENSE](LICENSE). Keep
generated output, local configuration, credentials, logs, and machine-specific
paths out of commits.
