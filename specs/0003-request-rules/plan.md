# Implementation Plan: Request Rules, Title Leases, and the Shared Policy Core

**Spec**: [spec.md](./spec.md) · **Tasks**: [tasks.md](./tasks.md)
**Status**: Approved by the operator 2026-09-04.
**Worktree**: `.worktrees/request-rules`, branch `feature/request-rules` off `release-NEXT` @ `102eac7fe`.
**Pipeline**: Claude (Fable) plans, briefs, reviews, commits (SSH-signed). Up to two Opus implementers work one package each in this worktree, never commit, never bump versions, never run clippy (one capstone pass at the end).

## Constitution Check (specs/constitution.md v1.1.0)

| Principle | How this plan satisfies it |
|---|---|
| C1 Migrations are immutable | Three new forward migrations (0218–0220), paired SQLite/Postgres; no shipped migration is edited. `resolved_by_user_id` nullability uses the table-rebuild pattern from 0206 on SQLite. |
| C2 Preview before mutate | Rules are validated and previewable before saving; requesters see the decision before submitting; lease-driven deletion runs only through the existing maintenance preview-fingerprinted delete path. |
| C3 Nothing silent, nothing destroyed | Denials carry reasons the requester sees; leases, claims, tags, and every decision trace are visible; partial metadata is flagged rather than dropped; claims are released, never deleted from history. |
| C4 Destruction requires proof | Unchanged: deletion stays inside `delete_title_by_policy` with its fresh manifest fingerprint; leases and keep claims add a hold, never a shortcut. |
| C5 Long-running work is asynchronous | Claim activation and expiry reconcile inside the existing maintenance evaluation job; request evaluation is synchronous by design (100 ms budget) and never blocks a submission on failure. |
| C6 External compatibility is a contract | GraphQL changes are additive; SMG selections widen existing queries against fields the frozen SMG schema already publishes; release scoring behaviour is preserved byte-for-byte. |
| C7 Platform differences | No filesystem semantics introduced. |

Deviation: none recorded.

---

## 1. Context

Scryer has two Rego policy families on one Regorus runtime: release scoring (`crates/scryer-rules/src/release.rs`) and maintenance rules (`maintenance.rs`). They share only `runtime.rs` (engine construction, limits, input bounding, package rewrite, content hash) and parts of `validation.rs`; the engine/evaluator loop, observation envelopes, wrapper generation, decode, and held-on-unknown logic were written for maintenance and would be copy-pasted a third time.

Media requests are auto-approved today by one blunt per-library permission (`LibraryPermission::AutoApproveRequests`, `crates/scryer-application/src/media_requests.rs:194`) or wait for a human. The operator wants a **request rules** family that decides approve / manual review / deny at submit time, tells the requester the outcome *before* they submit, can stamp tags on the resulting title, and understands a new **title lease** ("keep this for 14 days") threaded from the request dialog through approval, storage, UI, and maintenance.

### Decisions (operator-confirmed 2026-09-04 unless marked new)

| # | Decision |
|---|---|
| D1 | Votes are **approve / manual / deny**. Engine error, unknown fact, timeout, or no matching rule ⇒ **manual review**, never deny, never approve. |
| D2 | **Lease expiry acts only through maintenance rules.** Leases and keep claims become maintenance facts; a shipped maintenance template removes expired leases via the existing delete action, gates, arming, and preview. Any live lease or keep claim **holds every destructive maintenance action**. No new executor. |
| D3 | **Most restrictive wins**: deny > manual (incl. held/error) > approve. No priority numbers. The existing `AutoApproveRequests` permission is an approve vote in the same arbitration and shows in the trace. |
| D4 | Rules are **read-only over the lease** in v1; only a human approver adjusts it. |
| D5 *(new, per feedback)* | Build a **shared policy core** (`scryer-rules::policy`) that release, maintenance, and request families all ride on. Families supply input documents, wrapper heads, and a decision decoder; the core owns everything else. Release scoring must produce **identical decisions** after the move (RFC §6.1). |
| D6 *(new)* | The request family's hold head is **`manual if`** (maintenance keeps `unknown if`; the head name is a per-family parameter of the core). |
| D7 *(new)* | **Pre-flight evaluation**: the request dialog evaluates the draft as the requester types and shows "Will be auto-approved / Needs approval / Would be denied: reason". Preview and submit share one input builder and one enrichment, so they cannot disagree. |
| D8 *(new)* | Rules may emit **tags** (`tags contains "kids"`) that are applied to the created title on approval (auto or human), recorded on the request, and shown in the pre-flight banner and trace. |
| D9 *(new)* | Media requests store a **versioned metadata snapshot** of everything the rule surface can see, captured at submit; the request UI and facts read from it, so a decision can always be explained against the data it was made on. |

