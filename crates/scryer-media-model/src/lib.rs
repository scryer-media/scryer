//! The facts a media probe reports about a file: its streams, programs,
//! chapters, attachments, disc titles, tags and a report of how far the probe
//! got, plus a registry of recognized media suffixes and their container
//! families.
//!
//! This crate holds data only. It never opens media and depends on no
//! parser, so code that stores or displays results does not need one.
//!
//! [`AnalysisDetails`] is the root. Every type here serializes with serde,
//! and every struct reads missing fields as their defaults, so a record
//! written by an older version still loads. An `Option` field that is `None`
//! means the fact was not found or not examined, never that it is false or
//! zero; each field says what it means when absent.
//!
//! ```
//! use scryer_media_model::{AnalysisDetails, Codec, StreamDetail, StreamKind};
//!
//! let details = AnalysisDetails {
//!     duration_seconds: Some(1325.4),
//!     streams: vec![StreamDetail {
//!         kind: StreamKind::Audio,
//!         codec: Some(Codec::Eac3),
//!         channels: Some(6),
//!         ..Default::default()
//!     }],
//!     ..Default::default()
//! };
//! let audio = details.primary_audio().expect("one audio stream");
//! assert_eq!(audio.codec.as_ref().map(Codec::as_str), Some("eac3"));
//! assert_eq!(details.duration().map(|d| d.as_secs()), Some(1325));
//! ```

#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

mod codec;
pub mod formats;
mod picture;
pub mod warning;

pub use codec::Codec;
pub use picture::{ChromaLocation, ChromaSubsampling, HdrFormat, ParseLabelError};

/// The version of the analysis rules that produced a record. Bumped whenever
/// the same file would be described differently, so stored records written
/// by an older version can be recognized and refreshed.
pub const ANALYSIS_REVISION: u32 = DOLBY_VISION_RECORD_REVISION;

/// The first revision whose primary audio summary follows the authored default
/// track rather than the highest channel count or codec rank.
#[cfg(test)]
const AUTHORED_PRIMARY_AUDIO_REVISION: u32 = 2;

/// The first revision that reads Dolby Vision configuration from Matroska track
/// mappings, MP4 `dvwC` boxes and 4-byte records, per track.
const DOLBY_VISION_RECORD_REVISION: u32 = 3;

/// Where a fact came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Not recorded.
    #[default]
    Unknown,
    /// Declared by the container.
    Container,
    /// Read from the coded bitstream.
    Bitstream,
    /// Measured from the samples the probe read.
    Observed,
    /// Estimated from other facts, such as size over duration.
    Estimated,
    /// Carried over from a record written before provenance was kept.
    Legacy,
}

/// How a probe ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    /// Not recorded.
    #[default]
    Unknown,
    /// Every part the probe meant to read was read.
    Complete,
    /// The probe stopped early or skipped parts; see the warnings.
    Incomplete,
    /// The format is recognized but its contents cannot be described.
    Unsupported,
    /// The media is encrypted.
    Encrypted,
    /// The media is structurally invalid.
    Malformed,
}

/// How far a probe got and what it cost.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeReport {
    /// How the probe ended.
    pub status: ProbeStatus,
    /// Bytes read from the source.
    pub bytes_read: u64,
    /// Seeks issued to the source.
    pub seeks: u64,
    /// Wall-clock time the probe took, in milliseconds.
    pub elapsed_ms: u64,
    /// Whether the probe stopped at its read budget or I/O operation limit.
    pub budget_exhausted: bool,
    /// Conditions worth knowing about, in the order they were met.
    pub warnings: Vec<ProbeWarning>,
}

/// The latest persisted attempt, separate from the last successful analysis.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisAttempt {
    /// [`ANALYSIS_REVISION`] at the time of the attempt.
    pub revision: u32,
    /// When the attempt ran, as written by the caller that stored it.
    pub attempted_at: String,
    /// Whether the attempt produced an analysis.
    pub succeeded: bool,
    /// The attempt's probe report.
    pub report: ProbeReport,
    /// Inventory observed by this attempt; it does not replace the saved selection.
    pub disc: Option<DiscMetadata>,
}

/// Physical source ranges read by diagnostics; overlapping reads are merged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagnosticCoverage {
    /// Offset of the first byte read.
    pub offset: u64,
    /// Number of bytes read from `offset`.
    pub length: u64,
}

