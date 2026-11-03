//! Bounded Annex B AVC and MPEG sequence-header readers.
use crate::{
    MediaInfoError,
    source::MediaSource,
    types::{RawContainer, RawTrack},
};
use scryer_media_types::{AnalysisDetails, ProbeStatus};
use std::io::SeekFrom;

const LIMIT: u64 = 1024 * 1024;

pub(crate) fn sniff_h264(data: &[u8]) -> bool {
    let data = &data[..data.len().min(LIMIT as usize)];
    let Some(first) = crate::scan::find_start_code_prefix(data, 0) else {
        return false;
    };
    if data[..first].iter().any(|byte| *byte != 0) {
        return false;
    }
    let mut at = 0;
    for _ in 0..256 {
        let Some(start) = crate::scan::find_start_code_prefix(data, at) else {
            break;
        };
        let payload = start + 3;
        let end = crate::scan::find_start_code_prefix(data, payload).unwrap_or(data.len());
        at = end;
        let Some(nal) = data.get(payload..end) else {
            break;
        };
        if nal
            .first()
            .is_none_or(|byte| byte & 0x80 != 0 || !(1..=23).contains(&(byte & 31)))
        {
            return false;
        }
        if nal
            .first()
            .is_some_and(|byte| byte & 0x9f == 7 && byte & 0x60 != 0)
            && nal.len() <= 64 * 1024
        {
            let rbsp = crate::scan::h2645_unescape_rbsp(nal);
            if scuffle_h264::Sps::parse(std::io::Cursor::new(rbsp.as_ref())).is_ok() {
                return true;
            }
        }
    }
    false
}

pub(crate) fn parse_h264(source: &mut dyn MediaSource) -> Result<RawContainer, MediaInfoError> {
    parse(source, true)
}

pub(crate) fn parse_mpeg(source: &mut dyn MediaSource) -> Result<RawContainer, MediaInfoError> {
    parse(source, false)
}

fn parse(source: &mut dyn MediaSource, avc: bool) -> Result<RawContainer, MediaInfoError> {
    source.seek(SeekFrom::Start(0))?;
    let mut data = vec![0; source.len().min(LIMIT) as usize];
    source.read_exact(&mut data)?;
    let codec = if avc {
        if !sniff_h264(&data) {
            return Err(MediaInfoError::Parse(
                "AVC sequence parameter set not found".into(),
            ));
        }
        "h264"
    } else {
        let start = crate::scan::find_mpeg_start_code(&data, 0xb3)
            .ok_or_else(|| MediaInfoError::Parse("MPEG sequence header not found".into()))?
            + 4;
        let end = crate::legacy_video::mpeg_sequence_header_end(&data, start)
            .ok_or_else(|| MediaInfoError::Parse("invalid MPEG sequence header".into()))?;
        let suffix = &data[end..];
        let next = crate::scan::find_start_code_prefix(suffix, 0);
        if next.and_then(|at| suffix.get(at + 3)) == Some(&0xb5) {
            "mpeg2video"
        } else {
            "mpeg1video"
        }
    };
    let mut track = RawTrack {
        codec_id: codec.into(),
        codec_name: Some(codec.into()),
        ..Default::default()
    };
    crate::ts::probe_elementary_stream(&data, &mut track);
    if !avc && track.metadata.bit_depth != Some(8) {
        return Err(MediaInfoError::Parse(
            "invalid MPEG sequence signaling".into(),
        ));
    }
    let mut details = AnalysisDetails::default();
    details.report.status = ProbeStatus::Incomplete;
    details.report.bytes_read = data.len() as u64;
    details.report.budget_exhausted = source.len() > LIMIT;
    crate::video_metadata::annex_b(
        &data,
        &mut track,
        &mut details.report,
        &mut details.caption_services,
    );
    Ok(RawContainer {
        format_name: if avc { "h264" } else { "mpegvideo" }.into(),
        duration_seconds: None,
        num_chapters: None,
        tracks: vec![track],
        details,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn sequence_header_has_dimensions_without_invented_duration() {
        let bytes = [
            0, 0, 1, 0xb3, 0x2d, 0x01, 0xe0, 0x13, 0, 0, 0x60, 0, 0, 0, 1, 0, 0, 8,
        ];
        let result = parse_mpeg(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(result.tracks[0].width, Some(720));
        assert_eq!(result.tracks[0].height, Some(480));
        assert_eq!(result.duration_seconds, None);
    }
    #[test]
    fn truncation_and_wrong_signatures_fail() {
        for bytes in [&[][..], &[0, 0, 1, 0xb3][..], &[0xff; 64][..]] {
            assert!(parse_mpeg(&mut Cursor::new(bytes)).is_err());
            assert!(parse_h264(&mut Cursor::new(bytes)).is_err());
        }
    }
    #[test]
    fn sampling_stops_at_limit() {
        let mut data = vec![0; LIMIT as usize + 100];
        data[..5].copy_from_slice(&[0, 0, 1, 0x67, 0]);
        let mut input = Cursor::new(data);
        assert!(parse_h264(&mut input).is_err());
        assert_eq!(input.position(), LIMIT);
        assert!(!sniff_h264(&[0, 0, 1, 0x67, 0]));
    }
    #[test]
    fn baseline_sps_has_dimensions_but_no_duration_or_frame_rate() {
        let bytes = [0, 0, 1, 0x67, 66, 0, 30, 244, 5, 1, 236, 128];
        assert!(sniff_h264(&bytes));
        let parsed = parse_h264(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(parsed.tracks[0].width, Some(640));
        assert_eq!(parsed.tracks[0].height, Some(480));
        assert_eq!(parsed.tracks[0].frame_rate_fps, None);
        assert_eq!(parsed.duration_seconds, None);
    }
    #[test]
    fn annex_b_inside_a_container_is_not_raw_avc() {
        let sps = [0, 0, 1, 0x67, 66, 0, 30, 244, 5, 1, 236, 128];
        assert!(!sniff_h264(
            &[b"RIFF\x00\x00\x00\x00AVI ".as_slice(), &sps].concat()
        ));
        assert!(!sniff_h264(&[&[0, 0, 1, 0xba, 0x44][..], &sps].concat()));
        assert!(sniff_h264(&[&[0][..], &sps].concat()));
    }
}
