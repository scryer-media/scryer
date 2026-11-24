use super::*;
use scryer_media_model::Codec;

/// Lightweight counters for parser I/O during bounded probing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ProbeStats {
    pub bytes_read: u64,
    pub seeks: u64,
}

/// Read/seek wrapper that counts the parser's I/O.
struct TrackedReader<R> {
    inner: R,
    stats: ProbeStats,
}

impl<R> TrackedReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            stats: ProbeStats::default(),
        }
    }

    fn stats(&self) -> ProbeStats {
        self.stats
    }
}

impl<R: Read> Read for TrackedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.stats.bytes_read = self.stats.bytes_read.saturating_add(read as u64);
        Ok(read)
    }
}

impl<R: Seek> Seek for TrackedReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let next = self.inner.seek(pos)?;
        self.stats.seeks = self.stats.seeks.saturating_add(1);
        Ok(next)
    }
}

#[test]
fn tracked_reader_counts_reads_and_seeks() {
    let mut reader = TrackedReader::new(std::io::Cursor::new(vec![1_u8, 2, 3, 4]));
    let mut buf = [0_u8; 2];
    reader.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, &[1, 2]);

    reader.seek(SeekFrom::Start(1)).unwrap();
    reader.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, &[2, 3]);

    let stats = reader.stats();
    assert_eq!(stats.bytes_read, 4);
    assert_eq!(stats.seeks, 1);
}

fn timing_track(entries: &[(u32, u32)]) -> mp4parse::Track {
    let mut track = mp4parse::Track {
        timescale: Some(TrackTimeScale(1000, 0)),
        stts: Some(mp4parse::TimeToSampleBox {
            samples: Default::default(),
        }),
        ..Default::default()
    };
    for &(sample_count, sample_delta) in entries {
        track
            .stts
            .as_mut()
            .unwrap()
            .samples
            .push(mp4parse::Sample {
                sample_count,
                sample_delta,
            })
            .unwrap();
    }
    track
}

#[test]
fn sample_bitrate_uses_matching_tables_and_ignores_presentation_duration() {
    let mut track = sample_track();
    assert_eq!(accounted_track_bitrate(&track), Some(800));
    track.duration = Some(mp4parse::TrackScaledTime(1, 0));
    assert_eq!(accounted_track_bitrate(&track), Some(800));
    track.stts.as_mut().unwrap().samples[0].sample_count = 2;
    assert_eq!(accounted_track_bitrate(&track), None);
    track.stts.as_mut().unwrap().samples[0].sample_count = 3;
    track.stts.as_mut().unwrap().samples[0].sample_delta = 0;
    assert_eq!(accounted_track_bitrate(&track), None);
    track
        .stts
        .as_mut()
        .unwrap()
        .samples
        .push(mp4parse::Sample {
            sample_count: 0,
            sample_delta: 40,
        })
        .unwrap();
    assert_eq!(accounted_track_bitrate(&track), None);
}

#[test]
fn uniform_sample_timing_retains_declarations_without_claiming_observations() {
    use scryer_media_model::Rational;
    assert_eq!(
        uniform_sample_frame_rate(&timing_track(&[(1, 1000)])),
        Rational::new(1, 1)
    );
    assert_eq!(
        sample_timing_facts(&timing_track(&[(1, 1000)])),
        (None, None)
    );
    assert_eq!(
        uniform_sample_frame_rate(&timing_track(&[(3, 40), (2, 40)])),
        Rational::new(25, 1)
    );
    for entries in [
        &[][..],
        &[(0, 40)][..],
        &[(1, 0)][..],
        &[(1, 40), (1, 60)][..],
    ] {
        assert!(uniform_sample_frame_rate(&timing_track(entries)).is_none());
    }
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/hevc_hdr10plus.mp4");
    let analysis = crate::probe(&path).unwrap();
    let video = analysis
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_model::StreamKind::Video)
        .unwrap();
    assert_eq!(video.metadata.declared_frame_rate, Rational::new(1, 1));
    assert!(video.metadata.observed_frame_rate.is_none());
    assert!(video.metadata.variable_frame_rate.is_none());
}