### Standing assumptions (object now if wrong)

- A1 Ships behind the **experimental-features gate** (like maintenance) plus an instance evaluation gate defaulting **off**. Rule sets are created **disabled**; modes are `disabled | shadow | enforce`.
- A2 Dialog lease default is **Forever**; options Forever, 7, 14, 30, 60, 90 days, custom.
- A3 Forever ⇒ permanent keep claim; finite ⇒ dormant retention claim that **starts at first import** (RFC §13.2.5, §20.2).
- A4 Rules that read `input.requester.*` need `ManagePermissions` to author or preview (mirrors `require_person_fact_authority`, `maintenance_rules/service.rs:603`). Content-only rules need `ManageCatalogSettings`. **Pre-flight as the requester is exempt**: the requester is asking about themselves and sees only outcome, reasons, and tags, never the rule bodies.
- A5 Authoring UI = Rego editor + reference panel + template gallery + user picker. No no-code builder.
- A6 `request.origin` is fixed to `manual`; the field exists so Plex-watchlist requests (RFC §13.4) slot in later.

---

## 2. Shared policy core (D5) — `crates/scryer-rules/src/policy/`

### 2.1 What every family already does the same way

Build a Regorus engine with builtins and limits → add each user policy plus a generated wrapper → per subject: serialize input, bound it, `set_input`, loop rules, `eval_rule(wrapper path)`, decode a closed value, collect per-rule errors without aborting → hash policy sources → validate source against a JSON contract of allowed `input.*` paths → dry-run against a synthetic input. Maintenance adds the observation envelopes and host-derived holds. Release adds per-rule applicability (`applied_facets`) and managed-pack origin checks.

### 2.2 Core API

```rust
// policy/mod.rs
pub trait PolicyFamily: 'static {
    const NAME: &'static str;                     // "release" | "maintenance" | "request"
    const USER_PACKAGE_PREFIX: &'static str;      // scryer.<family>.user
    const WRAPPER_PACKAGE_PREFIX: &'static str;
    type Policy: PolicyRecord;                    // id, name, rego_source (+ family extras)
    type Input: PolicyInput;                      // Serialize + fact namespaces
    type Decision;                                // closed per-family output
    fn limits() -> RuntimeLimits;
    fn wrapper_source(rule_id: &str) -> String;   // reads the user package with object.get defaults
    fn wrapper_rule_path(rule_id: &str) -> String;
    fn decode(value: &regorus::Value) -> Result<Self::Decision, String>;
    fn hold_rule_name() -> Option<&'static str>;  // maintenance "unknown", request "manual", release None
    fn applies(policy: &Self::Policy, ctx: &Self::Input) -> bool { true }   // release: applied_facets
    fn contract_json() -> &'static str;           // input-path catalog for validation + web reference
    fn person_targeted_paths() -> &'static [&'static str];
    fn synthetic_input() -> Self::Input;
}

pub struct PolicyEngine<F: PolicyFamily>   { template: Arc<Engine>, rules: Vec<RuleHandle>, limits }
pub struct PolicyEvaluator<F: PolicyFamily>{ engine: Engine, rules: Vec<RuleHandle>, limits }
pub struct RuleHandle { id, name, content_hash, referenced_facts: BTreeSet<String>, extras: serde_json::Value }
pub struct EvalOutcome<D> { records: Vec<EvalRecord<D>>, errors: Vec<EvalError> }
pub enum Held { No, Yes(Vec<String>) }            // host-derived unknownness, reason codes in fact order
```

Core also owns (moved, not rewritten): `Observation<T>`, `SerializedFacts` + `facts`/`observations` namespace derivation, `unobservable_facts`/`held_reason_codes`, `decode_reasons` bounds (32 × 120), `rewrite_package_declaration_with_prefix`, `strip_editor_source`, `content_hash`, `bounded_input_value`, `configured_engine`, and `validation::{build_input_catalog, module_input_path_errors, module_referenced_facts, input_import_error}` generalized over `F::contract_json()`. New shared **tags output** decoder (`decode_tags`: ≤16 tags, ≤64 chars, `[A-Za-z0-9._ -]`, `scryer:` prefix rejected) available to any family whose wrapper exposes `tags`.

