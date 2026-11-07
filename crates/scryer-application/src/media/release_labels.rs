#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ResolvedAnalysisReleaseLabels {
    pub quality: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<String>,
    pub is_atmos: bool,
}

#[expect(
    clippy::too_many_arguments,
    reason = "label projection combines legacy summary fields with complete stream metadata"
)]
pub(crate) fn resolve_release_labels_from_analysis(
    video_width: Option<i32>,
    video_height: Option<i32>,
    video_codec: Option<&crate::release_parser::VideoCodec>,
    primary_audio_codec: Option<&str>,
    primary_audio_profile: Option<&str>,
    primary_audio_channels: Option<i32>,
    audio_streams: &[crate::AudioStreamDetail],
    details: &scryer_media_types::AnalysisDetails,
) -> ResolvedAnalysisReleaseLabels {
    let quality = quality_from_video_dimensions(video_width, video_height).map(str::to_string);
    let video_codec = video_codec
        .and_then(normalize_video_codec_for_release)
        .map(str::to_string);

    let metadata = selected_audio_metadata(details);
    let best = scryer_media_types::primary_audio_stream_index(
        audio_streams.iter().enumerate().map(|(index, stream)| {
            let disposition = metadata.get(index).map(|item| &item.metadata.disposition);
            (
                disposition.is_some_and(|d| d.default == Some(true)),
                disposition.is_some_and(|d| d.commentary == Some(true)),
                stream
                    .name
                    .as_deref()
                    .or_else(|| metadata.get(index).and_then(|item| item.name.as_deref())),
            )
        }),
    )
    .map(|index| {
        let stream = &audio_streams[index];
        (
            index,
            stream,
            normalize_audio_codec_for_release(stream.codec.as_deref(), stream.profile.as_deref()),
        )
    });
    let (best_audio_label, audio_channels) = if let Some((index, stream, label)) = best {
        // Release names and rename tokens spell channels as a count ("2.0",
        // "5.1"). Probe layouts spell the same thing as words ("stereo") or
        // with speaker-arrangement suffixes ("5.1(side)"), so the layout is
        // only a fallback for a stream whose count is unknown. Object-based
        // audio is carried by the codec label ("TrueHD Atmos"), not here.
        let channels = stream
            .channels
            .filter(|count| *count > 0)
            .map(format_audio_channels_for_release)
            .or_else(|| {
                metadata
                    .get(index)
                    .and_then(|stream| stream.metadata.channel_layout.as_deref())
                    .and_then(audio_channels_from_layout)
            });
        (label, channels)
    } else if audio_streams.is_empty() && details.revision == 0 {
        (
            normalize_audio_codec_for_release(primary_audio_codec, primary_audio_profile),
            primary_audio_channels.map(format_audio_channels_for_release),
        )
    } else {
        (None, None)
    };

    let is_atmos = best_audio_label
        .as_deref()
        .is_some_and(|label| label.contains("Atmos") || label == "DTS:X");

    ResolvedAnalysisReleaseLabels {
        quality,
        video_codec,
        audio_codec: best_audio_label,
        audio_channels,
        is_atmos,
    }
}

fn selected_audio_metadata(
    details: &scryer_media_types::AnalysisDetails,
) -> Vec<&scryer_media_types::StreamDetail> {
    details
        .streams
        .iter()
        .filter(|stream| {
            stream.kind == scryer_media_types::StreamKind::Audio
                && (details.selected_program_id.is_none()
                    || stream.metadata.program_id == details.selected_program_id)
        })
        .collect()
}

pub(crate) fn eligible_audio_streams(
    analysis: &crate::MediaFileAnalysis,
) -> Vec<crate::AudioStreamDetail> {
    let metadata = selected_audio_metadata(&analysis.details);
    analysis
        .audio_streams
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            metadata
                .get(*index)
                .is_none_or(|stream| stream.metadata.disposition.commentary != Some(true))
        })
        .map(|(_, stream)| stream.clone())
        .collect()
}

pub(crate) fn quality_from_video_dimensions(
    width: Option<i32>,
    height: Option<i32>,
) -> Option<&'static str> {
    let width = width.unwrap_or_default();
    let height = height.unwrap_or_default();
    match (width, height) {
        (w, h) if w >= 7680 || h >= 4200 => Some("4320p"),
        (w, h) if w >= 3200 || h >= 2100 => Some("2160p"),
        (w, h) if w >= 2400 || h >= 1300 => Some("1440p"),
        (w, h) if w >= 1800 || h >= 1000 => Some("1080p"),
        (w, h) if w >= 1200 || h >= 700 => Some("720p"),
        (w, h) if w >= 1000 || h >= 560 => Some("576p"),
        (w, h) if w > 0 && h > 0 => Some("480p"),
        _ => None,
    }
}

