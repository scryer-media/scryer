use std::collections::HashSet;
use std::path::Path;

use chrono::Utc;

use crate::activity::{ActivityChannel, ActivityKind, ActivitySeverity};
use crate::{
    normalize_release_attempt_hint, normalize_release_attempt_title, AppUseCase,
    ReleaseDownloadAttemptOutcome,
};
use scryer_domain::{ImportSkipReason, MediaFacet, Title};
use tracing::warn;

pub(crate) enum ImportedFileGateDecision {
    Accepted(ImportedFileAcceptance),
    Rejected(ImportedFileRejection),
}

pub(crate) struct ImportedFileAcceptance {
    pub analysis: Option<crate::MediaFileAnalysis>,
    pub scan_error: Option<String>,
}

pub(crate) struct ImportedFileRejection {
    pub message: String,
    pub recycle_reason: &'static str,
    pub skip_reason: Option<ImportSkipReason>,
    pub blocking_rule_codes: Vec<String>,
}

pub(crate) fn facet_to_category_hint(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => "movie",
        MediaFacet::Tv => "tv",
        MediaFacet::Anime => "anime",
        MediaFacet::Other => "other",
    }
}

pub(crate) fn build_import_profile_decision(
    profile: &crate::QualityProfile,
    parsed: &crate::ParsedReleaseMetadata,
    category_hint: &str,
    runtime_minutes: Option<i32>,
    size_bytes: Option<i64>,
    has_existing_file: bool,
) -> crate::QualityProfileDecision {
    let persona = profile.criteria.resolve_persona(Some(category_hint));
    let weights =
        crate::scoring_weights::build_weights(persona, &profile.criteria.scoring_overrides);
    let mut decision =
        crate::evaluate_against_profile(profile, parsed, has_existing_file, &weights);
    crate::quality_profile::apply_size_scoring_for_category(
        &mut decision,
        parsed,
        size_bytes,
        Some(category_hint),
        runtime_minutes,
        &weights,
    );
    decision
}

pub(crate) fn build_media_file_analysis(
    analysis: &scryer_mediainfo::MediaAnalysis,
) -> crate::MediaFileAnalysis {
    crate::MediaFileAnalysis {
        video_codec: analysis.video_codec.clone(),
        video_width: analysis.video_width,
        video_height: analysis.video_height,
        video_bitrate_kbps: analysis.video_bitrate_kbps,
        video_bit_depth: analysis.video_bit_depth,
        video_hdr_format: analysis.video_hdr_format.clone(),
        video_frame_rate: analysis.video_frame_rate.clone(),
        video_profile: analysis.video_profile.clone(),
        audio_codec: analysis.audio_codec.clone(),
        audio_channels: analysis.audio_channels,
        audio_bitrate_kbps: analysis.audio_bitrate_kbps,
        audio_languages: analysis.audio_languages.clone(),
        audio_streams: analysis
            .audio_streams
            .iter()
            .map(|stream| crate::AudioStreamDetail {
                codec: stream.codec.clone(),
                channels: stream.channels,
                language: stream.language.clone(),
                bitrate_kbps: stream.bitrate_kbps,
            })
            .collect(),
        subtitle_languages: analysis.subtitle_languages.clone(),
        subtitle_codecs: analysis.subtitle_codecs.clone(),
        subtitle_streams: analysis
            .subtitle_streams
            .iter()
            .map(|stream| crate::SubtitleStreamDetail {
                codec: stream.codec.clone(),
                language: stream.language.clone(),
                name: stream.name.clone(),
                forced: stream.forced,
                default: stream.default,
            })
            .collect(),
        has_multiaudio: analysis.has_multiaudio,
        duration_seconds: analysis.duration_seconds,
        num_chapters: analysis.num_chapters,
        container_format: analysis.container_format.clone(),
        raw_json: analysis.raw_json.clone(),
    }
}

