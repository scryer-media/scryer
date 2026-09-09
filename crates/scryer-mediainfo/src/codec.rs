use crate::scan;
#[cfg(test)]
use crate::types::RawTrack;

/// Codec profile and bit-depth information extracted from bitstream headers.
#[derive(Debug, Clone, Default)]
pub(crate) struct CodecInfo {
    pub profile: Option<String>,
    pub bit_depth: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub frame_rate_fps: Option<f64>,
    /// ITU-T H.273 TransferCharacteristics value extracted from bitstream VUI
    /// (e.g. 16 = SMPTE 2084/PQ, 18 = HLG). Used for HDR detection when the
    /// container doesn't carry this information.
    pub color_transfer: Option<u32>,
}

/// Maps a container-level codec identifier to an ffprobe-style normalized name.
///
/// Handles MKV codec IDs (e.g. `V_MPEG4/ISO/AVC`), MP4 FourCC codes (e.g.
/// `"avc1"`), and returns `None` for identifiers that require further
/// container-level disambiguation (e.g. `V_MS/VFW/FOURCC`).
pub(crate) fn normalize_codec_name(codec_id: &str) -> Option<String> {
    let name = match codec_id {
        // --- MKV video ---
        "V_MPEG4/ISO/AVC" => "h264",
        "V_MPEGH/ISO/HEVC" => "hevc",
        "V_AV1" => "av1",
        "V_VP8" => "vp8",
        "V_VP9" => "vp9",
        "V_MPEG4/ISO/SP" | "V_MPEG4/ISO/ASP" | "V_MPEG4/ISO/AP" => "mpeg4",
        "V_MPEG2" => "mpeg2video",
        "V_MPEG1" => "mpeg1video",
        "V_MJPEG" => "mjpeg",
        "V_MS/VFW/FOURCC" => return None,

        // --- MKV audio ---
        "A_AC3" => "ac3",
        "A_EAC3" => "eac3",
        "A_TRUEHD" => "truehd",
        "A_DTS" => "dts",
        "A_FLAC" => "flac",
        "A_OPUS" => "opus",
        "A_VORBIS" => "vorbis",
        "A_MPEG/L3" => "mp3",
        "A_MPEG/L2" => "mp2",
        "A_MPEG/L1" => "mp1",

        // --- MKV subtitle ---
        "S_TEXT/UTF8" => "subrip",
        "S_TEXT/ASS" | "S_TEXT/SSA" => "ass",
        "S_HDMV/PGS" => "hdmv_pgs_subtitle",
        "S_VOBSUB" => "dvd_subtitle",
        "S_TEXT/WEBVTT" => "webvtt",
        "D_WEBVTT/SUBTITLES" | "D_WEBVTT/CAPTIONS" => "webvtt",

        // --- MP4 FourCC ---
        "avc1" | "avc3" => "h264",
        "dva1" | "dvav" => "h264",
        "hvc1" | "hev1" => "hevc",
        "dvh1" | "dvhe" => "hevc",
        "av01" => "av1",
        "vp09" => "vp9",
        "mp4v" => "mpeg4",
        "jpeg" | "mjpg" => "mjpeg",
        "png " => "png",
        "mp4a" => "aac",
        "ac-3" => "ac3",
        "ec-3" => "eac3",
        "fLaC" => "flac",
        "Opus" => "opus",
        "text" | "tx3g" => "mov_text",
        "wvtt" => "webvtt",
        "stpp" => "ttml",

        // MKV AAC variants and PCM wildcard
        other => {
            if other == "A_AAC" || other.starts_with("A_AAC/") {
                return Some("aac".into());
            }
            if other == "A_DTS"
                || other.starts_with("A_DTS/")
                || other == "A_DTS/LOSSY"
                || other == "A_DTS/LOSSLESS"
                || other == "A_DTS/EXPRESS"
            {
                return Some("dts".into());
            }
            if other.starts_with("A_PCM/") {
                return Some("pcm".into());
            }
            return None;
        }
    };
    Some(name.into())
}

pub(crate) fn normalize_pcm_codec_name(codec_id: &str, bit_depth: Option<i32>) -> Option<String> {
    if !codec_id.starts_with("A_PCM/") {
        return None;
    }

    let depth = bit_depth.unwrap_or_default();
    let codec_name = match (
        codec_id.contains("/FLOAT"),
        codec_id.contains("/BIG"),
        depth,
    ) {
        (true, true, 32) => "pcm_f32be",
        (true, false, 32) => "pcm_f32le",
        (true, true, 64) => "pcm_f64be",
        (true, false, 64) => "pcm_f64le",
        (false, _, 8) => "pcm_s8",
        (false, true, 16) => "pcm_s16be",
        (false, false, 16) => "pcm_s16le",
        (false, true, 24) => "pcm_s24be",
        (false, false, 24) => "pcm_s24le",
        (false, true, 32) => "pcm_s32be",
        (false, false, 32) => "pcm_s32le",
        _ => "pcm",
    };

    Some(codec_name.to_owned())
}

pub(crate) fn normalize_video_fourcc_codec_name(fourcc: &str) -> Option<String> {
    let codec_name = match fourcc.trim_end_matches('\0') {
        "H264" | "h264" | "X264" | "x264" | "avc1" | "AVC1" => "h264",
        "HEVC" | "hevc" | "H265" | "h265" | "hvc1" | "HVC1" | "hev1" | "HEV1" => "hevc",
        "XVID" | "xvid" | "DX50" | "dx50" | "DIVX" | "divx" | "DIV3" | "div3" | "DIV4" | "div4"
        | "DIV5" | "div5" | "MP4V" | "mp4v" | "FMP4" | "fmp4" => "mpeg4",
        "MJPG" | "mjpg" => "mjpeg",
        "WVC1" | "wvc1" => "vc1",
        "WMV3" | "wmv3" => "wmv3",
        "MP2V" | "mp2v" | "mpg2" | "MPG2" => "mpeg2video",
        "MP1V" | "mp1v" | "mpg1" | "MPG1" => "mpeg1video",
        "VP80" | "vp80" => "vp8",
        "VP90" | "vp90" => "vp9",
        _ => return None,
    };

    Some(codec_name.to_owned())
}

pub(crate) fn normalize_vfw_codec_name(codec_private: Option<&[u8]>) -> Option<String> {
    let compression = codec_private
        .and_then(|data| data.get(16..20))
        .and_then(|fourcc| std::str::from_utf8(fourcc).ok())?;
    normalize_video_fourcc_codec_name(compression)
}

const DTS_SYNCWORD_CORE_BE: u32 = 0x7FFE_8001;
const DTS_SYNCWORD_CORE_LE: u32 = 0xFE7F_0180;
const DTS_SYNCWORD_CORE_14B_BE: u32 = 0x1FFF_E800;
const DTS_SYNCWORD_CORE_14B_LE: u32 = 0xFF1F_00E8;
const DTS_SYNCWORD_SUBSTREAM: u32 = 0x6458_2025;
const DTS_SYNCWORD_XCH: u32 = 0x5A5A_5A5A;
const DTS_SYNCWORD_XXCH: u32 = 0x4700_4A03;
const DTS_SYNCWORD_X96: u32 = 0x1D95_F262;
const DTS_SYNCWORD_XLL: u32 = 0x41A2_9547;
const DTS_SYNCWORD_XLL_X: u32 = 0x0200_0850;
const DTS_SYNCWORD_XLL_X_IMAX: u32 = 0xF140_00D0;
const TRUEHD_MAJOR_SYNCWORD: [u8; 4] = [0xF8, 0x72, 0x6F, 0xBA];
const DTS_CORE_PROBE_BYTES: usize = 32;
const DTS_XLL_CHSETS_MAX: u32 = 3;
const DTS_XLL_PBR_BUFFER_MAX: usize = 240 << 10;
const DTS_CHANNELS: [u8; 16] = [1, 2, 2, 2, 2, 3, 3, 4, 4, 5, 6, 6, 6, 7, 8, 8];
const DTS_SAMPLE_RATES: [u32; 16] = [
    0, 8_000, 16_000, 32_000, 0, 0, 11_025, 22_050, 44_100, 0, 0, 12_000, 24_000, 48_000, 96_000,
    192_000,
];
const DCA_EXSS_CORE: u16 = 0x010;
const DCA_EXSS_XBR: u16 = 0x020;
const DCA_EXSS_XXCH: u16 = 0x040;
const DCA_EXSS_X96: u16 = 0x080;
const DCA_EXSS_LBR: u16 = 0x100;
const DCA_EXSS_XLL: u16 = 0x200;
const DCA_EXSS_RSV1: u16 = 0x400;
const DCA_EXSS_RSV2: u16 = 0x800;

#[derive(Debug, Clone, Copy)]
pub(crate) struct AudioProfileProbeSpec {
    pub prefix_bytes: usize,
    pub suffix_bytes: usize,
}

pub(crate) fn audio_profile_probe_spec(codec_name: Option<&str>) -> AudioProfileProbeSpec {
    match codec_name {
        Some("aac") | Some("aac_latm") => AudioProfileProbeSpec {
            prefix_bytes: 512,
            suffix_bytes: 0,
        },
        Some("ac3" | "eac3") => AudioProfileProbeSpec {
            prefix_bytes: 64,
            suffix_bytes: 0,
        },
        Some("truehd") => AudioProfileProbeSpec {
            prefix_bytes: 64,
            suffix_bytes: 0,
        },
        Some("dts") => AudioProfileProbeSpec {
            prefix_bytes: 4096,
            suffix_bytes: 64,
        },
        _ => AudioProfileProbeSpec {
            prefix_bytes: 0,
            suffix_bytes: 0,
        },
    }
}

pub(crate) fn detect_header_audio_profile(
    codec_id: &str,
    codec_name: Option<&str>,
    codec_private: Option<&[u8]>,
) -> Option<String> {
    let normalized_codec = normalize_codec_name(codec_id);
    let codec_name = codec_name.or(normalized_codec.as_deref());

    match codec_name {
        Some("aac") => detect_aac_profile_from_codec_private(codec_private)
            .or_else(|| detect_aac_profile_from_codec_id(codec_id)),
        Some("dts") => detect_dts_profile_from_codec_id(codec_id),
        _ => None,
    }
}

pub(crate) fn detect_audio_profile_from_payload(
    codec_name: Option<&str>,
    payload: &[u8],
) -> Option<String> {
    if matches!(codec_name, Some("eac3" | "truehd")) {
        return detect_audio_profile_from_probe_bytes(
            codec_name,
            &payload[..payload.len().min(64 * 1024)],
            None,
        );
    }
    let spec = audio_profile_probe_spec(codec_name);
    if spec.prefix_bytes == 0 {
        return None;
    }

    let prefix_len = payload.len().min(spec.prefix_bytes);
    let prefix = &payload[..prefix_len];
    let suffix = if spec.suffix_bytes > 0 {
        let suffix_start = payload.len().saturating_sub(spec.suffix_bytes);
        (suffix_start >= prefix_len).then_some(&payload[suffix_start..])
    } else {
        None
    };

    detect_audio_profile_from_probe_bytes(codec_name, prefix, suffix)
}

pub(crate) fn detect_audio_profile_from_probe_bytes(
    codec_name: Option<&str>,
    prefix: &[u8],
    suffix: Option<&[u8]>,
) -> Option<String> {
    match codec_name {
        Some("aac") | Some("aac_latm") => detect_aac_profile_from_payload(prefix),
        Some("eac3") => detect_eac3_profile_samples(prefix),
        Some("truehd") => detect_truehd_profile_samples(prefix),
        Some("dts") => detect_dts_profile_from_probe_bytes(prefix, suffix),
        _ => None,
    }
}

pub(crate) fn merge_audio_profile(current: &mut Option<String>, candidate: Option<String>) {
    let Some(candidate) = candidate else {
        return;
    };

    if current
        .as_deref()
        .is_none_or(|existing| audio_profile_rank(&candidate) > audio_profile_rank(existing))
    {
        *current = Some(candidate);
    }
}

