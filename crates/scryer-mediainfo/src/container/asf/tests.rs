use super::*;

#[test]
fn stream_inventory_preserves_container_order_and_missing_language() {
    let mut objects = Vec::new();
    for id in [7_u16, 3] {
        let mut properties = vec![0; 54 + 16];
        properties[..16].copy_from_slice(&AUDIO_MEDIA_GUID);
        properties[40..44].copy_from_slice(&16_u32.to_le_bytes());
        properties[48..50].copy_from_slice(&id.to_le_bytes());
        properties[54..56].copy_from_slice(&1_u16.to_le_bytes());
        properties[56..58].copy_from_slice(&1_u16.to_le_bytes());
        properties[58..62].copy_from_slice(&48_000_u32.to_le_bytes());
        properties[62..66].copy_from_slice(&96_000_u32.to_le_bytes());
        properties[66..68].copy_from_slice(&2_u16.to_le_bytes());
        properties[68..70].copy_from_slice(&16_u16.to_le_bytes());
        objects.extend(STREAM_PROPERTIES_GUID);
        objects.extend((24 + properties.len() as u64).to_le_bytes());
        objects.extend(properties);
    }
    let mut image = Vec::from(ASF_HEADER_GUID);
    image.extend((30 + objects.len() as u64).to_le_bytes());
    image.extend(2_u32.to_le_bytes());
    image.extend([1, 2]);
    image.extend(objects);
    let container = parse_asf_source(&mut std::io::Cursor::new(image)).unwrap();
    assert_eq!(
        container
            .tracks
            .iter()
            .map(|track| track.metadata.id.as_deref())
            .collect::<Vec<_>>(),
        [Some("7"), Some("3")]
    );
    assert!(
        container
            .tracks
            .iter()
            .all(|track| track.language.is_none() && track.metadata.original_language.is_none())
    );
}

#[test]
fn wave_mp3_mono_layout_reaches_the_stream_details() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/wmv_wmv1_mp3_mono.wmv");
    let analysis = crate::probe(&path).unwrap();
    let audio = analysis
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_model::StreamKind::Audio)
        .unwrap();
    assert_eq!(
        audio.codec.as_ref().map(scryer_media_model::Codec::as_str),
        Some("mp3")
    );
    assert_eq!(audio.channels, Some(1));
    assert_eq!(audio.metadata.channel_layout.as_deref(), Some("mono"));
    assert_eq!(audio.metadata.original_language.as_deref(), Some("en"));
    assert_eq!(audio.language.as_deref(), Some("eng"));
    assert!(audio.metadata.sample_format.is_none());
}

#[test]
fn extensible_aac_uses_the_codec_configuration_after_the_wave_header() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/media/wmv_wmv1_aac_surround.wmv");
    let analysis = crate::probe(&path).unwrap();
    let audio = analysis
        .streams
        .iter()
        .find(|stream| stream.kind == scryer_media_model::StreamKind::Audio)
        .unwrap();
    assert_eq!(
        audio.codec.as_ref().map(scryer_media_model::Codec::as_str),
        Some("aac")
    );
    assert_eq!(audio.channels, Some(6));
    assert_eq!(audio.metadata.channel_layout.as_deref(), Some("5.1"));
    assert_eq!(audio.metadata.sample_rate, Some(48_000));
    assert_eq!(audio.metadata.profile.as_deref(), Some("LC"));
}

#[test]
fn legacy_windows_media_video_properties_reach_the_stream_details() {
    for fixture in ["wmv_wmv1_wmav1.wmv", "wmv_wmv2_video_only.wmv"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/media")
            .join(fixture);
        let analysis = crate::probe(&path).unwrap();
        let video = analysis
            .streams
            .iter()
            .find(|stream| stream.kind == scryer_media_model::StreamKind::Video)
            .unwrap();
        assert_eq!(video.metadata.bit_depth, Some(8), "{fixture}");
        assert_eq!(
            video.metadata.sample_aspect_ratio,
            scryer_media_model::Rational::new(1, 1),
            "{fixture}"
        );
        assert_eq!(
            video.metadata.display_aspect_ratio,
            scryer_media_model::Rational::new(20, 11),
            "{fixture}"
        );
        assert_eq!(
            video.metadata.pixel_format.as_deref(),
            Some("yuv420p"),
            "{fixture}"
        );
    }
}

