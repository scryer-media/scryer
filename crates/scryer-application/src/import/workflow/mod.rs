use crate::helpers::parse_usable_release_title;
#[cfg(test)]
use crate::import_title_resolution::normalize_imdb_id;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{
    AcquisitionScopeCompleteTransition, AcquisitionScopeStatesQuery, AppError, AppResult,
    AppUseCase, ClientJobLocator, DownloadSubmission, DownloadSubmissionIdentity, ImportArtifact,
    ParsedReleaseMetadata, SubmissionScope,
    activity::NotificationMediaUpdate,
    app_usecase_post_processing::{PostProcessingContext, spawn_post_processing},
    apply_remote_path_mappings_to_completed_download,
    domain_events::{
        created_media_update, deleted_media_update, new_title_domain_event, title_context_snapshot,
    },
    effective_title_folder_path,
    helpers::{has_usable_release_title_signal, normalize_release_title_signal},
    import_parameters::{extract_parameter, submission_has_scryer_origin},
    nfo::{render_episode_nfo, render_movie_nfo, render_plexmatch, render_tvshow_nfo},
    parse_download_client_remote_path_mappings, parse_release_metadata,
    polling_worker::PollingWorker,
    render_rename_template, sanitize_filesystem_component,
};
use chrono::{DateTime, Utc};
use scryer_domain::{
    Collection, CollectionType, CompletedDownload, DomainEventPayload, DownloadQueueItem, Id,
    ImportCompletedEventData, ImportDecision, ImportErrorCode, ImportRecord, ImportResult,
    ImportSkipReason, ImportStatus, ImportType, MediaFacet, Title, TrackedDownloadState, User,
    is_video_file,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const IMPORT_TRANSFER_PROGRESS_MIN_INTERVAL: Duration = Duration::from_secs(1);
const IMPORT_TRANSFER_PROGRESS_MIN_BYTES: u64 = 64 * 1024 * 1024;
const IMPORT_TRANSFER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const IMPORT_STALE_RECOVERY_SECONDS: i64 = 45 * 60;

fn should_persist_import_transfer_progress(
    progress: &crate::ImportFileTransferProgress,
    last_phase: Option<scryer_domain::ImportTransferPhase>,
    last_bytes: u64,
    last_emit: Option<Instant>,
) -> bool {
    if last_phase != Some(progress.phase) {
        return true;
    }
    if progress.bytes == 0 || progress.bytes >= progress.total_bytes {
        return true;
    }
    if progress.bytes.saturating_sub(last_bytes) >= IMPORT_TRANSFER_PROGRESS_MIN_BYTES {
        return true;
    }
    last_emit.is_none_or(|instant| instant.elapsed() >= IMPORT_TRANSFER_PROGRESS_MIN_INTERVAL)
}

fn should_persist_import_transfer_heartbeat(last_emit: Option<Instant>) -> bool {
    last_emit.is_none_or(|instant| instant.elapsed() >= IMPORT_TRANSFER_HEARTBEAT_INTERVAL)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportDestinationOwnership {
    episode_ids: Vec<String>,
    series_movie_link_ids: Vec<String>,
}

impl ImportDestinationOwnership {
    pub(crate) fn title() -> Self {
        Self {
            episode_ids: Vec::new(),
            series_movie_link_ids: Vec::new(),
        }
    }

    pub(crate) fn episodes(episode_ids: &[String]) -> Self {
        Self {
            episode_ids: episode_ids.to_vec(),
            series_movie_link_ids: Vec::new(),
        }
    }

    pub(crate) fn series_movie(
        series_movie_link_id: &str,
        linked_episode_id: Option<&str>,
    ) -> Self {
        Self {
            episode_ids: linked_episode_id.map(str::to_string).into_iter().collect(),
            series_movie_link_ids: vec![series_movie_link_id.to_string()],
        }
    }

    pub(crate) fn upgrade(episode_ids: &[String], existing_file: &crate::TitleMediaFile) -> Self {
        let episode_ids = if episode_ids.is_empty() {
            existing_file.episode_id.iter().cloned().collect()
        } else {
            episode_ids.to_vec()
        };
        Self {
            episode_ids,
            series_movie_link_ids: existing_file.series_movie_link_ids.clone(),
        }
    }

    fn associations(&self) -> crate::MediaFileAssociations {
        crate::MediaFileAssociations {
            episode_ids: self.episode_ids.clone(),
            series_movie_link_ids: self.series_movie_link_ids.clone(),
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "import progress wiring carries source, destination, library, and source validation context"
)]
pub(crate) async fn import_file_with_record_progress(
    app: &AppUseCase,
    import_id: &str,
    library_id: &str,
    facet: &scryer_domain::MediaFacet,
    ownership: &ImportDestinationOwnership,
    source: &Path,
    dest: &Path,
    mode: scryer_domain::ImportMode,
    expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
    completed: Option<&scryer_domain::CompletedDownload>,
) -> AppResult<CoordinatedImportFileResult> {
    let destination_permit = app
        .runtime
        .imports
        .execution_coordinator
        .acquire_destination(dest)
        .await;
    let permissions = app
        .resolve_import_file_permissions(Some(library_id), facet)
        .await?;
    let active_stream = app
        .runtime
        .imports
        .active_streams
        .register(import_id, library_id, facet.clone(), source, dest)
        .await;
    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::ImportFileTransferProgress>();
    let progress_app = app.clone();
    let progress_import_id = import_id.to_string();
    let progress_stream = active_stream.clone();
    let progress_task = tokio::spawn(async move {
        let mut last_phase = None;
        let mut last_bytes = 0u64;
        let mut last_emit = None;
        let mut last_progress: Option<crate::ImportFileTransferProgress> = None;
        let mut heartbeat = tokio::time::interval(IMPORT_TRANSFER_HEARTBEAT_INTERVAL);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                maybe_progress = progress_rx.recv() => {
                    let Some(progress) = maybe_progress else {
                        break;
                    };
                    progress_stream
                        .update_transfer(progress.phase, progress.bytes, progress.total_bytes)
                        .await;
                    last_progress = Some(progress.clone());
                    if !should_persist_import_transfer_progress(
                        &progress, last_phase, last_bytes, last_emit,
                    ) {
                        continue;
                    }

                    match progress_app
                        .update_import_transfer_progress_and_notify(
                            &progress_import_id,
                            progress.phase,
                            progress.bytes,
                            progress.total_bytes,
                        )
                        .await
                    {
                        Ok(()) => {
                            last_phase = Some(progress.phase);
                            last_bytes = progress.bytes;
                            last_emit = Some(Instant::now());
                        }
                        Err(error) => {
                            tracing::warn!(
                                import_id = %progress_import_id,
                                error = %error,
                                "failed to persist import transfer progress"
                            );
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    if !should_persist_import_transfer_heartbeat(last_emit) {
                        continue;
                    }
                    let Some(progress) = last_progress.clone() else {
                        continue;
                    };

                    match progress_app
                        .update_import_transfer_progress_and_notify(
                            &progress_import_id,
                            progress.phase,
                            progress.bytes,
                            progress.total_bytes,
                        )
                        .await
                    {
                        Ok(()) => {
                            last_phase = Some(progress.phase);
                            last_bytes = progress.bytes;
                            last_emit = Some(Instant::now());
                        }
                        Err(error) => {
                            tracing::warn!(
                                import_id = %progress_import_id,
                                error = %error,
                                "failed to persist import transfer heartbeat"
                            );
                        }
                    }
                }
            }
        }
    });

    let execution_context = crate::ImportFileExecutionContext::new(
        completed.map_or("", |item| item.client_id.as_str()),
        completed.map_or("", |item| item.client_type.as_str()),
    )
    .with_active_import_stream(active_stream.clone());
    // FR-045: the depth is resolved *at import time*, not at process start, so
    // a preference change takes effect on the next import rather than the next
    // restart.
    let verification_depth = app.resolve_verification_depth().await;
    let result = app
        .services
        .workflow
        .file_importer
        .import_file_verified_with_execution_context(
            source,
            dest,
            mode,
            expected_source,
            Some(progress_tx),
            &permissions,
            &execution_context,
            verification_depth,
        )
        .await;

    if matches!(&result, Err(AppError::Canceled(_))) {
        let temporary_destination = dest.with_extension("tmp_import");
        if let Err(error) = std::fs::remove_file(&temporary_destination)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                import_id,
                path = %temporary_destination.display(),
                error = %error,
                "failed to remove cancelled import temporary destination"
            );
        }
    }

    if let Err(error) = progress_task.await {
        tracing::warn!(import_id, error = %error, "import transfer progress task failed");
    }

    active_stream.finish().await;

    let result = result?;
    let finalization_permit = app
        .runtime
        .imports
        .execution_coordinator
        .acquire_finalization()
        .await;
    Ok(CoordinatedImportFileResult {
        result,
        ownership: ownership.clone(),
        _finalization_permit: finalization_permit,
        destination_permit: Arc::new(destination_permit),
    })
}

impl AppUseCase {
    pub async fn list_active_import_streams(
        &self,
        actor: &User,
    ) -> AppResult<Vec<crate::ActiveImportStream>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?;
        Ok(self
            .runtime
            .imports
            .active_streams
            .snapshot()
            .await
            .into_iter()
            .filter(|stream| allowed_library_ids.contains(&stream.library_id))
            .collect())
    }

    pub async fn subscribe_active_import_streams(
        &self,
        actor: &User,
    ) -> AppResult<tokio::sync::watch::Receiver<crate::ActiveImportStreamSync>> {
        if self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .is_empty()
        {
            return Err(AppError::Unauthorized(
                "You do not have access to any libraries".to_string(),
            ));
        }
        Ok(self.runtime.imports.active_streams.subscribe())
    }

    pub async fn cancel_active_import_stream(
        &self,
        actor: &User,
        stream_id: &str,
    ) -> AppResult<()> {
        let stream = self
            .runtime
            .imports
            .active_streams
            .get(stream_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("active import stream {stream_id}")))?;
        self.require_library_permission(
            actor,
            &stream.library_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        self.runtime
            .imports
            .active_streams
            .request_cancel(stream_id)
            .await
            .ok_or_else(|| {
                AppError::Validation("The import is no longer cancellable".to_string())
            })?;
        Ok(())
    }
}

