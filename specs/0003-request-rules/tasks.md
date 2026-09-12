# Tasks: Request Rules, Title Leases, and the Shared Policy Core

Status legend: `[ ]` not started · `[~]` in progress · `[x]` accepted by reviewer. Implementers never commit; the reviewer commits signed at each accepted checkpoint.

## WP0 — Foundation
- [x] Worktree `.worktrees/request-rules`, branch `feature/request-rules` off `release-NEXT` @ `102eac7fe`
- [x] `specs/0003-request-rules/{spec,plan,tasks}.md`

## WP1 — Shared policy core (`crates/scryer-rules/src/policy/`) — landed 2026-09-05, reviewed; one follow-up in flight
- [x] `PolicyFamily` trait, generic `PolicyEngine`/`PolicyEvaluator`, `RuleHandle`, `EvalOutcome`
- [x] Move `Observation`, fact-namespace serialization, held-reason logic, reason bounds, tag decoder into the core
- [x] Generalize validation catalog / referenced-path extraction over the family contract (`FamilyContract` statics)
- [x] Maintenance re-based on the core; public surface preserved via type aliases + re-exports
- [x] Release re-based on the core; signatures/facet skip/managed checks/`rule_identity()` unchanged; `BOUND_INPUT = false` keeps release unbounded (follow-up landed)
- [~] Oracle green: `scryer-rules` 122, `scryer-application` maintenance 179 / rules 141 / quality 277 / post_download_gate 37 — `integration_rego` + `integration_maintenance_rules` still to run by the reviewer once WP4 settles (plugin builtins now materialized in the worktree)

## WP2 — Request family (`crates/scryer-rules/src/request.rs`) — landed 2026-09-05, reviewed and accepted (scryer-rules 180 green)
- [x] Input document + 38 facts, `request-input-contract.json` (+ byte-identical web mirror, parity test)
- [x] Five-head wrapper, decode (manual > deny > approve > abstain, `held` flag), `request_defaults()` limits, synthetic input, `request_person_targeted_paths`, ladder + resolution helpers, five pinned examples (`REQUEST_RULE_EXAMPLES`)
- [x] Unit tests: precedence, abstain, held, tag bounds, budget, malformed heads, hash stability, examples end to end
- Deferred: generic `validate_family_rule<F>` (needs `contract()`/`synthetic_input()` on the trait); `ratings_by_source` is open-keyed (`object.get`), documented

## WP3 — Hydration widening + snapshot — landed 2026-09-05, reviewed and accepted
- [x] SMG selections (`get_movie`, `get_series`, `titles`) + `MovieMetadata`/`SeriesMetadata` fields + shared `ContentRating`/`MdblistSummary`/`TitleAward` types (discovery types now alias them)
- [x] `MediaRequestMetadataSnapshot` v1 with `partial`/`missing`, `schema_version == 0` sentinel, `raw_series` on `HydrationResult`; persisted at submit
- [x] `enrich_request_draft` read-through cache (5 min / 256) on `AppRuntimeCatalogState`
- [x] Tests: mapper fixtures present/omitted, snapshot round-trips, submit persists full/partial snapshot, one gateway call for preview+submit
- Moved to WP5: `request_rules/facts.rs` (ladder/resolution helpers landed in WP2). Deferred: `TitleMetadataUpdate` widening + `metadata_bulk.graphql`; shorter TTL for cached failures; `import::srrdb` tests flake under parallelism (pre-existing, chip raised)

## WP4 — Domain, migrations, stores, ports — landed 2026-09-05, reviewed and accepted
- [x] Domain types (`RequestRuleSet`, `RequestRuleRevision`, `RequestRuleEvaluationMode`, `RequestRuleDecisionRecord`, `LifecycleClaim` + enums); `MediaRequest` + event data additive fields
- [x] Migrations 0219/0220/0221 (renumbered from 0218-0220 when title tags took 0218 on release-NEXT) (SQLite + Postgres), manifest-registered; `resolved_by_user_id` was already nullable in both dialects
- [x] `RequestRuleSetStore`, `RequestRuleDecisionStore`, `LifecycleClaimStore`, request-store columns + history queries
- [x] Ports, `AppServices` wiring (incl. integration harness), in-memory test repos
- [x] Store tests on SQLite (Postgres twins env-gated, unverified locally); migration apply-then-validate test
- Follow-ups handed on: WP5 widens `MediaRequestResolution.resolved_by_user_id` to `Option<String>`; WP6 adds a `now` argument to `LifecycleClaimRepository::activate` (currently stamps `updated_at = starts_at`)

## WP5 — Application service + evaluation + pre-flight + tags + leases at approval — landed 2026-09-05 (after a disk-full restart), reviewed and accepted
- [x] `request_rules/{service,gates,engine,facts,arbitration,evaluation,preflight}.rs`: CRUD, validate, author preview, mode, one instance gate (`request_rules.evaluation_enabled`), engine cache on `AppCustomizationServices` rebuilt by every mutating call, library scope as a vote filter, person-targeting gate
- [x] Evaluation in `submit_media_request` / `update_my_media_request`; deny path (`resolved_by_user_id: None`, event with rule ids + reasons); trace with `votes_json`; tags applied only on approval; `record_decision_on_request` for pending rows
- [x] `preview_my_request_decision` (same input type as submit; never errors; no rule internals)
- [x] 6b: claim creation per resolved request (dormant `RetainUntil` / active `Keep`), release on cancel/dismiss, `list/extend/convert/release_title_claim` (library `ManageTitles`), `release_claim` port method, `count_titles_in_library`
- [x] `lib_tests/request_rules.rs` (34) + `request_rules_facts.rs` (14) + store tests; `integration_rego` 34 / `integration_maintenance_rules` 23 green (reviewer-run)
- Deferred: `approved_lease_days` column is uniform across overlapping resolutions while claims are per-request (cosmetic); `RequestRequesterDoc.created_at` always absent (User has no created_at); title-tags registry check is a marked TODO

