//! Small authored images for filesystem and playback-sequence regression tests.
use std::io::Cursor;

use super::*;

#[path = "../../tests/support/disc_image.rs"]
mod authored_image;
use authored_image::{both32, iso, mpls};

#[test]
fn udf_bluray_titles_reuse_native_stream_probing_across_metadata_extents() {
    let clip = video_clip();
    let files = [
        (
            "BDMV/PLAYLIST/00001.MPLS",
            mpls(&[("00001", 0, 45_000), ("00001", 0, 45_000)]),
        ),
        ("BDMV/PLAYLIST/00002.MPLS", mpls(&[("00001", 0, 45_000)])),
        (
            "BDMV/CLIPINF/00001.CLPI",
            clip_information((clip.len() / 192) as u32),
        ),
        ("BDMV/STREAM/00001.M2TS", clip),
    ];
    for metadata in [false, true] {
        let bytes = authored_image::udf_image::udf(&files, metadata);
        let mut source = Cursor::new(bytes);
        let inventory = image::open(&mut source).unwrap();
        assert_eq!(inventory.filesystem, "UDF");
        assert!(inventory.bytes_read < 64 * 2048);
        let analysis = inspect(
            &mut source,
            Default::default(),
            AnalysisProfile::DefaultRich,
        )
        .unwrap();
        assert_eq!(
            analysis.details.report.status,
            ProbeStatus::Complete,
            "{:?}",
            analysis.details.report
        );
        assert_eq!(analysis.video_codec.as_deref(), Some("h264"));
        assert_eq!(analysis.duration_seconds, Some(2));
        let disc = analysis.details.disc.as_ref().unwrap();
        assert_eq!(disc.titles.len(), 2);
        assert_eq!(disc.selected_title_id.as_deref(), Some("00001"));
        assert!(disc.automatic_selection);
        assert!(disc.titles.iter().all(|title| !title.streams.is_empty()));
        let selected = inspect(
            &mut source,
            DiscSelection {
                title_id: Some("00002".into()),
                ..Default::default()
            },
            AnalysisProfile::DefaultRich,
        )
        .unwrap();
        assert_eq!(selected.duration_seconds, Some(1));
        assert!(!selected.details.disc.unwrap().automatic_selection);
    }
}

#[test]
fn mastered_iso_inventory_reads_files_without_extracting_payloads() {
    let playlist = mpls(&[("00001", 45_000, 135_000), ("00002", 90_000, 135_000)]);
    let bytes = iso(&[
        ("BDMV/PLAYLIST/00001.MPLS", playlist.clone()),
        ("BDMV/STREAM/00001.M2TS", vec![0x47; 4096]),
        ("BDMV/STREAM/00002.M2TS", vec![0x47; 4096]),
    ]);
    let mut source = Cursor::new(bytes);
    let image = image::open(&mut source).unwrap();
    assert_eq!(image.files.len(), 3);
    assert!(image.bytes_read <= 6 * 2048);
    let read = read_navigation(
        &mut source,
        &image.files["BDMV/PLAYLIST/00001.MPLS"],
        &mut 8192,
    )
    .unwrap();
    assert_eq!(read, playlist);
    let title = navigation::playlist(&read, "00001".into()).unwrap();
    assert_eq!(title.duration_seconds, Some(3.0));
    assert_eq!(title.segments.len(), 2);
}

#[test]
fn invalid_iso_extent_is_rejected_before_payload_read() {
    let mut bytes = iso(&[("VIDEO_TS/VIDEO_TS.IFO", vec![0; 2048])]);
    both32(&mut bytes[16 * 2048 + 156..], 2, u32::MAX);
    assert!(matches!(
        image::open(&mut Cursor::new(bytes)),
        Err(ImageError::Io(_) | ImageError::Malformed(_))
    ));
}

