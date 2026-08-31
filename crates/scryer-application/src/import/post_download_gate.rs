use std::collections::HashSet;
use std::path::Path;

use crate::domain_events::{DomainEventActor, new_title_domain_event, title_context_snapshot};
use crate::media::release_labels::resolve_release_labels_from_analysis;
use crate::release_parser::AudioCodec;
use crate::{
    AppUseCase, NewBlocklistEntry, ReleaseDownloadAttemptOutcome,
    acquisition::convergence::CoverageReopen, normalize_release_attempt_hint,
    normalize_release_name,
};
use scryer_domain::{
    DomainEventPayload, ImportRejectedEventData, ImportSkipReason, ImportStatus, MediaFacet, Title,
};
use tracing::warn;

const SOURCE_CHANGED_AFTER_PROBE_CODE: &str = "source_changed_after_probe";

pub(crate) enum ImportedFileGateDecision {
    Accepted(Box<ImportedFileAcceptance>),
    #[cfg_attr(not(feature = "runtime-media-analysis"), allow(dead_code))]
    Rejected(ImportedFileRejection),
}

pub(crate) struct ImportedFileAcceptance {
    pub analysis: Option<crate::MediaFileAnalysis>,
    pub scan_error: Option<String>,
    pub rule_file_doc: Option<scryer_rules::FileDoc>,
    /// Set when the file was accepted but a required audio language could not be
    /// verified (untagged tracks, or the requirement could not be resolved).
    /// Currently emitted as an operator `warn!` log line at import time; the
    /// durable/UI review surface is deferred.
    pub audio_language_warning: Option<String>,
}

pub(crate) struct PreparedImportCandidate {
    pub parsed: crate::ParsedReleaseMetadata,
    pub accepted: Box<ImportedFileAcceptance>,
    pub rescore_changes: Vec<String>,
    pub source_snapshot: scryer_domain::ImportSourceSnapshot,
}

pub(crate) struct PostDownloadAcquisitionDecision {
    pub parsed: crate::ParsedReleaseMetadata,
    pub score: i32,
    /// Where the imported file's quality sits in the profile's ordering.
    /// Admission compares this before it looks at the score.
    pub tier_index: Option<usize>,
    /// PROPER/REPACK rank of the release, compared between tier and score (D9).
    /// From the same scoring pass as `score`, so the import side cannot read a
    /// different revision than the grab side did for the same release.
    pub revision: i32,
    /// Whether the analyzed evidence contradicted the announcement.
    ///
    /// Acted on before admission by [`resolve_truth_verdict_action`], which is
    /// the only place that turns a verdict into an import decision.
    pub truth_verdict: crate::canonical_scoring::TruthVerdict,
    pub scoring_log: Option<String>,
}

#[derive(Debug)]
pub struct ImportedFileRejection {
    pub message: String,
    pub recycle_reason: &'static str,
    pub skip_reason: Option<ImportSkipReason>,
    pub blocking_rule_codes: Vec<String>,
}

