//! Quality-upgrade workflow for media files.
//!
//! When a new import scores higher than an existing file for the same title,
//! the old file is recycled and the new one takes its place. If the new import
//! fails, the old file is restored from the recycle bin to avoid data loss.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::activity::{ActivityChannel, ActivityKind, ActivitySeverity};
use crate::recycle_bin::{self, RecycleBinConfig, RecycleManifest};
use crate::release_parser::ParsedReleaseMetadata;
use crate::types::TitleMediaFile;
use crate::{AppError, AppResult, AppUseCase, InsertMediaFileInput};
use scryer_domain::{CompletedDownload, NotificationEventType, Title, User};

/// Result of a successful upgrade operation.
#[derive(Debug)]
pub struct UpgradeOutcome {
    pub old_score: i32,
    pub new_score: i32,
    pub new_file_id: String,
}

pub enum UpgradeResult {
    Upgraded(UpgradeOutcome),
    Rejected(crate::post_download_gate::ImportedFileRejection),
}

/// Execute an atomic file upgrade: recycle old → import new → update DB.
///
/// If the new file import fails, the old file is restored from the recycle bin
/// so that we never lose both copies.
pub async fn execute_upgrade(
    app: &AppUseCase,
    actor: &User,
    title: &Title,
    existing_file: &TitleMediaFile,
    source_path: &std::path::Path,
    dest_path: &std::path::Path,
    parsed: &ParsedReleaseMetadata,
    quality_profile: &crate::QualityProfile,
    completed: &CompletedDownload,
    new_score: i32,
    old_score: i32,
    target_episode_ids: &[String],
    is_filler: bool,
    recycle_config: &RecycleBinConfig,
) -> AppResult<UpgradeResult> {
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
        title_id: Some(title.id.clone()),
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
            restore_old_file(&recycle_result, &old_path).await;
            return Err(AppError::Repository(format!(
                "upgrade import failed: {err}"
            )));
        }
    };

    match crate::post_download_gate::evaluate_imported_file_gate(
        app,
        title,
        parsed,
        quality_profile,
        dest_path,
        file_result.size_bytes as i64,
        true,
        Some(old_score),
        is_filler,
    )
    .await
    {
        crate::post_download_gate::ImportedFileGateDecision::Rejected(rejection) => {
            crate::post_download_gate::reject_imported_file(
                app,
                Some(&actor.id),
                title,
                &completed.name,
                dest_path,
                target_episode_ids,
                &rejection,
            )
            .await;
            restore_old_file(&recycle_result, &old_path).await;
            Ok(UpgradeResult::Rejected(rejection))
        }
        crate::post_download_gate::ImportedFileGateDecision::Accepted(accepted) => {
            // 4. Delete old media_files record
            let old_file_id = existing_file.id.clone();
            let old_episode_id = existing_file.episode_id.clone();
            if let Err(err) = app
                .services
                .media_files
                .delete_media_file(&old_file_id)
                .await
            {
                tracing::warn!(error = %err, file_id = %old_file_id, "failed to delete old media file record during upgrade");
            }

            // 5. Insert new record with rich schema
            let media_file_input = InsertMediaFileInput {
                title_id: title.id.clone(),
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
            crate::post_download_gate::persist_media_analysis_result(
                &app.services.media_files,
                &new_file_id,
                &accepted,
            )
            .await;

            // 6. Re-link episode mappings.
            if target_episode_ids.is_empty() {
                if let Some(ref episode_id) = old_episode_id {
                    let _ = app
                        .services
                        .media_files
                        .link_file_to_episode(&new_file_id, episode_id)
                        .await;
                }
            } else {
                for episode_id in target_episode_ids {
                    let _ = app
                        .services
                        .media_files
                        .link_file_to_episode(&new_file_id, episode_id)
                        .await;
                }
            }

            // 7. Record activity event
            let message = format!(
                "Upgraded file for '{}': score {} → {} (delta +{})",
                title.name,
                old_score,
                new_score,
                new_score - old_score
            );
            {
                let mut meta = HashMap::new();
                meta.insert("title_name".to_string(), serde_json::json!(title.name));
                meta.insert("old_score".to_string(), serde_json::json!(old_score));
                meta.insert("new_score".to_string(), serde_json::json!(new_score));
                if let Some(ref poster) = title.poster_url {
                    meta.insert("poster_url".to_string(), serde_json::json!(poster));
                }
                let envelope = crate::activity::NotificationEnvelope {
                    event_type: NotificationEventType::Upgrade,
                    title: format!("Upgraded: {}", title.name),
                    body: message.clone(),
                    facet: Some(format!("{:?}", title.facet).to_lowercase()),
                    metadata: meta,
                };
                app.services
                    .record_activity_event_with_notification(
                        None,
                        Some(title.id.clone()),
                        None,
                        ActivityKind::FileUpgraded,
                        message,
                        ActivitySeverity::Success,
                        vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                        envelope,
                    )
                    .await?;
            }

            Ok(UpgradeResult::Upgraded(UpgradeOutcome {
                old_score,
                new_score,
                new_file_id,
            }))
        }
    }
}

async fn restore_old_file(
    recycle_result: &Option<recycle_bin::RecycleResult>,
    old_path: &std::path::Path,
) {
    if let Some(ref recycle_result) = *recycle_result
        && let Err(restore_err) =
            recycle_bin::restore_from_recycle(&recycle_result.recycled_path, old_path).await
    {
        tracing::error!(
            error = %restore_err,
            recycled = %recycle_result.recycled_path.display(),
            "CRITICAL: failed to restore recycled file after upgrade failure"
        );
    }
}
