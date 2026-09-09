//! MPEG program-stream probing with bounded PES discovery and timestamp sampling.

use crate::{
    MediaInfoError,
    source::MediaSource,
    types::{RawContainer, RawTrack, TrackKind},
};
use scryer_media_types::{AnalysisDetails, ProbeReport, ProbeStatus, Provenance};
use std::collections::BTreeMap;
use std::io::SeekFrom;

const PREFIX_BYTES: u64 = 8 * 1024 * 1024;
const TAIL_BYTES: u64 = 2 * 1024 * 1024;
const STREAM_BYTES: usize = 64 * 1024;

#[derive(Default)]
struct PesStream {
    discovery_order: usize,
    first_pts: Option<u64>,
    last_pts: Option<u64>,
    invalid_timestamps: bool,
    payload: Vec<u8>,
}

pub(crate) fn parse_ps(source: &mut dyn MediaSource) -> Result<RawContainer, MediaInfoError> {
    let length = source.len();
    let mut prefix = vec![0; length.min(PREFIX_BYTES) as usize];
    source.seek(SeekFrom::Start(0))?;
    source.read_exact(&mut prefix)?;
    let mut streams = BTreeMap::new();
    scan_pes(&prefix, &mut streams, true);
    let tail_start = length.saturating_sub(TAIL_BYTES).max(prefix.len() as u64);
    if tail_start < length {
        source.seek(SeekFrom::Start(tail_start))?;
        let mut tail = vec![0; (length - tail_start) as usize];
        source.read_exact(&mut tail)?;
        scan_pes(&tail, &mut streams, false);
    }
    let mut tracks = Vec::new();
    let mut report = ProbeReport {
        status: ProbeStatus::Incomplete,
        bytes_read: prefix.len() as u64 + length - tail_start,
        ..Default::default()
    };
    let mut caption_services = Vec::new();
    let mut streams = streams.into_iter().collect::<Vec<_>>();
    streams.sort_by_key(|(_, stream)| stream.discovery_order);
    for (id, pes) in streams {
        let Some((kind, codec)) = classify(id) else {
            continue;
        };
        let mut track = RawTrack {
            kind,
            codec_id: format!("0x{id:04x}"),
            codec_name: Some(codec.into()),
            ..Default::default()
        };
        track.metadata.id = Some(format!("{id:04x}"));
        track.metadata.duration_seconds = pes
            .first_pts
            .zip(pes.last_pts)
            .filter(|_| !pes.invalid_timestamps)
            .and_then(|(start, end)| {
                let ticks = end.wrapping_sub(start) & ((1_u64 << 33) - 1);
                (ticks > 0 && ticks < 90_000 * 86_400).then_some(ticks as f64 / 90_000.0)
            });
        if pes.invalid_timestamps {
            report.warnings.push(scryer_media_types::ProbeWarning {
                code: "program_invalid_timestamps".into(),
                message: "Sampled PES timestamps are invalid; stream duration remains unknown"
                    .into(),
                stream_id: track.metadata.id.clone(),
                offset: None,
            });
        }
        if kind == TrackKind::Video {
            track.codec_name = identify_mpeg_video(&pes.payload).map(str::to_owned);
        }
        crate::ts::probe_elementary_stream(&pes.payload, &mut track);
        if (0xc0..=0xdf).contains(&id) && track.metadata.sample_rate.is_none() {
            track.codec_name = None;
        }
        if track.codec_name.is_none() {
            report.warnings.push(scryer_media_types::ProbeWarning {
                code: "program_codec_unknown".into(),
                message: "Bounded PES sampling did not identify the elementary codec".into(),
                stream_id: track.metadata.id.clone(),
                offset: None,
            });
        }
        if kind == TrackKind::Video {
            crate::video_metadata::annex_b(
                &pes.payload,
                &mut track,
                &mut report,
                &mut caption_services,
            );
        }
        // PTS identifies the start of the final picture, not its end.
        if track.kind == TrackKind::Video
            && let Some(fps) = track.frame_rate_fps.filter(|fps| *fps > 0.0)
        {
            track.metadata.duration_seconds = track
                .metadata
                .duration_seconds
                .map(|duration| duration + 1.0 / fps);
        }
        if codec == "pcm_dvd" && pes.payload.len() >= 3 {
            let flags = pes.payload[1];
            track.channels = Some(i32::from((flags & 7) + 1));
            track.metadata.sample_rate = Some(if flags & 0x10 != 0 { 96_000 } else { 48_000 });
            track.metadata.sample_bit_depth =
                [Some(16), Some(20), Some(24), None][usize::from(flags >> 6)];
        }
        tracks.push(track);
    }
    if tracks.is_empty() {
        return Err(MediaInfoError::Parse(
            "no supported PES streams in bounded program-stream probe".into(),
        ));
    }
    let duration_seconds = tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .and_then(|t| t.metadata.duration_seconds)
        .or_else(|| {
            tracks
                .iter()
                .filter_map(|t| t.metadata.duration_seconds)
                .reduce(f64::max)
        });
    Ok(RawContainer {
        format_name: "mpeg".into(),
        duration_seconds,
        num_chapters: None,
        tracks,
        details: AnalysisDetails {
            duration_provenance: Provenance::Observed,
            overall_bitrate_bps: duration_seconds
                .filter(|d| *d > 0.0)
                .map(|d| (length as f64 * 8.0 / d) as u64),
            report,
            caption_services,
            ..Default::default()
        },
    })
}

