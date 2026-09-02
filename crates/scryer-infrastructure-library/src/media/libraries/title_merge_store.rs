//! Persistence for the US7 merge engine: the read phase and the one
//! transaction that repoints, unions, and retires.
//!
//! Dual-engine through [`SqlRuntime`], following the `location_operation_store`
//! precedent. Every statement here is written to render identically on sqlite
//! and postgres — no engine-conditional SQL.
//!
//! # The transaction, and why its order is forced
//!
//! | Step | What | Why it must be here |
//! |---|---|---|
//! | 1 | `media_files` → `file_episode_map` → `file_series_movie_link_map` | `media_files` CASCADEs from `titles`; repointing it is what saves every file-keyed child from step 3's cascade. `file_episode_map` CASCADEs from `media_files` **and** `episodes`, so the episode side must be remapped while both sets of episodes still exist. |
//! | 2 | `history_events` + `domain_events` | `history_events.title_id` is `ON DELETE SET NULL`, so an un-repointed row survives step 3 with no title at all. |
//! | 3 | `DELETE FROM titles` | Everything else recorded against the source title goes with it, by cascade or through the ordinary title-delete path the caller runs afterwards. |
//!
//! The destination title's own rows are never rewritten: it wins everything
//! except the two things above (FR-063).

use async_trait::async_trait;
use scryer_application::location::merge::MergedMediaRole;
use scryer_application::location::merge::engine::{
    EPISODE_BEARING_EVENT_TYPES, MergeCatalogSnapshot, MergeOutcome, MergePlan,
    TitleMergeRepository,
};
use scryer_application::location::merge::map::{
    CollectionIdentityFacts, EpisodeIdentityFacts, MergeIdentityMap, SeriesMovieLinkIdentityFacts,
};
use scryer_application::location::merge::roles::{FileEpisodeRoleRow, TitleSlotFileRow};
use scryer_application::{AppError, AppResult};
use scryer_domain::{CollectionType, EpisodeType};
use serde_json::Value;
use std::collections::BTreeSet;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};
use scryer_infrastructure_sql::domain_event_payload::{
    decode_domain_event_payload, encode_domain_event_payload,
};

/// Operation states a location operation can no longer be resumed from, so an
/// operation in one of them cannot hold the source title any more.
const TERMINAL_OPERATION_STATES: &str = "'completed', 'completed_with_warnings', 'canceled',
    'failed'";

/// Checkpoint states that mean the operation still intends to process the
/// title.
const LIVE_CHECKPOINT_STATES: &str = "'pending', 'moving', 'verifying', 'reconciling',
    'cleaning_up'";

/// `download_submissions.tracked_state` values that still hold a claim on the
/// title — the SQL form of `submission_is_queued`.
const LIVE_TRACKED_STATES: &str = "'downloading', 'import_pending', 'importing', 'import_blocked'";

/// Everything recorded against the source title that retires with it, counted
/// once for the preview's single "source records dropped" figure. Media files
/// and history are not here: they are the two things the merge carries.
const DROPPED_TABLES: &[(&str, &str)] = &[
    ("wanted_items", "title_id"),
    ("download_submissions", "title_id"),
    ("download_import_artifacts", "title_id"),
    ("subtitle_downloads", "title_id"),
    ("workflow_operations", "title_id"),
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
    ("title_search_terms", "title_id"),
    ("title_external_ids", "title_id"),
    ("lifecycle_candidates", "title_id"),
    ("lifecycle_action_runs", "title_id"),
    ("maintenance_rule_exclusions", "title_id"),
    ("media_server_user_media_signals", "scryer_title_id"),
    ("library_scan_unmatched_items", "title_id"),
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

        let source_shape = load_title_shape(exec(), source_title_id).await?;
        let destination_shape = load_title_shape(exec(), destination_title_id).await?;
        let TitleShape {
            library_id: source_library_id,
            ..
        } = source_shape;
        let TitleShape {
            library_id: destination_library_id,
            name: destination_title_name,
        } = destination_shape;

        let (history_episode_ids, history_collection_ids) =
            load_history_payload_ids(exec(), source_title_id).await?;

        let mut dropped_record_count = 0i64;
        for (table, column) in DROPPED_TABLES {
            dropped_record_count += count_rows(exec(), table, column, source_title_id).await?;
        }

        Ok(MergeCatalogSnapshot {
            source_title_id: source_title_id.to_string(),
            destination_title_id: destination_title_id.to_string(),
            destination_title_name,
            source_library_id,
            destination_library_id,
            source_episodes: load_episodes(exec(), source_title_id).await?,
            destination_episodes: load_episodes(exec(), destination_title_id).await?,
            source_collections: load_collections(exec(), source_title_id).await?,
            destination_collections: load_collections(exec(), destination_title_id).await?,
            source_links: load_links(exec(), source_title_id).await?,
            destination_links: load_links(exec(), destination_title_id).await?,
            source_file_episode_rows: load_file_episode_rows(exec(), source_title_id).await?,
            destination_file_episode_rows: load_file_episode_rows(exec(), destination_title_id)
                .await?,
            source_title_slot_files: load_title_slot_files(exec(), source_title_id).await?,
            destination_title_slot_has_primary: load_title_slot_files(
                exec(),
                destination_title_id,
            )
            .await?
            .iter()
            .any(|file| file.role == MergedMediaRole::Primary),
            source_file_link_ids: load_file_link_ids(exec(), source_title_id).await?,
            history_episode_ids,
            history_collection_ids,
            media_file_count: count_rows(exec(), "media_files", "title_id", source_title_id).await?,
            history_row_count: count_history_rows(exec(), source_title_id).await?,
            dropped_record_count,
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
            active_acquisition_work: load_active_acquisition_work(exec(), source_title_id).await?,
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
                repoint_media_files(tx, &plan, &map, &mut outcome).await?;
                carry_history(tx, &map, &mut outcome).await?;
                retire_source_title(tx, &map, &mut outcome).await?;
                Ok(outcome)
            })
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// The read phase
// ---------------------------------------------------------------------------

/// The one `titles` row the read phase needs from each side of a merge.
struct TitleShape {
    library_id: Option<String>,
    /// The catalog's spelling of the title, for the FR-071 summary.
    name: Option<String>,
}

async fn load_title_shape(exec: SqlExec<'_, '_>, title_id: &str) -> AppResult<TitleShape> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT library_id, name FROM titles WHERE id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?
    .ok_or_else(|| AppError::NotFound(format!("title {title_id} not found")))?;
    Ok(TitleShape {
        library_id: row.opt_text("library_id")?,
        name: row
            .opt_text("name")?
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty()),
    })
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

