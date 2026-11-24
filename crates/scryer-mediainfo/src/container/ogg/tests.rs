use super::ogm_fixture as fixture;
use super::*;
use std::io::Cursor;

fn parse(bytes: Vec<u8>) -> RawContainer {
    parse_ogg_source(&mut Cursor::new(bytes)).unwrap()
}

#[test]
fn ogm_modern_multiplexed_tracks_preserve_metadata_and_end_time() {
    let raw = parse(fixture::video_audio_text());
    assert_eq!(raw.format_name, "ogm");
    assert_eq!(raw.duration_seconds, Some(3.0));
    assert_eq!(raw.tracks.len(), 3);
    let video = &raw.tracks[0];
    assert_eq!(video.kind, TrackKind::Video);
    assert_eq!(video.codec_id, "XVID");
    assert_eq!(
        video
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("mpeg4")
    );
    assert_eq!((video.width, video.height), (Some(1280), Some(720)));
    assert_eq!(video.frame_rate_fps, Some(25.0));
    assert_eq!(
        video.metadata.declared_frame_rate,
        scryer_media_model::Rational::new(25, 1)
    );
    assert_eq!(video.name.as_deref(), Some("Main video"));
    assert_eq!(
        raw.tracks[1]
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("vorbis")
    );
    assert_eq!(raw.tracks[1].language.as_deref(), Some("eng"));
    assert_eq!(raw.tracks[1].name.as_deref(), Some("Stereo"));
    assert_eq!(raw.tracks[2].kind, TrackKind::Subtitle);
    assert_eq!(
        raw.tracks[2]
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("text")
    );
    assert_eq!(raw.tracks[2].language.as_deref(), Some("jpn"));
    assert_eq!(raw.tracks[2].name.as_deref(), Some("Captions"));
    let details = crate::build::build_details(raw);
    let subtitles: Vec<_> = details
        .program_streams(scryer_media_model::StreamKind::Subtitle)
        .map(|stream| {
            (
                stream.codec.as_ref().map(scryer_media_model::Codec::as_str),
                stream.language.as_deref(),
            )
        })
        .collect();
    assert_eq!(subtitles, [(Some("text"), Some("jpn"))]);
}

#[test]
fn ogm_audio_only_reports_wave_metadata_without_becoming_video() {
    let raw = parse(fixture::audio_only());
    assert_eq!(raw.duration_seconds, Some(3.0));
    let audio = &raw.tracks[0];
    assert_eq!(audio.kind, TrackKind::Audio);
    assert_eq!(audio.codec_id, "2000");
    assert_eq!(
        audio
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("ac3")
    );
    assert_eq!(audio.channels, Some(6));
    assert_eq!(audio.metadata.sample_rate, Some(48_000));
    assert_eq!(audio.metadata.sample_bit_depth, None);
    assert_eq!(audio.bit_rate_bps, Some(384_000));
    assert_eq!(audio.language.as_deref(), Some("deu"));
    assert!(!crate::test_facts::has_video_with_duration(
        &crate::build::build_details(raw)
    ));
}

#[test]
fn ogm_modern_rational_video_and_unknown_tags_remain_distinct() {
    let mut header = fixture::modern_header(b"video\0\0\0", b"ZZZZ", 10_010_000, 24);
    let raw = parse(fixture::single_stream(
        &header,
        Some(&fixture::comments(&[])),
        &[0x48, 1, 0],
        23,
        true,
    ));
    assert_eq!(raw.tracks[0].codec_id, "ZZZZ");
    assert_eq!(raw.tracks[0].codec_name, None);
    assert_eq!(
        raw.tracks[0].metadata.declared_frame_rate,
        scryer_media_model::Rational::new(24_000, 1001)
    );
    assert_eq!(raw.duration_seconds, Some(1.001));
    assert_eq!(
        crate::test_facts::video_codec(&crate::build::build_details(raw)),
        None
    );
    header[9..13].copy_from_slice(b"H264");
    assert_eq!(
        parse_ogm_identification(&header)
            .unwrap()
            .track
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("h264")
    );
    let audio = fixture::modern_header(b"audio\0\0\0", b"dead", 10_000_000, 48_000);
    let stream = parse_ogm_identification(&audio).unwrap();
    assert_eq!(stream.track.kind, TrackKind::Audio);
    assert_eq!(stream.track.codec_id, "dead");
    assert_eq!(stream.track.codec_name, None);
}

