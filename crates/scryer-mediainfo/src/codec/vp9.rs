//! VP9 uncompressed frame header: profile, bit depth and color space.

use crate::raw::RawTrack;
use scryer_media_model::Provenance;

pub(crate) fn vp9(track: &mut RawTrack, data: &[u8]) -> Option<()> {
    let mut bits = crate::codec::bits::BitReader::new(data);
    if bits.read_bits(2)? != 2 {
        return None;
    }
    let low = bits.read_bits(1)?;
    let profile = low | (bits.read_bits(1)? << 1);
    if profile == 3 && bits.read_bits(1)? != 0 {
        return None;
    }
    if bits.read_bits(1)? != 0 || bits.read_bits(1)? != 0 {
        return None;
    }
    bits.read_bits(1)?; // show_frame
    bits.read_bits(1)?; // error_resilient
    if bits.read_bits(24)? != 0x49_83_42 {
        return None;
    }
    let depth = if profile >= 2 {
        if bits.read_bits(1)? != 0 { 12 } else { 10 }
    } else {
        8
    };
    let color_space = bits.read_bits(3)?;
    if color_space == 6 || (color_space == 7 && profile != 1 && profile != 3) {
        return None;
    }
    let full_range = if color_space != 7 {
        bits.read_bits(1)? != 0
    } else {
        true
    };
    let (x, y) = if profile == 1 || profile == 3 {
        if color_space == 7 {
            if bits.read_bits(1)? != 0 {
                return None;
            }
            (false, false)
        } else {
            let x = bits.read_bits(1)? != 0;
            let y = bits.read_bits(1)? != 0;
            if bits.read_bits(1)? != 0 {
                return None;
            }
            (x, y)
        }
    } else {
        (true, true)
    };
    track.metadata.profile = Some(format!("Profile {profile}"));
    track.metadata.bit_depth = Some(depth);
    track.metadata.field_order = Some("progressive".into());
    let format = if color_space == 7 {
        "gbrp"
    } else {
        match (x, y) {
            (true, true) => "yuv420p",
            (true, false) => "yuv422p",
            (false, false) => "yuv444p",
            (false, true) => "yuv440p",
        }
    };
    track.metadata.pixel_format = Some(if depth == 8 {
        format.into()
    } else {
        format!("{format}{depth}le")
    });
    track.metadata.color.full_range = Some(full_range);
    track.metadata.color.provenance = Provenance::Bitstream;
    // VP9 color_space is not an independently signaled transfer function.
    // In particular, BT.2020 must not overwrite a container's PQ/HLG transfer.
    let (primaries, matrix) = match color_space {
        1 => (None, Some(6)),
        2 => (Some(1), Some(1)),
        3 => (Some(6), Some(6)),
        4 => (Some(7), Some(7)),
        5 => (Some(9), None),
        7 => (Some(1), Some(0)),
        _ => (None, None),
    };
    if let Some(value) = primaries {
        track.metadata.color.primaries = Some(value);
    }
    if let Some(value) = matrix {
        track.metadata.color.matrix = Some(value);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::picture_headers::enrich;
    use scryer_media_model::Codec;

    #[test]
    fn vp9_color_space_preserves_explicit_container_transfer() {
        let mut track = RawTrack {
            codec_name: Some(Codec::Vp9),
            ..Default::default()
        };
        track.metadata.color.transfer = Some(16);
        track.metadata.color.matrix = Some(10);
        enrich(&mut track, &[0x92, 0x49, 0x83, 0x42, 0x50, 0, 0]);
        assert_eq!(track.metadata.color.primaries, Some(9));
        assert_eq!(track.metadata.color.transfer, Some(16));
        assert_eq!(track.metadata.color.matrix, Some(10));
        let mut unspecified = RawTrack {
            codec_name: Some(Codec::Vp9),
            ..Default::default()
        };
        enrich(&mut unspecified, &[0x92, 0x49, 0x83, 0x42, 0x50, 0, 0]);
        assert_eq!(unspecified.metadata.color.transfer, None);
        assert_eq!(unspecified.metadata.color.matrix, None);
    }

    #[test]
    fn vp9_10bit_keyframe_and_truncation() {
        let mut track = RawTrack {
            codec_name: Some(Codec::Vp9),
            ..Default::default()
        };
        // marker=2, profile=2, show-existing=0, keyframe=0, show=1, resilient=0.
        enrich(&mut track, &[0x92, 0x49, 0x83, 0x42, 0x50, 0, 0]);
        assert_eq!(track.metadata.bit_depth, Some(10));
        assert_eq!(track.metadata.profile.as_deref(), Some("Profile 2"));
        assert_eq!(track.metadata.pixel_format.as_deref(), Some("yuv420p10le"));
        let mut short = RawTrack {
            codec_name: Some(Codec::Vp9),
            ..Default::default()
        };
        enrich(&mut short, &[0x92, 0x49]);
        assert!(short.metadata.bit_depth.is_none());
    }
}