/// Header sampling is not a complete-file integrity check and never decodes media.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StructuralDiagnostics {
    /// The diagnostic pass's own report and findings.
    pub report: ProbeReport,
    /// The source's length in bytes.
    pub source_length: u64,
    /// The byte ranges that were read.
    pub coverage: Vec<DiagnosticCoverage>,
    /// Whether `coverage` was cut short at its size limit.
    pub coverage_truncated: bool,
    /// Whether findings past the warning limit were left out.
    pub warnings_truncated: bool,
    /// Container packets inspected.
    pub packets_sampled: u64,
    /// Elementary frame headers inspected.
    pub frame_headers_sampled: u64,
    /// The names of the checks that ran.
    pub checks: Vec<String>,
}

/// One condition a probe met, such as a budget running out or two sources
/// disagreeing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeWarning {
    /// The stable code to match on; see [`warning`] for every value.
    pub code: String,
    /// A description for people. Wording may change between versions.
    pub message: String,
    /// The stream it concerns, matching [`StreamMetadata::id`]; `None` when it
    /// concerns the file as a whole.
    pub stream_id: Option<String>,
    /// The byte offset it concerns; `None` when no single offset applies.
    pub offset: Option<u64>,
}

/// An exact fraction in lowest terms, such as a frame rate of 24000/1001.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    /// The numerator.
    pub numerator: i64,
    /// The denominator; nonzero in every value this crate builds.
    pub denominator: u64,
}

impl Rational {
    /// Preserve a declared binary floating-point value exactly, when it fits.
    /// `None` for NaN, infinities and values outside the `i64`/`u64` range.
    #[must_use]
    pub fn from_f64(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        if value == 0.0 {
            return Self::new(0, 1);
        }
        let bits = value.to_bits();
        let encoded_exponent = ((bits >> 52) & 0x7ff) as i32;
        let mut significand = bits & ((1_u64 << 52) - 1);
        let mut exponent = if encoded_exponent == 0 {
            -1074
        } else {
            significand |= 1_u64 << 52;
            encoded_exponent - 1023 - 52
        };
        let zeros = significand.trailing_zeros();
        significand >>= zeros;
        exponent += zeros as i32;
        let (magnitude, denominator) = if exponent >= 0 {
            (
                significand.checked_mul(1_u64.checked_shl(exponent as u32)?)?,
                1,
            )
        } else {
            (significand, 1_u64.checked_shl((-exponent) as u32)?)
        };
        let numerator = i128::from(magnitude) * if value.is_sign_negative() { -1 } else { 1 };
        Self::new(i64::try_from(numerator).ok()?, denominator)
    }

    /// The fraction in lowest terms; `None` when `denominator` is zero.
    #[must_use]
    pub fn new(numerator: i64, denominator: u64) -> Option<Self> {
        if denominator == 0 {
            return None;
        }
        let (mut a, mut b) = (numerator.unsigned_abs(), denominator);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Some(Self {
            numerator: (i128::from(numerator) / i128::from(a)) as i64,
            denominator: denominator / a,
        })
    }

    /// The value as a float; `None` when `denominator` is zero.
    #[must_use]
    pub fn as_f64(self) -> Option<f64> {
        (self.denominator != 0).then(|| self.numerator as f64 / self.denominator as f64)
    }
}

/// Stream roles the container declares. Each is `None` when the container
/// does not say, which is not the same as `Some(false)`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamDisposition {
    /// Marked as the default stream of its kind.
    pub default: Option<bool>,
    /// Marked as forced (for example foreign-dialogue subtitles).
    pub forced: Option<bool>,
    /// Marked as being in the original language.
    pub original: Option<bool>,
    /// Marked as commentary.
    pub commentary: Option<bool>,
    /// Marked for the hearing impaired.
    pub hearing_impaired: Option<bool>,
    /// Marked for the visually impaired.
    pub visual_impaired: Option<bool>,
    /// An attached picture, such as cover art, rather than a video.
    pub attached_picture: Option<bool>,
    /// A still image rather than a moving picture.
    pub still_image: Option<bool>,
}

/// Color description of a video stream, using the ITU-T H.273 code points.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorMetadata {
    /// ColourPrimaries code point (for example 1 for BT.709, 9 for BT.2020);
    /// `None` when unspecified.
    pub primaries: Option<u32>,
    /// TransferCharacteristics code point (for example 16 for PQ, 18 for
    /// HLG); `None` when unspecified.
    pub transfer: Option<u32>,
    /// MatrixCoefficients code point; `None` when unspecified.
    pub matrix: Option<u32>,
    /// Whether samples use the full range; `None` when unspecified.
    pub full_range: Option<bool>,
    /// Where the color description came from.
    pub provenance: Provenance,
    /// Mastering display color volume; `None` when not signaled.
    pub mastering_display: Option<MasteringDisplay>,
    /// Content light levels; `None` when not signaled.
    pub content_light: Option<ContentLight>,
}

