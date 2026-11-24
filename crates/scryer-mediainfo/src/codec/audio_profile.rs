//! Audio profile labels (AAC object type, DTS extensions, E-AC-3 and TrueHD
//! Atmos) from codec identifiers, decoder configuration and payload samples.

use super::aac::{
    detect_aac_profile_from_codec_id, detect_aac_profile_from_codec_private,
    detect_aac_profile_from_payload,
};
use super::bits::AudioBitReader;
use super::dts::{detect_dts_profile_from_codec_id, detect_dts_profile_from_probe_bytes};
use super::normalize_codec_name;

const TRUEHD_MAJOR_SYNCWORD: [u8; 4] = [0xF8, 0x72, 0x6F, 0xBA];

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::bits::TestBitWriter;

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
}