pub(crate) fn detect_dts_channels_from_probe_bytes(prefix: &[u8]) -> Option<i32> {
    let asset = parse_dts_exss_asset(prefix);
    asset.map(|asset| asset.nchannels_total)
}

fn audio_profile_rank(profile: &str) -> i32 {
    match profile {
        "DTS-HD MA + DTS:X IMAX" => 100,
        "DTS-HD MA + DTS:X" => 90,
        "Dolby TrueHD + Dolby Atmos" => 85,
        "Dolby Digital Plus + Dolby Atmos" => 80,
        "DTS-HD MA" => 70,
        "DTS-HD HRA" => 60,
        "DTS 96/24" => 55,
        "DTS Express" => 50,
        "DTS-ES" => 45,
        "DTS" => 40,
        "HE-AACv2" => 35,
        "HE-AAC" => 30,
        "LC" => 25,
        "Main" => 20,
        "LTP" => 18,
        "SSR" => 16,
        "LD" => 14,
        "ELD" => 12,
        _ => 1,
    }
}

fn detect_aac_profile_from_codec_id(codec_id: &str) -> Option<String> {
    let upper = codec_id.to_ascii_uppercase();
    if upper.contains("/SBR/PS") || upper.contains("HE-AACV2") {
        return Some("HE-AACv2".into());
    }
    if upper.contains("/SBR") || upper.contains("HE-AAC") {
        return Some("HE-AAC".into());
    }
    if upper.contains("/LC") {
        return Some("LC".into());
    }
    if upper.contains("/MAIN") {
        return Some("Main".into());
    }
    if upper.contains("/SSR") {
        return Some("SSR".into());
    }
    if upper.contains("/LTP") {
        return Some("LTP".into());
    }
    None
}

fn detect_dts_profile_from_codec_id(codec_id: &str) -> Option<String> {
    match codec_id {
        "A_DTS/LOSSLESS" => Some("DTS-HD MA".into()),
        "A_DTS/EXPRESS" => Some("DTS Express".into()),
        id if id == "A_DTS" || id == "A_DTS/LOSSY" || id.starts_with("A_DTS/") => {
            Some("DTS".into())
        }
        _ => None,
    }
}

fn detect_aac_profile_from_codec_private(codec_private: Option<&[u8]>) -> Option<String> {
    codec_private.and_then(parse_aac_audio_object_profile)
}

fn detect_aac_profile_from_payload(payload: &[u8]) -> Option<String> {
    detect_adts_aac_profile(payload).or_else(|| detect_latm_aac_profile(payload))
}

pub(crate) fn pcm_sample_representation(codec: &str) -> Option<(&str, u32)> {
    let format = codec.strip_prefix("pcm_")?;
    let depth = match format {
        "s8" | "u8" => 8,
        "s16le" | "s16be" | "u16le" | "u16be" => 16,
        "s24le" | "s24be" | "u24le" | "u24be" => 24,
        "s32le" | "s32be" | "u32le" | "u32be" | "f32le" | "f32be" => 32,
        "s64le" | "s64be" | "f64le" | "f64be" => 64,
        _ => return None,
    };
    Some((format, depth))
}

pub(crate) fn aac_channel_configuration(config: u8) -> Option<(u8, &'static str)> {
    Some(match config {
        1 => (1, "mono"),
        2 => (2, "stereo"),
        3 => (3, "3.0"),
        4 => (4, "4.0"),
        5 => (5, "5.0"),
        6 => (6, "5.1"),
        7 => (8, "7.1(wide)"),
        11 => (7, "6.1(back)"),
        12 => (8, "7.1"),
        _ => return None,
    })
}

