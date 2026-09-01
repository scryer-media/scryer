//! Persistence for the US7 merge engine (T085): the Group 0 read and the
//! Groups 1–5 transaction from `merge-inventory.md` §8.
//!
//! Dual-engine through [`SqlRuntime`], following the `location_operation_store`
//! precedent. Every statement here is written to render identically on sqlite
//! and postgres — no engine-conditional SQL — which is possible because the
//! merge relies on `ON CONFLICT DO NOTHING` (target-less, so it covers any
//! constraint on both engines) and on `EXISTS` pre-deletes rather than on
//! upserts against a named index.
//!
//! # Group ordering is forced, not chosen
//!
//! | Group | What | Why it must be here |
//! |---|---|---|
//! | 1 | `media_files` → `file_episode_map` → `file_series_movie_link_map` | `media_files` CASCADEs from `titles`; repointing it is what saves every file-keyed child from the Group 5 cascade. `file_episode_map` CASCADEs from `media_files` **and** `episodes`, so the episode side must be remapped while both sets of episodes still exist. |
//! | 2 | episode-scoped operational rows | Every one keys on an episode id that Group 5 destroys. |
//! | 3 | title-scoped unions | Ordered after Group 2 so a failure in the expensive, blocking-prone part rolls back the cheap part rather than the reverse. |
//! | 4 | `title_external_ids` source deletion | FR-055 guarantees both sides share `(source, external_id)`, and `idx_title_external_ids_library_lookup` is unique on `(library_id, source, external_id)`. The source rows go before anything could write destination external ids. |
//! | 5 | the FR-067 gate, then `DELETE FROM titles` | The gate asserts on the no-FK and SET NULL lists first, because nothing in the database will. |
//!
//! Group 6 is *not* here: it is returned as [`MergeOutcome::post_merge_work`]
//! for the caller to schedule.
//!
//! # Live-schema deviations
//!
//! `merge-inventory.md` lists five tables the live schema does not have —
//! `releases`, `policy_decisions`, `title_aliases`, `title_history`,
//! `quarantine_items` — because its reconstruction replayed `CREATE`/`ALTER`
//! but not `DROP TABLE`. None is touched here; the full list, with the
//! migration that removed each, is
//! `scryer_application::location::merge::engine::INVENTORY_DEVIATIONS`.

use async_trait::async_trait;
use scryer_application::location::merge::MergedMediaRole;
use scryer_application::location::merge::engine::{
    FR067_NO_FK_ASSERTIONS, FR067_SET_NULL_ASSERTIONS, MergeCatalogSnapshot, MergeGateIdKind,
    MergeOutcome, MergePlan, OQ8_EPISODE_BEARING_EVENT_TYPES, TitleMergeRepository,
};
use scryer_application::location::merge::map::{
    CollectionIdentityFacts, EpisodeIdentityFacts, FR066_BLOCKING_TABLES, MergeIdentityMap,
    SeriesMovieLinkIdentityFacts,
};
use scryer_application::location::merge::roles::FileEpisodeRoleRow;
use scryer_application::location::merge::summary::PostMergeWork;
use scryer_application::{AppError, AppResult};
use scryer_domain::{CollectionType, EpisodeType};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};
use scryer_infrastructure_sql::domain_event_payload::{
    decode_domain_event_payload, encode_domain_event_payload,
};

/// Operation states a location operation can no longer be resumed from, so an
/// operation in one of them cannot be the OQ7 hazard.
const TERMINAL_OPERATION_STATES: &str = "'completed', 'completed_with_warnings', 'canceled',
    'failed'";

/// Checkpoint states that mean the operation still intends to process the
/// title.
const LIVE_CHECKPOINT_STATES: &str = "'pending', 'moving', 'verifying', 'reconciling',
    'cleaning_up'";

/// Tables whose source row counts the FR-071 preview reports.
const COUNTED_TABLES: &[(&str, &str)] = &[
    ("media_files", "title_id"),
    ("wanted_items", "title_id"),
    ("download_submissions", "title_id"),
    ("download_import_artifacts", "title_id"),
    ("subtitle_downloads", "title_id"),
    ("workflow_operations", "title_id"),
    ("domain_events", "title_id"),
    ("blocklist", "title_id"),
    ("release_download_attempts", "title_id"),
    ("post_processing_script_runs", "title_id"),
    ("media_requests", "created_title_id"),
    ("indexer_search_learning", "title_id"),
    ("discovery_titles", "resolved_title_id"),
    ("discovery_item_library_provenance", "title_id"),
    ("discovery_submitted_subjects", "title_id"),
    ("discovery_pending_context_changes", "title_id"),
    ("location_operation_verifications", "title_id"),
    ("manual_import_selections", "title_id"),
    ("pending_releases", "title_id"),
    ("release_decisions", "title_id"),
    ("history_events", "title_id"),
    ("title_search_terms", "title_id"),
    ("title_external_ids", "title_id"),
];

#[derive(Clone)]
pub struct TitleMergeStore {
    datastore: StoreDatastore,
}

impl TitleMergeStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl TitleMergeRepository for TitleMergeStore {
    async fn load_merge_snapshot(
        &self,
        source_title_id: &str,
        destination_title_id: &str,
        current_operation_id: Option<&str>,
    ) -> AppResult<MergeCatalogSnapshot> {
        let exec = || self.datastore.read_exec();

        let (source_library_id, source_tags) = load_title_shape(exec(), source_title_id).await?;
        let (destination_library_id, destination_tags) =
            load_title_shape(exec(), destination_title_id).await?;

        let source_episodes = load_episodes(exec(), source_title_id).await?;
        let destination_episodes = load_episodes(exec(), destination_title_id).await?;
        let source_collections = load_collections(exec(), source_title_id).await?;
        let destination_collections = load_collections(exec(), destination_title_id).await?;
        let source_links = load_links(exec(), source_title_id).await?;
        let destination_links = load_links(exec(), destination_title_id).await?;
        let source_file_episode_rows = load_file_episode_rows(exec(), source_title_id).await?;
        let destination_file_episode_rows =
            load_file_episode_rows(exec(), destination_title_id).await?;

        let mut episode_references = BTreeMap::new();
        episode_references.insert(
            "file_episode_map".to_string(),
            source_file_episode_rows
                .iter()
                .map(|row| row.episode_id.clone())
                .collect::<BTreeSet<_>>(),
        );
        for (table, sql) in EPISODE_REFERENCE_QUERIES {
            let ids = load_episode_reference_ids(exec(), sql, source_title_id).await?;
            if !ids.is_empty() {
                episode_references.insert((*table).to_string(), ids);
            }
        }
        let domain_event_episodes =
            load_domain_event_episode_ids(exec(), source_title_id).await?;
        if !domain_event_episodes.is_empty() {
            episode_references.insert("domain_events".to_string(), domain_event_episodes);
        }
        debug_assert!(
            episode_references
                .keys()
                .all(|table| FR066_BLOCKING_TABLES.contains(&table.as_str())),
            "every reference query must name a table in the FR-066 blocking set"
        );

        let mut source_row_counts = BTreeMap::new();
        for (table, column) in COUNTED_TABLES {
            let count = count_rows(exec(), table, column, source_title_id).await?;
            if count > 0 {
                source_row_counts.insert((*table).to_string(), count);
            }
        }
        // The two scope tables key on a composed string, not on a column, so
        // their counts come from a LIKE over the four reversible key forms plus
        // every `episode_set:b3:` row the title could have produced. Counting
        // the reversible forms is enough for the preview: OQ4 drops all of them
        // either way.
        source_row_counts.insert(
            "scope_indexer_coverage".to_string(),
            count_scope_rows(exec(), "scope_indexer_coverage", source_title_id).await?,
        );
        source_row_counts.insert(
            "indexer_search_runs".to_string(),
            count_scope_rows(exec(), "indexer_search_runs", source_title_id).await?,
        );
        source_row_counts.retain(|_, count| *count > 0);

        Ok(MergeCatalogSnapshot {
            source_title_id: source_title_id.to_string(),
            destination_title_id: destination_title_id.to_string(),
            source_library_id,
            destination_library_id,
            source_episodes,
            destination_episodes,
            source_collections,
            destination_collections,
            source_links,
            destination_links,
            source_file_episode_rows,
            destination_file_episode_rows,
            source_tags,
            destination_tags,
            episode_references,
            source_row_counts,
            media_request_ids: load_media_request_ids(exec(), source_title_id).await?,
            resumable_operations_holding_source: load_resumable_operations(
                exec(),
                source_title_id,
                current_operation_id,
            )
            .await?,
            unconsumed_manual_import_selections: load_unconsumed_manual_import_selections(
                exec(),
                source_title_id,
            )
            .await?,
        })
    }