fn marker_payload(entries: &[(u64, &str)]) -> Vec<u8> {
    let mut payload = vec![0; 16];
    payload.extend((entries.len() as u32).to_le_bytes());
    payload.extend([0; 4]);
    for (time, title) in entries {
        let units = title.encode_utf16().chain([0]).collect::<Vec<_>>();
        payload.extend(50_u64.to_le_bytes());
        payload.extend(time.to_le_bytes());
        payload.extend((12 + units.len() as u16 * 2).to_le_bytes());
        payload.extend([0; 8]);
        payload.extend((units.len() as u32).to_le_bytes());
        payload.extend(units.into_iter().flat_map(u16::to_le_bytes));
    }
    payload
}

fn asf_chapter_fixture(markers_first: bool) -> Vec<u8> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/wmv_wmv1_wmav1.wmv");
    let original = std::fs::read(path).unwrap();
    let header_size = u64::from_le_bytes(original[16..24].try_into().unwrap());
    let count = u32::from_le_bytes(original[24..28].try_into().unwrap());
    let markers = marker_payload(&[(31_000_000, "Début 🎬"), (32_250_000, "第二章")]);
    let object_size = 24 + markers.len() as u64;
    let insert_at = if markers_first {
        30
    } else {
        header_size as usize
    };
    let mut bytes = original[..insert_at].to_vec();
    bytes[16..24].copy_from_slice(&(header_size + object_size).to_le_bytes());
    bytes[24..28].copy_from_slice(&(count + 1).to_le_bytes());
    // The parser's own expectations also cover markers preceding the preroll declaration.
    bytes.extend(MARKER_GUID);
    bytes.extend(object_size.to_le_bytes());
    bytes.extend(markers);
    bytes.extend(&original[insert_at..]);
    let file_size = bytes.len() as u64;
    let size_at = 70
        + if markers_first {
            object_size as usize
        } else {
            0
        };
    bytes[size_at..size_at + 8].copy_from_slice(&file_size.to_le_bytes());
    bytes
}

#[test]
fn markers_preserve_unicode_titles_and_precise_preroll_adjusted_times() {
    let analysis = crate::probe_bounded(
        &mut std::io::Cursor::new(asf_chapter_fixture(true)),
        "wmv",
        &crate::ProbeOptions::new(),
    )
    .unwrap();
    assert_eq!(analysis.chapter_count, Some(2));
    assert_eq!(analysis.chapters.len(), 2);
    assert_eq!(analysis.chapters[0].title.as_deref(), Some("Début 🎬"));
    assert_eq!(analysis.chapters[0].start_seconds, 0.0);
    assert_eq!(analysis.chapters[1].title.as_deref(), Some("第二章"));
    assert!((analysis.chapters[1].start_seconds - 0.125).abs() < 1e-9);
    assert!(
        analysis
            .chapters
            .iter()
            .all(|chapter| chapter.end_seconds.is_none())
    );
}

#[test]
fn marker_inventory_rejects_truncated_descriptions_and_bounds_retained_records() {
    let payload = marker_payload(&[(31_000_000, "Unicode 🎬")]);
    for end in 0..payload.len() {
        assert!(parse_markers(&payload[..end], &mut AsfState::default()).is_err());
    }
    let entries = vec![(31_000_000, ""); 4097];
    let mut state = AsfState::default();
    parse_markers(&marker_payload(&entries), &mut state).unwrap();
    assert_eq!(state.chapters, Some(4097));
    assert_eq!(state.markers.len(), 4096);
    assert!(state.marker_inventory_incomplete);
    let mut invalid_text = payload.clone();
    let length = invalid_text.len();
    invalid_text[length - 2..].copy_from_slice(&0xd800_u16.to_le_bytes());
    let mut state = AsfState::default();
    parse_markers(&invalid_text, &mut state).unwrap();
    assert!(state.marker_inventory_incomplete);
    assert!(state.markers[0].1.is_none());
}