pub(crate) struct AacConfiguration {
    pub sample_rate: u32,
    pub layout: Option<(u8, &'static str)>,
    pub sbr_signaled: Option<bool>,
    pub ps_signaled: Option<bool>,
}

pub(crate) fn aac_configuration_metadata(data: &[u8]) -> Option<AacConfiguration> {
    let mut bits = AudioBitReader::new(data.get(..data.len().min(512))?);
    let object_type = bits.read_aac_audio_object_type()?;
    if !matches!(object_type, 1..=5 | 17 | 19..=23 | 29 | 39) {
        return None;
    }
    let mut rate = bits.read_aac_sample_rate()?;
    let mut config = bits.read_bits(4)? as u8;
    let mut parametric_stereo = object_type == 29;
    let mut sbr_signaled = None;
    let mut ps_signaled = (object_type == 29).then_some(true);
    if matches!(object_type, 5 | 29) {
        sbr_signaled = Some(true);
        rate = bits.read_aac_sample_rate()?;
        if bits.read_aac_audio_object_type()? == 22 {
            config = bits.read_bits(4)? as u8;
        }
    } else {
        while bits.bits_left() > 15 {
            if bits.peek_bits(11)? == 0x2b7 {
                bits.read_bits(11)?;
                if bits.read_aac_audio_object_type()? == 5 {
                    let sbr = bits.read_bit()? == 1;
                    sbr_signaled = Some(sbr);
                    if sbr {
                        rate = bits.read_aac_sample_rate()?;
                    }
                    if bits.bits_left() > 11 && bits.read_bits(11)? == 0x548 {
                        parametric_stereo = bits.read_bit()? == 1;
                        ps_signaled = Some(parametric_stereo);
                    }
                }
                break;
            }
            bits.read_bit()?;
        }
    }
    if rate == 0 {
        return None;
    }
    let layout = if parametric_stereo && config == 1 {
        Some((2, "stereo"))
    } else {
        aac_channel_configuration(config)
    };
    Some(AacConfiguration {
        sample_rate: rate,
        layout,
        sbr_signaled,
        ps_signaled,
    })
}

fn parse_aac_audio_object_profile(data: &[u8]) -> Option<String> {
    let mut bits = AudioBitReader::new(data);
    let audio_object_type = bits.read_aac_audio_object_type()?;
    bits.read_aac_sample_rate()?;
    let mut _channel_config = bits.read_bits(4)?;

    let profile = match audio_object_type {
        1 => Some("Main"),
        2 => Some("LC"),
        3 => Some("SSR"),
        4 => Some("LTP"),
        5 => Some("HE-AAC"),
        23 => Some("LD"),
        29 => Some("HE-AACv2"),
        39 => Some("ELD"),
        _ => None,
    }?;

    if matches!(audio_object_type, 5 | 29) {
        bits.read_aac_sample_rate()?;
        let ext_audio_object_type = bits.read_aac_audio_object_type()?;
        if ext_audio_object_type == 22 {
            _channel_config = bits.read_bits(4)?;
        }
        return Some(profile.to_string());
    }

    if let Some(sync_extension_profile) = parse_aac_sync_extension_profile(&mut bits) {
        return Some(sync_extension_profile);
    }

    Some(profile.to_string())
}

fn parse_aac_sync_extension_profile(bits: &mut AudioBitReader<'_>) -> Option<String> {
    while bits.bits_left() > 15 {
        if bits.peek_bits(11)? == 0x2B7 {
            bits.read_bits(11)?;
            let ext_object_type = bits.read_aac_audio_object_type()?;
            if ext_object_type == 5 && bits.read_bit()? == 1 {
                bits.read_aac_sample_rate()?;
                if bits.bits_left() > 11 && bits.read_bits(11)? == 0x548 && bits.read_bit()? == 1 {
                    return Some("HE-AACv2".into());
                }
                return Some("HE-AAC".into());
            }
            break;
        }
        bits.read_bit()?;
    }
    None
}

fn detect_adts_aac_profile(data: &[u8]) -> Option<String> {
    if data.len() < 7 {
        return None;
    }

    for start in 0..=data.len() - 7 {
        let hdr = &data[start..];
        if hdr[0] != 0xFF || (hdr[1] & 0xF0) != 0xF0 {
            continue;
        }

        let profile_index = ((hdr[2] >> 6) & 0x03) + 1;
        let profile = match profile_index {
            1 => "Main",
            2 => "LC",
            3 => "SSR",
            4 => "LTP",
            _ => continue,
        };
        return Some(profile.to_string());
    }

    None
}

fn detect_latm_aac_profile(data: &[u8]) -> Option<String> {
    for start in 0..data.len().saturating_sub(3) {
        if data[start] != 0x56 || (data[start + 1] & 0xE0) != 0xE0 {
            continue;
        }

        let mut bits = AudioBitReader::new(&data[start..]);
        if bits.read_bits(11)? != 0x2B7 {
            continue;
        }
        let _mux_length = bits.read_bits(13)?;
        if bits.read_bit()? != 0 || bits.read_bit()? != 0 {
            continue;
        }
        let _all_streams_same_time_framing = bits.read_bit()?;
        if bits.read_bits(6)? != 0 || bits.read_bits(4)? != 0 || bits.read_bits(3)? != 0 {
            continue;
        }
        let audio_object_type = bits.read_aac_audio_object_type()?;
        let mut profile = match audio_object_type {
            1 => Some("Main"),
            2 => Some("LC"),
            3 => Some("SSR"),
            4 => Some("LTP"),
            5 => Some("HE-AAC"),
            23 => Some("LD"),
            29 => Some("HE-AACv2"),
            39 => Some("ELD"),
            _ => None,
        }?;

        bits.read_aac_sample_rate()?;
        let _channel_config = bits.read_bits(4)?;
        if matches!(audio_object_type, 5 | 29) {
            bits.read_aac_sample_rate()?;
            let ext_audio_object_type = bits.read_aac_audio_object_type()?;
            if ext_audio_object_type == 22 {
                let _ = bits.read_bits(4)?;
            }
            profile = if audio_object_type == 29 {
                Some("HE-AACv2")
            } else {
                Some("HE-AAC")
            }?;
        }
        return Some(profile.to_string());
    }

    None
}

fn detect_eac3_profile_samples(data: &[u8]) -> Option<String> {
    let data = &data[..data.len().min(64 * 1024)];
    let mut cursor = 0;
    for _ in 0..32 {
        let offset = crate::scan::find_pair_from(data, cursor, 0x0b, 0xff, 0x77)?;
        let header = data.get(offset..offset + 7)?;
        let frame_size = (usize::from(u16::from_be_bytes([header[2] & 7, header[3]])) + 1) * 2;
        if !(11..=16).contains(&(header[5] >> 3)) || header[2] >> 6 == 3 || frame_size < 7 {
            cursor = offset + 2;
            continue;
        }
        let end = (offset + frame_size).min(data.len());
        if let Some(profile) = detect_eac3_profile_from_payload(&data[offset..end]) {
            return Some(profile);
        }
        cursor = offset + frame_size;
    }
    None
}

fn detect_truehd_profile_samples(data: &[u8]) -> Option<String> {
    let data = &data[..data.len().min(64 * 1024)];
    let mut cursor = 0;
    let mut profile = None;
    for _ in 0..32 {
        let Some(offset) =
            crate::scan::find_syncword(data, u32::from_be_bytes(TRUEHD_MAJOR_SYNCWORD), cursor)
        else {
            break;
        };
        merge_audio_profile(
            &mut profile,
            detect_truehd_profile_from_payload(&data[offset..data.len().min(offset + 64)]),
        );
        if profile.as_deref() == Some("Dolby TrueHD + Dolby Atmos") {
            break;
        }
        cursor = offset + 4;
    }
    profile
}

fn detect_eac3_profile_from_payload(data: &[u8]) -> Option<String> {
    if data.len() < 7 || data[0] != 0x0B || data[1] != 0x77 {
        return None;
    }

    let mut bits = AudioBitReader::new(&data[2..]);
    let frame_type = bits.read_bits(2)?;
    if frame_type != 0 {
        return None;
    }
    if bits.read_bits(3)? != 0 {
        return None;
    }
    let _frame_size = bits.read_bits(11)?;

    let fscod = bits.read_bits(2)?;
    let num_blocks = if fscod == 3 {
        let _fscod2 = bits.read_bits(2)?;
        6
    } else {
        match bits.read_bits(2)? {
            0 => 1,
            1 => 2,
            2 => 3,
            3 => 6,
            _ => unreachable!(),
        }
    };

    let acmod = bits.read_bits(3)? as usize;
    let lfe_on = bits.read_bit()?;

    let bsid = bits.read_bits(5)?;
    if bsid <= 10 {
        return None;
    }

    let dialnorm_count = if acmod == 0 { 2 } else { 1 };
    for _ in 0..dialnorm_count {
        bits.skip_bits(5)?; // dialnorm
        if bits.read_bit()? != 0 {
            bits.skip_bits(8)?; // compr
        }
    }

    if frame_type == 1 && bits.read_bit()? != 0 {
        bits.skip_bits(16)?; // channel map
    }

    if bits.read_bit()? != 0 {
        if acmod > 2 {
            bits.skip_bits(2)?; // preferred downmix
            if (acmod & 1) != 0 {
                bits.skip_bits(6)?; // center mix levels
            }
            if (acmod & 4) != 0 {
                bits.skip_bits(6)?; // surround mix levels
            }
        }

        if lfe_on != 0 && bits.read_bit()? != 0 {
            bits.skip_bits(5)?; // lfe mix level
        }

        if frame_type == 0 {
            for _ in 0..dialnorm_count {
                if bits.read_bit()? != 0 {
                    bits.skip_bits(6)?; // program scale factor
                }
            }
            if bits.read_bit()? != 0 {
                bits.skip_bits(6)?; // external program scale factor
            }

            match bits.read_bits(2)? {
                1 => bits.skip_bits(5)?,
                2 => bits.skip_bits(12)?,
                3 => {
                    let mix_data_size = (bits.read_bits(5)? + 2) * 8;
                    bits.skip_bits(mix_data_size as usize)?;
                }
                _ => {}
            }

            if acmod < 2 {
                for _ in 0..dialnorm_count {
                    if bits.read_bit()? != 0 {
                        bits.skip_bits(14)?; // pan information
                    }
                }
            }

            if bits.read_bit()? != 0 {
                for _ in 0..num_blocks {
                    if num_blocks == 1 || bits.read_bit()? != 0 {
                        bits.skip_bits(5)?;
                    }
                }
            }
        }
    }

    if bits.read_bit()? != 0 {
        bits.skip_bits(3)?; // bitstream mode
        bits.skip_bits(2)?; // copyright/original
        if acmod == 2 {
            bits.skip_bits(4)?; // dolby surround + headphone mode
        }
        if acmod >= 6 {
            bits.skip_bits(2)?; // dolby surround ex mode
        }
        for _ in 0..dialnorm_count {
            if bits.read_bit()? != 0 {
                bits.skip_bits(8)?; // mix level / room type / A/D converter
            }
        }
        if fscod != 3 {
            bits.skip_bits(1)?; // source sample rate code
        }
    }

    if frame_type == 0 && num_blocks != 6 {
        bits.skip_bits(1)?; // converter synchronization flag
    }

    if frame_type == 2 && (num_blocks == 6 || bits.read_bit()? != 0) {
        bits.skip_bits(6)?; // original frame size code
    }

    if bits.read_bit()? != 0 {
        let addbsil = bits.read_bits(6)? as usize;
        for index in 0..=addbsil {
            if index == 0 {
                bits.skip_bits(7)?;
                let atmos = bits.read_bit()? != 0;
                if atmos {
                    return Some("Dolby Digital Plus + Dolby Atmos".into());
                }
            } else {
                bits.skip_bits(8)?;
            }
        }
    }

    None
}

fn detect_truehd_profile_from_payload(data: &[u8]) -> Option<String> {
    let start = find_truehd_major_sync_offset(data)?;
    let payload = &data[start..];
    let header_size = truehd_major_sync_size(payload)?;
    if payload.len() < header_size {
        return None;
    }

    let mut bits = AudioBitReader::new(payload);
    if bits.read_bits(24)? != 0xF8_72_6F {
        return None;
    }

    let stream_type = bits.read_bits(8)? as u8;
    if stream_type != 0xBA {
        return None;
    }

    let ratebits = bits.read_bits(4)?;
    let _ = ratebits;
    bits.skip_bits(4)?;
    bits.skip_bits(2)?;
    bits.skip_bits(2)?;
    bits.skip_bits(5)?;
    bits.skip_bits(2)?;
    bits.skip_bits(13)?;
    bits.skip_bits(48)?;
    let _is_vbr = bits.read_bit()?;
    bits.skip_bits(15)?;
    let num_substreams = bits.read_bits(4)? as u8;
    bits.skip_bits(2)?;
    let _extended_substream_info = bits.read_bits(2)?;
    let substream_info = bits.read_bits(8)? as u8;

    if num_substreams == 4 && (substream_info >> 7) == 1 {
        Some("Dolby TrueHD + Dolby Atmos".into())
    } else {
        None
    }
}

fn find_truehd_major_sync_offset(data: &[u8]) -> Option<usize> {
    const SEARCH_LIMIT: usize = 32;
    let max_start = data
        .len()
        .saturating_sub(TRUEHD_MAJOR_SYNCWORD.len())
        .min(SEARCH_LIMIT);
    (0..=max_start).find(|&start| {
        data.get(start..start + TRUEHD_MAJOR_SYNCWORD.len()) == Some(TRUEHD_MAJOR_SYNCWORD.as_ref())
    })
}

fn truehd_major_sync_size(payload: &[u8]) -> Option<usize> {
    if payload.len() < 28 || !payload.starts_with(&TRUEHD_MAJOR_SYNCWORD) {
        return None;
    }

    let mut size = 28usize;
    if (payload[25] & 1) != 0 {
        let extensions = (payload.get(26)? >> 4) as usize;
        size += 2 + extensions * 2;
    }
    Some(size)
}

fn detect_dts_profile_from_probe_bytes(prefix: &[u8], suffix: Option<&[u8]>) -> Option<String> {
    let core = parse_dts_core_header(prefix);
    let has_core = core.is_some();
    let [has_xch, has_xxch, has_x96] = crate::scan::syncword_presence(
        prefix,
        [DTS_SYNCWORD_XCH, DTS_SYNCWORD_XXCH, DTS_SYNCWORD_X96],
    );
    let has_xch = has_xch || has_xxch;
    if let Some(exss) = parse_dts_exss_asset(prefix) {
        let exss_follows_core = core
            .map(|core| exss.exss_start >= core.frame_size)
            .unwrap_or(false);
        if exss_follows_core
            && (exss.extension_mask & DCA_EXSS_XLL) != 0
            && dts_exss_has_valid_xll_sync(prefix, &exss)
        {
            let has_x = dts_exss_remainder_has_syncword(prefix, &exss, DTS_SYNCWORD_XLL_X)
                || suffix.is_some_and(|tail| contains_aligned_syncword(tail, DTS_SYNCWORD_XLL_X));
            let has_x_imax =
                dts_exss_remainder_has_shifted_syncword(prefix, &exss, DTS_SYNCWORD_XLL_X_IMAX)
                    || suffix.is_some_and(|tail| {
                        contains_aligned_syncword_shifted(tail, DTS_SYNCWORD_XLL_X_IMAX)
                    });
            if has_x_imax {
                return Some("DTS-HD MA + DTS:X IMAX".into());
            }
            if has_x {
                return Some("DTS-HD MA + DTS:X".into());
            }
            return Some("DTS-HD MA".into());
        }
        if (exss.extension_mask & DCA_EXSS_XBR) != 0 {
            return Some("DTS-HD HRA".into());
        }
        if (exss.extension_mask & DCA_EXSS_LBR) != 0 {
            return Some("DTS Express".into());
        }
    }
    if has_x96 {
        return Some("DTS 96/24".into());
    }
    if has_xch {
        return Some("DTS-ES".into());
    }
    if has_core {
        return Some("DTS".into());
    }
    None
}

fn contains_aligned_syncword(data: &[u8], syncword: u32) -> bool {
    if data.len() < 4 {
        return false;
    }
    let start = data.len() % 4;
    data[start..]
        .chunks_exact(4)
        .any(|chunk| u32::from_be_bytes(chunk.try_into().expect("chunk length")) == syncword)
}

fn contains_aligned_syncword_shifted(data: &[u8], syncword: u32) -> bool {
    if data.len() < 4 {
        return false;
    }
    let start = data.len() % 4;
    data[start..].chunks_exact(4).any(|chunk| {
        (u32::from_be_bytes(chunk.try_into().expect("chunk length")) >> 1) == (syncword >> 1)
    })
}

#[derive(Debug, Clone, Copy)]
struct DtsCoreHeaderInfo {
    frame_size: usize,
}

#[derive(Debug, Clone, Copy)]
struct DtsExssAssetInfo {
    exss_start: usize,
    nchannels_total: i32,
    extension_mask: u16,
    asset_size: usize,
    xll_offset: usize,
    xll_size: usize,
    xll_sync_offset: usize,
}

fn parse_dts_core_header(data: &[u8]) -> Option<DtsCoreHeaderInfo> {
    if data.len() < 11 {
        return None;
    }
    for start in 0..=data.len() - 11 {
        let marker = u32::from_be_bytes(data[start..start + 4].try_into().ok()?);
        if !matches!(
            marker,
            DTS_SYNCWORD_CORE_BE
                | DTS_SYNCWORD_CORE_LE
                | DTS_SYNCWORD_CORE_14B_BE
                | DTS_SYNCWORD_CORE_14B_LE
        ) {
            continue;
        }

        let probe_end = (start + DTS_CORE_PROBE_BYTES).min(data.len());
        let normalized = normalize_dts_core_prefix(&data[start..probe_end])?;
        let mut bits = AudioBitReader::new(&normalized);
        if bits.read_bits(32)? != DTS_SYNCWORD_CORE_BE {
            continue;
        }
        bits.read_bit()?;
        let deficit_samples = bits.read_bits(5)? as u8 + 1;
        if deficit_samples != 32 {
            continue;
        }
        bits.skip_bits(1)?;
        let npcmblocks = bits.read_bits(7)? as u8 + 1;
        if (npcmblocks & 0x07) != 0 {
            continue;
        }
        let frame_size = bits.read_bits(14)? + 1;
        if frame_size < 96 {
            continue;
        }
        let audio_mode = bits.read_bits(6)? as usize;
        if audio_mode >= DTS_CHANNELS.len() {
            continue;
        }
        let sample_rate_code = bits.read_bits(4)? as usize;
        if *DTS_SAMPLE_RATES.get(sample_rate_code)? == 0 {
            continue;
        }
        let _bit_rate_code = bits.read_bits(5)? as usize;
        if bits.read_bit()? != 0 {
            continue;
        }
        bits.skip_bits(1 + 1 + 1 + 1 + 3 + 1 + 1)?;
        let lfe_present = bits.read_bits(2)? as u8;
        if lfe_present == 0x3 {
            continue;
        }

        let _channels = DTS_CHANNELS[audio_mode] + u8::from(lfe_present > 0);
        return Some(DtsCoreHeaderInfo {
            frame_size: frame_size as usize,
        });
    }

    None
}

fn normalize_dts_core_prefix(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 4 {
        return None;
    }

    let marker = u32::from_be_bytes(data[..4].try_into().ok()?);
    match marker {
        DTS_SYNCWORD_CORE_BE => Some(data.to_vec()),
        DTS_SYNCWORD_CORE_LE => {
            let mut normalized = Vec::with_capacity(data.len());
            for chunk in data.chunks(2) {
                if chunk.len() == 2 {
                    normalized.push(chunk[1]);
                    normalized.push(chunk[0]);
                } else {
                    normalized.push(chunk[0]);
                }
            }
            Some(normalized)
        }
        DTS_SYNCWORD_CORE_14B_BE | DTS_SYNCWORD_CORE_14B_LE => {
            let mut normalized = Vec::with_capacity((data.len() * 14) / 16 + 2);
            let mut bit_buffer = 0u32;
            let mut bits_in_buffer = 0usize;

            for chunk in data.chunks_exact(2) {
                let word = if marker == DTS_SYNCWORD_CORE_14B_BE {
                    u16::from_be_bytes([chunk[0], chunk[1]])
                } else {
                    u16::from_le_bytes([chunk[0], chunk[1]])
                } & 0x3FFF;

                bit_buffer = (bit_buffer << 14) | u32::from(word);
                bits_in_buffer += 14;

                while bits_in_buffer >= 8 {
                    bits_in_buffer -= 8;
                    normalized.push(((bit_buffer >> bits_in_buffer) & 0xFF) as u8);
                    bit_buffer &= (1u32 << bits_in_buffer).saturating_sub(1);
                }
            }

            Some(normalized)
        }
        _ => None,
    }
}

fn parse_dts_exss_asset(data: &[u8]) -> Option<DtsExssAssetInfo> {
    if data.len() < 16 {
        return None;
    }

    for exss_start in 0..=data.len() - 4 {
        let marker = u32::from_be_bytes(data[exss_start..exss_start + 4].try_into().ok()?);
        if marker != DTS_SYNCWORD_SUBSTREAM {
            continue;
        }
        if let Some(asset) = parse_dts_exss_asset_at(data, exss_start) {
            return Some(asset);
        }
    }

    None
}

fn parse_dts_exss_asset_at(data: &[u8], exss_start: usize) -> Option<DtsExssAssetInfo> {
    let mut bits = AudioBitReader::new(&data[exss_start..]);
    if bits.read_bits(32)? != DTS_SYNCWORD_SUBSTREAM {
        return None;
    }

    bits.skip_bits(8)?;
    let exss_index = bits.read_bits(2)? as usize;
    let wide_hdr = bits.read_bit()? as usize;
    let header_size = bits.read_bits(8 + 4 * wide_hdr)? as usize + 1;
    let exss_size_nbits = 16 + 4 * wide_hdr;
    let _exss_size = bits.read_bits(exss_size_nbits)? as usize + 1;
    let static_fields_present = bits.read_bit()? != 0;

    let (nassets, mix_metadata_enabled, nmixoutconfigs, nmixoutchs) = if static_fields_present {
        bits.skip_bits(2)?;
        bits.skip_bits(3)?;
        if bits.read_bit()? != 0 {
            bits.skip_bits(32 + 4)?;
        }
        let npresents = bits.read_bits(3)? as usize + 1;
        let nassets = bits.read_bits(3)? as usize + 1;
        for _ in 0..npresents {
            let mask = bits.read_bits(exss_index + 1)?;
            bits.skip_bits(mask.count_ones() as usize * 8)?;
        }
        let mix_metadata_enabled = bits.read_bit()? != 0;
        let mut nmixoutchs = [0usize; 4];
        let nmixoutconfigs = if mix_metadata_enabled {
            bits.skip_bits(2)?;
            let spkr_mask_nbits = ((bits.read_bits(2)? as usize) + 1) << 2;
            let nmixoutconfigs = bits.read_bits(2)? as usize + 1;
            for slot in nmixoutchs.iter_mut().take(nmixoutconfigs) {
                *slot = count_dts_channels_for_mask(bits.read_bits(spkr_mask_nbits)?);
            }
            nmixoutconfigs
        } else {
            0
        };
        (nassets, mix_metadata_enabled, nmixoutconfigs, nmixoutchs)
    } else {
        (1, false, 0, [0; 4])
    };

    let mut asset_ranges = Vec::with_capacity(nassets);
    let mut next_asset_offset = header_size;
    for _ in 0..nassets {
        let size = bits.read_bits(exss_size_nbits)? as usize + 1;
        asset_ranges.push((next_asset_offset, size));
        next_asset_offset = next_asset_offset.checked_add(size)?;
    }

    let mut best: Option<DtsExssAssetInfo> = None;
    for (asset_offset, asset_size) in asset_ranges {
        let asset = parse_dts_exss_descriptor(
            &mut bits,
            static_fields_present,
            mix_metadata_enabled,
            nmixoutconfigs,
            &nmixoutchs,
            exss_size_nbits,
            exss_start,
            asset_offset,
            asset_size,
        )?;
        best = Some(match best {
            Some(current) => merge_dts_exss_assets(current, asset),
            None => asset,
        });
    }

    best
}

#[allow(clippy::too_many_arguments)]
fn parse_dts_exss_descriptor(
    bits: &mut AudioBitReader<'_>,
    static_fields_present: bool,
    mix_metadata_enabled: bool,
    nmixoutconfigs: usize,
    nmixoutchs: &[usize; 4],
    exss_size_nbits: usize,
    exss_start: usize,
    asset_offset: usize,
    asset_size: usize,
) -> Option<DtsExssAssetInfo> {
    let descr_size = bits.read_bits(9)? as usize + 1;
    let descr_start = bits.bit_pos;
    bits.skip_bits(3)?;

    let mut nchannels_total = 0usize;
    let mut embedded_stereo = false;
    let mut embedded_6ch = false;
    if static_fields_present {
        if bits.read_bit()? != 0 {
            bits.skip_bits(4)?;
        }
        if bits.read_bit()? != 0 {
            bits.skip_bits(24)?;
        }
        if bits.read_bit()? != 0 {
            let text_size = bits.read_bits(10)? as usize + 1;
            bits.skip_bits(text_size * 8)?;
        }
        bits.skip_bits(5)?;
        bits.skip_bits(4)?;
        nchannels_total = bits.read_bits(8)? as usize + 1;
        let one_to_one_map_ch_to_spkr = bits.read_bit()? != 0;
        if one_to_one_map_ch_to_spkr {
            embedded_stereo = nchannels_total > 2 && bits.read_bit()? != 0;
            embedded_6ch = nchannels_total > 6 && bits.read_bit()? != 0;
            let spkr_mask_enabled = bits.read_bit()? != 0;
            let spkr_mask_nbits = if spkr_mask_enabled {
                ((bits.read_bits(2)? as usize) + 1) << 2
            } else {
                0
            };
            if spkr_mask_enabled {
                bits.skip_bits(spkr_mask_nbits)?;
            }
            let spkr_remap_nsets = bits.read_bits(3)? as usize;
            if spkr_remap_nsets != 0 && !spkr_mask_enabled {
                return None;
            }
            for _ in 0..spkr_remap_nsets {
                let nspeakers = count_dts_channels_for_mask(bits.read_bits(spkr_mask_nbits)?);
                let nch_for_remaps = bits.read_bits(5)? as usize + 1;
                for _ in 0..nspeakers {
                    let remap_mask = bits.read_bits(nch_for_remaps)?;
                    bits.skip_bits(remap_mask.count_ones() as usize * 5)?;
                }
            }
        } else {
            bits.skip_bits(3)?;
        }
    }

    let drc_present = bits.read_bit()? != 0;
    if drc_present {
        bits.skip_bits(8)?;
    }
    if bits.read_bit()? != 0 {
        bits.skip_bits(5)?;
    }
    if drc_present && embedded_stereo {
        bits.skip_bits(8)?;
    }
    if mix_metadata_enabled && bits.read_bit()? != 0 {
        bits.skip_bits(1)?;
        bits.skip_bits(6)?;
        if bits.read_bits(2)? == 3 {
            bits.skip_bits(8)?;
        } else {
            bits.skip_bits(3)?;
        }
        if bits.read_bit()? != 0 {
            for &mixoutch in nmixoutchs.iter().take(nmixoutconfigs) {
                bits.skip_bits(6 * mixoutch)?;
            }
        } else {
            bits.skip_bits(6 * nmixoutconfigs)?;
        }

        let nchannels_dmix =
            nchannels_total + usize::from(embedded_6ch) * 6 + usize::from(embedded_stereo) * 2;
        for &mixoutch in nmixoutchs.iter().take(nmixoutconfigs) {
            if mixoutch == 0 {
                return None;
            }
            for _ in 0..nchannels_dmix {
                let mix_map_mask = bits.read_bits(mixoutch)?;
                bits.skip_bits(mix_map_mask.count_ones() as usize * 6)?;
            }
        }
    }

    let coding_mode = bits.read_bits(2)? as u8;
    let mut extension_mask = 0u16;
    let mut xll_offset = 0usize;
    let mut xll_size = 0usize;
    let mut xll_sync_offset = 0usize;

    match coding_mode {
        0 => {
            extension_mask = bits.read_bits(12)? as u16;
            let mut offset = asset_offset;
            if (extension_mask & DCA_EXSS_CORE) != 0 {
                let core_size = bits.read_bits(14)? as usize + 1;
                if bits.read_bit()? != 0 {
                    bits.skip_bits(2)?;
                }
                offset += core_size;
            }
            if (extension_mask & DCA_EXSS_XBR) != 0 {
                offset += bits.read_bits(14)? as usize + 1;
            }
            if (extension_mask & DCA_EXSS_XXCH) != 0 {
                offset += bits.read_bits(14)? as usize + 1;
            }
            if (extension_mask & DCA_EXSS_X96) != 0 {
                offset += bits.read_bits(12)? as usize + 1;
            }
            if (extension_mask & DCA_EXSS_LBR) != 0 {
                offset += parse_dts_exss_lbr(bits)?;
            }
            if (extension_mask & DCA_EXSS_XLL) != 0 {
                let (sync_offset, size) = parse_dts_exss_xll(bits, exss_size_nbits)?;
                xll_offset = offset;
                xll_size = size;
                xll_sync_offset = sync_offset;
            }
            if (extension_mask & DCA_EXSS_RSV1) != 0 {
                let _ = bits.read_bits(16)? as usize + 1;
            }
            if (extension_mask & DCA_EXSS_RSV2) != 0 {
                let _ = bits.read_bits(16)? as usize + 1;
            }
        }
        1 => {
            extension_mask = DCA_EXSS_XLL;
            let (sync_offset, size) = parse_dts_exss_xll(bits, exss_size_nbits)?;
            xll_offset = asset_offset;
            xll_size = size;
            xll_sync_offset = sync_offset;
        }
        2 => {
            extension_mask = DCA_EXSS_LBR;
            let _ = parse_dts_exss_lbr(bits)?;
        }
        3 => {
            bits.skip_bits(14 + 8)?;
            if bits.read_bit()? != 0 {
                bits.skip_bits(3)?;
            }
        }
        _ => return None,
    }

    if (extension_mask & DCA_EXSS_XLL) != 0 {
        bits.skip_bits(3)?;
    }

    let descr_end = descr_start.checked_add(descr_size * 8)?;
    if bits.bit_pos > descr_end {
        return None;
    }
    bits.skip_bits(descr_end - bits.bit_pos)?;

    Some(DtsExssAssetInfo {
        exss_start,
        nchannels_total: nchannels_total as i32,
        extension_mask,
        asset_size,
        xll_offset,
        xll_size,
        xll_sync_offset,
    })
}

fn parse_dts_exss_lbr(bits: &mut AudioBitReader<'_>) -> Option<usize> {
    let lbr_size = bits.read_bits(14)? as usize + 1;
    if bits.read_bit()? != 0 {
        bits.skip_bits(2)?;
    }
    Some(lbr_size)
}

fn merge_dts_exss_assets(
    current: DtsExssAssetInfo,
    candidate: DtsExssAssetInfo,
) -> DtsExssAssetInfo {
    if dts_extension_rank(candidate.extension_mask) > dts_extension_rank(current.extension_mask) {
        DtsExssAssetInfo {
            nchannels_total: current.nchannels_total.max(candidate.nchannels_total),
            ..candidate
        }
    } else {
        DtsExssAssetInfo {
            nchannels_total: current.nchannels_total.max(candidate.nchannels_total),
            ..current
        }
    }
}

fn dts_extension_rank(extension_mask: u16) -> i32 {
    if (extension_mask & DCA_EXSS_XLL) != 0 {
        3
    } else if (extension_mask & DCA_EXSS_XBR) != 0 {
        2
    } else if (extension_mask & DCA_EXSS_LBR) != 0 {
        1
    } else {
        0
    }
}

fn parse_dts_exss_xll(
    bits: &mut AudioBitReader<'_>,
    exss_size_nbits: usize,
) -> Option<(usize, usize)> {
    let xll_size = bits.read_bits(14)? as usize + 1;
    let xll_sync_offset = if bits.read_bit()? != 0 {
        bits.skip_bits(4)?;
        let xll_delay_nbits = bits.read_bits(5)? as usize + 1;
        bits.skip_bits(xll_delay_nbits)?;
        bits.read_bits(exss_size_nbits)? as usize
    } else {
        0
    };
    Some((xll_sync_offset, xll_size))
}

fn dts_exss_has_valid_xll_sync(prefix: &[u8], info: &DtsExssAssetInfo) -> bool {
    let Some(sync_start) = dts_exss_xll_sync_start(info) else {
        return false;
    };
    let Some(sync_bytes) = prefix.get(sync_start..sync_start + 4) else {
        return false;
    };
    if u32::from_be_bytes(sync_bytes.try_into().expect("sync length")) != DTS_SYNCWORD_XLL {
        return false;
    }

    let available_prefix = prefix.len().saturating_sub(sync_start);
    let available = info.asset_size.min(available_prefix);
    dts_xll_common_header_is_valid(&prefix[sync_start..sync_start + available], info)
}

fn dts_exss_remainder_has_syncword(prefix: &[u8], info: &DtsExssAssetInfo, syncword: u32) -> bool {
    dts_exss_remainder_region(prefix, info)
        .is_some_and(|region| contains_aligned_syncword(region, syncword))
}

fn dts_exss_remainder_has_shifted_syncword(
    prefix: &[u8],
    info: &DtsExssAssetInfo,
    syncword: u32,
) -> bool {
    dts_exss_remainder_region(prefix, info)
        .is_some_and(|region| contains_aligned_syncword_shifted(region, syncword))
}

fn dts_exss_xll_sync_start(info: &DtsExssAssetInfo) -> Option<usize> {
    let start = info
        .exss_start
        .checked_add(info.xll_offset)?
        .checked_add(info.xll_sync_offset)?;
    let end = info
        .exss_start
        .checked_add(info.xll_offset)?
        .checked_add(info.xll_size)?;
    (start.checked_add(4)? <= end).then_some(start)
}

fn dts_xll_common_header_is_valid(data: &[u8], info: &DtsExssAssetInfo) -> bool {
    let mut bits = AudioBitReader::new(data);
    if bits.read_bits(32) != Some(DTS_SYNCWORD_XLL) {
        return false;
    }

    let stream_ver = bits.read_bits(4).map(|value| value + 1);
    if stream_ver != Some(1) {
        return false;
    }

    let Some(header_size) = bits.read_bits(8).map(|value| value as usize + 1) else {
        return false;
    };
    if header_size > info.asset_size {
        return false;
    }
    let Some(frame_size_nbits) = bits.read_bits(5).map(|value| value as usize + 1) else {
        return false;
    };
    let Some(frame_size) = bits
        .read_bits(frame_size_nbits)
        .map(|value| value as usize + 1)
    else {
        return false;
    };

    if frame_size > info.asset_size || frame_size >= DTS_XLL_PBR_BUFFER_MAX {
        return false;
    }

    let Some(nchsets) = bits.read_bits(4).map(|value| value + 1) else {
        return false;
    };
    if nchsets > DTS_XLL_CHSETS_MAX {
        return false;
    }

    let Some(nframesegs_log2) = bits.read_bits(4) else {
        return false;
    };
    let nframesegs = 1u32 << nframesegs_log2;
    if nframesegs > 1024 {
        return false;
    }

    let Some(nsegsamples_log2) = bits.read_bits(4) else {
        return false;
    };
    if nsegsamples_log2 == 0 {
        return false;
    }
    let nsegsamples = 1u32 << nsegsamples_log2;
    if nsegsamples > 512 {
        return false;
    }

    let nframesamples = 1u32 << (nsegsamples_log2 + nframesegs_log2);
    if nframesamples > 65_536 {
        return false;
    }

    let Some(ch_mask_nbits) = bits.read_bits(5).map(|value| value + 1) else {
        return false;
    };
    if ch_mask_nbits == 0 || ch_mask_nbits > 32 {
        return false;
    }

    let Some(scalable_lsbs) = bits.read_bit() else {
        return false;
    };
    if scalable_lsbs != 0 && bits.read_bits(4).is_none() {
        return false;
    }

    let _ = ch_mask_nbits;
    true
}

fn dts_exss_remainder_region<'a>(prefix: &'a [u8], info: &DtsExssAssetInfo) -> Option<&'a [u8]> {
    let start = dts_exss_xll_sync_start(info)?;
    let end = info
        .exss_start
        .checked_add(info.asset_size)?
        .min(prefix.len());
    prefix.get(start..end)
}

