# Appendix: Merge Inventory (T081)

**Spec**: [spec.md](./spec.md) — FR-063–FR-067
**Plan**: [plan.md](./plan.md) — D8 ("Merge maps identities first")
**Task**: [tasks.md](./tasks.md) — T081
**Status**: Analysis complete; dispositions proposed. Ten items marked
`OPEN QUESTION` (§6) await adjudication before T085 (merge engine) is written.

## Purpose

D8 says the merge builds a full source→destination identity map up front, blocks
on unmappable episode-scoped records (FR-066), and then executes unions as
transactional id-rewrites at the title checkpoint. That plan is only executable
against a complete list of what actually bears a title id, an episode id, or a
media-file id. This appendix is that list.

It is also the input to two other guarantees. FR-066's blocking set — the record
types whose unmapped rows must stop the operation — is derived at the end of this
document from the per-table scope column. And FR-067's source-removal gate needs
to know which tables cascade, which silently `SET NULL`, and which have no
foreign key at all, because only the first class fails loudly when a rewrite is
missed.

## Sweep method

So a reviewer can audit coverage rather than trust it:

1. **Schema reconstruction.** All 205 files in
   `crates/scryer/src/db/migrations/` were parsed for `CREATE TABLE` blocks
   (182 distinct table names, including intermediate `_new`/`_0179`-style rebuild
   tables), then for `ALTER TABLE … ADD COLUMN` / `DROP COLUMN` / `RENAME COLUMN`
   across the full sequence, so the classification reflects the current schema
   rather than the first migration that created a table. SQLite is the canonical
   commentary per repo convention; the Postgres twins in
   `crates/scryer/src/db/postgres/migrations/` mirror the same columns.
2. **Column-name sweep.** Every reconstructed column matched against
   `title_id | episode_id | media_file_id | file_id | series_id | movie_id |
   collection_id | series_movie_link | entity_id | subject_id | target_key |
   tvdb | tmdb | imdb`.
3. **Constraint sweep.** All `REFERENCES titles|episodes|media_files|collections|
   series_movie_links|movie_entities|discovery_titles` clauses extracted with
   their `ON DELETE` action, and every `CREATE UNIQUE INDEX` in the migration set
   filtered for the entity tables.
4. **JSON / opaque-payload read.** A column scan alone misses references that
   live inside serialized payloads. Every `*_json`, `payload*`, `plan_json`,
   `data_json`, `metadata_json`, `progress_json`, `summary_json`,
   `env_payload_json`, and `payload_blob` column was traced to the Rust type that
   writes it (`crates/scryer-domain/src/lib.rs` event payloads,
   `crates/scryer-application/src/import/workflow/poller.rs`,
   `crates/scryer-application/src/location/model.rs`,
   `crates/scryer-application/src/ports.rs`).
5. **String-encoded identity sweep.** Several tables key on *composed strings*
   that embed entity ids and therefore never match a column-name pattern:
   `convergence_scope_key` in
   `crates/scryer-application/src/acquisition/convergence.rs`, and the reserved
   `scryer:*` tag namespace stored inside `titles.tags`.
6. **Liveness check.** Tables that look load-bearing were checked for production
   readers/writers, so a dead table is not given a merge rule it will never need.

## Legend

**Disposition** (matching `MergeDisposition` in
`crates/scryer-application/src/location/merge.rs`):

| Value | Meaning |
| --- | --- |
| `union` | Source rows are carried to the destination alongside the destination's own rows, with their ids rewritten. |
| `map` | Source rows are rewritten through the identity map in place; no new rows are added and none are kept side by side. |
| `destination-wins` | The destination's rows stand; the source's are discarded (usually by the cascade at FR-067). |
| `drop` | Source rows are intentionally not carried over, with a stated reason why that is safe. |

**Scope** determines whether a record falls in the FR-066 blocking set:
`episode`, `title`, `media-file`, `collection`, `link` (series-movie link), or
`operational`.

**Cascade** is the `ON DELETE` behavior when the source `titles` row is deleted at
the FR-067 gate. Three classes, and the difference matters:

- **CASCADE** — the row is deleted with the source title. A missed rewrite loses
  data, but loses it completely and consistently.
- **SET NULL** — the reference is silently blanked. A missed rewrite produces a
  surviving row with no title, which reads as an orphan in Activity and in the
  API. Invisible failure.
- **none** — no foreign key exists. The row keeps a dangling id pointing at a
  title, episode, or file that no longer exists. Completely invisible failure;
  these are the rows the FR-067 gate must assert on explicitly, because SQLite
  will not.

---

## 1. Identity map inputs

These are read to *build* the map (D8) before any write happens. Their own
dispositions are consequences of FR-063 and FR-065, not of FR-064.

| Table | Referencing columns | Scope | Disposition | Uniqueness hazards | Cascade |
| --- | --- | --- | --- | --- | --- |
| `titles` | (subject; `id`, `library_id`, `root_folder_id`, `tags`) | title | **destination-wins**, except `tags` — see §3 | `id` PK | subject of the delete |
| `episodes` | `title_id`, `collection_id` | episode | **map** — source episodes resolve onto destination episodes by season/episode/absolute/special identity; unresolvable → block (FR-066) | none | CASCADE from `titles`; `collection_id` SET NULL from `collections` |
| `collections` | `title_id` | collection | **map** — season identity; destination collection metadata wins (FR-065) | none | CASCADE |
| `series_movie_links` | `series_title_id`, `movie_entity_id`, `linked_episode_id`, `legacy_collection_id` | link | **map** — links are first-class map entries per D8 | `UNIQUE(legacy_collection_id)` — two links carrying the same legacy id cannot coexist; resolution: keep destination, null the source's legacy id before repointing | CASCADE from `titles` and `movie_entities`; `linked_episode_id` SET NULL from `episodes` |
| `movie_entities` | `imdb_id`, `tvdb_id`, `tmdb_id`, `mal_id`, `anidb_id` | — | **no-op** — shared metadata entity, not title-owned; survives independently | none | none |
| `media_files` | `title_id` | media-file | **union** — see §4 | `file_path` is globally `UNIQUE` | CASCADE |
| `file_episode_map` | `file_id`, `episode_id`, `role` | episode | **map** — see §2 | see §2 | CASCADE from both |
| `file_series_movie_link_map` | `file_id`, `series_movie_link_id` | link | **map** | PK `(file_id, series_movie_link_id)` | CASCADE from both |
| `episode_external_ids` | `title_id`, `episode_id` | episode | **destination-wins** — FR-063 gives the destination the metadata identity | `idx_episode_external_ids_unique (episode_id, source, external_id, provenance, source_scope)` | CASCADE from both |

