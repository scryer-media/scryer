//! Motion JPEG frame markers: precision, chroma sampling, aspect and size.

use crate::raw::RawTrack;
use scryer_media_model::Provenance;

/// Inspect marker headers only; entropy-coded image data is never read here.
pub(crate) fn jpeg(track: &mut RawTrack, data: &[u8]) -> Option<()> {
    use scryer_media_model::Rational;
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
                for component in payload[6..].as_chunks::<3>().0 {
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
                for component in payload[1..1 + components * 2].as_chunks::<2>().0 {
                    let id = usize::from(component[0]);
                    if ids[id]
                        || !header[6..]
                            .as_chunks::<3>()
                            .0
                            .iter()
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
                scryer_media_model::Rational::new(3, 4)
            );
            assert_eq!(
                track.metadata.display_aspect_ratio,
                scryer_media_model::Rational::new(1, 1)
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
}
