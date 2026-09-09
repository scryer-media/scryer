//! Versioned media facts shared by parsers, application DTOs, and rule inputs.
//! This crate contains data only and never opens media or depends on a parser.

use serde::{Deserialize, Serialize};
mod audio;
pub use audio::{audio_codec_rank_for_release_label, normalize_audio_codec_for_release};

pub const ANALYSIS_REVISION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    #[default]
    Unknown,
    Container,
    Bitstream,
    Observed,
    Estimated,
    Legacy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    #[default]
    Unknown,
    Complete,
    Incomplete,
    Unsupported,
    Encrypted,
    Malformed,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeReport {
    pub status: ProbeStatus,
    pub bytes_read: u64,
    pub seeks: u64,
    pub elapsed_ms: u64,
    pub budget_exhausted: bool,
    pub warnings: Vec<ProbeWarning>,
}

/// The latest persisted attempt, separate from the last successful analysis.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisAttempt {
    pub revision: u32,
    pub attempted_at: String,
    pub succeeded: bool,
    pub report: ProbeReport,
    /// Inventory observed by this attempt; it does not replace the saved selection.
    pub disc: Option<DiscMetadata>,
}

/// Physical source ranges read by diagnostics; overlapping reads are merged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagnosticCoverage {
    pub offset: u64,
    pub length: u64,
}

/// Header sampling is not a complete-file integrity check and never decodes media.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StructuralDiagnostics {
    pub report: ProbeReport,
    pub source_length: u64,
    pub coverage: Vec<DiagnosticCoverage>,
    pub coverage_truncated: bool,
    pub warnings_truncated: bool,
    pub packets_sampled: u64,
    pub frame_headers_sampled: u64,
    pub checks: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeWarning {
    pub code: String,
    pub message: String,
    pub stream_id: Option<String>,
    pub offset: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rational {
    pub numerator: i64,
    pub denominator: u64,
}

impl Rational {
    /// Preserve a declared binary floating-point value exactly, when it fits.
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

    pub fn as_f64(self) -> Option<f64> {
        (self.denominator != 0).then(|| self.numerator as f64 / self.denominator as f64)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamDisposition {
    pub default: Option<bool>,
    pub forced: Option<bool>,
    pub original: Option<bool>,
    pub commentary: Option<bool>,
    pub hearing_impaired: Option<bool>,
    pub visual_impaired: Option<bool>,
    pub attached_picture: Option<bool>,
    pub still_image: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorMetadata {
    pub primaries: Option<u32>,
    pub transfer: Option<u32>,
    pub matrix: Option<u32>,
    pub full_range: Option<bool>,
    pub provenance: Provenance,
    pub mastering_display: Option<MasteringDisplay>,
    pub content_light: Option<ContentLight>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MasteringDisplay {
    pub red_x: Option<f64>,
    pub red_y: Option<f64>,
    pub green_x: Option<f64>,
    pub green_y: Option<f64>,
    pub blue_x: Option<f64>,
    pub blue_y: Option<f64>,
    pub white_x: Option<f64>,
    pub white_y: Option<f64>,
    pub min_luminance: Option<f64>,
    pub max_luminance: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentLight {
    pub max_cll: Option<u32>,
    pub max_fall: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DolbyVision {
    pub profile: Option<u8>,
    pub level: Option<u8>,
    pub base_layer_compatibility_id: Option<u8>,
    pub rpu_present: Option<bool>,
    pub enhancement_layer_present: Option<bool>,
    pub base_layer_present: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HdrCapabilities {
    pub dolby_vision: Option<bool>,
    pub hdr10plus: Option<bool>,
    pub hdr10: Option<bool>,
    pub hlg: Option<bool>,
    pub pq: Option<bool>,
    pub dovi: Option<DolbyVision>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamMetadata {
    pub id: Option<String>,
    pub program_id: Option<u32>,
    pub original_language: Option<String>,
    pub language_provenance: Provenance,
    pub disposition: StreamDisposition,
    pub duration_seconds: Option<f64>,
    pub bitrate_bps: Option<u64>,
    pub bitrate_provenance: Provenance,
    pub estimated_bitrate_bps: Option<u64>,
    pub sample_rate: Option<u32>,
    /// Encoded PCM representation (for example s24le), not a decoder's output format.
    /// Compressed streams without a declared sample representation remain unknown.
    pub sample_format: Option<String>,
    pub sample_bit_depth: Option<u32>,
    pub channel_layout: Option<String>,
    pub pixel_format: Option<String>,
    pub profile: Option<String>,
    pub level: Option<u32>,
    pub bit_depth: Option<i32>,
    pub field_order: Option<String>,
    pub sample_aspect_ratio: Option<Rational>,
    pub display_aspect_ratio: Option<Rational>,
    pub rotation_degrees: Option<f64>,
    pub declared_frame_rate: Option<Rational>,
    pub observed_frame_rate: Option<Rational>,
    pub variable_frame_rate: Option<bool>,
    pub color: ColorMetadata,
    pub hdr: HdrCapabilities,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    #[default]
    Video,
    Audio,
    Subtitle,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamDetail {
    pub kind: StreamKind,
    pub codec: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub channels: Option<i32>,
    pub language: Option<String>,
    pub name: Option<String>,
    pub metadata: StreamMetadata,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Chapter {
    pub id: String,
    pub title: Option<String>,
    pub start_seconds: f64,
    pub end_seconds: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Attachment {
    pub id: String,
    pub name: Option<String>,
    pub media_type: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptionService {
    pub stream_id: String,
    pub standard: String,
    pub service_number: Option<u8>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Program {
    pub id: u32,
    pub name: Option<String>,
    pub stream_ids: Vec<String>,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscSegment {
    pub path: String,
    pub in_seconds: f64,
    pub out_seconds: f64,
    pub angle: u32,
    pub sequence_id: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscTitle {
    pub id: String,
    pub aliases: Vec<String>,
    pub duration_seconds: Option<f64>,
    pub angle_count: u32,
    pub segments: Vec<DiscSegment>,
    pub chapters: Vec<Chapter>,
    pub streams: Vec<StreamDetail>,
    pub report: ProbeReport,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscEpisodeMapping {
    pub disc_title_id: String,
    pub episode_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscSelection {
    pub title_id: Option<String>,
    pub episode_mappings: Vec<DiscEpisodeMapping>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscMetadata {
    pub disc_type: String,
    pub filesystem: String,
    pub volume_label: Option<String>,
    pub titles: Vec<DiscTitle>,
    pub selected_title_id: Option<String>,
    pub automatic_selection: bool,
    pub selection: DiscSelection,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisDetails {
    pub revision: u32,
    pub duration_seconds: Option<f64>,
    pub duration_provenance: Provenance,
    pub overall_bitrate_bps: Option<u64>,
    pub selected_video_id: Option<String>,
    pub selected_program_id: Option<u32>,
    pub streams: Vec<StreamDetail>,
    pub programs: Vec<Program>,
    pub chapters: Vec<Chapter>,
    pub attachments: Vec<Attachment>,
    pub caption_services: Vec<CaptionService>,
    pub disc: Option<DiscMetadata>,
    pub report: ProbeReport,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