fn import_source_changed_rejection(
    path: &Path,
    detail: impl std::fmt::Display,
) -> ImportedFileRejection {
    ImportedFileRejection {
        message: format!(
            "import source changed after validation probe: {} ({detail})",
            path.display()
        ),
        recycle_reason: "import_source_changed_after_probe",
        skip_reason: Some(ImportSkipReason::PolicyMismatch),
        blocking_rule_codes: vec![SOURCE_CHANGED_AFTER_PROBE_CODE.to_string()],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeSampleValidationMode {
    EnforceAutomatic,
    BypassRuntimeSampleCheck,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeSampleValidation {
    pub mode: RuntimeSampleValidationMode,
    pub expected_runtime_seconds: Option<i32>,
}

impl RuntimeSampleValidation {
    pub(crate) fn automatic(expected_runtime_seconds: Option<i32>) -> Self {
        Self {
            mode: RuntimeSampleValidationMode::EnforceAutomatic,
            expected_runtime_seconds,
        }
    }

    pub(crate) fn manual_override(expected_runtime_seconds: Option<i32>) -> Self {
        Self {
            mode: RuntimeSampleValidationMode::BypassRuntimeSampleCheck,
            expected_runtime_seconds,
        }
    }
}

#[cfg(any(feature = "runtime-media-analysis", test))]
pub(crate) const SAMPLE_RUNTIME_ZERO_CODE: &str = "sample_runtime_zero";
#[cfg(any(feature = "runtime-media-analysis", test))]
pub(crate) const SAMPLE_RUNTIME_TOO_SHORT_CODE: &str = "sample_runtime_too_short";
#[cfg(any(feature = "runtime-media-analysis", test))]
pub(crate) const SAMPLE_RUNTIME_INDETERMINATE_CODE: &str = "sample_runtime_indeterminate";
pub(crate) const RUNTIME_OUT_OF_BAND_CODE: &str = "runtime_out_of_band";

#[cfg(any(feature = "runtime-media-analysis", test))]
const MIN_EXPECTED_RUNTIME_FOR_SAMPLE_RATIO_SECONDS: i32 = 5 * 60;
#[cfg(any(feature = "runtime-media-analysis", test))]
const MIN_UNKNOWN_RUNTIME_SAMPLE_SECONDS: i32 = 60;
#[cfg(any(feature = "runtime-media-analysis", test))]
const MIN_RATIO_SAMPLE_SECONDS: i32 = 90;
#[cfg(any(feature = "runtime-media-analysis", test))]
const SAMPLE_RUNTIME_PERCENT: i32 = 10;
/// Coarse plausibility band around the expected runtime. Deliberately wide (2x
/// each way) so extended pilots, omnibus cuts, and padded encodes stay
/// importable while a file that is plainly a different episode is rejected.
/// Shared with the replace guard, which reuses the same band against the
/// incumbent file's stored duration when no expected runtime exists.
const RUNTIME_BAND_MIN_PERCENT: i32 = 50;
const RUNTIME_BAND_MAX_PERCENT: i32 = 200;

/// Reason code for an automatic replacement held back for manual resolution
/// because the incoming file's duration is implausible against the file it
/// would overwrite.
pub(crate) const REPLACE_BLOCKED_RUNTIME_MISMATCH_CODE: &str = "replace_blocked_runtime_mismatch";

pub(crate) fn facet_to_category_hint(facet: &MediaFacet) -> &'static str {
    facet.as_str()
}

#[cfg(any(feature = "runtime-media-analysis", test))]
fn runtime_sample_rejection(
    validation: RuntimeSampleValidation,
    actual_runtime_seconds: Option<i32>,
) -> Option<ImportedFileRejection> {
    if validation.mode == RuntimeSampleValidationMode::BypassRuntimeSampleCheck {
        return None;
    }

    let Some(actual_seconds) = actual_runtime_seconds else {
        return Some(imported_runtime_sample_rejection(
            SAMPLE_RUNTIME_INDETERMINATE_CODE,
            "video file could not be read to verify its duration; it may be incomplete or corrupted. Download it again and retry the import".to_string(),
        ));
    };

    if actual_seconds <= 0 {
        return Some(imported_runtime_sample_rejection(
            SAMPLE_RUNTIME_ZERO_CODE,
            "imported file runtime is zero for automatic import".to_string(),
        ));
    }

    if let Some(expected_seconds) = validation.expected_runtime_seconds
        && expected_seconds >= MIN_EXPECTED_RUNTIME_FOR_SAMPLE_RATIO_SECONDS
    {
        let threshold_seconds = MIN_RATIO_SAMPLE_SECONDS
            .max(expected_seconds.saturating_mul(SAMPLE_RUNTIME_PERCENT) / 100);
        if actual_seconds < threshold_seconds {
            return Some(imported_runtime_sample_rejection(
                SAMPLE_RUNTIME_TOO_SHORT_CODE,
                format!(
                    "imported file runtime is too short for automatic import: expected about {} minutes, probed file is {} seconds",
                    (expected_seconds + 59) / 60,
                    actual_seconds
                ),
            ));
        }

        // Past the sample detector, hold the file to a coarse plausibility band.
        // `expected_seconds` already arrives episode-count-normalized, so a
        // multi-episode file is compared against the summed expectation and is
        // never re-multiplied here. Band endpoints pass; only a duration strictly
        // outside them rejects.
        let band_min_seconds = expected_seconds.saturating_mul(RUNTIME_BAND_MIN_PERCENT) / 100;
        let band_max_seconds = expected_seconds.saturating_mul(RUNTIME_BAND_MAX_PERCENT) / 100;
        if actual_seconds < band_min_seconds {
            return Some(imported_runtime_sample_rejection(
                RUNTIME_OUT_OF_BAND_CODE,
                format!(
                    "imported file runtime is too short for the expected runtime: expected about {} minutes, probed file is {} seconds",
                    (expected_seconds + 59) / 60,
                    actual_seconds
                ),
            ));
        }
        if actual_seconds > band_max_seconds {
            return Some(imported_runtime_sample_rejection(
                RUNTIME_OUT_OF_BAND_CODE,
                format!(
                    "imported file runtime is too long for the expected runtime: expected about {} minutes, probed file is {} seconds",
                    (expected_seconds + 59) / 60,
                    actual_seconds
                ),
            ));
        }

        return None;
    }

    if validation.expected_runtime_seconds.is_none()
        && actual_seconds < MIN_UNKNOWN_RUNTIME_SAMPLE_SECONDS
    {
        return Some(imported_runtime_sample_rejection(
            SAMPLE_RUNTIME_TOO_SHORT_CODE,
            format!(
                "imported file runtime is too short for automatic import: probed file is {} seconds",
                actual_seconds
            ),
        ));
    }

    None
}

#[cfg(any(feature = "runtime-media-analysis", test))]
fn imported_runtime_sample_rejection(code: &'static str, message: String) -> ImportedFileRejection {
    ImportedFileRejection {
        message,
        recycle_reason: code,
        skip_reason: Some(ImportSkipReason::PolicyMismatch),
        blocking_rule_codes: vec![code.to_string()],
    }
}

/// Total stored duration of the primary files an automatic import would
/// overwrite. Mirrors `expected_runtime_seconds_for_episode_import`: the sum
/// only counts when *every* incumbent carries a positive stored duration, so a
/// partially scanned library leaves the replace guard inert rather than
/// comparing against a short-changed denominator.
pub(crate) fn incumbent_replace_runtime_seconds(
    durations: impl IntoIterator<Item = Option<i32>>,
) -> Option<i32> {
    let mut total: i32 = 0;
    let mut counted = false;
    for duration in durations {
        let duration = duration.filter(|seconds| *seconds > 0)?;
        total = total.saturating_add(duration);
        counted = true;
    }
    counted.then_some(total)
}

/// Replacing an existing primary file is held to a stricter standard than
/// filling an empty slot. When the expected-runtime band could not run because
/// the catalog has no runtime for the target, fall back to the incumbent file's
/// stored duration and hold an incoming file outside the same band for manual
/// resolution instead of overwriting the library copy.
///
/// Permissive by construction: without both a probed duration and a stored
/// incumbent duration the guard cannot run, so unscanned incumbents and builds
/// without media analysis keep today's behavior. Manual imports bypass it for
/// the same reason they bypass the sample check — the operator picked the file.
pub(crate) fn replace_runtime_band_block(
    validation: RuntimeSampleValidation,
    accepted: &ImportedFileAcceptance,
    incumbent_runtime_seconds: Option<i32>,
) -> Option<String> {
    if validation.mode == RuntimeSampleValidationMode::BypassRuntimeSampleCheck {
        return None;
    }
    // Only the no-expected-runtime case reaches here; when an expected runtime
    // exists the band already ran against it during the gate.
    if validation.expected_runtime_seconds.is_some() {
        return None;
    }

    let incumbent_seconds = incumbent_runtime_seconds.filter(|seconds| *seconds > 0)?;
    let actual_seconds = accepted
        .analysis
        .as_ref()
        .and_then(|analysis| analysis.duration_seconds)
        .filter(|seconds| *seconds > 0)?;

    let band_min_seconds = incumbent_seconds.saturating_mul(RUNTIME_BAND_MIN_PERCENT) / 100;
    let band_max_seconds = incumbent_seconds.saturating_mul(RUNTIME_BAND_MAX_PERCENT) / 100;
    // Wording deliberately avoids the "blocked"/"locked" vocabulary that
    // `completed_import_error_message_is_retryable` treats as transient.
    if actual_seconds < band_min_seconds {
        return Some(format!(
            "imported file runtime is too short to replace the existing file: existing file is about {} minutes, probed file is {} seconds; held for manual resolution",
            (incumbent_seconds + 59) / 60,
            actual_seconds
        ));
    }
    if actual_seconds > band_max_seconds {
        return Some(format!(
            "imported file runtime is too long to replace the existing file: existing file is about {} minutes, probed file is {} seconds; held for manual resolution",
            (incumbent_seconds + 59) / 60,
            actual_seconds
        ));
    }

    None
}

/// The profile decision handed to the import-time **rule** input.
///
/// Not a score: `total` comes from [`crate::canonical_scoring::score_release`]
/// on both sides of every comparison. This exists only because
/// `build_rule_input` wants a `QualityProfileDecision` to expose to rules, and
/// it is built from the same terms the canonical pass uses.
#[cfg(feature = "runtime-media-analysis")]
fn resolved_import_profile(
    profile: &crate::QualityProfile,
    required_audio_languages: &[String],
    persona: &crate::ScoringPersona,
) -> crate::QualityProfile {
    let mut resolved_profile = profile.clone();
    resolved_profile.criteria.required_audio_languages = required_audio_languages.to_vec();
    resolved_profile.criteria.scoring_persona = persona.clone();
    resolved_profile.criteria.facet_persona_overrides.clear();
    resolved_profile
}

#[cfg(feature = "runtime-media-analysis")]
pub(crate) fn build_import_profile_decision(
    profile: &crate::QualityProfile,
    parsed: &crate::ParsedReleaseMetadata,
    category_hint: &str,
    size_basis: crate::quality_profile::CoverageSizeBasis,
    size_bytes: Option<i64>,
    has_existing_file: bool,
) -> crate::QualityProfileDecision {
    let weights = crate::scoring_weights::build_weights_for_category(
        &profile.criteria.scoring_persona,
        &profile.criteria.scoring_overrides,
        Some(category_hint),
    );
    let mut decision = crate::quality_profile::evaluate_against_profile_for_category(
        profile,
        parsed,
        has_existing_file,
        &weights,
        Some(category_hint),
    );
    crate::quality_profile::apply_size_scoring_for_category_with_remux_preference(
        &mut decision,
        parsed,
        size_bytes,
        Some(category_hint),
        size_basis,
        profile.criteria.prefer_remux,
        &weights,
    );
    decision
}

#[cfg(feature = "runtime-media-analysis")]
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
        video_codec: analysis
            .video_codec
            .as_deref()
            .and_then(crate::release_parser::VideoCodec::parse),
        video_width: analysis.video_width,
        video_height: analysis.video_height,
        video_bitrate_kbps: analysis.video_bitrate_kbps,
        video_bit_depth: analysis.video_bit_depth,
        video_hdr_format: analysis.video_hdr_format.clone(),
        dovi_profile: analysis.dovi_profile,
        dovi_bl_compat_id: analysis.dovi_bl_compat_id,
        video_frame_rate: analysis.video_frame_rate.clone(),
        video_profile: analysis.video_profile.clone(),
        audio_codec: analysis.audio_codec.clone(),
        audio_profile: analysis.audio_profile.clone(),
        audio_channels: analysis.audio_channels,
        audio_bitrate_kbps: analysis.audio_bitrate_kbps,
        audio_languages,
        audio_streams: analysis
            .audio_streams
            .iter()
            .map(|stream| crate::AudioStreamDetail {
                codec: stream.codec.clone(),
                profile: stream.profile.clone(),
                channels: stream.channels,
                language: stream
                    .language
                    .as_deref()
                    .and_then(crate::normalize_detected_audio_language_code),
                name: stream.name.clone(),
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
    }
}

pub(crate) fn build_stream_pointer_media_file_analysis() -> crate::MediaFileAnalysis {
    crate::MediaFileAnalysis {
        video_codec: None,
        video_width: None,
        video_height: None,
        video_bitrate_kbps: None,
        video_bit_depth: None,
        video_hdr_format: None,
        dovi_profile: None,
        dovi_bl_compat_id: None,
        video_frame_rate: None,
        video_profile: None,
        audio_codec: None,
        audio_profile: None,
        audio_channels: None,
        audio_bitrate_kbps: None,
        audio_languages: Vec::new(),
        audio_streams: Vec::new(),
        subtitle_languages: Vec::new(),
        subtitle_codecs: Vec::new(),
        subtitle_streams: Vec::new(),
        has_multiaudio: false,
        duration_seconds: None,
        num_chapters: None,
        container_format: Some("strm".to_string()),
    }
}

fn build_synthetic_media_file_analysis(
    parsed: &crate::ParsedReleaseMetadata,
    container_format: Option<String>,
) -> crate::MediaFileAnalysis {
    let (video_width, video_height) = infer_video_dimensions(parsed.quality.as_deref());

    crate::MediaFileAnalysis {
        video_codec: None,
        video_width,
        video_height,
        video_bitrate_kbps: None,
        video_bit_depth: None,
        video_hdr_format: None,
        dovi_profile: None,
        dovi_bl_compat_id: None,
        video_frame_rate: None,
        video_profile: None,
        audio_codec: None,
        audio_profile: None,
        audio_channels: None,
        audio_bitrate_kbps: None,
        audio_languages: Vec::new(),
        audio_streams: Vec::new(),
        subtitle_languages: Vec::new(),
        subtitle_codecs: Vec::new(),
        subtitle_streams: Vec::new(),
        has_multiaudio: false,
        duration_seconds: None,
        num_chapters: None,
        container_format,
    }
}

#[cfg(feature = "runtime-media-analysis")]
fn build_stream_pointer_media_file_analysis_from_parsed(
    parsed: &crate::ParsedReleaseMetadata,
) -> crate::MediaFileAnalysis {
    build_synthetic_media_file_analysis(parsed, Some("strm".to_string()))
}

fn infer_video_dimensions(quality: Option<&str>) -> (Option<i32>, Option<i32>) {
    match quality
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("2160p") => (Some(3840), Some(2160)),
        Some("1080p") => (Some(1920), Some(1080)),
        Some("720p") => (Some(1280), Some(720)),
        Some("480p") => (Some(854), Some(480)),
        _ => (None, None),
    }
}

fn inferred_container_format_for_path(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_ascii_lowercase())
}

#[cfg(feature = "runtime-media-analysis")]
fn path_is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

