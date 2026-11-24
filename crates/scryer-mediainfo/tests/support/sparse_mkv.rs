//! Authored negative fixtures: codec signaling without a usable audio payload or video cadence.
//! Rebuilding the complete bytes guards the narrowly scoped reference-default expectations.
use serde_json::Value;

pub const NAMES: [&str; 4] = [
    "dv_profile5.mkv",
    "dv_profile7.mkv",
    "dv_profile8.mkv",
    "hevc_hdr10plus.mkv",
];

fn hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn element(id: u32, data: &[u8]) -> Vec<u8> {
    let id = id.to_be_bytes();
    let mut bytes = id[id.iter().position(|byte| *byte != 0).unwrap()..].to_vec();
    if data.len() < 127 {
        bytes.push(0x80 | data.len() as u8);
    } else if data.len() < 16_383 {
        bytes.extend((0x4000 | data.len() as u16).to_be_bytes());
    } else {
        assert!(data.len() < 0x0fff_ffff);
        bytes.extend((0x1000_0000 | data.len() as u32).to_be_bytes());
    }
    bytes.extend(data);
    bytes
}

fn ebml_header() -> Vec<u8> {
    element(
        0x1a45dfa3,
        &[
            uint(0x4286, 1),
            uint(0x42f7, 1),
            uint(0x42f2, 4),
            uint(0x42f3, 8),
            element(0x4282, b"matroska"),
            uint(0x4287, 4),
            uint(0x4285, 2),
        ]
        .concat(),
    )
}

fn segment_info() -> Vec<u8> {
    element(
        0x1549a966,
        &[
            uint(0x2ad7b1, 1_000_000),
            element(0x4489, &2000_f64.to_be_bytes()),
            element(0x4d80, b"test"),
            element(0x5741, b"test"),
        ]
        .concat(),
    )
}

/// The 24-byte DOVIDecoderConfigurationRecord FFmpeg writes: version 1.0, RPU and base
/// layer present, the enhancement layer only for dual-layer profile 7.
fn dovi_record(profile: u8, level: u8, compatibility: u8) -> Vec<u8> {
    let enhancement_layer = u8::from(profile == 7);
    let mut record = vec![0; 24];
    record[..5].copy_from_slice(&[
        1,
        0,
        profile << 1 | level >> 5,
        (level & 0x1f) << 3 | 1 << 2 | enhancement_layer << 1 | 1,
        compatibility << 4,
    ]);
    record
}

/// A BlockAdditionMapping laid out as FFmpeg's Matroska muxer writes it: name, type and
/// record, with no BlockAddIDValue.
fn dovi_mapping(block_add_id_type: u64, record: &[u8]) -> Vec<u8> {
    element(
        0x41e4,
        &[
            element(0x41a4, b"Dolby Vision configuration"),
            uint(0x41e7, block_add_id_type),
            element(0x41ed, record),
        ]
        .concat(),
    )
}

/// The BlockAddIDType FourCC muxers use: 'dvcC' up to profile 7, 'dvvC' from profile 8.
fn dovi_block_add_id_type(profile: u8) -> u64 {
    u32::from_be_bytes(if profile <= 7 { *b"dvcC" } else { *b"dvvC" }).into()
}

/// A length-prefixed (4-byte) HEVC TRAIL_R NAL unit of `size` bytes with no SEI.
fn hevc_frame(size: usize) -> Vec<u8> {
    let mut frame = vec![0; size];
    frame[..4].copy_from_slice(&(size as u32 - 4).to_be_bytes());
    frame[4..6].copy_from_slice(&[0x02, 0x01]);
    frame
}

fn simple_block(track: u8, keyframe: bool, payload: &[u8]) -> Vec<u8> {
    element(
        0xa3,
        &[
            &[0x80 | track, 0, 0, if keyframe { 0x80 } else { 0 }][..],
            payload,
        ]
        .concat(),
    )
}

