use serde::{Deserialize, Serialize};
use std::io::{Read, Seek};
use std::path::Path;

mod asf;
mod audio_metadata;
mod av1;
mod avi;
mod codec;
pub mod diagnostics;
mod disc;
mod dv;
mod elementary;
mod flv;
mod legacy_video;
mod mkv;
mod mp4;
mod ogg;
mod probe;
mod ps;
mod realmedia;
mod scan;
pub mod source;
mod ts;
mod types;
mod video_headers;
mod video_metadata;

use types::{RawContainer, RawTrack, TrackKind};

// ---------------------------------------------------------------------------
// Public error type
// ---------------------------------------------------------------------------

/// Errors from native container/codec parsing.
#[derive(Debug, Clone, thiserror::Error)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaAnalysis {
    pub details: scryer_media_types::AnalysisDetails,
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
    analyze_catalog_file(file_path)
}

/// Canonical analysis for import decisions, persisted metadata, and library scans.
/// Enrichment is bounded by each container parser and runs in the same pass as
/// metadata discovery so consumers do not reopen a file to fill missing facts.
pub fn analyze_catalog_file(file_path: &Path) -> Result<MediaAnalysis, MediaInfoError> {
    analyze_file_with_options(
        file_path,
        AnalyzeOptions {
            profile: AnalysisProfile::DefaultRich,
        },
    )
}

/// Analyze an intact disc image using a saved title choice and episode mappings.
/// A missing saved title remains unresolved and never falls back to automatic selection.
pub fn analyze_disc_file(
    file_path: &Path,
    selection: scryer_media_types::DiscSelection,
) -> Result<MediaAnalysis, MediaInfoError> {
    disc::analyze(file_path, selection, AnalysisProfile::DefaultRich)
}