/// Probe a file at the given path and validate it against the quality profile and user rules.
/// The file does NOT need to be at its final destination — this can probe a file in-place
/// at its download location before any move/copy.
#[expect(
    clippy::too_many_arguments,
    reason = "probe-and-validate carries the full import gate context through one decision point"
)]
#[cfg(feature = "runtime-media-analysis")]
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
    runtime_sample_validation: RuntimeSampleValidation,
) -> ImportedFileGateDecision {
    // Before anything touches the file, and on the calling thread (the override
    // is thread-local, and the real probe hands off to `spawn_blocking`).
    #[cfg(test)]
    if let Some(acceptance) = probe_override::take() {
        return ImportedFileGateDecision::Accepted(Box::new(acceptance));
    }

    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("strm"))
    {
        return ImportedFileGateDecision::Accepted(Box::new(ImportedFileAcceptance {
            analysis: Some(build_stream_pointer_media_file_analysis_from_parsed(parsed)),
            scan_error: None,
            rule_file_doc: None,
            audio_language_warning: None,
        }));
    }

    let analysis = match scryer_mediainfo::analyze_file(path) {
        Ok(analysis) => analysis,
        Err(error) => {
            warn!(error = %error, path = %path.display(), "media analysis failed");
            if let Some(rejection) = runtime_sample_rejection(runtime_sample_validation, None) {
                return ImportedFileGateDecision::Rejected(rejection);
            }
            let synthetic_analysis = path_is_symlink(path).then(|| {
                build_synthetic_media_file_analysis(
                    parsed,
                    inferred_container_format_for_path(path),
                )
            });
            return ImportedFileGateDecision::Accepted(Box::new(ImportedFileAcceptance {
                analysis: synthetic_analysis,
                scan_error: Some(error.to_string()),
                rule_file_doc: None,
                audio_language_warning: None,
            }));
        }
    };

    if analysis.video_codec.is_none() {
        return ImportedFileGateDecision::Rejected(ImportedFileRejection {
            message: "imported file is not a valid video".to_string(),
            recycle_reason: "invalid_file",
            skip_reason: None,
            blocking_rule_codes: Vec::new(),
        });
    }

    if let Some(rejection) =
        runtime_sample_rejection(runtime_sample_validation, analysis.duration_seconds)
    {
        return ImportedFileGateDecision::Rejected(rejection);
    }

    let category_hint = facet_to_category_hint(&title.facet);
    let required_audio_resolution = app.resolve_required_audio_languages_for_title(title).await;
    let required_audio_resolution_failed = required_audio_resolution.is_err();
    let required_audio_languages = required_audio_resolution.unwrap_or_else(|error| {
        warn!(
            error = %error,
            title_id = %title.id,
            "failed to resolve required audio languages; importing without language verification"
        );
        Vec::new()
    });

    let accepted_analysis = build_media_file_analysis(&analysis);

    // Required audio language gate (post-download, file truth). Distinguishes a
    // provable absence (reject) from an untagged/indeterminate result (accept +
    // flag), so a correctly-dubbed file with "und"/untagged tracks is not falsely
    // rejected. Uses the same title context + release hints as the search gate.
    //
    // Manual imports (operator-chosen files) always land: they bypass this gate
    // entirely, exactly as they bypass the runtime-sample check.
    let mut audio_language_warning: Option<String> = None;
    let enforce_required_audio =
        runtime_sample_validation.mode == RuntimeSampleValidationMode::EnforceAutomatic;
    if enforce_required_audio && required_audio_resolution_failed {
        audio_language_warning = Some(
            "required audio languages could not be resolved; imported without language verification"
                .to_string(),
        );
    }
    if enforce_required_audio && !required_audio_languages.is_empty() {
        let title_audio_context = crate::title_audio_language_context(
            title.language.as_deref(),
            title.country.as_deref(),
            Some(category_hint),
            &title.tags,
        );
        let release_audio_hints = crate::release_audio_language_hints_for_title(
            parsed,
            None,
            Some(&title_audio_context),
            false,
        );
        match crate::classify_required_audio(
            &required_audio_languages,
            &accepted_analysis.audio_streams,
            &release_audio_hints,
        ) {
            crate::RequiredAudioVerdict::Satisfied => {}
            crate::RequiredAudioVerdict::Missing(missing) => {
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
            crate::RequiredAudioVerdict::Indeterminate(unverified) => {
                // Neither provably present nor provably absent (untagged tracks):
                // accept rather than bury a possibly-good release, but flag it.
                audio_language_warning = Some(format!(
                    "audio language(s) {} could not be verified from file metadata (untagged track(s)); imported for review",
                    unverified.join(", ")
                ));
            }
        }
    }

    let persona = app
        .resolve_scoring_persona(Some(title.library_id.as_str()), Some(category_hint))
        .await
        .unwrap_or_else(|error| {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to resolve scoring persona, using canonical default"
            );
            crate::ScoringPersona::default()
        });

    // **One FileDoc constructor** (D5). Built from `accepted_analysis` — the
    // same `MediaFileAnalysis` this import persists on the row — so the document
    // a rule sees at import is byte-for-byte the one it sees when the bar is
    // re-derived from that row. The import path used to build its doc straight
    // from `scryer_mediainfo::MediaAnalysis`, whose `video_codec` is the raw
    // probe string (`"h264"`), while re-derivation goes through
    // `VideoCodec::as_str()` (`"H.264"`): any rule reading
    // `input.file.video_codec` scored differently on the two sides, permanently.
    let rule_file_doc = crate::user_rule_input::file_doc_from_analysis(&accepted_analysis);
    let accepted_for_rules = ImportedFileAcceptance {
        analysis: Some(accepted_analysis.clone()),
        scan_error: None,
        rule_file_doc: Some(rule_file_doc.clone()),
        audio_language_warning: None,
    };
    let (rescored_for_rules, _) = rescore_from_mediainfo(parsed, &accepted_for_rules);

    let user_rules_engine = app
        .services
        .customization
        .user_rules
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| scryer_rules::UserRulesEngine::empty());
    if !user_rules_engine.is_empty() {
        let library_name = match app
            .services
            .catalog
            .libraries
            .get_by_id(&title.library_id)
            .await
        {
            Ok(Some(library)) => Some(library.name),
            Ok(None) => None,
            Err(error) => {
                warn!(
                    error = %error,
                    library_id = %title.library_id,
                    "failed to resolve library name for post-download rule context"
                );
                None
            }
        };
        let resolved_profile =
            resolved_import_profile(quality_profile, &required_audio_languages, &persona);
        let decision = build_import_profile_decision(
            &resolved_profile,
            &rescored_for_rules,
            category_hint,
            // The probe knows the title and the file, not the scope: no
            // coverage reaches here, so this is the single-member basis the
            // lane has always used. The canonical score — the number anything
            // is compared by — is built in
            // [`compute_post_download_acquisition_decision`], from the scope's
            // real basis.
            crate::quality_profile::CoverageSizeBasis::single(title.runtime_minutes),
            Some(size_bytes),
            has_existing_file,
        );
        let input = crate::user_rule_input::build_rule_input(
            &rescored_for_rules,
            &resolved_profile,
            &decision,
            crate::user_rule_input::ReleaseRuntimeInfo {
                size_bytes: Some(size_bytes),
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: None,
            },
            crate::user_rule_input::RuleContextInfo {
                title_id: Some(&title.id),
                library_name: library_name.as_deref(),
                category: Some(facet_to_category_hint(&title.facet)),
                original_language: title.language.as_deref(),
                original_country: title.country.as_deref(),
                title_tags: &title.tags,
                has_existing_file,
                existing_score,
                search_mode: "post_download",
                runtime_minutes: title.runtime_minutes,
                is_filler,
            },
            Some(rule_file_doc.clone()),
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
        analysis: Some(accepted_analysis),
        scan_error: None,
        rule_file_doc: Some(rule_file_doc),
        audio_language_warning,
    }))
}

#[expect(
    clippy::too_many_arguments,
    reason = "probe-and-validate carries the full import gate context through one decision point"
)]
#[cfg(not(feature = "runtime-media-analysis"))]
pub(crate) async fn probe_and_validate(
    _app: &AppUseCase,
    _title: &Title,
    parsed: &crate::ParsedReleaseMetadata,
    _quality_profile: &crate::QualityProfile,
    path: &Path,
    _size_bytes: i64,
    _has_existing_file: bool,
    _existing_score: Option<i32>,
    _is_filler: bool,
    _runtime_sample_validation: RuntimeSampleValidation,
) -> ImportedFileGateDecision {
    #[cfg(test)]
    if let Some(acceptance) = probe_override::take() {
        return ImportedFileGateDecision::Accepted(Box::new(acceptance));
    }

    ImportedFileGateDecision::Accepted(Box::new(ImportedFileAcceptance {
        analysis: Some(build_synthetic_media_file_analysis(
            parsed,
            inferred_container_format_for_path(path),
        )),
        scan_error: Some("native media analysis is not compiled into this target".to_string()),
        rule_file_doc: None,
        audio_language_warning: None,
    }))
}

/// Test hook: what the probe reports for the next file.
///
/// Without it, `Blocked`, `Vetoed` and `Contradicted` are unreachable end to
/// end in the default test build. `scryer-application`'s `default = []` leaves
/// `runtime-media-analysis` off, so `probe_and_validate` returns
/// [`build_synthetic_media_file_analysis`] — an analysis derived *from the
/// release name*, which by construction agrees with it. Every truth-verdict
/// consequence (blocklist rows, reopened scopes, the disposition table) was
/// therefore only ever tested against hand-built verdicts, never through the
/// real import path.
///
/// **Both** `probe_and_validate` bodies consult it, feature-on and feature-off,
/// because which one compiles is not the test's choice: Cargo unifies features
/// across a workspace build, and `crates/scryer` depends on this crate with
/// `runtime`, which pulls in `runtime-media-analysis`. So
/// `cargo test -p scryer-application --lib` builds the synthetic body while
/// `cargo test --workspace --lib` builds the mediainfo one, and a hook on only
/// one of them makes the same test pass or fail depending on how it was invoked
/// — the mediainfo body would run a real probe against a sparse fixture and
/// reject it for having no readable duration.
///
/// Thread-local rather than a global: `#[tokio::test]` bodies run on the
/// current thread and lib tests run in parallel, so a shared cell would
/// cross-contaminate. That is also why the feature-on body reads it as its very
/// first statement, before the real probe hands the file to `spawn_blocking`.
/// Consumed once per probe, and the RAII guard clears it on drop, so a test that
/// panics mid-import cannot leak an override into whatever runs next on that
/// thread.
#[cfg(test)]
pub(crate) mod probe_override {
    use super::ImportedFileAcceptance;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    thread_local! {
        static NEXT: RefCell<VecDeque<ImportedFileAcceptance>> = const { RefCell::new(VecDeque::new()) };
    }

    /// Install the analysis the next probe on this thread will report.
    #[must_use = "the override is cleared when the guard drops"]
    pub(crate) fn install(acceptance: ImportedFileAcceptance) -> ProbeOverrideGuard {
        install_sequence([acceptance])
    }

    /// Install the analyses successive probes on this thread will report.
    #[must_use = "the override is cleared when the guard drops"]
    pub(crate) fn install_sequence(
        acceptances: impl IntoIterator<Item = ImportedFileAcceptance>,
    ) -> ProbeOverrideGuard {
        NEXT.with(|slot| {
            let mut queued = slot.borrow_mut();
            queued.clear();
            queued.extend(acceptances);
        });
        ProbeOverrideGuard
    }

    pub(super) fn take() -> Option<ImportedFileAcceptance> {
        NEXT.with(|slot| slot.borrow_mut().pop_front())
    }

    pub(crate) struct ProbeOverrideGuard;

    impl Drop for ProbeOverrideGuard {
        fn drop(&mut self) {
            NEXT.with(|slot| slot.borrow_mut().clear());
        }
    }
}

