use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, CatalogDiscoveryCandidatesRecord, CatalogDiscoverySectionCandidatesRecord,
    DiscoveryCanonicalTagFilterOption, DiscoveryContentRating, DiscoveryContextIncrementalCommit,
    DiscoveryContextSnapshotCommit, DiscoveryExternalIdRecord, DiscoveryFacetRecord,
    DiscoveryHomeCandidate, DiscoveryHomeFilterOptions, DiscoveryHomeFilters,
    DiscoveryHomeSectionCandidatesRecord, DiscoveryItemLibraryProvenanceRecord,
    DiscoveryItemRecord, DiscoveryItemsPageRecord, DiscoveryItemsStorageQuery,
    DiscoveryPendingContextChangeRecord, DiscoveryPruneReport, DiscoveryPublicFeedCommit,
    DiscoveryRankComponentRecord, DiscoveryRepository, DiscoverySectionRecord,
    DiscoverySourceTagRecord, DiscoverySubmittedSubjectRecord, DiscoverySyncRunRecord,
    DiscoverySyncStateRecord, TitleRatingSummary,
};
use scryer_infrastructure_sql::json::{decode_compressed_json, encode_compressed_json};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::debug;

use crate::media::canonical_tags::{
    load_discovery_title_metadata_ratings, load_discovery_title_metadata_tags,
    replace_discovery_title_metadata_ratings_tx, replace_discovery_title_metadata_tags_tx,
};
use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore, repo_err,
};
use crate::storage::sql::json::opt_json_text;

const DISCOVERY_SYNC_STATE_COLUMNS: &str = "scope_key, last_success_generation_id,
    last_public_feed_generation_id, last_subject_fingerprint,
    last_context_snapshot_completed_at, last_incremental_reload_completed_at,
    last_public_feed_completed_at, dirty_since, dirty_reason_mask, bootstrap_started_at,
    bootstrap_quiet_until, next_context_snapshot_eligible_at,
    next_incremental_reload_eligible_at, next_public_feed_eligible_at, backoff_until,
    transient_failure_count,
    startup_jitter_seconds, context_jitter_seconds, incremental_reload_jitter_seconds,
    public_feed_jitter_seconds, last_seen_domain_event_sequence,
    inflight_context_snapshot_run_id, inflight_subject_fingerprint,
    inflight_domain_event_sequence, lease_owner_id, lease_expires_at, updated_at";

const DISCOVERY_SYNC_RUN_COLUMNS: &str = "id, kind, status, trigger_source, region, language,
    subject_count, subject_fingerprint, previous_subject_fingerprint, base_generation_id,
    changed_subject_count, affected_target_count, smg_request_id, smg_status,
    discovery_index_watermark, page_count, item_count, facet_count, acknowledged_at,
    error_text, started_at, completed_at,
    created_at, updated_at";

const DISCOVERY_INDEXED_TITLE_LIMIT: usize = 1_000;
const TITLE_RECOMMENDATION_CARD_LIMIT: usize = 24;
const TITLE_RECOMMENDATION_PAYLOAD_VERSION: i32 = 1;

const PENDING_CONTEXT_CHANGE_COLUMNS: &str = "id, scope_key, subject_key, previous_subject_key,
    change_type, title_id, previous_title_id, library_facet, raw_subject_json,
    raw_previous_subject_json, first_seen_sequence, last_seen_sequence, first_seen_at,
    last_seen_at";

const SECTION_COLUMNS: &[&str] = &[
    "id",
    "run_id",
    "section_id",
    "section_type",
    "surface",
    "title",
    "sort_index",
    "created_at",
    "updated_at",
];

const ITEM_COLUMNS: &[&str] = &[
    "id",
    "run_id",
    "base_generation_id",
    "source_run_kind",
    "section_id",
    "sort_index",
    "target_key",
    "target_kind",
    "resolved",
    "resolved_title_id",
    "display_title",
    "original_title",
    "sort_title",
    "year",
    "poster_path",
    "poster_url",
    "background_url",
    "overview",
    "content_type",
    "rating",
    "best_source",
    "source_count",
    "edge_count",
    "relation_count",
    "source_subject_count",
    "rank_score",
    "matched_subject_count",
    "tmdb_collection_id",
    "tmdb_collection_name",
    "owned_in_input",
    "tombstoned_by_run_id",
    "tombstoned_at",
    "created_at",
    "updated_at",
];

const TITLE_COLUMNS: &[&str] = &[
    "id",
    "target_key",
    "target_key_norm",
    "language",
    "target_kind",
    "resolved",
    "resolved_title_id",
    "display_title",
    "original_title",
    "sort_title",
    "year",
    "poster_path",
    "poster_url",
    "background_url",
    "overview",
    "content_type",
    "is_adult",
    "content_ratings_json",
    "tmdb_collection_id",
    "tmdb_collection_name",
    "created_at",
    "updated_at",
];

const OCCURRENCE_COLUMNS: &[&str] = &[
    "id",
    "run_id",
    "base_generation_id",
    "discovery_title_id",
    "source_run_kind",
    "section_id",
    "sort_index",
    "best_source",
    "source_count",
    "edge_count",
    "relation_count",
    "source_subject_count",
    "rank_score",
    "matched_subject_count",
    "owned_in_input",
    "tombstoned_by_run_id",
    "tombstoned_at",
    "created_at",
    "updated_at",
];

const FACET_COLUMNS: &[&str] = &[
    "run_id",
    "facet_name",
    "facet_value",
    "smg_count",
    "local_count",
];

#[derive(Clone)]
pub struct DiscoveryStore {
    datastore: StoreDatastore,
}

impl DiscoveryStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl DiscoveryRepository for DiscoveryStore {
    async fn get_discovery_sync_state(
        &self,
        scope_key: &str,
    ) -> AppResult<Option<DiscoverySyncStateRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!("SELECT {DISCOVERY_SYNC_STATE_COLUMNS} FROM discovery_sync_state WHERE scope_key = {{}}"),
            &[SqlArg::Text(scope_key.to_string())],
        )
        .await?;
        row.as_ref().map(sync_state_from_row).transpose()
    }

    async fn upsert_discovery_sync_state(&self, state: &DiscoverySyncStateRecord) -> AppResult<()> {
        let args = sync_state_args(state);
        SqlRuntime::execute_write(
            &self.datastore,
            "upsert_discovery_sync_state",
            &upsert_sql(
                "discovery_sync_state",
                &split_columns(DISCOVERY_SYNC_STATE_COLUMNS),
                &["scope_key"],
            ),
            args,
        )
        .await?;
        Ok(())
    }

    async fn try_acquire_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        lease_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "try_acquire_discovery_sync_lease",
            "INSERT INTO discovery_sync_state
                (scope_key, lease_owner_id, lease_expires_at, updated_at)
             VALUES ({}, {}, {}, {})
             ON CONFLICT(scope_key) DO UPDATE SET
                lease_owner_id = excluded.lease_owner_id,
                lease_expires_at = excluded.lease_expires_at,
                updated_at = excluded.updated_at
             WHERE discovery_sync_state.lease_owner_id IS NULL
                OR discovery_sync_state.lease_expires_at IS NULL
                OR discovery_sync_state.lease_expires_at <= {}
                OR discovery_sync_state.lease_owner_id = {}",
            vec![
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(owner_id.to_string()),
                SqlArg::Timestamp(lease_expires_at),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
                SqlArg::Text(owner_id.to_string()),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn renew_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        lease_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "renew_discovery_sync_lease",
            "UPDATE discovery_sync_state
             SET lease_expires_at = {}, updated_at = {}
             WHERE scope_key = {} AND lease_owner_id = {}",
            vec![
                SqlArg::Timestamp(lease_expires_at),
                SqlArg::Timestamp(now),
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(owner_id.to_string()),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn release_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "release_discovery_sync_lease",
            "UPDATE discovery_sync_state
             SET lease_owner_id = NULL, lease_expires_at = NULL, updated_at = {}
             WHERE scope_key = {} AND lease_owner_id = {}",
            vec![
                SqlArg::Timestamp(now),
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(owner_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn get_discovery_sync_run(&self, id: &str) -> AppResult<Option<DiscoverySyncRunRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {DISCOVERY_SYNC_RUN_COLUMNS} FROM discovery_sync_runs WHERE id = {{}}"
            ),
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        row.as_ref().map(sync_run_from_row).transpose()
    }

    async fn list_recent_discovery_sync_runs(
        &self,
        limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {DISCOVERY_SYNC_RUN_COLUMNS}
                 FROM discovery_sync_runs
                 ORDER BY COALESCE(completed_at, started_at, updated_at, created_at) DESC,
                          created_at DESC
                 LIMIT {{}}"
            ),
            &[SqlArg::I64(limit.clamp(1, 100))],
        )
        .await?;
        rows.iter().map(sync_run_from_row).collect()
    }

    async fn list_unacked_discovery_context_snapshot_runs(
        &self,
        limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {DISCOVERY_SYNC_RUN_COLUMNS}
                 FROM discovery_sync_runs
                 WHERE kind = 'context_snapshot'
                   AND status IN ('complete', 'warning')
                   AND smg_request_id IS NOT NULL
                   AND acknowledged_at IS NULL
                 ORDER BY COALESCE(completed_at, updated_at, created_at) ASC,
                          created_at ASC
                 LIMIT {{}}"
            ),
            &[SqlArg::I64(limit.clamp(1, 100))],
        )
        .await?;
        rows.iter().map(sync_run_from_row).collect()
    }

    async fn upsert_discovery_sync_run(&self, run: &DiscoverySyncRunRecord) -> AppResult<()> {
        let columns = split_columns(DISCOVERY_SYNC_RUN_COLUMNS);
        SqlRuntime::execute_write(
            &self.datastore,
            "upsert_discovery_sync_run",
            &upsert_sql("discovery_sync_runs", &columns, &["id"]),
            sync_run_args(&self.datastore, run)?,
        )
        .await?;
        Ok(())
    }

    async fn commit_discovery_context_snapshot(
        &self,
        commit: &DiscoveryContextSnapshotCommit,
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        // Own the payload once; share it across SQLite busy-retries via Arc so the
        // retryable `Fn` closure does a cheap refcount bump instead of a whole-snapshot
        // deep clone on every attempt.
        let commit = std::sync::Arc::new(commit.clone());
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "commit_discovery_context_snapshot",
            move |tx| {
                let datastore = datastore.clone();
                let commit = std::sync::Arc::clone(&commit);
                Box::pin(async move {
                    upsert_sync_run_tx(tx, &datastore, &commit.run).await?;
                    upsert_sync_state_tx(tx, &commit.state).await?;
                    delete_for_run_tx(tx, "discovery_submitted_subjects", &commit.run.id).await?;
                    delete_item_children_for_run_tx(tx, &commit.run.id).await?;
                    delete_for_run_tx(tx, "discovery_items", &commit.run.id).await?;
                    delete_for_run_tx(tx, "discovery_facets", &commit.run.id).await?;

                    for subject in &commit.submitted_subjects {
                        insert_submitted_subject_tx(tx, &datastore, subject).await?;
                    }
                    for item in &commit.items {
                        insert_item_tx(tx, &datastore, item, &commit.run.language).await?;
                    }
                    let facet_rows: Vec<Vec<SqlArg>> =
                        commit.facets.iter().map(facet_row).collect();
                    SqlRuntime::execute_batch_insert(
                        tx,
                        &insert_into_prefix("discovery_facets", FACET_COLUMNS),
                        FACET_COLUMNS.len(),
                        facet_rows,
                        "",
                    )
                    .await?;
                    if let Some(sequence) = commit.clear_pending_through_sequence {
                        clear_pending_discovery_context_changes_tx(
                            tx,
                            &commit.state.scope_key,
                            sequence,
                        )
                        .await?;
                    }

                    enforce_discovery_indexed_title_limit_tx(tx, &commit.state.scope_key).await?;

                    Ok(())
                })
            },
        )
        .await
    }

    async fn commit_discovery_context_incremental(
        &self,
        commit: &DiscoveryContextIncrementalCommit,
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        // Arc-share the payload across SQLite busy-retries.
        let commit = std::sync::Arc::new(commit.clone());
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "commit_discovery_context_incremental",
            move |tx| {
                let datastore = datastore.clone();
                let commit = std::sync::Arc::clone(&commit);
                Box::pin(async move {
                    upsert_sync_run_tx(tx, &datastore, &commit.run).await?;
                    upsert_sync_state_tx(tx, &commit.state).await?;
                    delete_item_children_for_run_tx(tx, &commit.run.id).await?;
                    delete_for_run_tx(tx, "discovery_items", &commit.run.id).await?;
                    tombstone_discovery_items_tx(
                        tx,
                        commit.run.base_generation_id.as_deref(),
                        &commit.tombstone_target_keys,
                        &commit.run.id,
                        commit.run.completed_at.unwrap_or(commit.run.updated_at),
                    )
                    .await?;
                    for item in &commit.items {
                        insert_item_tx(tx, &datastore, item, &commit.run.language).await?;
                    }
                    if let Some(sequence) = commit.clear_pending_through_sequence {
                        clear_pending_discovery_context_changes_tx(
                            tx,
                            &commit.state.scope_key,
                            sequence,
                        )
                        .await?;
                    }
                    enforce_discovery_indexed_title_limit_tx(tx, &commit.state.scope_key).await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn commit_discovery_public_feed(
        &self,
        commit: &DiscoveryPublicFeedCommit,
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        // Arc-share the payload across SQLite busy-retries.
        let commit = std::sync::Arc::new(commit.clone());
        SqlRuntime::run_in_transaction(&self.datastore, "commit_discovery_public_feed", move |tx| {
            let datastore = datastore.clone();
            let commit = std::sync::Arc::clone(&commit);
            Box::pin(async move {
                upsert_sync_run_tx(tx, &datastore, &commit.run)
                    .await
                    .inspect_err(|error| {
                        log_discovery_public_feed_persistence_failure(
                            "upsert_sync_run",
                            commit.as_ref(),
                            error,
                        );
                    })?;
                upsert_sync_state_tx(tx, &commit.state)
                    .await
                    .inspect_err(|error| {
                        log_discovery_public_feed_persistence_failure(
                            "upsert_sync_state",
                            commit.as_ref(),
                            error,
                        );
                    })?;
                delete_for_run_tx(tx, "discovery_section_items", &commit.run.id)
                    .await
                    .inspect_err(|error| {
                        log_discovery_public_feed_persistence_failure(
                            "delete_section_items",
                            commit.as_ref(),
                            error,
                        );
                    })?;
                delete_for_run_tx(tx, "discovery_sections", &commit.run.id)
                    .await
                    .inspect_err(|error| {
                        log_discovery_public_feed_persistence_failure(
                            "delete_sections",
                            commit.as_ref(),
                            error,
                        );
                    })?;
                delete_item_children_for_run_tx(tx, &commit.run.id)
                    .await
                    .inspect_err(|error| {
                        log_discovery_public_feed_persistence_failure(
                            "delete_item_children",
                            commit.as_ref(),
                            error,
                        );
                    })?;
                delete_for_run_tx(tx, "discovery_items", &commit.run.id)
                    .await
                    .inspect_err(|error| {
                        log_discovery_public_feed_persistence_failure(
                            "delete_items",
                            commit.as_ref(),
                            error,
                        );
                    })?;
                for section in &commit.sections {
                    insert_section_tx(tx, &datastore, section)
                        .await
                        .inspect_err(|error| {
                            tracing::warn!(
                                run_id = %commit.run.id,
                                section_id = %section.section_id,
                                section_type = %section.section_type,
                                error = %error,
                                "failed to persist discovery public-feed section"
                            );
                        })?;
                }
                for item in &commit.items {
                    insert_item_tx(tx, &datastore, item, &commit.run.language)
                        .await
                        .inspect_err(|error| {
                            tracing::warn!(
                                run_id = %commit.run.id,
                                item_id = %item.id,
                                target_key = %item.target_key,
                                target_kind = %item.target_kind,
                                resolved_title_id = ?item.resolved_title_id,
                                section_id = ?item.section_id,
                                error = %error,
                                "failed to persist discovery public-feed item"
                            );
                        })?;
                }
                enforce_discovery_indexed_title_limit_tx(tx, &commit.state.scope_key).await?;
                Ok(())
            })
        })
        .await
    }

    async fn replace_discovery_submitted_subjects(
        &self,
        run_id: &str,
        subjects: &[DiscoverySubmittedSubjectRecord],
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let run_id = run_id.to_string();
        let subjects = subjects.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_discovery_subjects", move |tx| {
            let datastore = datastore.clone();
            let run_id = run_id.clone();
            let subjects = subjects.clone();
            Box::pin(async move {
                delete_for_run_tx(tx, "discovery_submitted_subjects", &run_id).await?;
                for subject in &subjects {
                    insert_submitted_subject_tx(tx, &datastore, subject).await?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn list_discovery_submitted_subjects(
        &self,
        run_id: &str,
    ) -> AppResult<Vec<DiscoverySubmittedSubjectRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT run_id, subject_key, title_id, library_id, library_facet, title_kind, display_title,
                    external_ids_json, raw_subject_json
             FROM discovery_submitted_subjects
             WHERE run_id = {}
             ORDER BY subject_key ASC, library_id ASC, title_id ASC",
            &[SqlArg::Text(run_id.to_string())],
        )
        .await?;
        rows.iter().map(submitted_subject_from_row).collect()
    }

    async fn upsert_pending_discovery_context_change(
        &self,
        change: &DiscoveryPendingContextChangeRecord,
    ) -> AppResult<()> {
        SqlRuntime::execute_write(
            &self.datastore,
            "upsert_pending_discovery_context_change",
            &upsert_pending_context_change_sql(),
            pending_context_change_args(&self.datastore, change)?,
        )
        .await?;
        Ok(())
    }

    async fn get_pending_discovery_context_change(
        &self,
        id: &str,
    ) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {PENDING_CONTEXT_CHANGE_COLUMNS}
                 FROM discovery_pending_context_changes
                 WHERE id = {{}}"
            ),
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        row.as_ref()
            .map(pending_context_change_from_row)
            .transpose()
    }

    async fn delete_pending_discovery_context_change(&self, id: &str) -> AppResult<u64> {
        SqlRuntime::execute_write(
            &self.datastore,
            "delete_pending_discovery_context_change",
            "DELETE FROM discovery_pending_context_changes WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn list_all_pending_discovery_context_changes(
        &self,
        scope_key: &str,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {PENDING_CONTEXT_CHANGE_COLUMNS}
                 FROM discovery_pending_context_changes
                 WHERE scope_key = {{}}
                 ORDER BY last_seen_at ASC, id ASC"
            ),
            &[SqlArg::Text(scope_key.to_string())],
        )
        .await?;
        rows.iter().map(pending_context_change_from_row).collect()
    }

    async fn list_pending_discovery_context_changes(
        &self,
        scope_key: &str,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {PENDING_CONTEXT_CHANGE_COLUMNS}
                 FROM discovery_pending_context_changes
                 WHERE scope_key = {{}}
                 ORDER BY last_seen_at ASC, id ASC
                 LIMIT {{}}"
            ),
            &[SqlArg::Text(scope_key.to_string()), SqlArg::I64(limit)],
        )
        .await?;
        rows.iter().map(pending_context_change_from_row).collect()
    }

    async fn count_pending_discovery_context_changes(&self, scope_key: &str) -> AppResult<i64> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS pending_count FROM discovery_pending_context_changes WHERE scope_key = {}",
            &[SqlArg::Text(scope_key.to_string())],
        )
        .await?;

        row.as_ref().map_or(Ok(0), |row| row.i64("pending_count"))
    }

    async fn clear_pending_discovery_context_changes_through_sequence(
        &self,
        scope_key: &str,
        last_seen_sequence: i64,
    ) -> AppResult<u64> {
        SqlRuntime::execute_write(
            &self.datastore,
            "clear_pending_discovery_context_changes_through_sequence",
            "DELETE FROM discovery_pending_context_changes
             WHERE scope_key = {}
               AND last_seen_sequence IS NOT NULL
               AND last_seen_sequence <= {}",
            vec![
                SqlArg::Text(scope_key.to_string()),
                SqlArg::I64(last_seen_sequence),
            ],
        )
        .await
    }

    async fn replace_discovery_sections(
        &self,
        run_id: &str,
        sections: &[DiscoverySectionRecord],
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let run_id = run_id.to_string();
        let sections = sections.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_discovery_sections", move |tx| {
            let datastore = datastore.clone();
            let run_id = run_id.clone();
            let sections = sections.clone();
            Box::pin(async move {
                delete_for_run_tx(tx, "discovery_section_items", &run_id).await?;
                delete_for_run_tx(tx, "discovery_sections", &run_id).await?;
                for section in &sections {
                    insert_section_tx(tx, &datastore, section).await?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn replace_discovery_items(
        &self,
        run_id: &str,
        items: &[DiscoveryItemRecord],
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let language = discovery_run_language(&self.datastore, run_id)
            .await?
            .unwrap_or_else(|| "eng".to_string());
        let run_id = run_id.to_string();
        let items = items.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_discovery_items", move |tx| {
            let datastore = datastore.clone();
            let language = language.clone();
            let run_id = run_id.clone();
            let items = items.clone();
            Box::pin(async move {
                delete_item_children_for_run_tx(tx, &run_id).await?;
                delete_for_run_tx(tx, "discovery_items", &run_id).await?;
                for item in &items {
                    insert_item_tx(tx, &datastore, item, &language).await?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn replace_discovery_facets(
        &self,
        run_id: &str,
        facets: &[DiscoveryFacetRecord],
    ) -> AppResult<()> {
        let run_id = run_id.to_string();
        let facets = facets.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_discovery_facets", move |tx| {
            let run_id = run_id.clone();
            let facets = facets.clone();
            Box::pin(async move {
                delete_for_run_tx(tx, "discovery_facets", &run_id).await?;
                let facet_rows: Vec<Vec<SqlArg>> = facets.iter().map(facet_row).collect();
                SqlRuntime::execute_batch_insert(
                    tx,
                    &insert_into_prefix("discovery_facets", FACET_COLUMNS),
                    FACET_COLUMNS.len(),
                    facet_rows,
                    "",
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn list_discovery_sections(
        &self,
        run_id: &str,
        surface: Option<&str>,
    ) -> AppResult<Vec<DiscoverySectionRecord>> {
        let mut args = vec![SqlArg::Text(run_id.to_string())];
        let surface_clause = if let Some(surface) = surface {
            args.push(SqlArg::Text(surface.to_string()));
            " AND surface = {}"
        } else {
            ""
        };
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {}
                 FROM discovery_sections
                 WHERE run_id = {{}}{surface_clause}
                 ORDER BY sort_index ASC, section_id ASC",
                SECTION_COLUMNS.join(", ")
            ),
            &args,
        )
        .await?;
        rows.iter().map(section_from_row).collect()
    }

    async fn list_public_discovery_section_items(
        &self,
        run_id: &str,
        allowed_media_kinds: &[String],
        include_unresolved: bool,
        filters: &DiscoveryHomeFilters,
        limit_per_section: i64,
    ) -> AppResult<Vec<DiscoveryHomeSectionCandidatesRecord>> {
        let sections = self.list_discovery_sections(run_id, Some("public")).await?;
        if sections.is_empty() {
            return Ok(Vec::new());
        }
        let rows = fetch_public_section_item_rows(
            &self.datastore,
            run_id,
            allowed_media_kinds,
            include_unresolved,
            filters,
            limit_per_section.clamp(1, 100),
        )
        .await?;
        section_candidates_from_rows(&self.datastore, sections, rows).await
    }

    async fn list_personalized_discovery_home_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        include_unresolved: bool,
        filters: &DiscoveryHomeFilters,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>> {
        fetch_personalized_home_candidates(
            &self.datastore,
            run_id,
            readable_library_ids,
            allowed_media_kinds,
            include_unresolved,
            filters,
            None,
            limit.clamp(1, 5_000),
        )
        .await
    }

    async fn list_personalized_complete_collection_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        include_unresolved: bool,
        filters: &DiscoveryHomeFilters,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>> {
        fetch_personalized_home_candidates(
            &self.datastore,
            run_id,
            readable_library_ids,
            allowed_media_kinds,
            include_unresolved,
            filters,
            Some(PersonalizedItemSubset::CompleteCollection),
            limit.clamp(1, 2_000),
        )
        .await
    }

    async fn list_personalized_discovery_facets(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        include_unresolved: bool,
    ) -> AppResult<Vec<DiscoveryFacetRecord>> {
        fetch_personalized_facets(
            &self.datastore,
            run_id,
            readable_library_ids,
            allowed_media_kinds,
            include_unresolved,
        )
        .await
    }

    async fn list_discovery_home_top_rated_items(
        &self,
        public_run_id: Option<&str>,
        context_run_id: Option<&str>,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        owned_library_ids: &[String],
        excluded_identity_keys: &[String],
        include_unresolved: bool,
        filters: &DiscoveryHomeFilters,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>> {
        fetch_discovery_home_top_rated_candidates(
            &self.datastore,
            public_run_id,
            context_run_id,
            readable_library_ids,
            allowed_media_kinds,
            owned_library_ids,
            excluded_identity_keys,
            include_unresolved,
            filters,
            limit.clamp(1, 5_000),
        )
        .await
    }

    async fn hydrate_discovery_home_candidates(
        &self,
        candidates: &mut [DiscoveryHomeCandidate],
    ) -> AppResult<()> {
        hydrate_discovery_home_candidates(&self.datastore, candidates).await
    }

    async fn hydrate_discovery_home_hero(
        &self,
        candidate: &mut DiscoveryHomeCandidate,
    ) -> AppResult<()> {
        hydrate_discovery_home_hero(&self.datastore, candidate).await
    }

    async fn list_discovery_home_filter_options(
        &self,
        public_run_id: Option<&str>,
        context_run_id: Option<&str>,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        include_unresolved: bool,
    ) -> AppResult<DiscoveryHomeFilterOptions> {
        fetch_discovery_home_filter_options(
            &self.datastore,
            public_run_id,
            context_run_id,
            readable_library_ids,
            allowed_media_kinds,
            include_unresolved,
        )
        .await
    }

    async fn list_catalog_public_discovery_items(
        &self,
        run_id: &str,
        owned_library_ids: &[String],
        excluded_identity_keys: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit: i64,
    ) -> AppResult<CatalogDiscoveryCandidatesRecord> {
        fetch_catalog_public_items(
            &self.datastore,
            run_id,
            owned_library_ids,
            excluded_identity_keys,
            media_kind,
            include_unresolved,
            limit.clamp(1, 1_000),
        )
        .await
    }

    async fn list_catalog_public_discovery_sections(
        &self,
        run_id: &str,
        owned_library_ids: &[String],
        excluded_identity_keys: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit_per_section: i64,
    ) -> AppResult<Vec<CatalogDiscoverySectionCandidatesRecord>> {
        fetch_catalog_public_sections(
            &self.datastore,
            run_id,
            owned_library_ids,
            excluded_identity_keys,
            media_kind,
            include_unresolved,
            limit_per_section.clamp(1, 1_000),
        )
        .await
    }

    async fn list_catalog_personalized_discovery_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit: i64,
    ) -> AppResult<CatalogDiscoveryCandidatesRecord> {
        fetch_catalog_personalized_items(
            &self.datastore,
            run_id,
            readable_library_ids,
            media_kind,
            include_unresolved,
            limit.clamp(1, 1_000),
        )
        .await
    }

    async fn query_discovery_items(
        &self,
        query: &DiscoveryItemsStorageQuery,
    ) -> AppResult<DiscoveryItemsPageRecord> {
        query_discovery_items_page(&self.datastore, query).await
    }

    async fn replace_title_more_like_this_items(
        &self,
        title_id: &str,
        language: &str,
        items: &[DiscoveryItemRecord],
    ) -> AppResult<()> {
        let datastore = self.datastore.clone();
        let title_id = title_id.to_string();
        let language = normalize_discovery_language(language);
        let mut items = items
            .iter()
            .take(TITLE_RECOMMENDATION_CARD_LIMIT)
            .cloned()
            .collect::<Vec<_>>();
        enrich_recommendation_items_from_normalized(&datastore, &language, &mut items).await?;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "replace_title_more_like_this_items",
            move |tx| {
                let title_id = title_id.clone();
                let language = language.clone();
                let items = items.clone();
                Box::pin(async move {
                    delete_title_more_like_this_items_tx(tx, &title_id).await?;
                    for item in &items {
                        insert_title_more_like_this_item_tx(tx, &title_id, item, &language).await?;
                    }
                    delete_orphan_title_recommendation_cards_tx(tx).await?;
                    delete_unreferenced_discovery_titles_tx(tx).await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn list_title_more_like_this_items(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT m.source_title_id, m.discovery_title_id, m.sort_index, m.rank_score,
                    m.best_source, m.source_count, m.edge_count, m.relation_count,
                    m.source_subject_count, m.created_at, m.updated_at,
                    c.payload_version, c.payload_blob
                 FROM title_more_like_this_items m
                 JOIN title_recommendation_cards c
                   ON c.discovery_title_id = m.discovery_title_id
                 WHERE m.source_title_id = {}
                 ORDER BY m.sort_index ASC,
                          COALESCE(m.rank_score, 0) DESC,
                          m.discovery_title_id ASC
                 LIMIT {}",
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::I64(limit.clamp(0, 100)),
            ],
        )
        .await?;
        let needs_legacy = rows
            .iter()
            .any(|row| row.opt_bytes("payload_blob").ok().flatten().is_none());
        let mut legacy_by_item_id = if needs_legacy {
            legacy_title_more_like_this_items(&self.datastore, title_id, limit)
                .await?
                .into_iter()
                .map(|item| (item.id.clone(), item))
                .collect::<HashMap<_, _>>()
        } else {
            HashMap::new()
        };

        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            let item_id = format!(
                "{}:more-like-this:{}",
                row.text("source_title_id")?,
                row.text("discovery_title_id")?
            );
            let item =
                recommendation_item_from_row(row)?.or_else(|| legacy_by_item_id.remove(&item_id));
            if let Some(item) = item.filter(recommendation_item_is_displayable) {
                items.push(item);
            }
        }
        Ok(items)
    }

    async fn list_discovery_items_for_generation(
        &self,
        base_generation_id: &str,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {}
                 FROM discovery_items i
                 JOIN discovery_titles t
                   ON t.id = i.discovery_title_id
                 WHERE i.base_generation_id = {{}}
                   AND i.tombstoned_at IS NULL
                 ORDER BY COALESCE(i.section_id, '') ASC,
                          i.sort_index ASC,
                          i.id ASC",
                discovery_item_projection(&self.datastore, "i", "t")
            ),
            &[SqlArg::Text(base_generation_id.to_string())],
        )
        .await?;
        let mut items = rows
            .iter()
            .map(item_from_row)
            .collect::<AppResult<Vec<_>>>()?;
        let title_ids = discovery_title_ids_from_rows(&rows)?;
        hydrate_discovery_items(&self.datastore, &mut items, &title_ids).await?;
        Ok(items)
    }

    async fn list_discovery_facets(&self, run_id: &str) -> AppResult<Vec<DiscoveryFacetRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT run_id, facet_name, facet_value, smg_count, local_count
             FROM discovery_facets
             WHERE run_id = {}
             ORDER BY facet_name ASC, facet_value ASC",
            &[SqlArg::Text(run_id.to_string())],
        )
        .await?;
        rows.iter().map(facet_from_row).collect()
    }

    async fn prune_discovery_history(
        &self,
        scope_key: &str,
        retain_successful_per_kind: usize,
        diagnostic_cutoff: DateTime<Utc>,
    ) -> AppResult<DiscoveryPruneReport> {
        backfill_title_recommendation_cards(&self.datastore).await?;
        // Prune runs from the housekeeping job, NOT under the
        // discovery sync lease, so it can race a concurrent commit. The whole
        // candidate read + keep-set decision + delete loop now runs inside one
        // transaction. On SQLite `run_in_transaction` holds the app-wide writer
        // gate, so prune and every commit_discovery_* are strictly mutually
        // exclusive — the prune<->commit race cannot occur. On Postgres the
        // generation pointers are read inside the same transaction as the
        // deletes, and every active/inflight generation id is always added to
        // keep_ids, so an atomic generation swap is never torn: a run that is (or
        // becomes) the active generation is retained. Atomicity also means a
        // failed prune leaves history untouched instead of half-deleted.
        let scope_key = scope_key.to_string();
        let runs_deleted =
            SqlRuntime::run_in_transaction(&self.datastore, "prune_discovery_history", move |tx| {
                let scope_key = scope_key.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::fetch_all(
                        SqlExec::Tx(tx),
                        "SELECT id, kind, status, base_generation_id, updated_at
                         FROM discovery_sync_runs
                         ORDER BY updated_at DESC, id DESC",
                        &[],
                    )
                    .await?;
                    let candidates = rows
                        .iter()
                        .map(discovery_run_prune_candidate_from_row)
                        .collect::<AppResult<Vec<_>>>()?;

                    // Read the live generation pointers inside the transaction so
                    // the keep-set reflects the freshest committed swap.
                    let state_row = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "SELECT last_success_generation_id,
                                last_public_feed_generation_id,
                                inflight_context_snapshot_run_id
                         FROM discovery_sync_state
                         WHERE scope_key = {}",
                        &[SqlArg::Text(scope_key.clone())],
                    )
                    .await?;

                    let mut keep_ids = HashSet::new();
                    let mut active_context_generation_id: Option<String> = None;
                    if let Some(row) = &state_row {
                        let last_success = row.opt_text("last_success_generation_id")?;
                        active_context_generation_id = last_success.clone();
                        keep_optional_id(&mut keep_ids, last_success.as_deref());
                        keep_optional_id(
                            &mut keep_ids,
                            row.opt_text("last_public_feed_generation_id")?.as_deref(),
                        );
                        keep_optional_id(
                            &mut keep_ids,
                            row.opt_text("inflight_context_snapshot_run_id")?.as_deref(),
                        );
                    }

                    if let Some(active_context_generation_id) =
                        active_context_generation_id.as_deref()
                    {
                        for candidate in &candidates {
                            if candidate.kind == "context_incremental"
                                && candidate.status == "complete"
                                && candidate.base_generation_id.as_deref()
                                    == Some(active_context_generation_id)
                            {
                                keep_ids.insert(candidate.id.clone());
                            }
                        }
                    }

                    let mut retained_successful_by_kind = HashMap::<String, usize>::new();
                    for candidate in &candidates {
                        if keep_ids.contains(&candidate.id)
                            && discovery_run_status_is_successful(&candidate.status)
                        {
                            retained_successful_by_kind.insert(candidate.kind.clone(), 1);
                        }
                    }
                    for candidate in &candidates {
                        if discovery_run_status_is_successful(&candidate.status) {
                            let retained = retained_successful_by_kind
                                .entry(candidate.kind.clone())
                                .or_default();
                            if *retained < retain_successful_per_kind {
                                keep_ids.insert(candidate.id.clone());
                                *retained += 1;
                            }
                        }

                        if discovery_run_status_is_diagnostic(&candidate.status)
                            && candidate.updated_at >= diagnostic_cutoff
                        {
                            keep_ids.insert(candidate.id.clone());
                        }

                        if candidate.status == "running" {
                            keep_ids.insert(candidate.id.clone());
                        }
                    }

                    let mut runs_deleted = 0u64;
                    for candidate in &candidates {
                        if keep_ids.contains(&candidate.id) {
                            continue;
                        }
                        runs_deleted += SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "DELETE FROM discovery_sync_runs WHERE id = {}",
                            &[SqlArg::Text(candidate.id.clone())],
                        )
                        .await?;
                    }
                    enforce_discovery_indexed_title_limit_tx(tx, &scope_key).await?;
                    delete_orphan_title_recommendation_cards_tx(tx).await?;
                    delete_unreferenced_discovery_titles_tx(tx).await?;

                    Ok(runs_deleted)
                })
            })
            .await?;

        Ok(DiscoveryPruneReport { runs_deleted })
    }
}

struct DiscoveryRunPruneCandidate {
    id: String,
    kind: String,
    status: String,
    base_generation_id: Option<String>,
    updated_at: DateTime<Utc>,
}

fn discovery_run_prune_candidate_from_row(row: &SqlRow) -> AppResult<DiscoveryRunPruneCandidate> {
    Ok(DiscoveryRunPruneCandidate {
        id: row.text("id")?,
        kind: row.text("kind")?,
        status: row.text("status")?,
        base_generation_id: row.opt_text("base_generation_id")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn keep_optional_id(keep_ids: &mut HashSet<String>, id: Option<&str>) {
    if let Some(id) = id {
        keep_ids.insert(id.to_string());
    }
}

async fn enforce_discovery_indexed_title_limit_tx(
    tx: &mut SqlTx<'_>,
    scope_key: &str,
) -> AppResult<()> {
    let Some(state) = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT last_success_generation_id, last_public_feed_generation_id
         FROM discovery_sync_state
         WHERE scope_key = {}",
        &[SqlArg::Text(scope_key.to_string())],
    )
    .await?
    else {
        return Ok(());
    };

    let context_run_id = state.opt_text("last_success_generation_id")?;
    let public_run_id = state.opt_text("last_public_feed_generation_id")?;
    let mut keep_title_ids = HashSet::new();

    if let Some(public_run_id) = public_run_id.as_deref() {
        for row in SqlRuntime::fetch_all(
            SqlExec::Tx(tx),
            "SELECT DISTINCT discovery_title_id
             FROM discovery_items
             WHERE run_id = {}",
            &[SqlArg::Text(public_run_id.to_string())],
        )
        .await?
        {
            keep_title_ids.insert(row.text("discovery_title_id")?);
        }
    }

    if keep_title_ids.len() < DISCOVERY_INDEXED_TITLE_LIMIT
        && let Some(context_run_id) = context_run_id.as_deref()
    {
        let candidates = SqlRuntime::fetch_all(
            SqlExec::Tx(tx),
            "SELECT discovery_title_id,
                        MIN(CASE WHEN owned_in_input = FALSE THEN 0 ELSE 1 END) AS owned_rank,
                        MAX(COALESCE(rank_score, 0)) AS best_rank_score,
                        MAX(matched_subject_count) AS best_matched_subject_count,
                        MAX(COALESCE(source_count, 0)) AS best_source_count
                 FROM discovery_items
                 WHERE tombstoned_at IS NULL
                   AND (run_id = {} OR base_generation_id = {})
                 GROUP BY discovery_title_id
                 ORDER BY owned_rank ASC,
                          best_rank_score DESC,
                          best_matched_subject_count DESC,
                          best_source_count DESC,
                          discovery_title_id ASC",
            &[
                SqlArg::Text(context_run_id.to_string()),
                SqlArg::Text(context_run_id.to_string()),
            ],
        )
        .await?;
        for candidate in candidates {
            if keep_title_ids.len() >= DISCOVERY_INDEXED_TITLE_LIMIT {
                break;
            }
            keep_title_ids.insert(candidate.text("discovery_title_id")?);
        }
    }

    let mut active_args = Vec::new();
    let mut active_clauses = Vec::new();
    if let Some(public_run_id) = public_run_id.as_deref() {
        active_clauses.push("run_id = {}".to_string());
        active_args.push(SqlArg::Text(public_run_id.to_string()));
    }
    if let Some(context_run_id) = context_run_id.as_deref() {
        active_clauses.push("(run_id = {} OR base_generation_id = {})".to_string());
        active_args.push(SqlArg::Text(context_run_id.to_string()));
        active_args.push(SqlArg::Text(context_run_id.to_string()));
    }
    if active_clauses.is_empty() {
        return Ok(());
    }

    let active_rows = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        &format!(
            "SELECT id, discovery_title_id
             FROM discovery_items
             WHERE {}",
            active_clauses.join(" OR ")
        ),
        &active_args,
    )
    .await?;
    let mut delete_item_ids = Vec::new();
    for row in active_rows {
        if !keep_title_ids.contains(&row.text("discovery_title_id")?) {
            delete_item_ids.push(row.text("id")?);
        }
    }
    for chunk in delete_item_ids.chunks(500) {
        let args = chunk.iter().cloned().map(SqlArg::Text).collect::<Vec<_>>();
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            &format!(
                "DELETE FROM discovery_items WHERE id IN ({})",
                placeholders(args.len())
            ),
            &args,
        )
        .await?;
    }
    delete_unreferenced_discovery_titles_tx(tx).await?;
    Ok(())
}

fn discovery_run_status_is_successful(status: &str) -> bool {
    status == "complete" || status == "warning"
}

fn discovery_run_status_is_diagnostic(status: &str) -> bool {
    status == "warning" || status == "failed" || status == "deferred"
}

fn split_columns(columns: &str) -> Vec<&str> {
    columns
        .split(',')
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .collect()
}

fn placeholders(count: usize) -> String {
    (0..count).map(|_| "{}").collect::<Vec<_>>().join(", ")
}

fn source_identifier_title_clause(datastore: &StoreDatastore, value_expression: &str) -> String {
    match datastore {
        StoreDatastore::Sqlite { .. } => {
            let normalized = format!("LOWER(TRIM({value_expression}))");
            let first_colon = format!("INSTR({normalized}, ':')");
            let after_first = format!("SUBSTR({normalized}, {first_colon} + 1)");
            let second_colon = format!("INSTR({after_first}, ':')");
            let source_part = format!("SUBSTR({normalized}, 1, {first_colon} - 1)");
            let kind_part = format!("SUBSTR({after_first}, 1, {second_colon} - 1)");
            format!(
                "{first_colon} > 1
                 AND {second_colon} > 1
                 AND {source_part} GLOB '[a-z]*'
                 AND {source_part} NOT GLOB '*[^a-z0-9_+-]*'
                 AND {kind_part} NOT GLOB '*[^a-z0-9_+-]*'"
            )
        }
        StoreDatastore::Postgres { .. } => {
            format!("TRIM({value_expression}) ~* '^[a-z][a-z0-9_+-]*:[a-z0-9_+-]+:'")
        }
    }
}

fn numeric_title_clause(datastore: &StoreDatastore, value_expression: &str) -> String {
    match datastore {
        StoreDatastore::Sqlite { .. } => {
            let normalized = format!("TRIM({value_expression})");
            format!("{normalized} GLOB '[0-9]*' AND {normalized} NOT GLOB '*[^0-9]*'")
        }
        StoreDatastore::Postgres { .. } => {
            format!("TRIM({value_expression}) ~ '^[0-9]+$'")
        }
    }
}

fn useful_discovery_title_clause(datastore: &StoreDatastore, value_expression: &str) -> String {
    format!(
        "NULLIF(TRIM({value_expression}), '') IS NOT NULL
         AND NOT ({})
         AND NOT ({})",
        source_identifier_title_clause(datastore, value_expression),
        numeric_title_clause(datastore, value_expression)
    )
}

fn displayable_discovery_title_clause(datastore: &StoreDatastore, title_alias: &str) -> String {
    format!(
        "NULLIF(TRIM({title_alias}.poster_url), '') IS NOT NULL
         AND (({}) OR ({}) OR ({}))",
        useful_discovery_title_clause(datastore, &format!("{title_alias}.display_title")),
        useful_discovery_title_clause(datastore, &format!("{title_alias}.sort_title")),
        useful_discovery_title_clause(datastore, &format!("{title_alias}.original_title"))
    )
}

fn typed_null_rating_expression(datastore: &StoreDatastore) -> &'static str {
    match datastore {
        StoreDatastore::Sqlite { .. } => "CAST(NULL AS REAL) AS rating",
        StoreDatastore::Postgres { .. } => "CAST(NULL AS DOUBLE PRECISION) AS rating",
    }
}

fn discovery_item_projection(
    datastore: &StoreDatastore,
    item_alias: &str,
    title_alias: &str,
) -> String {
    discovery_item_projection_with_presentation(datastore, item_alias, title_alias, true)
}

fn discovery_home_candidate_projection(
    datastore: &StoreDatastore,
    item_alias: &str,
    title_alias: &str,
) -> String {
    format!(
        "{}, CASE WHEN NULLIF(TRIM({title_alias}.background_url), '') IS NULL THEN FALSE ELSE TRUE END AS has_hero_backdrop",
        discovery_item_projection_with_presentation(datastore, item_alias, title_alias, false)
    )
}

fn discovery_item_projection_with_presentation(
    datastore: &StoreDatastore,
    item_alias: &str,
    title_alias: &str,
    include_presentation: bool,
) -> String {
    let background_url = if include_presentation {
        format!("{title_alias}.background_url AS background_url")
    } else {
        "NULL AS background_url".to_string()
    };
    let overview = if include_presentation {
        format!("{title_alias}.overview AS overview")
    } else {
        "NULL AS overview".to_string()
    };
    [
        format!("{item_alias}.id AS id"),
        format!("{item_alias}.run_id AS run_id"),
        format!("{item_alias}.base_generation_id AS base_generation_id"),
        format!("{item_alias}.source_run_kind AS source_run_kind"),
        format!("{item_alias}.section_id AS section_id"),
        format!("{item_alias}.sort_index AS sort_index"),
        format!("{title_alias}.target_key AS target_key"),
        format!("{title_alias}.target_kind AS target_kind"),
        format!("{title_alias}.resolved AS resolved"),
        format!("{title_alias}.resolved_title_id AS resolved_title_id"),
        format!("{title_alias}.display_title AS display_title"),
        format!("{title_alias}.original_title AS original_title"),
        format!("{title_alias}.sort_title AS sort_title"),
        format!("{title_alias}.year AS year"),
        format!("{title_alias}.poster_path AS poster_path"),
        format!("{title_alias}.poster_url AS poster_url"),
        background_url,
        overview,
        format!("{title_alias}.content_type AS content_type"),
        format!("{title_alias}.is_adult AS is_adult"),
        format!("{title_alias}.content_ratings_json AS content_ratings_json"),
        typed_null_rating_expression(datastore).to_string(),
        format!("{item_alias}.best_source AS best_source"),
        format!("{item_alias}.source_count AS source_count"),
        format!("{item_alias}.edge_count AS edge_count"),
        format!("{item_alias}.relation_count AS relation_count"),
        format!("{item_alias}.source_subject_count AS source_subject_count"),
        format!("{item_alias}.rank_score AS rank_score"),
        format!("{item_alias}.matched_subject_count AS matched_subject_count"),
        format!("{title_alias}.tmdb_collection_id AS tmdb_collection_id"),
        format!("{title_alias}.tmdb_collection_name AS tmdb_collection_name"),
        format!("{item_alias}.owned_in_input AS owned_in_input"),
        format!("{item_alias}.tombstoned_by_run_id AS tombstoned_by_run_id"),
        format!("{item_alias}.tombstoned_at AS tombstoned_at"),
        format!("{item_alias}.created_at AS created_at"),
        format!("{item_alias}.updated_at AS updated_at"),
        format!("{item_alias}.discovery_title_id AS discovery_title_id"),
    ]
    .join(", ")
}

fn discovery_item_row_columns() -> String {
    format!(
        "{}, is_adult, content_ratings_json, discovery_title_id",
        ITEM_COLUMNS.join(", ")
    )
}

fn discovery_home_candidate_row_columns() -> String {
    format!("{}, has_hero_backdrop", discovery_item_row_columns())
}

fn title_more_like_this_projection(datastore: &StoreDatastore) -> String {
    [
        "source_title_id || ':more-like-this:' || discovery_title_id AS id".to_string(),
        "source_title_id AS run_id".to_string(),
        "NULL AS base_generation_id".to_string(),
        "'title_more_like_this' AS source_run_kind".to_string(),
        "NULL AS section_id".to_string(),
        "title_more_like_this_items.sort_index AS sort_index".to_string(),
        "t.target_key AS target_key".to_string(),
        "t.target_kind AS target_kind".to_string(),
        "t.resolved AS resolved".to_string(),
        "t.resolved_title_id AS resolved_title_id".to_string(),
        "t.display_title AS display_title".to_string(),
        "t.original_title AS original_title".to_string(),
        "t.sort_title AS sort_title".to_string(),
        "t.year AS year".to_string(),
        "t.poster_path AS poster_path".to_string(),
        "t.poster_url AS poster_url".to_string(),
        "t.background_url AS background_url".to_string(),
        "t.overview AS overview".to_string(),
        "t.content_type AS content_type".to_string(),
        "t.is_adult AS is_adult".to_string(),
        "t.content_ratings_json AS content_ratings_json".to_string(),
        typed_null_rating_expression(datastore).to_string(),
        "title_more_like_this_items.best_source AS best_source".to_string(),
        "title_more_like_this_items.source_count AS source_count".to_string(),
        "title_more_like_this_items.edge_count AS edge_count".to_string(),
        "title_more_like_this_items.relation_count AS relation_count".to_string(),
        "title_more_like_this_items.source_subject_count AS source_subject_count".to_string(),
        "title_more_like_this_items.rank_score AS rank_score".to_string(),
        "0 AS matched_subject_count".to_string(),
        "t.tmdb_collection_id AS tmdb_collection_id".to_string(),
        "t.tmdb_collection_name AS tmdb_collection_name".to_string(),
        "FALSE AS owned_in_input".to_string(),
        "NULL AS tombstoned_by_run_id".to_string(),
        "NULL AS tombstoned_at".to_string(),
        "title_more_like_this_items.created_at AS created_at".to_string(),
        "title_more_like_this_items.updated_at AS updated_at".to_string(),
        "t.id AS discovery_title_id".to_string(),
    ]
    .join(", ")
}

async fn legacy_title_more_like_this_items(
    datastore: &StoreDatastore,
    title_id: &str,
    limit: i64,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        &format!(
            "SELECT {}
             FROM title_more_like_this_items
             JOIN discovery_titles t
               ON t.id = title_more_like_this_items.discovery_title_id
             WHERE source_title_id = {{}}
               AND EXISTS (
                   SELECT 1
                   FROM title_recommendation_cards c
                   WHERE c.discovery_title_id = title_more_like_this_items.discovery_title_id
                     AND c.payload_blob IS NULL
               )
               AND {}
             ORDER BY sort_index ASC,
                      COALESCE(rank_score, 0) DESC,
                      title_more_like_this_items.discovery_title_id ASC
             LIMIT {{}}",
            title_more_like_this_projection(datastore),
            displayable_discovery_title_clause(datastore, "t")
        ),
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::I64(limit.clamp(0, 100)),
        ],
    )
    .await?;
    let mut items = rows
        .iter()
        .map(item_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    let title_ids = discovery_title_ids_from_rows(&rows)?;
    hydrate_discovery_title_children(datastore, &mut items, &title_ids).await?;
    Ok(items)
}

