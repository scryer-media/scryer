//! MPEG-1/2 audio (Layer I-III) frame headers.

use crate::raw::RawTrack;
use crate::scan;

const MPEG_AUDIO_SAMPLE_RATES: [[u32; 4]; 4] = [
    [11_025, 12_000, 8_000, 0],
    [0, 0, 0, 0],
    [22_050, 24_000, 16_000, 0],
    [44_100, 48_000, 32_000, 0],
];
const MPEG_AUDIO_CHANNELS: [u8; 4] = [2, 2, 2, 1];
const MPEG_AUDIO_BITRATES_MPEG1_LAYER1: [u32; 16] = [
    0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0,
];
const MPEG_AUDIO_BITRATES_MPEG1_LAYER2: [u32; 16] = [
    0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0,
];
const MPEG_AUDIO_BITRATES_MPEG1_LAYER3: [u32; 16] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
];
const MPEG_AUDIO_BITRATES_MPEG2_LAYER1: [u32; 16] = [
    0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0,
];
const MPEG_AUDIO_BITRATES_MPEG2_LAYER2_3: [u32; 16] = [
    0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
];

#[derive(Debug, Clone, Copy)]
pub(crate) struct MpegAudioHeader {
    layer: u8,
    channels: u8,
    sample_rate: u32,
    bit_rate_bps: Option<u32>,
}

impl MpegAudioHeader {
    pub(crate) fn apply(self, track: &mut RawTrack) {
        track.codec_name = Some(
            match self.layer {
                1 => "mp1",
                2 => "mp2",
                _ => "mp3",
            }
            .into(),
        );
        track.channels = Some(i32::from(self.channels));
        track.metadata.sample_rate = Some(self.sample_rate);
        track.metadata.channel_layout =
            Some(if self.channels == 1 { "mono" } else { "stereo" }.into());
        // A frame's bitrate is not a declaration that the whole stream is CBR.
        track.metadata.estimated_bitrate_bps = self.bit_rate_bps.map(u64::from);
    }
}

pub(crate) fn find_mpeg_audio_header(data: &[u8]) -> Option<MpegAudioHeader> {
    if data.len() < 4 {
        return None;
    }

    let mut start = 0;
    while let Some(i) = scan::find_mpeg_audio_sync(data, start) {
        if i + 4 > data.len() {
            return None;
        }
        start = i + 1;
        let header = u32::from_be_bytes(data[i..i + 4].try_into().ok()?);
        if (header & 0xFFE0_0000) != 0xFFE0_0000 {
            continue;
        }

        let version_id = ((header >> 19) & 0x3) as usize;
        let layer_index = ((header >> 17) & 0x3) as usize;
        let bitrate_index = ((header >> 12) & 0xF) as usize;
        let sample_rate_index = ((header >> 10) & 0x3) as usize;
        let padding = (header >> 9) & 0x1;
        let channel_mode = ((header >> 6) & 0x3) as usize;

        if version_id == 1 || layer_index == 0 || bitrate_index == 0 || bitrate_index == 0xF {
            continue;
        }

        let sample_rate = *MPEG_AUDIO_SAMPLE_RATES
            .get(version_id)?
            .get(sample_rate_index)?;
        if sample_rate == 0 {
            continue;
        }

        let bit_rate_kbps = match (version_id == 3, 4 - layer_index as u8) {
            (true, 1) => MPEG_AUDIO_BITRATES_MPEG1_LAYER1[bitrate_index],
            (true, 2) => MPEG_AUDIO_BITRATES_MPEG1_LAYER2[bitrate_index],
            (true, 3) => MPEG_AUDIO_BITRATES_MPEG1_LAYER3[bitrate_index],
            (false, 1) => MPEG_AUDIO_BITRATES_MPEG2_LAYER1[bitrate_index],
            (false, 2 | 3) => MPEG_AUDIO_BITRATES_MPEG2_LAYER2_3[bitrate_index],
            _ => 0,
        };
        if bit_rate_kbps == 0 {
            continue;
        }

        let layer = 4 - layer_index as u8;
        let bit_rate_bps = bit_rate_kbps * 1000;
        let frame_size = match (version_id == 3, layer) {
            (_, 1) => (((12 * bit_rate_bps) / sample_rate) + padding) * 4,
            (_, 2) | (true, 3) => ((144 * bit_rate_bps) / sample_rate) + padding,
            (false, 3) => ((72 * bit_rate_bps) / sample_rate) + padding,
            _ => 0,
        } as usize;
        if frame_size < 4 {
            continue;
        }
        if i + frame_size + 1 < data.len()
            && (data[i + frame_size] != 0xFF || data[i + frame_size + 1] & 0xE0 != 0xE0)
        {
            continue;
        }
        if frame_size > data.len().saturating_sub(i) && data.len().saturating_sub(i) > 4 {
            continue;
        }

        return Some(MpegAudioHeader {
            layer,
            channels: MPEG_AUDIO_CHANNELS[channel_mode],
            sample_rate,
            bit_rate_bps: Some(bit_rate_bps),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mpeg_audio_header() {
        let mut data = vec![0_u8; 388];
        data[..4].copy_from_slice(&[0xFF, 0xFD, 0x84, 0x80]);
        data[384..388].copy_from_slice(&[0xFF, 0xFD, 0x84, 0x80]);
        let header = find_mpeg_audio_header(&data).unwrap();
        assert_eq!(header.channels, 2);
        assert_eq!(header.bit_rate_bps, Some(128_000));
        assert!(
            find_mpeg_audio_header(&data[..384]).is_some(),
            "one complete MP2 frame is sufficient"
        );
        assert!(
            find_mpeg_audio_header(&data[..383]).is_none(),
            "a truncated frame is not complete"
        );
        let mut mp3 = vec![0; 192];
        mp3[..4].copy_from_slice(&[0xff, 0xfb, 0x54, 0x64]);
        let header = find_mpeg_audio_header(&mp3).unwrap();
        assert_eq!(header.sample_rate, 48_000);
        assert_eq!(header.bit_rate_bps, Some(64_000));
        let mut track = RawTrack::default();
        header.apply(&mut track);
        assert_eq!(track.bit_rate_bps, None);
        assert_eq!(track.metadata.estimated_bitrate_bps, Some(64_000));
        track.bit_rate_bps = Some(96_000);
        track.metadata.bitrate_provenance = scryer_media_model::Provenance::Container;
        header.apply(&mut track);
        assert_eq!(track.bit_rate_bps, Some(96_000));
        assert_eq!(
            track.metadata.bitrate_provenance,
            scryer_media_model::Provenance::Container
        );
        let mut low_rate_mp2 = vec![0; 772];
        low_rate_mp2[..4].copy_from_slice(&[0xff, 0xf5, 0xc4, 0xc0]);
        low_rate_mp2[768..772].copy_from_slice(&[0xff, 0xf5, 0xc4, 0xc0]);
        let header = find_mpeg_audio_header(&low_rate_mp2).unwrap();
        assert_eq!(
            (header.layer, header.sample_rate, header.bit_rate_bps),
            (2, 24_000, Some(128_000))
        );
        assert!(
            find_mpeg_audio_header(&low_rate_mp2[..384]).is_none(),
            "MPEG-2 Layer II still uses 1152 samples per frame"
        );
    }
}