#[test]
#[ignore = "development reference requires FFprobe"]
fn asf_marker_chapter_reference_matches_ffprobe() {
    let version = std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .unwrap();
    assert!(version.status.success());
    eprintln!("{}", String::from_utf8_lossy(&version.stdout));
    // FFprobe 8.1.1 applies preroll while reading each marker; it does not
    // revisit markers appearing before File Properties. Compare its usual
    // object ordering, while the always-run parser test covers the reverse.
    let bytes = asf_chapter_fixture(false);
    let mut child = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_chapters",
            "-of",
            "json",
            "-i",
            "pipe:0",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), &bytes).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reference: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let parsed = crate::probe_bounded(
        &mut std::io::Cursor::new(bytes),
        "wmv",
        &crate::ProbeOptions::new(),
    )
    .unwrap();
    let chapters = reference["chapters"].as_array().unwrap();
    assert_eq!(chapters.len(), parsed.chapters.len());
    for (reference, chapter) in chapters.iter().zip(&parsed.chapters) {
        assert_eq!(
            reference["tags"]["title"].as_str(),
            chapter.title.as_deref()
        );
        let start = reference["start_time"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!(
            (chapter.start_seconds - start).abs() < 1e-6,
            "parsed={} reference={start}",
            chapter.start_seconds
        );
    }
}

#[test]
fn pixel_aspect_attributes_are_stream_specific_and_conflict_aware() {
    fn record(stream: u16, name: &str, value: u32) -> Vec<u8> {
        let name = name
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let mut bytes = 1_u16.to_le_bytes().to_vec();
        bytes.extend(0_u16.to_le_bytes());
        bytes.extend(stream.to_le_bytes());
        bytes.extend((name.len() as u16).to_le_bytes());
        bytes.extend(3_u16.to_le_bytes());
        bytes.extend(4_u32.to_le_bytes());
        bytes.extend(name);
        bytes.extend(value.to_le_bytes());
        bytes
    }
    let mut state = AsfState::default();
    let x = record(7, "AspectRatioX", 12);
    for end in 0..x.len() {
        assert!(parse_stream_metadata(&x[..end], &mut AsfState::default()).is_err());
    }
    parse_object(METADATA_GUID, &x, &mut state).unwrap();
    parse_object(
        METADATA_LIBRARY_GUID,
        &record(7, "AspectRatioY", 11),
        &mut state,
    )
    .unwrap();
    parse_stream_metadata(&record(3, "AspectRatioX", 1), &mut state).unwrap();
    assert_eq!(state.streams[&7].aspect_x, Some(12));
    assert_eq!(state.streams[&7].aspect_y, Some(11));
    assert!(state.streams[&3].aspect_y.is_none());
    assert!(!state.streams[&7].aspect_conflict);
    parse_stream_metadata(&x, &mut state).unwrap();
    assert!(!state.streams[&7].aspect_conflict);
    parse_stream_metadata(&record(7, "AspectRatioX", 16), &mut state).unwrap();
    assert!(state.streams[&7].aspect_conflict);
}

