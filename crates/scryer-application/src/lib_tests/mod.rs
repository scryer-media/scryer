use super::*;
#[allow(unused_imports)]
use crate::test_wait::{TEST_WAIT_DEADLINE, wait_for, wait_until, within_deadline};
use async_trait::async_trait;
use base64::Engine as _;
use scryer_domain::{
    Collection, CollectionType, DomainEventFilter, DomainEventPayload, DomainEventType, Episode,
    EpisodeType, EventType, ImportSkipReason, ImportType, JobRunCompletedEventData,
    JobRunStartedEventData, MediaRequestRequester, MediaRequestStatus, RootFolderEntry,
    TrackedDownloadState,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, Instant, sleep, timeout};

mod acquisition_recovery;
mod bridge_cour_titles;
mod consolidation;
mod cross_library_transfer;
mod diacritic_release_matching;
mod discovery_sync;
mod downloads;
mod episode_file_delete;
mod folder_match;
mod full_hash_backfill;
mod golden_resolution_corpus;
mod import_rejection_reopen;
mod indexer_backoff_reset;
mod indexer_download_client_mappings;
mod interactive_release_search;
mod landed_bar_memo;
mod libraries;
mod library_scan;
mod list_actions;
mod list_experimental_gate;
mod list_provider_settings;
mod maintenance_action_sequences;
mod maintenance_claims;
mod maintenance_evaluation;
mod maintenance_execution;
mod maintenance_rules;
mod maintenance_safety;
mod maintenance_sequence_deletion;
mod maintenance_sequence_execution;
mod maintenance_sequence_history;
mod maintenance_storage_execution;
mod maintenance_title_completion;
mod maintenance_watch_facts;
mod media_analysis;
mod media_requests;
mod media_server_signals;
mod metadata_search;
mod multilingual_title_matching;
mod queueing;
mod recycle_bin_roots;
mod release_anchor_prefetch;
mod request_rules;
mod request_rules_facts;
mod romaji_release_matching;
mod root_change;
mod root_move;
mod routing_settings;
mod rule_tester_listing;
mod search_cutoff;
mod security_auth;
mod seeding_gate;
mod seeding_profiles;
mod series_metadata;
mod series_title_hydration;
mod subtitle_permissions;
mod title_catalog_reads;
mod title_hydration;
mod title_image_cache;
mod title_matcher_invalidation;
mod title_updates;
mod user_permissions;
mod users_admin_titles;
mod verification_settings;

mod request_rules_support;
mod support_acquisition_downloads;
mod support_bootstrap_fixtures;
mod support_catalog;
mod support_events_requests;
mod support_imports;
mod support_indexers_metadata;
mod support_library_show;
mod support_settings_scan;
use support_acquisition_downloads::*;
pub(crate) use support_bootstrap_fixtures::bootstrap;
#[cfg(unix)]
pub(crate) use support_bootstrap_fixtures::bootstrap_application_upgrade;
use support_bootstrap_fixtures::*;
use support_catalog::*;
use support_events_requests::*;
pub(crate) use support_imports::MockMediaFileRepo;
use support_imports::*;
use support_indexers_metadata::*;
use support_library_show::*;
use support_settings_scan::*;

/// Answer a movie `titles` request the way SMG answers it for TVDB-backed
/// movies, from a test double's TVDB-keyed bulk answer. A ref without a TVDB
/// id, or one the double has no movie for, is reported missing.
async fn movie_titles_from_tvdb_bulk<G: MetadataGateway + ?Sized>(
    gateway: &G,
    refs: &[MovieTitleRef],
    language: &str,
) -> AppResult<MovieTitleBulkResult> {
    let tvdb_ids = refs
        .iter()
        .filter_map(|movie_ref| movie_ref.tvdb_id)
        .collect::<Vec<_>>();
    let bulk = if tvdb_ids.is_empty() {
        BulkMetadataResult::default()
    } else {
        gateway.get_metadata_bulk(&tvdb_ids, &[], language).await?
    };
    let mut result = MovieTitleBulkResult::default();
    for (index, movie_ref) in refs.iter().enumerate() {
        match movie_ref
            .tvdb_id
            .and_then(|tvdb_id| bulk.movies.get(&tvdb_id))
        {
            Some(movie) => {
                result.by_ref_index.insert(index, movie.clone());
            }
            None => result.missing_ref_indexes.push(index),
        }
    }
    Ok(result)
}