fn identify_mpeg_video(data: &[u8]) -> Option<&'static str> {
    let start = crate::scan::find_mpeg_start_code(data, 0xb3)? + 4;
    let header_end = crate::legacy_video::mpeg_sequence_header_end(data, start)?;
    let suffix = data.get(header_end..)?;
    let next = crate::scan::find_start_code_prefix(suffix, 0)?;
    let codec = match *suffix.get(next + 3)? {
        0xb5 => "mpeg2video",
        0 | 0xb2 | 0xb8 => "mpeg1video",
        _ => return None,
    };
    let mut candidate = RawTrack {
        kind: TrackKind::Video,
        codec_name: Some(codec.into()),
        ..Default::default()
    };
    crate::legacy_video::enrich(&mut candidate, data);
    (candidate.metadata.bit_depth == Some(8)).then_some(codec)
}

fn classify(id: u16) -> Option<(TrackKind, &'static str)> {
    match id {
        0xe0..=0xef => Some((TrackKind::Video, "unknown")),
        0xc0..=0xdf => Some((TrackKind::Audio, "mp2")),
        0xbd80..=0xbd87 => Some((TrackKind::Audio, "ac3")),
        0xbd88..=0xbd8f => Some((TrackKind::Audio, "dts")),
        0xbda0..=0xbdaf => Some((TrackKind::Audio, "pcm_dvd")),
        0xbd20..=0xbd3f => Some((TrackKind::Subtitle, "dvd_subtitle")),
        _ => None,
    }
}

fn pts(bytes: &[u8], prefix: u8) -> Option<u64> {
    let b = bytes.get(..5)?;
    if b[0] >> 4 != prefix || b[0] & 1 == 0 || b[2] & 1 == 0 || b[4] & 1 == 0 {
        return None;
    }
    Some(
        (u64::from((b[0] >> 1) & 7) << 30)
            | (u64::from(b[1]) << 22)
            | (u64::from(b[2] >> 1) << 15)
            | (u64::from(b[3]) << 7)
            | u64::from(b[4] >> 1),
    )
}

pub(crate) struct PesPayload<'a> {
    pub payload: &'a [u8],
    pub timestamp: Option<u64>,
    pub timestamps_valid: bool,
}

pub(crate) fn pes_payload(packet: &[u8]) -> Option<PesPayload<'_>> {
    let mut pos = 0;
    if packet.first()? & 0xc0 == 0x80 {
        if packet.len() < 3 || packet[0] & 0x30 != 0 {
            return None;
        }
        let end = 3 + usize::from(packet[2]);
        let header = packet.get(3..end)?;
        let flags = packet[1] >> 6;
        let timestamp = match flags {
            2 => pts(header, 2),
            3 => header
                .get(5..)
                .and_then(|dts| pts(dts, 1))
                .and_then(|_| pts(header, 3)),
            _ => None,
        };
        return Some(PesPayload {
            payload: &packet[end..],
            timestamp,
            timestamps_valid: flags == 0 || (flags >= 2 && timestamp.is_some()),
        });
    }
    while packet.get(pos) == Some(&0xff) {
        pos += 1;
    }
    if packet.get(pos)? & 0xc0 == 0x40 {
        pos += 2;
    }
    let flags = packet.get(pos)? >> 4;
    let (end, timestamp) = match flags {
        2 => (pos + 5, pts(&packet[pos..], 2)),
        3 => (
            pos + 10,
            pts(packet.get(pos + 5..)?, 1).and_then(|_| pts(&packet[pos..], 3)),
        ),
        _ if packet[pos] == 0x0f => (pos + 1, None),
        _ => return None,
    };
    Some(PesPayload {
        payload: packet.get(end..)?,
        timestamp,
        timestamps_valid: flags == 0 || timestamp.is_some(),
    })
}

