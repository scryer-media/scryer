//! Read-only views of parser facts that the integration tests assert on.
//!
//! These read the analysis details directly. They do not reproduce Scryer's
//! stored summary; where a test needs a selection rule (the primary audio
//! stream, the languages worth listing) it is stated here once and applied to
//! the native facts and the reference facts alike.
#![allow(dead_code)]

use scryer_media_model::{AnalysisDetails, StreamDetail, StreamKind};

pub fn video(details: &AnalysisDetails) -> Option<&StreamDetail> {
    details.selected_video()
}

pub fn video_codec(details: &AnalysisDetails) -> Option<&str> {
    video(details).and_then(|video| video.codec.as_deref())
}

/// Picture size: the container's dimensions, else the codec configuration's.
pub fn video_size(details: &AnalysisDetails) -> (Option<i32>, Option<i32>) {
    video(details).map_or((None, None), |video| {
        (
            video.width.or(video.metadata.configured_width),
            video.height.or(video.metadata.configured_height),
        )
    })
}

pub fn video_profile(details: &AnalysisDetails) -> Option<&str> {
    video(details).and_then(|video| video.metadata.profile.as_deref())
}

pub fn video_bit_depth(details: &AnalysisDetails) -> Option<i32> {
    video(details).and_then(|video| video.metadata.bit_depth)
}

pub fn video_bitrate_kbps(details: &AnalysisDetails) -> Option<i32> {
    video(details).and_then(|video| kbps(video.metadata.bitrate_bps))
}

/// The nominal frame rate, when it is a positive number.
pub fn video_fps(details: &AnalysisDetails) -> Option<f64> {
    video(details)
        .and_then(|video| video.metadata.nominal_frame_rate_fps)
        .filter(|fps| *fps > 0.0)
}

/// HDR signals on the selected video: Dolby Vision, HDR10+, HDR10, HLG.
pub fn hdr_signals(details: &AnalysisDetails) -> [bool; 4] {
    video(details).map_or([false; 4], |video| {
        let hdr = &video.metadata.hdr;
        [
            hdr.dolby_vision == Some(true),
            hdr.hdr10plus == Some(true),
            hdr.hdr10 == Some(true),
            hdr.hlg == Some(true),
        ]
    })
}

pub const DOLBY_VISION: [bool; 4] = [true, false, false, false];
pub const HDR10_PLUS: [bool; 4] = [false, true, false, false];
pub const HDR10: [bool; 4] = [false, false, true, false];
pub const NO_HDR: [bool; 4] = [false; 4];

/// Dolby Vision profile and base-layer compatibility id of the selected video.
pub fn dovi(details: &AnalysisDetails) -> (Option<u8>, Option<u8>) {
    video(details)
        .and_then(|video| video.metadata.hdr.dovi.as_ref())
        .map_or((None, None), |dovi| {
            (dovi.profile, dovi.base_layer_compatibility_id)
        })
}

/// A selected video stream with a codec and a duration of at least one second
/// once rounded.
pub fn has_video_with_duration(details: &AnalysisDetails) -> bool {
    video_codec(details).is_some() && duration_seconds(details).is_some_and(|seconds| seconds > 0)
}

pub fn duration_seconds(details: &AnalysisDetails) -> Option<i32> {
    details
        .duration_seconds
        .map(|seconds| seconds.round() as i32)
}

pub fn chapter_count(details: &AnalysisDetails) -> Option<i32> {
    details
        .chapter_count
        .and_then(|count| i32::try_from(count).ok())
}

pub fn audio(details: &AnalysisDetails) -> Vec<&StreamDetail> {
    details.program_streams(StreamKind::Audio).collect()
}

pub fn subtitles(details: &AnalysisDetails) -> Vec<&StreamDetail> {
    details.program_streams(StreamKind::Subtitle).collect()
}

/// A language tag that says something: neither empty nor `und`.
pub fn known_language(stream: &StreamDetail) -> Option<&str> {
    stream
        .language
        .as_deref()
        .filter(|language| !language.is_empty() && *language != "und")
}

/// Known languages of the program's audio, commentary excluded.
pub fn audio_languages(details: &AnalysisDetails) -> Vec<String> {
    audio(details)
        .into_iter()
        .filter(|stream| stream.metadata.disposition.commentary != Some(true))
        .filter_map(known_language)
        .map(str::to_owned)
        .collect()
}

pub fn subtitle_languages(details: &AnalysisDetails) -> Vec<String> {
    subtitles(details)
        .into_iter()
        .filter_map(known_language)
        .map(str::to_owned)
        .collect()
}

pub fn subtitle_codecs(details: &AnalysisDetails) -> Vec<String> {
    subtitles(details)
        .into_iter()
        .filter_map(|stream| stream.codec.clone())
        .collect()
}

/// The authored default stream that is not commentary, else the first that is
/// not. A stream counts as commentary when flagged so or titled so.
pub fn primary_audio_index<'a>(
    streams: impl IntoIterator<Item = (bool, bool, Option<&'a str>)>,
) -> Option<usize> {
    streams
        .into_iter()
        .enumerate()
        .filter(|(_, (_, commentary, title))| {
            !commentary
                && !title.is_some_and(|title| {
                    title
                        .split(|ch: char| !ch.is_alphanumeric())
                        .any(|word| word.eq_ignore_ascii_case("commentary"))
                })
        })
        .min_by_key(|(index, (default, _, _))| (!default, *index))
        .map(|(index, _)| index)
}

pub fn primary_audio(details: &AnalysisDetails) -> Option<&StreamDetail> {
    let audio = audio(details);
    primary_audio_index(audio.iter().map(|stream| {
        let disposition = &stream.metadata.disposition;
        (
            disposition.default.unwrap_or(false),
            disposition.commentary == Some(true),
            stream.name.as_deref(),
        )
    }))
    .map(|index| audio[index])
}

pub fn kbps(bits_per_second: Option<u64>) -> Option<i32> {
    bits_per_second.map(|bps| (bps / 1000) as i32)
}
