use super::*;
use crate::ProbeError;
use scryer_media_model::Codec;
use std::io::Cursor;

fn make_ebml_size(size: u64) -> Vec<u8> {
    for len in 1..=8 {
        let value_bits = len * 7;
        let unknown_size = 1_u64 << value_bits;
        if size < unknown_size - 1 {
            let mut bytes = vec![0_u8; len];
            let mut value = size;
            for byte in bytes.iter_mut().rev() {
                *byte = (value & 0xFF) as u8;
                value >>= 8;
            }
            bytes[0] |= 1 << (8 - len);
            return bytes;
        }
    }

    panic!("EBML test size is too large");
}

fn make_ebml_element(id: &[u8], payload: &[u8]) -> Vec<u8> {
    make_ebml_element_with_declared_size(id, payload.len() as u64, payload)
}

#[test]
fn stereo_mode_is_detected_from_video_track_metadata() {
    for (mode, expected) in [
        (0, false),
        (1, true),
        (3, true),
        (14, true),
        (15, false),
        (255, false),
    ] {
        let video = make_ebml_element(&[0xe0], &make_ebml_element(&[0x53, 0xb8], &[mode]));
        let mut entry = make_ebml_element(&[0xd7], &[1]);
        entry.extend(make_ebml_element(&[0x83], &[1]));
        entry.extend(make_ebml_element(&[0x86], b"V_MPEG4/ISO/AVC"));
        entry.extend(video);
        let track = parse_mkv_track_entry(&entry).unwrap();
        assert_eq!(track.raw.metadata.is_3d, expected, "mode {mode}");
    }
    let truncated = make_ebml_element(&[0xe0], &make_ebml_element(&[0x53, 0xb8], &[]));
    assert!(!mkv_stream_metadata(&truncated, 1).is_3d);
    assert!(!mkv_stream_metadata(&[], 1).is_3d);
}

fn make_ebml_element_with_declared_size(
    id: &[u8],
    declared_size: u64,
    payload_prefix: &[u8],
) -> Vec<u8> {
    let size = make_ebml_size(declared_size);
    let mut element = Vec::with_capacity(id.len() + size.len() + payload_prefix.len());
    element.extend_from_slice(id);
    element.extend_from_slice(&size);
    element.extend_from_slice(payload_prefix);
    element
}

fn make_chapter_atom(start: u64, nested: &[u8]) -> Vec<u8> {
    let mut payload = make_ebml_element(&[0x73, 0xC4], &[1]);
    payload.extend_from_slice(&make_ebml_element(&[0x91], &start.to_be_bytes()));
    payload.extend_from_slice(nested);
    make_ebml_element(&[0xB6], &payload)
}

#[test]
fn track_flags_fill_only_an_unreadable_disposition() {
    let entry = |flags: &[Vec<u8>]| {
        let mut entry = make_ebml_element(&[0xd7], &[1]);
        entry.extend(make_ebml_element(&[0x83], &[2]));
        entry.extend(make_ebml_element(&[0x86], b"A_AAC"));
        for flag in flags {
            entry.extend_from_slice(flag);
        }
        parse_mkv_track_entry(&entry)
            .unwrap()
            .raw
            .metadata
            .disposition
    };
    let unreadable = |id: &[u8]| make_ebml_element(id, &[1; 9]);

    let absent = entry(&[]);
    assert_eq!((absent.default, absent.forced), (Some(true), Some(false)));
    let unreadable_default = entry(&[unreadable(&[0x88])]);
    assert_eq!(unreadable_default.default, Some(true));
    let explicit = entry(&[
        make_ebml_element(&[0x88], &[0]),
        make_ebml_element(&[0x55, 0xaa], &[1]),
    ]);
    assert_eq!(
        (explicit.default, explicit.forced),
        (Some(false), Some(true))
    );
    let unreadable_forced = entry(&[unreadable(&[0x55, 0xaa])]);
    assert_eq!(unreadable_forced.forced, None);
}

#[test]
fn metadata_defaults_and_cropped_display_ratios_are_explicit() {
    let video = |extra: Vec<u8>| {
        let mut payload = make_uint_element(&[0xb0], 720);
        payload.extend(make_uint_element(&[0xba], 480));
        payload.extend(extra);
        mkv_stream_metadata(&make_ebml_element(&[0xe0], &payload), 1)
    };
    let default = video(Vec::new());
    assert_eq!(default.disposition.default, Some(true));
    assert_eq!(default.disposition.forced, Some(false));
    assert_eq!(default.display_aspect_ratio.unwrap().as_f64(), Some(1.5));
    assert_eq!(default.sample_aspect_ratio.unwrap().as_f64(), Some(1.0));

    let mut extra = make_uint_element(&[0x54, 0xcc], 8);
    extra.extend(make_uint_element(&[0x54, 0xdd], 8));
    extra.extend(make_uint_element(&[0x54, 0xb2], 3));
    extra.extend(make_uint_element(&[0x54, 0xb0], 4));
    extra.extend(make_uint_element(&[0x54, 0xba], 3));
    let cropped = video(extra);
    assert_eq!(
        cropped.display_aspect_ratio.unwrap().as_f64(),
        Some(4.0 / 3.0)
    );
    assert_eq!(
        cropped.sample_aspect_ratio.unwrap().as_f64(),
        Some(10.0 / 11.0)
    );

    for extra in [
        make_uint_element(&[0x54, 0xcc], 721),
        make_uint_element(&[0x54, 0xb2], 4),
        make_uint_element(&[0x54, 0xb2], 1),
    ] {
        let unknown = video(extra);
        assert!(unknown.display_aspect_ratio.is_none());
        assert!(unknown.sample_aspect_ratio.is_none());
    }
    let explicit = mkv_stream_metadata(&make_uint_element(&[0x88], 0), 1);
    assert_eq!(explicit.disposition.default, Some(false));
}

