use crate::types::RawTrack;
use scryer_media_types::Provenance;

pub(crate) fn enrich(track: &mut RawTrack, data: &[u8]) {
    match track.codec_name.as_deref() {
        Some("mjpeg") => {
            let _ = jpeg(track, data);
        }
        Some("vp9") => {
            let _ = vp9(track, data);
        }
        Some("vp8") if data.len() >= 10 && data[0] & 1 == 0 && data[3..6] == [0x9d, 1, 0x2a] => {
            track.metadata.bit_depth = Some(8);
            track.metadata.profile = Some(((data[0] >> 1) & 7).to_string());
            track.metadata.pixel_format = Some("yuv420p".into());
        }
        Some("mpeg1video" | "mpeg2video") => {
            let _ = mpeg12(track, data);
        }
        Some("mpeg4") => {
            for index in 0..data.len().saturating_sub(4) {
                if data[index..index + 4] == [0, 0, 1, 0xb0] {
                    let profile_level = data[index + 4];
                    track.metadata.profile = match profile_level >> 4 {
                        0 => Some("Simple Profile"),
                        1 => Some("Simple Scalable Profile"),
                        2 => Some("Core Profile"),
                        3 => Some("Main Profile"),
                        4 => Some("N-bit Profile"),
                        5 => Some("Scalable Texture Profile"),
                        6 => Some("Simple Face Animation Profile"),
                        7 => Some("Basic Animated Texture Profile"),
                        8 => Some("Hybrid Profile"),
                        9 => Some("Advanced Real Time Simple Profile"),
                        10 => Some("Core Scalable Profile"),
                        11 => Some("Advanced Coding Efficiency Profile"),
                        12 => Some("Advanced Core Profile"),
                        13 => Some("Advanced Scalable Texture Profile"),
                        14 => Some("Simple Studio Profile"),
                        15 => Some("Advanced Simple Profile"),
                        _ => None,
                    }
                    .map(str::to_string);
                    track.metadata.level = Some(u32::from(profile_level & 15));
                    break;
                }
            }
            if let Some(start) = data
                .windows(4)
                .position(|bytes| bytes[..3] == [0, 0, 1] && (0x20..=0x2f).contains(&bytes[3]))
            {
                let _ = mpeg4_vol(track, &data[start + 4..]);
            }
        }
        Some("vc1") => {
            for index in 0..data.len().saturating_sub(5) {
                if data[index..index + 4] == [0, 0, 1, 0x0f] && data[index + 4] >> 6 == 3 {
                    track.metadata.profile = Some("Advanced".into());
                    track.metadata.level = Some(u32::from((data[index + 4] >> 3) & 7));
                    track.metadata.bit_depth = Some(8);
                    track.metadata.pixel_format = Some("yuv420p".into());
                    break;
                }
            }
        }
        _ => {}
    }
}
/// Inspect marker headers only; entropy-coded image data is never read here.
fn jpeg(track: &mut RawTrack, data: &[u8]) -> Option<()> {
    use scryer_media_types::Rational;
    let data = &data[..data.len().min(64 * 1024)];
    if !data.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut at = 2;
    let mut frame = None;
    let mut jfif = false;
    let mut aspect = None;
    let mut adobe = None;
    let mut found_scan = false;
    for _ in 0..256 {
        if *data.get(at)? != 0xff {
            return None;
        }
        while data.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = *data.get(at)?;
        at += 1;
        if matches!(marker, 0 | 0xd0..=0xd9) {
            return None;
        }
        if marker == 1 {
            continue;
        }
        let length = usize::from(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?));
        if length < 2 {
            return None;
        }
        let payload = data.get(at + 2..at.checked_add(length)?)?;
        at += length;
        match marker {
            0xe0 if payload.starts_with(b"JFIF\0") => {
                if payload.len() < 14 || payload[5] != 1 || payload[7] > 2 {
                    return None;
                }
                let thumbnail_bytes = usize::from(payload[12]) * usize::from(payload[13]) * 3;
                if payload.len() < 14 + thumbnail_bytes {
                    return None;
                }
                jfif = true;
                let x = u16::from_be_bytes([payload[8], payload[9]]);
                let y = u16::from_be_bytes([payload[10], payload[11]]);
                aspect = (x > 0 && y > 0)
                    .then(|| Rational::new(i64::from(y), u64::from(x)))
                    .flatten();
            }
            0xee if payload.starts_with(b"Adobe") => {
                adobe = Some(*payload.get(11)?);
            }
            0xc0..=0xc3 => {
                if frame.is_some() || payload.len() < 6 {
                    return None;
                }
                let depth = payload[0];
                let valid_depth = match marker {
                    0xc0 => depth == 8,
                    0xc1 | 0xc2 => matches!(depth, 8 | 12),
                    _ => (2..=16).contains(&depth),
                };
                let components = usize::from(payload[5]);
                if !valid_depth
                    || !(1..=4).contains(&components)
                    || payload.len() != 6 + components * 3
                {
                    return None;
                }
                let mut ids = [false; 256];
                for component in payload[6..].chunks_exact(3) {
                    let id = usize::from(component[0]);
                    if ids[id]
                        || !(1..=4).contains(&(component[1] >> 4))
                        || !(1..=4).contains(&(component[1] & 15))
                        || component[2] > 3
                    {
                        return None;
                    }
                    ids[id] = true;
                }
                frame = Some((marker, payload));
            }
            0xda => {
                let components = usize::from(*payload.first()?);
                if !(1..=4).contains(&components) || payload.len() != 4 + components * 2 {
                    return None;
                }
                let (_, header) = frame?;
                let mut ids = [false; 256];
                for component in payload[1..1 + components * 2].chunks_exact(2) {
                    let id = usize::from(component[0]);
                    if ids[id]
                        || !header[6..]
                            .chunks_exact(3)
                            .any(|entry| entry[0] == component[0])
                        || component[1] >> 4 > 3
                        || component[1] & 15 > 3
                    {
                        return None;
                    }
                    ids[id] = true;
                }
                found_scan = true;
                break;
            }
            _ => {}
        }
    }
    if !found_scan {
        return None;
    }
    let (marker, frame) = frame?;
    let height = u16::from_be_bytes([frame[1], frame[2]]);
    let width = u16::from_be_bytes([frame[3], frame[4]]);
    if width == 0 || height == 0 {
        return None;
    }
    let depth = i32::from(frame[0]);
    track.metadata.bit_depth = Some(depth);
    track.metadata.profile = Some(
        match marker {
            0xc0 => "Baseline",
            0xc1 => "Extended sequential",
            0xc2 => "Progressive",
            _ => "Lossless",
        }
        .into(),
    );
    // A stored video frame can contain two field pictures. Preserve its full
    // container dimensions instead of replacing them with one field's height.
    track.width = track.width.or(Some(i32::from(width)));
    track.height = track.height.or(Some(i32::from(height)));
    let ycbcr = (jfif || adobe == Some(1))
        && adobe.is_none_or(|value| value == 1)
        && frame[5] == 3
        && frame[6] == 1
        && frame[9] == 2
        && frame[12] == 3;
    if ycbcr {
        let sampling = (frame[7], frame[10], frame[13]);
        if depth == 8 {
            track.metadata.pixel_format = match sampling {
                (0x22, 0x11, 0x11) => Some("yuvj420p"),
                (0x21, 0x11, 0x11) => Some("yuvj422p"),
                (0x11, 0x11, 0x11) => Some("yuvj444p"),
                (0x12, 0x11, 0x11) => Some("yuvj440p"),
                _ => None,
            }
            .map(str::to_owned);
        }
        track.metadata.color.full_range = Some(true);
        track.metadata.color.matrix = Some(5);
        track.metadata.color.provenance = Provenance::Bitstream;
    }
    if let Some(aspect) = aspect {
        track.metadata.sample_aspect_ratio = Some(aspect);
        track.metadata.display_aspect_ratio = track.width.zip(track.height).and_then(|(w, h)| {
            (w > 0 && h > 0)
                .then(|| {
                    Rational::new(
                        aspect.numerator * i64::from(w),
                        aspect.denominator * h as u64,
                    )
                })
                .flatten()
        });
    }
    Some(())
}

