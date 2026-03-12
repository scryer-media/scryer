use scryer_mediainfo::{analyze_file, is_valid_video};
use std::path::PathBuf;

fn media(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("media")
        .join(name)
}

// ---------------------------------------------------------------------------
// MKV
// ---------------------------------------------------------------------------

#[test]
fn mkv_h264_aac() {
    let a = analyze_file(&media("h264_aac.mkv")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("h264"));
    assert_eq!(a.video_width, Some(128));
    assert_eq!(a.video_height, Some(72));
    assert_eq!(a.video_bit_depth, Some(8));
    assert_eq!(a.video_hdr_format, None);
    assert_eq!(a.audio_codec.as_deref(), Some("aac"));
    assert_eq!(a.audio_channels, Some(2));
    assert!(!a.has_multiaudio);
    assert_eq!(a.num_chapters, Some(0));
    assert_eq!(a.container_format.as_deref(), Some("matroska"));
    assert!(a.duration_seconds.unwrap() >= 1);
    assert!(is_valid_video(&a));
}

#[test]
fn mkv_hevc_hdr10_flac() {
    let a = analyze_file(&media("hevc_hdr10.mkv")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_width, Some(128));
    assert_eq!(a.video_height, Some(72));
    assert_eq!(a.video_bit_depth, Some(10));
    assert_eq!(a.video_hdr_format.as_deref(), Some("HDR10"));
    assert_eq!(a.audio_codec.as_deref(), Some("flac"));
    assert_eq!(a.audio_channels, Some(6));
    assert_eq!(a.container_format.as_deref(), Some("matroska"));
    assert!(is_valid_video(&a));
}

#[test]
fn mkv_hevc_hlg() {
    let a = analyze_file(&media("hevc_hlg.mkv")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_bit_depth, Some(10));
    assert_eq!(a.video_hdr_format.as_deref(), Some("HLG"));
    assert_eq!(a.audio_codec.as_deref(), Some("aac"));
    assert!(is_valid_video(&a));
}

#[test]
fn mkv_av1_flac() {
    let a = analyze_file(&media("av1_flac.mkv")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("av1"));
    assert_eq!(a.video_width, Some(128));
    assert_eq!(a.video_height, Some(72));
    assert_eq!(a.audio_codec.as_deref(), Some("flac"));
    assert_eq!(a.audio_channels, Some(6));
    assert_eq!(a.container_format.as_deref(), Some("matroska"));
    assert!(is_valid_video(&a));
}

#[test]
fn mkv_dual_audio_subtitles() {
    let a = analyze_file(&media("dual_audio_subs.mkv")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("h264"));
    assert_eq!(a.audio_codec.as_deref(), Some("flac"));
    assert_eq!(a.audio_channels, Some(6));
    assert!(a.has_multiaudio);
    assert_eq!(a.audio_languages, vec!["jpn", "eng"]);
    assert_eq!(a.subtitle_languages, vec!["eng"]);
    assert_eq!(a.audio_streams.len(), 2);
    assert_eq!(a.audio_streams[0].codec.as_deref(), Some("flac"));
    assert_eq!(a.audio_streams[0].channels, Some(6));
    assert_eq!(a.audio_streams[0].language.as_deref(), Some("jpn"));
    assert_eq!(a.audio_streams[1].codec.as_deref(), Some("aac"));
    assert_eq!(a.audio_streams[1].channels, Some(2));
    assert_eq!(a.audio_streams[1].language.as_deref(), Some("eng"));
    assert!(is_valid_video(&a));
}

// ---------------------------------------------------------------------------
// MP4
// ---------------------------------------------------------------------------

#[test]
fn mp4_h264_aac() {
    let a = analyze_file(&media("h264_aac.mp4")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("h264"));
    assert_eq!(a.video_width, Some(128));
    assert_eq!(a.video_height, Some(72));
    assert_eq!(a.audio_codec.as_deref(), Some("aac"));
    assert_eq!(a.audio_channels, Some(2));
    assert_eq!(a.num_chapters, None);
    assert_eq!(a.container_format.as_deref(), Some("mp4"));
    assert!(a.duration_seconds.unwrap() >= 1);
    assert!(is_valid_video(&a));
}

#[test]
fn mp4_av1_aac() {
    let a = analyze_file(&media("av1_aac.mp4")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("av1"));
    assert_eq!(a.video_width, Some(128));
    assert_eq!(a.video_height, Some(72));
    assert_eq!(a.audio_codec.as_deref(), Some("aac"));
    assert_eq!(a.audio_channels, Some(2));
    assert_eq!(a.container_format.as_deref(), Some("mp4"));
    assert!(is_valid_video(&a));
}