#[test]
fn duplicate_and_branching_playlists_preserve_distinct_authored_cuts() {
    let a = navigation::playlist(
        &mpls(&[("00001", 0, 90_000), ("00002", 0, 45_000)]),
        "00003".into(),
    )
    .unwrap();
    let mut duplicate = a.clone();
    duplicate.id = "00004".into();
    let b = navigation::playlist(
        &mpls(&[("00002", 0, 45_000), ("00001", 0, 90_000)]),
        "00002".into(),
    )
    .unwrap();
    let mut disc = DiscMetadata {
        titles: vec![a, duplicate, b],
        ..Default::default()
    };
    deduplicate_titles(&mut disc.titles);
    select_title(&mut disc);
    assert_eq!(disc.titles.len(), 2);
    assert_eq!(disc.selected_title_id.as_deref(), Some("00002"));
}

fn video_clip() -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/matrix_ts_002.m2ts"))
        .expect("mandatory generated TS fixture")
}

fn clip_information(source_packets: u32) -> Vec<u8> {
    authored_image::clpi(source_packets, 135_000, &[(0, 0)])
}

#[test]
fn clpi_stream_inventory_validates_authored_ranges_and_packet_bounds() {
    let clip = video_clip();
    let info = clip_information((clip.len() / 192) as u32);
    let make = |output, info: Vec<u8>| {
        iso(&[
            ("BDMV/INDEX.BDMV", b"INDX0300".to_vec()),
            ("BDMV/PLAYLIST/00001.MPLS", mpls(&[("00001", 0, output)])),
            ("BDMV/CLIPINF/00001.CLPI", info),
            ("BDMV/STREAM/00001.M2TS", clip.clone()),
        ])
    };
    let analysis = inspect(
        &mut Cursor::new(make(90_000, info.clone())),
        Default::default(),
        AnalysisProfile::DefaultRich,
    )
    .unwrap();
    assert_eq!(
        analysis.details.report.status,
        ProbeStatus::Complete,
        "{:?}",
        analysis.details.report
    );
    assert_eq!(
        analysis.details.disc.as_ref().unwrap().disc_type,
        "uhd_bluray"
    );
    assert_eq!(analysis.audio_languages, ["fra"]);
    assert_eq!(
        analysis.details.disc.as_ref().unwrap().titles[0].segments[0].sequence_id,
        Some(0)
    );
    let bad_trim = inspect(
        &mut Cursor::new(make(180_000, info)),
        Default::default(),
        AnalysisProfile::DefaultRich,
    )
    .unwrap();
    assert_eq!(
        bad_trim.details.disc.as_ref().unwrap().titles[0]
            .report
            .status,
        ProbeStatus::Malformed
    );
    let truncated = inspect(
        &mut Cursor::new(make(
            90_000,
            clip_information((clip.len() / 192 + 1) as u32),
        )),
        Default::default(),
        AnalysisProfile::DefaultRich,
    )
    .unwrap();
    assert_eq!(
        truncated.details.disc.as_ref().unwrap().titles[0]
            .report
            .status,
        ProbeStatus::Malformed
    );
}

#[test]
fn clpi_rejects_truncation_overlaps_and_unknown_clock_sequences() {
    let info = clip_information(1000);
    for length in 0..info.len() {
        assert!(
            clip_info::parse(&info[..length]).is_err(),
            "accepted truncated CLPI of {length} bytes"
        );
    }
    let parsed = clip_info::parse(&info).unwrap();
    let segment = scryer_media_types::DiscSegment {
        in_seconds: 0.0,
        out_seconds: 2.0,
        sequence_id: Some(1),
        ..Default::default()
    };
    assert!(parsed.streams_for(&segment).is_err());
    let mut overlapping = info.clone();
    overlapping[8..12].copy_from_slice(&40_u32.to_be_bytes());
    assert!(clip_info::parse(&overlapping).is_err());
    let mut overlapping = info;
    overlapping[12..16].copy_from_slice(&40_u32.to_be_bytes());
    assert!(clip_info::parse(&overlapping).is_err());
}

