use scryer_mediainfo::{
    AnalysisProfile, AnalyzeOptions, MediaAnalysis, analyze_file, analyze_file_with_options,
    is_valid_video,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn media(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("media")
        .join(name)
}

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    fixtures: Vec<FixtureExpectation>,
}

#[derive(Debug, Deserialize)]
struct FixtureExpectation {
    name: String,
    generated: bool,
    container: String,
    video_codec: String,
    width: i32,
    height: i32,
    fps: i32,
    audio_codecs: Vec<String>,
    audio_channels: Vec<i32>,
    audio_languages: Vec<String>,
    subtitle_stream_count: usize,
    min_duration_seconds: i32,
    valid_video: bool,
}

fn fixture_manifest() -> FixtureManifest {
    toml::from_str(include_str!("media-fixtures.toml"))
        .expect("media fixture manifest should parse")
}

fn analyze_fixture(fixture: &FixtureExpectation) -> MediaAnalysis {
    analyze_file_with_options(
        &media(&fixture.name),
        AnalyzeOptions {
            profile: AnalysisProfile::DefaultRich,
        },
    )
    .unwrap_or_else(|error| panic!("{} should analyze: {error}", fixture.name))
}

#[test]
fn fixture_matrix_has_expected_generated_size() {
    let manifest = fixture_manifest();
    let generated = manifest
        .fixtures
        .iter()
        .filter(|fixture| fixture.generated)
        .count();

    assert_eq!(
        generated, 244,
        "fixture manifest should contain 235 matrix fixtures plus 9 dense SIMD fixtures"
    );
}

#[test]
fn generated_container_corpus_covers_reproducible_codec_and_layout_branches() {
    let manifest = fixture_manifest();
    let fixtures_for = |extension: &str| {
        manifest
            .fixtures
            .iter()
            .filter(|fixture| fixture.name.ends_with(extension))
            .collect::<Vec<_>>()
    };
    let wmv = fixtures_for(".wmv");
    let ogv = fixtures_for(".ogv");
    let flv = fixtures_for(".flv");

    assert!(wmv.len() >= 13, "ASF/WMV corpus unexpectedly shrank");
    assert!(ogv.len() >= 10, "Ogg/Theora corpus unexpectedly shrank");
    assert!(flv.len() >= 12, "FLV corpus unexpectedly shrank");

    let video_codecs = |fixtures: &[&FixtureExpectation]| {
        fixtures
            .iter()
            .map(|fixture| fixture.video_codec.clone())
            .collect::<BTreeSet<_>>()
    };
    let audio_codecs = |fixtures: &[&FixtureExpectation]| {
        fixtures
            .iter()
            .flat_map(|fixture| fixture.audio_codecs.iter().cloned())
            .collect::<BTreeSet<_>>()
    };
    let codec_set = |codecs: &[&str]| {
        codecs
            .iter()
            .map(|codec| (*codec).to_owned())
            .collect::<BTreeSet<_>>()
    };
    let channels = |fixtures: &[&FixtureExpectation]| {
        fixtures
            .iter()
            .flat_map(|fixture| fixture.audio_channels.iter().copied())
            .collect::<BTreeSet<_>>()
    };
    let dimensions = |fixtures: &[&FixtureExpectation]| {
        fixtures
            .iter()
            .map(|fixture| (fixture.width, fixture.height))
            .collect::<BTreeSet<_>>()
    };
    let frame_rates = |fixtures: &[&FixtureExpectation]| {
        fixtures
            .iter()
            .map(|fixture| fixture.fps)
            .collect::<BTreeSet<_>>()
    };
    let languages = |fixtures: &[&FixtureExpectation]| {
        fixtures
            .iter()
            .flat_map(|fixture| fixture.audio_languages.iter().cloned())
            .collect::<BTreeSet<_>>()
    };

    // These are the codecs the fixture FFmpeg can encode into valid files. Header-only mappings
    // without an FFmpeg encoder (WMV3/VC-1 and FLV VP6/MPEG-4) have focused parser unit tests.
    assert_eq!(video_codecs(&wmv), codec_set(&["wmv1", "wmv2"]));
    assert!(
        audio_codecs(&wmv).is_superset(&codec_set(&[
            "aac",
            "ac3",
            "mp3",
            "pcm_f32le",
            "pcm_s16le",
            "pcm_s24le",
            "pcm_s32le",
            "pcm_u8",
            "wmav1",
            "wmav2",
        ])),
        "ASF/WMV corpus lost a WAVEFORMATEX or WAVEFORMATEXTENSIBLE branch"
    );
    assert!(channels(&wmv).is_superset(&BTreeSet::from([1, 2, 6])));
    assert_eq!(languages(&wmv), codec_set(&["eng", "jpn", "spa"]));
    assert!(wmv.iter().any(|fixture| fixture.audio_codecs.is_empty()));
    assert!(wmv.iter().any(|fixture| fixture.audio_codecs.len() == 2));

    assert_eq!(video_codecs(&ogv), codec_set(&["theora"]));
    assert_eq!(audio_codecs(&ogv), codec_set(&["opus", "vorbis"]));
    assert!(channels(&ogv).is_superset(&BTreeSet::from([1, 2, 6])));
    assert_eq!(languages(&ogv), codec_set(&["eng", "jpn", "spa"]));
    assert!(ogv.iter().any(|fixture| fixture.audio_codecs.is_empty()));
    assert!(ogv.iter().any(|fixture| fixture.audio_codecs.len() == 2));

    assert_eq!(video_codecs(&flv), codec_set(&["flv1", "h264"]));
    assert!(
        audio_codecs(&flv).is_superset(&codec_set(&[
            "aac",
            "adpcm_swf",
            "mp3",
            "nellymoser",
            "pcm_alaw",
            "pcm_mulaw",
            "pcm_s16le",
            "pcm_u8",
            "speex",
        ])),
        "FLV corpus lost coverage for one of the demuxer audio tag IDs"
    );
    assert!(flv.iter().any(|fixture| fixture.audio_codecs.is_empty()));

    for (format, fixtures) in [("WMV", &wmv), ("OGV", &ogv), ("FLV", &flv)] {
        assert!(
            dimensions(fixtures).len() >= 4,
            "{format} corpus must retain four dimension pairs"
        );
        assert!(
            frame_rates(fixtures).len() >= 5,
            "{format} corpus must retain five frame rates"
        );
    }
}