### 2.3 Families after the refactor

- **maintenance.rs** → `MaintenanceFamily` + input/decision types only. `MaintenanceRulesEngine`/`Evaluator` become type aliases over the core. Public surface used by the application (`MaintenancePolicy`, `MaintenanceInput`, `MaintenanceOutcome`, `MaintenanceEvalRecord`, `PERSON_TARGETED_MAINTENANCE_FACTS`, `rewrite_package_declaration`, `synthetic_maintenance_input`) is preserved as re-exports so `scryer-application` compiles unchanged in WP1.
- **release.rs** → `ReleaseFamily`. `UserRulesEngine::{build, empty, is_empty, rule_count, rule_identity, evaluator}` and `UserRulesEvaluator::evaluate(input, facet)` keep their exact signatures and semantics: `applies` implements the facet skip, `decode` implements `extract_entries` (i32 clamp, NaN skip), managed-origin validation stays a release hook run at build (`validate_managed_rule`) and after decode (`validate_managed_entries`). Release has no fact envelopes and no hold; `hold_rule_name = None` and `referenced_facts` is empty. **Oracle**: the existing release tests in `release.rs`, `rules/rules.rs`, `managed_trash.rs`, `canonical_tests.rs`, `post_download_gate.rs`, `quality/trash_ranking_corpus_tests.rs`, and `crates/scryer/tests/integration_rego.rs` must pass byte-for-byte; `rule_identity()` output and the scoring fingerprint are unchanged.
- **request.rs** → `RequestFamily` (new, §3).

Validation contracts: `crates/scryer-rules/{rule,maintenance,request}-input-contract.json` mirrored under `apps/scryer-web/lib/contracts/`; the existing parity test (`validation.rs:1225`) is extended to the third pair.

---

## 3. Request family — `crates/scryer-rules/src/request.rs`

**Limits**: 100 ms per evaluation (synchronous, on the submit path), 256 KiB policy, 1 MiB input.

**Heads the author writes**: `approve if {…}`, `deny if {…}`, `manual if {…}` (D6), `reasons contains "…"`, `tags contains "…"` (D8). Wrapper reads all five with `object.get` defaults.

**Vote decode** (per rule): `manual` ⇒ Manual; else `deny` ⇒ Deny; else `approve` ⇒ Approve; else Abstain. Tags and reasons are collected on every path where the rule ran. Non-boolean heads, oversized reasons/tags, runtime error, or timeout ⇒ per-rule error (treated as Manual by arbitration, tags dropped). A rule held for an unobservable fact contributes Manual with the observation's reason codes and no tags.

### 3.1 Input document (schema v1)

Everything under `requester`, `library`, `request`, and `now` is always known. Everything under `facts` is an `Observation` and may be unknown (⇒ held ⇒ manual) or absent (⇒ missing key, a real answer). Person-targeted paths: `input.requester.*`.

```jsonc
{
  "schema_version": 1,
  "evaluation_time": "2026-09-04T18:00:00Z",
  "now": { "weekday": "thursday", "hour_utc": 18 },              // convenience; time.* builtins also work
  "requester": {
    "user_id": "…", "username": "…", "account_kind": "local|external_auto_provisioned",
    "app_permissions": ["manage_users", …],
    "library_permissions": ["view", "request", "auto_approve_requests", …],   // target library
    "linked_providers": ["jellyfin", "plex"],                                 // verified external accounts
    "created_at": "…"
  },
  "library": { "id": "…", "name": "…", "facet": "movie|series|anime", "is_default": true },
  "request": {
    "origin": "manual",
    "title": "…", "year": 2024, "external_ids": { "tmdb": "…", "imdb": "…", "tvdb": "…" },
    "quality_profile_id": "…", "quality_profile_name": "…",
    "monitor_type": "futureepisodes", "monitor_selection_season_count": 2,
    "lease_forever": false, "lease_days": 14
  },
  "facts": { … }, "observations": { … }
}
```

### 3.2 Facts (all `Observation<T>`)

