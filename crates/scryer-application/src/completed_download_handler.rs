//! CompletedDownloadHandler — two-phase import bridge (plan 055).
//!
//! Phase 1 (check): validate completed downloads, resolve title, gate auto-import.
//! Phase 2 (import): run the import pipeline, verify completion across passes.

use scryer_domain::{
    CompletedDownload, DownloadQueueState, ImportDecision, ImportResult, ImportSkipReason,
    ImportStatus, NotificationEventType, TitleMatchType, TrackedDownloadState,
    TrackedDownloadStatus,
};
use std::collections::HashSet;

use crate::app_usecase_import::import_completed_download;
use crate::tracked_downloads::TrackedDownload;
use crate::{ActivityChannel, ActivityKind, ActivitySeverity, AppUseCase, User};

enum ExpectedEpisodeResolution {
    NotApplicable,
    Unresolved,
    Resolved(HashSet<String>),
}

/// Phase 1: evaluate a tracked download whose client reports completion.
///
/// Called every poll cycle for downloads in Downloading or ImportBlocked state.
/// Transitions to ImportPending if all validations pass, or ImportBlocked with
/// warnings if auto-import is not safe.
pub async fn check(app: &AppUseCase, td: &mut TrackedDownload) {
    // Only process if client reports completed.
    if td.client_item.state != DownloadQueueState::Completed {
        return;
    }

    // Only process if still in a check-eligible state.
    if td.state != TrackedDownloadState::Downloading
        && td.state != TrackedDownloadState::ImportBlocked
    {
        return;
    }

    // Don't re-evaluate a post-import block. Import already ran and returned
    // Skipped/Failed — stay blocked until the user explicitly retries.
    if td.state == TrackedDownloadState::ImportBlocked && td.import_attempted {
        return;
    }

    // Validate output path.
    // (For now, we trust the client's dest_dir will be available at import time.
    //  A full ValidatePath check would need the CompletedDownload which requires
    //  an async call — handled in the import phase.)

    // Auto-import safety gating.
    match td.match_type {
        TitleMatchType::Unmatched => {
            if !td.status_messages.iter().any(|m| m.contains("couldn't be matched")) {
                td.status_messages.clear();
                td.warn("Download couldn't be matched to a library title. Assign a title manually or check the download name.");
            }
            set_state_to_import_blocked(app, td).await;
            return;
        }
        TitleMatchType::IdOnly => {
            // ID-only matches from automated grabs are too risky for auto-import.
            // Interactive searches (user confirmed) are trusted.
            if !td.client_item.is_scryer_origin {
                if !td.status_messages.iter().any(|m| m.contains("matched by ID only")) {
                    td.status_messages.clear();
                    td.warn("Download was matched to a title by ID only. Manual confirmation required to import.");
                }
                set_state_to_import_blocked(app, td).await;
                return;
            }
        }
        TitleMatchType::Submission | TitleMatchType::ClientParameter | TitleMatchType::TitleParse => {
            // High-confidence matches — proceed.
        }
    }

    // Check that the resolved title still exists.
    // (This is a sync check against cached data; the actual title lookup
    //  was done during resolve_title. If the title was deleted since then,
    //  title_id will still be set but import will fail gracefully.)

    if td.title_id.is_none() || td.title_id.as_deref() == Some("") {
        td.warn("No title linked to this download.");
        set_state_to_import_blocked(app, td).await;
        return;
    }

    // All checks passed — queue for import.
    tracing::info!(
        id = %td.id,
        title_id = ?td.title_id,
        match_type = ?td.match_type,
        "check: transitioning to ImportPending"
    );
    td.state = TrackedDownloadState::ImportPending;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();
}