#[test]
fn sample_timing_handles_reordering_vfr_and_bounded_unknowns() {
    use scryer_media_model::Rational;
    let constant = timing_track(&[(4, 40)]);
    assert_eq!(
        sample_timing_facts(&constant),
        (Rational::new(25, 1), Some(false))
    );
    assert_eq!(
        sample_timing_facts(&timing_track(&[(3, 40), (2, 60)])).1,
        Some(true)
    );
    let mut reordered = timing_track(&[(4, 40)]);
    reordered.ctts = Some(mp4parse::CompositionOffsetBox {
        samples: Default::default(),
    });
    for offset in [0, 80, -40, -40] {
        reordered
            .ctts
            .as_mut()
            .unwrap()
            .samples
            .push(mp4parse::TimeOffset {
                sample_count: 1,
                time_offset: mp4parse::TimeOffsetVersion::Version1(offset),
            })
            .unwrap();
    }
    assert_eq!(
        sample_timing_facts(&reordered),
        (Rational::new(25, 1), Some(false))
    );
    let mut bounded = timing_track(&[(10_000, 40)]);
    bounded.ctts = Some(mp4parse::CompositionOffsetBox {
        samples: Default::default(),
    });
    bounded
        .ctts
        .as_mut()
        .unwrap()
        .samples
        .push(mp4parse::TimeOffset {
            sample_count: 10_000,
            time_offset: mp4parse::TimeOffsetVersion::Version0(0),
        })
        .unwrap();
    assert_eq!(sample_timing_facts(&bounded), (Rational::new(25, 1), None));
    assert_eq!(sample_timing_facts(&timing_track(&[(4, 0)])), (None, None));
}

#[test]
fn partial_fragment_walk_preserves_headers_without_using_partial_timing() {
    let mut bytes = include_bytes!("../../../tests/media/hevc_hdr10plus.mp4").to_vec();
    let baseline =
        parse_mp4_source(&mut Cursor::new(bytes.clone()), "mp4", ProbeDepth::Full).unwrap();
    for index in 0..200_u32 {
        let tfhd = [
            0x18_u32.to_be_bytes(),
            1_u32.to_be_bytes(),
            1_000_000_u32.to_be_bytes(),
            10_000_u32.to_be_bytes(),
        ]
        .concat();
        let tfdt = [0_u32.to_be_bytes(), (index * 1_000_000).to_be_bytes()].concat();
        let trun = [0_u32.to_be_bytes(), 1_u32.to_be_bytes()].concat();
        let traf = [
            make_box(b"tfhd", &tfhd),
            make_box(b"tfdt", &tfdt),
            make_box(b"trun", &trun),
        ]
        .concat();
        bytes.extend_from_slice(&make_box(b"moof", &make_box(b"traf", &traf)));
    }
    let raw = parse_mp4_source(&mut Cursor::new(bytes), "mp4", ProbeDepth::Full).unwrap();
    assert!(raw.details.report.budget_exhausted);
    assert_eq!(raw.duration_seconds, baseline.duration_seconds);
    assert_eq!(raw.tracks[0].codec_name, baseline.tracks[0].codec_name);
    assert_eq!(raw.tracks[0].bit_rate_bps, baseline.tracks[0].bit_rate_bps);
}

#[test]
fn fragment_budget_exhaustion_is_visible_in_outer_report() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/hevc_hdr10plus.mp4");
    let mut bytes = std::fs::read(path).unwrap();
    let mut tfhd = vec![0; 4];
    tfhd.extend_from_slice(&1_u32.to_be_bytes());
    let mut trun = vec![0; 4];
    trun.extend_from_slice(&4_000_001_u32.to_be_bytes());
    let mut traf = make_box(b"tfhd", &tfhd);
    traf.extend_from_slice(&make_box(b"trun", &trun));
    bytes.extend_from_slice(&make_box(b"moof", &make_box(b"traf", &traf)));

    let container = parse_mp4_source(&mut Cursor::new(bytes), "mp4", ProbeDepth::Full).unwrap();
    let report = container.details.report;
    assert_eq!(report.status, scryer_media_model::ProbeStatus::Incomplete);
    assert!(report.budget_exhausted);
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].code, "mp4_fragment_inventory_budget");
}