/// Probe a source file once, apply the existing gate, and merge detected media
/// facts back into parsed metadata so downstream rename and scoring decisions
/// use the same resolved view that will later be persisted.
///
/// `parsed` is the canonical import parse of the release evidence — the same
/// title-aware parse the grab path scored (see
/// `import_workflow::parse_import_release_for_title`) — and is consumed as is:
/// re-parsing here would either double-parse or diverge from what was grabbed.
#[expect(
    clippy::too_many_arguments,
    reason = "prepared import candidates need the full gate context plus caller scoring state"
)]
pub(crate) async fn prepare_import_candidate(
    app: &AppUseCase,
    title: &Title,
    parsed: &crate::ParsedReleaseMetadata,
    quality_profile: &crate::QualityProfile,
    path: &Path,
    size_bytes: i64,
    has_existing_file: bool,
    existing_score: Option<i32>,
    is_filler: bool,
    runtime_sample_validation: RuntimeSampleValidation,
) -> Result<PreparedImportCandidate, ImportedFileRejection> {
    let source_snapshot_before = app
        .services
        .workflow
        .file_importer
        .snapshot_import_source(path)
        .await
        .map_err(|err| import_source_changed_rejection(path, err))?;

    match probe_and_validate(
        app,
        title,
        parsed,
        quality_profile,
        path,
        size_bytes,
        has_existing_file,
        existing_score,
        is_filler,
        runtime_sample_validation,
    )
    .await
    {
        ImportedFileGateDecision::Rejected(rejection) => Err(rejection),
        ImportedFileGateDecision::Accepted(accepted) => {
            let source_snapshot_after = app
                .services
                .workflow
                .file_importer
                .snapshot_import_source(path)
                .await
                .map_err(|err| import_source_changed_rejection(path, err))?;
            if source_snapshot_after != source_snapshot_before {
                return Err(import_source_changed_rejection(
                    path,
                    "source identity or content proof changed",
                ));
            }

            let (parsed, rescore_changes) = rescore_from_mediainfo(parsed, accepted.as_ref());
            if !rescore_changes.is_empty() {
                tracing::debug!(
                    title = %title.name,
                    path = %path.display(),
                    changes = ?rescore_changes,
                    "mediainfo rescore prepared import candidate"
                );
            }
            // Surface a required-audio "could not verify" flag (untagged tracks):
            // the file was accepted for review rather than falsely rejected.
            if let Some(warning) = accepted.audio_language_warning.as_deref() {
                warn!(
                    title_id = %title.id,
                    title = %title.name,
                    path = %path.display(),
                    warning,
                    "imported file accepted with unverified required audio language(s) for review"
                );
            }

            Ok(PreparedImportCandidate {
                parsed,
                accepted,
                rescore_changes,
                source_snapshot: source_snapshot_after,
            })
        }
    }
}

/// Merge mediainfo-detected values into a release-name-parsed metadata struct.
/// Prefers mediainfo when it detects a concrete value that differs from the release name.
/// Returns the merged metadata and a log of what changed.
pub(crate) fn rescore_from_mediainfo(
    parsed: &crate::ParsedReleaseMetadata,
    acceptance: &ImportedFileAcceptance,
) -> (crate::ParsedReleaseMetadata, Vec<String>) {
    rescore_parsed_from_analysis(parsed, acceptance.analysis.as_ref())
}

