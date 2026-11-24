// disk-guard: test, example and build-script code may call the filesystem directly.
#![allow(clippy::disallowed_methods)]

#[path = "support/details_facts.rs"]
mod facts;

use scryer_mediainfo::{AnalysisDetails, ProbeDepth, ProbeOptions, probe, probe_path};
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
        let analysis = scryer_mediainfo::probe(&media(name)).unwrap();
        assert_eq!(analysis.chapter_count, Some(2), "{name}");
        assert_eq!(analysis.chapters.len(), 2, "{name}: {:?}", analysis.report);
        assert_eq!(analysis.chapters[0].title.as_deref(), Some("Opening"));
        assert_eq!(analysis.chapters[1].title.as_deref(), Some("幕間"));
        assert_eq!(analysis.chapters[0].start_seconds, 0.0);
        assert_eq!(analysis.chapters[0].end_seconds, Some(0.5));
        assert_eq!(analysis.chapters[1].start_seconds, 0.5);
        assert_eq!(analysis.chapters[1].end_seconds, Some(1.0));
        assert!(
            facts::subtitles(&analysis).is_empty(),
            "chapter text must not become a subtitle track"
        );
        assert!(
            !analysis
                .report
                .warnings
                .iter()
                .any(|warning| warning.code.starts_with("mp4_chapter")),
            "{name}: {:?}",
            analysis.report
        );
    }
}

#[test]
fn program_stream_codec_headers_reach_the_stream_details() {
    for (name, codec) in [
        ("ps_mpeg1_mp2.mpg", "mpeg1video"),
        ("ps_mpeg2_mp2.vob", "mpeg2video"),
    ] {
        let analysis = scryer_mediainfo::probe(&media(name)).unwrap();
        assert_eq!(facts::video_codec(&analysis), Some(codec), "{name}");
        assert_eq!(facts::video_size(&analysis), (Some(160), Some(120)));
        let video = analysis
            .streams
            .iter()
            .find(|stream| stream.kind == scryer_media_model::StreamKind::Video)
            .unwrap();
        assert_eq!(video.metadata.id.as_deref(), Some("00e0"));
        assert_eq!(video.metadata.bitrate_bps, None);
        assert_eq!(facts::video_bitrate_kbps(&analysis), None);
        assert_eq!(video.metadata.bit_depth, Some(8));
        assert_eq!(video.metadata.pixel_format.as_deref(), Some("yuv420p"));
        assert_eq!(
            video.metadata.declared_frame_rate,
            scryer_media_model::Rational::new(25, 1)
        );
        assert_eq!(
            video.metadata.profile.as_deref(),
            (codec == "mpeg2video").then_some("Main")
        );
        assert_eq!(
            video.metadata.sample_aspect_ratio,
            scryer_media_model::Rational::new(1, 1)
        );
        let audio = analysis
            .streams
            .iter()
            .find(|stream| stream.kind == scryer_media_model::StreamKind::Audio)
            .unwrap();
        assert_eq!(
            audio.codec.as_ref().map(scryer_media_model::Codec::as_str),
            Some("mp2")
        );
        assert_eq!(audio.metadata.sample_rate, Some(48_000));
        assert_eq!(audio.metadata.channel_layout.as_deref(), Some("mono"));
        assert_eq!(
            analysis.report.status,
            scryer_media_model::ProbeStatus::Incomplete
        );
        assert!(analysis.report.bytes_read < 256 * 1024);
    }
}

#[test]
fn asf_timing_languages_and_stream_ids_reach_the_stream_details() {
    let analysis = scryer_mediainfo::probe(&media("wmv_wmv1_aac_surround.wmv")).unwrap();
    let video = analysis
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_model::StreamKind::Video)
        .unwrap();
    let audio = analysis
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_model::StreamKind::Audio)
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
        scryer_media_model::Provenance::Container
    );
}

