//! Subtitle timing synchronization using pure Rust audio decoding and alignment.
//!
//! The sync pipeline:
//! 1. Decodes a usable audio track with Symphonia.
//! 2. Builds speech spans with a lightweight adaptive VAD.
//! 3. Parses subtitle timing spans from SRT or ASS/SSA.
//! 4. Uses alass-core to estimate a constant offset and skips low-consistency
//!    alignments instead of forcing a risky rewrite.

use std::{fmt, path::Path};

use crate::{AppError, AppResult};
use alass_core::{NoProgressHandler, TimeDelta, TimePoint, TimeSpan};

/// Result of a subtitle sync operation.
#[derive(Debug, Clone)]
pub struct SyncResult {
    /// Time offset applied in milliseconds.
    pub offset_ms: i64,
    /// Whether the sync was applied.
    pub applied: bool,
    /// Parsed subtitle format when one was recognized.
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

const WINDOW_MS: i64 = 10;
const SPLIT_PENALTY: f64 = 7.0;
const MIN_REFERENCE_SPANS: usize = 3;
const MIN_SUBTITLE_SPANS: usize = 3;
const MIN_EFFECTIVE_OFFSET_MS: i64 = 50;
const DELTA_CONSISTENCY_TOLERANCE_MS: i64 = 350;
const MIN_CONSISTENT_DELTA_RATIO: f64 = 0.5;
const VAD_START_THRESHOLD_MIN: f64 = 500.0;
const VAD_STOP_THRESHOLD_MIN: f64 = 250.0;
const VAD_START_MULTIPLIER: f64 = 3.0;
const VAD_STOP_MULTIPLIER: f64 = 1.8;
const VAD_NOISE_SMOOTHING: f64 = 0.05;
const VAD_MIN_SILENCE_WINDOWS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleTimingFormat {
    Srt,
    Ass,
}

impl SubtitleTimingFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Ass => "ass/ssa",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncSkipReason {
    Disabled,
    ForcedSubtitle,
    ScoreAboveThreshold,
    UnsupportedSubtitleFormat,
    NotEnoughReferenceSpans,
    NotEnoughSubtitleSpans,
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
            Self::UnsupportedSubtitleFormat => "unsupported_subtitle_format",
            Self::NotEnoughReferenceSpans => "not_enough_reference_spans",
            Self::NotEnoughSubtitleSpans => "not_enough_subtitle_spans",
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

#[derive(Debug, Clone, Copy)]
struct AlignmentSummary {
    offset_ms: i64,
    consistency_ratio: f64,
    nosplit_score: f64,
    split_score: f64,
}

#[derive(Debug, Clone, Copy)]
struct AssEventFormat {
    field_count: usize,
    start_idx: usize,
    end_idx: usize,
}

impl Default for AssEventFormat {
    fn default() -> Self {
        Self {
            field_count: 10,
            start_idx: 1,
            end_idx: 2,
        }
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

    sync_subtitle(video_path, subtitle_path, policy.max_offset_seconds).await
}

/// Synchronize a subtitle file with a video file's audio track.
pub async fn sync_subtitle(
    video_path: &Path,
    subtitle_path: &Path,
    max_offset_seconds: i64,
) -> AppResult<SyncResult> {
    let reference_spans = extract_audio_speech_spans(video_path).await?;
    if reference_spans.len() < MIN_REFERENCE_SPANS {
        tracing::debug!(
            path = %video_path.display(),
            spans = reference_spans.len(),
            "subtitle sync skipped: not enough reference speech spans"
        );
        return Ok(skipped_sync_result(
            0,
            None,
            None,
            None,
            None,
            SyncSkipReason::NotEnoughReferenceSpans,
        ));
    }

    let Some((subtitle_format, subtitle_spans)) = read_subtitle_spans(subtitle_path)? else {
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
    if subtitle_spans.len() < MIN_SUBTITLE_SPANS {
        tracing::debug!(
            path = %subtitle_path.display(),
            format = subtitle_format.label(),
            spans = subtitle_spans.len(),
            "subtitle sync skipped: not enough subtitle spans"
        );
        return Ok(skipped_sync_result(
            0,
            Some(subtitle_format),
            None,
            None,
            None,
            SyncSkipReason::NotEnoughSubtitleSpans,
        ));
    }

    let alignment = tokio::task::spawn_blocking(move || {
        crate::nice_thread();
        compute_alignment(&reference_spans, &subtitle_spans)
    })
    .await
    .map_err(|e| AppError::Repository(format!("alass task panicked: {e}")))?;

    if alignment.nosplit_score <= 0.0 || alignment.split_score <= 0.0 {
        tracing::warn!(
            path = %subtitle_path.display(),
            format = subtitle_format.label(),
            offset_ms = alignment.offset_ms,
            nosplit_score = alignment.nosplit_score,
            split_score = alignment.split_score,
            "subtitle sync skipped: alignment score too weak"
        );
        return Ok(skipped_sync_result(
            alignment.offset_ms,
            Some(subtitle_format),
            Some(alignment.consistency_ratio),
            Some(alignment.nosplit_score),
            Some(alignment.split_score),
            SyncSkipReason::WeakAlignment,
        ));
    }

    if alignment.consistency_ratio < MIN_CONSISTENT_DELTA_RATIO {
        tracing::warn!(
            path = %subtitle_path.display(),
            format = subtitle_format.label(),
            offset_ms = alignment.offset_ms,
            consistency_ratio = alignment.consistency_ratio,
            "subtitle sync skipped: low alignment consistency"
        );
        return Ok(skipped_sync_result(
            alignment.offset_ms,
            Some(subtitle_format),
            Some(alignment.consistency_ratio),
            Some(alignment.nosplit_score),
            Some(alignment.split_score),
            SyncSkipReason::LowAlignmentConsistency,
        ));
    }

    if alignment.offset_ms.unsigned_abs() > (max_offset_seconds as u64 * 1000) {
        tracing::warn!(
            path = %subtitle_path.display(),
            format = subtitle_format.label(),
            offset_ms = alignment.offset_ms,
            max_offset_seconds,
            "subtitle sync skipped: offset exceeds configured maximum"
        );
        return Ok(skipped_sync_result(
            alignment.offset_ms,
            Some(subtitle_format),
            Some(alignment.consistency_ratio),
            Some(alignment.nosplit_score),
            Some(alignment.split_score),
            SyncSkipReason::OffsetExceedsMaximum,
        ));
    }

    if alignment.offset_ms.unsigned_abs() < MIN_EFFECTIVE_OFFSET_MS as u64 {
        tracing::debug!(
            path = %subtitle_path.display(),
            format = subtitle_format.label(),
            offset_ms = alignment.offset_ms,
            "subtitle sync skipped: offset too small to apply"
        );
        return Ok(skipped_sync_result(
            alignment.offset_ms,
            Some(subtitle_format),
            Some(alignment.consistency_ratio),
            Some(alignment.nosplit_score),
            Some(alignment.split_score),
            SyncSkipReason::OffsetTooSmall,
        ));
    }

    apply_subtitle_offset(subtitle_path, subtitle_format, alignment.offset_ms)?;

    tracing::info!(
        path = %subtitle_path.display(),
        format = subtitle_format.label(),
        offset_ms = alignment.offset_ms,
        consistency_ratio = alignment.consistency_ratio,
        nosplit_score = alignment.nosplit_score,
        split_score = alignment.split_score,
        "subtitle synchronized"
    );
    Ok(SyncResult {
        offset_ms: alignment.offset_ms,
        applied: true,
        format: Some(subtitle_format),
        consistency_ratio: Some(alignment.consistency_ratio),
        nosplit_score: Some(alignment.nosplit_score),
        split_score: Some(alignment.split_score),
        skipped_reason: None,
    })
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

fn compute_alignment(
    reference_spans: &[TimeSpan],
    subtitle_spans: &[TimeSpan],
) -> AlignmentSummary {
    let (offset, nosplit_score) = alass_core::align_nosplit(
        reference_spans,
        subtitle_spans,
        alass_core::standard_scoring,
        NoProgressHandler,
    );
    let (split_deltas, split_score) = alass_core::align(
        reference_spans,
        subtitle_spans,
        SPLIT_PENALTY,
        None,
        alass_core::standard_scoring,
        NoProgressHandler,
    );

    let offset_ms = offset.as_i64();
    let consistency_ratio = delta_consistency_ratio(&split_deltas, offset_ms);

    AlignmentSummary {
        offset_ms,
        consistency_ratio,
        nosplit_score,
        split_score,
    }
}

fn delta_consistency_ratio(deltas: &[TimeDelta], offset_ms: i64) -> f64 {
    if deltas.is_empty() {
        return 0.0;
    }

    let consistent = deltas
        .iter()
        .filter(|delta| (delta.as_i64() - offset_ms).abs() <= DELTA_CONSISTENCY_TOLERANCE_MS)
        .count();
    consistent as f64 / deltas.len() as f64
}

// ── Audio extraction via Symphonia (pure Rust, no external deps) ────────────

/// Extract speech spans from a video file's audio track using Symphonia.
async fn extract_audio_speech_spans(video_path: &Path) -> AppResult<Vec<TimeSpan>> {
    let path = video_path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        crate::nice_thread();
        decode_audio_to_speech_spans(&path)
    })
    .await
    .map_err(|e| AppError::Repository(format!("audio decode task panicked: {e}")))?
}

struct SelectedAudioTrack {
    id: u32,
    sample_rate: u32,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
}

/// Decode a usable audio track and produce speech spans via adaptive energy VAD.
fn decode_audio_to_speech_spans(path: &Path) -> AppResult<Vec<TimeSpan>> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|e| {
        AppError::Repository(format!("cannot open video for audio extraction: {e}"))
    })?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| AppError::Repository(format!("audio probe failed: {e}")))?;

