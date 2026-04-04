use std::collections::HashSet;
use std::path::Path;

use chrono::Utc;

use crate::domain_events::{new_title_domain_event, title_context_snapshot};
use crate::{
    AppUseCase, ReleaseDownloadAttemptOutcome, WantedSearchTransition,
    normalize_release_attempt_hint, normalize_release_attempt_title,
};
use scryer_domain::{
    DomainEventPayload, ImportRejectedEventData, ImportSkipReason, ImportStatus, MediaFacet, Title,
};
use tracing::warn;

pub(crate) enum ImportedFileGateDecision {
    Accepted(Box<ImportedFileAcceptance>),
    Rejected(ImportedFileRejection),
}

pub(crate) struct ImportedFileAcceptance {
    pub analysis: Option<crate::MediaFileAnalysis>,
    pub scan_error: Option<String>,
}

pub struct ImportedFileRejection {
    pub message: String,
    pub recycle_reason: &'static str,
    pub skip_reason: Option<ImportSkipReason>,
    pub blocking_rule_codes: Vec<String>,
}

pub(crate) fn facet_to_category_hint(facet: &MediaFacet) -> &'static str {
    facet.as_str()
}

pub(crate) fn build_import_profile_decision(
    profile: &crate::QualityProfile,
    required_audio_languages: &[String],
    persona: &crate::ScoringPersona,
    parsed: &crate::ParsedReleaseMetadata,
    category_hint: &str,
    runtime_minutes: Option<i32>,
    size_bytes: Option<i64>,
    has_existing_file: bool,
) -> crate::QualityProfileDecision {
    let mut resolved_profile = profile.clone();
    resolved_profile.criteria.required_audio_languages = required_audio_languages.to_vec();
    resolved_profile.criteria.scoring_persona = persona.clone();
    resolved_profile.criteria.facet_persona_overrides.clear();
    let weights = crate::scoring_weights::build_weights_for_category(
        persona,
        &resolved_profile.criteria.scoring_overrides,
        Some(category_hint),
    );
    let mut decision = crate::quality_profile::evaluate_against_profile_for_category(
        &resolved_profile,
        parsed,
        has_existing_file,
        &weights,
        Some(category_hint),
    );
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
    let audio_languages = crate::normalize_detected_audio_languages(
        analysis.audio_languages.iter().map(String::as_str),
    );
    let subtitle_languages = crate::normalize_detected_subtitle_languages(
        analysis.subtitle_languages.iter().map(String::as_str),
    );

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
        audio_languages,
        audio_streams: analysis
            .audio_streams
            .iter()
            .map(|stream| crate::AudioStreamDetail {
                codec: stream.codec.clone(),
                channels: stream.channels,
                language: stream
                    .language
                    .as_deref()
                    .and_then(crate::normalize_detected_audio_language_code),
                bitrate_kbps: stream.bitrate_kbps,
            })
            .collect(),
        subtitle_languages,
        subtitle_codecs: analysis.subtitle_codecs.clone(),
        subtitle_streams: analysis
            .subtitle_streams
            .iter()
            .map(|stream| crate::SubtitleStreamDetail {
                codec: stream.codec.clone(),
                language: stream
                    .language
                    .as_deref()
                    .and_then(crate::normalize_detected_subtitle_language_code),
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

/// Probe a file at the given path and validate it against the quality profile and user rules.
/// The file does NOT need to be at its final destination — this can probe a file in-place
/// at its download location before any move/copy.
pub(crate) async fn probe_and_validate(
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
            return ImportedFileGateDecision::Accepted(Box::new(ImportedFileAcceptance {
                analysis: None,
                scan_error: Some(error.to_string()),
            }));
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

    let category_hint = facet_to_category_hint(&title.facet);
    let required_audio_languages = app
        .resolve_required_audio_languages(
            Some(&title.id),
            Some(category_hint),
            Some(quality_profile),
        )
        .await
        .unwrap_or_else(|error| {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to resolve required audio languages, using canonical default"
            );
            Vec::new()
        });
    if !required_audio_languages.is_empty() {
        let missing = crate::missing_required_audio_languages(
            &required_audio_languages,
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
    let persona = app
        .resolve_scoring_persona(
            Some(category_hint),
            Some(quality_profile),
            Some(category_hint),
        )
        .await
        .unwrap_or_else(|error| {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to resolve scoring persona, using canonical default"
            );
            crate::ScoringPersona::default()
        });

    let user_rules_engine = app
        .services
        .user_rules
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| scryer_rules::UserRulesEngine::empty());
    if !user_rules_engine.is_empty() {
        let decision = build_import_profile_decision(
            quality_profile,
            &required_audio_languages,
            &persona,
            parsed,
            category_hint,
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
                indexer_languages: None,
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

    ImportedFileGateDecision::Accepted(Box::new(ImportedFileAcceptance {
        analysis: Some(build_media_file_analysis(&analysis)),
        scan_error: None,
    }))
}

/// Merge mediainfo-detected values into a release-name-parsed metadata struct.
/// Prefers mediainfo when it detects a concrete value that differs from the release name.
/// Returns the merged metadata and a log of what changed.
pub(crate) fn rescore_from_mediainfo(
    parsed: &crate::ParsedReleaseMetadata,
    acceptance: &ImportedFileAcceptance,
) -> (crate::ParsedReleaseMetadata, Vec<String>) {
    let Some(ref analysis) = acceptance.analysis else {
        return (parsed.clone(), vec![]);
    };

    let mut merged = parsed.clone();
    let mut changes = Vec::new();

    // Override resolution from video height
    if let Some(height) = analysis.video_height {
        let detected = match height {
            h if h >= 2100 => Some("2160p"),
            h if h >= 1000 => Some("1080p"),
            h if h >= 700 => Some("720p"),
            h if h >= 480 => Some("480p"),
            _ => None,
        };
        if let Some(detected) = detected
            && merged.quality.as_deref() != Some(detected)
        {
            changes.push(format!(
                "resolution: {} → {}",
                merged.quality.as_deref().unwrap_or("?"),
                detected
            ));
            merged.quality = Some(detected.to_string());
        }
    }

    // Override video codec (map mediainfo names → release parser names)
    if let Some(ref mediainfo_codec) = analysis.video_codec {
        let normalized = normalize_mediainfo_video_codec(mediainfo_codec);
        if let Some(normalized) = normalized
            && merged.video_codec.as_deref() != Some(normalized)
        {
            changes.push(format!(
                "video_codec: {} → {}",
                merged.video_codec.as_deref().unwrap_or("?"),
                normalized
            ));
            merged.video_codec = Some(normalized.to_string());
        }
    }

    if analysis.video_bit_depth.unwrap_or_default() >= 10 && !merged.is_10bit {
        changes.push("video_bit_depth: detected 10-bit".to_string());
        merged.is_10bit = true;
    }

    // Override HDR format
    if let Some(ref hdr_format) = analysis.video_hdr_format {
        let hdr_upper = hdr_format.to_ascii_uppercase();
        if hdr_upper.contains("DOLBY VISION") && !merged.is_dolby_vision {
            changes.push("hdr: detected Dolby Vision".to_string());
            merged.is_dolby_vision = true;
        }
        if hdr_upper.contains("HDR10") && !merged.has_hdr_fallback {
            changes.push("hdr: detected HDR fallback".to_string());
            merged.has_hdr_fallback = true;
        }
        if (hdr_upper.contains("HDR10+") || hdr_upper.contains("HDR10PLUS")) && !merged.is_hdr10plus
        {
            changes.push("hdr: detected HDR10+".to_string());
            merged.is_hdr10plus = true;
        }
        if hdr_upper.contains("HDR10") && !merged.detected_hdr {
            changes.push("hdr: detected HDR10".to_string());
            merged.detected_hdr = true;
        }
    }

    // Override audio: iterate all streams to find best codec and max channels.
    if !analysis.audio_streams.is_empty() {
        let best_stream = analysis
            .audio_streams
            .iter()
            .max_by_key(|s| audio_codec_rank(s.codec.as_deref().unwrap_or("")));

        if let Some(best) = best_stream
            && let Some(ref codec) = best.codec
        {
            let normalized = normalize_mediainfo_audio_codec(codec);
            if let Some(normalized) = normalized
                && merged.audio.as_deref() != Some(normalized)
            {
                changes.push(format!(
                    "audio: {} → {}",
                    merged.audio.as_deref().unwrap_or("?"),
                    normalized
                ));
                merged.audio = Some(normalized.to_string());
            }
        }

        let max_channels = analysis
            .audio_streams
            .iter()
            .filter_map(|s| s.channels)
            .max();
        if let Some(channels) = max_channels {
            let ch_str = format_audio_channels(channels);
            if merged.audio_channels.as_deref() != Some(&ch_str) {
                changes.push(format!(
                    "audio_channels: {} → {}",
                    merged.audio_channels.as_deref().unwrap_or("?"),
                    ch_str
                ));
                merged.audio_channels = Some(ch_str);
            }
        }

        // Detect multi-audio from stream count
        if analysis.audio_streams.len() > 1 && !merged.is_dual_audio {
            changes.push("dual_audio: detected multiple audio tracks".to_string());
            merged.is_dual_audio = true;
        }

        // Detect Atmos from stream codec names
        let has_atmos = analysis.audio_streams.iter().any(|s| {
            s.codec
                .as_deref()
                .is_some_and(|c| c.to_ascii_lowercase().contains("atmos"))
        });
        if has_atmos && !merged.is_atmos {
            changes.push("atmos: detected from audio streams".to_string());
            merged.is_atmos = true;
        }
    }

    (merged, changes)
}

/// Map mediainfo video codec names to release-parser canonical names.
fn normalize_mediainfo_video_codec(codec: &str) -> Option<&'static str> {
    match codec.to_ascii_lowercase().as_str() {
        "hevc" | "h265" | "h.265" | "hvc1" | "hev1" => Some("H.265"),
        "h264" | "h.264" | "avc" | "avc1" => Some("H.264"),
        "av1" | "av01" => Some("AV1"),
        "vp9" => Some("VP9"),
        "mpeg4" | "mp4v" | "xvid" | "divx" => Some("MPEG-4"),
        _ => None,
    }
}

/// Map mediainfo audio codec names to release-parser canonical names.
fn normalize_mediainfo_audio_codec(codec: &str) -> Option<&'static str> {
    let lower = codec.to_ascii_lowercase();
    if lower.contains("truehd") && lower.contains("atmos") {
        return Some("TrueHD Atmos");
    }
    if lower.contains("truehd") {
        return Some("TrueHD");
    }
    if lower.contains("dts") && lower.contains("atmos") {
        return Some("DTS:X");
    }
    if lower.contains("dts-hd ma") || lower.contains("dts-hd master") {
        return Some("DTS-HD MA");
    }
    if lower.contains("dts-hd") {
        return Some("DTS-HD");
    }
    if lower.contains("dts") {
        return Some("DTS");
    }
    if lower.contains("e-ac-3") || lower.contains("eac3") || lower.contains("dd+") {
        if lower.contains("atmos") {
            return Some("EAC3 Atmos");
        }
        return Some("EAC3");
    }
    if lower.contains("ac-3") || lower.contains("ac3") {
        return Some("AC3");
    }
    if lower.contains("flac") {
        return Some("FLAC");
    }
    if lower.contains("aac") {
        return Some("AAC");
    }
    if lower.contains("mp3") || lower.contains("mpeg audio") {
        return Some("MP3");
    }
    if lower.contains("opus") {
        return Some("Opus");
    }
    if lower.contains("vorbis") {
        return Some("Vorbis");
    }
    if lower.contains("pcm") || lower.contains("lpcm") {
        return Some("PCM");
    }
    None
}

/// Rank audio codecs for "best track" selection when iterating streams.
fn audio_codec_rank(codec: &str) -> i32 {
    let lower = codec.to_ascii_lowercase();
    if lower.contains("truehd") && lower.contains("atmos") {
        return 100;
    }
    if lower.contains("truehd") {
        return 90;
    }
    if lower.contains("dts") && lower.contains("atmos") {
        return 95;
    }
    if lower.contains("dts-hd ma") || lower.contains("dts-hd master") {
        return 85;
    }
    if lower.contains("flac") {
        return 80;
    }
    if lower.contains("e-ac-3") || lower.contains("eac3") || lower.contains("dd+") {
        return 70;
    }
    if lower.contains("dts-hd") {
        return 65;
    }
    if lower.contains("dts") {
        return 60;
    }
    if lower.contains("ac-3") || lower.contains("ac3") {
        return 50;
    }
    if lower.contains("aac") {
        return 40;
    }
    if lower.contains("opus") {
        return 40;
    }
    if lower.contains("mp3") {
        return 30;
    }
    20
}

/// Format channel count to standard notation (e.g., 6 → "5.1", 8 → "7.1").
fn format_audio_channels(channels: i32) -> String {
    match channels {
        8 => "7.1".to_string(),
        7 => "6.1".to_string(),
        6 => "5.1".to_string(),
        2 => "2.0".to_string(),
        1 => "1.0".to_string(),
        n => format!("{n}.0"),
    }
}

/// Compute acquisition score from a gate acceptance, applying mediainfo rescoring.
/// Returns the final score and the rescored parsed metadata (for logging).
pub(crate) async fn compute_acquisition_score(
    app: &AppUseCase,
    parsed: &crate::ParsedReleaseMetadata,
    acceptance: &ImportedFileAcceptance,
    profile: &crate::QualityProfile,
    title: &Title,
    size_bytes: i64,
    has_existing_file: bool,
) -> i32 {
    let (rescored, changes) = rescore_from_mediainfo(parsed, acceptance);
    let category = facet_to_category_hint(&title.facet);
    let required_audio_languages = app
        .resolve_required_audio_languages(Some(&title.id), Some(category), Some(profile))
        .await
        .unwrap_or_default();
    let persona = app
        .resolve_scoring_persona(Some(category), Some(profile), Some(category))
        .await
        .unwrap_or_default();
    let decision = build_import_profile_decision(
        profile,
        &required_audio_languages,
        &persona,
        &rescored,
        category,
        title.runtime_minutes,
        Some(size_bytes),
        has_existing_file,
    );
    let score = decision.preference_score;
    if !changes.is_empty() {
        tracing::debug!(
            title = %title.name,
            score,
            changes = ?changes,
            "mediainfo rescore applied to acquisition score"
        );
    }
    score
}

/// Convenience wrapper: probe and validate a file that is already at its final destination.
/// Used by non-upgrade import paths where the file has already been moved.
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
    probe_and_validate(
        app,
        title,
        parsed,
        quality_profile,
        path,
        size_bytes,
        has_existing_file,
        existing_score,
        is_filler,
    )
    .await
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

    let reason = Some(format!(
        "{}{}",
        rejection.message,
        if rejection.blocking_rule_codes.is_empty() {
            String::new()
        } else {
            format!(" [{}]", rejection.blocking_rule_codes.join(", "))
        }
    ));
    let _ = app
        .services
        .append_domain_event(new_title_domain_event(
            actor_user_id.map(str::to_owned),
            title,
            DomainEventPayload::ImportRejected(ImportRejectedEventData {
                title: Some(title_context_snapshot(title)),
                status: ImportStatus::Skipped,
                source_path: Some(path.display().to_string()),
                reason,
                episode_ids: episode_ids.to_vec(),
            }),
        ))
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
                let next_search_at = now_str.clone();
                let _ = app
                    .services
                    .wanted_items
                    .schedule_wanted_item_search(&WantedSearchTransition {
                        id: item.id.clone(),
                        next_search_at: Some(next_search_at),
                        last_search_at: None,
                        search_count: item.search_count,
                        current_score: item.current_score,
                        grabbed_release: None,
                    })
                    .await;
            }
            Ok(None) => {}
            Err(error) => {
                warn!(error = %error, title_id = %title_id, "failed to reset wanted item")
            }
        }
    }
}
