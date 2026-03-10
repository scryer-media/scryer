use std::collections::HashMap;
use std::path::Path;

use matroska_demuxer::{Frame, MatroskaFile, TrackType, TransferCharacteristics};

use crate::codec::normalize_codec_name;
use crate::types::{RawContainer, RawTrack, TrackKind};
use crate::MediaInfoError;

/// Maximum number of seconds of frame data to sample when estimating per-track
/// bitrates. For files shorter than this, all frames are counted.
const BITRATE_SAMPLE_SECONDS: f64 = 30.0;

/// Parse an MKV/WebM file into a [`RawContainer`].
pub(crate) fn parse_mkv(path: &Path) -> Result<RawContainer, MediaInfoError> {
    let file = std::fs::File::open(path).map_err(|e| MediaInfoError::Io(e.to_string()))?;
    let mut mkv = MatroskaFile::open(file)
        .map_err(|e| MediaInfoError::Parse(format!("matroska open: {e}")))?;

    // -- container format ------------------------------------------------
    let doc_type = mkv.ebml_header().doc_type().trim_end_matches('\0');
    let format_name = if doc_type == "webm" {
        "webm".to_owned()
    } else {
        "matroska".to_owned()
    };

    // -- duration --------------------------------------------------------
    // duration() is in TimestampScale units; multiply by timestamp_scale to
    // get nanoseconds, then divide by 1e9 to get seconds.
    let timestamp_scale_ns = mkv.info().timestamp_scale().get() as f64;
    let duration_seconds = mkv.info().duration().map(|d| d * timestamp_scale_ns / 1e9);

    // -- tracks ----------------------------------------------------------
    let mut tracks: Vec<RawTrack> = Vec::new();
    // Map track_number -> index in `tracks` for bitrate accumulation.
    let mut track_index: HashMap<u64, usize> = HashMap::new();

    for entry in mkv.tracks() {
        let kind = match entry.track_type() {
            TrackType::Video => TrackKind::Video,
            TrackType::Audio => TrackKind::Audio,
            TrackType::Subtitle => TrackKind::Subtitle,
            _ => continue,
        };

        let codec_id_str = entry.codec_id();
        let mut raw = RawTrack {
            kind,
            codec_id: codec_id_str.to_owned(),
            codec_name: normalize_codec_name(codec_id_str),
            codec_private: entry.codec_private().map(|b| b.to_vec()),
            width: None,
            height: None,
            channels: None,
            bit_rate_bps: None,
            language: entry
                .language_bcp47()
                .or_else(|| entry.language())
                .filter(|l| !l.is_empty() && *l != "und")
                .map(str::to_owned),
            name: entry.name().map(str::to_owned),
            forced: entry.flag_forced(),
            default_track: entry.flag_default(),
            frame_rate_fps: None,
            color_transfer: None,
            dovi_config: None,
            has_hdr10plus: false,
        };

        match kind {
            TrackKind::Video => {
                if let Some(video) = entry.video() {
                    raw.width = Some(video.pixel_width().get() as i32);
                    raw.height = Some(video.pixel_height().get() as i32);

                    if let Some(colour) = video.colour() {
                        raw.color_transfer =
                            colour.transfer_characteristics().map(transfer_to_itu_value);
                    }
                }

                // default_duration is in nanoseconds (not scaled by TimestampScale).
                // fps = 1e9 / default_duration_ns.
                if let Some(dd) = entry.default_duration() {
                    let ns = dd.get() as f64;
                    if ns > 0.0 {
                        raw.frame_rate_fps = Some(1e9 / ns);
                    }
                }
            }
            TrackKind::Audio => {
                if let Some(audio) = entry.audio() {
                    raw.channels = Some(audio.channels().get() as i32);
                }
            }
            TrackKind::Subtitle => { /* codec_id and language are sufficient */ }
        }

        let track_num = entry.track_number().get();
        track_index.insert(track_num, tracks.len());
        tracks.push(raw);
    }

    // -- Dolby Vision detection via raw EBML scanning ----------------------
    // matroska-demuxer 0.7 doesn't expose BlockAdditionMapping, so we scan
    // the raw file bytes for the BlockAddIDType element (0x41E7) with value
    // 0x6476 ("dv") and extract the adjacent BlockAddIDExtraData (0x41ED).
    if let Some(dovi_config) = scan_mkv_dovi_config(path) {
        if let Some(vt) = tracks.iter_mut().find(|t| t.kind == TrackKind::Video) {
            vt.dovi_config = Some(dovi_config);
        }
    }

    // -- per-track bitrate estimation ------------------------------------
    estimate_bitrates(&mut mkv, &track_index, &mut tracks, duration_seconds)?;

    Ok(RawContainer {
        format_name,
        duration_seconds,
        tracks,
    })
}

