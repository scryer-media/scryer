//! AV1 sequence and metadata OBUs. Picture tiles are skipped without decoding.
use crate::types::RawTrack;
use scryer_media_types::{
    ColorMetadata, ContentLight, MasteringDisplay, ProbeReport, ProbeStatus, ProbeWarning,
    Provenance, Rational, StreamMetadata,
};

struct Bits<'a> {
    data: &'a [u8],
    at: usize,
}
impl Bits<'_> {
    fn read(&mut self, count: usize) -> Option<u32> {
        if count > 32 || self.at.checked_add(count)? > self.data.len().checked_mul(8)? {
            return None;
        }
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1) | u32::from((self.data[self.at / 8] >> (7 - self.at % 8)) & 1);
            self.at += 1;
        }
        Some(value)
    }
    fn flag(&mut self) -> Option<bool> {
        Some(self.read(1)? != 0)
    }
    fn uvlc(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while !self.flag()? {
            zeros += 1;
            if zeros >= 32 {
                return None;
            }
        }
        self.read(zeros)?.checked_add((1_u32 << zeros) - 1)
    }
    fn trailing(&mut self) -> Option<()> {
        if !self.flag()? {
            return None;
        }
        while self.at < self.data.len() * 8 {
            if self.flag()? {
                return None;
            }
        }
        Some(())
    }
}

fn leb128(data: &[u8], at: &mut usize) -> Option<u32> {
    let mut value = 0_u64;
    for index in 0..8 {
        let byte = *data.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 127) << (index * 7);
        if byte & 128 == 0 {
            return u32::try_from(value).ok();
        }
    }
    None
}

fn sequence(data: &[u8]) -> Option<StreamMetadata> {
    let mut bits = Bits { data, at: 0 };
    let profile = bits.read(3)?;
    if profile > 2 {
        return None;
    }
    let still = bits.flag()?;
    let reduced = bits.flag()?;
    if reduced && !still {
        return None;
    }
    let mut metadata = StreamMetadata {
        profile: Some(["Main", "High", "Professional"][profile as usize].into()),
        ..Default::default()
    };
    let mut delay_bits = None;
    if reduced {
        metadata.level = Some(bits.read(5)?);
    } else {
        if bits.flag()? {
            let tick = bits.read(32)?;
            let scale = bits.read(32)?;
            if tick == 0 || scale == 0 {
                return None;
            }
            if bits.flag()? {
                let ticks = u64::from(bits.uvlc()?) + 1;
                metadata.declared_frame_rate =
                    Rational::new(i64::from(scale), u64::from(tick).checked_mul(ticks)?);
            }
            if bits.flag()? {
                delay_bits = Some(bits.read(5)? as usize + 1);
                bits.read(32)?;
                bits.read(5)?;
                bits.read(5)?;
            }
        }
        let display_delay = bits.flag()?;
        let count = bits.read(5)? + 1;
        for index in 0..count {
            bits.read(12)?;
            let level = bits.read(5)?;
            if index == 0 {
                metadata.level = Some(level);
            }
            if level > 7 {
                bits.flag()?;
            }
            if let Some(width) = delay_bits {
                if bits.flag()? {
                    bits.read(width)?;
                    bits.read(width)?;
                    bits.flag()?;
                }
            }
            if display_delay && bits.flag()? {
                bits.read(4)?;
            }
        }
    }
    let width = bits.read(4)? as usize + 1;
    let height = bits.read(4)? as usize + 1;
    // Sequence maxima are not the displayed dimensions of every frame.
    bits.read(width)?;
    bits.read(height)?;
    if !reduced && bits.flag()? {
        bits.read(4)?;
        bits.read(3)?;
    }
    bits.read(3)?;
    if !reduced {
        bits.read(4)?;
        let order_hint = bits.flag()?;
        if order_hint {
            bits.read(2)?;
        }
        let screen_tools = bits.flag()? || bits.flag()?;
        if screen_tools && !bits.flag()? {
            bits.flag()?;
        }
        if order_hint {
            bits.read(3)?;
        }
    }
    bits.read(3)?;
    let high_depth = bits.flag()?;
    let depth = if profile == 2 && high_depth && bits.flag()? {
        12
    } else if high_depth {
        10
    } else {
        8
    };
    let mono = profile != 1 && bits.flag()?;
    let (primaries, transfer, matrix) = if bits.flag()? {
        (bits.read(8)?, bits.read(8)?, bits.read(8)?)
    } else {
        (2, 2, 2)
    };
    let rgb = !mono && primaries == 1 && transfer == 13 && matrix == 0;
    let full_range = rgb || bits.flag()?;
    let (sub_x, sub_y) = if mono {
        (true, true)
    } else if rgb || profile == 1 {
        (false, false)
    } else if profile == 0 {
        (true, true)
    } else if depth == 12 {
        let x = bits.flag()?;
        (x, x && bits.flag()?)
    } else {
        (true, false)
    };
    if !mono {
        if sub_x && sub_y {
            bits.read(2)?;
        }
        bits.flag()?;
    }
    bits.flag()?; // film grain parameters present
    bits.trailing()?;
    let family = if mono {
        "gray"
    } else if rgb {
        "gbrp"
    } else if sub_x && sub_y {
        "yuv420p"
    } else if sub_x {
        "yuv422p"
    } else {
        "yuv444p"
    };
    metadata.pixel_format = Some(if depth == 8 {
        family.into()
    } else {
        format!("{family}{depth}le")
    });
    metadata.bit_depth = Some(depth);
    metadata.field_order = Some("progressive".into());
    metadata.color = ColorMetadata {
        primaries: (primaries != 2).then_some(primaries),
        transfer: (transfer != 2).then_some(transfer),
        matrix: (matrix != 2).then_some(matrix),
        full_range: Some(full_range),
        provenance: Provenance::Bitstream,
        ..Default::default()
    };
    Some(metadata)
}

