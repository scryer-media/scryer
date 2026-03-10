use serde::{Deserialize, Serialize};
use std::path::Path;

mod avi;
mod codec;
mod mkv;
mod mp4;
mod ts;
mod types;

use types::{RawContainer, RawTrack, TrackKind};

// ---------------------------------------------------------------------------
// Public error type
// ---------------------------------------------------------------------------

/// Errors from native container/codec parsing.
#[derive(Debug, thiserror::Error)]
pub enum MediaInfoError {
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("parse error: {0}")]
    Parse(String),
}

impl From<std::io::Error> for MediaInfoError {
    fn from(e: std::io::Error) -> Self {
        MediaInfoError::Io(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Public types (unchanged from ffprobe era)
// ---------------------------------------------------------------------------

/// A single audio stream extracted from media analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStreamDetail {
    pub codec: Option<String>,
    pub channels: Option<i32>,
    pub language: Option<String>,
    pub bitrate_kbps: Option<i32>,
}

/// A single subtitle stream extracted from media analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleStreamDetail {
    pub codec: Option<String>,
    pub language: Option<String>,
    pub name: Option<String>,
    pub forced: bool,
    pub default: bool,
}

/// Parsed media properties.
#[derive(Debug, Clone)]
pub struct MediaAnalysis {
    pub video_codec: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub video_bitrate_kbps: Option<i32>,
    pub video_bit_depth: Option<i32>,
    /// "Dolby Vision", "HDR10+", "HDR10", or "HLG"
    pub video_hdr_format: Option<String>,
    /// Dolby Vision profile number (5, 7, 8, etc.) if DV is detected
    pub dovi_profile: Option<u8>,
    /// Dolby Vision base-layer signal compatibility ID
    pub dovi_bl_compat_id: Option<u8>,
    /// Frame rate as a decimal string, e.g. "23.976", "24", "60"
    pub video_frame_rate: Option<String>,
    /// Codec profile, e.g. "Main 10", "High", "Main"
    pub video_profile: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<i32>,
    /// Bitrate of the primary audio stream in kbps
    pub audio_bitrate_kbps: Option<i32>,
    /// Language tags from all audio streams (BCP-47 / ISO 639-2), "und" filtered out
    pub audio_languages: Vec<String>,
    /// All audio streams with per-stream details
    pub audio_streams: Vec<AudioStreamDetail>,
    /// Language tags from all subtitle streams
    pub subtitle_languages: Vec<String>,
    /// Codec names for all subtitle streams
    pub subtitle_codecs: Vec<String>,
    /// All subtitle streams with per-stream details
    pub subtitle_streams: Vec<SubtitleStreamDetail>,
    pub has_multiaudio: bool,
    pub duration_seconds: Option<i32>,
    pub container_format: Option<String>,
    /// Structured JSON representation of the analysis
    pub raw_json: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns `true` if the analysis describes a valid video file (has a video
/// stream and non-zero duration). Returns `false` for executables, audio-only
/// files, corrupt containers, etc.
pub fn is_valid_video(analysis: &MediaAnalysis) -> bool {
    analysis.video_codec.is_some() && analysis.duration_seconds.map(|d| d > 0).unwrap_or(false)
}

/// Analyzes a media file using pure Rust container parsers. Dispatches to the
/// appropriate parser based on file extension.
pub fn analyze_file(file_path: &Path) -> Result<MediaAnalysis, MediaInfoError> {
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    let raw = match ext.as_str() {
        "mkv" | "webm" => mkv::parse_mkv(file_path)?,
        "mp4" | "m4v" | "mov" => mp4::parse_mp4(file_path)?,
        "avi" => avi::parse_avi(file_path)?,
        "ts" | "m2ts" => ts::parse_ts(file_path)?,
        _ => return Err(MediaInfoError::UnsupportedFormat(ext)),
    };

    Ok(build_analysis(raw))
}

// ---------------------------------------------------------------------------
// Internal: convert RawContainer → MediaAnalysis
// ---------------------------------------------------------------------------

fn build_analysis(raw: RawContainer) -> MediaAnalysis {
    let video_track = raw.tracks.iter().find(|t| t.kind == TrackKind::Video);
    let audio_tracks: Vec<&RawTrack> = raw
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .collect();
    let subtitle_tracks: Vec<&RawTrack> = raw
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Subtitle)
        .collect();

    // --- Video ---
    let video_codec = video_track.and_then(|t| t.codec_name.clone());
    let video_width = video_track.and_then(|t| t.width);
    let video_height = video_track.and_then(|t| t.height);
    let video_bitrate_kbps = video_track
        .and_then(|t| t.bit_rate_bps)
        .map(|bps| (bps / 1000) as i32);

    // Extract profile + bit depth from codec private data
    let codec_info = video_track.map(extract_codec_info).unwrap_or_default();

    let video_bit_depth = codec_info.bit_depth;
    let video_profile = codec_info.profile;
    // Try container-level HDR detection first; fall back to bitstream VUI
    // color_transfer (e.g. HEVC SPS) when the container doesn't carry it.
    let video_hdr_format = video_track.and_then(codec::detect_hdr_format).or_else(|| {
        codec_info.color_transfer.and_then(|ct| match ct {
            16 => Some("HDR10".into()),
            18 => Some("HLG".into()),
            _ => None,
        })
    });

    // Parse Dolby Vision config record for profile details.
    let dovi_info = video_track
        .and_then(|t| t.dovi_config.as_deref())
        .and_then(codec::parse_dovi_config);
    let dovi_profile = dovi_info.as_ref().map(|d| d.profile);
    let dovi_bl_compat_id = dovi_info.as_ref().map(|d| d.bl_signal_compatibility_id);

    let video_frame_rate = video_track.and_then(|t| t.frame_rate_fps).and_then(|fps| {
        if fps <= 0.0 {
            return None;
        }
        let s = format!("{fps:.3}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        Some(s.to_owned())
    });

    // --- Audio ---
    let primary_audio = audio_tracks.first().copied();
    let audio_codec = primary_audio.and_then(|t| t.codec_name.clone());
    let audio_channels = primary_audio.and_then(|t| t.channels);
    let audio_bitrate_kbps = primary_audio
        .and_then(|t| t.bit_rate_bps)
        .map(|bps| (bps / 1000) as i32);

    let audio_languages: Vec<String> = audio_tracks
        .iter()
        .filter_map(|t| t.language.as_deref())
        .filter(|l| !l.is_empty() && *l != "und")
        .map(str::to_owned)
        .collect();

    let audio_streams: Vec<AudioStreamDetail> = audio_tracks
        .iter()
        .map(|t| AudioStreamDetail {
            codec: t.codec_name.clone(),
            channels: t.channels,
            language: t
                .language
                .as_deref()
                .filter(|l| !l.is_empty() && *l != "und")
                .map(str::to_owned),
            bitrate_kbps: t.bit_rate_bps.map(|bps| (bps / 1000) as i32),
        })
        .collect();

    let has_multiaudio = audio_tracks.len() > 1;

    // --- Subtitles ---
    let subtitle_languages: Vec<String> = subtitle_tracks
        .iter()
        .filter_map(|t| t.language.as_deref())
        .filter(|l| !l.is_empty() && *l != "und")
        .map(str::to_owned)
        .collect();

    let subtitle_codecs: Vec<String> = subtitle_tracks
        .iter()
        .filter_map(|t| t.codec_name.clone())
        .collect();

    let subtitle_streams: Vec<SubtitleStreamDetail> = subtitle_tracks
        .iter()
        .map(|t| SubtitleStreamDetail {
            codec: t.codec_name.clone(),
            language: t
                .language
                .as_deref()
                .filter(|l| !l.is_empty() && *l != "und")
                .map(str::to_owned),
            name: t.name.clone(),
            forced: t.forced,
            default: t.default_track,
        })
        .collect();

    // --- Container ---
    let duration_seconds = raw.duration_seconds.map(|d| d as i32);
    let container_format = Some(raw.format_name.clone());

    // --- Structured JSON (replaces ffprobe raw JSON) ---
    let raw_json = build_raw_json(&raw);

    MediaAnalysis {
        video_codec,
        video_width,
        video_height,
        video_bitrate_kbps,
        video_bit_depth,
        video_hdr_format,
        dovi_profile,
        dovi_bl_compat_id,
        video_frame_rate,
        video_profile,
        audio_codec,
        audio_channels,
        audio_bitrate_kbps,
        audio_languages,
        audio_streams,
        subtitle_languages,
        subtitle_codecs,
        subtitle_streams,
        has_multiaudio,
        duration_seconds,
        container_format,
        raw_json,
    }
}

/// Dispatch to the right codec extractor based on normalized codec name.
fn extract_codec_info(track: &RawTrack) -> codec::CodecInfo {
    let codec_name = track.codec_name.as_deref().unwrap_or("");
    match codec_name {
        "h264" => track
            .codec_private
            .as_deref()
            .map(codec::extract_h264_info)
            .unwrap_or_default(),
        "hevc" => track
            .codec_private
            .as_deref()
            .map(codec::extract_h265_info)
            .unwrap_or_default(),
        "av1" => track
            .codec_private
            .as_deref()
            .map(codec::extract_av1_info)
            .unwrap_or_default(),
        _ => codec::CodecInfo::default(),
    }
}

/// Serialize the raw container data into a structured JSON string.
fn build_raw_json(raw: &RawContainer) -> String {
    #[derive(Serialize)]
    struct JsonAnalysis<'a> {
        format: &'a str,
        duration_seconds: Option<f64>,
        tracks: Vec<JsonTrack<'a>>,
    }

    #[derive(Serialize)]
    struct JsonTrack<'a> {
        kind: &'a str,
        codec_id: &'a str,
        codec_name: Option<&'a str>,
        width: Option<i32>,
        height: Option<i32>,
        channels: Option<i32>,
        bit_rate_bps: Option<i64>,
        language: Option<&'a str>,
        frame_rate_fps: Option<f64>,
    }

    let analysis = JsonAnalysis {
        format: &raw.format_name,
        duration_seconds: raw.duration_seconds,
        tracks: raw
            .tracks
            .iter()
            .map(|t| JsonTrack {
                kind: match t.kind {
                    TrackKind::Video => "video",
                    TrackKind::Audio => "audio",
                    TrackKind::Subtitle => "subtitle",
                },
                codec_id: &t.codec_id,
                codec_name: t.codec_name.as_deref(),
                width: t.width,
                height: t.height,
                channels: t.channels,
                bit_rate_bps: t.bit_rate_bps,
                language: t.language.as_deref(),
                frame_rate_fps: t.frame_rate_fps,
            })
            .collect(),
    };

    serde_json::to_string(&analysis).unwrap_or_else(|_| "{}".to_string())
}
