use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;

mod avi;
mod codec;
mod mkv;
mod mp4;
mod probe;
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
    pub num_chapters: Option<i32>,
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
/// appropriate parser based on container sniffing with an extension fallback.
pub fn analyze_file(file_path: &Path) -> Result<MediaAnalysis, MediaInfoError> {
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    let format = sniff_container_format(file_path).or(match ext.as_str() {
        "mkv" | "webm" => Some(ContainerFormat::Matroska),
        "mp4" | "m4v" | "mov" => Some(ContainerFormat::Mp4),
        "avi" => Some(ContainerFormat::Avi),
        "ts" | "m2ts" => Some(ContainerFormat::Ts),
        _ => None,
    });

    let raw = match format {
        Some(ContainerFormat::Matroska) => mkv::parse_mkv(file_path)?,
        Some(ContainerFormat::Mp4) => mp4::parse_mp4(file_path)?,
        Some(ContainerFormat::Avi) => avi::parse_avi(file_path)?,
        Some(ContainerFormat::Ts) => ts::parse_ts(file_path)?,
        None => return Err(MediaInfoError::UnsupportedFormat(ext)),
    };

    Ok(build_analysis(raw))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContainerFormat {
    Matroska,
    Mp4,
    Avi,
    Ts,
}

fn sniff_container_format(file_path: &Path) -> Option<ContainerFormat> {
    let mut file = std::fs::File::open(file_path).ok()?;
    let mut header = [0_u8; 564];
    let bytes_read = file.read(&mut header).ok()?;
    sniff_container_format_from_bytes(&header[..bytes_read])
}

fn sniff_container_format_from_bytes(data: &[u8]) -> Option<ContainerFormat> {
    if data.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(ContainerFormat::Matroska);
    }

    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"AVI " {
        return Some(ContainerFormat::Avi);
    }

    if looks_like_transport_stream(data) {
        return Some(ContainerFormat::Ts);
    }

    if looks_like_mp4(data) {
        return Some(ContainerFormat::Mp4);
    }

    None
}

fn looks_like_transport_stream(data: &[u8]) -> bool {
    const TS_PACKET_SIZE: usize = 188;

    [0_usize, 4].into_iter().any(|offset| {
        data.len() > offset + TS_PACKET_SIZE * 2 && {
            data[offset] == 0x47
                && data[offset + TS_PACKET_SIZE] == 0x47
                && data[offset + TS_PACKET_SIZE * 2] == 0x47
        }
    })
}

fn looks_like_mp4(data: &[u8]) -> bool {
    if data.len() < 8 {
        return false;
    }

    let name = &data[4..8];
    let printable_name = name.iter().all(u8::is_ascii_alphanumeric)
        || matches!(name, b"ac-3" | b"ec-3" | b"mp4a" | b".mp3");

    printable_name
        && matches!(
            name,
            b"ftyp" | b"moov" | b"moof" | b"mdat" | b"free" | b"skip" | b"wide" | b"styp"
        )
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
    let primary_audio = select_primary_audio_track(&audio_tracks);
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
    let num_chapters = raw.num_chapters;
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
        num_chapters,
        container_format,
        raw_json,
    }
}

fn select_primary_audio_track<'a>(audio_tracks: &[&'a RawTrack]) -> Option<&'a RawTrack> {
    audio_tracks
        .iter()
        .find(|track| track.default_track)
        .copied()
        .or_else(|| audio_tracks.first().copied())
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
        num_chapters: Option<i32>,
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
        num_chapters: raw.num_chapters,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_prefers_default_audio_track_for_primary_fields() {
        let analysis = build_analysis(RawContainer {
            format_name: "matroska".into(),
            duration_seconds: Some(60.0),
            num_chapters: Some(0),
            tracks: vec![
                RawTrack {
                    kind: TrackKind::Video,
                    codec_id: "V_MPEG4/ISO/AVC".into(),
                    codec_name: Some("h264".into()),
                    codec_private: None,
                    width: Some(1920),
                    height: Some(1080),
                    channels: None,
                    bit_rate_bps: Some(8_000_000),
                    language: None,
                    name: None,
                    forced: false,
                    default_track: false,
                    frame_rate_fps: Some(24.0),
                    color_transfer: None,
                    dovi_config: None,
                    has_hdr10plus: false,
                },
                RawTrack {
                    kind: TrackKind::Audio,
                    codec_id: "A_AAC".into(),
                    codec_name: Some("aac".into()),
                    codec_private: None,
                    width: None,
                    height: None,
                    channels: Some(2),
                    bit_rate_bps: Some(128_000),
                    language: Some("eng".into()),
                    name: None,
                    forced: false,
                    default_track: false,
                    frame_rate_fps: None,
                    color_transfer: None,
                    dovi_config: None,
                    has_hdr10plus: false,
                },
                RawTrack {
                    kind: TrackKind::Audio,
                    codec_id: "A_FLAC".into(),
                    codec_name: Some("flac".into()),
                    codec_private: None,
                    width: None,
                    height: None,
                    channels: Some(6),
                    bit_rate_bps: Some(640_000),
                    language: Some("jpn".into()),
                    name: None,
                    forced: false,
                    default_track: true,
                    frame_rate_fps: None,
                    color_transfer: None,
                    dovi_config: None,
                    has_hdr10plus: false,
                },
            ],
        });

        assert_eq!(analysis.audio_codec.as_deref(), Some("flac"));
        assert_eq!(analysis.audio_channels, Some(6));
        assert_eq!(analysis.audio_bitrate_kbps, Some(640));
    }

    #[test]
    fn sniff_container_format_prefers_matroska_magic_over_extension_hint() {
        assert_eq!(
            sniff_container_format_from_bytes(&[0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0]),
            Some(ContainerFormat::Matroska)
        );
    }

    #[test]
    fn sniff_container_format_detects_avi_and_transport_stream() {
        assert_eq!(
            sniff_container_format_from_bytes(b"RIFF\0\0\0\0AVI LIST"),
            Some(ContainerFormat::Avi)
        );

        let mut ts = vec![0_u8; 564];
        ts[0] = 0x47;
        ts[188] = 0x47;
        ts[376] = 0x47;
        assert_eq!(
            sniff_container_format_from_bytes(&ts),
            Some(ContainerFormat::Ts)
        );
    }

    #[test]
    fn sniff_container_format_detects_mp4_box_headers() {
        let mut bytes = vec![0_u8; 16];
        bytes[..4].copy_from_slice(&16_u32.to_be_bytes());
        bytes[4..8].copy_from_slice(b"ftyp");
        assert_eq!(
            sniff_container_format_from_bytes(&bytes),
            Some(ContainerFormat::Mp4)
        );
    }
}