fn scan_pes(bytes: &[u8], streams: &mut BTreeMap<u16, PesStream>, collect: bool) {
    let mut pos = 0;
    while let Some(found) = crate::scan::find_start_code_prefix(bytes, pos) {
        pos = found;
        if bytes.len() - pos < 6 {
            break;
        }
        let stream_id = bytes[pos + 3];
        if !matches!(stream_id, 0xbd | 0xc0..=0xef) {
            pos += 4;
            continue;
        }
        let size = usize::from(u16::from_be_bytes([bytes[pos + 4], bytes[pos + 5]]));
        let payload_start = pos + 6;
        let end = if size == 0 {
            crate::scan::find_program_start_code(bytes, payload_start).unwrap_or(bytes.len())
        } else {
            (payload_start + size).min(bytes.len())
        };
        if let Some(PesPayload {
            mut payload,
            timestamp,
            timestamps_valid,
        }) = pes_payload(&bytes[payload_start..end])
        {
            let id = if stream_id == 0xbd {
                let Some(&substream) = payload.first() else {
                    pos = end;
                    continue;
                };
                let skip = if substream >= 0x80 { 4 } else { 1 };
                let Some(rest) = payload.get(skip..) else {
                    pos = end;
                    continue;
                };
                payload = rest;
                0xbd00 | u16::from(substream)
            } else {
                u16::from(stream_id)
            };
            if classify(id).is_some() {
                let discovery_order = streams.len();
                let stream = streams.entry(id).or_insert_with(|| PesStream {
                    discovery_order,
                    ..Default::default()
                });
                stream.invalid_timestamps |= !timestamps_valid;
                if let Some(timestamp) = timestamp {
                    if collect {
                        stream.first_pts.get_or_insert(timestamp);
                    }
                    stream.last_pts = Some(timestamp);
                }
                if collect {
                    let count = STREAM_BYTES
                        .saturating_sub(stream.payload.len())
                        .min(payload.len());
                    stream.payload.extend_from_slice(&payload[..count]);
                }
            }
        }
        pos = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_pes_is_bounded_and_does_not_create_tracks() {
        let mut streams = BTreeMap::new();
        scan_pes(&[0, 0, 1, 0xe0, 0, 3, 0x80, 0x80, 255], &mut streams, true);
        assert!(streams.is_empty());
        assert!(pts(&[0; 5], 2).is_none());
    }

    #[test]
    fn pes_timestamp_fields_stay_inside_the_header_and_validate_dts() {
        let timestamp = [0x21, 0, 1, 0, 1];
        // Bytes after a zero-length optional header are payload, even when they
        // resemble PTS. Invalid timing must not become a catalog duration.
        for flags in [0x40, 0x80, 0xc0] {
            let data = [&[0x80, flags, 0][..], &timestamp].concat();
            let parsed = pes_payload(&data).unwrap();
            assert_eq!(parsed.payload, timestamp);
            assert_eq!(parsed.timestamp, None);
            assert!(!parsed.timestamps_valid);
        }
        for mpeg2 in [false, true] {
            let mut data = if mpeg2 {
                vec![0x80, 0xc0, 10]
            } else {
                vec![0xff, 0x40, 0]
            };
            let start = data.len();
            data.extend([0x31, 0, 1, 0, 1, 0x11, 0, 1, 0, 1]);
            data.extend([0xaa, 0xbb]);
            let parsed = pes_payload(&data).unwrap();
            assert_eq!(parsed.timestamp, Some(0));
            assert!(parsed.timestamps_valid);
            assert_eq!(parsed.payload, [0xaa, 0xbb]);
            for (index, value) in [
                (0, if mpeg2 { 0x21 } else { 0x30 }),
                (2, 0),
                (5, 0x31),
                (7, 0),
            ] {
                let mut invalid = data.clone();
                invalid[start + index] = value;
                let parsed = pes_payload(&invalid).unwrap();
                assert_eq!(parsed.timestamp, None);
                assert!(!parsed.timestamps_valid);
                assert_eq!(parsed.payload, [0xaa, 0xbb]);
            }
        }
    }
    #[test]
    fn invalid_sampled_timestamps_leave_catalog_duration_unknown() {
        fn packet(timestamp: [u8; 5], header_length: u8) -> Vec<u8> {
            let mut bytes = vec![0, 0, 1, 0xe0, 0, 8, 0x80, 0x80, header_length];
            bytes.extend(timestamp);
            bytes
        }
        let start = packet([0x21, 0, 1, 0, 1], 5);
        let end = packet([0x21, 0, 5, 0xbf, 0x21], 5); // 90,000 ticks.
        let valid = [start.clone(), end.clone()].concat();
        let raw = parse_ps(&mut std::io::Cursor::new(valid)).unwrap();
        assert_eq!(raw.duration_seconds, Some(1.0));
        let invalid = [start, packet([0x21, 0, 1, 0, 1], 0), end].concat();
        let raw = parse_ps(&mut std::io::Cursor::new(invalid)).unwrap();
        assert_eq!(raw.duration_seconds, None);
        assert!(
            raw.details
                .report
                .warnings
                .iter()
                .any(|warning| warning.code == "program_invalid_timestamps"
                    && warning.stream_id.as_deref() == Some("00e0"))
        );
    }

    #[test]
    fn program_video_identification_uses_sequence_syntax_and_skips_matrices() {
        for (bytes, expected) in [
            (
                &include_bytes!("../tests/media/ps_mpeg1_mp2.mpg")[..],
                "mpeg1video",
            ),
            (
                &include_bytes!("../tests/media/ps_mpeg2_mp2.vob")[..],
                "mpeg2video",
            ),
        ] {
            let mut streams = BTreeMap::new();
            scan_pes(bytes, &mut streams, true);
            let data = &streams[&0xe0].payload;
            assert_eq!(identify_mpeg_video(data), Some(expected));
            let start = data
                .windows(4)
                .position(|bytes| bytes == [0, 0, 1, 0xb3])
                .unwrap()
                + 4;
            assert_eq!(identify_mpeg_video(&data[..start + 8]), None);
            let mut matrix = [0_u8; 64];
            matrix[8..12].copy_from_slice(&[0, 0, 1, 0xb5]);
            let mut with_matrix = data.to_vec();
            with_matrix[start + 7] |= 2;
            with_matrix.splice(start + 8..start + 8, matrix);
            assert_eq!(identify_mpeg_video(&with_matrix), Some(expected));
            assert_eq!(identify_mpeg_video(&with_matrix[..start + 40]), None);
        }
        let bytes = [
            0, 0, 1, 0xe0, 0, 3, 0x80, 0, 0, 0, 0, 1, 0xc0, 0, 3, 0x80, 0, 0,
        ];
        let raw = parse_ps(&mut std::io::Cursor::new(bytes)).unwrap();
        assert!(raw.tracks.iter().all(|track| track.codec_name.is_none()));
        assert_eq!(
            raw.details
                .report
                .warnings
                .iter()
                .filter(|warning| warning.code == "program_codec_unknown")
                .count(),
            2
        );
    }

    #[test]
    fn catalog_selection_preserves_program_stream_discovery_order() {
        fn packet(id: u8, data: &[u8]) -> Vec<u8> {
            let mut bytes = vec![0, 0, 1, id];
            bytes.extend(((3 + data.len()) as u16).to_be_bytes());
            bytes.extend([0x80, 0, 0]);
            bytes.extend(data);
            bytes
        }
        let sd = [
            0, 0, 1, 0xb3, 0x2d, 0x01, 0xe0, 0x13, 0, 0, 0x60, 0, 0, 0, 1, 0, 0, 8,
        ];
        let hd = [
            0, 0, 1, 0xb3, 0x50, 0x02, 0xd0, 0x13, 0, 0, 0x60, 0, 0, 0, 1, 0, 0, 8,
        ];
        let bytes = [
            packet(0xc4, &[]),
            packet(0xe7, &sd),
            packet(0xc0, &[]),
            packet(0xe1, &hd),
            packet(0xe7, &sd),
        ]
        .concat();
        let analysis = crate::analyze_source(
            &mut std::io::Cursor::new(bytes),
            "mpg",
            crate::AnalyzeOptions {
                profile: crate::AnalysisProfile::DefaultRich,
            },
        )
        .unwrap();
        assert_eq!(
            analysis
                .details
                .streams
                .iter()
                .map(|stream| stream.metadata.id.as_deref())
                .collect::<Vec<_>>(),
            [Some("00c4"), Some("00e7"), Some("00c0"), Some("00e1")]
        );
        assert_eq!(analysis.details.selected_video_id.as_deref(), Some("00e7"));
        assert_eq!(analysis.video_width, Some(720));
        assert_eq!(analysis.video_height, Some(480));
    }

    #[test]
    fn dvd_private_stream_types_are_distinct() {
        assert_eq!(classify(0xbd80), Some((TrackKind::Audio, "ac3")));
        assert_eq!(
            classify(0xbd20),
            Some((TrackKind::Subtitle, "dvd_subtitle"))
        );
        assert_eq!(classify(0xbda0), Some((TrackKind::Audio, "pcm_dvd")));
    }
}