/// Phase 2: run the actual import for a download in ImportPending state.
///
/// This is async because it calls the import pipeline. Returns true if the
/// download transitioned to a terminal state (Imported or ImportBlocked).
pub async fn import(
    app: &AppUseCase,
    actor: &User,
    td: &mut TrackedDownload,
) -> bool {
    if td.state != TrackedDownloadState::ImportPending {
        return false;
    }

    td.state = TrackedDownloadState::Importing;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();

    let Some(completed) = find_completed_download(app, td).await else {
        tracing::debug!(
            id = %td.id,
            item_id = %td.client_item.download_client_item_id,
            "import: completed download not found in client history, will retry"
        );
        td.state = TrackedDownloadState::ImportPending;
        return false;
    };

    tracing::info!(
        id = %td.id,
        dest_dir = %completed.dest_dir,
        title_id = ?td.title_id,
        "import: starting import from completed download"
    );

    let success_before = total_successful_artifacts(app, td).await;
    td.import_attempted = true;

    match import_completed_download(app, actor, &completed).await {
        Ok(result) => {
            let success_after = total_successful_artifacts(app, td).await;
            let files_imported_this_pass = success_after.saturating_sub(success_before) as usize;
            tracing::info!(
                id = %td.id,
                decision = ?result.decision,
                skip_reason = ?result.skip_reason,
                error_message = ?result.error_message,
                files_imported_this_pass,
                "import: pipeline returned result"
            );
            apply_import_result(app, td, result, files_imported_this_pass).await
        }
        Err(error) => {
            tracing::warn!(
                id = %td.id,
                error = %error,
                dest_dir = %completed.dest_dir,
                "import: pipeline returned error"
            );
            td.state = TrackedDownloadState::ImportBlocked;
            td.status = TrackedDownloadStatus::Error;
            td.status_messages = vec![format!("Import failed: {error}")];
            false
        }
    }
}

/// Verify whether a download's import is complete by checking cumulative
/// artifact history across all passes.
///
/// Returns true if all expected files are accounted for (imported or already_present).
pub async fn verify_import(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
) -> bool {
    let source_ref = &td.client_item.download_client_item_id;

    let artifacts = match app
        .services
        .import_artifacts
        .list_by_source_ref(&td.client_type, source_ref)
        .await
    {
        Ok(artifacts) => artifacts,
        Err(_) => return false,
    };

    if artifacts.is_empty() {
        return false;
    }

    let current_visible_files = current_visible_video_file_count(app, td).await;
    let mut successful_units = HashSet::new();
    let mut rejected_units = HashSet::new();

    for artifact in artifacts {
        let logical_unit = artifact
            .episode_id
            .clone()
            .unwrap_or_else(|| format!("{}:{}", artifact.media_kind, artifact.normalized_file_name));

        match artifact.result.as_str() {
            "imported" | "already_present" => {
                successful_units.insert(logical_unit);
            }
            "rejected" => {
                rejected_units.insert(logical_unit);
            }
            _ => {}
        }
    }

    if successful_units.is_empty() {
        return false;
    }

    if td.facet.as_deref() == Some("movie") {
        return !successful_units.is_empty();
    }

    match expected_episode_units(app, td).await {
        ExpectedEpisodeResolution::Resolved(expected_episode_units) => {
            if expected_episode_units.is_empty() {
                return false;
            }

            return expected_episode_units
                .iter()
                .all(|unit| successful_units.contains(unit));
        }
        ExpectedEpisodeResolution::Unresolved => {
            if successful_units_cover_visible_files(successful_units.len(), current_visible_files) {
                return true;
            }

            return files_imported_this_pass > 0 && rejected_units.is_empty();
        }
        ExpectedEpisodeResolution::NotApplicable => {}
    }

    if successful_units_cover_visible_files(successful_units.len(), current_visible_files) {
        return true;
    }

    !successful_units.is_empty()
}

fn successful_units_cover_visible_files(
    successful_unit_count: usize,
    current_visible_files: usize,
) -> bool {
    current_visible_files > 0 && successful_unit_count >= current_visible_files
}

async fn find_completed_download(
    app: &AppUseCase,
    td: &TrackedDownload,
) -> Option<CompletedDownload> {
    let completed_downloads = match app.services.download_client.list_completed_downloads().await {
        Ok(downloads) => downloads,
        Err(error) => {
            tracing::warn!(error = %error, "find_completed_download: failed to fetch from client");
            return None;
        }
    };
    tracing::debug!(
        count = completed_downloads.len(),
        looking_for = %td.client_item.download_client_item_id,
        "find_completed_download: searching client history"
    );
    let completed = completed_downloads.into_iter().find(|completed| {
        completed.client_type == td.client_type
            && completed.download_client_item_id == td.client_item.download_client_item_id
    });
    match completed {
        Some(completed) => {
            if completed.dest_dir.trim().is_empty() {
                tracing::warn!(id = %td.id, "find_completed_download: matched but dest_dir is empty");
            }
            let path = std::path::Path::new(&completed.dest_dir);
            if !path.as_os_str().is_empty() && !path.exists() {
                tracing::warn!(
                    id = %td.id,
                    dest_dir = %completed.dest_dir,
                    "find_completed_download: dest_dir does not exist on disk — check volume mounts"
                );
            }
            Some(with_tracked_metadata(td, completed))
        }
        None => {
            tracing::debug!(
                id = %td.id,
                item_id = %td.client_item.download_client_item_id,
                client_type = %td.client_type,
                "find_completed_download: no matching item in client history"
            );
            None
        }
    }
}