/// Mastering display color volume (SMPTE ST 2086). Chromaticities are CIE
/// 1931 xy coordinates; luminance is in candelas per square metre. A field is
/// `None` when the source left it out or gave an invalid value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MasteringDisplay {
    /// Red primary, x.
    pub red_x: Option<f64>,
    /// Red primary, y.
    pub red_y: Option<f64>,
    /// Green primary, x.
    pub green_x: Option<f64>,
    /// Green primary, y.
    pub green_y: Option<f64>,
    /// Blue primary, x.
    pub blue_x: Option<f64>,
    /// Blue primary, y.
    pub blue_y: Option<f64>,
    /// White point, x.
    pub white_x: Option<f64>,
    /// White point, y.
    pub white_y: Option<f64>,
    /// Minimum display luminance.
    pub min_luminance: Option<f64>,
    /// Maximum display luminance.
    pub max_luminance: Option<f64>,
}

/// Content light levels (CTA-861.3), in candelas per square metre.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentLight {
    /// Maximum content light level; `None` when not signaled.
    pub max_cll: Option<u32>,
    /// Maximum frame-average light level; `None` when not signaled.
    pub max_fall: Option<u32>,
}

/// A Dolby Vision configuration record. A field is `None` when the record
/// does not carry it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DolbyVision {
    /// Dolby Vision profile number.
    pub profile: Option<u8>,
    /// Dolby Vision level number.
    pub level: Option<u8>,
    /// Base-layer signal compatibility id.
    pub base_layer_compatibility_id: Option<u8>,
    /// Whether a reference processing unit is present.
    pub rpu_present: Option<bool>,
    /// Whether an enhancement layer is present.
    pub enhancement_layer_present: Option<bool>,
    /// Whether a base layer is present.
    pub base_layer_present: Option<bool>,
}

/// The high-dynamic-range signals a video stream carries. Each flag is
/// `None` when the probe could not tell; use [`StreamDetail::hdr_format`] for
/// the single strongest format.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HdrCapabilities {
    /// A Dolby Vision configuration is present.
    pub dolby_vision: Option<bool>,
    /// HDR10+ dynamic metadata is present.
    pub hdr10plus: Option<bool>,
    /// The stream meets HDR10: PQ transfer, 10-bit or deeper, and BT.2020
    /// primaries or mastering display metadata.
    pub hdr10: Option<bool>,
    /// The transfer is hybrid log-gamma.
    pub hlg: Option<bool>,
    /// The transfer is PQ (SMPTE ST 2084).
    pub pq: Option<bool>,
    /// The Dolby Vision configuration record, when present.
    pub dovi: Option<DolbyVision>,
}

