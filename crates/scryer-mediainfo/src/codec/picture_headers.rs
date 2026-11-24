//! Profile, level, depth and color from the stream headers of MJPEG, VP8,
//! VP9, MPEG-1/2, MPEG-4 Part 2 and VC-1 video.

use super::mjpeg::jpeg;
use super::mpeg_video::{mpeg4_vol, mpeg12};
use super::vp9::vp9;
use crate::raw::RawTrack;
use scryer_media_model::Codec;

/// Fills what the codec's stream headers declare; tracks of other codecs are
/// left unchanged.
pub(crate) fn enrich(track: &mut RawTrack, data: &[u8]) {
    match track.codec_name {
        Some(Codec::Mjpeg) => {
            let _ = jpeg(track, data);
        }
        Some(Codec::Vp9) => {
            let _ = vp9(track, data);
        }
        Some(Codec::Vp8)
            if data.len() >= 10 && data[0] & 1 == 0 && data[3..6] == [0x9d, 1, 0x2a] =>
        {
            track.metadata.bit_depth = Some(8);
            track.metadata.profile = Some(((data[0] >> 1) & 7).to_string());
            track.metadata.pixel_format = Some("yuv420p".into());
        }
        Some(Codec::Mpeg1Video | Codec::Mpeg2Video) => {
            let _ = mpeg12(track, data);
        }
        Some(Codec::Mpeg4) => {
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
        Some(Codec::Vc1) => {
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

#[cfg(test)]
mod tests {
    #[test]
    fn mjpeg_properties_reach_avi_and_matroska_analysis() {
        for fixture in ["matrix_avi_005.avi", "matrix_mkv_015.mkv"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/media")
                .join(fixture);
            let analysis = crate::probe(&path).unwrap();
            let video = analysis
                .streams
                .iter()
                .find(|stream| stream.codec == Some(scryer_media_model::Codec::Mjpeg))
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
                scryer_media_model::Rational::new(1, 1),
                "{fixture}"
            );
            assert_eq!(video.metadata.color.matrix, Some(5), "{fixture}");
        }
    }
}