pub(crate) fn mpeg_sequence_header_end(data: &[u8], start: usize) -> Option<usize> {
    // Matrix data may resemble start codes and must stay outside the extension
    // scan. The second load flag follows the optional intra matrix bit field.
    let intra_bytes = if data.get(start + 7)? & 2 != 0 { 64 } else { 0 };
    let non_intra_bytes = if data.get(start + 7 + intra_bytes)? & 1 != 0 {
        64
    } else {
        0
    };
    let end = start + 8 + intra_bytes + non_intra_bytes;
    data.get(start..end)?;
    Some(end)
}

fn mpeg12(track: &mut RawTrack, data: &[u8]) -> Option<()> {
    use scryer_media_types::Rational;
    let start = data.windows(4).position(|bytes| bytes == [0, 0, 1, 0xb3])? + 4;
    let header_end = mpeg_sequence_header_end(data, start)?;
    let mut bits = crate::ts::BitReader::new(data.get(start..start + 8)?);
    let mut width = bits.read_bits(12)?;
    let mut height = bits.read_bits(12)?;
    let aspect_code = bits.read_bits(4)? as usize;
    let rate_code = bits.read_bits(4)?;
    bits.read_bits(18)?;
    marker(&mut bits)?;
    let (mut n, mut d) = match rate_code {
        1 => (24_000, 1001),
        2 => (24, 1),
        3 => (25, 1),
        4 => (30_000, 1001),
        5 => (30, 1),
        6 => (50, 1),
        7 => (60_000, 1001),
        8 => (60, 1),
        _ => return None,
    };
    let mpeg2 = track.codec_name.as_deref() == Some("mpeg2video");
    let mut progressive = !mpeg2;
    let mut format = "yuv420p";
    let mut profile_level = None;
    let mut color = track.metadata.color.clone();
    let mut display_dimensions = None;
    if mpeg2 {
        for (at, code) in data[header_end..]
            .windows(4)
            .enumerate()
            .filter(|(_, bytes)| bytes[..3] == [0, 0, 1])
            .take(64)
        {
            if matches!(code[3], 0 | 0xb3 | 0xb8) {
                break;
            }
            if code[3] != 0xb5 {
                continue;
            }
            let mut ext = crate::ts::BitReader::new(&data[header_end + at + 4..]);
            match ext.read_bits(4)? {
                1 => {
                    if profile_level.is_some() {
                        return None;
                    }
                    profile_level = Some(ext.read_bits(8)?);
                    progressive = ext.read_bits(1)? != 0;
                    format = match ext.read_bits(2)? {
                        1 => "yuv420p",
                        2 => "yuv422p",
                        3 => "yuv444p",
                        _ => return None,
                    };
                    width |= ext.read_bits(2)? << 12;
                    height |= ext.read_bits(2)? << 12;
                    ext.read_bits(12)?;
                    marker(&mut ext)?;
                    ext.read_bits(9)?;
                    n *= i64::from(ext.read_bits(2)? + 1);
                    d *= u64::from(ext.read_bits(5)? + 1);
                }
                2 => {
                    ext.read_bits(3)?;
                    if ext.read_bits(1)? != 0 {
                        let primaries = ext.read_bits(8)?;
                        let transfer = ext.read_bits(8)?;
                        let matrix = ext.read_bits(8)?;
                        if primaries != 2 {
                            color.primaries = Some(primaries);
                        }
                        if transfer != 2 {
                            color.transfer = Some(transfer);
                        }
                        if matrix != 2 {
                            color.matrix = Some(matrix);
                        }
                    }
                    let display_width = ext.read_bits(14)?;
                    marker(&mut ext)?;
                    let display_height = ext.read_bits(14)?;
                    if display_width > 0 && display_height > 0 {
                        display_dimensions = Some((display_width, display_height));
                    }
                }
                _ => {}
            }
        }
        profile_level?;
    }
    if width == 0 || height == 0 {
        return None;
    }
    let aspect = if mpeg2 {
        match aspect_code {
            1 => Rational::new(1, 1),
            2..=4 => {
                let (dar_n, dar_d) = match aspect_code {
                    2 => (4, 3),
                    3 => (16, 9),
                    _ => (221, 100),
                };
                let (display_width, display_height) = display_dimensions.unwrap_or((width, height));
                Rational::new(
                    dar_n * i64::from(display_height),
                    dar_d * u64::from(display_width),
                )
            }
            _ => None,
        }
    } else {
        // MPEG-1 encodes the reciprocal sample aspect ratio in units of 1/10000.
        let reciprocal = [
            0, 10000, 6735, 7031, 7615, 8055, 8437, 8935, 9157, 9815, 10255, 10695, 10950, 11575,
            12015,
        ];
        reciprocal
            .get(aspect_code)
            .filter(|value| **value > 0)
            .and_then(|value| Rational::new(10_000, *value))
    };
    apply_mpeg4_picture(track, width, height, aspect, !progressive, 8, format);
    track.metadata.declared_frame_rate = Rational::new(n, d);
    track.frame_rate_fps = Some(n as f64 / d as f64);
    color.full_range = Some(false);
    color.provenance = Provenance::Bitstream;
    track.metadata.color = color;
    if let Some(value) = profile_level {
        track.metadata.profile = match (value >> 4) & 7 {
            1 => Some("High"),
            2 => Some("Spatially Scalable"),
            3 => Some("SNR Scalable"),
            4 => Some("Main"),
            5 => Some("Simple"),
            _ => None,
        }
        .map(str::to_string);
        track.metadata.level = Some(value & 15);
    }
    Some(())
}

