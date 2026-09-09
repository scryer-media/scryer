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

#[test]
fn mp4_chapter_lists_and_referenced_text_tracks_preserve_authored_titles() {
    for name in ["chapters_nero.mp4", "chapters_quicktime.mp4"] {
        let analysis = scryer_mediainfo::analyze_catalog_file(&media(name)).unwrap();
        assert_eq!(analysis.num_chapters, Some(2), "{name}");
        assert_eq!(
            analysis.details.chapters.len(),
            2,
            "{name}: {:?}",
            analysis.details.report
        );
        assert_eq!(
            analysis.details.chapters[0].title.as_deref(),
            Some("Opening")
        );
        assert_eq!(analysis.details.chapters[1].title.as_deref(), Some("幕間"));
        assert_eq!(analysis.details.chapters[0].start_seconds, 0.0);
        assert_eq!(analysis.details.chapters[0].end_seconds, Some(0.5));
        assert_eq!(analysis.details.chapters[1].start_seconds, 0.5);
        assert_eq!(analysis.details.chapters[1].end_seconds, Some(1.0));
        assert!(
            analysis.subtitle_streams.is_empty(),
            "chapter text must not become a subtitle track"
        );
        assert!(
            !analysis
                .details
                .report
                .warnings
                .iter()
                .any(|warning| warning.code.starts_with("mp4_chapter")),
            "{name}: {:?}",
            analysis.details.report
        );
    }
}

#[test]
fn asf_timing_languages_and_stream_ids_reach_the_catalog_contract() {
    let analysis =
        scryer_mediainfo::analyze_catalog_file(&media("wmv_wmv1_aac_surround.wmv")).unwrap();
    let video = analysis
        .details
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_types::StreamKind::Video)
        .unwrap();
    let audio = analysis
        .details
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_types::StreamKind::Audio)
        .unwrap();
    assert_eq!(video.metadata.id.as_deref(), Some("1"));
    assert_eq!(audio.metadata.id.as_deref(), Some("2"));
    let observed = video.metadata.observed_frame_rate.unwrap();
    assert!((observed.numerator as f64 / observed.denominator as f64 - 12.0).abs() < 0.05);
    assert_eq!(video.metadata.variable_frame_rate, Some(false));
    assert_eq!(audio.language.as_deref(), Some("spa"));
    assert!(audio.metadata.original_language.is_some());
    assert_eq!(
        audio.metadata.language_provenance,
        scryer_media_types::Provenance::Container
    );
}

#[test]
fn mpeg_dts_and_extensible_aac_preserve_audio_header_properties() {
    for (name, codec, channels, layout) in [
        ("matrix_mkv_001.mkv", "mp3", 2, "stereo"),
        ("matrix_mkv_002.mkv", "mp3", 2, "stereo"),
        ("matrix_ts_005.ts", "dts", 6, "5.1(side)"),
        ("wmv_wmv1_aac_surround.wmv", "aac", 6, "5.1"),
    ] {
        let analysis = scryer_mediainfo::analyze_catalog_file(&media(name)).unwrap();
        let stream = analysis
            .details
            .streams
            .iter()
            .find(|stream| stream.codec.as_deref() == Some(codec))
            .unwrap();
        assert_eq!(stream.channels, Some(channels), "{name}");
        assert_eq!(stream.metadata.sample_rate, Some(48_000), "{name}");
        assert_eq!(
            stream.metadata.channel_layout.as_deref(),
            Some(layout),
            "{name}"
        );
        if codec == "mp3" {
            assert_eq!(
                stream.metadata.estimated_bitrate_bps,
                Some(64_000),
                "{name}"
            );
        }
    }
}

#[test]
fn opus_vorbis_and_flac_headers_reach_the_canonical_contract() {
    for (name, codec, channels, layout) in [
        ("matrix_mkv_003.mkv", "flac", 1, "mono"),
        ("matrix_mkv_007.mkv", "vorbis", 2, "stereo"),
        ("matrix_mkv_020.mkv", "opus", 6, "5.1"),
        ("matrix_mp4_008.m4v", "opus", 6, "5.1"),
        ("ogv_theora_opus_surround.ogv", "opus", 6, "5.1"),
        ("ogv_theora_vorbis.ogv", "vorbis", 2, "stereo"),
    ] {
        let analysis = scryer_mediainfo::analyze_catalog_file(&media(name)).unwrap();
        let stream = analysis
            .details
            .streams
            .iter()
            .find(|stream| stream.kind == scryer_media_types::StreamKind::Audio)
            .unwrap();
        assert_eq!(stream.codec.as_deref(), Some(codec), "{name}");
        assert_eq!(stream.channels, Some(channels), "{name}");
        assert_eq!(stream.metadata.sample_rate, Some(48_000), "{name}");
        assert_eq!(
            stream.metadata.channel_layout.as_deref(),
            Some(layout),
            "{name}"
        );
        assert!(
            stream.metadata.sample_format.is_none(),
            "{name}: compressed audio has no prescribed decoder representation"
        );
        if codec == "flac" {
            assert_eq!(stream.metadata.sample_bit_depth, Some(16), "{name}");
        }
    }
}