pub(crate) fn missing_audio_languages<'a>(
    required: &'a [String],
    actual: &[String],
) -> Vec<&'a str> {
    let actual_upper: std::collections::HashSet<String> = actual
        .iter()
        .map(|language| language.to_ascii_uppercase())
        .collect();
    required
        .iter()
        .filter(|required_language| !actual_upper.contains(required_language.as_str()))
        .map(String::as_str)
        .collect()
}

pub(crate) async fn evaluate_imported_file_gate(
    app: &AppUseCase,
    title: &Title,
    parsed: &crate::ParsedReleaseMetadata,
    quality_profile: &crate::QualityProfile,
    path: &Path,
    size_bytes: i64,
    has_existing_file: bool,
    existing_score: Option<i32>,
    is_filler: bool,
) -> ImportedFileGateDecision {
    let analysis = match scryer_mediainfo::analyze_file(path) {
        Ok(analysis) => analysis,
        Err(error) => {
            warn!(error = %error, path = %path.display(), "media analysis failed");
            return ImportedFileGateDecision::Accepted(ImportedFileAcceptance {
                analysis: None,
                scan_error: Some(error.to_string()),
            });
        }
    };

    if !scryer_mediainfo::is_valid_video(&analysis) {
        return ImportedFileGateDecision::Rejected(ImportedFileRejection {
            message: "imported file is not a valid video".to_string(),
            recycle_reason: "invalid_file",
            skip_reason: None,
            blocking_rule_codes: Vec::new(),
        });
    }

    if !quality_profile.criteria.required_audio_languages.is_empty() {
        let missing = missing_audio_languages(
            &quality_profile.criteria.required_audio_languages,
            &analysis.audio_languages,
        );
        if !missing.is_empty() {
            return ImportedFileGateDecision::Rejected(ImportedFileRejection {
                message: format!(
                    "imported file is missing required audio language(s): {}",
                    missing.join(", ")
                ),
                recycle_reason: "language_mismatch",
                skip_reason: None,
                blocking_rule_codes: Vec::new(),
            });
        }
    }

    let user_rules_engine = app
        .services
        .user_rules
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| scryer_rules::UserRulesEngine::empty());
    if !user_rules_engine.is_empty() {
        let decision = build_import_profile_decision(
            quality_profile,
            parsed,
            facet_to_category_hint(&title.facet),
            title.runtime_minutes,
            Some(size_bytes),
            has_existing_file,
        );
        let input = crate::user_rule_input::build_rule_input(
            parsed,
            quality_profile,
            &decision,
            crate::user_rule_input::ReleaseRuntimeInfo {
                size_bytes: Some(size_bytes),
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                extra: None,
            },
            crate::user_rule_input::RuleContextInfo {
                title_id: Some(&title.id),
                category: Some(facet_to_category_hint(&title.facet)),
                title_tags: &title.tags,
                has_existing_file,
                existing_score,
                search_mode: "post_download",
                runtime_minutes: title.runtime_minutes,
                is_filler,
            },
            Some(crate::user_rule_input::build_file_doc(&analysis)),
        );
        let mut evaluator = user_rules_engine.evaluator();
        match evaluator.evaluate(&input, facet_to_category_hint(&title.facet)) {
            Ok(result) => {
                if !result.errors.is_empty() {
                    warn!(
                        title_id = %title.id,
                        error_count = result.errors.len(),
                        "post-download rule evaluation had runtime errors; failing open"
                    );
                }

                let blocking_rule_codes: Vec<String> = result
                    .entries
                    .iter()
                    .filter(|entry| entry.delta <= scryer_rules::BLOCK_SCORE_THRESHOLD)
                    .map(|entry| entry.code.clone())
                    .collect();

                if !blocking_rule_codes.is_empty() {
                    return ImportedFileGateDecision::Rejected(ImportedFileRejection {
                        message: format!(
                            "post-download rule(s) blocked import: {}",
                            blocking_rule_codes.join(", ")
                        ),
                        recycle_reason: "post_download_rule_blocked",
                        skip_reason: Some(ImportSkipReason::PostDownloadRuleBlocked),
                        blocking_rule_codes,
                    });
                }
            }
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    "post-download rule evaluation failed; failing open"
                );
            }
        }
    }

    ImportedFileGateDecision::Accepted(ImportedFileAcceptance {
        analysis: Some(build_media_file_analysis(&analysis)),
        scan_error: None,
    })
}

