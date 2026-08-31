use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;

mod asf;
mod avi;
mod codec;
mod flv;
mod mkv;
mod mp4;
mod ogg;
mod probe;
mod scan;
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

/// Analysis behavior profile for media probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisProfile {
    /// Minimal bounded analysis needed to identify a playable video file.
    /// This avoids optional chapter, frame-rate, HDR, and audio enrichment while
    /// retaining the bounded container and duration work needed for validation.
    ContentProbe,
    /// Fast bounded metadata pass. This avoids payload/sample deep probes and
    /// leaves richer confirmation to callers that need it.
    Fast,
    /// Preserve the richer native analyzer behavior, including bounded deep
    /// scans for metadata such as HDR10+ where cheaper signals justify it.
    DefaultRich,
    /// Favor parity with Sonarr's bundled ffprobe workflow: a stream/format
    /// analysis pass with larger probe budgets when needed, plus a cheap
    /// first-frame HDR follow-up for PQ video instead of richer native scans.
    FfprobeParity,
}

impl AnalysisProfile {
    pub(crate) fn skips_deep_probes(self) -> bool {
        matches!(self, Self::ContentProbe | Self::Fast)
    }
}

/// Options that control media analysis behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalyzeOptions {
    pub profile: AnalysisProfile,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            profile: AnalysisProfile::Fast,
        }
    }
}

// ---------------------------------------------------------------------------
// Public types (unchanged from ffprobe era)
// ---------------------------------------------------------------------------

/// A single audio stream extracted from media analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStreamDetail {
    pub codec: Option<String>,
    pub profile: Option<String>,
    pub channels: Option<i32>,
    pub language: Option<String>,
    /// Human-readable track title from the container (e.g. "English", "日本語").
    /// Often set by uploaders even when the ISO language tag is missing/`und`.
    pub name: Option<String>,
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
    pub audio_profile: Option<String>,
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
    analyze_file_with_options(file_path, AnalyzeOptions::default())
}

/// Analyzes a media file with the requested analysis behavior profile.
pub fn analyze_file_with_options(
    file_path: &Path,
    options: AnalyzeOptions,
) -> Result<MediaAnalysis, MediaInfoError> {
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    let format = resolve_container_format(&ext, sniff_container_format(file_path));

    let raw = match format {
        Some(ContainerFormat::Matroska) => {
            let profile = if options.profile == AnalysisProfile::Fast {
                AnalysisProfile::DefaultRich
            } else {
                options.profile
            };
            mkv::parse_mkv(file_path, profile)?
        }
        Some(ContainerFormat::Mp4) => mp4::parse_mp4(file_path, options.profile)?,
        Some(ContainerFormat::Avi) => avi::parse_avi(file_path, options.profile)?,
        Some(ContainerFormat::Ts) => ts::parse_ts(file_path, options.profile)?,
        Some(ContainerFormat::Asf) => asf::parse_asf(file_path)?,
        Some(ContainerFormat::Ogg) => ogg::parse_ogg(file_path)?,
        Some(ContainerFormat::Flv) => flv::parse_flv(file_path)?,
        None => return Err(MediaInfoError::UnsupportedFormat(ext)),
    };

    Ok(build_analysis(raw))
}

fn resolve_container_format(
    ext: &str,
    sniffed: Option<ContainerFormat>,
) -> Option<ContainerFormat> {
    sniffed.or_else(|| container_format_from_extension(ext))
}