#[test]
fn dolby_audio_properties_survive_mkv_mp4_and_transport_streams() {
    for (name, codec, channels, layout) in [
        ("matrix_mkv_004.mkv", "ac3", 2, "stereo"),
        ("matrix_mkv_005.mkv", "eac3", 6, "5.1(side)"),
        ("matrix_mp4_002.m4v", "ac3", 6, "5.1(side)"),
        ("matrix_mp4_011.m4v", "eac3", 6, "5.1(side)"),
        ("matrix_ts_003.ts", "ac3", 1, "mono"),
        ("matrix_ts_004.m2ts", "eac3", 2, "stereo"),
    ] {
        let analysis = scryer_mediainfo::analyze_catalog_file(&media(name)).unwrap();
        let track = analysis
            .details
            .streams
            .iter()
            .find(|stream| stream.kind == scryer_media_types::StreamKind::Audio)
            .unwrap();
        assert_eq!(track.codec.as_deref(), Some(codec), "{name}");
        assert_eq!(track.channels, Some(channels), "{name}");
        assert_eq!(track.metadata.sample_rate, Some(48_000), "{name}");
        assert_eq!(
            track.metadata.channel_layout.as_deref(),
            Some(layout),
            "{name}"
        );
        if codec == "ac3" {
            assert_eq!(track.metadata.bitrate_bps, Some(96_000), "{name}");
        } else if name.starts_with("matrix_ts") {
            assert!(
                track.metadata.bitrate_bps.is_none(),
                "a single E-AC-3 frame cannot establish the stream average"
            );
            assert_eq!(track.metadata.estimated_bitrate_bps, Some(96_000));
        }
    }
}

#[test]
fn encoded_audio_properties_do_not_invent_decoder_formats_or_surround_layouts() {
    let pcm = scryer_mediainfo::analyze_catalog_file(&media("matrix_avi_002.avi")).unwrap();
    let track = pcm
        .details
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_types::StreamKind::Audio)
        .unwrap();
    assert_eq!(track.channels, Some(6));
    assert_eq!(track.metadata.sample_format.as_deref(), Some("s16le"));
    assert_eq!(track.metadata.sample_bit_depth, Some(16));
    assert_eq!(track.metadata.sample_rate, Some(48_000));
    assert_eq!(
        track.metadata.channel_layout.as_deref(),
        Some("5.1"),
        "the authored WAVEFORMATEXTENSIBLE mask is 0x3f"
    );
    let mut unassigned = std::fs::read(media("matrix_avi_002.avi")).unwrap();
    // This authored fixture's 40-byte WAVEFORMATEXTENSIBLE starts at byte 4500.
    assert_eq!(&unassigned[4492..4504], b"strf\x28\0\0\0\xfe\xff\x06\0");
    unassigned[4520..4524].fill(0);
    let unassigned = scryer_mediainfo::analyze_source(
        &mut std::io::Cursor::new(unassigned),
        "avi",
        AnalyzeOptions::default(),
    )
    .unwrap();
    let track = unassigned
        .details
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_types::StreamKind::Audio)
        .unwrap();
    assert_eq!(track.channels, Some(6));
    assert!(
        track.metadata.channel_layout.is_none(),
        "six channels without an assigned speaker mask must stay unknown"
    );
    let aac = scryer_mediainfo::analyze_catalog_file(&media("h264_aac.mkv")).unwrap();
    let track = aac
        .details
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_types::StreamKind::Audio)
        .unwrap();
    assert_eq!(track.metadata.channel_layout.as_deref(), Some("stereo"));
    assert!(
        track.metadata.sample_format.is_none(),
        "compressed AAC does not prescribe a floating-point decoder output"
    );
    assert_eq!(
        track.metadata.sample_bit_depth,
        Some(32),
        "the explicit Matroska bit-depth declaration is retained"
    );
}

