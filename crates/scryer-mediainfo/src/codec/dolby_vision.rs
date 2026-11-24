//! Dolby Vision decoder configuration record (dvcC, dvvC, dvwC).

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

/// Parse a DOVIDecoderConfigurationRecord (≥4 bytes).
///
/// Layout:
/// - Byte 0: dv_version_major
/// - Byte 1: dv_version_minor
/// - Byte 2-3: dv_profile(7) | dv_level(6) | rpu_present_flag(1) |
///   el_present_flag(1) | bl_present_flag(1)
/// - Byte 4 (optional): dv_bl_signal_compatibility_id(4) | reserved(4);
///   a record without it has compatibility id 0 (none).
pub(crate) fn parse_dovi_config(data: &[u8]) -> Option<DoviConfigInfo> {
    if data.len() < 4 {
        return None;
    }
    let dv_profile = (data[2] >> 1) & 0x7F;
    let bl_compat = data.get(4).map_or(0, |byte| (byte >> 4) & 0x0F);
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
mod tests {
    use super::*;

    fn dovi_fields(data: &[u8]) -> Option<(u8, u8, bool, bool, bool, u8)> {
        parse_dovi_config(data).map(|dovi| {
            (
                dovi.profile,
                dovi.level,
                dovi.rpu_present,
                dovi.enhancement_layer_present,
                dovi.base_layer_present,
                dovi.bl_signal_compatibility_id,
            )
        })
    }

    #[test]
    fn dovi_record_decodes_profile_7_dual_layer() {
        // version 1.0, profile 7, level 6, rpu 1, el 1, bl 1, compatibility id 6.
        assert_eq!(
            dovi_fields(&[1, 0, 0x0E, 0x37, 0x60, 0, 0, 0]),
            Some((7, 6, true, true, true, 6))
        );
    }

    #[test]
    fn dovi_record_without_compatibility_byte_defaults_it_to_none() {
        assert_eq!(
            dovi_fields(&[1, 0, 0x10, 0x35]),
            Some((8, 6, true, false, true, 0))
        );
        assert_eq!(dovi_fields(&[1, 0, 0x10]), None);
    }
}