pub(crate) struct CoordinatedImportFileResult {
    result: scryer_domain::ImportFileResult,
    ownership: ImportDestinationOwnership,
    _finalization_permit: tokio::sync::OwnedSemaphorePermit,
    destination_permit: ImportDestinationPermit,
}

pub(crate) type ImportDestinationPermit = Arc<tokio::sync::OwnedMutexGuard<()>>;

pub(crate) struct CoordinatedMediaFilePersistence {
    pub(crate) media_file_id: String,
    pub(crate) reused_existing: bool,
    pub(crate) destination_created: bool,
}

impl CoordinatedImportFileResult {
    pub(crate) async fn insert_or_reuse_media_file(
        &self,
        app: &AppUseCase,
        input: &crate::InsertMediaFileInput,
    ) -> AppResult<CoordinatedMediaFilePersistence> {
        let associations = self.ownership.associations();
        let destination_created = matches!(
            self.result.destination_disposition,
            scryer_domain::ImportDestinationDisposition::Created
        );
        let claimed = app
            .services
            .library
            .media_files
            .claim_import_destination(input, &associations)
            .await?;
        self.persist_content_hashes(app, &claimed.media_file_id)
            .await;
        Ok(CoordinatedMediaFilePersistence {
            media_file_id: claimed.media_file_id,
            reused_existing: matches!(
                claimed.disposition,
                crate::MediaFileCatalogDisposition::Reused
            ),
            destination_created,
        })
    }

