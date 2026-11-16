//! Subtitle timing synchronization using an optional plugin-owned sync engine.
//!
//! The sync pipeline:
//! 1. Applies policy gates that do not require media analysis.
//! 2. Reads subtitle bytes and detects the rewrite format.
//! 3. Sends stored media metadata hints, then delegates audio/VAD timing and rewriting to the plugin.
//! 4. Atomically applies the rewritten subtitle bytes returned by the plugin.

use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    AppError, AppResult,
    disk::{Access, Disk, Replace},
    ports::{SubtitleSyncClient, SubtitleSyncJob, SubtitleSyncReferenceSubtitle},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scryer_plugin_sdk::{
    SubtitleSyncAlignResponse, SubtitleSyncAlignSkipReason, SubtitleSyncAudioCodec,
    SubtitleSyncMediaMetadataSnapshot, SubtitleSyncOptions,
};

/// Result of a subtitle sync operation.
#[derive(Debug, Clone)]
pub struct SyncResult {
    /// Time offset applied in milliseconds.
    pub offset_ms: i64,
    /// Whether the sync was applied.
    pub applied: bool,
    /// Detected subtitle format when one was recognized.
    pub format: Option<SubtitleTimingFormat>,
    /// Alignment consistency across split deltas.
    pub consistency_ratio: Option<f64>,
    /// Constant-offset alignment score.
    pub nosplit_score: Option<f64>,
    /// Split alignment score.
    pub split_score: Option<f64>,
    /// Why sync was skipped when `applied` is false.
    pub skipped_reason: Option<SyncSkipReason>,
}

pub const ENHANCED_SUBTITLE_SYNC_PLUGIN_ID: &str = "enhanced-subtitle-sync";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleTimingFormat {
    Srt,
    Vtt,
    Ass,
}

impl SubtitleTimingFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Ass => "ass/ssa",
        }
    }

    fn sdk_format(self, path: &Path) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Ass => {
                if path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("ssa"))
                {
                    "ssa"
                } else {
                    "ass"
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncSkipReason {
    Disabled,
    ForcedSubtitle,
    ScoreAboveThreshold,
    SubtitleSyncPluginRequired,
    UnsupportedSubtitleFormat,
    AudioDecodeFailed,
    NotEnoughReferenceSpans,
    WeakAlignment,
    LowAlignmentConsistency,
    OffsetExceedsMaximum,
    OffsetTooSmall,
}

impl SyncSkipReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::ForcedSubtitle => "forced_subtitle",
            Self::ScoreAboveThreshold => "score_above_threshold",
            Self::SubtitleSyncPluginRequired => "subtitle_sync_plugin_required",
            Self::UnsupportedSubtitleFormat => "unsupported_subtitle_format",
            Self::AudioDecodeFailed => "audio_decode_failed",
            Self::NotEnoughReferenceSpans => "not_enough_reference_spans",
            Self::WeakAlignment => "weak_alignment",
            Self::LowAlignmentConsistency => "low_alignment_consistency",
            Self::OffsetExceedsMaximum => "offset_exceeds_maximum",
            Self::OffsetTooSmall => "offset_too_small",
        }
    }
}

impl fmt::Display for SyncSkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SyncPolicy {
    pub enabled: bool,
    pub forced: bool,
    pub score: Option<i32>,
    pub threshold: Option<i32>,
    pub max_offset_seconds: i64,
}

impl SyncPolicy {
    fn skip_reason(self) -> Option<SyncSkipReason> {
        if !self.enabled {
            return Some(SyncSkipReason::Disabled);
        }

        if self.forced {
            return Some(SyncSkipReason::ForcedSubtitle);
        }

        if let (Some(score), Some(threshold)) = (self.score, self.threshold)
            && score > threshold
        {
            return Some(SyncSkipReason::ScoreAboveThreshold);
        }

        None
    }
}

impl SyncResult {
    pub fn summary(&self) -> String {
        if self.applied {
            format!("applied {}ms offset", self.offset_ms)
        } else if let Some(reason) = self.skipped_reason {
            format!("skipped ({reason})")
        } else {
            "skipped".to_string()
        }
    }
}

/// Synchronize a subtitle file with a video file's audio track using a Bazarr-style policy gate.
pub async fn sync_subtitle_with_policy(
    video_path: &Path,
    subtitle_path: &Path,
    policy: SyncPolicy,
) -> AppResult<SyncResult> {
    sync_subtitle_with_policy_and_plugin_sync(
        video_path,
        subtitle_path,
        policy,
        None,
        false,
        None,
        None,
    )
    .await
}

