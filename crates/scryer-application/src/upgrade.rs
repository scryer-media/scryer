//! Quality-upgrade workflow for media files.
//!
//! When a new import scores higher than an existing file for the same title,
//! the old file is recycled and the new one takes its place. If the new import
//! fails, the old file is restored from the recycle bin to avoid data loss.

use std::path::PathBuf;
use std::sync::Arc;

use crate::activity::{ActivityChannel, ActivityKind, ActivitySeverity};
use crate::recycle_bin::{self, RecycleBinConfig, RecycleManifest};
use crate::release_parser::ParsedReleaseMetadata;
use crate::types::TitleMediaFile;
use crate::{AppError, AppResult, AppUseCase, InsertMediaFileInput};

/// Result of a successful upgrade operation.
#[derive(Debug)]
pub struct UpgradeOutcome {
    pub old_file_id: String,
    pub new_file_id: String,
    pub old_score: i32,
    pub new_score: i32,
}

/// Execute an atomic file upgrade: recycle old → import new → update DB.
///
/// If the new file import fails, the old file is restored from the recycle bin
/// so that we never lose both copies.
pub async fn execute_upgrade(
    app: &AppUseCase,
    title_name: &str,
    title_id: &str,
    existing_file: &TitleMediaFile,
    source_path: &std::path::Path,
    dest_path: &std::path::Path,
    parsed: &ParsedReleaseMetadata,
    new_score: i32,
    old_score: i32,
    recycle_config: &RecycleBinConfig,
) -> AppResult<UpgradeOutcome> {
    let old_path = PathBuf::from(&existing_file.file_path);
    let scoring_log = format!(
        "upgrade {} → {} (delta {})",
        old_score,
        new_score,
        new_score - old_score
    );

    // 1. Recycle the old file
    let manifest = RecycleManifest {
        recycled_at: chrono::Utc::now().to_rfc3339(),
        original_path: existing_file.file_path.clone(),
        size_bytes: existing_file.size_bytes as u64,
        title_id: Some(title_id.to_string()),
        reason: "upgrade_replaced".to_string(),
    };
    let recycle_result = recycle_bin::recycle_file(recycle_config, &old_path, manifest).await?;

    // 2. Import the new file
    let import_result = app
        .services
        .file_importer
        .import_file(source_path, dest_path)
        .await;

    let file_result = match import_result {
        Ok(r) => r,
        Err(err) => {
            // 3. Restore old file on failure
            tracing::error!(
                error = %err,
                old_path = %old_path.display(),
                new_source = %source_path.display(),
                "upgrade import failed, restoring old file"
            );
            if let Some(ref rr) = recycle_result {
                if let Err(restore_err) =
                    recycle_bin::restore_from_recycle(&rr.recycled_path, &old_path).await
                {
                    tracing::error!(
                        error = %restore_err,
                        recycled = %rr.recycled_path.display(),
                        "CRITICAL: failed to restore recycled file after upgrade failure"
                    );
                }
            }
            return Err(AppError::Repository(format!(
                "upgrade import failed: {err}"
            )));
        }
    };

    // 4. Delete old media_files record
    let old_file_id = existing_file.id.clone();
    let old_episode_id = existing_file.episode_id.clone();
    if let Err(err) = app.services.media_files.delete_media_file(&old_file_id).await {
        tracing::warn!(error = %err, file_id = %old_file_id, "failed to delete old media file record during upgrade");
    }

    // 5. Insert new record with rich schema
    let media_file_input = InsertMediaFileInput {
        title_id: title_id.to_string(),
        file_path: dest_path.to_string_lossy().to_string(),
        size_bytes: file_result.size_bytes as i64,
        quality_label: parsed.quality.clone(),
        scene_name: Some(parsed.raw_title.clone()),
        release_group: parsed.release_group.clone(),
        source_type: parsed.source.clone(),
        resolution: parsed.quality.clone(),
        video_codec_parsed: parsed.video_codec.clone(),
        audio_codec_parsed: parsed.audio.clone(),
        original_file_path: Some(source_path.to_string_lossy().to_string()),
        acquisition_score: Some(new_score),
        scoring_log: Some(scoring_log.clone()),
        ..Default::default()
    };
    let new_file_id = app
        .services
        .media_files
        .insert_media_file(&media_file_input)
        .await?;

    // 6. Re-link episode mappings if the old file was linked to an episode
    if let Some(ref episode_id) = old_episode_id {
        let _ = app
            .services
            .media_files
            .link_file_to_episode(&new_file_id, episode_id)
            .await;
    }

    // 7. Spawn media analysis
    {
        let media_files = Arc::clone(&app.services.media_files);
        let wanted_items = Arc::clone(&app.services.wanted_items);
        let release_attempts = Arc::clone(&app.services.release_attempts);
        let file_id = new_file_id.clone();
        let path = dest_path.to_path_buf();
        let tid = title_id.to_string();
        tokio::spawn(async move {
            crate::app_usecase_import::run_media_analysis(
                media_files,
                wanted_items,
                release_attempts,
                file_id,
                path,
                tid,
                vec![],
            )
            .await;
        });
    }

    // 8. Record activity event
    let message = format!(
        "Upgraded file for '{}': score {} → {} (delta +{})",
        title_name,
        old_score,
        new_score,
        new_score - old_score
    );
    app.services
        .record_activity_event(
            None,
            Some(title_id.to_string()),
            ActivityKind::FileUpgraded,
            message,
            ActivitySeverity::Success,
            vec![ActivityChannel::WebUi, ActivityChannel::Toast],
        )
        .await?;

    Ok(UpgradeOutcome {
        old_file_id,
        new_file_id,
        old_score,
        new_score,
    })
}
