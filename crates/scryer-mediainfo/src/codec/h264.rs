//! H.264 decoder configuration (avcC): profile, bit depth and dimensions.

use super::CodecInfo;
use crate::scan;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h264_profile_mapping() {
        assert_eq!(map_h264_profile(66).as_deref(), Some("Baseline"));
        assert_eq!(map_h264_profile(77).as_deref(), Some("Main"));
        assert_eq!(map_h264_profile(100).as_deref(), Some("High"));
        assert_eq!(map_h264_profile(110).as_deref(), Some("High 10"));
        assert_eq!(map_h264_profile(0), None);
    }
}