fn make_uint_element(id: &[u8], value: u64) -> Vec<u8> {
    let bytes = if value <= u64::from(u8::MAX) {
        vec![value as u8]
    } else {
        value
            .to_be_bytes()
            .into_iter()
            .skip_while(|byte| *byte == 0)
            .collect()
    };
    make_ebml_element(id, &bytes)
}

fn make_simple_block(track_num: u8, relative_timestamp: i16, flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut block = Vec::with_capacity(4 + payload.len());
    block.push(0x80 | track_num);
    block.extend_from_slice(&relative_timestamp.to_be_bytes());
    block.push(flags);
    block.extend_from_slice(payload);
    make_ebml_element(&[0xA3], &block)
}

fn audio_test_track(codec_name: &str, channels: Option<i32>) -> RawTrack {
    RawTrack {
        metadata: Default::default(),
        kind: TrackKind::Audio,
        codec_id: format!("A_{}", codec_name.to_ascii_uppercase()),
        codec_name: Some(codec_name.into()),
        audio_profile: None,
        codec_private: None,
        width: None,
        height: None,
        channels,
        bit_rate_bps: None,
        language: None,
        name: None,
        frame_rate_fps: None,
        color_transfer: None,
        dovi_config: None,
        has_hdr10plus: false,
    }
}

fn make_frame_rate_probe_cluster() -> Vec<u8> {
    make_ebml_element(
        &[0x1F, 0x43, 0xB6, 0x75],
        &[
            make_uint_element(&[0xE7], 0),
            make_simple_block(1, 0, 0, &[0x00]),
            make_simple_block(1, 40, 0, &[0x01]),
            make_simple_block(1, 80, 0, &[0x02]),
            make_simple_block(1, 120, 0, &[0x03]),
            make_simple_block(1, 160, 0, &[0x04]),
        ]
        .concat(),
    )
}

fn wrap_in_segments(mut payload: Vec<u8>, count: usize) -> Vec<u8> {
    for _ in 0..count {
        payload = make_ebml_element(&[0x18, 0x53, 0x80, 0x67], &payload);
    }
    payload
}

fn make_mkv_with_declared_info_size(declared_size: u64) -> Vec<u8> {
    let info = make_ebml_element_with_declared_size(&[0x15, 0x49, 0xA9, 0x66], declared_size, &[]);
    let segment_declared_size = declared_size + info.len() as u64;
    [
        make_ebml_element(&[0x1A, 0x45, 0xDF, 0xA3], &[]),
        make_ebml_element_with_declared_size(
            &[0x18, 0x53, 0x80, 0x67],
            segment_declared_size,
            &info,
        ),
    ]
    .concat()
}

fn unique_temp_mkv_path(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("scryer-{name}-{}-{nanos}.mkv", std::process::id()))
}

#[test]
fn ebml_vint_parsing() {
    // 0x81 = 1 byte VINT, value 1
    assert_eq!(parse_ebml_vint(&[0x81]), Some((1, 1)));
    // 0x82 = 1 byte VINT, value 2
    assert_eq!(parse_ebml_vint(&[0x82]), Some((2, 1)));
    // 0x85 = 1 byte VINT, value 5
    assert_eq!(parse_ebml_vint(&[0x85]), Some((5, 1)));
    // 0x40 0x18 = 2 byte VINT, value 24
    assert_eq!(parse_ebml_vint(&[0x40, 0x18]), Some((24, 2)));
}

#[test]
fn ebml_signed_vint_parsing_matches_matroska_lace_deltas() {
    assert_eq!(parse_ebml_sint_value(&[0xBE]), Some((-1, 1)));
    assert_eq!(parse_ebml_sint_value(&[0xBF]), Some((0, 1)));
    assert_eq!(parse_ebml_sint_value(&[0xC0]), Some((1, 1)));
}

#[test]
fn first_laced_frame_parser_handles_non_laced_payloads() {
    let mut cursor = Cursor::new(vec![1, 2, 3, 4]);
    assert_eq!(
        parse_first_laced_frame_from_reader(&mut cursor, 4, 0).unwrap(),
        Some((0, 4))
    );
}

#[test]
fn first_laced_frame_parser_handles_xiph_lacing() {
    let mut payload = vec![0x02, 0x04, 0x05];
    payload.extend_from_slice(&[0_u8; 15]);
    let mut cursor = Cursor::new(payload);
    assert_eq!(
        parse_first_laced_frame_from_reader(&mut cursor, 18, 1).unwrap(),
        Some((3, 4))
    );
}

#[test]
fn first_laced_frame_parser_handles_fixed_lacing() {
    let mut payload = vec![0x02];
    payload.extend_from_slice(&[0_u8; 15]);
    let mut cursor = Cursor::new(payload);
    assert_eq!(
        parse_first_laced_frame_from_reader(&mut cursor, 16, 2).unwrap(),
        Some((1, 5))
    );
}

#[test]
fn first_laced_frame_parser_handles_ebml_lacing() {
    let mut payload = vec![0x02, 0x84, 0xC0];
    payload.extend_from_slice(&[0_u8; 15]);
    let mut cursor = Cursor::new(payload);
    assert_eq!(
        parse_first_laced_frame_from_reader(&mut cursor, 18, 3).unwrap(),
        Some((3, 4))
    );
}