fn apply_sequence(track: &mut RawTrack, incoming: StreamMetadata, report: &mut ProbeReport) {
    let current = &mut track.metadata;
    let conflict = [
        (current.color.primaries, incoming.color.primaries),
        (current.color.transfer, incoming.color.transfer),
        (current.color.matrix, incoming.color.matrix),
    ]
    .iter()
    .any(|(old, new)| old.zip(*new).is_some_and(|(old, new)| old != new));
    if conflict {
        report.warnings.push(ProbeWarning {
            code: "color_signaling_conflict".into(),
            message: "AV1 sequence color signaling overrides conflicting metadata".into(),
            stream_id: current.id.clone(),
            ..Default::default()
        });
    }
    current.profile = incoming.profile;
    current.level = incoming.level;
    current.bit_depth = incoming.bit_depth;
    current.pixel_format = incoming.pixel_format;
    current.field_order = incoming.field_order;
    current.declared_frame_rate = current.declared_frame_rate.or(incoming.declared_frame_rate);
    track.frame_rate_fps = track
        .frame_rate_fps
        .or(incoming.declared_frame_rate.and_then(|rate| rate.as_f64()));
    current.color.primaries = incoming.color.primaries.or(current.color.primaries);
    current.color.transfer = incoming.color.transfer.or(current.color.transfer);
    current.color.matrix = incoming.color.matrix.or(current.color.matrix);
    current.color.full_range = incoming.color.full_range;
    current.color.provenance = Provenance::Bitstream;
    track.color_transfer = current.color.transfer;
}

fn metadata(track: &mut RawTrack, data: &[u8]) -> Option<()> {
    let mut at = 0;
    let kind = leb128(data, &mut at)?;
    let mut bits = Bits {
        data: data.get(at..)?,
        at: 0,
    };
    match kind {
        1 => {
            let light = ContentLight {
                max_cll: Some(bits.read(16)?),
                max_fall: Some(bits.read(16)?),
            };
            bits.trailing()?;
            track.metadata.color.content_light = Some(light);
        }
        2 => {
            let mut xy = [0.0; 8];
            for value in &mut xy {
                *value = f64::from(bits.read(16)?) / 65536.0;
            }
            let max_luminance = f64::from(bits.read(32)?) / 256.0;
            let min_luminance = f64::from(bits.read(32)?) / 16384.0;
            bits.trailing()?;
            if max_luminance < min_luminance {
                return None;
            }
            track.metadata.color.mastering_display = Some(MasteringDisplay {
                red_x: Some(xy[0]),
                red_y: Some(xy[1]),
                green_x: Some(xy[2]),
                green_y: Some(xy[3]),
                blue_x: Some(xy[4]),
                blue_y: Some(xy[5]),
                white_x: Some(xy[6]),
                white_y: Some(xy[7]),
                min_luminance: Some(min_luminance),
                max_luminance: Some(max_luminance),
            });
        }
        4 if crate::codec::scan_itu_t35_payload_for_hdr10plus(bits.data) => {
            track.has_hdr10plus = true;
            track.metadata.hdr.hdr10plus = Some(true);
        }
        _ => {}
    }
    Some(())
}