pub async fn sync_subtitle_with_policy_and_plugin_sync(
    video_path: &Path,
    subtitle_path: &Path,
    policy: SyncPolicy,
    subtitle_sync_client: Option<Arc<dyn SubtitleSyncClient>>,
    plugin_installed: bool,
    reference_subtitle_path: Option<&Path>,
    media_metadata: Option<SubtitleSyncMediaMetadataSnapshot>,
) -> AppResult<SyncResult> {
    if let Some(reason) = policy.skip_reason() {
        tracing::debug!(
            path = %subtitle_path.display(),
            score = policy.score,
            threshold = policy.threshold,
            reason = %reason,
            "subtitle sync skipped by policy"
        );
        return Ok(skipped_sync_result(0, None, None, None, None, reason));
    }

    sync_subtitle_with_plugin_sync(
        video_path,
        subtitle_path,
        policy.max_offset_seconds,
        subtitle_sync_client,
        plugin_installed,
        reference_subtitle_path,
        media_metadata,
    )
    .await
}

/// Synchronize a subtitle file with a video file's audio track.
pub async fn sync_subtitle(
    video_path: &Path,
    subtitle_path: &Path,
    max_offset_seconds: i64,
) -> AppResult<SyncResult> {
    sync_subtitle_with_plugin_sync(
        video_path,
        subtitle_path,
        max_offset_seconds,
        None,
        false,
        None,
        None,
    )
    .await
}

struct PreparedSubtitleSyncJob {
    subtitle_content: Vec<u8>,
    subtitle_format: SubtitleTimingFormat,
    subtitle_file_name: Option<String>,
    subtitle_encoding_hint: Option<String>,
    reference_subtitle: Option<SubtitleSyncReferenceSubtitle>,
    expected_codec: Option<SubtitleSyncAudioCodec>,
    media_metadata: Option<SubtitleSyncMediaMetadataSnapshot>,
}

pub async fn sync_subtitle_with_plugin_sync(
    video_path: &Path,
    subtitle_path: &Path,
    max_offset_seconds: i64,
    subtitle_sync_client: Option<Arc<dyn SubtitleSyncClient>>,
    plugin_installed: bool,
    reference_subtitle_path: Option<&Path>,
    media_metadata: Option<SubtitleSyncMediaMetadataSnapshot>,
) -> AppResult<SyncResult> {
    let prepared = prepare_subtitle_sync_job_blocking(
        video_path.to_path_buf(),
        subtitle_path.to_path_buf(),
        reference_subtitle_path.map(Path::to_path_buf),
        media_metadata,
    )
    .await?;

    let Some(prepared) = prepared else {
        tracing::debug!(
            path = %subtitle_path.display(),
            "subtitle sync skipped: unsupported subtitle format"
        );
        return Ok(skipped_sync_result(
            0,
            None,
            None,
            None,
            None,
            SyncSkipReason::UnsupportedSubtitleFormat,
        ));
    };

    let subtitle_format = prepared.subtitle_format;
    let Some(subtitle_sync_client) = subtitle_sync_client else {
        tracing::warn!(
            path = %video_path.display(),
            plugin_id = ENHANCED_SUBTITLE_SYNC_PLUGIN_ID,
            "{}",
            if plugin_installed {
                missing_enhanced_sync_update_hint()
            } else {
                missing_enhanced_sync_install_hint()
            }
        );
        return Ok(skipped_sync_result(
            0,
            Some(subtitle_format),
            None,
            None,
            None,
            SyncSkipReason::SubtitleSyncPluginRequired,
        ));
    };

    let response = match subtitle_sync_client
        .align_subtitle(SubtitleSyncJob {
            input_path: video_path.to_path_buf(),
            subtitle_content: prepared.subtitle_content,
            subtitle_format: subtitle_format.sdk_format(subtitle_path).to_string(),
            subtitle_file_name: prepared.subtitle_file_name,
            subtitle_encoding_hint: prepared.subtitle_encoding_hint,
            reference_subtitle: prepared.reference_subtitle,
            max_offset_seconds,
            sync_options: SubtitleSyncOptions::default(),
            expected_codec: prepared.expected_codec,
            media_metadata: prepared.media_metadata,
        })
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                path = %video_path.display(),
                plugin_id = ENHANCED_SUBTITLE_SYNC_PLUGIN_ID,
                error = %error,
                "subtitle sync skipped: plugin execution failed"
            );
            return Ok(skipped_sync_result(
                0,
                Some(subtitle_format),
                None,
                None,
                None,
                SyncSkipReason::AudioDecodeFailed,
            ));
        }
    };

    if !response.warnings.is_empty() {
        tracing::debug!(
            path = %video_path.display(),
            backend = response.backend.as_str(),
            warnings = ?response.warnings,
            "subtitle sync plugin reported warnings"
        );
    }

    if !response.applied {
        return Ok(sync_result_from_plugin_response(subtitle_format, response));
    }

    let rewritten_subtitle = response.rewritten_subtitle.as_ref().ok_or_else(|| {
        AppError::Repository(
            "subtitle sync plugin reported applied without rewritten_subtitle".to_string(),
        )
    })?;
    let bytes = BASE64
        .decode(&rewritten_subtitle.content_base64)
        .map_err(|error| {
            AppError::Repository(format!(
                "subtitle sync plugin returned invalid rewritten subtitle base64: {error}"
            ))
        })?;
    // Written with owner-only access, as the rewritten file always has been.
    Disk::process()
        .write_atomic(subtitle_path, bytes, Replace::Allowed, Access::OwnerOnly)
        .await
        .map_err(|e| AppError::Repository(format!("cannot replace subtitle file: {e}")))?;

    tracing::info!(
        path = %subtitle_path.display(),
        format = subtitle_format.label(),
        backend = response.backend.as_str(),
        offset_ms = response.offset_ms,
        consistency_ratio = response.consistency_ratio,
        nosplit_score = response.nosplit_score,
        split_score = response.split_score,
        "subtitle synchronized"
    );
    Ok(SyncResult {
        offset_ms: response.offset_ms,
        applied: true,
        format: Some(subtitle_format),
        consistency_ratio: response.consistency_ratio,
        nosplit_score: response.nosplit_score,
        split_score: response.split_score,
        skipped_reason: None,
    })
}

