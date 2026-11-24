//! HEVC decoder configuration (hvcC): profile, bit depth, transfer and NAL
//! length size.

use super::CodecInfo;

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
    fn h265_profile_mapping() {
        assert_eq!(map_h265_profile(1).as_deref(), Some("Main"));
        assert_eq!(map_h265_profile(2).as_deref(), Some("Main 10"));
        assert_eq!(map_h265_profile(3).as_deref(), Some("Main Still Picture"));
        assert_eq!(map_h265_profile(0), None);
    }
}