/// Walk frames and accumulate byte sizes per track to estimate bitrate.
///
/// For files longer than [`BITRATE_SAMPLE_SECONDS`] we stop early and
/// extrapolate from the sampled duration.
///
/// As a side-effect, scans the first HEVC video frame for HDR10+ (SMPTE ST
/// 2094-40) dynamic metadata and sets `has_hdr10plus` on the video track.
fn estimate_bitrates<R: std::io::Read + std::io::Seek>(
    mkv: &mut MatroskaFile<R>,
    track_index: &HashMap<u64, usize>,
    tracks: &mut [RawTrack],
    duration_seconds: Option<f64>,
) -> Result<(), MediaInfoError> {
    let timestamp_scale_ns = mkv.info().timestamp_scale().get() as f64;

    // Determine the sampling cutoff in TimestampScale units.
    let sample_limit_ts: Option<u64> = duration_seconds
        .filter(|&d| d > BITRATE_SAMPLE_SECONDS)
        .map(|_| (BITRATE_SAMPLE_SECONDS * 1e9 / timestamp_scale_ns) as u64);

    // Identify the HEVC video track (if any) for HDR10+ scanning.
    let hevc_video_track_num: Option<u64> = track_index.iter().find_map(|(&num, &idx)| {
        let t = &tracks[idx];
        if t.kind == TrackKind::Video && t.codec_name.as_deref() == Some("hevc") {
            Some(num)
        } else {
            None
        }
    });
    let hevc_nal_len = hevc_video_track_num.and_then(|num| {
        let idx = *track_index.get(&num)?;
        tracks[idx]
            .codec_private
            .as_deref()
            .map(crate::codec::hevc_nal_length_size)
    });
    let mut checked_hdr10plus = false;

    // Accumulate total bytes per track and track the last timestamp seen.
    let mut bytes_per_track: HashMap<u64, u64> = HashMap::new();
    let mut last_timestamp: u64 = 0;

    let mut frame = Frame::default();
    loop {
        let has_frame = mkv
            .next_frame(&mut frame)
            .map_err(|e| MediaInfoError::Parse(format!("matroska frame read: {e}")))?;
        if !has_frame {
            break;
        }

        // If we're past the sample window, stop.
        if let Some(limit) = sample_limit_ts {
            if frame.timestamp > limit {
                break;
            }
        }

        if track_index.contains_key(&frame.track) {
            *bytes_per_track.entry(frame.track).or_insert(0) += frame.data.len() as u64;
        }
        if frame.timestamp > last_timestamp {
            last_timestamp = frame.timestamp;
        }

        // Check first HEVC video frame for HDR10+ SEI.
        if !checked_hdr10plus {
            if let (Some(hevc_num), Some(nal_len)) = (hevc_video_track_num, hevc_nal_len) {
                if frame.track == hevc_num {
                    checked_hdr10plus = true;
                    if crate::codec::scan_hevc_frame_for_hdr10plus(&frame.data, nal_len) {
                        if let Some(&idx) = track_index.get(&hevc_num) {
                            tracks[idx].has_hdr10plus = true;
                        }
                    }
                }
            }
        }
    }

    // Convert last_timestamp (TimestampScale units) to seconds.
    let sampled_seconds = (last_timestamp as f64) * timestamp_scale_ns / 1e9;
    if sampled_seconds <= 0.0 {
        return Ok(());
    }

    for (&track_num, &total_bytes) in &bytes_per_track {
        if let Some(&idx) = track_index.get(&track_num) {
            // bits per second = bytes * 8 / seconds
            let bps = ((total_bytes as f64) * 8.0 / sampled_seconds) as i64;
            tracks[idx].bit_rate_bps = Some(bps);
        }
    }

    Ok(())
}