/// Analyzes a media file with the requested analysis behavior profile.
pub fn analyze_file_with_options(
    file_path: &Path,
    options: AnalyzeOptions,
) -> Result<MediaAnalysis, MediaInfoError> {
    let ext = scryer_media_types::formats::canonical_video_extension(file_path)
        .map(str::to_owned)
        .or_else(|| {
            file_path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_default();

    if scryer_media_types::formats::media_format_for_extension(&ext).is_some_and(|entry| {
        entry.handling == scryer_media_types::formats::MediaHandling::DiscImage
    }) {
        return disc::analyze(file_path, Default::default(), options.profile);
    }
    let mut source = match source::FileSource::open(file_path) {
        Ok(source) => source,
        // An unregistered suffix is unsupported whether or not the file is
        // readable; a registered suffix, or none at all, reports the I/O failure.
        Err(_)
            if !ext.is_empty()
                && scryer_media_types::formats::media_format_for_extension(&ext).is_none() =>
        {
            return Err(MediaInfoError::UnsupportedFormat(ext));
        }
        Err(error) => return Err(error.into()),
    };
    let analysis = analyze_source(&mut source, &ext, options)?;
    if !source.unchanged(file_path)? {
        return Err(MediaInfoError::Io(
            "media source changed during analysis".into(),
        ));
    }
    Ok(analysis)
}

/// Analyze a seekable logical media file, including a file assembled from disc extents.
/// The extension is only a hint; recognized container signatures take precedence.
pub fn analyze_source(
    input: &mut dyn source::MediaSource,
    extension: &str,
    options: AnalyzeOptions,
) -> Result<MediaAnalysis, MediaInfoError> {
    analyze_source_with_limits(input, extension, options, 64 * 1024 * 1024, 1024)
}

/// Analyze with explicit budgets, so exhaustion can be exercised without
/// movie-sized fixtures. Callers outside tests use [`analyze_source`].
pub(crate) fn analyze_source_with_limits(
    input: &mut dyn source::MediaSource,
    extension: &str,
    options: AnalyzeOptions,
    byte_budget: u64,
    io_limit: u64,
) -> Result<MediaAnalysis, MediaInfoError> {
    let started = std::time::Instant::now();
    let ext = extension.to_ascii_lowercase();
    let mut source = source::BoundedSource::new(input, byte_budget).with_io_limit(io_limit);
    source.seek(std::io::SeekFrom::Start(0))?;
    let mut header = [0_u8; 564];
    let size = source.read(&mut header)?;
    source.seek(std::io::SeekFrom::Start(0))?;
    let format = resolve_container_format(&ext, sniff_container_format_from_bytes(&header[..size]));

    let raw = match format {
        Some(ContainerFormat::Matroska) => {
            let profile = if options.profile == AnalysisProfile::Fast {
                AnalysisProfile::DefaultRich
            } else {
                options.profile
            };
            mkv::parse_mkv_source(&mut source, profile)
        }
        Some(ContainerFormat::Mp4) => mp4::parse_mp4_source(&mut source, &ext, options.profile),
        Some(ContainerFormat::Avi) => avi::parse_avi_source(&mut source, options.profile),
        Some(ContainerFormat::Ts) => ts::parse_ts_source(&mut source, options.profile),
        Some(ContainerFormat::Asf) => asf::parse_asf_source(&mut source),
        Some(ContainerFormat::Ogg) => ogg::parse_ogg_source(&mut source),
        Some(ContainerFormat::Flv) => flv::parse_flv_source(&mut source),
        Some(ContainerFormat::Ps) => ps::parse_ps(&mut source),
        Some(ContainerFormat::RealMedia) => realmedia::parse(&mut source),
        Some(ContainerFormat::Dv) => dv::parse(&mut source),
        Some(ContainerFormat::H264) => elementary::parse_h264(&mut source),
        Some(ContainerFormat::MpegVideo) => elementary::parse_mpeg(&mut source),
        Some(format @ (ContainerFormat::FlacAudio | ContainerFormat::WaveAudio)) => {
            parse_audio_only(&mut source, format)
        }
        None => {
            if size == 0 || header.starts_with(b"MZ") || header.starts_with(b"\x7fELF") {
                return Err(MediaInfoError::Parse(
                    "empty or executable input is not media".into(),
                ));
            }
            return Err(MediaInfoError::UnsupportedFormat(ext));
        }
    };

    // Parsers stop enriching once the budget is gone, so a bounded failure here
    // means exhaustion struck before any track was parsed and there is nothing
    // to keep.
    let mut analysis = match raw {
        Ok(raw) => build_analysis(raw),
        Err(_) if source.exhausted => MediaAnalysis::default(),
        Err(error) => return Err(error),
    };
    analysis.details.revision = scryer_media_types::ANALYSIS_REVISION;
    let report = &mut analysis.details.report;
    report.bytes_read = source.bytes_read;
    report.seeks = source.seeks;
    report.elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    report.budget_exhausted |= source.exhausted;
    if report.status == scryer_media_types::ProbeStatus::Unknown || source.exhausted {
        report.status = scryer_media_types::ProbeStatus::Incomplete;
    }
    if source.exhausted {
        report.warnings.push(scryer_media_types::ProbeWarning {
            code: "read_budget_exhausted".into(),
            message: "The aggregate media byte or I/O-operation budget was exhausted".into(),
            ..Default::default()
        });
    }
    Ok(analysis)
}

/// Recognize explicit audio containers before a video suffix can turn them into
/// an unavailable probe. Their framing is checked without decoding audio data.
fn parse_audio_only(
    source: &mut dyn source::MediaSource,
    format: ContainerFormat,
) -> Result<RawContainer, MediaInfoError> {
    use std::io::SeekFrom;
    let malformed = || MediaInfoError::Parse("invalid audio container framing".into());
    source.seek(SeekFrom::Start(0))?;
    let name = if format == ContainerFormat::FlacAudio {
        let mut streaminfo = [0u8; 42];
        source.read_exact(&mut streaminfo)?;
        if &streaminfo[..4] != b"fLaC"
            || streaminfo[4] & 0x7f != 0
            || streaminfo[5..8] != [0, 0, 34]
        {
            return Err(malformed());
        }
        let minimum = u16::from_be_bytes([streaminfo[8], streaminfo[9]]);
        let maximum = u16::from_be_bytes([streaminfo[10], streaminfo[11]]);
        let rate = (u32::from(streaminfo[18]) << 12)
            | (u32::from(streaminfo[19]) << 4)
            | (u32::from(streaminfo[20]) >> 4);
        let bits = ((streaminfo[20] & 1) << 4 | streaminfo[21] >> 4) + 1;
        if minimum < 16 || maximum < minimum || rate == 0 || !(4..=32).contains(&bits) {
            return Err(malformed());
        }
        let mut last = streaminfo[4] & 0x80 != 0;
        let mut position = 42u64;
        while !last {
            let mut block = [0u8; 4];
            source.read_exact(&mut block)?;
            if matches!(block[0] & 0x7f, 0 | 127) {
                return Err(malformed());
            }
            let length = u32::from_be_bytes([0, block[1], block[2], block[3]]);
            position = position
                .checked_add(4 + u64::from(length))
                .filter(|end| *end <= source.len())
                .ok_or_else(malformed)?;
            source.seek(SeekFrom::Start(position))?;
            last = block[0] & 0x80 != 0;
        }
        "flac"
    } else {
        let mut riff = [0u8; 12];
        source.read_exact(&mut riff)?;
        let end = u64::from(u32::from_le_bytes(riff[4..8].try_into().unwrap())) + 8;
        if &riff[..4] != b"RIFF" || &riff[8..] != b"WAVE" || end < 12 || end > source.len() {
            return Err(malformed());
        }
        let mut position = 12u64;
        let mut has_format = false;
        let mut has_data = false;
        while position < end {
            if end - position < 8 {
                return Err(malformed());
            }
            source.seek(SeekFrom::Start(position))?;
            let mut chunk = [0u8; 8];
            source.read_exact(&mut chunk)?;
            let length = u64::from(u32::from_le_bytes(chunk[4..8].try_into().unwrap()));
            let next = position
                .checked_add(8 + length + (length & 1))
                .filter(|next| *next <= end)
                .ok_or_else(malformed)?;
            if &chunk[..4] == b"fmt " {
                if length < 16 {
                    return Err(malformed());
                }
                let mut fmt = [0u8; 16];
                source.read_exact(&mut fmt)?;
                if u16::from_le_bytes([fmt[0], fmt[1]]) == 0
                    || u16::from_le_bytes([fmt[2], fmt[3]]) == 0
                    || u32::from_le_bytes(fmt[4..8].try_into().unwrap()) == 0
                    || u16::from_le_bytes([fmt[12], fmt[13]]) == 0
                {
                    return Err(malformed());
                }
                has_format = true;
            } else if &chunk[..4] == b"data" {
                has_data = true;
            }
            position = next;
        }
        if !has_format || !has_data {
            return Err(malformed());
        }
        "wav"
    };
    let mut details = scryer_media_types::AnalysisDetails::default();
    details.report.status = scryer_media_types::ProbeStatus::Complete;
    Ok(RawContainer {
        format_name: name.into(),
        duration_seconds: None,
        num_chapters: None,
        tracks: Vec::new(),
        details,
    })
}

fn resolve_container_format(
    ext: &str,
    sniffed: Option<ContainerFormat>,
) -> Option<ContainerFormat> {
    sniffed.or_else(|| container_format_from_extension(ext))
}

fn container_format_from_extension(ext: &str) -> Option<ContainerFormat> {
    use scryer_media_types::formats::{
        MediaFormat, MediaProbeCapability, media_format_for_extension,
    };
    let registration = media_format_for_extension(ext)?;
    if registration.probe != MediaProbeCapability::Native {
        return None;
    }
    match registration.format {
        MediaFormat::Matroska | MediaFormat::WebM => Some(ContainerFormat::Matroska),
        MediaFormat::Mp4 | MediaFormat::QuickTime => Some(ContainerFormat::Mp4),
        MediaFormat::Avi => Some(ContainerFormat::Avi),
        MediaFormat::MpegTs => Some(ContainerFormat::Ts),
        MediaFormat::MpegProgram => Some(ContainerFormat::Ps),
        MediaFormat::Asf => Some(ContainerFormat::Asf),
        MediaFormat::Ogg | MediaFormat::Ogm => Some(ContainerFormat::Ogg),
        MediaFormat::Flv => Some(ContainerFormat::Flv),
        MediaFormat::RealMedia => Some(ContainerFormat::RealMedia),
        MediaFormat::Dv => Some(ContainerFormat::Dv),
        MediaFormat::H264 => Some(ContainerFormat::H264),
        MediaFormat::MpegVideo => Some(ContainerFormat::MpegVideo),
        MediaFormat::StreamLink
        | MediaFormat::DiscImage
        | MediaFormat::Playlist
        | MediaFormat::Companion
        | MediaFormat::Unsupported
        | MediaFormat::Svq3 => None,
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
    Ps,
    RealMedia,
    Dv,
    H264,
    MpegVideo,
    FlacAudio,
    WaveAudio,
}

#[test]
fn every_registered_native_format_has_a_parser_dispatch() {
    use scryer_media_types::formats::{MEDIA_FORMATS, MediaProbeCapability};
    for entry in MEDIA_FORMATS {
        assert_eq!(
            container_format_from_extension(entry.extension).is_some(),
            entry.probe == MediaProbeCapability::Native,
            "{}",
            entry.extension
        );
    }
}

fn sniff_container_format_from_bytes(data: &[u8]) -> Option<ContainerFormat> {
    if data.starts_with(b"fLaC") {
        return Some(ContainerFormat::FlacAudio);
    }
    if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WAVE") {
        return Some(ContainerFormat::WaveAudio);
    }
    if dv::sniff(data) {
        return Some(ContainerFormat::Dv);
    }
    if data.starts_with(b".RMF") {
        return Some(ContainerFormat::RealMedia);
    }
    if data.starts_with(&[0, 0, 1, 0xb3]) {
        return Some(ContainerFormat::MpegVideo);
    }
    if data.starts_with(&[0, 0, 1, 0xba]) {
        return Some(ContainerFormat::Ps);
    }
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

    if elementary::sniff_h264(data) {
        return Some(ContainerFormat::H264);
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

fn build_analysis(mut raw: RawContainer) -> MediaAnalysis {
    populate_analysis_details(&mut raw);
    let video_tracks: Vec<&RawTrack> = raw
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .collect();
    let video_track = select_primary_video_track(&video_tracks);
    let selected_program = video_track.and_then(|track| track.metadata.program_id);
    let audio_tracks: Vec<&RawTrack> = raw
        .tracks
        .iter()
        .filter(|t| {
            t.kind == TrackKind::Audio
                && (selected_program.is_none() || t.metadata.program_id == selected_program)
        })
        .collect();
    let subtitle_tracks: Vec<&RawTrack> = raw
        .tracks
        .iter()
        .filter(|t| {
            t.kind == TrackKind::Subtitle
                && (selected_program.is_none() || t.metadata.program_id == selected_program)
        })
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

    let video_bit_depth = video_track.and_then(|track| track.metadata.bit_depth);
    let video_profile = video_track.and_then(|track| track.metadata.profile.clone());
    let video_hdr_format = video_track
        .and_then(|track| {
            let hdr = &track.metadata.hdr;
            if hdr.dolby_vision == Some(true) {
                Some("Dolby Vision")
            } else if hdr.hdr10plus == Some(true) {
                Some("HDR10+")
            } else if hdr.hdr10 == Some(true) {
                Some("HDR10")
            } else if hdr.hlg == Some(true) {
                Some("HLG")
            } else {
                None
            }
        })
        .map(str::to_string);

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
        .filter(|track| track.metadata.disposition.commentary != Some(true))
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
        details: raw.details,
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
    scryer_media_types::primary_audio_stream_index(audio_tracks.iter().map(|track| {
        (
            track
                .metadata
                .disposition
                .default
                .unwrap_or(track.default_track),
            track.metadata.disposition.commentary == Some(true),
            track.name.as_deref(),
        )
    }))
    .map(|index| audio_tracks[index])
}

fn populate_analysis_details(raw: &mut RawContainer) {
    use scryer_media_types::{ANALYSIS_REVISION, Provenance, StreamDetail, StreamKind};
    raw.details.revision = ANALYSIS_REVISION;
    raw.details.duration_seconds = raw.duration_seconds;
    if raw.duration_seconds.is_some() && raw.details.duration_provenance == Provenance::Unknown {
        raw.details.duration_provenance = Provenance::Container;
    }
    for (index, track) in raw.tracks.iter_mut().enumerate() {
        track.metadata.id.get_or_insert_with(|| index.to_string());
        if track.default_track {
            track.metadata.disposition.default.get_or_insert(true);
        }
        if track.kind == TrackKind::Audio
            && let Some((code, exhausted)) = audio_metadata::enrich_private_header(track)
        {
            if matches!(
                raw.details.report.status,
                scryer_media_types::ProbeStatus::Unknown
                    | scryer_media_types::ProbeStatus::Complete
            ) {
                raw.details.report.status = scryer_media_types::ProbeStatus::Incomplete;
            }
            raw.details.report.budget_exhausted |= exhausted;
            raw.details
                .report
                .warnings
                .push(scryer_media_types::ProbeWarning {
                    code: code.into(),
                    message: "Audio configuration metadata could not be completely interpreted"
                        .into(),
                    stream_id: track.metadata.id.clone(),
                    ..Default::default()
                });
        }
        if track.metadata.original_language.is_none() {
            track.metadata.original_language = track.language.clone();
        }
        track.metadata.bitrate_bps = track
            .bit_rate_bps
            .and_then(|value| u64::try_from(value).ok());
        if track.metadata.bitrate_bps.is_some()
            && track.metadata.bitrate_provenance == Provenance::Unknown
        {
            track.metadata.bitrate_provenance = Provenance::Container;
        }
        if track.kind == TrackKind::Video {
            if let Some(private) = track.codec_private.clone() {
                legacy_video::enrich(track, &private);
            }
            let info = extract_codec_info(track);
            let observed_av1_sequence = track.codec_name.as_deref() == Some("av1")
                && track.metadata.color.provenance == Provenance::Bitstream;
            if !observed_av1_sequence {
                track.metadata.bit_depth = info.bit_depth.or(track.metadata.bit_depth);
                track.metadata.profile = info.profile.or(track.metadata.profile.take());
            }
            let container_transfer = track
                .metadata
                .color
                .transfer
                .or(track.color_transfer)
                .filter(|value| *value != 2);
            let bitstream_transfer = info.color_transfer.filter(|value| *value != 2);
            if let Some(bitstream) = bitstream_transfer {
                if container_transfer.is_some_and(|container| container != bitstream) {
                    raw.details
                        .report
                        .warnings
                        .push(scryer_media_types::ProbeWarning {
                        code: "color_signaling_conflict".into(),
                        message:
                            "Bitstream transfer signaling overrides conflicting container metadata"
                                .into(),
                        stream_id: track.metadata.id.clone(),
                        ..Default::default()
                    });
                }
                track.metadata.color.provenance = Provenance::Bitstream;
            }
            track.metadata.color.transfer = bitstream_transfer.or(container_transfer);
            if track.codec_name.as_deref() == Some("av1")
                && !observed_av1_sequence
                && let Some(private) = track
                    .codec_private
                    .clone()
                    .filter(|bytes| bytes.len() > 4 && bytes[0] == 0x81)
            {
                av1::enrich(track, &private[4..], &mut raw.details.report);
            }
            if let Some(private) = track.codec_private.clone() {
                video_headers::enrich(track, &private, &mut raw.details.report);
            }
            let transfer = track.metadata.color.transfer;
            track.metadata.hdr.pq = transfer.map(|value| value == 16);
            track.metadata.hdr.hlg = transfer.map(|value| value == 18);
            if track.has_hdr10plus {
                track.metadata.hdr.hdr10plus = Some(true);
            }
            track.metadata.hdr.hdr10 = match (transfer, track.metadata.bit_depth) {
                (Some(16), Some(depth))
                    if depth >= 10
                        && (track.metadata.color.primaries == Some(9)
                            || track.metadata.color.mastering_display.is_some()) =>
                {
                    Some(true)
                }
                (Some(value), _) if value != 16 => Some(false),
                (_, Some(depth)) if depth < 10 => Some(false),
                _ => None,
            };
            if let Some(dovi) = track
                .dovi_config
                .as_deref()
                .and_then(codec::parse_dovi_config)
            {
                track.metadata.hdr.dolby_vision = Some(true);
                track.metadata.hdr.dovi = Some(scryer_media_types::DolbyVision {
                    profile: Some(dovi.profile),
                    level: Some(dovi.level),
                    base_layer_compatibility_id: Some(dovi.bl_signal_compatibility_id),
                    rpu_present: Some(dovi.rpu_present),
                    enhancement_layer_present: Some(dovi.enhancement_layer_present),
                    base_layer_present: Some(dovi.base_layer_present),
                });
            }
        } else {
            track.metadata.profile = track
                .audio_profile
                .clone()
                .or(track.metadata.profile.take());
            if let Some((format, depth)) = track
                .codec_name
                .as_deref()
                .and_then(codec::pcm_sample_representation)
            {
                track.metadata.sample_format = Some(format.into());
                track.metadata.sample_bit_depth.get_or_insert(depth);
            }
            if track.codec_name.as_deref() == Some("aac")
                && let Some(config) = track
                    .codec_private
                    .as_deref()
                    .and_then(codec::aac_configuration_metadata)
            {
                // Implicit SBR/PS may be signaled only in later payloads.
                // Keep an explicit output rate/count when the ASC describes
                // a compatible core without declaring the extension state.
                if config.sbr_signaled.is_some()
                    || track.metadata.sample_rate != config.sample_rate.checked_mul(2)
                {
                    track.metadata.sample_rate = Some(config.sample_rate);
                }
                if let Some((channels, layout)) = config.layout {
                    if config.ps_signaled.is_none() && channels == 1 && track.channels == Some(2) {
                        track.metadata.channel_layout = None;
                    } else {
                        track.channels = Some(i32::from(channels));
                        track.metadata.channel_layout = Some(layout.into());
                    }
                }
            }
        }
    }
    let videos: Vec<_> = raw
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .collect();
    if let Some(video) = select_primary_video_track(&videos) {
        raw.details.selected_video_id = video.metadata.id.clone();
        raw.details.selected_program_id = video.metadata.program_id;
    }
    raw.details.streams = raw
        .tracks
        .iter()
        .map(|track| StreamDetail {
            kind: match track.kind {
                TrackKind::Video => StreamKind::Video,
                TrackKind::Audio => StreamKind::Audio,
                TrackKind::Subtitle => StreamKind::Subtitle,
            },
            codec: track.codec_name.clone(),
            width: track.width,
            height: track.height,
            channels: track.channels,
            language: track.language.clone(),
            name: track.name.clone(),
            metadata: track.metadata.clone(),
        })
        .collect();
}

fn select_primary_video_track<'a>(video_tracks: &[&'a RawTrack]) -> Option<&'a RawTrack> {
    video_tracks.iter().copied().find(|track| {
        let disposition = &track.metadata.disposition;
        if disposition.attached_picture == Some(true) || disposition.still_image == Some(true) {
            return false;
        }
        video_tracks.len() == 1
            || disposition.attached_picture == Some(false)
            || disposition.still_image == Some(false)
            || !matches!(track.codec_name.as_deref(), Some("mjpeg" | "png"))
    })
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
    fn primary_audio_probe_keeps_mono_main_track_over_commentary() {
        let mut mono = test_track(TrackKind::Audio, "aac");
        mono.channels = Some(1);
        let mut commentary = test_track(TrackKind::Audio, "flac");
        commentary.channels = Some(2);
        commentary.default_track = true;
        commentary.name = Some("Director's Commentary".into());
        let analysis = build_analysis(RawContainer {
            format_name: "matroska".into(),
            duration_seconds: Some(60.0),
            num_chapters: None,
            details: Default::default(),
            tracks: vec![commentary, mono],
        });
        assert_eq!(analysis.audio_channels, Some(1));
        assert_eq!(analysis.audio_codec.as_deref(), Some("aac"));
    }

    #[test]
    fn primary_audio_probe_uses_default_then_first_non_commentary() {
        let mut stereo = test_track(TrackKind::Audio, "flac");
        stereo.channels = Some(2);
        let mut mono = test_track(TrackKind::Audio, "aac");
        mono.channels = Some(1);
        mono.default_track = true;
        assert_eq!(
            select_primary_audio_track(&[&stereo, &mono])
                .unwrap()
                .channels,
            Some(1)
        );
        mono.default_track = false;
        assert_eq!(
            select_primary_audio_track(&[&mono, &stereo])
                .unwrap()
                .channels,
            Some(1)
        );
        mono.metadata.disposition.commentary = Some(true);
        stereo.metadata.disposition.commentary = Some(true);
        assert!(select_primary_audio_track(&[&mono, &stereo]).is_none());
    }

    #[test]
    fn container_signatures_win_over_embedded_elementary_headers() {
        let sps = [0, 0, 1, 0x67, 66, 0, 30, 244, 5, 1, 236, 128];
        for (prefix, expected) in [
            (vec![0, 0, 1, 0xba], ContainerFormat::Ps),
            (
                vec![0, 0, 0, 24, b'f', b't', b'y', b'p'],
                ContainerFormat::Mp4,
            ),
            (vec![0x1a, 0x45, 0xdf, 0xa3], ContainerFormat::Matroska),
        ] {
            let bytes = [prefix, sps.to_vec()].concat();
            assert_eq!(
                resolve_container_format("avc", sniff_container_format_from_bytes(&bytes)),
                Some(expected)
            );
        }
    }

    #[test]
    fn content_only_aliases_never_force_a_container() {
        for extension in ["dat", "svq3", "bivx", "divx", "xvid"] {
            assert_eq!(resolve_container_format(extension, None), None);
            assert_eq!(
                resolve_container_format(extension, Some(ContainerFormat::Avi)),
                Some(ContainerFormat::Avi)
            );
        }
    }

    #[test]
    fn unsupported_media_is_distinct_from_empty_executable_and_io_failure() {
        use std::io::Cursor;
        for bytes in [
            Vec::new(),
            b"MZnot-media".to_vec(),
            b"\x7fELFnot-media".to_vec(),
        ] {
            assert!(matches!(
                analyze_source(&mut Cursor::new(bytes), "wtv", AnalyzeOptions::default()),
                Err(MediaInfoError::Parse(_))
            ));
        }
        assert!(matches!(
            analyze_source(
                &mut Cursor::new(b"legacy format"),
                "wtv",
                AnalyzeOptions::default()
            ),
            Err(MediaInfoError::UnsupportedFormat(_))
        ));
        assert!(matches!(
            analyze_file(Path::new("")),
            Err(MediaInfoError::Io(_))
        ));
    }

    #[test]
    fn audio_only_signatures_override_video_suffixes_without_becoming_unsupported() {
        let mut flac = b"fLaC\x80\0\0\x22".to_vec();
        flac.extend([0x10, 0, 0x10, 0, 0, 0, 0, 0, 0, 0]);
        flac.extend(((44100u64 << 44) | (15u64 << 36)).to_be_bytes());
        flac.extend([0; 16]);
        let mut wave = b"RIFF".to_vec();
        wave.extend(38u32.to_le_bytes());
        wave.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
        wave.extend(44100u32.to_le_bytes());
        wave.extend(88200u32.to_le_bytes());
        wave.extend(b"\x02\0\x10\0data\x02\0\0\0\0\0");
        for bytes in [&flac, &wave] {
            for extension in ["dat", "xvid", "wtv", "mkv"] {
                let analysis = analyze_source(
                    &mut std::io::Cursor::new(bytes),
                    extension,
                    AnalyzeOptions::default(),
                )
                .unwrap();
                assert!(!is_valid_video(&analysis));
                assert_eq!(analysis.video_codec, None);
                assert_eq!(analysis.details.selected_video_id, None);
                assert_eq!(
                    analysis.details.report.status,
                    scryer_media_types::ProbeStatus::Complete
                );
            }
        }
        flac[7] = 33;
        wave[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        for bytes in [flac, wave, b"fLaC".to_vec()] {
            assert!(
                analyze_source(
                    &mut std::io::Cursor::new(bytes),
                    "wtv",
                    AnalyzeOptions::default(),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn analysis_selects_default_audio_and_keeps_primary_fields_on_that_track() {
        let analysis = build_analysis(RawContainer {
            details: Default::default(),
            format_name: "matroska".into(),
            duration_seconds: Some(60.0),
            num_chapters: Some(0),
            tracks: vec![
                RawTrack {
                    metadata: Default::default(),
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
                    metadata: Default::default(),
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
                    metadata: Default::default(),
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

        assert_eq!(analysis.audio_codec.as_deref(), Some("flac"));
        assert_eq!(analysis.audio_profile, None);
        assert_eq!(analysis.audio_channels, Some(6));
        assert_eq!(analysis.audio_bitrate_kbps, Some(640));
        assert_eq!(
            analysis.details.streams[2].metadata.disposition.default,
            Some(true)
        );
    }

    #[test]
    fn aac_core_configuration_preserves_compatible_explicit_output_metadata() {
        let mut audio = test_track(TrackKind::Audio, "aac");
        audio.codec_private = Some(vec![0x13, 0x88]); // AAC-LC, 22050 Hz mono core
        audio.channels = Some(2);
        audio.metadata.sample_rate = Some(44100);
        let mut raw = RawContainer {
            tracks: vec![audio],
            format_name: "matroska".into(),
            duration_seconds: None,
            num_chapters: None,
            details: Default::default(),
        };
        populate_analysis_details(&mut raw);
        let audio = &raw.details.streams[0];
        assert_eq!(audio.channels, Some(2));
        assert_eq!(audio.metadata.sample_rate, Some(44100));
        assert!(
            audio.metadata.channel_layout.is_none(),
            "unobserved PS cannot establish the output layout"
        );
        raw.tracks[0].codec_private = Some(vec![0x11, 0xb0]); // explicit 48000 Hz, 5.1
        populate_analysis_details(&mut raw);
        let audio = &raw.details.streams[0];
        assert_eq!(audio.channels, Some(6));
        assert_eq!(audio.metadata.sample_rate, Some(48000));
        assert_eq!(audio.metadata.channel_layout.as_deref(), Some("5.1"));
    }

    #[test]
    fn selected_program_excludes_commentary_and_preserves_hdr_combinations() {
        let mut video = test_track(TrackKind::Video, "hevc");
        video.metadata.program_id = Some(1);
        video.metadata.bit_depth = Some(10);
        video.metadata.color.primaries = Some(9);
        video.color_transfer = Some(16);
        video.dovi_config = Some(vec![1, 0, 0x10, 0x00, 0x10]);
        video.has_hdr10plus = true;
        let mut commentary = test_track(TrackKind::Audio, "truehd");
        commentary.language = Some("fra".into());
        commentary.metadata.program_id = Some(1);
        commentary.metadata.disposition.commentary = Some(true);
        let mut normal = test_track(TrackKind::Audio, "aac");
        normal.channels = Some(2);
        normal.language = Some("eng".into());
        normal.metadata.program_id = Some(1);
        let mut described = normal.clone();
        described.channels = Some(6);
        described.metadata.disposition.visual_impaired = Some(true);
        let mut other = test_track(TrackKind::Audio, "flac");
        other.metadata.program_id = Some(2);
        other.language = Some("jpn".into());
        let analysis = build_analysis(RawContainer {
            tracks: vec![video, commentary, normal, described, other],
            format_name: "mpegts".into(),
            duration_seconds: Some(10.0),
            num_chapters: None,
            details: Default::default(),
        });
        assert_eq!(analysis.audio_codec.as_deref(), Some("aac"));
        // The first ordinary audio track is primary; a later six-channel
        // descriptive track must not replace its channel count.
        assert_eq!(analysis.audio_channels, Some(2));
        assert_eq!(analysis.audio_streams.len(), 3);
        assert!(
            analysis
                .audio_languages
                .iter()
                .all(|language| language == "eng")
        );
        assert_eq!(analysis.details.streams.len(), 5);
        let hdr = &analysis.details.streams[0].metadata.hdr;
        assert_eq!(hdr.dolby_vision, Some(true));
        assert_eq!(hdr.hdr10plus, Some(true));
        assert_eq!(hdr.hdr10, Some(true));
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
            details: Default::default(),
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
            metadata: Default::default(),
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
