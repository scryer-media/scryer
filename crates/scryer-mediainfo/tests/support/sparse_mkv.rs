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
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn element(id: u32, data: &[u8]) -> Vec<u8> {
    let id = id.to_be_bytes();
    let mut bytes = id[id.iter().position(|byte| *byte != 0).unwrap()..].to_vec();
    assert!(data.len() < 16_383);
    if data.len() < 127 {
        bytes.push(0x80 | data.len() as u8);
    } else {
        bytes.extend((0x4000 | data.len() as u16).to_be_bytes());
    }
    bytes.extend(data);
    bytes
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
    let ebml = element(
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
    );
    let info = element(
        0x1549a966,
        &[
            uint(0x2ad7b1, 1_000_000),
            element(0x4489, &2000_f64.to_be_bytes()),
            element(0x4d80, b"test"),
            element(0x5741, b"test"),
        ]
        .concat(),
    );
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
        let mut dovi = vec![0; 24];
        dovi[..5].copy_from_slice(&[1, 0, profile << 1, 0x30, compatibility << 4]);
        video.extend(element(
            0x41e4,
            &[
                uint(0x41f0, 1),
                element(0x41a4, b"dvcC"),
                uint(0x41e7, 0x6476),
                element(0x41ed, &dovi),
            ]
            .concat(),
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
        "/video_frame_rate",
        "/details/streams/0/language",
        "/details/streams/0/metadata/original_language",
        "/details/streams/0/metadata/declared_frame_rate",
        "/details/streams/0/metadata/observed_frame_rate",
        "/details/streams/0/metadata/variable_frame_rate",
        "/details/streams/1/language",
        "/details/streams/1/metadata/original_language",
        "/details/streams/1/metadata/channel_layout",
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