#[test]
fn short_transport_stream_audio_retains_separate_bitrate_estimates() {
    for (name, expected) in [
        ("h264_aac.ts", 129_042),
        ("matrix_ts_001.ts", 4_970),
        ("simd_dense_h264_aac.ts", 4_924),
        ("matrix_ts_016.m2ts", 4_970),
    ] {
        let analysis = scryer_mediainfo::probe(&media(name)).unwrap();
        let audio = analysis
            .streams
            .iter()
            .find(|stream| {
                stream.codec.as_ref().map(scryer_media_model::Codec::as_str) == Some("aac")
            })
            .unwrap();
        assert_eq!(
            audio.metadata.estimated_bitrate_bps,
            Some(expected),
            "{name}"
        );
        assert!(audio.metadata.bitrate_bps.is_none());
        assert_eq!(
            audio.metadata.bitrate_provenance,
            scryer_media_model::Provenance::Unknown
        );
        assert!(
            facts::primary_audio(&analysis)
                .is_some_and(|primary| primary.metadata.bitrate_bps.is_none())
        );
    }
}

#[test]
fn flv_declared_frame_rates_do_not_claim_observed_timing() {
    for (name, fps) in [
        ("flv_flv1_video_only.flv", 24),
        ("flv_flv1_adpcm_swf.flv", 15),
        ("flv_flv1_aac_mono.flv", 25),
    ] {
        let analysis = scryer_mediainfo::probe(&media(name)).unwrap();
        let video = analysis
            .streams
            .iter()
            .find(|stream| stream.kind == scryer_media_model::StreamKind::Video)
            .unwrap();
        assert_eq!(
            video.metadata.declared_frame_rate,
            scryer_media_model::Rational::new(fps, 1),
            "{name}"
        );
        assert!(video.metadata.observed_frame_rate.is_none());
        assert!(video.metadata.variable_frame_rate.is_none());
        assert_eq!(video.metadata.bit_depth, Some(8));
        assert_eq!(video.metadata.pixel_format.as_deref(), Some("yuv420p"));
    }
}

#[test]
fn flv_audio_headers_reach_the_canonical_contract() {
    for (name, codec, rate, layout, depth) in [
        ("flv_flv1_adpcm_swf.flv", "adpcm_swf", 44_100, "mono", None),
        ("flv_flv1_mp3.flv", "mp3", 48_000, "stereo", None),
        (
            "flv_flv1_nellymoser.flv",
            "nellymoser",
            16_000,
            "mono",
            None,
        ),
        ("flv_flv1_pcm_mulaw.flv", "pcm_mulaw", 8_000, "mono", None),
        (
            "flv_flv1_pcm_s16le.flv",
            "pcm_s16le",
            44_100,
            "stereo",
            Some(16),
        ),
        ("flv_flv1_pcm_u8.flv", "pcm_u8", 44_100, "mono", Some(8)),
        ("flv_flv1_speex.flv", "speex", 16_000, "mono", None),
        ("flv_h264_mp3.flv", "mp3", 48_000, "mono", None),
        ("flv_h264_pcm_alaw.flv", "pcm_alaw", 8_000, "mono", None),
    ] {
        let analysis = scryer_mediainfo::probe(&media(name)).unwrap();
        let audio = analysis
            .streams
            .iter()
            .find(|stream| {
                stream.codec.as_ref().map(scryer_media_model::Codec::as_str) == Some(codec)
            })
            .unwrap();
        assert_eq!(audio.metadata.sample_rate, Some(rate), "{name}");
        assert_eq!(
            audio.metadata.channel_layout.as_deref(),
            Some(layout),
            "{name}"
        );
        assert_eq!(audio.metadata.sample_bit_depth, depth, "{name}");
    }
}