/// Everything known about one stream beyond its codec and basic shape.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamMetadata {
    /// The stream's identifier, unique within its file: the container's track
    /// number or packet identifier when it has one, else its position.
    pub id: Option<String>,
    /// The program the stream belongs to, for containers with programs
    /// (MPEG transport streams); `None` elsewhere.
    pub program_id: Option<u32>,
    /// The language tag as the container wrote it; `None` when absent.
    pub original_language: Option<String>,
    /// Where the language came from.
    pub language_provenance: Provenance,
    /// Roles the container declares.
    pub disposition: StreamDisposition,
    /// The stream's own duration; `None` when the container states none.
    pub duration_seconds: Option<f64>,
    /// Bitrate in bits per second; `None` when neither declared nor measured.
    pub bitrate_bps: Option<u64>,
    /// Where `bitrate_bps` came from.
    pub bitrate_provenance: Provenance,
    /// Bitrate estimated from size over duration; `None` when not estimated.
    pub estimated_bitrate_bps: Option<u64>,
    /// Audio sample rate in hertz; `None` when unknown.
    pub sample_rate: Option<u32>,
    /// Encoded PCM representation (for example s24le), not a decoder's output format.
    /// Compressed streams without a declared sample representation remain unknown.
    pub sample_format: Option<String>,
    /// Bits per audio sample; `None` when unknown.
    pub sample_bit_depth: Option<u32>,
    /// Audio channel layout in ffprobe's naming (for example `5.1(side)`);
    /// `None` when unknown.
    pub channel_layout: Option<String>,
    /// Picture pixel format in ffprobe's naming (for example `yuv420p10le`);
    /// `None` when unknown.
    pub pixel_format: Option<String>,
    /// Codec profile name (for example `High` or `Main 10`); `None` when unknown.
    pub profile: Option<String>,
    /// Codec level as the bitstream codes it; `None` when unknown.
    pub level: Option<u32>,
    /// Bits per picture sample; `None` when unknown.
    pub bit_depth: Option<i32>,
    /// `progressive`, `interlaced`, `top_first` or `bottom_first`; `None`
    /// when unknown.
    pub field_order: Option<String>,
    /// An explicit stereoscopic container declaration. False means not detected.
    pub is_3d: bool,
    /// Sample (pixel) aspect ratio; `None` when unknown or unspecified.
    pub sample_aspect_ratio: Option<Rational>,
    /// Display aspect ratio; `None` when unknown.
    pub display_aspect_ratio: Option<Rational>,
    /// Clockwise display rotation in degrees; `None` when none is declared.
    pub rotation_degrees: Option<f64>,
    /// Frame rate the container or bitstream declares; `None` when none.
    pub declared_frame_rate: Option<Rational>,
    /// Frame rate measured from timestamps; `None` when not measured.
    pub observed_frame_rate: Option<Rational>,
    /// Whether frame timing varies; `None` when not measured.
    pub variable_frame_rate: Option<bool>,
    /// Color description.
    pub color: ColorMetadata,
    /// High-dynamic-range signals.
    pub hdr: HdrCapabilities,
    /// Picture width declared by the codec configuration (for example an H.264
    /// or HEVC parameter set), independent of the container's track header.
    pub configured_width: Option<i32>,
    /// Picture height declared by the codec configuration.
    pub configured_height: Option<i32>,
    /// The parser's single frame-rate figure for a video stream: the
    /// container's rate, or the codec configuration's timing when the
    /// container states none. Unlike the rationals above it is never derived
    /// from disc navigation.
    pub nominal_frame_rate_fps: Option<f64>,
    /// Chroma subsampling, from the bitstream or the container; `None` when
    /// neither states it or the picture is RGB.
    #[serde(deserialize_with = "picture::lenient_label")]
    pub chroma_subsampling: Option<ChromaSubsampling>,
    /// Chroma sample location, only where the bitstream or container states
    /// it; `None` otherwise, including where a codec implies one by default.
    #[serde(deserialize_with = "picture::lenient_label")]
    pub chroma_location: Option<ChromaLocation>,
    /// Text tags the container attaches to this stream, such as a title,
    /// handler name or encoder. Keys keep the container's own spelling and
    /// case, so the same idea may be `title` in one format and `TITLE` or
    /// `©nam` in another. Values that are binary or not UTF-8 are left out.
    /// Empty when the container has none or the parser does not read them.
    pub tags: BTreeMap<String, String>,
}

/// The kind of media a stream carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    /// Pictures, including attached cover art.
    #[default]
    Video,
    /// Sound.
    Audio,
    /// Subtitles, captions and other timed text or graphics.
    Subtitle,
}

/// One stream of a file or disc title.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamDetail {
    /// The kind of media.
    pub kind: StreamKind,
    /// The codec; `None` when the container identifies none.
    pub codec: Option<Codec>,
    /// Picture width in pixels from the container; `None` when it states
    /// none. See [`picture_size`](Self::picture_size) for a fallback.
    pub width: Option<i32>,
    /// Picture height in pixels from the container; `None` when it states none.
    pub height: Option<i32>,
    /// Audio channel count; `None` when unknown.
    pub channels: Option<i32>,
    /// Language tag, as ISO 639 or BCP 47; `None` when absent.
    pub language: Option<String>,
    /// The stream's title; `None` when absent.
    pub name: Option<String>,
    /// Everything else known about the stream.
    pub metadata: StreamMetadata,
}

impl StreamDetail {
    /// Picture size: the container's dimensions, each falling back to the
    /// codec configuration's. Either half is `None` when neither states it.
    #[must_use]
    pub fn picture_size(&self) -> (Option<i32>, Option<i32>) {
        (
            self.width.or(self.metadata.configured_width),
            self.height.or(self.metadata.configured_height),
        )
    }

    /// The strongest HDR format the stream signals, in the precedence
    /// Dolby Vision, HDR10+, HDR10, HLG; `None` for standard dynamic range
    /// or when the probe could not tell.
    #[must_use]
    pub fn hdr_format(&self) -> Option<HdrFormat> {
        let hdr = &self.metadata.hdr;
        if hdr.dolby_vision == Some(true) {
            Some(HdrFormat::DolbyVision)
        } else if hdr.hdr10plus == Some(true) {
            Some(HdrFormat::Hdr10Plus)
        } else if hdr.hdr10 == Some(true) {
            Some(HdrFormat::Hdr10)
        } else if hdr.hlg == Some(true) {
            Some(HdrFormat::Hlg)
        } else {
            None
        }
    }