| group | fact | type | source |
|---|---|---|---|
| **content rating** | `age_rating` | int | SMG `content_ratings[].age_rating` / `mdblist.age_rating` (minimum age) |
| | `certifications` | `[{country,value,source}]` | SMG `content_ratings`, flattened |
| | `certification_label` | string | US value when present (`G`…`NC-17`, `TV-Y`…`TV-MA`) |
| | `certification_rank` | int 0–4 | host ladder: G/TV-Y/TV-Y7/TV-G=0 · PG/TV-PG=1 · PG-13/TV-14=2 · R=3 · NC-17/TV-MA=4; unknown without a US label |
| | `commonsense_recommended` | bool | SMG `mdblist.commonsense` |
| **title metadata** | `genres` | [string] | SMG `genres` |
| | `canonical_tag_keys` | [string] | SMG `canonical_tags[].key` (e.g. `canonical:genre:horror`, `canonical:theme:…`) |
| | `themes` | [string] | canonical tags with category `theme` |
| | `is_adult` | bool | any canonical tag `is_adult` |
| | `rating` | float | combined rating |
| | `ratings_by_source` | `{source: normalized}` | `external_ratings` |
| | `tmdb_vote_average`, `tmdb_vote_count`, `popularity` | numbers | SMG |
| | `runtime_minutes`, `original_language`, `country`, `network`, `studio`, `content_status` | scalars | SMG |
| | `release_date` / `first_aired`, `release_age_days` | string / int | SMG; age from `evaluation_time` |
| | `award_count` | int | SMG `awards` |
| **quality** | `quality_profile_tiers` | [string] | `QualityProfileCriteria.quality_tiers` |
| | `quality_profile_max_resolution` | int | parsed max of tiers (480/720/1080/2160) |
| | `quality_profile_allows_upgrades` | bool | criteria |
| **catalog** | `exists_in_library_ids` | [string] | same facet, other libraries, by external id (`find_by_external_id_in_facet`) |
| | `previous_request_count`, `previously_denied`, `previously_approved` | int/bool | history for this identity fingerprint (any requester) |
| **requester history** | `pending_request_count`, `approved_last_30d`, `denied_last_30d`, `total_approved`, `active_lease_count`, `days_since_last_request` | ints | `media_requests` + `lifecycle_claims` |
| **library** | `library_title_count` | int | `titles` count |

Disk free space per root is **not** exposed (no port exists today); listed as a follow-up.

### 3.3 Arbitration (D3, application layer)

```
any Deny                                  → Deny         { rule ids, reasons }
else any Manual | held | error            → ManualReview { fallback: rule|held|error }
else any Approve | permission vote        → AutoApprove  { rule ids | "library_permission" }
else                                      → ManualReview { fallback: no_rule_matched }
tags = union over every rule that ran
```

Gate off, no enabled rule set, or shadow ⇒ trace recorded, **effective** decision = today's behaviour (permission ⇒ approve, else manual); tags are still recorded but not applied in shadow.

### 3.4 Worked examples

```rego
requesters := {"alice", "bob", "carol"}

approve if {                               # 1. family-rated content for named users
	input.requester.username in requesters
	input.facts.certification_rank <= 2
}
tags contains "family" if { input.facts.certification_rank <= 1 }

approve if {                               # 2. short leases for bob (forever must not match)
	input.requester.username == "bob"
	not input.request.lease_forever
	input.request.lease_days <= 14
}

approve if {                               # 3. alice, library scoped on the rule set, 720p or lower
	input.requester.username == "alice"
	input.facts.quality_profile_max_resolution <= 720
}

deny if { input.facts.is_adult }           # deny with a reason the requester sees
reasons contains "adult_content" if { input.facts.is_adult }

manual if { input.facts.approved_last_30d >= 5 }   # quota: a human looks after five approvals a month
```

---

## 4. Request flow, pre-flight, snapshot, tags

### 4.1 Metadata snapshot (D9)

`enrich_media_request_metadata` (`media_requests.rs:1036`) already calls SMG at submit. WP3 widens what it returns and stores a `MediaRequestMetadataSnapshot { schema_version: 1, content_ratings, mdblist, genres, canonical_tags, external_ratings, tmdb_vote_average, tmdb_vote_count, popularity, runtime_minutes, original_language, country, network, studio, content_status, release_date, first_aired, awards, is_adult }` as `metadata_snapshot_json` on the request. Requires:

- SMG selections extended in `crates/scryer-infrastructure-metadata/src/metadata/gateway/metadata_gateway/{get_movie,get_series,titles}.graphql` — all fields exist on `MovieTitle`/`TvdbSeries` (`smg/graph/schema.graphqls:695, 901`); no SMG change.
- `MovieMetadata` / `SeriesMetadata` (`library/scan/scanner.rs:535, 583`) gain `genres`, `content_ratings`, `mdblist`, `tmdb_vote_average`, `tmdb_vote_count`, `awards`; `TitleMetadataUpdate` (`types.rs:32`) gains `content_ratings: Option<Vec<ContentRating>>` and `genres` (additive, `Default`), so titles hydrate them too and maintenance can read them later.
- **Audit item (feedback)**: today's snapshot keeps title/year/overview/runtime/language/content_status/poster/background/ratings only, and the movie path goes through `get_movie_titles` first with a `get_movie` fallback while the enrichment swallows errors into an empty summary. WP3 makes an enrichment failure an explicit `snapshot.partial = true` with the failing fields listed, so facts derived from them are **unknown** (⇒ manual review) instead of quietly absent, and the pre-flight banner says "metadata unavailable — will need approval".

### 4.2 Shared input builder + one enrichment

`request_rules/facts.rs::build_request_input(actor, library, draft, snapshot, quality_profile, history)`. An in-process enrichment cache (`moka`-style TTL ≈ 5 min, key = facet + normalized external ids) sits in front of SMG so the dialog's pre-flight calls and the eventual submit hit SMG once.

### 4.3 Pre-flight (D7)

New query `previewMyRequestDecision(input: SubmitMediaRequestInput)` — same permission as submit (`LibraryPermission::Request` on the library), evaluates without persisting, returns `{ outcome: AUTO_APPROVE|MANUAL_REVIEW|DENY, reasons: [{code, ruleName}], tags: [String], metadataPartial: Boolean, evaluationMode }`. Rule bodies are never returned. The dialog debounces (~400 ms) on library / profile / monitor / lease changes and renders a banner; in shadow mode the banner shows the *shadow* verdict labelled "preview" and the effective verdict.

### 4.4 Submit / update / approve

`submit_media_request` after the submit transaction (~L183) replaces the permission check at L194 with: build input → evaluate (engine cached per rule-set revision; invalidated on any rule-set write) → persist `RequestRuleDecision` trace (always) → act on the effective decision:

- `AutoApprove` → existing `auto_approve_submitted_media_request`, with policy tags merged into `media_request_to_new_title` and provenance on the `MediaRequestApproved` event.
- `Deny` → `deny_submitted_media_request`: `resolve_pending_overlapping(status: Rejected, resolved_by_user_id: None, resolved_by_policy: rule ids)`; `MediaRequestRejected` event carries reasons. `resolved_by_user_id` becomes nullable (0220) instead of inventing a system user.
- `ManualReview` → pending, with the trace and pending tags visible to approvers.

Evaluation failures never fail the submission (log, `fallback: error`, request stays pending). `update_my_media_request` re-evaluates (an edit can now satisfy a rule). `approve_media_request` gains `lease_days`/`lease_forever` and `tags` (pre-filled from the policy tags, editable).

---

## 5. Leases and claims (D2, A3)

Table `lifecycle_claims` (RFC §11 named it; 0210/0211 never created it):

```
id PK, title_id, library_id, producer ('request_lease'|'request_permanent'|'operator_keep'),
producer_ref (request id), kind ('retain_until'|'keep'),
state ('dormant'|'active'|'expired'|'released'|'converted'),
duration_days NULL, starts_at NULL, expires_at NULL,
created_by NULL, created_at, updated_at, released_reason NULL
UNIQUE (producer, producer_ref) WHERE state IN ('dormant','active');  INDEX (title_id, state)
```

- **Create** at approval (human or policy): finite ⇒ `retain_until/dormant`; forever ⇒ `keep/active`. One claim per requester on overlapping requests (joiners inherit forever in v1; join-dialog term is a follow-up).
- **Activate** at first import of the created title (hook where `ImportCompleted` is appended; exact anchor confirmed in WP5) with the maintenance pass as a safety net (activates dormant claims whose title has `first_imported_at`).
- **Expiry** derived (`expires_at <= now`) wherever read; the maintenance pass also flips `active → expired`.
- **Release** on cancel/reject before availability and on title delete (`delete_title_logical_cleanup`); admin **extend / convert to permanent / release** mutations.