fn sample_track() -> mp4parse::Track {
    let mut track = timing_track(&[(3, 40)]);
    track.track_id = Some(1);
    track.stsz = Some(mp4parse::SampleSizeBox {
        sample_size: 0,
        sample_sizes: Default::default(),
    });
    for size in [3, 4, 5] {
        track
            .stsz
            .as_mut()
            .unwrap()
            .sample_sizes
            .push(size)
            .unwrap();
    }
    track.stco = Some(mp4parse::ChunkOffsetBox {
        offsets: Default::default(),
    });
    for offset in [100, 500] {
        track.stco.as_mut().unwrap().offsets.push(offset).unwrap();
    }
    track.stsc = Some(mp4parse::SampleToChunkBox {
        samples: Default::default(),
    });
    for (first_chunk, samples_per_chunk) in [(1, 2), (2, 1)] {
        track
            .stsc
            .as_mut()
            .unwrap()
            .samples
            .push(mp4parse::SampleToChunk {
                first_chunk,
                samples_per_chunk,
                sample_description_index: 1,
            })
            .unwrap();
    }
    track
}

#[test]
fn sample_ranges_follow_chunk_boundaries_and_reject_overflow() {
    let mut track = sample_track();
    assert_eq!(
        track_sample_ranges(&track),
        Some((vec![(100, 3), (103, 4), (500, 5)], true))
    );
    track.stco.as_mut().unwrap().offsets[0] = u64::MAX;
    assert!(track_sample_ranges(&track).is_none());
    let mut track = sample_track();
    track.stsc.as_mut().unwrap().samples[0].samples_per_chunk = 0;
    assert!(track_sample_ranges(&track).is_none());
}

#[test]
fn av1_mp4_enrichment_reads_metadata_after_the_first_sample() {
    let mut track = sample_track();
    track.stsz.as_mut().unwrap().sample_sizes[0] = 2;
    track.stsz.as_mut().unwrap().sample_sizes[1] = 8;
    track.stsz.as_mut().unwrap().sample_sizes[2] = 2;
    let mut bytes = vec![0; 502];
    bytes[100..102].copy_from_slice(&[18, 0]);
    bytes[102..110].copy_from_slice(&[42, 6, 1, 3, 232, 1, 144, 128]);
    bytes[500..502].copy_from_slice(&[18, 0]);
    let mut context = MediaContext::default();
    context.tracks.push(track).unwrap();
    let mut tracks = vec![ParsedMp4Track {
        track_id: Some(1),
        raw: RawTrack {
            kind: TrackKind::Video,
            codec_name: Some(Codec::Av1),
            ..Default::default()
        },
    }];
    let mut report = scryer_media_model::ProbeReport::default();
    scan_mp4_sample_probes(
        &mut Cursor::new(bytes),
        &context,
        &mut tracks,
        ProbeDepth::Full,
        &mut report,
    );
    assert_eq!(
        tracks[0]
            .raw
            .metadata
            .color
            .content_light
            .as_ref()
            .unwrap()
            .max_cll,
        Some(1000)
    );
    assert!(report.warnings.is_empty());
}

#[test]
fn rotation_rejects_reflections_and_preserves_clockwise_sign() {
    let matrix = mp4parse::Matrix {
        a: 0,
        b: 65536,
        c: -65536,
        d: 0,
        u: 0,
        v: 0,
        w: 1 << 30,
        x: 0,
        y: 0,
    };
    assert_eq!(matrix_rotation(&matrix), Some(-90.0));
    assert_eq!(
        matrix_rotation(&mp4parse::Matrix { c: 65536, ..matrix }),
        None
    );
}

fn make_box(name: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = (8 + payload.len()) as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(payload);
    out
}