/// Inspect bounded low-overhead OBUs from av1C configOBUs or a container sample.
pub(crate) fn enrich(track: &mut RawTrack, data: &[u8], report: &mut ProbeReport) {
    let mut at = 0;
    let mut count = 0;
    let result = (|| -> Option<()> {
        while at < data.len() {
            count += 1;
            if count > 256 || at >= 2 * 1024 * 1024 {
                report.budget_exhausted = true;
                return None;
            }
            let header = *data.get(at)?;
            at += 1;
            if header & 0x81 != 0 {
                return None;
            }
            let kind = (header >> 3) & 15;
            if header & 4 != 0 {
                let extension = *data.get(at)?;
                at += 1;
                if extension & 7 != 0 || kind == 1 || kind == 2 {
                    return None;
                }
            }
            let size = if header & 2 != 0 {
                leb128(data, &mut at)? as usize
            } else {
                data.len() - at
            };
            let end = at.checked_add(size)?;
            let payload = data.get(at..end)?;
            if matches!(kind, 1 | 5) && payload.len() > 64 * 1024 {
                report.budget_exhausted = true;
                return None;
            }
            match kind {
                1 => apply_sequence(track, sequence(payload)?, report),
                5 => {
                    metadata(track, payload)?;
                }
                _ => {}
            }
            at = end;
        }
        Some(())
    })();
    if result.is_none() {
        report.status = ProbeStatus::Incomplete;
        if !report.warnings.iter().any(|warning| {
            warning.code == "av1_enrichment_incomplete" && warning.stream_id == track.metadata.id
        }) {
            report.warnings.push(ProbeWarning {
                code: "av1_enrichment_incomplete".into(),
                message:
                    "AV1 header enrichment reached incomplete, malformed, or bounded sample data"
                        .into(),
                stream_id: track.metadata.id.clone(),
                ..Default::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn obu(kind: u8, payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 128);
        let mut bytes = vec![(kind << 3) | 2, payload.len() as u8];
        bytes.extend(payload);
        bytes
    }
    fn still_sequence(transfer: u32) -> Vec<u8> {
        let mut bits = Vec::new();
        for (value, width) in [
            (0, 3),
            (1, 1),
            (1, 1),
            (0, 5),
            (5, 4),
            (5, 4),
            (63, 6),
            (63, 6),
            (0, 3),
            (0, 3),
            (1, 1),
            (0, 1),
            (1, 1),
            (9, 8),
            (transfer, 8),
            (9, 8),
            (0, 1),
            (0, 2),
            (0, 1),
            (0, 1),
            (1, 1),
        ] {
            for bit in (0..width).rev() {
                bits.push(((value >> bit) & 1) as u8);
            }
        }
        bits.resize(bits.len().div_ceil(8) * 8, 0);
        let bytes: Vec<u8> = bits
            .chunks(8)
            .map(|chunk| chunk.iter().fold(0, |value, bit| (value << 1) | bit))
            .collect();
        obu(1, &bytes)
    }

    #[test]
    fn av1_sequence_overrides_conflicting_color_and_preserves_unknown_absence() {
        let mut track = RawTrack::default();
        track.metadata.color.transfer = Some(1);
        let mut report = ProbeReport::default();
        enrich(&mut track, &still_sequence(16), &mut report);
        assert_eq!(track.metadata.bit_depth, Some(10));
        assert_eq!(track.metadata.pixel_format.as_deref(), Some("yuv420p10le"));
        assert_eq!(track.metadata.color.transfer, Some(16));
        assert_eq!(track.metadata.color.primaries, Some(9));
        assert_eq!(track.metadata.color.full_range, Some(false));
        assert_eq!(track.metadata.hdr.hdr10plus, None);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].code, "color_signaling_conflict");
    }

    #[test]
    fn av1_static_and_dynamic_metadata_can_arrive_in_later_samples() {
        let mut track = RawTrack::default();
        let mut report = ProbeReport::default();
        enrich(&mut track, &still_sequence(16), &mut report);
        enrich(&mut track, &obu(5, &[1, 3, 232, 1, 144, 128]), &mut report);
        let mut mastering = vec![2];
        for value in [32768_u16, 16384, 8192, 4096, 2048, 1024, 16384, 16384] {
            mastering.extend(value.to_be_bytes());
        }
        mastering.extend(256_000_u32.to_be_bytes());
        mastering.extend(8192_u32.to_be_bytes());
        mastering.push(128);
        enrich(&mut track, &obu(5, &mastering), &mut report);
        enrich(
            &mut track,
            &obu(5, &[4, 0xb5, 0, 0x3c, 0, 1, 4, 1, 128]),
            &mut report,
        );
        assert_eq!(
            track.metadata.color.content_light.as_ref().unwrap().max_cll,
            Some(1000)
        );
        let display = track.metadata.color.mastering_display.unwrap();
        assert_eq!(display.red_x, Some(0.5));
        assert_eq!(display.max_luminance, Some(1000.0));
        assert_eq!(display.min_luminance, Some(0.5));
        assert_eq!(track.metadata.hdr.hdr10plus, Some(true));
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn av1_truncated_sequences_do_not_publish_partial_color() {
        let bytes = still_sequence(18);
        for len in 1..bytes.len() {
            let mut track = RawTrack::default();
            let mut report = ProbeReport::default();
            enrich(&mut track, &bytes[..len], &mut report);
            assert_eq!(track.metadata.color.transfer, None);
            assert_eq!(report.status, ProbeStatus::Incomplete);
        }
        let mut report = ProbeReport::default();
        enrich(
            &mut RawTrack::default(),
            &[10, 255, 255, 255, 255, 255, 255, 255, 255],
            &mut report,
        );
        assert_eq!(report.status, ProbeStatus::Incomplete);
        let mut report = ProbeReport::default();
        enrich(&mut RawTrack::default(), &[18, 0].repeat(257), &mut report);
        assert!(report.budget_exhausted);
    }
}