async fn prepare_subtitle_sync_job_blocking(
    video_path: PathBuf,
    subtitle_path: PathBuf,
    reference_subtitle_path: Option<PathBuf>,
    media_metadata: Option<SubtitleSyncMediaMetadataSnapshot>,
) -> AppResult<Option<PreparedSubtitleSyncJob>> {
    tokio::task::spawn_blocking(move || {
        prepare_subtitle_sync_job(
            &video_path,
            &subtitle_path,
            reference_subtitle_path.as_deref(),
            media_metadata,
        )
    })
    .await
    .map_err(|error| {
        AppError::Repository(format!("subtitle sync preparation task failed: {error}"))
    })?
}

fn prepare_subtitle_sync_job(
    _video_path: &Path,
    subtitle_path: &Path,
    reference_subtitle_path: Option<&Path>,
    media_metadata: Option<SubtitleSyncMediaMetadataSnapshot>,
) -> AppResult<Option<PreparedSubtitleSyncJob>> {
    let subtitle_content = Disk::process()
        .read_blocking(subtitle_path)
        .map_err(|e| AppError::Repository(format!("cannot read subtitle file: {e}")))?;
    let Some(subtitle_format) = detect_subtitle_format(subtitle_path, &subtitle_content) else {
        return Ok(None);
    };

    let expected_codec = expected_codec_from_metadata(media_metadata.as_ref());

    Ok(Some(PreparedSubtitleSyncJob {
        subtitle_file_name: subtitle_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string),
        subtitle_encoding_hint: subtitle_encoding_hint(&subtitle_content),
        reference_subtitle: reference_subtitle_from_path(reference_subtitle_path, subtitle_path),
        expected_codec,
        media_metadata,
        subtitle_content,
        subtitle_format,
    }))
}

fn missing_enhanced_sync_install_hint() -> String {
    format!(
        "subtitle sync requires plugin '{}'; install and enable it to use subtitle sync",
        ENHANCED_SUBTITLE_SYNC_PLUGIN_ID,
    )
}

fn missing_enhanced_sync_update_hint() -> String {
    format!(
        "subtitle sync plugin '{}' is installed but too old; update it to restore subtitle sync support",
        ENHANCED_SUBTITLE_SYNC_PLUGIN_ID,
    )
}

fn sync_result_from_plugin_response(
    subtitle_format: SubtitleTimingFormat,
    response: SubtitleSyncAlignResponse,
) -> SyncResult {
    skipped_sync_result(
        response.offset_ms,
        Some(subtitle_format),
        response.consistency_ratio,
        response.nosplit_score,
        response.split_score,
        response
            .skipped_reason
            .map(map_plugin_skip_reason)
            .unwrap_or(SyncSkipReason::AudioDecodeFailed),
    )
}