    let mut format = probed.format;
    let SelectedAudioTrack {
        id: track_id,
        sample_rate,
        mut decoder,
    } = select_audio_track(format.as_mut())?;
    let mut detector = SpeechSpanDetector::new(sample_rate);

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(_) => continue,
        };

        let spec = *decoded.spec();
        let num_frames = decoded.frames();
        let channels = spec.channels.count().max(1);

        let mut sample_buf = SampleBuffer::<i16>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        detector.push_interleaved_i16(sample_buf.samples(), channels);
    }

    Ok(detector.finish())
}

fn select_audio_track(
    format: &mut dyn symphonia::core::formats::FormatReader,
) -> AppResult<SelectedAudioTrack> {
    use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};

    let default_track_id = format.default_track().map(|track| track.id);
    let mut best_track: Option<(i32, SelectedAudioTrack)> = None;

    for track in format.tracks() {
        if track.codec_params.codec == CODEC_TYPE_NULL {
            continue;
        }

        let Ok(decoder) =
            symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())
        else {
            continue;
        };

        let sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
        let channel_count = track
            .codec_params
            .channels
            .map(|channels| channels.count())
            .unwrap_or(1);
        let priority = track_selection_priority(track, default_track_id)
            + channel_count as i32
            + sample_rate as i32 / 1000;

        let selected = SelectedAudioTrack {
            id: track.id,
            sample_rate,
            decoder,
        };

        match &best_track {
            Some((best_priority, _)) if *best_priority >= priority => {}
            _ => best_track = Some((priority, selected)),
        }
    }

    best_track
        .map(|(_, track)| track)
        .ok_or_else(|| AppError::Repository("no decodable audio track found".into()))
}