/// Media files that hang off the title itself rather than off an episode: a
/// movie's file, or a title-level extra. `media_files.role` is the whole story
/// for those, because `file_episode_map` says nothing about them.
async fn load_title_slot_files(
    exec: SqlExec<'_, '_>,
    title_id: &str,
) -> AppResult<Vec<TitleSlotFileRow>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT file.id AS file_id, file.role AS role
           FROM media_files AS file
          WHERE file.title_id = {}
            AND NOT EXISTS (SELECT 1 FROM file_episode_map AS map WHERE map.file_id = file.id)
          ORDER BY file.id",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok(TitleSlotFileRow {
                file_id: row.text("file_id")?,
                role: if row.opt_text("role")?.as_deref() == Some(MergedMediaRole::Primary.as_str())
                {
                    MergedMediaRole::Primary
                } else {
                    MergedMediaRole::Additional
                },
            })
        })
        .collect()
}

/// The source series-movie links a source media file is attached to. Only these
/// have to map: a link nothing points at retires with the title.
async fn load_file_link_ids(
    exec: SqlExec<'_, '_>,
    source_title_id: &str,
) -> AppResult<BTreeSet<String>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT DISTINCT map.series_movie_link_id AS link_id
           FROM file_series_movie_link_map AS map
           INNER JOIN media_files AS file ON file.id = map.file_id
          WHERE file.title_id = {}",
        &[SqlArg::Text(source_title_id.to_string())],
    )
    .await?;
    rows.iter().map(|row| row.text("link_id")).collect()
}

/// The episode and collection ids that live inside the compressed history
/// payloads the merge carries. Only the event types that carry
/// `$.data.episode_ids[]` are decoded, so the cost is bounded to a minority of a
/// long-lived title's events.
async fn load_history_payload_ids(
    exec: SqlExec<'_, '_>,
    source_title_id: &str,
) -> AppResult<(BTreeSet<String>, BTreeSet<String>)> {
    let rows = SqlRuntime::fetch_all(
        exec,
        &domain_event_payload_select(),
        &[
            SqlArg::Text(source_title_id.to_string()),
            SqlArg::Text(source_title_id.to_string()),
        ],
    )
    .await?;
    let mut episodes = BTreeSet::new();
    let mut collections = BTreeSet::new();
    for row in &rows {
        let Some(payload) = decode_payload(row)? else {
            continue;
        };
        let Some(data) = payload.get("data") else {
            continue;
        };
        if let Some(values) = data.get("episode_ids").and_then(Value::as_array) {
            episodes.extend(values.iter().filter_map(Value::as_str).map(str::to_string));
        }
        if let Some(collection_id) = data.get("collection_id").and_then(Value::as_str) {
            collections.insert(collection_id.to_string());
        }
    }
    Ok((episodes, collections))
}

fn domain_event_payload_select() -> String {
    let types = EPISODE_BEARING_EVENT_TYPES
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

async fn count_history_rows(exec: SqlExec<'_, '_>, source_title_id: &str) -> AppResult<i64> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT (SELECT COUNT(*) FROM history_events WHERE title_id = {})
              + (SELECT COUNT(*) FROM domain_events
                  WHERE title_id = {} OR (stream_kind = 'title' AND stream_id = {}))
                AS row_count",
        &[
            SqlArg::Text(source_title_id.to_string()),
            SqlArg::Text(source_title_id.to_string()),
            SqlArg::Text(source_title_id.to_string()),
        ],
    )
    .await?
    .ok_or_else(|| AppError::Repository("missing history count".to_string()))?;
    row.i64("row_count")
}