async fn backfill_title_recommendation_cards(datastore: &StoreDatastore) -> AppResult<()> {
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        &format!(
            "SELECT {}
             FROM title_more_like_this_items
             JOIN discovery_titles t
               ON t.id = title_more_like_this_items.discovery_title_id
             WHERE EXISTS (
                 SELECT 1
                 FROM title_recommendation_cards c
                 WHERE c.discovery_title_id = title_more_like_this_items.discovery_title_id
                   AND c.payload_blob IS NULL
             )
             ORDER BY title_more_like_this_items.discovery_title_id ASC,
                      source_title_id ASC",
            title_more_like_this_projection(datastore)
        ),
        &[],
    )
    .await?;
    if rows.is_empty() {
        return Ok(());
    }

    let mut items = rows
        .iter()
        .map(item_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    let title_ids = discovery_title_ids_from_rows(&rows)?;
    hydrate_discovery_title_children(datastore, &mut items, &title_ids).await?;

    let mut seen = HashSet::new();
    let cards = title_ids
        .into_iter()
        .zip(items)
        .filter(|(discovery_title_id, _)| seen.insert(discovery_title_id.clone()))
        .collect::<Vec<_>>();
    let cards = std::sync::Arc::new(cards);
    SqlRuntime::run_in_transaction(
        datastore,
        "backfill_title_recommendation_cards",
        move |tx| {
            let cards = std::sync::Arc::clone(&cards);
            Box::pin(async move {
                for (discovery_title_id, item) in cards.iter() {
                    upsert_title_recommendation_card_tx(tx, discovery_title_id, item).await?;
                }
                Ok(())
            })
        },
    )
    .await
}

async fn enrich_recommendation_items_from_normalized(
    datastore: &StoreDatastore,
    language: &str,
    items: &mut [DiscoveryItemRecord],
) -> AppResult<()> {
    if items.is_empty() {
        return Ok(());
    }
    let requested_title_ids = items
        .iter()
        .map(|item| {
            discovery_title_id_for(
                &discovery_title_target_key_norm(item),
                &normalize_discovery_language(language),
            )
        })
        .collect::<Vec<_>>();
    let rows = fetch_child_rows(
        datastore,
        &format!(
            "SELECT {}
             FROM discovery_items i
             JOIN discovery_titles t ON t.id = i.discovery_title_id
             WHERE i.discovery_title_id IN ({{}})
             ORDER BY i.updated_at DESC, i.id ASC",
            discovery_item_projection(datastore, "i", "t")
        ),
        &requested_title_ids,
    )
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut normalized_items = rows
        .iter()
        .map(item_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    let normalized_title_ids = discovery_title_ids_from_rows(&rows)?;
    hydrate_discovery_title_children(datastore, &mut normalized_items, &normalized_title_ids)
        .await?;
    let mut normalized_by_title_id = HashMap::new();
    for (discovery_title_id, item) in normalized_title_ids.into_iter().zip(normalized_items) {
        normalized_by_title_id
            .entry(discovery_title_id)
            .or_insert(item);
    }
    for (item, discovery_title_id) in items.iter_mut().zip(requested_title_ids) {
        if let Some(normalized) = normalized_by_title_id.get(&discovery_title_id) {
            merge_recommendation_item_from_normalized(item, normalized);
        }
    }
    Ok(())
}

fn merge_recommendation_item_from_normalized(
    item: &mut DiscoveryItemRecord,
    normalized: &DiscoveryItemRecord,
) {
    if !useful_recommendation_title(&item.display_title) {
        item.display_title.clone_from(&normalized.display_title);
    }
    if item.target_kind.trim().is_empty() {
        item.target_kind.clone_from(&normalized.target_kind);
    }
    item.resolved |= normalized.resolved;
    fill_missing(&mut item.resolved_title_id, &normalized.resolved_title_id);
    fill_missing(&mut item.original_title, &normalized.original_title);
    fill_missing(&mut item.sort_title, &normalized.sort_title);
    fill_missing(&mut item.year, &normalized.year);
    fill_missing(&mut item.poster_path, &normalized.poster_path);
    fill_missing(&mut item.poster_url, &normalized.poster_url);
    fill_missing(&mut item.background_url, &normalized.background_url);
    fill_missing(&mut item.overview, &normalized.overview);
    fill_missing(&mut item.content_type, &normalized.content_type);
    fill_missing(&mut item.rating, &normalized.rating);
    fill_missing(&mut item.tmdb_collection_id, &normalized.tmdb_collection_id);
    fill_missing(
        &mut item.tmdb_collection_name,
        &normalized.tmdb_collection_name,
    );
    fill_missing(&mut item.studio_slug, &normalized.studio_slug);
    item.is_adult |= normalized.is_adult;
    fill_empty(&mut item.canonical_tags, &normalized.canonical_tags);
    fill_empty(&mut item.content_ratings, &normalized.content_ratings);
    fill_empty(&mut item.rating_sources, &normalized.rating_sources);
    fill_empty(&mut item.external_ratings, &normalized.external_ratings);
    fill_empty(&mut item.external_ids, &normalized.external_ids);
    fill_empty(&mut item.status_tags, &normalized.status_tags);
    fill_empty(&mut item.source_tags, &normalized.source_tags);
    fill_empty(&mut item.sources, &normalized.sources);
    fill_empty(&mut item.relation_types, &normalized.relation_types);
    fill_empty(&mut item.relation_subtypes, &normalized.relation_subtypes);
    fill_empty(&mut item.chart_signals, &normalized.chart_signals);
    fill_empty(&mut item.provider_signals, &normalized.provider_signals);
    fill_empty(&mut item.person_ids, &normalized.person_ids);
    fill_empty(&mut item.facet_terms, &normalized.facet_terms);
    fill_empty(&mut item.context_terms, &normalized.context_terms);
}

fn fill_missing<T: Clone>(target: &mut Option<T>, source: &Option<T>) {
    if target.is_none() {
        target.clone_from(source);
    }
}

fn fill_empty<T: Clone>(target: &mut Vec<T>, source: &[T]) {
    if target.is_empty() {
        target.extend_from_slice(source);
    }
}

fn recommendation_item_from_row(row: &SqlRow) -> AppResult<Option<DiscoveryItemRecord>> {
    let payload_version = row.i32("payload_version")?;
    if payload_version != TITLE_RECOMMENDATION_PAYLOAD_VERSION {
        tracing::warn!(
            discovery_title_id = %row.text("discovery_title_id")?,
            payload_version,
            "skipping recommendation card with unsupported payload version"
        );
        return Ok(None);
    }
    let Some(payload_blob) = row.opt_bytes("payload_blob")? else {
        return Ok(None);
    };
    let mut item = match decode_compressed_json::<DiscoveryItemRecord>(&payload_blob) {
        Ok(item) => item,
        Err(error) => {
            tracing::warn!(
                discovery_title_id = %row.text("discovery_title_id")?,
                error = %error,
                "skipping invalid recommendation card payload"
            );
            return Ok(None);
        }
    };
    let source_title_id = row.text("source_title_id")?;
    let discovery_title_id = row.text("discovery_title_id")?;
    item.id = format!("{source_title_id}:more-like-this:{discovery_title_id}");
    item.run_id = source_title_id;
    item.base_generation_id = None;
    item.source_run_kind = "title_more_like_this".to_string();
    item.section_id = None;
    item.sort_index = row.i32("sort_index")?;
    item.rank_score = row.opt_f64("rank_score")?;
    item.best_source = row.opt_text("best_source")?;
    item.source_count = row.opt_i32("source_count")?;
    item.edge_count = row.opt_i32("edge_count")?;
    item.relation_count = row.opt_i32("relation_count")?;
    item.source_subject_count = row.opt_i32("source_subject_count")?;
    item.matched_subject_count = 0;
    item.matched_subject_keys.clear();
    item.matched_subject_titles.clear();
    item.library_provenance.clear();
    item.owned_in_input = false;
    item.tombstoned_by_run_id = None;
    item.tombstoned_at = None;
    item.created_at = row.timestamp("created_at")?;
    item.updated_at = row.timestamp("updated_at")?;
    Ok(Some(item))
}

fn recommendation_item_is_displayable(item: &DiscoveryItemRecord) -> bool {
    !item
        .poster_url
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
        && [
            Some(item.display_title.as_str()),
            item.sort_title.as_deref(),
            item.original_title.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(useful_recommendation_title)
}

fn useful_recommendation_title(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.chars().all(|character| character.is_ascii_digit())
        && !recommendation_title_is_source_identifier(value)
}

fn recommendation_title_is_source_identifier(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    let mut parts = normalized.split(':');
    let Some(source) = parts.next() else {
        return false;
    };
    let Some(kind) = parts.next() else {
        return false;
    };
    if parts.next().is_none()
        || !source.starts_with(|character: char| character.is_ascii_alphabetic())
    {
        return false;
    }
    let allowed = |character: char| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '_' | '+' | '-')
    };
    source.chars().all(allowed) && !kind.is_empty() && kind.chars().all(allowed)
}

fn storage_text(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn normalize_discovery_language(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        "und".to_string()
    } else {
        value
    }
}

fn discovery_title_target_key_norm(item: &DiscoveryItemRecord) -> String {
    let target_key = item.target_key.trim().to_ascii_lowercase();
    if target_key.is_empty() {
        format!(
            "__scryer_occurrence:{}",
            item.id.trim().to_ascii_lowercase()
        )
    } else {
        target_key
    }
}

fn discovery_title_id_for(target_key_norm: &str, language: &str) -> String {
    let digest = blake3::hash(format!("{language}\0{target_key_norm}").as_bytes());
    format!("discovery-title:{}", digest.to_hex())
}

fn empty_to_none(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn discovery_item_authoritative_media_kind(item: &DiscoveryItemRecord) -> Option<String> {
    normalized_discovery_media_kind(item.content_type.as_deref())
        .or_else(|| normalized_discovery_media_kind(Some(item.target_kind.as_str())))
        .map(str::to_string)
}

fn normalized_discovery_media_kind(value: Option<&str>) -> Option<&'static str> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "anime" => Some("anime"),
        "movie" => Some("movie"),
        "series" => Some("series"),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum PersonalizedItemSubset {
    CompleteCollection,
}

struct DiscoveryItemsSql {
    cte_sql: String,
    args: Vec<SqlArg>,
}

async fn fetch_public_section_item_rows(
    datastore: &StoreDatastore,
    run_id: &str,
    allowed_media_kinds: &[String],
    include_unresolved: bool,
    filters: &DiscoveryHomeFilters,
    limit_per_section: i64,
) -> AppResult<Vec<SqlRow>> {
    if allowed_media_kinds.is_empty() {
        return Ok(Vec::new());
    }
    let resolved_clause = if include_unresolved {
        ""
    } else {
        " AND t.resolved = TRUE"
    };
    let mut args = vec![
        SqlArg::Text(run_id.to_string()),
        SqlArg::Text(run_id.to_string()),
    ];
    let mut media_kind_clauses = Vec::new();
    append_authoritative_media_kind_filter(
        &mut media_kind_clauses,
        &mut args,
        "t",
        allowed_media_kinds,
    );
    let mut home_filter_clauses = Vec::new();
    append_discovery_home_filters(&mut home_filter_clauses, &mut args, filters);
    args.push(SqlArg::I64(limit_per_section));
    SqlRuntime::fetch_all(
        datastore.read_exec(),
        &format!(
            "WITH candidates AS (
                SELECT {}, si.section_id AS result_section_id, si.sort_index AS section_sort_index,
                       ROW_NUMBER() OVER (
                           PARTITION BY si.section_id,
                                        CASE WHEN TRIM(t.target_key) = '' THEN i.id ELSE t.target_key END
                           ORDER BY si.sort_index ASC, i.id ASC
                       ) AS identity_rank
                FROM discovery_section_items si
                JOIN discovery_sections s
                  ON s.run_id = si.run_id
                 AND s.section_id = si.section_id
                JOIN discovery_items i
                  ON i.id = si.item_id
                JOIN discovery_titles t
                  ON t.id = i.discovery_title_id
                WHERE si.run_id = {{}}
                  AND i.base_generation_id = {{}}
                  AND i.tombstoned_at IS NULL
                  AND i.owned_in_input = FALSE
                  AND s.surface = 'public'
                  AND UPPER(TRIM(s.section_type)) <> 'COMPLETE_THE_COLLECTION'
                  AND {}
                  AND {}
                  AND {}
                  {resolved_clause}
             ),
             deduped AS (
                SELECT * FROM candidates WHERE identity_rank = 1
             ),
             ranked AS (
                SELECT *,
                       ROW_NUMBER() OVER (
                           PARTITION BY result_section_id
                           ORDER BY section_sort_index ASC, id ASC
                       ) AS section_rank,
                       COUNT(*) OVER (PARTITION BY result_section_id) AS section_total_count
                FROM deduped
             )
             SELECT {}, result_section_id, section_total_count
             FROM ranked
             WHERE section_rank <= {{}}
            ORDER BY result_section_id ASC, section_rank ASC",
            discovery_home_candidate_projection(datastore, "i", "t"),
            displayable_discovery_title_clause(datastore, "t"),
            media_kind_clauses.join(" AND "),
            home_filter_clauses.join(" AND "),
            discovery_home_candidate_row_columns()
        ),
        &args,
    )
    .await
}

async fn section_candidates_from_rows(
    datastore: &StoreDatastore,
    sections: Vec<DiscoverySectionRecord>,
    rows: Vec<SqlRow>,
) -> AppResult<Vec<DiscoveryHomeSectionCandidatesRecord>> {
    let mut item_metadata = Vec::new();
    let mut candidates = Vec::new();
    for row in &rows {
        item_metadata.push((
            row.text("result_section_id")?,
            row.i64("section_total_count")?,
        ));
        candidates.push(home_candidate_from_row(row)?);
    }
    hydrate_discovery_home_candidate_ratings(datastore, &mut candidates).await?;

    let mut items_by_section = HashMap::<String, Vec<DiscoveryHomeCandidate>>::new();
    let mut totals_by_section = HashMap::<String, i64>::new();
    for (candidate, (section_id, total_count)) in candidates.into_iter().zip(item_metadata) {
        totals_by_section
            .entry(section_id.clone())
            .or_insert(total_count);
        items_by_section
            .entry(section_id)
            .or_default()
            .push(candidate);
    }

    Ok(sections
        .into_iter()
        .filter(|section| !discovery_section_type_is_complete(&section.section_type))
        .filter_map(|section| {
            let items = items_by_section.remove(&section.section_id)?;
            Some(DiscoveryHomeSectionCandidatesRecord {
                total_count: totals_by_section
                    .remove(&section.section_id)
                    .unwrap_or(items.len() as i64),
                section,
                items,
            })
        })
        .collect())
}

fn discovery_section_type_is_complete(section_type: &str) -> bool {
    section_type
        .trim()
        .eq_ignore_ascii_case("COMPLETE_THE_COLLECTION")
}

#[allow(clippy::too_many_arguments)]
async fn fetch_personalized_home_candidates(
    datastore: &StoreDatastore,
    run_id: &str,
    readable_library_ids: &[String],
    allowed_media_kinds: &[String],
    include_unresolved: bool,
    filters: &DiscoveryHomeFilters,
    subset: Option<PersonalizedItemSubset>,
    limit: i64,
) -> AppResult<Vec<DiscoveryHomeCandidate>> {
    if readable_library_ids.is_empty() || allowed_media_kinds.is_empty() {
        return Ok(Vec::new());
    }

    let mut args = vec![SqlArg::Text(run_id.to_string())];
    let mut clauses = vec![
        "i.base_generation_id = {}".to_string(),
        "i.tombstoned_at IS NULL".to_string(),
        "i.owned_in_input = FALSE".to_string(),
        displayable_discovery_title_clause(datastore, "t"),
    ];
    if !include_unresolved {
        clauses.push("t.resolved = TRUE".to_string());
    }
    append_authoritative_media_kind_filter(&mut clauses, &mut args, "t", allowed_media_kinds);
    clauses.push(library_provenance_exists_clause(
        "i",
        readable_library_ids,
        &mut args,
    ));
    append_discovery_home_filters(&mut clauses, &mut args, filters);
    if matches!(subset, Some(PersonalizedItemSubset::CompleteCollection)) {
        clauses.push(authoritative_media_kind_clause("t", "movie"));
        clauses.push(collection_signal_clause("i", "t"));
    }
    args.push(SqlArg::I64(limit));

    let sql = format!(
        "SELECT {}
         FROM discovery_items i
         JOIN discovery_titles t
           ON t.id = i.discovery_title_id
         WHERE {}
         ORDER BY COALESCE(i.rank_score, -999999999.0) DESC,
                  COALESCE(t.sort_title, t.display_title) ASC,
                  t.target_key ASC
         LIMIT {{}}",
        discovery_home_candidate_projection(datastore, "i", "t"),
        clauses.join(" AND ")
    );
    fetch_discovery_home_candidates_with_sql(datastore, &sql, &args).await
}