/// The same rescore, reachable from a bare `MediaFileAnalysis`.
///
/// Canonical scoring re-derives an incumbent's bar from a media row, which
/// carries the stored analysis but no import-time acceptance. Keeping one body
/// behind both entry points is what makes the re-derived score identical to the
/// one the import originally wrote.
pub(crate) fn rescore_parsed_from_analysis(
    parsed: &crate::ParsedReleaseMetadata,
    analysis: Option<&crate::MediaFileAnalysis>,
) -> (crate::ParsedReleaseMetadata, Vec<String>) {
    let Some(analysis) = analysis else {
        return (parsed.clone(), vec![]);
    };

    let mut merged = parsed.clone();
    let mut changes = Vec::new();
    let resolved = resolve_release_labels_from_analysis(
        analysis.video_width,
        analysis.video_height,
        analysis.video_codec.as_ref(),
        analysis.audio_codec.as_deref(),
        analysis.audio_profile.as_deref(),
        analysis.audio_channels,
        &analysis.audio_streams,
    );

    // Override resolution from video height
    if let Some(ref detected) = resolved.quality
        && merged.quality.as_deref() != Some(detected.as_str())
    {
        changes.push(format!(
            "resolution: {} → {}",
            merged.quality.as_deref().unwrap_or("?"),
            detected
        ));
        merged.quality = Some(detected.clone());
    }

    // Override video codec (map mediainfo names → release parser names)
    if let Some(ref normalized) = resolved.video_codec
        && let Some(parsed_codec) = crate::release_parser::VideoCodec::parse(normalized.as_str())
        && merged.video_codec.as_ref() != Some(&parsed_codec)
    {
        changes.push(format!(
            "video_codec: {} → {}",
            merged
                .video_codec
                .as_ref()
                .map(ToString::to_string)
                .as_deref()
                .unwrap_or("?"),
            normalized
        ));
        merged.video_codec = Some(parsed_codec);
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
    if let Some(ref normalized) = resolved.audio_codec
        && let Some(codec) = AudioCodec::parse(normalized)
        && merged.audio.as_ref() != Some(&codec)
    {
        changes.push(format!(
            "audio: {} → {}",
            merged.audio.as_ref().map(AudioCodec::as_str).unwrap_or("?"),
            normalized
        ));
        merged.audio = Some(codec);
    }

    if let Some(ref ch_str) = resolved.audio_channels
        && merged.audio_channels.as_deref() != Some(ch_str.as_str())
    {
        changes.push(format!(
            "audio_channels: {} → {}",
            merged.audio_channels.as_deref().unwrap_or("?"),
            ch_str
        ));
        merged.audio_channels = Some(ch_str.clone());
    }

    if !analysis.audio_streams.is_empty() {
        // Detect multi-audio from stream count
        if analysis.audio_streams.len() > 1 && !merged.is_dual_audio {
            changes.push("dual_audio: detected multiple audio tracks".to_string());
            merged.is_dual_audio = true;
        }

        if resolved.is_atmos && !merged.is_atmos {
            changes.push("atmos: detected from audio streams".to_string());
            merged.is_atmos = true;
        }
    }

    (merged, changes)
}
/// Recycle reason for an import the analyzed pass vetoed outright.
pub(crate) const TRUTH_BLOCKED_CODE: &str = "truth_blocked";
/// Recycle reason for a release that landed in a worse quality tier than it
/// advertised.
pub(crate) const TRUTH_QUALITY_DOWNGRADE_CODE: &str = "truth_quality_downgrade";
/// Recycle reason for a file the profile vetoes over a property its release name
/// never disclosed. This is burned so convergence can seek another candidate.
pub(crate) const TRUTH_VETOED_CODE: &str = "truth_vetoed";

/// Prefix of the verdict code [`crate::canonical_scoring::score_release`] emits
/// when the landed quality differs from the announced one.
const QUALITY_CONTRADICTED_PREFIX: &str = "quality_contradicted:";

/// What a truth verdict means for the import in front of it.
///
/// One enum, one resolver, three call sites (episode, title, series-movie link)
/// — the decision is the release's, not the scope's, so it must not be spelled
/// three ways.
#[derive(Debug)]
pub(crate) enum TruthVerdictAction {
    /// Nothing the verdict says stops this import.
    Import,
    /// Import it — the scope is empty and an honest file at its real tier beats
    /// nothing — but blocklist the release for this title so the next upgrade
    /// search cannot re-grab the same lie and "upgrade" the scope to the tier it
    /// already has. Sonarr loops here; we do not.
    ImportAndBlocklist { code: &'static str, reason: String },
    /// Refuse it: recycle the bytes, blocklist the release for this title, and
    /// reopen the scope's search so it seeks a different candidate.
    Reject(ImportedFileRejection),
}

/// The `(announced, landed)` pair from a quality-contradiction code, if one is
/// present.
fn quality_contradiction(codes: &[String]) -> Option<(&str, &str)> {
    codes.iter().find_map(|code| {
        code.strip_prefix(QUALITY_CONTRADICTED_PREFIX)
            .and_then(|pair| pair.split_once("->"))
    })
}

/// Whether the landed quality sits below the announced one in this profile's
/// ordering. A quality the profile does not list ranks below every quality it
/// does, matching [`crate::admission`]'s tier comparison.
fn landed_tier_is_worse(
    criteria: &crate::QualityProfileCriteria,
    announced: &str,
    landed: &str,
) -> bool {
    let announced_index = crate::quality_profile::quality_tier_index(criteria, Some(announced));
    let landed_index = crate::quality_profile::quality_tier_index(criteria, Some(landed));
    match (announced_index, landed_index) {
        (Some(announced_index), Some(landed_index)) => landed_index > announced_index,
        // Announced a tier the profile ranks, landed something it does not.
        (Some(_), None) => true,
        // The announcement was already unranked; landing somewhere ranked, or
        // somewhere equally unranked, is not a downgrade.
        (None, _) => false,
    }
}

/// Turn a truth verdict into an action. **The only place that decision is made.**
///
/// Two things earn a blocklist, and they are matched on explicitly rather than
/// inferred from the size of a number:
///
/// 1. [`crate::canonical_scoring::TruthVerdict::Blocked`] — the announcement
///    asserted a field and the file contradicts it: a stated codec that is not
///    the stream's codec and is on the profile's blocklist, a measured
///    resolution outside the profile's tiers. The release is not what it claimed
///    and no scope should take it.
/// 2. A `quality_contradicted:<announced>-><landed>` code whose landed tier is
///    *worse* than the announced one. An occupied scope would refuse it on tier
///    anyway, so naming the reason costs nothing; an empty scope keeps the file
///    (an honest 720p beats no episode) but must never be offered the same
///    release again as an upgrade.
///
/// A probe veto is an import failure. [`crate::canonical_scoring::TruthVerdict::Vetoed`]
/// means the downloaded file carries something this quality profile refuses,
/// even though the name did not state it. Blocklist the release and reopen the
/// scope: the next search excludes the burned release, deliberately walking
/// codec-silent releases into the blocklist one at a time until an import
/// succeeds.
///
/// A score-only `Contradicted` is deliberately **not** a blocklist: one size
/// bucket of drift is the ordinary case for usenet (par2/RAR overhead, a short
/// episode), and treating it as a lie would burn good releases. This is where
/// Radarr gave up and removed its equivalent check — its quality comparison
/// folded in WEBDL-vs-WEBRip source noise, so honest releases tripped it. Ours
/// cannot: [`crate::quality_profile::normalize_quality_tier`] keys on the
/// resolution alone, so a source relabel never reads as a tier change.
pub(crate) fn resolve_truth_verdict_action_for_origin(
    verdict: &crate::canonical_scoring::TruthVerdict,
    criteria: &crate::QualityProfileCriteria,
    scope_is_occupied: bool,
    origin: crate::import_decide::ImportOrigin,
) -> TruthVerdictAction {
    use crate::canonical_scoring::TruthVerdict;

    match verdict {
        TruthVerdict::Consistent => TruthVerdictAction::Import,
        TruthVerdict::Blocked { codes } => TruthVerdictAction::Reject(ImportedFileRejection {
            message: format!(
                "the downloaded file contradicts what the release advertised and cannot be imported: {}",
                if codes.is_empty() {
                    "file evidence blocked by the quality profile".to_string()
                } else {
                    codes.join(", ")
                }
            ),
            recycle_reason: TRUTH_BLOCKED_CODE,
            skip_reason: Some(ImportSkipReason::PolicyMismatch),
            blocking_rule_codes: codes.clone(),
        }),
        // The profile refuses the file over something the name never claimed.
        // A proven tier downgrade still takes the dedicated path first; every
        // other veto is burned so convergence can seek another candidate.
        TruthVerdict::Vetoed { codes } => {
            if let Some((announced, landed)) = quality_contradiction(codes)
                && landed_tier_is_worse(criteria, announced, landed)
            {
                return TruthVerdictAction::Reject(quality_downgrade_rejection(announced, landed));
            }
            TruthVerdictAction::Reject(ImportedFileRejection {
                message: format!(
                    "the downloaded file carries something this quality profile refuses, which the release never advertised: {}",
                    if codes.is_empty() {
                        "file evidence blocked by the quality profile".to_string()
                    } else {
                        codes.join(", ")
                    }
                ),
                recycle_reason: TRUTH_VETOED_CODE,
                skip_reason: Some(ImportSkipReason::PolicyMismatch),
                blocking_rule_codes: codes.clone(),
            })
        }
        TruthVerdict::Contradicted { codes } => {
            let Some((announced, landed)) = quality_contradiction(codes) else {
                // Size-only drift. Accept it and score it honestly.
                return TruthVerdictAction::Import;
            };
            if !landed_tier_is_worse(criteria, announced, landed) {
                return TruthVerdictAction::Import;
            }
            if origin == crate::import_decide::ImportOrigin::OperatorQueued {
                return TruthVerdictAction::Reject(quality_downgrade_rejection(announced, landed));
            }
            if scope_is_occupied {
                TruthVerdictAction::Reject(quality_downgrade_rejection(announced, landed))
            } else {
                TruthVerdictAction::ImportAndBlocklist {
                    code: TRUTH_QUALITY_DOWNGRADE_CODE,
                    reason: format!(
                        "release advertised {announced} but the file is {landed}; imported at its \
                         real quality and blocklisted so it is never re-grabbed as an upgrade"
                    ),
                }
            }
        }
    }
}

/// The refusal for a release whose file landed in a worse tier than it claimed.
/// One shape, so the `Contradicted` and `Vetoed` paths cannot word the same
/// finding two ways.
fn quality_downgrade_rejection(announced: &str, landed: &str) -> ImportedFileRejection {
    ImportedFileRejection {
        message: format!(
            "release advertised {announced} but the file is {landed}; refusing the import and \
             looking for another release"
        ),
        recycle_reason: TRUTH_QUALITY_DOWNGRADE_CODE,
        skip_reason: Some(ImportSkipReason::PolicyMismatch),
        blocking_rule_codes: vec![format!(
            "{QUALITY_CONTRADICTED_PREFIX}{announced}->{landed}"
        )],
    }
}

/// Which scope a blocklisted release belongs to.
///
/// The same three fields every grab-side failure writer persists
/// (`crate::decision_helpers::blocklist_entry_data`), so an import rejection and
/// a grab failure for the same release group together in the UI instead of the
/// import one filing itself under the linked episode.
#[derive(Clone, Copy, Default)]
pub(crate) struct BlocklistAttribution<'a> {
    pub episode_ids: &'a [String],
    /// Season-pack scope. Always `None` from import today: a pack is imported
    /// member by member, so each rejection is attributed to its episodes.
    pub collection_id: Option<&'a str>,
    pub series_movie_link_id: Option<&'a str>,
}

/// Blocklist a release for one title. The single writer, shared by the rejection
/// path and by an accepted-but-mis-advertised import.
///
/// No indexer is recorded. A release rejected here failed on its *content* --
/// it is not what it advertised -- so it is equally bad whoever served it, and
/// the empty indexer blocks it on all of them.
///
/// The same content argument keys the row on the grab-time infohash when the
/// title has a submission for this release name: a content-bad torrent is the
/// same bad content under any name on any indexer, and the hash is the one key
/// that follows it there. The lookup degrading to `None` degrades the block to
/// the name, which was the whole behaviour before the hash existed.
pub(crate) async fn blocklist_release_for_title(
    app: &AppUseCase,
    title: &Title,
    release_title: &str,
    reason: Option<String>,
) {
    let Some(release_name) = normalize_release_name(Some(release_title)) else {
        return;
    };
    let info_hash = app
        .services
        .workflow
        .download_submissions
        .find_info_hash_for_title_release(&title.id, &release_name)
        .await
        .unwrap_or_else(|error| {
            warn!(
                error = %error,
                title_id = %title.id,
                release_name = release_name.as_str(),
                "failed to resolve grab-time infohash for rejected import; blocking by name only"
            );
            None
        });
    if let Err(error) = app
        .services
        .workflow
        .blocklist_repo
        .block(&NewBlocklistEntry {
            title_id: title.id.clone(),
            release_name: release_name.clone(),
            indexer_id: String::new(),
            info_hash,
            reason,
        })
        .await
    {
        warn!(
            error = %error,
            title_id = %title.id,
            release_name = release_name.as_str(),
            "failed to persist blocklist entry for rejected import"
        );
    }
}

/// Score an imported file, canonically.
///
/// Delegates to [`crate::canonical_scoring::score_release`] — the same function
/// and the same resolved context the grab path uses. That shared calculator is
/// what stops grab and import disagreeing about the same release; this function
/// only assembles the evidence and shapes the result for the import flow.
///
/// Note what is *not* here: `has_existing_file` and `existing_score` are gone.
/// What is already on disk is an admission question, decided in
/// [`crate::admission`] against the real incumbent set. A score that moved with
/// the state of the library could never serve as the next comparison's bar.
///
/// `announced_size_bytes` is the landed size: by import time the advertised size
/// is no longer plumbed here. The variance therefore still catches a release
/// whose *streams* contradict its name — the anime case — but not one that lied
/// about its size. Plumbing the advertised size through the download record is a
/// prerequisite for acting on `truth_verdict`, not for scoring.
/// `context` is resolved by the caller and reused: the first-import paths score
/// twice (once to decide, once over the size that actually landed), and
/// resolving the profile, weights, rules and language requirements for each of
/// those was a database round trip buying nothing. With the context handed in,
/// this function is a pure term-pipeline run — which is also what makes
/// [`crate::import_decide::rescore_landed_size`] free.
#[allow(
    clippy::too_many_arguments,
    reason = "import scoring needs the full file context; the incumbent-state \
              arguments are already gone"
)]
pub(crate) fn compute_post_download_acquisition_decision(
    context: &crate::quality::canonical_context::ResolvedScoringContext,
    title: &Title,
    parsed: &crate::ParsedReleaseMetadata,
    acceptance: &ImportedFileAcceptance,
    size_basis: crate::quality_profile::CoverageSizeBasis,
    size_bytes: i64,
    prior_rescore_changes: &[String],
    is_filler: bool,
) -> PostDownloadAcquisitionDecision {
    let (rescored, changes) = rescore_from_mediainfo(parsed, acceptance);
    let mut rescore_changes = prior_rescore_changes.to_vec();
    for change in changes {
        if !rescore_changes
            .iter()
            .any(|existing_change| existing_change == &change)
        {
            rescore_changes.push(change);
        }
    }

    let view = context.view(size_basis, is_filler);

    // **Parse parity with the grab lane.** Most release names say nothing about
    // audio, and the grab path fills that in from the title's original language
    // (`announced_metadata_for_title`) before scoring. Import did not, so a
    // profile with `required_audio_languages` raised
    // `required_audio_language_missing` against every release the grab side had
    // just accepted — a veto on the announced pass of a file that is fine. The
    // dedicated `classify_required_audio` gate, which reads the file's real
    // audio streams, is the one that should speak here, and it already ran.
    let announced_parsed = crate::quality::canonical_context::announced_metadata_for_title(
        title,
        parsed,
        context.required_audio_languages(),
        None,
    );

    let mut evidence =
        crate::canonical_scoring::ReleaseEvidence::announced(announced_parsed, Some(size_bytes));
    if let Some(analysis) = acceptance.analysis.as_ref() {
        evidence = evidence.with_analysis(crate::canonical_scoring::AnalyzedFacts {
            analysis: analysis.clone(),
            actual_size_bytes: size_bytes,
            rule_file_doc: acceptance.rule_file_doc.clone(),
        });
    }

    let scored = crate::canonical_scoring::score_release(&evidence, &view);
    let score = scored.total;

    if !rescore_changes.is_empty() {
        tracing::debug!(
            title = %title.name,
            score,
            changes = ?rescore_changes,
            "mediainfo rescore applied to acquisition score"
        );
    }

    // Log the pass that set the number: with analysis present that is the
    // analyzed pass, otherwise the announced one is all there was.
    let logged = scored
        .analyzed_decision
        .as_ref()
        .unwrap_or(&scored.announced_decision);
    let scoring_log = serialize_post_download_scoring_log(logged, &rescore_changes);

    let tier_index = crate::quality_profile::quality_tier_index(
        &context.profile().criteria,
        scored.parsed_quality.as_deref(),
    );

    PostDownloadAcquisitionDecision {
        parsed: rescored,
        score,
        tier_index,
        revision: scored.revision,
        truth_verdict: scored.truth_verdict,
        scoring_log,
    }
}