## WP6 — Leases (split: 6a maintenance side, 6b request side folded into WP5)
- [x] 6a: activation at first import (hooked on the `ImportCompleted` domain event in `publish_stored_domain_event`), maintenance-pass reconcile (`expire_due` + dormant sweep ≤500, backdated to `first_imported_at`, runs before the evaluation gate), release on title delete — landed 2026-09-05, reviewed and accepted
- [x] 6a: four maintenance facts (`keep_claim_active`, `request_lease_state`, `request_lease_expires_at`, `active_retention_claims`), same claims context at evaluation/preview/executor, `RETENTION_CLAIM_HOLD` (high-risk), `list_retention_history_for_titles` port, `activate(now)`
- [x] 6a: `expired-request-leases` template pinned; 17 flow tests + 9 fact tests + store assertions (maintenance 204, integration_maintenance_rules 23 green before the disk filled)
- [ ] 6b (in WP5): claim creation on approval per resolved request; release on cancel/dismiss; admin extend / convert / release
- Handed to WP8: `settings.maintenanceTemplateExpiredLeases*` (en/ru placeholders) and the eight `settings.refMaintFacts*` strings

## Environment note
- 2026-09-05 ~00:40Z the host disk filled (this worktree's `target/` reached 149 GB); WP5 was interrupted. `target/debug/{incremental,deps,build,.fingerprint}` wiped; WP5 relaunched as a continuation. Implementers build with `CARGO_INCREMENTAL=0`.

## WP7 — GraphQL — landed 2026-09-05, reviewed and accepted
- [x] 6 enums, 14 object types, 11 inputs, 9 query roots, 11 mutation roots; additive fields on `MediaRequestPayload`, `SubmitMediaRequestInput`, `UpdateMediaRequestInput`, `ApproveMediaRequestInput`/`Payload`; `schema.graphql` regenerated via `export-graphql-schema`; compat = 5 `OPTIONAL_INPUT_FIELD_ADDED` only
- [x] Redaction is one projection parameter (`from_request_rule_decision(record, redacted)`); pre-flight payload has no votes field; `request_rules/read_model.rs` batches claim/trace reads per page
- [x] Fixed: `request_rules.evaluation_enabled` seeded in `settings_bootstrap.rs`; integration harness now wires `MediaRequestStore`
- [x] `crates/scryer/tests/integration_request_rules.rs` (11) + 5 mapper tests; `integration_graphql` 341, `integration_rego` 34, `integration_maintenance_rules` 23, `integration_notifications` 34 green

## WP8 — Web — landed 2026-09-05, reviewed and accepted (`npm run lint` clean, `npm test` 706, react-compiler + graphql-compat green; 428 documents validated against the schema)
- [x] Rules page pane (`/automation/rules/request`, experimental-gated), editor, reference panel, template gallery pinned to `REQUEST_RULE_EXAMPLES`, user picker, author preview, gate (locked when `ManageSystemSettings` is missing), recent decisions
- [x] Request dialog "Keep for" lease picker (`null` = forever) + 400 ms debounced pre-flight banner; `submitMediaRequestInput` is the single builder for submit and pre-flight
- [x] Requests view lease badge / decision popover / pending tags / deny reasons; approve dialog lease + tags; retention-claim panel (extend / permanent / release)
- [x] `expired-request-leases` template + four lease facts resolve in en+ru (WP6a's ru placeholders replaced); 10 locales (85 pane keys + 6 dialog keys everywhere; 208 deep keys en+ru only, matching the maintenance feature's existing line)
- Handed to WP9: `RequestPreflightPayload.fallbackReason` (banner currently approximates from `metadataPartial` + first reason code)

## WP9 — E2E, docs, capstone — `[~]` implementer launched 2026-09-05
- [x] `fallbackReason` on `RequestPreflightPayload` (application `RequestPreflight` + mapper + schema regen; banner prefers it over the approximation and falls back for a server that predates it)
- [x] E2E specs (sibling `e2e` repo): preflight-and-auto-approve, deny, shadow-records-only, tags, lease-blocks-maintenance-delete, permissions — plus `request-rule-helpers.ts` (arming + oracles, one classified GraphQL seam), request-rule selectors, and the shortcut-audit baseline entry
- [x] Two ids added for the e2e flows: `settings-request-template-gallery-toggle` on the gallery header, and contentId/confirm/cancel ids on the discard-draft confirmation
- [x] Release-notes entry (`release-notes/scryer-vNEXT-request-rules.md`); RFC 137 §13.5 status note in `scryer-docs`
- [x] Capstone: single clippy pass, fmt check, full targeted Rust sweep, four web checks
- Not done, deliberately: no gate-flow registration for the new specs in the `e2e` Go harness (flow constants, seed profile, parallel weights and the `flow_plan_test` golden list are a separate change); the operator runs them directly. No commits in either repository.