#[allow(clippy::too_many_arguments)]
async fn fetch_discovery_home_top_rated_candidates(
    datastore: &StoreDatastore,
    public_run_id: Option<&str>,
    context_run_id: Option<&str>,
    readable_library_ids: &[String],
    allowed_media_kinds: &[String],
    owned_library_ids: &[String],
    excluded_identity_keys: &[String],
    include_unresolved: bool,
    filters: &DiscoveryHomeFilters,
    limit: i64,
) -> AppResult<Vec<DiscoveryHomeCandidate>> {
    if allowed_media_kinds.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = Vec::new();
    let mut branches = Vec::new();

    if let Some(run_id) = public_run_id {
        let mut clauses = vec![
            "si.run_id = {}".to_string(),
            "i.base_generation_id = {}".to_string(),
            "i.tombstoned_at IS NULL".to_string(),
            "i.owned_in_input = FALSE".to_string(),
            "s.surface = 'public'".to_string(),
            "UPPER(TRIM(s.section_type)) <> 'COMPLETE_THE_COLLECTION'".to_string(),
            displayable_discovery_title_clause(datastore, "t"),
        ];
        args.push(SqlArg::Text(run_id.to_string()));
        args.push(SqlArg::Text(run_id.to_string()));
        append_authoritative_media_kind_filter(&mut clauses, &mut args, "t", allowed_media_kinds);
        if !include_unresolved {
            clauses.push("t.resolved = TRUE".to_string());
        }
        if !owned_library_ids.is_empty() {
            let placeholders = placeholders(owned_library_ids.len());
            args.extend(owned_library_ids.iter().cloned().map(SqlArg::Text));
            clauses.push(format!(
                "NOT EXISTS (
                    SELECT 1
                    FROM titles owned
                    WHERE owned.id = t.resolved_title_id
                      AND owned.library_id IN ({placeholders})
                 )"
            ));
        }
        if !excluded_identity_keys.is_empty() {
            let placeholders = placeholders(excluded_identity_keys.len());
            args.extend(excluded_identity_keys.iter().cloned().map(SqlArg::Text));
            clauses.push(format!(
                "CASE WHEN TRIM(t.target_key) = '' THEN LOWER(i.id) ELSE LOWER(TRIM(t.target_key)) END NOT IN ({placeholders})"
            ));
        }
        append_discovery_home_filters(&mut clauses, &mut args, filters);
        branches.push(format!(
            "SELECT {},
                    CASE WHEN TRIM(t.target_key) = '' THEN LOWER(i.id) ELSE LOWER(TRIM(t.target_key)) END AS identity_key,
                    (
                        SELECT MAX(CASE WHEN r.normalized <= 1.0 THEN r.normalized * 10.0 ELSE r.normalized END)
                        FROM discovery_title_metadata_external_ratings r
                        WHERE r.discovery_title_id = t.id
                          AND r.normalized IS NOT NULL
                          AND r.normalized > 0
                    ) AS external_rating_score,
                    (
                        SELECT MAX(COALESCE(r.votes, 0))
                        FROM discovery_title_metadata_external_ratings r
                        WHERE r.discovery_title_id = t.id
                          AND r.normalized IS NOT NULL
                          AND r.normalized > 0
                    ) AS external_rating_votes
             FROM discovery_section_items si
             JOIN discovery_sections s
               ON s.run_id = si.run_id
              AND s.section_id = si.section_id
             JOIN discovery_items i
               ON i.id = si.item_id
             JOIN discovery_titles t
               ON t.id = i.discovery_title_id
             WHERE {}",
            discovery_home_candidate_projection(datastore, "i", "t"),
            clauses.join(" AND ")
        ));
    }

    if let Some(run_id) = context_run_id.filter(|_| !readable_library_ids.is_empty()) {
        let mut clauses = vec![
            "i.base_generation_id = {}".to_string(),
            "i.tombstoned_at IS NULL".to_string(),
            "i.owned_in_input = FALSE".to_string(),
            displayable_discovery_title_clause(datastore, "t"),
        ];
        args.push(SqlArg::Text(run_id.to_string()));
        append_authoritative_media_kind_filter(&mut clauses, &mut args, "t", allowed_media_kinds);
        if !include_unresolved {
            clauses.push("t.resolved = TRUE".to_string());
        }
        clauses.push(library_provenance_exists_clause(
            "i",
            readable_library_ids,
            &mut args,
        ));
        if !owned_library_ids.is_empty() {
            let placeholders = placeholders(owned_library_ids.len());
            args.extend(owned_library_ids.iter().cloned().map(SqlArg::Text));
            clauses.push(format!(
                "NOT EXISTS (
                    SELECT 1
                    FROM titles owned
                    WHERE owned.id = t.resolved_title_id
                      AND owned.library_id IN ({placeholders})
                 )"
            ));
        }
        if !excluded_identity_keys.is_empty() {
            let placeholders = placeholders(excluded_identity_keys.len());
            args.extend(excluded_identity_keys.iter().cloned().map(SqlArg::Text));
            clauses.push(format!(
                "CASE WHEN TRIM(t.target_key) = '' THEN LOWER(i.id) ELSE LOWER(TRIM(t.target_key)) END NOT IN ({placeholders})"
            ));
        }
        append_discovery_home_filters(&mut clauses, &mut args, filters);
        branches.push(format!(
            "SELECT {},
                    CASE WHEN TRIM(t.target_key) = '' THEN LOWER(i.id) ELSE LOWER(TRIM(t.target_key)) END AS identity_key,
                    (
                        SELECT MAX(CASE WHEN r.normalized <= 1.0 THEN r.normalized * 10.0 ELSE r.normalized END)
                        FROM discovery_title_metadata_external_ratings r
                        WHERE r.discovery_title_id = t.id
                          AND r.normalized IS NOT NULL
                          AND r.normalized > 0
                    ) AS external_rating_score,
                    (
                        SELECT MAX(COALESCE(r.votes, 0))
                        FROM discovery_title_metadata_external_ratings r
                        WHERE r.discovery_title_id = t.id
                          AND r.normalized IS NOT NULL
                          AND r.normalized > 0
                    ) AS external_rating_votes
             FROM discovery_items i
             JOIN discovery_titles t
               ON t.id = i.discovery_title_id
             WHERE {}",
            discovery_home_candidate_projection(datastore, "i", "t"),
            clauses.join(" AND ")
        ));
    }

    if branches.is_empty() {
        return Ok(Vec::new());
    }

    args.push(SqlArg::I64(limit));
    let sql = format!(
        "WITH candidates AS (
            {}
         ),
         ranked AS (
            SELECT *,
                   COALESCE(
                       external_rating_score,
                       CASE
                           WHEN rating IS NULL THEN NULL
                           WHEN rating <= 1.0 THEN rating * 10.0
                           ELSE rating
                       END,
                       -1.0
                   ) AS effective_rating,
                   ROW_NUMBER() OVER (
                       PARTITION BY identity_key
                       ORDER BY
                           CASE WHEN external_rating_score IS NULL THEN 0 ELSE 1 END DESC,
                           COALESCE(external_rating_score, -1.0) DESC,
                           COALESCE(external_rating_votes, 0) DESC,
                           CASE
                               WHEN rating IS NULL THEN -1.0
                               WHEN rating <= 1.0 THEN rating * 10.0
                               ELSE rating
                           END DESC,
                           COALESCE(rank_score, -999999999.0) DESC,
                           source_count DESC,
                           target_key ASC,
                           id ASC
                   ) AS identity_rank
            FROM candidates
         )
         SELECT {}
         FROM ranked
         WHERE identity_rank = 1
         ORDER BY
             CASE WHEN external_rating_score IS NULL THEN 0 ELSE 1 END DESC,
             effective_rating DESC,
             COALESCE(external_rating_votes, 0) DESC,
             COALESCE(rank_score, -999999999.0) DESC,
             source_count DESC,
             target_key ASC,
             id ASC
         LIMIT {{}}",
        branches.join("\nUNION ALL\n"),
        discovery_home_candidate_row_columns()
    );
    fetch_discovery_home_rating_candidates_with_sql(datastore, &sql, &args).await
}

async fn fetch_discovery_home_filter_options(
    datastore: &StoreDatastore,
    public_run_id: Option<&str>,
    context_run_id: Option<&str>,
    readable_library_ids: &[String],
    allowed_media_kinds: &[String],
    include_unresolved: bool,
) -> AppResult<DiscoveryHomeFilterOptions> {
    if allowed_media_kinds.is_empty() {
        return Ok(DiscoveryHomeFilterOptions::default());
    }
    let mut args = Vec::new();
    let mut branches = Vec::new();
    if let Some(run_id) = public_run_id {
        let mut clauses = vec![
            "i.base_generation_id = {}".to_string(),
            "i.tombstoned_at IS NULL".to_string(),
            "i.owned_in_input = FALSE".to_string(),
            displayable_discovery_title_clause(datastore, "t"),
            "EXISTS (
                SELECT 1
                FROM discovery_section_items si
                JOIN discovery_sections s
                  ON s.run_id = si.run_id
                 AND s.section_id = si.section_id
                WHERE si.run_id = {}
                  AND si.item_id = i.id
                  AND s.surface = 'public'
                  AND UPPER(TRIM(s.section_type)) <> 'COMPLETE_THE_COLLECTION'
             )"
            .to_string(),
        ];
        args.push(SqlArg::Text(run_id.to_string()));
        args.push(SqlArg::Text(run_id.to_string()));
        if !include_unresolved {
            clauses.push("t.resolved = TRUE".to_string());
        }
        append_authoritative_media_kind_filter(&mut clauses, &mut args, "t", allowed_media_kinds);
        branches.push(format!(
            "SELECT DISTINCT i.discovery_title_id
             FROM discovery_items i
             JOIN discovery_titles t
               ON t.id = i.discovery_title_id
             WHERE {}",
            clauses.join(" AND ")
        ));
    }
    if let Some(run_id) = context_run_id.filter(|_| !readable_library_ids.is_empty()) {
        let mut clauses = vec![
            "i.base_generation_id = {}".to_string(),
            "i.tombstoned_at IS NULL".to_string(),
            "i.owned_in_input = FALSE".to_string(),
            displayable_discovery_title_clause(datastore, "t"),
        ];
        args.push(SqlArg::Text(run_id.to_string()));
        if !include_unresolved {
            clauses.push("t.resolved = TRUE".to_string());
        }
        append_authoritative_media_kind_filter(&mut clauses, &mut args, "t", allowed_media_kinds);
        clauses.push(library_provenance_exists_clause(
            "i",
            readable_library_ids,
            &mut args,
        ));
        branches.push(format!(
            "SELECT DISTINCT i.discovery_title_id
             FROM discovery_items i
             JOIN discovery_titles t
               ON t.id = i.discovery_title_id
             WHERE {}",
            clauses.join(" AND ")
        ));
    }
    if branches.is_empty() {
        return Ok(DiscoveryHomeFilterOptions::default());
    }
    let sql = format!(
        "WITH entitled_titles AS (
            {}
         ),
         options AS (
            SELECT 'genre' AS option_kind, tag.tag_key AS option_key, tag.name AS option_name
            FROM discovery_title_metadata_tags tag
            JOIN entitled_titles entitled
              ON entitled.discovery_title_id = tag.discovery_title_id
            WHERE LOWER(tag.category) = 'genre'
              AND TRIM(tag.tag_key) <> ''
              AND TRIM(tag.name) <> ''
            UNION ALL
            SELECT 'theme' AS option_kind, tag.tag_key AS option_key, tag.name AS option_name
            FROM discovery_title_metadata_tags tag
            JOIN entitled_titles entitled
              ON entitled.discovery_title_id = tag.discovery_title_id
            WHERE LOWER(tag.category) = 'theme'
              AND TRIM(tag.tag_key) <> ''
              AND TRIM(tag.name) <> ''
            UNION ALL
            SELECT 'studio' AS option_kind, term.term_value AS option_key, term.term_value AS option_name
            FROM discovery_title_terms term
            JOIN entitled_titles entitled
              ON entitled.discovery_title_id = term.discovery_title_id
            WHERE term.term_kind = 'studio'
              AND TRIM(term.term_value) <> ''
         ),
         deduped_options AS (
            SELECT option_kind,
                   MIN(option_key) AS option_key,
                   MIN(option_name) AS option_name
            FROM options
            GROUP BY option_kind,
                     CASE WHEN option_kind = 'studio' THEN LOWER(option_key) ELSE option_key END
         )
         SELECT option_kind, option_key, option_name
         FROM deduped_options
         ORDER BY option_kind ASC, LOWER(option_name) ASC, option_name ASC, option_key ASC",
        branches.join("\nUNION\n")
    );
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args).await?;
    let mut options = DiscoveryHomeFilterOptions::default();
    for row in rows {
        match row.text("option_kind")?.as_str() {
            "genre" => options.genres.push(DiscoveryCanonicalTagFilterOption {
                key: row.text("option_key")?,
                name: row.text("option_name")?,
            }),
            "theme" => options.themes.push(DiscoveryCanonicalTagFilterOption {
                key: row.text("option_key")?,
                name: row.text("option_name")?,
            }),
            "studio" => options.studio_slugs.push(row.text("option_name")?),
            _ => {}
        }
    }
    Ok(options)
}

async fn fetch_catalog_public_items(
    datastore: &StoreDatastore,
    run_id: &str,
    owned_library_ids: &[String],
    excluded_identity_keys: &[String],
    media_kind: &str,
    include_unresolved: bool,
    limit: i64,
) -> AppResult<CatalogDiscoveryCandidatesRecord> {
    let resolved_clause = if include_unresolved {
        ""
    } else {
        " AND t.resolved = TRUE"
    };
    let mut args = vec![
        SqlArg::Text(run_id.to_string()),
        SqlArg::Text(run_id.to_string()),
    ];
    let owned_clause = if owned_library_ids.is_empty() {
        String::new()
    } else {
        let placeholders = placeholders(owned_library_ids.len());
        args.extend(owned_library_ids.iter().cloned().map(SqlArg::Text));
        format!(
            " AND NOT EXISTS (
                SELECT 1
                FROM titles owned
                WHERE owned.id = t.resolved_title_id
                  AND owned.library_id IN ({placeholders})
             )"
        )
    };
    let excluded_identity_clause = if excluded_identity_keys.is_empty() {
        String::new()
    } else {
        let placeholders = placeholders(excluded_identity_keys.len());
        args.extend(excluded_identity_keys.iter().cloned().map(SqlArg::Text));
        format!(
            " AND CASE WHEN TRIM(t.target_key) = '' THEN LOWER(i.id) ELSE LOWER(TRIM(t.target_key)) END NOT IN ({placeholders})"
        )
    };
    let sql = format!(
        "WITH candidates AS (
            SELECT {}, s.sort_index AS section_sort_index, si.sort_index AS section_item_sort_index,
                   ROW_NUMBER() OVER (
                       PARTITION BY CASE WHEN TRIM(t.target_key) = '' THEN i.id ELSE t.target_key END
                       ORDER BY s.sort_index ASC, si.sort_index ASC, i.id ASC
                   ) AS identity_rank
            FROM discovery_section_items si
            JOIN discovery_sections s
              ON s.run_id = si.run_id
             AND s.section_id = si.section_id
            JOIN discovery_items i
              ON i.id = si.item_id
            JOIN discovery_titles t
              ON t.id = i.discovery_title_id
            WHERE si.run_id = {{}}
              AND i.base_generation_id = {{}}
              AND i.tombstoned_at IS NULL
              AND i.owned_in_input = FALSE
              AND s.surface = 'public'
              AND UPPER(TRIM(s.section_type)) <> 'COMPLETE_THE_COLLECTION'
              AND {}
              AND {}
              {owned_clause}
              {excluded_identity_clause}
              {resolved_clause}
         ),
         deduped AS (
            SELECT * FROM candidates WHERE identity_rank = 1
         ),
         ranked AS (
            SELECT *,
                   COUNT(*) OVER () AS total_count
            FROM deduped
         )
         SELECT {}, total_count
         FROM ranked
        ORDER BY section_sort_index ASC, section_item_sort_index ASC, id ASC
        LIMIT {{}}",
        discovery_item_projection(datastore, "i", "t"),
        authoritative_media_kind_clause("t", media_kind),
        displayable_discovery_title_clause(datastore, "t"),
        discovery_item_row_columns()
    );
    args.push(SqlArg::I64(limit));
    fetch_catalog_candidates_with_sql(datastore, &sql, &args).await
}

async fn fetch_catalog_public_sections(
    datastore: &StoreDatastore,
    run_id: &str,
    owned_library_ids: &[String],
    excluded_identity_keys: &[String],
    media_kind: &str,
    include_unresolved: bool,
    limit_per_section: i64,
) -> AppResult<Vec<CatalogDiscoverySectionCandidatesRecord>> {
    let resolved_clause = if include_unresolved {
        ""
    } else {
        " AND t.resolved = TRUE"
    };
    let mut args = vec![
        SqlArg::Text(run_id.to_string()),
        SqlArg::Text(run_id.to_string()),
    ];
    let owned_clause = if owned_library_ids.is_empty() {
        String::new()
    } else {
        let placeholders = placeholders(owned_library_ids.len());
        args.extend(owned_library_ids.iter().cloned().map(SqlArg::Text));
        format!(
            " AND NOT EXISTS (
                SELECT 1
                FROM titles owned
                WHERE owned.id = t.resolved_title_id
                  AND owned.library_id IN ({placeholders})
             )"
        )
    };
    let excluded_identity_clause = if excluded_identity_keys.is_empty() {
        String::new()
    } else {
        let placeholders = placeholders(excluded_identity_keys.len());
        args.extend(excluded_identity_keys.iter().cloned().map(SqlArg::Text));
        format!(
            " AND CASE WHEN TRIM(t.target_key) = '' THEN LOWER(i.id) ELSE LOWER(TRIM(t.target_key)) END NOT IN ({placeholders})"
        )
    };
    let sql = format!(
        "WITH candidates AS (
            SELECT {}, s.section_id AS result_section_id,
                   s.section_type AS result_section_type,
                   s.title AS result_section_title,
                   s.sort_index AS section_sort_index,
                   si.sort_index AS section_item_sort_index,
                   ROW_NUMBER() OVER (
                       PARTITION BY s.section_id,
                                    CASE WHEN TRIM(t.target_key) = '' THEN i.id ELSE t.target_key END
                       ORDER BY si.sort_index ASC, i.id ASC
                   ) AS identity_rank
            FROM discovery_section_items si
            JOIN discovery_sections s
              ON s.run_id = si.run_id
             AND s.section_id = si.section_id
            JOIN discovery_items i
              ON i.id = si.item_id
            JOIN discovery_titles t
              ON t.id = i.discovery_title_id
            WHERE si.run_id = {{}}
              AND i.base_generation_id = {{}}
              AND i.tombstoned_at IS NULL
              AND i.owned_in_input = FALSE
              AND s.surface = 'public'
              AND UPPER(TRIM(s.section_type)) <> 'COMPLETE_THE_COLLECTION'
              AND {}
              AND {}
              {owned_clause}
              {excluded_identity_clause}
              {resolved_clause}
         ),
         deduped AS (
            SELECT * FROM candidates WHERE identity_rank = 1
         ),
         ranked AS (
            SELECT *,
                   ROW_NUMBER() OVER (
                       PARTITION BY result_section_id
                       ORDER BY section_sort_index ASC, section_item_sort_index ASC, id ASC
                   ) AS section_rank,
                   COUNT(*) OVER (PARTITION BY result_section_id) AS section_total_count
            FROM deduped
         )
         SELECT {}, result_section_id, result_section_type, result_section_title, section_total_count
         FROM ranked
         WHERE section_rank <= {{}}
        ORDER BY section_sort_index ASC, section_rank ASC, id ASC",
        discovery_item_projection(datastore, "i", "t"),
        authoritative_media_kind_clause("t", media_kind),
        displayable_discovery_title_clause(datastore, "t"),
        discovery_item_row_columns()
    );
    args.push(SqlArg::I64(limit_per_section));
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args).await?;
    let mut item_metadata = Vec::new();
    let mut items = Vec::new();
    for row in &rows {
        item_metadata.push((
            row.text("result_section_id")?,
            row.text("result_section_type")?,
            row.opt_text("result_section_title")?,
            row.i64("section_total_count")?,
        ));
        items.push(item_from_row(row)?);
    }
    let title_ids = discovery_title_ids_from_rows(&rows)?;
    hydrate_discovery_items(datastore, &mut items, &title_ids).await?;

    let mut sections = Vec::<CatalogDiscoverySectionCandidatesRecord>::new();
    for (item, (section_id, section_type, title, total_count)) in
        items.into_iter().zip(item_metadata)
    {
        if let Some(section) = sections
            .last_mut()
            .filter(|section| section.section_id == section_id)
        {
            section.items.push(item);
        } else {
            sections.push(CatalogDiscoverySectionCandidatesRecord {
                section_id,
                section_type,
                title,
                total_count,
                items: vec![item],
            });
        }
    }
    Ok(sections)
}

async fn fetch_catalog_personalized_items(
    datastore: &StoreDatastore,
    run_id: &str,
    readable_library_ids: &[String],
    media_kind: &str,
    include_unresolved: bool,
    limit: i64,
) -> AppResult<CatalogDiscoveryCandidatesRecord> {
    if readable_library_ids.is_empty() {
        return Ok(CatalogDiscoveryCandidatesRecord {
            total_count: 0,
            items: Vec::new(),
        });
    }

    let mut args = vec![SqlArg::Text(run_id.to_string())];
    let mut clauses = vec![
        "i.base_generation_id = {}".to_string(),
        "i.tombstoned_at IS NULL".to_string(),
        "i.owned_in_input = FALSE".to_string(),
        authoritative_media_kind_clause("t", media_kind),
        displayable_discovery_title_clause(datastore, "t"),
    ];
    if !include_unresolved {
        clauses.push("t.resolved = TRUE".to_string());
    }
    clauses.push(library_provenance_exists_clause(
        "i",
        readable_library_ids,
        &mut args,
    ));
    args.push(SqlArg::I64(limit));

    let sql = format!(
        "WITH candidates AS (
            SELECT {},
                   ROW_NUMBER() OVER (
                       PARTITION BY CASE WHEN TRIM(t.target_key) = '' THEN i.id ELSE t.target_key END
                       ORDER BY COALESCE(i.rank_score, -999999999.0) DESC,
                                COALESCE(t.sort_title, t.display_title) ASC,
                                t.target_key ASC
                   ) AS identity_rank
            FROM discovery_items i
            JOIN discovery_titles t
              ON t.id = i.discovery_title_id
            WHERE {}
         ),
         deduped AS (
            SELECT * FROM candidates WHERE identity_rank = 1
         ),
         ranked AS (
            SELECT *,
                   COUNT(*) OVER () AS total_count
            FROM deduped
         )
         SELECT {}, total_count
         FROM ranked
         ORDER BY COALESCE(rank_score, -999999999.0) DESC,
                  COALESCE(sort_title, display_title) ASC,
                  target_key ASC
        LIMIT {{}}",
        discovery_item_projection(datastore, "i", "t"),
        clauses.join(" AND "),
        discovery_item_row_columns()
    );
    fetch_catalog_candidates_with_sql(datastore, &sql, &args).await
}

async fn fetch_catalog_candidates_with_sql(
    datastore: &StoreDatastore,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<CatalogDiscoveryCandidatesRecord> {
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), sql, args).await?;
    let total_count = rows
        .first()
        .map(|row| row.i64("total_count"))
        .transpose()?
        .unwrap_or_default();
    let mut items = rows
        .iter()
        .map(item_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    let title_ids = discovery_title_ids_from_rows(&rows)?;
    hydrate_discovery_items(datastore, &mut items, &title_ids).await?;
    Ok(CatalogDiscoveryCandidatesRecord { total_count, items })
}

async fn fetch_personalized_facets(
    datastore: &StoreDatastore,
    run_id: &str,
    readable_library_ids: &[String],
    allowed_media_kinds: &[String],
    include_unresolved: bool,
) -> AppResult<Vec<DiscoveryFacetRecord>> {
    if readable_library_ids.is_empty() || allowed_media_kinds.is_empty() {
        return Ok(Vec::new());
    }

    let mut item_args = vec![SqlArg::Text(run_id.to_string())];
    let mut item_clauses = vec![
        "i.base_generation_id = {}".to_string(),
        "i.tombstoned_at IS NULL".to_string(),
        "i.owned_in_input = FALSE".to_string(),
    ];
    item_clauses.push(library_provenance_exists_clause(
        "i",
        readable_library_ids,
        &mut item_args,
    ));

    let mut title_args = Vec::new();
    let mut title_clauses = vec![
        "t.term_kind = 'facet_term'".to_string(),
        "(LOWER(t.term_value) LIKE 'canonical:genre:%'
          OR LOWER(t.term_value) LIKE 'canonical:theme:%')"
            .to_string(),
    ];
    if !include_unresolved {
        title_clauses.push("dt.resolved = TRUE".to_string());
    }
    append_authoritative_media_kind_filter(
        &mut title_clauses,
        &mut title_args,
        "dt",
        allowed_media_kinds,
    );
    let mut args = item_args;
    args.extend(title_args);

    // SQLite otherwise starts with every facet term, then repeatedly probes the
    // eligible item generation. Materializing the small eligible-item set and
    // using CROSS JOIN keeps the join item-driven without affecting Postgres.
    let sql = match datastore {
        StoreDatastore::Sqlite { .. } => format!(
            "WITH eligible_items AS MATERIALIZED (
                 SELECT i.id, i.discovery_title_id
                 FROM discovery_items i
                 WHERE {}
             )
             SELECT t.term_value AS facet_term,
                    COUNT(DISTINCT i.id) AS local_count
             FROM eligible_items i
             CROSS JOIN discovery_titles dt
             CROSS JOIN discovery_title_terms t
             WHERE dt.id = i.discovery_title_id
               AND t.discovery_title_id = dt.id
               AND {}
             GROUP BY t.term_value
             HAVING COUNT(DISTINCT i.id) > 0
             ORDER BY t.term_value ASC",
            item_clauses.join(" AND "),
            title_clauses.join(" AND "),
        ),
        StoreDatastore::Postgres { .. } => {
            let mut clauses = item_clauses;
            clauses.extend(title_clauses);
            format!(
                "SELECT t.term_value AS facet_term,
                        COUNT(DISTINCT i.id) AS local_count
                 FROM discovery_items i
                 JOIN discovery_titles dt
                   ON dt.id = i.discovery_title_id
                 JOIN discovery_title_terms t
                   ON t.discovery_title_id = dt.id
                 WHERE {}
                 GROUP BY t.term_value
                 HAVING COUNT(DISTINCT i.id) > 0
                 ORDER BY t.term_value ASC",
                clauses.join(" AND ")
            )
        }
    };
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args).await?;
    rows.iter()
        .filter_map(|row| canonical_facet_from_row(run_id, row).transpose())
        .collect()
}

async fn query_discovery_items_page(
    datastore: &StoreDatastore,
    query: &DiscoveryItemsStorageQuery,
) -> AppResult<DiscoveryItemsPageRecord> {
    let Some(sql) = build_discovery_items_sql(datastore, query) else {
        return Ok(DiscoveryItemsPageRecord {
            items: Vec::new(),
            total_count: 0,
        });
    };

    let count_sql = format!(
        "{}
         SELECT COUNT(*) AS total_count
         FROM deduped
         WHERE identity_rank = 1",
        sql.cte_sql
    );
    let total_count = SqlRuntime::fetch_optional(datastore.read_exec(), &count_sql, &sql.args)
        .await?
        .map(|row| row.i64("total_count"))
        .transpose()?
        .unwrap_or_default();
    if total_count == 0 {
        return Ok(DiscoveryItemsPageRecord {
            items: Vec::new(),
            total_count,
        });
    }

    let mut page_args = sql.args;
    page_args.push(SqlArg::I64(query.limit as i64));
    page_args.push(SqlArg::I64(query.offset as i64));
    let page_sql = format!(
        "{}
         SELECT {}
         FROM deduped
         WHERE identity_rank = 1
         ORDER BY COALESCE(rank_score, -999999999.0) DESC,
                  COALESCE(sort_title, display_title) ASC,
                  target_key ASC
         LIMIT {{}}
         OFFSET {{}}",
        sql.cte_sql,
        discovery_item_row_columns()
    );
    let items = fetch_items_with_sql(datastore, &page_sql, &page_args).await?;
    Ok(DiscoveryItemsPageRecord { items, total_count })
}

fn build_discovery_items_sql(
    datastore: &StoreDatastore,
    query: &DiscoveryItemsStorageQuery,
) -> Option<DiscoveryItemsSql> {
    if query.allowed_media_kinds.is_empty() {
        return None;
    }

    let mut args = Vec::new();
    let mut sources = Vec::new();
    if let Some(context_run_id) = query.context_run_id.as_deref()
        && !query.readable_library_ids.is_empty()
    {
        let mut source_args = vec![SqlArg::Text(context_run_id.to_string())];
        let provenance_clause =
            library_provenance_exists_clause("i", &query.readable_library_ids, &mut source_args);
        args.extend(source_args);
        sources.push(format!(
            "SELECT {}, 0 AS source_priority
             FROM discovery_items i
             JOIN discovery_titles t
               ON t.id = i.discovery_title_id
             WHERE i.base_generation_id = {{}}
               AND i.tombstoned_at IS NULL
               AND {provenance_clause}",
            discovery_item_projection(datastore, "i", "t")
        ));
    }
    if let Some(public_run_id) = query.public_run_id.as_deref() {
        args.push(SqlArg::Text(public_run_id.to_string()));
        sources.push(format!(
            "SELECT {}, 1 AS source_priority
             FROM discovery_items i
             JOIN discovery_titles t
               ON t.id = i.discovery_title_id
             WHERE i.base_generation_id = {{}}
               AND i.tombstoned_at IS NULL",
            discovery_item_projection(datastore, "i", "t")
        ));
    }
    if sources.is_empty() {
        return None;
    }

    let mut clauses = Vec::new();
    append_authoritative_media_kind_filter(
        &mut clauses,
        &mut args,
        "i",
        &query.allowed_media_kinds,
    );
    append_discovery_items_filters(&mut clauses, &mut args, &query.filters);
    let where_clause = if clauses.is_empty() {
        "1 = 1".to_string()
    } else {
        clauses.join(" AND ")
    };

    Some(DiscoveryItemsSql {
        cte_sql: format!(
            "WITH visible AS (
                {}
             ),
             filtered AS (
                SELECT *
                FROM visible i
                WHERE {where_clause}
             ),
             deduped AS (
                SELECT *,
                       ROW_NUMBER() OVER (
                           PARTITION BY CASE WHEN TRIM(target_key) = '' THEN id ELSE target_key END
                           ORDER BY source_priority ASC,
                                    COALESCE(rank_score, -999999999.0) DESC,
                                    COALESCE(sort_title, display_title) ASC,
                                    target_key ASC
                       ) AS identity_rank
                FROM filtered
             )",
            sources.join(" UNION ALL ")
        ),
        args,
    })
}

fn append_discovery_home_filters(
    clauses: &mut Vec<String>,
    args: &mut Vec<SqlArg>,
    filters: &DiscoveryHomeFilters,
) {
    let clause_count = clauses.len();
    append_authoritative_media_kind_filter(clauses, args, "t", &filters.content_types);
    append_canonical_tag_key_filter(clauses, args, "genre", &filters.genre_tag_keys);
    append_canonical_tag_key_filter(clauses, args, "theme", &filters.theme_tag_keys);
    append_term_filter(clauses, args, "studio", &filters.studio_slugs);
    if let Some(minimum_year) = filters.minimum_year {
        clauses.push("(t.year IS NULL OR t.year >= {})".to_string());
        args.push(SqlArg::I32(minimum_year));
    }
    if let Some(maximum_year) = filters.maximum_year {
        clauses.push("(t.year IS NULL OR t.year <= {})".to_string());
        args.push(SqlArg::I32(maximum_year));
    }
    if let Some(minimum_rating) = filters.minimum_rating.filter(|value| value.is_finite()) {
        clauses.push(
            "COALESCE(
                (
                    SELECT MAX(CASE WHEN external.normalized <= 1.0
                                    THEN external.normalized * 10.0
                                    ELSE external.normalized END)
                    FROM discovery_title_metadata_external_ratings external
                    WHERE external.discovery_title_id = i.discovery_title_id
                      AND external.normalized IS NOT NULL
                      AND external.normalized > 0
                ),
                (
                    SELECT CASE WHEN rating_summary.rating <= 1.0
                                THEN rating_summary.rating * 10.0
                                ELSE rating_summary.rating END
                    FROM discovery_title_metadata_rating_summaries rating_summary
                    WHERE rating_summary.discovery_title_id = i.discovery_title_id
                )
             ) >= {}"
                .to_string(),
        );
        args.push(SqlArg::F64(minimum_rating));
    }
    if clauses.len() == clause_count {
        clauses.push("TRUE".to_string());
    }
}