#[test]
fn mpeg_dts_and_extensible_aac_preserve_audio_header_properties() {
    for (name, codec, channels, layout) in [
        ("matrix_mkv_001.mkv", "mp3", 2, "stereo"),
        ("matrix_mkv_002.mkv", "mp3", 2, "stereo"),
        ("matrix_ts_005.ts", "dts", 6, "5.1(side)"),
        ("wmv_wmv1_aac_surround.wmv", "aac", 6, "5.1"),
    ] {
        let analysis = scryer_mediainfo::probe(&media(name)).unwrap();
        let stream = analysis
            .streams
            .iter()
            .find(|stream| {
                stream.codec.as_ref().map(scryer_media_model::Codec::as_str) == Some(codec)
            })
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
        let analysis = scryer_mediainfo::probe(&media(name)).unwrap();
        let stream = analysis
            .streams
            .iter()
            .find(|stream| stream.kind == scryer_media_model::StreamKind::Audio)
            .unwrap();
        assert_eq!(
            stream.codec.as_ref().map(scryer_media_model::Codec::as_str),
            Some(codec),
            "{name}"
        );
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
        let analysis = scryer_mediainfo::probe(&media(name)).unwrap();
        let track = analysis
            .streams
            .iter()
            .find(|stream| stream.kind == scryer_media_model::StreamKind::Audio)
            .unwrap();
        assert_eq!(
            track.codec.as_ref().map(scryer_media_model::Codec::as_str),
            Some(codec),
            "{name}"
        );
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
    let pcm = scryer_mediainfo::probe(&media("matrix_avi_002.avi")).unwrap();
    let track = pcm
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_model::StreamKind::Audio)
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
    let unassigned = scryer_mediainfo::probe_source(
        &mut std::io::Cursor::new(unassigned),
        "avi",
        &ProbeOptions::new().with_depth(ProbeDepth::Quick),
    )
    .unwrap();
    let track = unassigned
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_model::StreamKind::Audio)
        .unwrap();
    assert_eq!(track.channels, Some(6));
    assert!(
        track.metadata.channel_layout.is_none(),
        "six channels without an assigned speaker mask must stay unknown"
    );
    let aac = scryer_mediainfo::probe(&media("h264_aac.mkv")).unwrap();
    let track = aac
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_model::StreamKind::Audio)
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
        let analysis = scryer_mediainfo::probe(&media(name)).unwrap();
        assert_eq!(facts::video_codec(&analysis), Some("av1"), "{name}");
        assert_eq!(facts::video_profile(&analysis), Some("Main"), "{name}");
        assert_eq!(facts::video_bit_depth(&analysis), Some(10), "{name}");
        assert_eq!(facts::hdr_signals(&analysis), facts::HDR10, "{name}");
        assert!(
            (analysis.duration_seconds.unwrap() - 1.0).abs() < 0.01,
            "{name}"
        );
        let metadata = &analysis.streams[0].metadata;
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
            scryer_media_model::Provenance::Bitstream
        );
        assert_eq!(metadata.hdr.hdr10plus, None);
        assert!(
            !analysis
                .report
                .warnings
                .iter()
                .any(|warning| warning.code == "av1_enrichment_incomplete"),
            "{:?}",
            analysis.report
        );
    }
}

