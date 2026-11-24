//! AAC AudioSpecificConfig, ADTS and LATM headers: object type, sample rate
//! and channel layout.

use super::bits::{AudioBitReader, BitReader};
use crate::scan;

pub(crate) fn detect_aac_profile_from_codec_id(codec_id: &str) -> Option<String> {
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

pub(crate) fn detect_aac_profile_from_codec_private(
    codec_private: Option<&[u8]>,
) -> Option<String> {
    codec_private.and_then(parse_aac_audio_object_profile)
}

pub(crate) fn detect_aac_profile_from_payload(payload: &[u8]) -> Option<String> {
    detect_adts_aac_profile(payload).or_else(|| detect_latm_aac_profile(payload))
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

const AAC_CHANNEL_CONFIGS: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 8, 0, 0, 0, 7, 8, 0, 8, 0];
const AAC_SAMPLE_RATES: [u32; 16] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350, 0, 0, 0,
];

pub(crate) struct AdtsHeader {
    pub(crate) channels: u8,
    pub(crate) channel_layout: &'static str,
    pub(crate) sample_rate: u32,
    pub(crate) bit_rate_bps: Option<u32>,
}

pub(crate) struct LatmHeader {
    pub(crate) channels: u8,
}

pub(crate) fn find_adts_header(data: &[u8]) -> Option<AdtsHeader> {
    find_adts_header_with_minimum(data, 128)
}

pub(crate) fn find_adts_header_with_minimum(
    data: &[u8],
    minimum_frames: usize,
) -> Option<AdtsHeader> {
    const ADTS_BITRATE_SAMPLE_FRAMES: usize = 128;

    if data.len() < 7 {
        return None;
    }
    let mut start = 0;
    let mut detected_channels = None;
    let mut detected_sample_rate = None;
    let mut total_frame_bytes = 0_u64;
    let mut total_samples = 0_u64;
    let mut frames = 0usize;

    while let Some(candidate) = scan::find_audio_sync_candidate(data, start) {
        if candidate.kind != scan::AudioSyncKind::Adts {
            start = candidate.offset + 1;
            continue;
        }
        let i = candidate.offset;
        if i + 7 > data.len() {
            break;
        }
        start = i + 1;
        let hdr = &data[i..];
        if (hdr[1] & 0xF0) != 0xF0 {
            continue;
        }

        let sampling_frequency_index = (hdr[2] >> 2) & 0x0F;
        let frame_sample_rate: u32 = match sampling_frequency_index {
            0 => 96_000,
            1 => 88_200,
            2 => 64_000,
            3 => 48_000,
            4 => 44_100,
            5 => 32_000,
            6 => 24_000,
            7 => 22_050,
            8 => 16_000,
            9 => 12_000,
            10 => 11_025,
            11 => 8_000,
            12 => 7_350,
            _ => continue,
        };

        let channel_config = ((hdr[2] & 0x01) << 2) | ((hdr[3] >> 6) & 0x03);
        let Some(frame_channels) = crate::codec::aac::aac_channel_configuration(channel_config)
        else {
            continue;
        };

        let frame_length = (((hdr[3] & 0x03) as usize) << 11)
            | ((hdr[4] as usize) << 3)
            | (((hdr[5] >> 5) & 0x07) as usize);
        if frame_length < 7 {
            continue;
        }
        if i + frame_length > data.len() {
            break;
        }
        if i + frame_length + 1 < data.len()
            && (data[i + frame_length] != 0xFF || data[i + frame_length + 1] & 0xF0 != 0xF0)
        {
            continue;
        }

        let number_of_raw_data_blocks = hdr[6] & 0x03;
        let samples_per_frame = 1024_u32 * (u32::from(number_of_raw_data_blocks) + 1);
        detected_channels = Some(frame_channels);
        detected_sample_rate = Some(frame_sample_rate);
        total_frame_bytes += frame_length as u64;
        total_samples += u64::from(samples_per_frame);
        frames += 1;
        start = i + frame_length;
        if frames >= ADTS_BITRATE_SAMPLE_FRAMES {
            break;
        }
    }

    let (channels, channel_layout) = detected_channels?;
    let bit_rate_bps = detected_sample_rate.and_then(|sample_rate| {
        (frames >= minimum_frames && total_samples > 0)
            .then(|| (total_frame_bytes * 8 * u64::from(sample_rate)) / total_samples)
            .and_then(|bitrate| u32::try_from(bitrate).ok())
    });

    Some(AdtsHeader {
        channels,
        channel_layout,
        sample_rate: detected_sample_rate?,
        bit_rate_bps,
    })
}