**Key structural fact for everything below**: `media_files.id` is *stable* across
a merge. The merge repoints `media_files.title_id`; it never reissues file ids.
So every media-file-scoped child table (`subtitle_blacklist`,
`external_subtitle_probe_cache`, `quarantine_items`,
`download_import_artifacts.imported_media_file_id`,
`policy_decisions.media_file_id`, `workflow_operations.media_file_id`,
`location_operation_verifications.media_file_id`) needs **no rewrite at all** —
their key survives. The two exceptions are `file_episode_map` (whose *other* key
is an episode id that does change) and any file deduplicated away under FR-073,
whose children must be re-pointed at the surviving file or dropped with it.

---

## 2. Episode-scoped records (FR-066 territory)

Every row here references a source episode id, directly or through a scope
string. Under FR-066 an unmapped row in any of these blocks the operation.

| Table | Referencing columns / JSON paths | Disposition | Uniqueness hazards | Cascade |
| --- | --- | --- | --- | --- |
| `file_episode_map` | `episode_id`, plus `role IN ('primary','additional')` | **map** + role resolution per FR-068/069/070 | `idx_file_episode_map_one_primary_per_episode ON (episode_id) WHERE role = 'primary'` — **this partial unique index is the mechanical enforcement point for FR-068/070.** A source file arriving as `primary` for an episode the destination already covers violates it. Resolution: demote incoming to `additional` (never demote the destination's), and record the change for the preview (FR-070). FR-069 falls out naturally: the same `file_id` may hold `primary` for an uncovered episode and `additional` for a covered one, because the index is per-episode. | CASCADE from `media_files` **and** `episodes` — repointing the file alone is not enough; a row whose `episode_id` is not remapped dies with the source episode |
| `wanted_items` | `title_id`, `episode_id`, `collection_id`, `series_movie_link_id` | **union** with map | `UNIQUE(title_id, episode_id)` in the table body; `idx_wanted_items_collection_id` (partial); `idx_wanted_items_series_movie_link` (partial). All three collide when the destination already wants the same episode/season/link. Resolution: **keep destination** — its `next_search_at`, `search_count`, and `search_phase` are the live acquisition cursor; adopting the source's would reset or double-schedule searches. | CASCADE from `titles` **and** `episodes` |
| `download_submissions` | `title_id`, `episode_id`, `collection_id`, `series_movie_link_id` | **union** with map | `UNIQUE(download_client_id, download_client_type, download_client_item_id)` — client-keyed, so a union cannot collide on it | **none** — this table has no FK to `titles` or `episodes` at all. Source rows survive the FR-067 delete with dangling ids and no signal. |
| `download_submission_episode_links` | `episode_id` | **map** | PK `(download_id, episode_id)` — remapping two source episodes onto one destination episode collapses to one row; that is correct, but the collapse must be `ON CONFLICT DO NOTHING`, not an error | CASCADE from `download_submissions` only; `episode_id` has **no** FK |
| `download_import_artifacts` | `title_id`, `episode_id`, `imported_media_file_id` | **union** with map | none | all three **SET NULL** — a missed rewrite silently produces a provenance record with no title, episode, or file |
| `subtitle_downloads` | `media_file_id`, `title_id`, `episode_id` | **union**; `media_file_id` needs no rewrite, `title_id` and `episode_id` do | none | CASCADE from `media_files` and `titles`; `episode_id` has **no** FK |
| `releases` | `title_id`, `collection_id`, `episode_id` | **OPEN QUESTION 1** | none | all three SET NULL |
| `policy_decisions` | `title_id`, `collection_id`, `episode_id`, `media_file_id` | **OPEN QUESTION 2** | none | all four SET NULL |
| `workflow_operations` | `title_id`, `collection_id`, `episode_id`, `media_file_id`, `series_movie_link_id` | **map** — an operation's audit row should describe the surviving identity | none | all SET NULL |
| `domain_events` | column `title_id`; column `stream_id` **when `stream_kind = 'title'`**; and inside `payload_json`: `$.episode_ids[]`, `$.collection_id`, `$.file_id`, `$.previous_file_id`, `$.current_file_id` | **union** with map — **OPEN QUESTION 8** on payload depth | `event_id` UNIQUE (per-event, no collision) | **none** — no FK on `title_id` or `stream_id`. This is the largest silently-dangling surface in the schema. |
| `media_server_playback_items` | `entity_id` where `entity_kind = 'episode'` (and `= 'title'`) | **map** | PK `(connection_id, entity_kind, entity_id)` — collides when the destination episode is already linked for the same connection; resolution: keep destination (its `provider_item_id` is the live link) | CASCADE from `media_server_connections` only; `entity_id` has **no** FK to `titles` or `episodes` |
| `pending_releases` | `title_id`, `wanted_item_id`, `coverage_identity` (= `'scope:' || wanted_item_id`) | **OPEN QUESTION 5** | `idx_pending_releases_active_release_identity`, `idx_pending_releases_active_coverage`, `idx_pending_releases_active_unknown_age` — all partial on active rows; a union of two delay-queue entries for the same release identity violates the first | **none** — no FK to `titles` *or* `wanted_items`; rows dangle twice over |
| `release_decisions` | `title_id`, `wanted_item_id` | **OPEN QUESTION 5** | none | CASCADE from `wanted_items` only; `title_id` has **no** FK |
| `scope_indexer_coverage` | `scope_key` — a composed string: `episode:<id>`, `episode_set:b3:<blake3>`, `collection:<id>`, `series_movie:<id>`, `title:<id>` | **OPEN QUESTION 4** | PK `(scope_key, facet, indexer_id)` | **none** |
| `indexer_search_runs` | `scope_key` (same encoding) | **OPEN QUESTION 4** | none | **none** |
| `title_history` | `title_id`, `episode_id`, `collection_id` | **drop** — dead schema. It has zero production readers and zero production writers; `title_history` rows are *projected* from `domain_events` at read time by `title_history_records_from_domain_event` in `crates/scryer-application/src/events/event_views.rs`. Carrying it forward would carry nothing. | none | CASCADE from `titles`; `episode_id` and `collection_id` have **no** FK |

---

## 3. Title-scoped records

| Table | Referencing columns / paths | Disposition | Uniqueness hazards | Cascade |
| --- | --- | --- | --- | --- |
| `titles.tags` (JSON array on the title row) | user tags **and** the reserved `scryer:*` namespace | **filtered union** — see the note below; **OPEN QUESTION 9** | none (array) | subject of the delete |
| `title_external_ids` | `title_id`, plus `facet` and `library_id` projections | **destination-wins**, and the source rows must be **deleted, never repointed** | Three unique indexes: `idx_title_external_ids_lookup (title_id, source, external_id)`, `idx_title_external_ids_facet_lookup (facet, source, external_id)`, `idx_title_external_ids_library_lookup (library_id, source, external_id)`. **The collision here is not a risk, it is a certainty**: FR-055 only merges when source and destination share a canonical metadata identity, which means they share `(source, external_id)` by construction. Repointing a source row at the destination title violates all three. Resolution: destination keeps its rows; source rows go with the cascade. If any write to destination external ids happens in the same transaction, the source deletion must be ordered *before* it. | CASCADE |
| `title_aliases` | `title_id` | **union** with dedupe | `idx_title_aliases_title_alias (title_id, alias_type, alias_value)` — resolution: `ON CONFLICT DO NOTHING` | CASCADE |
| `blocklist` | `title_id` | **union** with dedupe — FR-064 names blocklists | `idx_blocklist_release_unique (title_id, indexer_id, normalized_release_name) WHERE info_hash IS NULL` and `idx_blocklist_info_hash_unique (title_id, info_hash) WHERE info_hash IS NOT NULL`. Both collide when both titles blocked the same release. Resolution: keep destination (older `created_at` and its reason text are the operator's record), `ON CONFLICT DO NOTHING`. Note 0194 removed `episode_id`/`data_json`, so blocklist is now purely title-scoped — it is *not* in the FR-066 blocking set. | CASCADE |
| `title_images`, `title_image_variants` | `title_id`; variants chain via `title_image_id` | **destination-wins** | `title_images UNIQUE (title_id, kind)` — one poster, one fanart per title; a union is structurally impossible | CASCADE (variants CASCADE from images) |
| `title_metadata_tags`, `title_metadata_tag_sources`, `title_metadata_tag_source_keys` | `title_id` (+ composite `(title_id, tag_key)` FK chain) | **destination-wins** — FR-063 keeps the destination's metadata identity, and these are hydrated from it | `title_metadata_tags` PK `(title_id, tag_key)`; children `UNIQUE (title_id, tag_key, source)` / `(title_id, tag_key, source_tag_key)` | CASCADE, three levels deep |
| `title_metadata_external_ratings`, `title_metadata_rating_sources`, `title_metadata_rating_summaries` | `title_id`, `movie_entity_id` | **destination-wins** | Paired partial unique indexes `…_title_owner` / `…_movie_owner` | CASCADE from both owners |
| `title_credits` | `title_id`, `movie_entity_id` | **destination-wins** | `idx_title_credits_title_owner (title_id, position) WHERE title_id IS NOT NULL`; XOR `CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL))` — a rewrite must not produce a row with both | CASCADE from both |
| `title_search_terms` | `title_id` | **drop and rebuild** — a derived spellfix projection, cheaper and safer to regenerate than to merge | `idx_title_search_terms_title_kind_normalized (title_id, term_kind, normalized_term)` | CASCADE |
| `collection_external_ids` | `title_id`, `collection_id` | **destination-wins** — FR-065 gives duplicate collection metadata to the destination | `idx_collection_external_ids_unique (collection_id, source, external_id, provenance, source_scope)` | CASCADE from both |
| `library_probe_signatures` | `title_id` (PK) | **destination-wins** — the signature describes a folder path, and after the move the destination's path is the live one; the source's is stale by definition | PK `title_id` — one row per title, so union is impossible | CASCADE |
| `indexer_search_learning` | `title_id` | **OPEN QUESTION 3** | PK `(indexer_id, title_id, facet, strategy_key)` — guaranteed collision when both titles were searched on the same indexer with the same strategy | **none** |
| `post_processing_script_runs` | `title_id`; also denormalized `title_name`, `facet`, `file_path`, and `env_payload_json` | **union**, rewriting `title_id` only. `title_name` is a deliberate historical snapshot ("denormalized for history" in 0051) and must **not** be rewritten — doing so would falsify what the script actually saw. Same for `env_payload_json`. | none | **none** |
| `release_download_attempts` | `title_id` | **union** | none | SET NULL |
| `manual_import_selections`, `manual_import_selection_candidates` | `title_id`; candidates chain via `selection_id` | **map** — but in practice unreachable: an unconsumed manual-import selection is an active import, and FR-086 blocks the title before the merge starts. Handle as **block**, with `map` as the fallback if the gate is ever relaxed. | `manual_import_selection_candidates UNIQUE (selection_id, canonical_path)` | **none** |
| `media_requests` | `created_title_id`, `library_id` | **union** with map — FR-064 names requests explicitly; **OPEN QUESTION 10** on the library side | none | `created_title_id` SET NULL; `library_id` CASCADE from `libraries` |
| `media_request_external_ids`, `media_request_requesters` | (chain via `request_id`) | **follows parent** | — | CASCADE from `media_requests` |
| `imports` | `payload_json` → `$.target_title_id` (legacy alias `$.manual_title_id`), inside `StoredCompletedImportRequestPayload::Current` | **map** — a JSON rewrite, not a column update. `crates/scryer-application/src/import/workflow/poller.rs:156`. The `Legacy(CompletedDownload)` variant carries no title id. | none | **none** |
| `discovery_titles` | `resolved_title_id` | **map** | `UNIQUE (target_key_norm, language)` — not title-keyed, so a remap is safe | SET NULL |
| `discovery_item_library_provenance` | `title_id`, `library_id` | **map** with dedupe | `UNIQUE (item_id, subject_key, title_id, library_id)` — remapping source onto destination can duplicate an existing row; resolution: `ON CONFLICT DO NOTHING`. Note `library_id` also changes on a cross-library merge. | CASCADE from `discovery_items` only; `title_id` has **no** FK |
| `discovery_submitted_subjects` | `title_id`, `library_id` | **map** | none | SET NULL |
| `discovery_pending_context_changes` | `title_id`, `previous_title_id` | **map** — both columns | none | `title_id` SET NULL; `previous_title_id` has **no** FK |
| `title_more_like_this_items_new`, `title_recommendation_cards` | `source_title_id`; cards keyed by `discovery_title_id` | **drop and rebuild** — a recommendation cache | `UNIQUE (source_title_id, discovery_title_id)` | CASCADE |

### `titles.tags` is not a plain tag set

FR-064 says "additive data is unioned: **tags**, history, requests…". Taken
literally against this schema, that instruction is wrong, and the merge engine
must not follow it literally.

`titles.tags` is a `TEXT NOT NULL DEFAULT '[]'` JSON array that stores user tags
**and** a reserved `scryer:` namespace carrying per-title configuration. The
prefixes, confirmed against
`crates/scryer-infrastructure-library/src/media/titles/store.rs:3380` and
`crates/scryer-application/src/ports.rs:33`:

| Prefix | Carries | Governing FR |
| --- | --- | --- |
| `scryer:quality-profile:` | the title's quality profile assignment | FR-063 (quality configuration) |
| `scryer:monitor-type:` | monitoring mode | FR-063 (monitoring) |
| `scryer:monitor-specials:` | specials monitoring | FR-063 |
| `scryer:filler-policy:` | filler handling | FR-063 (explicit settings) |
| `scryer:recap-policy:` | recap handling | FR-063 |
| `scryer:inter-season-movies:` | series-movie inclusion | FR-063 / FR-060 |
| `scryer:season-folder:` | season-folder layout | FR-058 / FR-063 (naming behavior) |
| `scryer:root-folder:` | legacy root assignment (superseded by `titles.root_folder_id`) | the move itself |
| `scryer:mal-score:`, `scryer:anime-media-type:`, `scryer:anime-status:` | metadata-derived, stripped on re-match (`REMATCH_DERIVED_TAG_PREFIXES`) | FR-063 (metadata identity) |

A naive set-union produces a merged title carrying *two* `scryer:quality-profile:`
tags and *two* `scryer:monitor-type:` tags. Every reader in the codebase uses
`find_map(strip_prefix(...))` — first match wins — so the outcome is a silent,
order-dependent violation of FR-063 for exactly the settings FR-063 names.

**Proposed rule**: partition `titles.tags` on the `scryer:` prefix. Free-form user
tags **union** (deduped, case-folded consistently with existing tag handling).
Reserved-namespace tags are **destination-wins** and the source's are dropped. Any
reserved tag where source and destination disagree is a preview line item under
FR-071 ("which values are dropped or converted"). See OPEN QUESTION 9.

---

## 4. Media-file-scoped records

| Table | Referencing columns | Disposition | Uniqueness hazards | Cascade |
| --- | --- | --- | --- | --- |
| `media_files` | `title_id`, `role` | **union** — repoint `title_id`; `id` and `file_path` are untouched | `file_path` is **globally** `UNIQUE`. The merge does not move bytes (the move already happened by this checkpoint), so a collision here means two catalog rows already claim one path — a pre-existing corruption the merge must surface, not paper over. FR-072/073/074 handle the filesystem-level collision earlier. | CASCADE |
| `file_episode_map` | `file_id`, `episode_id`, `role` | **map** — see §2 | see §2 | CASCADE from both |
| `file_series_movie_link_map` | `file_id`, `series_movie_link_id` | **map** — the link id changes, the file id does not | PK `(file_id, series_movie_link_id)`; two source links mapping onto one destination link collapse — `ON CONFLICT DO NOTHING` | CASCADE from both |
| `subtitle_blacklist` | `media_file_id` | **no rewrite needed** (file id stable); carried implicitly | `UNIQUE (media_file_id, provider, provider_file_id)` | **none** — survives even file deletion |
| `external_subtitle_probe_cache` | `media_file_id`, `file_path` | **no rewrite needed** | PK `(media_file_id, file_path)` | CASCADE from `media_files` |
| `quarantine_items` | `media_file_id`, `file_path` | **no rewrite needed** | none | SET NULL |
| `location_operation_verifications` | `title_id`, `media_file_id` | **map** on `title_id`; `media_file_id` stable | none | CASCADE from `location_operations` only; `title_id` and `media_file_id` have **no** FK |

---

## 5. Operational bookkeeping

| Table | Referencing columns / paths | Disposition | Notes |
| --- | --- | --- | --- |
| `location_operations` | `plan_json` (serialized plan carrying per-title entries: `title_id`, `merged_into_title_id`), `source_library_id`, `destination_*` | **map** for a *concurrently in-flight or resumable* operation; **frozen** for a completed one | The confirmed plan is a historical artifact for a finished operation — rewriting it would falsify what was confirmed and invalidate `plan_fingerprint` (FR-081/FR-089). But D7's ownership registry should already have prevented a second live operation from holding the same title. **OPEN QUESTION 7** covers the resumable middle case. |
| `location_operation_title_checkpoints` | `title_id`, `merged_into_title_id` | **map on `merged_into_title_id` only** | `title_id` is part of the PK `(operation_id, title_id)` and records which title the operation *processed*. Rewriting it would erase the merge's own audit trail. `merged_into_title_id` is the forward pointer and is written by this very merge. |
| `location_operation_owned_entities` | `entity_id` where `entity_type = 'title'` | **map** — a live claim must follow the surviving title, or the ownership guard protects a ghost | PK `(operation_id, entity_type, entity_id)`; CASCADE from `location_operations` |
| `history_events` | `title_id` | **drop** — legacy. Only surviving production SQL is a housekeeping `DELETE` in `crates/scryer-infrastructure-library/src/media/libraries/state_store/store.rs:1315` and a delete-by-title path; no reads, no inserts. | SET NULL |
| `event_outboxes` | (chains to `history_events`) | **drop** — follows its dead parent | CASCADE from `history_events` |
| `download_identity_states`, `download_client_bindings`, `downloads`, `download_queue_commands`, `download_jobs` | keyed on download/client identity, not on catalog entities | **no-op** | `download_jobs` reaches the catalog only via `release_id → releases`, covered by OQ1 |
| `library_scan_unmatched_items` | `library_id`, `item_path` — no title id (unmatched is the point) | **no-op** for the merge; the *move* may invalidate `item_path`, which is a US1–US5 concern, not US7 | none |
| `library_root_id_remaps` | root ids only | **no-op** | |
| `scheduler_jobs` | `payload_json` — job names and cadence; no catalog ids observed | **no-op** | |
| `upstream_scheduler_states`, `upstream_scheduler_rss_cadence`, `upstream_destination_cooldowns`, `indexer_api_quotas`, `indexer_errors`, `indexer_system_backoffs` | host/indexer keyed | **no-op** | |
| `indexer_search_candidates`, `indexer_search_candidate_sources`, `indexer_search_run_candidate_sources` | `response_tvdb_id` / `response_tmdb_id` / `response_imdb_id` are *provider* ids echoed back, not Scryer ids | **no-op** — but they expire; `reusable_until` / `expires_at` bound the staleness | |
| `settings_values` | `scope`, `scope_id` | **no-op** — verified: every production call site uses the `"system"` scope (`get_setting_json("system", …)`); no title-scoped settings row is ever written | |
| `notification_subscriptions` | `scope`, `scope_id` | **no-op** — verified: the only scope literal written is `"global"` (`crates/scryer-infrastructure-notifications/src/notifications/store.rs:522`) | If per-title notification scoping is ever added, this row moves into §3 as a `union`. |
| `quality_profiles` and its seven allow/block-list children | `scope`, `scope_id` | **no-op** — profiles are not title-owned; a title's profile lives in `titles.tags` (§3) | |
| `rule_sets`, `rule_set_assignments`, `rule_set_history`, `quality_rules`, `seeding_profiles` | no catalog ids | **no-op** | `policy_decisions.rule_set_id` is the only bridge, covered by OQ2 |
| `plugin_installations`, `plugin_catalog_sources`, `plugin_catalog_status` | none | **no-op** — no plugin table carries a title reference | |
| `image_proxy_sources` | `owner_type`, `owner_id` | **map** where `owner_type` denotes a title | Cache-class; a stale token degrades to the fallback class rather than erroring. Low priority, but it belongs on the list. |
| `external_import_monitor_snapshots`, `…_chunks` | `payload_json` / `payload_ndjson` — external-instance entries keyed by *external* ids | **no-op** | |

### Off-database title references

One reference lives outside the database entirely and would be missed by any
schema sweep:

| Location | Reference | Disposition |
| --- | --- | --- |
| Recycle-bin entry `manifest.json` (`crates/scryer-application/src/library/recycle_bin.rs:47`, `RecycleManifest.title_id: Option<String>`) | `$.title_id` in a per-entry JSON file under `.scryer-recycle/` | **OPEN QUESTION 6** |

---

## 6. Open questions

Each of these has a defensible answer on both sides. They are for the
maintainers to adjudicate, not for the implementer to guess.

**OQ1 — `releases`: map or drop?**
`releases` is the indexer result cache: `title_id`, `collection_id`, `episode_id`,
`raw_payload_json`, `last_seen_at`. *Map* keeps the destination's interactive
search view populated immediately after the merge and preserves
`download_jobs.release_id → releases` and `quarantine_items.release_id` linkage.
*Drop* is simpler and self-healing — the next RSS or interactive search
repopulates within one cadence — but it nulls `release_id` on any surviving
`download_jobs` and `policy_decisions` rows (both SET NULL), losing the trail from
a queued job back to what it grabbed. Weight: the linkage argues for map; the
volume (this is the highest-row-count table in the set) argues for drop.

**OQ2 — `policy_decisions`: map or drop?**
Pure scoring diagnostics — four SET NULL entity refs plus `reason_json`. *Map*
preserves "why did Scryer grab this" for the merged title. *Drop* accepts that a
merge is a discontinuity and that decisions made against the source title's
profile and library inheritance are misleading once the destination's
configuration wins (FR-063). The second argument is the stronger one on
correctness grounds, and it also disposes of the `rule_set_id` question (the
source's rule set may not even apply in the destination library).

**OQ3 — `indexer_search_learning`: additive union, destination-wins, or drop?**
PK `(indexer_id, title_id, facet, strategy_key)` with counters `attempts`,
`empty_successes`, `usable_successes`, and a `suppressed` flag. Collision is
guaranteed on any indexer both titles were searched on. *Additive* (sum the
counters, OR the suppression, max the timestamps) preserves the most learning but
is the only disposition in this whole document that computes a new value rather
than choosing an existing one — a novel merge semantic worth being deliberate
about. *Destination-wins* is consistent with FR-063's spirit. *Drop* is safe
(learning re-accumulates) but re-opens suppressed strategies, which can produce a
burst of known-useless queries against an indexer immediately after a merge.

**OQ4 — `scope_indexer_coverage` / `indexer_search_runs.scope_key`: partial
rewrite or wholesale drop?**
`scope_key` is a composed string from `convergence_scope_key`
(`crates/scryer-application/src/acquisition/convergence.rs:497`). Four of its five
forms are reversible — `title:<id>`, `episode:<id>`, `collection:<id>`,
`series_movie:<id>` — and could be rewritten by parsing and re-composing. The
fifth, `episode_set:b3:<hex>`, is a BLAKE3 hash over the sorted episode-id list
(`HashDomain::EpisodeSetScope`) and is **irreversible**: the original episode set
cannot be recovered from the key, so it cannot be recomputed against the mapped
ids. Options: (a) drop every scope row for the source title — coverage
re-accumulates, at the cost of one redundant search sweep per indexer; (b) rewrite
the four reversible forms and drop only the `episode_set` rows, which leaves
season-pack coverage inconsistent with episode coverage for one cycle. Note this
is *not* an FR-066 blocking case in the strict sense — nothing is being attached
to a guessed identity, the rows are being discarded — but it is the one place
where the identity map provably cannot be applied.

**OQ5 — In-flight acquisition (`pending_releases`, `release_decisions`,
`wanted_items`): carry or block?**
`pending_releases` is the delay-queue: rows with `status = 'waiting'` and a
`delay_until` in the future represent a decision Scryer has already made and will
act on. Carrying them means the merged title grabs a release that was chosen
against the *source* title's quality profile — which FR-063 has just replaced.
Blocking the merge until the delay queue drains is the conservative reading of
FR-067 ("every required relationship has been transferred or intentionally
resolved") but could stall a merge for the full delay window. A third option:
carry the rows but force re-evaluation against the destination profile before the
grab. Same question applies to `wanted_items` rows in `status = 'grabbed'`.

**OQ6 — Recycle-bin manifests: rewrite on disk or leave stale?**
Each recycle entry directory holds a `manifest.json` with `title_id`. *Rewrite*
keeps restore targeting correct but means the merge performs filesystem writes
outside the database transaction — it cannot be rolled back with the rest of the
union, and a partially rewritten recycle bin is worse than a uniformly stale one.
*Leave stale* means a restore from an entry recycled under the source title finds
no such title and fails or orphans. Given the manifest's `title_id` is
`Option<String>`, a third option is to *null* it, degrading restore to a manual
placement rather than a broken one.

**OQ7 — In-flight `location_operations.plan_json` referencing the source title.**
D7's ownership registry should make a concurrent live operation on the same title
impossible. But a *resumable* operation — one that was interrupted and whose
checkpoints are still `pending` — can hold the source title id in its plan while
this merge deletes it. Options: (a) treat a resumable operation holding the source
title as a hard block (consistent with FR-086's active-work gate); (b) rewrite the
plan and re-derive `plan_fingerprint`, which breaks FR-089's staleness contract
because the user confirmed a different plan; (c) mark the interrupted operation
failed with an explicit reason. (a) and (c) are both defensible; (b) is probably
not.

**OQ8 — `domain_events` payloads: full rewrite or column-only?**
This is the FR-066 crux, and the most expensive item in the document.
`payload_json` is a `BLOB` holding zstd-compressed JSON with a shared dictionary
(`crates/scryer-infrastructure-sql/src/domain_event_payload.rs`). It is
**not** rewritable by SQL — every affected event must be read, decompressed,
deserialized, id-mapped, re-serialized, and recompressed in Rust, inside the
title-checkpoint transaction. For a long-lived series with thousands of events
that is a substantial unit of work at exactly the point the operation is meant to
be resumable.
*Full rewrite* is what FR-066 requires read literally: `$.episode_ids[]` is "a
record referencing source episode ids," and Activity's episode filter
(`activity_api.rs` matches on `record.episode_id`) breaks without it.
*Column-only* — rewrite `title_id` and `stream_id` (both plain TEXT columns, both
FK-free) and leave payload episode ids stale — is orders of magnitude cheaper and
keeps the title's Activity feed intact, at the cost of per-episode Activity
filtering silently missing pre-merge events.
A middle path: rewrite payloads only for event types that carry `episode_ids`
(`ReleaseGrabbed`, `DownloadFailed`, `ReleaseBlocklisted`, `ImportCompleted`,
`ImportRejected`, `MediaFileAnalyzed`, `MediaFileRenamed`, `MediaFileDeleted`,
`MediaFileUpgraded`), which is a minority of rows, and skip the rest.
Note also that `TitleContextSnapshot` (embedded in nearly every payload) contains
**no** title id — only `title_name`, `facet`, `external_ids`, `poster_url`,
`year`. It is a display snapshot and should be left alone under any option.

**OQ9 — Reserved `scryer:*` tags: silent drop or preview conflict?**
Per §3, source reserved tags must not survive alongside the destination's. The
question is whether a *differing* value is a silent drop (destination wins,
mentioned only in the FR-071 summary line) or an explicit per-setting conflict row
in the preview. FR-071 requires the preview to summarize "which values are dropped
or converted," which argues for at least naming each differing setting. FR-057
already sets the precedent for enumerating every setting that changes meaning on a
facet conversion.

**OQ10 — `media_requests.library_id` on a cross-library merge.**
FR-064 names requests as union data, and `created_title_id` maps cleanly. But
`media_requests.library_id` has a `CASCADE` FK to `libraries` and represents *which
library the user requested into*. After a cross-library merge the created title
lives elsewhere. *Repoint* `library_id` to the destination and the request history
follows the content, but the record no longer reflects what the user actually
asked for. *Leave* it and the request stays truthful but points at a title in a
library it does not belong to — and the requester's library permissions
(`user_library_permission_masks`) may no longer grant them visibility of the
result.

---

## 7. The FR-066 blocking set

FR-066: "Episode-identity mapping applies to every episode-scoped record being
unioned … Ambiguous episode or special identities block the operation rather than
attaching records to guessed identities."

The merge plan MUST block when any source row in the following record types
references an episode id that the identity map cannot resolve to exactly one
destination episode. This is the exact list; the title checkpoint enters
`Blocked` with `blocked_reason` naming the table and the unmapped episode.

1. `episodes` — the map itself; an unmappable source episode blocks immediately.
2. `file_episode_map` — FR-065/068's core; also the `role` uniqueness resolution.
3. `episode_external_ids` — destination-wins, but an ambiguous episode means the
   map is ambiguous, so it blocks upstream at (1).
4. `wanted_items` (`episode_id`)
5. `download_submissions` (`episode_id`)
6. `download_submission_episode_links` (`episode_id`)
7. `download_import_artifacts` (`episode_id`)
8. `subtitle_downloads` (`episode_id`)
9. `domain_events` (`payload_json → $.episode_ids[]`) — subject to OQ8; if the
   column-only option wins, this drops off the blocking set and the preview must
   say so explicitly under FR-071.
10. `media_server_playback_items` (`entity_id` where `entity_kind = 'episode'`)
11. `workflow_operations` (`episode_id`)
12. `releases` (`episode_id`) — only if OQ1 resolves to `map`
13. `policy_decisions` (`episode_id`) — only if OQ2 resolves to `map`
14. `pending_releases` / `release_decisions` — episode-scoped transitively through
    `wanted_item_id`; blocking behavior follows OQ5

Deliberately **not** in the blocking set, with reasons:

- `blocklist` — title-scoped since 0194 removed `episode_id` and `data_json`.
- `title_history` — dead schema (no production reads or writes).
- `scope_indexer_coverage` / `indexer_search_runs` — the `episode_set:b3:` key is
  irreversible, so these are dropped rather than mapped; nothing is attached to a
  guessed identity, so FR-066's hazard does not arise (OQ4).
- Everything media-file-scoped — `media_files.id` is stable across the merge.

---

## 8. Table-group ordering for D8

D8 executes "transactional unions per table group at the title checkpoint." The
order below is forced by the foreign-key and cascade structure, not chosen for
convenience. The whole of Groups 1–5 runs in **one** transaction per title
checkpoint; Group 0 is read-only and precedes it; Group 6 follows it.

**Group 0 — Build the map (read-only, outside the write transaction).**
`titles`, `episodes`, `collections`, `series_movie_links`, `episode_external_ids`,
`media_files`, `file_episode_map`, `file_series_movie_link_map`. Produces the full
source→destination identity map plus the unmapped set. **The FR-066 block is
decided here**, before anything is written, so a blocked title costs no rollback.

**Group 1 — Media ownership flip.**
`media_files.title_id` → destination, then `file_episode_map` (episode remap +
FR-068/069/070 role resolution against
`idx_file_episode_map_one_primary_per_episode`), then
`file_series_movie_link_map`.
*Why first*: `media_files` CASCADEs from `titles`. Repointing the file rows is
what saves them — and, transitively, `subtitle_downloads`,
`external_subtitle_probe_cache`, and every other file-keyed child — from the
Group 5 cascade. And `file_episode_map` CASCADEs from `media_files` **and**
`episodes`, so repointing the file alone is insufficient; the episode side must be
remapped in the same group, while both sets of episodes still exist.

**Group 2 — Episode-scoped operational rows.**
`wanted_items`, `download_submissions`, `download_submission_episode_links`,
`download_import_artifacts`, `subtitle_downloads`, `workflow_operations`,
`domain_events`, `media_server_playback_items`, plus `releases` /
`policy_decisions` / `pending_releases` / `release_decisions` per OQ1/OQ2/OQ5.
*Why here*: every one of these keys on an episode id that Group 5 destroys. They
must be rewritten while the source episodes are still resolvable. Within the
group, `download_submission_episode_links` follows `download_submissions` (FK).

**Group 3 — Title-scoped additive unions.**
`blocklist`, `title_aliases`, `release_download_attempts`,
`post_processing_script_runs`, `media_requests`, `imports.payload_json`,
`indexer_search_learning`, `discovery_titles`,
`discovery_item_library_provenance`, `discovery_submitted_subjects`,
`discovery_pending_context_changes`, `image_proxy_sources`,
`location_operation_owned_entities`, `location_operation_title_checkpoints`
(`merged_into_title_id`).
*Why after Group 2*: nothing forces it structurally, but ordering title-scoped
work after episode-scoped work means a failure in the expensive, blocking-prone
part rolls back the cheap part rather than the reverse. Every conflicting insert
here resolves `ON CONFLICT DO NOTHING` against the unique indexes named in §3.

**Group 4 — Destination-wins deletions that must precede any destination write.**
`title_external_ids` source rows, and — only if the merge writes to them —
`title_images`, `title_metadata_*`, `title_credits`, `collection_external_ids`,
`episode_external_ids`, `library_probe_signatures`.
*Why a separate group*: `title_external_ids` carries three unique indexes,
including `(library_id, source, external_id)` and `(facet, source, external_id)`,
and FR-055 guarantees source and destination share `(source, external_id)`. If
anything writes destination external ids in this transaction, the source rows must
already be gone. In the common case where the merge writes nothing here, this
group is empty and the Group 5 cascade does the work.

**Group 5 — Source removal (the FR-067 gate).**
Delete the source `titles` row. The cascade then removes `collections` → sets
`episodes.collection_id` null → removes `episodes` → and all remaining CASCADE
children.
*Before the delete*, the gate must assert that no rows remain referencing source
ids in the **no-FK** tables, because nothing else will:
`domain_events` (`title_id`, `stream_id`), `download_submissions`,
`subtitle_downloads.episode_id`, `subtitle_blacklist`,
`post_processing_script_runs`, `manual_import_selections`,
`indexer_search_learning`, `media_server_playback_items`,
`discovery_item_library_provenance`, `discovery_pending_context_changes.previous_title_id`,
`pending_releases`, `release_decisions.title_id`,
`location_operation_verifications`, `title_history`.
The **SET NULL** tables are the second-priority assertion, since they fail quietly
rather than not at all: `history_events`, `releases`, `policy_decisions`,
`workflow_operations`, `download_import_artifacts`, `media_requests`,
`quarantine_items`, `release_download_attempts`, `discovery_titles`,
`discovery_submitted_subjects`.

**Group 6 — Post-transaction rebuilds (outside the transaction, idempotent).**
`title_search_terms` reindex, `title_more_like_this_items_new` /
`title_recommendation_cards` regeneration, statistics recomputation, and the
`scope_indexer_coverage` / `indexer_search_runs` drop per OQ4. All are derived
caches: a crash between Group 5 and Group 6 leaves a correct catalog with a stale
search index, which the next scheduled job repairs.

---

## 9. Coverage statement

**Scanned.** All 205 SQLite migrations in `crates/scryer/src/db/migrations/`
(182 distinct `CREATE TABLE` names, reduced to the current live schema by
replaying every `ALTER TABLE`); all `REFERENCES` clauses to the six catalog
tables with their `ON DELETE` actions; all `CREATE UNIQUE INDEX` statements
touching those tables; every JSON- and blob-bearing column traced to its Rust
writer; the composed-string identity encodings in `convergence.rs`; the reserved
`scryer:*` tag namespace; and the 74 repository traits in
`crates/scryer-application/src/ports.rs` for record types whose entity references
are indirect.

**Classified.** 65 tables bear a title, episode, collection, series-movie-link, or
media-file reference (directly, through a JSON payload, or through a composed
identity string) and carry a disposition in §1–§5. A further 36 were examined and
explicitly classified `no-op` in §5, with the reason recorded. One reference lives
outside the database entirely (recycle-bin `manifest.json`).

**Not classified — needs a decision, not more analysis.** The ten `OPEN QUESTION`
items in §6. Each has both options stated with the trade-off; none is blocked on
further code reading.

**Known limits of this sweep.**

1. **Plugin-authored storage.** No plugin table in the schema carries a title
   reference, and the plugin host surface
   (`crates/scryer-plugins/src/wasmtime_host/`) exposes no catalog-id persistence.
   But plugins receive title context at call time; if a future plugin persists a
   title id in its own key-value space, it is outside this inventory by
   construction.
2. **Postgres divergence.** The Postgres migration set is consolidated
   differently (82 files vs 205). The column sets are documented to mirror, and
   the location tables (0204–0206) exist in both, but the *unique index* set was
   verified against SQLite only. Before T085 ships, the partial-index hazards
   named in §2 and §3 — particularly
   `idx_file_episode_map_one_primary_per_episode` and the two `blocklist`
   partials — should be confirmed present and equivalently defined on Postgres,
   since a merge that relies on `ON CONFLICT` against an index that exists on only
   one engine will behave differently per deployment.
   *Review note (2026-08-31)*: the three indexes named above were verified present
   on Postgres (`postgres/migrations/0158`, `0194`); the remaining unique indexes
   in §2–§3 still need the same check before T085 ships.
3. **Grab-planner data.** T081's brief names "grab-planner data (season-set
   claims, fingerprints)" as a target. It has **no dedicated tables**: the
   season-pack arbitration introduced in `e30b9e21b` lives in in-process state in
   `crates/scryer-application/src/acquisition/workflow/task_runner.rs` and
   `convergence.rs`. Its only persisted surfaces are `wanted_items`,
   `pending_releases` (`coverage_identity`, `role`, `release_identity`), and the
   `scope_key` coverage tables — all inventoried above. Nothing survives a
   restart, so nothing needs merging beyond those rows.
4. **`payload_json` in `scheduler_jobs`, `download_queue_commands`, and
   `event_outboxes`** was read for catalog ids and none were found, but these are
   free-form JSON columns without a single owning Rust type, so the finding is
   "none observed" rather than "none possible."
