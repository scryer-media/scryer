//! Scryer's stored media summary, built from the parser's analysis details.
//!
//! The parser reports facts per stream and picks the primary audio, the HDR
//! format, the frame rate text and whether a file is playable video. Which
//! languages are worth keeping, how bitrates and durations are rounded and
//! how codecs map to release names are Scryer's decisions, made here.

use scryer_media_types::{AnalysisDetails, StreamDetail, StreamKind};

use crate::{AudioStreamDetail, MediaFileAnalysis, SubtitleStreamDetail};

/// Build the stored summary, keeping the details alongside it unchanged.
pub(crate) fn summarize(details: AnalysisDetails) -> MediaFileAnalysis {
    let mut summary = summarize_facts(&details);
    summary.details = details;
    summary
}

/// Every summary field except the details themselves.
fn summarize_facts(details: &AnalysisDetails) -> MediaFileAnalysis {
    let video = details.selected_video();
    let audio: Vec<&StreamDetail> = details.program_streams(StreamKind::Audio).collect();
    let subtitles: Vec<&StreamDetail> = details.program_streams(StreamKind::Subtitle).collect();
    let primary_audio = details.primary_audio();
    let (video_width, video_height) = video.map(StreamDetail::picture_size).unwrap_or_default();

    MediaFileAnalysis {
        details: AnalysisDetails::default(),
        video_codec: video
            .and_then(|video| video.codec.as_ref())
            .and_then(|codec| crate::release_parser::VideoCodec::parse(codec.as_str())),
        video_width,
        video_height,
        video_bitrate_kbps: video.and_then(|video| kbps(video.metadata.bitrate_bps)),
        video_bit_depth: video.and_then(|video| video.metadata.bit_depth),
        video_hdr_format: video
            .and_then(StreamDetail::hdr_format)
            .map(|format| format.as_str().to_owned()),
        dovi_profile: video
            .and_then(|video| video.metadata.hdr.dovi.as_ref())
            .and_then(|dovi| dovi.profile),
        dovi_bl_compat_id: video
            .and_then(|video| video.metadata.hdr.dovi.as_ref())
            .and_then(|dovi| dovi.base_layer_compatibility_id),
        video_frame_rate: details.frame_rate_text(),
        video_profile: video.and_then(|video| video.metadata.profile.clone()),
        audio_codec: primary_audio.and_then(codec_name),
        audio_profile: primary_audio.and_then(|stream| stream.metadata.profile.clone()),
        audio_channels: primary_audio.and_then(|stream| stream.channels),
        audio_bitrate_kbps: primary_audio.and_then(|stream| kbps(stream.metadata.bitrate_bps)),
        audio_languages: crate::normalize_detected_audio_languages(
            audio
                .iter()
                .filter(|stream| stream.metadata.disposition.commentary != Some(true))
                .filter_map(|stream| known_language(stream)),
        ),
        audio_streams: audio
            .iter()
            .map(|stream| AudioStreamDetail {
                codec: codec_name(stream),
                profile: stream.metadata.profile.clone(),
                channels: stream.channels,
                language: known_language(stream)
                    .and_then(crate::normalize_detected_audio_language_code),
                name: stream.name.clone(),
                bitrate_kbps: kbps(stream.metadata.bitrate_bps),
            })
            .collect(),
        subtitle_languages: crate::normalize_detected_subtitle_languages(
            subtitles.iter().filter_map(|stream| known_language(stream)),
        ),
        subtitle_codecs: subtitles
            .iter()
            .filter_map(|stream| codec_name(stream))
            .collect(),
        subtitle_streams: subtitles
            .iter()
            .map(|stream| SubtitleStreamDetail {
                codec: codec_name(stream),
                language: known_language(stream)
                    .and_then(crate::normalize_detected_subtitle_language_code),
                name: stream.name.clone(),
                forced: stream.metadata.disposition.forced == Some(true),
                default: stream.metadata.disposition.default == Some(true),
            })
            .collect(),
        has_multiaudio: audio.len() > 1,
        duration_seconds: duration_seconds(details),
        num_chapters: details
            .chapter_count
            .and_then(|count| i32::try_from(count).ok()),
        container_format: details.container_format.clone(),
    }
}

/// Duration rounded to whole seconds.
pub(crate) fn duration_seconds(details: &AnalysisDetails) -> Option<i32> {
    details
        .duration_seconds
        .map(|seconds| seconds.round() as i32)
}

/// The codec name Scryer stores.
fn codec_name(stream: &StreamDetail) -> Option<String> {
    stream.codec.as_ref().map(|codec| codec.as_str().to_owned())
}