#[test]
fn normalize_track_language_prefers_explicit_modern_tags_and_keeps_missing_unknown() {
    assert_eq!(
        normalize_mkv_track_language(TrackKind::Video, None, None),
        None
    );
    assert_eq!(
        normalize_mkv_track_language(TrackKind::Audio, Some("en-US"), None),
        Some("en-US".to_string())
    );
    assert_eq!(
        normalize_mkv_track_language(TrackKind::Subtitle, None, Some("en-US")),
        Some("en-US".to_string())
    );
    assert_eq!(
        normalize_mkv_track_language(TrackKind::Subtitle, Some("pt-BR"), Some("por")),
        Some("pt-BR".to_string())
    );
    assert_eq!(
        normalize_mkv_track_language(TrackKind::Subtitle, Some("fil"), Some("fil")),
        Some("fil".to_string())
    );
    assert_eq!(
        normalize_mkv_track_language(TrackKind::Subtitle, Some("jad"), Some("und")),
        Some("jad".to_string())
    );
    assert_eq!(
        normalize_mkv_track_language(TrackKind::Subtitle, None, Some("zxx")),
        Some("zxx".to_string())
    );
    assert_eq!(
        normalize_mkv_track_language(TrackKind::Audio, None, Some("???")),
        Some("???".to_string())
    );
    assert_eq!(
        normalize_mkv_track_language(TrackKind::Audio, Some("ja-JP"), Some("eng")),
        Some("ja-JP".to_string())
    );
    assert_eq!(
        normalize_mkv_track_language(TrackKind::Audio, None, None),
        None
    );
}

#[test]
fn matroska_webvtt_track_keeps_subtitle_stream_without_codec_name() {
    let track_entry = [
        make_uint_element(&[0xD7], 1),
        make_uint_element(&[0x83], 17),
        make_ebml_element(&[0x86], b"S_TEXT/WEBVTT"),
        make_ebml_element(&[0x22, 0xB5, 0x9C], b"eng"),
    ]
    .concat();

    let track = parse_mkv_track_entry(&track_entry).unwrap();
    assert_eq!(track.raw.kind, TrackKind::Subtitle);
    assert_eq!(track.raw.codec_id, "S_TEXT/WEBVTT");
    assert_eq!(track.raw.codec_name, None);
    assert_eq!(track.raw.language.as_deref(), Some("eng"));
}

#[test]
fn scanned_audio_channels_replace_container_channels() {
    assert_eq!(merge_scanned_audio_channels(Some(2), Some(6)), Some(6));
    assert_eq!(merge_scanned_audio_channels(Some(6), Some(2)), Some(2));
    assert_eq!(merge_scanned_audio_channels(Some(2), None), Some(2));
}

#[test]
fn unique_ac3_prefix_channel_fallback_fills_missing_channels() {
    let mut scanner = MkvRawScanner::new(Cursor::new(vec![
        0x1A, 0x45, 0xDF, 0xA3, 0x0B, 0x77, 0x0A, 0xA2, 0x1C, 0x30, 0x43, 0x0B, 0x77, 0x9A, 0xE2,
        0x1C, 0x30, 0xE1,
    ]))
    .unwrap();
    let mut tracks = vec![audio_test_track("ac3", None)];

    apply_unique_audio_prefix_channel_fallback(&mut scanner, &mut tracks);

    assert_eq!(tracks[0].channels, Some(6));
}

#[test]
fn unique_audio_prefix_channel_fallback_skips_same_codec_multi_audio() {
    let mut scanner =
        MkvRawScanner::new(Cursor::new(vec![0x0B, 0x77, 0x0A, 0xA2, 0x1C, 0x30, 0x43])).unwrap();
    let mut tracks = vec![
        audio_test_track("ac3", Some(2)),
        audio_test_track("ac3", Some(2)),
    ];

    apply_unique_audio_prefix_channel_fallback(&mut scanner, &mut tracks);

    assert_eq!(tracks[0].channels, Some(2));
    assert_eq!(tracks[1].channels, Some(2));
}

#[test]
fn parses_flac_channels_from_matroska_codec_private() {
    let mut codec_private = Vec::new();
    codec_private.extend_from_slice(b"fLaC");
    codec_private.extend_from_slice(&[0x00, 0x00, 0x00, 0x22]);
    let mut streaminfo = [0_u8; 34];
    streaminfo[12] = 0;
    streaminfo[13] = 0;
    codec_private.extend_from_slice(&streaminfo);

    assert_eq!(
        parse_mkv_flac_codec_private_channels(&codec_private),
        Some(1)
    );
}

#[test]
fn count_increasing_chapter_starts_skips_starts_not_after_the_latest() {
    assert_eq!(
        count_increasing_chapter_starts([0, 90, 1320, 6, 51, 1429]),
        4
    );
    assert_eq!(
        count_increasing_chapter_starts([0, 15, 105, 1226, 1315, 1409]),
        6
    );
}

#[test]
fn chapter_scan_uses_first_edition_only() {
    let first_edition = [
        make_chapter_atom(0, &[]),
        make_chapter_atom(90, &[]),
        make_chapter_atom(742, &[]),
    ]
    .concat();
    let second_edition = [
        make_chapter_atom(33, &[]),
        make_chapter_atom(73, &[]),
        make_chapter_atom(164, &[]),
        make_chapter_atom(636, &[]),
    ]
    .concat();
    let chapters = make_ebml_element(
        &[0x10, 0x43, 0xA7, 0x70],
        &[
            make_ebml_element(&[0x45, 0xB9], &first_edition),
            make_ebml_element(&[0x45, 0xB9], &second_edition),
        ]
        .concat(),
    );

    assert_eq!(count_mkv_chapters_from_bytes(&chapters), Some(3));
}

#[test]
fn chapter_scan_ignores_nested_atoms_and_backwards_starts() {
    let nested = make_chapter_atom(105, &[]);
    let edition = [
        make_chapter_atom(0, &nested),
        make_chapter_atom(90, &[]),
        make_chapter_atom(15, &[]),
        make_chapter_atom(1409, &[]),
    ]
    .concat();
    let chapters = make_ebml_element(
        &[0x10, 0x43, 0xA7, 0x70],
        &make_ebml_element(&[0x45, 0xB9], &edition),
    );

    assert_eq!(count_mkv_chapters_from_bytes(&chapters), Some(3));
}