fn append_discovery_items_filters(
    clauses: &mut Vec<String>,
    args: &mut Vec<SqlArg>,
    query: &scryer_application::DiscoveryItemsQuery,
) {
    if !query.include_owned {
        clauses.push("i.owned_in_input = FALSE".to_string());
    }
    if !query.include_unresolved {
        clauses.push("i.resolved = TRUE".to_string());
    }
    if let Some(query_text) = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let pattern = escaped_like_pattern(query_text);
        let text_columns = [
            "i.display_title",
            "i.original_title",
            "i.sort_title",
            "i.overview",
            "i.tmdb_collection_name",
        ];
        clauses.push(format!(
            "({})",
            text_columns
                .iter()
                .map(|column| {
                    args.push(SqlArg::Text(pattern.clone()));
                    format!("LOWER(COALESCE({column}, '')) LIKE {{}} ESCAPE '\\'")
                })
                .collect::<Vec<_>>()
                .join(" OR ")
        ));
    }
    let target_keys = normalized_filter_values(&query.target_keys);
    if !target_keys.is_empty() {
        let placeholders = placeholders(target_keys.len());
        args.extend(target_keys.into_iter().map(SqlArg::Text));
        clauses.push(format!("LOWER(i.target_key) IN ({placeholders})"));
    }
    append_term_filter(clauses, args, "media_kind", &query.target_kinds);
    append_source_filter(clauses, args, &query.sources);
    append_term_filter(clauses, args, "relation_type", &query.relation_types);
    append_term_filter(clauses, args, "relation_subtype", &query.relation_subtypes);
    append_canonical_facet_filter(clauses, args, "genre", &query.genres);
    append_term_filter(clauses, args, "status_tag", &query.status_tags);
    append_term_filter(clauses, args, "facet_term", &query.facet_terms);
}

fn append_term_filter(
    clauses: &mut Vec<String>,
    args: &mut Vec<SqlArg>,
    term_kind: &str,
    filters: &[String],
) {
    let values = normalized_filter_values(filters);
    if values.is_empty() {
        return;
    }
    let placeholders = placeholders(values.len());
    args.extend(values.into_iter().map(SqlArg::Text));
    clauses.push(format!(
        "EXISTS (
            SELECT 1
            FROM discovery_title_terms t
            WHERE t.discovery_title_id = i.discovery_title_id
              AND t.term_kind = '{term_kind}'
              AND LOWER(t.term_value) IN ({placeholders})
         )"
    ));
}

fn append_canonical_tag_key_filter(
    clauses: &mut Vec<String>,
    args: &mut Vec<SqlArg>,
    category: &str,
    keys: &[String],
) {
    if keys.is_empty() {
        return;
    }
    let placeholders = placeholders(keys.len());
    args.push(SqlArg::Text(category.trim().to_ascii_lowercase()));
    args.extend(keys.iter().cloned().map(SqlArg::Text));
    clauses.push(format!(
        "EXISTS (
            SELECT 1
            FROM discovery_title_metadata_tags tag
            WHERE tag.discovery_title_id = i.discovery_title_id
              AND LOWER(tag.category) = {{}}
              AND tag.tag_key IN ({placeholders})
         )"
    ));
}

fn append_canonical_facet_filter(
    clauses: &mut Vec<String>,
    args: &mut Vec<SqlArg>,
    kind: &str,
    filters: &[String],
) {
    let values = canonical_facet_filter_values(kind, filters);
    if values.is_empty() {
        return;
    }
    let placeholders = placeholders(values.len());
    args.push(SqlArg::Text(kind.trim().to_ascii_lowercase()));
    args.extend(values.iter().cloned().map(SqlArg::Text));
    args.extend(values.into_iter().map(SqlArg::Text));
    clauses.push(format!(
        "EXISTS (
            SELECT 1
            FROM discovery_title_metadata_tags tag
            WHERE tag.discovery_title_id = i.discovery_title_id
              AND LOWER(tag.category) = {{}}
              AND (
                    LOWER(tag.tag_key) IN ({placeholders})
                    OR LOWER(tag.name) IN ({placeholders})
              )
         )"
    ));
}

fn canonical_facet_filter_values(kind: &str, filters: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for filter in filters {
        let normalized = filter.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.clone()) {
            values.push(normalized.clone());
        }
        if normalized.starts_with(&format!("canonical:{kind}:")) {
            continue;
        }
        let parts = normalized
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            continue;
        }
        for separator in ["_", "-", " "] {
            let value = format!("canonical:{kind}:{}", parts.join(separator));
            if seen.insert(value.clone()) {
                values.push(value);
            }
        }
    }
    values
}

fn append_source_filter(clauses: &mut Vec<String>, args: &mut Vec<SqlArg>, filters: &[String]) {
    let values = normalized_filter_values(filters);
    if values.is_empty() {
        return;
    }
    let best_source_placeholders = placeholders(values.len());
    args.extend(values.iter().cloned().map(SqlArg::Text));
    let source_term_placeholders = placeholders(values.len());
    args.extend(values.into_iter().map(SqlArg::Text));
    clauses.push(format!(
        "(LOWER(COALESCE(i.best_source, '')) IN ({best_source_placeholders})
          OR EXISTS (
            SELECT 1
            FROM discovery_title_terms t
            WHERE t.discovery_title_id = i.discovery_title_id
              AND t.term_kind = 'source'
              AND LOWER(t.term_value) IN ({source_term_placeholders})
          ))"
    ));
}

fn normalized_filter_values(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}

fn escaped_like_pattern(value: &str) -> String {
    let mut pattern = String::from("%");
    for character in value.trim().to_ascii_lowercase().chars() {
        if matches!(character, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

fn library_provenance_exists_clause(
    item_alias: &str,
    readable_library_ids: &[String],
    args: &mut Vec<SqlArg>,
) -> String {
    let placeholders = placeholders(readable_library_ids.len());
    args.extend(readable_library_ids.iter().cloned().map(SqlArg::Text));
    format!(
        "EXISTS (
            SELECT 1
            FROM discovery_item_library_provenance p
            WHERE p.item_id = {item_alias}.id
              AND p.library_id IN ({placeholders})
         )"
    )
}

fn authoritative_media_kind_clause(item_alias: &str, media_kind: &str) -> String {
    format!(
        "LOWER(COALESCE(NULLIF(TRIM({item_alias}.content_type), ''), {item_alias}.target_kind)) = '{media_kind}'"
    )
}

fn append_authoritative_media_kind_filter(
    clauses: &mut Vec<String>,
    args: &mut Vec<SqlArg>,
    item_alias: &str,
    media_kinds: &[String],
) {
    let media_kinds = normalized_filter_values(media_kinds);
    if media_kinds.is_empty() {
        return;
    }
    let placeholders = placeholders(media_kinds.len());
    args.extend(media_kinds.into_iter().map(SqlArg::Text));
    clauses.push(format!(
        "LOWER(COALESCE(NULLIF(TRIM({item_alias}.content_type), ''), {item_alias}.target_kind)) IN ({placeholders})"
    ));
}

fn collection_signal_clause(item_alias: &str, title_alias: &str) -> String {
    format!(
        "({title_alias}.tmdb_collection_id IS NOT NULL
          OR TRIM(COALESCE({title_alias}.tmdb_collection_name, '')) <> ''
          OR EXISTS (
            SELECT 1
            FROM discovery_title_terms t
            WHERE t.discovery_title_id = {item_alias}.discovery_title_id
              AND t.term_kind IN ('relation_type', 'relation_subtype')
              AND (
                LOWER(t.term_value) = 'tmdb.collection'
                OR LOWER(t.term_value) LIKE '%collection%'
                OR LOWER(t.term_value) LIKE '%franchise%'
              )
          ))"
    )
}

async fn fetch_items_with_sql(
    datastore: &StoreDatastore,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<DiscoveryItemRecord>> {
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), sql, args).await?;
    let mut items = rows
        .iter()
        .map(item_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    let title_ids = discovery_title_ids_from_rows(&rows)?;
    hydrate_discovery_items(datastore, &mut items, &title_ids).await?;
    Ok(items)
}

async fn fetch_discovery_home_candidates_with_sql(
    datastore: &StoreDatastore,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<DiscoveryHomeCandidate>> {
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), sql, args).await?;
    let mut candidates = rows
        .iter()
        .map(home_candidate_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    hydrate_discovery_home_candidate_selection(datastore, &mut candidates).await?;
    Ok(candidates)
}

async fn fetch_discovery_home_rating_candidates_with_sql(
    datastore: &StoreDatastore,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<DiscoveryHomeCandidate>> {
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), sql, args).await?;
    let mut candidates = rows
        .iter()
        .map(home_candidate_from_row)
        .collect::<AppResult<Vec<_>>>()?;
    hydrate_discovery_home_candidate_ratings(datastore, &mut candidates).await?;
    Ok(candidates)
}

fn home_candidate_from_row(row: &SqlRow) -> AppResult<DiscoveryHomeCandidate> {
    Ok(DiscoveryHomeCandidate {
        item: item_from_row(row)?,
        discovery_title_id: row.text("discovery_title_id")?,
        matched_subject_keys: Vec::new(),
        affinity_terms: Vec::new(),
        has_hero_backdrop: row.bool("has_hero_backdrop")?,
        rating_source_count: 0,
        best_external_rating: None,
        best_external_rating_votes: 0,
    })
}

async fn hydrate_discovery_home_candidate_selection(
    datastore: &StoreDatastore,
    candidates: &mut [DiscoveryHomeCandidate],
) -> AppResult<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    hydrate_discovery_home_candidate_ratings(datastore, candidates).await?;
    let mut title_indexes = HashMap::<String, Vec<usize>>::new();
    let mut item_indexes = HashMap::<String, Vec<usize>>::new();
    for (index, candidate) in candidates.iter().enumerate() {
        title_indexes
            .entry(candidate.discovery_title_id.clone())
            .or_default()
            .push(index);
        item_indexes
            .entry(candidate.item.id.clone())
            .or_default()
            .push(index);
    }
    let mut title_ids = title_indexes.keys().cloned().collect::<Vec<_>>();
    title_ids.sort();
    let mut item_ids = item_indexes.keys().cloned().collect::<Vec<_>>();
    item_ids.sort();

    let affinity_rows = fetch_child_rows(
        datastore,
        "SELECT discovery_title_id, term_value, sort_index
             FROM discovery_title_terms
             WHERE discovery_title_id IN ({})
               AND term_kind = 'facet_term'
             ORDER BY discovery_title_id ASC, sort_index ASC, term_value ASC",
        &title_ids,
    )
    .await?;
    for row in affinity_rows {
        let title_id = row.text("discovery_title_id")?;
        let Some(indexes) = title_indexes.get(&title_id) else {
            continue;
        };
        let term_value = row.text("term_value")?;
        for index in indexes {
            candidates[*index].affinity_terms.push(term_value.clone());
        }
    }

    let matched_rows = fetch_child_rows(
        datastore,
        "SELECT item_id, subject_key, sort_index
         FROM discovery_item_subject_links
         WHERE item_id IN ({})
           AND link_type = 'matched'
         ORDER BY item_id ASC, sort_index ASC",
        &item_ids,
    )
    .await?;
    for row in matched_rows {
        let item_id = row.text("item_id")?;
        let Some(indexes) = item_indexes.get(&item_id) else {
            continue;
        };
        let subject_key = row.text("subject_key")?;
        for index in indexes {
            candidates[*index]
                .matched_subject_keys
                .push(subject_key.clone());
        }
    }
    Ok(())
}

async fn hydrate_discovery_home_candidate_ratings(
    datastore: &StoreDatastore,
    candidates: &mut [DiscoveryHomeCandidate],
) -> AppResult<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    let mut title_indexes = HashMap::<String, Vec<usize>>::new();
    for (index, candidate) in candidates.iter().enumerate() {
        title_indexes
            .entry(candidate.discovery_title_id.clone())
            .or_default()
            .push(index);
    }
    let mut title_ids = title_indexes.keys().cloned().collect::<Vec<_>>();
    title_ids.sort();
    let source_identity = rating_source_identity_sql("sources.source");
    let rows = fetch_child_rows(
        datastore,
        &format!(
            "SELECT t.id AS discovery_title_id,
                    summary.rating AS rating,
                    COALESCE((
                        SELECT COUNT(DISTINCT {source_identity})
                        FROM (
                            SELECT source
                            FROM discovery_title_metadata_rating_sources
                            WHERE discovery_title_id = t.id
                            UNION
                            SELECT source
                            FROM discovery_title_metadata_external_ratings
                            WHERE discovery_title_id = t.id
                        ) sources
                        WHERE TRIM(sources.source) <> ''
                    ), 0) AS rating_source_count,
                    (
                        SELECT MAX(CASE WHEN external.normalized <= 1.0
                                        THEN external.normalized * 10.0
                                        ELSE external.normalized END)
                        FROM discovery_title_metadata_external_ratings external
                        WHERE external.discovery_title_id = t.id
                          AND external.normalized IS NOT NULL
                          AND external.normalized > 0
                    ) AS best_external_rating,
                    COALESCE((
                        SELECT MAX(COALESCE(external.votes, 0))
                        FROM discovery_title_metadata_external_ratings external
                        WHERE external.discovery_title_id = t.id
                          AND external.normalized IS NOT NULL
                          AND external.normalized > 0
                    ), 0) AS best_external_rating_votes
             FROM discovery_titles t
             LEFT JOIN discovery_title_metadata_rating_summaries summary
               ON summary.discovery_title_id = t.id
             WHERE t.id IN ({{}})"
        ),
        &title_ids,
    )
    .await?;
    for row in rows {
        let title_id = row.text("discovery_title_id")?;
        let Some(indexes) = title_indexes.get(&title_id) else {
            continue;
        };
        for index in indexes {
            candidates[*index].item.rating = row.opt_f64("rating")?;
            candidates[*index].rating_source_count = row.i64("rating_source_count")? as i32;
            candidates[*index].best_external_rating = row.opt_f64("best_external_rating")?;
            candidates[*index].best_external_rating_votes =
                row.i64("best_external_rating_votes")? as i32;
        }
    }
    Ok(())
}

fn rating_source_identity_sql(column: &str) -> String {
    let normalized = normalized_rating_source_sql(column);
    format!(
        "CASE {normalized}
            WHEN 'rottentomatoes' THEN 'tomatoes'
            WHEN 'audience' THEN 'tomatoes'
            WHEN 'popcorn' THEN 'tomatoes'
            WHEN 'popcornmeter' THEN 'tomatoes'
            WHEN 'mcuser' THEN 'metacritic'
            WHEN 'metacriticuser' THEN 'metacritic'
            WHEN 'themoviedb' THEN 'tmdb'
            WHEN 'thetvdb' THEN 'tvdb'
            WHEN 'myanimelist' THEN 'mal'
            WHEN 'myanimelistnet' THEN 'mal'
            ELSE {normalized}
         END"
    )
}

fn normalized_rating_source_sql(column: &str) -> String {
    format!(
        "LOWER(REPLACE(REPLACE(REPLACE(REPLACE(REPLACE({column}, ' ', ''), '.', ''), '-', ''), '_', ''), '/', ''))"
    )
}

async fn hydrate_discovery_home_candidates(
    datastore: &StoreDatastore,
    candidates: &mut [DiscoveryHomeCandidate],
) -> AppResult<()> {
    hydrate_discovery_home_candidates_with_counts(datastore, candidates)
        .await
        .map(|_| ())
}

async fn hydrate_discovery_home_hero(
    datastore: &StoreDatastore,
    candidate: &mut DiscoveryHomeCandidate,
) -> AppResult<()> {
    let started_at = Instant::now();
    let item_ids = [candidate.item.id.clone()];
    let rows = fetch_child_rows(
        datastore,
        "SELECT i.id, i.discovery_title_id, t.background_url, t.overview
         FROM discovery_items i
         JOIN discovery_titles t
           ON t.id = i.discovery_title_id
         WHERE i.id IN ({})",
        &item_ids,
    )
    .await?;
    let row = rows.first().ok_or_else(|| {
        AppError::Repository(format!(
            "selected discovery-home hero {} no longer exists",
            candidate.item.id
        ))
    })?;
    let discovery_title_id = row.text("discovery_title_id")?;
    if discovery_title_id != candidate.discovery_title_id {
        return Err(AppError::Repository(format!(
            "selected discovery-home hero {} changed titles during hydration",
            candidate.item.id
        )));
    }
    candidate.item.background_url = row.opt_text("background_url")?;
    candidate.item.overview = row.opt_text("overview")?;
    let title_ids = [candidate.discovery_title_id.clone()];
    let ratings = load_discovery_title_metadata_ratings(datastore.read_exec(), &title_ids).await?;
    if let Some(rating) = ratings.get(&candidate.discovery_title_id) {
        candidate.item.rating = rating.rating;
        candidate.item.rating_sources = rating.rating_sources.clone();
        candidate.item.external_ratings = rating.external_ratings.clone();
    }
    let mut tags = load_discovery_title_metadata_tags(datastore.read_exec(), &title_ids).await?;
    candidate.item.canonical_tags = tags
        .remove(&candidate.discovery_title_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|tag| tag.category.eq_ignore_ascii_case("genre"))
        .collect();
    debug!(
        operation = "discovery_home",
        stage = "hero_presentation_hydration_children",
        selected_item_count = 1,
        selected_title_count = 1,
        canonical_tag_rows = candidate.item.canonical_tags.len(),
        title_term_rows = 0,
        rating_summary_rows = usize::from(candidate.item.rating.is_some()),
        rating_source_rows = candidate.item.rating_sources.len(),
        external_rating_rows = candidate.item.external_ratings.len(),
        elapsed_ms = started_at.elapsed().as_millis(),
        "discovery home hero presentation metadata hydrated"
    );
    Ok(())
}

async fn hydrate_discovery_home_candidates_with_counts(
    datastore: &StoreDatastore,
    candidates: &mut [DiscoveryHomeCandidate],
) -> AppResult<DiscoveryItemHydrationCounts> {
    if candidates.is_empty() {
        return Ok(DiscoveryItemHydrationCounts::default());
    }
    let started_at = Instant::now();
    let item_ids = candidates
        .iter()
        .map(|candidate| candidate.item.id.clone())
        .collect::<Vec<_>>();
    let resolved_title_rows = fetch_child_rows(
        datastore,
        "SELECT id, discovery_title_id
         FROM discovery_items
         WHERE id IN ({})",
        &item_ids,
    )
    .await?;
    let resolved_title_ids = resolved_title_rows
        .iter()
        .map(|row| Ok((row.text("id")?, row.text("discovery_title_id")?)))
        .collect::<AppResult<HashMap<_, _>>>()?;
    let item_title_ids = candidates
        .iter()
        .map(|candidate| {
            let resolved_title_id =
                resolved_title_ids.get(&candidate.item.id).ok_or_else(|| {
                    AppError::Repository(format!(
                        "selected discovery-home item {} no longer exists",
                        candidate.item.id
                    ))
                })?;
            if resolved_title_id != &candidate.discovery_title_id {
                return Err(AppError::Repository(format!(
                    "selected discovery-home item {} changed titles during hydration",
                    candidate.item.id
                )));
            }
            Ok(resolved_title_id.clone())
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut title_ids = item_title_ids.clone();
    title_ids.sort();
    title_ids.dedup();
    let mut items = candidates
        .iter()
        .map(|candidate| candidate.item.clone())
        .collect::<Vec<_>>();
    let hydration_counts =
        hydrate_discovery_items_with_counts(datastore, &mut items, &item_title_ids).await?;
    for (candidate, item) in candidates.iter_mut().zip(items) {
        candidate.item = item;
    }
    debug!(
        operation = "discovery_home",
        stage = "selected_hydration_children",
        selected_card_count = candidates.len(),
        selected_title_count = title_ids.len(),
        canonical_tag_rows = hydration_counts.canonical_tag_rows,
        title_term_rows = hydration_counts.title_term_rows,
        source_tag_rows = hydration_counts.source_tag_rows,
        source_tag_value_rows = hydration_counts.source_tag_value_rows,
        rating_summary_rows = hydration_counts.rating_summary_rows,
        rating_source_rows = hydration_counts.rating_source_rows,
        external_rating_rows = hydration_counts.external_rating_rows,
        external_id_rows = hydration_counts.external_id_rows,
        rank_component_rows = hydration_counts.rank_component_rows,
        subject_link_rows = hydration_counts.subject_link_rows,
        library_provenance_rows = hydration_counts.library_provenance_rows,
        elapsed_ms = started_at.elapsed().as_millis(),
        "discovery home selected child metadata hydrated"
    );
    Ok(hydration_counts)
}

async fn discovery_run_language(
    datastore: &StoreDatastore,
    run_id: &str,
) -> AppResult<Option<String>> {
    SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT language FROM discovery_sync_runs WHERE id = {}",
        &[SqlArg::Text(run_id.to_string())],
    )
    .await?
    .map(|row| row.text("language"))
    .transpose()
}

fn upsert_sql(table: &str, columns: &[&str], conflict_columns: &[&str]) -> String {
    let updates = columns
        .iter()
        .filter(|column| !conflict_columns.contains(column))
        .map(|column| format!("{column} = excluded.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {table} ({}) VALUES ({})
         ON CONFLICT({}) DO UPDATE SET {updates}",
        columns.join(", "),
        placeholders(columns.len()),
        conflict_columns.join(", ")
    )
}

/// Upsert for `discovery_pending_context_changes`, resolving `title_id` through
/// `titles` instead of binding the raw id.
///
/// These rows are produced by replaying the *historical* domain-event log, so an
/// event legitimately names a title that no longer exists — a `title_deleted`
/// event always does, and an older `title_added`/`title_updated` event does once
/// that title is later removed. Sqlite declares
/// `title_id TEXT REFERENCES titles(id) ON DELETE SET NULL`, so binding a
/// dangling id raises `FOREIGN KEY constraint failed` (sqlite extended code 787)
/// and aborts the whole discovery sync. That is unrecoverable rather than
/// transient: the catch-up watermark (`last_seen_domain_event_sequence`) only
/// advances when the job succeeds, so the very next run replays the same event
/// and fails identically, freezing discovery indefinitely.
///
/// The `(SELECT id FROM titles WHERE id = {})` scalar subquery yields NULL when
/// the title is gone, which is exactly the value the declared
/// `ON DELETE SET NULL` would leave behind had the title been deleted a moment
/// later — so the column keeps its meaning ("the live title this change refers
/// to, if any") instead of dangling. The postgres schema declares the same
/// column with no foreign key at all, so resolving here also makes both backends
/// agree on that invariant. Resolution happens inside the write statement, so it
/// stays correct when a sqlite busy-retry re-runs the whole closure.
///
/// The id that was replayed is not lost: `title_context_change_record` derives
/// the row `id` (`{scope_key}:title:{title_id}`) from it before persistence, and
/// `coalesce_pending_context_change` carries it into `previous_title_id`, which
/// is deliberately declared without a foreign key.
fn upsert_pending_context_change_sql() -> String {
    let columns = split_columns(PENDING_CONTEXT_CHANGE_COLUMNS);
    let values = columns
        .iter()
        .map(|column| match *column {
            "title_id" => "(SELECT id FROM titles WHERE id = {})",
            _ => "{}",
        })
        .collect::<Vec<_>>()
        .join(", ");
    let updates = columns
        .iter()
        .filter(|column| **column != "id")
        .map(|column| format!("{column} = excluded.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO discovery_pending_context_changes ({}) VALUES ({values})
         ON CONFLICT(id) DO UPDATE SET {updates}",
        columns.join(", ")
    )
}

fn upsert_discovery_title_sql() -> String {
    format!(
        "INSERT INTO discovery_titles ({}) VALUES ({})
         ON CONFLICT(target_key_norm, language) DO UPDATE SET
            target_key = COALESCE(NULLIF(excluded.target_key, ''), discovery_titles.target_key),
            target_kind = COALESCE(NULLIF(excluded.target_kind, ''), discovery_titles.target_kind),
            resolved = CASE
                WHEN excluded.resolved THEN excluded.resolved
                ELSE discovery_titles.resolved
            END,
            resolved_title_id = COALESCE(
                NULLIF(excluded.resolved_title_id, ''),
                discovery_titles.resolved_title_id
            ),
            display_title = COALESCE(
                NULLIF(excluded.display_title, ''),
                discovery_titles.display_title
            ),
            original_title = COALESCE(
                NULLIF(excluded.original_title, ''),
                discovery_titles.original_title
            ),
            sort_title = COALESCE(NULLIF(excluded.sort_title, ''), discovery_titles.sort_title),
            year = COALESCE(excluded.year, discovery_titles.year),
            poster_path = COALESCE(NULLIF(excluded.poster_path, ''), discovery_titles.poster_path),
            poster_url = COALESCE(NULLIF(excluded.poster_url, ''), discovery_titles.poster_url),
            background_url = COALESCE(
                NULLIF(excluded.background_url, ''),
                discovery_titles.background_url
            ),
            overview = COALESCE(NULLIF(excluded.overview, ''), discovery_titles.overview),
            content_type = COALESCE(
                NULLIF(excluded.content_type, ''),
                discovery_titles.content_type
            ),
            is_adult = excluded.is_adult,
            content_ratings_json = excluded.content_ratings_json,
            tmdb_collection_id = COALESCE(
                NULLIF(excluded.tmdb_collection_id, ''),
                discovery_titles.tmdb_collection_id
            ),
            tmdb_collection_name = COALESCE(
                NULLIF(excluded.tmdb_collection_name, ''),
                discovery_titles.tmdb_collection_name
            ),
            updated_at = excluded.updated_at",
        TITLE_COLUMNS.join(", "),
        placeholders(TITLE_COLUMNS.len())
    )
}

async fn delete_for_run_tx(tx: &mut SqlTx<'_>, table: &'static str, run_id: &str) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &format!("DELETE FROM {table} WHERE run_id = {{}}"),
        &[SqlArg::Text(run_id.to_string())],
    )
    .await?;
    Ok(())
}

async fn delete_item_children_for_run_tx(tx: &mut SqlTx<'_>, run_id: &str) -> AppResult<()> {
    for table in [
        "discovery_section_items",
        "discovery_item_rank_components",
        "discovery_item_subject_links",
        "discovery_item_library_provenance",
    ] {
        delete_for_run_tx(tx, table, run_id).await?;
    }
    Ok(())
}

async fn upsert_sync_state_tx(
    tx: &mut SqlTx<'_>,
    state: &DiscoverySyncStateRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &upsert_sql(
            "discovery_sync_state",
            &split_columns(DISCOVERY_SYNC_STATE_COLUMNS),
            &["scope_key"],
        ),
        &sync_state_args(state),
    )
    .await?;
    Ok(())
}

async fn upsert_sync_run_tx(
    tx: &mut SqlTx<'_>,
    datastore: &StoreDatastore,
    run: &DiscoverySyncRunRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &upsert_sql(
            "discovery_sync_runs",
            &split_columns(DISCOVERY_SYNC_RUN_COLUMNS),
            &["id"],
        ),
        &sync_run_args(datastore, run)?,
    )
    .await?;
    Ok(())
}

async fn tombstone_discovery_items_tx(
    tx: &mut SqlTx<'_>,
    base_generation_id: Option<&str>,
    target_keys: &[String],
    tombstone_run_id: &str,
    tombstoned_at: chrono::DateTime<chrono::Utc>,
) -> AppResult<()> {
    let Some(base_generation_id) = base_generation_id else {
        return Ok(());
    };
    for target_key in target_keys {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "UPDATE discovery_items
             SET tombstoned_by_run_id = {}, tombstoned_at = {}, updated_at = {}
             WHERE base_generation_id = {}
               AND discovery_title_id IN (
                    SELECT id
                    FROM discovery_titles
                    WHERE target_key = {}
               )
               AND tombstoned_at IS NULL",
            &[
                SqlArg::Text(tombstone_run_id.to_string()),
                SqlArg::Timestamp(tombstoned_at),
                SqlArg::Timestamp(tombstoned_at),
                SqlArg::Text(base_generation_id.to_string()),
                SqlArg::Text(target_key.clone()),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn clear_pending_discovery_context_changes_tx(
    tx: &mut SqlTx<'_>,
    scope_key: &str,
    last_seen_sequence: i64,
) -> AppResult<u64> {
    let deleted = SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM discovery_pending_context_changes
         WHERE scope_key = {}
           AND last_seen_sequence IS NOT NULL
           AND last_seen_sequence <= {}",
        &[
            SqlArg::Text(scope_key.to_string()),
            SqlArg::I64(last_seen_sequence),
        ],
    )
    .await?;
    Ok(deleted)
}

fn sync_state_args(state: &DiscoverySyncStateRecord) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(state.scope_key.clone()),
        SqlArg::OptText(state.last_success_generation_id.clone()),
        SqlArg::OptText(state.last_public_feed_generation_id.clone()),
        SqlArg::OptText(state.last_subject_fingerprint.clone()),
        SqlArg::OptTimestamp(state.last_context_snapshot_completed_at),
        SqlArg::OptTimestamp(state.last_incremental_reload_completed_at),
        SqlArg::OptTimestamp(state.last_public_feed_completed_at),
        SqlArg::OptTimestamp(state.dirty_since),
        SqlArg::I64(state.dirty_reason_mask),
        SqlArg::OptTimestamp(state.bootstrap_started_at),
        SqlArg::OptTimestamp(state.bootstrap_quiet_until),
        SqlArg::OptTimestamp(state.next_context_snapshot_eligible_at),
        SqlArg::OptTimestamp(state.next_incremental_reload_eligible_at),
        SqlArg::OptTimestamp(state.next_public_feed_eligible_at),
        SqlArg::OptTimestamp(state.backoff_until),
        SqlArg::I64(state.transient_failure_count),
        SqlArg::I64(state.startup_jitter_seconds),
        SqlArg::I64(state.context_jitter_seconds),
        SqlArg::I64(state.incremental_reload_jitter_seconds),
        SqlArg::I64(state.public_feed_jitter_seconds),
        SqlArg::OptI64(state.last_seen_domain_event_sequence),
        SqlArg::OptText(state.inflight_context_snapshot_run_id.clone()),
        SqlArg::OptText(state.inflight_subject_fingerprint.clone()),
        SqlArg::OptI64(state.inflight_domain_event_sequence),
        SqlArg::OptText(state.lease_owner_id.clone()),
        SqlArg::OptTimestamp(state.lease_expires_at),
        SqlArg::Timestamp(state.updated_at),
    ]
}

