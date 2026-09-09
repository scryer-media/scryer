//! Typed GraphQL projections of the shared media analysis contract.
use super::Long;
use async_graphql::{Enum, ID, InputObject, SimpleObject};

#[derive(InputObject)]
/// Disc title override and explicit episode mappings for one intact image.
pub struct MediaDiscSelectionInput {
    /// Manual authored title ID; null requests automatic selection.
    pub title_id: Option<String>,
    /// Explicit authored-title-to-episode associations for this physical image.
    pub episode_mappings: Option<Vec<MediaDiscEpisodeMappingInput>>,
}
impl From<MediaDiscSelectionInput> for scryer_media_types::DiscSelection {
    fn from(value: MediaDiscSelectionInput) -> Self {
        Self {
            title_id: value.title_id,
            episode_mappings: value
                .episode_mappings
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

#[derive(InputObject)]
/// Associate one complete authored disc title with one catalogued episode.
pub struct MediaDiscEpisodeMappingInput {
    /// Authored disc title or playlist ID.
    pub disc_title_id: String,
    /// Catalogued episode ID.
    pub episode_id: ID,
}

#[derive(SimpleObject)]
/// An episode eligible for explicit disc-title mapping.
pub struct MediaDiscEpisodeTargetPayload {
    /// Catalogued episode ID.
    pub episode_id: ID,
    /// Display label for this episode.
    pub label: String,
    /// Playback duration in seconds, when established.
    pub duration_seconds: Option<Long>,
}
impl From<MediaDiscEpisodeMappingInput> for scryer_media_types::DiscEpisodeMapping {
    fn from(value: MediaDiscEpisodeMappingInput) -> Self {
        Self {
            disc_title_id: value.disc_title_id,
            episode_ids: vec![value.episode_id.to_string()],
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
/// Evidence source for a probed media fact.
pub enum MediaProvenanceValue {
    /// Evidence source is unknown.
    Unknown,
    /// Read from container metadata.
    Container,
    /// Read from encoded media headers.
    Bitstream,
    /// Derived from bounded observations.
    Observed,
    /// Estimated from a bounded sample.
    Estimated,
    /// Historical value with unverified provenance.
    Legacy,
}
impl From<scryer_media_types::Provenance> for MediaProvenanceValue {
    fn from(value: scryer_media_types::Provenance) -> Self {
        match value {
            scryer_media_types::Provenance::Unknown => Self::Unknown,
            scryer_media_types::Provenance::Container => Self::Container,
            scryer_media_types::Provenance::Bitstream => Self::Bitstream,
            scryer_media_types::Provenance::Observed => Self::Observed,
            scryer_media_types::Provenance::Estimated => Self::Estimated,
            scryer_media_types::Provenance::Legacy => Self::Legacy,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
/// Outcome of bounded probing; complete metadata does not guarantee complete-file integrity.
pub enum MediaProbeStatusValue {
    /// No usable analysis status is available.
    Unknown,
    /// Configured metadata inspection completed; payload integrity is not guaranteed.
    Complete,
    /// Some requested facts could not be established within available coverage.
    Incomplete,
    /// Required container or filesystem features are not implemented.
    Unsupported,
    /// Encrypted payload prevents supported inspection.
    Encrypted,
    /// Inspected structures violate the supported format.
    Malformed,
}
impl From<scryer_media_types::ProbeStatus> for MediaProbeStatusValue {
    fn from(value: scryer_media_types::ProbeStatus) -> Self {
        match value {
            scryer_media_types::ProbeStatus::Unknown => Self::Unknown,
            scryer_media_types::ProbeStatus::Complete => Self::Complete,
            scryer_media_types::ProbeStatus::Incomplete => Self::Incomplete,
            scryer_media_types::ProbeStatus::Unsupported => Self::Unsupported,
            scryer_media_types::ProbeStatus::Encrypted => Self::Encrypted,
            scryer_media_types::ProbeStatus::Malformed => Self::Malformed,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
/// Media carried by a container stream.
pub enum MediaStreamKindValue {
    /// Video stream.
    Video,
    /// Audio stream.
    Audio,
    /// Subtitle stream.
    Subtitle,
}
impl From<scryer_media_types::StreamKind> for MediaStreamKindValue {
    fn from(value: scryer_media_types::StreamKind) -> Self {
        match value {
            scryer_media_types::StreamKind::Video => Self::Video,
            scryer_media_types::StreamKind::Audio => Self::Audio,
            scryer_media_types::StreamKind::Subtitle => Self::Subtitle,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Latest analysis attempt, retained separately from the last successful metadata.
pub struct MediaAnalysisAttemptPayload {
    /// Analysis schema revision; zero denotes legacy metadata.
    pub revision: i64,
    /// Timestamp of the analysis attempt in RFC 3339 form.
    pub attempted_at: String,
    /// Whether this attempt published successful metadata.
    pub succeeded: bool,
    /// Inspection status, coverage counters, and warnings.
    pub report: MediaProbeReportPayload,
    /// Detected disc metadata, when this source is a disc image.
    pub disc: Option<MediaDiscMetadataPayload>,
}
impl From<scryer_media_types::AnalysisAttempt> for MediaAnalysisAttemptPayload {
    fn from(value: scryer_media_types::AnalysisAttempt) -> Self {
        Self {
            revision: i64::from(value.revision),
            attempted_at: value.attempted_at,
            succeeded: value.succeeded,
            report: value.report.into(),
            disc: value.disc.map(Into::into),
        }
    }
}

#[test]
fn analysis_attempt_projection_keeps_failure_separate_and_exposes_typed_fields() {
    let legacy: scryer_media_types::AnalysisAttempt = serde_json::from_value(serde_json::json!({
        "revision": 1, "attempted_at": "2026-09-08T00:00:00Z", "succeeded": false,
        "report": { "status": "incomplete" }
    }))
    .unwrap();
    assert!(legacy.disc.is_none());
    assert_eq!(
        legacy.report.status,
        scryer_media_types::ProbeStatus::Incomplete
    );
    let attempt = scryer_media_types::AnalysisAttempt {
        revision: 3,
        attempted_at: "2026-09-08T12:00:00Z".into(),
        succeeded: false,
        disc: Some(scryer_media_types::DiscMetadata {
            titles: vec![scryer_media_types::DiscTitle {
                id: "00077".into(),
                ..Default::default()
            }],
            ..Default::default()
        }),
        report: scryer_media_types::ProbeReport {
            status: scryer_media_types::ProbeStatus::Encrypted,
            warnings: vec![scryer_media_types::ProbeWarning {
                code: "encrypted_payload".into(),
                message: "The title payload is encrypted".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
    };
    let payload = MediaAnalysisAttemptPayload::from(attempt);
    assert_eq!(payload.revision, 3);
    assert!(!payload.succeeded);
    assert_eq!(payload.report.status, MediaProbeStatusValue::Encrypted);
    assert_eq!(payload.report.warnings[0].code, "encrypted_payload");
    assert_eq!(payload.disc.as_ref().unwrap().titles[0].id, "00077");
    let schema = async_graphql::Schema::build(
        payload,
        async_graphql::EmptyMutation,
        async_graphql::EmptySubscription,
    )
    .finish()
    .sdl();
    for field in [
        "attemptedAt: String!",
        "succeeded: Boolean!",
        "report: MediaProbeReportPayload!",
        "disc: MediaDiscMetadataPayload",
        "status: MediaProbeStatusValue!",
        "warnings: [MediaProbeWarningPayload!]!",
    ] {
        assert!(
            schema.contains(field),
            "missing typed field {field}: {schema}"
        );
    }
}

#[derive(SimpleObject, Clone)]
/// Probe coverage, resource usage, and reasons for incomplete metadata.
pub struct MediaProbeReportPayload {
    /// Outcome of this bounded inspection.
    pub status: MediaProbeStatusValue,
    /// Total bytes read while probing, including repeated reads.
    pub bytes_read: Long,
    /// Number of seek operations performed.
    pub seeks: Long,
    /// Elapsed probe time in milliseconds.
    pub elapsed_ms: Long,
    /// Whether any configured inspection budget was exhausted.
    pub budget_exhausted: bool,
    /// Structured warnings explaining conflicts, unsupported features, or incomplete coverage.
    pub warnings: Vec<MediaProbeWarningPayload>,
}
impl From<scryer_media_types::ProbeReport> for MediaProbeReportPayload {
    fn from(value: scryer_media_types::ProbeReport) -> Self {
        Self {
            status: value.status.into(),
            bytes_read: Long::from(i64::try_from(value.bytes_read).unwrap_or(i64::MAX)),
            seeks: Long::from(i64::try_from(value.seeks).unwrap_or(i64::MAX)),
            elapsed_ms: Long::from(i64::try_from(value.elapsed_ms).unwrap_or(i64::MAX)),
            budget_exhausted: value.budget_exhausted,
            warnings: value
                .warnings
                .into_iter()
                .map(|value| value.into())
                .collect(),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// A structured warning from bounded media inspection.
pub struct MediaProbeWarningPayload {
    /// Stable warning code.
    pub code: String,
    /// Human-readable warning details.
    pub message: String,
    /// Container stream ID associated with this result.
    pub stream_id: Option<String>,
    /// Byte offset from the start of the inspected source.
    pub offset: Option<Long>,
}
impl From<scryer_media_types::ProbeWarning> for MediaProbeWarningPayload {
    fn from(value: scryer_media_types::ProbeWarning) -> Self {
        Self {
            code: value.code,
            message: value.message,
            stream_id: value.stream_id,
            offset: value
                .offset
                .map(|value| Long::from(i64::try_from(value).unwrap_or(i64::MAX))),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Byte range read during structural diagnostics.
pub struct MediaDiagnosticCoveragePayload {
    /// Byte offset from the start of the inspected source.
    pub offset: Long,
    /// Length of this inspected range in bytes.
    pub length: Long,
}

#[derive(SimpleObject, Clone)]
/// Opt-in packet and frame-header sampling without decoding media.
pub struct MediaStructuralDiagnosticsPayload {
    /// Inspection status, coverage counters, and warnings.
    pub report: MediaProbeReportPayload,
    /// Length of the complete physical source in bytes.
    pub source_length: Long,
    /// Recorded byte ranges read during diagnostics.
    pub coverage: Vec<MediaDiagnosticCoveragePayload>,
    /// Whether the bounded coverage inventory omitted additional ranges.
    pub coverage_truncated: bool,
    /// Whether additional diagnostic warnings exceeded the inventory limit.
    pub warnings_truncated: bool,
    /// Number of packet headers inspected.
    pub packets_sampled: Long,
    /// Number of elementary frame headers inspected.
    pub frame_headers_sampled: Long,
    /// Names of structural checks performed.
    pub checks: Vec<String>,
}
impl From<scryer_media_types::StructuralDiagnostics> for MediaStructuralDiagnosticsPayload {
    fn from(value: scryer_media_types::StructuralDiagnostics) -> Self {
        fn long(value: u64) -> Long {
            Long::from(i64::try_from(value).unwrap_or(i64::MAX))
        }
        Self {
            report: value.report.into(),
            source_length: long(value.source_length),
            coverage: value
                .coverage
                .into_iter()
                .map(|range| MediaDiagnosticCoveragePayload {
                    offset: long(range.offset),
                    length: long(range.length),
                })
                .collect(),
            coverage_truncated: value.coverage_truncated,
            warnings_truncated: value.warnings_truncated,
            packets_sampled: long(value.packets_sampled),
            frame_headers_sampled: long(value.frame_headers_sampled),
            checks: value.checks,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Exact rational value with a nonzero denominator.
pub struct MediaRationalPayload {
    /// Signed numerator.
    pub numerator: Long,
    /// Positive denominator.
    pub denominator: Long,
}
impl From<scryer_media_types::Rational> for MediaRationalPayload {
    fn from(value: scryer_media_types::Rational) -> Self {
        Self {
            numerator: Long::from(value.numerator),
            denominator: Long::from(i64::try_from(value.denominator).unwrap_or(i64::MAX)),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Declared stream roles; null means the role is unknown.
pub struct MediaStreamDispositionPayload {
    /// Whether the container marks this stream as the default.
    pub default: Option<bool>,
    /// Whether the container marks this stream as forced.
    pub forced: Option<bool>,
    /// Whether the container marks this as the original track.
    pub original: Option<bool>,
    /// Whether this track is explicitly marked as commentary.
    pub commentary: Option<bool>,
    /// Whether this track is marked for viewers with hearing impairments.
    pub hearing_impaired: Option<bool>,
    /// Whether this track is marked as audio description.
    pub visual_impaired: Option<bool>,
    /// Whether this stream is an attached cover picture.
    pub attached_picture: Option<bool>,
    /// Whether this stream is explicitly marked as a still image.
    pub still_image: Option<bool>,
}
impl From<scryer_media_types::StreamDisposition> for MediaStreamDispositionPayload {
    fn from(value: scryer_media_types::StreamDisposition) -> Self {
        Self {
            default: value.default,
            forced: value.forced,
            original: value.original,
            commentary: value.commentary,
            hearing_impaired: value.hearing_impaired,
            visual_impaired: value.visual_impaired,
            attached_picture: value.attached_picture,
            still_image: value.still_image,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Color signaling and static HDR metadata, with evidence provenance.
pub struct MediaColorMetadataPayload {
    /// Numeric color primaries code.
    pub primaries: Option<u32>,
    /// Numeric transfer characteristic code.
    pub transfer: Option<u32>,
    /// Numeric matrix coefficients code.
    pub matrix: Option<u32>,
    /// Whether full-range samples are explicitly signaled.
    pub full_range: Option<bool>,
    /// Evidence source used for these values.
    pub provenance: MediaProvenanceValue,
    /// Static mastering display metadata, when present.
    pub mastering_display: Option<MediaMasteringDisplayPayload>,
    /// Static content light metadata, when present.
    pub content_light: Option<MediaContentLightPayload>,
}
impl From<scryer_media_types::ColorMetadata> for MediaColorMetadataPayload {
    fn from(value: scryer_media_types::ColorMetadata) -> Self {
        Self {
            primaries: value.primaries,
            transfer: value.transfer,
            matrix: value.matrix,
            full_range: value.full_range,
            provenance: value.provenance.into(),
            mastering_display: value.mastering_display.map(|value| value.into()),
            content_light: value.content_light.map(|value| value.into()),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Mastering display chromaticity coordinates and luminance.
pub struct MediaMasteringDisplayPayload {
    /// X chromaticity coordinate of the red primary.
    pub red_x: Option<f64>,
    /// Y chromaticity coordinate of the red primary.
    pub red_y: Option<f64>,
    /// X chromaticity coordinate of the green primary.
    pub green_x: Option<f64>,
    /// Y chromaticity coordinate of the green primary.
    pub green_y: Option<f64>,
    /// X chromaticity coordinate of the blue primary.
    pub blue_x: Option<f64>,
    /// Y chromaticity coordinate of the blue primary.
    pub blue_y: Option<f64>,
    /// X chromaticity coordinate of the white point.
    pub white_x: Option<f64>,
    /// Y chromaticity coordinate of the white point.
    pub white_y: Option<f64>,
    /// Minimum mastering display luminance in candelas per square metre.
    pub min_luminance: Option<f64>,
    /// Maximum mastering display luminance in candelas per square metre.
    pub max_luminance: Option<f64>,
}
impl From<scryer_media_types::MasteringDisplay> for MediaMasteringDisplayPayload {
    fn from(value: scryer_media_types::MasteringDisplay) -> Self {
        Self {
            red_x: value.red_x,
            red_y: value.red_y,
            green_x: value.green_x,
            green_y: value.green_y,
            blue_x: value.blue_x,
            blue_y: value.blue_y,
            white_x: value.white_x,
            white_y: value.white_y,
            min_luminance: value.min_luminance,
            max_luminance: value.max_luminance,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Content light level metadata in candelas per square metre.
pub struct MediaContentLightPayload {
    /// Maximum content light level in candelas per square metre.
    pub max_cll: Option<u32>,
    /// Maximum frame-average light level in candelas per square metre.
    pub max_fall: Option<u32>,
}
impl From<scryer_media_types::ContentLight> for MediaContentLightPayload {
    fn from(value: scryer_media_types::ContentLight) -> Self {
        Self {
            max_cll: value.max_cll,
            max_fall: value.max_fall,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Dolby Vision configuration reported by the container or bitstream.
pub struct MediaDolbyVisionPayload {
    /// Codec profile reported for this stream.
    pub profile: Option<u8>,
    /// Codec-specific level identifier.
    pub level: Option<u8>,
    /// Dolby Vision base layer compatibility identifier.
    pub base_layer_compatibility_id: Option<u8>,
    /// Whether Dolby Vision reference processing metadata is signaled.
    pub rpu_present: Option<bool>,
    /// Whether a Dolby Vision enhancement layer is signaled.
    pub enhancement_layer_present: Option<bool>,
    /// Whether a Dolby Vision base layer is signaled.
    pub base_layer_present: Option<bool>,
}
impl From<scryer_media_types::DolbyVision> for MediaDolbyVisionPayload {
    fn from(value: scryer_media_types::DolbyVision) -> Self {
        Self {
            profile: value.profile,
            level: value.level,
            base_layer_compatibility_id: value.base_layer_compatibility_id,
            rpu_present: value.rpu_present,
            enhancement_layer_present: value.enhancement_layer_present,
            base_layer_present: value.base_layer_present,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Independent HDR capabilities; combinations are preserved and null means unknown.
pub struct MediaHdrCapabilitiesPayload {
    /// Whether Dolby Vision is established by the available evidence.
    pub dolby_vision: Option<bool>,
    /// Whether HDR10+ dynamic metadata was detected.
    pub hdr10plus: Option<bool>,
    /// Whether the available bit-depth and color evidence establishes HDR10.
    pub hdr10: Option<bool>,
    /// Whether Hybrid Log-Gamma transfer is signaled.
    pub hlg: Option<bool>,
    /// Whether Perceptual Quantizer transfer is signaled; this alone does not establish HDR10.
    pub pq: Option<bool>,
    /// Dolby Vision configuration details.
    pub dovi: Option<MediaDolbyVisionPayload>,
}
impl From<scryer_media_types::HdrCapabilities> for MediaHdrCapabilitiesPayload {
    fn from(value: scryer_media_types::HdrCapabilities) -> Self {
        Self {
            dolby_vision: value.dolby_vision,
            hdr10plus: value.hdr10plus,
            hdr10: value.hdr10,
            hlg: value.hlg,
            pq: value.pq,
            dovi: value.dovi.map(|value| value.into()),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Precise properties of one stream; missing values remain unknown.
pub struct MediaStreamMetadataPayload {
    /// Identifier within the source metadata.
    pub id: Option<String>,
    /// Program containing this stream, when known.
    pub program_id: Option<u32>,
    /// Original container language tag before normalization.
    pub original_language: Option<String>,
    /// Evidence source for the language tag.
    pub language_provenance: MediaProvenanceValue,
    /// Declared stream roles and accessibility flags.
    pub disposition: MediaStreamDispositionPayload,
    /// Playback duration in seconds, when established.
    pub duration_seconds: Option<f64>,
    /// Declared or completely accounted stream bitrate in bits per second.
    pub bitrate_bps: Option<Long>,
    /// Evidence source for the stream bitrate.
    pub bitrate_provenance: MediaProvenanceValue,
    /// Bounded stream bitrate estimate in bits per second, separate from measured bitrate.
    pub estimated_bitrate_bps: Option<Long>,
    /// Audio sample rate in samples per second.
    pub sample_rate: Option<u32>,
    /// Encoded sample representation, when known; no decoder output format is inferred.
    pub sample_format: Option<String>,
    /// Encoded audio sample precision in bits.
    pub sample_bit_depth: Option<u32>,
    /// Declared speaker layout; unknown layouts remain null.
    pub channel_layout: Option<String>,
    /// Video pixel sampling and component representation.
    pub pixel_format: Option<String>,
    /// Codec profile reported for this stream.
    pub profile: Option<String>,
    /// Codec-specific level identifier.
    pub level: Option<u32>,
    /// Video component precision in bits.
    pub bit_depth: Option<i32>,
    /// Declared or observed field order.
    pub field_order: Option<String>,
    /// Pixel width-to-height ratio.
    pub sample_aspect_ratio: Option<MediaRationalPayload>,
    /// Displayed image width-to-height ratio.
    pub display_aspect_ratio: Option<MediaRationalPayload>,
    /// Display rotation in degrees.
    pub rotation_degrees: Option<f64>,
    /// Container or bitstream frame-rate declaration in frames per second.
    pub declared_frame_rate: Option<MediaRationalPayload>,
    /// Rational frame rate from bounded timestamp observations.
    pub observed_frame_rate: Option<MediaRationalPayload>,
    /// Whether bounded timing observations establish variable frame rate; null is inconclusive.
    pub variable_frame_rate: Option<bool>,
    /// Color signaling and static HDR metadata.
    pub color: MediaColorMetadataPayload,
    /// Independent HDR capability evidence.
    pub hdr: MediaHdrCapabilitiesPayload,
}
impl From<scryer_media_types::StreamMetadata> for MediaStreamMetadataPayload {
    fn from(value: scryer_media_types::StreamMetadata) -> Self {
        Self {
            id: value.id,
            program_id: value.program_id,
            original_language: value.original_language,
            language_provenance: value.language_provenance.into(),
            disposition: value.disposition.into(),
            duration_seconds: value.duration_seconds,
            bitrate_bps: value
                .bitrate_bps
                .map(|value| Long::from(i64::try_from(value).unwrap_or(i64::MAX))),
            bitrate_provenance: value.bitrate_provenance.into(),
            estimated_bitrate_bps: value
                .estimated_bitrate_bps
                .map(|value| Long::from(i64::try_from(value).unwrap_or(i64::MAX))),
            sample_rate: value.sample_rate,
            sample_format: value.sample_format,
            sample_bit_depth: value.sample_bit_depth,
            channel_layout: value.channel_layout,
            pixel_format: value.pixel_format,
            profile: value.profile,
            level: value.level,
            bit_depth: value.bit_depth,
            field_order: value.field_order,
            sample_aspect_ratio: value.sample_aspect_ratio.map(|value| value.into()),
            display_aspect_ratio: value.display_aspect_ratio.map(|value| value.into()),
            rotation_degrees: value.rotation_degrees,
            declared_frame_rate: value.declared_frame_rate.map(|value| value.into()),
            observed_frame_rate: value.observed_frame_rate.map(|value| value.into()),
            variable_frame_rate: value.variable_frame_rate,
            color: value.color.into(),
            hdr: value.hdr.into(),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// One detected stream, including supplementary streams excluded from scoring.
pub struct MediaStreamDetailPayload {
    /// Video, audio, or subtitle stream kind.
    pub kind: MediaStreamKindValue,
    /// Normalized codec name.
    pub codec: Option<String>,
    /// Video width in pixels.
    pub width: Option<i32>,
    /// Video height in pixels.
    pub height: Option<i32>,
    /// Number of audio channels; this does not imply a speaker layout.
    pub channels: Option<i32>,
    /// Normalized language tag, when known.
    pub language: Option<String>,
    /// Name supplied by the container metadata.
    pub name: Option<String>,
    /// Precise stream properties and evidence.
    pub metadata: MediaStreamMetadataPayload,
}
impl From<scryer_media_types::StreamDetail> for MediaStreamDetailPayload {
    fn from(value: scryer_media_types::StreamDetail) -> Self {
        Self {
            kind: value.kind.into(),
            codec: value.codec,
            width: value.width,
            height: value.height,
            channels: value.channels,
            language: value.language,
            name: value.name,
            metadata: value.metadata.into(),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Chapter timing on the relevant media or selected-title timeline.
pub struct MediaChapterPayload {
    /// Identifier within the source metadata.
    pub id: String,
    /// Authored chapter title, when supplied.
    pub title: Option<String>,
    /// Chapter start time in seconds.
    pub start_seconds: f64,
    /// Chapter end time in seconds, when known.
    pub end_seconds: Option<f64>,
}
impl From<scryer_media_types::Chapter> for MediaChapterPayload {
    fn from(value: scryer_media_types::Chapter) -> Self {
        Self {
            id: value.id,
            title: value.title,
            start_seconds: value.start_seconds,
            end_seconds: value.end_seconds,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Embedded attachment inventory without extracting the attachment.
pub struct MediaAttachmentPayload {
    /// Identifier within the source metadata.
    pub id: String,
    /// Name supplied by the container metadata.
    pub name: Option<String>,
    /// Attachment media type, when supplied.
    pub media_type: Option<String>,
    /// Attachment size in bytes.
    pub size_bytes: Long,
}
impl From<scryer_media_types::Attachment> for MediaAttachmentPayload {
    fn from(value: scryer_media_types::Attachment) -> Self {
        Self {
            id: value.id,
            name: value.name,
            media_type: value.media_type,
            size_bytes: Long::from(i64::try_from(value.size_bytes).unwrap_or(i64::MAX)),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Caption service detected in sampled headers; no transcription is performed.
pub struct MediaCaptionServicePayload {
    /// Container stream ID associated with this result.
    pub stream_id: String,
    /// Caption standard, such as CEA-608 or CEA-708.
    pub standard: String,
    /// Caption service number, when available.
    pub service_number: Option<u8>,
    /// Normalized language tag, when known.
    pub language: Option<String>,
}
impl From<scryer_media_types::CaptionService> for MediaCaptionServicePayload {
    fn from(value: scryer_media_types::CaptionService) -> Self {
        Self {
            stream_id: value.stream_id,
            standard: value.standard,
            service_number: value.service_number,
            language: value.language,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Container program and its member streams.
pub struct MediaProgramPayload {
    /// Identifier within the source metadata.
    pub id: u32,
    /// Name supplied by the container metadata.
    pub name: Option<String>,
    /// Container stream IDs belonging to this program.
    pub stream_ids: Vec<String>,
    /// Playback duration in seconds, when established.
    pub duration_seconds: Option<f64>,
}
impl From<scryer_media_types::Program> for MediaProgramPayload {
    fn from(value: scryer_media_types::Program) -> Self {
        Self {
            id: value.id,
            name: value.name,
            stream_ids: value.stream_ids,
            duration_seconds: value.duration_seconds,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// One ordered segment and trim range in an authored playback sequence.
pub struct MediaDiscSegmentPayload {
    /// Referenced media path inside the disc image.
    pub path: String,
    /// Playback trim start in seconds.
    pub in_seconds: f64,
    /// Playback trim end in seconds.
    pub out_seconds: f64,
    /// Selected authored angle number.
    pub angle: u32,
    /// Authored sequence identifier used to preserve playback ordering.
    pub sequence_id: Option<i32>,
}
impl From<scryer_media_types::DiscSegment> for MediaDiscSegmentPayload {
    fn from(value: scryer_media_types::DiscSegment) -> Self {
        Self {
            path: value.path,
            in_seconds: value.in_seconds,
            out_seconds: value.out_seconds,
            angle: value.angle,
            sequence_id: value.sequence_id.map(i32::from),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Distinct authored playback title, its timeline, and inspected stream facts.
pub struct MediaDiscTitlePayload {
    /// Identifier within the source metadata.
    pub id: String,
    /// Other title IDs with the same ordered segment and trim sequence.
    pub aliases: Vec<String>,
    /// Playback duration in seconds, when established.
    pub duration_seconds: Option<f64>,
    /// Number of authored angles reported for this title.
    pub angle_count: u32,
    /// Ordered playback segments including trims.
    pub segments: Vec<MediaDiscSegmentPayload>,
    /// Authored chapter titles and timing.
    pub chapters: Vec<MediaChapterPayload>,
    /// Detected streams in container order, including supplementary tracks.
    pub streams: Vec<MediaStreamDetailPayload>,
    /// Inspection status, coverage counters, and warnings.
    pub report: MediaProbeReportPayload,
}
impl From<scryer_media_types::DiscTitle> for MediaDiscTitlePayload {
    fn from(value: scryer_media_types::DiscTitle) -> Self {
        Self {
            id: value.id,
            aliases: value.aliases,
            duration_seconds: value.duration_seconds,
            angle_count: value.angle_count,
            segments: value
                .segments
                .into_iter()
                .map(|value| value.into())
                .collect(),
            chapters: value
                .chapters
                .into_iter()
                .map(|value| value.into())
                .collect(),
            streams: value
                .streams
                .into_iter()
                .map(|value| value.into())
                .collect(),
            report: value.report.into(),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Persisted association between an authored disc title and catalogued episodes.
pub struct MediaDiscEpisodeMappingPayload {
    /// Authored disc title or playlist ID.
    pub disc_title_id: String,
    /// Catalogued episodes explicitly associated with this authored title.
    pub episode_ids: Vec<String>,
}
impl From<scryer_media_types::DiscEpisodeMapping> for MediaDiscEpisodeMappingPayload {
    fn from(value: scryer_media_types::DiscEpisodeMapping) -> Self {
        Self {
            disc_title_id: value.disc_title_id,
            episode_ids: value.episode_ids,
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Saved manual title override and episode mappings.
pub struct MediaDiscSelectionPayload {
    /// Manual authored title ID; null requests automatic selection.
    pub title_id: Option<String>,
    /// Explicit authored-title-to-episode associations for this physical image.
    pub episode_mappings: Vec<MediaDiscEpisodeMappingPayload>,
}
impl From<scryer_media_types::DiscSelection> for MediaDiscSelectionPayload {
    fn from(value: scryer_media_types::DiscSelection) -> Self {
        Self {
            title_id: value.title_id,
            episode_mappings: value
                .episode_mappings
                .into_iter()
                .map(|value| value.into())
                .collect(),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Detected disc structure, title inventory, and current selection.
pub struct MediaDiscMetadataPayload {
    /// Detected disc type, independent of the physical file extension.
    pub disc_type: String,
    /// Detected image filesystem.
    pub filesystem: String,
    /// Filesystem volume label, when present.
    pub volume_label: Option<String>,
    /// Distinct authored titles retained by bounded inspection.
    pub titles: Vec<MediaDiscTitlePayload>,
    /// Currently resolved authored title ID; null means no valid selection.
    pub selected_title_id: Option<String>,
    /// Whether the current title was chosen automatically.
    pub automatic_selection: bool,
    /// Persisted override and explicit episode mappings.
    pub selection: MediaDiscSelectionPayload,
}
impl From<scryer_media_types::DiscMetadata> for MediaDiscMetadataPayload {
    fn from(value: scryer_media_types::DiscMetadata) -> Self {
        Self {
            disc_type: value.disc_type,
            filesystem: value.filesystem,
            volume_label: value.volume_label,
            titles: value.titles.into_iter().map(|value| value.into()).collect(),
            selected_title_id: value.selected_title_id,
            automatic_selection: value.automatic_selection,
            selection: value.selection.into(),
        }
    }
}

#[derive(SimpleObject, Clone)]
/// Versioned native analysis shared by imports, scans, rules, and media details.
pub struct MediaAnalysisDetailsPayload {
    /// Analysis schema revision; zero denotes legacy metadata.
    pub revision: u32,
    /// Playback duration in seconds, when established.
    pub duration_seconds: Option<f64>,
    /// Evidence source for playback duration.
    pub duration_provenance: MediaProvenanceValue,
    /// Overall source bitrate in bits per second, distinct from video bitrate.
    pub overall_bitrate_bps: Option<Long>,
    /// Container ID of the primary non-cover video stream.
    pub selected_video_id: Option<String>,
    /// Program used for summary and scoring facts.
    pub selected_program_id: Option<u32>,
    /// Detected streams in container order, including supplementary tracks.
    pub streams: Vec<MediaStreamDetailPayload>,
    /// Detected programs, including programs excluded from summary scoring.
    pub programs: Vec<MediaProgramPayload>,
    /// Authored chapter titles and timing.
    pub chapters: Vec<MediaChapterPayload>,
    /// Embedded attachment inventory without extracted payloads.
    pub attachments: Vec<MediaAttachmentPayload>,
    /// Caption services detected within the inspected headers.
    pub caption_services: Vec<MediaCaptionServicePayload>,
    /// Detected disc metadata, when this source is a disc image.
    pub disc: Option<MediaDiscMetadataPayload>,
    /// Inspection status, coverage counters, and warnings.
    pub report: MediaProbeReportPayload,
}
impl From<scryer_media_types::AnalysisDetails> for MediaAnalysisDetailsPayload {
    fn from(value: scryer_media_types::AnalysisDetails) -> Self {
        Self {
            revision: value.revision,
            duration_seconds: value.duration_seconds,
            duration_provenance: value.duration_provenance.into(),
            overall_bitrate_bps: value
                .overall_bitrate_bps
                .map(|value| Long::from(i64::try_from(value).unwrap_or(i64::MAX))),
            selected_video_id: value.selected_video_id,
            selected_program_id: value.selected_program_id,
            streams: value
                .streams
                .into_iter()
                .map(|value| value.into())
                .collect(),
            programs: value
                .programs
                .into_iter()
                .map(|value| value.into())
                .collect(),
            chapters: value
                .chapters
                .into_iter()
                .map(|value| value.into())
                .collect(),
            attachments: value
                .attachments
                .into_iter()
                .map(|value| value.into())
                .collect(),
            caption_services: value
                .caption_services
                .into_iter()
                .map(|value| value.into())
                .collect(),
            disc: value.disc.map(|value| value.into()),
            report: value.report.into(),
        }
    }
}