#[test]
fn observed_frame_timing_preserves_rationals_and_sampling_uncertainty() {
    let timing = observe_frame_timing(&[0, 120, 40, 80, 160], 1_000_000.0).unwrap();
    assert_eq!(
        timing.rate,
        scryer_media_model::Rational::new(25, 1).unwrap()
    );
    assert!(!timing.variable);
    let rounded = observe_frame_timing(&[0, 33, 67, 100], 1_000_000.0).unwrap();
    assert_eq!(
        rounded.rate,
        scryer_media_model::Rational::new(30, 1).unwrap()
    );
    assert!(!rounded.variable);
    let variable = observe_frame_timing(&[0, 40, 80, 160], 1_000_000.0).unwrap();
    assert_eq!(
        variable.rate,
        scryer_media_model::Rational::new(75, 4).unwrap()
    );
    assert!(variable.variable);
    assert!(observe_frame_timing(&[0, 120, 40, 80, 240], 1_000_000.0).is_none());
    assert!(observe_frame_timing(&[0, 40, 40, 80], 1_000_000.0).is_none());
    for scale in [f64::NAN, f64::INFINITY, 0.0, 1.5, u64::MAX as f64] {
        assert!(observe_frame_timing(&[0, 40, 80, 120], scale).is_none());
    }
}

#[test]
fn estimate_frame_rate_from_timestamp_deltas_uses_millisecond_units() {
    let fps = estimate_frame_rate_from_timestamps(&[0, 40, 80, 120, 160], 1_000_000.0);
    assert_eq!(fps, Some(25.0));
}

#[test]
fn estimate_frame_rate_requires_more_than_sparse_samples() {
    assert_eq!(
        estimate_frame_rate_from_timestamps(&[0, 1000], 1_000_000.0),
        None
    );
    assert_eq!(
        estimate_frame_rate_from_timestamps(&[0, 1000, 2000], 1_000_000.0),
        None
    );
}

#[test]
fn plausible_frame_rate_guard_matches_expected_bounds() {
    assert!(is_plausible_frame_rate(Some(23.976)));
    assert!(is_plausible_frame_rate(Some(240.0)));
    assert!(!is_plausible_frame_rate(None));
    assert!(!is_plausible_frame_rate(Some(0.5)));
    assert!(!is_plausible_frame_rate(Some(1000.0)));
}

#[test]
fn only_replaces_clearly_bogus_existing_frame_rates() {
    assert!(should_replace_frame_rate(None, 24.0));
    assert!(should_replace_frame_rate(Some(1.0), 1000.0));
    assert!(!should_replace_frame_rate(Some(24.0), 6.0));
    assert!(!should_replace_frame_rate(Some(23.976), 24.0));
}

#[test]
fn track_entry_signals_detect_t35_mapping_and_dovi_config() {
    let track_entry_with_dovi = |block_add_id_type: u64, dovi_config: &[u8]| {
        [
            make_uint_element(&[0xD7], 1),
            make_ebml_element(
                &[0x41, 0xE4],
                &make_uint_element(&[0x41, 0xE7], MATROSKA_BLOCK_ADD_ID_TYPE_ITU_T_T35),
            ),
            make_ebml_element(
                &[0x41, 0xE4],
                &[
                    make_uint_element(&[0x41, 0xE7], block_add_id_type),
                    make_ebml_element(&[0x41, 0xED], dovi_config),
                ]
                .concat(),
            ),
        ]
        .concat()
    };

    // Wire values as muxers write them: the FourCCs 'dvcC' (profile 7 here) and
    // 'dvvC' (profile 8 here).
    for (block_add_id_type, dovi_config) in [
        (0x6476_6343, vec![1, 0, 0x0E, 0x37, 0x60]),
        (0x6476_7643, vec![1, 0, 0x10, 0x35, 0x10]),
    ] {
        let signals = track_entry_signals(&track_entry_with_dovi(block_add_id_type, &dovi_config));
        assert!(signals.has_itu_t_t35_mapping);
        assert_eq!(
            signals.dovi_config,
            Some(dovi_config),
            "{block_add_id_type:#x}"
        );
    }

    // A truncated two-byte type is not a Dolby Vision mapping.
    let signals = track_entry_signals(&track_entry_with_dovi(0x6476, &[1, 0, 0x10, 0x35, 0x10]));
    assert!(signals.has_itu_t_t35_mapping);
    assert_eq!(signals.dovi_config, None);
}

#[test]
fn track_entry_signals_keep_the_last_dovi_record_that_parses() {
    let mapping = |block_add_id_type: u64, record: &[u8]| {
        make_ebml_element(
            &[0x41, 0xE4],
            &[
                make_uint_element(&[0x41, 0xE7], block_add_id_type),
                make_ebml_element(&[0x41, 0xED], record),
            ]
            .concat(),
        )
    };
    let profile_7 = [1, 0, 0x0E, 0x37, 0x60];
    let profile_8 = [1, 0, 0x10, 0x35, 0x10];
    let truncated = [1, 0, 0x10];

    // Two valid mappings: the later one wins, as in FFmpeg.
    let signals = track_entry_signals(
        &[
            mapping(0x6476_6343, &profile_7),
            mapping(0x6476_7643, &profile_8),
        ]
        .concat(),
    );
    assert_eq!(signals.dovi_config.as_deref(), Some(&profile_8[..]));

    // A record too short to parse is never stored and never displaces a valid one.
    let signals = track_entry_signals(&mapping(0x6476_7643, &truncated));
    assert_eq!(signals.dovi_config, None);
    let signals = track_entry_signals(
        &[
            mapping(0x6476_6343, &profile_7),
            mapping(0x6476_7643, &truncated),
        ]
        .concat(),
    );
    assert_eq!(signals.dovi_config.as_deref(), Some(&profile_7[..]));
}