pub(crate) async fn persist_media_analysis_result(
    media_files: &std::sync::Arc<dyn crate::MediaFileRepository>,
    file_id: &str,
    accepted: &ImportedFileAcceptance,
) {
    if let Some(ref analysis) = accepted.analysis {
        if let Err(error) = media_files
            .update_media_file_analysis(file_id, analysis.clone())
            .await
        {
            warn!(error = %error, file_id = %file_id, "failed to store media analysis");
            let _ = media_files
                .mark_scan_failed(file_id, &error.to_string())
                .await;
        }
        return;
    }

    if let Some(ref error) = accepted.scan_error {
        let _ = media_files.mark_scan_failed(file_id, error).await;
    }
}

pub(crate) async fn reject_imported_file(
    app: &AppUseCase,
    actor_user_id: Option<&str>,
    title: &Title,
    completed_name: &str,
    path: &Path,
    episode_ids: &[String],
    rejection: &ImportedFileRejection,
) {
    recycle_imported_file(path, &title.id, rejection.recycle_reason).await;

    let _ = app
        .services
        .release_attempts
        .record_release_attempt(
            Some(title.id.clone()),
            normalize_release_attempt_hint(None),
            normalize_release_attempt_title(Some(completed_name)),
            ReleaseDownloadAttemptOutcome::Failed,
            Some(rejection.message.clone()),
            None,
        )
        .await;

    reset_wanted_items_for_retry(app, &title.id, episode_ids).await;

    let _ = app
        .services
        .record_activity_event(
            actor_user_id.map(str::to_owned),
            Some(title.id.clone()),
            ActivityKind::ImportRejected,
            format!(
                "Rejected import for '{}': {}{}",
                title.name,
                rejection.message,
                if rejection.blocking_rule_codes.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", rejection.blocking_rule_codes.join(", "))
                }
            ),
            ActivitySeverity::Warning,
            vec![ActivityChannel::WebUi],
        )
        .await;
}

async fn recycle_imported_file(path: &Path, title_id: &str, reason: &str) {
    let recycle_config = crate::recycle_bin::config_from_file_path(path);
    let manifest = crate::recycle_bin::RecycleManifest {
        recycled_at: Utc::now().to_rfc3339(),
        original_path: path.display().to_string(),
        size_bytes: tokio::fs::metadata(path)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or(0),
        title_id: Some(title_id.to_string()),
        reason: reason.to_string(),
    };
    if let Err(error) = crate::recycle_bin::recycle_file(&recycle_config, path, manifest).await {
        warn!(
            error = %error,
            path = %path.display(),
            "failed to recycle rejected file from disk"
        );
    }
}

async fn reset_wanted_items_for_retry(app: &AppUseCase, title_id: &str, episode_ids: &[String]) {
    let now_str = Utc::now().to_rfc3339();
    let targets: Vec<Option<&str>> = if episode_ids.is_empty() {
        vec![None]
    } else {
        let mut seen = HashSet::new();
        episode_ids
            .iter()
            .filter(|episode_id| seen.insert((*episode_id).clone()))
            .map(|episode_id| Some(episode_id.as_str()))
            .collect()
    };

    for episode_id in targets {
        match app
            .services
            .wanted_items
            .get_wanted_item_for_title(title_id, episode_id)
            .await
        {
            Ok(Some(item)) => {
                let _ = app
                    .services
                    .wanted_items
                    .update_wanted_item_status(
                        &item.id,
                        "wanted",
                        Some(&now_str),
                        None,
                        item.search_count,
                        item.current_score,
                        None,
                    )
                    .await;
            }
            Ok(None) => {}
            Err(error) => {
                warn!(error = %error, title_id = %title_id, "failed to reset wanted item")
            }
        }
    }
}
