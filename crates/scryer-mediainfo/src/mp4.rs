use std::path::Path;

use mp4parse::{
    AudioCodecSpecific, CodecType, MediaTimeScale, SampleEntry, TrackTimeScale, TrackType,
    VideoCodecSpecific,
};

use crate::MediaInfoError;
use crate::codec::normalize_codec_name;
use crate::types::{RawContainer, RawTrack, TrackKind};

/// Parse an MP4/MOV/M4V file into a [`RawContainer`].
pub(crate) fn parse_mp4(path: &Path) -> Result<RawContainer, MediaInfoError> {
    let file_len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let mut file = std::fs::File::open(path).map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let ctx = mp4parse::read_mp4(&mut file)
        .map_err(|e| MediaInfoError::Parse(format!("mp4 parse: {e:?}")))?;

    // -- duration ----------------------------------------------------------
    // The movie-level timescale converts track-header durations to seconds.
    let movie_timescale = ctx.timescale.map(|MediaTimeScale(ts)| ts);

    // Compute overall duration from the longest track header duration. The
    // movie-level duration lives in the private MovieHeaderBox.duration field
    // that mp4parse does not expose, so we derive it from the track headers
    // which store their duration in movie-timescale units.
    let duration_seconds = movie_timescale.and_then(|ts| {
        if ts == 0 {
            return None;
        }
        ctx.tracks
            .iter()
            .filter_map(|t| t.tkhd.as_ref().map(|h| h.duration))
            .max()
            .map(|dur| dur as f64 / ts as f64)
    });

    // -- tracks ------------------------------------------------------------
    let mut tracks = Vec::new();

    for track in ctx.tracks.iter() {
        let kind = match track.track_type {
            TrackType::Video | TrackType::Picture | TrackType::AuxiliaryVideo => TrackKind::Video,
            TrackType::Audio => TrackKind::Audio,
            // mp4parse does not distinguish subtitle tracks; they appear as
            // Metadata or Unknown. We skip them since the parser cannot
            // reliably extract subtitle codec info.
            TrackType::Metadata | TrackType::Unknown => continue,
        };

        let descriptions = match track.stsd {
            Some(ref stsd) => &stsd.descriptions,
            None => continue,
        };

        let first_entry = match descriptions.first() {
            Some(entry) => entry,
            None => continue,
        };

        let mut raw = RawTrack {
            kind,
            codec_id: String::new(),
            codec_name: None,
            codec_private: None,
            width: None,
            height: None,
            channels: None,
            bit_rate_bps: None,
            language: None,
            frame_rate_fps: None,
            color_transfer: None,
            dovi_config: None,
            has_hdr10plus: false,
            name: None,
            forced: false,
            default_track: false,
        };

        match first_entry {
            SampleEntry::Video(video) => {
                raw.width = Some(i32::from(video.width));
                raw.height = Some(i32::from(video.height));

                let (codec_id, codec_private) = video_codec_info(&video.codec_specific);
                let codec_id = codec_id.unwrap_or_else(|| codec_type_to_fourcc(video.codec_type));
                raw.codec_id = codec_id.clone();
                raw.codec_name = normalize_codec_name(&codec_id);
                raw.codec_private = codec_private;

                // Frame rate from stts (time-to-sample table).
                raw.frame_rate_fps = estimate_frame_rate(track);
            }
            SampleEntry::Audio(audio) => {
                raw.channels = Some(audio.channelcount as i32);

                let codec_id = audio_codec_id(&audio.codec_specific)
                    .unwrap_or_else(|| codec_type_to_fourcc(audio.codec_type));
                raw.codec_id = codec_id.clone();
                raw.codec_name = normalize_codec_name(&codec_id);

                // Extract codec private data from ESDS when available.
                if let AudioCodecSpecific::ES_Descriptor(ref esds) = audio.codec_specific
                    && !esds.decoder_specific_data.is_empty()
                {
                    raw.codec_private = Some(esds.decoder_specific_data.iter().copied().collect());
                }
            }
            SampleEntry::Unknown => {
                // mp4parse doesn't support HEVC — tracks appear as Unknown.
                // Keep the track so DV scanning and bitrate estimation still work.
                raw.codec_id = "unknown".into();
            }
        }

        // -- per-track bitrate estimation ----------------------------------
        raw.bit_rate_bps = estimate_track_bitrate(track, duration_seconds);

        tracks.push(raw);
    }

    // If no per-track bitrate was computed but we have a file size and
    // duration, fall back to overall bitrate for the first video track.
    if file_len > 0
        && let Some(dur) = duration_seconds
        && dur > 0.0
    {
        let overall_bps = (file_len as f64 * 8.0 / dur) as i64;
        let has_any_video_bitrate = tracks
            .iter()
            .any(|t| t.kind == TrackKind::Video && t.bit_rate_bps.is_some());
        if !has_any_video_bitrate
            && let Some(vt) = tracks.iter_mut().find(|t| t.kind == TrackKind::Video)
        {
            vt.bit_rate_bps = Some(overall_bps);
        }
    }

    // -- HDR10+ detection via first video sample ----------------------------
    // For HEVC video tracks with PQ transfer, read the first sample and scan
    // NAL units for SMPTE ST 2094-40 dynamic metadata.
    scan_mp4_hdr10plus(path, &ctx, &mut tracks);

    // -- Dolby Vision detection via raw box scanning -----------------------
    // mp4parse doesn't expose dvcC/dvvC boxes. Scan the raw file bytes for
    // the FourCC and extract the DOVIDecoderConfigurationRecord.
    if let Some(dovi_config) = scan_mp4_dovi_config(path)
        && let Some(vt) = tracks.iter_mut().find(|t| t.kind == TrackKind::Video)
    {
        // If the track was unknown (HEVC not supported by mp4parse),
        // mark it as HEVC since DV requires HEVC.
        if vt.codec_name.is_none() {
            vt.codec_id = "hvc1".into();
            vt.codec_name = Some("hevc".into());
        }
        vt.dovi_config = Some(dovi_config);
    }

    // Derive container format name from file extension.
    let format_name = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .map(|ext| match ext.as_str() {
            "mov" => "mov",
            _ => "mp4",
        })
        .unwrap_or("mp4")
        .to_owned();

    Ok(RawContainer {
        format_name,
        duration_seconds,
        num_chapters: None,
        tracks,
    })
}

