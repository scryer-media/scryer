//! Project authored playback titles without mixing their stream or storage facts.
use scryer_media_types::{DiscTitle, StreamKind};

pub(crate) fn for_title(
    image: &crate::MediaFileAnalysis,
    title: &DiscTitle,
) -> crate::MediaFileAnalysis {
    let video = title.streams.iter().find(|stream| {
        stream.kind == StreamKind::Video
            && stream.metadata.disposition.attached_picture != Some(true)
            && stream.metadata.disposition.still_image != Some(true)
            && (stream.metadata.disposition.attached_picture.is_some()
                || !matches!(stream.codec.as_deref(), Some("mjpeg" | "png")))
    });
    let program = video.and_then(|stream| stream.metadata.program_id);
    let streams = title
        .streams
        .iter()
        .filter(|stream| program.is_none() || stream.metadata.program_id == program)
        .collect::<Vec<_>>();
    let audio = streams
        .iter()
        .filter(|stream| stream.kind == StreamKind::Audio)
        .collect::<Vec<_>>();
    let eligible = audio
        .iter()
        .enumerate()
        .filter(|(_, stream)| stream.metadata.disposition.commentary != Some(true));
    let best = eligible
        .clone()
        .max_by_key(|(index, stream)| {
            (
                scryer_media_types::normalize_audio_codec_for_release(
                    stream.codec.as_deref(),
                    stream.metadata.profile.as_deref(),
                )
                .as_deref()
                .map(scryer_media_types::audio_codec_rank_for_release_label)
                .unwrap_or(0),
                stream.channels.unwrap_or(0),
                std::cmp::Reverse(*index),
            )
        })
        .map(|(_, stream)| *stream);
    let subtitle = streams
        .iter()
        .filter(|stream| stream.kind == StreamKind::Subtitle)
        .collect::<Vec<_>>();
    let mut analysis = crate::MediaFileAnalysis {
        details: image.details.clone(),
        container_format: Some("iso".into()),
        duration_seconds: title
            .duration_seconds
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0 && *seconds <= i32::MAX as f64)
            .map(|seconds| seconds as i32),
        num_chapters: i32::try_from(title.chapters.len()).ok(),
        video_codec: video
            .and_then(|stream| stream.codec.as_deref())
            .and_then(crate::release_parser::VideoCodec::parse),
        video_width: video.and_then(|stream| stream.width),
        video_height: video.and_then(|stream| stream.height),
        video_bitrate_kbps: video.and_then(|stream| kbps(stream.metadata.bitrate_bps)),
        video_bit_depth: video.and_then(|stream| stream.metadata.bit_depth),
        video_profile: video.and_then(|stream| stream.metadata.profile.clone()),
        video_frame_rate: video
            .and_then(|stream| {
                stream
                    .metadata
                    .declared_frame_rate
                    .or(stream.metadata.observed_frame_rate)
            })
            .filter(|rate| rate.denominator > 0)
            .map(|rate| format!("{:.3}", rate.numerator as f64 / rate.denominator as f64)),
        audio_codec: best.and_then(|stream| stream.codec.clone()),
        audio_profile: best.and_then(|stream| stream.metadata.profile.clone()),
        audio_channels: best.and_then(|stream| stream.channels),
        audio_bitrate_kbps: best.and_then(|stream| kbps(stream.metadata.bitrate_bps)),
        audio_languages: crate::normalize_detected_audio_languages(
            eligible
                .clone()
                .filter_map(|(_, stream)| stream.language.as_deref()),
        ),
        has_multiaudio: eligible.count() > 1,
        audio_streams: audio
            .into_iter()
            .map(|stream| crate::AudioStreamDetail {
                codec: stream.codec.clone(),
                profile: stream.metadata.profile.clone(),
                channels: stream.channels,
                language: stream
                    .language
                    .as_deref()
                    .and_then(crate::normalize_detected_audio_language_code),
                name: stream.name.clone(),
                bitrate_kbps: kbps(stream.metadata.bitrate_bps),
            })
            .collect(),
        subtitle_languages: crate::normalize_detected_subtitle_languages(
            subtitle
                .iter()
                .filter_map(|stream| stream.language.as_deref()),
        ),
        subtitle_codecs: subtitle
            .iter()
            .filter_map(|stream| stream.codec.clone())
            .collect(),
        subtitle_streams: subtitle
            .into_iter()
            .map(|stream| crate::SubtitleStreamDetail {
                codec: stream.codec.clone(),
                language: stream
                    .language
                    .as_deref()
                    .and_then(crate::normalize_detected_subtitle_language_code),
                name: stream.name.clone(),
                forced: stream.metadata.disposition.forced.unwrap_or(false),
                default: stream.metadata.disposition.default.unwrap_or(false),
            })
            .collect(),
        ..Default::default()
    };
    if let Some(video) = video {
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
        analysis.dovi_profile = hdr.dovi.as_ref().and_then(|dovi| dovi.profile);
        analysis.dovi_bl_compat_id = hdr
            .dovi
            .as_ref()
            .and_then(|dovi| dovi.base_layer_compatibility_id);
    }
    analysis.details.streams = title.streams.clone();
    analysis.details.selected_video_id = video.and_then(|stream| stream.metadata.id.clone());
    analysis.details.selected_program_id = program;
    analysis.details.duration_seconds = title.duration_seconds;
    analysis.details.duration_provenance = scryer_media_types::Provenance::Container;
    analysis.details.overall_bitrate_bps = None;
    analysis.details.chapters = title.chapters.clone();
    analysis.details.report = title.report.clone();
    if let Some(disc) = &mut analysis.details.disc {
        disc.selected_title_id = Some(title.id.clone());
    }
    analysis
}

fn kbps(value: Option<u64>) -> Option<i32> {
    value
        .and_then(|bps| i32::try_from(bps / 1000).ok())
        .filter(|bps| *bps > 0)
}
