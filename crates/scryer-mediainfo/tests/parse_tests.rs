use scryer_mediainfo::{is_valid_video, parse_ffprobe_output};

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("fixture not found: {path}"))
}

#[test]
fn test_hevc_dv_truehd() {
    let analysis = parse_ffprobe_output(&fixture("hevc_dv_truehd.json")).unwrap();
    assert_eq!(analysis.video_codec.as_deref(), Some("hevc"));
    assert_eq!(analysis.video_width, Some(3840));
    assert_eq!(analysis.video_height, Some(2160));
    assert_eq!(analysis.video_bit_depth, Some(10));
    assert_eq!(analysis.video_hdr_format.as_deref(), Some("Dolby Vision"));
    assert_eq!(analysis.audio_codec.as_deref(), Some("truehd"));
    assert_eq!(analysis.audio_channels, Some(8));
    assert_eq!(analysis.audio_languages, vec!["jpn", "eng"]);
    assert!(analysis.has_multiaudio);
    assert_eq!(analysis.container_format.as_deref(), Some("matroska"));
    assert_eq!(analysis.duration_seconds, Some(7200));
    assert!(is_valid_video(&analysis));
}

#[test]
fn test_h264_web_aac() {
    let analysis = parse_ffprobe_output(&fixture("h264_web_aac.json")).unwrap();
    assert_eq!(analysis.video_codec.as_deref(), Some("h264"));
    assert_eq!(analysis.video_width, Some(1920));
    assert_eq!(analysis.video_height, Some(1080));
    assert_eq!(analysis.video_bit_depth, Some(8));
    assert_eq!(analysis.video_hdr_format, None);
    assert_eq!(analysis.audio_codec.as_deref(), Some("aac"));
    assert_eq!(analysis.audio_channels, Some(2));
    assert_eq!(analysis.audio_languages, vec!["eng"]);
    assert!(!analysis.has_multiaudio);
    assert!(analysis.subtitle_languages.is_empty());
    assert!(is_valid_video(&analysis));
}

#[test]
fn test_av1_hdr10_flac() {
    let analysis = parse_ffprobe_output(&fixture("av1_hdr10_flac.json")).unwrap();
    assert_eq!(analysis.video_codec.as_deref(), Some("av1"));
    assert_eq!(analysis.video_hdr_format.as_deref(), Some("HDR10"));
    assert_eq!(analysis.audio_codec.as_deref(), Some("flac"));
    assert_eq!(analysis.audio_channels, Some(6));
    assert!(is_valid_video(&analysis));
}

#[test]
fn test_dual_audio_anime() {
    let analysis = parse_ffprobe_output(&fixture("dual_audio_anime.json")).unwrap();
    assert_eq!(analysis.video_codec.as_deref(), Some("hevc"));
    assert_eq!(analysis.audio_codec.as_deref(), Some("flac"));
    assert_eq!(analysis.audio_languages, vec!["jpn", "eng"]);
    assert_eq!(analysis.subtitle_languages, vec!["eng"]);
    assert!(analysis.has_multiaudio);
    assert!(is_valid_video(&analysis));
}

#[test]
fn test_hevc_hlg() {
    let analysis = parse_ffprobe_output(&fixture("hevc_hlg.json")).unwrap();
    assert_eq!(analysis.video_codec.as_deref(), Some("hevc"));
    assert_eq!(analysis.video_hdr_format.as_deref(), Some("HLG"));
    assert!(is_valid_video(&analysis));
}

#[test]
fn test_no_language_tags() {
    let analysis = parse_ffprobe_output(&fixture("no_language_tags.json")).unwrap();
    assert_eq!(analysis.video_codec.as_deref(), Some("h264"));
    assert!(analysis.audio_languages.is_empty());
    assert!(analysis.subtitle_languages.is_empty());
    assert!(is_valid_video(&analysis));
}

#[test]
fn test_is_not_valid_video_when_no_streams() {
    let analysis = parse_ffprobe_output(&fixture("no_video_streams.json")).unwrap();
    assert!(analysis.video_codec.is_none());
    assert!(!is_valid_video(&analysis));
}

#[test]
fn test_unknown_fields_are_ignored() {
    // Simulates a future ffprobe version that adds extra fields.
    // The parser must not deny unknown fields.
    let json = r#"{
        "streams": [
            {
                "index": 0,
                "codec_name": "h264",
                "codec_type": "video",
                "width": 1920,
                "height": 1080,
                "bit_rate": "4000000",
                "future_stream_field": "some_value",
                "future_stream_number": 42,
                "tags": { "language": "eng", "future_tag": "ignored" },
                "side_data_list": [
                    { "side_data_type": "Unknown future side data", "extra": true }
                ]
            }
        ],
        "format": {
            "format_name": "matroska,webm",
            "duration": "3600.0",
            "future_format_field": true
        },
        "top_level_future_key": "ignored"
    }"#;

    let analysis = parse_ffprobe_output(json).unwrap();
    assert_eq!(analysis.video_codec.as_deref(), Some("h264"));
    assert_eq!(analysis.video_width, Some(1920));
    assert_eq!(analysis.duration_seconds, Some(3600));
    assert!(is_valid_video(&analysis));
}

#[test]
fn test_numeric_bit_rate_fields() {
    // Some ffprobe builds output numeric values for bit_rate / bits_per_raw_sample
    let json = r#"{
        "streams": [
            {
                "codec_name": "hevc",
                "codec_type": "video",
                "width": 3840,
                "height": 2160,
                "bit_rate": 30000000,
                "bits_per_raw_sample": 10,
                "color_transfer": "smpte2084",
                "tags": {}
            }
        ],
        "format": {
            "format_name": "matroska,webm",
            "duration": "7200.0"
        }
    }"#;

    let analysis = parse_ffprobe_output(json).unwrap();
    assert_eq!(analysis.video_codec.as_deref(), Some("hevc"));
    assert_eq!(analysis.video_bitrate_kbps, Some(30000));
    assert_eq!(analysis.video_bit_depth, Some(10));
}
