//! Bounded RealMedia object and MDPR metadata reader.
use crate::{
    MediaInfoError,
    source::MediaSource,
    types::{RawContainer, RawTrack, TrackKind},
};
use scryer_media_types::{AnalysisDetails, ProbeStatus, Provenance};
use std::io::SeekFrom;
const LIMIT: u64 = 1024 * 1024;
enum Descriptor {
    Track(Box<RawTrack>),
    Metadata,
    Unresolved,
}
fn u16be(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}
fn u32be(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

pub(crate) fn parse(source: &mut dyn MediaSource) -> Result<RawContainer, MediaInfoError> {
    source.seek(SeekFrom::Start(0))?;
    let mut data = vec![0; source.len().min(LIMIT) as usize];
    source.read_exact(&mut data)?;
    if !data.starts_with(b".RMF") {
        return Err(MediaInfoError::Parse("invalid RealMedia signature".into()));
    }
    let mut at = 0usize;
    let mut tracks = Vec::new();
    let mut duration = None;
    let mut budget_exhausted = false;
    let mut unsupported = false;
    for object_index in 0..1024 {
        let Some(header) = data.get(at..at.saturating_add(10)) else {
            if at < data.len() && source.len() <= LIMIT {
                return Err(MediaInfoError::Parse(
                    "truncated RealMedia object header".into(),
                ));
            }
            budget_exhausted = (at as u64) < source.len();
            break;
        };
        let size = u32be(header, 4).unwrap() as usize;
        if size < 10 {
            return Err(MediaInfoError::Parse(
                "invalid RealMedia object size".into(),
            ));
        }
        let Some(end) = at.checked_add(size) else {
            return Err(MediaInfoError::Parse(
                "RealMedia object offset overflow".into(),
            ));
        };
        if end as u64 > source.len() {
            return Err(MediaInfoError::Parse("truncated RealMedia object".into()));
        }
        if &header[..4] == b"DATA" && size < 18 {
            return Err(MediaInfoError::Parse(
                "truncated RealMedia data header".into(),
            ));
        }
        if u16be(header, 8) != Some(0) {
            return Err(MediaInfoError::UnsupportedFormat(
                "RealMedia object version".into(),
            ));
        }
        if &header[..4] == b"DATA" {
            break;
        }
        let Some(body) = data.get(at + 10..end) else {
            budget_exhausted = true;
            break;
        };
        match &header[..4] {
            b".RMF" if body.len() < 8 => {
                return Err(MediaInfoError::Parse(
                    "truncated RealMedia file header".into(),
                ));
            }
            b"PROP" => {
                if body.len() < 40 {
                    return Err(MediaInfoError::Parse(
                        "truncated RealMedia properties header".into(),
                    ));
                }
                duration = u32be(body, 20)
                    .filter(|d| *d > 0)
                    .map(|d| f64::from(d) / 1000.0)
            }
            b"MDPR" if tracks.len() < 128 => match mdpr(body)? {
                Descriptor::Track(track) => tracks.push(*track),
                Descriptor::Metadata => {}
                Descriptor::Unresolved => unsupported = true,
            },
            b"MDPR" => {
                budget_exhausted = true;
                break;
            }
            _ => {}
        }
        at = end;
        if object_index == 1023
            && (at as u64) < source.len()
            && data.get(at..at + 4) != Some(b"DATA")
        {
            budget_exhausted = true;
        }
    }
    if unsupported && !tracks.iter().any(|track| track.kind == TrackKind::Video) {
        return Err(MediaInfoError::UnsupportedFormat(
            "unresolved RealMedia stream kind".into(),
        ));
    }
    if tracks.is_empty() {
        return Err(MediaInfoError::Parse(
            "no supported RealMedia stream headers".into(),
        ));
    }
    let mut details = AnalysisDetails::default();
    details.report.status = if unsupported {
        ProbeStatus::Unsupported
    } else {
        ProbeStatus::Incomplete
    };
    details.report.bytes_read = data.len() as u64;
    details.report.budget_exhausted = budget_exhausted;
    if duration.is_some() {
        details.duration_provenance = Provenance::Container;
    }
    Ok(RawContainer {
        format_name: "rm".into(),
        duration_seconds: duration,
        num_chapters: None,
        tracks,
        details,
    })
}

fn mdpr(data: &[u8]) -> Result<Descriptor, MediaInfoError> {
    // Validate the common descriptor before classifying an unsupported codec.
    let fields = || -> Option<(&[u8], &[u8])> {
        data.get(..30)?;
        let name_length = usize::from(*data.get(30)?);
        let mime_at = 31 + name_length;
        data.get(31..mime_at)?;
        let mime_length = usize::from(*data.get(mime_at)?);
        let codec_at = mime_at + 1 + mime_length;
        let mime = data.get(mime_at + 1..codec_at)?;
        let size = u32be(data, codec_at)? as usize;
        Some((
            mime,
            data.get(codec_at + 4..(codec_at + 4).checked_add(size)?)?,
        ))
    };
    let (mime, codec) = fields()
        .ok_or_else(|| MediaInfoError::Parse("malformed RealMedia stream descriptor".into()))?;
    if codec.get(4..8) == Some(b"VIDO") {
        let known = matches!(
            codec.get(8..12),
            Some(b"RV10" | b"RV20" | b"RV30" | b"RV40")
        );
        if codec.len() < 12 || (known && codec.len() < 26) {
            return Err(MediaInfoError::Parse("truncated RealVideo header".into()));
        }
    } else if mime == b"audio/x-pn-realaudio" && codec.starts_with(b".ra\xfd") {
        match u16be(codec, 4) {
            Some(3) if codec.len() >= 24 => {}
            Some(4 | 5) => {}
            Some(3) | None => {
                return Err(MediaInfoError::Parse("truncated RealAudio header".into()));
            }
            Some(_) => return Ok(Descriptor::Unresolved),
        }
    } else if mime == b"logical-fileinfo" {
        return Ok(Descriptor::Metadata);
    } else {
        return Ok(Descriptor::Unresolved);
    }
    let track = decode_mdpr(data)
        .ok_or_else(|| MediaInfoError::Parse("malformed RealMedia codec header".into()))?;
    Ok(Descriptor::Track(Box::new(track)))
}

fn decode_mdpr(data: &[u8]) -> Option<RawTrack> {
    let id = u16be(data, 0)?;
    let bitrate = u32be(data, 6)?;
    let duration = u32be(data, 26)?;
    let mut at = 30;
    let name_len = usize::from(*data.get(at)?);
    at += 1;
    let name = data.get(at..at + name_len)?;
    at += name_len;
    let mime_len = usize::from(*data.get(at)?);
    at += 1;
    let mime = data.get(at..at + mime_len)?;
    at += mime_len;
    let size = u32be(data, at)? as usize;
    at += 4;
    let codec = data.get(at..at.checked_add(size)?)?;
    let mut track = RawTrack::default();
    if codec.get(4..8) == Some(b"VIDO") {
        let tag = codec.get(8..12)?;
        track.codec_name = match tag {
            b"RV10" => Some("rv10"),
            b"RV20" => Some("rv20"),
            b"RV30" => Some("rv30"),
            b"RV40" => Some("rv40"),
            _ => None,
        }
        .map(str::to_owned);
        track.codec_id = String::from_utf8_lossy(tag).into_owned();
        if track.codec_name.is_some() {
            track.width = u16be(codec, 12).filter(|v| *v > 0).map(i32::from);
            track.height = u16be(codec, 14).filter(|v| *v > 0).map(i32::from);
            track.frame_rate_fps = u32be(codec, 22)
                .filter(|v| *v > 0)
                .map(|v| f64::from(v) / 65536.0);
        }
    } else if mime == b"audio/x-pn-realaudio" && codec.starts_with(b".ra\xfd") {
        track.kind = TrackKind::Audio;
        let version = u16be(codec, 4)?;
        if version == 3 {
            track.codec_name = Some("ra_144".into());
            track.channels = Some(1);
            track.metadata.sample_rate = Some(8000);
        } else if version == 4 || version == 5 {
            let offset = if version == 5 { 54 } else { 48 };
            track.metadata.sample_rate = u16be(codec, offset).filter(|v| *v > 0).map(u32::from);
            track.channels = u16be(codec, offset + 6).filter(|v| *v > 0).map(i32::from);
            let tag = if version == 5 {
                codec.get(offset + 12..offset + 16)?
            } else {
                let first = offset + 8;
                let n = usize::from(*codec.get(first)?);
                let second = first + 1 + n;
                let n = usize::from(*codec.get(second)?);
                codec.get(second + 1..second + 1 + n)?
            };
            track.codec_id = String::from_utf8_lossy(tag).into_owned();
            track.codec_name = match tag {
                b"cook" => Some("cook"),
                b"sipr" => Some("sipr"),
                b"atrc" => Some("atrac3"),
                b"dnet" => Some("ac3"),
                b"28_8" => Some("ra_288"),
                b"raac" | b"racp" => Some("aac"),
                _ => None,
            }
            .map(str::to_owned);
        }
    } else {
        return None;
    }
    track.metadata.id = Some(id.to_string());
    track.metadata.duration_seconds = (duration > 0).then_some(f64::from(duration) / 1000.0);
    track.bit_rate_bps = (bitrate > 0).then_some(i64::from(bitrate));
    track.name = (!name.is_empty()).then(|| String::from_utf8_lossy(name).into_owned());
    Some(track)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn object(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = tag.to_vec();
        v.extend(((body.len() + 10) as u32).to_be_bytes());
        v.extend([0, 0]);
        v.extend(body);
        v
    }
    fn fixture() -> Vec<u8> {
        let mut codec = vec![0; 26];
        codec[4..12].copy_from_slice(b"VIDORV40");
        codec[12..14].copy_from_slice(&640u16.to_be_bytes());
        codec[14..16].copy_from_slice(&480u16.to_be_bytes());
        let mut body = vec![0; 30];
        body.extend([0, 0]);
        body.extend((codec.len() as u32).to_be_bytes());
        body.extend(codec);
        [object(b".RMF", &[0; 8]), object(b"MDPR", &body)].concat()
    }
    #[test]
    fn video_metadata_and_unknown_timing() {
        let r = parse(&mut Cursor::new(fixture())).unwrap();
        assert_eq!(r.tracks[0].codec_name.as_deref(), Some("rv40"));
        assert_eq!(r.tracks[0].width, Some(640));
        assert_eq!(r.duration_seconds, None);
        assert_eq!(r.tracks[0].frame_rate_fps, None);
    }
    #[test]
    fn object_sizes_never_escape_input() {
        let mut bytes = fixture();
        bytes[22..26].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse(&mut Cursor::new(bytes)).is_err());
        for n in 0..50 {
            assert!(parse(&mut Cursor::new(&fixture()[..n])).is_err());
        }
    }
    #[test]
    fn unsupported_version_is_distinct_from_malformed_size() {
        let mut bytes = fixture();
        bytes[8..10].copy_from_slice(&2u16.to_be_bytes());
        assert!(matches!(
            parse(&mut Cursor::new(bytes)),
            Err(MediaInfoError::UnsupportedFormat(_))
        ));
    }
    #[test]
    fn malformed_second_descriptor_is_not_silently_dropped() {
        let mut bytes = fixture();
        bytes.extend(object(b"MDPR", &[0; 32]));
        assert!(matches!(
            parse(&mut Cursor::new(bytes)),
            Err(MediaInfoError::Parse(_))
        ));
    }
    #[test]
    fn unknown_video_codec_retains_video_identity_and_unknown_fields() {
        let mut bytes = fixture();
        let end = bytes.len();
        bytes[end - 18..end - 14].copy_from_slice(b"RV99");
        bytes.extend(audio_descriptor());
        let parsed = parse(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(parsed.tracks.len(), 2);
        assert_eq!(parsed.tracks[0].kind, TrackKind::Video);
        assert_eq!(parsed.tracks[0].codec_id, "RV99");
        assert_eq!(parsed.tracks[0].codec_name, None);
        assert_eq!(parsed.tracks[0].width, None);
        assert_eq!(parsed.tracks[0].height, None);
        assert_eq!(parsed.tracks[1].kind, TrackKind::Audio);
        assert_eq!(parsed.tracks[1].codec_name.as_deref(), Some("ra_144"));
    }
    fn descriptor(codec: &[u8], mime: &[u8]) -> Vec<u8> {
        let mut body = vec![0; 30];
        body[1] = 1;
        body.push(0);
        body.push(mime.len() as u8);
        body.extend(mime);
        body.extend((codec.len() as u32).to_be_bytes());
        body.extend(codec);
        object(b"MDPR", &body)
    }
    fn audio_descriptor() -> Vec<u8> {
        let mut codec = vec![0; 24];
        codec[..6].copy_from_slice(b".ra\xfd\x00\x03");
        descriptor(&codec, b"audio/x-pn-realaudio")
    }
    #[test]
    fn unresolved_multistream_does_not_establish_audio_only() {
        let bytes = [
            object(b".RMF", &[0; 8]),
            audio_descriptor(),
            descriptor(b"MLTI\x00\x00", b"video/x-pn-realvideo"),
        ]
        .concat();
        assert!(matches!(
            parse(&mut Cursor::new(bytes)),
            Err(MediaInfoError::UnsupportedFormat(_))
        ));
        let bytes = [
            fixture(),
            descriptor(b"MLTI\x00\x00", b"video/x-pn-realvideo"),
        ]
        .concat();
        let partial = parse(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(partial.details.report.status, ProbeStatus::Unsupported);
        assert_eq!(partial.tracks[0].codec_name.as_deref(), Some("rv40"));
    }
    #[test]
    fn metadata_caps_report_exhaustion_only_when_more_metadata_remains() {
        let mut bytes = fixture();
        for _ in 0..128 {
            bytes.extend(audio_descriptor());
        }
        assert!(
            parse(&mut Cursor::new(bytes))
                .unwrap()
                .details
                .report
                .budget_exhausted
        );
        let mut bytes = fixture();
        for _ in 0..1023 {
            bytes.extend(object(b"JUNK", &[]));
        }
        assert!(
            parse(&mut Cursor::new(bytes))
                .unwrap()
                .details
                .report
                .budget_exhausted
        );
        let mut bytes = fixture();
        for _ in 0..1022 {
            bytes.extend(object(b"JUNK", &[]));
        }
        bytes.extend(object(b"DATA", &[0; 8]));
        assert!(
            !parse(&mut Cursor::new(bytes))
                .unwrap()
                .details
                .report
                .budget_exhausted
        );
    }
    #[test]
    fn known_fixed_headers_require_their_complete_bodies() {
        for length in 0..8 {
            let bytes = [object(b".RMF", &vec![0; length]), fixture()[18..].to_vec()].concat();
            assert!(matches!(
                parse(&mut Cursor::new(bytes)),
                Err(MediaInfoError::Parse(_))
            ));
        }
        for length in 0..40 {
            let bytes = [fixture(), object(b"PROP", &vec![0; length])].concat();
            assert!(matches!(
                parse(&mut Cursor::new(bytes)),
                Err(MediaInfoError::Parse(_))
            ));
        }
    }
    #[test]
    fn data_payload_is_not_a_header_budget() {
        let mut bytes = fixture();
        bytes.extend(object(b"DATA", &[0; 8]));
        bytes.resize(LIMIT as usize + 10, 0);
        let parsed = parse(&mut Cursor::new(bytes)).unwrap();
        assert!(!parsed.details.report.budget_exhausted);
        assert_ne!(parsed.details.duration_provenance, Provenance::Container);
    }

    #[test]
    fn object_bounds_are_validated_before_unsupported_versions_or_data() {
        for tag in [b"DATA", b"JUNK"] {
            let mut tail = object(tag, &[0; 8]);
            tail[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
            if tag == b"JUNK" {
                tail[8..10].copy_from_slice(&2u16.to_be_bytes());
            }
            let bytes = [fixture(), tail].concat();
            assert!(matches!(
                parse(&mut Cursor::new(bytes)),
                Err(MediaInfoError::Parse(_))
            ));
        }
        for length in 0..8 {
            let bytes = [fixture(), object(b"DATA", &vec![0; length])].concat();
            assert!(matches!(
                parse(&mut Cursor::new(bytes)),
                Err(MediaInfoError::Parse(_))
            ));
        }
    }

    #[test]
    fn data_payload_can_extend_beyond_metadata_sample() {
        let mut bytes = fixture();
        bytes.extend(object(b"DATA", &vec![0; LIMIT as usize]));
        let parsed = parse(&mut Cursor::new(bytes)).unwrap();
        assert!(!parsed.details.report.budget_exhausted);
        assert_eq!(parsed.tracks[0].codec_name.as_deref(), Some("rv40"));
    }
}