fn track_selection_priority(
    track: &symphonia::core::formats::Track,
    default_track_id: Option<u32>,
) -> i32 {
    let mut priority = 0;
    if Some(track.id) == default_track_id {
        priority += 10_000;
    }
    if track.codec_params.sample_rate.is_some() {
        priority += 1_000;
    }
    if track.language.is_some() {
        priority += 10;
    }
    priority
}

struct SpeechSpanDetector {
    samples_per_window: usize,
    frames_in_window: usize,
    window_energy_sum: f64,
    current_window_start_ms: i64,
    noise_floor: f64,
    noise_floor_initialized: bool,
    below_threshold_windows: usize,
    in_speech: bool,
    speech_start_ms: i64,
    spans: Vec<TimeSpan>,
}

impl SpeechSpanDetector {
    fn new(sample_rate: u32) -> Self {
        Self {
            samples_per_window: (sample_rate / 100).max(1) as usize,
            frames_in_window: 0,
            window_energy_sum: 0.0,
            current_window_start_ms: 0,
            noise_floor: 0.0,
            noise_floor_initialized: false,
            below_threshold_windows: 0,
            in_speech: false,
            speech_start_ms: 0,
            spans: Vec::new(),
        }
    }

    fn push_interleaved_i16(&mut self, samples: &[i16], channels: usize) {
        let channels = channels.max(1);
        for frame in samples.chunks_exact(channels) {
            let mean_sq = frame
                .iter()
                .map(|sample| {
                    let sample = *sample as f64;
                    sample * sample
                })
                .sum::<f64>()
                / channels as f64;
            self.push_frame_energy(mean_sq);
        }
    }