#[test]
fn hdr10plus_confirmation_gate_short_circuits_dovi_hlg_and_non_hevc() {
    let hevc_track = RawTrack {
        metadata: Default::default(),
        kind: TrackKind::Video,
        codec_id: "V_MPEGH/ISO/HEVC".into(),
        codec_name: Some(Codec::Hevc),
        audio_profile: None,
        codec_private: None,
        width: None,
        height: None,
        channels: None,
        bit_rate_bps: None,
        language: None,
        name: None,
        frame_rate_fps: None,
        color_transfer: Some(16),
        dovi_config: None,
        has_hdr10plus: false,
    };

    assert!(should_confirm_mkv_hdr10plus(
        &hevc_track,
        &MkvTrackSignals {
            has_itu_t_t35_mapping: true,
            dovi_config: None,
            header_strip_prefix: None,
            ..Default::default()
        }
    ));

    let mut dovi_track = hevc_track.clone();
    dovi_track.dovi_config = Some(vec![1, 0, 0x10, 0x00, 0x10]);
    assert!(should_confirm_mkv_hdr10plus(
        &dovi_track,
        &MkvTrackSignals {
            has_itu_t_t35_mapping: true,
            dovi_config: None,
            header_strip_prefix: None,
            ..Default::default()
        }
    ));

    let mut hlg_track = hevc_track.clone();
    hlg_track.color_transfer = Some(18);
    assert!(!should_confirm_mkv_hdr10plus(
        &hlg_track,
        &MkvTrackSignals::default()
    ));

    let mut h264_track = hevc_track.clone();
    h264_track.codec_name = Some(Codec::H264);
    assert!(!should_confirm_mkv_hdr10plus(
        &h264_track,
        &MkvTrackSignals::default()
    ));
}

#[test]
fn observed_frame_timing_reaches_mkv_metadata_without_replacing_declarations() {
    for track_scale in [None, Some(1.0_f64), Some(2.0_f64)] {
        let mut track = [
            make_uint_element(&[0xd7], 1),
            make_uint_element(&[0x83], 1),
            make_ebml_element(&[0x86], b"V_MPEG2"),
            make_uint_element(&[0x23, 0xe3, 0x83], 40_000_000),
        ]
        .concat();
        if let Some(scale) = track_scale {
            track.extend(make_ebml_element(&[0x23, 0x31, 0x4f], &scale.to_be_bytes()));
        }
        let tracks = make_ebml_element(
            &[0x16, 0x54, 0xae, 0x6b],
            &make_ebml_element(&[0xae], &track),
        );
        let cluster = make_ebml_element(
            &[0x1f, 0x43, 0xb6, 0x75],
            &[
                make_uint_element(&[0xe7], 0),
                make_simple_block(1, 0, 0, &[0]),
                make_simple_block(1, 40, 0, &[0]),
                make_simple_block(1, 80, 0, &[0]),
                make_simple_block(1, 160, 0, &[0]),
            ]
            .concat(),
        );
        let bytes = [
            make_ebml_element(&[0x1a, 0x45, 0xdf, 0xa3], &[]),
            make_ebml_element(&[0x18, 0x53, 0x80, 0x67], &[tracks, cluster].concat()),
        ]
        .concat();
        for profile in [ProbeDepth::Full, ProbeDepth::FfprobeParity] {
            let parsed = parse_mkv_source(&mut Cursor::new(bytes.clone()), profile).unwrap();
            let video = &parsed.tracks[0];
            assert_eq!(video.frame_rate_fps, Some(25.0));
            assert_eq!(
                video.metadata.declared_frame_rate,
                scryer_media_model::Rational::new(25, 1)
            );
            if track_scale == Some(2.0) {
                assert!(video.metadata.observed_frame_rate.is_none());
                assert!(video.metadata.variable_frame_rate.is_none());
            } else {
                assert_eq!(
                    video.metadata.observed_frame_rate,
                    scryer_media_model::Rational::new(75, 4)
                );
                assert_eq!(video.metadata.variable_frame_rate, Some(true));
            }
            assert!(
                parsed
                    .details
                    .report
                    .warnings
                    .iter()
                    .any(|warning| warning.code == "mkv_frame_timing_sampled")
            );
        }
    }
}

#[test]
fn observed_frame_timing_ignores_invisible_blocks_and_rejects_lacing() {
    for flags in [0x08, 0x02, 0x04, 0x06] {
        let cluster = make_ebml_element(
            &[0x1f, 0x43, 0xb6, 0x75],
            &[
                make_uint_element(&[0xe7], 0),
                make_simple_block(1, 0, 0, &[0]),
                make_simple_block(1, 20, flags, &[1, 0, 0]),
                make_simple_block(1, 40, 0, &[0]),
                make_simple_block(1, 80, 0, &[0]),
                make_simple_block(1, 120, 0, &[0]),
            ]
            .concat(),
        );
        let mut scanner = MkvRawScanner::new(Cursor::new(cluster)).unwrap();
        let fps = scanner.probe_frame_rate(1, 1_000_000.0).unwrap();
        assert_eq!(fps, (flags == 0x08).then_some(25.0));
    }
}