fn map_plugin_skip_reason(reason: SubtitleSyncAlignSkipReason) -> SyncSkipReason {
    match reason {
        SubtitleSyncAlignSkipReason::AudioDecodeFailed => SyncSkipReason::AudioDecodeFailed,
        SubtitleSyncAlignSkipReason::NotEnoughReferenceSpans => {
            SyncSkipReason::NotEnoughReferenceSpans
        }
        SubtitleSyncAlignSkipReason::WeakAlignment => SyncSkipReason::WeakAlignment,
        SubtitleSyncAlignSkipReason::LowAlignmentConsistency => {
            SyncSkipReason::LowAlignmentConsistency
        }
        SubtitleSyncAlignSkipReason::OffsetExceedsMaximum => SyncSkipReason::OffsetExceedsMaximum,
        SubtitleSyncAlignSkipReason::OffsetTooSmall => SyncSkipReason::OffsetTooSmall,
    }
}

fn skipped_sync_result(
    offset_ms: i64,
    format: Option<SubtitleTimingFormat>,
    consistency_ratio: Option<f64>,
    nosplit_score: Option<f64>,
    split_score: Option<f64>,
    reason: SyncSkipReason,
) -> SyncResult {
    SyncResult {
        offset_ms,
        applied: false,
        format,
        consistency_ratio,
        nosplit_score,
        split_score,
        skipped_reason: Some(reason),
    }
}

fn expected_codec_from_metadata(
    metadata: Option<&SubtitleSyncMediaMetadataSnapshot>,
) -> Option<SubtitleSyncAudioCodec> {
    let metadata = metadata?;
    targeted_audio_codec(
        metadata.audio_codec.as_deref(),
        metadata.audio_profile.as_deref(),
    )
}

fn targeted_audio_codec(
    codec: Option<&str>,
    profile: Option<&str>,
) -> Option<SubtitleSyncAudioCodec> {
    let normalized = codec?.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "ac3" => Some(SubtitleSyncAudioCodec::Ac3),
        "eac3" => Some(SubtitleSyncAudioCodec::Eac3),
        "truehd" | "mlp" => Some(SubtitleSyncAudioCodec::TrueHd),
        "dts" => {
            if profile
                .map(|profile| profile.to_ascii_lowercase().contains("dts-hd ma"))
                .unwrap_or(false)
            {
                Some(SubtitleSyncAudioCodec::DtsHdMaCore)
            } else {
                Some(SubtitleSyncAudioCodec::Dts)
            }
        }
        _ => None,
    }
}

fn subtitle_encoding_hint(bytes: &[u8]) -> Option<String> {
    if std::str::from_utf8(bytes).is_ok() {
        return Some("utf-8".to_string());
    }

    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    Some(
        detector
            .guess(None, chardetng::Utf8Detection::Allow)
            .name()
            .to_ascii_lowercase(),
    )
}

fn detect_subtitle_format(path: &Path, content: &[u8]) -> Option<SubtitleTimingFormat> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("srt") => return Some(SubtitleTimingFormat::Srt),
        Some("vtt") => return Some(SubtitleTimingFormat::Vtt),
        Some("ass") | Some("ssa") => return Some(SubtitleTimingFormat::Ass),
        _ => {}
    }

    if contains_ascii_case_insensitive(content, b"WEBVTT") {
        return Some(SubtitleTimingFormat::Vtt);
    }
    if content.windows(3).any(|window| window == b"-->") {
        return Some(SubtitleTimingFormat::Srt);
    }
    if contains_ascii_case_insensitive(content, b"[Events]")
        || contains_ascii_case_insensitive(content, b"Dialogue:")
    {
        return Some(SubtitleTimingFormat::Ass);
    }

    None
}