#[test]
fn av1_sequence_color_matches_native_expectations_in_mp4_and_mkv() {
    // Authored black 64x64/24 fps, SVT-AV1 4.1.0, Main 10-bit with explicit
    // sequence primaries=9, transfer=16, matrix=9; remuxed without re-encoding.
    for name in ["av1_sequence_pq.mp4", "av1_sequence_pq.mkv"] {
        let analysis = scryer_mediainfo::analyze_catalog_file(&media(name)).unwrap();
        assert_eq!(analysis.video_codec.as_deref(), Some("av1"), "{name}");
        assert_eq!(analysis.video_profile.as_deref(), Some("Main"), "{name}");
        assert_eq!(analysis.video_bit_depth, Some(10), "{name}");
        assert_eq!(
            analysis.video_hdr_format.as_deref(),
            Some("HDR10"),
            "{name}"
        );
        assert!(
            (analysis.details.duration_seconds.unwrap() - 1.0).abs() < 0.01,
            "{name}"
        );
        let metadata = &analysis.details.streams[0].metadata;
        assert_eq!(
            metadata.pixel_format.as_deref(),
            Some("yuv420p10le"),
            "{name}"
        );
        assert_eq!(
            (
                metadata.color.primaries,
                metadata.color.transfer,
                metadata.color.matrix
            ),
            (Some(9), Some(16), Some(9)),
            "{name}"
        );
        assert_eq!(
            metadata.color.provenance,
            scryer_media_types::Provenance::Bitstream
        );
        assert_eq!(metadata.hdr.hdr10plus, None);
        assert!(
            !analysis
                .details
                .report
                .warnings
                .iter()
                .any(|warning| warning.code == "av1_enrichment_incomplete"),
            "{:?}",
            analysis.details.report
        );
    }
}

#[test]
fn hevc_sequence_and_sei_metadata_survive_mp4_and_mkv_projection() {
    for name in ["hevc_sequence_pq.mp4", "hevc_sequence_pq.mkv"] {
        let analysis = scryer_mediainfo::analyze_catalog_file(&media(name)).unwrap();
        let metadata = &analysis.details.streams[0].metadata;
        assert_eq!(analysis.video_profile.as_deref(), Some("Main 10"), "{name}");
        assert_eq!(metadata.level, Some(30));
        assert_eq!(metadata.pixel_format.as_deref(), Some("yuv420p10le"));
        assert_eq!(metadata.color.transfer, Some(16));
        assert_eq!(metadata.color.full_range, Some(false));
        assert_eq!(metadata.field_order.as_deref(), Some("progressive"));
        assert_eq!(
            metadata.sample_aspect_ratio,
            scryer_media_types::Rational::new(1, 1)
        );
        let mastering = metadata.color.mastering_display.as_ref().expect(name);
        assert_eq!(mastering.red_x, Some(0.68));
        assert_eq!(mastering.green_y, Some(0.69));
        assert_eq!(mastering.max_luminance, Some(1000.0));
        assert_eq!(mastering.min_luminance, Some(0.005));
        let light = metadata.color.content_light.as_ref().expect(name);
        assert_eq!(light.max_cll, Some(1000));
        assert_eq!(light.max_fall, Some(400));
        assert_eq!(metadata.hdr.hdr10, Some(true));
    }
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
    assert_eq!(
        a.details.duration_provenance,
        scryer_media_types::Provenance::Estimated
    );
    assert_eq!(
        a.details.report.status,
        scryer_media_types::ProbeStatus::Incomplete
    );
    assert!(!a.details.report.budget_exhausted);
    assert!(
        a.details
            .report
            .warnings
            .iter()
            .any(|warning| warning.code == "content_probe_duration_estimate")
    );
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

#[test]
fn canonical_path_and_fragmented_sources_have_identical_facts() {
    use scryer_mediainfo::source::{BoundedSource, Extent, ExtentSource};
    use std::io::Cursor;
    let manifest = fixture_manifest();
    for extension in ["mkv", "mp4", "avi", "ts", "wmv", "ogv", "flv"] {
        let fixture = manifest
            .fixtures
            .iter()
            .find(|fixture| fixture.name.ends_with(&format!(".{extension}")))
            .unwrap();
        let path = media(&fixture.name);
        let bytes = std::fs::read(&path).unwrap();
        let split = bytes.len() / 2;
        let mut physical = Vec::new();
        physical.extend_from_slice(&bytes[split..]);
        physical.extend_from_slice(&[0xaa; 37]);
        physical.extend_from_slice(&bytes[..split]);
        let mut physical = BoundedSource::new(Cursor::new(physical), 32 * 1024 * 1024);
        let mut logical = ExtentSource::new(
            &mut physical,
            vec![
                Extent {
                    offset: (bytes.len() - split + 37) as u64,
                    length: split as u64,
                },
                Extent {
                    offset: 0,
                    length: (bytes.len() - split) as u64,
                },
            ],
        )
        .unwrap();
        let mut actual = scryer_mediainfo::analyze_source(
            &mut logical,
            extension,
            AnalyzeOptions {
                profile: AnalysisProfile::DefaultRich,
            },
        )
        .unwrap();
        let mut expected = analyze_file(&path).unwrap();
        assert!(actual.details.report.bytes_read > 0);
        assert!(actual.details.report.bytes_read < 32 * 1024 * 1024);
        // Extent boundaries change the number of underlying reads and seeks only.
        actual.details.report = Default::default();
        expected.details.report = Default::default();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "{}",
            fixture.name
        );
    }
}
