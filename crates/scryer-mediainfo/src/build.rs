//! Turns a parsed [`RawContainer`] into the published [`AnalysisDetails`].

use crate::AnalysisDetails;
use crate::codec::{self, audio_config, av1, parameter_sets, picture_headers};
use crate::raw::{RawContainer, RawTrack, TrackKind};

pub(crate) fn build_details(mut raw: RawContainer) -> AnalysisDetails {
    populate_analysis_details(&mut raw);
    let mut details = raw.details;
    details.container_format = Some(raw.format_name);
    details.chapter_count = raw.num_chapters.and_then(|count| u32::try_from(count).ok());
    details
}

fn populate_analysis_details(raw: &mut RawContainer) {
    use scryer_media_model::{ANALYSIS_REVISION, Provenance, StreamDetail, StreamKind};
    raw.details.revision = ANALYSIS_REVISION;
    raw.details.duration_seconds = raw.duration_seconds;
    if raw.duration_seconds.is_some() && raw.details.duration_provenance == Provenance::Unknown {
        raw.details.duration_provenance = Provenance::Container;
    }
    for (index, track) in raw.tracks.iter_mut().enumerate() {
        track.metadata.id.get_or_insert_with(|| index.to_string());
        if track.kind == TrackKind::Audio
            && let Some((code, exhausted)) = audio_config::enrich_private_header(track)
        {
            if matches!(
                raw.details.report.status,
                scryer_media_model::ProbeStatus::Unknown
                    | scryer_media_model::ProbeStatus::Complete
            ) {
                raw.details.report.status = scryer_media_model::ProbeStatus::Incomplete;
            }
            raw.details.report.budget_exhausted |= exhausted;
            raw.details
                .report
                .warnings
                .push(scryer_media_model::ProbeWarning {
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
                picture_headers::enrich(track, &private);
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
                        .push(scryer_media_model::ProbeWarning {
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
                parameter_sets::enrich(track, &private, &mut raw.details.report);
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
                .and_then(codec::dolby_vision::parse_dovi_config)
            {
                track.metadata.hdr.dolby_vision = Some(true);
                track.metadata.hdr.dovi = Some(scryer_media_model::DolbyVision {
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
                .and_then(codec::audio_config::pcm_sample_representation)
            {
                track.metadata.sample_format = Some(format.into());
                track.metadata.sample_bit_depth.get_or_insert(depth);
            }
            if track.codec_name.as_deref() == Some("aac")
                && let Some(config) = track
                    .codec_private
                    .as_deref()
                    .and_then(codec::aac::aac_configuration_metadata)
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
            metadata: stream_metadata(track),
        })
        .collect();
}

/// The track's metadata plus the facts only its codec configuration holds.
fn stream_metadata(track: &RawTrack) -> scryer_media_model::StreamMetadata {
    let mut metadata = track.metadata.clone();
    if track.kind == TrackKind::Video {
        let info = extract_codec_info(track);
        metadata.configured_width = info.width;
        metadata.configured_height = info.height;
        metadata.nominal_frame_rate_fps = track.frame_rate_fps.or(info.frame_rate_fps);
    }
    metadata
}

pub(crate) fn select_primary_video_track<'a>(
    video_tracks: &[&'a RawTrack],
) -> Option<&'a RawTrack> {
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
            .map(codec::h264::extract_h264_info)
            .unwrap_or_default(),
        "hevc" => track
            .codec_private
            .as_deref()
            .map(codec::hevc::extract_h265_info)
            .unwrap_or_default(),
        "av1" => track
            .codec_private
            .as_deref()
            .map(codec::av1::extract_av1_info)
            .unwrap_or_default(),
        _ => codec::CodecInfo::default(),
    }
}

/// Parser facts the unit tests assert, read from the details.
#[cfg(test)]
pub(crate) mod test_facts {
    use scryer_media_model::AnalysisDetails;

    /// The selected video stream's codec.
    pub(crate) fn video_codec(details: &AnalysisDetails) -> Option<&str> {
        details.selected_video()?.codec.as_deref()
    }

    /// A selected video stream with a codec and a duration that rounds to at
    /// least one second.
    pub(crate) fn has_video_with_duration(details: &AnalysisDetails) -> bool {
        video_codec(details).is_some()
            && details
                .duration_seconds
                .is_some_and(|seconds| seconds.round() > 0.0)
    }

    /// The selected video's size, from the container or else the codec
    /// configuration.
    pub(crate) fn video_size(details: &AnalysisDetails) -> (Option<i32>, Option<i32>) {
        details.selected_video().map_or((None, None), |video| {
            (
                video.width.or(video.metadata.configured_width),
                video.height.or(video.metadata.configured_height),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commentary_role_and_default_flag_reach_the_audio_streams() {
        let mut mono = test_track(TrackKind::Audio, "aac");
        mono.channels = Some(1);
        let mut commentary = test_track(TrackKind::Audio, "flac");
        commentary.channels = Some(2);
        commentary.metadata.disposition.default = Some(true);
        commentary.name = Some("Director's Commentary".into());
        let details = build_details(RawContainer {
            format_name: "matroska".into(),
            duration_seconds: Some(60.0),
            num_chapters: None,
            details: Default::default(),
            tracks: vec![commentary, mono],
        });
        let commentary = &details.streams[0];
        assert_eq!(commentary.codec.as_deref(), Some("flac"));
        assert_eq!(commentary.metadata.disposition.default, Some(true));
        assert_eq!(commentary.name.as_deref(), Some("Director's Commentary"));
        let mono = &details.streams[1];
        assert_eq!(mono.codec.as_deref(), Some("aac"));
        assert_eq!(mono.channels, Some(1));
        assert_eq!(mono.metadata.disposition.default, None);
        assert_eq!(mono.metadata.disposition.commentary, None);
    }

    #[test]
    fn analysis_keeps_default_audio_facts_on_that_track() {
        let details = build_details(RawContainer {
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
                    frame_rate_fps: None,
                    color_transfer: None,
                    dovi_config: None,
                    has_hdr10plus: false,
                },
                RawTrack {
                    metadata: scryer_media_model::StreamMetadata {
                        disposition: scryer_media_model::StreamDisposition {
                            default: Some(true),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
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
                    frame_rate_fps: None,
                    color_transfer: None,
                    dovi_config: None,
                    has_hdr10plus: false,
                },
            ],
        });

        let default = &details.streams[2];
        assert_eq!(default.codec.as_deref(), Some("flac"));
        assert_eq!(default.metadata.profile, None);
        assert_eq!(default.channels, Some(6));
        assert_eq!(default.metadata.bitrate_bps, Some(640_000));
        assert_eq!(default.metadata.disposition.default, Some(true));
        let other = &details.streams[1];
        assert_eq!(other.metadata.profile.as_deref(), Some("LC"));
        assert_eq!(other.metadata.disposition.default, None);
        assert_eq!(details.container_format.as_deref(), Some("matroska"));
        assert_eq!(details.chapter_count, Some(0));
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
        let analysis = build_details(RawContainer {
            tracks: vec![video, commentary, normal, described, other],
            format_name: "mpegts".into(),
            duration_seconds: Some(10.0),
            num_chapters: None,
            details: Default::default(),
        });
        assert_eq!(analysis.selected_program_id, Some(1));
        let program_audio: Vec<_> = analysis
            .program_streams(scryer_media_model::StreamKind::Audio)
            .map(|stream| (stream.codec.as_deref(), stream.language.as_deref()))
            .collect();
        assert_eq!(
            program_audio,
            [
                (Some("truehd"), Some("fra")),
                (Some("aac"), Some("eng")),
                (Some("aac"), Some("eng"))
            ]
        );
        assert_eq!(
            analysis.streams[1].metadata.disposition.commentary,
            Some(true)
        );
        assert_eq!(analysis.streams.len(), 5);
        let hdr = &analysis.streams[0].metadata.hdr;
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

        let analysis = build_details(RawContainer {
            details: Default::default(),
            format_name: "matroska".into(),
            duration_seconds: Some(60.0),
            num_chapters: None,
            tracks: vec![cover, main],
        });

        assert_eq!(test_facts::video_codec(&analysis), Some("h264"));
        assert_eq!(test_facts::video_size(&analysis), (Some(1920), Some(1080)));
        assert_eq!(
            analysis
                .selected_video()
                .unwrap()
                .metadata
                .nominal_frame_rate_fps,
            Some(24000.0 / 1001.0)
        );
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
            frame_rate_fps: None,
            color_transfer: None,
            dovi_config: None,
            has_hdr10plus: false,
        }
    }

    /// The HDR signals the parser records for one video track, in the order
    /// Dolby Vision, HDR10+, HDR10, HLG; each is true only when positively signaled.
    fn detect_hdr_signals(track: &RawTrack) -> [bool; 4] {
        let details = crate::build::build_details(crate::raw::RawContainer {
            format_name: "fixture".into(),
            duration_seconds: None,
            num_chapters: None,
            tracks: vec![track.clone()],
            details: Default::default(),
        });
        let hdr = &details.selected_video().expect("video track").metadata.hdr;
        [hdr.dolby_vision, hdr.hdr10plus, hdr.hdr10, hdr.hlg].map(|flag| flag == Some(true))
    }

    #[test]
    fn detect_hdr_dolby_vision() {
        let track = RawTrack {
            metadata: Default::default(),
            kind: TrackKind::Video,
            codec_id: "V_MPEGH/ISO/HEVC".into(),
            codec_name: Some("hevc".into()),
            audio_profile: None,
            codec_private: None,
            width: Some(3840),
            height: Some(2160),
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: Some(16),
            dovi_config: Some(vec![1, 0, 0x10, 0x0d, 0x10]),
            has_hdr10plus: false,
            name: None,
        };
        // PQ without a bit depth does not establish HDR10 beside Dolby Vision.
        assert_eq!(detect_hdr_signals(&track), [true, false, false, false]);
    }

    #[test]
    fn detect_hdr_hdr10() {
        let track = RawTrack {
            metadata: Default::default(),
            kind: TrackKind::Video,
            codec_id: "hvc1".into(),
            codec_name: Some("hevc".into()),
            audio_profile: None,
            codec_private: None,
            width: Some(3840),
            height: Some(2160),
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: Some(16),
            dovi_config: None,
            has_hdr10plus: false,
            name: None,
        };
        assert_eq!(
            detect_hdr_signals(&track),
            [false; 4],
            "PQ alone does not establish HDR10"
        );
        let mut track = track;
        track.metadata.bit_depth = Some(10);
        track.metadata.color.primaries = Some(9);
        assert_eq!(detect_hdr_signals(&track), [false, false, true, false]);
    }

    #[test]
    fn detect_hdr_hdr10plus() {
        let track = RawTrack {
            metadata: Default::default(),
            kind: TrackKind::Video,
            codec_id: "hvc1".into(),
            codec_name: Some("hevc".into()),
            audio_profile: None,
            codec_private: None,
            width: Some(3840),
            height: Some(2160),
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: Some(16),
            dovi_config: None,
            has_hdr10plus: true,
            name: None,
        };
        assert_eq!(detect_hdr_signals(&track), [false, true, false, false]);
    }

    #[test]
    fn detect_hdr_hlg() {
        let track = RawTrack {
            metadata: Default::default(),
            kind: TrackKind::Video,
            codec_id: "hvc1".into(),
            codec_name: Some("hevc".into()),
            audio_profile: None,
            codec_private: None,
            width: Some(3840),
            height: Some(2160),
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: Some(18),
            dovi_config: None,
            has_hdr10plus: false,
            name: None,
        };
        assert_eq!(detect_hdr_signals(&track), [false, false, false, true]);
    }

    #[test]
    fn detect_hdr_sdr() {
        let track = RawTrack {
            metadata: Default::default(),
            kind: TrackKind::Video,
            codec_id: "avc1".into(),
            codec_name: Some("h264".into()),
            audio_profile: None,
            codec_private: None,
            width: Some(1920),
            height: Some(1080),
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: Some(1),
            dovi_config: None,
            has_hdr10plus: false,
            name: None,
        };
        assert_eq!(detect_hdr_signals(&track), [false; 4]);
    }
}