fn count_dts_channels_for_mask(mask: u32) -> usize {
    ((mask & 0xFFFF) | ((mask & 0xAE66) << 16)).count_ones() as usize
}

struct AudioBitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> AudioBitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn read_bit(&mut self) -> Option<u8> {
        Some(self.read_bits(1)? as u8)
    }

    fn read_bits(&mut self, count: usize) -> Option<u32> {
        if count == 0 || count > 32 || self.bit_pos + count > self.data.len() * 8 {
            return None;
        }

        let mut value = 0_u32;
        for _ in 0..count {
            let byte_index = self.bit_pos / 8;
            let bit_index = 7 - (self.bit_pos % 8);
            value = (value << 1) | u32::from((self.data[byte_index] >> bit_index) & 0x01);
            self.bit_pos += 1;
        }
        Some(value)
    }

    fn peek_bits(&self, count: usize) -> Option<u32> {
        if count == 0 || count > 32 || self.bit_pos + count > self.data.len() * 8 {
            return None;
        }

        let mut value = 0_u32;
        for bit_pos in self.bit_pos..(self.bit_pos + count) {
            let byte_index = bit_pos / 8;
            let bit_index = 7 - (bit_pos % 8);
            value = (value << 1) | u32::from((self.data[byte_index] >> bit_index) & 0x01);
        }
        Some(value)
    }

    fn skip_bits(&mut self, count: usize) -> Option<()> {
        if self.bit_pos + count > self.data.len() * 8 {
            return None;
        }
        self.bit_pos += count;
        Some(())
    }

    fn bits_left(&self) -> usize {
        self.data.len() * 8 - self.bit_pos
    }

    fn read_aac_audio_object_type(&mut self) -> Option<u8> {
        let object_type = self.read_bits(5)? as u8;
        if object_type == 31 {
            Some(32 + self.read_bits(6)? as u8)
        } else {
            Some(object_type)
        }
    }

    fn read_aac_sample_rate(&mut self) -> Option<u32> {
        const AAC_SAMPLE_RATES: [u32; 16] = [
            96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
            8_000, 7_350, 0, 0, 0,
        ];

        let sample_rate_index = self.read_bits(4)? as usize;
        if sample_rate_index == 0xF {
            self.read_bits(24)
        } else {
            AAC_SAMPLE_RATES.get(sample_rate_index).copied()
        }
    }
}