fn container_format_from_extension(ext: &str) -> Option<ContainerFormat> {
    match ext {
        "mkv" | "webm" => Some(ContainerFormat::Matroska),
        "mp4" | "m4v" | "mov" => Some(ContainerFormat::Mp4),
        "avi" => Some(ContainerFormat::Avi),
        "ts" | "m2ts" => Some(ContainerFormat::Ts),
        "wmv" => Some(ContainerFormat::Asf),
        "ogv" => Some(ContainerFormat::Ogg),
        "flv" => Some(ContainerFormat::Flv),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContainerFormat {
    Matroska,
    Mp4,
    Avi,
    Ts,
    Asf,
    Ogg,
    Flv,
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

    if data.starts_with(&asf::ASF_HEADER_GUID) {
        return Some(ContainerFormat::Asf);
    }

    if data.len() >= 5 && &data[..4] == b"OggS" && data[4] == 0 {
        return Some(ContainerFormat::Ogg);
    }

    if data.len() >= 9 && &data[..3] == b"FLV" && data[3] < 5 {
        let data_offset = u32::from_be_bytes([data[5], data[6], data[7], data[8]]);
        if data_offset >= 9 {
            return Some(ContainerFormat::Flv);
        }
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
    let video_tracks: Vec<&RawTrack> = raw
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .collect();
    let video_track = select_primary_video_track(&video_tracks);
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
    let video_bitrate_kbps = video_track
        .and_then(|t| t.bit_rate_bps)
        .map(|bps| (bps / 1000) as i32);

    // Extract profile + bit depth from codec private data
    let codec_info = video_track.map(extract_codec_info).unwrap_or_default();
    let video_width = video_track
        .and_then(|track| track.width)
        .or(codec_info.width);
    let video_height = video_track
        .and_then(|track| track.height)
        .or(codec_info.height);

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

    let video_frame_rate = video_track
        .and_then(|track| track.frame_rate_fps)
        .or(codec_info.frame_rate_fps)
        .and_then(|fps| {
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
    let audio_profile = primary_audio.and_then(|t| t.audio_profile.clone());
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
            profile: t.audio_profile.clone(),
            channels: t.channels,
            language: t
                .language
                .as_deref()
                .filter(|l| !l.is_empty() && *l != "und")
                .map(str::to_owned),
            name: t.name.clone(),
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
    let duration_seconds = raw.duration_seconds.map(|d| d.round() as i32);
    let num_chapters = raw.num_chapters;
    let container_format = Some(raw.format_name.clone());

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
        audio_profile,
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
    }
}

fn select_primary_audio_track<'a>(audio_tracks: &[&'a RawTrack]) -> Option<&'a RawTrack> {
    audio_tracks.first().copied()
}

fn select_primary_video_track<'a>(video_tracks: &[&'a RawTrack]) -> Option<&'a RawTrack> {
    if video_tracks.len() <= 1 {
        return video_tracks.first().copied();
    }

    video_tracks
        .iter()
        .copied()
        .find(|track| !matches!(track.codec_name.as_deref(), Some("mjpeg" | "png")))
        .or_else(|| video_tracks.first().copied())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_uses_first_audio_track_for_sonarr_primary_fields() {
        let analysis = build_analysis(RawContainer {
            format_name: "matroska".into(),
            duration_seconds: Some(60.0),
            num_chapters: Some(0),
            tracks: vec![
                RawTrack {
                    kind: TrackKind::Video,
                    codec_id: "V_MPEG4/ISO/AVC".into(),
                    codec_name: Some("h264".into()),
                    audio_profile: None,
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
                    audio_profile: Some("LC".into()),
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
                    audio_profile: None,
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

        assert_eq!(analysis.audio_codec.as_deref(), Some("aac"));
        assert_eq!(analysis.audio_profile.as_deref(), Some("LC"));
        assert_eq!(analysis.audio_channels, Some(2));
        assert_eq!(analysis.audio_bitrate_kbps, Some(128));
    }

    #[test]
    fn analysis_skips_motion_image_video_when_multiple_video_streams_exist() {
        let mut cover = test_track(TrackKind::Video, "mjpeg");
        cover.width = Some(600);
        cover.height = Some(900);
        let mut main = test_track(TrackKind::Video, "h264");
        main.width = Some(1920);
        main.height = Some(1080);
        main.frame_rate_fps = Some(24000.0 / 1001.0);

        let analysis = build_analysis(RawContainer {
            format_name: "matroska".into(),
            duration_seconds: Some(60.0),
            num_chapters: None,
            tracks: vec![cover, main],
        });

        assert_eq!(analysis.video_codec.as_deref(), Some("h264"));
        assert_eq!(analysis.video_width, Some(1920));
        assert_eq!(analysis.video_height, Some(1080));
    }

    #[test]
    fn sniff_container_format_prefers_matroska_magic_over_extension_hint() {
        assert_eq!(
            resolve_container_format(
                "mp4",
                sniff_container_format_from_bytes(&[0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0])
            ),
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

    #[test]
    fn sniff_container_format_detects_asf_ogg_and_flv() {
        assert_eq!(
            sniff_container_format_from_bytes(&asf::ASF_HEADER_GUID),
            Some(ContainerFormat::Asf)
        );
        assert_eq!(
            sniff_container_format_from_bytes(b"OggS\0"),
            Some(ContainerFormat::Ogg)
        );

        let mut flv = *b"FLV\x01\x05\0\0\0\x09";
        assert_eq!(
            sniff_container_format_from_bytes(&flv),
            Some(ContainerFormat::Flv)
        );
        flv[8] = 8;
        assert_eq!(sniff_container_format_from_bytes(&flv), None);
    }

    fn test_track(kind: TrackKind, codec_name: &str) -> RawTrack {
        RawTrack {
            kind,
            codec_id: codec_name.to_owned(),
            codec_name: Some(codec_name.to_owned()),
            audio_profile: None,
            codec_private: None,
            width: None,
            height: None,
            channels: None,
            bit_rate_bps: None,
            language: None,
            name: None,
            forced: false,
            default_track: false,
            frame_rate_fps: None,
            color_transfer: None,
            dovi_config: None,
            has_hdr10plus: false,
        }
    }
}