fn serialize_post_download_scoring_log(
    decision: &crate::QualityProfileDecision,
    rescore_changes: &[String],
) -> Option<String> {
    let scoring_log = decision
        .scoring_log
        .iter()
        .map(|entry| {
            serde_json::json!({
                "code": entry.code,
                "delta": entry.delta,
                "source": scoring_source_json(&entry.source),
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&serde_json::json!({
        "kind": "post_download_acquisition_score",
        "release_score": decision.release_score,
        "preference_score": decision.preference_score,
        "allowed": decision.allowed,
        "block_codes": decision.block_codes,
        "rescore_changes": rescore_changes,
        "scoring_log": scoring_log,
    }))
    .ok()
}

fn scoring_source_json(source: &crate::ScoringSource) -> serde_json::Value {
    match source {
        crate::ScoringSource::Builtin => {
            serde_json::json!({"kind": "builtin"})
        }
        crate::ScoringSource::UserRule { id, name } => {
            serde_json::json!({"kind": "user_rule", "id": id, "name": name})
        }
        crate::ScoringSource::SystemRule { id, name } => {
            serde_json::json!({"kind": "system_rule", "id": id, "name": name})
        }
    }
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

#[expect(
    clippy::too_many_arguments,
    reason = "a refusal needs the actor, the title, the release name and path, the blocklist \
              attribution, the narrower reopen set and the rejection itself; bundling them would \
              only move the same eight facts into a struct every caller builds once"
)]
pub(crate) async fn reject_source_file_before_import(
    app: &AppUseCase,
    actor: impl Into<DomainEventActor>,
    title: &Title,
    completed_name: &str,
    path: &Path,
    attribution: BlocklistAttribution<'_>,
    // `Some` narrows the reopen to these episode scopes while the blocklist row
    // keeps the full `attribution`: a pack that imported some members and
    // refused others names the whole release on the row but may only reopen
    // the refused members — the imported ones are already marked completed.
    // `None` reopens whatever the attribution names.
    reopen_episode_ids: Option<&[String]>,
    rejection: &ImportedFileRejection,
) {
    finalize_import_rejection(
        app,
        actor,
        title,
        completed_name,
        path,
        attribution,
        reopen_episode_ids,
        rejection,
    )
    .await;
}

#[expect(
    clippy::too_many_arguments,
    reason = "mirrors `reject_source_file_before_import`, which is its only caller"
)]
async fn finalize_import_rejection(
    app: &AppUseCase,
    actor: impl Into<DomainEventActor>,
    title: &Title,
    completed_name: &str,
    path: &Path,
    attribution: BlocklistAttribution<'_>,
    reopen_episode_ids: Option<&[String]>,
    rejection: &ImportedFileRejection,
) {
    let episode_ids = attribution.episode_ids;
    let normalized_source_title = normalize_release_name(Some(completed_name));
    let failure_reason = Some(rejection.message.clone());
    let _ = app
        .services
        .workflow
        .release_attempts
        .record_release_attempt(
            Some(title.id.clone()),
            normalize_release_attempt_hint(None),
            normalized_source_title.clone(),
            ReleaseDownloadAttemptOutcome::Failed,
            failure_reason,
            None,
        )
        .await;

    let reason = Some(format!(
        "{}{}",
        rejection.message,
        if rejection.blocking_rule_codes.is_empty() {
            String::new()
        } else {
            format!(" [{}]", rejection.blocking_rule_codes.join(", "))
        }
    ));
    blocklist_release_for_title(app, title, completed_name, reason.clone()).await;

    // Re-open the refused scopes under their existing coverage: the cursor
    // walks each scope's saved search results before it would spend an indexer
    // query (the burned release is excluded by the blocklist row written above),
    // and a scope whose saved results are exhausted stays converged.
    let coverage = CoverageReopen::Keep;
    match reopen_episode_ids {
        // A pack reopens only the members that were refused; the row written
        // above is still attributed to every member the download covered.
        Some(episode_ids) => {
            reset_scopes_for_retry(
                app,
                &title.id,
                BlocklistAttribution {
                    episode_ids,
                    collection_id: None,
                    series_movie_link_id: None,
                },
                &coverage,
            )
            .await;
        }
        None => reset_scopes_for_retry(app, &title.id, attribution, &coverage).await,
    }
    let _ = app
        .append_domain_event(new_title_domain_event(
            actor,
            title,
            DomainEventPayload::ImportRejected(ImportRejectedEventData {
                title: Some(title_context_snapshot(title)),
                status: ImportStatus::Skipped,
                import_id: None,
                source_system: None,
                source_ref: None,
                source_title: Some(completed_name.to_string()),
                source_path: Some(path.display().to_string()),
                dest_path: None,
                quality: None,
                reason,
                // The rejection's own reason: a truth-verdict blocklist and a
                // quality lie are not rule blocks, and D17 makes the reason
                // load-bearing for whoever reads the event.
                skip_reason: rejection.skip_reason.clone(),
                episode_ids: episode_ids.to_vec(),
            }),
        ))
        .await;
}

/// Re-open the scopes a rejected import belongs to, so convergence looks for a
/// different candidate.
///
/// Only the `Blocklist` disposition reaches here (see
/// [`crate::import::decide::RejectionDisposition`]): the release has been burned
/// for this title, so the re-opened search seeks a *different* candidate rather
/// than re-grabbing the same lie. `Skip` and `Hold` deliberately do not reopen —
/// nothing about the scope changed and the next search would fetch the same
/// class of file.
///
/// **Which** scopes is read from the same [`BlocklistAttribution`] the blocklist
/// entry is filed under, so the row that is reopened and the row the operator
/// sees the blocklist against are the same row. Before this, a rejection with no
/// episode ids always reopened the *title* scope: correct for a movie, a no-op
/// for a series-movie link (whose scope row carries a link id), and wrong for a
/// season pack (whose members are episode scopes). Both gaps were live — a
/// refused link import left the link permanently un-searched.
///
/// `coverage` is the convergence-coverage invalidation applied to each re-opened
/// scope: only the indexer the burned release came from (so just that indexer is
/// re-queried), or the whole scope when the grab cannot be attributed.
async fn reset_scopes_for_retry(
    app: &AppUseCase,
    title_id: &str,
    attribution: BlocklistAttribution<'_>,
    coverage: &CoverageReopen,
) {
    if !attribution.episode_ids.is_empty() {
        reopen_episode_scopes(app, title_id, attribution.episode_ids, coverage).await;
        return;
    }

    if let Some(series_movie_link_id) = attribution.series_movie_link_id {
        reopen_series_movie_link_scope(app, title_id, series_movie_link_id, coverage).await;
        return;
    }

    if let Some(collection_id) = attribution.collection_id {
        match app
            .services
            .catalog
            .shows
            .list_episodes_for_collection(collection_id)
            .await
        {
            Ok(episodes) => {
                let episode_ids: Vec<String> = episodes
                    .into_iter()
                    .filter(|episode| episode.title_id == title_id)
                    .map(|episode| episode.id)
                    .collect();
                reopen_episode_scopes(app, title_id, &episode_ids, coverage).await;
            }
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title_id,
                    collection_id,
                    "failed to resolve collection members while re-opening scopes"
                );
            }
        }
        return;
    }

    reopen_episode_scope(app, title_id, None, coverage).await;
}

async fn reopen_episode_scopes(
    app: &AppUseCase,
    title_id: &str,
    episode_ids: &[String],
    coverage: &CoverageReopen,
) {
    let mut seen = HashSet::new();
    for episode_id in episode_ids {
        if !seen.insert(episode_id.as_str()) {
            continue;
        }
        reopen_episode_scope(app, title_id, Some(episode_id.as_str()), coverage).await;
    }
}

async fn reopen_episode_scope(
    app: &AppUseCase,
    title_id: &str,
    episode_id: Option<&str>,
    coverage: &CoverageReopen,
) {
    match app
        .services
        .workflow
        .acquisition_scope_states
        .get_acquisition_scope_state_for_title(title_id, episode_id)
        .await
    {
        Ok(Some(item)) => {
            app.reopen_wanted_scope_for_acquisition(&item, coverage.clone())
                .await;
        }
        Ok(None) => {}
        Err(error) => {
            warn!(error = %error, title_id = %title_id, "failed to reset wanted item")
        }
    }
}

