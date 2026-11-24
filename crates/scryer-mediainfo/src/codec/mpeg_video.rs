//! MPEG-1/2 video sequence headers and their extensions, and MPEG-4 Part 2
//! video object layer headers.

use super::find_start_code_payload;
use crate::raw::RawTrack;
use scryer_media_model::Provenance;

const MPEG_VIDEO_FRAME_RATES: [Option<f64>; 16] = [
    None,
    Some(24000.0 / 1001.0),
    Some(24.0),
    Some(25.0),
    Some(30000.0 / 1001.0),
    Some(30.0),
    Some(50.0),
    Some(60000.0 / 1001.0),
    Some(60.0),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];

pub(crate) struct MpegVideoSequenceHeader {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) frame_rate_fps: Option<f64>,
    pub(crate) bit_rate_bps: Option<u32>,
}

pub(crate) fn find_mpeg_video_sequence_header(data: &[u8]) -> Option<MpegVideoSequenceHeader> {
    let payload = find_start_code_payload(data, 0xB3)?;
    if payload.len() < 8 {
        return None;
    }

    let width = ((u16::from(payload[0])) << 4) | u16::from(payload[1] >> 4);
    let height = ((u16::from(payload[1] & 0x0F)) << 8) | u16::from(payload[2]);
    let frame_rate_code = (payload[3] & 0x0F) as usize;
    let bit_rate_value = ((u32::from(payload[4])) << 10)
        | ((u32::from(payload[5])) << 2)
        | (u32::from(payload[6]) >> 6);

    Some(MpegVideoSequenceHeader {
        width,
        height,
        frame_rate_fps: MPEG_VIDEO_FRAME_RATES
            .get(frame_rate_code)
            .copied()
            .flatten(),
        bit_rate_bps: if bit_rate_value == 0 || bit_rate_value == 0x3_FFFF {
            None
        } else {
            Some(bit_rate_value * 400)
        },
    })
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

pub(crate) fn mpeg12(track: &mut RawTrack, data: &[u8]) -> Option<()> {
    use scryer_media_model::Rational;
    let start = data.windows(4).position(|bytes| bytes == [0, 0, 1, 0xb3])? + 4;
    let header_end = mpeg_sequence_header_end(data, start)?;
    let mut bits = crate::codec::bits::BitReader::new(data.get(start..start + 8)?);
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
            let mut ext = crate::codec::bits::BitReader::new(&data[header_end + at + 4..]);
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

fn marker(bits: &mut crate::codec::bits::BitReader<'_>) -> Option<()> {
    (bits.read_bits(1)? == 1).then_some(())
}

fn mpeg4_aspect(
    bits: &mut crate::codec::bits::BitReader<'_>,
) -> Option<Option<scryer_media_model::Rational>> {
    use scryer_media_model::Rational;
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

pub(crate) fn mpeg4_vol(track: &mut RawTrack, data: &[u8]) -> Option<()> {
    use scryer_media_model::Rational;
    let mut bits = crate::codec::bits::BitReader::new(data);
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

fn mpeg4_studio_vol(
    track: &mut RawTrack,
    bits: &mut crate::codec::bits::BitReader<'_>,
) -> Option<()> {
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
    aspect: Option<scryer_media_model::Rational>,
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
            .and_then(|(n, d)| scryer_media_model::Rational::new(n, d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::picture_headers::enrich;

    #[test]
    fn parses_mpeg2_video_sequence_header() {
        let data = [
            0x00, 0x00, 0x01, 0xB3, 0x2D, 0x01, 0xE0, 0x34, 0x00, 0x40, 0x00, 0x00,
        ];
        let header = find_mpeg_video_sequence_header(&data).unwrap();
        assert_eq!(header.width, 720);
        assert_eq!(header.height, 480);
        assert_eq!(header.frame_rate_fps, Some(30000.0 / 1001.0));
        assert!(header.bit_rate_bps.is_some());
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
            scryer_media_model::Rational::new(32, 27)
        );
        assert_eq!(
            track.metadata.display_aspect_ratio,
            scryer_media_model::Rational::new(16, 9)
        );
        assert_eq!(
            track.metadata.declared_frame_rate,
            scryer_media_model::Rational::new(20_000, 1001)
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
            scryer_media_model::Rational::new(10_000, 6735)
        );
        assert_eq!(
            track.metadata.declared_frame_rate,
            scryer_media_model::Rational::new(24_000, 1001)
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
        let bytes = include_bytes!("../../tests/media/matrix_mp4_008.m4v");
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
            scryer_media_model::Rational::new(1, 1)
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
                scryer_media_model::Rational::new(18, 11)
            );
            assert_eq!(
                track.metadata.declared_frame_rate,
                if fixed {
                    scryer_media_model::Rational::new(30_000, 1001)
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
                scryer_media_model::Rational::new(16, 9)
            );
            assert_eq!(track.metadata.declared_frame_rate, None);
        }
    }
}
