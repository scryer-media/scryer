//! FLAC and WAVE files: their framing is checked so an audio-only file is never
//! reported as unsupported video, and no audio data is decoded.

use crate::MediaInfoError;
use crate::detect::ContainerFormat;
use crate::raw::RawContainer;
use crate::source;

/// Recognize explicit audio containers before a video suffix can turn them into
/// an unavailable probe. Their framing is checked without decoding audio data.
pub(crate) fn parse_audio_only(
    source: &mut dyn source::MediaSource,
    format: ContainerFormat,
) -> Result<RawContainer, MediaInfoError> {
    use std::io::SeekFrom;
    let malformed = || MediaInfoError::Parse("invalid audio container framing".into());
    source.seek(SeekFrom::Start(0))?;
    let name = if format == ContainerFormat::FlacAudio {
        let mut streaminfo = [0u8; 42];
        source.read_exact(&mut streaminfo)?;
        if &streaminfo[..4] != b"fLaC"
            || streaminfo[4] & 0x7f != 0
            || streaminfo[5..8] != [0, 0, 34]
        {
            return Err(malformed());
        }
        let minimum = u16::from_be_bytes([streaminfo[8], streaminfo[9]]);
        let maximum = u16::from_be_bytes([streaminfo[10], streaminfo[11]]);
        let rate = (u32::from(streaminfo[18]) << 12)
            | (u32::from(streaminfo[19]) << 4)
            | (u32::from(streaminfo[20]) >> 4);
        let bits = ((streaminfo[20] & 1) << 4 | streaminfo[21] >> 4) + 1;
        if minimum < 16 || maximum < minimum || rate == 0 || !(4..=32).contains(&bits) {
            return Err(malformed());
        }
        let mut last = streaminfo[4] & 0x80 != 0;
        let mut position = 42u64;
        while !last {
            let mut block = [0u8; 4];
            source.read_exact(&mut block)?;
            if matches!(block[0] & 0x7f, 0 | 127) {
                return Err(malformed());
            }
            let length = u32::from_be_bytes([0, block[1], block[2], block[3]]);
            position = position
                .checked_add(4 + u64::from(length))
                .filter(|end| *end <= source.len())
                .ok_or_else(malformed)?;
            source.seek(SeekFrom::Start(position))?;
            last = block[0] & 0x80 != 0;
        }
        "flac"
    } else {
        let mut riff = [0u8; 12];
        source.read_exact(&mut riff)?;
        let end = u64::from(u32::from_le_bytes(riff[4..8].try_into().unwrap())) + 8;
        if &riff[..4] != b"RIFF" || &riff[8..] != b"WAVE" || end < 12 || end > source.len() {
            return Err(malformed());
        }
        let mut position = 12u64;
        let mut has_format = false;
        let mut has_data = false;
        while position < end {
            if end - position < 8 {
                return Err(malformed());
            }
            source.seek(SeekFrom::Start(position))?;
            let mut chunk = [0u8; 8];
            source.read_exact(&mut chunk)?;
            let length = u64::from(u32::from_le_bytes(chunk[4..8].try_into().unwrap()));
            let next = position
                .checked_add(8 + length + (length & 1))
                .filter(|next| *next <= end)
                .ok_or_else(malformed)?;
            if &chunk[..4] == b"fmt " {
                if length < 16 {
                    return Err(malformed());
                }
                let mut fmt = [0u8; 16];
                source.read_exact(&mut fmt)?;
                if u16::from_le_bytes([fmt[0], fmt[1]]) == 0
                    || u16::from_le_bytes([fmt[2], fmt[3]]) == 0
                    || u32::from_le_bytes(fmt[4..8].try_into().unwrap()) == 0
                    || u16::from_le_bytes([fmt[12], fmt[13]]) == 0
                {
                    return Err(malformed());
                }
                has_format = true;
            } else if &chunk[..4] == b"data" {
                has_data = true;
            }
            position = next;
        }
        if !has_format || !has_data {
            return Err(malformed());
        }
        "wav"
    };
    let mut details = scryer_media_model::AnalysisDetails::default();
    details.report.status = scryer_media_model::ProbeStatus::Complete;
    Ok(RawContainer {
        format_name: name.into(),
        duration_seconds: None,
        num_chapters: None,
        tracks: Vec::new(),
        details,
    })
}

#[cfg(test)]
mod tests {
    use crate::{AnalyzeOptions, analyze_source, test_facts};

    #[test]
    fn audio_only_signatures_override_video_suffixes_without_becoming_unsupported() {
        let mut flac = b"fLaC\x80\0\0\x22".to_vec();
        flac.extend([0x10, 0, 0x10, 0, 0, 0, 0, 0, 0, 0]);
        flac.extend(((44100u64 << 44) | (15u64 << 36)).to_be_bytes());
        flac.extend([0; 16]);
        let mut wave = b"RIFF".to_vec();
        wave.extend(38u32.to_le_bytes());
        wave.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
        wave.extend(44100u32.to_le_bytes());
        wave.extend(88200u32.to_le_bytes());
        wave.extend(b"\x02\0\x10\0data\x02\0\0\0\0\0");
        for bytes in [&flac, &wave] {
            for extension in ["dat", "xvid", "wtv", "mkv"] {
                let analysis = analyze_source(
                    &mut std::io::Cursor::new(bytes),
                    extension,
                    AnalyzeOptions::default(),
                )
                .unwrap();
                assert!(!test_facts::has_video_with_duration(&analysis));
                assert_eq!(test_facts::video_codec(&analysis), None);
                assert_eq!(analysis.selected_video_id, None);
                assert_eq!(
                    analysis.report.status,
                    scryer_media_model::ProbeStatus::Complete
                );
            }
        }
        flac[7] = 33;
        wave[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        for bytes in [flac, wave, b"fLaC".to_vec()] {
            assert!(
                analyze_source(
                    &mut std::io::Cursor::new(bytes),
                    "wtv",
                    AnalyzeOptions::default(),
                )
                .is_err()
            );
        }
    }
}