pub(crate) fn normalize_video_codec_for_release(
    codec: &crate::release_parser::VideoCodec,
) -> Option<&'static str> {
    match codec {
        crate::release_parser::VideoCodec::H265 => Some("H.265"),
        crate::release_parser::VideoCodec::H264 => Some("H.264"),
        crate::release_parser::VideoCodec::Av1 => Some("AV1"),
        crate::release_parser::VideoCodec::Vp9 => Some("VP9"),
        crate::release_parser::VideoCodec::Mpeg4
        | crate::release_parser::VideoCodec::Xvid
        | crate::release_parser::VideoCodec::Divx => Some("MPEG-4"),
        _ => None,
    }
}

pub(crate) use scryer_media_types::normalize_audio_codec_for_release;

pub(crate) fn format_audio_channels_for_release(channels: i32) -> String {
    match channels {
        8 => "7.1".to_string(),
        7 => "6.1".to_string(),
        6 => "5.1".to_string(),
        2 => "2.0".to_string(),
        1 => "1.0".to_string(),
        n => format!("{n}.0"),
    }
}

/// The count form of a probe channel layout, for a stream whose channel count
/// is unknown. Layouts that name no count yield nothing rather than a word.
fn audio_channels_from_layout(layout: &str) -> Option<String> {
    let layout = layout.trim();
    if layout.eq_ignore_ascii_case("mono") {
        return Some(format_audio_channels_for_release(1));
    }
    if layout.eq_ignore_ascii_case("stereo") {
        return Some(format_audio_channels_for_release(2));
    }
    let mut parts = layout.splitn(3, |ch: char| !ch.is_ascii_digit());
    let main = parts.next().filter(|part| !part.is_empty())?;
    let separator = layout[main.len()..].chars().next()?;
    let lfe = parts.next().filter(|part| !part.is_empty())?;
    (separator == '.').then(|| format!("{main}.{lfe}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_audio_labels_keep_mono_instead_of_stereo_commentary() {
        let (mut streams, mut details) = probed_audio(Some(1), Some("mono"));
        let mut commentary = streams[0].clone();
        commentary.channels = Some(2);
        commentary.codec = Some("flac".into());
        commentary.name = Some("Director Commentary".into());
        streams.push(commentary);
        let mut metadata = details.streams[0].clone();
        metadata.channels = Some(2);
        metadata.metadata.disposition.default = Some(true);
        details.streams.push(metadata);
        let labels = resolve_release_labels_from_analysis(
            None,
            None,
            None,
            Some("flac"),
            None,
            Some(2),
            &streams,
            &details,
        );
        assert_eq!(labels.audio_channels.as_deref(), Some("1.0"));
        assert_eq!(labels.audio_codec.as_deref(), Some("AAC"));

        streams[1].name = None;
        details.streams[1].metadata.disposition.commentary = Some(true);
        let labels = resolve_release_labels_from_analysis(
            None, None, None, None, None, None, &streams, &details,
        );
        assert_eq!(labels.audio_channels.as_deref(), Some("1.0"));
    }

    #[test]
    fn primary_audio_labels_follow_default_then_order_without_borrowing_channels() {
        let (mut streams, mut details) = probed_audio(Some(8), None);
        let mut mono = streams[0].clone();
        mono.channels = Some(1);
        streams.push(mono);
        let mut metadata = details.streams[0].clone();
        metadata.channels = Some(1);
        metadata.metadata.disposition.default = Some(true);
        details.streams.push(metadata);
        let labels = resolve_release_labels_from_analysis(
            None, None, None, None, None, None, &streams, &details,
        );
        assert_eq!(labels.audio_channels.as_deref(), Some("1.0"));
        streams[1].channels = None;
        details.streams[1].metadata.channel_layout = None;
        let labels = resolve_release_labels_from_analysis(
            None,
            None,
            None,
            None,
            None,
            Some(8),
            &streams,
            &details,
        );
        assert_eq!(labels.audio_channels, None);
        streams.swap(0, 1);
        let labels = resolve_release_labels_from_analysis(
            None,
            None,
            None,
            None,
            None,
            None,
            &streams,
            &Default::default(),
        );
        assert_eq!(
            labels.audio_channels, None,
            "legacy stream order must not select maximum channels"
        );
    }

    #[test]
    fn maps_profile_driven_audio_labels() {
        assert_eq!(
            normalize_audio_codec_for_release(Some("dts"), Some("DTS-HD MA + DTS:X IMAX"))
                .as_deref(),
            Some("DTS:X")
        );
        assert_eq!(
            normalize_audio_codec_for_release(Some("truehd"), Some("Dolby TrueHD + Dolby Atmos"))
                .as_deref(),
            Some("TrueHD Atmos")
        );
        assert_eq!(
            normalize_audio_codec_for_release(
                Some("eac3"),
                Some("Dolby Digital Plus + Dolby Atmos")
            )
            .as_deref(),
            Some("EAC3 Atmos")
        );
    }

    #[test]
    fn formats_audio_channels_like_release_names() {
        assert_eq!(format_audio_channels_for_release(8), "7.1");
        assert_eq!(format_audio_channels_for_release(6), "5.1");
        assert_eq!(format_audio_channels_for_release(2), "2.0");
    }

    fn probed_audio(
        channels: Option<i32>,
        layout: Option<&str>,
    ) -> (
        Vec<crate::AudioStreamDetail>,
        scryer_media_types::AnalysisDetails,
    ) {
        let streams = vec![crate::AudioStreamDetail {
            codec: Some("aac".to_string()),
            profile: None,
            channels,
            language: Some("en".to_string()),
            name: None,
            bitrate_kbps: None,
        }];
        let details = scryer_media_types::AnalysisDetails {
            revision: scryer_media_types::ANALYSIS_REVISION,
            streams: vec![scryer_media_types::StreamDetail {
                kind: scryer_media_types::StreamKind::Audio,
                codec: Some("aac".to_string()),
                channels,
                metadata: scryer_media_types::StreamMetadata {
                    channel_layout: layout.map(str::to_string),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        (streams, details)
    }

    fn resolved_channels(channels: Option<i32>, layout: Option<&str>) -> Option<String> {
        let (streams, details) = probed_audio(channels, layout);
        resolve_release_labels_from_analysis(None, None, None, None, None, None, &streams, &details)
            .audio_channels
    }

    #[test]
    fn audio_channels_use_the_count_even_when_the_probe_names_the_layout() {
        assert_eq!(
            resolved_channels(Some(2), Some("stereo")).as_deref(),
            Some("2.0")
        );
        assert_eq!(
            resolved_channels(Some(1), Some("mono")).as_deref(),
            Some("1.0")
        );
        assert_eq!(
            resolved_channels(Some(6), Some("5.1(side)")).as_deref(),
            Some("5.1")
        );
        assert_eq!(
            resolved_channels(Some(8), Some("7.1(wide)")).as_deref(),
            Some("7.1")
        );
    }

    #[test]
    fn audio_channels_fall_back_to_the_count_form_of_the_layout() {
        assert_eq!(
            resolved_channels(None, Some("stereo")).as_deref(),
            Some("2.0")
        );
        assert_eq!(
            resolved_channels(None, Some("5.1(side)")).as_deref(),
            Some("5.1")
        );
        assert_eq!(resolved_channels(None, Some("quad")), None);
        assert_eq!(resolved_channels(None, None), None);
    }

    #[test]
    fn resolution_boundaries_use_either_dimension() {
        for (width, height, tier, below) in [
            (7680, 4200, "4320p", "2160p"),
            (3200, 2100, "2160p", "1440p"),
            (2400, 1300, "1440p", "1080p"),
            (1800, 1000, "1080p", "720p"),
            (1200, 700, "720p", "576p"),
            (1000, 560, "576p", "480p"),
        ] {
            for (w, h, expected) in [
                (width - 1, 1, below),
                (width, 1, tier),
                (1, height - 1, below),
                (1, height, tier),
            ] {
                assert_eq!(
                    quality_from_video_dimensions(Some(w), Some(h)),
                    Some(expected),
                    "{w}x{h}"
                );
            }
        }
    }

    #[test]
    fn resolution_crops_and_overlapping_tiers_choose_the_highest_match() {
        for (width, height, expected) in [
            (1916, 800, "1080p"),
            (1276, 536, "720p"),
            (3836, 1600, "2160p"),
            (2556, 1068, "1440p"),
            (2560, 1080, "1440p"),
            (1920, 1080, "1080p"),
            (2560, 1440, "1440p"),
            (3840, 2160, "2160p"),
            (7680, 4320, "4320p"),
            (1800, 2100, "2160p"),
            (7680, 560, "4320p"),
            (720, 576, "576p"),
            (854, 480, "480p"),
            (640, 360, "480p"),
        ] {
            assert_eq!(
                quality_from_video_dimensions(Some(width), Some(height)),
                Some(expected),
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn resolution_partial_dimensions_require_a_positive_threshold_or_both_for_sd() {
        for (width, height, expected) in [
            (None, None, None),
            (Some(0), Some(0), None),
            (Some(-1), Some(-1), None),
            (Some(999), None, None),
            (None, Some(559), None),
            (Some(1), Some(0), None),
            (Some(-1), Some(1), None),
            (Some(1), Some(1), Some("480p")),
            (Some(1000), None, Some("576p")),
            (None, Some(560), Some("576p")),
            (Some(1800), Some(-1), Some("1080p")),
            (Some(0), Some(1300), Some("1440p")),
        ] {
            assert_eq!(
                quality_from_video_dimensions(width, height),
                expected,
                "{width:?}x{height:?}"
            );
        }
    }

    #[test]
    fn quality_uses_width_for_widescreen_files() {
        assert_eq!(
            quality_from_video_dimensions(Some(1920), Some(800)),
            Some("1080p")
        );
        assert_eq!(
            quality_from_video_dimensions(Some(3840), Some(1600)),
            Some("2160p")
        );
        assert_eq!(
            quality_from_video_dimensions(Some(7680), Some(3200)),
            Some("4320p")
        );
    }
}