#[test]
fn fixture_matrix_expected_metadata() {
    let manifest = fixture_manifest();

    for fixture in manifest.fixtures.iter().filter(|fixture| fixture.generated) {
        let analysis = analyze_fixture(fixture);

        assert_eq!(
            analysis.container_format.as_deref(),
            Some(fixture.container.as_str()),
            "{} container",
            fixture.name
        );
        assert_eq!(
            analysis.video_codec.as_deref(),
            Some(fixture.video_codec.as_str()),
            "{} video codec",
            fixture.name
        );
        assert_eq!(
            analysis.video_width,
            Some(fixture.width),
            "{} video width",
            fixture.name
        );
        assert_eq!(
            analysis.video_height,
            Some(fixture.height),
            "{} video height",
            fixture.name
        );
        let actual_fps = analysis
            .video_frame_rate
            .as_deref()
            .and_then(|fps| fps.parse::<f64>().ok())
            .unwrap_or_default();
        assert!(
            (actual_fps - f64::from(fixture.fps)).abs() < 0.001,
            "{} frame rate {:?} should equal {}",
            fixture.name,
            analysis.video_frame_rate,
            fixture.fps
        );
        assert!(
            analysis.duration_seconds.unwrap_or_default() >= fixture.min_duration_seconds,
            "{} duration {:?} should be at least {}",
            fixture.name,
            analysis.duration_seconds,
            fixture.min_duration_seconds
        );
        assert_eq!(
            is_valid_video(&analysis),
            fixture.valid_video,
            "{} validity",
            fixture.name
        );

        let actual_audio_codecs: Vec<_> = analysis
            .audio_streams
            .iter()
            .filter_map(|stream| stream.codec.clone())
            .collect();
        assert_eq!(
            actual_audio_codecs, fixture.audio_codecs,
            "{} audio codecs",
            fixture.name
        );

        let actual_audio_channels: Vec<_> = analysis
            .audio_streams
            .iter()
            .filter_map(|stream| stream.channels)
            .collect();
        assert_eq!(
            actual_audio_channels, fixture.audio_channels,
            "{} audio channels",
            fixture.name
        );

        assert_eq!(
            analysis.has_multiaudio,
            fixture.audio_codecs.len() > 1,
            "{} multiaudio flag",
            fixture.name
        );
        assert_eq!(
            analysis.audio_languages, fixture.audio_languages,
            "{} audio languages",
            fixture.name
        );
        assert_eq!(
            analysis.subtitle_streams.len(),
            fixture.subtitle_stream_count,
            "{} subtitle stream count",
            fixture.name
        );
    }
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
// Emerging MKV metadata
// ---------------------------------------------------------------------------

#[test]
fn mkv_h264_8k_fixture_reports_dimensions() {
    let a = analyze_file(&media("h264_8k_aac.mkv")).unwrap();
    assert_eq!(a.container_format.as_deref(), Some("matroska"));
    assert_eq!(a.video_codec.as_deref(), Some("h264"));
    assert_eq!(a.video_width, Some(7680));
    assert_eq!(a.video_height, Some(4320));
    assert!(is_valid_video(&a));
}

// ---------------------------------------------------------------------------
// HDR10+ (MKV + MP4)
// ---------------------------------------------------------------------------

#[test]
fn mkv_hevc_hdr10plus() {
    let a = analyze_file_with_options(
        &media("hevc_hdr10plus.mkv"),
        AnalyzeOptions {
            profile: AnalysisProfile::DefaultRich,
        },
    )
    .unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_hdr_format.as_deref(), Some("HDR10+"));
    assert_eq!(a.video_bit_depth, Some(10));
    assert!(is_valid_video(&a));
}