#[test]
fn ogm_legacy_video_and_audio_use_their_own_layouts() {
    let raw = parse(fixture::single_stream(
        &fixture::legacy_video(),
        None,
        &[0x48, 1, 0],
        74,
        true,
    ));
    assert_eq!(raw.format_name, "ogm");
    assert_eq!(raw.duration_seconds, Some(3.0));
    assert_eq!(
        raw.tracks[0]
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("mpeg4")
    );
    assert_eq!(
        (raw.tracks[0].width, raw.tracks[0].height),
        (Some(1280), Some(720))
    );
    assert_eq!(
        raw.tracks[0].metadata.declared_frame_rate,
        scryer_media_model::Rational::new(25, 1)
    );
    let raw = parse(fixture::single_stream(
        &fixture::legacy_audio(),
        None,
        &[0x80, 0, 6, 0],
        142_464,
        true,
    ));
    assert_eq!(raw.duration_seconds, Some(3.0));
    assert_eq!(
        raw.tracks[0]
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("ac3")
    );
    assert_eq!(raw.tracks[0].metadata.sample_rate, Some(48_000));
    assert_eq!(raw.tracks[0].channels, Some(6));
    assert_eq!(raw.tracks[0].bit_rate_bps, Some(384_000));
}

#[test]
fn ogm_truncated_headers_invalid_counts_and_arithmetic_are_rejected() {
    let header = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
    for length in 6..header.len() {
        assert!(
            parse_ogm_identification(&header[..length]).is_err(),
            "length={length}"
        );
    }
    for (offset, bytes) in [
        (13, 51_u32.to_le_bytes().to_vec()),
        (13, u32::MAX.to_le_bytes().to_vec()),
        (17, 0_u64.to_le_bytes().to_vec()),
        (25, 0_u64.to_le_bytes().to_vec()),
        (25, u64::MAX.to_le_bytes().to_vec()),
        (45, u32::MAX.to_le_bytes().to_vec()),
        (49, 0_u32.to_le_bytes().to_vec()),
    ] {
        let mut bad = header.clone();
        bad[offset..offset + bytes.len()].copy_from_slice(&bytes);
        assert!(parse_ogm_identification(&bad).is_err(), "offset={offset}");
    }
    let legacy = fixture::legacy_video();
    for length in 35..legacy.len() {
        assert!(parse_ogm_legacy_identification(&legacy[..length]).is_err());
    }
    let mut comments = fixture::comments(&[])[7..].to_vec();
    comments[11..15].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(parse_comments(&comments).is_err());
    assert!(parse_comments(&u32::MAX.to_le_bytes()).is_err());
    assert!(parse_ogg_source(&mut Cursor::new(b"OggS\0".to_vec())).is_err());
}

#[test]
fn ogm_audio_private_config_excludes_aac_prefix_and_checks_declared_size() {
    let mut header = fixture::modern_header(b"audio\0\0\0", b"00ff", 10_000_000, 48_000);
    header[13..17].copy_from_slice(&58_u32.to_le_bytes());
    header.extend_from_slice(&[0, 0, 0, 0, 0x11, 0x90]);
    assert_eq!(
        parse_ogm_identification(&header)
            .unwrap()
            .track
            .codec_private,
        Some(vec![0x11, 0x90])
    );
    header.pop();
    assert!(parse_ogm_identification(&header).is_err());
    let mut header = fixture::modern_header(b"audio\0\0\0", b"0001", 10_000_000, 48_000);
    header[41..43].copy_from_slice(&24_u16.to_le_bytes());
    assert_eq!(
        parse_ogm_identification(&header)
            .unwrap()
            .track
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("pcm_s24le")
    );
    header[45..47].fill(0);
    assert!(parse_ogm_identification(&header).is_err());
}