/// A 2160p PQ (BT.2020, SMPTE ST 2084) HEVC track with a profile 8.1 Dolby Vision mapping
/// stored under `block_add_id_type`, and 24 clusters of 25 fps frames. The first frame is
/// 256 KiB, so the frame-payload HDR10+ probe reads past the cluster headers.
pub fn pq_dolby_vision_bytes(block_add_id_type: u64) -> Vec<u8> {
    let video = [
        uint(0xd7, 1),
        uint(0x73c5, 1),
        uint(0x83, 1),
        element(0x86, b"V_MPEGH/ISO/HEVC"),
        element(
            0x63a2,
            &hex("0102600000009000000000005df000fcfcfafa00000300"),
        ),
        element(
            0xe0,
            &[
                uint(0xb0, 3840),
                uint(0xba, 2160),
                element(
                    0x55b0,
                    &[uint(0x55b1, 9), uint(0x55ba, 16), uint(0x55bb, 9)].concat(),
                ),
            ]
            .concat(),
        ),
        dovi_mapping(block_add_id_type, &dovi_record(8, 6, 1)),
    ]
    .concat();
    let tracks = element(0x1654ae6b, &element(0xae, &video));
    let clusters = (0..24_u64)
        .map(|index| {
            let frame = hevc_frame(if index == 0 { 256 * 1024 } else { 64 });
            element(
                0x1f43b675,
                &[uint(0xe7, index * 40), simple_block(1, index == 0, &frame)].concat(),
            )
        })
        .collect::<Vec<_>>()
        .concat();
    [
        ebml_header(),
        element(0x18538067, &[segment_info(), tracks, clusters].concat()),
    ]
    .concat()
}

/// A cover-art style MJPEG video track ahead of a 1080p HEVC track that carries a
/// profile 7 Dolby Vision mapping.
pub fn cover_art_then_dolby_vision_bytes() -> Vec<u8> {
    let cover = [
        uint(0xd7, 1),
        uint(0x73c5, 1),
        uint(0x83, 1),
        element(0x536e, b"cover"),
        element(0x86, b"V_MJPEG"),
        element(0xe0, &[uint(0xb0, 600), uint(0xba, 600)].concat()),
    ]
    .concat();
    let video = [
        uint(0xd7, 2),
        uint(0x73c5, 2),
        uint(0x83, 1),
        element(0x86, b"V_MPEGH/ISO/HEVC"),
        element(
            0x63a2,
            &hex("0102600000009000000000005df000fcfcfafa00000300"),
        ),
        element(
            0xe0,
            &[
                uint(0xb0, 1920),
                uint(0xba, 1080),
                element(
                    0x55b0,
                    &[uint(0x55b1, 9), uint(0x55ba, 16), uint(0x55bb, 9)].concat(),
                ),
            ]
            .concat(),
        ),
        dovi_mapping(dovi_block_add_id_type(7), &dovi_record(7, 6, 6)),
    ]
    .concat();
    let tracks = element(
        0x1654ae6b,
        &[element(0xae, &cover), element(0xae, &video)].concat(),
    );
    let clusters = (0..2_u64)
        .map(|index| {
            element(
                0x1f43b675,
                &[
                    uint(0xe7, index * 1000),
                    simple_block(2, index == 0, &hevc_frame(64)),
                ]
                .concat(),
            )
        })
        .collect::<Vec<_>>()
        .concat();
    [
        ebml_header(),
        element(0x18538067, &[segment_info(), tracks, clusters].concat()),
    ]
    .concat()
}

fn uint(id: u32, value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    element(
        id,
        &bytes[bytes.iter().position(|byte| *byte != 0).unwrap_or(7)..],
    )
}