    fn push_frame_energy(&mut self, mean_sq: f64) {
        self.window_energy_sum += mean_sq;
        self.frames_in_window += 1;

        if self.frames_in_window >= self.samples_per_window {
            let rms = (self.window_energy_sum / self.frames_in_window as f64).sqrt();
            self.process_window(rms);
            self.frames_in_window = 0;
            self.window_energy_sum = 0.0;
        }
    }

    fn process_window(&mut self, rms: f64) {
        if !self.noise_floor_initialized {
            self.noise_floor = rms.clamp(1.0, VAD_START_THRESHOLD_MIN / VAD_START_MULTIPLIER);
            self.noise_floor_initialized = true;
        } else if !self.in_speech || rms < self.noise_floor * VAD_START_MULTIPLIER {
            self.noise_floor =
                (1.0 - VAD_NOISE_SMOOTHING) * self.noise_floor + VAD_NOISE_SMOOTHING * rms.max(1.0);
        }

        let start_threshold =
            (self.noise_floor * VAD_START_MULTIPLIER).max(VAD_START_THRESHOLD_MIN);
        let stop_threshold = (self.noise_floor * VAD_STOP_MULTIPLIER).max(VAD_STOP_THRESHOLD_MIN);
        let window_start_ms = self.current_window_start_ms;

        if rms > start_threshold {
            self.below_threshold_windows = 0;
            if !self.in_speech {
                self.in_speech = true;
                self.speech_start_ms = window_start_ms;
            }
        } else if self.in_speech && rms <= stop_threshold {
            self.below_threshold_windows += 1;
            if self.below_threshold_windows >= VAD_MIN_SILENCE_WINDOWS {
                let end_ms = window_start_ms - ((VAD_MIN_SILENCE_WINDOWS as i64 - 1) * WINDOW_MS);
                self.push_span(self.speech_start_ms, end_ms);
                self.in_speech = false;
                self.below_threshold_windows = 0;
            }
        } else if self.in_speech {
            self.below_threshold_windows = 0;
        }

        self.current_window_start_ms += WINDOW_MS;
    }

    fn finish(mut self) -> Vec<TimeSpan> {
        if self.frames_in_window > 0 {
            let rms = (self.window_energy_sum / self.frames_in_window as f64).sqrt();
            self.process_window(rms);
            self.frames_in_window = 0;
            self.window_energy_sum = 0.0;
        }

        if self.in_speech {
            self.push_span(self.speech_start_ms, self.current_window_start_ms);
            self.in_speech = false;
        }

        self.spans
    }

    fn push_span(&mut self, start_ms: i64, end_ms: i64) {
        if end_ms <= start_ms {
            return;
        }

        if let Some(last) = self.spans.last_mut() {
            let last_end = last.end.as_i64();
            if start_ms - last_end <= WINDOW_MS {
                *last = TimeSpan::new(last.start, TimePoint::from(end_ms));
                return;
            }
        }

        self.spans.push(TimeSpan::new(
            TimePoint::from(start_ms),
            TimePoint::from(end_ms),
        ));
    }
}

// ── Subtitle parsing and shifting with charset detection ─────────────────────