/// Answer a movie `searchTitlesBatch` request with a test double's TVDB batch
/// answers. Other kinds are not served.
async fn movie_title_batch_from_tvdb<G: MetadataGateway + ?Sized>(
    gateway: &G,
    queries: &[MetadataSearchQuery],
    kind: &str,
    language: &str,
) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
    if kind != "movie" {
        return Err(AppError::Repository(
            "metadata gateway searchTitlesBatch is not implemented".into(),
        ));
    }
    gateway.search_tvdb_batch(queries, language).await
}

/// Location operations run in the background by contract (FR-030): a story
/// test watches the operation row until it reaches a terminal state, the way
/// Activity does. Shared by every location story fixture.
async fn settle_location_operation(
    app: &AppUseCase,
    operation_id: &str,
) -> crate::location::model::LocationOperation {
    wait_for(
        "the location operation to reach a terminal state",
        || async {
            let operation = app
                .location_operation(operation_id)
                .await
                .expect("read operation")
                .expect("operation row exists");
            operation.state.is_terminal().then_some(operation)
        },
    )
    .await
}

#[derive(Default)]
pub(super) struct RecordingScopeIndexerCoverageRepo {
    rows: Mutex<Vec<(String, String, String, String)>>,
    /// How many times `list_coverage_for_scope_keys` was called.
    list_calls: AtomicUsize,
}

impl RecordingScopeIndexerCoverageRepo {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) async fn recorded(&self) -> Vec<(String, String, String, String)> {
        self.rows.lock().await.clone()
    }

    pub(super) fn list_calls(&self) -> usize {
        self.list_calls.load(Ordering::SeqCst)
    }

    pub(super) async fn indexers_for_scope(&self, scope_key: &str) -> Vec<String> {
        self.rows
            .lock()
            .await
            .iter()
            .filter(|(recorded_scope_key, _, _, _)| recorded_scope_key == scope_key)
            .map(|(_, _, indexer_id, _)| indexer_id.clone())
            .collect()
    }
}

#[async_trait]
impl ScopeIndexerCoverageRepository for RecordingScopeIndexerCoverageRepo {
    async fn record_coverage(
        &self,
        scope_key: &str,
        facet: &str,
        indexer_id: &str,
        fingerprint: &str,
    ) -> AppResult<()> {
        let mut rows = self.rows.lock().await;
        rows.retain(|(sk, f, id, _)| !(sk == scope_key && f == facet && id == indexer_id));
        rows.push((
            scope_key.to_string(),
            facet.to_string(),
            indexer_id.to_string(),
            fingerprint.to_string(),
        ));
        Ok(())
    }

    async fn covered_indexers(
        &self,
        scope_key: &str,
        facet: &str,
        fingerprint: &str,
        _stale_before: Option<chrono::DateTime<chrono::Utc>>,
    ) -> AppResult<Vec<String>> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .filter(|(sk, f, _, fp)| sk == scope_key && f == facet && fp == fingerprint)
            .map(|(_, _, indexer_id, _)| indexer_id.clone())
            .collect())
    }

    async fn prune_scope(&self, scope_key: &str) -> AppResult<()> {
        self.rows
            .lock()
            .await
            .retain(|(sk, _, _, _)| sk != scope_key);
        Ok(())
    }

    async fn prune_scope_indexer(&self, scope_key: &str, indexer_id: &str) -> AppResult<()> {
        self.rows
            .lock()
            .await
            .retain(|(sk, _, id, _)| sk != scope_key || id != indexer_id);
        Ok(())
    }

    async fn list_coverage_for_scope_keys(
        &self,
        scope_keys: &[String],
    ) -> AppResult<Vec<ScopeCoverageRow>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        let wanted: HashSet<&str> = scope_keys.iter().map(String::as_str).collect();
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .filter(|(scope_key, _, _, _)| wanted.contains(scope_key.as_str()))
            .map(|(scope_key, _, indexer_id, fingerprint)| ScopeCoverageRow {
                scope_key: scope_key.clone(),
                indexer_id: indexer_id.clone(),
                fingerprint: fingerprint.clone(),
                searched_at: chrono::Utc::now().to_rfc3339(),
            })
            .collect())
    }
}