#[test]
fn ogm_duration_requires_eos_complete_packet_and_checked_end() {
    let header = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
    let comment = fixture::comments(&[]);
    for (packet, granule, eos) in [
        (&[0x48, 1, 0][..], 74, false),
        (&[0xc0][..], 74, true),
        (&[0x48, 1, 0][..], u64::MAX - 1, true),
        (&[0x48, 1, 0][..], i64::MAX as u64, true),
        (&[0x48, 1, 0][..], i64::MAX as u64 + 1, true),
    ] {
        assert_eq!(
            parse(fixture::single_stream(
                &header,
                Some(&comment),
                packet,
                granule,
                eos
            ))
            .duration_seconds,
            None
        );
    }
    assert_eq!(
        parse(fixture::single_stream(
            &header,
            Some(&comment),
            &[0x08, 0],
            74,
            true
        ))
        .duration_seconds,
        Some(3.0)
    );
    assert_eq!(
        ogm_packet_duration(&[0x02, 0x78, 0x56, 0x34, 0x12]),
        Ok(Some(0x1234_5678))
    );
    assert!(
        parse(fixture::single_stream(
            &header,
            Some(&comment),
            &[0x48, 1, 0],
            i64::MAX as u64 - 1,
            true
        ))
        .duration_seconds
        .is_some()
    );
    assert_eq!(
        ogm_packet_duration(&[0xc2, 1, 2, 3, 4, 5, 6, 7]),
        Ok(Some(0x07_0605_0403_0201))
    );
}

#[test]
fn ogm_clipped_tail_recaptures_past_false_magic_before_packet_and_nil_eos_parsing() {
    let header = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
    let packet_tail = fixture::single_stream(
        &header,
        Some(&fixture::comments(&[])),
        &[0x48, 1, 0],
        74,
        true,
    );
    for suffix in [packet_tail, fixture::nil_eos_page(7, 75, 3)] {
        let mut bytes = vec![0x55; MAX_TAIL_BYTES as usize];
        let mut fake = [0; 27];
        fake[..4].copy_from_slice(b"OggS");
        fake[4] = 1; // Invalid version before another false, CRC-invalid header.
        bytes.extend_from_slice(&fake);
        fake[4] = 0;
        bytes.extend_from_slice(&fake);
        bytes.extend_from_slice(&suffix);
        let length = bytes.len() as u64;
        let tail = read_stream_ends(&mut Cursor::new(bytes), length).unwrap();
        assert!(tail.ends.contains_key(&7));
        assert_eq!(tail.last_packet, Some((7, true)));
    }
}

#[test]
fn ogm_tail_does_not_resync_after_valid_boundary_or_accept_embedded_nil_eos() {
    let header = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
    let embedded = fixture::nil_eos_page(99, 100, 0);
    let mut payload = vec![0x48, 1];
    payload.extend_from_slice(&embedded);
    let valid = fixture::single_stream(&header, Some(&fixture::comments(&[])), &payload, 74, true);
    let tail = read_stream_ends(&mut Cursor::new(valid.clone()), valid.len() as u64).unwrap();
    assert!(!tail.ends.contains_key(&99));
    let mut corrupt = valid;
    let mut bad = fixture::nil_eos_page(7, 75, 3);
    bad[22] ^= 1;
    corrupt.extend_from_slice(&bad);
    corrupt.extend_from_slice(&fixture::nil_eos_page(7, 75, 4));
    let length = corrupt.len() as u64;
    assert!(read_stream_ends(&mut Cursor::new(corrupt), length).is_err());
    let mut captures = Vec::new();
    for _ in 0..=MAX_TAIL_CAPTURE_CANDIDATES {
        captures.extend_from_slice(b"OggS");
    }
    assert!(first_valid_tail_page(&captures, true).is_err());
}