#[test]
fn clpi_access_points_bound_trims_and_reject_invalid_tables() {
    let info = authored_image::clpi(300, 196_608, &[(0, 0), (100, 65_536), (200, 131_072)]);
    let parsed = clip_info::parse(&info).unwrap();
    let segment = scryer_media_types::DiscSegment {
        in_seconds: 90_000.0 / 45_000.0,
        out_seconds: 120_000.0 / 45_000.0,
        sequence_id: Some(0),
        ..Default::default()
    };
    assert_eq!(parsed.packet_range(&segment).unwrap(), 100..200);
    let mut edge = segment.clone();
    edge.in_seconds = 65_536.0 / 45_000.0;
    edge.out_seconds = 131_072.0 / 45_000.0;
    assert_eq!(parsed.packet_range(&edge).unwrap(), 100..200);
    let cpi = u32::from_be_bytes(info[16..20].try_into().unwrap()) as usize;
    let map = cpi + 4 + 16;
    for (offset, value) in [
        (cpi + 4 + 12, u32::MAX),
        (map, 4),
        (map + 4, 1 << 14),
        (map + 12, 0),
        (map + 4 + 24, 511),
    ] {
        let mut malformed = info.clone();
        be32(&mut malformed, offset, value);
        assert!(
            matches!(clip_info::parse(&malformed), Err(ImageError::Malformed(_))),
            "offset {offset}"
        );
    }
    let mut oversized = info.clone();
    let packed = (1_u64 << 34) | (1_u64 << 18) | 131_072;
    oversized[cpi + 10..cpi + 16].copy_from_slice(&packed.to_be_bytes()[2..]);
    assert!(matches!(
        clip_info::parse(&oversized),
        Err(ImageError::Budget)
    ));
    let mut no_map = info;
    no_map[16..20].fill(0);
    let no_map = clip_info::parse(&no_map).unwrap();
    assert!(matches!(
        no_map.packet_range(&segment),
        Err(ImageError::Unsupported(_))
    ));
    edge.in_seconds = 0.0;
    edge.out_seconds = 196_608.0 / 45_000.0;
    assert_eq!(no_map.packet_range(&edge).unwrap(), 0..300);
}

#[test]
fn clpi_clock_selection_prevents_pts_reset_from_reusing_the_first_sequence() {
    let mut info = authored_image::clpi(200, 65_536, &[(0, 0), (100, 0)]);
    let sequence = u32::from_be_bytes(info[8..12].try_into().unwrap()) as usize;
    let program = u32::from_be_bytes(info[12..16].try_into().unwrap()) as usize;
    let cpi = u32::from_be_bytes(info[16..20].try_into().unwrap()) as usize;
    let mut second = 0x1011_u16.to_be_bytes().to_vec();
    second.extend(100_u32.to_be_bytes());
    second.extend(0_u32.to_be_bytes());
    second.extend(65_536_u32.to_be_bytes());
    info.splice(program..program, second);
    be32(&mut info, sequence, 36);
    info[sequence + 10] = 2;
    be32(&mut info, 12, (program + 14) as u32);
    be32(&mut info, 16, (cpi + 14) as u32);
    let parsed = clip_info::parse(&info).unwrap();
    let mut segment = scryer_media_types::DiscSegment {
        in_seconds: 0.25,
        out_seconds: 1.0,
        sequence_id: Some(1),
        ..Default::default()
    };
    assert_eq!(parsed.packet_range(&segment).unwrap(), 100..200);
    segment.sequence_id = Some(0);
    assert_eq!(parsed.packet_range(&segment).unwrap(), 0..100);
}

#[test]
fn missing_clip_information_retains_inventory_without_claiming_a_valid_title() {
    let bytes = iso(&[
        ("BDMV/PLAYLIST/00001.MPLS", mpls(&[("00001", 0, 45_000)])),
        ("BDMV/STREAM/00001.M2TS", video_clip()),
    ]);
    let analysis = inspect(
        &mut Cursor::new(bytes),
        Default::default(),
        AnalysisProfile::DefaultRich,
    )
    .unwrap();
    assert_eq!(analysis.details.report.status, ProbeStatus::Incomplete);
    assert!(
        analysis
            .details
            .report
            .warnings
            .iter()
            .any(|warning| warning.code == "missing_clip_information")
    );
    let disc = analysis.details.disc.unwrap();
    assert!(disc.selected_title_id.is_none());
    assert!(!disc.titles[0].streams.is_empty());
    assert_eq!(disc.titles[0].duration_seconds, Some(1.0));
}