    async fn execute_title_merge(&self, plan: &MergePlan) -> AppResult<MergeOutcome> {
        if plan.is_blocked() {
            return Err(AppError::Validation(
                plan.summary
                    .blocked_reason()
                    .unwrap_or_else(|| "the merge is blocked".to_string()),
            ));
        }
        let map = plan.require_identity_map()?.clone();
        let plan = plan.clone();

        SqlRuntime::run_in_transaction(&self.datastore, "execute_title_merge", move |tx| {
            let plan = plan.clone();
            let map = map.clone();
            Box::pin(async move {
                let mut outcome = MergeOutcome {
                    source_title_id: map.source_title_id.clone(),
                    destination_title_id: map.destination_title_id.clone(),
                    ..MergeOutcome::default()
                };
                group_1_media_ownership(tx, &plan, &map, &mut outcome).await?;
                group_2_episode_scoped(tx, &map, &mut outcome).await?;
                group_3_title_scoped(tx, &plan, &map, &mut outcome).await?;
                group_4_destination_wins_deletions(tx, &map, &mut outcome).await?;
                group_5_source_removal(tx, &map, &mut outcome).await?;
                outcome.post_merge_work = vec![
                    PostMergeWork::ReindexTitleSearchTerms,
                    PostMergeWork::RegenerateRecommendations,
                    PostMergeWork::RecomputeStatistics,
                    PostMergeWork::DropSourceIndexerCoverage,
                ];
                Ok(outcome)
            })
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Group 0 — reads
// ---------------------------------------------------------------------------

/// Table → the query that lists the source episode ids its rows reference.
/// Every entry must name a table in `FR066_BLOCKING_TABLES`; the `debug_assert`
/// in `load_merge_snapshot` holds that invariant.
const EPISODE_REFERENCE_QUERIES: &[(&str, &str)] = &[
    (
        "episode_external_ids",
        "SELECT DISTINCT episode_id AS episode_id FROM episode_external_ids
         WHERE title_id = {} AND episode_id IS NOT NULL",
    ),
    (
        "wanted_items",
        "SELECT DISTINCT episode_id AS episode_id FROM wanted_items
         WHERE episode_id IS NOT NULL
           AND (title_id = {} OR episode_id IN (SELECT id FROM episodes WHERE title_id = {}))",
    ),
    (
        "download_submissions",
        "SELECT DISTINCT episode_id AS episode_id FROM download_submissions
         WHERE episode_id IS NOT NULL
           AND (title_id = {} OR episode_id IN (SELECT id FROM episodes WHERE title_id = {}))",
    ),
    (
        "download_submission_episode_links",
        "SELECT DISTINCT episode_id AS episode_id FROM download_submission_episode_links
         WHERE episode_id IN (SELECT id FROM episodes WHERE title_id = {})",
    ),
    (
        "download_import_artifacts",
        "SELECT DISTINCT episode_id AS episode_id FROM download_import_artifacts
         WHERE episode_id IS NOT NULL
           AND (title_id = {} OR episode_id IN (SELECT id FROM episodes WHERE title_id = {}))",
    ),
    (
        "subtitle_downloads",
        "SELECT DISTINCT episode_id AS episode_id FROM subtitle_downloads
         WHERE episode_id IS NOT NULL
           AND (title_id = {} OR episode_id IN (SELECT id FROM episodes WHERE title_id = {}))",
    ),
    (
        "workflow_operations",
        "SELECT DISTINCT episode_id AS episode_id FROM workflow_operations
         WHERE episode_id IS NOT NULL
           AND (title_id = {} OR episode_id IN (SELECT id FROM episodes WHERE title_id = {}))",
    ),
    (
        "media_server_playback_items",
        "SELECT DISTINCT entity_id AS episode_id FROM media_server_playback_items
         WHERE entity_kind = 'episode'
           AND entity_id IN (SELECT id FROM episodes WHERE title_id = {})",
    ),
];

async fn load_title_shape(
    exec: SqlExec<'_, '_>,
    title_id: &str,
) -> AppResult<(Option<String>, Vec<String>)> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT library_id, tags FROM titles WHERE id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?
    .ok_or_else(|| AppError::NotFound(format!("title {title_id} not found")))?;
    Ok((row.opt_text("library_id")?, parse_tags(&row.text("tags")?)))
}

fn parse_tags(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

async fn load_episodes(
    exec: SqlExec<'_, '_>,
    title_id: &str,
) -> AppResult<Vec<EpisodeIdentityFacts>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT id, episode_type, season_number, episode_number, absolute_number, collection_id
           FROM episodes WHERE title_id = {} ORDER BY id",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok(EpisodeIdentityFacts {
                id: row.text("id")?,
                episode_type: EpisodeType::parse(row.text("episode_type")?.as_str())
                    .unwrap_or_default(),
                season_number: row.opt_text("season_number")?,
                episode_number: row.opt_text("episode_number")?,
                absolute_number: row.opt_text("absolute_number")?,
                collection_id: row.opt_text("collection_id")?,
            })
        })
        .collect()
}