#[test]
fn invalid_or_missing_pixel_aspect_does_not_become_square_pixels() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/wmv_wmv1_wmav1.wmv");
    let original = std::fs::read(path).unwrap();
    let positions = ["AspectRatioX", "AspectRatioY"].map(|name| {
        let name = name
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let start = original
            .windows(name.len())
            .position(|bytes| bytes == name)
            .unwrap();
        (start, start + name.len())
    });
    for (x, y, incomplete) in [
        (Some(4_u32), Some(3_u32), false),
        (Some(0), Some(1), true),
        (None, Some(1), true),
        (None, None, false),
    ] {
        let mut bytes = original.clone();
        for ((name_at, value_at), value) in positions.into_iter().zip([x, y]) {
            if let Some(value) = value {
                bytes[value_at..value_at + 4].copy_from_slice(&value.to_le_bytes());
            } else {
                bytes[name_at] = b'Z';
            }
        }
        let parsed = parse_asf_source(&mut std::io::Cursor::new(bytes)).unwrap();
        let video = parsed
            .tracks
            .iter()
            .find(|track| track.kind == TrackKind::Video)
            .unwrap();
        if x == Some(4) {
            assert_eq!(
                video.metadata.sample_aspect_ratio,
                scryer_media_model::Rational::new(4, 3)
            );
            assert_eq!(
                video.metadata.display_aspect_ratio,
                scryer_media_model::Rational::new(80, 33)
            );
        } else {
            assert!(video.metadata.sample_aspect_ratio.is_none());
            assert!(video.metadata.display_aspect_ratio.is_none());
        }
        assert_eq!(
            parsed.details.report.status == scryer_media_model::ProbeStatus::Incomplete,
            incomplete
        );
    }
}

#[test]
fn maps_windows_media_codecs() {
    assert_eq!(
        map_video_fourcc(*b"WMV3"),
        Some(scryer_media_model::Codec::Wmv3)
    );
    assert_eq!(
        map_video_fourcc(*b"WVC1"),
        Some(scryer_media_model::Codec::Vc1)
    );
    assert_eq!(
        map_audio_format_tag(0x0162, 0),
        Some(scryer_media_model::Codec::WmaPro)
    );
    assert_eq!(
        map_audio_format_tag(0x0163, 0),
        Some(scryer_media_model::Codec::WmaLossless)
    );
    assert_eq!(
        map_audio_format_tag(0x0001, 8),
        Some(scryer_media_model::Codec::PcmU8)
    );
    assert_eq!(
        map_audio_format_tag(0x0001, 24),
        Some(scryer_media_model::Codec::PcmS24Le)
    );
    assert_eq!(
        map_audio_format_tag(0x0003, 32),
        Some(scryer_media_model::Codec::PcmF32Le)
    );
}

#[test]
fn resolves_wave_format_extensible_subtype_and_valid_bits() {
    let mut extra = vec![0_u8; 22];
    extra[0..2].copy_from_slice(&32_u16.to_le_bytes());
    extra[6..10].copy_from_slice(&3_u32.to_le_bytes());
    extra[10..22].copy_from_slice(&[
        0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
    ]);

    assert_eq!(
        resolve_audio_format(0xfffe, 32, Some(&extra)),
        Some((3, 32))
    );
    assert_eq!(
        resolve_audio_format(0xfffe, 32, Some(&extra))
            .and_then(|(tag, bits)| map_audio_format_tag(tag, bits)),
        Some(scryer_media_model::Codec::PcmF32Le)
    );
    extra[0..2].copy_from_slice(&20_u16.to_le_bytes());
    extra[6..10].copy_from_slice(&1_u32.to_le_bytes());
    assert_eq!(
        resolve_audio_format(0xfffe, 24, Some(&extra)),
        Some((1, 24))
    );
    let mut wave = vec![0_u8; 18];
    wave[0..2].copy_from_slice(&0xfffe_u16.to_le_bytes());
    wave[2..4].copy_from_slice(&2_u16.to_le_bytes());
    wave[4..8].copy_from_slice(&48_000_u32.to_le_bytes());
    wave[14..16].copy_from_slice(&24_u16.to_le_bytes());
    wave[16..18].copy_from_slice(&22_u16.to_le_bytes());
    wave.extend(extra);
    let track = parse_audio_type_data(&wave).unwrap();
    assert_eq!(
        track
            .codec_name
            .as_ref()
            .map(scryer_media_model::Codec::as_str),
        Some("pcm_s24le")
    );
    assert_eq!(track.metadata.sample_bit_depth, Some(20));
    assert_eq!(track.metadata.sample_rate, Some(48_000));
}