/// Any *other* non-terminal operation that still claims the source title,
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

/// FR-086 for the merge: a download the source title still holds. The durable
/// tracked-state ledger is what is consulted, plus a submission Scryer has
/// accepted but not yet bound to a client item — just as much an in-flight claim
/// as one that is downloading.
async fn load_active_acquisition_work(
    exec: SqlExec<'_, '_>,
    source_title_id: &str,
) -> AppResult<Vec<String>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        &format!(
            "SELECT id FROM download_submissions
              WHERE title_id = {{}}
                AND (tracked_state IN ({LIVE_TRACKED_STATES})
                     OR (COALESCE(tracked_state, '') = ''
                         AND download_client_item_id IS NULL
                         AND EXISTS (SELECT 1 FROM download_client_bindings AS binding
                                      WHERE binding.download_id = download_submissions.id
                                        AND binding.native_item_id IS NULL
                                        AND binding.ended_at IS NULL)))
              ORDER BY id"
        ),
        &[SqlArg::Text(source_title_id.to_string())],
    )
    .await?;
    rows.iter().map(|row| row.text("id")).collect()
}

// ---------------------------------------------------------------------------
// The transaction
// ---------------------------------------------------------------------------

fn record(outcome: &mut MergeOutcome, key: &str, rows: u64) {
    if rows > 0 {
        *outcome.rows_affected.entry(key.to_string()).or_default() += rows;
    }
}

/// Step 1. Repointing `media_files` is what saves the source's files — and every
/// file-keyed child — from step 3's cascade; `file_episode_map` has to be
/// remapped in the same step because it CASCADEs from `episodes` too.
async fn repoint_media_files(
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
    record(outcome, "files:media_files", moved);

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
        record(outcome, "files:file_episode_map_removed", removed);
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
        record(outcome, "files:file_episode_map", inserted);
    }

    // FR-068 for the title's own slot: a movie file arriving beside a
    // destination primary becomes an additional.
    for row in &plan.role_plan.title_slot_rows {
        let updated = tx
            .execute(
                "UPDATE media_files SET role = {} WHERE id = {}",
                &[
                    SqlArg::Text(row.role.as_str().to_string()),
                    SqlArg::Text(row.file_id.clone()),
                ],
            )
            .await?;
        record(outcome, "files:media_files_role", updated);
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
        record(
            outcome,
            "files:file_series_movie_link_map_collapsed",
            collapsed,
        );
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
        record(outcome, "files:file_series_movie_link_map", repointed);
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

/// Step 2. History follows the content: `history_events.title_id` and
/// `domain_events`' `title_id` / title `stream_id` are rewritten for every row,
/// so the merged title's Activity feed stays whole. Payloads are decompressed,
/// remapped, and recompressed only for the event types that carry
/// `$.data.episode_ids[]`, which is the minority of rows.
/// `TitleContextSnapshot`, embedded in nearly every payload, holds no title id
/// and is never touched.
async fn carry_history(
    tx: &mut SqlTx<'_>,
    map: &MergeIdentityMap,
    outcome: &mut MergeOutcome,
) -> AppResult<()> {
    let rows = tx
        .execute(
            "UPDATE history_events SET title_id = {} WHERE title_id = {}",
            &[
                SqlArg::Text(map.destination_title_id.clone()),
                SqlArg::Text(map.source_title_id.clone()),
            ],
        )
        .await?;
    record(outcome, "history:history_events", rows);

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
    record(outcome, "history:domain_events_title", rows);
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
    record(outcome, "history:domain_events_stream", rows);
    Ok(())
}

/// Remap the identity-bearing fields inside one decoded payload. Returns
/// whether anything changed, so an untouched event is not re-encoded.
///
/// `collection_id` rides along with `episode_ids` because it is in the same
/// payloads and its ids move in the same map. File ids are deliberately
/// untouched: `media_files.id` is stable across a merge.
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

/// Step 3. The source title row goes, and every cascading dependent with it.
/// The caller runs the ordinary title-delete path's logical cleanup afterwards
/// for the rows no foreign key reaches.
async fn retire_source_title(
    tx: &mut SqlTx<'_>,
    map: &MergeIdentityMap,
    outcome: &mut MergeOutcome,
) -> AppResult<()> {
    // `title_external_ids` carries `idx_title_external_ids_library_lookup` on
    // `(library_id, source, external_id)` and the destination already holds the
    // same `(source, external_id)` under FR-055, so the source rows go first —
    // before anything could write destination external ids.
    let removed = tx
        .execute(
            "DELETE FROM title_external_ids WHERE title_id = {}",
            &[SqlArg::Text(map.source_title_id.clone())],
        )
        .await?;
    record(outcome, "retire:title_external_ids", removed);

    let removed = tx
        .execute(
            "DELETE FROM titles WHERE id = {}",
            &[SqlArg::Text(map.source_title_id.clone())],
        )
        .await?;
    if removed == 0 {
        return Err(AppError::Repository(format!(
            "source title {} was already gone",
            map.source_title_id
        )));
    }
    record(outcome, "retire:titles", removed);
    Ok(())
}

#[cfg(test)]
mod tests;