/// Convert a [`TransferCharacteristics`] enum value to the ITU-T H.273 numeric
/// value stored in [`RawTrack::color_transfer`].
fn transfer_to_itu_value(tc: TransferCharacteristics) -> u32 {
    match tc {
        TransferCharacteristics::Bt709 => 1,
        TransferCharacteristics::Bt407m => 4,
        TransferCharacteristics::Bt407bg => 5,
        TransferCharacteristics::Smpte170 => 6,
        TransferCharacteristics::Smpte240 => 7,
        TransferCharacteristics::Linear => 8,
        TransferCharacteristics::Log => 9,
        TransferCharacteristics::LogSqrt => 10,
        TransferCharacteristics::Iec61966_2_4 => 11,
        TransferCharacteristics::Bt1361 => 12,
        TransferCharacteristics::Iec61966_2_1 => 13,
        TransferCharacteristics::Bt220_10 => 14,
        TransferCharacteristics::Bt220_12 => 15,
        TransferCharacteristics::Bt2100 => 16,
        TransferCharacteristics::SmpteSt428_1 => 17,
        TransferCharacteristics::Hlg => 18,
        // Unknown has no standard ITU-T code; use 2 ("unspecified").
        TransferCharacteristics::Unknown => 2,
    }
}

/// Scan raw MKV bytes for a Dolby Vision configuration record.
///
/// Looks for the EBML pattern: BlockAddIDType (element ID 0x41E7) with value
/// 0x6476 ("dv"), then extracts the sibling BlockAddIDExtraData (0x41ED) which
/// contains the DOVIDecoderConfigurationRecord.
///
/// Only reads the first 256KB of the file (the Tracks element is always near
/// the beginning).
fn scan_mkv_dovi_config(path: &Path) -> Option<Vec<u8>> {
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

    // Search for BlockAddIDType (0x41E7) with value 0x6476.
    // Pattern: [0x41, 0xE7, 0x82, 0x64, 0x76]
    //   0x41E7 = element ID (2 bytes)
    //   0x82 = VINT size = 2
    //   0x6476 = value (2 bytes, "dv")
    let dv_marker = [0x41, 0xE7, 0x82, 0x64, 0x76];
    let marker_pos = buf.windows(5).position(|w| w == dv_marker)?;

    // Now search backward and forward from the marker for the parent
    // BlockAdditionMapping element, and within it find BlockAddIDExtraData
    // (element ID 0x41ED). The extra data follows nearby.
    let search_start = marker_pos.saturating_sub(64);
    let search_end = (marker_pos + 256).min(n);
    let region = &buf[search_start..search_end];

    // Look for BlockAddIDExtraData element ID (0x41ED)
    let extra_data_id = [0x41, 0xED];
    for i in 0..region.len().saturating_sub(3) {
        if region[i] == extra_data_id[0] && region[i + 1] == extra_data_id[1] {
            // Parse the EBML VINT size
            let (size, size_len) = parse_ebml_vint(&region[i + 2..])?;
            let data_start = i + 2 + size_len;
            let data_end = data_start + size;
            if data_end <= region.len() && size >= 5 {
                return Some(region[data_start..data_end].to_vec());
            }
        }
    }

    None
}

/// Parse an EBML variable-size integer (VINT). Returns (value, bytes_consumed).
fn parse_ebml_vint(data: &[u8]) -> Option<(usize, usize)> {
    if data.is_empty() {
        return None;
    }
    let first = data[0];
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || len > data.len() {
        return None;
    }
    let mut value = (first & (0xFF >> len)) as usize;
    for &b in &data[1..len] {
        value = (value << 8) | b as usize;
    }
    Some((value, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_itu_values() {
        assert_eq!(transfer_to_itu_value(TransferCharacteristics::Bt2100), 16);
        assert_eq!(transfer_to_itu_value(TransferCharacteristics::Hlg), 18);
        assert_eq!(transfer_to_itu_value(TransferCharacteristics::Bt709), 1);
        assert_eq!(transfer_to_itu_value(TransferCharacteristics::Unknown), 2);
    }

    #[test]
    fn ebml_vint_parsing() {
        // 0x81 = 1 byte VINT, value 1
        assert_eq!(parse_ebml_vint(&[0x81]), Some((1, 1)));
        // 0x82 = 1 byte VINT, value 2
        assert_eq!(parse_ebml_vint(&[0x82]), Some((2, 1)));
        // 0x85 = 1 byte VINT, value 5
        assert_eq!(parse_ebml_vint(&[0x85]), Some((5, 1)));
        // 0x40 0x18 = 2 byte VINT, value 24
        assert_eq!(parse_ebml_vint(&[0x40, 0x18]), Some((24, 2)));
    }
}
