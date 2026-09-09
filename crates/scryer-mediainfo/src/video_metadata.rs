//! Bounded SEI and MPEG user-data inventory; no picture or caption decoding.
use crate::types::RawTrack;
use scryer_media_types::{
    CaptionService, ContentLight, MasteringDisplay, ProbeReport, ProbeStatus, ProbeWarning,
};

fn incomplete(track: &RawTrack, report: &mut ProbeReport, budget: bool) {
    report.status = ProbeStatus::Incomplete;
    report.budget_exhausted |= budget;
    if !report.warnings.iter().any(|warning| {
        warning.code == "video_metadata_incomplete" && warning.stream_id == track.metadata.id
    }) {
        report.warnings.push(ProbeWarning { code: "video_metadata_incomplete".into(), message: "Video metadata sampling encountered an incomplete header or exhausted its bounded inventory".into(), stream_id: track.metadata.id.clone(), ..Default::default() });
    }
}

fn service(
    services: &mut Vec<CaptionService>,
    track: &RawTrack,
    standard: &str,
    number: Option<u8>,
    report: &mut ProbeReport,
) {
    let entry = CaptionService {
        stream_id: track.metadata.id.clone().unwrap_or_default(),
        standard: standard.into(),
        service_number: number,
        language: None,
    };
    if services.contains(&entry) {
        return;
    }
    if services.len() >= 256 {
        incomplete(track, report, true);
        return;
    }
    services.push(entry);
}

fn digital_services(
    packet: &[u8],
    track: &RawTrack,
    services: &mut Vec<CaptionService>,
    report: &mut ProbeReport,
) -> Option<()> {
    let mut at = 0;
    while at < packet.len() {
        let header = packet[at];
        at += 1;
        let size = usize::from(header & 31);
        let mut number = header >> 5;
        if number == 7 {
            number = *packet.get(at)? & 63;
            at += 1;
            if number < 7 {
                return None;
            }
        }
        if number == 0 {
            if size != 0 {
                return None;
            }
            continue;
        }
        at = at.checked_add(size)?;
        if at > packet.len() {
            return None;
        }
        if size > 0 {
            service(services, track, "CEA-708", Some(number), report);
        }
    }
    Some(())
}

/// GA94 user data starts with its type code, followed by bounded CC triplets.
fn captions(
    data: &[u8],
    track: &RawTrack,
    report: &mut ProbeReport,
    services: &mut Vec<CaptionService>,
) {
    if data.first() != Some(&3) || data.get(1).is_none_or(|flags| flags & 0x40 == 0) {
        return;
    }
    let count = usize::from(data[1] & 31);
    let end = 3 + count * 3;
    if data.get(end) != Some(&255) {
        incomplete(track, report, false);
        return;
    }
    let mut digital = Vec::with_capacity(128);
    let mut expected = None;
    for triplet in data[3..end].chunks_exact(3) {
        if triplet[0] & 0xf8 != 0xf8 || triplet[0] & 4 == 0 {
            continue;
        }
        match triplet[0] & 3 {
            field @ (0 | 1) => {
                if triplet[1].count_ones() % 2 != 1 || triplet[2].count_ones() % 2 != 1 {
                    continue;
                }
                let first = triplet[1] & 127;
                if first == 0 && triplet[2] & 127 == 0 {
                    continue;
                }
                let channel = match first {
                    0x14 => Some(1 + field * 2),
                    0x1c => Some(2 + field * 2),
                    _ => None,
                };
                service(services, track, "CEA-608", channel, report);
            }
            kind => {
                if kind == 3 {
                    digital.clear();
                    let size = usize::from(triplet[1] & 63);
                    expected = Some(if size == 0 { 127 } else { size * 2 - 1 });
                    digital.push(triplet[2]);
                } else if expected.is_some() && digital.len() < 127 {
                    digital.extend_from_slice(&triplet[1..]);
                }
                if let Some(length) = expected {
                    if digital.len() >= length {
                        if digital_services(&digital[..length], track, services, report).is_none() {
                            incomplete(track, report, false);
                        }
                        expected = None;
                        digital.clear();
                    }
                } else if kind == 2 {
                    service(services, track, "CEA-708", None, report);
                }
            }
        }
    }
    if expected.is_some() {
        service(services, track, "CEA-708", None, report);
        incomplete(track, report, false);
    }
}

