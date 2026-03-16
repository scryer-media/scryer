//! Subtitle timing synchronization using alass-core.
//!
//! Extracts audio from a video file via ffmpeg, then uses the alass algorithm
//! to compute the optimal time offset to align subtitle timestamps with the audio.

use std::path::Path;

use crate::{AppError, AppResult};
use alass_core::{NoProgressHandler, TimePoint, TimeSpan};

/// Result of a subtitle sync operation.
#[derive(Debug, Clone)]
pub struct SyncResult {
    /// Time offset applied in milliseconds.
    pub offset_ms: i64,
    /// Whether the sync was applied.
    pub applied: bool,
}

/// Synchronize a subtitle file with a video file's audio track.
///
/// Uses ffmpeg to extract raw audio, then alass-core to compute alignment.
pub async fn sync_subtitle(
    video_path: &Path,
    subtitle_path: &Path,
    max_offset_seconds: i64,
) -> AppResult<SyncResult> {
    let reference_spans = extract_audio_speech_spans(video_path).await?;
    if reference_spans.is_empty() {
        return Ok(SyncResult {
            offset_ms: 0,
            applied: false,
        });
    }

    let subtitle_spans = read_srt_spans(subtitle_path)?;
    if subtitle_spans.is_empty() {
        return Ok(SyncResult {
            offset_ms: 0,
            applied: false,
        });
    }

    // alass alignment is CPU-intensive — run on the blocking pool
    let (deltas, _score) = tokio::task::spawn_blocking(move || {
        alass_core::align(
            &reference_spans,
            &subtitle_spans,
            7.0,
            None,
            alass_core::standard_scoring,
            NoProgressHandler,
        )
    })
    .await
    .map_err(|e| AppError::Repository(format!("alass task panicked: {e}")))?;

    // Use the median offset as the global shift
    let offset = if deltas.is_empty() {
        0i64
    } else {
        let mut offsets: Vec<i64> = deltas.iter().map(|d| d.as_i64()).collect();
        offsets.sort();
        offsets[offsets.len() / 2]
    };

    if offset.unsigned_abs() > (max_offset_seconds as u64 * 1000) {
        tracing::warn!(
            offset_ms = offset,
            max_offset_seconds,
            "sync offset exceeds max, skipping"
        );
        return Ok(SyncResult {
            offset_ms: offset,
            applied: false,
        });
    }

    if offset == 0 {
        return Ok(SyncResult {
            offset_ms: 0,
            applied: false,
        });
    }

    apply_srt_offset(subtitle_path, offset)?;

    tracing::info!(path = %subtitle_path.display(), offset_ms = offset, "subtitle synchronized");
    Ok(SyncResult {
        offset_ms: offset,
        applied: true,
    })
}

// ── Audio extraction via symphonia (pure Rust, no external deps) ────────────

/// Extract speech spans from a video file's audio track using symphonia.
///
/// Decodes the first audio track to raw PCM samples, then runs energy-based
/// VAD to produce speech/silence time spans for alass alignment.
/// No external binaries needed — symphonia handles MKV/MP4/AAC/FLAC/MP3/etc.
async fn extract_audio_speech_spans(video_path: &Path) -> AppResult<Vec<TimeSpan>> {
    let path = video_path.to_path_buf();

    // Audio decoding is CPU-bound — run on blocking pool
    tokio::task::spawn_blocking(move || decode_audio_to_speech_spans(&path))
        .await
        .map_err(|e| AppError::Repository(format!("audio decode task panicked: {e}")))?
}