#[test]
fn intact_bluray_uses_authored_trims_and_persists_manual_choice() {
    let clip = video_clip();
    let bytes = iso(&[
        (
            "BDMV/PLAYLIST/00001.MPLS",
            mpls(&[("00001", 45_000, 135_000)]),
        ),
        ("BDMV/PLAYLIST/00002.MPLS", mpls(&[("00001", 0, 45_000)])),
        (
            "BDMV/PLAYLIST/00003.MPLS",
            mpls(&[("00001", 45_000, 135_000)]),
        ),
        (
            "BDMV/CLIPINF/00001.CLPI",
            clip_information((clip.len() / 192) as u32),
        ),
        ("BDMV/STREAM/00001.M2TS", clip),
    ]);
    let mut source = BoundedSource::new(Cursor::new(bytes.clone()), 8 * 1024 * 1024);
    let automatic = inspect(
        &mut source,
        DiscSelection::default(),
        AnalysisProfile::DefaultRich,
    )
    .unwrap();
    assert_eq!(
        automatic.details.report.status,
        ProbeStatus::Complete,
        "{:?}",
        automatic.details.report
    );
    assert_eq!(automatic.duration_seconds, Some(2));
    let disc = automatic.details.disc.unwrap();
    assert_eq!(disc.selected_title_id.as_deref(), Some("00001"));
    assert_eq!(disc.titles.len(), 2);
    assert_eq!(disc.titles[0].aliases, ["00003"]);
    assert!(disc.automatic_selection);
    assert!(source.bytes_read < 8 * 1024 * 1024);
    let selected = inspect(
        &mut Cursor::new(bytes.clone()),
        DiscSelection {
            title_id: Some("00002".into()),
            ..Default::default()
        },
        AnalysisProfile::DefaultRich,
    )
    .unwrap();
    assert_eq!(selected.duration_seconds, Some(1));
    assert!(!selected.details.disc.unwrap().automatic_selection);
    let missing = inspect(
        &mut Cursor::new(bytes),
        DiscSelection {
            title_id: Some("99999".into()),
            ..Default::default()
        },
        AnalysisProfile::DefaultRich,
    )
    .unwrap();
    assert!(missing.video_codec.is_none());
    assert!(missing.details.disc.unwrap().selected_title_id.is_none());
    assert_eq!(missing.details.report.status, ProbeStatus::Incomplete);
}

#[test]
fn encrypted_longest_title_falls_back_only_for_automatic_selection() {
    let clip = video_clip();
    let mut scrambled = clip.clone();
    for packet in scrambled.chunks_exact_mut(192) {
        if packet[4] == 0x47 {
            packet[7] |= 0x80;
        }
    }
    let bytes = iso(&[
        ("BDMV/PLAYLIST/00001.MPLS", mpls(&[("00001", 0, 180_000)])),
        ("BDMV/PLAYLIST/00002.MPLS", mpls(&[("00002", 0, 90_000)])),
        (
            "BDMV/CLIPINF/00001.CLPI",
            authored_image::clpi((clip.len() / 192) as u32, 180_000, &[(0, 0)]),
        ),
        (
            "BDMV/CLIPINF/00002.CLPI",
            clip_information((clip.len() / 192) as u32),
        ),
        ("BDMV/STREAM/00001.M2TS", scrambled),
        ("BDMV/STREAM/00002.M2TS", clip),
    ]);
    let automatic = inspect(
        &mut Cursor::new(bytes.clone()),
        DiscSelection::default(),
        AnalysisProfile::DefaultRich,
    )
    .unwrap();
    assert_eq!(
        automatic
            .details
            .disc
            .as_ref()
            .unwrap()
            .selected_title_id
            .as_deref(),
        Some("00002")
    );
    assert_eq!(
        automatic.details.disc.as_ref().unwrap().titles[0]
            .report
            .status,
        ProbeStatus::Encrypted
    );
    let selected = inspect(
        &mut Cursor::new(bytes),
        DiscSelection {
            title_id: Some("00001".into()),
            ..Default::default()
        },
        AnalysisProfile::DefaultRich,
    )
    .unwrap();
    assert!(selected.video_codec.is_none());
    assert_eq!(selected.details.report.status, ProbeStatus::Encrypted);
    assert_eq!(
        selected.details.disc.unwrap().selected_title_id.as_deref(),
        Some("00001")
    );
}