fn t35(
    data: &[u8],
    track: &mut RawTrack,
    report: &mut ProbeReport,
    services: &mut Vec<CaptionService>,
) {
    if crate::codec::scan_itu_t35_payload_for_hdr10plus(data) {
        track.has_hdr10plus = true;
    }
    if data.starts_with(&[181, 0, 49, b'G', b'A', b'9', b'4']) {
        captions(&data[7..], track, report, services);
    }
}

fn sei(
    nal: &[u8],
    header_size: usize,
    track: &mut RawTrack,
    report: &mut ProbeReport,
    services: &mut Vec<CaptionService>,
) {
    if nal.len() > 64 * 1024 {
        incomplete(track, report, true);
        return;
    }
    let Some(payload) = nal.get(header_size..) else {
        incomplete(track, report, false);
        return;
    };
    let rbsp = crate::scan::h2645_unescape_rbsp(payload);
    let bytes = rbsp.as_ref();
    let mut at = 0;
    for _ in 0..256 {
        if at == bytes.len()
            || bytes.get(at) == Some(&128) && bytes[at + 1..].iter().all(|byte| *byte == 0)
        {
            return;
        }
        let mut field = || {
            let mut value = 0_usize;
            loop {
                let byte = *bytes.get(at)?;
                at += 1;
                value = value.checked_add(usize::from(byte))?;
                if byte != 255 {
                    return Some(value);
                }
            }
        };
        let Some((kind, size)) = field().zip(field()) else {
            incomplete(track, report, false);
            return;
        };
        let Some(end) = at.checked_add(size).filter(|end| *end <= bytes.len()) else {
            incomplete(track, report, false);
            return;
        };
        let message = &bytes[at..end];
        match kind {
            4 => t35(message, track, report, services),
            137 if message.len() == 24 => {
                let word = |at| u16::from_be_bytes([message[at], message[at + 1]]);
                let dword = |at| u32::from_be_bytes(message[at..at + 4].try_into().unwrap());
                if (0..8).any(|index| word(index * 2) > 50_000) || dword(16) < dword(20) {
                    incomplete(track, report, false);
                } else {
                    track.metadata.color.mastering_display = Some(MasteringDisplay {
                        green_x: Some(f64::from(word(0)) / 50_000.0),
                        green_y: Some(f64::from(word(2)) / 50_000.0),
                        blue_x: Some(f64::from(word(4)) / 50_000.0),
                        blue_y: Some(f64::from(word(6)) / 50_000.0),
                        red_x: Some(f64::from(word(8)) / 50_000.0),
                        red_y: Some(f64::from(word(10)) / 50_000.0),
                        white_x: Some(f64::from(word(12)) / 50_000.0),
                        white_y: Some(f64::from(word(14)) / 50_000.0),
                        max_luminance: Some(f64::from(dword(16)) / 10_000.0),
                        min_luminance: Some(f64::from(dword(20)) / 10_000.0),
                    });
                }
            }
            144 if message.len() == 4 => {
                track.metadata.color.content_light = Some(ContentLight {
                    max_cll: Some(u32::from(u16::from_be_bytes([message[0], message[1]]))),
                    max_fall: Some(u32::from(u16::from_be_bytes([message[2], message[3]]))),
                })
            }
            137 | 144 => incomplete(track, report, false),
            _ => {}
        }
        at = end;
    }
    if at < bytes.len() {
        incomplete(track, report, true);
    }
}

fn nal(
    data: &[u8],
    track: &mut RawTrack,
    report: &mut ProbeReport,
    services: &mut Vec<CaptionService>,
) {
    match track.codec_name.as_deref() {
        Some("h264") if data.first().is_some_and(|byte| byte & 0x9f == 6) => {
            sei(data, 1, track, report, services)
        }
        Some("hevc")
            if data.len() >= 2
                && data[0] & 128 == 0
                && data[1] & 7 != 0
                && matches!((data[0] >> 1) & 63, 39 | 40) =>
        {
            sei(data, 2, track, report, services)
        }
        Some("mpeg1video" | "mpeg2video") if data.starts_with(&[0xb2, b'G', b'A', b'9', b'4']) => {
            captions(&data[5..], track, report, services)
        }
        _ => {}
    }
}

