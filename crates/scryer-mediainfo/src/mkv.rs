use std::collections::HashMap;
use std::path::Path;

use matroska_demuxer::{Frame, MatroskaFile, TrackType, TransferCharacteristics};

use crate::MediaInfoError;
use crate::codec::{normalize_codec_name, normalize_pcm_codec_name, normalize_vfw_codec_name};
use crate::probe::ProbeBudget;
use crate::types::{RawContainer, RawTrack, TrackKind};

const HDR10PLUS_SCAN_MAX_BYTES: u64 = 4 * 1024 * 1024;
const MKV_FPS_SCAN_MAX_BYTES: u64 = 8 * 1024 * 1024;
const MKV_FPS_SCAN_MAX_FRAMES: usize = 96;
const MKV_CHAPTER_SCAN_MAX_BYTES: usize = 2 * 1024 * 1024;
const EBML_ID_CHAPTERS: u32 = 0x1043_A770;
const EBML_ID_EDITION_ENTRY: u32 = 0x45B9;
const EBML_ID_CHAPTER_ATOM: u32 = 0xB6;
const EBML_ID_CHAPTER_TIME_START: u32 = 0x91;

fn normalize_mkv_track_language(
    kind: TrackKind,
    _language_bcp47: Option<&str>,
    language: Option<&str>,
) -> Option<String> {
    if let Some(language) = language {
        return normalize_explicit_mkv_language_tag(language);
    }
    (kind != TrackKind::Video).then_some("eng".to_owned())
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
    let num_chapters = Some(
        scan_mkv_chapter_count_ffprobe_style(path).unwrap_or_else(|| count_mkv_chapters(&mkv)),
    );

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
        let audio_bit_depth = entry
            .audio()
            .and_then(|audio| audio.bit_depth())
            .map(|depth| depth.get() as i32);
        let mut raw = RawTrack {
            kind,
            codec_id: codec_id_str.to_owned(),
            codec_name: normalize_pcm_codec_name(codec_id_str, audio_bit_depth)
                .or_else(|| {
                    (codec_id_str == "V_MS/VFW/FOURCC")
                        .then(|| normalize_vfw_codec_name(entry.codec_private()))
                        .flatten()
                })
                .or_else(|| normalize_codec_name(codec_id_str)),
            codec_private: entry.codec_private().map(|b| b.to_vec()),
            width: None,
            height: None,
            channels: None,
            bit_rate_bps: None,
            language: normalize_mkv_track_language(kind, entry.language_bcp47(), entry.language()),
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

    scan_video_frames_for_metadata(&mut mkv, &track_index, &mut tracks)?;

    if let Some(video_track) = tracks
        .iter_mut()
        .find(|track| track.kind == TrackKind::Video)
        && video_track.frame_rate_fps.is_none()
    {
        video_track.frame_rate_fps = fallback_frame_rate_from_timestamp_scale(timestamp_scale_ns);
    }

    Ok(RawContainer {
        format_name,
        duration_seconds,
        num_chapters,
        tracks,
    })
}

fn count_mkv_chapters<R: std::io::Read + std::io::Seek>(mkv: &MatroskaFile<R>) -> i32 {
    mkv.chapters()
        .and_then(|editions| editions.first())
        .map(|edition| {
            count_ffprobe_style_chapter_starts(
                edition
                    .chapter_atoms()
                    .iter()
                    .map(|chapter| chapter.time_start()),
            )
        })
        .unwrap_or(0)
}

fn normalize_explicit_mkv_language_tag(language: &str) -> Option<String> {
    let trimmed = language.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("und") {
        return None;
    }
    Some(trimmed.to_owned())
}

fn scan_mkv_chapter_count_ffprobe_style(path: &Path) -> Option<i32> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0_u8; MKV_CHAPTER_SCAN_MAX_BYTES];
    let n = {
        let mut total = 0;
        while total < buf.len() {
            match file.read(&mut buf[total..]) {
                Ok(0) => break,
                Ok(read) => total += read,
                Err(_) => break,
            }
        }
        total
    };
    count_mkv_chapters_ffprobe_style_from_bytes(&buf[..n])
}

