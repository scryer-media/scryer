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
    let best = audio_streams
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            metadata
                .get(*index)
                .is_none_or(|stream| stream.metadata.disposition.commentary != Some(true))
        })
        .map(|(index, stream)| {
            (
                index,
                stream,
                normalize_audio_codec_for_release(
                    stream.codec.as_deref(),
                    stream.profile.as_deref(),
                ),
            )
        })
        .max_by_key(|(index, stream, label)| {
            (
                label
                    .as_deref()
                    .map(audio_codec_rank_for_release_label)
                    .unwrap_or(0),
                stream.channels.unwrap_or(0),
                std::cmp::Reverse(*index),
            )
        });
    let (best_audio_label, audio_channels) = if let Some((index, stream, label)) = best {
        let layout = metadata
            .get(index)
            .and_then(|stream| stream.metadata.channel_layout.clone());
        let channels = layout.or_else(|| {
            (details.revision == 0)
                .then(|| stream.channels.map(format_audio_channels_for_release))
                .flatten()
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

#[cfg(any(feature = "runtime-media-analysis", test))]
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
        (w, h) if w >= 3840 || h >= 2100 => Some("2160p"),
        (_, h) if h >= 1300 => Some("1440p"),
        (w, h) if w >= 1920 || h >= 1000 => Some("1080p"),
        (w, h) if w >= 1280 || h >= 700 => Some("720p"),
        (w, h) if w >= 854 || h >= 480 => Some("480p"),
        (_, h) if h >= 300 => Some("360p"),
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

pub(crate) use scryer_media_types::{
    audio_codec_rank_for_release_label, normalize_audio_codec_for_release,
};

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

#[cfg(test)]
mod tests {
    use super::*;

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