fn sync_state_from_row(row: &SqlRow) -> AppResult<DiscoverySyncStateRecord> {
    Ok(DiscoverySyncStateRecord {
        scope_key: row.text("scope_key")?,
        last_success_generation_id: row.opt_text("last_success_generation_id")?,
        last_public_feed_generation_id: row.opt_text("last_public_feed_generation_id")?,
        last_subject_fingerprint: row.opt_text("last_subject_fingerprint")?,
        last_context_snapshot_completed_at: row
            .opt_timestamp("last_context_snapshot_completed_at")?,
        last_incremental_reload_completed_at: row
            .opt_timestamp("last_incremental_reload_completed_at")?,
        last_public_feed_completed_at: row.opt_timestamp("last_public_feed_completed_at")?,
        dirty_since: row.opt_timestamp("dirty_since")?,
        dirty_reason_mask: row.i64("dirty_reason_mask")?,
        bootstrap_started_at: row.opt_timestamp("bootstrap_started_at")?,
        bootstrap_quiet_until: row.opt_timestamp("bootstrap_quiet_until")?,
        next_context_snapshot_eligible_at: row
            .opt_timestamp("next_context_snapshot_eligible_at")?,
        next_incremental_reload_eligible_at: row
            .opt_timestamp("next_incremental_reload_eligible_at")?,
        next_public_feed_eligible_at: row.opt_timestamp("next_public_feed_eligible_at")?,
        backoff_until: row.opt_timestamp("backoff_until")?,
        transient_failure_count: row.i64("transient_failure_count")?,
        startup_jitter_seconds: row.i64("startup_jitter_seconds")?,
        context_jitter_seconds: row.i64("context_jitter_seconds")?,
        incremental_reload_jitter_seconds: row.i64("incremental_reload_jitter_seconds")?,
        public_feed_jitter_seconds: row.i64("public_feed_jitter_seconds")?,
        last_seen_domain_event_sequence: row.opt_i64("last_seen_domain_event_sequence")?,
        inflight_context_snapshot_run_id: row.opt_text("inflight_context_snapshot_run_id")?,
        inflight_subject_fingerprint: row.opt_text("inflight_subject_fingerprint")?,
        inflight_domain_event_sequence: row.opt_i64("inflight_domain_event_sequence")?,
        lease_owner_id: row.opt_text("lease_owner_id")?,
        lease_expires_at: row.opt_timestamp("lease_expires_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn sync_run_args(
    _datastore: &StoreDatastore,
    run: &DiscoverySyncRunRecord,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(run.id.clone()),
        SqlArg::Text(run.kind.clone()),
        SqlArg::Text(run.status.clone()),
        SqlArg::Text(run.trigger_source.clone()),
        SqlArg::Text(run.region.clone()),
        SqlArg::Text(run.language.clone()),
        SqlArg::I64(run.subject_count),
        SqlArg::OptText(run.subject_fingerprint.clone()),
        SqlArg::OptText(run.previous_subject_fingerprint.clone()),
        SqlArg::OptText(run.base_generation_id.clone()),
        SqlArg::I64(run.changed_subject_count),
        SqlArg::I64(run.affected_target_count),
        SqlArg::OptText(run.smg_request_id.clone()),
        SqlArg::OptText(run.smg_status.clone()),
        SqlArg::OptText(run.discovery_index_watermark.clone()),
        SqlArg::OptI32(run.page_count),
        SqlArg::OptI64(run.item_count),
        SqlArg::OptI64(run.facet_count),
        SqlArg::OptTimestamp(run.acknowledged_at),
        SqlArg::OptText(run.error_text.clone()),
        SqlArg::OptTimestamp(run.started_at),
        SqlArg::OptTimestamp(run.completed_at),
        SqlArg::Timestamp(run.created_at),
        SqlArg::Timestamp(run.updated_at),
    ])
}

fn sync_run_from_row(row: &SqlRow) -> AppResult<DiscoverySyncRunRecord> {
    Ok(DiscoverySyncRunRecord {
        id: row.text("id")?,
        kind: row.text("kind")?,
        status: row.text("status")?,
        trigger_source: row.text("trigger_source")?,
        region: row.text("region")?,
        language: row.text("language")?,
        subject_count: row.i64("subject_count")?,
        subject_fingerprint: row.opt_text("subject_fingerprint")?,
        previous_subject_fingerprint: row.opt_text("previous_subject_fingerprint")?,
        base_generation_id: row.opt_text("base_generation_id")?,
        changed_subject_count: row.i64("changed_subject_count")?,
        affected_target_count: row.i64("affected_target_count")?,
        smg_request_id: row.opt_text("smg_request_id")?,
        smg_status: row.opt_text("smg_status")?,
        discovery_index_watermark: row.opt_text("discovery_index_watermark")?,
        page_count: row.opt_i32("page_count")?,
        item_count: row.opt_i64("item_count")?,
        facet_count: row.opt_i64("facet_count")?,
        acknowledged_at: row.opt_timestamp("acknowledged_at")?,
        error_text: row.opt_text("error_text")?,
        started_at: row.opt_timestamp("started_at")?,
        completed_at: row.opt_timestamp("completed_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn pending_context_change_args(
    datastore: &StoreDatastore,
    change: &DiscoveryPendingContextChangeRecord,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(change.id.clone()),
        SqlArg::Text(change.scope_key.clone()),
        SqlArg::OptText(change.subject_key.clone()),
        SqlArg::OptText(change.previous_subject_key.clone()),
        SqlArg::Text(change.change_type.clone()),
        SqlArg::OptText(change.title_id.clone()),
        SqlArg::OptText(change.previous_title_id.clone()),
        SqlArg::OptText(change.library_facet.clone()),
        opt_json_arg(datastore, change.raw_subject_json.as_deref())?,
        opt_json_arg(datastore, change.raw_previous_subject_json.as_deref())?,
        SqlArg::OptI64(change.first_seen_sequence),
        SqlArg::OptI64(change.last_seen_sequence),
        SqlArg::Timestamp(change.first_seen_at),
        SqlArg::Timestamp(change.last_seen_at),
    ])
}

fn pending_context_change_from_row(row: &SqlRow) -> AppResult<DiscoveryPendingContextChangeRecord> {
    Ok(DiscoveryPendingContextChangeRecord {
        id: row.text("id")?,
        scope_key: row.text("scope_key")?,
        subject_key: row.opt_text("subject_key")?,
        previous_subject_key: row.opt_text("previous_subject_key")?,
        change_type: row.text("change_type")?,
        title_id: row.opt_text("title_id")?,
        previous_title_id: row.opt_text("previous_title_id")?,
        library_facet: row.opt_text("library_facet")?,
        raw_subject_json: opt_json_text(row, "raw_subject_json")?,
        raw_previous_subject_json: opt_json_text(row, "raw_previous_subject_json")?,
        first_seen_sequence: row.opt_i64("first_seen_sequence")?,
        last_seen_sequence: row.opt_i64("last_seen_sequence")?,
        first_seen_at: row.timestamp("first_seen_at")?,
        last_seen_at: row.timestamp("last_seen_at")?,
    })
}

fn section_from_row(row: &SqlRow) -> AppResult<DiscoverySectionRecord> {
    Ok(DiscoverySectionRecord {
        id: row.text("id")?,
        run_id: row.text("run_id")?,
        section_id: row.text("section_id")?,
        section_type: row.text("section_type")?,
        surface: row.text("surface")?,
        title: row.text("title")?,
        sort_index: row.i32("sort_index")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn item_from_row(row: &SqlRow) -> AppResult<DiscoveryItemRecord> {
    Ok(DiscoveryItemRecord {
        id: row.text("id")?,
        run_id: row.text("run_id")?,
        base_generation_id: row.opt_text("base_generation_id")?,
        source_run_kind: row.text("source_run_kind")?,
        section_id: row.opt_text("section_id")?,
        sort_index: row.i32("sort_index")?,
        target_key: row.text("target_key")?,
        target_kind: row.text("target_kind")?,
        resolved: row.bool("resolved")?,
        resolved_title_id: row.opt_text("resolved_title_id")?,
        display_title: row.text("display_title")?,
        original_title: row.opt_text("original_title")?,
        sort_title: row.opt_text("sort_title")?,
        year: row.opt_i32("year")?,
        poster_path: row.opt_text("poster_path")?,
        poster_url: row.opt_text("poster_url")?,
        background_url: row.opt_text("background_url")?,
        overview: row.opt_text("overview")?,
        content_type: row.opt_text("content_type")?,
        canonical_tags: Vec::new(),
        is_adult: row.bool("is_adult")?,
        content_ratings: discovery_content_ratings_from_json(&row.text("content_ratings_json")?)?,
        rating: row.opt_f64("rating")?,
        rating_sources: Vec::new(),
        external_ratings: Vec::new(),
        external_ids: Vec::new(),
        status_tags: Vec::new(),
        source_tags: Vec::new(),
        sources: Vec::new(),
        best_source: row.opt_text("best_source")?,
        relation_types: Vec::new(),
        relation_subtypes: Vec::new(),
        chart_signals: Vec::new(),
        provider_signals: Vec::new(),
        rank_components: Vec::new(),
        source_count: row.opt_i32("source_count")?,
        edge_count: row.opt_i32("edge_count")?,
        relation_count: row.opt_i32("relation_count")?,
        source_subject_count: row.opt_i32("source_subject_count")?,
        rank_score: row.opt_f64("rank_score")?,
        matched_subject_keys: Vec::new(),
        matched_subject_titles: Vec::new(),
        matched_subject_count: row.i32("matched_subject_count")?,
        library_provenance: Vec::new(),
        tmdb_collection_id: row.opt_text("tmdb_collection_id")?,
        tmdb_collection_name: row.opt_text("tmdb_collection_name")?,
        owned_in_input: row.bool("owned_in_input")?,
        studio_slug: None,
        person_ids: Vec::new(),
        facet_terms: Vec::new(),
        context_terms: Vec::new(),
        change_subject_keys: Vec::new(),
        removed_subject_keys: Vec::new(),
        tombstoned_by_run_id: row.opt_text("tombstoned_by_run_id")?,
        tombstoned_at: row.opt_timestamp("tombstoned_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn facet_from_row(row: &SqlRow) -> AppResult<DiscoveryFacetRecord> {
    Ok(DiscoveryFacetRecord {
        run_id: row.text("run_id")?,
        facet_name: row.text("facet_name")?,
        facet_value: row.text("facet_value")?,
        smg_count: row.opt_i64("smg_count")?,
        local_count: row.opt_i64("local_count")?,
    })
}

fn canonical_facet_from_row(run_id: &str, row: &SqlRow) -> AppResult<Option<DiscoveryFacetRecord>> {
    let facet_term = row.text("facet_term")?;
    let Some((facet_name, facet_value)) = canonical_facet_display_value(&facet_term) else {
        return Ok(None);
    };
    Ok(Some(DiscoveryFacetRecord {
        run_id: run_id.to_string(),
        facet_name,
        facet_value,
        smg_count: None,
        local_count: row.opt_i64("local_count")?,
    }))
}

fn canonical_facet_display_value(value: &str) -> Option<(String, String)> {
    let value = value.trim();
    let mut parts = value.splitn(3, ':');
    if !parts.next()?.eq_ignore_ascii_case("canonical") {
        return None;
    }
    let kind = parts.next()?.trim();
    if !kind.eq_ignore_ascii_case("genre") && !kind.eq_ignore_ascii_case("theme") {
        return None;
    }
    let tail = parts.next()?.trim();
    if tail.is_empty() {
        return None;
    }
    Some((kind.to_ascii_lowercase(), canonical_label_from_slug(tail)))
}

fn canonical_label_from_slug(value: &str) -> String {
    value
        .split(|character: char| {
            character == '-' || character == '_' || character == ':' || character.is_whitespace()
        })
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => {
                    let mut word = first.to_uppercase().collect::<String>();
                    word.extend(characters.flat_map(char::to_lowercase));
                    word
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn hydrate_discovery_items(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
) -> AppResult<()> {
    hydrate_discovery_items_with_counts(datastore, items, discovery_title_ids)
        .await
        .map(|_| ())
}

#[derive(Default)]
struct DiscoveryItemHydrationCounts {
    canonical_tag_rows: usize,
    title_term_rows: usize,
    source_tag_rows: usize,
    source_tag_value_rows: usize,
    rating_summary_rows: usize,
    rating_source_rows: usize,
    external_rating_rows: usize,
    external_id_rows: usize,
    rank_component_rows: usize,
    subject_link_rows: usize,
    library_provenance_rows: usize,
}

async fn hydrate_discovery_items_with_counts(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
) -> AppResult<DiscoveryItemHydrationCounts> {
    if items.is_empty() {
        return Ok(DiscoveryItemHydrationCounts::default());
    }
    let item_ids = items.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
    let mut item_indexes = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        item_indexes.insert(item.id.clone(), index);
    }
    let mut counts =
        hydrate_discovery_title_children(datastore, items, discovery_title_ids).await?;
    counts.rank_component_rows =
        hydrate_item_rank_components(datastore, items, &item_ids, &item_indexes).await?;
    counts.subject_link_rows =
        hydrate_item_subject_links(datastore, items, &item_ids, &item_indexes).await?;
    counts.library_provenance_rows =
        hydrate_item_library_provenance(datastore, items, &item_ids, &item_indexes).await?;
    Ok(counts)
}

fn discovery_title_ids_from_rows(rows: &[SqlRow]) -> AppResult<Vec<String>> {
    rows.iter()
        .map(|row| row.text("discovery_title_id"))
        .collect()
}

async fn hydrate_discovery_title_children(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
) -> AppResult<DiscoveryItemHydrationCounts> {
    if items.is_empty() {
        return Ok(DiscoveryItemHydrationCounts::default());
    }
    let mut title_indexes = HashMap::<String, Vec<usize>>::new();
    for (index, title_id) in discovery_title_ids.iter().enumerate() {
        if !title_id.trim().is_empty() {
            title_indexes
                .entry(title_id.clone())
                .or_default()
                .push(index);
        }
    }
    if title_indexes.is_empty() {
        return Ok(DiscoveryItemHydrationCounts::default());
    }
    let mut unique_title_ids = title_indexes.keys().cloned().collect::<Vec<_>>();
    unique_title_ids.sort();
    let canonical_tag_rows =
        hydrate_title_canonical_tags(datastore, items, &unique_title_ids, &title_indexes).await?;
    let title_term_rows =
        hydrate_title_terms(datastore, items, &unique_title_ids, &title_indexes).await?;
    let (source_tag_rows, source_tag_value_rows) =
        hydrate_title_source_tags(datastore, items, &unique_title_ids, &title_indexes).await?;
    let (rating_summary_rows, rating_source_rows, external_rating_rows) =
        hydrate_title_ratings(datastore, items, &unique_title_ids, &title_indexes).await?;
    let external_id_rows =
        hydrate_title_external_ids(datastore, items, &unique_title_ids, &title_indexes).await?;
    Ok(DiscoveryItemHydrationCounts {
        canonical_tag_rows,
        title_term_rows,
        source_tag_rows,
        source_tag_value_rows,
        rating_summary_rows,
        rating_source_rows,
        external_rating_rows,
        external_id_rows,
        ..DiscoveryItemHydrationCounts::default()
    })
}

async fn hydrate_title_canonical_tags(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
    title_indexes: &HashMap<String, Vec<usize>>,
) -> AppResult<usize> {
    let tags_by_title =
        load_discovery_title_metadata_tags(datastore.read_exec(), discovery_title_ids).await?;
    let row_count = tags_by_title.values().map(Vec::len).sum();
    for (discovery_title_id, tags) in tags_by_title {
        if tags.is_empty() {
            continue;
        }
        let Some(indexes) = title_indexes.get(&discovery_title_id) else {
            continue;
        };
        for index in indexes {
            items[*index].canonical_tags = tags.clone();
        }
    }
    Ok(row_count)
}

async fn hydrate_title_terms(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
    title_indexes: &HashMap<String, Vec<usize>>,
) -> AppResult<usize> {
    let rows = fetch_child_rows(
        datastore,
        "SELECT discovery_title_id, term_kind, term_category, term_value, sort_index
         FROM discovery_title_terms
         WHERE discovery_title_id IN ({})
         ORDER BY discovery_title_id ASC, term_kind ASC, sort_index ASC, term_value ASC",
        discovery_title_ids,
    )
    .await?;
    let row_count = rows.len();
    for row in rows {
        let discovery_title_id = row.text("discovery_title_id")?;
        let Some(indexes) = title_indexes.get(&discovery_title_id) else {
            continue;
        };
        let term_kind = row.text("term_kind")?;
        let term_value = row.text("term_value")?;
        for index in indexes {
            let item = &mut items[*index];
            match term_kind.as_str() {
                "status_tag" => item.status_tags.push(term_value.clone()),
                "source" => item.sources.push(term_value.clone()),
                "relation_type" => item.relation_types.push(term_value.clone()),
                "relation_subtype" => item.relation_subtypes.push(term_value.clone()),
                "chart_signal" => item.chart_signals.push(term_value.clone()),
                "provider_signal" => item.provider_signals.push(term_value.clone()),
                "facet_term" => item.facet_terms.push(term_value.clone()),
                "context_term" => item.context_terms.push(term_value.clone()),
                "studio" => item.studio_slug = Some(term_value.clone()),
                "person" => {
                    if let Ok(person_id) = term_value.parse::<i32>() {
                        item.person_ids.push(person_id);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(row_count)
}

async fn hydrate_title_source_tags(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
    title_indexes: &HashMap<String, Vec<usize>>,
) -> AppResult<(usize, usize)> {
    let mut source_tag_indexes = HashMap::<(String, i32), Vec<(usize, usize)>>::new();
    let rows = fetch_child_rows(
        datastore,
        "SELECT discovery_title_id, category, name, sort_index
         FROM discovery_title_source_tags
         WHERE discovery_title_id IN ({})
         ORDER BY discovery_title_id ASC, sort_index ASC",
        discovery_title_ids,
    )
    .await?;
    let source_tag_row_count = rows.len();
    for row in rows {
        let discovery_title_id = row.text("discovery_title_id")?;
        let Some(indexes) = title_indexes.get(&discovery_title_id) else {
            continue;
        };
        let sort_index = row.i32("sort_index")?;
        let category = empty_to_none(row.text("category")?);
        let name = empty_to_none(row.text("name")?);
        let mut source_tag_item_indexes = Vec::new();
        for index in indexes {
            let source_tag_index = items[*index].source_tags.len();
            items[*index].source_tags.push(DiscoverySourceTagRecord {
                category: category.clone(),
                name: name.clone(),
                values: Vec::new(),
            });
            source_tag_item_indexes.push((*index, source_tag_index));
        }
        source_tag_indexes.insert((discovery_title_id, sort_index), source_tag_item_indexes);
    }

    let value_rows = fetch_child_rows(
        datastore,
        "SELECT discovery_title_id, source_tag_sort_index, source_tag_value, value_sort_index
         FROM discovery_title_source_tag_values
         WHERE discovery_title_id IN ({})
         ORDER BY discovery_title_id ASC, source_tag_sort_index ASC, value_sort_index ASC",
        discovery_title_ids,
    )
    .await?;
    let source_tag_value_row_count = value_rows.len();
    for row in value_rows {
        let discovery_title_id = row.text("discovery_title_id")?;
        let source_tag_sort_index = row.i32("source_tag_sort_index")?;
        let Some(source_tag_item_indexes) =
            source_tag_indexes.get(&(discovery_title_id, source_tag_sort_index))
        else {
            continue;
        };
        let source_tag_value = row.text("source_tag_value")?;
        for (item_index, source_tag_index) in source_tag_item_indexes {
            items[*item_index].source_tags[*source_tag_index]
                .values
                .push(source_tag_value.clone());
        }
    }
    Ok((source_tag_row_count, source_tag_value_row_count))
}

async fn hydrate_title_ratings(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
    title_indexes: &HashMap<String, Vec<usize>>,
) -> AppResult<(usize, usize, usize)> {
    let ratings_by_title =
        load_discovery_title_metadata_ratings(datastore.read_exec(), discovery_title_ids).await?;
    let rating_summary_rows = ratings_by_title.len();
    let rating_source_rows = ratings_by_title
        .values()
        .map(|ratings| ratings.rating_sources.len())
        .sum();
    let external_rating_rows = ratings_by_title
        .values()
        .map(|ratings| ratings.external_ratings.len())
        .sum();
    for (discovery_title_id, ratings) in ratings_by_title {
        let Some(indexes) = title_indexes.get(&discovery_title_id) else {
            continue;
        };
        for index in indexes {
            items[*index].rating = ratings.rating;
            items[*index].rating_sources = ratings.rating_sources.clone();
            items[*index].external_ratings = ratings.external_ratings.clone();
        }
    }
    Ok((
        rating_summary_rows,
        rating_source_rows,
        external_rating_rows,
    ))
}

async fn hydrate_title_external_ids(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    discovery_title_ids: &[String],
    title_indexes: &HashMap<String, Vec<usize>>,
) -> AppResult<usize> {
    let rows = fetch_child_rows(
        datastore,
        "SELECT discovery_title_id, source, external_kind, external_id, external_key, sort_index
         FROM discovery_title_external_ids
         WHERE discovery_title_id IN ({})
         ORDER BY discovery_title_id ASC, sort_index ASC, source ASC, external_kind ASC",
        discovery_title_ids,
    )
    .await?;
    let row_count = rows.len();
    for row in rows {
        let discovery_title_id = row.text("discovery_title_id")?;
        let Some(indexes) = title_indexes.get(&discovery_title_id) else {
            continue;
        };
        let external_id = DiscoveryExternalIdRecord {
            source: row.text("source")?,
            kind: row.text("external_kind")?,
            id: row.text("external_id")?,
            key: row.text("external_key")?,
        };
        for index in indexes {
            items[*index].external_ids.push(external_id.clone());
        }
    }
    Ok(row_count)
}

async fn hydrate_item_rank_components(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    item_ids: &[String],
    item_indexes: &HashMap<String, usize>,
) -> AppResult<usize> {
    let rows = fetch_child_rows(
        datastore,
        "SELECT item_id, component_index, component_name, component_value
         FROM discovery_item_rank_components
         WHERE item_id IN ({})
         ORDER BY item_id ASC, component_index ASC",
        item_ids,
    )
    .await?;
    let row_count = rows.len();
    for row in rows {
        let item_id = row.text("item_id")?;
        let Some(index) = item_indexes.get(&item_id).copied() else {
            continue;
        };
        items[index]
            .rank_components
            .push(DiscoveryRankComponentRecord {
                component_index: row.i32("component_index")?,
                component_name: empty_to_none(row.text("component_name")?),
                component_value: empty_to_none(row.text("component_value")?),
            });
    }
    Ok(row_count)
}

async fn hydrate_item_subject_links(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    item_ids: &[String],
    item_indexes: &HashMap<String, usize>,
) -> AppResult<usize> {
    let rows = fetch_child_rows(
        datastore,
        "SELECT item_id, link_type, subject_key, sort_index
         FROM discovery_item_subject_links
         WHERE item_id IN ({})
         ORDER BY item_id ASC, link_type ASC, sort_index ASC",
        item_ids,
    )
    .await?;
    let row_count = rows.len();
    for row in rows {
        let item_id = row.text("item_id")?;
        let Some(index) = item_indexes.get(&item_id).copied() else {
            continue;
        };
        let link_type = row.text("link_type")?;
        let subject_key = row.text("subject_key")?;
        match link_type.as_str() {
            "matched" => items[index].matched_subject_keys.push(subject_key),
            "change" => items[index].change_subject_keys.push(subject_key),
            "removed" => items[index].removed_subject_keys.push(subject_key),
            _ => {}
        }
    }
    Ok(row_count)
}

async fn hydrate_item_library_provenance(
    datastore: &StoreDatastore,
    items: &mut [DiscoveryItemRecord],
    item_ids: &[String],
    item_indexes: &HashMap<String, usize>,
) -> AppResult<usize> {
    let rows = fetch_child_rows(
        datastore,
        "SELECT item_id, subject_key, title_id, library_id
         FROM discovery_item_library_provenance
         WHERE item_id IN ({})
         ORDER BY item_id ASC, subject_key ASC, library_id ASC, title_id ASC",
        item_ids,
    )
    .await?;
    let row_count = rows.len();
    for row in rows {
        let item_id = row.text("item_id")?;
        let Some(index) = item_indexes.get(&item_id).copied() else {
            continue;
        };
        items[index]
            .library_provenance
            .push(DiscoveryItemLibraryProvenanceRecord {
                subject_key: row.text("subject_key")?,
                title_id: empty_to_none(row.text("title_id")?),
                library_id: empty_to_none(row.text("library_id")?),
            });
    }
    Ok(row_count)
}

async fn fetch_child_rows(
    datastore: &StoreDatastore,
    sql_template: &str,
    item_ids: &[String],
) -> AppResult<Vec<SqlRow>> {
    let sql = sql_template.replace("{}", &placeholders(item_ids.len()));
    let args = item_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args).await
}

fn submitted_subject_from_row(row: &SqlRow) -> AppResult<DiscoverySubmittedSubjectRecord> {
    Ok(DiscoverySubmittedSubjectRecord {
        run_id: row.text("run_id")?,
        subject_key: row.text("subject_key")?,
        title_id: row.opt_text("title_id")?,
        library_id: row.opt_text("library_id")?,
        library_facet: row.opt_text("library_facet")?,
        title_kind: row.opt_text("title_kind")?,
        display_title: row.opt_text("display_title")?,
        external_ids_json: json_text(row, "external_ids_json")?,
        raw_subject_json: json_text(row, "raw_subject_json")?,
    })
}

fn json_text(row: &SqlRow, column: &str) -> AppResult<String> {
    row.opt_json(column)?
        .map(|value| serde_json::to_string(&value).map_err(repo_err))
        .transpose()
        .map(|value| value.unwrap_or_else(|| JsonValue::Null.to_string()))
}

async fn insert_submitted_subject_tx(
    tx: &mut SqlTx<'_>,
    datastore: &StoreDatastore,
    subject: &DiscoverySubmittedSubjectRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO discovery_submitted_subjects
         (run_id, subject_key, title_id, library_id, library_facet, title_kind, display_title,
          external_ids_json, raw_subject_json)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(subject.run_id.clone()),
            SqlArg::Text(subject.subject_key.clone()),
            SqlArg::OptText(subject.title_id.clone()),
            SqlArg::OptText(subject.library_id.clone()),
            SqlArg::OptText(subject.library_facet.clone()),
            SqlArg::OptText(subject.title_kind.clone()),
            SqlArg::OptText(subject.display_title.clone()),
            json_arg(datastore, &subject.external_ids_json)?,
            json_arg(datastore, &subject.raw_subject_json)?,
        ],
    )
    .await?;
    Ok(())
}

async fn insert_section_tx(
    tx: &mut SqlTx<'_>,
    _datastore: &StoreDatastore,
    section: &DiscoverySectionRecord,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &insert_sql("discovery_sections", SECTION_COLUMNS),
        &[
            SqlArg::Text(section.id.clone()),
            SqlArg::Text(section.run_id.clone()),
            SqlArg::Text(section.section_id.clone()),
            SqlArg::Text(section.section_type.clone()),
            SqlArg::Text(section.surface.clone()),
            SqlArg::Text(section.title.clone()),
            SqlArg::I32(section.sort_index),
            SqlArg::Timestamp(section.created_at),
            SqlArg::Timestamp(section.updated_at),
        ],
    )
    .await?;
    Ok(())
}

fn log_discovery_public_feed_persistence_failure(
    operation: &'static str,
    commit: &DiscoveryPublicFeedCommit,
    error: &impl std::fmt::Display,
) {
    tracing::warn!(
        operation,
        run_id = %commit.run.id,
        scope_key = %commit.state.scope_key,
        section_count = commit.sections.len(),
        item_count = commit.items.len(),
        error = %error,
        "failed to persist discovery public feed"
    );
}

fn log_discovery_item_persistence_failure(
    operation: &'static str,
    item: &DiscoveryItemRecord,
    error: &impl std::fmt::Display,
) {
    tracing::warn!(
        operation,
        run_id = %item.run_id,
        item_id = %item.id,
        target_key = %item.target_key,
        target_kind = %item.target_kind,
        section_id = ?item.section_id,
        source_run_kind = %item.source_run_kind,
        error = %error,
        "failed to persist discovery item"
    );
}

async fn insert_item_tx(
    tx: &mut SqlTx<'_>,
    _datastore: &StoreDatastore,
    item: &DiscoveryItemRecord,
    language: &str,
) -> AppResult<()> {
    let discovery_title_id = upsert_discovery_title_tx(tx, item, language, true, true).await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        &insert_sql("discovery_items", OCCURRENCE_COLUMNS),
        &occurrence_args(item, &discovery_title_id),
    )
    .await
    .inspect_err(|error| {
        log_discovery_item_persistence_failure("insert_discovery_item", item, error);
    })?;
    insert_item_children_tx(tx, item)
        .await
        .inspect_err(|error| {
            log_discovery_item_persistence_failure("insert_discovery_item_children", item, error);
        })?;
    Ok(())
}

async fn delete_title_more_like_this_items_tx(tx: &mut SqlTx<'_>, title_id: &str) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM title_more_like_this_items WHERE source_title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    Ok(())
}

async fn delete_unreferenced_discovery_titles_tx(tx: &mut SqlTx<'_>) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM discovery_titles
         WHERE NOT EXISTS (
            SELECT 1
            FROM discovery_items i
            WHERE i.discovery_title_id = discovery_titles.id
         )
         AND NOT EXISTS (
            SELECT 1
            FROM title_recommendation_cards c
            WHERE c.discovery_title_id = discovery_titles.id
              AND c.payload_blob IS NULL
         )",
        &[],
    )
    .await?;
    Ok(())
}

async fn delete_orphan_title_recommendation_cards_tx(tx: &mut SqlTx<'_>) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM title_recommendation_cards
         WHERE NOT EXISTS (
            SELECT 1
            FROM title_more_like_this_items m
            WHERE m.discovery_title_id = title_recommendation_cards.discovery_title_id
         )",
        &[],
    )
    .await?;
    Ok(())
}

async fn upsert_title_recommendation_card_tx(
    tx: &mut SqlTx<'_>,
    discovery_title_id: &str,
    item: &DiscoveryItemRecord,
) -> AppResult<()> {
    let payload_blob = encode_compressed_json(item)?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO title_recommendation_cards
         (discovery_title_id, payload_version, payload_blob, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {})
         ON CONFLICT(discovery_title_id) DO UPDATE SET
            payload_version = excluded.payload_version,
            payload_blob = excluded.payload_blob,
            updated_at = excluded.updated_at",
        &[
            SqlArg::Text(discovery_title_id.to_string()),
            SqlArg::I32(TITLE_RECOMMENDATION_PAYLOAD_VERSION),
            SqlArg::OptBytes(Some(payload_blob)),
            SqlArg::Timestamp(item.created_at),
            SqlArg::Timestamp(item.updated_at),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_title_more_like_this_item_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    item: &DiscoveryItemRecord,
    language: &str,
) -> AppResult<()> {
    let discovery_title_id = discovery_title_id_for(
        &discovery_title_target_key_norm(item),
        &normalize_discovery_language(language),
    );
    if SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id FROM discovery_titles WHERE id = {}",
        &[SqlArg::Text(discovery_title_id.clone())],
    )
    .await?
    .is_some()
    {
        upsert_discovery_title_tx(tx, item, language, false, false).await?;
    }
    upsert_title_recommendation_card_tx(tx, &discovery_title_id, item).await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO title_more_like_this_items
         (source_title_id, discovery_title_id, sort_index, rank_score, best_source,
          source_count, edge_count, relation_count, source_subject_count, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(source_title_id, discovery_title_id) DO UPDATE SET
            sort_index = excluded.sort_index,
            rank_score = excluded.rank_score,
            best_source = excluded.best_source,
            source_count = excluded.source_count,
            edge_count = excluded.edge_count,
            relation_count = excluded.relation_count,
            source_subject_count = excluded.source_subject_count,
            updated_at = excluded.updated_at",
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(discovery_title_id),
            SqlArg::I32(item.sort_index),
            SqlArg::OptF64(item.rank_score),
            SqlArg::OptText(item.best_source.clone()),
            SqlArg::OptI32(item.source_count),
            SqlArg::OptI32(item.edge_count),
            SqlArg::OptI32(item.relation_count),
            SqlArg::OptI32(item.source_subject_count),
            SqlArg::Timestamp(item.created_at),
            SqlArg::Timestamp(item.updated_at),
        ],
    )
    .await?;
    Ok(())
}

fn facet_row(facet: &DiscoveryFacetRecord) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(facet.run_id.clone()),
        SqlArg::Text(facet.facet_name.clone()),
        SqlArg::Text(facet.facet_value.clone()),
        SqlArg::OptI64(facet.smg_count),
        SqlArg::OptI64(facet.local_count),
    ]
}

fn insert_sql(table: &str, columns: &[&str]) -> String {
    format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        columns.join(", "),
        placeholders(columns.len())
    )
}

/// Column-list prefix (`INSERT INTO t (a, b, c)`) for `execute_batch_insert`,
/// which supplies the multi-row `VALUES (..),(..)` tail itself.
fn insert_into_prefix(table: &str, columns: &[&str]) -> String {
    format!("INSERT INTO {table} ({})", columns.join(", "))
}

async fn upsert_discovery_title_tx(
    tx: &mut SqlTx<'_>,
    item: &DiscoveryItemRecord,
    language: &str,
    replace_canonical_tags: bool,
    replace_canonical_ratings: bool,
) -> AppResult<String> {
    let language = normalize_discovery_language(language);
    let target_key_norm = discovery_title_target_key_norm(item);
    let discovery_title_id = discovery_title_id_for(&target_key_norm, &language);
    let args = title_args(item, &discovery_title_id, &target_key_norm, &language)?;
    SqlRuntime::execute(SqlExec::Tx(tx), &upsert_discovery_title_sql(), &args)
        .await
        .inspect_err(|error| {
            log_discovery_item_persistence_failure("upsert_discovery_title", item, error);
        })?;
    if replace_canonical_tags {
        replace_discovery_title_metadata_tags_tx(tx, &discovery_title_id, &item.canonical_tags)
            .await
            .inspect_err(|error| {
                log_discovery_item_persistence_failure(
                    "replace_discovery_title_canonical_tags",
                    item,
                    error,
                );
            })?;
    }
    let ratings = TitleRatingSummary {
        rating: item.rating,
        rating_sources: item.rating_sources.clone(),
        external_ratings: item.external_ratings.clone(),
    };
    if replace_canonical_ratings
        || ratings.rating.is_some()
        || !ratings.rating_sources.is_empty()
        || !ratings.external_ratings.is_empty()
    {
        replace_discovery_title_metadata_ratings_tx(tx, &discovery_title_id, &ratings)
            .await
            .inspect_err(|error| {
                log_discovery_item_persistence_failure(
                    "replace_discovery_title_ratings",
                    item,
                    error,
                );
            })?;
    }
    insert_title_children_tx(tx, item, &discovery_title_id)
        .await
        .inspect_err(|error| {
            log_discovery_item_persistence_failure("insert_discovery_title_children", item, error);
        })?;
    Ok(discovery_title_id)
}

fn discovery_content_ratings_from_json(raw: &str) -> AppResult<Vec<DiscoveryContentRating>> {
    serde_json::from_str(raw).map_err(repo_err)
}

fn title_args(
    item: &DiscoveryItemRecord,
    discovery_title_id: &str,
    target_key_norm: &str,
    language: &str,
) -> AppResult<Vec<SqlArg>> {
    let content_ratings_json = serde_json::to_string(&item.content_ratings).map_err(repo_err)?;
    Ok(vec![
        SqlArg::Text(discovery_title_id.to_string()),
        SqlArg::Text(item.target_key.clone()),
        SqlArg::Text(target_key_norm.to_string()),
        SqlArg::Text(language.to_string()),
        SqlArg::Text(item.target_kind.clone()),
        SqlArg::Bool(item.resolved),
        SqlArg::OptText(item.resolved_title_id.clone()),
        SqlArg::Text(item.display_title.clone()),
        SqlArg::OptText(item.original_title.clone()),
        SqlArg::OptText(item.sort_title.clone()),
        SqlArg::OptI32(item.year),
        SqlArg::OptText(item.poster_path.clone()),
        SqlArg::OptText(item.poster_url.clone()),
        SqlArg::OptText(item.background_url.clone()),
        SqlArg::OptText(item.overview.clone()),
        SqlArg::OptText(item.content_type.clone()),
        SqlArg::Bool(item.is_adult),
        SqlArg::Text(content_ratings_json),
        SqlArg::OptText(item.tmdb_collection_id.clone()),
        SqlArg::OptText(item.tmdb_collection_name.clone()),
        SqlArg::Timestamp(item.created_at),
        SqlArg::Timestamp(item.updated_at),
    ])
}

fn occurrence_args(item: &DiscoveryItemRecord, discovery_title_id: &str) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(item.id.clone()),
        SqlArg::Text(item.run_id.clone()),
        SqlArg::OptText(item.base_generation_id.clone()),
        SqlArg::Text(discovery_title_id.to_string()),
        SqlArg::Text(item.source_run_kind.clone()),
        SqlArg::OptText(item.section_id.clone()),
        SqlArg::I32(item.sort_index),
        SqlArg::OptText(item.best_source.clone()),
        SqlArg::OptI32(item.source_count),
        SqlArg::OptI32(item.edge_count),
        SqlArg::OptI32(item.relation_count),
        SqlArg::OptI32(item.source_subject_count),
        SqlArg::OptF64(item.rank_score),
        SqlArg::I32(item.matched_subject_count),
        SqlArg::Bool(item.owned_in_input),
        SqlArg::OptText(item.tombstoned_by_run_id.clone()),
        SqlArg::OptTimestamp(item.tombstoned_at),
        SqlArg::Timestamp(item.created_at),
        SqlArg::Timestamp(item.updated_at),
    ]
}

async fn insert_item_children_tx(tx: &mut SqlTx<'_>, item: &DiscoveryItemRecord) -> AppResult<()> {
    if let Some(section_id) = item.section_id.as_deref() {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO discovery_section_items
             (run_id, section_id, item_id, sort_index)
             VALUES ({}, {}, {}, {})",
            &[
                SqlArg::Text(item.run_id.clone()),
                SqlArg::Text(section_id.to_string()),
                SqlArg::Text(item.id.clone()),
                SqlArg::I32(item.sort_index),
            ],
        )
        .await?;
    }

    let rank_rows: Vec<Vec<SqlArg>> = item
        .rank_components
        .iter()
        .map(|component| rank_component_row(item, component))
        .collect();
    SqlRuntime::execute_batch_insert(
        tx,
        "INSERT INTO discovery_item_rank_components \
         (item_id, run_id, component_index, component_name, component_value)",
        5,
        rank_rows,
        "ON CONFLICT DO NOTHING",
    )
    .await?;

    insert_subject_links_tx(tx, item, "matched", &item.matched_subject_keys).await?;
    insert_subject_links_tx(tx, item, "change", &item.change_subject_keys).await?;
    insert_subject_links_tx(tx, item, "removed", &item.removed_subject_keys).await?;

    let provenance_rows: Vec<Vec<SqlArg>> = item
        .library_provenance
        .iter()
        .map(|provenance| library_provenance_row(item, provenance))
        .collect();
    SqlRuntime::execute_batch_insert(
        tx,
        "INSERT INTO discovery_item_library_provenance \
         (item_id, run_id, subject_key, title_id, library_id)",
        5,
        provenance_rows,
        "ON CONFLICT DO NOTHING",
    )
    .await?;
    Ok(())
}

