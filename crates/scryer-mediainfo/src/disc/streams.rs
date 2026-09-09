use crate::{
    MediaAnalysis,
    types::{RawContainer, RawTrack, TrackKind},
};
use scryer_media_types::{StreamDetail, StreamKind};

pub(super) fn apply_navigation(analysis: &mut MediaAnalysis, declared: &[StreamDetail]) {
    if declared.is_empty() {
        return;
    }
    for declaration in declared {
        let actual = analysis.details.streams.iter_mut().find(|stream| {
            stream.kind == declaration.kind && stream.metadata.id == declaration.metadata.id
        });
        if let Some(actual) = actual {
            if declaration.language.is_some() {
                actual.language = declaration.language.clone();
                actual.metadata.original_language = declaration.metadata.original_language.clone();
                actual.metadata.language_provenance = declaration.metadata.language_provenance;
            }
            macro_rules! disposition {
                ($($field:ident),*) => { $(actual.metadata.disposition.$field = declaration.metadata.disposition.$field.or(actual.metadata.disposition.$field);)* };
            }
            disposition!(
                default,
                forced,
                original,
                commentary,
                hearing_impaired,
                visual_impaired,
                attached_picture,
                still_image
            );
            actual.channels = actual.channels.or(declaration.channels);
            actual.metadata.profile = actual
                .metadata
                .profile
                .clone()
                .or_else(|| declaration.metadata.profile.clone());
            if declaration.channels.is_none() || actual.channels == declaration.channels {
                actual.metadata.channel_layout = actual
                    .metadata
                    .channel_layout
                    .clone()
                    .or_else(|| declaration.metadata.channel_layout.clone());
            }
            actual.metadata.sample_rate = actual
                .metadata
                .sample_rate
                .or(declaration.metadata.sample_rate);
            actual.metadata.sample_bit_depth = actual
                .metadata
                .sample_bit_depth
                .or(declaration.metadata.sample_bit_depth);
            actual.metadata.display_aspect_ratio = actual
                .metadata
                .display_aspect_ratio
                .or(declaration.metadata.display_aspect_ratio);
            actual.metadata.declared_frame_rate = actual
                .metadata
                .declared_frame_rate
                .or(declaration.metadata.declared_frame_rate);
            if actual.metadata.color.primaries.is_none()
                && declaration.metadata.color.primaries.is_some()
            {
                actual.metadata.color.primaries = declaration.metadata.color.primaries;
                if actual.metadata.color.provenance == scryer_media_types::Provenance::Unknown {
                    actual.metadata.color.provenance = scryer_media_types::Provenance::Container;
                }
            }
            if declaration.metadata.hdr.dolby_vision == Some(true) {
                actual.metadata.hdr.dolby_vision = Some(true);
            }
            let contradicts_pq = actual.metadata.bit_depth.is_some_and(|depth| depth < 10)
                || actual
                    .metadata
                    .color
                    .transfer
                    .is_some_and(|transfer| transfer != 16);
            if (declaration.metadata.hdr.hdr10 == Some(true)
                || declaration.metadata.hdr.hdr10plus == Some(true))
                && contradicts_pq
            {
                analysis.details.report.warnings.push(scryer_media_types::ProbeWarning {
                    code: "disc_hdr_signaling_conflict".into(), message: "Disc HDR signaling conflicts with the elementary video; bitstream signaling takes precedence".into(),
                    stream_id: actual.metadata.id.clone(), ..Default::default()
                });
            } else {
                if declaration.metadata.hdr.hdr10 == Some(true) {
                    actual.metadata.hdr.hdr10 = Some(true);
                    actual.metadata.hdr.pq = Some(true);
                }
                if declaration.metadata.hdr.hdr10plus == Some(true) {
                    actual.metadata.hdr.hdr10plus = Some(true);
                }
            }
        } else {
            analysis.details.streams.push(declaration.clone());
        }
    }
    // Reuse ordinary-file audio selection and summary projection after adding
    // IFO languages and roles. A commentary track must not remain the winner
    // merely because its role was learned after payload probing.
    let tracks = analysis
        .details
        .streams
        .iter()
        .filter(|stream| {
            stream.kind != StreamKind::Video
                && (analysis.details.selected_program_id.is_none()
                    || stream.metadata.program_id == analysis.details.selected_program_id)
        })
        .map(|stream| RawTrack {
            kind: if stream.kind == StreamKind::Audio {
                TrackKind::Audio
            } else {
                TrackKind::Subtitle
            },
            codec_id: stream.codec.clone().unwrap_or_default(),
            codec_name: stream.codec.clone(),
            audio_profile: stream.metadata.profile.clone(),
            channels: stream.channels,
            bit_rate_bps: stream
                .metadata
                .bitrate_bps
                .and_then(|rate| i64::try_from(rate).ok()),
            language: stream.language.clone(),
            name: stream.name.clone(),
            forced: stream.metadata.disposition.forced == Some(true),
            default_track: stream.metadata.disposition.default == Some(true),
            metadata: stream.metadata.clone(),
            ..Default::default()
        })
        .collect();
    let summary = crate::build_analysis(RawContainer {
        format_name: "disc".into(),
        duration_seconds: None,
        num_chapters: None,
        tracks,
        details: Default::default(),
    });
    analysis.audio_codec = summary.audio_codec;
    analysis.audio_profile = summary.audio_profile;
    analysis.audio_channels = summary.audio_channels;
    analysis.audio_bitrate_kbps = summary.audio_bitrate_kbps;
    analysis.audio_languages = summary.audio_languages;
    analysis.audio_streams = summary.audio_streams;
    analysis.has_multiaudio = summary.has_multiaudio;
    analysis.subtitle_streams = summary.subtitle_streams;
    analysis.subtitle_languages = summary.subtitle_languages;
    analysis.subtitle_codecs = summary.subtitle_codecs;
    if let Some(video) = analysis.details.streams.iter().find(|stream| {
        stream.kind == StreamKind::Video && stream.metadata.id == analysis.details.selected_video_id
    }) {
        let hdr = &video.metadata.hdr;
        analysis.video_hdr_format = if hdr.dolby_vision == Some(true) {
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
        .map(str::to_string);
        if analysis.video_frame_rate.is_none() {
            analysis.video_frame_rate = video
                .metadata
                .declared_frame_rate
                .and_then(|rate| rate.as_f64())
                .map(|rate| format!("{rate:.3}"));
        }
    }
}