    /// Persist what the copy proved about this file (FR-041/045, migration
    /// 0205).
    ///
    /// Only a copy has hashes to persist: a hardlink or same-filesystem rename
    /// moved no bytes, so it carries no verification and leaves the columns
    /// alone for the backfill job (FR-047) to fill in later.
    ///
    /// A write failure is logged, never propagated. The bytes are already
    /// placed and proven; losing the hash costs a later backfill pass, while
    /// failing the import here would cost the user a completed download.
    async fn persist_content_hashes(&self, app: &AppUseCase, media_file_id: &str) {
        let Some(verification) = self.result.verification.as_ref() else {
            return;
        };
        if !verification.permits_source_removal() {
            return;
        }

        let hashes = crate::location::model::PersistedContentHashes::from_streamed(
            &verification.hashes,
            Utc::now(),
        );
        match app
            .services
            .library
            .media_files
            .update_media_file_content_hashes(media_file_id, &hashes)
            .await
        {
            Ok(()) => tracing::debug!(
                media_file_id,
                depth = %verification.depth.label(),
                "persisted import content hashes"
            ),
            Err(error) => tracing::warn!(
                error = %error,
                media_file_id,
                "failed to persist import content hashes; the backfill job will recompute them"
            ),
        }
    }

    pub(crate) fn destination_permit(&self) -> ImportDestinationPermit {
        Arc::clone(&self.destination_permit)
    }
}

impl std::ops::Deref for CoordinatedImportFileResult {
    type Target = scryer_domain::ImportFileResult;

    fn deref(&self) -> &Self::Target {
        &self.result
    }
}

// This facade keeps the previous module scope while the former junk drawer is
// mechanically split into functional source files.
include!("poller.rs");
include!("completed.rs");
include!("movie.rs");
include!("series_movie.rs");
include!("series_plan.rs");
include!("series.rs");
include!("paths.rs");
include!("metadata.rs");
include!("wanted.rs");
include!("manual.rs");
include!("results.rs");
include!("burned_source.rs");
include!("tests.rs");
