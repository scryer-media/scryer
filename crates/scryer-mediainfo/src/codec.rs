use crate::types::RawTrack;

/// Codec profile and bit-depth information extracted from bitstream headers.
#[derive(Debug, Clone, Default)]
pub(crate) struct CodecInfo {
    pub profile: Option<String>,
    pub bit_depth: Option<i32>,
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
        "V_VP9" => "vp9",
        "V_MPEG4/ISO/SP" => "mpeg4",
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

        // --- MKV subtitle ---
        "S_TEXT/UTF8" => "subrip",
        "S_TEXT/ASS" | "S_TEXT/SSA" => "ass",
        "S_HDMV/PGS" => "hdmv_pgs_subtitle",
        "S_VOBSUB" => "dvd_subtitle",
        "S_TEXT/WEBVTT" => "webvtt",

        // --- MP4 FourCC ---
        "avc1" | "avc3" => "h264",
        "hvc1" | "hev1" => "hevc",
        "av01" => "av1",
        "vp09" => "vp9",
        "mp4a" => "aac",
        "ac-3" => "ac3",
        "ec-3" => "eac3",
        "fLaC" => "flac",
        "Opus" => "opus",
        "tx3g" => "mov_text",
        "wvtt" => "webvtt",
        "stpp" => "ttml",

        // MKV AAC variants and PCM wildcard
        other => {
            if other == "A_AAC" || other.starts_with("A_AAC/") {
                return Some("aac".into());
            }
            if other.starts_with("A_PCM/") {
                return Some("pcm".into());
            }
            return None;
        }
    };
    Some(name.into())
}

/// Extracts codec profile and bit depth from an H.264 AVCDecoderConfigurationRecord.
///
/// The input bytes are the raw codec-private data stored in the container
/// (e.g. MKV CodecPrivate or MP4 avcC box contents).
pub(crate) fn extract_h264_info(codec_private: &[u8]) -> CodecInfo {
    use bytes::Bytes;
    use scuffle_h264::AVCDecoderConfigurationRecord;
    use std::io;

    let data = Bytes::from(codec_private.to_vec());
    let config = match AVCDecoderConfigurationRecord::parse(&mut io::Cursor::new(data)) {
        Ok(c) => c,
        Err(_) => return CodecInfo::default(),
    };

    let profile = map_h264_profile(config.profile_indication);

    // Bit depth: prefer the extended_config field (from the AVCC record itself),
    // fall back to parsing the first SPS NAL unit.
    let bit_depth = if let Some(ref ext) = config.extended_config {
        Some(ext.bit_depth_luma_minus8 as i32 + 8)
    } else {
        // Try parsing the first SPS to extract bit depth from the SPS extension
        config.sps.first().and_then(|sps_bytes| {
            scuffle_h264::Sps::parse_with_emulation_prevention(io::Cursor::new(sps_bytes))
                .ok()
                .and_then(|sps| sps.ext.map(|ext| ext.bit_depth_luma_minus8 as i32 + 8))
        })
    };

    CodecInfo {
        profile,
        bit_depth: bit_depth.or(Some(8)),
        color_transfer: None,
    }
}

/// Extracts codec profile, bit depth, and color transfer characteristics from
/// an HEVC (H.265) decoder configuration record.
pub(crate) fn extract_h265_info(codec_private: &[u8]) -> CodecInfo {
    use scuffle_h265::{HEVCDecoderConfigurationRecord, NALUnitType, SpsNALUnit};
    use std::io;

    let config = match HEVCDecoderConfigurationRecord::demux(&mut io::Cursor::new(codec_private)) {
        Ok(c) => c,
        Err(_) => return CodecInfo::default(),
    };

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
            let vui = sps.rbsp.vui_parameters;
            let vui = vui?;
            let vst = vui.video_signal_type;
            let tc = vst.transfer_characteristics;
            if tc > 0 && tc != 2 {
                Some(tc as u32)
            } else {
                None
            }
        });

    CodecInfo {
        profile,
        bit_depth,
        color_transfer,
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
    }
}

/// Parsed fields from a DOVIDecoderConfigurationRecord.
#[derive(Debug, Clone)]
pub(crate) struct DoviConfigInfo {
    pub profile: u8,
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
        bl_signal_compatibility_id: bl_compat,
    })
}

