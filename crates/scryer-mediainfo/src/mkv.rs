use std::collections::HashMap;
use std::path::Path;

use matroska_demuxer::{Frame, MatroskaFile, TrackType, TransferCharacteristics};

use crate::MediaInfoError;
use crate::codec::normalize_codec_name;
use crate::probe::ProbeBudget;
use crate::types::{RawContainer, RawTrack, TrackKind};

const HDR10PLUS_SCAN_MAX_BYTES: u64 = 4 * 1024 * 1024;
const HDR10PLUS_SCAN_MAX_FRAMES: usize = 2048;

fn normalize_mkv_track_language(
    language_bcp47: Option<&str>,
    language: Option<&str>,
) -> Option<String> {
    language_bcp47
        .or(language)
        .filter(|value| !value.is_empty() && *value != "und")
        .map(str::to_owned)
}

/// Parse an MKV/WebM file into a [`RawContainer`].
pub(crate) fn parse_mkv(path: &Path) -> Result<RawContainer, MediaInfoError> {
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
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
    let num_chapters = Some(count_mkv_chapters(&mkv));

    // -- tracks ----------------------------------------------------------
    let mut tracks: Vec<RawTrack> = Vec::new();
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
            language: normalize_mkv_track_language(entry.language_bcp47(), entry.language()),
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
    if let Some(dovi_config) = scan_mkv_dovi_config(path)
        && let Some(vt) = tracks.iter_mut().find(|t| t.kind == TrackKind::Video)
    {
        vt.dovi_config = Some(dovi_config);
    }

    // Use a fast overall bitrate estimate for the primary video stream instead
    // of walking large frame ranges over networked filesystems.
    if file_size > 0
        && let Some(duration_seconds) = duration_seconds
        && duration_seconds > 0.0
        && let Some(video_track) = tracks
            .iter_mut()
            .find(|track| track.kind == TrackKind::Video)
    {
        video_track.bit_rate_bps = Some((file_size as f64 * 8.0 / duration_seconds) as i64);
    }

    scan_first_hevc_frame_for_hdr10plus(&mut mkv, &track_index, &mut tracks)?;

    Ok(RawContainer {
        format_name,
        duration_seconds,
        num_chapters,
        tracks,
    })
}

fn count_mkv_chapters<R: std::io::Read + std::io::Seek>(mkv: &MatroskaFile<R>) -> i32 {
    mkv.chapters()
        .map(|editions| {
            editions
                .iter()
                .map(|edition| edition.chapter_atoms().len() as i32)
                .sum()
        })
        .unwrap_or(0)
}

/// Scan only until the first HEVC video frame is seen, with explicit budgets,
/// so MKV/WebM analysis stays header-driven instead of frame-walking.
fn scan_first_hevc_frame_for_hdr10plus<R: std::io::Read + std::io::Seek>(
    mkv: &mut MatroskaFile<R>,
    track_index: &HashMap<u64, usize>,
    tracks: &mut [RawTrack],
) -> Result<(), MediaInfoError> {
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
    let Some((hevc_track_num, nal_length_size)) = hevc_video_track_num.zip(hevc_nal_len) else {
        return Ok(());
    };

    let mut payload_budget = ProbeBudget::new(HDR10PLUS_SCAN_MAX_BYTES);
    let mut frame = Frame::default();
    for _ in 0..HDR10PLUS_SCAN_MAX_FRAMES {
        let has_frame = mkv
            .next_frame(&mut frame)
            .map_err(|e| MediaInfoError::Parse(format!("matroska frame read: {e}")))?;
        if !has_frame {
            break;
        }

        if payload_budget.exhausted() {
            break;
        }
        payload_budget.consume(frame.data.len());

        if frame.track == hevc_track_num {
            if crate::codec::scan_hevc_frame_for_hdr10plus(&frame.data, nal_length_size)
                && let Some(&idx) = track_index.get(&hevc_track_num)
            {
                tracks[idx].has_hdr10plus = true;
            }
            break;
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

    #[test]
    fn normalize_track_language_preserves_unknown() {
        assert_eq!(normalize_mkv_track_language(None, None), None);
        assert_eq!(normalize_mkv_track_language(Some(""), None), None);
        assert_eq!(normalize_mkv_track_language(None, Some("und")), None);
        assert_eq!(
            normalize_mkv_track_language(Some("jpn"), Some("eng")),
            Some("jpn".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(None, Some("eng")),
            Some("eng".to_string())
        );
    }
}