#[test]
fn bluray_cuts_in_one_clip_probe_only_their_own_packet_ranges() {
    let clear = video_clip();
    let packets = (clear.len() / 192) as u32;
    let mut joined = clear.clone();
    for packet in joined.chunks_exact_mut(192) {
        if packet[4] == 0x47 {
            packet[7] |= 0x80;
        }
    }
    joined.extend(clear);
    let files = [
        ("BDMV/PLAYLIST/00001.MPLS", mpls(&[("00001", 0, 65_536)])),
        (
            "BDMV/PLAYLIST/00002.MPLS",
            mpls(&[("00001", 65_536, 131_072)]),
        ),
        (
            "BDMV/CLIPINF/00001.CLPI",
            authored_image::clpi(packets * 2, 131_072, &[(0, 0), (packets, 65_536)]),
        ),
        ("BDMV/STREAM/00001.M2TS", joined),
    ];
    for image in [iso(&files), authored_image::udf_image::udf(&files, true)] {
        let mut source = BoundedSource::new(Cursor::new(image), 8 * 1024 * 1024);
        let analysis = inspect(
            &mut source,
            Default::default(),
            AnalysisProfile::DefaultRich,
        )
        .unwrap();
        assert_eq!(analysis.video_codec.as_deref(), Some("h264"));
        assert_eq!(analysis.details.duration_seconds, Some(65_536.0 / 45_000.0));
        assert_eq!(
            analysis.details.report.status,
            ProbeStatus::Complete,
            "{:?}",
            analysis.details.report
        );
        let disc = analysis.details.disc.unwrap();
        assert_eq!(disc.selected_title_id.as_deref(), Some("00002"));
        assert_eq!(disc.titles[0].report.status, ProbeStatus::Encrypted);
        assert_eq!(disc.titles[1].report.status, ProbeStatus::Complete);
        assert!(source.bytes_read < 8 * 1024 * 1024);
    }
}

fn be16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
}
fn be32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
}
fn dvd_navigation() -> (Vec<u8>, Vec<u8>) {
    let mut vmg = vec![0; 4096];
    vmg[..12].copy_from_slice(b"DVDVIDEO-VMG");
    be32(&mut vmg, 0xc4, 1);
    be16(&mut vmg, 2048, 1);
    be32(&mut vmg, 2052, 19);
    vmg[2057] = 1;
    be16(&mut vmg, 2058, 2);
    vmg[2062] = 1;
    vmg[2063] = 1;
    let mut vts = vec![0; 6144];
    vts[..12].copy_from_slice(b"DVDVIDEO-VTS");
    be32(&mut vts, 0xc8, 1);
    be32(&mut vts, 0xcc, 2);
    vts[0x200] = 0x40;
    vts[0x203] = 2;
    vts[0x204..0x20c].copy_from_slice(&[4, 1, b'e', b'n', 0, 1, 0, 0]);
    vts[0x20c..0x214].copy_from_slice(&[0xc4, 5, b'f', b'r', 0, 3, 0, 0]);
    vts[0x255] = 1;
    vts[0x256..0x25c].copy_from_slice(&[1, 0, b'e', b'n', 0, 5]);
    be16(&mut vts, 2048, 1);
    be32(&mut vts, 2052, 19);
    be32(&mut vts, 2056, 12);
    be16(&mut vts, 2060, 1);
    be16(&mut vts, 2062, 2);
    be16(&mut vts, 2064, 1);
    be16(&mut vts, 2066, 3);
    be16(&mut vts, 4096, 1);
    be32(&mut vts, 4100, 327);
    be32(&mut vts, 4108, 16);
    let pgc = &mut vts[4112..];
    pgc[2] = 3;
    pgc[3] = 3;
    be16(pgc, 12, 0x8000);
    be16(pgc, 14, 0x8100);
    be32(pgc, 28, 0x8000_0000);
    be16(pgc, 230, 236);
    be16(pgc, 232, 240);
    pgc[236..239].copy_from_slice(&[1, 2, 3]);
    for index in 0..3 {
        let cell = &mut pgc[240 + index * 24..264 + index * 24];
        cell[6] = (index + 1) as u8;
        cell[7] = 0x40;
        be32(cell, 8, index as u32);
        be32(cell, 20, index as u32);
    }
    (vmg, vts)
}
fn dvd_cell_payload() -> Vec<u8> {
    let mut bytes = vec![0, 0, 1, 0xba, 0x44, 0, 4, 0, 4, 1, 1, 1, 1, 0];
    for timestamp in [0_u64, 90_000] {
        let pts = [
            0x21 | (((timestamp >> 30) as u8 & 7) << 1),
            (timestamp >> 22) as u8,
            (((timestamp >> 15) as u8 & 127) << 1) | 1,
            (timestamp >> 7) as u8,
            ((timestamp as u8 & 127) << 1) | 1,
        ];
        let mut payload = vec![0x80, 0x80, 5];
        payload.extend(pts);
        // 720x480 MPEG-2 sequence with the required marker and sequence extension.
        payload.extend([
            0, 0, 1, 0xb3, 0x2d, 0x01, 0xe0, 0x34, 0xff, 0xff, 0xe0, 0x18, 0, 0, 1, 0xb5, 0x14,
            0x8a, 0, 1, 0, 0,
        ]);
        bytes.extend([0, 0, 1, 0xe0]);
        bytes.extend((payload.len() as u16).to_be_bytes());
        bytes.extend(payload);
    }
    bytes.resize(2048, 0xff);
    bytes
}