// ---------------------------------------------------------------------------
// AVI
// ---------------------------------------------------------------------------

#[test]
fn avi_mpeg4_mp3() {
    let a = analyze_file(&media("mpeg4_mp3.avi")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("mpeg4"));
    assert_eq!(a.video_width, Some(128));
    assert_eq!(a.video_height, Some(72));
    assert_eq!(a.audio_codec.as_deref(), Some("mp3"));
    assert_eq!(a.audio_channels, Some(2));
    assert_eq!(a.container_format.as_deref(), Some("avi"));
    assert!(a.duration_seconds.unwrap() >= 1);
    assert!(is_valid_video(&a));
}

// ---------------------------------------------------------------------------
// MPEG-TS
// ---------------------------------------------------------------------------

#[test]
fn ts_h264_aac() {
    let a = analyze_file(&media("h264_aac.ts")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("h264"));
    assert_eq!(a.audio_codec.as_deref(), Some("aac"));
    assert_eq!(a.container_format.as_deref(), Some("mpegts"));
    // TS duration is estimated from PTS delta; for a 2-second file it should
    // be at least 1 second after truncation.
    assert!(a.duration_seconds.is_some(), "duration should be present");
    assert!(
        a.duration_seconds.unwrap() >= 1,
        "duration {} should be >= 1",
        a.duration_seconds.unwrap()
    );
    assert!(is_valid_video(&a));
}

// ---------------------------------------------------------------------------
// Dolby Vision (MKV)
// ---------------------------------------------------------------------------

#[test]
fn mkv_dv_profile5() {
    let a = analyze_file(&media("dv_profile5.mkv")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_hdr_format.as_deref(), Some("Dolby Vision"));
    assert_eq!(a.dovi_profile, Some(5));
    assert_eq!(a.dovi_bl_compat_id, Some(0));
    assert!(is_valid_video(&a));
}

#[test]
fn mkv_dv_profile7() {
    let a = analyze_file(&media("dv_profile7.mkv")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_hdr_format.as_deref(), Some("Dolby Vision"));
    assert_eq!(a.dovi_profile, Some(7));
    assert_eq!(a.dovi_bl_compat_id, Some(6));
    assert!(is_valid_video(&a));
}

#[test]
fn mkv_dv_profile8() {
    let a = analyze_file(&media("dv_profile8.mkv")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_hdr_format.as_deref(), Some("Dolby Vision"));
    assert_eq!(a.dovi_profile, Some(8));
    assert_eq!(a.dovi_bl_compat_id, Some(1));
    assert!(is_valid_video(&a));
}

// ---------------------------------------------------------------------------
// Dolby Vision (MP4)
// ---------------------------------------------------------------------------

#[test]
fn mp4_dv_profile7() {
    let a = analyze_file(&media("dv_profile7.mp4")).unwrap();
    assert_eq!(a.video_hdr_format.as_deref(), Some("Dolby Vision"));
    assert_eq!(a.dovi_profile, Some(7));
    assert_eq!(a.dovi_bl_compat_id, Some(6));
    assert!(is_valid_video(&a));
}

#[test]
fn mp4_dv_profile8() {
    let a = analyze_file(&media("dv_profile8.mp4")).unwrap();
    assert_eq!(a.video_hdr_format.as_deref(), Some("Dolby Vision"));
    assert_eq!(a.dovi_profile, Some(8));
    assert_eq!(a.dovi_bl_compat_id, Some(1));
    assert!(is_valid_video(&a));
}

// ---------------------------------------------------------------------------
// HDR10+ (MKV + MP4)
// ---------------------------------------------------------------------------

#[test]
fn mkv_hevc_hdr10plus() {
    let a = analyze_file(&media("hevc_hdr10plus.mkv")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_hdr_format.as_deref(), Some("HDR10+"));
    assert_eq!(a.video_bit_depth, Some(10));
    assert!(is_valid_video(&a));
}

#[test]
fn mp4_hevc_hdr10plus() {
    let a = analyze_file(&media("hevc_hdr10plus.mp4")).unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_hdr_format.as_deref(), Some("HDR10+"));
    assert!(is_valid_video(&a));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn unsupported_extension_returns_error() {
    let err = analyze_file(&PathBuf::from("/tmp/fake.wmv")).unwrap_err();
    assert!(err.to_string().contains("unsupported format"));
}