#[test]
fn stereo_box_is_detected_only_for_supported_video_declarations() {
    for (payload, expected) in [
        (vec![0, 0, 0, 0, 0], false),
        (vec![0, 0, 0, 0, 1], true),
        (vec![0, 0, 0, 0, 2], true),
        (vec![0, 0, 0, 0, 3], true),
        (vec![0, 0, 0, 0, 4], true),
        (vec![0, 0, 0, 0, 5], false),
        (vec![1, 0, 0, 0, 1], false),
        (vec![0, 0, 0, 0], false),
    ] {
        let mut entry = vec![0_u8; 78];
        entry.extend(make_box(b"st3d", &payload));
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend(make_box(b"avc1", &entry));
        let mut track = Mp4TrackMetadata::default();
        parse_stsd(&stsd, &mut track);
        assert_eq!(track.details.is_3d, expected, "{payload:?}");
        stsd.pop();
        let mut truncated = Mp4TrackMetadata::default();
        parse_stsd(&stsd, &mut truncated);
        assert!(!truncated.details.is_3d);
    }
}

#[test]
fn every_dolby_vision_configuration_box_yields_the_record() {
    for (name, record) in [
        (b"dvcC", [1, 0, 0x0E, 0x37, 0x60]),
        (b"dvvC", [1, 0, 0x10, 0x35, 0x10]),
        (b"dvwC", [1, 0, 0x28, 0x35, 0x10]),
    ] {
        let mut entry = vec![0_u8; 78];
        entry.extend(make_box(name, &record));
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend(make_box(b"hvc1", &entry));
        let mut track = Mp4TrackMetadata::default();
        parse_stsd(&stsd, &mut track);
        assert_eq!(
            track.dovi_config.as_deref(),
            Some(&record[..]),
            "{}",
            String::from_utf8_lossy(name)
        );
    }
}

#[test]
fn dolby_vision_box_too_short_to_parse_is_not_stored() {
    let parse = |boxes: &[Vec<u8>]| {
        let mut entry = vec![0_u8; 78];
        entry.extend(boxes.concat());
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend(make_box(b"hvc1", &entry));
        let mut track = Mp4TrackMetadata::default();
        parse_stsd(&stsd, &mut track);
        track.dovi_config
    };
    let valid = [1, 0, 0x10, 0x35, 0x10];
    assert_eq!(parse(&[make_box(b"dvvC", &[1, 0, 0x10])]), None);
    assert_eq!(
        parse(&[make_box(b"dvvC", &valid), make_box(b"dvwC", &[1, 0, 0x28])]).as_deref(),
        Some(&valid[..])
    );
}

fn make_meta_box(payload: &[u8]) -> Vec<u8> {
    let mut full_box = vec![0_u8; 4];
    full_box.extend_from_slice(payload);
    make_box(b"meta", &full_box)
}

fn make_hdlr_box(
    pre_defined: u32,
    handler_type: &[u8; 4],
    reserved: [u32; 3],
    name: &[u8],
) -> Vec<u8> {
    let mut payload = vec![0_u8; 4];
    payload.extend_from_slice(&pre_defined.to_be_bytes());
    payload.extend_from_slice(handler_type);
    for value in reserved {
        payload.extend_from_slice(&value.to_be_bytes());
    }
    payload.extend_from_slice(name);
    make_box(b"hdlr", &payload)
}

#[test]
fn codec_type_to_fourcc_roundtrips() {
    assert_eq!(codec_type_to_fourcc(CodecType::H264), "avc1");
    assert_eq!(codec_type_to_fourcc(CodecType::AV1), "av01");
    assert_eq!(codec_type_to_fourcc(CodecType::VP9), "vp09");
    assert_eq!(codec_type_to_fourcc(CodecType::AAC), "mp4a");
    assert_eq!(codec_type_to_fourcc(CodecType::FLAC), "fLaC");
    assert_eq!(codec_type_to_fourcc(CodecType::Opus), "Opus");
    assert_eq!(codec_type_to_fourcc(CodecType::Unknown), "unknown");
}