#[test]
fn intact_dvd_uses_title_program_range_chapters_and_ifo_roles() {
    let (vmg, vts) = dvd_navigation();
    let bytes = iso(&[
        ("VIDEO_TS/VIDEO_TS.IFO", vmg),
        ("VIDEO_TS/VTS_01_0.IFO", vts),
        ("VIDEO_TS/VTS_01_1.VOB", dvd_cell_payload().repeat(3)),
    ]);
    let analysis = inspect(
        &mut Cursor::new(bytes),
        Default::default(),
        AnalysisProfile::DefaultRich,
    )
    .unwrap();
    assert_eq!(analysis.details.report.status, ProbeStatus::Complete);
    assert_eq!(analysis.duration_seconds, Some(5));
    assert_eq!(analysis.audio_codec.as_deref(), Some("ac3"));
    assert_eq!(analysis.audio_languages, ["en"]);
    assert_eq!(analysis.audio_streams.len(), 2);
    assert_eq!(analysis.subtitle_streams[0].language.as_deref(), Some("en"));
    let disc = analysis.details.disc.unwrap();
    let title = &disc.titles[0];
    assert_eq!(disc.disc_type, "dvd");
    assert_eq!(title.segments.len(), 2);
    assert_eq!(
        title
            .chapters
            .iter()
            .map(|chapter| chapter.start_seconds)
            .collect::<Vec<_>>(),
        [0.0, 2.0]
    );
    assert_eq!(title.chapters[1].end_seconds, Some(5.0));
    assert!(
        title
            .streams
            .iter()
            .any(|stream| stream.metadata.disposition.commentary == Some(true))
    );
    assert!(
        title
            .streams
            .iter()
            .any(|stream| stream.metadata.disposition.hearing_impaired == Some(true))
    );
}

#[test]
fn dvd_post_title_exits_preserve_the_authored_timeline() {
    for command in [
        [0; 8],
        [0x30, 1, 0, 0, 0, 0, 0, 0],
        [0x30, 6, 0, 0, 0, 0x42, 0, 0],
        [0x30, 6, 0, 1, 1, 0x83, 0, 0],
    ] {
        let (vmg, mut vts) = dvd_navigation();
        dvd_commands(&mut vts, 0, 1, 0, command);
        let bytes = iso(&[
            ("VIDEO_TS/VIDEO_TS.IFO", vmg),
            ("VIDEO_TS/VTS_01_0.IFO", vts),
            ("VIDEO_TS/VTS_01_1.VOB", dvd_cell_payload().repeat(3)),
        ]);
        let analysis = inspect(
            &mut Cursor::new(bytes),
            Default::default(),
            AnalysisProfile::DefaultRich,
        )
        .unwrap();
        assert_eq!(
            analysis.details.report.status,
            ProbeStatus::Complete,
            "{command:?}"
        );
        assert_eq!(analysis.details.duration_seconds, Some(5.0));
    }
}