#[test]
fn observed_frame_timing_reads_headers_with_a_fixed_sample_budget() {
    struct Metered {
        inner: Cursor<Vec<u8>>,
        bytes: usize,
    }
    impl Read for Metered {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let count = self.inner.read(output)?;
            self.bytes += count;
            Ok(count)
        }
    }
    impl Seek for Metered {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(position)
        }
    }
    let payload = vec![0; 64 * 1024];
    let mut blocks = make_uint_element(&[0xe7], 0);
    for index in 0..48 {
        blocks.extend(make_simple_block(1, index * 40, 0, &payload));
    }
    let cluster = make_ebml_element(&[0x1f, 0x43, 0xb6, 0x75], &blocks);
    let source = Metered {
        inner: Cursor::new(cluster),
        bytes: 0,
    };
    let mut scanner = MkvRawScanner::new(source).unwrap();
    let mut plan = MkvDeepProbePlan::new(
        Some(MkvFrameRateProbeRequest {
            target_track_num: 1,
            timestamp_scale_ns: 1_000_000.0,
        }),
        None,
        Vec::new(),
    );
    scanner.run_deep_probe_plan(&mut plan).unwrap();
    assert_eq!(
        plan.frame_rate.as_ref().unwrap().state.timestamps.len(),
        MKV_FPS_PROBE_MAX_TIMESTAMPS
    );
    assert!(
        scanner.reader.bytes < 4096,
        "read {} bytes for timing",
        scanner.reader.bytes
    );
    let result = plan.into_result();
    assert!(result.frame_timing_budget_exhausted);
    assert_eq!(result.frame_rate_fps, Some(25.0));
}

#[test]
fn raw_block_timestamp_probe_derives_frame_rate_without_payload_scan() {
    let cluster = make_frame_rate_probe_cluster();
    let segment = make_ebml_element(&[0x18, 0x53, 0x80, 0x67], &cluster);
    let mut scanner = MkvRawScanner::new(Cursor::new(segment)).expect("scanner");

    let fps = scanner
        .probe_frame_rate(1, 1_000_000.0)
        .expect("probe should succeed");
    assert_eq!(fps, Some(25.0));
}

#[test]
fn raw_block_timestamp_probe_respects_deep_probe_depth_cap() {
    let within_cap = wrap_in_segments(make_frame_rate_probe_cluster(), MKV_DEEP_PROBE_MAX_DEPTH);
    let mut scanner = MkvRawScanner::new(Cursor::new(within_cap)).expect("scanner");
    let fps = scanner
        .probe_frame_rate(1, 1_000_000.0)
        .expect("probe should complete within depth cap");
    assert_eq!(fps, Some(25.0));

    let beyond_cap = wrap_in_segments(
        make_frame_rate_probe_cluster(),
        MKV_DEEP_PROBE_MAX_DEPTH + 1,
    );
    let mut scanner = MkvRawScanner::new(Cursor::new(beyond_cap)).expect("scanner");
    let fps = scanner
        .probe_frame_rate(1, 1_000_000.0)
        .expect("probe should complete beyond depth cap");
    assert_eq!(fps, None);
}

#[test]
fn sized_payload_rejects_declared_end_beyond_file_len_before_read() {
    let mut scanner =
        MkvRawScanner::new_with_file_len(Cursor::new(Vec::<u8>::new()), 16).expect("scanner");
    let header = EbmlElementHeader {
        id: EBML_ID_INFO,
        data_offset: 8,
        size: Some(MKV_KEEP_ELEMENT_MAX_BYTES),
    };

    let error = scanner
        .read_sized_payload(header, u64::MAX)
        .expect_err("oversized declared payload should fail before read");
    assert!(matches!(
        error,
        ProbeError::Malformed(message) if message.contains("beyond remaining input")
    ));
}

#[test]
fn parse_mkv_uses_real_file_length_for_declared_payload_limit() {
    let path = unique_temp_mkv_path("declared-payload");
    let data = make_mkv_with_declared_info_size(MKV_KEEP_ELEMENT_MAX_BYTES);
    assert!(data.len() < 1024, "test file should stay tiny");
    std::fs::write(&path, data).expect("write tiny mkv");

    let error = parse_mkv(&path, ProbeDepth::Full)
        .expect_err("declared payload beyond file length should fail");
    let _ = std::fs::remove_file(&path);
    assert!(matches!(
        error,
        ProbeError::Malformed(message) if message.contains("beyond remaining input")
    ));
}

#[test]
fn parse_mkv_rejects_missing_ebml_header_immediately() {
    let path = std::env::temp_dir().join(format!("scryer-invalid-{}.mkv", std::process::id()));
    let file = std::fs::File::create(&path).expect("create invalid mkv");
    file.set_len(60 * 1024 * 1024).expect("set sparse size");

    let error = parse_mkv(&path, ProbeDepth::Full).expect_err("invalid mkv should fail fast");
    let _ = std::fs::remove_file(&path);
    assert!(matches!(error, ProbeError::Malformed(message) if message.contains("ebml header")));
}

// A virtual sparse file serves real EBML headers without allocating or
// writing gigabytes. Repeated clusters model a large remux on a NAS.
struct SparseMkv {
    head: Vec<u8>,
    cluster: Vec<u8>,
    cluster_span: u64,
    cluster_count: u64,
    tail: Vec<u8>,
    position: u64,
    reads: usize,
    seeks: usize,
    bytes: usize,
}

impl crate::source::MediaSource for SparseMkv {
    fn len(&self) -> u64 {
        self.head.len() as u64 + self.cluster_span * self.cluster_count + self.tail.len() as u64
    }
}