async fn insert_title_children_tx(
    tx: &mut SqlTx<'_>,
    item: &DiscoveryItemRecord,
    discovery_title_id: &str,
) -> AppResult<()> {
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "status_tag",
        None,
        &item.status_tags,
    )
    .await?;
    insert_title_terms_tx(tx, discovery_title_id, "source", None, &item.sources).await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "relation_type",
        None,
        &item.relation_types,
    )
    .await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "relation_subtype",
        None,
        &item.relation_subtypes,
    )
    .await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "chart_signal",
        None,
        &item.chart_signals,
    )
    .await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "provider_signal",
        None,
        &item.provider_signals,
    )
    .await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "facet_term",
        None,
        &item.facet_terms,
    )
    .await?;
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "context_term",
        None,
        &item.context_terms,
    )
    .await?;
    if let Some(media_kind) = discovery_item_authoritative_media_kind(item) {
        insert_title_terms_tx(tx, discovery_title_id, "media_kind", None, &[media_kind]).await?;
    }
    // Project SMG studio_slug / person_ids into the reverse-indexed
    // terms table so "more from studio / person" lookups reuse the existing
    // idx_discovery_title_terms_kind_value index (zero schema migration).
    if let Some(studio_slug) = &item.studio_slug {
        insert_title_terms_tx(
            tx,
            discovery_title_id,
            "studio",
            None,
            std::slice::from_ref(studio_slug),
        )
        .await?;
    }
    if !item.person_ids.is_empty() {
        let person_values = item
            .person_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>();
        insert_title_terms_tx(tx, discovery_title_id, "person", None, &person_values).await?;
    }
    for (index, source_tag) in item.source_tags.iter().enumerate() {
        insert_title_source_tag_tx(tx, discovery_title_id, source_tag, index as i32).await?;
    }
    for (index, external_id) in item.external_ids.iter().enumerate() {
        insert_title_external_id_tx(tx, discovery_title_id, external_id, index as i32).await?;
    }
    Ok(())
}

async fn insert_title_terms_tx(
    tx: &mut SqlTx<'_>,
    discovery_title_id: &str,
    term_kind: &str,
    term_category: Option<&str>,
    values: &[String],
) -> AppResult<()> {
    // Accumulate one row per non-empty term, preserving the trim/skip-empty
    // rule and the original enumerate index as sort_index, then batch-insert.
    let mut rows: Vec<Vec<SqlArg>> = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        rows.push(vec![
            SqlArg::Text(discovery_title_id.to_string()),
            SqlArg::Text(term_kind.to_string()),
            SqlArg::Text(storage_text(term_category)),
            SqlArg::Text(value.to_string()),
            SqlArg::I32(index as i32),
        ]);
    }
    SqlRuntime::execute_batch_insert(
        tx,
        "INSERT INTO discovery_title_terms \
         (discovery_title_id, term_kind, term_category, term_value, sort_index)",
        5,
        rows,
        "ON CONFLICT DO NOTHING",
    )
    .await?;
    Ok(())
}

async fn insert_title_source_tag_tx(
    tx: &mut SqlTx<'_>,
    discovery_title_id: &str,
    source_tag: &DiscoverySourceTagRecord,
    index: i32,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO discovery_title_source_tags
         (discovery_title_id, category, name, sort_index)
         VALUES ({}, {}, {}, {})
         ON CONFLICT DO NOTHING",
        &[
            SqlArg::Text(discovery_title_id.to_string()),
            SqlArg::Text(storage_text(source_tag.category.as_deref())),
            SqlArg::Text(storage_text(source_tag.name.as_deref())),
            SqlArg::I32(index),
        ],
    )
    .await?;
    if let Some(name) = source_tag.name.as_deref() {
        insert_title_terms_tx(
            tx,
            discovery_title_id,
            "source_tag",
            source_tag.category.as_deref(),
            &[name.to_string()],
        )
        .await?;
    }
    insert_title_terms_tx(
        tx,
        discovery_title_id,
        "source_tag_value",
        source_tag.category.as_deref(),
        &source_tag.values,
    )
    .await?;
    // Accumulate one row per non-empty value (preserving the trim/skip-empty
    // rule and the original enumerate index as value_sort_index), then batch.
    let mut value_rows: Vec<Vec<SqlArg>> = Vec::with_capacity(source_tag.values.len());
    for (value_index, value) in source_tag.values.iter().enumerate() {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        value_rows.push(vec![
            SqlArg::Text(discovery_title_id.to_string()),
            SqlArg::I32(index),
            SqlArg::Text(value.to_string()),
            SqlArg::I32(value_index as i32),
        ]);
    }
    SqlRuntime::execute_batch_insert(
        tx,
        "INSERT INTO discovery_title_source_tag_values \
         (discovery_title_id, source_tag_sort_index, source_tag_value, value_sort_index)",
        4,
        value_rows,
        "ON CONFLICT DO NOTHING",
    )
    .await?;
    Ok(())
}

async fn insert_title_external_id_tx(
    tx: &mut SqlTx<'_>,
    discovery_title_id: &str,
    external_id: &DiscoveryExternalIdRecord,
    index: i32,
) -> AppResult<()> {
    let source = external_id.source.trim();
    let id = external_id.id.trim();
    let key = external_id.key.trim();
    let identity = if id.is_empty() { key } else { id };
    if source.is_empty() || identity.is_empty() {
        return Ok(());
    }
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO discovery_title_external_ids
         (discovery_title_id, source, external_kind, external_id, external_key, external_identity, sort_index)
         VALUES ({}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(discovery_title_id, source, external_kind, external_identity)
         DO UPDATE SET
            sort_index = CASE
                WHEN discovery_title_external_ids.sort_index <= excluded.sort_index
                    THEN discovery_title_external_ids.sort_index
                ELSE excluded.sort_index
            END",
        &[
            SqlArg::Text(discovery_title_id.to_string()),
            SqlArg::Text(source.to_ascii_lowercase()),
            SqlArg::Text(external_id.kind.trim().to_ascii_lowercase()),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(key.to_string()),
            SqlArg::Text(identity.to_string()),
            SqlArg::I32(index),
        ],
    )
    .await?;
    Ok(())
}

fn rank_component_row(
    item: &DiscoveryItemRecord,
    component: &DiscoveryRankComponentRecord,
) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(item.id.clone()),
        SqlArg::Text(item.run_id.clone()),
        SqlArg::I32(component.component_index),
        SqlArg::Text(storage_text(component.component_name.as_deref())),
        SqlArg::Text(storage_text(component.component_value.as_deref())),
    ]
}

async fn insert_subject_links_tx(
    tx: &mut SqlTx<'_>,
    item: &DiscoveryItemRecord,
    link_type: &str,
    subject_keys: &[String],
) -> AppResult<()> {
    // Accumulate one row per non-empty subject key, preserving the
    // trim/skip-empty rule and the original enumerate index as sort_index,
    // then batch-insert.
    let mut rows: Vec<Vec<SqlArg>> = Vec::with_capacity(subject_keys.len());
    for (index, subject_key) in subject_keys.iter().enumerate() {
        let subject_key = subject_key.trim();
        if subject_key.is_empty() {
            continue;
        }
        rows.push(vec![
            SqlArg::Text(item.id.clone()),
            SqlArg::Text(item.run_id.clone()),
            SqlArg::Text(link_type.to_string()),
            SqlArg::Text(subject_key.to_string()),
            SqlArg::I32(index as i32),
        ]);
    }
    SqlRuntime::execute_batch_insert(
        tx,
        "INSERT INTO discovery_item_subject_links \
         (item_id, run_id, link_type, subject_key, sort_index)",
        5,
        rows,
        "ON CONFLICT DO NOTHING",
    )
    .await?;
    Ok(())
}

fn library_provenance_row(
    item: &DiscoveryItemRecord,
    provenance: &DiscoveryItemLibraryProvenanceRecord,
) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(item.id.clone()),
        SqlArg::Text(item.run_id.clone()),
        SqlArg::Text(provenance.subject_key.clone()),
        SqlArg::Text(storage_text(provenance.title_id.as_deref())),
        SqlArg::Text(storage_text(provenance.library_id.as_deref())),
    ]
}

fn json_arg(datastore: &StoreDatastore, raw: &str) -> AppResult<SqlArg> {
    match datastore {
        StoreDatastore::Sqlite { .. } => Ok(SqlArg::Text(raw.to_string())),
        StoreDatastore::Postgres { .. } => serde_json::from_str::<JsonValue>(raw)
            .map(SqlArg::Json)
            .map_err(repo_err),
    }
}