**Maintenance facts** (additive keys in the v2 facts doc + contract JSON): `keep_claim_active` (bool), `request_lease_state` (`none|dormant|active|expired`, aggregated), `request_lease_expires_at` (latest active), `active_retention_claims` (int).

**Executor hold**: in the safety recheck (`action_execution.rs` ~L975, beside the location-operation hold) a high-risk action holds with `RETENTION_CLAIM_HOLD` while any dormant/active retention or keep claim exists; unreadable claim store ⇒ `UNKNOWN_AT_EXECUTION`.

**Shipped maintenance template** `expired-request-leases` (`apps/scryer-web/lib/constants/maintenance-rule-templates.ts`): `DELETE_TITLE_AND_FILES`, grace 7 d, `match if { input.facts.request_lease_state == "expired"; not input.facts.keep_claim_active }`. Inherits the destructive gate, per-rule arming with count acknowledgement, playback/acquisition holds, preview fingerprint, recycle disposition.

---

## 6. Persistence (paired SQLite + Postgres, forward-only; next free numbers after 0217)

| migration | contents |
|---|---|
| `0218_request_rule_sets.sql` | `request_rule_sets(id, name, description, enabled, evaluation_mode 'disabled', library_ids '[]', current_revision_number, created_at, updated_at)`; `request_rule_revisions(id, rule_set_id FK cascade, revision_number, rego_source, matcher_content_hash, created_by, created_at, UNIQUE(rule_set_id, revision_number))`; `request_rule_decisions(id, request_id, evaluated_at, mode, effective_outcome, policy_outcome, fallback_reason, votes_json, tags_json, input_hash, input_schema_version, created_at)` + index `(request_id)`. |
| `0219_lifecycle_claims.sql` | §5 table. |
| `0220_media_request_policy.sql` | `media_requests` + `requested_lease_days INT NULL`, `approved_lease_days INT NULL`, `decision_id TEXT NULL`, `decided_by_rule_set_ids TEXT NOT NULL DEFAULT '[]'`, `policy_tags_json TEXT NOT NULL DEFAULT '[]'`, `metadata_snapshot_json TEXT NOT NULL DEFAULT '{}'`; `resolved_by_user_id` → nullable via the 0206 rebuild pattern (SQLite) / `DROP NOT NULL` (PG). |

Renumber per the standing recipe if `main` claims 0218+ first.

Stores: `RequestRuleSetStore` + `RequestRuleDecisionStore` in `crates/scryer-infrastructure-configuration/src/customization/` (pattern: `maintenance_rule_set_store.rs`), `LifecycleClaimStore` in `crates/scryer-infrastructure-library/src/media/`, new columns in `media/requests.rs` (`insert_media_request_tx`, row mapper ~L760, `resolve_*`). Ports in `ports.rs`: `RequestRuleSetRepository` (mirror `MaintenanceRuleSetRepository` L5880 minus arming), `RequestRuleDecisionRepository`, `LifecycleClaimRepository`, extended `NewMediaRequest`/`MediaRequestResolution`/`MediaRequestQuery` (history counters: `count_for_requester(user_id, status, since)`, `history_for_fingerprint`). Domain: `RequestRuleSet`, `RequestRuleRevision`, `RequestRuleEvaluationMode`, `RequestRuleDecisionRecord`, `LifecycleClaim` (+ enums); `MediaRequest` gains the new columns; `MediaRequestResolvedEventData` gains `#[serde(default)] decided_by_rule_set_ids`, `decision_reason_codes`, `approved_lease_days`, `policy_tags`.

---

## 7. GraphQL (`api/graphql/schema.graphql` regenerated)

Types `crates/scryer-interface-media-types/src/types/request_rules.rs`, mappers `crates/scryer-interface-media/src/mappers/request_rules.rs`, mutations `crates/scryer-interface/src/mutation/request_rules.rs` (+ `mutation/mod.rs`), queries beside `maintenance_rule_sets` in `crates/scryer-interface-query/src/lib.rs:3197`.