fn marker(bits: &mut crate::ts::BitReader<'_>) -> Option<()> {
    (bits.read_bits(1)? == 1).then_some(())
}

fn mpeg4_aspect(
    bits: &mut crate::ts::BitReader<'_>,
) -> Option<Option<scryer_media_types::Rational>> {
    use scryer_media_types::Rational;
    let pair = match bits.read_bits(4)? {
        1 => Some((1, 1)),
        2 => Some((12, 11)),
        3 => Some((10, 11)),
        4 => Some((16, 11)),
        5 => Some((40, 33)),
        15 => Some((bits.read_bits(8)?, bits.read_bits(8)?)),
        _ => None,
    };
    Some(
        pair.filter(|(n, d)| *n > 0 && *d > 0)
            .and_then(|(n, d)| Rational::new(i64::from(n), u64::from(d))),
    )
}

fn mpeg4_vol(track: &mut RawTrack, data: &[u8]) -> Option<()> {
    use scryer_media_types::Rational;
    let mut bits = crate::ts::BitReader::new(data);
    bits.read_bits(1)?; // random access
    let object_type = bits.read_bits(8)?;
    if object_type == 0 || object_type > 17 {
        return None;
    }
    if matches!(object_type, 14 | 15) {
        return mpeg4_studio_vol(track, &mut bits);
    }
    let version = if bits.read_bits(1)? != 0 {
        let version = bits.read_bits(4)?;
        bits.read_bits(3)?;
        if version == 0 {
            return None;
        }
        version
    } else {
        1
    };
    let aspect = mpeg4_aspect(&mut bits)?;
    if bits.read_bits(1)? != 0 {
        if bits.read_bits(2)? != 1 {
            return None;
        } // non-studio VOL uses 4:2:0
        bits.read_bits(1)?;
        if bits.read_bits(1)? != 0 {
            for width in [15, 15, 15, 14, 15] {
                bits.read_bits(width)?;
                marker(&mut bits)?;
            }
        }
    }
    if bits.read_bits(2)? != 0 {
        return None;
    } // nonrectangular shape needs separate syntax
    marker(&mut bits)?;
    let clock = bits.read_bits(16)?;
    if clock == 0 {
        return None;
    }
    marker(&mut bits)?;
    let rate = if bits.read_bits(1)? != 0 {
        let width = (32 - (clock - 1).leading_zeros()).max(1) as usize;
        let step = bits.read_bits(width)?;
        if step == 0 {
            return None;
        }
        Rational::new(i64::from(clock), u64::from(step))
    } else {
        None
    };
    marker(&mut bits)?;
    let width = bits.read_bits(13)?;
    marker(&mut bits)?;
    let height = bits.read_bits(13)?;
    marker(&mut bits)?;
    if width == 0 || height == 0 {
        return None;
    }
    let interlaced = bits.read_bits(1)? != 0;
    bits.read_bits(1)?; // OBMC disable
    let sprite = bits.read_bits(if version == 1 { 1 } else { 2 })?;
    if sprite == 3 {
        return None;
    }
    if sprite == 1 {
        for _ in 0..4 {
            bits.read_bits(13)?;
            marker(&mut bits)?;
        }
    }
    if sprite != 0 {
        if bits.read_bits(6)? > 3 {
            return None;
        }
        bits.read_bits(3)?; // accuracy and brightness change
        if sprite == 1 {
            bits.read_bits(1)?;
        }
    }
    let depth = if bits.read_bits(1)? != 0 {
        let precision = bits.read_bits(4)?;
        let depth = bits.read_bits(4)?;
        if !(3..=9).contains(&precision) || !(4..=12).contains(&depth) {
            return None;
        }
        depth
    } else {
        8
    };
    apply_mpeg4_picture(track, width, height, aspect, interlaced, depth, "yuv420p");
    if let Some(rate) = rate {
        track.frame_rate_fps = Some(rate.numerator as f64 / rate.denominator as f64);
        track.metadata.declared_frame_rate = Some(rate);
    }
    Some(())
}