/// Extracts codec profile and bit depth from an H.264 AVCDecoderConfigurationRecord.
///
/// The input bytes are the raw codec-private data stored in the container
/// (e.g. MKV CodecPrivate or MP4 avcC box contents).
pub(crate) fn is_valid_h264_avcc(codec_private: &[u8]) -> bool {
    use bytes::Bytes;
    use scuffle_h264::{AVCDecoderConfigurationRecord, Sps};
    use std::io;

    let Ok(config) = AVCDecoderConfigurationRecord::parse(&mut io::Cursor::new(
        Bytes::copy_from_slice(codec_private),
    )) else {
        return false;
    };
    if config.configuration_version != 1 || config.sps.is_empty() || config.pps.is_empty() {
        return false;
    }
    config.sps.iter().any(|sps_bytes| {
        let rbsp = scan::h2645_unescape_rbsp(sps_bytes);
        Sps::parse(io::Cursor::new(rbsp.as_ref())).is_ok()
    })
}

pub(crate) fn extract_h264_info(codec_private: &[u8]) -> CodecInfo {
    use bytes::Bytes;
    use scuffle_h264::{AVCDecoderConfigurationRecord, Sps};
    use std::io;

    let data = Bytes::from(codec_private.to_vec());
    if let Ok(config) = AVCDecoderConfigurationRecord::parse(&mut io::Cursor::new(data)) {
        let profile = map_h264_profile(config.profile_indication);
        let parse_sps = |sps_bytes: &[u8]| {
            let rbsp = scan::h2645_unescape_rbsp(sps_bytes);
            Sps::parse(io::Cursor::new(rbsp.as_ref())).ok()
        };

        // Bit depth: prefer the extended_config field (from the AVCC record itself),
        // fall back to parsing the first SPS NAL unit.
        let bit_depth = if let Some(ref ext) = config.extended_config {
            Some(ext.bit_depth_luma_minus8 as i32 + 8)
        } else {
            config.sps.first().and_then(|sps_bytes| {
                parse_sps(sps_bytes)
                    .and_then(|sps| sps.ext.map(|ext| ext.bit_depth_luma_minus8 as i32 + 8))
            })
        };

        let first_sps = config
            .sps
            .first()
            .and_then(|sps_bytes| parse_sps(sps_bytes));
        let color_transfer = first_sps.as_ref().and_then(|sps| {
            let transfer = sps.color_config.as_ref()?.transfer_characteristics as u32;
            if transfer > 0 && transfer != 2 {
                Some(transfer)
            } else {
                None
            }
        });

        return CodecInfo {
            profile,
            bit_depth: bit_depth.or(Some(8)),
            color_transfer,
            width: first_sps
                .as_ref()
                .and_then(|sps| i32::try_from(sps.width()).ok()),
            height: first_sps
                .as_ref()
                .and_then(|sps| i32::try_from(sps.height()).ok()),
            frame_rate_fps: first_sps.as_ref().and_then(Sps::frame_rate),
        };
    }

    let rbsp = scan::h2645_unescape_rbsp(codec_private);
    let sps = match Sps::parse(io::Cursor::new(rbsp.as_ref())) {
        Ok(sps) => sps,
        Err(_) => return CodecInfo::default(),
    };
    let bit_depth = sps
        .ext
        .as_ref()
        .map(|ext| ext.bit_depth_luma_minus8 as i32 + 8)
        .or(Some(8));
    let color_transfer = sps.color_config.as_ref().and_then(|color| {
        let transfer = color.transfer_characteristics as u32;
        if transfer > 0 && transfer != 2 {
            Some(transfer)
        } else {
            None
        }
    });

    CodecInfo {
        profile: map_h264_profile(sps.profile_idc),
        bit_depth,
        color_transfer,
        width: i32::try_from(sps.width()).ok(),
        height: i32::try_from(sps.height()).ok(),
        frame_rate_fps: sps.frame_rate(),
    }
}