    /// Whether the container marks the stream as the default of its kind.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.metadata.disposition.default == Some(true)
    }

    /// Whether the stream is commentary: marked so by the container, or
    /// titled with the word "commentary" in any case.
    #[must_use]
    pub fn is_commentary(&self) -> bool {
        AudioCandidate::new(
            false,
            self.metadata.disposition.commentary == Some(true),
            self.name.as_deref(),
        )
        .is_commentary()
    }

    /// The facts [`primary_audio_position`] reads from this stream, for
    /// choosing among audio streams the caller has filtered itself.
    #[must_use]
    pub fn audio_candidate(&self) -> AudioCandidate<'_> {
        AudioCandidate::new(
            self.is_default(),
            self.metadata.disposition.commentary == Some(true),
            self.name.as_deref(),
        )
    }
}

/// The facts [`primary_audio_position`] reads from one audio stream, for
/// callers that hold their audio streams in their own types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioCandidate<'a> {
    /// Marked as the default audio.
    pub default: bool,
    /// Marked as commentary.
    pub commentary: bool,
    /// The stream's title, if any.
    pub title: Option<&'a str>,
}

impl<'a> AudioCandidate<'a> {
    /// Collects one stream's facts.
    #[must_use]
    pub const fn new(default: bool, commentary: bool, title: Option<&'a str>) -> Self {
        Self {
            default,
            commentary,
            title,
        }
    }

    fn is_commentary(&self) -> bool {
        self.commentary
            || self.title.is_some_and(|title| {
                title
                    .split(|ch: char| !ch.is_alphanumeric())
                    .any(|word| word.eq_ignore_ascii_case("commentary"))
            })
    }
}

/// The position of the primary audio among `candidates`, given in file
/// order: the first default stream that is not commentary, else the first
/// stream that is not commentary. Channel count and codec never override the
/// authored default. `None` when every candidate is commentary or there are
/// none.
///
/// [`AnalysisDetails::primary_audio`] applies this to a file's audio; use this
/// form when the streams live in another type.
///
/// ```
/// use scryer_media_model::{primary_audio_position, AudioCandidate};
///
/// let streams = [
///     AudioCandidate::new(true, false, Some("Director's Commentary")),
///     AudioCandidate::new(false, false, Some("Main")),
/// ];
/// assert_eq!(primary_audio_position(streams), Some(1));
/// ```
#[must_use]
pub fn primary_audio_position<'a>(
    candidates: impl IntoIterator<Item = AudioCandidate<'a>>,
) -> Option<usize> {
    candidates
        .into_iter()
        .enumerate()
        .filter(|(_, candidate)| !candidate.is_commentary())
        .min_by_key(|(index, candidate)| (!candidate.default, *index))
        .map(|(index, _)| index)
}

/// A chapter marker.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chapter {
    /// The chapter's identifier within its file or title.
    pub id: String,
    /// The chapter's title; `None` when it has none.
    pub title: Option<String>,
    /// Start time in seconds.
    pub start_seconds: f64,
    /// End time in seconds; `None` when the container states none.
    pub end_seconds: Option<f64>,
}

/// A file attached to the container, such as a font.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Attachment {
    /// The attachment's identifier within its file.
    pub id: String,
    /// The attached file's name; `None` when absent.
    pub name: Option<String>,
    /// The attached file's media type; `None` when absent.
    pub media_type: Option<String>,
    /// The attached file's size in bytes.
    pub size_bytes: u64,
}

/// A closed-caption service carried inside a video stream.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptionService {
    /// The id of the video stream that carries it.
    pub stream_id: String,
    /// The caption standard, for example `CEA-608` or `CEA-708`.
    pub standard: String,
    /// The channel or service number; `None` when not stated.
    pub service_number: Option<u8>,
    /// The service's language; `None` when not stated.
    pub language: Option<String>,
}

/// A program of a multi-program container (MPEG transport streams).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Program {
    /// The program number.
    pub id: u32,
    /// The program's name; `None` when absent.
    pub name: Option<String>,
    /// Ids of the streams in the program.
    pub stream_ids: Vec<String>,
    /// The program's duration; `None` when not measured.
    pub duration_seconds: Option<f64>,
}

/// One clip a disc title plays.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscSegment {
    /// The clip's path inside the disc image.
    pub path: String,
    /// Where playback of the clip starts, in seconds.
    pub in_seconds: f64,
    /// Where playback of the clip ends, in seconds.
    pub out_seconds: f64,
    /// The camera angle this segment belongs to.
    pub angle: u32,
    /// The clip's sequence id, for formats that number them; `None` elsewhere.
    pub sequence_id: Option<u8>,
}