/// Explicit variants add real declarations so the differential cannot excuse missing facts.
pub fn authored_bytes(name: &str, explicit: bool) -> Option<Vec<u8>> {
    let (profile, compatibility) = match name {
        "dv_profile5.mkv" => (5, 0),
        "dv_profile7.mkv" => (7, 6),
        "dv_profile8.mkv" => (8, 1),
        "hevc_hdr10plus.mkv" => (0, 0),
        _ => return None,
    };
    let ebml = ebml_header();
    let info = segment_info();
    // No parameter-set arrays or timing declarations are present in these hvcC records.
    let config = hex(if profile == 0 {
        "0102600000009000000000005df000fcfcfafa00000300"
    } else {
        "0101600000009000000000005df000fcfcfafa00000000"
    });
    let mut video = [
        uint(0xd7, 1),
        uint(0x73c5, 1),
        uint(0x83, 1),
        element(0x86, b"V_MPEGH/ISO/HEVC"),
        element(0x63a2, &config),
        element(
            0xe0,
            &[
                uint(0xb0, if profile == 0 { 3840 } else { 128 }),
                uint(0xba, if profile == 0 { 2160 } else { 72 }),
            ]
            .concat(),
        ),
    ]
    .concat();
    if profile != 0 {
        video.extend(dovi_mapping(
            dovi_block_add_id_type(profile),
            &dovi_record(profile, 6, compatibility),
        ));
    }
    let mut audio = [
        uint(0xd7, 2),
        uint(0x73c5, 2),
        uint(0x83, 2),
        element(0x86, b"A_AAC"),
        element(
            0xe1,
            &[element(0xb5, &48_000_f64.to_be_bytes()), uint(0x9f, 2)].concat(),
        ),
    ]
    .concat();
    if explicit {
        video.extend(element(0x22b59c, b"eng"));
        video.extend(uint(0x23e383, 40_000_000));
        audio.extend(element(0x22b59c, b"eng"));
        audio.extend(element(0x63a2, &[0x11, 0x90])); // AAC-LC, 48 kHz, stereo.
    }
    let tracks = element(
        0x1654ae6b,
        &[element(0xae, &video), element(0xae, &audio)].concat(),
    );
    let block = if profile == 0 {
        hex("810000800000000f4e01040ab5003c00010401000000800000000a02010000000000000000")
    } else {
        [&[0x81, 0, 0, 0x80][..], &[0; 16]].concat()
    };
    let clusters = [0, 1000]
        .map(|time| {
            element(
                0x1f43b675,
                &[uint(0xe7, time), element(0xa3, &block)].concat(),
            )
        })
        .concat();
    Some(
        [
            ebml,
            element(0x18538067, &[info, tracks, clusters].concat()),
        ]
        .concat(),
    )
}

pub fn unknown_fact_errors(native: &Value) -> Vec<String> {
    [
        "/streams/0/metadata/nominal_frame_rate_fps",
        "/streams/0/language",
        "/streams/0/metadata/original_language",
        "/streams/0/metadata/declared_frame_rate",
        "/streams/0/metadata/observed_frame_rate",
        "/streams/0/metadata/variable_frame_rate",
        "/streams/1/language",
        "/streams/1/metadata/original_language",
        "/streams/1/metadata/channel_layout",
    ]
    .into_iter()
    .filter(|path| native.pointer(path) != Some(&Value::Null))
    .map(|path| format!("authored sparse fixture requires unknown {path}"))
    .collect()
}

/// Returns a comparison view while retaining every raw reference value in the caller's report.
/// Only byte-for-byte authored negative fixtures qualify; changed fixtures require a new audit.
pub fn comparison_view(
    name: &str,
    bytes: &[u8],
    reference: &Value,
    native: &Value,
) -> Result<(Value, Vec<Value>, Vec<String>), String> {
    let Some(authored) = authored_bytes(name, false) else {
        return Ok((reference.clone(), Vec::new(), Vec::new()));
    };
    if bytes != authored {
        return Err(format!(
            "{name}: sparse reference expectations no longer match the authored bytes"
        ));
    }
    if reference["streams"].as_array().map(Vec::len) != Some(2)
        || reference["streams"][0]["codec_type"] != "video"
        || reference["streams"][1]["codec_type"] != "audio"
    {
        return Err(format!(
            "{name}: reference stream inventory changed; audit required"
        ));
    }
    let mut errors = unknown_fact_errors(native);
    let mut comparison = reference.clone();
    let mut assumptions = Vec::new();
    // These are FFprobe's container defaults/synthesized values, not stored declarations.
    for (path, expected, reason) in [
        (
            "/streams/0/tags/language",
            "eng",
            "Matroska Language default; no language element is stored",
        ),
        (
            "/streams/1/tags/language",
            "eng",
            "Matroska Language default; no language element is stored",
        ),
        (
            "/streams/1/channel_layout",
            "stereo",
            "AAC configuration synthesized from channel count; no audio CodecPrivate or payload is stored",
        ),
        (
            "/streams/0/r_frame_rate",
            "1000/1",
            "Inverse timestamp time base; no declared cadence or usable video timing headers are stored",
        ),
    ] {
        if let Some(value) = comparison
            .pointer_mut(path)
            .filter(|value| !value.is_null())
        {
            if value.as_str() != Some(expected) {
                errors.push(format!(
                    "{name}: unaudited reference value at {path}: {value}"
                ));
            } else {
                assumptions
                    .push(serde_json::json!({"field":path,"value":value.clone(),"reason":reason}));
                *value = Value::Null;
            }
        }
    }
    Ok((comparison, assumptions, errors))
}
