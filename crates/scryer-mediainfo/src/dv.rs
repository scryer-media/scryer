//! Read DV DIF headers without decoding pictures or audio.
use crate::{
    MediaInfoError,
    source::MediaSource,
    types::{RawContainer, RawTrack},
};
use scryer_media_types::{AnalysisDetails, ProbeStatus};
use std::io::SeekFrom;

pub(crate) fn sniff(data: &[u8]) -> bool {
    if data.len() < 480 {
        return false;
    }
    // Validate the six DIF IDs, including block numbers and sequence/channel
    // agreement. Payload bytes alone cannot identify a DV stream.
    for (block, section, number) in [
        (0, 0, 0),
        (1, 1, 0),
        (2, 1, 1),
        (3, 2, 0),
        (4, 2, 1),
        (5, 2, 2),
    ] {
        let at = block * 80;
        if data[at] >> 5 != section || data[at + 1] != data[1] || data[at + 2] != number {
            return false;
        }
    }
    data[0] == 0x1f && data[1] & 0xf0 == 0
}

pub(crate) fn parse(source: &mut dyn MediaSource) -> Result<RawContainer, MediaInfoError> {
    source.seek(SeekFrom::Start(0))?;
    let mut header = [0; 480];
    source.read_exact(&mut header)?;
    // DIF header, subcode and VAUX blocks must occur at their prescribed positions.
    if !sniff(&header) {
        return Err(MediaInfoError::Parse("invalid DV DIF header".into()));
    }
    let pal = header[3] & 0x80 != 0;
    let stype = header[451] & 0x1f;
    let profile = match (pal, stype) {
        (false, 0) => Some((720, 480, 30000.0 / 1001.0, "yuv411p")),
        (true, 0 | 1) => Some((
            720,
            576,
            25.0,
            if header[4] & 7 != 0 {
                "yuv411p"
            } else {
                "yuv420p"
            },
        )),
        (false, 4) => Some((720, 480, 30000.0 / 1001.0, "yuv422p")),
        (true, 4) => Some((720, 576, 25.0, "yuv422p")),
        (false, 0x14) => Some((1280, 1080, 30000.0 / 1001.0, "yuv422p")),
        (true, 0x14) => Some((1440, 1080, 25.0, "yuv422p")),
        (false, 0x18) => Some((960, 720, 60000.0 / 1001.0, "yuv422p")),
        (true, 0x18) => Some((960, 720, 50.0, "yuv422p")),
        _ => None,
    };
    let mut track = RawTrack {
        codec_id: "dvvideo".into(),
        codec_name: Some("dvvideo".into()),
        ..Default::default()
    };
    if let Some((width, height, fps, pixel)) = profile {
        track.width = Some(width);
        track.height = Some(height);
        track.frame_rate_fps = Some(fps);
        track.metadata.pixel_format = Some(pixel.into());
        track.metadata.bit_depth = Some(8);
    }
    let mut details = AnalysisDetails::default();
    details.report.status = ProbeStatus::Incomplete;
    details.report.bytes_read = 480;
    // The first frame does not prove that the file keeps one profile, or has
    // complete frames. File size is deliberately not converted into duration.
    Ok(RawContainer {
        format_name: "dv".into(),
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
    fn header() -> [u8; 480] {
        let mut data = [0; 480];
        data[0] = 0x1f;
        data[80] = 0x3f;
        data[160] = 0x3f;
        data[162] = 1;
        data[240] = 0x56;
        data[320] = 0x56;
        data[322] = 1;
        data[400] = 0x56;
        data[402] = 2;
        data
    }
    #[test]
    fn pal_profile_and_unknown_profile_remain_distinct() {
        let mut data = header();
        data[3] = 0x80;
        let parsed = parse(&mut Cursor::new(data)).unwrap();
        assert_eq!(parsed.tracks[0].height, Some(576));
        assert_eq!(parsed.duration_seconds, None);
        data[451] = 31;
        assert_eq!(parse(&mut Cursor::new(data)).unwrap().tracks[0].width, None);
    }
    #[test]
    fn truncated_or_random_dif_is_rejected() {
        assert!(parse(&mut Cursor::new([0; 480])).is_err());
        assert!(parse(&mut Cursor::new(&header()[..479])).is_err());
    }
}
