use scryer_media_types::StreamMetadata;

fn u16le(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn u32le(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

pub(crate) fn pcm_codec(tag: u16, bits: u16) -> Option<&'static str> {
    match (tag, bits) {
        (1, 8) => Some("pcm_u8"),
        (1, 16) => Some("pcm_s16le"),
        (1, 24) => Some("pcm_s24le"),
        (1, 32) => Some("pcm_s32le"),
        (1, 64) => Some("pcm_s64le"),
        (3, 32) => Some("pcm_f32le"),
        (3, 64) => Some("pcm_f64le"),
        _ => None,
    }
}

pub(crate) fn wave_format(data: &[u8]) -> StreamMetadata {
    let mut metadata = StreamMetadata::default();
    metadata.sample_rate = u32le(data, 4).filter(|rate| *rate > 0);
    let channels = u16le(data, 2).unwrap_or(0);
    let Some(mut tag) = u16le(data, 0) else {
        return metadata;
    };
    let Some(bits) = u16le(data, 14) else {
        return metadata;
    };
    let mut valid_bits = bits;
    if tag == 0xfffe {
        let Some(extra) = u16le(data, 16).filter(|size| *size >= 22) else {
            return metadata;
        };
        if data.len() < 18 + usize::from(extra) {
            return metadata;
        }
        let mask = u32le(data, 20).unwrap_or(0);
        metadata.channel_layout = speaker_mask_layout(mask, channels);
        // Only the standard wave-format GUID namespace carries integer tags.
        if data.get(26..40) != Some(&[0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]) {
            return metadata;
        }
        tag = u16le(data, 24).unwrap_or(0);
        valid_bits = u16le(data, 18).filter(|value| *value != 0).unwrap_or(bits);
    }
    if let Some(codec) = pcm_codec(tag, bits)
        && valid_bits <= bits
    {
        metadata.sample_format =
            crate::codec::pcm_sample_representation(codec).map(|(format, _)| format.into());
        metadata.sample_bit_depth = Some(u32::from(valid_bits));
    }
    metadata
}

struct AudioHeader {
    channels: u8,
    sample_rate: u32,
    layout: Option<String>,
    bit_depth: Option<u32>,
    bitrate: Option<i64>,
    warning: Option<(&'static str, bool)>,
}

pub(crate) fn enrich_private_header(
    track: &mut crate::types::RawTrack,
) -> Option<(&'static str, bool)> {
    if !matches!(
        track.codec_name.as_deref(),
        Some("opus" | "vorbis" | "flac")
    ) {
        return None;
    }
    let Some(data) = track.codec_private.as_deref() else {
        return Some(("audio_configuration_unavailable", false));
    };
    let header = match track.codec_name.as_deref() {
        Some("opus") => opus(data),
        Some("vorbis") => vorbis(data),
        Some("flac") => flac(data),
        _ => None,
    };
    let Some(header) = header else {
        return Some(("audio_configuration_malformed", false));
    };
    track.channels = Some(i32::from(header.channels));
    track.metadata.sample_rate = Some(header.sample_rate);
    track.metadata.channel_layout = header.layout;
    if let Some(depth) = header.bit_depth {
        track.metadata.sample_bit_depth = Some(depth);
    }
    if track.bit_rate_bps.is_none() && header.bitrate.is_some() {
        track.bit_rate_bps = header.bitrate;
        track.metadata.bitrate_provenance = scryer_media_types::Provenance::Bitstream;
    }
    header.warning
}

fn conventional_layout(channels: u8) -> Option<String> {
    Some(
        match channels {
            1 => "mono",
            2 => "stereo",
            3 => "3.0",
            4 => "quad",
            5 => "5.0",
            6 => "5.1",
            7 => "6.1",
            8 => "7.1",
            _ => return None,
        }
        .into(),
    )
}

fn opus(data: &[u8]) -> Option<AudioHeader> {
    let (channels, family, mapping_at) = if data.starts_with(b"OpusHead") {
        if data.len() < 19 || data[8] & 0xf0 != 0 {
            return None;
        }
        (data[9], data[18], 19)
    } else {
        // MP4 dOps stores its scalar fields in big endian; channel fields
        // remain octets and the encoded timeline is still 48 kHz.
        if data.len() < 11 || data[0] != 0 {
            return None;
        }
        (data[1], data[10], 11)
    };
    if channels == 0 {
        return None;
    }
    if family == 0 {
        if channels > 2 {
            return None;
        }
    } else {
        let streams = *data.get(mapping_at)?;
        let coupled = *data.get(mapping_at + 1)?;
        let decoded = u16::from(streams) + u16::from(coupled);
        if streams == 0 || coupled > streams || decoded > 255 {
            return None;
        }
        let mapping = data.get(mapping_at + 2..mapping_at + 2 + usize::from(channels))?;
        if mapping
            .iter()
            .any(|index| *index != 255 && u16::from(*index) >= decoded)
        {
            return None;
        }
        if family == 1 && channels > 8 {
            return None;
        }
    }
    // OpusHead's input sample rate describes the original input, not the
    // encoded 48 kHz timeline. Unspecified/ambisonic mappings have no surround label.
    Some(AudioHeader {
        channels,
        sample_rate: 48_000,
        layout: if family <= 1 {
            conventional_layout(channels)
        } else {
            None
        },
        bit_depth: None,
        bitrate: None,
        warning: (2..=254)
            .contains(&family)
            .then_some(("audio_channel_mapping_unsupported", false)),
    })
}

fn vorbis(data: &[u8]) -> Option<AudioHeader> {
    let identification = if data.starts_with(b"\x01vorbis") {
        data
    } else {
        if data.first() != Some(&2) {
            return None;
        }
        let mut offset = 1_usize;
        let mut sizes = [0_usize; 2];
        for size in &mut sizes {
            loop {
                let byte = *data.get(offset)?;
                offset += 1;
                *size = size.checked_add(usize::from(byte))?;
                if byte != 255 {
                    break;
                }
            }
        }
        let end = offset.checked_add(sizes[0])?;
        data.get(end..end.checked_add(sizes[1])?)?;
        data.get(offset..end)?
    };
    if identification.len() < 30
        || !identification.starts_with(b"\x01vorbis")
        || u32le(identification, 7)? != 0
        || identification[29] != 1
    {
        return None;
    }
    let channels = identification[11];
    let sample_rate = u32le(identification, 12)?;
    let small = identification[28] & 15;
    let large = identification[28] >> 4;
    if channels == 0
        || sample_rate == 0
        || !(6..=13).contains(&small)
        || !(small..=13).contains(&large)
    {
        return None;
    }
    let nominal = i32::from_le_bytes(identification[20..24].try_into().ok()?);
    Some(AudioHeader {
        channels,
        sample_rate,
        layout: conventional_layout(channels),
        bit_depth: None,
        bitrate: (nominal > 0).then_some(i64::from(nominal)),
        warning: None,
    })
}

fn flac(data: &[u8]) -> Option<AudioHeader> {
    let (info, remaining) = if data.starts_with(b"fLaC") {
        if data.get(4)? & 0x7f != 0 || data.get(5..8)? != [0, 0, 34] {
            return None;
        }
        (data.get(8..42)?, data.get(42..)?)
    } else if data.len() == 34 {
        (data, &[][..])
    } else {
        return None;
    };
    let packed = u64::from_be_bytes(info[10..18].try_into().ok()?);
    let sample_rate = (packed >> 44) as u32;
    let channels = ((packed >> 41) & 7) as u8 + 1;
    let depth = ((packed >> 36) & 31) as u32 + 1;
    if sample_rate == 0 || depth < 4 {
        return None;
    }
    let mut layout = conventional_layout(channels);
    let mut warning = None;
    // FLAC permits an explicit speaker-mask override in Vorbis comments.
    // Walk only the already bounded codec-private metadata, never audio frames.
    let mut remaining = remaining;
    let mut blocks = 0;
    let mut seen_comments = false;
    while !remaining.is_empty() {
        blocks += 1;
        if blocks > 256 || remaining.len() < 4 {
            layout = None;
            warning = Some(("flac_metadata_incomplete", blocks > 256));
            break;
        }
        let size = (usize::from(remaining[1]) << 16)
            | (usize::from(remaining[2]) << 8)
            | usize::from(remaining[3]);
        let Some(payload) = remaining.get(4..4 + size) else {
            layout = None;
            warning = Some(("flac_metadata_incomplete", false));
            break;
        };
        if remaining[0] & 0x7f == 4 {
            let comment_count = u32le(payload, 0)
                .and_then(|size| usize::try_from(size).ok())
                .and_then(|size| size.checked_add(4))
                .and_then(|at| u32le(payload, at));
            let exhausted = comment_count.is_some_and(|count| count > 4096);
            if seen_comments || exhausted {
                layout = None;
                warning = Some(("flac_metadata_incomplete", exhausted));
                break;
            }
            seen_comments = true;
            match flac_comment_mask(payload, channels) {
                Some(CommentMask::Explicit(mask)) => layout = mask,
                Some(CommentMask::Unspecified) => {}
                None => {
                    layout = None;
                    warning = Some(("flac_metadata_incomplete", false));
                    break;
                }
            }
        }
        remaining = &remaining[4 + size..];
    }
    Some(AudioHeader {
        channels,
        sample_rate,
        layout,
        bit_depth: Some(depth),
        bitrate: None,
        warning,
    })
}

enum CommentMask {
    Unspecified,
    Explicit(Option<String>),
}

fn flac_comment_mask(data: &[u8], channels: u8) -> Option<CommentMask> {
    let vendor = usize::try_from(u32le(data, 0)?).ok()?;
    let mut at = 4_usize.checked_add(vendor)?;
    let count = u32le(data, at)?;
    at = at.checked_add(4)?;
    if count > 4096 {
        return None;
    }
    let mut result = None;
    for _ in 0..count {
        let size = usize::try_from(u32le(data, at)?).ok()?;
        at = at.checked_add(4)?;
        let end = at.checked_add(size)?;
        let entry = data.get(at..end)?;
        at = end;
        let Some((key, value)) = std::str::from_utf8(entry).ok()?.split_once('=') else {
            continue;
        };
        if key.eq_ignore_ascii_case("WAVEFORMATEXTENSIBLE_CHANNEL_MASK") {
            let value = value.trim();
            let mask = value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
                .and_then(|value| u32::from_str_radix(value, 16).ok())
                .or_else(|| value.parse().ok())?;
            let layout = speaker_mask_layout(mask, u16::from(channels));
            if (mask != 0 && layout.is_none())
                || result.as_ref().is_some_and(|previous| *previous != layout)
            {
                return None;
            }
            result = Some(layout);
        }
    }
    Some(match result {
        Some(layout) => CommentMask::Explicit(layout),
        None => CommentMask::Unspecified,
    })
}

fn speaker_mask_layout(mask: u32, channels: u16) -> Option<String> {
    if mask == 0 || mask & !0x3ffff != 0 || mask.count_ones() != u32::from(channels) {
        return None;
    }
    let named = match mask {
        0x4 => Some("mono"),
        0x3 => Some("stereo"),
        0xb => Some("2.1"),
        0x7 => Some("3.0"),
        0xf => Some("3.1"),
        0x107 => Some("4.0"),
        0x33 => Some("quad"),
        0x603 => Some("quad(side)"),
        0x37 => Some("5.0"),
        0x607 => Some("5.0(side)"),
        0x3f => Some("5.1"),
        0x60f => Some("5.1(side)"),
        0x63f => Some("7.1"),
        _ => None,
    };
    if let Some(name) = named {
        return Some(name.into());
    }
    const SPEAKERS: [&str; 18] = [
        "FL", "FR", "FC", "LFE", "BL", "BR", "FLC", "FRC", "BC", "SL", "SR", "TC", "TFL", "TFC",
        "TFR", "TBL", "TBC", "TBR",
    ];
    Some(
        SPEAKERS
            .iter()
            .enumerate()
            .filter_map(|(bit, name)| (mask & (1 << bit) != 0).then_some(*name))
            .collect::<Vec<_>>()
            .join("+"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_mapping_families_and_input_rate_remain_distinct() {
        let mut head =
            b"OpusHead\x01\x06\0\0\x44\xac\0\0\0\0\x01\x04\x02\x00\x04\x01\x02\x03\x05".to_vec();
        let parsed = opus(&head).unwrap();
        assert_eq!(parsed.sample_rate, 48_000);
        assert_eq!(parsed.layout.as_deref(), Some("5.1"));
        head[18] = 255;
        assert!(opus(&head).unwrap().layout.is_none());
        head[18] = 0;
        assert!(opus(&head).is_none());
        head[18] = 1;
        head[26] = 6;
        assert!(opus(&head).is_none());
        assert!(opus(&head[..20]).is_none());
        let dops = [0, 2, 0, 0, 0, 0, 0xac, 0x44, 0, 0, 0];
        assert_eq!(opus(&dops).unwrap().layout.as_deref(), Some("stereo"));
        assert_eq!(opus(&dops).unwrap().sample_rate, 48_000);
    }

    #[test]
    fn flac_streaminfo_preserves_depth_and_explicit_speaker_overrides() {
        let mut data = b"fLaC\0\0\0\x22".to_vec();
        let mut info = [0_u8; 34];
        let packed = (96_000_u64 << 44) | (5_u64 << 41) | (23_u64 << 36);
        info[10..18].copy_from_slice(&packed.to_be_bytes());
        data.extend(info);
        let header = flac(&data).unwrap();
        assert_eq!(header.sample_rate, 96_000);
        assert_eq!(header.channels, 6);
        assert_eq!(header.bit_depth, Some(24));
        assert_eq!(header.layout.as_deref(), Some("5.1"));
        let entry = b"WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x60f";
        let mut comment = vec![0_u8; 4];
        comment.extend(1_u32.to_le_bytes());
        comment.extend((entry.len() as u32).to_le_bytes());
        comment.extend(entry);
        data.extend([0x84, 0, 0, comment.len() as u8]);
        data.extend(comment);
        assert_eq!(flac(&data).unwrap().layout.as_deref(), Some("5.1(side)"));
        data.pop();
        assert!(flac(&data).unwrap().layout.is_none());
        assert_eq!(
            flac(&data).unwrap().warning,
            Some(("flac_metadata_incomplete", false))
        );
        assert!(flac(&data[..41]).is_none());
        data.truncate(42);
        data.extend([0x84, 0, 0, 8]);
        data.extend(0_u32.to_le_bytes());
        data.extend(4097_u32.to_le_bytes());
        let header = flac(&data).unwrap();
        assert_eq!(header.warning, Some(("flac_metadata_incomplete", true)));
        assert_eq!(header.sample_rate, 96_000);
        assert!(header.layout.is_none());
    }

    #[test]
    fn vorbis_xiph_lacing_requires_a_bounded_identification_packet() {
        let mut identification = vec![0_u8; 30];
        identification[..7].copy_from_slice(b"\x01vorbis");
        identification[11] = 6;
        identification[12..16].copy_from_slice(&48_000_u32.to_le_bytes());
        identification[20..24].copy_from_slice(&192_000_i32.to_le_bytes());
        identification[28] = 0xb8;
        identification[29] = 1;
        assert_eq!(
            vorbis(&identification).unwrap().layout.as_deref(),
            Some("5.1")
        );
        let mut private = vec![2, 30, 1];
        private.extend(identification);
        private.extend([0, 0]);
        let header = vorbis(&private).unwrap();
        assert_eq!(header.sample_rate, 48_000);
        assert_eq!(header.bitrate, Some(192_000));
        private[1] = 255;
        assert!(vorbis(&private).is_none());
    }

    #[test]
    fn wave_extensible_keeps_sample_precision_separate_from_storage_width() {
        let mut data = [0_u8; 40];
        data[0..2].copy_from_slice(&0xfffe_u16.to_le_bytes());
        data[2..4].copy_from_slice(&6_u16.to_le_bytes());
        data[4..8].copy_from_slice(&96_000_u32.to_le_bytes());
        data[14..16].copy_from_slice(&24_u16.to_le_bytes());
        data[16..18].copy_from_slice(&22_u16.to_le_bytes());
        data[18..20].copy_from_slice(&20_u16.to_le_bytes());
        data[20..24].copy_from_slice(&0x60f_u32.to_le_bytes());
        data[24..40].copy_from_slice(&[
            1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71,
        ]);
        let metadata = wave_format(&data);
        assert_eq!(metadata.sample_rate, Some(96_000));
        assert_eq!(metadata.sample_format.as_deref(), Some("s24le"));
        assert_eq!(metadata.sample_bit_depth, Some(20));
        assert_eq!(metadata.channel_layout.as_deref(), Some("5.1(side)"));
        data[20..24].copy_from_slice(&0x3f_u32.to_le_bytes());
        assert_eq!(wave_format(&data).channel_layout.as_deref(), Some("5.1"));
        data[20..24].fill(0);
        assert!(wave_format(&data).channel_layout.is_none());
        data[18..20].copy_from_slice(&25_u16.to_le_bytes());
        assert!(wave_format(&data).sample_bit_depth.is_none());
        assert!(wave_format(&data[..39]).sample_bit_depth.is_none());
        assert!(speaker_mask_layout(0x3f, 2).is_none());
        assert!(speaker_mask_layout(1 << 31, 1).is_none());
    }
}