fn with_tracked_metadata(
    td: &TrackedDownload,
    mut completed: CompletedDownload,
) -> CompletedDownload {
    upsert_parameter(
        &mut completed.parameters,
        "*scryer_title_id",
        td.title_id.clone().unwrap_or_default(),
    );
    upsert_parameter(
        &mut completed.parameters,
        "*scryer_facet",
        td.facet.clone().unwrap_or_default(),
    );
    completed
}

fn upsert_parameter(params: &mut Vec<(String, String)>, key: &str, value: String) {
    if value.trim().is_empty() {
        return;
    }

    if let Some((_, existing)) = params.iter_mut().find(|(existing_key, _)| existing_key == key) {
        *existing = value;
    } else {
        params.push((key.to_string(), value));
    }
}

async fn apply_import_result(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    result: ImportResult,
    files_imported_this_pass: usize,
) -> bool {
    match result.decision {
        ImportDecision::Imported => {
            if verify_import(app, td, files_imported_this_pass).await {
                td.state = TrackedDownloadState::Imported;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                true
            } else {
                td.state = TrackedDownloadState::ImportPending;
                td.status = TrackedDownloadStatus::Warning;
                td.status_messages = vec![
                    "Import partially completed; waiting for remaining files or verification."
                        .to_string(),
                ];
                false
            }
        }
        ImportDecision::Skipped
            if matches!(result.skip_reason, Some(ImportSkipReason::AlreadyImported)) =>
        {
            if verify_import(app, td, files_imported_this_pass).await {
                td.state = TrackedDownloadState::Imported;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                true
            } else {
                td.state = TrackedDownloadState::ImportBlocked;
                td.status = TrackedDownloadStatus::Warning;
                td.status_messages = vec![import_result_message(&result, ImportStatus::Skipped)];
                false
            }
        }
        ImportDecision::Failed => {
            td.state = TrackedDownloadState::ImportBlocked;
            td.status = TrackedDownloadStatus::Error;
            td.status_messages = vec![import_result_message(&result, ImportStatus::Failed)];
            false
        }
        _ => {
            td.state = TrackedDownloadState::ImportBlocked;
            td.status = TrackedDownloadStatus::Warning;
            td.status_messages = vec![import_result_message(&result, ImportStatus::Skipped)];
            false
        }
    }
}

fn import_result_message(result: &ImportResult, fallback_status: ImportStatus) -> String {
    if let Some(message) = result.error_message.as_ref().filter(|message| !message.trim().is_empty()) {
        return message.clone();
    }

    if let Some(skip_reason) = result.skip_reason.as_ref() {
        return format!("Import blocked: {}", skip_reason.as_str());
    }

    format!("Import ended with status {}", fallback_status.as_str())
}

async fn expected_episode_units(
    app: &AppUseCase,
    td: &TrackedDownload,
) -> ExpectedEpisodeResolution {
    let Some(title_id) = td.title_id.as_deref() else {
        return ExpectedEpisodeResolution::Unresolved;
    };
    let Some(title) = app.services.titles.get_by_id(title_id).await.ok().flatten() else {
        return ExpectedEpisodeResolution::Unresolved;
    };

    let release_title = td
        .source_title
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(td.client_item.title_name.as_str());
    let parsed = crate::parse_release_metadata(release_title);
    let Some(ep_meta) = parsed.episode.as_ref() else {
        return ExpectedEpisodeResolution::NotApplicable;
    };
    let season_str = ep_meta.season.unwrap_or(1).to_string();
    let episodes = crate::app_usecase_import::resolve_target_episodes(app, &title, ep_meta, &season_str).await;

    if episodes.is_empty() {
        return ExpectedEpisodeResolution::Unresolved;
    }

    let expected_lookup_count = if ep_meta.season.is_some() && !ep_meta.episode_numbers.is_empty() {
        ep_meta.episode_numbers.iter().copied().collect::<HashSet<_>>().len()
    } else if ep_meta.absolute_episode.is_some() {
        if ep_meta.episode_numbers.is_empty() {
            1
        } else {
            ep_meta.episode_numbers.iter().copied().collect::<HashSet<_>>().len()
        }
    } else {
        0
    };

    if expected_lookup_count > 0 && episodes.len() < expected_lookup_count {
        return ExpectedEpisodeResolution::Unresolved;
    }

    ExpectedEpisodeResolution::Resolved(episodes.into_iter().map(|episode| episode.id).collect())
}