async fn load_collections(
    exec: SqlExec<'_, '_>,
    title_id: &str,
) -> AppResult<Vec<CollectionIdentityFacts>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT id, collection_type, collection_index FROM collections
          WHERE title_id = {} ORDER BY id",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok(CollectionIdentityFacts {
                id: row.text("id")?,
                collection_type: CollectionType::parse(row.text("collection_type")?.as_str())
                    .unwrap_or_default(),
                collection_index: row.text("collection_index")?,
            })
        })
        .collect()
}

async fn load_links(
    exec: SqlExec<'_, '_>,
    title_id: &str,
) -> AppResult<Vec<SeriesMovieLinkIdentityFacts>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT id, movie_entity_id, legacy_collection_id FROM series_movie_links
          WHERE series_title_id = {} ORDER BY id",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok(SeriesMovieLinkIdentityFacts {
                id: row.text("id")?,
                movie_entity_id: row.text("movie_entity_id")?,
                legacy_collection_id: row.opt_text("legacy_collection_id")?,
            })
        })
        .collect()
}

async fn load_file_episode_rows(
    exec: SqlExec<'_, '_>,
    title_id: &str,
) -> AppResult<Vec<FileEpisodeRoleRow>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT map.file_id, map.episode_id, map.role, map.is_filler
           FROM file_episode_map AS map
           INNER JOIN media_files AS file ON file.id = map.file_id
          WHERE file.title_id = {}
          ORDER BY map.file_id, map.episode_id",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok(FileEpisodeRoleRow {
                file_id: row.text("file_id")?,
                episode_id: row.text("episode_id")?,
                role: if row.text("role")? == MergedMediaRole::Primary.as_str() {
                    MergedMediaRole::Primary
                } else {
                    MergedMediaRole::Additional
                },
                is_filler: row.opt_bool("is_filler")?.unwrap_or(false),
            })
        })
        .collect()
}

async fn load_episode_reference_ids(
    exec: SqlExec<'_, '_>,
    sql: &str,
    source_title_id: &str,
) -> AppResult<BTreeSet<String>> {
    let placeholders = sql.matches("{}").count();
    let args = vec![SqlArg::Text(source_title_id.to_string()); placeholders];
    let rows = SqlRuntime::fetch_all(exec, sql, &args).await?;
    rows.iter().map(|row| row.text("episode_id")).collect()
}

/// OQ8's read half: the episode ids that live inside compressed payloads. Only
/// the nine event types that carry `$.data.episode_ids[]` are decoded, so the
/// cost is bounded to a minority of a long-lived title's events.
async fn load_domain_event_episode_ids(
    exec: SqlExec<'_, '_>,
    source_title_id: &str,
) -> AppResult<BTreeSet<String>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        &domain_event_payload_select(),
        &[
            SqlArg::Text(source_title_id.to_string()),
            SqlArg::Text(source_title_id.to_string()),
        ],
    )
    .await?;
    let mut ids = BTreeSet::new();
    for row in &rows {
        let Some(payload) = decode_payload(row)? else {
            continue;
        };
        ids.extend(payload_episode_ids(&payload));
    }
    Ok(ids)
}

fn domain_event_payload_select() -> String {
    let types = OQ8_EPISODE_BEARING_EVENT_TYPES
        .iter()
        .map(|value| format!("'{value}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT sequence, event_id, event_type, payload_json FROM domain_events
          WHERE event_type IN ({types})
            AND (title_id = {{}} OR (stream_kind = 'title' AND stream_id = {{}}))
          ORDER BY sequence"
    )
}

fn decode_payload(row: &SqlRow) -> AppResult<Option<Value>> {
    let Some(encoded) = row.opt_bytes("payload_json")? else {
        return Ok(None);
    };
    decode_domain_event_payload(&encoded)
        .map(Some)
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to decode domain event {}: {error}",
                row.text("event_id").unwrap_or_default()
            ))
        })
}

fn payload_episode_ids(payload: &Value) -> Vec<String> {
    payload
        .get("data")
        .and_then(|data| data.get("episode_ids"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

async fn count_rows(
    exec: SqlExec<'_, '_>,
    table: &str,
    column: &str,
    title_id: &str,
) -> AppResult<i64> {
    let row = SqlRuntime::fetch_optional(
        exec,
        &format!("SELECT COUNT(*) AS row_count FROM {table} WHERE {column} = {{}}"),
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?
    .ok_or_else(|| AppError::Repository(format!("missing count for {table}")))?;
    row.i64("row_count")
}

/// `scope_key` is a composed string (`convergence_scope_key`), so the count is
/// over the four reversible forms. The fifth, `episode_set:b3:<hex>`, is a
/// BLAKE3 hash that cannot be attributed back to a title — which is exactly why
/// OQ4 drops the whole scope set rather than rewriting it.
async fn count_scope_rows(
    exec: SqlExec<'_, '_>,
    table: &str,
    source_title_id: &str,
) -> AppResult<i64> {
    let row = SqlRuntime::fetch_optional(
        exec,
        &format!(
            "SELECT COUNT(*) AS row_count FROM {table}
              WHERE scope_key = {{}}
                 OR scope_key IN (SELECT 'episode:' || id FROM episodes WHERE title_id = {{}})
                 OR scope_key IN (SELECT 'collection:' || id FROM collections WHERE title_id = {{}})
                 OR scope_key IN (
                        SELECT 'series_movie:' || id FROM series_movie_links
                         WHERE series_title_id = {{}})"
        ),
        &[
            SqlArg::Text(format!("title:{source_title_id}")),
            SqlArg::Text(source_title_id.to_string()),
            SqlArg::Text(source_title_id.to_string()),
            SqlArg::Text(source_title_id.to_string()),
        ],
    )
    .await?
    .ok_or_else(|| AppError::Repository(format!("missing scope count for {table}")))?;
    row.i64("row_count")
}

async fn load_media_request_ids(
    exec: SqlExec<'_, '_>,
    source_title_id: &str,
) -> AppResult<Vec<String>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT id FROM media_requests WHERE created_title_id = {} ORDER BY id",
        &[SqlArg::Text(source_title_id.to_string())],
    )
    .await?;
    rows.iter().map(|row| row.text("id")).collect()
}

/// OQ7: any *other* non-terminal operation that still claims the source title,
/// through the ownership registry or through a checkpoint it has not finished.
async fn load_resumable_operations(
    exec: SqlExec<'_, '_>,
    source_title_id: &str,
    current_operation_id: Option<&str>,
) -> AppResult<Vec<String>> {
    let excluded = current_operation_id.unwrap_or("").to_string();
    let rows = SqlRuntime::fetch_all(
        exec,
        &format!(
            "SELECT DISTINCT operation.id AS operation_id
               FROM location_operations AS operation
              WHERE operation.state NOT IN ({TERMINAL_OPERATION_STATES})
                AND operation.id <> {{}}
                AND (
                    EXISTS (
                        SELECT 1 FROM location_operation_owned_entities AS owned
                         WHERE owned.operation_id = operation.id
                           AND owned.entity_type = 'title'
                           AND owned.entity_id = {{}}
                           AND owned.released_at IS NULL)
                    OR EXISTS (
                        SELECT 1 FROM location_operation_title_checkpoints AS checkpoint
                         WHERE checkpoint.operation_id = operation.id
                           AND checkpoint.title_id = {{}}
                           AND checkpoint.state IN ({LIVE_CHECKPOINT_STATES}))
                )
              ORDER BY operation.id"
        ),
        &[
            SqlArg::Text(excluded),
            SqlArg::Text(source_title_id.to_string()),
            SqlArg::Text(source_title_id.to_string()),
        ],
    )
    .await?;
    rows.iter().map(|row| row.text("operation_id")).collect()
}

async fn load_unconsumed_manual_import_selections(
    exec: SqlExec<'_, '_>,
    source_title_id: &str,
) -> AppResult<Vec<String>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT id FROM manual_import_selections
          WHERE title_id = {} AND consumed_at IS NULL ORDER BY id",
        &[SqlArg::Text(source_title_id.to_string())],
    )
    .await?;
    rows.iter().map(|row| row.text("id")).collect()
}