#[test]
fn ogm_nil_eos_pages_use_validated_end_boundary_without_adding_a_frame() {
    let header = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
    let mut bytes = fixture::single_stream(
        &header,
        Some(&fixture::comments(&[])),
        &[0x48, 1, 0],
        74,
        false,
    );
    let nil = fixture::nil_eos_page(7, 75, 3);
    bytes.extend_from_slice(&nil);
    assert_eq!(parse(bytes.clone()).duration_seconds, Some(3.0));
    *bytes.last_mut().unwrap() ^= 1;
    assert_eq!(parse(bytes).duration_seconds, None);
}

#[test]
fn ogm_tail_is_bounded_and_earlier_subtitle_end_keeps_terminal_runtime() {
    use ogg::{PacketWriteEndInfo, PacketWriter};
    let mut output = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new(&mut output);
    let video = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
    let text = fixture::modern_header(b"text\0\0\0\0", &[0; 4], 10_000, 1);
    writer
        .write_packet(&video, 1, PacketWriteEndInfo::EndPage, 0)
        .unwrap();
    writer
        .write_packet(&text, 2, PacketWriteEndInfo::EndPage, 0)
        .unwrap();
    for serial in [1, 2] {
        writer
            .write_packet(
                fixture::comments(&[]),
                serial,
                PacketWriteEndInfo::EndPage,
                0,
            )
            .unwrap();
    }
    writer
        .write_packet(&[0x40, 1, b'a'], 2, PacketWriteEndInfo::EndStream, 1_000)
        .unwrap();
    writer
        .write_packet(
            vec![0; MAX_TAIL_BYTES as usize + 1],
            1,
            PacketWriteEndInfo::EndPage,
            50,
        )
        .unwrap();
    writer
        .write_packet(&[0x48, 1, 0], 1, PacketWriteEndInfo::EndStream, 74)
        .unwrap();
    drop(writer);
    let length = output.get_ref().len() as u64;
    let raw = parse(output.get_ref().clone());
    assert_eq!(raw.duration_seconds, Some(3.0));
    assert_eq!(
        crate::build::build_details(raw.clone()).duration_provenance,
        scryer_media_model::Provenance::Estimated
    );
    assert_eq!(
        raw.details.report.warnings[0].code,
        "ogg_earlier_stream_end_outside_tail"
    );
    assert!(length > MAX_TAIL_BYTES);
    struct Counted {
        cursor: Cursor<Vec<u8>>,
        bytes: u64,
    }
    impl Read for Counted {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let size = self.cursor.read(bytes)?;
            self.bytes += size as u64;
            Ok(size)
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
            self.cursor.seek(offset)
        }
    }
    impl crate::source::MediaSource for Counted {
        fn len(&self) -> u64 {
            self.cursor.get_ref().len() as u64
        }
    }
    let mut counted = Counted {
        cursor: output,
        bytes: 0,
    };
    let ends = read_stream_ends(&mut counted, length).unwrap();
    assert!(ends.ends.contains_key(&1));
    assert!(!ends.ends.contains_key(&2));
    assert!(counted.bytes <= MAX_TAIL_BYTES);
}

#[test]
fn ogm_sniffing_wins_over_extensions_and_audio_only_is_not_video() {
    for extension in ["ogm", "OGM", "", "mkv", "dat"] {
        let analysis = crate::probe_bounded(
            &mut Cursor::new(fixture::video_audio_text()),
            extension,
            &crate::ProbeOptions::new().with_depth(crate::ProbeDepth::Quick),
        )
        .unwrap();
        assert_eq!(analysis.container_format.as_deref(), Some("ogm"));
        assert_eq!(crate::test_facts::video_codec(&analysis), Some("mpeg4"));
        let subtitle_codecs: Vec<_> = analysis
            .program_streams(scryer_media_model::StreamKind::Subtitle)
            .map(|stream| stream.codec.as_ref().map(scryer_media_model::Codec::as_str))
            .collect();
        assert_eq!(subtitle_codecs, [Some("text")]);
        assert!(crate::test_facts::has_video_with_duration(&analysis));
    }
    let analysis = crate::probe_bounded(
        &mut Cursor::new(fixture::audio_only()),
        "ogm",
        &crate::ProbeOptions::new().with_depth(crate::ProbeDepth::Quick),
    )
    .unwrap();
    assert!(!crate::test_facts::has_video_with_duration(&analysis));
}

