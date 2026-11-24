//! HDR10+ (SMPTE ST 2094-40) detection in HEVC SEI and ITU-T T.35 payloads.

use crate::scan;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