/// Determines the HDR format of a video track using a priority cascade:
///
/// 1. Dolby Vision configuration present -> `"Dolby Vision"`
/// 2. HDR10+ dynamic metadata found -> `"HDR10+"`
/// 3. color_transfer == 16 (SMPTE ST 2084 / PQ) -> `"HDR10"`
/// 4. color_transfer == 18 (ARIB STD-B67 / HLG) -> `"HLG"`
/// 5. Otherwise -> `None`
pub(crate) fn detect_hdr_format(track: &RawTrack) -> Option<String> {
    if track.dovi_config.is_some() {
        return Some("Dolby Vision".into());
    }
    if track.has_hdr10plus {
        return Some("HDR10+".into());
    }
    match track.color_transfer {
        Some(16) => Some("HDR10".into()),
        Some(18) => Some("HLG".into()),
        _ => None,
    }
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
    let mut offset = 0;
    while offset + nal_length_size <= frame.len() {
        let nal_len = read_be_length(&frame[offset..], nal_length_size);
        offset += nal_length_size;
        if nal_len == 0 || offset + nal_len > frame.len() {
            break;
        }
        let nal_data = &frame[offset..offset + nal_len];
        // HEVC NAL header is 2 bytes. nal_unit_type is bits 1-6 of byte 0.
        if nal_data.len() >= 3 {
            let nal_type = (nal_data[0] >> 1) & 0x3F;
            // PREFIX_SEI_NUT = 39, SUFFIX_SEI_NUT = 40
            if (nal_type == 39 || nal_type == 40)
                && check_sei_for_hdr10plus(&nal_data[2..])
            {
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
            // country_code 0xB5 (USA), provider_code 0x003C (Samsung),
            // provider_oriented_code 0x0001 (HDR10+)
            if p[0] == 0xB5 && p[1] == 0x00 && p[2] == 0x3C && p[3] == 0x00 && p[4] == 0x01 {
                return true;
            }
        }

        offset = payload_end;
    }
    false
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
    fn normalize_mkv_video_codecs() {
        assert_eq!(normalize_codec_name("V_MPEG4/ISO/AVC").as_deref(), Some("h264"));
        assert_eq!(normalize_codec_name("V_MPEGH/ISO/HEVC").as_deref(), Some("hevc"));
        assert_eq!(normalize_codec_name("V_AV1").as_deref(), Some("av1"));
        assert_eq!(normalize_codec_name("V_VP9").as_deref(), Some("vp9"));
        assert_eq!(normalize_codec_name("V_MPEG4/ISO/SP").as_deref(), Some("mpeg4"));
        assert_eq!(normalize_codec_name("V_MS/VFW/FOURCC"), None);
    }

    #[test]
    fn normalize_mkv_audio_codecs() {
        assert_eq!(normalize_codec_name("A_AAC").as_deref(), Some("aac"));
        assert_eq!(normalize_codec_name("A_AAC/MPEG2/LC").as_deref(), Some("aac"));
        assert_eq!(normalize_codec_name("A_AAC/MPEG4/LC/SBR").as_deref(), Some("aac"));
        assert_eq!(normalize_codec_name("A_AC3").as_deref(), Some("ac3"));
        assert_eq!(normalize_codec_name("A_EAC3").as_deref(), Some("eac3"));
        assert_eq!(normalize_codec_name("A_TRUEHD").as_deref(), Some("truehd"));
        assert_eq!(normalize_codec_name("A_DTS").as_deref(), Some("dts"));
        assert_eq!(normalize_codec_name("A_FLAC").as_deref(), Some("flac"));
        assert_eq!(normalize_codec_name("A_OPUS").as_deref(), Some("opus"));
        assert_eq!(normalize_codec_name("A_VORBIS").as_deref(), Some("vorbis"));
        assert_eq!(normalize_codec_name("A_PCM/INT/LIT").as_deref(), Some("pcm"));
        assert_eq!(normalize_codec_name("A_PCM/FLOAT/IEEE").as_deref(), Some("pcm"));
        assert_eq!(normalize_codec_name("A_MPEG/L3").as_deref(), Some("mp3"));
    }

    #[test]
    fn normalize_mkv_subtitle_codecs() {
        assert_eq!(normalize_codec_name("S_TEXT/UTF8").as_deref(), Some("subrip"));
        assert_eq!(normalize_codec_name("S_TEXT/ASS").as_deref(), Some("ass"));
        assert_eq!(normalize_codec_name("S_TEXT/SSA").as_deref(), Some("ass"));
        assert_eq!(normalize_codec_name("S_HDMV/PGS").as_deref(), Some("hdmv_pgs_subtitle"));
        assert_eq!(normalize_codec_name("S_VOBSUB").as_deref(), Some("dvd_subtitle"));
        assert_eq!(normalize_codec_name("S_TEXT/WEBVTT").as_deref(), Some("webvtt"));
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
            kind: TrackKind::Video,
            codec_id: "V_MPEGH/ISO/HEVC".into(),
            codec_name: Some("hevc".into()),
            codec_private: None,
            width: Some(3840),
            height: Some(2160),
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: Some(16),
            dovi_config: Some(vec![0x01]),
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
            kind: TrackKind::Video,
            codec_id: "hvc1".into(),
            codec_name: Some("hevc".into()),
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
        assert_eq!(detect_hdr_format(&track).as_deref(), Some("HDR10"));
    }

    #[test]
    fn detect_hdr_hdr10plus() {
        let track = RawTrack {
            kind: TrackKind::Video,
            codec_id: "hvc1".into(),
            codec_name: Some("hevc".into()),
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
            kind: TrackKind::Video,
            codec_id: "hvc1".into(),
            codec_name: Some("hevc".into()),
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
            kind: TrackKind::Video,
            codec_id: "avc1".into(),
            codec_name: Some("h264".into()),
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
            0x04,                         // payload_type = 4
            0x07,                         // payload_size = 7
            0xB5,                         // country_code (USA)
            0x00, 0x3C,                   // provider_code (Samsung)
            0x00, 0x01,                   // provider_oriented_code (HDR10+)
            0x04, 0x00,                   // application_identifier + filler
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
}