#[test]
fn normalize_mp4_codecs() {
    assert_eq!(
        normalize_codec_name("avc1")
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("h264")
    );
    assert_eq!(
        normalize_codec_name("av01")
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("av1")
    );
    assert_eq!(
        normalize_codec_name("mp4a")
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("aac")
    );
    assert_eq!(
        normalize_codec_name("fLaC")
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("flac")
    );
    assert_eq!(
        normalize_codec_name("Opus")
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("opus")
    );
    assert_eq!(normalize_codec_name("unknown"), None);
}

#[test]
fn box_walk_is_independent_of_fragment_count() {
    for count in [1_000, 100_000] {
        let file = make_box(b"free", &[]).repeat(count);
        let mut reader = TrackedReader::new(Cursor::new(file));
        let (_, length, exhausted) = prepare_mp4_metadata_from_reader(&mut reader).unwrap();
        assert!(exhausted);
        assert_eq!(length, (count * 8) as u64);
        assert!(reader.stats().bytes_read <= 128 * 8);
        assert!(reader.stats().seeks <= 260);
        let raw = parse_mp4_source(
            &mut Cursor::new(make_box(b"free", &[]).repeat(count)),
            "mp4",
            ProbeDepth::Full,
        )
        .unwrap();
        assert!(raw.details.report.budget_exhausted);
        assert_eq!(raw.duration_seconds, None);
    }
}

#[test]
fn metadata_copy_seeks_over_mdat_payload() {
    let ftyp = make_box(b"ftyp", b"isom\0\0\0\0isom");
    let mdat_payload = vec![0_u8; 1024 * 1024];
    let mdat = make_box(b"mdat", &mdat_payload);
    let moov = make_box(b"moov", &make_box(b"mvhd", &[0_u8; 32]));
    let file = [ftyp.clone(), mdat.clone(), moov.clone()].concat();

    let mut reader = TrackedReader::new(Cursor::new(file));
    let (metadata, file_len_hint, exhausted) =
        prepare_mp4_metadata_from_reader(&mut reader).unwrap();
    assert!(!exhausted);
    let stats = reader.stats();

    assert_eq!(metadata, [ftyp, moov].concat());
    assert_eq!(file_len_hint, metadata.len() as u64 + mdat.len() as u64);
    assert!(
        stats.bytes_read < 512,
        "unexpected payload read: {:?}",
        stats
    );
    assert!(stats.seeks >= 1, "expected explicit mdat skip: {:?}", stats);
}

#[test]
fn metadata_copy_rejects_truncated_kept_box_before_allocating() {
    let file = [
        &(MP4_KEEP_BOX_MAX_BYTES as u32).to_be_bytes(),
        b"moov".as_slice(),
    ]
    .concat();
    let mut reader = TrackedReader::new(Cursor::new(file));

    let error = prepare_mp4_metadata_from_reader(&mut reader)
        .expect_err("truncated moov must be rejected before allocation");
    let ProbeError::Malformed(message) = error else {
        panic!("expected parse error for truncated moov");
    };
    assert!(message.contains("extends past end of input"));
}

#[test]
fn sanitize_prepared_metadata_stops_at_depth_limit() {
    const DEPTH: usize = 65_000;
    const FIXTURE_BYTES: usize = 2_665_000;

    let hdlr = make_box(b"hdlr", &[0_u8; 24]);
    let free_payload_len = FIXTURE_BYTES - DEPTH * 8 - hdlr.len() - 8;
    let mut metadata = vec![0_u8; FIXTURE_BYTES];

    for level in 0..DEPTH {
        let offset = level * 8;
        let size = u32::try_from(FIXTURE_BYTES - offset).unwrap();
        metadata[offset..offset + 4].copy_from_slice(&size.to_be_bytes());
        metadata[offset + 4..offset + 8].copy_from_slice(b"moov");
    }

    let hdlr_start = DEPTH * 8;
    metadata[hdlr_start..hdlr_start + hdlr.len()].copy_from_slice(&hdlr);
    let free_start = hdlr_start + hdlr.len();
    let free_size = u32::try_from(free_payload_len + 8).unwrap();
    metadata[free_start..free_start + 4].copy_from_slice(&free_size.to_be_bytes());
    metadata[free_start + 4..free_start + 8].copy_from_slice(b"free");

    sanitize_prepared_mp4_metadata(&mut metadata);

    assert_eq!(metadata.len(), FIXTURE_BYTES);
}