/// Read a subtitle file, auto-detecting charset for common wild-text encodings.
fn read_subtitle_to_string(path: &Path) -> AppResult<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| AppError::Repository(format!("cannot read subtitle file: {e}")))?;

    if let Ok(s) = std::str::from_utf8(&bytes) {
        return Ok(s.to_string());
    }

    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(&bytes, true);
    let encoding = detector.guess(None, true);

    let (decoded, _, had_errors) = encoding.decode(&bytes);
    if had_errors {
        tracing::warn!(
            path = %path.display(),
            encoding = %encoding.name(),
            "subtitle file had encoding errors during charset conversion"
        );
    }

    Ok(decoded.into_owned())
}

fn read_subtitle_spans(path: &Path) -> AppResult<Option<(SubtitleTimingFormat, Vec<TimeSpan>)>> {
    let content = read_subtitle_to_string(path)?;
    let Some(format) = detect_subtitle_format(path, &content) else {
        return Ok(None);
    };

    let spans = match format {
        SubtitleTimingFormat::Srt => read_srt_spans_from_str(&content),
        SubtitleTimingFormat::Ass => read_ass_spans_from_str(&content),
    };
    Ok(Some((format, spans)))
}

fn detect_subtitle_format(path: &Path, content: &str) -> Option<SubtitleTimingFormat> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("srt") => return Some(SubtitleTimingFormat::Srt),
        Some("ass") | Some("ssa") => return Some(SubtitleTimingFormat::Ass),
        _ => {}
    }

    if content.contains("-->") {
        return Some(SubtitleTimingFormat::Srt);
    }
    if content.contains("[Events]")
        || content
            .lines()
            .any(|line| line_starts_with_ignore_ascii_case(line.trim_start(), "Dialogue:"))
    {
        return Some(SubtitleTimingFormat::Ass);
    }

    None
}

fn apply_subtitle_offset(
    subtitle_path: &Path,
    format: SubtitleTimingFormat,
    offset_ms: i64,
) -> AppResult<()> {
    let content = read_subtitle_to_string(subtitle_path)?;
    let shifted = match format {
        SubtitleTimingFormat::Srt => shift_srt_content(&content, offset_ms),
        SubtitleTimingFormat::Ass => shift_ass_content(&content, offset_ms),
    };

    std::fs::write(subtitle_path, shifted)
        .map_err(|e| AppError::Repository(format!("cannot write subtitle file: {e}")))?;
    Ok(())
}

fn read_srt_spans_from_str(content: &str) -> Vec<TimeSpan> {
    let mut spans = Vec::new();
    for line in content.lines() {
        if let Some((start, end)) = line.split_once("-->")
            && let (Some(start), Some(end)) = (parse_srt_ts(start.trim()), parse_srt_ts(end.trim()))
        {
            spans.push(TimeSpan::new(TimePoint::from(start), TimePoint::from(end)));
        }
    }
    spans
}