/// Extract codec identifier and codec-private bytes from a video sample entry.
fn video_codec_info(codec_specific: &VideoCodecSpecific) -> (Option<String>, Option<Vec<u8>>) {
    match codec_specific {
        VideoCodecSpecific::AVCConfig(data) => {
            let private: Vec<u8> = data.iter().copied().collect();
            (Some("avc1".into()), Some(private))
        }
        VideoCodecSpecific::VPxConfig(vpx) => {
            let private: Vec<u8> = vpx.codec_init.iter().copied().collect();
            let private = if private.is_empty() {
                None
            } else {
                Some(private)
            };
            // VPxConfig is used for both VP8 and VP9; the codec_type on the
            // parent VideoSampleEntry disambiguates, so we return None here
            // and let the caller fall through to codec_type_to_fourcc.
            (None, private)
        }
        VideoCodecSpecific::AV1Config(av1c) => {
            let private: Vec<u8> = av1c.raw_config.iter().copied().collect();
            (Some("av01".into()), Some(private))
        }
        VideoCodecSpecific::ESDSConfig(data) => {
            let private: Vec<u8> = data.iter().copied().collect();
            (Some("mp4v".into()), Some(private))
        }
        VideoCodecSpecific::H263Config(data) => {
            let private: Vec<u8> = data.iter().copied().collect();
            (Some("s263".into()), Some(private))
        }
    }
}

/// Map an `AudioCodecSpecific` variant to a FourCC / codec identifier string.
fn audio_codec_id(codec_specific: &AudioCodecSpecific) -> Option<String> {
    match codec_specific {
        AudioCodecSpecific::ES_Descriptor(esds) => match esds.audio_codec {
            CodecType::AAC => Some("mp4a".into()),
            CodecType::MP3 => Some(".mp3".into()),
            _ => None,
        },
        AudioCodecSpecific::FLACSpecificBox(_) => Some("fLaC".into()),
        AudioCodecSpecific::OpusSpecificBox(_) => Some("Opus".into()),
        AudioCodecSpecific::ALACSpecificBox(_) => Some("alac".into()),
        AudioCodecSpecific::MP3 => Some(".mp3".into()),
        AudioCodecSpecific::LPCM => Some("lpcm".into()),
    }
}

/// Fallback: derive a FourCC-style string from the mp4parse `CodecType` enum.
fn codec_type_to_fourcc(ct: CodecType) -> String {
    match ct {
        CodecType::H264 => "avc1".into(),
        CodecType::AV1 => "av01".into(),
        CodecType::VP9 => "vp09".into(),
        CodecType::VP8 => "vp08".into(),
        CodecType::MP4V => "mp4v".into(),
        CodecType::H263 => "s263".into(),
        CodecType::AAC => "mp4a".into(),
        CodecType::MP3 => ".mp3".into(),
        CodecType::FLAC => "fLaC".into(),
        CodecType::Opus => "Opus".into(),
        CodecType::ALAC => "alac".into(),
        CodecType::LPCM => "lpcm".into(),
        CodecType::EncryptedVideo => "encv".into(),
        CodecType::EncryptedAudio => "enca".into(),
        CodecType::Unknown => "unknown".into(),
    }
}