/// Decode the first audio track and produce speech spans via energy VAD.
fn decode_audio_to_speech_spans(path: &Path) -> AppResult<Vec<TimeSpan>> {
    use symphonia::core::codecs::DecoderOptions;
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

    // Find the first audio track
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| AppError::Repository("no audio track found".into()))?;

    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| AppError::Repository(format!("audio decoder init failed: {e}")))?;

    // VAD parameters: 10ms windows at the decoded sample rate
    let samples_per_window = (sample_rate / 100) as usize;
    let energy_threshold: f64 = 500.0;

    let mut spans = Vec::new();
    let mut in_speech = false;
    let mut speech_start_ms = 0i64;
    let mut window_buf: Vec<f64> = Vec::with_capacity(samples_per_window);
    let mut total_samples: u64 = 0;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
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
            Ok(d) => d,
            Err(_) => continue,
        };

        // Convert to mono f64 samples — take only the first channel
        let spec = *decoded.spec();
        let num_frames = decoded.frames();
        let channels = spec.channels.count().max(1);

        // Use AudioBuffer to get interleaved samples
        let mut sample_buf =
            symphonia::core::audio::SampleBuffer::<i16>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        let samples = sample_buf.samples();

        // Extract first channel only (stride by channel count)
        for i in (0..samples.len()).step_by(channels) {
            window_buf.push(samples[i] as f64);
            total_samples += 1;

            if window_buf.len() >= samples_per_window {
                // Compute RMS energy for this window
                let sum_sq: f64 = window_buf.iter().map(|s| s * s).sum();
                let rms = (sum_sq / samples_per_window as f64).sqrt();
                let is_speech = rms > energy_threshold;

                let current_ms = ((total_samples - samples_per_window as u64) * 1000
                    / sample_rate as u64) as i64;

                if is_speech && !in_speech {
                    speech_start_ms = current_ms;
                    in_speech = true;
                } else if !is_speech && in_speech {
                    spans.push(TimeSpan::new(
                        TimePoint::from(speech_start_ms),
                        TimePoint::from(current_ms),
                    ));
                    in_speech = false;
                }

                window_buf.clear();
            }
        }
    }

    // Close any open span
    if in_speech {
        let end_ms = (total_samples * 1000 / sample_rate as u64) as i64;
        spans.push(TimeSpan::new(
            TimePoint::from(speech_start_ms),
            TimePoint::from(end_ms),
        ));
    }

    Ok(spans)
}

// ── SRT parsing with charset detection ──────────────────────────────────────