pub(crate) fn length_prefixed(
    data: &[u8],
    length_size: usize,
    track: &mut RawTrack,
    report: &mut ProbeReport,
    services: &mut Vec<CaptionService>,
) {
    if !(1..=4).contains(&length_size) {
        incomplete(track, report, false);
        return;
    }
    let mut at = 0;
    for _ in 0..256 {
        if at == data.len() {
            return;
        }
        let Some(prefix) = data.get(at..at + length_size) else {
            incomplete(track, report, false);
            return;
        };
        let size = prefix
            .iter()
            .fold(0_u64, |size, byte| (size << 8) | u64::from(*byte));
        at += length_size;
        let Some(end) = usize::try_from(size)
            .ok()
            .and_then(|size| at.checked_add(size))
            .filter(|end| *end <= data.len() && *end > at)
        else {
            incomplete(track, report, false);
            return;
        };
        nal(&data[at..end], track, report, services);
        at = end;
    }
    if at < data.len() {
        incomplete(track, report, true);
    }
}

pub(crate) fn annex_b(
    data: &[u8],
    track: &mut RawTrack,
    report: &mut ProbeReport,
    services: &mut Vec<CaptionService>,
) {
    let mut start = None;
    let mut count = 0;
    let mut cursor = 0;
    while let Some(at) = crate::scan::find_start_code_prefix(data, cursor) {
        if let Some(start) = start {
            nal(&data[start..at], track, report, services);
        }
        cursor = at + 3;
        start = Some(cursor);
        count += 1;
        if count >= 256 {
            incomplete(track, report, true);
            return;
        }
    }
    if let Some(start) = start {
        nal(&data[start..], track, report, services);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn caption_payload() -> Vec<u8> {
        vec![
            181, 0, 49, b'G', b'A', b'9', b'4', 3, 0x43, 0, 0xfc, 0x94, 0x2c, 0xff, 2, 0x21, 0xfe,
            0x41, 0, 255,
        ]
    }
    fn sei_nal(hevc: bool, messages: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let mut rbsp = Vec::new();
        for (kind, payload) in messages {
            rbsp.extend([*kind, payload.len() as u8]);
            rbsp.extend(payload);
        }
        rbsp.push(128);
        let mut nal = if hevc { vec![78, 1] } else { vec![6] };
        let mut zeros = 0;
        for byte in rbsp {
            if zeros >= 2 && byte <= 3 {
                nal.push(3);
                zeros = 0;
            }
            nal.push(byte);
            zeros = if byte == 0 { zeros + 1 } else { 0 };
        }
        nal
    }

    #[test]
    fn accelerated_annex_b_preserves_metadata_boundaries_and_budget() {
        fn reference(
            data: &[u8],
            track: &mut RawTrack,
            report: &mut ProbeReport,
            services: &mut Vec<CaptionService>,
        ) {
            let mut start = None;
            let mut count = 0;
            for (at, bytes) in data.windows(3).enumerate() {
                if bytes == [0, 0, 1] {
                    if let Some(start) = start {
                        nal(&data[start..at], track, report, services);
                    }
                    start = Some(at + 3);
                    count += 1;
                    if count >= 256 {
                        incomplete(track, report, true);
                        return;
                    }
                }
            }
            if let Some(start) = start {
                nal(&data[start..], track, report, services);
            }
        }
        for codec in ["h264", "hevc", "mpeg2video"] {
            let payload = caption_payload();
            let body = if codec == "mpeg2video" {
                [vec![0xb2], payload[3..].to_vec()].concat()
            } else {
                sei_nal(codec == "hevc", &[(4, payload)])
            };
            for count in [0, 1, 2, 255, 256, 257] {
                let mut data = vec![0x55; 31];
                for index in 0..count {
                    data.extend(std::iter::repeat_n(0, 2 + index % 3));
                    data.push(1);
                    data.extend_from_slice(&body);
                }
                data.extend_from_slice(&[0, 0, 1, 0]);
                for truncate in 0..=4 {
                    let data = &data[..data.len() - truncate];
                    let mut values = Vec::new();
                    for parser in [reference, annex_b] {
                        let mut track = RawTrack {
                            codec_name: Some(codec.into()),
                            ..Default::default()
                        };
                        let mut report = ProbeReport::default();
                        let mut services = Vec::new();
                        parser(data, &mut track, &mut report, &mut services);
                        values.push(serde_json::json!({"track": track.metadata, "report": report, "services": services}));
                    }
                    assert_eq!(
                        values[0], values[1],
                        "codec={codec} count={count} truncate={truncate}"
                    );
                }
            }
        }
    }

    #[test]
    fn cea_services_are_inventoried_from_avc_hevc_and_mpeg_user_data() {
        for codec in ["h264", "hevc", "mpeg2video"] {
            let mut track = RawTrack {
                codec_name: Some(codec.into()),
                ..Default::default()
            };
            track.metadata.id = Some("video-7".into());
            let mut report = ProbeReport::default();
            let mut services = Vec::new();
            let payload = caption_payload();
            let body = if codec == "mpeg2video" {
                let mut body = vec![0xb2];
                body.extend_from_slice(&payload[3..]);
                body
            } else {
                sei_nal(codec == "hevc", &[(4, payload)])
            };
            let mut data = vec![0, 0, 1];
            data.extend(body);
            annex_b(&data, &mut track, &mut report, &mut services);
            assert!(
                services
                    .iter()
                    .any(|service| service.standard == "CEA-608"
                        && service.service_number == Some(1)),
                "{codec}: {services:?}"
            );
            assert!(
                services
                    .iter()
                    .any(|service| service.standard == "CEA-708"
                        && service.service_number == Some(1)),
                "{codec}: {services:?}"
            );
            assert!(
                services
                    .iter()
                    .all(|service| service.stream_id == "video-7" && service.language.is_none())
            );
            assert!(report.warnings.is_empty(), "{codec}: {report:?}");
        }
    }

    #[test]
    fn sei_mastering_and_content_light_units_are_not_av1_units() {
        let mut mastering = Vec::new();
        for value in [13250_u16, 34500, 7500, 3000, 34000, 16000, 15635, 16450] {
            mastering.extend(value.to_be_bytes());
        }
        mastering.extend(10_000_000_u32.to_be_bytes());
        mastering.extend(50_u32.to_be_bytes());
        let nal = sei_nal(true, &[(137, mastering), (144, vec![3, 232, 1, 144])]);
        let mut data = (nal.len() as u32).to_be_bytes().to_vec();
        data.extend(nal);
        let mut track = RawTrack {
            codec_name: Some("hevc".into()),
            ..Default::default()
        };
        let mut report = ProbeReport::default();
        length_prefixed(&data, 4, &mut track, &mut report, &mut Vec::new());
        let display = track.metadata.color.mastering_display.unwrap();
        assert_eq!(display.red_x, Some(0.68));
        assert_eq!(display.green_y, Some(0.69));
        assert_eq!(display.max_luminance, Some(1000.0));
        assert_eq!(display.min_luminance, Some(0.005));
        assert_eq!(
            track.metadata.color.content_light.unwrap().max_fall,
            Some(400)
        );
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn caption_inventory_exhaustion_is_observable_without_duplicate_warnings() {
        let mut services = Vec::new();
        let mut report = ProbeReport::default();
        for id in 0..256 {
            let mut track = RawTrack::default();
            track.metadata.id = Some(id.to_string());
            service(&mut services, &track, "CEA-708", Some(1), &mut report);
        }
        let mut track = RawTrack::default();
        track.metadata.id = Some("255".into());
        service(&mut services, &track, "CEA-708", Some(1), &mut report);
        assert!(report.warnings.is_empty());
        for _ in 0..2 {
            service(&mut services, &track, "CEA-708", Some(2), &mut report);
        }
        assert_eq!(services.len(), 256);
        assert!(report.budget_exhausted);
        assert_eq!(report.status, ProbeStatus::Incomplete);
        assert_eq!(report.warnings.len(), 1);
    }

    #[test]
    fn malformed_caption_lengths_and_nal_lengths_remain_incomplete() {
        let mut track = RawTrack {
            codec_name: Some("h264".into()),
            ..Default::default()
        };
        let mut report = ProbeReport::default();
        let mut services = Vec::new();
        let mut payload = caption_payload();
        payload.pop();
        t35(&payload, &mut track, &mut report, &mut services);
        assert!(services.is_empty());
        assert_eq!(report.status, ProbeStatus::Incomplete);
        let mut report = ProbeReport::default();
        length_prefixed(
            &[255, 255, 255, 255, 6],
            4,
            &mut track,
            &mut report,
            &mut services,
        );
        assert_eq!(report.status, ProbeStatus::Incomplete);
    }
}