#[test]
fn derives_frame_rate_from_replicated_video_timestamps() {
    fn packet(object_number: u8, timestamp: u32) -> Vec<u8> {
        let mut packet = vec![0x82, 0, 0, 0, 0x15];
        packet.extend_from_slice(&0_u32.to_le_bytes());
        packet.extend_from_slice(&0_u16.to_le_bytes());
        packet.extend_from_slice(&[0x81, object_number, 0, 8]);
        packet.extend_from_slice(&1_u32.to_le_bytes());
        packet.extend_from_slice(&timestamp.to_le_bytes());
        packet.push(0);
        assert_eq!(packet.len(), 24);
        packet
    }

    let video_streams = BTreeSet::from([1]);
    let mut timestamps = BTreeMap::new();
    for (object_number, timestamp) in [(1, 0), (2, 40), (3, 80)] {
        let packet = packet(object_number, timestamp);
        assert_eq!(
            collect_asf_packet_timestamps(&packet, 24, &video_streams, &mut timestamps),
            Some(24)
        );
    }
    let timing = derive_frame_rate(&timestamps[&1]).unwrap();
    assert_eq!(timing.summary_fps, 25.0);
    assert_eq!(
        timing.observed_rate,
        scryer_media_model::Rational::new(25, 1).unwrap()
    );
    assert!(!timing.variable);
    assert!(derive_frame_rate(&[0, 40, 100]).unwrap().variable);
    assert!(!derive_frame_rate(&[0, 33, 67, 100]).unwrap().variable);
    assert!(derive_frame_rate(&[0, 40, 20]).is_none());
    assert!(derive_frame_rate(&[0, 40]).is_none());

    let truncated = packet(4, 120);
    assert!(
        collect_asf_packet_timestamps(
            &truncated[..truncated.len() - 1],
            24,
            &video_streams,
            &mut timestamps,
        )
        .is_none()
    );
}

#[test]
fn header_extension_budget_preserves_siblings_and_reports_incomplete_metadata() {
    fn object(guid: [u8; 16], payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::from(guid);
        bytes.extend((24 + payload.len() as u64).to_le_bytes());
        bytes.extend(payload);
        bytes
    }
    let mut properties = vec![0; 80];
    properties[40..48].copy_from_slice(&20_000_000_u64.to_le_bytes());
    for depth in [
        1,
        MAX_HEADER_EXTENSION_DEPTH,
        MAX_HEADER_EXTENSION_DEPTH + 1,
    ] {
        let mut nested = object(FILE_PROPERTIES_GUID, &properties);
        for _ in 0..depth {
            let mut extension = vec![0; 18];
            extension.extend((nested.len() as u32).to_le_bytes());
            extension.extend(nested);
            nested = object(HEADER_EXTENSION_GUID, &extension);
        }
        let exhausted = depth > MAX_HEADER_EXTENSION_DEPTH;
        let mut state = AsfState::default();
        parse_objects(&mut SliceReader::new(&nested), Some(1), &mut state).unwrap();
        assert_eq!(state.duration_seconds, (!exhausted).then_some(2.0));
        assert_eq!(state.header_extension_depth, 0);
        assert_eq!(state.header_nesting_exhausted, exhausted);
        assert!(state.object_count <= MAX_HEADER_EXTENSION_DEPTH + 1);

        let mut sibling = properties.clone();
        sibling[40..48].copy_from_slice(&30_000_000_u64.to_le_bytes());
        nested.extend(object(FILE_PROPERTIES_GUID, &sibling));
        let mut audio = vec![0; 70];
        audio[..16].copy_from_slice(&AUDIO_MEDIA_GUID);
        audio[40..44].copy_from_slice(&16_u32.to_le_bytes());
        audio[48..50].copy_from_slice(&1_u16.to_le_bytes());
        audio[54..56].copy_from_slice(&1_u16.to_le_bytes());
        audio[56..58].copy_from_slice(&1_u16.to_le_bytes());
        audio[58..62].copy_from_slice(&48_000_u32.to_le_bytes());
        audio[62..66].copy_from_slice(&96_000_u32.to_le_bytes());
        audio[66..68].copy_from_slice(&2_u16.to_le_bytes());
        audio[68..70].copy_from_slice(&16_u16.to_le_bytes());
        nested.extend(object(STREAM_PROPERTIES_GUID, &audio));
        let mut image = Vec::from(ASF_HEADER_GUID);
        image.extend((30 + nested.len() as u64).to_le_bytes());
        image.extend(3_u32.to_le_bytes());
        image.extend([1, 2]);
        image.extend(nested);
        let result = parse_asf_source(&mut std::io::Cursor::new(image)).unwrap();
        assert_eq!(result.tracks.len(), 1);
        assert_eq!(result.duration_seconds, Some(3.0));
        assert_eq!(result.details.report.budget_exhausted, exhausted);
        assert_eq!(
            result
                .details
                .report
                .warnings
                .iter()
                .any(|warning| { warning.code == "asf_header_nesting_limit" }),
            exhausted
        );
        if exhausted {
            assert_eq!(
                result.details.report.status,
                scryer_media_model::ProbeStatus::Incomplete
            );
        }
    }
    let mut state = AsfState::default();
    let mut malformed = vec![0; 18];
    malformed.extend(24_u32.to_le_bytes());
    malformed.extend(object(FILE_PROPERTIES_GUID, &[]));
    assert!(parse_header_extension(&malformed, &mut state).is_err());
    assert_eq!(state.header_extension_depth, 0);
}