/// Read a subtitle file, auto-detecting charset (handles Windows-1252,
/// ISO-8859-1, etc. common in wild .srt files from OpenSubtitles).
fn read_srt_to_string(path: &Path) -> AppResult<String> {
    let bytes =
        std::fs::read(path).map_err(|e| AppError::Repository(format!("cannot read srt: {e}")))?;

    // Try UTF-8 first (fast path)
    if let Ok(s) = std::str::from_utf8(&bytes) {
        return Ok(s.to_string());
    }

    // Detect charset
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

/// Parse SRT timestamps into alass TimeSpans with charset detection.
fn read_srt_spans(srt_path: &Path) -> AppResult<Vec<TimeSpan>> {
    let content = read_srt_to_string(srt_path)?;
    let mut spans = Vec::new();
    for line in content.lines() {
        if let Some((start, end)) = line.split_once("-->")
            && let (Some(s), Some(e)) = (parse_srt_ts(start.trim()), parse_srt_ts(end.trim()))
        {
            spans.push(TimeSpan::new(TimePoint::from(s), TimePoint::from(e)));
        }
    }
    Ok(spans)
}

// ── SRT timestamp parsing and formatting ────────────────────────────────────

fn parse_srt_ts(ts: &str) -> Option<i64> {
    let p: Vec<&str> = ts.split([':', ',', '.']).collect();
    if p.len() < 4 {
        return None;
    }
    let h: i64 = p[0].trim().parse().ok()?;
    let m: i64 = p[1].trim().parse().ok()?;
    let s: i64 = p[2].trim().parse().ok()?;
    let ms: i64 = p[3].trim().parse().ok()?;
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

fn apply_srt_offset(srt_path: &Path, offset_ms: i64) -> AppResult<()> {
    let content = read_srt_to_string(srt_path)?;

    let mut out = String::with_capacity(content.len());
    for line in content.lines() {
        if let Some((start_str, end_str)) = line.split_once("-->")
            && let (Some(s), Some(e)) =
                (parse_srt_ts(start_str.trim()), parse_srt_ts(end_str.trim()))
        {
            out.push_str(&format_srt_ts(s + offset_ms));
            out.push_str(" --> ");
            out.push_str(&format_srt_ts(e + offset_ms));
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }

    // Always write back as UTF-8
    std::fs::write(srt_path, out)
        .map_err(|e| AppError::Repository(format!("cannot write srt: {e}")))?;
    Ok(())
}

// ── Standalone VAD for testing ──────────────────────────────────────────────

/// Simple energy-based VAD on raw 16-bit mono PCM (used by tests and
/// the streaming extractor above).
#[cfg(test)]
fn detect_speech_spans(pcm: &[u8], sample_rate: u32) -> Vec<TimeSpan> {
    let samples_per_window = (sample_rate / 100) as usize;
    let bytes_per_window = samples_per_window * 2;
    let energy_threshold: f64 = 500.0;

    let mut spans = Vec::new();
    let mut in_speech = false;
    let mut speech_start_ms = 0i64;

    for (i, chunk) in pcm.chunks(bytes_per_window).enumerate() {
        if chunk.len() < bytes_per_window {
            break;
        }

        let mut sum_sq: f64 = 0.0;
        for s in chunk.chunks_exact(2) {
            let sample = i16::from_le_bytes([s[0], s[1]]) as f64;
            sum_sq += sample * sample;
        }
        let rms = (sum_sq / samples_per_window as f64).sqrt();
        let is_speech = rms > energy_threshold;
        let current_ms = (i * 10) as i64;

        if is_speech && !in_speech {
            speech_start_ms = current_ms;
            in_speech = true;
        } else if !is_speech && in_speech {
            spans.push(TimeSpan::new(
                TimePoint::from(speech_start_ms),
                TimePoint::from(current_ms),
            ));
            in_speech = false;
        }
    }

    if in_speech {
        let end_ms = (pcm.len() / bytes_per_window * 10) as i64;
        spans.push(TimeSpan::new(
            TimePoint::from(speech_start_ms),
            TimePoint::from(end_ms),
        ));
    }

    spans
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
    fn silent_audio_no_spans() {
        let silent = vec![0u8; 32000];
        assert!(detect_speech_spans(&silent, 16000).is_empty());
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
        let sample_rate = 16000u32;
        let spw = (sample_rate / 100) as usize;
        let num_windows = 50;
        let mut pcm = Vec::with_capacity(spw * num_windows * 2);
        for _ in 0..(spw * num_windows) {
            pcm.extend_from_slice(&32767i16.to_le_bytes());
        }
        let spans = detect_speech_spans(&pcm, sample_rate);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn alternating_loud_silent_windows() {
        let sample_rate = 16000u32;
        let spw = (sample_rate / 100) as usize;
        let mut pcm = Vec::new();
        for w in 0..10u32 {
            let sample: i16 = if w % 2 == 0 { 20000 } else { 0 };
            for _ in 0..spw {
                pcm.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let spans = detect_speech_spans(&pcm, sample_rate);
        assert_eq!(spans.len(), 5);
    }

    #[test]
    fn two_loud_bursts_separated_by_silence() {
        let sample_rate = 16000u32;
        let spw = (sample_rate / 100) as usize;
        let mut pcm = Vec::new();
        for w in 0..10u32 {
            let sample: i16 = if w < 3 || w >= 7 { 20000 } else { 0 };
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
        assert!(detect_speech_spans(&[], 16000).is_empty());
    }

    #[test]
    fn near_threshold_audio() {
        let sample_rate = 16000u32;
        let spw = (sample_rate / 100) as usize;

        let mut pcm_at = Vec::new();
        for _ in 0..spw {
            pcm_at.extend_from_slice(&500i16.to_le_bytes());
        }
        assert!(detect_speech_spans(&pcm_at, sample_rate).is_empty());

        let mut pcm_above = Vec::new();
        for _ in 0..spw {
            pcm_above.extend_from_slice(&501i16.to_le_bytes());
        }
        assert_eq!(detect_speech_spans(&pcm_above, sample_rate).len(), 1);
    }

    #[test]
    fn charset_detection_utf8_passthrough() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "1\n00:00:01,000 --> 00:00:02,000\nHello\n").unwrap();
        let content = read_srt_to_string(tmp.path()).unwrap();
        assert!(content.contains("Hello"));
    }

    #[test]
    fn charset_detection_latin1() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // Latin-1 encoded: "café" = [99, 97, 102, 0xe9]
        let mut data = b"1\n00:00:01,000 --> 00:00:02,000\ncaf".to_vec();
        data.push(0xe9); // é in Latin-1
        data.push(b'\n');
        std::fs::write(tmp.path(), &data).unwrap();
        let content = read_srt_to_string(tmp.path()).unwrap();
        assert!(
            content.contains("caf"),
            "should contain 'caf' after charset conversion"
        );
    }
}