/// Extracts codec profile, bit depth, and color transfer characteristics from
/// an HEVC (H.265) decoder configuration record.
pub(crate) fn extract_h265_info(codec_private: &[u8]) -> CodecInfo {
    use scuffle_h265::{HEVCDecoderConfigurationRecord, NALUnitType, SpsNALUnit};
    use std::io;

    if let Ok(config) = HEVCDecoderConfigurationRecord::demux(&mut io::Cursor::new(codec_private)) {
        let profile = map_h265_profile(config.general_profile_idc);
        let bit_depth = Some(config.bit_depth_luma_minus8 as i32 + 8);

        // Extract transfer_characteristics from the first SPS VUI.
        let color_transfer = config
            .arrays
            .iter()
            .filter(|arr| arr.nal_unit_type == NALUnitType::SpsNut)
            .flat_map(|arr| arr.nalus.iter())
            .find_map(|nalu_bytes| {
                let sps = SpsNALUnit::parse(io::Cursor::new(nalu_bytes.clone())).ok()?;
                let vui = sps.rbsp.vui_parameters?;
                let tc = vui.video_signal_type.transfer_characteristics;
                if tc > 0 && tc != 2 {
                    Some(tc as u32)
                } else {
                    None
                }
            });

        return CodecInfo {
            profile,
            bit_depth,
            color_transfer,
            ..CodecInfo::default()
        };
    }

    let sps = match SpsNALUnit::parse(io::Cursor::new(codec_private)) {
        Ok(sps) => sps,
        Err(_) => return CodecInfo::default(),
    };
    let color_transfer = sps.rbsp.vui_parameters.as_ref().and_then(|vui| {
        let tc = vui.video_signal_type.transfer_characteristics;
        if tc > 0 && tc != 2 {
            Some(tc as u32)
        } else {
            None
        }
    });

    CodecInfo {
        profile: map_h265_profile(sps.rbsp.profile_tier_level.general_profile.profile_idc),
        bit_depth: Some(sps.rbsp.bit_depth_luma_minus8 as i32 + 8),
        color_transfer,
        ..CodecInfo::default()
    }
}

/// Extracts codec profile and bit depth from an AV1 codec configuration record
/// (AV1CodecConfigurationRecord, 4 bytes).
///
/// Layout (ISO/IEC 14496-12 AV1 binding):
/// - Byte 0: marker(1) | version(7) — must be 0x81
/// - Byte 1: seq_profile(3) | seq_level_idx_0(5)
/// - Byte 2: seq_tier_0(1) | high_bitdepth(1) | twelve_bit(1) | monochrome(1) |
///   chroma_subsampling_x(1) | chroma_subsampling_y(1) | chroma_sample_position(2)
/// - Byte 3: initial_presentation_delay fields
pub(crate) fn extract_av1_info(codec_private: &[u8]) -> CodecInfo {
    if codec_private.len() < 4 {
        return CodecInfo::default();
    }

    let marker_version = codec_private[0];
    if marker_version != 0x81 {
        return CodecInfo::default();
    }

    let seq_profile = (codec_private[1] >> 5) & 0x07;
    let high_bitdepth = (codec_private[2] >> 6) & 0x01;
    let twelve_bit = (codec_private[2] >> 5) & 0x01;

    let profile = match seq_profile {
        0 => Some("Main".into()),
        1 => Some("High".into()),
        2 => Some("Professional".into()),
        _ => None,
    };

    let bit_depth: i32 = if high_bitdepth == 0 {
        8
    } else if twelve_bit == 1 {
        12
    } else {
        10
    };

    CodecInfo {
        profile,
        bit_depth: Some(bit_depth),
        color_transfer: None,
        ..CodecInfo::default()
    }
}

/// Parsed fields from a DOVIDecoderConfigurationRecord.
#[derive(Debug, Clone)]
pub(crate) struct DoviConfigInfo {
    pub profile: u8,
    pub level: u8,
    pub rpu_present: bool,
    pub enhancement_layer_present: bool,
    pub base_layer_present: bool,
    pub bl_signal_compatibility_id: u8,
}

/// Parse a DOVIDecoderConfigurationRecord (≥5 bytes).
///
/// Layout:
/// - Byte 0: dv_version_major
/// - Byte 1: dv_version_minor
/// - Byte 2-3: dv_profile(7) | dv_level(6) | rpu_present_flag(1) |
///   el_present_flag(1) | bl_present_flag(1)
/// - Byte 4: dv_bl_signal_compatibility_id(4) | reserved(4)
pub(crate) fn parse_dovi_config(data: &[u8]) -> Option<DoviConfigInfo> {
    if data.len() < 5 {
        return None;
    }
    let dv_profile = (data[2] >> 1) & 0x7F;
    let bl_compat = (data[4] >> 4) & 0x0F;
    Some(DoviConfigInfo {
        profile: dv_profile,
        level: ((data[2] & 1) << 5) | (data[3] >> 3),
        rpu_present: data[3] & 4 != 0,
        enhancement_layer_present: data[3] & 2 != 0,
        base_layer_present: data[3] & 1 != 0,
        bl_signal_compatibility_id: bl_compat,
    })
}

#[cfg(test)]
fn detect_hdr_format(track: &RawTrack) -> Option<String> {
    crate::build_analysis(crate::types::RawContainer {
        format_name: "fixture".into(),
        duration_seconds: None,
        num_chapters: None,
        tracks: vec![track.clone()],
        details: Default::default(),
    })
    .video_hdr_format
}

/// Extract the NAL unit length prefix size from an HEVCDecoderConfigurationRecord.
///
/// Returns `lengthSizeMinusOne + 1` (typically 4). Falls back to 4 if the
/// record is too short.
pub(crate) fn hevc_nal_length_size(hvcc: &[u8]) -> usize {
    if hvcc.len() > 21 {
        ((hvcc[21] & 0x03) as usize) + 1
    } else {
        4
    }
}

/// Scan an HEVC video frame (length-prefixed NAL units) for HDR10+ SEI metadata
/// (SMPTE ST 2094-40).
///
/// Returns `true` if a registered user data SEI with country_code 0xB5 (USA),
/// provider_code 0x003C (Samsung), and provider_oriented_code 0x0001 is found.
pub(crate) fn scan_hevc_frame_for_hdr10plus(frame: &[u8], nal_length_size: usize) -> bool {
    if !(1..=4).contains(&nal_length_size) {
        return false;
    }

    let mut offset = 0;
    while offset + nal_length_size <= frame.len() {
        let Some(candidate) =
            scan::find_hevc_sei_nal_header_candidate(frame, offset + nal_length_size)
        else {
            return false;
        };
        let nal_len = read_be_length(&frame[offset..], nal_length_size);
        offset += nal_length_size;
        if nal_len == 0 || offset + nal_len > frame.len() {
            break;
        }
        if candidate.offset >= offset + nal_len {
            offset += nal_len;
            continue;
        }
        let nal_data = &frame[offset..offset + nal_len];
        // HEVC NAL header is 2 bytes. nal_unit_type is bits 1-6 of byte 0.
        if nal_data.len() >= 3 && candidate.offset == offset {
            let nal_type = candidate.nal_type;
            // PREFIX_SEI_NUT = 39, SUFFIX_SEI_NUT = 40
            let rbsp = scan::h2645_unescape_rbsp(&nal_data[2..]);
            if (nal_type == 39 || nal_type == 40) && check_sei_for_hdr10plus(rbsp.as_ref()) {
                return true;
            }
        }
        offset += nal_len;
    }
    false
}

/// Read a big-endian integer of `size` bytes from the start of `data`.
fn read_be_length(data: &[u8], size: usize) -> usize {
    let mut val = 0usize;
    for &b in &data[..size] {
        val = (val << 8) | b as usize;
    }
    val
}

/// Parse SEI message payloads and check for SMPTE ST 2094-40 (HDR10+).
fn check_sei_for_hdr10plus(sei_rbsp: &[u8]) -> bool {
    let mut offset = 0;
    while offset < sei_rbsp.len() {
        // Parse payload_type (ff_byte run + last_byte).
        let mut payload_type: u32 = 0;
        while offset < sei_rbsp.len() && sei_rbsp[offset] == 0xFF {
            payload_type += 255;
            offset += 1;
        }
        if offset >= sei_rbsp.len() {
            break;
        }
        payload_type += sei_rbsp[offset] as u32;
        offset += 1;

        // Parse payload_size.
        let mut payload_size: u32 = 0;
        while offset < sei_rbsp.len() && sei_rbsp[offset] == 0xFF {
            payload_size += 255;
            offset += 1;
        }
        if offset >= sei_rbsp.len() {
            break;
        }
        payload_size += sei_rbsp[offset] as u32;
        offset += 1;

        let payload_end = offset + payload_size as usize;
        if payload_end > sei_rbsp.len() {
            break;
        }

        // user_data_registered_itu_t_t35, payload_type = 4
        if payload_type == 4 && payload_size >= 5 {
            let p = &sei_rbsp[offset..payload_end];
            if scan_itu_t35_payload_for_hdr10plus(p) {
                return true;
            }
        }

        offset = payload_end;
    }
    false
}

/// Check raw ITU-T T.35 payload bytes for HDR10+ metadata.
pub(crate) fn scan_itu_t35_payload_for_hdr10plus(payload: &[u8]) -> bool {
    scan::find_hdr10plus_itu_t35_candidate(payload, 0) == Some(0)
}

/// Maps an H.264 profile_idc value to a human-readable profile name.
fn map_h264_profile(profile_idc: u8) -> Option<String> {
    let name = match profile_idc {
        66 => "Baseline",
        77 => "Main",
        88 => "Extended",
        100 => "High",
        110 => "High 10",
        122 => "High 4:2:2",
        244 => "High 4:4:4 Predictive",
        44 => "CAVLC 4:4:4 Intra",
        83 => "Scalable Baseline",
        86 => "Scalable High",
        118 => "Multiview High",
        128 => "Stereo High",
        _ => return None,
    };
    Some(name.into())
}