fn mpeg4_studio_vol(track: &mut RawTrack, bits: &mut crate::ts::BitReader<'_>) -> Option<()> {
    bits.read_bits(4)?; // version
    if bits.read_bits(2)? != 0 {
        return None;
    }
    bits.read_bits(4)?; // shape extension
    let progressive = bits.read_bits(1)? != 0;
    let rgb = bits.read_bits(1)? != 0;
    let chroma = bits.read_bits(2)?;
    let format = match (rgb, chroma) {
        (false, 2) => "yuv422p",
        (false, 3) => "yuv444p",
        (true, 3) => "gbrp",
        _ => return None,
    };
    let depth = bits.read_bits(4)?;
    if !(8..=12).contains(&depth) {
        return None;
    }
    marker(bits)?;
    let width = bits.read_bits(14)?;
    marker(bits)?;
    let height = bits.read_bits(14)?;
    marker(bits)?;
    if width == 0 || height == 0 {
        return None;
    }
    let aspect = mpeg4_aspect(bits)?;
    apply_mpeg4_picture(track, width, height, aspect, !progressive, depth, format);
    Some(())
}

fn apply_mpeg4_picture(
    track: &mut RawTrack,
    width: u32,
    height: u32,
    aspect: Option<scryer_media_types::Rational>,
    interlaced: bool,
    depth: u32,
    format: &str,
) {
    track.width = Some(width as i32);
    track.height = Some(height as i32);
    track.metadata.bit_depth = i32::try_from(depth).ok();
    track.metadata.pixel_format = match depth {
        8 => Some(format.into()),
        9 | 10 | 12 => Some(format!("{format}{depth}le")),
        _ => None,
    };
    track.metadata.field_order = Some(
        if interlaced {
            "interlaced"
        } else {
            "progressive"
        }
        .into(),
    );
    if let Some(aspect) = aspect {
        track.metadata.sample_aspect_ratio = Some(aspect);
        track.metadata.display_aspect_ratio = i64::from(width)
            .checked_mul(aspect.numerator)
            .zip(u64::from(height).checked_mul(aspect.denominator))
            .and_then(|(n, d)| scryer_media_types::Rational::new(n, d));
    }
}