- Queries: `requestRuleSets`, `requestRuleSet(id)`, `requestRuleRevisions`, `requestRuleInstanceGates`, `requestRuleDecision(requestId)`, `requestRuleDecisions(limit, outcome)`, `requestRuleInputReference`, `previewMyRequestDecision(input)` (§4.3), `titleClaims(titleId)`.
- Mutations: `createRequestRuleSet`, `updateRequestRuleMatcher`, `updateRequestRuleMetadata`, `setRequestRuleMode`, `deleteRequestRuleSet`, `validateRequestRule`, `previewRequestRule` (author-side: pick user + sample title + profile + lease; returns vote, reasons, tags, rendered input), `setRequestRuleInstanceGates`, `extendTitleClaim`, `convertTitleClaimToPermanent`, `releaseTitleClaim`.
- Existing inputs gain `requestedLeaseDays` (submit/update), `leaseDays`/`leaseForever`/`tags` (approve). `MediaRequestPayload` gains `requestedLeaseDays`, `approvedLeaseDays`, `lease { state startsAt expiresAt }`, `decision { outcome fallbackReason votes[] tags }`, `policyTags`, `metadata { contentRatings genres … }`.
- Permissions: authoring `ManageCatalogSettings` (+ `ManagePermissions` for person paths, service-side); gates `ManageSystemSettings`; claims admin library `ManageTitles`; pre-flight `Request` on the library.

---

## 8. Web (`apps/scryer-web`)

- **Navigation**: `RulesSection` + `"request"` (`/automation/rules/request`), `SettingsSection` + `requestRules`, shared Rules sidebar entry (`root-sidebar.tsx` L124–133, L386–394), experimental-gated like maintenance (`settings-container.tsx` L389–397, `routing.ts` L84–100, `routing.test.ts`). `root-sidebar.tsx` is dirty in the shared tree; work lands in the worktree.
- **Request rules pane** (`components/views/settings/settings-request-rules-section.tsx` + container): list, editor (`LazyRegoEditor`), validation, reference panel driven by `lib/contracts/request-input-contract.json` (same mechanism as `refMaint*`), template gallery (`lib/constants/request-rule-templates.ts`: the three examples + deny-adult + monthly-quota), user picker rewriting the `requesters` set, author preview form, gate toggle, recent decisions table.
- **Request dialog** (`components/root/request-media-dialog.tsx`): "Keep for" select; pre-flight banner (outcome, reasons, tags, "metadata unavailable").
- **Requests view** (`components/views/requests-view.tsx`): lease badge, decision chip + trace popover, pending tags, approver lease/tag controls in the approve dialog, claim actions on approved rows, deny reason in the requester's own list.
- **Maintenance**: template + new facts in its reference panel.
- i18n: all 10 locales; `npm run lint` before handoff.

---

## 9. Work packages