#[test]
fn sanitize_prepared_metadata_repairs_malformed_hdlr_boxes() {
    let hdlr = make_hdlr_box(1, b"vide", [2, 3, 4], b"VideoHandler");
    let mdia = make_box(b"mdia", &hdlr);
    let trak = make_box(b"trak", &mdia);
    let mut moov = make_box(b"moov", &trak);

    let original_len = moov.len();
    let original_moov_size = read_box_header_from_bytes(&moov).unwrap().size;

    sanitize_prepared_mp4_metadata(&mut moov);

    assert_eq!(moov.len(), original_len + 1);
    assert_eq!(
        read_box_header_from_bytes(&moov).unwrap().size,
        original_moov_size + 1
    );

    let moov_header = read_box_header_from_bytes(&moov).unwrap();
    let trak = &moov[moov_header.header_size..moov_header.size as usize];
    let trak_header = read_box_header_from_bytes(trak).unwrap();
    let mdia = &trak[trak_header.header_size..trak_header.size as usize];
    let mdia_header = read_box_header_from_bytes(mdia).unwrap();
    let hdlr = &mdia[mdia_header.header_size..mdia_header.size as usize];
    let hdlr_header = read_box_header_from_bytes(hdlr).unwrap();
    let payload = &hdlr[hdlr_header.header_size..hdlr_header.size as usize];

    assert_eq!(&payload[4..8], &[0, 0, 0, 0]);
    assert_eq!(&payload[12..24], &[0; 12]);
    assert_eq!(payload.last().copied(), Some(0));
}

#[test]
fn parse_trak_extracts_default_and_forced_subtitle_flags() {
    let mut tkhd_payload = vec![0_u8; 24];
    tkhd_payload[3] = MOV_TKHD_FLAG_ENABLED as u8;
    tkhd_payload[12..16].copy_from_slice(&7_u32.to_be_bytes());

    let kind_payload = [
        [0_u8, 0, 0, 0].as_slice(),
        b"urn:mpeg:dash:role:2011\0forced-subtitle\0".as_slice(),
    ]
    .concat();

    let trak_payload = [
        make_box(b"tkhd", &tkhd_payload),
        make_box(b"udta", &make_box(b"kind", &kind_payload)),
    ]
    .concat();

    let track = parse_trak(&trak_payload);

    assert_eq!(track.track_id, Some(7));
    assert!(track.default_track);
    assert!(track.forced);
}

#[test]
fn parse_trak_ignores_kind_box_hidden_inside_leaf_payload() {
    let kind_payload = [
        [0_u8, 0, 0, 0].as_slice(),
        b"urn:mpeg:dash:role:2011\0forced-subtitle\0".as_slice(),
    ]
    .concat();

    let trak_payload = make_box(
        b"udta",
        &make_box(b"name", &make_box(b"kind", &kind_payload)),
    );

    let track = parse_trak(&trak_payload);

    assert!(!track.forced);
}

#[test]
fn parse_trak_stops_descending_past_max_depth() {
    let kind_payload = [
        [0_u8, 0, 0, 0].as_slice(),
        b"urn:mpeg:dash:role:2011\0forced-subtitle\0".as_slice(),
    ]
    .concat();

    let mut nested = make_box(b"kind", &kind_payload);
    for _ in 0..=MP4_BOX_MAX_DEPTH {
        nested = make_meta_box(&nested);
    }

    let track = parse_trak(&make_box(b"udta", &nested));

    assert!(!track.forced);
}