fn vp9(track: &mut RawTrack, data: &[u8]) -> Option<()> {
    let mut bits = crate::ts::BitReader::new(data);
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

    fn jpeg_headers(jfif: bool, marker: u8, depth: u8, sampling: u8) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8];
        if jfif {
            bytes.extend([0xff, 0xe0, 0, 16]);
            bytes.extend(b"JFIF\0");
            bytes.extend([1, 2, 0, 0, 4, 0, 3, 0, 0]);
        }
        bytes.extend([
            0xff, marker, 0, 17, depth, 0, 120, 1, 64, 3, 1, sampling, 0, 2, 0x11, 1, 3, 0x11, 1,
        ]);
        bytes.extend([0xff, 0xda, 0, 12, 3, 1, 0, 2, 0x11, 3, 0x11, 0, 63, 0]);
        bytes
    }

    #[test]
    fn jpeg_headers_preserve_depth_chroma_aspect_and_full_frame_dimensions() {
        for (sampling, format) in [
            (0x22, "yuvj420p"),
            (0x21, "yuvj422p"),
            (0x11, "yuvj444p"),
            (0x12, "yuvj440p"),
        ] {
            let bytes = jpeg_headers(true, 0xc0, 8, sampling);
            let mut track = RawTrack {
                width: Some(320),
                height: Some(240),
                ..Default::default()
            };
            assert!(jpeg(&mut track, &bytes).is_some());
            assert_eq!(track.metadata.bit_depth, Some(8));
            assert_eq!(track.metadata.profile.as_deref(), Some("Baseline"));
            assert_eq!(track.metadata.pixel_format.as_deref(), Some(format));
            assert_eq!(track.metadata.color.full_range, Some(true));
            assert_eq!(track.metadata.color.matrix, Some(5));
            assert_eq!(track.metadata.color.provenance, Provenance::Bitstream);
            assert_eq!(track.height, Some(240));
            assert_eq!(
                track.metadata.sample_aspect_ratio,
                scryer_media_types::Rational::new(3, 4)
            );
            assert_eq!(
                track.metadata.display_aspect_ratio,
                scryer_media_types::Rational::new(1, 1)
            );
            assert!(track.metadata.field_order.is_none());
        }
        let mut high_depth = RawTrack::default();
        assert!(jpeg(&mut high_depth, &jpeg_headers(true, 0xc2, 12, 0x22)).is_some());
        assert_eq!(high_depth.metadata.bit_depth, Some(12));
        assert_eq!(high_depth.metadata.profile.as_deref(), Some("Progressive"));
        assert!(high_depth.metadata.pixel_format.is_none());
        let mut unspecified = RawTrack::default();
        assert!(jpeg(&mut unspecified, &jpeg_headers(false, 0xc0, 8, 0x22)).is_some());
        assert!(unspecified.metadata.pixel_format.is_none());
        assert!(unspecified.metadata.color.full_range.is_none());
        assert!(unspecified.metadata.sample_aspect_ratio.is_none());
    }

    #[test]
    fn jpeg_incomplete_or_invalid_headers_do_not_publish_partial_facts() {
        let bytes = jpeg_headers(true, 0xc0, 8, 0x22);
        for end in 0..bytes.len() {
            let mut track = RawTrack::default();
            assert!(jpeg(&mut track, &bytes[..end]).is_none(), "prefix {end}");
            assert!(track.metadata.bit_depth.is_none());
        }
        for bytes in [
            jpeg_headers(true, 0xc0, 12, 0x22),
            jpeg_headers(true, 0xc0, 8, 0x02),
        ] {
            assert!(jpeg(&mut RawTrack::default(), &bytes).is_none());
        }
        let scan = bytes
            .windows(2)
            .position(|pair| pair == [0xff, 0xda])
            .unwrap();
        let mut bad_reference = bytes.clone();
        bad_reference[scan + 5] = 99;
        assert!(jpeg(&mut RawTrack::default(), &bad_reference).is_none());
        let mut duplicate = bytes.clone();
        duplicate[scan + 7] = duplicate[scan + 5];
        assert!(jpeg(&mut RawTrack::default(), &duplicate).is_none());
        let mut too_many = vec![0xff, 0xd8];
        for _ in 0..256 {
            too_many.extend([0xff, 1]);
        }
        too_many.extend(&bytes[2..]);
        assert!(jpeg(&mut RawTrack::default(), &too_many).is_none());
        let mut oversized = vec![0xff, 0xd8, 0xff, 0xfe, 0xff, 0xff];
        oversized.resize(65539, 0);
        oversized.extend(&bytes[2..]);
        assert!(jpeg(&mut RawTrack::default(), &oversized).is_none());
    }

    #[test]
    fn mjpeg_properties_reach_avi_and_matroska_catalog_analysis() {
        for fixture in ["matrix_avi_005.avi", "matrix_mkv_015.mkv"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/media")
                .join(fixture);
            let analysis = crate::analyze_catalog_file(&path).unwrap();
            let video = analysis
                .details
                .streams
                .iter()
                .find(|stream| stream.codec.as_deref() == Some("mjpeg"))
                .unwrap();
            assert_eq!(video.metadata.bit_depth, Some(8), "{fixture}");
            assert_eq!(
                video.metadata.pixel_format.as_deref(),
                Some("yuvj420p"),
                "{fixture}"
            );
            assert_eq!(
                video.metadata.profile.as_deref(),
                Some("Baseline"),
                "{fixture}"
            );
            assert_eq!(
                video.metadata.sample_aspect_ratio,
                scryer_media_types::Rational::new(1, 1),
                "{fixture}"
            );
            assert_eq!(video.metadata.color.matrix, Some(5), "{fixture}");
        }
    }

    fn packed(fields: &[(u32, usize)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut offset = 0;
        for &(value, width) in fields {
            assert!(width == 32 || value < 1 << width);
            for shift in (0..width).rev() {
                if offset % 8 == 0 {
                    bytes.push(0);
                }
                let last = bytes.last_mut().unwrap();
                *last |= (((value >> shift) & 1) as u8) << (7 - offset % 8);
                offset += 1;
            }
        }
        bytes
    }

    #[test]
    fn mpeg2_sequence_extensions_preserve_aspect_chroma_color_and_timing() {
        let mut bytes = vec![0, 0, 1, 0xb3];
        bytes.extend(packed(&[
            (720, 12),
            (480, 12),
            (3, 4),
            (4, 4),
            (10_000, 18),
            (1, 1),
            (0, 10),
            (0, 3),
        ]));
        bytes.extend([0, 0, 1, 0xb5]);
        bytes.extend(packed(&[
            (1, 4),
            (0x48, 8),
            (0, 1),
            (2, 2),
            (0, 2),
            (0, 2),
            (0, 12),
            (1, 1),
            (0, 8),
            (0, 1),
            (1, 2),
            (2, 5),
        ]));
        let mut track = RawTrack {
            codec_name: Some("mpeg2video".into()),
            ..Default::default()
        };
        assert!(mpeg12(&mut track, &bytes).is_some());
        assert_eq!(track.metadata.profile.as_deref(), Some("Main"));
        assert_eq!(track.metadata.level, Some(8));
        assert_eq!(track.metadata.field_order.as_deref(), Some("interlaced"));
        assert_eq!(track.metadata.pixel_format.as_deref(), Some("yuv422p"));
        assert_eq!(
            track.metadata.sample_aspect_ratio,
            scryer_media_types::Rational::new(32, 27)
        );
        assert_eq!(
            track.metadata.display_aspect_ratio,
            scryer_media_types::Rational::new(16, 9)
        );
        assert_eq!(
            track.metadata.declared_frame_rate,
            scryer_media_types::Rational::new(20_000, 1001)
        );
        let mut short = RawTrack {
            codec_name: Some("mpeg2video".into()),
            ..Default::default()
        };
        assert!(mpeg12(&mut short, &bytes[..bytes.len() - 1]).is_none());
        assert_eq!(short.metadata.bit_depth, None);

        bytes.extend([0, 0, 1, 0xb5]);
        bytes.extend(packed(&[
            (2, 4),
            (5, 3),
            (1, 1),
            (1, 8),
            (1, 8),
            (1, 8),
            (720, 14),
            (1, 1),
            (480, 14),
        ]));
        assert!(mpeg12(&mut track, &bytes).is_some());
        assert_eq!(track.metadata.color.primaries, Some(1));
        assert_eq!(track.metadata.color.transfer, Some(1));
        assert_eq!(track.metadata.color.matrix, Some(1));
        assert_eq!(track.metadata.color.full_range, Some(false));
    }

    #[test]
    fn mpeg1_aspect_codes_and_missing_sequence_remain_distinct() {
        let mut bytes = vec![0, 0, 1, 0xb3];
        bytes.extend(packed(&[
            (352, 12),
            (240, 12),
            (2, 4),
            (1, 4),
            (1000, 18),
            (1, 1),
            (0, 10),
            (0, 3),
        ]));
        let mut track = RawTrack {
            codec_name: Some("mpeg1video".into()),
            ..Default::default()
        };
        assert!(mpeg12(&mut track, &bytes).is_some());
        assert_eq!(
            track.metadata.sample_aspect_ratio,
            scryer_media_types::Rational::new(10_000, 6735)
        );
        assert_eq!(
            track.metadata.declared_frame_rate,
            scryer_media_types::Rational::new(24_000, 1001)
        );
        assert_eq!(track.metadata.profile, None);
        let mut missing = RawTrack {
            codec_name: Some("mpeg1video".into()),
            ..Default::default()
        };
        enrich(&mut missing, &[0, 0, 1, 0xb3]);
        assert_eq!(missing.metadata.bit_depth, None);
    }

    #[test]
    fn mpeg4_authored_simple_profile_and_truncated_vol() {
        let bytes = include_bytes!("../tests/media/matrix_mp4_008.m4v");
        let start = bytes
            .windows(4)
            .position(|b| b[..3] == [0, 0, 1] && b[3] == 0x20)
            .unwrap()
            + 4;
        let mut track = RawTrack {
            codec_name: Some("mpeg4".into()),
            ..Default::default()
        };
        enrich(&mut track, bytes);
        assert_eq!(track.metadata.profile.as_deref(), Some("Simple Profile"));
        assert_eq!(track.metadata.level, Some(1));
        assert_eq!((track.width, track.height), (Some(64), Some(36)));
        assert_eq!(track.metadata.pixel_format.as_deref(), Some("yuv420p"));
        assert_eq!(track.metadata.bit_depth, Some(8));
        assert_eq!(
            track.metadata.sample_aspect_ratio,
            scryer_media_types::Rational::new(1, 1)
        );
        assert_eq!(track.metadata.field_order.as_deref(), Some("progressive"));
        for length in 0..8 {
            let mut short = RawTrack::default();
            assert!(mpeg4_vol(&mut short, &bytes[start..start + length]).is_none());
            assert_eq!(short.metadata.bit_depth, None);
        }
    }

    #[test]
    fn mpeg4_vol_only_declares_fixed_rates_and_explicit_precision() {
        for fixed in [false, true] {
            let mut fields = vec![
                (0, 1),
                (4, 8),
                (0, 1),
                (2, 4),
                (0, 1),
                (0, 2),
                (1, 1),
                (30_000, 16),
                (1, 1),
                (u32::from(fixed), 1),
            ];
            if fixed {
                fields.push((1001, 15));
            }
            fields.extend([
                (1, 1),
                (720, 13),
                (1, 1),
                (480, 13),
                (1, 1),
                (1, 1),
                (1, 1),
                (0, 1),
                (1, 1),
                (5, 4),
                (10, 4),
            ]);
            let bytes = packed(&fields);
            let mut track = RawTrack::default();
            assert!(mpeg4_vol(&mut track, &bytes).is_some());
            assert_eq!(track.metadata.bit_depth, Some(10));
            assert_eq!(track.metadata.pixel_format.as_deref(), Some("yuv420p10le"));
            assert_eq!(track.metadata.field_order.as_deref(), Some("interlaced"));
            assert_eq!(
                track.metadata.display_aspect_ratio,
                scryer_media_types::Rational::new(18, 11)
            );
            assert_eq!(
                track.metadata.declared_frame_rate,
                if fixed {
                    scryer_media_types::Rational::new(30_000, 1001)
                } else {
                    None
                }
            );
            assert_eq!(track.frame_rate_fps.is_some(), fixed);
            fields[6] = (0, 1); // required timing marker
            let mut malformed = RawTrack::default();
            assert!(mpeg4_vol(&mut malformed, &packed(&fields)).is_none());
            assert_eq!(malformed.metadata.bit_depth, None);
        }
    }

    #[test]
    fn mpeg4_studio_preserves_chroma_and_rgb_precision() {
        for (rgb, chroma, expected) in [
            (0, 2, "yuv422p10le"),
            (0, 3, "yuv444p10le"),
            (1, 3, "gbrp10le"),
        ] {
            let fields = [
                (0, 1),
                (14, 8),
                (1, 4),
                (0, 2),
                (0, 4),
                (1, 1),
                (rgb, 1),
                (chroma, 2),
                (10, 4),
                (1, 1),
                (1920, 14),
                (1, 1),
                (1080, 14),
                (1, 1),
                (1, 4),
            ];
            let mut track = RawTrack::default();
            assert!(mpeg4_vol(&mut track, &packed(&fields)).is_some());
            assert_eq!(track.metadata.pixel_format.as_deref(), Some(expected));
            assert_eq!(track.metadata.bit_depth, Some(10));
            assert_eq!(
                track.metadata.display_aspect_ratio,
                scryer_media_types::Rational::new(16, 9)
            );
            assert_eq!(track.metadata.declared_frame_rate, None);
        }
    }

    #[test]
    fn vp9_color_space_preserves_explicit_container_transfer() {
        let mut track = RawTrack {
            codec_name: Some("vp9".into()),
            ..Default::default()
        };
        track.metadata.color.transfer = Some(16);
        track.metadata.color.matrix = Some(10);
        enrich(&mut track, &[0x92, 0x49, 0x83, 0x42, 0x50, 0, 0]);
        assert_eq!(track.metadata.color.primaries, Some(9));
        assert_eq!(track.metadata.color.transfer, Some(16));
        assert_eq!(track.metadata.color.matrix, Some(10));
        let mut unspecified = RawTrack {
            codec_name: Some("vp9".into()),
            ..Default::default()
        };
        enrich(&mut unspecified, &[0x92, 0x49, 0x83, 0x42, 0x50, 0, 0]);
        assert_eq!(unspecified.metadata.color.transfer, None);
        assert_eq!(unspecified.metadata.color.matrix, None);
    }

    #[test]
    fn vp9_10bit_keyframe_and_truncation() {
        let mut track = RawTrack {
            codec_name: Some("vp9".into()),
            ..Default::default()
        };
        // marker=2, profile=2, show-existing=0, keyframe=0, show=1, resilient=0.
        enrich(&mut track, &[0x92, 0x49, 0x83, 0x42, 0x50, 0, 0]);
        assert_eq!(track.metadata.bit_depth, Some(10));
        assert_eq!(track.metadata.profile.as_deref(), Some("Profile 2"));
        assert_eq!(track.metadata.pixel_format.as_deref(), Some("yuv420p10le"));
        let mut short = RawTrack {
            codec_name: Some("vp9".into()),
            ..Default::default()
        };
        enrich(&mut short, &[0x92, 0x49]);
        assert!(short.metadata.bit_depth.is_none());
    }
}