/// Maps an H.265 general_profile_idc value to a human-readable profile name.
fn map_h265_profile(profile_idc: u8) -> Option<String> {
    let name = match profile_idc {
        1 => "Main",
        2 => "Main 10",
        3 => "Main Still Picture",
        4 => "Format Range Extensions",
        5 => "High Throughput",
        9 => "Screen Content Coding",
        11 => "High Throughput Screen Content Coding",
        _ => return None,
    };
    Some(name.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TrackKind;

    #[test]
    fn aac_configurations_preserve_explicit_layout_and_extension_rate() {
        let properties = |bytes: &[u8]| {
            aac_configuration_metadata(bytes).map(|value| (value.sample_rate, value.layout))
        };
        assert_eq!(
            properties(&[0x12, 0x10]),
            Some((44100, Some((2, "stereo"))))
        );
        assert_eq!(properties(&[0x11, 0xb0]), Some((48000, Some((6, "5.1")))));
        assert_eq!(
            properties(&[0x11, 0xb8]),
            Some((48000, Some((8, "7.1(wide)"))))
        );
        assert_eq!(
            properties(&[0x12, 0x00]),
            Some((44100, None)),
            "PCE layout stays unknown until parsed"
        );
        assert!(aac_configuration_metadata(&[0x12]).is_none());
        let core = aac_configuration_metadata(&[0x12, 0x08]).unwrap();
        assert_eq!((core.sbr_signaled, core.ps_signaled), (None, None));
        let mut bits = TestBitWriter::new();
        for (value, count) in [(29, 5), (7, 4), (1, 4), (4, 4), (2, 5)] {
            bits.write_bits(value, count);
        }
        assert_eq!(
            properties(&bits.bytes),
            Some((44100, Some((2, "stereo")))),
            "explicit parametric stereo expands the mono core"
        );
        let output = aac_configuration_metadata(&bits.bytes).unwrap();
        assert_eq!(
            (output.sbr_signaled, output.ps_signaled),
            (Some(true), Some(true))
        );
    }

    struct TestBitWriter {
        bytes: Vec<u8>,
        bit_pos: usize,
    }

    impl TestBitWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                bit_pos: 0,
            }
        }

        fn write_bits(&mut self, value: u64, count: usize) {
            for shift in (0..count).rev() {
                let bit = ((value >> shift) & 1) as u8;
                if self.bit_pos.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                let byte_index = self.bit_pos / 8;
                self.bytes[byte_index] |= bit << (7 - (self.bit_pos % 8));
                self.bit_pos += 1;
            }
        }

        fn finish(self) -> Vec<u8> {
            self.bytes
        }
    }

    #[test]
    fn hevc_hdr10plus_scan_unescapes_sei_rbsp() {
        let mut sei_rbsp = vec![0x00, 0x00, 0x03, 0x04, 0x06];
        sei_rbsp.extend_from_slice(&[0xB5, 0x00, 0x3C, 0x00, 0x01, 0x04]);
        assert!(!check_sei_for_hdr10plus(&sei_rbsp));

        let nal_len = 2 + sei_rbsp.len();
        let mut frame = Vec::new();
        frame.extend_from_slice(&(nal_len as u16).to_be_bytes());
        frame.extend_from_slice(&[0x4E, 0x01]);
        frame.extend_from_slice(&sei_rbsp);

        assert!(scan_hevc_frame_for_hdr10plus(&frame, 2));
    }

    #[test]
    fn normalize_mkv_video_codecs() {
        assert_eq!(
            normalize_codec_name("V_MPEG4/ISO/AVC").as_deref(),
            Some("h264")
        );
        assert_eq!(
            normalize_codec_name("V_MPEGH/ISO/HEVC").as_deref(),
            Some("hevc")
        );
        assert_eq!(normalize_codec_name("V_AV1").as_deref(), Some("av1"));
        assert_eq!(normalize_codec_name("V_VP9").as_deref(), Some("vp9"));
        assert_eq!(
            normalize_codec_name("V_MPEG4/ISO/SP").as_deref(),
            Some("mpeg4")
        );
        assert_eq!(normalize_codec_name("V_MS/VFW/FOURCC"), None);
    }

    #[test]
    fn normalize_mkv_audio_codecs() {
        assert_eq!(normalize_codec_name("A_AAC").as_deref(), Some("aac"));
        assert_eq!(
            normalize_codec_name("A_AAC/MPEG2/LC").as_deref(),
            Some("aac")
        );
        assert_eq!(
            normalize_codec_name("A_AAC/MPEG4/LC/SBR").as_deref(),
            Some("aac")
        );
        assert_eq!(normalize_codec_name("A_AC3").as_deref(), Some("ac3"));
        assert_eq!(normalize_codec_name("A_EAC3").as_deref(), Some("eac3"));
        assert_eq!(normalize_codec_name("A_TRUEHD").as_deref(), Some("truehd"));
        assert_eq!(normalize_codec_name("A_DTS").as_deref(), Some("dts"));
        assert_eq!(normalize_codec_name("A_FLAC").as_deref(), Some("flac"));
        assert_eq!(normalize_codec_name("A_OPUS").as_deref(), Some("opus"));
        assert_eq!(normalize_codec_name("A_VORBIS").as_deref(), Some("vorbis"));
        assert_eq!(
            normalize_codec_name("A_PCM/INT/LIT").as_deref(),
            Some("pcm")
        );
        assert_eq!(
            normalize_codec_name("A_PCM/FLOAT/IEEE").as_deref(),
            Some("pcm")
        );
        assert_eq!(normalize_codec_name("A_MPEG/L3").as_deref(), Some("mp3"));
    }

    #[test]
    fn normalize_mkv_subtitle_codecs() {
        assert_eq!(
            normalize_codec_name("S_TEXT/UTF8").as_deref(),
            Some("subrip")
        );
        assert_eq!(normalize_codec_name("S_TEXT/ASS").as_deref(), Some("ass"));
        assert_eq!(normalize_codec_name("S_TEXT/SSA").as_deref(), Some("ass"));
        assert_eq!(
            normalize_codec_name("S_HDMV/PGS").as_deref(),
            Some("hdmv_pgs_subtitle")
        );
        assert_eq!(
            normalize_codec_name("S_VOBSUB").as_deref(),
            Some("dvd_subtitle")
        );
        assert_eq!(
            normalize_codec_name("S_TEXT/WEBVTT").as_deref(),
            Some("webvtt")
        );
    }

    #[test]
    fn normalize_mp4_fourcc() {
        assert_eq!(normalize_codec_name("avc1").as_deref(), Some("h264"));
        assert_eq!(normalize_codec_name("avc3").as_deref(), Some("h264"));
        assert_eq!(normalize_codec_name("hvc1").as_deref(), Some("hevc"));
        assert_eq!(normalize_codec_name("hev1").as_deref(), Some("hevc"));
        assert_eq!(normalize_codec_name("av01").as_deref(), Some("av1"));
        assert_eq!(normalize_codec_name("vp09").as_deref(), Some("vp9"));
        assert_eq!(normalize_codec_name("mp4a").as_deref(), Some("aac"));
        assert_eq!(normalize_codec_name("ac-3").as_deref(), Some("ac3"));
        assert_eq!(normalize_codec_name("ec-3").as_deref(), Some("eac3"));
        assert_eq!(normalize_codec_name("fLaC").as_deref(), Some("flac"));
        assert_eq!(normalize_codec_name("Opus").as_deref(), Some("opus"));
        assert_eq!(normalize_codec_name("tx3g").as_deref(), Some("mov_text"));
        assert_eq!(normalize_codec_name("wvtt").as_deref(), Some("webvtt"));
        assert_eq!(normalize_codec_name("stpp").as_deref(), Some("ttml"));
    }

    #[test]
    fn normalize_unknown_returns_none() {
        assert_eq!(normalize_codec_name("XYZZY"), None);
        assert_eq!(normalize_codec_name(""), None);
    }

    #[test]
    fn detects_aac_profiles_from_codec_private_and_codec_id() {
        assert_eq!(
            detect_header_audio_profile("mp4a", Some("aac"), Some(&[0x12, 0x10])).as_deref(),
            Some("LC")
        );
        assert_eq!(
            detect_header_audio_profile("A_AAC/MPEG4/LC/SBR", Some("aac"), None).as_deref(),
            Some("HE-AAC")
        );
        assert_eq!(
            detect_header_audio_profile("A_AAC/MPEG4/LC/SBR/PS", Some("aac"), None).as_deref(),
            Some("HE-AACv2")
        );
    }

    #[test]
    fn detects_aac_he_profile_from_sync_extension_in_codec_private() {
        let mut bits = TestBitWriter::new();
        bits.write_bits(2, 5); // AAC LC
        bits.write_bits(4, 4); // 44100
        bits.write_bits(2, 4); // stereo
        bits.write_bits(0x2B7, 11); // sync extension
        bits.write_bits(5, 5); // SBR
        bits.write_bits(1, 1); // sbr present
        bits.write_bits(4, 4); // extension sample rate 44100

        assert_eq!(
            detect_header_audio_profile("mp4a", Some("aac"), Some(&bits.finish())).as_deref(),
            Some("HE-AAC")
        );
    }

    #[test]
    fn detects_plain_dts_from_core_syncword() {
        let payload = [
            0x7F, 0xFE, 0x80, 0x01, 0x7C, 0x7C, 0x05, 0xF2, 0xB7, 0x00, 0x00,
        ];
        assert_eq!(
            detect_audio_profile_from_payload(Some("dts"), &payload).as_deref(),
            Some("DTS")
        );
    }

    #[test]
    fn detects_dts_es_profile_from_xch_syncword() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&DTS_SYNCWORD_CORE_BE.to_be_bytes());
        payload.extend_from_slice(&DTS_SYNCWORD_XCH.to_be_bytes());
        assert_eq!(
            detect_audio_profile_from_payload(Some("dts"), &payload).as_deref(),
            Some("DTS-ES")
        );
    }

    #[test]
    fn does_not_promote_bare_xll_syncwords_without_valid_exss() {
        let mut prefix = vec![
            0x7F, 0xFE, 0x80, 0x01, 0x7C, 0x7C, 0x05, 0xF2, 0xB7, 0x00, 0x00,
        ];
        prefix.extend_from_slice(&DTS_SYNCWORD_XLL.to_be_bytes());
        prefix.resize(4096, 0);

        let mut suffix = vec![0; 64];
        suffix[16..20].copy_from_slice(&DTS_SYNCWORD_XLL_X.to_be_bytes());

        assert_eq!(
            detect_audio_profile_from_probe_bytes(Some("dts"), &prefix, Some(&suffix)).as_deref(),
            Some("DTS")
        );
    }

    #[test]
    fn detects_truehd_atmos_from_major_sync_header() {
        let mut bits = TestBitWriter::new();
        bits.write_bits(0xF8_72_6F, 24);
        bits.write_bits(0xBA, 8);
        bits.write_bits(0, 4); // ratebits
        bits.write_bits(0, 4); // skip
        bits.write_bits(0, 2); // stream0 modifier
        bits.write_bits(0, 2); // stream1 modifier
        bits.write_bits(0, 5); // stream1 channel arrangement
        bits.write_bits(0, 2); // stream2 modifier
        bits.write_bits(0, 13); // stream2 channel arrangement
        bits.write_bits(0, 48); // reserved
        bits.write_bits(0, 1); // is_vbr
        bits.write_bits(0, 15); // peak bitrate
        bits.write_bits(4, 4); // num_substreams
        bits.write_bits(0, 2); // skip
        bits.write_bits(0, 2); // extended_substream_info
        bits.write_bits(0x80, 8); // substream_info, top bit set
        let mut payload = bits.finish();
        payload.resize(28, 0);

        assert_eq!(
            detect_audio_profile_from_payload(Some("truehd"), &payload).as_deref(),
            Some("Dolby TrueHD + Dolby Atmos")
        );
        let mut plain = payload.clone();
        plain[17] = 0;
        assert!(detect_audio_profile_from_payload(Some("truehd"), &plain).is_none());
        let mut later = plain.clone();
        later.resize(256, 0);
        later.extend(&payload);
        assert_eq!(
            detect_audio_profile_from_payload(Some("truehd"), &later).as_deref(),
            Some("Dolby TrueHD + Dolby Atmos")
        );
        let mut too_many = plain.repeat(32);
        too_many.extend(&payload);
        assert!(detect_audio_profile_from_payload(Some("truehd"), &too_many).is_none());
        let mut beyond_budget = vec![0; 64 * 1024];
        beyond_budget.extend(payload);
        assert!(detect_audio_profile_from_payload(Some("truehd"), &beyond_budget).is_none());
    }

    #[test]
    fn detects_eac3_atmos_from_additional_bitstream_info() {
        let mut bits = TestBitWriter::new();
        bits.write_bits(0, 2); // frame_type independent
        bits.write_bits(0, 3); // substream id
        bits.write_bits(0x80, 11); // frame size code
        bits.write_bits(0, 2); // fscod
        bits.write_bits(3, 2); // numblkscod => 6 blocks
        bits.write_bits(2, 3); // acmod stereo
        bits.write_bits(1, 1); // lfe_on
        bits.write_bits(16, 5); // bsid
        bits.write_bits(0, 5); // dialnorm
        bits.write_bits(0, 1); // no compression
        bits.write_bits(0, 1); // no mixing metadata
        bits.write_bits(0, 1); // no informational metadata
        bits.write_bits(1, 1); // additional bitstream info present
        bits.write_bits(0, 6); // addbsil
        bits.write_bits(0, 7); // reserved
        bits.write_bits(1, 1); // flag_ec3_extension_type_a
        let mut payload = vec![0x0B, 0x77];
        payload.extend_from_slice(&bits.finish());

        assert_eq!(
            detect_audio_profile_from_payload(Some("eac3"), &payload).as_deref(),
            Some("Dolby Digital Plus + Dolby Atmos")
        );
        let mut plain = payload.clone();
        *plain.last_mut().unwrap() = 0;
        plain.resize(258, 0);
        assert!(detect_audio_profile_from_payload(Some("eac3"), &plain).is_none());
        let mut later = plain.clone();
        later.extend(&payload);
        assert_eq!(
            detect_audio_profile_from_payload(Some("eac3"), &later).as_deref(),
            Some("Dolby Digital Plus + Dolby Atmos")
        );
        let mut too_many = plain.repeat(32);
        too_many.extend(&payload);
        assert!(detect_audio_profile_from_payload(Some("eac3"), &too_many).is_none());
        let mut alternate = payload.clone();
        alternate[2] |= 1 << 3;
        assert!(detect_audio_profile_from_payload(Some("eac3"), &alternate).is_none());
        let mut beyond_budget = vec![0; 64 * 1024];
        beyond_budget.extend(payload);
        assert!(detect_audio_profile_from_payload(Some("eac3"), &beyond_budget).is_none());
    }

    #[test]
    fn ignores_legacy_ac3_bsid_when_probably_not_true_eac3() {
        let mut bits = TestBitWriter::new();
        bits.write_bits(0, 2); // frame_type independent
        bits.write_bits(0, 3); // substream id
        bits.write_bits(0x80, 11); // frame size code
        bits.write_bits(0, 2); // fscod
        bits.write_bits(2, 2); // numblkscod => 3 blocks
        bits.write_bits(0, 3); // acmod dual mono
        bits.write_bits(0, 1); // lfe_on
        bits.write_bits(6, 5); // bsid: legacy AC-3 range, not E-AC-3
        bits.write_bits(0, 5); // dialnorm ch1
        bits.write_bits(0, 1); // no compr ch1
        bits.write_bits(0, 5); // dialnorm ch2
        bits.write_bits(0, 1); // no compr ch2
        bits.write_bits(0, 1); // no mixing metadata
        bits.write_bits(0, 1); // no informational metadata
        bits.write_bits(1, 1); // additional bitstream info present
        bits.write_bits(0, 6); // addbsil
        bits.write_bits(0, 7); // reserved
        bits.write_bits(1, 1); // flag_ec3_extension_type_a
        let mut payload = vec![0x0B, 0x77];
        payload.extend_from_slice(&bits.finish());

        assert_eq!(
            detect_audio_profile_from_payload(Some("eac3"), &payload),
            None
        );
    }

    #[test]
    fn av1_info_main_profile_8bit() {
        // marker=1, version=1 -> 0x81
        // seq_profile=0 (Main), seq_level_idx_0=0 -> byte1 = 0x00
        // high_bitdepth=0, twelve_bit=0 -> 8-bit; rest zero -> byte2 = 0x00
        // byte3 = 0x00
        let data = [0x81, 0x00, 0x00, 0x00];
        let info = extract_av1_info(&data);
        assert_eq!(info.profile.as_deref(), Some("Main"));
        assert_eq!(info.bit_depth, Some(8));
    }

    #[test]
    fn av1_info_high_profile_10bit() {
        // seq_profile=1 (High) -> bits 7-5 of byte1 = 001 -> byte1 = 0x20
        // high_bitdepth=1, twelve_bit=0 -> 10-bit; byte2 = 0b01_000000 = 0x40
        let data = [0x81, 0x20, 0x40, 0x00];
        let info = extract_av1_info(&data);
        assert_eq!(info.profile.as_deref(), Some("High"));
        assert_eq!(info.bit_depth, Some(10));
    }

    #[test]
    fn av1_info_professional_12bit() {
        // seq_profile=2 (Professional) -> bits 7-5 of byte1 = 010 -> byte1 = 0x40
        // high_bitdepth=1, twelve_bit=1 -> 12-bit; byte2 = 0b011_00000 = 0x60
        let data = [0x81, 0x40, 0x60, 0x00];
        let info = extract_av1_info(&data);
        assert_eq!(info.profile.as_deref(), Some("Professional"));
        assert_eq!(info.bit_depth, Some(12));
    }

    #[test]
    fn av1_info_too_short() {
        let info = extract_av1_info(&[0x81, 0x00]);
        assert_eq!(info.profile, None);
        assert_eq!(info.bit_depth, None);
    }

    #[test]
    fn av1_info_bad_marker() {
        let info = extract_av1_info(&[0x00, 0x00, 0x00, 0x00]);
        assert_eq!(info.profile, None);
        assert_eq!(info.bit_depth, None);
    }

    #[test]
    fn detect_hdr_dolby_vision() {
        let track = RawTrack {
            metadata: Default::default(),
            kind: TrackKind::Video,
            codec_id: "V_MPEGH/ISO/HEVC".into(),
            codec_name: Some("hevc".into()),
            audio_profile: None,
            codec_private: None,
            width: Some(3840),
            height: Some(2160),
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: Some(16),
            dovi_config: Some(vec![1, 0, 0x10, 0x0d, 0x10]),
            has_hdr10plus: false,
            name: None,
            forced: false,
            default_track: false,
        };
        // DV takes priority over color_transfer
        assert_eq!(detect_hdr_format(&track).as_deref(), Some("Dolby Vision"));
    }

    #[test]
    fn detect_hdr_hdr10() {
        let track = RawTrack {
            metadata: Default::default(),
            kind: TrackKind::Video,
            codec_id: "hvc1".into(),
            codec_name: Some("hevc".into()),
            audio_profile: None,
            codec_private: None,
            width: Some(3840),
            height: Some(2160),
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: Some(16),
            dovi_config: None,
            has_hdr10plus: false,
            name: None,
            forced: false,
            default_track: false,
        };
        assert_eq!(
            detect_hdr_format(&track),
            None,
            "PQ alone does not establish HDR10"
        );
        let mut track = track;
        track.metadata.bit_depth = Some(10);
        track.metadata.color.primaries = Some(9);
        assert_eq!(detect_hdr_format(&track).as_deref(), Some("HDR10"));
    }

    #[test]
    fn detect_hdr_hdr10plus() {
        let track = RawTrack {
            metadata: Default::default(),
            kind: TrackKind::Video,
            codec_id: "hvc1".into(),
            codec_name: Some("hevc".into()),
            audio_profile: None,
            codec_private: None,
            width: Some(3840),
            height: Some(2160),
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: Some(16),
            dovi_config: None,
            has_hdr10plus: true,
            name: None,
            forced: false,
            default_track: false,
        };
        // HDR10+ takes priority over HDR10
        assert_eq!(detect_hdr_format(&track).as_deref(), Some("HDR10+"));
    }

    #[test]
    fn detect_hdr_hlg() {
        let track = RawTrack {
            metadata: Default::default(),
            kind: TrackKind::Video,
            codec_id: "hvc1".into(),
            codec_name: Some("hevc".into()),
            audio_profile: None,
            codec_private: None,
            width: Some(3840),
            height: Some(2160),
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: Some(18),
            dovi_config: None,
            has_hdr10plus: false,
            name: None,
            forced: false,
            default_track: false,
        };
        assert_eq!(detect_hdr_format(&track).as_deref(), Some("HLG"));
    }

    #[test]
    fn detect_hdr_sdr() {
        let track = RawTrack {
            metadata: Default::default(),
            kind: TrackKind::Video,
            codec_id: "avc1".into(),
            codec_name: Some("h264".into()),
            audio_profile: None,
            codec_private: None,
            width: Some(1920),
            height: Some(1080),
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: Some(1),
            dovi_config: None,
            has_hdr10plus: false,
            name: None,
            forced: false,
            default_track: false,
        };
        assert_eq!(detect_hdr_format(&track), None);
    }

    #[test]
    fn h264_profile_mapping() {
        assert_eq!(map_h264_profile(66).as_deref(), Some("Baseline"));
        assert_eq!(map_h264_profile(77).as_deref(), Some("Main"));
        assert_eq!(map_h264_profile(100).as_deref(), Some("High"));
        assert_eq!(map_h264_profile(110).as_deref(), Some("High 10"));
        assert_eq!(map_h264_profile(0), None);
    }

    #[test]
    fn hevc_nal_length_size_extraction() {
        // Byte 21 = 0xFF -> lengthSizeMinusOne = 3, length size = 4
        let mut hvcc = vec![0u8; 23];
        hvcc[21] = 0xFF;
        assert_eq!(hevc_nal_length_size(&hvcc), 4);

        // Byte 21 = 0xFC -> lengthSizeMinusOne = 0, length size = 1
        hvcc[21] = 0xFC;
        assert_eq!(hevc_nal_length_size(&hvcc), 1);

        // Too short -> default 4
        assert_eq!(hevc_nal_length_size(&[0u8; 10]), 4);
    }

    #[test]
    fn scan_hevc_frame_hdr10plus_found() {
        // Build a synthetic HEVC frame with a PREFIX_SEI NAL containing HDR10+ metadata.
        // NAL length prefix = 4 bytes.
        // NAL header: 0x4E (type=39 PREFIX_SEI, layer_id=0), 0x01 (temporal_id_plus1=1)
        // SEI: payload_type=4, payload_size=7, country=0xB5, provider=0x003C, oriented=0x0001, filler
        let sei_payload = [
            0x04, // payload_type = 4
            0x07, // payload_size = 7
            0xB5, // country_code (USA)
            0x00, 0x3C, // provider_code (Samsung)
            0x00, 0x01, // provider_oriented_code (HDR10+)
            0x04, 0x00, // application_identifier + filler
        ];
        let nal_header = [0x4E, 0x01]; // PREFIX_SEI_NUT
        let nal_len = (nal_header.len() + sei_payload.len()) as u32;
        let mut frame = Vec::new();
        frame.extend_from_slice(&nal_len.to_be_bytes());
        frame.extend_from_slice(&nal_header);
        frame.extend_from_slice(&sei_payload);
        assert!(scan_hevc_frame_for_hdr10plus(&frame, 4));
    }

    #[test]
    fn scan_hevc_frame_hdr10plus_not_found() {
        // Frame with a non-SEI NAL (type=1, VCL slice)
        let nal_header = [0x02, 0x01]; // type=1 (TRAIL_R)
        let nal_data = [0x00; 8];
        let nal_len = (nal_header.len() + nal_data.len()) as u32;
        let mut frame = Vec::new();
        frame.extend_from_slice(&nal_len.to_be_bytes());
        frame.extend_from_slice(&nal_header);
        frame.extend_from_slice(&nal_data);
        assert!(!scan_hevc_frame_for_hdr10plus(&frame, 4));
    }

    #[test]
    fn h265_profile_mapping() {
        assert_eq!(map_h265_profile(1).as_deref(), Some("Main"));
        assert_eq!(map_h265_profile(2).as_deref(), Some("Main 10"));
        assert_eq!(map_h265_profile(3).as_deref(), Some("Main Still Picture"));
        assert_eq!(map_h265_profile(0), None);
    }

    #[test]
    fn normalizes_pcm_codec_names_from_depth_and_endianness() {
        assert_eq!(
            normalize_pcm_codec_name("A_PCM/INT/LIT", Some(24)).as_deref(),
            Some("pcm_s24le")
        );
        assert_eq!(
            normalize_pcm_codec_name("A_PCM/INT/BIG", Some(16)).as_deref(),
            Some("pcm_s16be")
        );
        assert_eq!(
            normalize_pcm_codec_name("A_PCM/FLOAT/IEEE", Some(32)).as_deref(),
            Some("pcm_f32le")
        );
    }

    #[test]
    fn normalizes_vfw_fourcc_video_codecs() {
        assert_eq!(
            normalize_codec_name("V_MPEG2").as_deref(),
            Some("mpeg2video")
        );
        assert_eq!(
            normalize_video_fourcc_codec_name("MPG2").as_deref(),
            Some("mpeg2video")
        );
        assert_eq!(
            normalize_vfw_codec_name(Some(&[
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b'M', b'P', b'G', b'2'
            ]))
            .as_deref(),
            Some("mpeg2video")
        );
    }
}