#[test]
fn extended_language_tags_override_legacy_tags_in_either_box_order() {
    let mut mdhd = vec![0; 24];
    mdhd[20..22].copy_from_slice(&((5_u16 << 10) | (14 << 5) | 7).to_be_bytes());
    let legacy = make_box(b"mdhd", &mdhd);
    let modern = make_box(b"elng", &[&[0, 0, 0, 0][..], b"zh-Hant-TW\0"].concat());
    for boxes in [[legacy.clone(), modern.clone()], [modern, legacy.clone()]] {
        let track = parse_trak(&make_box(b"mdia", &boxes.concat()));
        assert_eq!(track.language.as_deref(), Some("zh-Hant-TW"));
        assert_eq!(
            track.details.original_language.as_deref(),
            Some("zh-Hant-TW")
        );
        assert_eq!(
            track.details.language_provenance,
            scryer_media_model::Provenance::Container
        );
    }
    let invalid = make_box(b"elng", b"\0\0\0\0fr\0hidden");
    let track = parse_trak(&make_box(b"mdia", &[legacy, invalid].concat()));
    assert_eq!(track.language.as_deref(), Some("eng"));
}

#[test]
fn parse_chpl_counts_chapters() {
    let chpl_payload = [[0_u8, 0, 0, 0].as_slice(), &[3], &[0_u8; 27]].concat();
    let metadata = parse_mp4_chapter_metadata(&make_box(
        b"moov",
        &make_box(b"udta", &make_box(b"chpl", &chpl_payload)),
    ));

    assert_eq!(metadata.chpl_count, Some(3));
}

#[test]
fn parse_chpl_ignores_boxes_hidden_inside_ilst_metadata() {
    let chpl_payload = [[0_u8, 0, 0, 0].as_slice(), &[3], &[0_u8; 27]].concat();
    let metadata = parse_mp4_chapter_metadata(&make_box(
        b"moov",
        &make_box(
            b"udta",
            &make_meta_box(&make_box(b"ilst", &make_box(b"chpl", &chpl_payload))),
        ),
    ));

    assert_eq!(metadata.chpl_count, None);
}

#[test]
fn parse_chap_track_ids_reads_references() {
    let ids = parse_chap_track_ids(&[0, 0, 0, 7, 0, 0, 0, 9]);
    assert_eq!(ids, vec![7, 9]);
}

#[test]
fn parses_aac_audio_specific_config_channel_count() {
    assert_eq!(
        parse_aac_audio_specific_config_channels(&[0x11, 0xB0]),
        Some(6)
    );
}

#[test]
fn parses_ac3_and_eac3_channel_counts_from_codec_private() {
    assert_eq!(parse_dac3_channels(&[0x00, 0x3C, 0x00]), Some(6));
    assert_eq!(
        parse_dec3_channels(&[0x00, 0x00, 0x00, 0x0F, 0x00]),
        Some(6)
    );
}

#[test]
fn combines_codec_specific_audio_channels_with_sample_entry_defaults() {
    assert_eq!(
        mp4_audio_channels("mp4a", Some(&[0x11, 0xB0]), Some(2)),
        Some(6)
    );
    assert_eq!(
        mp4_audio_channels("mp4a", Some(&[0x13, 0x08]), Some(2)),
        Some(2)
    );
    assert_eq!(
        mp4_audio_channels("ac-3", Some(&[0x00, 0x3C, 0x00]), None),
        Some(6)
    );
}

#[test]
fn runtime_prefers_video_then_audio_then_format_duration() {
    assert_eq!(
        preferred_runtime(Some(1920.96), Some(1919.167), Some(1920.96)),
        Some(1919.167)
    );
    assert_eq!(
        preferred_runtime(Some(1517.0), Some(0.0), Some(1519.0)),
        Some(1517.0)
    );
    assert_eq!(preferred_runtime(None, None, Some(42.0)), Some(42.0));
}

#[test]
fn decodes_legacy_mdhd_english_language_code() {
    assert_eq!(decode_mdhd_language(0), Some("eng".to_string()));
    assert_eq!(decode_mdhd_language(6), Some("spa".to_string()));
    assert_eq!(decode_mdhd_language(11), Some("jpn".to_string()));
}