/// A link scope has no episode id of its own, so it can only be found by
/// listing the title's series-movie scope rows and matching the link.
///
/// `limit: i64::MAX` on purpose: the paged default of 100 silently truncates a
/// title with many linked movies, and a truncated page here is a scope that is
/// never searched again. No `statuses` filter either — the row this rejection
/// belongs to is whatever state the grab left it in (`grabbed`, not `wanted`),
/// and the per-episode lookup above does not filter on status either.
async fn reopen_series_movie_link_scope(
    app: &AppUseCase,
    title_id: &str,
    series_movie_link_id: &str,
    coverage: &CoverageReopen,
) {
    match app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states(crate::AcquisitionScopeStatesQuery {
            media_types: vec!["series_movie".into()],
            title_id: Some(title_id.to_string()),
            limit: i64::MAX,
            ..crate::AcquisitionScopeStatesQuery::default()
        })
        .await
    {
        Ok(items) => {
            for item in items {
                if item.series_movie_link_id.as_deref() == Some(series_movie_link_id) {
                    app.reopen_wanted_scope_for_acquisition(&item, coverage.clone())
                        .await;
                    return;
                }
            }
        }
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title_id,
                series_movie_link_id,
                "failed to look up series-movie link scope while re-opening it"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn automatic(expected_runtime_seconds: Option<i32>) -> RuntimeSampleValidation {
        RuntimeSampleValidation::automatic(expected_runtime_seconds)
    }

    fn manual(expected_runtime_seconds: Option<i32>) -> RuntimeSampleValidation {
        RuntimeSampleValidation::manual_override(expected_runtime_seconds)
    }

    // The title-facet guide-fact derivation formerly asserted here against
    // `contextualize_import_release` now happens once, in the canonical import
    // parse (`import_workflow::parse_import_release_for_title`); see
    // `canonical_import_parse_derives_title_facet_guide_facts` in
    // `import/app_usecase_import_tests.rs`.

    fn criteria(tiers: &[&str]) -> crate::QualityProfileCriteria {
        let tiers = tiers
            .iter()
            .map(|tier| format!("\"{tier}\""))
            .collect::<Vec<_>>()
            .join(",");
        crate::QualityProfile::parse(&format!(
            r#"{{"id":"t","name":"T","criteria":{{"quality_tiers":[{tiers}],"allow_upgrades":true}}}}"#
        ))
        .expect("profile fixture should parse")
        .criteria
    }

    #[cfg(feature = "runtime-media-analysis")]
    #[test]
    fn resolved_import_profile_exposes_concrete_audio_to_post_download_rules() {
        let profile = crate::QualityProfile::parse(
            r#"{"id":"p","name":"P","criteria":{"quality_tiers":["1080P"]}}"#,
        )
        .expect("profile fixture should parse");
        let resolved = resolved_import_profile(
            &profile,
            &["jpn".to_string()],
            &crate::ScoringPersona::default(),
        );
        let parsed = crate::parse_release_metadata("Example Title 1080p");
        let decision = build_import_profile_decision(
            &resolved,
            &parsed,
            "movie",
            crate::quality_profile::CoverageSizeBasis::default(),
            None,
            false,
        );
        let input = crate::user_rule_input::build_rule_input(
            &parsed,
            &resolved,
            &decision,
            crate::user_rule_input::ReleaseRuntimeInfo {
                size_bytes: None,
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: None,
            },
            crate::user_rule_input::RuleContextInfo {
                title_id: None,
                library_name: None,
                category: Some("movie"),
                original_language: Some("jpn"),
                original_country: None,
                title_tags: &[],
                has_existing_file: false,
                existing_score: None,
                search_mode: "post_download",
                runtime_minutes: None,
                is_filler: false,
            },
            None,
        );

        assert_eq!(input.profile.required_audio_languages, vec!["jpn"]);
        assert_eq!(input.release.languages_audio, vec!["jpn"]);
    }

    /// The tier comparison behind a `quality_contradicted:` blocklist. It reads
    /// the *profile's* ordering, not a global one, so its edge cases are the
    /// profile's: an unlisted quality, a single-tier profile, and the casing the
    /// verdict code happens to carry.
    #[test]
    fn landed_tier_is_worse_only_when_the_profile_ranks_it_lower() {
        let c = criteria(&["2160P", "1080P", "720P"]);
        assert!(landed_tier_is_worse(&c, "1080p", "720p"));
        assert!(!landed_tier_is_worse(&c, "720p", "1080p"));
        assert!(!landed_tier_is_worse(&c, "1080p", "1080p"));
    }

    /// The code the scorer emits is already normalized (`1080P`), but nothing
    /// stops a stored or hand-written one carrying the lowercase form. Both must
    /// read as the same tier — treating `1080p` and `1080P` as different would
    /// make an identical-quality import look like a downgrade and blocklist a
    /// release that told the truth.
    #[test]
    fn landed_tier_is_worse_is_case_insensitive_about_the_resolution_suffix() {
        let c = criteria(&["2160P", "1080P"]);
        assert!(!landed_tier_is_worse(&c, "1080P", "1080p"));
        assert!(!landed_tier_is_worse(&c, "1080p", "1080P"));
    }

    /// A single-tier profile ranks exactly one quality. Anything else the file
    /// turns out to be is outside the profile, which is a downgrade — and the
    /// reverse (announced unranked, landed ranked) is not, because there was
    /// never a claim to fall short of.
    #[test]
    fn landed_tier_is_worse_handles_a_single_tier_profile() {
        let c = criteria(&["1080P"]);
        assert!(
            landed_tier_is_worse(&c, "1080p", "720p"),
            "landing outside the only tier the profile ranks is a downgrade"
        );
        assert!(
            !landed_tier_is_worse(&c, "720p", "1080p"),
            "an unranked announcement cannot be fallen short of"
        );
        assert!(!landed_tier_is_worse(&c, "1080p", "1080p"));
    }

    /// An empty tier list ranks nothing, so nothing can be a tier downgrade —
    /// and `quality_not_in_profile_tiers` cannot fire either. The verdict falls
    /// through to the honest size/score comparison instead of burning releases
    /// against an ordering the operator never set.
    #[test]
    fn landed_tier_is_worse_is_never_true_without_a_tier_list() {
        let c = criteria(&[]);
        assert!(!landed_tier_is_worse(&c, "1080p", "480p"));
    }

    #[test]
    fn automatic_movie_import_rejects_twenty_second_runtime_for_normal_movie() {
        let rejection = runtime_sample_rejection(automatic(Some(90 * 60)), Some(20))
            .expect("short normal-runtime movie should reject");

        assert_eq!(rejection.recycle_reason, SAMPLE_RUNTIME_TOO_SHORT_CODE);
        assert_eq!(
            rejection.skip_reason,
            Some(ImportSkipReason::PolicyMismatch)
        );
        assert_eq!(
            rejection.blocking_rule_codes,
            vec![SAMPLE_RUNTIME_TOO_SHORT_CODE.to_string()]
        );
    }

    #[test]
    fn automatic_episode_import_rejects_twenty_second_runtime_for_normal_episode() {
        let rejection = runtime_sample_rejection(automatic(Some(42 * 60)), Some(20))
            .expect("short normal-runtime episode should reject");

        assert_eq!(rejection.recycle_reason, SAMPLE_RUNTIME_TOO_SHORT_CODE);
    }

    #[test]
    fn automatic_import_accepts_short_form_movie_above_fixture_runtime_floor() {
        let rejection = runtime_sample_rejection(automatic(Some(3 * 60)), Some(180));

        assert!(rejection.is_none());
    }

    #[test]
    fn automatic_import_rejects_unknown_positive_runtime_under_one_minute() {
        let rejection = runtime_sample_rejection(automatic(None), Some(59))
            .expect("unknown-runtime short clip should reject");

        assert_eq!(rejection.recycle_reason, SAMPLE_RUNTIME_TOO_SHORT_CODE);
    }

    #[test]
    fn automatic_import_rejects_zero_runtime() {
        let rejection = runtime_sample_rejection(automatic(Some(42 * 60)), Some(0))
            .expect("zero runtime should reject");

        assert_eq!(rejection.recycle_reason, SAMPLE_RUNTIME_ZERO_CODE);
    }

    #[test]
    fn automatic_import_rejects_indeterminate_runtime() {
        let rejection = runtime_sample_rejection(automatic(Some(42 * 60)), None)
            .expect("indeterminate runtime should reject");

        assert_eq!(rejection.recycle_reason, SAMPLE_RUNTIME_INDETERMINATE_CODE);
        assert_eq!(
            rejection.message,
            "video file could not be read to verify its duration; it may be incomplete or corrupted. Download it again and retry the import"
        );
    }

    #[test]
    fn manual_queued_import_bypasses_runtime_sample_rejection() {
        let rejection = runtime_sample_rejection(manual(Some(42 * 60)), Some(20));

        assert!(rejection.is_none());
    }

    #[test]
    fn automatic_import_rejects_anime_length_file_for_live_action_episode() {
        // Tide Chart incident: a 24:55 anime episode imported against the ~55-minute
        // live-action episode. Well clear of the sample threshold, far below the band.
        let rejection = runtime_sample_rejection(automatic(Some(3300)), Some(1495))
            .expect("45%-of-expected runtime should reject");

        assert_eq!(rejection.recycle_reason, RUNTIME_OUT_OF_BAND_CODE);
        assert_eq!(
            rejection.skip_reason,
            Some(ImportSkipReason::PolicyMismatch)
        );
        assert_eq!(
            rejection.blocking_rule_codes,
            vec![RUNTIME_OUT_OF_BAND_CODE.to_string()]
        );
        assert_eq!(
            rejection.message,
            "imported file runtime is too short for the expected runtime: expected about 55 minutes, probed file is 1495 seconds"
        );
    }

    #[test]
    fn automatic_import_rejects_hour_long_file_for_anime_episode() {
        // Inverse of the incident: a 60-minute file against a 24-minute episode.
        let rejection = runtime_sample_rejection(automatic(Some(1440)), Some(3600))
            .expect("250%-of-expected runtime should reject");

        assert_eq!(rejection.recycle_reason, RUNTIME_OUT_OF_BAND_CODE);
        assert_eq!(
            rejection.message,
            "imported file runtime is too long for the expected runtime: expected about 24 minutes, probed file is 3600 seconds"
        );
    }

    #[test]
    fn automatic_import_accepts_double_episode_file_within_band() {
        // S09E23E24: the expectation is already summed across both episodes, so a
        // ~190% file stays inside the band.
        let rejection = runtime_sample_rejection(automatic(Some(2640)), Some(5040));

        assert!(rejection.is_none());
    }

    #[test]
    fn automatic_import_accepts_exact_runtime_band_endpoints() {
        // Band endpoints are inclusive: exactly 50% and exactly 200% pass.
        assert!(runtime_sample_rejection(automatic(Some(3300)), Some(1650)).is_none());
        assert!(runtime_sample_rejection(automatic(Some(3300)), Some(6600)).is_none());
    }

    #[test]
    fn automatic_import_skips_runtime_band_when_expected_runtime_is_unknown() {
        assert!(runtime_sample_rejection(automatic(None), Some(3600)).is_none());
        assert!(runtime_sample_rejection(automatic(None), Some(120)).is_none());
    }

    #[test]
    fn automatic_import_skips_runtime_band_below_expected_runtime_floor() {
        // Expected runtimes under the 5-minute floor stay permissive.
        assert!(runtime_sample_rejection(automatic(Some(240)), Some(30)).is_none());
    }

    #[test]
    fn manual_queued_import_bypasses_runtime_band() {
        assert!(runtime_sample_rejection(manual(Some(3300)), Some(1495)).is_none());
        assert!(runtime_sample_rejection(manual(Some(1440)), Some(3600)).is_none());
    }

    fn probed(duration_seconds: Option<i32>) -> ImportedFileAcceptance {
        let mut analysis = build_stream_pointer_media_file_analysis();
        analysis.duration_seconds = duration_seconds;
        ImportedFileAcceptance {
            analysis: Some(analysis),
            scan_error: None,
            rule_file_doc: None,
            audio_language_warning: None,
        }
    }

    fn unprobed() -> ImportedFileAcceptance {
        ImportedFileAcceptance {
            analysis: None,
            scan_error: Some("native media analysis is not compiled into this target".to_string()),
            rule_file_doc: None,
            audio_language_warning: None,
        }
    }

    #[test]
    fn replace_guard_blocks_anime_length_file_over_live_action_incumbent() {
        // Tide Chart incident with no catalog runtime: the band cannot run against
        // metadata, so the 24:55 file is measured against the ~55-minute file it
        // would overwrite.
        let message = replace_runtime_band_block(
            automatic(None),
            &probed(Some(1495)),
            incumbent_replace_runtime_seconds([Some(3300)]),
        )
        .expect("45%-of-incumbent runtime should be held");

        assert_eq!(
            message,
            "imported file runtime is too short to replace the existing file: existing file is about 55 minutes, probed file is 1495 seconds; held for manual resolution"
        );
    }

    #[test]
    fn replace_guard_blocks_hour_long_file_over_anime_incumbent() {
        let message = replace_runtime_band_block(
            automatic(None),
            &probed(Some(3600)),
            incumbent_replace_runtime_seconds([Some(1440)]),
        )
        .expect("250%-of-incumbent runtime should be held");

        assert_eq!(
            message,
            "imported file runtime is too long to replace the existing file: existing file is about 24 minutes, probed file is 3600 seconds; held for manual resolution"
        );
    }

    #[test]
    fn replace_guard_is_permissive_when_incumbent_duration_is_unknown() {
        assert!(
            replace_runtime_band_block(
                automatic(None),
                &probed(Some(1495)),
                incumbent_replace_runtime_seconds([None]),
            )
            .is_none()
        );
        assert!(
            replace_runtime_band_block(
                automatic(None),
                &probed(Some(1495)),
                incumbent_replace_runtime_seconds([Some(0)]),
            )
            .is_none()
        );
    }

    #[test]
    fn replace_guard_is_permissive_without_a_probed_duration() {
        assert!(
            replace_runtime_band_block(
                automatic(None),
                &probed(None),
                incumbent_replace_runtime_seconds([Some(3300)]),
            )
            .is_none()
        );
        assert!(
            replace_runtime_band_block(
                automatic(None),
                &unprobed(),
                incumbent_replace_runtime_seconds([Some(3300)]),
            )
            .is_none()
        );
    }

    #[test]
    fn replace_guard_leaves_gap_fill_imports_alone() {
        // No incumbent files at all: nothing is being overwritten.
        assert!(
            replace_runtime_band_block(
                automatic(None),
                &probed(Some(1495)),
                incumbent_replace_runtime_seconds(std::iter::empty()),
            )
            .is_none()
        );
        assert!(
            replace_runtime_band_block(
                automatic(None),
                &probed(Some(30)),
                incumbent_replace_runtime_seconds(std::iter::empty()),
            )
            .is_none()
        );
    }

    #[test]
    fn replace_guard_defers_to_the_expected_runtime_band() {
        // With an expected runtime the gate already applied the band; the replace
        // guard must not double-judge the same file against the incumbent.
        assert!(
            replace_runtime_band_block(
                automatic(Some(1440)),
                &probed(Some(1495)),
                incumbent_replace_runtime_seconds([Some(3300)]),
            )
            .is_none()
        );
    }

    #[test]
    fn manual_replacement_bypasses_the_replace_guard() {
        assert!(
            replace_runtime_band_block(
                manual(None),
                &probed(Some(1495)),
                incumbent_replace_runtime_seconds([Some(3300)]),
            )
            .is_none()
        );
    }

    #[test]
    fn replace_guard_band_endpoints_are_inclusive() {
        assert!(
            replace_runtime_band_block(
                automatic(None),
                &probed(Some(1650)),
                incumbent_replace_runtime_seconds([Some(3300)]),
            )
            .is_none()
        );
        assert!(
            replace_runtime_band_block(
                automatic(None),
                &probed(Some(6600)),
                incumbent_replace_runtime_seconds([Some(3300)]),
            )
            .is_none()
        );
    }

    #[test]
    fn incumbent_replace_runtime_sums_multi_file_targets_only_when_all_are_known() {
        assert_eq!(
            incumbent_replace_runtime_seconds([Some(1320), Some(1320)]),
            Some(2640)
        );
        assert_eq!(
            incumbent_replace_runtime_seconds([Some(1320), None]),
            None,
            "a partially scanned target must leave the guard inert"
        );
        assert_eq!(incumbent_replace_runtime_seconds(std::iter::empty()), None);
    }

    #[test]
    fn replace_guard_compares_double_episode_file_against_summed_incumbents() {
        // Two single-episode incumbents replaced by one double-episode file.
        assert!(
            replace_runtime_band_block(
                automatic(None),
                &probed(Some(2600)),
                incumbent_replace_runtime_seconds([Some(1320), Some(1320)]),
            )
            .is_none()
        );
    }

    // ── Truth verdicts become import actions (D2) ─────────────────────────────

    use crate::canonical_scoring::TruthVerdict;

    fn tiered_criteria() -> crate::QualityProfileCriteria {
        crate::QualityProfileCriteria {
            quality_tiers: vec!["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
            ..Default::default()
        }
    }

    fn codes(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn a_consistent_verdict_just_imports() {
        assert!(matches!(
            resolve_truth_verdict_action_for_origin(
                &TruthVerdict::Consistent,
                &tiered_criteria(),
                true,
                crate::import_decide::ImportOrigin::Automatic,
            ),
            TruthVerdictAction::Import
        ));
    }

    /// The analyzed pass vetoed the file. No scope takes it, occupied or not,
    /// and the release is blocklisted so the reopened search seeks another one.
    #[test]
    fn a_blocked_verdict_rejects_whether_or_not_the_scope_is_occupied() {
        for occupied in [true, false] {
            let action = resolve_truth_verdict_action_for_origin(
                &TruthVerdict::Blocked {
                    codes: codes(&["required_audio_missing"]),
                },
                &tiered_criteria(),
                occupied,
                crate::import_decide::ImportOrigin::Automatic,
            );
            let TruthVerdictAction::Reject(rejection) = action else {
                panic!("a hard block must refuse the import (occupied = {occupied})");
            };
            assert_eq!(rejection.recycle_reason, TRUTH_BLOCKED_CODE);
            assert_eq!(
                rejection.skip_reason,
                Some(ImportSkipReason::PolicyMismatch)
            );
            assert!(rejection.message.contains("required_audio_missing"));
        }
    }

    #[test]
    fn an_operator_queued_guard_failure_is_rejected_for_manual_import() {
        let action = resolve_truth_verdict_action_for_origin(
            &TruthVerdict::Blocked {
                codes: codes(&["required_audio_missing"]),
            },
            &tiered_criteria(),
            false,
            crate::import_decide::ImportOrigin::OperatorQueued,
        );
        assert!(matches!(action, TruthVerdictAction::Reject(_)));

        let quality_lie = resolve_truth_verdict_action_for_origin(
            &TruthVerdict::Contradicted {
                codes: codes(&["quality_contradicted:1080P->720P"]),
            },
            &tiered_criteria(),
            false,
            crate::import_decide::ImportOrigin::OperatorQueued,
        );
        assert!(matches!(quality_lie, TruthVerdictAction::Reject(_)));
    }

    /// Advertised 1080p, landed 720p, and something already occupies the scope:
    /// the tier gate would refuse it anyway, so name the reason and stop the
    /// release from being offered again.
    #[test]
    fn a_quality_lie_into_an_occupied_scope_is_rejected() {
        let action = resolve_truth_verdict_action_for_origin(
            &TruthVerdict::Contradicted {
                codes: codes(&["size_expected", "quality_contradicted:1080P->720P"]),
            },
            &tiered_criteria(),
            true,
            crate::import_decide::ImportOrigin::Automatic,
        );
        let TruthVerdictAction::Reject(rejection) = action else {
            panic!("a landed tier below the announced one must not overwrite a file");
        };
        assert_eq!(rejection.recycle_reason, TRUTH_QUALITY_DOWNGRADE_CODE);
    }

    /// Same lie into an empty scope: an honest 720p beats no file at all, so it
    /// lands — but the release is blocklisted so a later upgrade search cannot
    /// re-grab it and "upgrade" the scope to the tier it already has.
    #[test]
    fn a_quality_lie_into_an_empty_scope_imports_and_blocklists() {
        let action = resolve_truth_verdict_action_for_origin(
            &TruthVerdict::Contradicted {
                codes: codes(&["quality_contradicted:1080P->720P"]),
            },
            &tiered_criteria(),
            false,
            crate::import_decide::ImportOrigin::Automatic,
        );
        let TruthVerdictAction::ImportAndBlocklist { code, reason } = action else {
            panic!("an empty scope keeps an honest file at its real tier");
        };
        assert_eq!(code, TRUTH_QUALITY_DOWNGRADE_CODE);
        assert!(reason.contains("1080P"));
        assert!(reason.contains("720P"));
    }

    /// One size bucket of drift is the ordinary usenet case (par2/RAR overhead,
    /// a short episode). Treating it as a lie would burn good releases.
    #[test]
    fn a_size_only_contradiction_imports_normally() {
        for occupied in [true, false] {
            assert!(
                matches!(
                    resolve_truth_verdict_action_for_origin(
                        &TruthVerdict::Contradicted {
                            codes: codes(&["size_slightly_small", "bitrate_low"]),
                        },
                        &tiered_criteria(),
                        occupied,
                        crate::import_decide::ImportOrigin::Automatic,
                    ),
                    TruthVerdictAction::Import
                ),
                "a score-only contradiction must never blocklist (occupied = {occupied})"
            );
        }
    }

    /// The other direction is not a lie worth punishing: the file turned out
    /// *better* than advertised.
    #[test]
    fn a_quality_contradiction_that_landed_better_imports_normally() {
        assert!(matches!(
            resolve_truth_verdict_action_for_origin(
                &TruthVerdict::Contradicted {
                    codes: codes(&["quality_contradicted:720P->1080P"]),
                },
                &tiered_criteria(),
                true,
                crate::import_decide::ImportOrigin::Automatic,
            ),
            TruthVerdictAction::Import
        ));
    }

    /// A landed quality the profile does not rank at all is below every quality
    /// it does, matching the admission gate's tier comparison.
    #[test]
    fn landing_outside_the_profiles_tiers_counts_as_a_downgrade() {
        assert!(landed_tier_is_worse(&tiered_criteria(), "1080P", "480P"));
        assert!(landed_tier_is_worse(&tiered_criteria(), "1080P", "unknown"));
        // An unranked announcement cannot be downgraded from.
        assert!(!landed_tier_is_worse(&tiered_criteria(), "unknown", "480P"));
    }

    #[test]
    fn only_the_quality_contradiction_code_is_parsed_as_a_tier_change() {
        assert_eq!(
            quality_contradiction(&codes(&["quality_contradicted:1080P->720P"])),
            Some(("1080P", "720P"))
        );
        assert_eq!(quality_contradiction(&codes(&["size_very_small"])), None);
        // A malformed code is data, not a panic.
        assert_eq!(
            quality_contradiction(&codes(&["quality_contradicted:garbage"])),
            None
        );
    }
}