fn count_mkv_chapters_ffprobe_style_from_bytes(data: &[u8]) -> Option<i32> {
    let chapters_payload = find_ebml_element_payload(data, EBML_ID_CHAPTERS)?;
    let first_edition_payload =
        find_first_direct_ebml_child(chapters_payload, EBML_ID_EDITION_ENTRY)?;
    Some(count_top_level_mkv_chapters_ffprobe_style(
        first_edition_payload,
    ))
}

fn find_ebml_element_payload(mut data: &[u8], target_id: u32) -> Option<&[u8]> {
    while !data.is_empty() {
        let (id, payload, consumed) = next_ebml_element(data)?;
        if id == target_id {
            return Some(payload);
        }
        data = &data[consumed..];
    }
    None
}

fn find_first_direct_ebml_child(data: &[u8], target_id: u32) -> Option<&[u8]> {
    let mut current = data;
    while !current.is_empty() {
        let (id, payload, consumed) = next_ebml_element(current)?;
        if id == target_id {
            return Some(payload);
        }
        current = &current[consumed..];
    }
    None
}

fn count_top_level_mkv_chapters_ffprobe_style(data: &[u8]) -> i32 {
    let mut starts = Vec::new();
    let mut current = data;
    while !current.is_empty() {
        let Some((id, payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        if id == EBML_ID_CHAPTER_ATOM
            && let Some(start_payload) =
                find_first_direct_ebml_child(payload, EBML_ID_CHAPTER_TIME_START)
            && let Some(start) = parse_ebml_uint(start_payload)
        {
            starts.push(start);
        }
        current = &current[consumed..];
    }
    count_ffprobe_style_chapter_starts(starts)
}

fn count_ffprobe_style_chapter_starts(starts: impl IntoIterator<Item = u64>) -> i32 {
    let mut max_start = None;
    let mut count = 0_i32;
    for start in starts {
        if max_start.is_none_or(|max_start| start > max_start) {
            max_start = Some(start);
            count += 1;
        }
    }
    count
}

fn next_ebml_element(data: &[u8]) -> Option<(u32, &[u8], usize)> {
    let (id, id_len) = parse_ebml_id(data)?;
    let (size, size_len) = parse_ebml_vint(&data[id_len..])?;
    let payload_start = id_len + size_len;
    let payload_end = payload_start.checked_add(size)?;
    if payload_end > data.len() {
        return None;
    }
    Some((id, &data[payload_start..payload_end], payload_end))
}

fn parse_ebml_id(data: &[u8]) -> Option<(u32, usize)> {
    if data.is_empty() {
        return None;
    }
    let first = data[0];
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len == 0 || len > 4 || len > data.len() {
        return None;
    }

    let mut value = 0_u32;
    for &byte in &data[..len] {
        value = (value << 8) | u32::from(byte);
    }
    Some((value, len))
}

fn parse_ebml_uint(data: &[u8]) -> Option<u64> {
    if data.is_empty() || data.len() > 8 {
        return None;
    }

    let mut value = 0_u64;
    for &byte in data {
        value = (value << 8) | u64::from(byte);
    }
    Some(value)
}

/// Scan a bounded number of video frames to fill the remaining ffprobe-style
/// gaps without turning MKV analysis back into a deep payload walk.
fn scan_video_frames_for_metadata<R: std::io::Read + std::io::Seek>(
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
    let fps_track_num = tracks
        .iter()
        .position(|track| track.kind == TrackKind::Video)
        .and_then(|track_idx| {
            track_index
                .iter()
                .find_map(|(&num, &idx)| (idx == track_idx).then_some(num))
        });

    let (hevc_track_num, nal_length_size) = match hevc_video_track_num.zip(hevc_nal_len) {
        Some(values) => values,
        None => {
            if fps_track_num.is_none() {
                return Ok(());
            }
            (0, 0)
        }
    };

    if hevc_video_track_num.is_none() && fps_track_num.is_none() {
        return Ok(());
    }

    let mut payload_budget = ProbeBudget::new(MKV_FPS_SCAN_MAX_BYTES.max(HDR10PLUS_SCAN_MAX_BYTES));
    let mut frame = Frame::default();
    let mut fps_timestamps = Vec::new();
    let mut fps_done = fps_track_num.is_none();
    let mut hdr_done = hevc_video_track_num.is_none();

    for _ in 0..MKV_FPS_SCAN_MAX_FRAMES {
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

        if !fps_done && Some(frame.track) == fps_track_num {
            if fps_timestamps.last().copied() != Some(frame.timestamp) {
                fps_timestamps.push(frame.timestamp);
            }
            if let Some(fps) = estimate_frame_rate_from_timestamps(&fps_timestamps)
                && let Some(&idx) = track_index.get(&frame.track)
                && should_replace_frame_rate(tracks[idx].frame_rate_fps, fps)
            {
                tracks[idx].frame_rate_fps = Some(fps);
                fps_done = true;
            }
        }

        if !hdr_done && frame.track == hevc_track_num {
            if crate::codec::scan_hevc_frame_for_hdr10plus(&frame.data, nal_length_size)
                && let Some(&idx) = track_index.get(&hevc_track_num)
            {
                tracks[idx].has_hdr10plus = true;
            }
            hdr_done = true;
        }

        if fps_done && hdr_done {
            break;
        }
    }

    if !fps_done
        && let Some(fps) = estimate_frame_rate_from_timestamps(&fps_timestamps)
        && let Some(track_num) = fps_track_num
        && let Some(&idx) = track_index.get(&track_num)
        && should_replace_frame_rate(tracks[idx].frame_rate_fps, fps)
    {
        tracks[idx].frame_rate_fps = Some(fps);
    }

    Ok(())
}

fn estimate_frame_rate_from_timestamps(timestamps: &[u64]) -> Option<f64> {
    if timestamps.len() < 4 {
        return None;
    }

    let mut deltas: Vec<u64> = timestamps
        .windows(2)
        .filter_map(|window| window[1].checked_sub(window[0]))
        .filter(|delta| *delta > 0)
        .collect();
    if deltas.is_empty() {
        return None;
    }

    deltas.sort_unstable();
    let median_delta = deltas[deltas.len() / 2] as f64;
    let delta_seconds = median_delta / 1000.0;
    if delta_seconds <= 0.0 {
        return None;
    }

    let fps = 1.0 / delta_seconds;
    if (1.0..=240.0).contains(&fps) {
        Some(fps)
    } else {
        None
    }
}

fn fallback_frame_rate_from_timestamp_scale(timestamp_scale_ns: f64) -> Option<f64> {
    if timestamp_scale_ns <= 0.0 {
        return None;
    }

    let fps = 1e9 / timestamp_scale_ns;
    (1.0..=1000.0).contains(&fps).then_some(fps)
}

fn should_replace_frame_rate(existing: Option<f64>, observed: f64) -> bool {
    match existing {
        None => true,
        Some(current) if current <= 0.0 => true,
        Some(current) => current < 10.0 && observed >= current * 10.0,
    }
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
    let value_mask = if len == 8 { 0 } else { 0xFF >> len };
    let mut value = (first & value_mask) as usize;
    for &b in &data[1..len] {
        value = (value << 8) | b as usize;
    }
    Some((value, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ebml_element(id: &[u8], payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 0x7F);
        let mut element = Vec::with_capacity(id.len() + 1 + payload.len());
        element.extend_from_slice(id);
        element.push(0x80 | payload.len() as u8);
        element.extend_from_slice(payload);
        element
    }

    fn make_chapter_atom(start: u64, nested: &[u8]) -> Vec<u8> {
        let mut payload = make_ebml_element(&[0x73, 0xC4], &[1]);
        payload.extend_from_slice(&make_ebml_element(&[0x91], &start.to_be_bytes()));
        payload.extend_from_slice(nested);
        make_ebml_element(&[0xB6], &payload)
    }

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
    fn normalize_track_language_matches_ffmpeg_matroska_metadata_rules() {
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Video, None, None),
            None
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Audio, Some("en-US"), None),
            Some("eng".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Subtitle, None, Some("en-US")),
            Some("en-US".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Subtitle, Some("pt-BR"), Some("por")),
            Some("por".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Subtitle, Some("fil"), Some("fil")),
            Some("fil".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Subtitle, Some("jad"), Some("und")),
            None
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Subtitle, None, Some("zxx")),
            Some("zxx".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Audio, Some("ja-JP"), Some("eng")),
            Some("eng".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Audio, None, None),
            Some("eng".to_string())
        );
    }

    #[test]
    fn count_ffprobe_style_chapter_starts_matches_ffmpeg_guard() {
        assert_eq!(
            count_ffprobe_style_chapter_starts([0, 90, 1320, 6, 51, 1429]),
            4
        );
        assert_eq!(
            count_ffprobe_style_chapter_starts([0, 15, 105, 1226, 1315, 1409]),
            6
        );
    }

    #[test]
    fn chapter_scan_uses_first_edition_only() {
        let first_edition = [
            make_chapter_atom(0, &[]),
            make_chapter_atom(90, &[]),
            make_chapter_atom(742, &[]),
        ]
        .concat();
        let second_edition = [
            make_chapter_atom(33, &[]),
            make_chapter_atom(73, &[]),
            make_chapter_atom(164, &[]),
            make_chapter_atom(636, &[]),
        ]
        .concat();
        let chapters = make_ebml_element(
            &[0x10, 0x43, 0xA7, 0x70],
            &[
                make_ebml_element(&[0x45, 0xB9], &first_edition),
                make_ebml_element(&[0x45, 0xB9], &second_edition),
            ]
            .concat(),
        );

        assert_eq!(
            count_mkv_chapters_ffprobe_style_from_bytes(&chapters),
            Some(3)
        );
    }

    #[test]
    fn chapter_scan_ignores_nested_atoms_and_backwards_starts() {
        let nested = make_chapter_atom(105, &[]);
        let edition = [
            make_chapter_atom(0, &nested),
            make_chapter_atom(90, &[]),
            make_chapter_atom(15, &[]),
            make_chapter_atom(1409, &[]),
        ]
        .concat();
        let chapters = make_ebml_element(
            &[0x10, 0x43, 0xA7, 0x70],
            &make_ebml_element(&[0x45, 0xB9], &edition),
        );

        assert_eq!(
            count_mkv_chapters_ffprobe_style_from_bytes(&chapters),
            Some(3)
        );
    }

    #[test]
    fn estimate_frame_rate_from_timestamp_deltas_uses_millisecond_units() {
        let fps = estimate_frame_rate_from_timestamps(&[0, 40, 80, 120, 160]);
        assert_eq!(fps, Some(25.0));
    }

    #[test]
    fn estimate_frame_rate_requires_more_than_sparse_samples() {
        assert_eq!(estimate_frame_rate_from_timestamps(&[0, 1000]), None);
        assert_eq!(estimate_frame_rate_from_timestamps(&[0, 1000, 2000]), None);
    }

    #[test]
    fn fallback_frame_rate_uses_timestamp_scale_timebase() {
        assert_eq!(
            fallback_frame_rate_from_timestamp_scale(1_000_000.0),
            Some(1000.0)
        );
        let approx_24fps = fallback_frame_rate_from_timestamp_scale(41_666_667.0).unwrap();
        assert!((approx_24fps - 24.0).abs() < 0.001);
        assert_eq!(fallback_frame_rate_from_timestamp_scale(0.0), None);
    }

    #[test]
    fn only_replaces_clearly_bogus_existing_frame_rates() {
        assert!(should_replace_frame_rate(None, 24.0));
        assert!(should_replace_frame_rate(Some(1.0), 1000.0));
        assert!(!should_replace_frame_rate(Some(24.0), 6.0));
        assert!(!should_replace_frame_rate(Some(23.976), 24.0));
    }
}
