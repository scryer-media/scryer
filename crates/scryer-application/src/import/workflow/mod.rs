use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{
    AppError, AppResult, AppUseCase, DownloadSourceIdentity, DownloadSubmission,
    DownloadSubmissionIdentity, ImportArtifact, ParsedEpisodeMetadata, ParsedReleaseMetadata,
    SubmissionScope, WantedCompleteTransition, WantedItemsQuery,
    activity::NotificationMediaUpdate,
    app_usecase_post_processing::{PostProcessingContext, spawn_post_processing},
    apply_remote_path_mappings_to_completed_download,
    domain_events::{
        created_media_update, deleted_media_update, new_title_domain_event, title_context_snapshot,
    },
    effective_title_folder_path,
    helpers::{
        has_usable_release_title_signal, normalize_release_title_signal, parse_usable_release_title,
    },
    import_parameters::{extract_parameter, has_scryer_origin, submission_has_scryer_origin},
    import_title_resolution::normalize_imdb_id,
    nfo::{render_episode_nfo, render_movie_nfo, render_plexmatch, render_tvshow_nfo},
    parse_download_client_remote_path_mappings, parse_release_metadata,
    polling_worker::PollingWorker,
    render_rename_template, render_title_folder_template, sanitize_filesystem_component,
};
use chrono::{DateTime, Utc};
use scryer_domain::{
    Collection, CollectionType, CompletedDownload, DomainEventPayload, DownloadQueueItem,
    DownloadQueueState, Id, ImportCompletedEventData, ImportDecision, ImportErrorCode,
    ImportRecord, ImportResult, ImportSkipReason, ImportStatus, ImportType, MediaFacet, Title,
    TrackedDownloadState, User, is_video_file,
};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const IMPORT_TRANSFER_PROGRESS_MIN_INTERVAL: Duration = Duration::from_secs(1);
const IMPORT_TRANSFER_PROGRESS_MIN_BYTES: u64 = 64 * 1024 * 1024;

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

async fn import_file_with_record_progress(
    app: &AppUseCase,
    import_id: &str,
    source: &Path,
    dest: &Path,
    mode: scryer_domain::ImportMode,
    expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
) -> AppResult<scryer_domain::ImportFileResult> {
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let progress_app = app.clone();
    let progress_import_id = import_id.to_string();
    let progress_task = tokio::spawn(async move {
        let mut last_phase = None;
        let mut last_bytes = 0u64;
        let mut last_emit = None;

        while let Some(progress) = progress_rx.recv().await {
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
    });

    let result = app
        .services
        .workflow
        .file_importer
        .import_file_with_progress(source, dest, mode, expected_source, Some(progress_tx))
        .await;

    if let Err(error) = progress_task.await {
        tracing::warn!(import_id, error = %error, "import transfer progress task failed");
    }

    result
}

// This facade keeps the previous module scope while the former junk drawer is
// mechanically split into functional source files.
include!("poller.rs");
include!("completed.rs");
include!("movie.rs");
include!("series_movie.rs");
include!("series.rs");
include!("paths.rs");
include!("metadata.rs");
include!("wanted.rs");
include!("manual.rs");
include!("results.rs");
include!("tests.rs");