| WP | Scope | Key anchors | Tests / acceptance |
|---|---|---|---|
| **0** | Worktree + branch; `specs/0003-request-rules/{spec,plan,tasks}.md` with Constitution Check (C1 forward migrations; C2 preview before mutate; C3 deny/lease/tags visible; C5 claims reconcile inside the existing maintenance job; C6 additive API) | `specs/0002-indexer-search/` | — |
| **1** | **Policy core** (§2): `policy/` module, generic engine/evaluator/validation, move maintenance onto it, move release onto it behaviour-preserving | `maintenance.rs`, `release.rs:342–580`, `validation.rs:205–420,730–935`, `runtime.rs` | every existing `scryer-rules`, `rules/*`, `quality/*`, `post_download_gate` and `integration_rego` test green unchanged; `rule_identity()` and scoring fingerprints byte-identical; no application-crate source change needed beyond imports |
| **2** | `RequestFamily` (§3): contract JSON (+ web mirror), wrapper with `manual`/`tags`, decode, limits, synthetic input, person paths | §3 | unit tests: vote precedence, abstain, held-on-unknown, tags bounds/prefix rejection, budget, malformed heads, hash stability |
| **3** | **Hydration widening + snapshot** (§4.1): SMG selections, metadata structs, `TitleMetadataUpdate`, `MediaRequestMetadataSnapshot`, partial-failure flag, enrichment cache, fact builder incl. rating ladder, resolution parser, history counters | `metadata_gateway/*.graphql`, `scanner.rs:535,583`, `handler.rs:127,220`, `media_requests.rs:1036`, `quality/profile.rs` | gateway wiremock tests; fact-builder tests for every ladder label, "no US cert ⇒ unknown", partial snapshot ⇒ unknown facts |
| **4** | Domain + migrations 0218–0220 + stores + ports + in-memory repos | §6; `maintenance_rule_set_store.rs`; `media/requests.rs`; 0206 rebuild pattern | store tests on SQLite and Postgres; migration tests; request round-trips new columns |
| **5** | **Application**: `request_rules/{service,evaluation,arbitration,facts}.rs`, CRUD/validate/author-preview/mode/gates, engine cache, evaluation in submit/update, pre-flight use case, deny path, trace, tags on approval, event provenance, person authority | `maintenance_rules/service.rs`, `media_requests.rs:62–206,277,419,515`, `evaluation.rs:110` gate pattern | `lib_tests/request_rules.rs` (RFC §18): unauthorized never auto-approved; failure ⇒ manual; deny beats approve and permission; `manual if` holds; shadow/gate-off change nothing; re-evaluate on edit; pre-flight == submit for identical draft; tags land on the title; partial metadata ⇒ manual |
| **6** | **Leases** (§5): claims, activation hook + reconcile, release paths, admin mutations, maintenance facts, executor hold, template | `media_requests.rs:277,515`, import-completed emit site, `facts.rs:233`, `action_execution.rs:~975`, `maintenance-rule-templates.ts` | clock starts at import; overlapping leases retain to latest; forever/keep blocks delete; expired lease matches template; active lease holds an unrelated destructive rule; release on cancel/delete |
| **7** | GraphQL (§7), schema regen, `npm run test:graphql-compat` | `mutation/maintenance_rules.rs` | `crates/scryer/tests/integration_request_rules.rs`: CRUD, validate, previews (author + requester), permission gating, submit → approve/deny/manual over HTTP with trace and tags |
| **8** | Web (§8), 10 locales, `npm run lint`, `npm test`, `npm run check:react-compiler` | §8 anchors | `routing.test.ts`; contract mirror parity |
| **9** | E2E specs in the sibling `e2e` repo (real SMG at `smg.scryer.media`, so content ratings are live): `request-rules-preflight-and-auto-approve`, `request-rules-deny`, `request-rules-tags`, `request-lease-blocks-maintenance-delete`; release-notes entry; RFC 137 status note; **capstone clippy** | `e2e/playwright/tests/media-request-auto-approve.spec.ts`, `seedRequestUser`, `postScryerGraphQL` | operator runs the gate |

Rough size: ~12–14k lines app code + tests; WP1 is the riskiest (behaviour preservation), WP3/5/8 the largest.

---

## 10. Verification

- Rust per WP: `cargo test -p scryer-rules`, `cargo test -p scryer-application request_rules`, `… media_requests`, `… maintenance_evaluation`, `… rules::`, store crates on both datastores, `cargo test -p scryer --test integration_request_rules --test integration_rego --test integration_maintenance_rules`.
- Web: `npm run lint && npm test && npm run check:react-compiler && npm run test:graphql-compat`.
- Manual smoke (skill `scryer-live-api`): create the five template rules; as a non-admin open the request dialog and watch the banner change with lease/profile; submit and confirm trace, tags on the title, deny reason; approve a 1-day lease, import, confirm claim activation, run maintenance evaluation, confirm the template matches only after expiry and an active lease holds a destructive rule.
- E2E: hand the operator the four spec names.

---

## 11. Risks and follow-ups

- **Release-family refactor** could drift scoring; mitigated by keeping signatures and running the full existing test corpus as the oracle before any request code lands.
- **Content-rating coverage** is TMDB/MDBList-dependent; absent ⇒ held ⇒ manual (safe). Pre-flight surfaces it before rules go live.
- **Pre-flight SMG load**: one enrichment per dialog session via the TTL cache; the banner only re-evaluates locally-changing inputs.
- **Migration numbering** 0218–0220 vs `main`; renumber recipe exists.
- **Follow-ups (not in scope)**: no-code builder; per-library lease presets; requester-initiated extensions; joiner lease terms; Plex-watchlist origin; season/episode leases; root free-space facts (needs a disk port); maintenance-side `tags` output (core supports it; maintenance would adopt via a tag action).