fn reference_subtitle_from_path(
    reference_path: Option<&Path>,
    target_path: &Path,
) -> Option<SubtitleSyncReferenceSubtitle> {
    let reference_path = reference_path?;
    if same_path(reference_path, target_path) {
        return None;
    }

    let content = match Disk::process().read_blocking(reference_path) {
        Ok(content) => content,
        Err(error) => {
            tracing::debug!(
                path = %reference_path.display(),
                error = %error,
                "subtitle sync reference subtitle unavailable"
            );
            return None;
        }
    };
    let Some(format) = detect_subtitle_format(reference_path, &content) else {
        tracing::debug!(
            path = %reference_path.display(),
            "subtitle sync reference subtitle ignored: unsupported format"
        );
        return None;
    };

    Some(SubtitleSyncReferenceSubtitle {
        encoding_hint: subtitle_encoding_hint(&content),
        file_name: reference_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string),
        format: format.sdk_format(reference_path).to_string(),
        content,
    })
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    let disk = Disk::process();
    match (
        disk.canonicalize_blocking(left),
        disk.canonicalize_blocking(right),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RewritingSubtitleSyncClient {
        expected_subtitle_content: Vec<u8>,
        rewritten_subtitle_content: Vec<u8>,
        expected_subtitle_format: &'static str,
        expected_subtitle_file_name: &'static str,
        expected_reference_subtitle: Option<ExpectedReferenceSubtitle>,
    }

    #[derive(Debug)]
    struct ExpectedReferenceSubtitle {
        content: Vec<u8>,
        format: &'static str,
        file_name: &'static str,
    }

    #[async_trait::async_trait]
    impl crate::ports::SubtitleSyncClient for RewritingSubtitleSyncClient {
        async fn align_subtitle(
            &self,
            job: crate::ports::SubtitleSyncJob,
        ) -> AppResult<SubtitleSyncAlignResponse> {
            assert_eq!(job.subtitle_content, self.expected_subtitle_content);
            assert_eq!(job.subtitle_format, self.expected_subtitle_format);
            assert_eq!(
                job.subtitle_file_name.as_deref(),
                Some(self.expected_subtitle_file_name)
            );
            assert_eq!(job.subtitle_encoding_hint.as_deref(), Some("utf-8"));
            match (&job.reference_subtitle, &self.expected_reference_subtitle) {
                (Some(actual), Some(expected)) => {
                    assert_eq!(actual.content, expected.content);
                    assert_eq!(actual.format, expected.format);
                    assert_eq!(actual.file_name.as_deref(), Some(expected.file_name));
                    assert_eq!(actual.encoding_hint.as_deref(), Some("utf-8"));
                }
                (None, None) => {}
                other => panic!("unexpected reference subtitle: {other:?}"),
            }
            assert_eq!(job.sync_options.start_seconds, 0);
            assert_eq!(job.sync_options.max_subtitle_duration_ms, 10_000);
            assert!(job.sync_options.precise_framerate_search);
            assert_eq!(job.sync_options.output_encoding, "same");
            Ok(SubtitleSyncAlignResponse {
                applied: true,
                offset_ms: -1000,
                rewritten_subtitle: Some(scryer_plugin_sdk::SubtitleSyncRewrittenSubtitle {
                    content_base64: BASE64.encode(&self.rewritten_subtitle_content),
                    format: self.expected_subtitle_format.to_string(),
                }),
                score: Some(42.0),
                selected_framerate_ratio: Some(1.0),
                consistency_ratio: Some(1.0),
                nosplit_score: Some(42.0),
                split_score: None,
                skipped_reason: None,
                backend: "test-subtitle-sync".to_string(),
                warnings: Vec::new(),
                message: None,
            })
        }
    }

    #[test]
    fn detects_subtitle_format_from_extension_or_content() {
        assert_eq!(
            detect_subtitle_format(Path::new("subtitle.srt"), b""),
            Some(SubtitleTimingFormat::Srt)
        );
        assert_eq!(
            detect_subtitle_format(Path::new("subtitle.vtt"), b""),
            Some(SubtitleTimingFormat::Vtt)
        );
        assert_eq!(
            detect_subtitle_format(Path::new("subtitle.ass"), b""),
            Some(SubtitleTimingFormat::Ass)
        );
        assert_eq!(
            detect_subtitle_format(
                Path::new("subtitle"),
                b"1\n00:00:01,000 --> 00:00:02,000\nHello\n"
            ),
            Some(SubtitleTimingFormat::Srt)
        );
        assert_eq!(
            detect_subtitle_format(
                Path::new("subtitle"),
                b"WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHello\n"
            ),
            Some(SubtitleTimingFormat::Vtt)
        );
        assert_eq!(
            detect_subtitle_format(Path::new("subtitle"), b"[Events]\nDialogue: 0,0:00:01.00"),
            Some(SubtitleTimingFormat::Ass)
        );
        assert_eq!(
            detect_subtitle_format(Path::new("subtitle"), b"plain"),
            None
        );
    }

    #[test]
    fn subtitle_encoding_hint_identifies_utf8_and_single_byte_inputs() {
        assert_eq!(
            subtitle_encoding_hint("hello".as_bytes()).as_deref(),
            Some("utf-8")
        );

        let mut bytes = b"caf".to_vec();
        bytes.push(0xe9);
        let hint = subtitle_encoding_hint(&bytes).unwrap();
        assert_ne!(hint, "utf-8");
        assert!(!hint.is_empty());
    }

    #[test]
    fn targeted_audio_codec_routes_only_ffmpeg_wasm_codecs() {
        assert_eq!(
            targeted_audio_codec(Some("ac3"), None),
            Some(SubtitleSyncAudioCodec::Ac3)
        );
        assert_eq!(
            targeted_audio_codec(Some("eac3"), None),
            Some(SubtitleSyncAudioCodec::Eac3)
        );
        assert_eq!(
            targeted_audio_codec(Some("truehd"), None),
            Some(SubtitleSyncAudioCodec::TrueHd)
        );
        assert_eq!(
            targeted_audio_codec(Some("dts"), Some("DTS-HD MA")),
            Some(SubtitleSyncAudioCodec::DtsHdMaCore)
        );
        assert_eq!(
            targeted_audio_codec(Some("dts"), Some("DTS Core")),
            Some(SubtitleSyncAudioCodec::Dts)
        );
        assert_eq!(targeted_audio_codec(Some("aac"), None), None);
        assert_eq!(targeted_audio_codec(Some("flac"), None), None);
    }

    #[test]
    fn missing_enhanced_sync_plugin_warning_mentions_install_hint() {
        let message = missing_enhanced_sync_install_hint();
        assert!(message.contains("install and enable it"));
        assert!(message.contains(ENHANCED_SUBTITLE_SYNC_PLUGIN_ID));
    }

    #[test]
    fn missing_enhanced_sync_plugin_warning_mentions_update_hint() {
        let message = missing_enhanced_sync_update_hint();
        assert!(message.contains("too old"));
        assert!(message.contains("update"));
        assert!(message.contains(ENHANCED_SUBTITLE_SYNC_PLUGIN_ID));
    }

    #[tokio::test]
    async fn sync_applies_rewritten_subtitle_bytes_from_plugin() {
        let temp_dir = tempfile::tempdir().unwrap();
        let subtitle_path = temp_dir.path().join("subtitle.srt");
        let original = b"1\n00:00:01,000 --> 00:00:02,000\nHello\n\n2\n00:00:03,000 --> 00:00:04,000\nWorld\n\n3\n00:00:05,000 --> 00:00:06,000\nAgain\n".to_vec();
        let rewritten = b"1\n00:00:00,000 --> 00:00:01,000\nHello\n\n2\n00:00:02,000 --> 00:00:03,000\nWorld\n\n3\n00:00:04,000 --> 00:00:05,000\nAgain\n".to_vec();
        std::fs::write(&subtitle_path, &original).unwrap();

        let result = sync_subtitle_with_policy_and_plugin_sync(
            Path::new("/tmp/video.mkv"),
            &subtitle_path,
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
            Some(Arc::new(RewritingSubtitleSyncClient {
                expected_subtitle_content: original,
                rewritten_subtitle_content: rewritten.clone(),
                expected_subtitle_format: "srt",
                expected_subtitle_file_name: "subtitle.srt",
                expected_reference_subtitle: None,
            })),
            true,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(result.applied);
        assert_eq!(result.offset_ms, -1000);
        assert_eq!(std::fs::read(&subtitle_path).unwrap(), rewritten);
    }

    #[tokio::test]
    async fn sync_routes_and_replaces_vtt_subtitles() {
        let temp_dir = tempfile::tempdir().unwrap();
        let subtitle_path = temp_dir.path().join("subtitle.vtt");
        let original = b"WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHello\n\n00:00:03.000 --> 00:00:04.000\nWorld\n".to_vec();
        let rewritten = b"WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nHello\n\n00:00:02.000 --> 00:00:03.000\nWorld\n".to_vec();
        std::fs::write(&subtitle_path, &original).unwrap();

        let result = sync_subtitle_with_policy_and_plugin_sync(
            Path::new("/tmp/video.mkv"),
            &subtitle_path,
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
            Some(Arc::new(RewritingSubtitleSyncClient {
                expected_subtitle_content: original,
                rewritten_subtitle_content: rewritten.clone(),
                expected_subtitle_format: "vtt",
                expected_subtitle_file_name: "subtitle.vtt",
                expected_reference_subtitle: None,
            })),
            true,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(result.applied);
        assert_eq!(result.format, Some(SubtitleTimingFormat::Vtt));
        assert_eq!(std::fs::read(&subtitle_path).unwrap(), rewritten);
    }

    #[tokio::test]
    async fn sync_sends_reference_subtitle_when_available() {
        let temp_dir = tempfile::tempdir().unwrap();
        let subtitle_path = temp_dir.path().join("subtitle.srt");
        let reference_path = temp_dir.path().join("reference.vtt");
        let original = b"1\n00:00:01,000 --> 00:00:02,000\nHello\n".to_vec();
        let reference = b"WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nHello\n\n00:00:02.000 --> 00:00:03.000\nWorld\n".to_vec();
        let rewritten = b"1\n00:00:00,000 --> 00:00:01,000\nHello\n".to_vec();
        std::fs::write(&subtitle_path, &original).unwrap();
        std::fs::write(&reference_path, &reference).unwrap();

        let result = sync_subtitle_with_policy_and_plugin_sync(
            Path::new("/tmp/video.mkv"),
            &subtitle_path,
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
            Some(Arc::new(RewritingSubtitleSyncClient {
                expected_subtitle_content: original,
                rewritten_subtitle_content: rewritten.clone(),
                expected_subtitle_format: "srt",
                expected_subtitle_file_name: "subtitle.srt",
                expected_reference_subtitle: Some(ExpectedReferenceSubtitle {
                    content: reference,
                    format: "vtt",
                    file_name: "reference.vtt",
                }),
            })),
            true,
            Some(&reference_path),
            None,
        )
        .await
        .unwrap();

        assert!(result.applied);
        assert_eq!(std::fs::read(&subtitle_path).unwrap(), rewritten);
    }

    #[tokio::test]
    async fn sync_delegates_single_cue_subtitles_to_plugin() {
        let temp_dir = tempfile::tempdir().unwrap();
        let subtitle_path = temp_dir.path().join("subtitle.srt");
        let original = b"1\n00:00:01,000 --> 00:00:02,000\nHello\n".to_vec();
        let rewritten = b"1\n00:00:02,000 --> 00:00:03,000\nHello\n".to_vec();
        std::fs::write(&subtitle_path, &original).unwrap();

        let result = sync_subtitle_with_policy_and_plugin_sync(
            Path::new("/tmp/video.mkv"),
            &subtitle_path,
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
            Some(Arc::new(RewritingSubtitleSyncClient {
                expected_subtitle_content: original,
                rewritten_subtitle_content: rewritten.clone(),
                expected_subtitle_format: "srt",
                expected_subtitle_file_name: "subtitle.srt",
                expected_reference_subtitle: None,
            })),
            true,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(result.applied);
        assert_eq!(std::fs::read(&subtitle_path).unwrap(), rewritten);
    }

    #[tokio::test]
    async fn sync_rejects_applied_plugin_response_without_rewritten_subtitle() {
        struct MissingRewriteClient;

        #[async_trait::async_trait]
        impl crate::ports::SubtitleSyncClient for MissingRewriteClient {
            async fn align_subtitle(
                &self,
                _job: crate::ports::SubtitleSyncJob,
            ) -> AppResult<SubtitleSyncAlignResponse> {
                Ok(SubtitleSyncAlignResponse {
                    applied: true,
                    offset_ms: 1000,
                    rewritten_subtitle: None,
                    score: Some(1.0),
                    selected_framerate_ratio: Some(1.0),
                    consistency_ratio: Some(1.0),
                    nosplit_score: Some(1.0),
                    split_score: None,
                    skipped_reason: None,
                    backend: "test-subtitle-sync".to_string(),
                    warnings: Vec::new(),
                    message: None,
                })
            }
        }

        let temp_dir = tempfile::tempdir().unwrap();
        let subtitle_path = temp_dir.path().join("subtitle.srt");
        std::fs::write(
            &subtitle_path,
            "1\n00:00:01,000 --> 00:00:04,000\nHello\n\n2\n00:00:05,000 --> 00:00:08,000\nWorld\n\n3\n00:00:09,000 --> 00:00:12,000\nAgain\n",
        )
        .unwrap();

        let error = sync_subtitle_with_policy_and_plugin_sync(
            Path::new("/tmp/video.mkv"),
            &subtitle_path,
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
            Some(Arc::new(MissingRewriteClient)),
            true,
            None,
            None,
        )
        .await
        .expect_err("missing rewrite should fail");

        assert!(error.to_string().contains("rewritten_subtitle"));
    }

    #[tokio::test]
    async fn sync_requires_plugin_when_unavailable() {
        let subtitle = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            subtitle.path(),
            "1\n00:00:01,000 --> 00:00:04,000\nHello\n\n2\n00:00:05,000 --> 00:00:08,000\nWorld\n\n3\n00:00:09,000 --> 00:00:12,000\nAgain\n",
        )
        .unwrap();

        let result = sync_subtitle_with_policy_and_plugin_sync(
            Path::new("/tmp/video.mkv"),
            subtitle.path(),
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(!result.applied);
        assert_eq!(
            result.skipped_reason,
            Some(SyncSkipReason::SubtitleSyncPluginRequired)
        );
        assert_eq!(result.format, Some(SubtitleTimingFormat::Srt));
    }

    #[tokio::test]
    async fn sync_uses_stored_metadata_for_expected_codec_hint() {
        struct MetadataAssertingClient;

        #[async_trait::async_trait]
        impl crate::ports::SubtitleSyncClient for MetadataAssertingClient {
            async fn align_subtitle(
                &self,
                job: crate::ports::SubtitleSyncJob,
            ) -> AppResult<SubtitleSyncAlignResponse> {
                assert_eq!(job.expected_codec, Some(SubtitleSyncAudioCodec::Eac3));
                assert_eq!(
                    job.media_metadata
                        .as_ref()
                        .and_then(|metadata| metadata.audio_codec.as_deref()),
                    Some("eac3")
                );
                Ok(SubtitleSyncAlignResponse {
                    applied: false,
                    offset_ms: 0,
                    rewritten_subtitle: None,
                    score: None,
                    selected_framerate_ratio: None,
                    consistency_ratio: None,
                    nosplit_score: None,
                    split_score: None,
                    skipped_reason: Some(SubtitleSyncAlignSkipReason::WeakAlignment),
                    backend: "test-subtitle-sync".to_string(),
                    warnings: Vec::new(),
                    message: None,
                })
            }
        }

        let temp_dir = tempfile::tempdir().unwrap();
        let subtitle_path = temp_dir.path().join("subtitle.srt");
        std::fs::write(&subtitle_path, "1\n00:00:01,000 --> 00:00:02,000\nHello\n").unwrap();
        let metadata = SubtitleSyncMediaMetadataSnapshot {
            analysis_source: "scryer_import".to_string(),
            container_format: Some("matroska".to_string()),
            duration_seconds: Some(120),
            video_codec: None,
            video_width: None,
            video_height: None,
            video_bitrate_kbps: None,
            video_bit_depth: None,
            video_hdr_format: None,
            video_frame_rate: None,
            video_profile: None,
            audio_codec: Some("eac3".to_string()),
            audio_profile: None,
            audio_channels: Some(6),
            audio_bitrate_kbps: None,
            audio_languages: vec!["eng".to_string()],
            audio_streams: Vec::new(),
            subtitle_languages: Vec::new(),
            subtitle_codecs: Vec::new(),
            subtitle_streams: Vec::new(),
            has_multiaudio: false,
            num_chapters: None,
        };

        let result = sync_subtitle_with_policy_and_plugin_sync(
            Path::new("/tmp/video.mkv"),
            &subtitle_path,
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
            Some(Arc::new(MetadataAssertingClient)),
            true,
            None,
            Some(metadata),
        )
        .await
        .unwrap();

        assert!(!result.applied);
        assert_eq!(result.skipped_reason, Some(SyncSkipReason::WeakAlignment));
    }

    #[tokio::test]
    async fn policy_skip_when_disabled() {
        let result = sync_subtitle_with_policy(
            Path::new("/tmp/video.mkv"),
            Path::new("/tmp/subtitle.srt"),
            SyncPolicy {
                enabled: false,
                forced: false,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
        )
        .await
        .unwrap();

        assert!(!result.applied);
        assert_eq!(result.skipped_reason, Some(SyncSkipReason::Disabled));
    }

    #[tokio::test]
    async fn policy_skip_when_forced() {
        let result = sync_subtitle_with_policy(
            Path::new("/tmp/video.mkv"),
            Path::new("/tmp/subtitle.srt"),
            SyncPolicy {
                enabled: true,
                forced: true,
                score: Some(10),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
        )
        .await
        .unwrap();

        assert!(!result.applied);
        assert_eq!(result.skipped_reason, Some(SyncSkipReason::ForcedSubtitle));
    }

    #[tokio::test]
    async fn policy_skip_when_score_above_threshold() {
        let result = sync_subtitle_with_policy(
            Path::new("/tmp/video.mkv"),
            Path::new("/tmp/subtitle.srt"),
            SyncPolicy {
                enabled: true,
                forced: false,
                score: Some(91),
                threshold: Some(90),
                max_offset_seconds: 60,
            },
        )
        .await
        .unwrap();

        assert!(!result.applied);
        assert_eq!(
            result.skipped_reason,
            Some(SyncSkipReason::ScoreAboveThreshold)
        );
    }

    #[test]
    fn sync_result_summary_uses_skip_reason() {
        let result = skipped_sync_result(0, None, None, None, None, SyncSkipReason::ForcedSubtitle);
        assert_eq!(result.summary(), "skipped (forced_subtitle)");
    }
}