impl Read for SparseMkv {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        use crate::source::MediaSource;
        self.reads += 1;
        if self.reads > 1024 {
            return Err(std::io::Error::other("unexpected whole-file traversal"));
        }
        let count = (self.len().saturating_sub(self.position)).min(output.len() as u64) as usize;
        let head_end = self.head.len() as u64;
        let tail_start = head_end + self.cluster_span * self.cluster_count;
        let mut written = 0;
        while written < count {
            let (data, offset, available) = if self.position < head_end {
                (&self.head, self.position, head_end - self.position)
            } else if self.position < tail_start {
                let offset = (self.position - head_end) % self.cluster_span;
                (&self.cluster, offset, self.cluster_span - offset)
            } else {
                (
                    &self.tail,
                    self.position - tail_start,
                    self.len() - self.position,
                )
            };
            let length = available.min((count - written) as u64) as usize;
            output[written..written + length].fill(0);
            if offset < data.len() as u64 {
                let copied = length.min(data.len() - offset as usize);
                output[written..written + copied]
                    .copy_from_slice(&data[offset as usize..offset as usize + copied]);
            }
            self.position += length as u64;
            written += length;
        }
        self.bytes += count;
        Ok(count)
    }
}

impl Seek for SparseMkv {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        use crate::source::MediaSource;
        let position = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.len()) + i128::from(offset),
        };
        self.position = u64::try_from(position).map_err(std::io::Error::other)?;
        self.seeks += 1;
        Ok(self.position)
    }
}

fn sparse_mkv(cluster_count: u64, indexed: bool, with_duration: bool) -> SparseMkv {
    let mut info = make_uint_element(&[0x2a, 0xd7, 0xb1], 1_000_000);
    if with_duration {
        info.extend(make_ebml_element(&[0x44, 0x89], &600_000_f64.to_be_bytes()));
    }
    let mut metadata = make_ebml_element(&[0x15, 0x49, 0xa9, 0x66], &info);
    let track = [
        make_uint_element(&[0xd7], 1),
        make_uint_element(&[0x73, 0xc5], 1),
        make_uint_element(&[0x83], 1),
        make_uint_element(&[0x23, 0xe3, 0x83], 40_000_000),
        make_ebml_element(&[0x86], b"V_MPEG4/ISO/AVC"),
        make_ebml_element(
            &[0xe0],
            &[
                make_uint_element(&[0xb0], 1920),
                make_uint_element(&[0xba], 1080),
            ]
            .concat(),
        ),
    ]
    .concat();
    metadata.extend(make_ebml_element(
        &[0x16, 0x54, 0xae, 0x6b],
        &make_ebml_element(&[0xae], &track),
    ));
    let chapters = make_ebml_element(
        &[0x10, 0x43, 0xa7, 0x70],
        &make_ebml_element(
            &[0x45, 0xb9],
            &make_chapter_atom(
                0,
                &make_ebml_element(&[0x80], &make_ebml_element(&[0x85], b"Opening")),
            ),
        ),
    );
    let tags = make_ebml_element(
        &[0x12, 0x54, 0xc3, 0x67],
        &make_ebml_element(
            &[0x73, 0x73],
            &[
                make_ebml_element(&[0x63, 0xc0], &make_uint_element(&[0x63, 0xc5], 1)),
                make_ebml_element(
                    &[0x67, 0xc8],
                    &[
                        make_ebml_element(&[0x45, 0xa3], b"BPS"),
                        make_ebml_element(&[0x44, 0x87], b"8000000"),
                    ]
                    .concat(),
                ),
            ]
            .concat(),
        ),
    );
    let attachments = make_ebml_element(
        &[0x19, 0x41, 0xa4, 0x69],
        &make_ebml_element(
            &[0x61, 0xa7],
            &[
                make_uint_element(&[0x46, 0xae], 7),
                make_ebml_element(&[0x46, 0x6e], b"font.ttf"),
                make_ebml_element(&[0x46, 0x60], b"font/ttf"),
                make_ebml_element(&[0x46, 0x5c], &[0; 16]),
            ]
            .concat(),
        ),
    );
    let cluster_span = 4 * 1024 * 1024;
    let seek_head = |base: u64| {
        let entries = [
            (EBML_ID_CHAPTERS, base),
            (0x1254_c367, base + chapters.len() as u64),
            (
                0x1941_a469,
                base + chapters.len() as u64 + tags.len() as u64,
            ),
        ]
        .into_iter()
        .flat_map(|(id, position)| {
            make_ebml_element(
                &[0x4d, 0xbb],
                &[
                    make_ebml_element(&[0x53, 0xab], &id.to_be_bytes()),
                    make_ebml_element(&[0x53, 0xac], &position.to_be_bytes()),
                ]
                .concat(),
            )
        })
        .collect::<Vec<_>>();
        make_ebml_element(&[0x11, 0x4d, 0x9b, 0x74], &entries)
    };
    if indexed {
        let base = metadata.len() as u64 + seek_head(0).len() as u64 + cluster_span * cluster_count;
        metadata.extend(seek_head(base));
    }
    let tail = [chapters, tags, attachments].concat();
    let head = [
        make_ebml_element(&[0x1a, 0x45, 0xdf, 0xa3], &[]),
        make_ebml_element_with_declared_size(
            &[0x18, 0x53, 0x80, 0x67],
            metadata.len() as u64 + cluster_span * cluster_count + tail.len() as u64,
            &metadata,
        ),
    ]
    .concat();
    let mut blocks = make_uint_element(&[0xe7], 0);
    for index in 0..16 {
        blocks.extend(make_simple_block(1, index * 40, 0, &[0]));
    }
    // Four-byte size fields keep both the Cluster and padding headers eight/five bytes long.
    let padding_size = cluster_span - 8 - blocks.len() as u64 - 5;
    blocks.extend(make_ebml_element_with_declared_size(
        &[0xec],
        padding_size,
        &[],
    ));
    let cluster =
        make_ebml_element_with_declared_size(&[0x1f, 0x43, 0xb6, 0x75], cluster_span - 8, &blocks);
    SparseMkv {
        head,
        cluster,
        cluster_span,
        cluster_count,
        tail,
        position: 0,
        reads: 0,
        seeks: 0,
        bytes: 0,
    }
}