#[test]
fn ogm_multiple_audio_streams_with_interleaved_headers_are_discovered() {
    use ogg::{PacketWriteEndInfo, PacketWriter};
    let mut output = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new(&mut output);
    let video = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
    let audio = fixture::modern_header(b"audio\0\0\0", b"2000", 10_000_000, 48_000);
    writer
        .write_packet(&video, 1, PacketWriteEndInfo::EndPage, 0)
        .unwrap();
    for serial in [2, 3] {
        writer
            .write_packet(&audio, serial, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
    }
    writer
        .write_packet(fixture::comments(&[]), 1, PacketWriteEndInfo::EndPage, 0)
        .unwrap();
    for (serial, language) in [(2, "LANGUAGE=eng"), (3, "LANGUAGE=jpn")] {
        writer
            .write_packet(
                fixture::comments(&[language]),
                serial,
                PacketWriteEndInfo::EndPage,
                0,
            )
            .unwrap();
    }
    writer
        .write_packet(&[0x48, 1, 0], 1, PacketWriteEndInfo::EndStream, 74)
        .unwrap();
    for serial in [2, 3] {
        writer
            .write_packet(
                &[0x80, 0, 6, 0],
                serial,
                PacketWriteEndInfo::EndStream,
                142_464,
            )
            .unwrap();
    }
    drop(writer);
    let details = crate::build::build_details(parse(output.into_inner()));
    let languages: Vec<_> = details
        .program_streams(scryer_media_model::StreamKind::Audio)
        .map(|stream| stream.language.as_deref())
        .collect();
    assert_eq!(languages, [Some("eng"), Some("jpn")]);
}

#[test]
fn ordinary_ogg_theora_and_opus_keep_their_granule_meaning() {
    let mut source =
        Cursor::new(include_bytes!("../../../tests/media/ogv_theora_vorbis.ogv").as_slice());
    let raw = parse_ogg_source(&mut source).unwrap();
    assert_eq!(raw.format_name, "ogg");
    assert_eq!(
        raw.tracks[0]
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("theora")
    );
    assert_eq!(
        raw.tracks[1]
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("vorbis")
    );
    assert!(
        raw.duration_seconds
            .is_some_and(|duration| (duration - 1.4).abs() < 0.001)
    );
    use ogg::{PacketWriteEndInfo, PacketWriter};
    let mut output = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new(&mut output);
    writer
        .write_packet(
            b"OpusHead\x01\x02\0\0\x80\xbb\0\0\0\0\0",
            9,
            PacketWriteEndInfo::EndPage,
            0,
        )
        .unwrap();
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&[0; 8]);
    writer
        .write_packet(&tags, 9, PacketWriteEndInfo::EndPage, 0)
        .unwrap();
    writer
        .write_packet(
            &[0xf8, 0xff, 0xfe],
            9,
            PacketWriteEndInfo::EndStream,
            144_000,
        )
        .unwrap();
    drop(writer);
    let raw = parse(output.into_inner());
    assert_eq!(raw.format_name, "ogg");
    assert_eq!(raw.duration_seconds, Some(3.0));
    assert_eq!(
        raw.tracks[0]
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("opus")
    );
    assert_eq!(raw.tracks[0].channels, Some(2));
}