/// A playable title on a disc.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscTitle {
    /// The title's identifier, such as a playlist number.
    pub id: String,
    /// Other identifiers that name the same title.
    pub aliases: Vec<String>,
    /// The title's duration; `None` when unknown.
    pub duration_seconds: Option<f64>,
    /// The number of camera angles.
    pub angle_count: u32,
    /// The clips the title plays, in order.
    pub segments: Vec<DiscSegment>,
    /// The title's chapters.
    pub chapters: Vec<Chapter>,
    /// The title's streams.
    pub streams: Vec<StreamDetail>,
    /// The report for inspecting this title.
    pub report: ProbeReport,
}

/// A saved mapping from a disc title to the episodes it holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscEpisodeMapping {
    /// The disc title's id.
    pub disc_title_id: String,
    /// The caller's identifiers for the episodes the title holds.
    pub episode_ids: Vec<String>,
}

/// Which disc title to describe, and any saved episode mappings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscSelection {
    /// The title to describe; `None` lets the probe choose the main title.
    pub title_id: Option<String>,
    /// Saved title-to-episode mappings to check against the disc.
    pub episode_mappings: Vec<DiscEpisodeMapping>,
}

/// What a disc image holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscMetadata {
    /// The disc type, for example `dvd`, `bluray` or `uhd_bluray`.
    pub disc_type: String,
    /// The image's filesystem, for example `udf`.
    pub filesystem: String,
    /// The volume label; `None` when absent.
    pub volume_label: Option<String>,
    /// Every playable title found.
    pub titles: Vec<DiscTitle>,
    /// The title the analysis describes; `None` when none is selected.
    pub selected_title_id: Option<String>,
    /// Whether the probe chose the title rather than the caller.
    pub automatic_selection: bool,
    /// The selection the caller asked for.
    pub selection: DiscSelection,
}

/// Everything a probe found in one file or disc image.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisDetails {
    /// [`ANALYSIS_REVISION`] of the rules that produced this record.
    pub revision: u32,
    /// Container family the parser recognized, for example `matroska`, `mp4`
    /// or `iso` for a disc image.
    pub container_format: Option<String>,
    /// The file's duration in seconds; `None` when it could not be found.
    pub duration_seconds: Option<f64>,
    /// Where the duration came from.
    pub duration_provenance: Provenance,
    /// The file's overall bitrate; `None` when unknown.
    pub overall_bitrate_bps: Option<u64>,
    /// The id of the main video stream; `None` when there is none.
    pub selected_video_id: Option<String>,
    /// The program of the main video stream; `None` outside multi-program
    /// containers.
    pub selected_program_id: Option<u32>,
    /// Every stream, in file order.
    pub streams: Vec<StreamDetail>,
    /// Programs, for multi-program containers.
    pub programs: Vec<Program>,
    /// Chapter markers.
    pub chapters: Vec<Chapter>,
    /// Chapter count in the convention ffprobe reports. Containers whose
    /// chapter markers are counted without being listed in `chapters` still
    /// report a count; `None` means the container was not examined for them.
    pub chapter_count: Option<u32>,
    /// Attached files.
    pub attachments: Vec<Attachment>,
    /// Closed-caption services found inside video streams.
    pub caption_services: Vec<CaptionService>,
    /// Disc inventory, for disc images; `None` for other files.
    pub disc: Option<DiscMetadata>,
    /// How far the probe got.
    pub report: ProbeReport,
    /// Text tags the container attaches to the file as a whole, such as a
    /// title, encoder or creation time. Keys keep the container's own
    /// spelling and case. Values that are binary or not UTF-8 are left out.
    /// Empty when the container has none or the parser does not read them.
    pub tags: BTreeMap<String, String>,
}

impl AnalysisDetails {
    /// The stream the parser selected as the primary picture; `None` when
    /// the file has no video, or only cover art.
    #[must_use]
    pub fn selected_video(&self) -> Option<&StreamDetail> {
        let id = self.selected_video_id.as_ref()?;
        let mut matches = self.streams.iter().filter(|stream| {
            stream.kind == StreamKind::Video && stream.metadata.id.as_ref() == Some(id)
        });
        let first = matches.next()?;
        if matches.next().is_none() {
            return Some(first);
        }
        // A damaged file can give two video streams one id. The selection
        // was the first stream that is a real picture, so the same test
        // finds it again among the streams sharing the id.
        let videos = self.video_streams().count();
        self.streams
            .iter()
            .filter(|stream| {
                stream.kind == StreamKind::Video && stream.metadata.id.as_ref() == Some(id)
            })
            .find(|stream| {
                let disposition = &stream.metadata.disposition;
                if disposition.attached_picture == Some(true)
                    || disposition.still_image == Some(true)
                {
                    return false;
                }
                videos == 1
                    || disposition.attached_picture == Some(false)
                    || disposition.still_image == Some(false)
                    || !matches!(stream.codec, Some(Codec::Mjpeg | Codec::Png))
            })
            .or(Some(first))
    }