#[test]
fn hevc_sequence_and_sei_metadata_survive_mp4_and_mkv_projection() {
    for name in ["hevc_sequence_pq.mp4", "hevc_sequence_pq.mkv"] {
        let analysis = scryer_mediainfo::probe(&media(name)).unwrap();
        let metadata = &analysis.streams[0].metadata;
        assert_eq!(facts::video_profile(&analysis), Some("Main 10"), "{name}");
        assert_eq!(metadata.level, Some(30));
        assert_eq!(metadata.pixel_format.as_deref(), Some("yuv420p10le"));
        assert_eq!(metadata.color.transfer, Some(16));
        assert_eq!(metadata.color.full_range, Some(false));
        assert_eq!(metadata.field_order.as_deref(), Some("progressive"));
        assert_eq!(
            metadata.sample_aspect_ratio,
            scryer_media_model::Rational::new(1, 1)
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

fn analyze_fixture(fixture: &FixtureExpectation) -> AnalysisDetails {
    probe_path(
        &media(&fixture.name),
        &ProbeOptions::new().with_depth(ProbeDepth::Full),
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
            facts::video_codec(&analysis),
            Some(fixture.video_codec.as_str()),
            "{} video codec",
            fixture.name
        );
        let (width, height) = facts::video_size(&analysis);
        assert_eq!(width, Some(fixture.width), "{} video width", fixture.name);
        assert_eq!(
            height,
            Some(fixture.height),
            "{} video height",
            fixture.name
        );
        // The rate as a reader sees it: three decimals, so 23.9764 reads as 23.976.
        let actual_fps = facts::video_fps(&analysis)
            .map(|fps| (fps * 1000.0).round() / 1000.0)
            .unwrap_or_default();
        assert!(
            (actual_fps - f64::from(fixture.fps)).abs() < 0.001,
            "{} frame rate {:?} should equal {}",
            fixture.name,
            facts::video_fps(&analysis),
            fixture.fps
        );
        let duration = facts::duration_seconds(&analysis);
        assert!(
            duration.unwrap_or_default() >= fixture.min_duration_seconds,
            "{} duration {:?} should be at least {}",
            fixture.name,
            duration,
            fixture.min_duration_seconds
        );
        assert_eq!(
            facts::has_video_with_duration(&analysis),
            fixture.valid_video,
            "{} validity",
            fixture.name
        );

        let audio = facts::audio(&analysis);
        let actual_audio_codecs: Vec<_> = audio
            .iter()
            .filter_map(|stream| stream.codec.as_ref().map(ToString::to_string))
            .collect();
        assert_eq!(
            actual_audio_codecs, fixture.audio_codecs,
            "{} audio codecs",
            fixture.name
        );

        let actual_audio_channels: Vec<_> =
            audio.iter().filter_map(|stream| stream.channels).collect();
        assert_eq!(
            actual_audio_channels, fixture.audio_channels,
            "{} audio channels",
            fixture.name
        );

        assert_eq!(
            audio.len() > 1,
            fixture.audio_codecs.len() > 1,
            "{} multiaudio flag",
            fixture.name
        );
        assert_eq!(
            facts::audio_languages(&analysis),
            fixture.audio_languages,
            "{} audio languages",
            fixture.name
        );
        assert_eq!(
            facts::subtitles(&analysis).len(),
            fixture.subtitle_stream_count,
            "{} subtitle stream count",
            fixture.name
        );
    }
}

// ---------------------------------------------------------------------------
// Dolby Vision (MKV)
// ---------------------------------------------------------------------------

/// Profile, level, compatibility id and the RPU / enhancement-layer / base-layer flags
/// reported for the primary video stream.
fn mkv_dovi_record(name: &str) -> (u8, u8, u8, bool, bool, bool) {
    let a = probe(&media(name)).unwrap();
    assert_eq!(facts::video_codec(&a), Some("hevc"));
    assert!(facts::hdr_signals(&a)[0], "Dolby Vision signal");
    assert!(facts::has_video_with_duration(&a));
    let dovi = a.streams[0].metadata.hdr.dovi.clone().unwrap();
    assert_eq!(
        (dovi.profile, dovi.base_layer_compatibility_id),
        facts::dovi(&a),
        "the record belongs to the selected video"
    );
    (
        dovi.profile.unwrap(),
        dovi.level.unwrap(),
        dovi.base_layer_compatibility_id.unwrap(),
        dovi.rpu_present.unwrap(),
        dovi.enhancement_layer_present.unwrap(),
        dovi.base_layer_present.unwrap(),
    )
}

#[test]
fn mkv_dv_profile5() {
    assert_eq!(
        mkv_dovi_record("dv_profile5.mkv"),
        (5, 6, 0, true, false, true)
    );
}

#[test]
fn mkv_dv_profile7() {
    assert_eq!(
        mkv_dovi_record("dv_profile7.mkv"),
        (7, 6, 6, true, true, true)
    );
}

#[test]
fn mkv_dv_profile8() {
    assert_eq!(
        mkv_dovi_record("dv_profile8.mkv"),
        (8, 6, 1, true, false, true)
    );
}

// ---------------------------------------------------------------------------
// Dolby Vision (MP4)
// ---------------------------------------------------------------------------

#[test]
fn mp4_dv_profile7() {
    let a = probe(&media("dv_profile7.mp4")).unwrap();
    assert!(facts::hdr_signals(&a)[0], "Dolby Vision signal");
    assert_eq!(facts::dovi(&a), (Some(7), Some(6)));
    assert!(facts::has_video_with_duration(&a));
}

#[test]
fn mp4_dv_profile8() {
    let a = probe(&media("dv_profile8.mp4")).unwrap();
    assert!(facts::hdr_signals(&a)[0], "Dolby Vision signal");
    assert_eq!(facts::dovi(&a), (Some(8), Some(1)));
    assert!(facts::has_video_with_duration(&a));
}

// ---------------------------------------------------------------------------
// Emerging MKV metadata
// ---------------------------------------------------------------------------

#[test]
fn mkv_h264_8k_fixture_reports_dimensions() {
    let a = probe(&media("h264_8k_aac.mkv")).unwrap();
    assert_eq!(a.container_format.as_deref(), Some("matroska"));
    assert_eq!(facts::video_codec(&a), Some("h264"));
    assert_eq!(facts::video_size(&a), (Some(7680), Some(4320)));
    assert!(facts::has_video_with_duration(&a));
}

// ---------------------------------------------------------------------------
// HDR10+ (MKV + MP4)
// ---------------------------------------------------------------------------

#[test]
fn mkv_hevc_hdr10plus() {
    let a = probe_path(
        &media("hevc_hdr10plus.mkv"),
        &ProbeOptions::new().with_depth(ProbeDepth::Full),
    )
    .unwrap();
    assert_eq!(facts::video_codec(&a), Some("hevc"));
    assert!(facts::hdr_signals(&a)[1], "HDR10+ signal");
    assert!(!facts::hdr_signals(&a)[0], "no Dolby Vision signal");
    assert_eq!(facts::video_bit_depth(&a), Some(10));
    assert!(facts::has_video_with_duration(&a));
}

#[test]
fn mkv_hevc_hdr10plus_content_probe_profile() {
    let a = probe_path(
        &media("hevc_hdr10plus.mkv"),
        &ProbeOptions::new().with_depth(ProbeDepth::Identify),
    )
    .unwrap();
    assert_eq!(facts::video_codec(&a), Some("hevc"));
    assert_eq!(facts::hdr_signals(&a), facts::NO_HDR);
    assert!(facts::has_video_with_duration(&a));
}

#[test]
fn mpegts_content_probe_profile_identifies_video_without_deep_track_enrichment() {
    let a = probe_path(
        &media("matrix_ts_023.ts"),
        &ProbeOptions::new().with_depth(ProbeDepth::Identify),
    )
    .unwrap();
    assert_eq!(a.container_format.as_deref(), Some("mpegts"));
    assert_eq!(facts::video_codec(&a), Some("h264"));
    assert_eq!(facts::video_size(&a), (None, None));
    assert!(facts::has_video_with_duration(&a));
    assert_eq!(
        a.duration_provenance,
        scryer_media_model::Provenance::Estimated
    );
    assert_eq!(a.report.status, scryer_media_model::ProbeStatus::Incomplete);
    assert!(!a.report.budget_exhausted);
    assert!(
        a.report
            .warnings
            .iter()
            .any(|warning| warning.code == "content_probe_duration_estimate")
    );
}

#[test]
fn mkv_hevc_hdr10plus_ffprobe_parity_profile() {
    let a = probe_path(
        &media("hevc_hdr10plus.mkv"),
        &ProbeOptions::new().with_depth(ProbeDepth::FfprobeParity),
    )
    .unwrap();
    assert_eq!(facts::video_codec(&a), Some("hevc"));
    assert!(
        facts::hdr_signals(&a)[1] && !facts::hdr_signals(&a)[0],
        "HDR10+ signal"
    );
    assert!(facts::has_video_with_duration(&a));
}

#[test]
fn mp4_hevc_hdr10plus() {
    let a = probe_path(
        &media("hevc_hdr10plus.mp4"),
        &ProbeOptions::new().with_depth(ProbeDepth::Full),
    )
    .unwrap();
    assert_eq!(facts::video_codec(&a), Some("hevc"));
    assert!(
        facts::hdr_signals(&a)[1] && !facts::hdr_signals(&a)[0],
        "HDR10+ signal"
    );
    assert!(facts::has_video_with_duration(&a));
}

#[test]
fn mp4_hevc_hdr10plus_ffprobe_parity_profile() {
    let a = probe_path(
        &media("hevc_hdr10plus.mp4"),
        &ProbeOptions::new().with_depth(ProbeDepth::FfprobeParity),
    )
    .unwrap();
    assert_eq!(facts::video_codec(&a), Some("hevc"));
    assert_eq!(facts::hdr_signals(&a), facts::NO_HDR);
    assert!(facts::has_video_with_duration(&a));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn unsupported_extension_returns_error() {
    let err = probe(&PathBuf::from("/tmp/fake.unsupported")).unwrap_err();
    assert!(
        matches!(err, scryer_mediainfo::ProbeError::Unsupported(ref ext) if ext == "unsupported"),
        "{err:?}"
    );
}

/// A reader over a file stored as two swapped halves with filler between
/// them, so that every read crossing the middle is split and remapped.
struct SwappedHalves {
    physical: Vec<u8>,
    split: usize,
    tail_len: usize,
    position: u64,
}

impl SwappedHalves {
    fn new(bytes: &[u8]) -> Self {
        let split = bytes.len() / 2;
        let mut physical = Vec::with_capacity(bytes.len() + 37);
        physical.extend_from_slice(&bytes[split..]);
        physical.extend_from_slice(&[0xaa; 37]);
        physical.extend_from_slice(&bytes[..split]);
        Self {
            physical,
            split,
            tail_len: bytes.len() - split,
            position: 0,
        }
    }

    fn len(&self) -> u64 {
        (self.split + self.tail_len) as u64
    }
}

impl std::io::Read for SwappedHalves {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let Ok(position) = usize::try_from(self.position) else {
            return Ok(0);
        };
        let (start, available) = if position < self.split {
            (self.tail_len + 37 + position, self.split - position)
        } else if position < self.split + self.tail_len {
            (position - self.split, self.split + self.tail_len - position)
        } else {
            return Ok(0);
        };
        let count = available.min(bytes.len());
        bytes[..count].copy_from_slice(&self.physical[start..start + count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl std::io::Seek for SwappedHalves {
    fn seek(&mut self, offset: std::io::SeekFrom) -> std::io::Result<u64> {
        let next = match offset {
            std::io::SeekFrom::Start(n) => i128::from(n),
            std::io::SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            std::io::SeekFrom::End(n) => i128::from(self.len()) + i128::from(n),
        };
        self.position = u64::try_from(next).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before start")
        })?;
        Ok(self.position)
    }
}

#[test]
fn path_and_caller_reader_sources_have_identical_facts() {
    const BUDGET: u64 = 32 * 1024 * 1024;
    let manifest = fixture_manifest();
    for extension in ["mkv", "mp4", "avi", "ts", "wmv", "ogv", "flv"] {
        let fixture = manifest
            .fixtures
            .iter()
            .find(|fixture| fixture.name.ends_with(&format!(".{extension}")))
            .unwrap();
        let path = media(&fixture.name);
        let bytes = std::fs::read(&path).unwrap();
        let mut reader = SwappedHalves::new(&bytes);
        let mut actual = scryer_mediainfo::probe_source(
            &mut reader,
            extension,
            &ProbeOptions::new().with_read_budget_bytes(BUDGET),
        )
        .unwrap();
        let mut expected = probe(&path).unwrap();
        assert!(actual.report.bytes_read > 0);
        assert!(actual.report.bytes_read < BUDGET);
        // Split reads change the number of underlying reads and seeks only.
        actual.report = Default::default();
        expected.report = Default::default();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "{}",
            fixture.name
        );
    }
}
