//! Typed GraphQL projections of the shared media analysis contract.
use super::Long;
use async_graphql::{Enum, ID, InputObject, SimpleObject};

#[derive(InputObject)]
pub struct MediaDiscSelectionInput {
    pub title_id: Option<String>,
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
pub struct MediaDiscEpisodeMappingInput {
    pub disc_title_id: String,
    pub episode_id: ID,
}

#[derive(SimpleObject)]
pub struct MediaDiscEpisodeTargetPayload {
    pub episode_id: ID,
    pub label: String,
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
pub enum MediaProvenanceValue {
    Unknown,
    Container,
    Bitstream,
    Observed,
    Estimated,
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
pub enum MediaProbeStatusValue {
    Unknown,
    Complete,
    Incomplete,
    Unsupported,
    Encrypted,
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
pub enum MediaStreamKindValue {
    Video,
    Audio,
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
pub struct MediaAnalysisAttemptPayload {
    pub revision: i64,
    pub attempted_at: String,
    pub succeeded: bool,
    pub report: MediaProbeReportPayload,
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
pub struct MediaProbeReportPayload {
    pub status: MediaProbeStatusValue,
    pub bytes_read: Long,
    pub seeks: Long,
    pub elapsed_ms: Long,
    pub budget_exhausted: bool,
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
pub struct MediaProbeWarningPayload {
    pub code: String,
    pub message: String,
    pub stream_id: Option<String>,
    pub offset: Option<Long>,
}
impl From<scryer_media_types::ProbeWarning> for MediaProbeWarningPayload {
    fn from(value: scryer_media_types::ProbeWarning) -> Self {
        Self {
            code: value.code,
            message: value.message,
            stream_id: value.stream_id.map(|value| value),
            offset: value
                .offset
                .map(|value| Long::from(i64::try_from(value).unwrap_or(i64::MAX))),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaDiagnosticCoveragePayload {
    pub offset: Long,
    pub length: Long,
}

#[derive(SimpleObject, Clone)]
pub struct MediaStructuralDiagnosticsPayload {
    pub report: MediaProbeReportPayload,
    pub source_length: Long,
    pub coverage: Vec<MediaDiagnosticCoveragePayload>,
    pub coverage_truncated: bool,
    pub warnings_truncated: bool,
    pub packets_sampled: Long,
    pub frame_headers_sampled: Long,
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
pub struct MediaRationalPayload {
    pub numerator: Long,
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
pub struct MediaStreamDispositionPayload {
    pub default: Option<bool>,
    pub forced: Option<bool>,
    pub original: Option<bool>,
    pub commentary: Option<bool>,
    pub hearing_impaired: Option<bool>,
    pub visual_impaired: Option<bool>,
    pub attached_picture: Option<bool>,
    pub still_image: Option<bool>,
}
impl From<scryer_media_types::StreamDisposition> for MediaStreamDispositionPayload {
    fn from(value: scryer_media_types::StreamDisposition) -> Self {
        Self {
            default: value.default.map(|value| value),
            forced: value.forced.map(|value| value),
            original: value.original.map(|value| value),
            commentary: value.commentary.map(|value| value),
            hearing_impaired: value.hearing_impaired.map(|value| value),
            visual_impaired: value.visual_impaired.map(|value| value),
            attached_picture: value.attached_picture.map(|value| value),
            still_image: value.still_image.map(|value| value),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaColorMetadataPayload {
    pub primaries: Option<u32>,
    pub transfer: Option<u32>,
    pub matrix: Option<u32>,
    pub full_range: Option<bool>,
    pub provenance: MediaProvenanceValue,
    pub mastering_display: Option<MediaMasteringDisplayPayload>,
    pub content_light: Option<MediaContentLightPayload>,
}
impl From<scryer_media_types::ColorMetadata> for MediaColorMetadataPayload {
    fn from(value: scryer_media_types::ColorMetadata) -> Self {
        Self {
            primaries: value.primaries.map(|value| value),
            transfer: value.transfer.map(|value| value),
            matrix: value.matrix.map(|value| value),
            full_range: value.full_range.map(|value| value),
            provenance: value.provenance.into(),
            mastering_display: value.mastering_display.map(|value| value.into()),
            content_light: value.content_light.map(|value| value.into()),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaMasteringDisplayPayload {
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
impl From<scryer_media_types::MasteringDisplay> for MediaMasteringDisplayPayload {
    fn from(value: scryer_media_types::MasteringDisplay) -> Self {
        Self {
            red_x: value.red_x.map(|value| value),
            red_y: value.red_y.map(|value| value),
            green_x: value.green_x.map(|value| value),
            green_y: value.green_y.map(|value| value),
            blue_x: value.blue_x.map(|value| value),
            blue_y: value.blue_y.map(|value| value),
            white_x: value.white_x.map(|value| value),
            white_y: value.white_y.map(|value| value),
            min_luminance: value.min_luminance.map(|value| value),
            max_luminance: value.max_luminance.map(|value| value),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaContentLightPayload {
    pub max_cll: Option<u32>,
    pub max_fall: Option<u32>,
}
impl From<scryer_media_types::ContentLight> for MediaContentLightPayload {
    fn from(value: scryer_media_types::ContentLight) -> Self {
        Self {
            max_cll: value.max_cll.map(|value| value),
            max_fall: value.max_fall.map(|value| value),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaDolbyVisionPayload {
    pub profile: Option<u8>,
    pub level: Option<u8>,
    pub base_layer_compatibility_id: Option<u8>,
    pub rpu_present: Option<bool>,
    pub enhancement_layer_present: Option<bool>,
    pub base_layer_present: Option<bool>,
}
impl From<scryer_media_types::DolbyVision> for MediaDolbyVisionPayload {
    fn from(value: scryer_media_types::DolbyVision) -> Self {
        Self {
            profile: value.profile.map(|value| value),
            level: value.level.map(|value| value),
            base_layer_compatibility_id: value.base_layer_compatibility_id.map(|value| value),
            rpu_present: value.rpu_present.map(|value| value),
            enhancement_layer_present: value.enhancement_layer_present.map(|value| value),
            base_layer_present: value.base_layer_present.map(|value| value),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaHdrCapabilitiesPayload {
    pub dolby_vision: Option<bool>,
    pub hdr10plus: Option<bool>,
    pub hdr10: Option<bool>,
    pub hlg: Option<bool>,
    pub pq: Option<bool>,
    pub dovi: Option<MediaDolbyVisionPayload>,
}
impl From<scryer_media_types::HdrCapabilities> for MediaHdrCapabilitiesPayload {
    fn from(value: scryer_media_types::HdrCapabilities) -> Self {
        Self {
            dolby_vision: value.dolby_vision.map(|value| value),
            hdr10plus: value.hdr10plus.map(|value| value),
            hdr10: value.hdr10.map(|value| value),
            hlg: value.hlg.map(|value| value),
            pq: value.pq.map(|value| value),
            dovi: value.dovi.map(|value| value.into()),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaStreamMetadataPayload {
    pub id: Option<String>,
    pub program_id: Option<u32>,
    pub original_language: Option<String>,
    pub language_provenance: MediaProvenanceValue,
    pub disposition: MediaStreamDispositionPayload,
    pub duration_seconds: Option<f64>,
    pub bitrate_bps: Option<Long>,
    pub bitrate_provenance: MediaProvenanceValue,
    pub estimated_bitrate_bps: Option<Long>,
    pub sample_rate: Option<u32>,
    pub sample_format: Option<String>,
    pub sample_bit_depth: Option<u32>,
    pub channel_layout: Option<String>,
    pub pixel_format: Option<String>,
    pub profile: Option<String>,
    pub level: Option<u32>,
    pub bit_depth: Option<i32>,
    pub field_order: Option<String>,
    pub sample_aspect_ratio: Option<MediaRationalPayload>,
    pub display_aspect_ratio: Option<MediaRationalPayload>,
    pub rotation_degrees: Option<f64>,
    pub declared_frame_rate: Option<MediaRationalPayload>,
    pub observed_frame_rate: Option<MediaRationalPayload>,
    pub variable_frame_rate: Option<bool>,
    pub color: MediaColorMetadataPayload,
    pub hdr: MediaHdrCapabilitiesPayload,
}
impl From<scryer_media_types::StreamMetadata> for MediaStreamMetadataPayload {
    fn from(value: scryer_media_types::StreamMetadata) -> Self {
        Self {
            id: value.id.map(|value| value),
            program_id: value.program_id.map(|value| value),
            original_language: value.original_language.map(|value| value),
            language_provenance: value.language_provenance.into(),
            disposition: value.disposition.into(),
            duration_seconds: value.duration_seconds.map(|value| value),
            bitrate_bps: value
                .bitrate_bps
                .map(|value| Long::from(i64::try_from(value).unwrap_or(i64::MAX))),
            bitrate_provenance: value.bitrate_provenance.into(),
            estimated_bitrate_bps: value
                .estimated_bitrate_bps
                .map(|value| Long::from(i64::try_from(value).unwrap_or(i64::MAX))),
            sample_rate: value.sample_rate.map(|value| value),
            sample_format: value.sample_format.map(|value| value),
            sample_bit_depth: value.sample_bit_depth.map(|value| value),
            channel_layout: value.channel_layout.map(|value| value),
            pixel_format: value.pixel_format.map(|value| value),
            profile: value.profile.map(|value| value),
            level: value.level.map(|value| value),
            bit_depth: value.bit_depth.map(|value| value),
            field_order: value.field_order.map(|value| value),
            sample_aspect_ratio: value.sample_aspect_ratio.map(|value| value.into()),
            display_aspect_ratio: value.display_aspect_ratio.map(|value| value.into()),
            rotation_degrees: value.rotation_degrees.map(|value| value),
            declared_frame_rate: value.declared_frame_rate.map(|value| value.into()),
            observed_frame_rate: value.observed_frame_rate.map(|value| value.into()),
            variable_frame_rate: value.variable_frame_rate.map(|value| value),
            color: value.color.into(),
            hdr: value.hdr.into(),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaStreamDetailPayload {
    pub kind: MediaStreamKindValue,
    pub codec: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub channels: Option<i32>,
    pub language: Option<String>,
    pub name: Option<String>,
    pub metadata: MediaStreamMetadataPayload,
}
impl From<scryer_media_types::StreamDetail> for MediaStreamDetailPayload {
    fn from(value: scryer_media_types::StreamDetail) -> Self {
        Self {
            kind: value.kind.into(),
            codec: value.codec.map(|value| value),
            width: value.width.map(|value| value),
            height: value.height.map(|value| value),
            channels: value.channels.map(|value| value),
            language: value.language.map(|value| value),
            name: value.name.map(|value| value),
            metadata: value.metadata.into(),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaChapterPayload {
    pub id: String,
    pub title: Option<String>,
    pub start_seconds: f64,
    pub end_seconds: Option<f64>,
}
impl From<scryer_media_types::Chapter> for MediaChapterPayload {
    fn from(value: scryer_media_types::Chapter) -> Self {
        Self {
            id: value.id,
            title: value.title.map(|value| value),
            start_seconds: value.start_seconds,
            end_seconds: value.end_seconds.map(|value| value),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaAttachmentPayload {
    pub id: String,
    pub name: Option<String>,
    pub media_type: Option<String>,
    pub size_bytes: Long,
}
impl From<scryer_media_types::Attachment> for MediaAttachmentPayload {
    fn from(value: scryer_media_types::Attachment) -> Self {
        Self {
            id: value.id,
            name: value.name.map(|value| value),
            media_type: value.media_type.map(|value| value),
            size_bytes: Long::from(i64::try_from(value.size_bytes).unwrap_or(i64::MAX)),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaCaptionServicePayload {
    pub stream_id: String,
    pub standard: String,
    pub service_number: Option<u8>,
    pub language: Option<String>,
}
impl From<scryer_media_types::CaptionService> for MediaCaptionServicePayload {
    fn from(value: scryer_media_types::CaptionService) -> Self {
        Self {
            stream_id: value.stream_id,
            standard: value.standard,
            service_number: value.service_number.map(|value| value),
            language: value.language.map(|value| value),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaProgramPayload {
    pub id: u32,
    pub name: Option<String>,
    pub stream_ids: Vec<String>,
    pub duration_seconds: Option<f64>,
}
impl From<scryer_media_types::Program> for MediaProgramPayload {
    fn from(value: scryer_media_types::Program) -> Self {
        Self {
            id: value.id,
            name: value.name.map(|value| value),
            stream_ids: value.stream_ids.into_iter().map(|value| value).collect(),
            duration_seconds: value.duration_seconds.map(|value| value),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaDiscSegmentPayload {
    pub path: String,
    pub in_seconds: f64,
    pub out_seconds: f64,
    pub angle: u32,
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
pub struct MediaDiscTitlePayload {
    pub id: String,
    pub aliases: Vec<String>,
    pub duration_seconds: Option<f64>,
    pub angle_count: u32,
    pub segments: Vec<MediaDiscSegmentPayload>,
    pub chapters: Vec<MediaChapterPayload>,
    pub streams: Vec<MediaStreamDetailPayload>,
    pub report: MediaProbeReportPayload,
}
impl From<scryer_media_types::DiscTitle> for MediaDiscTitlePayload {
    fn from(value: scryer_media_types::DiscTitle) -> Self {
        Self {
            id: value.id,
            aliases: value.aliases.into_iter().map(|value| value).collect(),
            duration_seconds: value.duration_seconds.map(|value| value),
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
pub struct MediaDiscEpisodeMappingPayload {
    pub disc_title_id: String,
    pub episode_ids: Vec<String>,
}
impl From<scryer_media_types::DiscEpisodeMapping> for MediaDiscEpisodeMappingPayload {
    fn from(value: scryer_media_types::DiscEpisodeMapping) -> Self {
        Self {
            disc_title_id: value.disc_title_id,
            episode_ids: value.episode_ids.into_iter().map(|value| value).collect(),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaDiscSelectionPayload {
    pub title_id: Option<String>,
    pub episode_mappings: Vec<MediaDiscEpisodeMappingPayload>,
}
impl From<scryer_media_types::DiscSelection> for MediaDiscSelectionPayload {
    fn from(value: scryer_media_types::DiscSelection) -> Self {
        Self {
            title_id: value.title_id.map(|value| value),
            episode_mappings: value
                .episode_mappings
                .into_iter()
                .map(|value| value.into())
                .collect(),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaDiscMetadataPayload {
    pub disc_type: String,
    pub filesystem: String,
    pub volume_label: Option<String>,
    pub titles: Vec<MediaDiscTitlePayload>,
    pub selected_title_id: Option<String>,
    pub automatic_selection: bool,
    pub selection: MediaDiscSelectionPayload,
}
impl From<scryer_media_types::DiscMetadata> for MediaDiscMetadataPayload {
    fn from(value: scryer_media_types::DiscMetadata) -> Self {
        Self {
            disc_type: value.disc_type,
            filesystem: value.filesystem,
            volume_label: value.volume_label.map(|value| value),
            titles: value.titles.into_iter().map(|value| value.into()).collect(),
            selected_title_id: value.selected_title_id.map(|value| value),
            automatic_selection: value.automatic_selection,
            selection: value.selection.into(),
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct MediaAnalysisDetailsPayload {
    pub revision: u32,
    pub duration_seconds: Option<f64>,
    pub duration_provenance: MediaProvenanceValue,
    pub overall_bitrate_bps: Option<Long>,
    pub selected_video_id: Option<String>,
    pub selected_program_id: Option<u32>,
    pub streams: Vec<MediaStreamDetailPayload>,
    pub programs: Vec<MediaProgramPayload>,
    pub chapters: Vec<MediaChapterPayload>,
    pub attachments: Vec<MediaAttachmentPayload>,
    pub caption_services: Vec<MediaCaptionServicePayload>,
    pub disc: Option<MediaDiscMetadataPayload>,
    pub report: MediaProbeReportPayload,
}
impl From<scryer_media_types::AnalysisDetails> for MediaAnalysisDetailsPayload {
    fn from(value: scryer_media_types::AnalysisDetails) -> Self {
        Self {
            revision: value.revision,
            duration_seconds: value.duration_seconds.map(|value| value),
            duration_provenance: value.duration_provenance.into(),
            overall_bitrate_bps: value
                .overall_bitrate_bps
                .map(|value| Long::from(i64::try_from(value).unwrap_or(i64::MAX))),
            selected_video_id: value.selected_video_id.map(|value| value),
            selected_program_id: value.selected_program_id.map(|value| value),
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