#[test]
fn mkv_inventory_io_is_independent_of_remux_size() {
    let mut baseline = None;
    for cluster_count in [256, 7680] {
        // 1 GiB and 30 GiB of clusters.
        let mut source = sparse_mkv(cluster_count, true, true);
        let analysis = crate::probe_bounded(
            &mut source,
            "mkv",
            &crate::ProbeOptions::new().with_depth(crate::ProbeDepth::Quick),
        )
        .unwrap();
        assert_eq!(crate::test_facts::video_codec(&analysis), Some("h264"));
        assert_eq!(analysis.duration_seconds.map(f64::round), Some(600.0));
        assert_eq!(
            analysis
                .selected_video()
                .and_then(|video| video.metadata.bitrate_bps)
                .map(|bps| bps / 1000),
            Some(8000)
        );
        assert_eq!(analysis.chapters[0].title.as_deref(), Some("Opening"));
        assert_eq!(analysis.attachments[0].name.as_deref(), Some("font.ttf"));
        assert!(analysis.report.budget_exhausted);
        assert!(
            analysis
                .report
                .warnings
                .iter()
                .any(|warning| warning.code == "mkv_inventory_coverage_limit")
        );
        assert!(source.reads < 256, "{} reads", source.reads);
        assert!(source.seeks < 256, "{} seeks", source.seeks);
        assert!(source.bytes < 2 * 1024 * 1024, "{} bytes", source.bytes);
        let costs = (source.reads, source.seeks, source.bytes);
        eprintln!("{cluster_count} clusters: {costs:?} (reads, seeks, bytes)");
        if let Some(previous) = baseline {
            assert_eq!(costs, previous);
        }
        baseline = Some(costs);
    }
}

#[test]
fn budget_exhausted_during_enrichment_keeps_the_track_table() {
    // The op budget runs out inside video enrichment, after the tracks have
    // been parsed. Every pill on the title page comes from this analysis, so
    // the gap has to be reported without throwing the analysis away.
    let analysis = (16_u64..128)
        .filter_map(|limit| {
            let mut source = sparse_mkv(7680, true, true);
            let analysis = crate::probe_bounded(
                &mut source,
                "mkv",
                &crate::ProbeOptions::new()
                    .with_depth(crate::ProbeDepth::Quick)
                    .with_io_limit(limit),
            )
            .unwrap_or_else(|error| panic!("io limit {limit} failed the analysis: {error}"));
            analysis
                .report
                .warnings
                .iter()
                .any(|warning| warning.code == "mkv_video_enrichment_budget")
                .then_some(analysis)
        })
        .next()
        .expect("some io limit must exhaust the budget inside video enrichment");
    assert_eq!(crate::test_facts::video_codec(&analysis), Some("h264"));
    assert_eq!(analysis.duration_seconds.map(f64::round), Some(600.0));
    assert!(
        crate::test_facts::has_video_with_duration(&analysis),
        "an enrichment gap must not make the file unrecognizable as video"
    );
    assert!(analysis.report.budget_exhausted);
    assert_eq!(
        analysis.report.status,
        scryer_media_model::ProbeStatus::Incomplete
    );
    assert!(
        analysis.report.bytes_read > 0,
        "the report must account for the work that was done"
    );
    assert!(
        analysis
            .report
            .warnings
            .iter()
            .any(|warning| warning.code == "read_budget_exhausted"),
        "the aggregate budget warning still belongs on the report"
    );
}

#[test]
fn unindexed_large_mkv_does_not_invent_final_duration_or_chapter_absence() {
    let mut source = sparse_mkv(7680, false, false);
    let analysis = crate::probe_bounded(
        &mut source,
        "mkv",
        &crate::ProbeOptions::new().with_depth(crate::ProbeDepth::Quick),
    )
    .unwrap();
    assert_eq!(crate::test_facts::video_codec(&analysis), Some("h264"));
    assert!(analysis.duration_seconds.is_none());
    assert!(analysis.chapter_count.is_none());
    assert!(analysis.report.budget_exhausted);
    assert!(source.reads < 256, "{} reads", source.reads);
}

#[test]
fn colour_element_states_chroma_subsampling_and_siting() {
    let colour = |entries: &[([u8; 2], u64)]| {
        let mut colour = Vec::new();
        for (id, value) in entries {
            colour.extend(make_uint_element(id, *value));
        }
        let mut video = make_uint_element(&[0xb0], 720);
        video.extend(make_uint_element(&[0xba], 480));
        video.extend(make_ebml_element(&[0x55, 0xb0], &colour));
        mkv_stream_metadata(&make_ebml_element(&[0xe0], &video), 1)
    };
    let stated = colour(&[
        ([0x55, 0xb3], 1),
        ([0x55, 0xb4], 1),
        ([0x55, 0xb7], 1),
        ([0x55, 0xb8], 2),
    ]);
    assert_eq!(
        stated.chroma_subsampling,
        Some(scryer_media_model::ChromaSubsampling::Yuv420)
    );
    assert_eq!(
        stated.chroma_location,
        Some(scryer_media_model::ChromaLocation::Left)
    );
    let four_two_two = colour(&[
        ([0x55, 0xb3], 1),
        ([0x55, 0xb4], 0),
        ([0x55, 0xb7], 2),
        ([0x55, 0xb8], 2),
    ]);
    assert_eq!(
        four_two_two.chroma_subsampling,
        Some(scryer_media_model::ChromaSubsampling::Yuv422)
    );
    assert_eq!(
        four_two_two.chroma_location,
        Some(scryer_media_model::ChromaLocation::Center)
    );
    // One axis alone, or an unspecified siting, states nothing.
    let partial = colour(&[([0x55, 0xb3], 1), ([0x55, 0xb7], 0), ([0x55, 0xb8], 1)]);
    assert_eq!(partial.chroma_subsampling, None);
    assert_eq!(partial.chroma_location, None);
}