#[test]
fn rejects_object_smaller_than_header() {
    let mut object = Vec::from(FILE_PROPERTIES_GUID);
    object.extend_from_slice(&23_u64.to_le_bytes());
    let error = parse_objects(
        &mut SliceReader::new(&object),
        None,
        &mut AsfState::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("smaller"));
}

#[test]
fn subtracts_preroll_from_file_duration() {
    let mut properties = vec![0_u8; 80];
    properties[40..48].copy_from_slice(&30_000_000_u64.to_le_bytes());
    properties[56..64].copy_from_slice(&500_u64.to_le_bytes());
    let mut state = AsfState::default();
    parse_file_properties(&properties, &mut state).unwrap();
    assert_eq!(state.duration_seconds, Some(2.5));
    properties[40..48].copy_from_slice(&30_001_234_u64.to_le_bytes());
    parse_file_properties(&properties, &mut state).unwrap();
    assert!((state.duration_seconds.unwrap() - 2.5001234).abs() < 1e-9);
}

#[test]
fn extended_stream_properties_override_bitrate_records() {
    let mut bitrate = Vec::new();
    bitrate.extend_from_slice(&1_u16.to_le_bytes());
    bitrate.extend_from_slice(&7_u16.to_le_bytes());
    bitrate.extend_from_slice(&64_000_u32.to_le_bytes());
    let mut state = AsfState::default();
    parse_stream_bitrates(&bitrate, &mut state).unwrap();

    let mut extended = vec![0_u8; 64];
    extended[16..20].copy_from_slice(&128_000_u32.to_le_bytes());
    extended[48..50].copy_from_slice(&7_u16.to_le_bytes());
    extended[50..52].copy_from_slice(&1_u16.to_le_bytes());
    extended[52..60].copy_from_slice(&400_000_u64.to_le_bytes());
    parse_extended_stream_properties(&extended, &mut state).unwrap();

    let stream = state.streams.get(&7).unwrap();
    assert_eq!(stream.bit_rate_bps, Some(128_000));
    assert_eq!(stream.frame_rate_fps, Some(25.0));
    assert_eq!(
        stream.declared_frame_rate,
        scryer_media_model::Rational::new(25, 1)
    );
    assert_eq!(stream.language_index, Some(1));
}

#[test]
fn normalizes_rfc1766_languages_to_iso_639_3() {
    let mut languages = Vec::new();
    languages.extend_from_slice(&1_u16.to_le_bytes());
    languages.push(12);
    for value in "ja-JP\0".encode_utf16() {
        languages.extend_from_slice(&value.to_le_bytes());
    }
    let mut state = AsfState::default();
    parse_languages(&languages, &mut state).unwrap();
    assert_eq!(state.languages, ["jpn"]);
    assert_eq!(state.original_languages, ["ja-JP"]);
}