fn opt_json_arg(datastore: &StoreDatastore, raw: Option<&str>) -> AppResult<SqlArg> {
    match (datastore, raw) {
        (StoreDatastore::Sqlite { .. }, Some(raw)) => Ok(SqlArg::OptText(Some(raw.to_string()))),
        (StoreDatastore::Sqlite { .. }, None) => Ok(SqlArg::OptText(None)),
        (StoreDatastore::Postgres { .. }, Some(raw)) => serde_json::from_str::<JsonValue>(raw)
            .map(|value| SqlArg::OptJson(Some(value)))
            .map_err(repo_err),
        (StoreDatastore::Postgres { .. }, None) => Ok(SqlArg::OptJson(None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_application::{
        AppError, DISCOVERY_DEFAULT_SCOPE_KEY, DiscoveryContentCertification, DiscoveryItemsQuery,
        DiscoveryItemsStorageQuery, TitleExternalRating,
    };
    use scryer_domain::{CanonicalMediaTag, Id};
    use scryer_infrastructure_datastore::postgres::PostgresServices;
    use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};
    use serde_json::json;

    fn canonical_genre_tags(labels: &[&str]) -> Vec<CanonicalMediaTag> {
        labels
            .iter()
            .map(|label| CanonicalMediaTag {
                key: format!(
                    "canonical:genre:{}",
                    label.trim().to_ascii_lowercase().replace(' ', "_")
                ),
                category: "genre".to_string(),
                name: (*label).to_string(),
                confidence: Some(1.0),
                sources: Vec::new(),
                source_tag_keys: Vec::new(),
                is_adult: false,
                is_spoiler: false,
            })
            .collect()
    }

    async fn discovery_title_tag_count(pool: &sqlx::SqlitePool, target_key_norm: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*)
               FROM discovery_title_metadata_tags tags
               JOIN discovery_titles titles ON titles.id = tags.discovery_title_id
              WHERE titles.target_key_norm = ?",
        )
        .bind(target_key_norm)
        .fetch_one(pool)
        .await
        .expect("discovery title tag count should load")
    }

    async fn discovery_title_rating_row_count(
        pool: &sqlx::SqlitePool,
        target_key_norm: &str,
    ) -> i64 {
        sqlx::query_scalar(
            "SELECT
                (SELECT COUNT(*)
                   FROM discovery_title_metadata_rating_summaries ratings
                   JOIN discovery_titles titles ON titles.id = ratings.discovery_title_id
                  WHERE titles.target_key_norm = ?)
              + (SELECT COUNT(*)
                   FROM discovery_title_metadata_rating_sources ratings
                   JOIN discovery_titles titles ON titles.id = ratings.discovery_title_id
                  WHERE titles.target_key_norm = ?)
              + (SELECT COUNT(*)
                   FROM discovery_title_metadata_external_ratings ratings
                   JOIN discovery_titles titles ON titles.id = ratings.discovery_title_id
                  WHERE titles.target_key_norm = ?)",
        )
        .bind(target_key_norm)
        .bind(target_key_norm)
        .bind(target_key_norm)
        .fetch_one(pool)
        .await
        .expect("discovery title rating row count should load")
    }

    #[test]
    fn canonical_facet_filter_values_accepts_labels_and_terms() {
        let values = canonical_facet_filter_values(
            "genre",
            &[
                "Action".to_string(),
                "Science Fiction".to_string(),
                "canonical:genre:drama".to_string(),
            ],
        );

        assert!(values.contains(&"canonical:genre:action".to_string()));
        assert!(values.contains(&"canonical:genre:science_fiction".to_string()));
        assert!(values.contains(&"canonical:genre:science-fiction".to_string()));
        assert!(values.contains(&"canonical:genre:science fiction".to_string()));
        assert!(values.contains(&"canonical:genre:drama".to_string()));
    }

    fn sqlite_projection_datastore() -> StoreDatastore {
        StoreDatastore::Sqlite {
            pool: sqlx::SqlitePool::connect_lazy("sqlite::memory:")
                .expect("lazy sqlite pool should build"),
            writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    fn postgres_projection_datastore() -> StoreDatastore {
        StoreDatastore::Postgres {
            pool: sqlx::PgPool::connect_lazy("postgres://localhost/scryer")
                .expect("lazy postgres pool should build"),
        }
    }

    #[tokio::test]
    async fn title_more_like_this_projection_does_not_read_legacy_discovery_rating() {
        let sqlite = sqlite_projection_datastore();
        let postgres = postgres_projection_datastore();
        let sqlite_projection = title_more_like_this_projection(&sqlite);
        let postgres_projection = title_more_like_this_projection(&postgres);

        assert!(sqlite_projection.contains("CAST(NULL AS REAL) AS rating"));
        assert!(postgres_projection.contains("CAST(NULL AS DOUBLE PRECISION) AS rating"));
        assert!(!sqlite_projection.contains("t.rating AS rating"));
        assert!(!postgres_projection.contains("t.rating AS rating"));
    }

    #[tokio::test]
    async fn discovery_item_upsert_empty_canonical_tags_clear_existing_tags() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_canonical_tags_clear_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let run_id = "run-canonical-tags-clear";

        store
            .upsert_discovery_sync_run(&discovery_prune_run(
                run_id,
                "context_snapshot",
                "complete",
                now,
            ))
            .await
            .expect("run should upsert");

        let mut item = discovery_prune_item(run_id, now);
        item.canonical_tags = canonical_genre_tags(&["Drama"]);
        store
            .replace_discovery_items(run_id, &[item.clone()])
            .await
            .expect("tagged item should upsert");
        assert_eq!(
            discovery_title_tag_count(&services.pool, "tmdb:movie:604").await,
            1
        );

        item.canonical_tags = Vec::new();
        store
            .replace_discovery_items(run_id, &[item])
            .await
            .expect("empty-tag item should upsert");
        assert_eq!(
            discovery_title_tag_count(&services.pool, "tmdb:movie:604").await,
            0
        );

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn sqlite_discovery_home_filter_options_deduplicate_by_exact_key() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_home_filter_options_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let run_id = "run-discovery-home-filter-options";
        let library_id = "movie-library";
        store
            .upsert_discovery_sync_run(&discovery_prune_run(
                run_id,
                "context_snapshot",
                "complete",
                now,
            ))
            .await
            .expect("run should upsert");

        let mut first = discovery_prune_item(run_id, now);
        first.id = format!("{run_id}:first");
        first.target_key = "tmdb:movie:201".to_string();
        first.poster_url = Some("https://example.com/poster-201.jpg".to_string());
        first.canonical_tags = canonical_genre_tags(&["Drama"]);
        first.canonical_tags.push(CanonicalMediaTag {
            key: "canonical:theme:found-family".to_string(),
            category: "theme".to_string(),
            name: "Found Family".to_string(),
            confidence: Some(1.0),
            sources: Vec::new(),
            source_tag_keys: Vec::new(),
            is_adult: false,
            is_spoiler: false,
        });
        first.studio_slug = Some("A24".to_string());
        first.library_provenance = vec![DiscoveryItemLibraryProvenanceRecord {
            subject_key: first.target_key.clone(),
            title_id: Some("library-title-201".to_string()),
            library_id: Some(library_id.to_string()),
        }];

        let mut second = first.clone();
        second.id = format!("{run_id}:second");
        second.target_key = "tmdb:movie:202".to_string();
        second.poster_url = Some("https://example.com/poster-202.jpg".to_string());
        second.canonical_tags[0].name = "drama".to_string();
        second.studio_slug = Some("a24".to_string());
        second.library_provenance[0].subject_key = second.target_key.clone();
        second.library_provenance[0].title_id = Some("library-title-202".to_string());

        let mut case_distinct = first.clone();
        case_distinct.id = format!("{run_id}:case-distinct");
        case_distinct.target_key = "tmdb:movie:203".to_string();
        case_distinct.canonical_tags[0].key = "Canonical:genre:drama".to_string();
        case_distinct.canonical_tags[0].name = "Drama Case Distinct".to_string();
        case_distinct.library_provenance[0].subject_key = case_distinct.target_key.clone();
        case_distinct.library_provenance[0].title_id = Some("library-title-203".to_string());

        store
            .replace_discovery_items(run_id, &[first, second, case_distinct])
            .await
            .expect("items should upsert");

        let options = fetch_discovery_home_filter_options(
            &store.datastore,
            None,
            Some(run_id),
            &[library_id.to_string()],
            &["movie".to_string()],
            true,
        )
        .await
        .expect("filter options should load");

        assert_eq!(options.genres.len(), 2);
        assert_eq!(options.themes.len(), 1);
        assert_eq!(options.studio_slugs.len(), 1);
        assert_eq!(options.genres[0].key, "canonical:genre:drama");
        assert_eq!(options.genres[0].name, "Drama");
        assert_eq!(options.genres[1].key, "Canonical:genre:drama");
        assert_eq!(options.genres[1].name, "Drama Case Distinct");
        assert_eq!(options.themes[0].key, "canonical:theme:found-family");
        assert_eq!(options.themes[0].name, "Found Family");
        assert_eq!(options.studio_slugs[0].to_ascii_lowercase(), "a24");

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn sqlite_personalized_home_minimum_rating_uses_effective_rating() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_personalized_home_filters_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let run_id = "run-personalized-home-filters";
        let library_id = "movie-library";
        store
            .upsert_discovery_sync_run(&discovery_prune_run(
                run_id,
                "context_snapshot",
                "complete",
                now,
            ))
            .await
            .expect("run should upsert");

        let tags = vec![
            CanonicalMediaTag {
                key: "canonical:genre:drama".to_string(),
                category: "genre".to_string(),
                name: "Drama".to_string(),
                confidence: Some(1.0),
                sources: Vec::new(),
                source_tag_keys: Vec::new(),
                is_adult: false,
                is_spoiler: false,
            },
            CanonicalMediaTag {
                key: "canonical:theme:noir".to_string(),
                category: "theme".to_string(),
                name: "Noir".to_string(),
                confidence: Some(1.0),
                sources: Vec::new(),
                source_tag_keys: Vec::new(),
                is_adult: false,
                is_spoiler: false,
            },
        ];
        let provenance = DiscoveryItemLibraryProvenanceRecord {
            subject_key: "tmdb:movie:100".to_string(),
            title_id: Some("library-title-100".to_string()),
            library_id: Some(library_id.to_string()),
        };
        let mut rated = discovery_prune_item(run_id, now);
        rated.id = format!("{run_id}:rated");
        rated.target_key = "tmdb:movie:100".to_string();
        rated.display_title = "Rated match".to_string();
        rated.sort_title = Some(rated.display_title.clone());
        rated.poster_url = Some("https://example.invalid/rated-match.jpg".to_string());
        rated.year = Some(2022);
        rated.rating = Some(9.5);
        rated.rank_score = Some(10.0);
        rated.canonical_tags = tags.clone();
        rated.relation_types = vec!["sequel".to_string()];
        rated.studio_slug = Some("a24".to_string());
        rated.library_provenance = vec![provenance.clone()];

        let mut external_high = rated.clone();
        external_high.id = format!("{run_id}:external-high");
        external_high.target_key = "tmdb:movie:101".to_string();
        external_high.display_title = "External high match".to_string();
        external_high.sort_title = Some(external_high.display_title.clone());
        external_high.year = None;
        external_high.rating = None;
        external_high.external_ratings = vec![TitleExternalRating {
            source: "trakt".to_string(),
            value: Some(9.0),
            score: None,
            normalized: 9.0,
            votes: Some(100),
            url: "https://trakt.tv/movies/external-high".to_string(),
        }];
        external_high.rank_score = Some(100.0);
        external_high.library_provenance[0].subject_key = "tmdb:movie:101".to_string();

        let mut external_low = external_high.clone();
        external_low.id = format!("{run_id}:external-low");
        external_low.target_key = "tmdb:movie:102".to_string();
        external_low.display_title = "External low match".to_string();
        external_low.sort_title = Some(external_low.display_title.clone());
        external_low.external_ratings[0].value = Some(7.5);
        external_low.external_ratings[0].normalized = 6.944_444_444_444_445;
        external_low.external_ratings[0].votes = Some(2);
        external_low.rank_score = Some(1_000.0);
        external_low.library_provenance[0].subject_key = "tmdb:movie:102".to_string();

        let mut unrated = rated.clone();
        unrated.id = format!("{run_id}:unrated");
        unrated.target_key = "tmdb:movie:103".to_string();
        unrated.display_title = "Unrated match".to_string();
        unrated.sort_title = Some(unrated.display_title.clone());
        unrated.year = None;
        unrated.rating = None;
        unrated.rank_score = Some(10_000.0);
        unrated.library_provenance[0].subject_key = "tmdb:movie:103".to_string();

        let mut excluded = rated.clone();
        excluded.id = format!("{run_id}:excluded");
        excluded.target_key = "tmdb:movie:104".to_string();
        excluded.display_title = "Wrong genre".to_string();
        excluded.sort_title = Some(excluded.display_title.clone());
        excluded.rank_score = Some(1_000.0);
        excluded.canonical_tags[0].key = "canonical:genre:comedy".to_string();
        excluded.canonical_tags[0].name = "Comedy".to_string();
        excluded.library_provenance[0].subject_key = "tmdb:movie:104".to_string();

        store
            .replace_discovery_items(
                run_id,
                &[rated, external_high, external_low, unrated, excluded],
            )
            .await
            .expect("items should upsert");

        let filters = DiscoveryHomeFilters {
            content_types: vec!["movie".to_string(), "series".to_string()],
            genre_tag_keys: vec![
                "canonical:genre:drama".to_string(),
                "canonical:genre:science-fiction".to_string(),
            ],
            theme_tag_keys: vec!["canonical:theme:noir".to_string()],
            studio_slugs: vec!["a24".to_string()],
            minimum_year: Some(2020),
            maximum_year: Some(2024),
            minimum_rating: Some(8.5),
        };
        let candidates = fetch_personalized_home_candidates(
            &store.datastore,
            run_id,
            &[library_id.to_string()],
            &["movie".to_string()],
            true,
            &filters,
            None,
            25,
        )
        .await
        .expect("filtered candidates should load");

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.item.target_key.as_str())
                .collect::<Vec<_>>(),
            vec!["tmdb:movie:101", "tmdb:movie:100"],
            "minimum rating must use the normalized external score before the blended rating"
        );

        let mut no_rating_filter = filters.clone();
        no_rating_filter.minimum_rating = None;
        let candidates_without_minimum = fetch_personalized_home_candidates(
            &store.datastore,
            run_id,
            &[library_id.to_string()],
            &["movie".to_string()],
            true,
            &no_rating_filter,
            None,
            25,
        )
        .await
        .expect("unfiltered candidates should load");
        let candidate_keys_without_minimum = candidates_without_minimum
            .iter()
            .map(|candidate| candidate.item.target_key.as_str())
            .collect::<Vec<_>>();
        assert!(candidate_keys_without_minimum.contains(&"tmdb:movie:102"));
        assert!(candidate_keys_without_minimum.contains(&"tmdb:movie:103"));

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn discovery_item_upsert_empty_canonical_ratings_clear_existing_ratings() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_canonical_ratings_clear_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let run_id = "run-canonical-ratings-clear";

        store
            .upsert_discovery_sync_run(&discovery_prune_run(
                run_id,
                "context_snapshot",
                "complete",
                now,
            ))
            .await
            .expect("run should upsert");

        let mut item = discovery_prune_item(run_id, now);
        item.rating = Some(7.5);
        item.rating_sources = vec!["tmdb".to_string()];
        item.external_ratings = vec![TitleExternalRating {
            source: "imdb".to_string(),
            value: Some(8.2),
            score: Some(82.0),
            normalized: 0.82,
            votes: Some(1234),
            url: "https://www.imdb.com/title/tt0000604/".to_string(),
        }];
        item.is_adult = true;
        item.content_ratings = vec![DiscoveryContentRating {
            country: "US".to_string(),
            certifications: vec![DiscoveryContentCertification {
                value: "R".to_string(),
                source: "tmdb".to_string(),
                release_type: Some(3),
            }],
            age_rating: Some(17),
            age_rating_source: Some("tmdb".to_string()),
        }];
        store
            .replace_discovery_items(run_id, &[item.clone()])
            .await
            .expect("rated item should upsert");
        assert_eq!(
            discovery_title_rating_row_count(&services.pool, "tmdb:movie:604").await,
            3
        );
        let stored_content_classification: (i64, String) = sqlx::query_as(
            "SELECT is_adult, content_ratings_json
               FROM discovery_titles
              WHERE target_key_norm = 'tmdb:movie:604'",
        )
        .fetch_one(&services.pool)
        .await
        .expect("stored content classification should load");
        assert_eq!(stored_content_classification.0, 1);
        assert!(
            stored_content_classification
                .1
                .contains("\"country\":\"US\"")
        );

        item.rating = None;
        item.rating_sources.clear();
        item.external_ratings.clear();
        item.is_adult = false;
        item.content_ratings.clear();
        store
            .replace_discovery_items(run_id, &[item])
            .await
            .expect("empty-rating item should upsert");
        assert_eq!(
            discovery_title_rating_row_count(&services.pool, "tmdb:movie:604").await,
            0
        );
        let cleared_content_classification: (i64, String) = sqlx::query_as(
            "SELECT is_adult, content_ratings_json
               FROM discovery_titles
              WHERE target_key_norm = 'tmdb:movie:604'",
        )
        .fetch_one(&services.pool)
        .await
        .expect("cleared content classification should load");
        assert_eq!(cleared_content_classification, (0, "[]".to_string()));

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn discovery_item_projects_studio_slug_and_person_ids_into_terms() {
        // studio_slug + person_ids project into discovery_title_terms
        // (term_kind 'studio' / 'person') on write and hydrate back onto the item.
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_studio_person_terms_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let run_id = "run-studio-person";

        store
            .upsert_discovery_sync_run(&discovery_prune_run(
                run_id,
                "context_snapshot",
                "complete",
                now,
            ))
            .await
            .expect("run should upsert");

        let mut item = discovery_prune_item(run_id, now);
        item.studio_slug = Some("a24".to_string());
        item.person_ids = vec![101, 202];
        store
            .replace_discovery_items(run_id, &[item])
            .await
            .expect("item should upsert");

        let studio_terms: Vec<String> = sqlx::query_scalar(
            "SELECT term_value FROM discovery_title_terms
              WHERE term_kind = 'studio' ORDER BY sort_index ASC",
        )
        .fetch_all(&services.pool)
        .await
        .expect("studio terms should load");
        assert_eq!(studio_terms, vec!["a24".to_string()]);

        let person_terms: Vec<String> = sqlx::query_scalar(
            "SELECT term_value FROM discovery_title_terms
              WHERE term_kind = 'person' ORDER BY sort_index ASC",
        )
        .fetch_all(&services.pool)
        .await
        .expect("person terms should load");
        assert_eq!(person_terms, vec!["101".to_string(), "202".to_string()]);

        let hydrated = store
            .list_discovery_items_for_generation(run_id)
            .await
            .expect("items should hydrate");
        assert_eq!(hydrated.len(), 1);
        assert_eq!(hydrated[0].studio_slug.as_deref(), Some("a24"));
        assert_eq!(hydrated[0].person_ids, vec![101, 202]);

        let _ = std::fs::remove_file(db);
    }

    #[test]
    fn canonical_facet_display_value_accepts_only_genre_and_theme_terms() {
        assert_eq!(
            canonical_facet_display_value("canonical:genre:science_fiction"),
            Some(("genre".to_string(), "Science Fiction".to_string()))
        );
        assert_eq!(
            canonical_facet_display_value("canonical:theme:psychological"),
            Some(("theme".to_string(), "Psychological".to_string()))
        );
        assert_eq!(canonical_facet_display_value("Drama"), None);
        assert_eq!(
            canonical_facet_display_value("mal:theme:psychological"),
            None
        );
        assert_eq!(canonical_facet_display_value("canonical:source:tmdb"), None);
    }

    #[tokio::test]
    async fn sqlite_public_sections_filter_identifier_only_discovery_titles() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_displayable_store_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let run_id = "run-displayable";

        store
            .upsert_discovery_sync_run(&discovery_prune_run(run_id, "public_feed", "complete", now))
            .await
            .expect("run should upsert");
        store
            .replace_discovery_sections(
                run_id,
                &[DiscoverySectionRecord {
                    id: "section-row-displayable".to_string(),
                    run_id: run_id.to_string(),
                    section_id: "popular".to_string(),
                    section_type: "POPULAR_RIGHT_NOW".to_string(),
                    surface: "public".to_string(),
                    title: "Popular Right Now".to_string(),
                    sort_index: 0,
                    created_at: now,
                    updated_at: now,
                }],
            )
            .await
            .expect("section should replace");

        let make_item = |id: &str,
                         sort_index: i32,
                         target_key: &str,
                         display_title: &str,
                         sort_title: Option<&str>,
                         original_title: Option<&str>| {
            let mut item = discovery_prune_item(run_id, now);
            item.id = id.to_string();
            item.source_run_kind = "public_feed".to_string();
            item.section_id = Some("popular".to_string());
            item.sort_index = sort_index;
            item.target_key = target_key.to_string();
            item.target_kind = "movie".to_string();
            item.resolved = true;
            item.display_title = display_title.to_string();
            item.original_title = original_title.map(str::to_string);
            item.sort_title = sort_title.map(str::to_string);
            item.poster_url = Some(format!("https://images.example.test/{id}.jpg"));
            item.content_type = Some("movie".to_string());
            item
        };
        let mut hidden_anime = make_item(
            "item-hidden-anime",
            -1,
            "anilist:anime:1",
            "Hidden Anime",
            Some("Hidden Anime"),
            None,
        );
        hidden_anime.target_kind = "series".to_string();
        hidden_anime.content_type = Some("anime".to_string());
        let mut unknown_content_type = make_item(
            "item-unknown-content-type",
            -2,
            "tmdb:movie:99",
            "Unknown Content Type",
            Some("Unknown Content Type"),
            None,
        );
        unknown_content_type.content_type = Some("documentary".to_string());
        let mut missing_poster = make_item(
            "item-missing-poster",
            -3,
            "tmdb:movie:98",
            "Missing Poster",
            Some("Missing Poster"),
            None,
        );
        missing_poster.poster_url = None;

        store
            .replace_discovery_items(
                run_id,
                &[
                    missing_poster,
                    unknown_content_type,
                    hidden_anime,
                    make_item(
                        "item-human-title",
                        0,
                        "tmdb:movie:1",
                        "Human Movie",
                        Some("Human Movie"),
                        None,
                    ),
                    make_item("item-blank-title", 1, "tvdb:movie:2", " ", None, None),
                    make_item(
                        "item-source-title",
                        2,
                        "tvdb:movie:3",
                        "tvdb:movie:3",
                        None,
                        None,
                    ),
                    make_item("item-numeric-title", 3, "tvdb:movie:4", "12345", None, None),
                    make_item(
                        "item-source-title-with-sort",
                        4,
                        "tvdb:movie:5",
                        "tvdb:movie:5",
                        Some("Recovered Sort Title"),
                        None,
                    ),
                ],
            )
            .await
            .expect("items should replace");

        let sections = store
            .list_public_discovery_section_items(
                run_id,
                &["movie".to_string()],
                true,
                &DiscoveryHomeFilters::default(),
                1,
            )
            .await
            .expect("public items should list");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].total_count, 2);
        let target_keys = sections[0]
            .items
            .iter()
            .map(|candidate| candidate.item.target_key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(target_keys, vec!["tmdb:movie:1"]);

        let catalog_sections = store
            .list_catalog_public_discovery_sections(run_id, &[], &[], "movie", true, 10)
            .await
            .expect("catalog public sections should list");
        assert_eq!(catalog_sections.len(), 1);
        let catalog_target_keys = catalog_sections[0]
            .items
            .iter()
            .map(|item| item.target_key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(catalog_target_keys, vec!["tmdb:movie:1", "tvdb:movie:5"]);

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn sqlite_discovery_home_top_rated_projects_hero_backdrop() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_top_rated_backdrop_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let run_id = "run-top-rated-backdrop";

        store
            .upsert_discovery_sync_run(&discovery_prune_run(run_id, "public_feed", "complete", now))
            .await
            .expect("run should upsert");
        store
            .replace_discovery_sections(
                run_id,
                &[DiscoverySectionRecord {
                    id: "section-row-top-rated-backdrop".to_string(),
                    run_id: run_id.to_string(),
                    section_id: "popular".to_string(),
                    section_type: "POPULAR_RIGHT_NOW".to_string(),
                    surface: "public".to_string(),
                    title: "Popular Right Now".to_string(),
                    sort_index: 0,
                    created_at: now,
                    updated_at: now,
                }],
            )
            .await
            .expect("section should replace");

        let mut item = discovery_prune_item(run_id, now);
        item.id = "item-top-rated-backdrop".to_string();
        item.source_run_kind = "public_feed".to_string();
        item.section_id = Some("popular".to_string());
        item.target_key = "tmdb:movie:1001".to_string();
        item.resolved = true;
        item.display_title = "Backdrop Movie".to_string();
        item.sort_title = Some("Backdrop Movie".to_string());
        item.poster_url = Some("https://images.example.test/poster.jpg".to_string());
        item.background_url = Some("https://images.example.test/backdrop.jpg".to_string());
        store
            .replace_discovery_items(run_id, &[item])
            .await
            .expect("items should replace");

        let items = store
            .list_discovery_home_top_rated_items(
                Some(run_id),
                None,
                &[],
                &["movie".to_string()],
                &[],
                &[],
                true,
                &DiscoveryHomeFilters::default(),
                10,
            )
            .await
            .expect("top-rated items should list");

        assert_eq!(items.len(), 1);
        assert!(items[0].has_hero_backdrop);

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn postgres_discovery_home_top_rated_accepts_typed_null_rating() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL discovery top-rated typed null test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = format!(
            "scryer_test_{}_{}",
            std::process::id(),
            Id::new().0.replace('-', "_")
        );

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let mut url = url::Url::parse(&raw_url).map_err(|error| {
                AppError::Validation(format!("invalid postgres test URL: {error}"))
            })?;
            url.query_pairs_mut()
                .append_pair("options", &format!("-csearch_path={schema}"));
            let services =
                PostgresServices::new_with_mode(url.to_string(), MigrationMode::Apply).await?;
            let store = DiscoveryStore::new(services.datastore());
            let now = Utc::now();
            let run_id = "run-top-rated-typed-null";

            store
                .upsert_discovery_sync_run(&discovery_prune_run(
                    run_id,
                    "public_feed",
                    "complete",
                    now,
                ))
                .await?;
            store
                .replace_discovery_sections(
                    run_id,
                    &[DiscoverySectionRecord {
                        id: "section-row-top-rated".to_string(),
                        run_id: run_id.to_string(),
                        section_id: "popular".to_string(),
                        section_type: "POPULAR_RIGHT_NOW".to_string(),
                        surface: "public".to_string(),
                        title: "Popular Right Now".to_string(),
                        sort_index: 0,
                        created_at: now,
                        updated_at: now,
                    }],
                )
                .await?;

            let mut item = discovery_prune_item(run_id, now);
            item.id = "item-top-rated-typed-null".to_string();
            item.source_run_kind = "public_feed".to_string();
            item.section_id = Some("popular".to_string());
            item.target_key = "tmdb:movie:1000".to_string();
            item.resolved = true;
            item.display_title = "Typed Null Movie".to_string();
            item.sort_title = Some("Typed Null Movie".to_string());
            item.rating = None;
            item.external_ratings.clear();
            store.replace_discovery_items(run_id, &[item]).await?;

            let items = store
                .list_discovery_home_top_rated_items(
                    Some(run_id),
                    None,
                    &[],
                    &["movie".to_string()],
                    &[],
                    &[],
                    true,
                    &DiscoveryHomeFilters::default(),
                    10,
                )
                .await?;
            services.pool().close().await;

            assert_eq!(items.len(), 1);
            assert_eq!(items[0].item.target_key, "tmdb:movie:1000");
            Ok(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        cleanup.map_err(|error| {
            AppError::Repository(format!("failed to drop test schema: {error}"))
        })?;
        result
    }

    #[tokio::test]
    async fn sqlite_store_round_trips_inflight_snapshot_and_discovery_lease() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_lease_store_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let lease_expires_at = now + chrono::Duration::minutes(30);

        let state = DiscoverySyncStateRecord {
            inflight_context_snapshot_run_id: Some("run-inflight".to_string()),
            lease_owner_id: Some("owner-a".to_string()),
            lease_expires_at: Some(lease_expires_at),
            transient_failure_count: 2,
            updated_at: now,
            ..DiscoverySyncStateRecord::default()
        };
        store
            .upsert_discovery_sync_state(&state)
            .await
            .expect("state should upsert");

        let loaded_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("state should load")
            .expect("state should exist");
        assert_eq!(
            loaded_state.inflight_context_snapshot_run_id.as_deref(),
            Some("run-inflight")
        );
        assert_eq!(loaded_state.lease_owner_id.as_deref(), Some("owner-a"));
        assert!(loaded_state.lease_expires_at.is_some());
        assert_eq!(loaded_state.transient_failure_count, 2);

        assert!(
            !store
                .try_acquire_discovery_sync_lease(
                    DISCOVERY_DEFAULT_SCOPE_KEY,
                    "owner-b",
                    now + chrono::Duration::minutes(31),
                    now + chrono::Duration::minutes(1),
                )
                .await
                .expect("live lease should be checked"),
            "a different owner must not steal a live lease"
        );
        assert!(
            store
                .renew_discovery_sync_lease(
                    DISCOVERY_DEFAULT_SCOPE_KEY,
                    "owner-a",
                    now + chrono::Duration::minutes(45),
                    now + chrono::Duration::minutes(2),
                )
                .await
                .expect("lease should renew"),
            "current owner should renew the lease"
        );
        assert!(
            store
                .try_acquire_discovery_sync_lease(
                    DISCOVERY_DEFAULT_SCOPE_KEY,
                    "owner-b",
                    now + chrono::Duration::minutes(90),
                    now + chrono::Duration::minutes(60),
                )
                .await
                .expect("expired lease should be checked"),
            "expired leases can be stolen"
        );
        let stolen_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("state should load after steal")
            .expect("state should exist after steal");
        assert_eq!(stolen_state.lease_owner_id.as_deref(), Some("owner-b"));

        store
            .release_discovery_sync_lease(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                "owner-a",
                now + chrono::Duration::minutes(61),
            )
            .await
            .expect("wrong owner release should be harmless");
        let still_leased_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("state should load after wrong release")
            .expect("state should exist after wrong release");
        assert_eq!(
            still_leased_state.lease_owner_id.as_deref(),
            Some("owner-b")
        );

        store
            .release_discovery_sync_lease(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                "owner-b",
                now + chrono::Duration::minutes(62),
            )
            .await
            .expect("lease should release");
        let released_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("state should load after release")
            .expect("state should exist after release");
        assert!(released_state.lease_owner_id.is_none());
        assert!(released_state.lease_expires_at.is_none());
        assert_eq!(
            released_state.inflight_context_snapshot_run_id.as_deref(),
            Some("run-inflight")
        );
    }

    #[tokio::test]
    async fn sqlite_store_round_trips_state_run_and_pending_change() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_store_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();

        let state = DiscoverySyncStateRecord {
            dirty_since: Some(now),
            next_incremental_reload_eligible_at: Some(now),
            incremental_reload_jitter_seconds: 731,
            updated_at: now,
            ..DiscoverySyncStateRecord::default()
        };
        store
            .upsert_discovery_sync_state(&state)
            .await
            .expect("state should upsert");

        let loaded_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("state should load")
            .expect("state should exist");
        assert_eq!(loaded_state.scope_key, DISCOVERY_DEFAULT_SCOPE_KEY);
        assert_eq!(loaded_state.incremental_reload_jitter_seconds, 731);
        assert!(loaded_state.next_incremental_reload_eligible_at.is_some());

        let run = DiscoverySyncRunRecord {
            id: "run-1".to_string(),
            kind: "context_incremental".to_string(),
            status: "complete".to_string(),
            trigger_source: "scheduled_incremental".to_string(),
            region: "US".to_string(),
            language: "eng".to_string(),
            subject_count: 1,
            subject_fingerprint: Some("fingerprint-current".to_string()),
            previous_subject_fingerprint: Some("fingerprint-previous".to_string()),
            base_generation_id: None,
            changed_subject_count: 1,
            affected_target_count: 1,
            smg_request_id: None,
            smg_status: Some("COMPLETE".to_string()),
            discovery_index_watermark: Some("watermark".to_string()),
            page_count: None,
            item_count: Some(0),
            facet_count: Some(0),
            acknowledged_at: None,
            error_text: None,
            started_at: Some(now),
            completed_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        store
            .upsert_discovery_sync_run(&run)
            .await
            .expect("run should upsert");

        let loaded_run = store
            .get_discovery_sync_run("run-1")
            .await
            .expect("run should load")
            .expect("run should exist");
        assert_eq!(loaded_run.kind, "context_incremental");
        assert_eq!(loaded_run.smg_status.as_deref(), Some("COMPLETE"));
        assert!(loaded_run.acknowledged_at.is_none());
        let recent_runs = store
            .list_recent_discovery_sync_runs(5)
            .await
            .expect("recent runs should list");
        assert_eq!(recent_runs.len(), 1);
        assert_eq!(recent_runs[0].id, "run-1");

        store
            .replace_discovery_submitted_subjects(
                "run-1",
                &[
                    DiscoverySubmittedSubjectRecord {
                        run_id: "run-1".to_string(),
                        subject_key: "tvdb:series:1".to_string(),
                        title_id: None,
                        library_id: Some("series-library-a".to_string()),
                        library_facet: Some("series".to_string()),
                        title_kind: Some("series".to_string()),
                        display_title: Some("Example Series A".to_string()),
                        external_ids_json: json!([{"source": "tvdb", "value": "1"}]).to_string(),
                        raw_subject_json: json!({"key": "tvdb:series:1"}).to_string(),
                    },
                    DiscoverySubmittedSubjectRecord {
                        run_id: "run-1".to_string(),
                        subject_key: "tvdb:series:1".to_string(),
                        title_id: None,
                        library_id: Some("series-library-b".to_string()),
                        library_facet: Some("series".to_string()),
                        title_kind: Some("series".to_string()),
                        display_title: Some("Example Series B".to_string()),
                        external_ids_json: json!([{"source": "tvdb", "value": "1"}]).to_string(),
                        raw_subject_json: json!({"key": "tvdb:series:1"}).to_string(),
                    },
                ],
            )
            .await
            .expect("submitted subjects should replace");
        let read_subjects = store
            .list_discovery_submitted_subjects("run-1")
            .await
            .expect("submitted subjects should list");
        assert_eq!(read_subjects.len(), 2);
        assert_eq!(read_subjects[0].subject_key, "tvdb:series:1");
        assert_eq!(
            read_subjects[0].library_id.as_deref(),
            Some("series-library-a")
        );
        assert_eq!(
            read_subjects[1].library_id.as_deref(),
            Some("series-library-b")
        );
        store
            .replace_discovery_sections(
                "run-1",
                &[DiscoverySectionRecord {
                    id: "section-row-1".to_string(),
                    run_id: "run-1".to_string(),
                    section_id: "for_you".to_string(),
                    section_type: "FOR_YOU".to_string(),
                    surface: "personalized".to_string(),
                    title: "For You".to_string(),
                    sort_index: 0,
                    created_at: now,
                    updated_at: now,
                }],
            )
            .await
            .expect("sections should replace");
        store
            .replace_discovery_items(
                "run-1",
                &[
                    DiscoveryItemRecord {
                        id: "item-row-1".to_string(),
                        run_id: "run-1".to_string(),
                        base_generation_id: Some("run-1".to_string()),
                        source_run_kind: "context_incremental".to_string(),
                        section_id: Some("for_you".to_string()),
                        sort_index: 0,
                        target_key: "tmdb:movie:10".to_string(),
                        target_kind: "movie".to_string(),
                        resolved: true,
                        resolved_title_id: None,
                        display_title: "Example Movie".to_string(),
                        original_title: None,
                        sort_title: Some("Example Movie".to_string()),
                        year: Some(2026),
                        poster_path: None,
                        poster_url: None,
                        background_url: Some(
                            "https://images.example.test/movie-bg.jpg".to_string(),
                        ),
                        overview: Some("Rich canonical overview".to_string()),
                        content_type: Some(String::new()),
                        canonical_tags: canonical_genre_tags(&["Drama", "Drama"]),
                        is_adult: true,
                        content_ratings: vec![DiscoveryContentRating {
                            country: "US".to_string(),
                            certifications: vec![DiscoveryContentCertification {
                                value: "PG-13".to_string(),
                                source: "tmdb".to_string(),
                                release_type: Some(3),
                            }],
                            age_rating: Some(13),
                            age_rating_source: Some("tmdb".to_string()),
                        }],
                        rating: Some(7.5),
                        rating_sources: vec!["tmdb".to_string(), "tmdb".to_string()],
                        external_ratings: vec![TitleExternalRating {
                            source: "imdb".to_string(),
                            value: Some(8.2),
                            score: Some(82.0),
                            normalized: 0.82,
                            votes: Some(1234),
                            url: "https://www.imdb.com/title/tt0000010/".to_string(),
                        }],
                        external_ids: vec![
                            DiscoveryExternalIdRecord {
                                source: "tmdb".to_string(),
                                kind: "movie".to_string(),
                                id: "10".to_string(),
                                key: "tmdb:movie:10".to_string(),
                            },
                            DiscoveryExternalIdRecord {
                                source: "tmdb".to_string(),
                                kind: "movie".to_string(),
                                id: "10".to_string(),
                                key: "tmdb:movie:alternate-10".to_string(),
                            },
                            DiscoveryExternalIdRecord {
                                source: "imdb".to_string(),
                                kind: "movie".to_string(),
                                id: "tt0000010".to_string(),
                                key: "imdb:movie:tt0000010".to_string(),
                            },
                        ],
                        status_tags: vec!["available".to_string()],
                        source_tags: vec![DiscoverySourceTagRecord {
                            category: Some("theme".to_string()),
                            name: Some("Isekai".to_string()),
                            values: vec![
                                "theme".to_string(),
                                "Isekai".to_string(),
                                "Isekai".to_string(),
                            ],
                        }],
                        sources: vec!["smg".to_string()],
                        best_source: None,
                        relation_types: Vec::new(),
                        relation_subtypes: Vec::new(),
                        chart_signals: vec!["trending".to_string()],
                        provider_signals: Vec::new(),
                        rank_components: vec![DiscoveryRankComponentRecord {
                            component_index: 0,
                            component_name: Some("score".to_string()),
                            component_value: Some("0.42".to_string()),
                        }],
                        source_count: Some(1),
                        edge_count: Some(1),
                        relation_count: Some(0),
                        source_subject_count: Some(1),
                        rank_score: Some(0.42),
                        matched_subject_keys: vec![
                            "tvdb:series:1".to_string(),
                            "tvdb:series:1".to_string(),
                        ],
                        matched_subject_titles: vec!["Example Series".to_string()],
                        matched_subject_count: 1,
                        library_provenance: vec![
                            DiscoveryItemLibraryProvenanceRecord {
                                subject_key: "tvdb:series:1".to_string(),
                                title_id: None,
                                library_id: Some("series-library-a".to_string()),
                            },
                            DiscoveryItemLibraryProvenanceRecord {
                                subject_key: "tvdb:series:1".to_string(),
                                title_id: None,
                                library_id: Some("series-library-a".to_string()),
                            },
                        ],
                        tmdb_collection_id: None,
                        tmdb_collection_name: None,
                        owned_in_input: false,
                        studio_slug: None,
                        person_ids: Vec::new(),
                        facet_terms: vec![
                            "Drama".to_string(),
                            "canonical:genre:drama".to_string(),
                            "mal:theme:psychological".to_string(),
                            "canonical:theme:isekai".to_string(),
                        ],
                        context_terms: Vec::new(),
                        change_subject_keys: vec!["tvdb:series:1".to_string()],
                        removed_subject_keys: Vec::new(),
                        tombstoned_by_run_id: None,
                        tombstoned_at: None,
                        created_at: now,
                        updated_at: now,
                    },
                    DiscoveryItemRecord {
                        id: "item-row-raw-only".to_string(),
                        run_id: "run-1".to_string(),
                        base_generation_id: Some("run-1".to_string()),
                        source_run_kind: "context_incremental".to_string(),
                        section_id: None,
                        sort_index: 1,
                        target_key: "tvdb:series:2".to_string(),
                        target_kind: "series".to_string(),
                        resolved: true,
                        resolved_title_id: None,
                        display_title: "Raw Label Series".to_string(),
                        original_title: None,
                        sort_title: Some("Raw Label Series".to_string()),
                        year: Some(2026),
                        poster_path: None,
                        poster_url: None,
                        background_url: None,
                        overview: None,
                        content_type: Some("series".to_string()),
                        canonical_tags: canonical_genre_tags(&["Drama"]),
                        is_adult: false,
                        content_ratings: Vec::new(),
                        rating: None,
                        rating_sources: Vec::new(),
                        external_ratings: Vec::new(),
                        external_ids: Vec::new(),
                        status_tags: Vec::new(),
                        source_tags: Vec::new(),
                        sources: vec!["smg".to_string()],
                        best_source: None,
                        relation_types: Vec::new(),
                        relation_subtypes: Vec::new(),
                        chart_signals: Vec::new(),
                        provider_signals: Vec::new(),
                        rank_components: Vec::new(),
                        source_count: Some(1),
                        edge_count: Some(0),
                        relation_count: Some(0),
                        source_subject_count: Some(1),
                        rank_score: Some(0.1),
                        matched_subject_keys: vec!["tvdb:series:1".to_string()],
                        matched_subject_titles: vec!["Example Series".to_string()],
                        matched_subject_count: 1,
                        library_provenance: vec![DiscoveryItemLibraryProvenanceRecord {
                            subject_key: "tvdb:series:1".to_string(),
                            title_id: None,
                            library_id: Some("series-library-a".to_string()),
                        }],
                        tmdb_collection_id: None,
                        tmdb_collection_name: None,
                        owned_in_input: false,
                        studio_slug: None,
                        person_ids: Vec::new(),
                        facet_terms: vec!["canonical:genre:drama".to_string()],
                        context_terms: Vec::new(),
                        change_subject_keys: Vec::new(),
                        removed_subject_keys: Vec::new(),
                        tombstoned_by_run_id: None,
                        tombstoned_at: None,
                        created_at: now,
                        updated_at: now,
                    },
                ],
            )
            .await
            .expect("items should replace");
        store
            .replace_discovery_facets(
                "run-1",
                &[DiscoveryFacetRecord {
                    run_id: "run-1".to_string(),
                    facet_name: "genre".to_string(),
                    facet_value: "Drama".to_string(),
                    smg_count: Some(1),
                    local_count: Some(1),
                }],
            )
            .await
            .expect("facets should replace");
        let read_sections = store
            .list_discovery_sections("run-1", Some("personalized"))
            .await
            .expect("sections should list");
        assert_eq!(read_sections.len(), 1);
        assert_eq!(read_sections[0].section_id, "for_you");
        let read_items = store
            .list_discovery_items_for_generation("run-1")
            .await
            .expect("items should list");
        assert_eq!(read_items.len(), 2);
        let read_item = read_items
            .iter()
            .find(|item| item.id == "item-row-1")
            .expect("canonical fixture item should round trip");
        assert_eq!(read_item.target_key, "tmdb:movie:10");
        assert_eq!(
            read_item.background_url.as_deref(),
            Some("https://images.example.test/movie-bg.jpg")
        );
        assert_eq!(
            read_item.overview.as_deref(),
            Some("Rich canonical overview")
        );
        assert_eq!(read_item.canonical_tags.len(), 1);
        assert_eq!(read_item.canonical_tags[0].category, "genre");
        assert_eq!(read_item.canonical_tags[0].name, "Drama");
        assert_eq!(read_item.rating_sources, vec!["tmdb".to_string()]);
        assert_eq!(read_item.external_ratings.len(), 1);
        assert_eq!(read_item.external_ratings[0].source, "imdb");
        assert_eq!(read_item.external_ratings[0].normalized, 0.82);
        assert_eq!(read_item.external_ratings[0].votes, Some(1234));
        assert_eq!(read_item.external_ids.len(), 2);
        assert_eq!(read_item.external_ids[0].source, "tmdb");
        assert_eq!(read_item.external_ids[0].kind, "movie");
        assert_eq!(read_item.external_ids[0].id, "10");
        assert_eq!(read_item.external_ids[0].key, "tmdb:movie:10");
        assert_eq!(read_item.external_ids[1].source, "imdb");
        assert_eq!(read_item.external_ids[1].id, "tt0000010");
        assert_eq!(read_item.source_tags.len(), 1);
        assert_eq!(
            read_item.source_tags[0].values,
            vec!["theme".to_string(), "Isekai".to_string()]
        );
        assert_eq!(read_item.matched_subject_keys, vec!["tvdb:series:1"]);
        assert_eq!(read_item.change_subject_keys, vec!["tvdb:series:1"]);
        assert_eq!(read_item.library_provenance.len(), 1);
        assert_eq!(
            read_item.library_provenance[0].library_id.as_deref(),
            Some("series-library-a")
        );
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO titles (
                id, library_id, name, name_normalized, facet, root_folder_id, created_at
             )
             VALUES ({}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text("source-title-1".to_string()),
                SqlArg::Text("movie_default_library".to_string()),
                SqlArg::Text("Source Title".to_string()),
                SqlArg::Text("source title".to_string()),
                SqlArg::Text("movie".to_string()),
                SqlArg::Text("canonical_root_for_movie_default_library".to_string()),
                SqlArg::Timestamp(now),
            ],
        )
        .await
        .expect("source title should insert");
        let mut sparse_title_rec_item = (*read_item).clone();
        sparse_title_rec_item.poster_url =
            Some("https://images.example.test/movie-poster.jpg".to_string());
        sparse_title_rec_item.background_url = None;
        sparse_title_rec_item.overview = None;
        sparse_title_rec_item.rating = None;
        sparse_title_rec_item.canonical_tags.clear();
        sparse_title_rec_item.rating_sources.clear();
        sparse_title_rec_item.external_ratings.clear();
        sparse_title_rec_item.source_tags.clear();
        sparse_title_rec_item.facet_terms.clear();
        let mut invalid_identifier_rec_item = sparse_title_rec_item.clone();
        invalid_identifier_rec_item.id = "title-rec-invalid-identifier".to_string();
        invalid_identifier_rec_item.target_key = "tvdb:movie:".to_string();
        invalid_identifier_rec_item.display_title.clear();
        invalid_identifier_rec_item.original_title = None;
        invalid_identifier_rec_item.sort_title = None;
        invalid_identifier_rec_item.year = None;
        invalid_identifier_rec_item.poster_url = None;
        invalid_identifier_rec_item.background_url = None;
        invalid_identifier_rec_item.overview = None;
        invalid_identifier_rec_item.content_type = None;
        invalid_identifier_rec_item.external_ids.clear();
        let mut missing_poster_rec_item = sparse_title_rec_item.clone();
        missing_poster_rec_item.id = "title-rec-missing-poster".to_string();
        missing_poster_rec_item.target_key = "tmdb:movie:11".to_string();
        missing_poster_rec_item.display_title = "Missing Poster Recommendation".to_string();
        missing_poster_rec_item.sort_title = Some("Missing Poster Recommendation".to_string());
        missing_poster_rec_item.poster_url = None;
        store
            .replace_title_more_like_this_items(
                "source-title-1",
                "eng",
                &[
                    invalid_identifier_rec_item,
                    missing_poster_rec_item,
                    sparse_title_rec_item,
                ],
            )
            .await
            .expect("title recommendations should replace");
        let more_like_this = store
            .list_title_more_like_this_items("source-title-1", 10)
            .await
            .expect("title recommendations should list");
        assert_eq!(more_like_this.len(), 1);
        assert_eq!(more_like_this[0].target_key, "tmdb:movie:10");
        assert!(more_like_this[0].is_adult);
        assert_eq!(more_like_this[0].content_ratings.len(), 1);
        assert_eq!(more_like_this[0].content_ratings[0].country, "US");
        assert_eq!(
            more_like_this[0].content_ratings[0].certifications[0].value,
            "PG-13"
        );
        assert_eq!(
            more_like_this[0].background_url.as_deref(),
            Some("https://images.example.test/movie-bg.jpg")
        );
        assert_eq!(
            more_like_this[0].overview.as_deref(),
            Some("Rich canonical overview")
        );
        assert_eq!(more_like_this[0].canonical_tags.len(), 1);
        assert_eq!(more_like_this[0].canonical_tags[0].category, "genre");
        assert_eq!(more_like_this[0].canonical_tags[0].name, "Drama");
        assert_eq!(
            more_like_this[0].source_tags[0].values,
            vec!["theme".to_string(), "Isekai".to_string()]
        );
        assert_eq!(more_like_this[0].external_ratings.len(), 1);
        assert_eq!(more_like_this[0].external_ratings[0].source, "imdb");
        let discovery_title_rows = SqlRuntime::fetch_all(
            store.datastore.read_exec(),
            "SELECT id
             FROM discovery_titles
             WHERE target_key_norm = {} AND language = {}
             ORDER BY id ASC",
            &[
                SqlArg::Text("tmdb:movie:10".to_string()),
                SqlArg::Text("eng".to_string()),
            ],
        )
        .await
        .expect("discovery title rows should query");
        assert_eq!(
            discovery_title_rows.len(),
            1,
            "snapshot occurrences and title recommendations should share one discovery title row"
        );
        let occurrence_title_id = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT discovery_title_id
             FROM discovery_items
             WHERE id = {}",
            &[SqlArg::Text("item-row-1".to_string())],
        )
        .await
        .expect("occurrence title id should query")
        .expect("occurrence title id should exist")
        .text("discovery_title_id")
        .expect("occurrence title id should read");
        let link_title_id = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT discovery_title_id
             FROM title_more_like_this_items
             WHERE source_title_id = {}",
            &[SqlArg::Text("source-title-1".to_string())],
        )
        .await
        .expect("link title id should query")
        .expect("link title id should exist")
        .text("discovery_title_id")
        .expect("link title id should read");
        assert_eq!(occurrence_title_id, link_title_id);
        let read_facets = store
            .list_discovery_facets("run-1")
            .await
            .expect("facets should list");
        assert_eq!(read_facets.len(), 1);
        assert_eq!(read_facets[0].facet_value, "Drama");
        let catalog_movie_candidates = store
            .list_catalog_personalized_discovery_items(
                "run-1",
                &["series-library-a".to_string()],
                "movie",
                false,
                10,
            )
            .await
            .expect("catalog personalized candidates should apply provenance and media kind");
        assert_eq!(catalog_movie_candidates.total_count, 1);
        assert_eq!(catalog_movie_candidates.items[0].id, "item-row-1");
        let hidden_catalog_candidates = store
            .list_catalog_personalized_discovery_items(
                "run-1",
                &["series-library-b".to_string()],
                "movie",
                false,
                10,
            )
            .await
            .expect("catalog personalized candidates should apply library scope");
        assert!(hidden_catalog_candidates.items.is_empty());
        let personalized_facets = store
            .list_personalized_discovery_facets(
                "run-1",
                &["series-library-a".to_string()],
                &["movie".to_string(), "series".to_string()],
                false,
            )
            .await
            .expect("personalized facets should list from canonical terms");
        assert_eq!(personalized_facets.len(), 2);
        assert!(
            personalized_facets
                .iter()
                .any(|facet| facet.facet_name == "genre"
                    && facet.facet_value == "Drama"
                    && facet.smg_count.is_none()
                    && facet.local_count == Some(2))
        );
        assert!(
            personalized_facets
                .iter()
                .any(|facet| facet.facet_name == "theme"
                    && facet.facet_value == "Isekai"
                    && facet.smg_count.is_none()
                    && facet.local_count == Some(1))
        );
        assert!(personalized_facets.iter().all(|facet| {
            facet.facet_value != "mal:theme:psychological" && facet.facet_value != "Drama:"
        }));
        let series_only_facets = store
            .list_personalized_discovery_facets(
                "run-1",
                &["series-library-a".to_string()],
                &["series".to_string()],
                false,
            )
            .await
            .expect("personalized facets should apply media-kind scope before counting");
        assert_eq!(series_only_facets.len(), 1);
        assert_eq!(series_only_facets[0].facet_name, "genre");
        assert_eq!(series_only_facets[0].facet_value, "Drama");
        assert_eq!(series_only_facets[0].local_count, Some(1));
        let hidden_library_facets = store
            .list_personalized_discovery_facets(
                "run-1",
                &["series-library-b".to_string()],
                &["movie".to_string(), "series".to_string()],
                false,
            )
            .await
            .expect("personalized facets should apply library provenance");
        assert!(hidden_library_facets.is_empty());
        let mut pagination_items = store
            .list_discovery_items_for_generation("run-1")
            .await
            .expect("discovery items should list before pagination setup");
        let movie_template = pagination_items
            .iter()
            .find(|item| item.target_key == "tmdb:movie:10")
            .expect("movie template should exist")
            .clone();
        let mut second_movie = movie_template.clone();
        second_movie.id = "item-row-movie-2".to_string();
        second_movie.target_key = "tmdb:movie:11".to_string();
        second_movie.display_title = "Second Movie".to_string();
        second_movie.sort_title = Some("Second Movie".to_string());
        second_movie.content_type = Some("movie".to_string());
        second_movie.rank_score = Some(0.2);
        let mut unknown_content_type = movie_template;
        unknown_content_type.id = "item-row-unknown-kind".to_string();
        unknown_content_type.target_key = "tmdb:movie:12".to_string();
        unknown_content_type.display_title = "Unknown Kind".to_string();
        unknown_content_type.sort_title = Some("Unknown Kind".to_string());
        unknown_content_type.content_type = Some("documentary".to_string());
        unknown_content_type.rank_score = Some(100.0);
        pagination_items.extend([second_movie, unknown_content_type]);
        store
            .replace_discovery_items("run-1", &pagination_items)
            .await
            .expect("pagination items should replace");
        let movie_page = store
            .query_discovery_items(&DiscoveryItemsStorageQuery {
                context_run_id: Some("run-1".to_string()),
                public_run_id: None,
                readable_library_ids: vec!["series-library-a".to_string()],
                allowed_media_kinds: vec!["movie".to_string()],
                filters: DiscoveryItemsQuery {
                    target_kinds: vec!["movie".to_string()],
                    include_unresolved: false,
                    ..DiscoveryItemsQuery::default()
                },
                limit: 1,
                offset: 0,
            })
            .await
            .expect("movie query should use normalized media kind");
        assert_eq!(movie_page.total_count, 2);
        assert_eq!(movie_page.items[0].target_key, "tmdb:movie:10");
        let second_movie_page = store
            .query_discovery_items(&DiscoveryItemsStorageQuery {
                context_run_id: Some("run-1".to_string()),
                public_run_id: None,
                readable_library_ids: vec!["series-library-a".to_string()],
                allowed_media_kinds: vec!["movie".to_string()],
                filters: DiscoveryItemsQuery {
                    target_kinds: vec!["movie".to_string()],
                    include_unresolved: false,
                    ..DiscoveryItemsQuery::default()
                },
                limit: 1,
                offset: 1,
            })
            .await
            .expect("second movie page should preserve filtered pagination");
        assert_eq!(second_movie_page.total_count, 2);
        assert_eq!(second_movie_page.items[0].target_key, "tmdb:movie:11");

        store
            .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
                id: "snapshot-change-1".to_string(),
                scope_key: DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
                subject_key: Some("tmdb:movie:603".to_string()),
                previous_subject_key: None,
                change_type: "updated".to_string(),
                title_id: None,
                previous_title_id: None,
                library_facet: Some("movie".to_string()),
                raw_subject_json: Some(json!({"tmdbId": 603}).to_string()),
                raw_previous_subject_json: None,
                first_seen_sequence: Some(4),
                last_seen_sequence: Some(4),
                first_seen_at: now,
                last_seen_at: now,
            })
            .await
            .expect("snapshot pending change should upsert");
        assert_eq!(
            store
                .count_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY)
                .await
                .expect("pending changes should count"),
            1
        );

        let committed_state = DiscoverySyncStateRecord {
            last_success_generation_id: Some("run-2".to_string()),
            last_subject_fingerprint: Some("fingerprint-run-2".to_string()),
            last_context_snapshot_completed_at: Some(now),
            updated_at: now,
            ..DiscoverySyncStateRecord::default()
        };
        let committed_run = DiscoverySyncRunRecord {
            id: "run-2".to_string(),
            kind: "context_snapshot".to_string(),
            status: "complete".to_string(),
            trigger_source: "scheduled_interval".to_string(),
            region: "US".to_string(),
            language: "eng".to_string(),
            subject_count: 1,
            subject_fingerprint: Some("fingerprint-run-2".to_string()),
            previous_subject_fingerprint: Some("fingerprint-current".to_string()),
            base_generation_id: None,
            changed_subject_count: 0,
            affected_target_count: 0,
            smg_request_id: Some("request-2".to_string()),
            smg_status: Some("COMPLETE".to_string()),
            discovery_index_watermark: Some("watermark-2".to_string()),
            page_count: Some(1),
            item_count: Some(0),
            facet_count: Some(0),
            acknowledged_at: None,
            error_text: None,
            started_at: Some(now),
            completed_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        store
            .commit_discovery_context_snapshot(&DiscoveryContextSnapshotCommit {
                state: committed_state,
                run: committed_run,
                submitted_subjects: vec![DiscoverySubmittedSubjectRecord {
                    run_id: "run-2".to_string(),
                    subject_key: "tmdb:movie:603".to_string(),
                    title_id: None,
                    library_id: Some("movie-library".to_string()),
                    library_facet: Some("movie".to_string()),
                    title_kind: Some("movie".to_string()),
                    display_title: Some("Example Movie".to_string()),
                    external_ids_json: json!([{"source": "tmdb", "value": "603"}]).to_string(),
                    raw_subject_json: json!({"tmdbId": 603}).to_string(),
                }],
                items: Vec::new(),
                facets: Vec::new(),
                clear_pending_through_sequence: Some(4),
            })
            .await
            .expect("snapshot commit should be transactional");

        let active_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("committed state should load")
            .expect("committed state should exist");
        assert_eq!(
            active_state.last_success_generation_id.as_deref(),
            Some("run-2")
        );
        let committed_run = store
            .get_discovery_sync_run("run-2")
            .await
            .expect("committed run should load")
            .expect("committed run should exist");
        assert_eq!(committed_run.kind, "context_snapshot");
        assert_eq!(committed_run.smg_request_id.as_deref(), Some("request-2"));
        let unacked_runs = store
            .list_unacked_discovery_context_snapshot_runs(10)
            .await
            .expect("unacked context snapshot runs should list");
        assert_eq!(unacked_runs.len(), 1);
        assert_eq!(unacked_runs[0].id, "run-2");
        assert!(
            store
                .list_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY, 10)
                .await
                .expect("pending changes should list after snapshot")
                .is_empty()
        );
        assert_eq!(
            store
                .count_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY)
                .await
                .expect("pending changes should count after snapshot"),
            0
        );

        let incremental_state = DiscoverySyncStateRecord {
            last_success_generation_id: Some("run-2".to_string()),
            last_subject_fingerprint: Some("fingerprint-incremental".to_string()),
            last_context_snapshot_completed_at: Some(now),
            last_incremental_reload_completed_at: Some(now),
            last_seen_domain_event_sequence: Some(12),
            updated_at: now,
            ..DiscoverySyncStateRecord::default()
        };
        let incremental_run = DiscoverySyncRunRecord {
            id: "run-3".to_string(),
            kind: "context_incremental".to_string(),
            status: "complete".to_string(),
            trigger_source: "scheduled_incremental".to_string(),
            region: "US".to_string(),
            language: "eng".to_string(),
            subject_count: 1,
            subject_fingerprint: Some("fingerprint-incremental".to_string()),
            previous_subject_fingerprint: Some("fingerprint-run-2".to_string()),
            base_generation_id: Some("run-2".to_string()),
            changed_subject_count: 1,
            affected_target_count: 1,
            smg_request_id: None,
            smg_status: Some("COMPLETE".to_string()),
            discovery_index_watermark: Some("watermark-3".to_string()),
            page_count: None,
            item_count: Some(0),
            facet_count: None,
            acknowledged_at: None,
            error_text: None,
            started_at: Some(now),
            completed_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        store
            .commit_discovery_context_incremental(&DiscoveryContextIncrementalCommit {
                state: incremental_state,
                run: incremental_run,
                items: Vec::new(),
                tombstone_target_keys: vec!["tmdb:movie:10".to_string()],
                clear_pending_through_sequence: Some(12),
            })
            .await
            .expect("incremental commit should be transactional");
        let incremental_run = store
            .get_discovery_sync_run("run-3")
            .await
            .expect("incremental run should load")
            .expect("incremental run should exist");
        assert_eq!(incremental_run.kind, "context_incremental");
        let incremental_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("incremental state should load")
            .expect("incremental state should exist");
        assert_eq!(
            incremental_state.last_incremental_reload_completed_at,
            Some(now)
        );

        let public_state = DiscoverySyncStateRecord {
            last_success_generation_id: Some("run-2".to_string()),
            last_public_feed_generation_id: Some("run-4".to_string()),
            last_public_feed_completed_at: Some(now),
            updated_at: now,
            ..incremental_state.clone()
        };
        let public_run = DiscoverySyncRunRecord {
            id: "run-4".to_string(),
            kind: "public_feed".to_string(),
            status: "complete".to_string(),
            trigger_source: "scheduled_interval".to_string(),
            region: "US".to_string(),
            language: "eng".to_string(),
            subject_count: 0,
            subject_fingerprint: None,
            previous_subject_fingerprint: None,
            base_generation_id: None,
            changed_subject_count: 0,
            affected_target_count: 0,
            smg_request_id: None,
            smg_status: Some("COMPLETE".to_string()),
            discovery_index_watermark: None,
            page_count: Some(1),
            item_count: Some(0),
            facet_count: Some(0),
            acknowledged_at: None,
            error_text: None,
            started_at: Some(now),
            completed_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        store
            .commit_discovery_public_feed(&DiscoveryPublicFeedCommit {
                state: public_state,
                run: public_run,
                sections: Vec::new(),
                items: Vec::new(),
            })
            .await
            .expect("public feed commit should be transactional");
        let public_run = store
            .get_discovery_sync_run("run-4")
            .await
            .expect("public feed run should load")
            .expect("public feed run should exist");
        assert_eq!(public_run.kind, "public_feed");
        let public_state = store
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("public feed state should load")
            .expect("public feed state should exist");
        assert_eq!(
            public_state.last_public_feed_generation_id.as_deref(),
            Some("run-4")
        );

        let change = DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tvdb:series:1".to_string()),
            previous_subject_key: None,
            change_type: "added".to_string(),
            title_id: None,
            previous_title_id: None,
            library_facet: Some("series".to_string()),
            raw_subject_json: Some(json!({"key": "tvdb:series:1"}).to_string()),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: now,
            last_seen_at: now,
        };
        store
            .upsert_pending_discovery_context_change(&change)
            .await
            .expect("pending change should upsert");

        let pending = store
            .list_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY, 10)
            .await
            .expect("pending changes should list");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].change_type, "added");
        assert_eq!(
            pending[0].raw_subject_json.as_deref(),
            Some(r#"{"key":"tvdb:series:1"}"#)
        );

        let deleted = store
            .clear_pending_discovery_context_changes_through_sequence(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                12,
            )
            .await
            .expect("pending changes should clear");
        assert_eq!(deleted, 1);

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn sqlite_store_get_delete_and_list_all_pending_changes() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_pending_store_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let change = DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: None,
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(json!({"tmdbId": 603}).to_string()),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: now,
            last_seen_at: now,
        };

        store
            .upsert_pending_discovery_context_change(&change)
            .await
            .expect("pending change should upsert");
        let loaded = store
            .get_pending_discovery_context_change("change-1")
            .await
            .expect("pending change should load")
            .expect("pending change should exist");
        assert_eq!(loaded.subject_key.as_deref(), Some("tmdb:movie:603"));
        assert_eq!(loaded.first_seen_sequence, Some(10));

        let all = store
            .list_all_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await
            .expect("all pending changes should list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "change-1");

        assert_eq!(
            store
                .delete_pending_discovery_context_change("change-1")
                .await
                .expect("pending change should delete"),
            1
        );
        assert!(
            store
                .get_pending_discovery_context_change("change-1")
                .await
                .expect("pending change lookup should succeed")
                .is_none()
        );
        assert_eq!(
            store
                .delete_pending_discovery_context_change("change-1")
                .await
                .expect("missing pending change delete should be harmless"),
            0
        );

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn sqlite_store_bounds_and_garbage_collects_recommendation_cards() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_recommendation_cards_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();

        for source_title_id in ["source-title-a", "source-title-b"] {
            SqlRuntime::execute(
                store.datastore.read_exec(),
                "INSERT INTO titles (
                    id, library_id, name, name_normalized, facet, root_folder_id, created_at
                 )
                 VALUES ({}, {}, {}, {}, {}, {}, {})",
                &[
                    SqlArg::Text(source_title_id.to_string()),
                    SqlArg::Text("movie_default_library".to_string()),
                    SqlArg::Text(source_title_id.to_string()),
                    SqlArg::Text(source_title_id.to_string()),
                    SqlArg::Text("movie".to_string()),
                    SqlArg::Text("canonical_root_for_movie_default_library".to_string()),
                    SqlArg::Timestamp(now),
                ],
            )
            .await
            .expect("source title should insert");
        }

        let recommendations = (0..30)
            .map(|index| {
                let mut item = discovery_prune_item("recommendations", now);
                item.id = format!("recommendation-{index}");
                item.target_key = format!("tmdb:movie:{}", 20_000 + index);
                item.display_title = format!("Recommendation {index}");
                item.sort_title = Some(item.display_title.clone());
                item.poster_url = Some(format!("https://images.test/{index}.jpg"));
                item.sort_index = index;
                item
            })
            .collect::<Vec<_>>();
        store
            .replace_title_more_like_this_items("source-title-a", "eng", &recommendations)
            .await
            .expect("recommendations should replace");

        let edge_count = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM title_more_like_this_items",
            &[],
        )
        .await
        .expect("edge count should query")
        .expect("edge count should exist")
        .i64("count")
        .expect("edge count should parse");
        let card_count = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM title_recommendation_cards",
            &[],
        )
        .await
        .expect("card count should query")
        .expect("card count should exist")
        .i64("count")
        .expect("card count should parse");
        assert_eq!(edge_count, 24);
        assert_eq!(card_count, 24);

        store
            .replace_title_more_like_this_items("source-title-b", "eng", &recommendations[..12])
            .await
            .expect("shared recommendations should replace");
        store
            .replace_title_more_like_this_items("source-title-a", "eng", &[])
            .await
            .expect("source recommendations should clear");

        let remaining_cards = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM title_recommendation_cards",
            &[],
        )
        .await
        .expect("remaining card count should query")
        .expect("remaining card count should exist")
        .i64("count")
        .expect("remaining card count should parse");
        let normalized_titles = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM discovery_titles",
            &[],
        )
        .await
        .expect("normalized title count should query")
        .expect("normalized title count should exist")
        .i64("count")
        .expect("normalized title count should parse");
        assert_eq!(remaining_cards, 12);
        assert_eq!(normalized_titles, 0);

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn sqlite_housekeeping_backfills_legacy_recommendation_cards_before_pruning() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_legacy_recommendation_cards_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let old_at = now - chrono::Duration::days(60);

        store
            .upsert_discovery_sync_run(&discovery_prune_run(
                "legacy-run",
                "context_snapshot",
                "complete",
                old_at,
            ))
            .await
            .expect("legacy run should insert");
        let mut legacy_item = discovery_prune_item("legacy-run", old_at);
        legacy_item.poster_url = Some("https://images.test/legacy.jpg".to_string());
        legacy_item.display_title = "Legacy Recommendation".to_string();
        legacy_item.sort_title = Some(legacy_item.display_title.clone());
        store
            .replace_discovery_items("legacy-run", std::slice::from_ref(&legacy_item))
            .await
            .expect("legacy item should insert");
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO titles (
                id, library_id, name, name_normalized, facet, root_folder_id, created_at
             )
             VALUES ({}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text("legacy-source".to_string()),
                SqlArg::Text("movie_default_library".to_string()),
                SqlArg::Text("Legacy Source".to_string()),
                SqlArg::Text("legacy source".to_string()),
                SqlArg::Text("movie".to_string()),
                SqlArg::Text("canonical_root_for_movie_default_library".to_string()),
                SqlArg::Timestamp(old_at),
            ],
        )
        .await
        .expect("legacy source title should insert");
        let discovery_title_id = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT discovery_title_id FROM discovery_items WHERE id = {}",
            &[SqlArg::Text(legacy_item.id.clone())],
        )
        .await
        .expect("legacy title id should query")
        .expect("legacy title id should exist")
        .text("discovery_title_id")
        .expect("legacy title id should parse");
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO title_recommendation_cards
             (discovery_title_id, payload_version, payload_blob, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {})",
            &[
                SqlArg::Text(discovery_title_id.clone()),
                SqlArg::I32(TITLE_RECOMMENDATION_PAYLOAD_VERSION),
                SqlArg::OptBytes(None),
                SqlArg::Timestamp(old_at),
                SqlArg::Timestamp(old_at),
            ],
        )
        .await
        .expect("legacy card placeholder should insert");
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO title_more_like_this_items
             (source_title_id, discovery_title_id, sort_index, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {})",
            &[
                SqlArg::Text("legacy-source".to_string()),
                SqlArg::Text(discovery_title_id),
                SqlArg::I32(0),
                SqlArg::Timestamp(old_at),
                SqlArg::Timestamp(old_at),
            ],
        )
        .await
        .expect("legacy edge should insert");

        let legacy_fallback = store
            .list_title_more_like_this_items("legacy-source", 10)
            .await
            .expect("legacy recommendation should use normalized fallback");
        assert_eq!(legacy_fallback.len(), 1);
        assert_eq!(legacy_fallback[0].display_title, "Legacy Recommendation");

        store
            .replace_title_more_like_this_items("unrelated-source", "eng", &[])
            .await
            .expect("unrelated refresh should preserve legacy placeholders");
        assert_eq!(
            store
                .list_title_more_like_this_items("legacy-source", 10)
                .await
                .expect("legacy fallback should survive normal repository cleanup")
                .len(),
            1
        );

        store
            .prune_discovery_history(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                0,
                now - chrono::Duration::days(30),
            )
            .await
            .expect("legacy history should prune after card backfill");
        let payload_present = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT payload_blob IS NOT NULL AS present FROM title_recommendation_cards",
            &[],
        )
        .await
        .expect("card payload should query")
        .expect("card payload should exist")
        .bool("present")
        .expect("card payload presence should parse");
        let normalized_title_count = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM discovery_titles",
            &[],
        )
        .await
        .expect("normalized title count should query")
        .expect("normalized title count should exist")
        .i64("count")
        .expect("normalized title count should parse");
        assert!(payload_present);
        assert_eq!(normalized_title_count, 0);
        assert_eq!(
            store
                .list_title_more_like_this_items("legacy-source", 10)
                .await
                .expect("backfilled recommendation should remain readable")
                .len(),
            1
        );

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn sqlite_context_commit_enforces_merged_indexed_title_limit() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_indexed_title_limit_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();

        let public_run_id = "public-active";
        let mut public_items = Vec::new();
        for index in 0..2 {
            let mut item = discovery_prune_item(public_run_id, now);
            item.id = format!("public-item-{index}");
            item.base_generation_id = None;
            item.source_run_kind = "public_feed".to_string();
            item.target_key = format!("tmdb:movie:{}", 30_000 + index);
            item.display_title = format!("Public {index}");
            item.sort_title = Some(item.display_title.clone());
            public_items.push(item);
        }
        let public_state = DiscoverySyncStateRecord {
            last_public_feed_generation_id: Some(public_run_id.to_string()),
            updated_at: now,
            ..DiscoverySyncStateRecord::default()
        };
        store
            .commit_discovery_public_feed(&DiscoveryPublicFeedCommit {
                state: public_state,
                run: discovery_prune_run(public_run_id, "public_feed", "complete", now),
                sections: Vec::new(),
                items: public_items,
            })
            .await
            .expect("public feed should commit");

        let context_run_id = "context-active";
        let mut context_items = Vec::new();
        for index in 0..1_005 {
            let mut item = discovery_prune_item(context_run_id, now);
            item.id = format!("context-item-{index}");
            item.target_key = format!("tmdb:movie:{}", 40_000 + index);
            item.display_title = format!("Context {index}");
            item.sort_title = Some(item.display_title.clone());
            item.rank_score = Some(index as f64);
            item.sort_index = index;
            context_items.push(item);
        }
        let context_state = DiscoverySyncStateRecord {
            last_success_generation_id: Some(context_run_id.to_string()),
            last_public_feed_generation_id: Some(public_run_id.to_string()),
            updated_at: now,
            ..DiscoverySyncStateRecord::default()
        };
        store
            .commit_discovery_context_snapshot(&DiscoveryContextSnapshotCommit {
                state: context_state,
                run: discovery_prune_run(context_run_id, "context_snapshot", "complete", now),
                submitted_subjects: Vec::new(),
                items: context_items,
                facets: Vec::new(),
                clear_pending_through_sequence: None,
            })
            .await
            .expect("context snapshot should commit");

        let counts = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT COUNT(DISTINCT discovery_title_id) AS title_count,
                    SUM(CASE WHEN run_id = {} THEN 1 ELSE 0 END) AS public_count,
                    SUM(CASE WHEN run_id = {} THEN 1 ELSE 0 END) AS context_count
             FROM discovery_items",
            &[
                SqlArg::Text(public_run_id.to_string()),
                SqlArg::Text(context_run_id.to_string()),
            ],
        )
        .await
        .expect("active counts should query")
        .expect("active counts should exist");
        assert_eq!(counts.i64("title_count").expect("title count"), 1_000);
        assert_eq!(counts.i64("public_count").expect("public count"), 2);
        assert_eq!(counts.i64("context_count").expect("context count"), 998);

        let low_rank_exists = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT 1 AS present FROM discovery_items WHERE id = {}",
            &[SqlArg::Text("context-item-0".to_string())],
        )
        .await
        .expect("low-rank item should query")
        .is_some();
        let high_rank_exists = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT 1 AS present FROM discovery_items WHERE id = {}",
            &[SqlArg::Text("context-item-1004".to_string())],
        )
        .await
        .expect("high-rank item should query")
        .is_some();
        assert!(!low_rank_exists);
        assert!(high_rank_exists);

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn sqlite_store_prunes_discovery_history_with_retention_guards() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_prune_store_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let active_snapshot_at = now - chrono::Duration::days(10);
        let old_at = now - chrono::Duration::days(60);

        for run in [
            discovery_prune_run(
                "snapshot-active",
                "context_snapshot",
                "complete",
                active_snapshot_at,
            ),
            discovery_prune_run("snapshot-newer", "context_snapshot", "complete", now),
            discovery_prune_run("snapshot-pruned", "context_snapshot", "complete", old_at),
            discovery_prune_run("public-active", "public_feed", "complete", old_at),
            discovery_prune_run(
                "incremental-attached",
                "context_incremental",
                "complete",
                old_at,
            ),
            discovery_prune_run("deferred-old", "context_incremental", "deferred", old_at),
            discovery_prune_run("failed-recent", "context_incremental", "failed", now),
            discovery_prune_run("running-old", "context_snapshot", "running", old_at),
        ] {
            let mut run = run;
            if run.id == "incremental-attached" {
                run.base_generation_id = Some("snapshot-active".to_string());
            }
            store
                .upsert_discovery_sync_run(&run)
                .await
                .expect("run should upsert");
        }
        store
            .upsert_discovery_sync_state(&DiscoverySyncStateRecord {
                last_success_generation_id: Some("snapshot-active".to_string()),
                last_public_feed_generation_id: Some("public-active".to_string()),
                updated_at: now,
                ..DiscoverySyncStateRecord::default()
            })
            .await
            .expect("state should upsert");
        store
            .replace_discovery_items(
                "snapshot-pruned",
                &[discovery_prune_item("snapshot-pruned", old_at)],
            )
            .await
            .expect("item should insert for pruned run");

        let report = store
            .prune_discovery_history(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                2,
                now - chrono::Duration::days(30),
            )
            .await
            .expect("discovery history should prune");
        assert_eq!(report.runs_deleted, 2);

        for id in [
            "snapshot-active",
            "snapshot-newer",
            "public-active",
            "incremental-attached",
            "failed-recent",
            "running-old",
        ] {
            assert!(
                store
                    .get_discovery_sync_run(id)
                    .await
                    .expect("run lookup should succeed")
                    .is_some(),
                "{id} should be retained"
            );
        }
        for id in ["snapshot-pruned", "deferred-old"] {
            assert!(
                store
                    .get_discovery_sync_run(id)
                    .await
                    .expect("run lookup should succeed")
                    .is_none(),
                "{id} should be pruned"
            );
        }
        let item_count = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT COUNT(*) AS count FROM discovery_items WHERE run_id = {}",
            &[SqlArg::Text("snapshot-pruned".to_string())],
        )
        .await
        .expect("item count should query")
        .expect("item count should return")
        .i64("count")
        .expect("item count should parse");
        assert_eq!(item_count, 0);

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn discovery_home_hydration_reads_terms_only_for_selected_candidates() {
        const CANDIDATE_COUNT: usize = 128;
        const SELECTED_COUNT: usize = 3;
        const TERMS_PER_TITLE: usize = 64;
        const TITLE_TERM_ROWS_PER_TITLE: usize = TERMS_PER_TITLE + 1; // authoritative media_kind

        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_home_selected_hydration_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let run_id = "run-selected-home-hydration";
        store
            .upsert_discovery_sync_run(&discovery_prune_run(
                run_id,
                "context_snapshot",
                "complete",
                now,
            ))
            .await
            .expect("run should upsert");

        let mut all_candidates = Vec::with_capacity(CANDIDATE_COUNT);
        for index in 0..CANDIDATE_COUNT {
            let mut item = discovery_prune_item(run_id, now);
            item.id = format!("{run_id}:item:{index}");
            item.target_key = format!("tmdb:movie:{}", 10_000 + index);
            item.display_title = format!("Candidate {index}");
            item.sort_title = Some(item.display_title.clone());
            item.sort_index = index as i32;
            item.context_terms = (0..TERMS_PER_TITLE)
                .map(|term_index| format!("term-{index}-{term_index}"))
                .collect();
            all_candidates.push(item);
        }
        store
            .replace_discovery_items(run_id, &all_candidates)
            .await
            .expect("candidates should upsert");

        let mut selected_candidates = Vec::with_capacity(SELECTED_COUNT);
        for mut item in all_candidates.into_iter().take(SELECTED_COUNT) {
            let discovery_title_id: String =
                sqlx::query_scalar("SELECT discovery_title_id FROM discovery_items WHERE id = ?")
                    .bind(&item.id)
                    .fetch_one(&services.pool)
                    .await
                    .expect("selected candidate title should resolve");
            item.context_terms.clear();
            selected_candidates.push(DiscoveryHomeCandidate {
                item,
                discovery_title_id,
                matched_subject_keys: Vec::new(),
                affinity_terms: Vec::new(),
                has_hero_backdrop: false,
                rating_source_count: 0,
                best_external_rating: None,
                best_external_rating_votes: 0,
            });
        }

        let counts = hydrate_discovery_home_candidates_with_counts(
            &store.datastore,
            &mut selected_candidates,
        )
        .await
        .expect("selected candidates should hydrate");

        assert_eq!(
            counts.title_term_rows,
            SELECTED_COUNT * TITLE_TERM_ROWS_PER_TITLE
        );
        assert!(
            selected_candidates
                .iter()
                .all(|candidate| candidate.item.context_terms.len() == TERMS_PER_TITLE)
        );

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn sqlite_personalized_home_candidates_load_selection_signals() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_personalized_home_candidates_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let run_id = "run-personalized-home-candidates";
        store
            .upsert_discovery_sync_run(&discovery_prune_run(
                run_id,
                "context_snapshot",
                "complete",
                now,
            ))
            .await
            .expect("run should upsert");

        let mut item = discovery_prune_item(run_id, now);
        item.id = format!("{run_id}:readable");
        item.target_key = "tmdb:movie:10".to_string();
        item.poster_url = Some("https://example.com/poster.jpg".to_string());
        item.facet_terms = vec!["canonical:genre:drama".to_string()];
        item.library_provenance = vec![DiscoveryItemLibraryProvenanceRecord {
            subject_key: "tmdb:movie:10".to_string(),
            title_id: Some("library-title-10".to_string()),
            library_id: Some("movie-library".to_string()),
        }];
        store
            .replace_discovery_items(run_id, &[item])
            .await
            .expect("item should upsert");

        let candidates = fetch_personalized_home_candidates(
            &store.datastore,
            run_id,
            &["movie-library".to_string()],
            &["movie".to_string()],
            true,
            &DiscoveryHomeFilters::default(),
            None,
            18,
        )
        .await
        .expect("personalized home candidates should load");

        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].affinity_terms,
            vec!["canonical:genre:drama".to_string()]
        );

        let _ = std::fs::remove_file(db);
    }

    #[tokio::test]
    async fn sqlite_personalized_facets_use_only_readable_library_items() {
        let db = std::env::temp_dir().join(format!(
            "scryer_discovery_personalized_facets_{}.db",
            Utc::now().timestamp_micros()
        ));
        let services = SqliteServices::new(db.to_string_lossy())
            .await
            .expect("sqlite services should initialize");
        let store = DiscoveryStore::new(services.datastore());
        let now = Utc::now();
        let run_id = "run-personalized-facets";
        store
            .upsert_discovery_sync_run(&discovery_prune_run(
                run_id,
                "context_snapshot",
                "complete",
                now,
            ))
            .await
            .expect("run should upsert");

        let mut readable_item = discovery_prune_item(run_id, now);
        readable_item.id = format!("{run_id}:readable");
        readable_item.target_key = "tmdb:movie:10".to_string();
        readable_item.facet_terms = vec!["canonical:genre:drama".to_string()];
        readable_item.library_provenance = vec![DiscoveryItemLibraryProvenanceRecord {
            subject_key: "tmdb:movie:10".to_string(),
            title_id: Some("library-title-10".to_string()),
            library_id: Some("movie-library".to_string()),
        }];

        let mut unreadable_item = discovery_prune_item(run_id, now);
        unreadable_item.id = format!("{run_id}:unreadable");
        unreadable_item.target_key = "tmdb:movie:11".to_string();
        unreadable_item.facet_terms = vec!["canonical:genre:comedy".to_string()];
        unreadable_item.library_provenance = vec![DiscoveryItemLibraryProvenanceRecord {
            subject_key: "tmdb:movie:11".to_string(),
            title_id: Some("library-title-11".to_string()),
            library_id: Some("other-library".to_string()),
        }];

        store
            .replace_discovery_items(run_id, &[readable_item, unreadable_item])
            .await
            .expect("items should upsert");

        let facets = fetch_personalized_facets(
            &store.datastore,
            run_id,
            &["movie-library".to_string()],
            &["movie".to_string()],
            true,
        )
        .await
        .expect("facets should load");

        assert_eq!(facets.len(), 1);
        assert_eq!(facets[0].facet_name, "genre");
        assert_eq!(facets[0].facet_value, "Drama");
        assert_eq!(facets[0].local_count, Some(1));

        let _ = std::fs::remove_file(db);
    }

    fn discovery_prune_run(
        id: &str,
        kind: &str,
        status: &str,
        observed_at: chrono::DateTime<Utc>,
    ) -> DiscoverySyncRunRecord {
        DiscoverySyncRunRecord {
            id: id.to_string(),
            kind: kind.to_string(),
            status: status.to_string(),
            trigger_source: "scheduled_interval".to_string(),
            region: "US".to_string(),
            language: "eng".to_string(),
            subject_count: 1,
            subject_fingerprint: Some(format!("{id}-fingerprint")),
            previous_subject_fingerprint: None,
            base_generation_id: None,
            changed_subject_count: 0,
            affected_target_count: 0,
            smg_request_id: None,
            smg_status: Some(status.to_string()),
            discovery_index_watermark: None,
            page_count: None,
            item_count: Some(0),
            facet_count: Some(0),
            acknowledged_at: None,
            error_text: None,
            started_at: Some(observed_at),
            completed_at: if status == "running" {
                None
            } else {
                Some(observed_at)
            },
            created_at: observed_at,
            updated_at: observed_at,
        }
    }

    fn discovery_prune_item(
        run_id: &str,
        observed_at: chrono::DateTime<Utc>,
    ) -> DiscoveryItemRecord {
        DiscoveryItemRecord {
            id: format!("{run_id}:item:tmdb:movie:604"),
            run_id: run_id.to_string(),
            base_generation_id: Some(run_id.to_string()),
            source_run_kind: "context_snapshot".to_string(),
            section_id: None,
            sort_index: 0,
            target_key: "tmdb:movie:604".to_string(),
            target_kind: "movie".to_string(),
            resolved: false,
            resolved_title_id: None,
            display_title: "Pruned Movie".to_string(),
            original_title: None,
            sort_title: Some("Pruned Movie".to_string()),
            year: Some(2026),
            poster_path: None,
            poster_url: None,
            background_url: None,
            overview: None,
            content_type: Some("movie".to_string()),
            canonical_tags: Vec::new(),
            is_adult: false,
            content_ratings: Vec::new(),
            rating: None,
            rating_sources: Vec::new(),
            external_ratings: Vec::new(),
            external_ids: Vec::new(),
            status_tags: Vec::new(),
            source_tags: Vec::new(),
            sources: Vec::new(),
            best_source: None,
            relation_types: Vec::new(),
            relation_subtypes: Vec::new(),
            chart_signals: Vec::new(),
            provider_signals: Vec::new(),
            rank_components: Vec::new(),
            source_count: Some(1),
            edge_count: Some(0),
            relation_count: Some(0),
            source_subject_count: Some(0),
            rank_score: Some(0.1),
            matched_subject_keys: Vec::new(),
            matched_subject_titles: Vec::new(),
            matched_subject_count: 0,
            library_provenance: Vec::new(),
            tmdb_collection_id: None,
            tmdb_collection_name: None,
            owned_in_input: false,
            studio_slug: None,
            person_ids: Vec::new(),
            facet_terms: Vec::new(),
            context_terms: Vec::new(),
            change_subject_keys: Vec::new(),
            removed_subject_keys: Vec::new(),
            tombstoned_by_run_id: None,
            tombstoned_at: None,
            created_at: observed_at,
            updated_at: observed_at,
        }
    }
}
