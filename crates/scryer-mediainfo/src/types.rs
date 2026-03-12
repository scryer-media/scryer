/// Track type discriminator for intermediate parsing results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackKind {
    Video,
    Audio,
    Subtitle,
}

/// Intermediate representation of a single track extracted from any container
/// format. Container-specific parsers populate this struct; codec analysis and
/// HDR detection then operate on it uniformly.
#[derive(Debug, Clone)]
pub(crate) struct RawTrack {
    pub kind: TrackKind,
    /// Raw container codec identifier, e.g. `"V_MPEG4/ISO/AVC"` (MKV) or
    /// `"avc1"` (MP4 FourCC).
    pub codec_id: String,
    /// Normalized codec name (ffprobe-style), e.g. `"h264"`.
    pub codec_name: Option<String>,
    /// Codec-private / decoder configuration data stored in the container
    /// header (e.g. AVCDecoderConfigurationRecord bytes).
    pub codec_private: Option<Vec<u8>>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub channels: Option<i32>,
    pub bit_rate_bps: Option<i64>,
    pub language: Option<String>,
    /// Human-readable track name from the container (e.g. "Commentary", "Forced").
    pub name: Option<String>,
    /// MKV FlagForced — subtitle eligible for automatic selection.
    pub forced: bool,
    /// MKV FlagDefault — player should prefer this track.
    pub default_track: bool,
    pub frame_rate_fps: Option<f64>,
    /// ITU-T H.273 TransferCharacteristics value (16 = SMPTE 2084/PQ,
    /// 18 = ARIB STD-B67/HLG).
    pub color_transfer: Option<u32>,
    /// Raw Dolby Vision configuration record bytes, if present.
    pub dovi_config: Option<Vec<u8>>,
    /// Set to `true` when SMPTE ST 2094-40 (HDR10+) dynamic metadata is found
    /// in the video bitstream.
    pub has_hdr10plus: bool,
}

/// Parsed container-level metadata.
#[derive(Debug, Clone)]
pub(crate) struct RawContainer {
    /// Container format name, e.g. `"matroska"`, `"mp4"`.
    pub format_name: String,
    /// Total duration in fractional seconds.
    pub duration_seconds: Option<f64>,
    /// Number of chapters detected in the container, when supported.
    pub num_chapters: Option<i32>,
    /// All tracks found in the container.
    pub tracks: Vec<RawTrack>,
}