fn shift_srt_content(content: &str, offset_ms: i64) -> String {
    let mut out = String::with_capacity(content.len());
    for line in content.lines() {
        if let Some((start_str, end_str)) = line.split_once("-->")
            && let (Some(start), Some(end)) =
                (parse_srt_ts(start_str.trim()), parse_srt_ts(end_str.trim()))
        {
            out.push_str(&format_srt_ts(start + offset_ms));
            out.push_str(" --> ");
            out.push_str(&format_srt_ts(end + offset_ms));
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn read_ass_spans_from_str(content: &str) -> Vec<TimeSpan> {
    let mut spans = Vec::new();
    let mut in_events = false;
    let mut event_format = AssEventFormat::default();

    for line in content.lines() {
        let trimmed = line.trim();
        if is_section_header(trimmed) {
            in_events = trimmed.eq_ignore_ascii_case("[Events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if line_starts_with_ignore_ascii_case(trimmed, "Format:") {
            if let Some(parsed) = parse_ass_event_format(trimmed) {
                event_format = parsed;
            }
            continue;
        }
        if !line_starts_with_ignore_ascii_case(trimmed, "Dialogue:") {
            continue;
        }

        let Some(fields) = split_ass_fields(trimmed, event_format.field_count) else {
            continue;
        };
        if let (Some(start), Some(end)) = (
            parse_ass_ts(fields[event_format.start_idx].trim()),
            parse_ass_ts(fields[event_format.end_idx].trim()),
        ) {
            spans.push(TimeSpan::new(TimePoint::from(start), TimePoint::from(end)));
        }
    }

    spans
}

fn shift_ass_content(content: &str, offset_ms: i64) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_events = false;
    let mut event_format = AssEventFormat::default();

    for line in content.lines() {
        let trimmed = line.trim();
        if is_section_header(trimmed) {
            in_events = trimmed.eq_ignore_ascii_case("[Events]");
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if in_events && line_starts_with_ignore_ascii_case(trimmed, "Format:") {
            if let Some(parsed) = parse_ass_event_format(trimmed) {
                event_format = parsed;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if in_events && let Some(rewritten) = rewrite_ass_event_line(line, &event_format, offset_ms)
        {
            out.push_str(&rewritten);
            out.push('\n');
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }

    out
}

fn parse_ass_event_format(line: &str) -> Option<AssEventFormat> {
    let (_, rest) = line.split_once(':')?;
    let fields: Vec<String> = rest
        .split(',')
        .map(|field| field.trim().to_ascii_lowercase())
        .collect();
    let start_idx = fields.iter().position(|field| field == "start")?;
    let end_idx = fields.iter().position(|field| field == "end")?;

    Some(AssEventFormat {
        field_count: fields.len(),
        start_idx,
        end_idx,
    })
}

fn split_ass_fields(line: &str, field_count: usize) -> Option<Vec<&str>> {
    let (_, rest) = line.split_once(':')?;
    let fields: Vec<&str> = rest.trim_start().splitn(field_count, ',').collect();
    if fields.len() != field_count {
        return None;
    }
    Some(fields)
}

fn rewrite_ass_event_line(line: &str, format: &AssEventFormat, offset_ms: i64) -> Option<String> {
    let colon_index = line.find(':')?;
    let prefix = &line[..colon_index];
    let event_kind = prefix.trim();
    if !matches!(
        event_kind.to_ascii_lowercase().as_str(),
        "dialogue" | "comment" | "picture" | "sound" | "movie" | "command"
    ) {
        return None;
    }

    let rest = &line[colon_index + 1..];
    let leading_ws_len = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    let leading_ws = &rest[..leading_ws_len];

    let mut fields: Vec<String> = rest
        .trim_start()
        .splitn(format.field_count, ',')
        .map(|field| field.to_string())
        .collect();
    if fields.len() != format.field_count {
        return None;
    }

    let start = parse_ass_ts(fields[format.start_idx].trim())?;
    let end = parse_ass_ts(fields[format.end_idx].trim())?;
    fields[format.start_idx] = format_ass_ts(start + offset_ms);
    fields[format.end_idx] = format_ass_ts(end + offset_ms);

    Some(format!("{prefix}:{leading_ws}{}", fields.join(",")))
}

fn is_section_header(line: &str) -> bool {
    line.starts_with('[') && line.ends_with(']')
}

fn line_starts_with_ignore_ascii_case(line: &str, prefix: &str) -> bool {
    line.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

// ── Timestamp parsing and formatting ─────────────────────────────────────────

fn parse_srt_ts(ts: &str) -> Option<i64> {
    let parts: Vec<&str> = ts.split([':', ',', '.']).collect();
    if parts.len() < 4 {
        return None;
    }
    let h: i64 = parts[0].trim().parse().ok()?;
    let m: i64 = parts[1].trim().parse().ok()?;
    let s: i64 = parts[2].trim().parse().ok()?;
    let ms: i64 = parts[3].trim().parse().ok()?;
    Some(h * 3_600_000 + m * 60_000 + s * 1_000 + ms)
}

fn format_srt_ts(ms: i64) -> String {
    let ms = ms.max(0);
    let ts = ms / 1000;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ts / 3600,
        (ts % 3600) / 60,
        ts % 60,
        ms % 1000
    )
}

fn parse_ass_ts(ts: &str) -> Option<i64> {
    let ts = ts.trim();
    let separator = ts.find(['.', ','])?;
    let main = &ts[..separator];
    let frac = &ts[separator + 1..];

    let parts: Vec<&str> = main.split(':').collect();
    if parts.len() != 3 {
        return None;
    }

    let h: i64 = parts[0].trim().parse().ok()?;
    let m: i64 = parts[1].trim().parse().ok()?;
    let s: i64 = parts[2].trim().parse().ok()?;
    let ms = parse_fractional_ms(frac)?;

    Some(h * 3_600_000 + m * 60_000 + s * 1_000 + ms)
}

fn parse_fractional_ms(frac: &str) -> Option<i64> {
    let digits: String = frac
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .take(3)
        .collect();
    if digits.is_empty() {
        return Some(0);
    }

    let value: i64 = digits.parse().ok()?;
    Some(match digits.len() {
        1 => value * 100,
        2 => value * 10,
        _ => value,
    })
}

fn format_ass_ts(ms: i64) -> String {
    let total_cs = (ms.max(0) + 5) / 10;
    let total_seconds = total_cs / 100;
    format!(
        "{}:{:02}:{:02}.{:02}",
        total_seconds / 3600,
        (total_seconds % 3600) / 60,
        total_seconds % 60,
        total_cs % 100
    )
}

// ── Standalone VAD for testing ───────────────────────────────────────────────

/// Adaptive energy-based VAD on raw 16-bit mono PCM, used by unit tests.
#[cfg(test)]
fn detect_speech_spans(pcm: &[u8], sample_rate: u32) -> Vec<TimeSpan> {
    let mut detector = SpeechSpanDetector::new(sample_rate);
    for sample in pcm.chunks_exact(2) {
        let sample = i16::from_le_bytes([sample[0], sample[1]]) as f64;
        detector.push_frame_energy(sample * sample);
    }
    detector.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_roundtrip() {
        assert_eq!(parse_srt_ts("00:01:23,456"), Some(83_456));
        assert_eq!(format_srt_ts(83_456), "00:01:23,456");
        assert_eq!(format_srt_ts(0), "00:00:00,000");
    }

    #[test]
    fn ass_parse_and_format_roundtrip() {
        assert_eq!(parse_ass_ts("0:01:23.45"), Some(83_450));
        assert_eq!(format_ass_ts(83_450), "0:01:23.45");
        assert_eq!(format_ass_ts(-100), "0:00:00.00");
    }

    #[test]
    fn silent_audio_no_spans() {
        let silent = vec![0u8; 32_000];
        assert!(detect_speech_spans(&silent, 16_000).is_empty());
    }

    #[test]
    fn format_srt_ts_clamps_negative() {
        assert_eq!(format_srt_ts(-1000), "00:00:00,000");
        assert_eq!(format_srt_ts(-1), "00:00:00,000");
    }

    #[test]
    fn parse_srt_ts_with_dot_separator() {
        assert_eq!(parse_srt_ts("00:01:23.456"), Some(83_456));
    }

    #[test]
    fn parse_srt_ts_hours_greater_than_23() {
        assert_eq!(parse_srt_ts("25:00:00,000"), Some(25 * 3_600_000));
    }

    #[test]
    fn parse_srt_ts_rejects_too_few_parts() {
        assert_eq!(parse_srt_ts("00:01:23"), None);
        assert_eq!(parse_srt_ts(""), None);
    }

    #[test]
    fn parse_srt_ts_rejects_non_numeric() {
        assert_eq!(parse_srt_ts("ab:cd:ef,ghi"), None);
    }

    #[test]
    fn loud_audio_produces_one_big_span() {
        let sample_rate = 16_000u32;
        let spw = (sample_rate / 100) as usize;
        let num_windows = 50;
        let mut pcm = Vec::with_capacity(spw * num_windows * 2);
        for _ in 0..(spw * num_windows) {
            pcm.extend_from_slice(&32_767i16.to_le_bytes());
        }
        let spans = detect_speech_spans(&pcm, sample_rate);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn alternating_loud_silent_windows_get_smoothed_into_one_span() {
        let sample_rate = 16_000u32;
        let spw = (sample_rate / 100) as usize;
        let mut pcm = Vec::new();
        for w in 0..10u32 {
            let sample: i16 = if w % 2 == 0 { 20_000 } else { 0 };
            for _ in 0..spw {
                pcm.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let spans = detect_speech_spans(&pcm, sample_rate);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start.as_i64(), 0);
        assert_eq!(spans[0].end.as_i64(), 100);
    }

    #[test]
    fn two_loud_bursts_separated_by_silence() {
        let sample_rate = 16_000u32;
        let spw = (sample_rate / 100) as usize;
        let mut pcm = Vec::new();
        for w in 0..10u32 {
            let sample: i16 = if !(3..7).contains(&w) { 20_000 } else { 0 };
            for _ in 0..spw {
                pcm.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let spans = detect_speech_spans(&pcm, sample_rate);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].start.as_i64(), 0);
        assert_eq!(spans[0].end.as_i64(), 30);
        assert_eq!(spans[1].start.as_i64(), 70);
        assert_eq!(spans[1].end.as_i64(), 100);
    }

    #[test]
    fn empty_pcm_no_spans() {
        assert!(detect_speech_spans(&[], 16_000).is_empty());
    }

    #[test]
    fn near_threshold_audio() {
        let sample_rate = 16_000u32;
        let spw = (sample_rate / 100) as usize;

        let mut pcm_at = Vec::new();
        for _ in 0..spw {
            pcm_at.extend_from_slice(&500i16.to_le_bytes());
        }
        assert!(detect_speech_spans(&pcm_at, sample_rate).is_empty());

        let mut pcm_above = Vec::new();
        for _ in 0..(spw * 2) {
            pcm_above.extend_from_slice(&501i16.to_le_bytes());
        }
        assert_eq!(detect_speech_spans(&pcm_above, sample_rate).len(), 1);
    }

    #[test]
    fn charset_detection_utf8_passthrough() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "1\n00:00:01,000 --> 00:00:02,000\nHello\n").unwrap();
        let content = read_subtitle_to_string(tmp.path()).unwrap();
        assert!(content.contains("Hello"));
    }

    #[test]
    fn charset_detection_latin1() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut data = b"1\n00:00:01,000 --> 00:00:02,000\ncaf".to_vec();
        data.push(0xe9);
        data.push(b'\n');
        std::fs::write(tmp.path(), &data).unwrap();
        let content = read_subtitle_to_string(tmp.path()).unwrap();
        assert!(
            content.contains("caf"),
            "should contain 'caf' after charset conversion"
        );
    }

    #[test]
    fn ass_spans_extract_dialogue_lines_only() {
        let content = "[Script Info]\n\
Title: Demo\n\
\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Comment: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,ignored\n\
Dialogue: 0,0:00:03.00,0:00:05.00,Default,,0,0,0,,Hello\n";
        let spans = read_ass_spans_from_str(content);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start.as_i64(), 3_000);
        assert_eq!(spans[0].end.as_i64(), 5_000);
    }

    #[test]
    fn shift_ass_content_rewrites_event_times_and_preserves_text_with_commas() {
        let content = "[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:03.00,0:00:05.00,Default,,0,0,0,,Hello, world\n\
Comment: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,note\n";
        let shifted = shift_ass_content(content, 1_500);
        assert!(shifted.contains("Dialogue: 0,0:00:04.50,0:00:06.50,Default,,0,0,0,,Hello, world"));
        assert!(shifted.contains("Comment: 0,0:00:02.50,0:00:03.50,Default,,0,0,0,,note"));
    }

    #[test]
    fn delta_consistency_ratio_counts_clustered_offsets() {
        let clustered = vec![
            TimeDelta::from_i64(1_000),
            TimeDelta::from_i64(1_050),
            TimeDelta::from_i64(900),
            TimeDelta::from_i64(1_320),
        ];
        let inconsistent = vec![
            TimeDelta::from_i64(1_000),
            TimeDelta::from_i64(1_500),
            TimeDelta::from_i64(2_100),
        ];

        assert!(delta_consistency_ratio(&clustered, 1_000) > 0.7);
        assert!(delta_consistency_ratio(&inconsistent, 1_000) < 0.5);
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