#[test]
fn theora_header_preserves_rationals_and_rejects_reserved_fields() {
    let fixture = include_bytes!("../../../tests/media/ogv_theora_vorbis.ogv");
    let offset = fixture
        .windows(7)
        .position(|bytes| bytes == b"\x80theora")
        .unwrap();
    let mut header = fixture[offset..offset + 42].to_vec();
    let (track, _, _, _) = parse_theora_identification(&header).unwrap();
    assert_eq!(
        track.metadata.declared_frame_rate,
        scryer_media_model::Rational::new(15, 1)
    );
    assert_eq!(
        track.metadata.sample_aspect_ratio,
        scryer_media_model::Rational::new(1, 1)
    );
    assert_eq!(
        track.metadata.display_aspect_ratio,
        scryer_media_model::Rational::new(20, 11)
    );
    assert_eq!(track.metadata.bit_depth, Some(8));
    assert_eq!(track.metadata.pixel_format.as_deref(), Some("yuv420p"));
    header[30..36].fill(0);
    assert!(
        parse_theora_identification(&header)
            .unwrap()
            .0
            .metadata
            .sample_aspect_ratio
            .is_none()
    );
    header[41] = (header[41] & !0x18) | 0x10;
    assert_eq!(
        parse_theora_identification(&header)
            .unwrap()
            .0
            .metadata
            .pixel_format
            .as_deref(),
        Some("yuv422p")
    );
    header[41] = (header[41] & !0x18) | 0x08;
    assert!(parse_theora_identification(&header).is_err());
    header[41] &= !0x18;
    header[22..26].fill(0);
    assert!(
        parse_theora_identification(&header).is_err(),
        "invalid frame rate must not become 25 fps"
    );
}

#[test]
fn parses_opus_identification() {
    let mut header = b"OpusHead\x01\x02\x38\x01\x80\xbb\0\0\0\0\0".to_vec();
    header.truncate(19);
    let track = parse_opus_identification(&header).unwrap();
    assert_eq!(
        track
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("opus")
    );
    assert_eq!(track.channels, Some(2));
}

#[test]
fn parses_language_comment_case_insensitively() {
    let mut comments = Vec::new();
    comments.extend_from_slice(&3_u32.to_le_bytes());
    comments.extend_from_slice(b"lib");
    comments.extend_from_slice(&1_u32.to_le_bytes());
    comments.extend_from_slice(&12_u32.to_le_bytes());
    comments.extend_from_slice(b"Language=eng");
    assert_eq!(
        parse_comments(&comments).unwrap().language.as_deref(),
        Some("eng")
    );
}

#[test]
fn bounds_continued_packets_before_completion() {
    use ogg::{PacketWriteEndInfo, PacketWriter};
    use std::io::Cursor;

    let packet = vec![0_u8; MAX_HEADER_BYTES + 1];
    let mut cursor = Cursor::new(Vec::new());
    PacketWriter::new(&mut cursor)
        .write_packet(&packet, 7, PacketWriteEndInfo::EndStream, 0)
        .unwrap();
    cursor.set_position(0);

    let mut reader = PacketReader::new(HeaderReader::new(cursor, MAX_HEADER_READ_BYTES));
    assert!(!matches!(reader.read_packet(), Ok(Some(_))));
}

#[test]
fn comments_become_stream_tags_without_encoded_pictures() {
    let entries: [&[u8]; 4] = [
        b"TITLE=Synthetic Theme",
        b"Encoder=synthetic-enc 2",
        b"METADATA_BLOCK_PICTURE=AAAAAwAAAAlpbWFnZS9wbmc=",
        b"NOSEPARATOR",
    ];
    let mut comments = Vec::new();
    comments.extend_from_slice(&3_u32.to_le_bytes());
    comments.extend_from_slice(b"lib");
    comments.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in entries {
        comments.extend_from_slice(&(entry.len() as u32).to_le_bytes());
        comments.extend_from_slice(entry);
    }
    let mut track = RawTrack::default();
    let mut report = scryer_media_model::ProbeReport::default();
    apply_comments(&mut track, &comments, &mut report).unwrap();
    assert_eq!(
        track.metadata.tags.into_iter().collect::<Vec<_>>(),
        [
            ("Encoder".to_owned(), "synthetic-enc 2".to_owned()),
            ("TITLE".to_owned(), "Synthetic Theme".to_owned()),
        ]
    );
    assert_eq!(track.name.as_deref(), Some("Synthetic Theme"));
    assert!(report.warnings.is_empty());
}
