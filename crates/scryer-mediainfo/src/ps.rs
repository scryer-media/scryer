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
        track.metadata.duration_seconds =
            pes.first_pts.zip(pes.last_pts).and_then(|(start, end)| {
                let ticks = end.wrapping_sub(start) & ((1_u64 << 33) - 1);
                (ticks > 0 && ticks < 90_000 * 86_400).then_some(ticks as f64 / 90_000.0)
            });
        crate::ts::probe_elementary_stream(&pes.payload, &mut track);
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

fn classify(id: u16) -> Option<(TrackKind, &'static str)> {
    match id {
        0xe0..=0xef => Some((TrackKind::Video, "mpeg2video")),
        0xc0..=0xdf => Some((TrackKind::Audio, "mp2")),
        0xbd80..=0xbd87 => Some((TrackKind::Audio, "ac3")),
        0xbd88..=0xbd8f => Some((TrackKind::Audio, "dts")),
        0xbda0..=0xbdaf => Some((TrackKind::Audio, "pcm_dvd")),
        0xbd20..=0xbd3f => Some((TrackKind::Subtitle, "dvd_subtitle")),
        _ => None,
    }
}

fn pts(bytes: &[u8]) -> Option<u64> {
    let b = bytes.get(..5)?;
    if b[0] & 1 == 0 || b[2] & 1 == 0 || b[4] & 1 == 0 {
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

pub(crate) fn pes_payload(packet: &[u8]) -> Option<(&[u8], Option<u64>)> {
    let mut pos = 0;
    if packet.first()? & 0xc0 == 0x80 {
        if packet.len() < 3 || packet[0] & 0x30 != 0 {
            return None;
        }
        let timestamp = (packet[1] & 0x80 != 0)
            .then(|| pts(packet.get(3..)?))
            .flatten();
        return Some((packet.get(3 + usize::from(packet[2])..)?, timestamp));
    }
    while packet.get(pos) == Some(&0xff) {
        pos += 1;
    }
    if packet.get(pos)? & 0xc0 == 0x40 {
        pos += 2;
    }
    match packet.get(pos)? >> 4 {
        2 => Some((packet.get(pos + 5..)?, pts(&packet[pos..]))),
        3 => Some((packet.get(pos + 10..)?, pts(&packet[pos..]))),
        _ if packet[pos] == 0x0f => Some((packet.get(pos + 1..)?, None)),
        _ => None,
    }
}

fn scan_pes(bytes: &[u8], streams: &mut BTreeMap<u16, PesStream>, collect: bool) {
    let mut pos = 0;
    while pos + 6 <= bytes.len() {
        if bytes[pos..pos + 3] != [0, 0, 1] {
            pos += 1;
            continue;
        }
        let stream_id = bytes[pos + 3];
        if !matches!(stream_id, 0xbd | 0xc0..=0xef) {
            pos += 4;
            continue;
        }
        let size = usize::from(u16::from_be_bytes([bytes[pos + 4], bytes[pos + 5]]));
        let payload_start = pos + 6;
        let end = if size == 0 {
            bytes[payload_start..]
                .windows(4)
                .position(|b| b[..3] == [0, 0, 1] && b[3] >= 0xb9)
                .map_or(bytes.len(), |offset| payload_start + offset)
        } else {
            (payload_start + size).min(bytes.len())
        };
        if let Some((mut payload, timestamp)) = pes_payload(&bytes[payload_start..end]) {
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
        assert!(pts(&[0; 5]).is_none());
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
        let sd = [0, 0, 1, 0xb3, 0x2d, 0x01, 0xe0, 0x13, 0, 0, 0x60, 0];
        let hd = [0, 0, 1, 0xb3, 0x50, 0x02, 0xd0, 0x13, 0, 0, 0x60, 0];
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