/// Estimate a track's bitrate from the `stsz` (sample size) box.
///
/// Returns bits-per-second, or `None` if the data is unavailable.
fn estimate_track_bitrate(track: &mp4parse::Track, container_duration: Option<f64>) -> Option<i64> {
    let stsz = track.stsz.as_ref()?;

    let total_bytes: u64 = if stsz.sample_size > 0 {
        // Fixed sample size: total = sample_size * sample_count.
        let count = if !stsz.sample_sizes.is_empty() {
            stsz.sample_sizes.len() as u64
        } else {
            // Derive count from the stts table when sample_sizes is empty.
            track
                .stts
                .as_ref()
                .map(|stts| stts.samples.iter().map(|s| u64::from(s.sample_count)).sum())
                .unwrap_or(0)
        };
        u64::from(stsz.sample_size) * count
    } else {
        stsz.sample_sizes.iter().map(|&s| u64::from(s)).sum()
    };

    if total_bytes == 0 {
        return None;
    }

    // Prefer track-local duration if available.
    let dur = track_duration_seconds(track).or(container_duration)?;
    if dur <= 0.0 {
        return None;
    }

    Some((total_bytes as f64 * 8.0 / dur) as i64)
}

/// Compute a track's duration in seconds from its mdhd timescale and duration.
fn track_duration_seconds(track: &mp4parse::Track) -> Option<f64> {
    let ts = track.timescale.as_ref().map(|TrackTimeScale(t, _)| *t)?;
    if ts == 0 {
        return None;
    }
    let dur = track.duration.as_ref().map(|d| d.0)?;
    Some(dur as f64 / ts as f64)
}

/// Estimate the frame rate from the `stts` (time-to-sample) table.
///
/// For constant-frame-rate content the table typically has a single entry
/// whose `sample_delta` gives the frame duration in track-timescale units.
fn estimate_frame_rate(track: &mp4parse::Track) -> Option<f64> {
    let ts = track.timescale.as_ref().map(|TrackTimeScale(t, _)| *t)?;
    if ts == 0 {
        return None;
    }
    let stts = track.stts.as_ref()?;
    if stts.samples.is_empty() {
        return None;
    }

    let total_samples: u64 = stts.samples.iter().map(|s| u64::from(s.sample_count)).sum();
    if total_samples == 0 {
        return None;
    }

    // If one entry covers >= 90% of samples, treat its delta as the frame
    // duration (constant frame rate).
    let dominant = stts.samples.iter().max_by_key(|s| s.sample_count)?;
    if u64::from(dominant.sample_count) * 10 >= total_samples * 9 && dominant.sample_delta > 0 {
        let fps = ts as f64 / f64::from(dominant.sample_delta);
        if fps > 0.0 && fps < 1000.0 {
            return Some(fps);
        }
    }

    // Fallback: average frame rate over the whole track.
    let total_delta: u64 = stts
        .samples
        .iter()
        .map(|s| u64::from(s.sample_count) * u64::from(s.sample_delta))
        .sum();
    if total_delta == 0 {
        return None;
    }
    let fps = total_samples as f64 * ts as f64 / total_delta as f64;
    if fps > 0.0 && fps < 1000.0 {
        Some(fps)
    } else {
        None
    }
}