    /// Streams of one kind that belong to the selected program, in file order.
    /// Every stream qualifies when no program is selected.
    pub fn program_streams(&self, kind: StreamKind) -> impl Iterator<Item = &StreamDetail> {
        let program = self.selected_program_id;
        self.streams.iter().filter(move |stream| {
            stream.kind == kind && (program.is_none() || stream.metadata.program_id == program)
        })
    }

    /// Every video stream, in file order, across all programs.
    pub fn video_streams(&self) -> impl Iterator<Item = &StreamDetail> {
        self.streams_of(StreamKind::Video)
    }

    /// Every audio stream, in file order, across all programs.
    pub fn audio_streams(&self) -> impl Iterator<Item = &StreamDetail> {
        self.streams_of(StreamKind::Audio)
    }

    /// Every subtitle stream, in file order, across all programs.
    pub fn subtitle_streams(&self) -> impl Iterator<Item = &StreamDetail> {
        self.streams_of(StreamKind::Subtitle)
    }

    fn streams_of(&self, kind: StreamKind) -> impl Iterator<Item = &StreamDetail> {
        self.streams
            .iter()
            .filter(move |stream| stream.kind == kind)
    }

    /// The primary audio stream of the selected program: the first default
    /// stream that is not commentary, else the first that is not commentary.
    /// See [`primary_audio_position`]. `None` when there is no such stream.
    #[must_use]
    pub fn primary_audio(&self) -> Option<&StreamDetail> {
        let index = primary_audio_position(
            self.program_streams(StreamKind::Audio)
                .map(StreamDetail::audio_candidate),
        )?;
        self.program_streams(StreamKind::Audio).nth(index)
    }

    /// The duration; `None` when unknown, negative or not finite.
    #[must_use]
    pub fn duration(&self) -> Option<Duration> {
        self.duration_seconds
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
    }

    /// Whether the file is playable video: a selected video stream with a
    /// codec, and a duration that rounds to at least one second. Executables,
    /// audio-only files and corrupt containers do not qualify.
    #[must_use]
    pub fn is_playable_video(&self) -> bool {
        self.selected_video()
            .is_some_and(|video| video.codec.is_some())
            && self
                .duration_seconds
                .is_some_and(|seconds| seconds.round() > 0.0)
    }

    /// Whether the selected video signals any HDR format.
    #[must_use]
    pub fn is_hdr(&self) -> bool {
        self.selected_video()
            .and_then(StreamDetail::hdr_format)
            .is_some()
    }

    /// The selected video's frame rate as text: its nominal rate with three
    /// decimals and trailing zeros removed, for example `"23.976"` or `"24"`.
    /// A disc title whose clip states no nominal rate falls back to its
    /// declared rate, written with all three decimals. `None` when there is
    /// no selected video or no usable rate.
    #[must_use]
    pub fn frame_rate_text(&self) -> Option<String> {
        let video = self.selected_video()?;
        let nominal = video.metadata.nominal_frame_rate_fps.and_then(|fps| {
            if fps <= 0.0 {
                return None;
            }
            let mut text = format!("{fps:.3}");
            let kept = text.trim_end_matches('0').trim_end_matches('.').len();
            text.truncate(kept);
            Some(text)
        });
        nominal.or_else(|| {
            self.disc.as_ref()?;
            video
                .metadata
                .declared_frame_rate
                .and_then(Rational::as_f64)
                .map(|rate| format!("{rate:.3}"))
        })
    }