async fn set_state_to_import_blocked(app: &AppUseCase, td: &mut TrackedDownload) {
    td.state = TrackedDownloadState::ImportBlocked;
    td.status = TrackedDownloadStatus::Warning;

    if td.notified_manual_interaction {
        return;
    }

    td.notified_manual_interaction = true;
    let message = td
        .status_messages
        .first()
        .cloned()
        .unwrap_or_else(|| "Manual interaction required for this download.".to_string());

    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        "download_client_item_id".to_string(),
        serde_json::json!(td.client_item.download_client_item_id),
    );
    metadata.insert("download_title".to_string(), serde_json::json!(td.client_item.title_name));

    let envelope = crate::activity::NotificationEnvelope {
        event_type: NotificationEventType::ManualInteractionRequired,
        title: format!("Manual interaction required: {}", td.client_item.title_name),
        body: message.clone(),
        facet: td.facet.clone(),
        metadata,
    };

    let _ = app
        .services
        .record_activity_event_with_notification(
            None,
            td.title_id.clone(),
            td.facet.clone(),
            ActivityKind::SystemNotice,
            message,
            ActivitySeverity::Warning,
            vec![ActivityChannel::WebUi, ActivityChannel::Toast],
            envelope,
        )
        .await;
}

async fn total_successful_artifacts(app: &AppUseCase, td: &TrackedDownload) -> u64 {
    let source_ref = &td.client_item.download_client_item_id;
    let imported = app
        .services
        .import_artifacts
        .count_by_result(&td.client_type, source_ref, "imported")
        .await
        .unwrap_or(0);
    let already_present = app
        .services
        .import_artifacts
        .count_by_result(&td.client_type, source_ref, "already_present")
        .await
        .unwrap_or(0);
    imported + already_present
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullEventRepository,
        NullIndexerClient, NullReleaseAttemptRepository, NullUserRepository,
    };
    use crate::{
        AppError, AppResult, AppServices, AppUseCase,
        FacetRegistry, ImportArtifact, ImportArtifactRepository, IndexerConfigRepository,
        JwtAuthConfig, QualityProfile, QualityProfileRepository, ShowRepository,
        TitleMetadataUpdate, TitleRepository,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use scryer_domain::{
        CalendarEpisode, Collection, CollectionType, DownloadQueueItem, DownloadQueueState,
        Episode, EpisodeType, Id, MediaFacet, NotificationEventType, Title,
        TitleMatchType, TrackedDownloadState, TrackedDownloadStatus, User,
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct TestTitleRepo {
        titles: Arc<Mutex<Vec<Title>>>,
    }

    #[async_trait]
    impl TitleRepository for TestTitleRepo {
        async fn list(
            &self,
            facet: Option<MediaFacet>,
            query: Option<String>,
        ) -> AppResult<Vec<Title>> {
            let titles = self.titles.lock().await.clone();
            Ok(titles
                .into_iter()
                .filter(|title| facet.as_ref().is_none_or(|expected| &title.facet == expected))
                .filter(|title| {
                    query
                        .as_ref()
                        .is_none_or(|value| title.name.to_ascii_lowercase().contains(&value.to_ascii_lowercase()))
                })
                .collect())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
            let titles = self.titles.lock().await;
            Ok(titles.iter().find(|title| title.id == id).cloned())
        }

        async fn create(&self, title: Title) -> AppResult<Title> {
            self.titles.lock().await.push(title.clone());
            Ok(title)
        }

        async fn update_metadata(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<MediaFacet>,
            _: Option<Vec<String>>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_monitored(&self, _: &str, _: bool) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_title_hydrated_metadata(
            &self,
            _: &str,
            _: TitleMetadataUpdate,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn set_folder_path(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn list_unhydrated(&self, _: usize, _: &str) -> AppResult<Vec<Title>> {
            Ok(vec![])
        }

        async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
            Ok(0)
        }
    }

    #[derive(Default)]
    struct TestShowRepo {
        collections: Arc<Mutex<Vec<Collection>>>,
        episodes: Arc<Mutex<Vec<Episode>>>,
    }

    #[async_trait]
    impl ShowRepository for TestShowRepo {
        async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
            let collections = self.collections.lock().await;
            Ok(collections
                .iter()
                .filter(|collection| collection.title_id == title_id)
                .cloned()
                .collect())
        }

        async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
            let collections = self.collections.lock().await;
            Ok(collections
                .iter()
                .find(|collection| collection.id == collection_id)
                .cloned())
        }

        async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
            self.collections.lock().await.push(collection.clone());
            Ok(collection)
        }

        async fn update_collection(
            &self,
            _: &str,
            _: Option<CollectionType>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<bool>,
        ) -> AppResult<Collection> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_interstitial_season_episode(
            &self,
            _: &str,
            _: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn set_collection_episodes_monitored(&self, _: &str, _: bool) -> AppResult<()> {
            Ok(())
        }

        async fn delete_collection(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .filter(|episode| episode.collection_id.as_deref() == Some(collection_id))
                .cloned()
                .collect())
        }

        async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes.iter().find(|episode| episode.id == episode_id).cloned())
        }

        async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
            self.episodes.lock().await.push(episode.clone());
            Ok(episode)
        }

        async fn update_episode(
            &self,
            _: &str,
            _: Option<EpisodeType>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<i64>,
            _: Option<bool>,
            _: Option<bool>,
            _: Option<bool>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
        ) -> AppResult<Episode> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete_episode(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn find_episode_by_title_and_numbers(
            &self,
            title_id: &str,
            season_number: &str,
            episode_number: &str,
        ) -> AppResult<Option<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .find(|episode| {
                    episode.title_id == title_id
                        && episode.season_number.as_deref() == Some(season_number)
                        && episode.episode_number.as_deref() == Some(episode_number)
                })
                .cloned())
        }

        async fn find_episode_by_title_and_absolute_number(
            &self,
            title_id: &str,
            absolute_number: &str,
        ) -> AppResult<Option<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .find(|episode| {
                    episode.title_id == title_id
                        && episode.absolute_number.as_deref() == Some(absolute_number)
                })
                .cloned())
        }

        async fn list_primary_collection_summaries(
            &self,
            _: &[String],
        ) -> AppResult<Vec<crate::PrimaryCollectionSummary>> {
            Ok(vec![])
        }

        async fn list_episodes_in_date_range(
            &self,
            _: &str,
            _: &str,
        ) -> AppResult<Vec<CalendarEpisode>> {
            Ok(vec![])
        }
    }

    #[derive(Default)]
    struct TestImportArtifactRepo {
        artifacts: Arc<Mutex<Vec<ImportArtifact>>>,
    }

    #[async_trait]
    impl ImportArtifactRepository for TestImportArtifactRepo {
        async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()> {
            self.artifacts.lock().await.push(artifact);
            Ok(())
        }

        async fn list_by_source_ref(
            &self,
            source_system: &str,
            source_ref: &str,
        ) -> AppResult<Vec<ImportArtifact>> {
            let artifacts = self.artifacts.lock().await;
            Ok(artifacts
                .iter()
                .filter(|artifact| {
                    artifact.source_system == source_system && artifact.source_ref == source_ref
                })
                .cloned()
                .collect())
        }

        async fn count_by_result(
            &self,
            source_system: &str,
            source_ref: &str,
            result: &str,
        ) -> AppResult<u64> {
            let artifacts = self.artifacts.lock().await;
            Ok(artifacts
                .iter()
                .filter(|artifact| {
                    artifact.source_system == source_system
                        && artifact.source_ref == source_ref
                        && artifact.result == result
                })
                .count() as u64)
        }
    }

    #[derive(Default)]
    struct TestIndexerConfigRepo;

    #[async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepo {
        async fn list(&self, _: Option<String>) -> AppResult<Vec<scryer_domain::IndexerConfig>> {
            Ok(vec![])
        }

        async fn get_by_id(&self, _: &str) -> AppResult<Option<scryer_domain::IndexerConfig>> {
            Ok(None)
        }

        async fn create(
            &self,
            _: scryer_domain::IndexerConfig,
        ) -> AppResult<scryer_domain::IndexerConfig> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn touch_last_error(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<i64>,
            _: Option<i64>,
            _: Option<bool>,
            _: Option<bool>,
            _: Option<bool>,
            _: Option<String>,
        ) -> AppResult<scryer_domain::IndexerConfig> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestQualityProfileRepo;

    #[async_trait]
    impl QualityProfileRepository for TestQualityProfileRepo {
        async fn list_quality_profiles(
            &self,
            _: &str,
            _: Option<String>,
        ) -> AppResult<Vec<QualityProfile>> {
            Ok(vec![])
        }
    }

    fn build_app(
        titles: Vec<Title>,
        collections: Vec<Collection>,
        episodes: Vec<Episode>,
        artifacts: Vec<ImportArtifact>,
    ) -> AppUseCase {
        let mut services = AppServices::with_default_channels(
            Arc::new(TestTitleRepo {
                titles: Arc::new(Mutex::new(titles)),
            }),
            Arc::new(TestShowRepo {
                collections: Arc::new(Mutex::new(collections)),
                episodes: Arc::new(Mutex::new(episodes)),
            }),
            Arc::new(NullUserRepository),
            Arc::new(NullEventRepository),
            Arc::new(TestIndexerConfigRepo),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(crate::null_repositories::NullSettingsRepository),
            Arc::new(TestQualityProfileRepo),
            String::new(),
        );
        services.import_artifacts = Arc::new(TestImportArtifactRepo {
            artifacts: Arc::new(Mutex::new(artifacts)),
        });

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn build_title(id: &str, name: &str, facet: MediaFacet) -> Title {
        Title {
            id: id.to_string(),
            name: name.to_string(),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn build_collection(id: &str, title_id: &str, season: &str) -> Collection {
        Collection {
            id: id.to_string(),
            title_id: title_id.to_string(),
            collection_type: CollectionType::Season,
            collection_index: season.to_string(),
            label: Some(format!("Season {season}")),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn build_episode(
        id: &str,
        title_id: &str,
        collection_id: &str,
        season_number: &str,
        episode_number: &str,
        absolute_number: Option<&str>,
    ) -> Episode {
        build_episode_with_details(
            id,
            title_id,
            collection_id,
            EpisodeType::Standard,
            season_number,
            episode_number,
            None,
            absolute_number,
        )
    }

    fn build_episode_with_details(
        id: &str,
        title_id: &str,
        collection_id: &str,
        episode_type: EpisodeType,
        season_number: &str,
        episode_number: &str,
        air_date: Option<&str>,
        absolute_number: Option<&str>,
    ) -> Episode {
        Episode {
            id: id.to_string(),
            title_id: title_id.to_string(),
            collection_id: Some(collection_id.to_string()),
            episode_type,
            episode_number: Some(episode_number.to_string()),
            season_number: Some(season_number.to_string()),
            episode_label: None,
            title: None,
            air_date: air_date.map(str::to_string),
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: absolute_number.map(str::to_string),
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn build_artifact(
        source_ref: &str,
        episode_id: &str,
        normalized_file_name: &str,
    ) -> ImportArtifact {
        build_artifact_with_result(
            source_ref,
            Some(episode_id),
            normalized_file_name,
            "imported",
        )
    }

    fn build_artifact_with_result(
        source_ref: &str,
        episode_id: Option<&str>,
        normalized_file_name: &str,
        result: &str,
    ) -> ImportArtifact {
        ImportArtifact {
            id: Id::new().0,
            source_system: "nzbget".to_string(),
            source_ref: source_ref.to_string(),
            import_id: None,
            relative_path: None,
            normalized_file_name: normalized_file_name.to_string(),
            media_kind: "episode".to_string(),
            title_id: Some("title-1".to_string()),
            episode_id: episode_id.map(str::to_string),
            season_number: Some(1),
            episode_number: None,
            result: result.to_string(),
            reason_code: None,
            imported_media_file_id: None,
            created_at: Utc::now(),
        }
    }

    fn build_tracked_download(
        title_id: &str,
        facet: &str,
        release_title: &str,
    ) -> TrackedDownload {
        TrackedDownload {
            id: format!("nzbget:{release_title}"),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: DownloadQueueItem {
                id: Id::new().0,
                title_id: Some(title_id.to_string()),
                title_name: release_title.to_string(),
                facet: Some(facet.to_string()),
                client_id: "client-1".to_string(),
                client_name: "NZBGet".to_string(),
                client_type: "nzbget".to_string(),
                state: DownloadQueueState::Completed,
                progress_percent: 100,
                size_bytes: None,
                remaining_seconds: None,
                queued_at: None,
                last_updated_at: None,
                attention_required: false,
                attention_reason: None,
                download_client_item_id: "dl-1".to_string(),
                import_status: None,
                import_error_message: None,
                imported_at: None,
                is_scryer_origin: true,
                tracked_state: None,
                tracked_status: None,
                tracked_status_messages: vec![],
                tracked_match_type: None,
            },
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: vec![],
            title_id: Some(title_id.to_string()),
            facet: Some(facet.to_string()),
            source_title: Some(release_title.to_string()),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
        }
    }

    #[tokio::test]
    async fn verify_import_requires_full_season_pack_coverage() {
        let title = build_title("title-1", "Star Trek Picard", MediaFacet::Series);
        let collection = build_collection("season-2", "title-1", "2");
        let episodes = vec![
            build_episode("ep-201", "title-1", "season-2", "2", "1", None),
            build_episode("ep-202", "title-1", "season-2", "2", "2", None),
            build_episode("ep-203", "title-1", "season-2", "2", "3", None),
        ];
        let artifacts = vec![
            build_artifact("dl-1", "ep-201", "S02E01.mkv"),
            build_artifact("dl-1", "ep-202", "S02E02.mkv"),
        ];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "series",
            "Star.Trek.Picard.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        );

        let parsed = crate::parse_release_metadata(
            "Star.Trek.Picard.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        );
        assert_eq!(parsed.episode.as_ref().and_then(|episode| episode.season), Some(2));

        match expected_episode_units(&app, &td).await {
            ExpectedEpisodeResolution::Resolved(expected) => assert_eq!(expected.len(), 3),
            _ => panic!("expected a resolved season-pack episode set"),
        }

        assert!(!verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_accepts_full_season_pack_coverage() {
        let title = build_title("title-1", "Star Trek Picard", MediaFacet::Series);
        let collection = build_collection("season-2", "title-1", "2");
        let episodes = vec![
            build_episode("ep-201", "title-1", "season-2", "2", "1", None),
            build_episode("ep-202", "title-1", "season-2", "2", "2", None),
            build_episode("ep-203", "title-1", "season-2", "2", "3", None),
        ];
        let artifacts = vec![
            build_artifact("dl-1", "ep-201", "S02E01.mkv"),
            build_artifact("dl-1", "ep-202", "S02E02.mkv"),
            build_artifact("dl-1", "ep-203", "S02E03.mkv"),
        ];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "series",
            "Star.Trek.Picard.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        );

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_ignores_rejected_extras_when_expected_units_are_satisfied() {
        let title = build_title("title-1", "Star Trek Picard", MediaFacet::Series);
        let collection = build_collection("season-2", "title-1", "2");
        let episodes = vec![
            build_episode("ep-201", "title-1", "season-2", "2", "1", None),
            build_episode("ep-202", "title-1", "season-2", "2", "2", None),
            build_episode("ep-203", "title-1", "season-2", "2", "3", None),
        ];
        let artifacts = vec![
            build_artifact("dl-1", "ep-201", "S02E01.mkv"),
            build_artifact("dl-1", "ep-202", "S02E02.mkv"),
            build_artifact("dl-1", "ep-203", "S02E03.mkv"),
            build_artifact_with_result("dl-1", None, "sample.mkv", "rejected"),
        ];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "series",
            "Star.Trek.Picard.S02.2022.Complete.1080p.Amazon.WEB-DL.AVC.DDP.5.1-DBTV",
        );

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_resolves_absolute_episode_ranges() {
        let title = build_title("title-1", "One Piece", MediaFacet::Anime);
        let collection = build_collection("season-22", "title-1", "22");
        let episodes = vec![
            build_episode("ep-1122", "title-1", "season-22", "22", "1", Some("1122")),
            build_episode("ep-1123", "title-1", "season-22", "22", "2", Some("1123")),
            build_episode("ep-1124", "title-1", "season-22", "22", "3", Some("1124")),
        ];
        let artifacts = vec![
            build_artifact("dl-1", "ep-1122", "1122.mkv"),
            build_artifact("dl-1", "ep-1123", "1123.mkv"),
            build_artifact("dl-1", "ep-1124", "1124.mkv"),
        ];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "anime",
            "[HatSubs] One Piece 1122-1124 (WEB 1080p)",
        );

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_resolves_daily_episode_by_air_date() {
        let title = build_title("title-1", "Series Title", MediaFacet::Series);
        let collection = build_collection("season-1", "title-1", "1");
        let episodes = vec![
            build_episode_with_details(
                "ep-101",
                "title-1",
                "season-1",
                EpisodeType::Standard,
                "1",
                "1",
                Some("2015-09-07"),
                None,
            ),
            build_episode_with_details(
                "ep-102",
                "title-1",
                "season-1",
                EpisodeType::Standard,
                "1",
                "2",
                Some("2015-09-08"),
                None,
            ),
        ];
        let artifacts = vec![build_artifact("dl-1", "ep-101", "Series.Title.2015.09.07.mkv")];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "series",
            "Series.Title.2015.09.07.Part.1.720p.HULU.WEBRip.AAC2.0.H.264-Sonarr",
        );

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_resolves_special_by_season_zero_number() {
        let title = build_title("title-1", "Another Anime Show", MediaFacet::Anime);
        let collection = build_collection("season-0", "title-1", "0");
        let episodes = vec![build_episode_with_details(
            "ep-special-1",
            "title-1",
            "season-0",
            EpisodeType::Ova,
            "0",
            "1",
            None,
            None,
        )];
        let artifacts = vec![build_artifact(
            "dl-1",
            "ep-special-1",
            "Another.Anime.Show.S00E01.ova.mkv",
        )];
        let app = build_app(vec![title], vec![collection], episodes, artifacts);
        let td = build_tracked_download(
            "title-1",
            "anime",
            "[DeadFish] Another Anime Show - 01 - OVA [BD][720p][AAC]",
        );

        assert!(verify_import(&app, &td, 0).await);
    }

    #[tokio::test]
    async fn verify_import_unresolved_episode_resolution_falls_back_to_successful_pass() {
        let title = build_title("title-1", "Mystery Show", MediaFacet::Series);
        let artifacts = vec![build_artifact_with_result(
            "dl-1",
            None,
            "Mystery.Show.S01E01.mkv",
            "imported",
        )];
        let app = build_app(vec![title], vec![], vec![], artifacts);
        let td = build_tracked_download(
            "title-1",
            "series",
            "Mystery.Show.S01E01.1080p.WEB-DL",
        );

        match expected_episode_units(&app, &td).await {
            ExpectedEpisodeResolution::Unresolved => {}
            _ => panic!("expected unresolved episodic resolution"),
        }

        assert!(verify_import(&app, &td, 1).await);
    }

    #[tokio::test]
    async fn check_emits_manual_interaction_notification_once() {
        let app = build_app(vec![], vec![], vec![], vec![]);
        let actor = User::new_admin("admin");
        let mut td = TrackedDownload {
            id: "nzbget:unmatched".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: DownloadQueueItem {
                id: Id::new().0,
                title_id: None,
                title_name: "Unknown.Show.S01.Complete.1080p".to_string(),
                facet: Some("series".to_string()),
                client_id: "client-1".to_string(),
                client_name: "NZBGet".to_string(),
                client_type: "nzbget".to_string(),
                state: DownloadQueueState::Completed,
                progress_percent: 100,
                size_bytes: None,
                remaining_seconds: None,
                queued_at: None,
                last_updated_at: None,
                attention_required: false,
                attention_reason: None,
                download_client_item_id: "dl-2".to_string(),
                import_status: None,
                import_error_message: None,
                imported_at: None,
                is_scryer_origin: false,
                tracked_state: None,
                tracked_status: None,
                tracked_status_messages: vec![],
                tracked_match_type: None,
            },
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: vec![],
            title_id: None,
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Unmatched,
            is_trackable: true,
            import_attempted: false,
        };

        check(&app, &mut td).await;
        check(&app, &mut td).await;

        assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
        assert!(td.notified_manual_interaction);

        let activity = app.recent_activity(&actor, 10, 0).await.unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(
            activity[0]
                .notification
                .as_ref()
                .map(|notification| notification.event_type),
            Some(NotificationEventType::ManualInteractionRequired)
        );
    }
}

async fn current_visible_video_file_count(app: &AppUseCase, td: &TrackedDownload) -> usize {
    let Some(completed) = find_completed_download(app, td).await else {
        return 0;
    };

    let path = std::path::Path::new(&completed.dest_dir);
    let filter_samples = td.facet.as_deref() != Some("movie");
    crate::app_usecase_import::find_video_files(path, filter_samples)
        .map(|files| files.len())
        .unwrap_or(0)
}