/// Scan the first video sample for HEVC HDR10+ (SMPTE ST 2094-40) SEI metadata.
///
/// Uses stco (chunk offsets) and stsz (sample sizes) from mp4parse to locate
/// the first video sample, then reads it and scans for SEI NAL units.
///
/// Works even for Unknown sample entries (mp4parse doesn't support HEVC) by
/// defaulting to a 4-byte NAL length prefix.
fn scan_mp4_hdr10plus(path: &Path, ctx: &mp4parse::MediaContext, tracks: &mut [RawTrack]) {
    use std::io::{Read, Seek, SeekFrom};

    // Find the first video track (may be HEVC or Unknown/unidentified).
    let raw_idx = match tracks.iter().position(|t| t.kind == TrackKind::Video) {
        Some(i) => i,
        None => return,
    };

    // Derive NAL length size from codec_private if available; default to 4.
    let nal_length_size = tracks[raw_idx]
        .codec_private
        .as_deref()
        .map(crate::codec::hevc_nal_length_size)
        .unwrap_or(4);

    // Find the corresponding mp4parse video track.
    let mp4_track = match ctx.tracks.iter().find(|t| {
        matches!(
            t.track_type,
            TrackType::Video | TrackType::Picture | TrackType::AuxiliaryVideo
        )
    }) {
        Some(t) => t,
        None => return,
    };

    // Get the offset and size of the first sample.
    let first_offset = mp4_track
        .stco
        .as_ref()
        .and_then(|stco| stco.offsets.first().copied());
    let first_size = mp4_track.stsz.as_ref().and_then(|stsz| {
        if stsz.sample_size > 0 {
            Some(stsz.sample_size as u64)
        } else {
            stsz.sample_sizes.first().map(|&s| s as u64)
        }
    });

    let (offset, size) = match (first_offset, first_size) {
        (Some(o), Some(s)) if s > 0 && s <= 4 * 1024 * 1024 => (o, s as usize),
        _ => return,
    };

    // Read the first sample from the file.
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return;
    }
    let mut buf = vec![0u8; size];
    if file.read_exact(&mut buf).is_err() {
        return;
    }

    if crate::codec::scan_hevc_frame_for_hdr10plus(&buf, nal_length_size) {
        let vt = &mut tracks[raw_idx];
        vt.has_hdr10plus = true;
        // If the track was unidentified (HEVC not supported by mp4parse),
        // mark it as HEVC since HDR10+ only exists on HEVC streams.
        if vt.codec_name.is_none() {
            vt.codec_id = "hvc1".into();
            vt.codec_name = Some("hevc".into());
        }
    }
}

/// Scan raw MP4 bytes for a Dolby Vision configuration box (dvcC or dvvC).
///
/// The box structure is: size(4) + fourcc(4) + DOVIDecoderConfigurationRecord.
/// Only reads the first 256KB of the file (moov/stsd are near the beginning).
fn scan_mp4_dovi_config(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 256 * 1024];
    let n = {
        let mut total = 0;
        while total < buf.len() {
            match file.read(&mut buf[total..]) {
                Ok(0) => break,
                Ok(n) => total += n,
                Err(_) => break,
            }
        }
        total
    };
    let buf = &buf[..n];

    // dvcC = [0x64, 0x76, 0x63, 0x43]
    // dvvC = [0x64, 0x76, 0x76, 0x43]
    for i in 4..n.saturating_sub(12) {
        let is_dvcc =
            buf[i] == 0x64 && buf[i + 1] == 0x76 && buf[i + 2] == 0x63 && buf[i + 3] == 0x43;
        let is_dvvc =
            buf[i] == 0x64 && buf[i + 1] == 0x76 && buf[i + 2] == 0x76 && buf[i + 3] == 0x43;
        if is_dvcc || is_dvvc {
            // Box size is the 4 bytes before the FourCC (big-endian u32).
            let box_size =
                u32::from_be_bytes([buf[i - 4], buf[i - 3], buf[i - 2], buf[i - 1]]) as usize;
            let content_size = box_size.saturating_sub(8);
            let content_start = i + 4;
            let content_end = content_start + content_size;
            if content_size >= 5 && content_end <= n {
                return Some(buf[content_start..content_end].to_vec());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_type_to_fourcc_roundtrips() {
        assert_eq!(codec_type_to_fourcc(CodecType::H264), "avc1");
        assert_eq!(codec_type_to_fourcc(CodecType::AV1), "av01");
        assert_eq!(codec_type_to_fourcc(CodecType::VP9), "vp09");
        assert_eq!(codec_type_to_fourcc(CodecType::AAC), "mp4a");
        assert_eq!(codec_type_to_fourcc(CodecType::FLAC), "fLaC");
        assert_eq!(codec_type_to_fourcc(CodecType::Opus), "Opus");
        assert_eq!(codec_type_to_fourcc(CodecType::Unknown), "unknown");
    }

    #[test]
    fn normalize_mp4_codecs() {
        assert_eq!(normalize_codec_name("avc1").as_deref(), Some("h264"));
        assert_eq!(normalize_codec_name("av01").as_deref(), Some("av1"));
        assert_eq!(normalize_codec_name("mp4a").as_deref(), Some("aac"));
        assert_eq!(normalize_codec_name("fLaC").as_deref(), Some("flac"));
        assert_eq!(normalize_codec_name("Opus").as_deref(), Some("opus"));
        assert_eq!(normalize_codec_name("unknown"), None);
    }
}