    /// Whether a video stream declares stereoscopy; absence is not verified 2D.
    #[must_use]
    pub fn is_3d(&self) -> bool {
        self.streams.iter().any(|stream| {
            stream.kind == StreamKind::Video
                && (stream.metadata.is_3d
                    || (stream.codec == Some(Codec::H264)
                        && matches!(
                            stream.metadata.profile.as_deref(),
                            Some("Multiview High" | "Stereo High")
                        )))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyses_from_before_authored_primary_audio_are_refreshed() {
        // Rows stored at revision 1 picked primary audio by channels and codec.
        let stored_before_policy = 1;
        assert!(stored_before_policy < ANALYSIS_REVISION);
        const { assert!(ANALYSIS_REVISION >= AUTHORED_PRIMARY_AUDIO_REVISION) };
        assert_eq!(
            primary_audio_position([
                AudioCandidate::new(false, false, None),
                AudioCandidate::new(true, false, None),
            ]),
            Some(1)
        );
    }

    #[test]
    fn analyses_from_before_dolby_vision_records_are_refreshed() {
        // Rows stored at revision 2 could report HDR10 for Dolby Vision tracks.
        let stored_before_records = 2;
        assert!(stored_before_records < ANALYSIS_REVISION);
        const { assert!(ANALYSIS_REVISION >= DOLBY_VISION_RECORD_REVISION) };
    }

    #[test]
    fn stereo_boolean_uses_video_metadata_and_ignores_audio() {
        let mut details = AnalysisDetails::default();
        assert!(!details.is_3d());
        details.streams.push(StreamDetail {
            kind: StreamKind::Video,
            codec: Some(Codec::H264),
            metadata: StreamMetadata {
                is_3d: true,
                ..Default::default()
            },
            ..Default::default()
        });
        assert!(details.is_3d());
        details.streams[0].metadata.is_3d = false;
        for (profile, expected) in [
            ("High", false),
            ("Multiview High", true),
            ("Stereo High", true),
        ] {
            details.streams[0].metadata.profile = Some(profile.into());
            assert_eq!(details.is_3d(), expected);
        }
        details.streams[0].kind = StreamKind::Audio;
        details.streams[0].metadata.is_3d = true;
        assert!(!details.is_3d());
    }

    #[test]
    fn rational_preserves_binary_declarations_without_rounding_or_overflow() {
        for value in [
            0.0,
            -0.0,
            25.0,
            23.976,
            30_000.0 / 1001.0,
            -90.5,
            i64::MIN as f64,
        ] {
            assert_eq!(Rational::from_f64(value).unwrap().as_f64(), Some(value));
        }
        assert_eq!(Rational::from_f64(25.0), Rational::new(25, 1));
        assert_eq!(Rational::from_f64(-90.5), Rational::new(-181, 2));
        for value in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MAX,
            f64::MIN_POSITIVE,
            i64::MAX as f64,
        ] {
            assert!(Rational::from_f64(value).is_none());
        }
    }

    #[test]
    fn rational_normalizes_without_overflow() {
        assert_eq!(Rational::new(48_000, 2_002), Rational::new(24_000, 1_001));
        assert_eq!(Rational::new(0, 100).unwrap().denominator, 1);
        assert_eq!(Rational::new(i64::MIN, 1).unwrap().numerator, i64::MIN);
        assert!(Rational::new(1, 0).is_none());
    }

    #[test]
    fn commentary_is_a_flag_or_a_title_word() {
        let named = |title| AudioCandidate::new(false, false, Some(title)).is_commentary();
        assert!(named("Director's Commentary"));
        assert!(named("COMMENTARY-track"));
        assert!(!named("Commentaries"));
        assert!(!named("Main"));
        assert!(AudioCandidate::new(false, true, None).is_commentary());
        assert_eq!(
            primary_audio_position([AudioCandidate::new(true, true, None)]),
            None
        );
    }

    #[test]
    fn duration_rejects_values_a_duration_cannot_hold() {
        let with = |seconds| AnalysisDetails {
            duration_seconds: seconds,
            ..Default::default()
        };
        assert_eq!(
            with(Some(1.5)).duration(),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(with(Some(-1.0)).duration(), None);
        assert_eq!(with(Some(f64::NAN)).duration(), None);
        assert_eq!(with(None).duration(), None);
    }
    #[test]
    fn streams_sharing_an_id_resolve_to_the_selected_picture() {
        let video = |codec: Codec| StreamDetail {
            kind: StreamKind::Video,
            codec: Some(codec),
            metadata: StreamMetadata {
                id: Some("1".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut details = AnalysisDetails {
            selected_video_id: Some("1".to_owned()),
            streams: vec![video(Codec::Mjpeg), video(Codec::H264)],
            ..Default::default()
        };
        assert_eq!(
            details.selected_video().and_then(|v| v.codec.clone()),
            Some(Codec::H264)
        );
        details.streams = vec![video(Codec::Hevc), video(Codec::H264)];
        assert_eq!(
            details.selected_video().and_then(|v| v.codec.clone()),
            Some(Codec::Hevc)
        );
        details.streams = vec![video(Codec::Mjpeg), video(Codec::Png)];
        assert_eq!(
            details.selected_video().and_then(|v| v.codec.clone()),
            Some(Codec::Mjpeg)
        );
    }
}