// ---------------------------------------------------------------------------
// Groups 1–5 — the transaction
// ---------------------------------------------------------------------------

fn record(outcome: &mut MergeOutcome, key: &str, rows: u64) {
    if rows > 0 {
        *outcome.rows_affected.entry(key.to_string()).or_default() += rows;
    }
}

/// Group 1. Repointing `media_files` is what saves the source's files — and
/// every file-keyed child — from the Group 5 cascade; `file_episode_map` has to
/// be remapped in the same group because it CASCADEs from `episodes` too.
async fn group_1_media_ownership(
    tx: &mut SqlTx<'_>,
    plan: &MergePlan,
    map: &MergeIdentityMap,
    outcome: &mut MergeOutcome,
) -> AppResult<()> {
    let moved = tx
        .execute(
            "UPDATE media_files SET title_id = {} WHERE title_id = {}",
            &[
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(map.source_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "1:media_files", moved);

    // Delete exactly the source rows the role plan replaces, then write the
    // resolved rows. The insert is deliberately *not* `ON CONFLICT DO NOTHING`:
    // `idx_file_episode_map_one_primary_per_episode` is the mechanical
    // enforcement point for FR-068/070, and a resolution bug must fail loudly
    // rather than silently drop a row.
    for row in &plan.source_file_episode_rows {
        let removed = tx
            .execute(
                "DELETE FROM file_episode_map WHERE file_id = {} AND episode_id = {}",
                &[
                    SqlArg::Text(row.file_id.clone()),
                    SqlArg::Text(row.episode_id.clone()),
                ],
            )
            .await?;
        record(outcome, "1:file_episode_map_removed", removed);
    }
    for row in &plan.role_plan.rows {
        let inserted = tx
            .execute(
                "INSERT INTO file_episode_map (file_id, episode_id, role, is_filler)
                 VALUES ({}, {}, {}, {})",
                &[
                    SqlArg::Text(row.file_id.clone()),
                    SqlArg::Text(row.episode_id.clone()),
                    SqlArg::Text(row.role.as_str().to_string()),
                    SqlArg::Bool(row.is_filler),
                ],
            )
            .await?;
        record(outcome, "1:file_episode_map", inserted);
    }

    // `series_movie_links` rows themselves die with the source title; what has
    // to survive is the file→link mapping. The pre-delete is the `ON CONFLICT
    // DO NOTHING` for the `(file_id, series_movie_link_id)` primary key.
    for (source_link, destination_link) in &map.series_movie_links {
        let collapsed = tx
            .execute(
                "DELETE FROM file_series_movie_link_map
                  WHERE series_movie_link_id = {}
                    AND file_id IN (SELECT file_id FROM file_series_movie_link_map
                                     WHERE series_movie_link_id = {})",
                &[
                    SqlArg::Text(source_link.clone()),
                    SqlArg::Text(destination_link.clone()),
                ],
            )
            .await?;
        record(outcome, "1:file_series_movie_link_map_collapsed", collapsed);
        let repointed = tx
            .execute(
                "UPDATE file_series_movie_link_map SET series_movie_link_id = {}
                  WHERE series_movie_link_id = {}",
                &[
                    SqlArg::Text(destination_link.clone()),
                    SqlArg::Text(source_link.clone()),
                ],
            )
            .await?;
        record(outcome, "1:file_series_movie_link_map", repointed);
    }
    // `UNIQUE(legacy_collection_id)`: the source link is about to be deleted, so
    // this is defensive — it keeps the invariant true for the whole window in
    // which both links exist.
    for source_link in &map.legacy_collection_ids_to_clear {
        tx.execute(
            "UPDATE series_movie_links SET legacy_collection_id = NULL WHERE id = {}",
            &[SqlArg::Text(source_link.clone())],
        )
        .await?;
    }

    Ok(())
}

/// Group 2. Every table here keys on an episode id Group 5 destroys, so all of
/// it happens while both sets of episodes still resolve.
async fn group_2_episode_scoped(
    tx: &mut SqlTx<'_>,
    map: &MergeIdentityMap,
    outcome: &mut MergeOutcome,
) -> AppResult<()> {
    // OQ5: the delay queue and its decisions were chosen against the source
    // title's quality profile, which FR-063 has just replaced. They re-derive
    // from `wanted_items` against the destination profile on the next
    // convergence pass. Deleted first so the `wanted_items` work below sees a
    // clean table.
    for table in ["pending_releases", "release_decisions"] {
        let removed = tx
            .execute(
                &format!("DELETE FROM {table} WHERE title_id = {{}}"),
                &[SqlArg::Text(map.source_title_id.clone())],
            )
            .await?;
        record(outcome, &format!("2:{table}_dropped"), removed);
    }

    merge_wanted_items(tx, map, outcome).await?;

    // `download_submissions` is client-keyed on
    // `UNIQUE(download_client_type, download_client_item_id)`, so a union
    // cannot collide.
    let repointed = tx
        .execute(
            "UPDATE download_submissions SET title_id = {} WHERE title_id = {}",
            &[
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(map.source_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "2:download_submissions", repointed);

    for (source_episode, destination_episode) in &map.episodes {
        for (table, column) in [
            ("download_submissions", "episode_id"),
            ("download_import_artifacts", "episode_id"),
            ("subtitle_downloads", "episode_id"),
            ("workflow_operations", "episode_id"),
        ] {
            let rows = tx
                .execute(
                    &format!("UPDATE {table} SET {column} = {{}} WHERE {column} = {{}}"),
                    &[
                        SqlArg::Text(destination_episode.clone()),
                        SqlArg::Text(source_episode.clone()),
                    ],
                )
                .await?;
            record(outcome, &format!("2:{table}"), rows);
        }

        // The `(download_id, episode_id)` primary key collapses when two source
        // episodes remap onto one destination episode; the pre-delete is that
        // collapse, done explicitly rather than through `ON CONFLICT`.
        let collapsed = tx
            .execute(
                "DELETE FROM download_submission_episode_links
                  WHERE episode_id = {}
                    AND download_id IN (SELECT download_id FROM download_submission_episode_links
                                         WHERE episode_id = {})",
                &[
                    SqlArg::Text(source_episode.clone()),
                    SqlArg::Text(destination_episode.clone()),
                ],
            )
            .await?;
        record(
            outcome,
            "2:download_submission_episode_links_collapsed",
            collapsed,
        );
        let rows = tx
            .execute(
                "UPDATE download_submission_episode_links SET episode_id = {} WHERE episode_id = {}",
                &[
                    SqlArg::Text(destination_episode.clone()),
                    SqlArg::Text(source_episode.clone()),
                ],
            )
            .await?;
        record(outcome, "2:download_submission_episode_links", rows);

        // `media_server_playback_items` collides on
        // `(connection_id, entity_kind, entity_id)`; the destination's
        // `provider_item_id` is the live link, so it wins.
        let collapsed = tx
            .execute(
                "DELETE FROM media_server_playback_items
                  WHERE entity_kind = 'episode' AND entity_id = {}
                    AND connection_id IN (SELECT connection_id FROM media_server_playback_items
                                           WHERE entity_kind = 'episode' AND entity_id = {})",
                &[
                    SqlArg::Text(source_episode.clone()),
                    SqlArg::Text(destination_episode.clone()),
                ],
            )
            .await?;
        record(outcome, "2:media_server_playback_items_collapsed", collapsed);
        let rows = tx
            .execute(
                "UPDATE media_server_playback_items SET entity_id = {}
                  WHERE entity_kind = 'episode' AND entity_id = {}",
                &[
                    SqlArg::Text(destination_episode.clone()),
                    SqlArg::Text(source_episode.clone()),
                ],
            )
            .await?;
        record(outcome, "2:media_server_playback_items", rows);
    }

    for (source_collection, destination_collection) in &map.collections {
        for table in ["download_submissions", "workflow_operations"] {
            let rows = tx
                .execute(
                    &format!(
                        "UPDATE {table} SET collection_id = {{}} WHERE collection_id = {{}}"
                    ),
                    &[
                        SqlArg::Text(destination_collection.clone()),
                        SqlArg::Text(source_collection.clone()),
                    ],
                )
                .await?;
            record(outcome, &format!("2:{table}_collection"), rows);
        }
    }
    for (source_link, destination_link) in &map.series_movie_links {
        for table in ["download_submissions", "workflow_operations"] {
            let rows = tx
                .execute(
                    &format!(
                        "UPDATE {table} SET series_movie_link_id = {{}}
                          WHERE series_movie_link_id = {{}}"
                    ),
                    &[
                        SqlArg::Text(destination_link.clone()),
                        SqlArg::Text(source_link.clone()),
                    ],
                )
                .await?;
            record(outcome, &format!("2:{table}_link"), rows);
        }
    }

    // The remaining title-keyed halves of the episode-scoped tables.
    for (table, column) in [
        ("download_import_artifacts", "title_id"),
        ("subtitle_downloads", "title_id"),
        ("workflow_operations", "title_id"),
    ] {
        let rows = tx
            .execute(
                &format!("UPDATE {table} SET {column} = {{}} WHERE {column} = {{}}"),
                &[
                    SqlArg::Text(map.destination_title_id.clone()),
                    SqlArg::Text(map.source_title_id.clone()),
                ],
            )
            .await?;
        record(outcome, &format!("2:{table}_title"), rows);
    }

    // `media_server_playback_items` also carries a title-scoped row.
    let collapsed = tx
        .execute(
            "DELETE FROM media_server_playback_items
              WHERE entity_kind = 'title' AND entity_id = {}
                AND connection_id IN (SELECT connection_id FROM media_server_playback_items
                                       WHERE entity_kind = 'title' AND entity_id = {})",
            &[
                SqlArg::Text(map.source_title_id.clone()),
                SqlArg::Text(map.destination_title_id.clone()),
            ],
        )
        .await?;
    record(
        outcome,
        "2:media_server_playback_items_title_collapsed",
        collapsed,
    );
    let rows = tx
        .execute(
            "UPDATE media_server_playback_items SET entity_id = {}
              WHERE entity_kind = 'title' AND entity_id = {}",
            &[
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(map.source_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "2:media_server_playback_items_title", rows);

    merge_domain_events(tx, map, outcome).await
}

/// `wanted_items` collides three ways — `UNIQUE(title_id, episode_id)`,
/// `idx_wanted_items_collection_id`, `idx_wanted_items_series_movie_link` —
/// plus `idx_wanted_items_movie_unique` for the fileless movie row. All four
/// resolve destination-wins (OQ5): the destination row is the live acquisition
/// cursor, and adopting the source's would reset or double-schedule searches.
async fn merge_wanted_items(
    tx: &mut SqlTx<'_>,
    map: &MergeIdentityMap,
    outcome: &mut MergeOutcome,
) -> AppResult<()> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        "SELECT id, episode_id, collection_id, series_movie_link_id FROM wanted_items
          WHERE title_id = {} ORDER BY id",
        &[SqlArg::Text(map.source_title_id.clone())],
    )
    .await?;

    let mut carried = Vec::new();
    for row in &rows {
        carried.push((
            row.text("id")?,
            remap(&map.episodes, row.opt_text("episode_id")?),
            remap(&map.collections, row.opt_text("collection_id")?),
            remap(
                &map.series_movie_links,
                row.opt_text("series_movie_link_id")?,
            ),
        ));
    }

    for (id, episode_id, collection_id, link_id) in carried {
        if wanted_item_collides(
            tx,
            &map.destination_title_id,
            episode_id.as_deref(),
            collection_id.as_deref(),
            link_id.as_deref(),
        )
        .await?
        {
            let removed = tx
                .execute(
                    "DELETE FROM wanted_items WHERE id = {}",
                    &[SqlArg::Text(id)],
                )
                .await?;
            record(outcome, "2:wanted_items_destination_wins", removed);
            continue;
        }
        let updated = tx
            .execute(
                "UPDATE wanted_items
                    SET title_id = {}, episode_id = {}, collection_id = {}, series_movie_link_id = {}
                  WHERE id = {}",
                &[
                    SqlArg::Text(map.destination_title_id.clone()),
                    SqlArg::OptText(episode_id),
                    SqlArg::OptText(collection_id),
                    SqlArg::OptText(link_id),
                    SqlArg::Text(id),
                ],
            )
            .await?;
        record(outcome, "2:wanted_items", updated);
    }
    Ok(())
}

/// A `None` stays `None`; an id the map does not carry stays as it is (it
/// already points at something that survives).
fn remap(mapping: &BTreeMap<String, String>, original: Option<String>) -> Option<String> {
    let original = original?;
    Some(mapping.get(&original).cloned().unwrap_or(original))
}

async fn wanted_item_collides(
    tx: &mut SqlTx<'_>,
    destination_title_id: &str,
    episode_id: Option<&str>,
    collection_id: Option<&str>,
    link_id: Option<&str>,
) -> AppResult<bool> {
    // `UNIQUE(title_id, episode_id)`. Kept as two statements rather than one
    // with a nullable bind: postgres cannot infer the type of a bare parameter
    // in `{} IS NULL`.
    let hit = match episode_id {
        Some(episode_id) => {
            SqlRuntime::fetch_optional(
                SqlExec::Tx(tx),
                "SELECT 1 AS hit FROM wanted_items
                  WHERE title_id = {} AND episode_id = {} LIMIT 1",
                &[
                    SqlArg::Text(destination_title_id.to_string()),
                    SqlArg::Text(episode_id.to_string()),
                ],
            )
            .await?
        }
        None => {
            SqlRuntime::fetch_optional(
                SqlExec::Tx(tx),
                "SELECT 1 AS hit FROM wanted_items
                  WHERE title_id = {} AND episode_id IS NULL LIMIT 1",
                &[SqlArg::Text(destination_title_id.to_string())],
            )
            .await?
        }
    };
    if hit.is_some() {
        return Ok(true);
    }
    if let Some(collection_id) = collection_id
        && SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            "SELECT 1 AS hit FROM wanted_items WHERE collection_id = {} LIMIT 1",
            &[SqlArg::Text(collection_id.to_string())],
        )
        .await?
        .is_some()
    {
        return Ok(true);
    }
    if let Some(link_id) = link_id
        && SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            "SELECT 1 AS hit FROM wanted_items WHERE series_movie_link_id = {} LIMIT 1",
            &[SqlArg::Text(link_id.to_string())],
        )
        .await?
        .is_some()
    {
        return Ok(true);
    }
    if episode_id.is_none()
        && collection_id.is_none()
        && link_id.is_none()
        && SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            "SELECT 1 AS hit FROM wanted_items
              WHERE title_id = {} AND episode_id IS NULL AND collection_id IS NULL
                AND series_movie_link_id IS NULL LIMIT 1",
            &[SqlArg::Text(destination_title_id.to_string())],
        )
        .await?
        .is_some()
    {
        return Ok(true);
    }
    Ok(false)
}

/// OQ8's middle path. `title_id` and title `stream_id` are rewritten by SQL for
/// **every** event, so the merged title's Activity feed stays whole. Payloads
/// are decompressed, remapped, and recompressed only for the event types that
/// carry `$.data.episode_ids[]`, which is the minority of rows — the price of
/// the literal FR-066 reading without paying it on every event of a long-lived
/// series. `TitleContextSnapshot`, embedded in nearly every payload, holds no
/// title id and is never touched.
async fn merge_domain_events(
    tx: &mut SqlTx<'_>,
    map: &MergeIdentityMap,
    outcome: &mut MergeOutcome,
) -> AppResult<()> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        &domain_event_payload_select(),
        &[
            SqlArg::Text(map.source_title_id.clone()),
            SqlArg::Text(map.source_title_id.clone()),
        ],
    )
    .await?;

    let mut rewrites = Vec::new();
    for row in &rows {
        let Some(mut payload) = decode_payload(row)? else {
            continue;
        };
        if !remap_payload_ids(&mut payload, map) {
            continue;
        }
        let encoded = encode_domain_event_payload(&payload).map_err(|error| {
            AppError::Repository(format!(
                "failed to re-encode domain event {}: {error}",
                row.text("event_id").unwrap_or_default()
            ))
        })?;
        rewrites.push((row.i64("sequence")?, encoded));
    }
    for (sequence, encoded) in rewrites {
        tx.execute(
            "UPDATE domain_events SET payload_json = {} WHERE sequence = {}",
            &[SqlArg::OptBytes(Some(encoded)), SqlArg::I64(sequence)],
        )
        .await?;
        outcome.domain_event_payloads_rewritten += 1;
    }

    let rows = tx
        .execute(
            "UPDATE domain_events SET title_id = {} WHERE title_id = {}",
            &[
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(map.source_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "2:domain_events_title", rows);
    let rows = tx
        .execute(
            "UPDATE domain_events SET stream_id = {}
              WHERE stream_kind = 'title' AND stream_id = {}",
            &[
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(map.source_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "2:domain_events_stream", rows);
    Ok(())
}

/// Remap the identity-bearing fields inside one decoded payload. Returns
/// whether anything changed, so an untouched event is not re-encoded.
///
/// `episode_ids` is OQ8's mandate. `collection_id` rides along because it is in
/// the same payloads and its ids move in the same map — leaving it stale in a
/// payload already being rewritten would be a bug, not a saving. File ids are
/// deliberately untouched: `media_files.id` is stable across a merge.
fn remap_payload_ids(payload: &mut Value, map: &MergeIdentityMap) -> bool {
    let Some(data) = payload.get_mut("data").and_then(Value::as_object_mut) else {
        return false;
    };
    let mut changed = false;
    if let Some(Value::Array(values)) = data.get_mut("episode_ids") {
        for value in values.iter_mut() {
            let Some(current) = value.as_str() else {
                continue;
            };
            if let Some(destination) = map.episodes.get(current) {
                *value = Value::String(destination.clone());
                changed = true;
            }
        }
    }
    if let Some(value) = data.get_mut("collection_id")
        && let Some(current) = value.as_str()
        && let Some(destination) = map.collections.get(current)
    {
        *value = Value::String(destination.clone());
        changed = true;
    }
    changed
}

/// Group 3. Title-scoped additive unions. Nothing forces this after Group 2
/// structurally; ordering it here means a failure in the expensive,
/// blocking-prone part rolls back the cheap part rather than the reverse.
async fn group_3_title_scoped(
    tx: &mut SqlTx<'_>,
    plan: &MergePlan,
    map: &MergeIdentityMap,
    outcome: &mut MergeOutcome,
) -> AppResult<()> {
    // `blocklist` — both 0194 partial indexes, deduped destination-wins: the
    // destination's older `created_at` and reason text are the operator's
    // record.
    let deduped = tx
        .execute(
            "DELETE FROM blocklist
              WHERE title_id = {}
                AND ((blocklist.info_hash IS NULL
                      AND EXISTS (SELECT 1 FROM blocklist AS kept
                                   WHERE kept.title_id = {}
                                     AND kept.info_hash IS NULL
                                     AND kept.indexer_id = blocklist.indexer_id
                                     AND kept.normalized_release_name
                                         = blocklist.normalized_release_name))
                  OR (blocklist.info_hash IS NOT NULL
                      AND EXISTS (SELECT 1 FROM blocklist AS kept
                                   WHERE kept.title_id = {}
                                     AND kept.info_hash = blocklist.info_hash)))",
            &[
                SqlArg::Text(map.source_title_id.clone()),
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(map.destination_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "3:blocklist_deduped", deduped);

    // Plain title-id repoints. `post_processing_script_runs.title_name` and
    // `env_payload_json` are deliberately NOT rewritten: they are historical
    // snapshots of what the script actually saw (migration 0051).
    for (table, column) in [
        ("blocklist", "title_id"),
        ("release_download_attempts", "title_id"),
        ("post_processing_script_runs", "title_id"),
        ("discovery_titles", "resolved_title_id"),
        ("discovery_submitted_subjects", "title_id"),
        ("discovery_pending_context_changes", "title_id"),
        ("discovery_pending_context_changes", "previous_title_id"),
        ("location_operation_verifications", "title_id"),
        ("manual_import_selections", "title_id"),
        ("location_operation_title_checkpoints", "merged_into_title_id"),
    ] {
        let rows = tx
            .execute(
                &format!("UPDATE {table} SET {column} = {{}} WHERE {column} = {{}}"),
                &[
                    SqlArg::Text(map.destination_title_id.clone()),
                    SqlArg::Text(map.source_title_id.clone()),
                ],
            )
            .await?;
        record(outcome, &format!("3:{table}.{column}"), rows);
    }

    // OQ10: the request history follows the content into the destination
    // library. Named in the FR-071 preview, never silent.
    let rows = match plan.destination_library_id.as_ref() {
        Some(library_id) => {
            tx.execute(
                "UPDATE media_requests SET created_title_id = {}, library_id = {}
                  WHERE created_title_id = {}",
                &[
                    SqlArg::Text(map.destination_title_id.clone()),
                    SqlArg::Text(library_id.clone()),
                    SqlArg::Text(map.source_title_id.clone()),
                ],
            )
            .await?
        }
        None => {
            tx.execute(
                "UPDATE media_requests SET created_title_id = {} WHERE created_title_id = {}",
                &[
                    SqlArg::Text(map.destination_title_id.clone()),
                    SqlArg::Text(map.source_title_id.clone()),
                ],
            )
            .await?
        }
    };
    record(outcome, "3:media_requests", rows);

    // OQ3: union with destination-wins on the
    // `(indexer_id, title_id, facet, strategy_key)` primary key. Counters are
    // never summed — that would be the only disposition in the whole inventory
    // that computes a new value rather than choosing an existing one.
    let carried = tx
        .execute(
            "INSERT INTO indexer_search_learning
                 (indexer_id, title_id, facet, strategy_key, attempts, empty_successes,
                  usable_successes, last_attempt_at, last_usable_at, suppressed, updated_at)
             SELECT indexer_id, {}, facet, strategy_key, attempts, empty_successes,
                    usable_successes, last_attempt_at, last_usable_at, suppressed, updated_at
               FROM indexer_search_learning WHERE title_id = {}
             ON CONFLICT DO NOTHING",
            &[
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(map.source_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "3:indexer_search_learning", carried);
    // No foreign key, so the source rows must go explicitly or the FR-067 gate
    // fires on them.
    let removed = tx
        .execute(
            "DELETE FROM indexer_search_learning WHERE title_id = {}",
            &[SqlArg::Text(map.source_title_id.clone())],
        )
        .await?;
    record(outcome, "3:indexer_search_learning_source_removed", removed);

    // `discovery_item_library_provenance` is `UNIQUE (item_id, subject_key,
    // title_id, library_id)` and its `library_id` also changes on a
    // cross-library merge, so the remapped row can duplicate one that is
    // already there.
    let destination_library = plan
        .destination_library_id
        .clone()
        .unwrap_or_default();
    let deduped = tx
        .execute(
            "DELETE FROM discovery_item_library_provenance
              WHERE title_id = {}
                AND EXISTS (SELECT 1 FROM discovery_item_library_provenance AS kept
                             WHERE kept.item_id = discovery_item_library_provenance.item_id
                               AND kept.subject_key
                                   = discovery_item_library_provenance.subject_key
                               AND kept.title_id = {}
                               AND kept.library_id = {})",
            &[
                SqlArg::Text(map.source_title_id.clone()),
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(destination_library.clone()),
            ],
        )
        .await?;
    record(outcome, "3:discovery_item_library_provenance_deduped", deduped);
    let rows = tx
        .execute(
            "UPDATE discovery_item_library_provenance SET title_id = {}, library_id = {}
              WHERE title_id = {}",
            &[
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(destination_library.clone()),
                SqlArg::Text(map.source_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "3:discovery_item_library_provenance", rows);

    if let Some(library_id) = plan.destination_library_id.as_ref() {
        let rows = tx
            .execute(
                "UPDATE discovery_submitted_subjects SET library_id = {} WHERE title_id = {}",
                &[
                    SqlArg::Text(library_id.clone()),
                    SqlArg::Text(map.destination_title_id.clone()),
                ],
            )
            .await?;
        record(outcome, "3:discovery_submitted_subjects_library", rows);
    }

    // Cache-class: a stale proxy token degrades to the fallback image class
    // rather than erroring, but it belongs on the list.
    let rows = tx
        .execute(
            "UPDATE image_proxy_sources SET owner_id = {}
              WHERE owner_type = 'title' AND owner_id = {}",
            &[
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(map.source_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "3:image_proxy_sources", rows);
    for (source_episode, destination_episode) in &map.episodes {
        tx.execute(
            "UPDATE image_proxy_sources SET owner_id = {}
              WHERE owner_type = 'episode' AND owner_id = {}",
            &[
                SqlArg::Text(destination_episode.clone()),
                SqlArg::Text(source_episode.clone()),
            ],
        )
        .await?;
    }

    // A live ownership claim must follow the surviving title, or the guard
    // protects a ghost. Two constraints bite, and both are identically defined
    // on sqlite and postgres (migration 0206): the primary key
    // `(operation_id, entity_type, entity_id)`, and
    // `idx_location_operation_owned_entities_active`, unique on
    // `(entity_type, entity_id) WHERE released_at IS NULL` — which is *global*,
    // so an active destination claim in any operation blocks the repoint. The
    // operation running this merge normally holds both titles, so this is the
    // common case, not an edge one.
    let collapsed = tx
        .execute(
            "DELETE FROM location_operation_owned_entities
              WHERE entity_type = 'title' AND entity_id = {}
                AND (operation_id IN (SELECT operation_id
                                        FROM location_operation_owned_entities
                                       WHERE entity_type = 'title' AND entity_id = {})
                     OR (released_at IS NULL
                         AND EXISTS (SELECT 1 FROM location_operation_owned_entities AS active
                                      WHERE active.entity_type = 'title'
                                        AND active.entity_id = {}
                                        AND active.released_at IS NULL)))",
            &[
                SqlArg::Text(map.source_title_id.clone()),
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(map.destination_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "3:location_operation_owned_entities_collapsed", collapsed);
    let rows = tx
        .execute(
            "UPDATE location_operation_owned_entities SET entity_id = {}
              WHERE entity_type = 'title' AND entity_id = {}",
            &[
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(map.source_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "3:location_operation_owned_entities", rows);

    // `imports.payload_json` carries `$.target_title_id` (legacy alias
    // `$.manual_title_id`) inside a stored request payload; a JSON rewrite, not
    // a column update.
    rewrite_import_payloads(tx, map, outcome).await?;

    // OQ9: free-form tags union, reserved `scryer:` tags destination-wins. The
    // merged array is computed in the application layer so the partition rule
    // is testable from literals.
    let tags_json = serde_json::to_string(&plan.tags.merged_tags)
        .map_err(|error| AppError::Repository(format!("failed to encode merged tags: {error}")))?;
    let rows = tx
        .execute(
            "UPDATE titles SET tags = {} WHERE id = {}",
            &[
                SqlArg::Text(tags_json),
                SqlArg::Text(map.destination_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "3:titles_tags", rows);

    // Legacy, and the only surviving production SQL is a housekeeping delete.
    let removed = tx
        .execute(
            "DELETE FROM history_events WHERE title_id = {}",
            &[SqlArg::Text(map.source_title_id.clone())],
        )
        .await?;
    record(outcome, "3:history_events_dropped", removed);

    Ok(())
}

async fn rewrite_import_payloads(
    tx: &mut SqlTx<'_>,
    map: &MergeIdentityMap,
    outcome: &mut MergeOutcome,
) -> AppResult<()> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        "SELECT id, payload_json FROM imports WHERE payload_json LIKE {} ORDER BY id",
        &[SqlArg::Text(format!("%{}%", map.source_title_id))],
    )
    .await?;
    let mut updates = Vec::new();
    for row in &rows {
        let id = row.text("id")?;
        let raw = row.text("payload_json")?;
        let Ok(mut payload) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let mut changed = false;
        for field in ["target_title_id", "manual_title_id"] {
            if let Some(value) = payload.get_mut(field)
                && value.as_str() == Some(map.source_title_id.as_str())
            {
                *value = Value::String(map.destination_title_id.clone());
                changed = true;
            }
        }
        if changed {
            updates.push((id, serde_json::to_string(&payload).map_err(|error| {
                AppError::Repository(format!("failed to encode import payload: {error}"))
            })?));
        }
    }
    for (id, payload) in updates {
        let rows = tx
            .execute(
                "UPDATE imports SET payload_json = {} WHERE id = {}",
                &[SqlArg::Text(payload), SqlArg::Text(id)],
            )
            .await?;
        record(outcome, "3:imports", rows);
    }
    Ok(())
}

/// Group 4. `title_external_ids` carries `idx_title_external_ids_library_lookup`
/// on `(library_id, source, external_id)`, and FR-055 only merges when both
/// sides share `(source, external_id)` — so the collision is a certainty, not a
/// risk. The destination keeps its rows; the source's are deleted, never
/// repointed, and deleted here so they are already gone before anything could
/// write destination external ids.
async fn group_4_destination_wins_deletions(
    tx: &mut SqlTx<'_>,
    map: &MergeIdentityMap,
    outcome: &mut MergeOutcome,
) -> AppResult<()> {
    let removed = tx
        .execute(
            "DELETE FROM title_external_ids WHERE title_id = {}",
            &[SqlArg::Text(map.source_title_id.clone())],
        )
        .await?;
    record(outcome, "4:title_external_ids", removed);
    Ok(())
}

/// Group 5. The FR-067 gate, then the delete.
///
/// The assertions are the whole point: the no-FK tables keep a dangling id that
/// nothing in the database will ever complain about, and the SET NULL tables
/// produce a surviving row with no title, which reads as an orphan in Activity.
/// A failed assertion aborts the transaction naming the table and column.
async fn group_5_source_removal(
    tx: &mut SqlTx<'_>,
    map: &MergeIdentityMap,
    outcome: &mut MergeOutcome,
) -> AppResult<()> {
    for (table, column, kind) in FR067_NO_FK_ASSERTIONS
        .iter()
        .chain(FR067_SET_NULL_ASSERTIONS.iter())
    {
        let remaining = count_gate_rows(tx, table, column, *kind, map).await?;
        if remaining > 0 {
            return Err(AppError::Repository(format!(
                "FR-067 gate: {remaining} row(s) in {table}.{column} still reference source title \
                 {} (or its episodes); the source title was not removed",
                map.source_title_id
            )));
        }
    }

    let removed = tx
        .execute(
            "DELETE FROM titles WHERE id = {}",
            &[SqlArg::Text(map.source_title_id.clone())],
        )
        .await?;
    if removed == 0 {
        return Err(AppError::Repository(format!(
            "FR-067 gate: source title {} was already gone",
            map.source_title_id
        )));
    }
    record(outcome, "5:titles", removed);
    Ok(())
}

async fn count_gate_rows(
    tx: &mut SqlTx<'_>,
    table: &str,
    column: &str,
    kind: MergeGateIdKind,
    map: &MergeIdentityMap,
) -> AppResult<i64> {
    let (sql, args) = match kind {
        MergeGateIdKind::Title => (
            format!("SELECT COUNT(*) AS row_count FROM {table} WHERE {column} = {{}}"),
            vec![SqlArg::Text(map.source_title_id.clone())],
        ),
        MergeGateIdKind::TitleStream => (
            format!(
                "SELECT COUNT(*) AS row_count FROM {table}
                  WHERE stream_kind = 'title' AND {column} = {{}}"
            ),
            vec![SqlArg::Text(map.source_title_id.clone())],
        ),
        MergeGateIdKind::Episode => (
            format!(
                "SELECT COUNT(*) AS row_count FROM {table}
                  WHERE {column} IN (SELECT id FROM episodes WHERE title_id = {{}})"
            ),
            vec![SqlArg::Text(map.source_title_id.clone())],
        ),
        MergeGateIdKind::PlaybackEntity => (
            format!(
                "SELECT COUNT(*) AS row_count FROM {table}
                  WHERE (entity_kind = 'title' AND {column} = {{}})
                     OR (entity_kind = 'episode'
                         AND {column} IN (SELECT id FROM episodes WHERE title_id = {{}}))"
            ),
            vec![
                SqlArg::Text(map.source_title_id.clone()),
                SqlArg::Text(map.source_title_id.clone()),
            ],
        ),
    };
    let row = SqlRuntime::fetch_optional(SqlExec::Tx(tx), &sql, &args)
        .await?
        .ok_or_else(|| AppError::Repository(format!("missing FR-067 gate count for {table}")))?;
    row.i64("row_count")
}

#[cfg(test)]
mod tests;