/// A language tag that says something: neither empty nor `und`.
fn known_language(stream: &StreamDetail) -> Option<&str> {
    stream
        .language
        .as_deref()
        .filter(|language| !language.is_empty() && *language != "und")
}

fn kbps(bits_per_second: Option<u64>) -> Option<i32> {
    bits_per_second.map(|bps| (bps / 1000) as i32)
}

#[cfg(test)]
mod tests {
    use scryer_media_types::{
        Codec, DiscMetadata, DolbyVision, HdrCapabilities, Rational, StreamDisposition,
        StreamMetadata,
    };

    use super::*;

    fn video(codec: Option<&str>) -> StreamDetail {
        StreamDetail {
            kind: StreamKind::Video,
            codec: codec.map(Codec::from),
            metadata: StreamMetadata {
                id: Some("v".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn audio(codec: &str, channels: i32, language: Option<&str>) -> StreamDetail {
        StreamDetail {
            kind: StreamKind::Audio,
            codec: Some(Codec::from(codec)),
            channels: Some(channels),
            language: language.map(str::to_owned),
            ..Default::default()
        }
    }

    fn subtitle(codec: &str, language: Option<&str>) -> StreamDetail {
        StreamDetail {
            kind: StreamKind::Subtitle,
            codec: Some(Codec::from(codec)),
            language: language.map(str::to_owned),
            ..Default::default()
        }
    }

    fn with_default(mut stream: StreamDetail, default: Option<bool>) -> StreamDetail {
        stream.metadata.disposition.default = default;
        stream
    }

    fn commentary(mut stream: StreamDetail) -> StreamDetail {
        stream.metadata.disposition.commentary = Some(true);
        stream
    }

    fn in_program(mut stream: StreamDetail, program: u32) -> StreamDetail {
        stream.metadata.program_id = Some(program);
        stream
    }

    fn details(streams: Vec<StreamDetail>) -> AnalysisDetails {
        let has_video = streams
            .iter()
            .any(|stream| stream.kind == StreamKind::Video);
        AnalysisDetails {
            selected_video_id: has_video.then(|| "v".to_owned()),
            duration_seconds: Some(60.0),
            streams,
            ..Default::default()
        }
    }

    #[test]
    fn primary_audio_keeps_a_mono_main_track_over_a_default_commentary() {
        let mut main = audio("ac3", 1, Some("eng"));
        main.name = Some("Main".to_owned());
        let mut director = with_default(commentary(audio("aac", 6, Some("eng"))), Some(true));
        director.name = Some("Director".to_owned());
        let summary = summarize(details(vec![video(Some("h264")), main, director]));

        assert_eq!(summary.audio_codec.as_deref(), Some("ac3"));
        assert_eq!(summary.audio_channels, Some(1));
        assert_eq!(summary.audio_streams.len(), 2);
        assert!(summary.has_multiaudio);
    }

    #[test]
    fn primary_audio_is_the_default_then_the_first_non_commentary() {
        let default_second = details(vec![
            video(Some("h264")),
            with_default(audio("aac", 2, None), Some(false)),
            with_default(audio("eac3", 6, None), Some(true)),
        ]);
        assert_eq!(
            default_second
                .primary_audio()
                .and_then(|s| s.codec.as_ref().map(Codec::as_str)),
            Some("eac3")
        );

        let mut titled = audio("aac", 2, None);
        titled.name = Some("Audio Commentary (Cast)".to_owned());
        let first_titled_commentary = details(vec![
            video(Some("h264")),
            titled,
            audio("dts", 6, None),
            audio("ac3", 6, None),
        ]);
        assert_eq!(
            first_titled_commentary
                .primary_audio()
                .and_then(|s| s.codec.as_ref().map(Codec::as_str)),
            Some("dts")
        );

        let undeclared_default = details(vec![
            video(Some("h264")),
            with_default(audio("flac", 2, None), None),
            audio("opus", 2, None),
        ]);
        assert_eq!(
            undeclared_default
                .primary_audio()
                .and_then(|s| s.codec.as_ref().map(Codec::as_str)),
            Some("flac")
        );

        let only_commentary = details(vec![video(Some("h264")), commentary(audio("aac", 2, None))]);
        assert!(only_commentary.primary_audio().is_none());
        let summary = summarize(only_commentary);
        assert_eq!(summary.audio_codec, None);
        assert_eq!(summary.audio_streams.len(), 1);
    }

    #[test]
    fn primary_audio_fields_come_from_the_default_stream() {
        let mut first = audio("aac", 2, Some("jpn"));
        first.metadata.profile = Some("LC".to_owned());
        first.metadata.bitrate_bps = Some(192_000);
        let mut default = with_default(audio("eac3", 6, Some("eng")), Some(true));
        default.metadata.bitrate_bps = Some(640_999);
        let summary = summarize(details(vec![video(Some("hevc")), first, default]));

        assert_eq!(summary.audio_codec.as_deref(), Some("eac3"));
        assert_eq!(summary.audio_profile, None);
        assert_eq!(summary.audio_channels, Some(6));
        assert_eq!(summary.audio_bitrate_kbps, Some(640));
        assert_eq!(summary.audio_streams[0].bitrate_kbps, Some(192));
        assert_eq!(summary.audio_streams[0].profile.as_deref(), Some("LC"));
    }

    #[test]
    fn the_selected_program_bounds_audio_and_subtitles() {
        let mut analysis = details(vec![
            in_program(video(Some("h264")), 1),
            in_program(audio("truehd", 8, Some("fra")), 1),
            in_program(with_default(audio("aac", 2, Some("eng")), Some(true)), 1),
            in_program(commentary(audio("aac", 2, Some("spa"))), 1),
            in_program(subtitle("subrip", Some("deu")), 1),
            in_program(audio("mp2", 2, Some("ita")), 2),
            in_program(subtitle("dvb_subtitle", Some("nld")), 2),
        ]);
        analysis.selected_program_id = Some(1);
        let summary = summarize(analysis.clone());

        assert_eq!(summary.audio_codec.as_deref(), Some("aac"));
        assert_eq!(summary.audio_channels, Some(2));
        assert_eq!(
            summary
                .audio_streams
                .iter()
                .map(|stream| stream.codec.as_deref())
                .collect::<Vec<_>>(),
            [Some("truehd"), Some("aac"), Some("aac")]
        );
        assert!(summary.has_multiaudio);
        assert_eq!(summary.audio_languages, ["fra", "eng"]);
        assert_eq!(summary.subtitle_codecs, ["subrip"]);
        assert_eq!(summary.subtitle_languages, ["deu"]);
    }

    #[test]
    fn empty_and_undetermined_languages_are_dropped() {
        let mut forced = subtitle("ass", Some("eng"));
        forced.metadata.disposition = StreamDisposition {
            forced: Some(true),
            default: Some(false),
            ..Default::default()
        };
        let summary = summarize(details(vec![
            video(Some("h264")),
            audio("aac", 2, Some("und")),
            audio("aac", 2, Some("")),
            audio("aac", 2, Some("jpn")),
            subtitle("subrip", Some("und")),
            forced,
        ]));

        assert_eq!(summary.audio_languages, ["jpn"]);
        assert_eq!(
            summary
                .audio_streams
                .iter()
                .map(|stream| stream.language.as_deref())
                .collect::<Vec<_>>(),
            [None, None, Some("jpn")]
        );
        assert_eq!(summary.subtitle_languages, ["eng"]);
        assert_eq!(summary.subtitle_codecs, ["subrip", "ass"]);
        assert_eq!(summary.subtitle_streams[0].language, None);
        assert!(!summary.subtitle_streams[0].forced);
        assert!(!summary.subtitle_streams[0].default);
        assert!(summary.subtitle_streams[1].forced);
        assert!(!summary.subtitle_streams[1].default);
    }

    #[test]
    fn hdr_label_names_the_strongest_signal() {
        let signals = |dolby_vision, hdr10plus, hdr10, hlg| HdrCapabilities {
            dolby_vision,
            hdr10plus,
            hdr10,
            hlg,
            pq: Some(true),
            dovi: None,
        };
        let yes = Some(true);
        let no = Some(false);
        for (hdr, label) in [
            (signals(yes, yes, yes, yes), Some("Dolby Vision")),
            (signals(no, yes, yes, yes), Some("HDR10+")),
            (signals(None, None, yes, yes), Some("HDR10")),
            (signals(None, None, None, yes), Some("HLG")),
            (signals(no, no, no, no), None),
            (HdrCapabilities::default(), None),
        ] {
            let mut picture = video(Some("hevc"));
            picture.metadata.hdr = hdr.clone();
            let summary = summarize(details(vec![picture]));
            assert_eq!(summary.video_hdr_format.as_deref(), label, "{hdr:?}");
        }
    }

    #[test]
    fn dolby_vision_record_reaches_the_summary() {
        let mut picture = video(Some("hevc"));
        picture.metadata.hdr = HdrCapabilities {
            dolby_vision: Some(true),
            dovi: Some(DolbyVision {
                profile: Some(8),
                base_layer_compatibility_id: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        };
        let summary = summarize(details(vec![picture]));
        assert_eq!(summary.dovi_profile, Some(8));
        assert_eq!(summary.dovi_bl_compat_id, Some(1));
    }

    #[test]
    fn frame_rate_text_trims_the_nominal_rate() {
        let text = |fps: Option<f64>| {
            let mut picture = video(Some("h264"));
            picture.metadata.nominal_frame_rate_fps = fps;
            picture.metadata.declared_frame_rate = Rational::new(30, 1);
            summarize(details(vec![picture])).video_frame_rate
        };
        assert_eq!(text(Some(24_000.0 / 1_001.0)).as_deref(), Some("23.976"));
        assert_eq!(text(Some(24.0)).as_deref(), Some("24"));
        assert_eq!(text(Some(25.5)).as_deref(), Some("25.5"));
        assert_eq!(text(Some(29.9996)).as_deref(), Some("30"));
        assert_eq!(text(Some(0.0)), None);
        assert_eq!(text(Some(-1.0)), None);
        // Outside a disc the declared rate is never a fallback.
        assert_eq!(text(None), None);
    }

    #[test]
    fn a_disc_title_without_a_nominal_rate_writes_its_declared_rate() {
        let disc = |fps: Option<f64>| {
            let mut picture = video(Some("mpeg2video"));
            picture.metadata.nominal_frame_rate_fps = fps;
            picture.metadata.declared_frame_rate = Rational::new(24, 1);
            let mut analysis = details(vec![picture]);
            analysis.disc = Some(DiscMetadata::default());
            summarize(analysis.clone()).video_frame_rate
        };
        assert_eq!(disc(None).as_deref(), Some("24.000"));
        assert_eq!(disc(Some(0.0)).as_deref(), Some("24.000"));
        assert_eq!(disc(Some(25.0)).as_deref(), Some("25"));
    }

    #[test]
    fn valid_video_needs_a_coded_picture_and_a_second_of_duration() {
        let with_duration = |streams: Vec<StreamDetail>, seconds: Option<f64>| {
            let mut analysis = details(streams);
            analysis.duration_seconds = seconds;
            analysis.is_playable_video()
        };
        assert!(with_duration(vec![video(Some("h264"))], Some(0.5)));
        assert!(!with_duration(vec![video(Some("h264"))], Some(0.49)));
        assert!(!with_duration(vec![video(Some("h264"))], None));
        assert!(!with_duration(vec![video(None)], Some(60.0)));
        assert!(!with_duration(vec![audio("mp3", 2, None)], Some(60.0)));

        let mut unselected = details(vec![video(Some("h264"))]);
        unselected.selected_video_id = None;
        assert!(!unselected.is_playable_video());
        assert_eq!(summarize(unselected).video_codec, None);
    }

    #[test]
    fn picture_size_falls_back_to_the_codec_configuration() {
        let mut configured = video(Some("h264"));
        configured.metadata.configured_width = Some(1920);
        configured.metadata.configured_height = Some(1080);
        let summary = summarize(details(vec![configured.clone()]));
        assert_eq!(
            (summary.video_width, summary.video_height),
            (Some(1920), Some(1080))
        );

        configured.width = Some(1440);
        configured.height = Some(1080);
        let summary = summarize(details(vec![configured]));
        assert_eq!(
            (summary.video_width, summary.video_height),
            (Some(1440), Some(1080))
        );
    }

    #[test]
    fn container_facts_are_rounded_and_counted() {
        let mut picture = video(Some("h264"));
        picture.metadata.bitrate_bps = Some(8_499_999);
        picture.metadata.bit_depth = Some(10);
        picture.metadata.profile = Some("High 10".to_owned());
        let mut analysis = details(vec![picture]);
        analysis.container_format = Some("matroska".to_owned());
        analysis.duration_seconds = Some(7_019.5);
        analysis.chapter_count = Some(12);
        let summary = summarize(analysis.clone());

        assert_eq!(summary.container_format.as_deref(), Some("matroska"));
        assert_eq!(summary.duration_seconds, Some(7_020));
        assert_eq!(summary.num_chapters, Some(12));
        assert_eq!(summary.video_bitrate_kbps, Some(8_499));
        assert_eq!(summary.video_bit_depth, Some(10));
        assert_eq!(summary.video_profile.as_deref(), Some("High 10"));
        assert!(!summary.has_multiaudio);
        assert_eq!(summary.details, analysis);

        analysis.chapter_count = Some(u32::MAX);
        assert_eq!(summarize(analysis.clone()).num_chapters, None);
        analysis.chapter_count = None;
        assert_eq!(summarize(analysis.clone()).num_chapters, None);
    }
}