pub(crate) fn find_latm_header(data: &[u8]) -> Option<LatmHeader> {
    let mut cursor = 0;
    while let Some(candidate) = scan::find_audio_sync_candidate(data, cursor) {
        if candidate.kind != scan::AudioSyncKind::Latm {
            cursor = candidate.offset + 1;
            continue;
        }
        let start = candidate.offset;
        if start + 3 > data.len() {
            return None;
        }
        cursor = start + 1;

        let mut bits = BitReader::new(&data[start..]);
        if bits.read_bits(11)? != 0x2B7 {
            continue;
        }
        let _mux_length = bits.read_bits(13)?;
        if bits.read_bit()? != 0 {
            continue;
        }
        if bits.read_bit()? != 0 {
            continue;
        }
        let _all_streams_same_time_framing = bits.read_bit()?;
        if bits.read_bits(6)? != 0 || bits.read_bits(4)? != 0 || bits.read_bits(3)? != 0 {
            continue;
        }
        let audio_object_type = read_aac_audio_object_type(&mut bits)?;
        let _sample_rate = read_aac_sample_rate(&mut bits)?;
        let mut channel_config = bits.read_bits(4)? as usize;

        if matches!(audio_object_type, 5 | 29) {
            let _ext_sample_rate = read_aac_sample_rate(&mut bits)?;
            let ext_audio_object_type = read_aac_audio_object_type(&mut bits)?;
            if ext_audio_object_type == 22 {
                channel_config = bits.read_bits(4)? as usize;
            }
        }

        let channels = *AAC_CHANNEL_CONFIGS.get(channel_config)?;
        if channels == 0 {
            continue;
        }
        return Some(LatmHeader { channels });
    }

    None
}

fn read_aac_audio_object_type(bits: &mut BitReader<'_>) -> Option<u8> {
    let object_type = bits.read_bits(5)? as u8;
    if object_type == 31 {
        Some(32 + bits.read_bits(6)? as u8)
    } else {
        Some(object_type)
    }
}

fn read_aac_sample_rate(bits: &mut BitReader<'_>) -> Option<u32> {
    let sample_rate_index = bits.read_bits(4)? as usize;
    if sample_rate_index == 0xF {
        bits.read_bits(24)
    } else {
        AAC_SAMPLE_RATES.get(sample_rate_index).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::audio_profile::detect_header_audio_profile;
    use crate::codec::bits::TestBitWriter;

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

    #[test]
    fn detects_aac_profiles_from_codec_private_and_codec_id() {
        assert_eq!(
            detect_header_audio_profile(
                "mp4a",
                Some(&scryer_media_model::Codec::Aac),
                Some(&[0x12, 0x10])
            )
            .as_deref(),
            Some("LC")
        );
        assert_eq!(
            detect_header_audio_profile(
                "A_AAC/MPEG4/LC/SBR",
                Some(&scryer_media_model::Codec::Aac),
                None
            )
            .as_deref(),
            Some("HE-AAC")
        );
        assert_eq!(
            detect_header_audio_profile(
                "A_AAC/MPEG4/LC/SBR/PS",
                Some(&scryer_media_model::Codec::Aac),
                None
            )
            .as_deref(),
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
            detect_header_audio_profile(
                "mp4a",
                Some(&scryer_media_model::Codec::Aac),
                Some(&bits.finish())
            )
            .as_deref(),
            Some("HE-AAC")
        );
    }

    #[test]
    fn parses_latm_header_channels() {
        let data = [0x56, 0xE0, 0x06, 0x20, 0x00, 0x12, 0x10];
        let header = find_latm_header(&data).unwrap();
        assert_eq!(header.channels, 2);
    }
}