fn dvd_commands(vts: &mut [u8], pre: u16, post: u16, cell: u16, command: [u8; 8]) {
    be32(vts, 4100, 343);
    be16(vts, 4112 + 228, 312);
    let at = 4112 + 312;
    be16(vts, at, pre);
    be16(vts, at + 2, post);
    be16(vts, at + 4, cell);
    vts[at + 8..at + 16].copy_from_slice(&command);
}

#[test]
fn dvd_playback_commands_remain_explicitly_unsupported() {
    let menu = [0x30, 6, 0, 0, 0, 0x42, 0, 0];
    for (pre, post, cell, command) in [
        (1, 0, 0, menu),
        (0, 0, 1, menu),
        (0, 1, 0, [0x30, 0x26, 0, 0, 0, 0x42, 0, 0]), // conditional menu
        (0, 1, 0, [0x30, 2, 0, 0, 0, 1, 0, 0]),       // another title
        (0, 1, 0, [0x30, 8, 0, 0, 0, 0x42, 0, 0]),    // menu call with resume
        (0, 1, 0, [0, 1, 0, 0, 0, 0, 0, 1]),          // command loop
    ] {
        let (vmg, mut vts) = dvd_navigation();
        dvd_commands(&mut vts, pre, post, cell, command);
        let reference = navigation::dvd_titles(&vmg).unwrap().remove(0);
        let (title, _) = navigation::dvd_title(&vts, &reference).unwrap();
        assert_eq!(title.report.status, ProbeStatus::Unsupported, "{command:?}");
        assert!(
            title
                .report
                .warnings
                .iter()
                .any(|warning| warning.code == "dvd_pgc_commands")
        );
    }
}

#[test]
fn dvd_navigation_rejects_cross_section_references_and_command_overflow() {
    let (vmg, original) = dvd_navigation();
    let reference = navigation::dvd_titles(&vmg).unwrap().remove(0);
    for (offset, value) in [(228, 12), (230, 12), (232, 236), (234, 240)] {
        let mut vts = original.clone();
        be16(&mut vts, 4112 + offset, value);
        assert!(
            navigation::dvd_title(&vts, &reference).is_err(),
            "PGC offset {offset}"
        );
    }
    let mut vts = original.clone();
    dvd_commands(&mut vts, 256, 0, 0, [0; 8]);
    assert!(navigation::dvd_title(&vts, &reference).is_err());
    dvd_commands(&mut vts, 0, 2, 0, [0; 8]); // declared commands exceed table
    assert!(navigation::dvd_title(&vts, &reference).is_err());
    for value in [0, 8, u32::MAX] {
        let mut vts = original.clone();
        be32(&mut vts, 2056, value);
        assert!(navigation::dvd_title(&vts, &reference).is_err());
        let mut vts = original.clone();
        be32(&mut vts, 0xcc, value);
        assert!(navigation::dvd_title(&vts, &reference).is_err());
    }
    let mut vts = original.clone();
    vts.copy_within(4112..4424, 4120);
    be16(&mut vts, 4096, 2);
    be32(&mut vts, 4100, 647);
    be32(&mut vts, 4108, 24);
    be32(&mut vts, 4116, 324);
    assert!(
        navigation::dvd_title(&vts, &reference).is_err(),
        "cell table must not read into another PGC"
    );
}

#[test]
fn dvd_first_angle_and_unsupported_interleaving_are_distinct() {
    let (vmg, mut vts) = dvd_navigation();
    let reference = navigation::dvd_titles(&vmg).unwrap().remove(0);
    be16(&mut vts, 2062, 1);
    vts[4112 + 240] = 0x50;
    vts[4112 + 264] = 0xd0;
    let (title, cells) = navigation::dvd_title(&vts, &reference).unwrap();
    assert_eq!(title.duration_seconds, Some(4.0));
    assert_eq!(cells.len(), 2);
    vts[4112 + 240] |= 4;
    let (title, _) = navigation::dvd_title(&vts, &reference).unwrap();
    assert_eq!(title.report.status, ProbeStatus::Unsupported);
    assert!(
        title
            .report
            .warnings
            .iter()
            .any(|warning| warning.code == "dvd_interleaved_angle")
    );
    be16(&mut vts, 2062, 0);
    assert!(navigation::dvd_title(&vts, &reference).is_err());
}