#[test]
fn mkv_hevc_hdr10plus_content_probe_profile() {
    let a = analyze_file_with_options(
        &media("hevc_hdr10plus.mkv"),
        AnalyzeOptions {
            profile: AnalysisProfile::ContentProbe,
        },
    )
    .unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_hdr_format, None);
    assert!(is_valid_video(&a));
}

#[test]
fn mpegts_content_probe_profile_identifies_video_without_deep_track_enrichment() {
    let a = analyze_file_with_options(
        &media("matrix_ts_023.ts"),
        AnalyzeOptions {
            profile: AnalysisProfile::ContentProbe,
        },
    )
    .unwrap();
    assert_eq!(a.container_format.as_deref(), Some("mpegts"));
    assert_eq!(a.video_codec.as_deref(), Some("h264"));
    assert_eq!(a.video_width, None);
    assert_eq!(a.video_height, None);
    assert!(is_valid_video(&a));
}

#[test]
fn mkv_hevc_hdr10plus_ffprobe_parity_profile() {
    let a = analyze_file_with_options(
        &media("hevc_hdr10plus.mkv"),
        AnalyzeOptions {
            profile: AnalysisProfile::FfprobeParity,
        },
    )
    .unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_hdr_format, None);
    assert!(is_valid_video(&a));
}

#[test]
fn mp4_hevc_hdr10plus() {
    let a = analyze_file_with_options(
        &media("hevc_hdr10plus.mp4"),
        AnalyzeOptions {
            profile: AnalysisProfile::DefaultRich,
        },
    )
    .unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_hdr_format.as_deref(), Some("HDR10+"));
    assert!(is_valid_video(&a));
}

#[test]
fn mp4_hevc_hdr10plus_ffprobe_parity_profile() {
    let a = analyze_file_with_options(
        &media("hevc_hdr10plus.mp4"),
        AnalyzeOptions {
            profile: AnalysisProfile::FfprobeParity,
        },
    )
    .unwrap();
    assert_eq!(a.video_codec.as_deref(), Some("hevc"));
    assert_eq!(a.video_hdr_format, None);
    assert!(is_valid_video(&a));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn unsupported_extension_returns_error() {
    let err = analyze_file(&PathBuf::from("/tmp/fake.unsupported")).unwrap_err();
    assert!(err.to_string().contains("unsupported format"));
}
