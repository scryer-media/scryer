# Indexer correctness review

Target: `release-0.19.13` at `dd37b3c2db60c9140563383e78e5cd4bb4df1627`.
Implementation branch: `bugfix/indexer-release-correctness` in a separate worktree.
`release-NEXT` at `81e8de958` was inspected as a compatibility reference and was
not modified. Its different proxy model and additional artifact consumers are
covered in [the NEXT integration note](indexer-correctness-next-integration.md).

## Correctness contract

- Observed calls count Scryer HTTP dispatch attempts after local validation,
  cooldown, and pacing. A failed response or timeout still counts; a rejected
  local dispatch does not. Retries and redirect hops are individually observed.
- Indexer dashboard totals count the configured indexer origin plus one request
  per challenge-solver POST made on that indexer's behalf, because the solver
  replays the request upstream and the indexer's quota pays for it. External
  redirect destinations remain auxiliary telemetry. Neither the solver's own
  retries nor Prowlarr internal work is inferred from a response.
- Prowlarr child requests use their child accounting identity. Native parent
  management operations, including their caps enrichment requests, use the
  parent management identity. These are observed Scryer calls, not upstream
  provider quota consumption.
- A saved connection test uses its saved accounting identity while executing
  the submitted configuration. Unsaved tests create no persistent quota rows.
- A one-time epoch clears the old observed count, preserving provider-reported
  API/grab readings and limits. The UI distinguishes these values and labels
  the first UTC day as partial. Missing provider readings display as unknown,
  rather than implying zero usage.
- Counts retain the existing 500 ms persistence batch. Orderly shutdown closes
  dispatch admission, waits for admitted accounting callbacks, then drains the
  batch. Failed drains are retried within the shutdown budget and reported;
  an abrupt crash can still lose an unflushed batch.

## Fixes and preserved behavior

The indexer resolver owns artifact HTTP, solver assignment, redirect handling,
artifact classification, and NZB staging. It fetches the download URL supplied
by search without invoking a plugin action. The release follow-up removed the
plugin grab contract and its fallback decoder so shipped plugins need no update.

The download router consumes staged NZBs, supplied bytes, torrents, or magnets.
It no longer fetches indexer URLs or selects challenge solvers. The staging lease
survives canonical submission and client failover, allowing one source fetch.
Native download adapters continue to stage already supplied bytes when needed.
Original source URL/kind are retained in submission history and identity while
the client receives the prepared artifact.

Validation covers XML/category rejection, size limits, cancellation and dropped
futures, temporary-file cleanup, torrents and magnets, routing mappings, facet
and library eligibility, client priorities, failover, and ambiguous submissions.
Redirects retain session credentials on the original origin and remove them on
an external hop; unsafe destinations are rejected before dispatch.

Prowlarr solver assignments are rejected on create/update/test, removed from
existing solver assignments by the epoch migration, omitted during child sync,
and suppressed defensively at runtime. Direct indexers retain solver behavior.
On NEXT this exclusion must apply only to `ChallengeSolver`; its transport and
tunnel assignments remain valid and must be retained.

Structured Newznab 500/501 quota errors survive HTTP 200 wrappers and component
errors, reach rate-limit handling, and do not initiate solver fallback. Newznab
910 remains an API-disabled error. HTTP 429 and vanished sources likewise stop
without another grab or solver call.

## Validation results

The final focused confirmation ran **822 tests, all passing**, covering the
acquisition infrastructure, plugins, outbound HTTP, native download integration,
and canonical artifact ownership/history. The preceding larger affected sweep
ran 1,976 tests: 1,974 passed and two fixture failures were corrected and included
in the final confirmation. Its other passing application and GraphQL coverage
was unchanged by those fixture corrections.

The confirmation also exercised package-scoped builds: test HTTP clients use
the application's configured client factory, so TLS initialization does not
depend on features incidentally enabled by another workspace package.

Web validation: **516 tests passed**, TypeScript type-checking passed, and all
six GraphQL compatibility tests passed. The schema exported by the actual
backend matched `api/graphql/schema.graphql` exactly. Formatting and strict
workspace Clippy passed (`--all-targets --all-features -- -D warnings`).

The completed broad workspace sweep ran 6,650 tests: 6,620 passed, 30 failed,
and 8 were skipped. Sixteen failures were in the changed artifact/routing test
paths and prompted further fixes and reruns. Fourteen failures also reproduced
on the unchanged release baseline, using an isolated local PostgreSQL fixture
where required; they were not weakened or modified in this work.

The unchanged-baseline failures are:

- Nine title/alias matching expectations in release search, RSS, and import
  title resolution. These include occupied-scope/cutoff admission, contextual
  aliases, episode identity, bracketed groups, stacked aliases, and trailing
  years.
- PostgreSQL blank-install secondary-index expectations.
- PostgreSQL blank-install legacy-column expectations.
- Concurrent PostgreSQL user grants: duplicate `libraries_pkey`.
- PostgreSQL discovery top-rated typed-null rating: zero rows instead of one.
- PostgreSQL legacy source-password backfill: a null
  `pending_releases.last_observed_at` violates its constraint.

Those baseline failures remain separate validation blockers; this review does
not certify the whole release as green. No production or live indexer behavior
was probed. There was no release, version change, publication, deployment, or
dependency change.

Evidence logs are under `/tmp/indexer-release-correctness-20260907`, including
`workspace-final.log`, `baseline-tests.log`, and the serial, isolated-fixture
`baseline-postgres.log`. The baseline archive used the same pinned builtin
plugin artifacts as the implementation worktree.

Exact unchanged-baseline test names:

```text
scryer-application
  acquisition::release_search::tests::an_occupied_scope_refuses_a_candidate_it_cannot_beat
  acquisition::release_search::tests::candidate_matches_title_subject_uses_contextual_alias_parse_when_needed
  acquisition::release_search::tests::cutoff::a_format_cutoff_target_is_still_grabbable
  acquisition::release_search::tests::episode_subject_rejects_candidates_without_episode_identity
  acquisition::rss::tests::stacked_alias_release_matches_via_bank
  acquisition::rss::tests::title_matches_with_bracketed_group_prefix
  acquisition::rss::tests::trailing_year_title_matches_with_and_without_release_year
  acquisition::rss::tests::two_token_bracket_group_prefix_matches
  import::title_resolution::tests::contextual_candidate_bank_prefers_stacked_anime_alias_match
scryer-infrastructure-datastore
  postgres::services::tests::postgres_blank_install_applies_parity_secondary_indexes
  postgres::services::tests::postgres_blank_install_smoke_from_env_url
  postgres::services::tests::postgres_set_grants_for_user_is_idempotent_under_concurrency
scryer-infrastructure-metadata
  discovery::store::tests::postgres_discovery_home_top_rated_accepts_typed_null_rating
scryer-infrastructure-runtime
  tests::settings_and_writer::source_password_backfill_encrypts_legacy_postgres_rows
```
