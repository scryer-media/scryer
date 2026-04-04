//! Quality-upgrade workflow for media files.
//!
//! When a new import scores higher than an existing file for the same title,
//! the old file is recycled and the new one takes its place. If the new import
//! fails, the old file is restored from the recycle bin to avoid data loss.

use std::path::PathBuf;

use crate::domain_events::{
    created_media_update, deleted_media_update, modified_media_update, new_title_domain_event,
    title_context_snapshot,
};
use crate::recycle_bin::{self, RecycleBinConfig, RecycleManifest};
use crate::release_parser::ParsedReleaseMetadata;
use crate::types::TitleMediaFile;
use crate::{AppError, AppResult, AppUseCase, InsertMediaFileInput, ReleaseDownloadAttemptOutcome};
use scryer_domain::{
    CompletedDownload, DomainEventPayload, MediaFileDeletedEventData, MediaFileDeletedReason,
    MediaFileUpgradedEventData, Title, User,
};

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
    _actor: &User,
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

    // 1. Probe source file in-place BEFORE any file moves.
    let source_size = std::fs::metadata(source_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let gate_result = crate::post_download_gate::probe_and_validate(
        app,
        title,
        parsed,
        quality_profile,
        source_path,
        source_size,
        true,
        Some(old_score),
        is_filler,
    )
    .await;

    // 2. If probe rejects, bail immediately with zero file moves.
    let accepted = match gate_result {
        crate::post_download_gate::ImportedFileGateDecision::Rejected(rejection) => {
            tracing::info!(
                title = %title.name,
                reason = %rejection.message,
                "upgrade probe rejected source file — no files moved"
            );
            // Record failed attempt + blocklist so this release isn't re-downloaded.
            let _ = app
                .services
                .release_attempts
                .record_release_attempt(
                    Some(title.id.clone()),
                    crate::normalize_release_attempt_hint(None),
                    crate::normalize_release_attempt_title(Some(&completed.name)),
                    ReleaseDownloadAttemptOutcome::Failed,
                    Some(rejection.message.clone()),
                    None,
                )
                .await;
            return Ok(UpgradeResult::Rejected(rejection));
        }
        crate::post_download_gate::ImportedFileGateDecision::Accepted(accepted) => accepted,
    };

    // 3. Rescore from mediainfo: merge detected values into parsed metadata and re-evaluate.
    let (rescored_parsed, rescore_changes) =
        crate::post_download_gate::rescore_from_mediainfo(parsed, &accepted);
    let original_candidate_score = new_score;
    let final_score = if rescore_changes.is_empty() {
        new_score
    } else {
        let category = crate::post_download_gate::facet_to_category_hint(&title.facet);
        let required_audio_languages = app
            .resolve_required_audio_languages(
                Some(&title.id),
                Some(category),
                Some(quality_profile),
            )
            .await
            .unwrap_or_default();
        let persona = app
            .resolve_scoring_persona(Some(category), Some(quality_profile), Some(category))
            .await
            .unwrap_or_default();
        let decision = crate::post_download_gate::build_import_profile_decision(
            quality_profile,
            &required_audio_languages,
            &persona,
            &rescored_parsed,
            category,
            title.runtime_minutes,
            Some(source_size),
            true,
        );
        let rescored = decision.preference_score;
        tracing::info!(
            title = %title.name,
            original_score = original_candidate_score,
            rescored = rescored,
            changes = ?rescore_changes,
            "mediainfo rescore applied"
        );
        rescored
    };

    // If rescored score no longer beats the existing file, abort upgrade.
    if final_score <= old_score {
        tracing::info!(
            title = %title.name,
            original_score = original_candidate_score,
            rescored = final_score,
            old_score,
            "mediainfo rescore eliminated upgrade advantage — aborting"
        );
        return Ok(UpgradeResult::Rejected(
            crate::post_download_gate::ImportedFileRejection {
                message: format!(
                    "mediainfo rescore reduced score from {} to {} (existing: {})",
                    original_candidate_score, final_score, old_score
                ),
                recycle_reason: "rescore_eliminated_advantage",
                skip_reason: None,
                blocking_rule_codes: Vec::new(),
            },
        ));
    }

    let scoring_log = format!(
        "upgrade {} → {} (delta {}){}",
        old_score,
        final_score,
        final_score - old_score,
        if rescore_changes.is_empty() {
            String::new()
        } else {
            format!("; rescore: {}", rescore_changes.join(", "))
        }
    );

    // 3. Recycle the old file
    let manifest = RecycleManifest {
        recycled_at: chrono::Utc::now().to_rfc3339(),
        original_path: existing_file.file_path.clone(),
        size_bytes: existing_file.size_bytes as u64,
        title_id: Some(title.id.clone()),
        reason: "upgrade_replaced".to_string(),
    };
    let recycle_result = recycle_bin::recycle_file(recycle_config, &old_path, manifest).await?;

    // 4. Import the new file
    let import_result = app
        .services
        .file_importer
        .import_file(source_path, dest_path)
        .await;

    let file_result = match import_result {
        Ok(r) => r,
        Err(err) => {
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

    // 5. Delete old media_files record
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

    // 6. Insert new record with rich schema
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
        acquisition_score: Some(final_score),
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

    // 7. Re-link episode mappings.
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

    {
        let media_updates = if existing_file.file_path == dest_path.to_string_lossy() {
            vec![modified_media_update(
                dest_path.to_string_lossy().to_string(),
            )]
        } else {
            vec![
                deleted_media_update(existing_file.file_path.clone()),
                created_media_update(dest_path.to_string_lossy().to_string()),
            ]
        };
        app.services
            .append_domain_event(new_title_domain_event(
                None,
                title,
                DomainEventPayload::MediaFileUpgraded(MediaFileUpgradedEventData {
                    title: title_context_snapshot(title),
                    media_updates,
                    previous_file_id: Some(existing_file.id.clone()),
                    current_file_id: Some(new_file_id.clone()),
                    old_score: Some(old_score),
                    new_score: Some(final_score),
                }),
            ))
            .await?;

        if existing_file.file_path != dest_path.to_string_lossy() {
            app.services
                .append_domain_event(new_title_domain_event(
                    None,
                    title,
                    DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                        title: title_context_snapshot(title),
                        media_updates: vec![deleted_media_update(existing_file.file_path.clone())],
                        file_id: Some(existing_file.id.clone()),
                        reason: MediaFileDeletedReason::UpgradeCleanup,
                        episode_ids: target_episode_ids.to_vec(),
                    }),
                ))
                .await?;
        }
    }

    Ok(UpgradeResult::Upgraded(UpgradeOutcome {
        old_score,
        new_score: final_score,
        new_file_id,
    }))
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
