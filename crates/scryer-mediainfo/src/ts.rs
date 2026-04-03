use crate::MediaInfoError;
use crate::probe::ProbeBudget;
use crate::types::{RawContainer, RawTrack, TrackKind};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Size of a single MPEG-TS packet.
const TS_PACKET_SIZE: usize = 188;
/// TS sync byte.
const SYNC_BYTE: u8 = 0x47;
/// PID of the Program Association Table.
const PAT_PID: u16 = 0x0000;
/// PTS clock rate (90 kHz).
const PTS_HZ: f64 = 90_000.0;
const FAST_DURATION_PROBE_PACKETS: usize = 10_000;
const FALLBACK_DURATION_PROBE_PACKETS: usize = 50_000;
const STREAM_PROBE_PACKET_LIMIT: usize = 20_000;
const STREAM_PROBE_MAX_BYTES_PER_PID: u64 = 256 * 1024;
const DOVI_VIDEO_STREAM_DESCRIPTOR: u8 = 0xB0;
const AC3_CHANNELS_BY_ACMOD: [u8; 8] = [2, 1, 2, 3, 3, 4, 4, 5];
const AC3_SAMPLE_RATES: [u32; 4] = [48_000, 44_100, 32_000, 0];
const AC3_BITRATES_KBPS: [u32; 19] = [
    32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 576, 640,
];
const EAC3_BLOCKS: [u32; 4] = [1, 2, 3, 6];
const MPEG_VIDEO_FRAME_RATES: [Option<f64>; 16] = [
    None,
    Some(24000.0 / 1001.0),
    Some(24.0),
    Some(25.0),
    Some(30000.0 / 1001.0),
    Some(30.0),
    Some(50.0),
    Some(60000.0 / 1001.0),
    Some(60.0),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];
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
const DTS_SAMPLE_RATES: [u32; 16] = [
    0, 8_000, 16_000, 32_000, 0, 0, 11_025, 22_050, 44_100, 0, 0, 12_000, 24_000, 48_000, 96_000,
    192_000,
];
const DTS_BIT_RATES: [u32; 32] = [
    32_000, 56_000, 64_000, 96_000, 112_000, 128_000, 192_000, 224_000, 256_000, 320_000, 384_000,
    448_000, 512_000, 576_000, 640_000, 768_000, 896_000, 1_024_000, 1_152_000, 1_280_000,
    1_344_000, 1_408_000, 1_411_200, 1_472_000, 1_536_000, 1_920_000, 2_048_000, 3_072_000,
    3_840_000, 1, 2, 3,
];
const DTS_CHANNELS: [u8; 16] = [1, 2, 2, 2, 2, 3, 3, 4, 4, 5, 6, 6, 6, 7, 8, 8];
const AAC_CHANNEL_CONFIGS: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 8, 0, 0, 0, 7, 8, 0, 8, 0];
const AAC_SAMPLE_RATES: [u32; 16] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350, 0, 0, 0,
];

/// Parse an MPEG Transport Stream file and extract stream metadata.
pub(crate) fn parse_ts(path: &Path) -> Result<RawContainer, MediaInfoError> {
    let mut file = std::fs::File::open(path).map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let file_size = file
        .metadata()
        .map_err(|e| MediaInfoError::Io(e.to_string()))?
        .len();

    let pmt_pid = find_pmt_pid(&mut file)?;
    let es_entries = parse_pmt(&mut file, pmt_pid)?;
    let mut tracks: Vec<RawTrack> = es_entries.iter().map(build_track).collect();

    let duration_seconds = estimate_duration(&mut file, file_size, &es_entries);
    enrich_tracks_from_probe(&mut file, &es_entries, &mut tracks)?;

    if file_size > 0
        && let Some(duration_seconds) = duration_seconds
        && duration_seconds > 0.0
        && let Some(video_track) = tracks
            .iter_mut()
            .find(|track| track.kind == TrackKind::Video)
        && video_track.bit_rate_bps.is_none()
    {
        video_track.bit_rate_bps = Some((file_size as f64 * 8.0 / duration_seconds) as i64);
    }

    Ok(RawContainer {
        format_name: "mpegts".into(),
        duration_seconds,
        num_chapters: None,
        tracks,
    })
}

#[derive(Clone)]
struct EsEntry {
    stream_type: u8,
    pid: u16,
    descriptors: Vec<u8>,
    dovi_config: Option<Vec<u8>>,
}

// ---------------------------------------------------------------------------
// PAT parsing
// ---------------------------------------------------------------------------

fn find_pmt_pid<T: Read + Seek>(stream: &mut T) -> Result<u16, MediaInfoError> {
    stream
        .seek(SeekFrom::Start(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut buf = [0u8; TS_PACKET_SIZE];
    let mut packets_scanned = 0u32;

    loop {
        if packets_scanned > 100_000 {
            return Err(MediaInfoError::Parse(
                "PAT not found within first 100k packets".into(),
            ));
        }

        let n = read_full(stream, &mut buf);
        if n < TS_PACKET_SIZE {
            return Err(MediaInfoError::Parse(
                "PAT not found before end of file".into(),
            ));
        }
        packets_scanned += 1;

        if buf[0] != SYNC_BYTE && !resync(stream, &mut buf)? {
            return Err(MediaInfoError::Parse("could not sync to TS packets".into()));
        }

        if ts_pid(&buf) != PAT_PID || buf[1] & 0x40 == 0 {
            continue;
        }

        let payload = ts_payload(&buf);
        if payload.is_empty() {
            continue;
        }

        let pointer = payload[0] as usize;
        let section_start = 1 + pointer;
        if section_start >= payload.len() {
            continue;
        }
        let section = &payload[section_start..];
        if section.is_empty() || section[0] != 0x00 || section.len() < 8 {
            continue;
        }

        let section_length = ((section[1] as u16 & 0x0F) << 8 | section[2] as u16) as usize;
        let available = section.len().saturating_sub(3);
        let data_len = section_length.min(available);
        if data_len < 9 {
            continue;
        }

        let program_data = &section[8..3 + data_len.saturating_sub(4)];
        for chunk in program_data.chunks_exact(4) {
            let program_number = (chunk[0] as u16) << 8 | chunk[1] as u16;
            let entry_pid = (chunk[2] as u16 & 0x1F) << 8 | chunk[3] as u16;
            if program_number != 0 {
                return Ok(entry_pid);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PMT parsing
// ---------------------------------------------------------------------------

fn parse_pmt<T: Read + Seek>(stream: &mut T, pmt_pid: u16) -> Result<Vec<EsEntry>, MediaInfoError> {
    stream
        .seek(SeekFrom::Start(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut buf = [0u8; TS_PACKET_SIZE];
    let mut packets_scanned = 0u32;

    loop {
        if packets_scanned > 200_000 {
            return Err(MediaInfoError::Parse(
                "PMT not found within first 200k packets".into(),
            ));
        }

        let n = read_full(stream, &mut buf);
        if n < TS_PACKET_SIZE {
            return Err(MediaInfoError::Parse(
                "PMT not found before end of file".into(),
            ));
        }
        packets_scanned += 1;

        if buf[0] != SYNC_BYTE && !resync(stream, &mut buf)? {
            return Err(MediaInfoError::Parse(
                "could not sync to TS packets for PMT".into(),
            ));
        }

        if ts_pid(&buf) != pmt_pid || buf[1] & 0x40 == 0 {
            continue;
        }

        let payload = ts_payload(&buf);
        if payload.is_empty() {
            continue;
        }

        let pointer = payload[0] as usize;
        let section_start = 1 + pointer;
        if section_start >= payload.len() {
            continue;
        }
        let section = &payload[section_start..];
        if section.is_empty() || section[0] != 0x02 || section.len() < 12 {
            continue;
        }

        let section_length = ((section[1] as u16 & 0x0F) << 8 | section[2] as u16) as usize;
        let available = section.len().saturating_sub(3);
        let data_len = section_length.min(available);
        let program_info_length = ((section[10] as u16 & 0x0F) << 8 | section[11] as u16) as usize;
        let es_start = 12 + program_info_length;
        let es_end = (3 + data_len).saturating_sub(4);
        if es_start > section.len() || es_end > section.len() || es_start > es_end {
            continue;
        }

        let es_data = &section[es_start..es_end];
        let mut entries = Vec::new();
        let mut pos = 0;

        while pos + 5 <= es_data.len() {
            let stream_type = es_data[pos];
            let es_pid = ((es_data[pos + 1] as u16 & 0x1F) << 8) | es_data[pos + 2] as u16;
            let es_info_length =
                ((es_data[pos + 3] as u16 & 0x0F) << 8 | es_data[pos + 4] as u16) as usize;
            let desc_end = (pos + 5 + es_info_length).min(es_data.len());
            let descriptors = es_data[pos + 5..desc_end].to_vec();

            entries.push(EsEntry {
                stream_type,
                pid: es_pid,
                dovi_config: extract_dovi_config(&descriptors),
                descriptors,
            });

            pos = desc_end;
        }

        return Ok(entries);
    }
}

// ---------------------------------------------------------------------------
// Track building
// ---------------------------------------------------------------------------

fn build_track(es: &EsEntry) -> RawTrack {
    let (kind, codec_name) = classify_stream_type(es.stream_type, &es.descriptors);

    RawTrack {
        kind,
        codec_id: format!("0x{:02X}", es.stream_type),
        codec_name: Some(codec_name.to_owned()),
        codec_private: None,
        width: None,
        height: None,
        channels: None,
        bit_rate_bps: None,
        language: extract_language(&es.descriptors),
        frame_rate_fps: None,
        color_transfer: None,
        dovi_config: es.dovi_config.clone(),
        has_hdr10plus: false,
        name: None,
        forced: false,
        default_track: false,
    }
}

fn classify_stream_type(stream_type: u8, descriptors: &[u8]) -> (TrackKind, &'static str) {
    match stream_type {
        0x01 => (TrackKind::Video, "mpeg1video"),
        0x02 => (TrackKind::Video, "mpeg2video"),
        0x10 => (TrackKind::Video, "mpeg4"),
        0x1B => (TrackKind::Video, "h264"),
        0x24 => (TrackKind::Video, "hevc"),
        0x42 => (TrackKind::Video, "cavs"),
        0xD2 => (TrackKind::Video, "avs2"),
        0xD4 => (TrackKind::Video, "avs3"),
        0xDB => (TrackKind::Video, "h264"),
        0xEA => (TrackKind::Video, "vc1"),
        0x03 | 0x04 => (TrackKind::Audio, "mp2"),
        0x0F => (TrackKind::Audio, "aac"),
        0x11 => (TrackKind::Audio, "aac_latm"),
        0x81 => (TrackKind::Audio, "ac3"),
        0x82 | 0x85 | 0xA2 => (TrackKind::Audio, "dts"),
        0x83 => (TrackKind::Audio, "truehd"),
        0x84 | 0x87 | 0xA1 | 0xC2 => (TrackKind::Audio, "eac3"),
        0x90 => (TrackKind::Subtitle, "hdmv_pgs_subtitle"),
        0x92 => (TrackKind::Subtitle, "hdmv_text_subtitle"),
        0xC1 => (TrackKind::Audio, "ac3"),
        0xCF => (TrackKind::Audio, "aac"),
        0x06 => classify_private_pes(descriptors),
        _ if stream_type >= 0x80 => (TrackKind::Video, "unknown"),
        _ => (TrackKind::Video, "unknown"),
    }
}

fn classify_private_pes(descriptors: &[u8]) -> (TrackKind, &'static str) {
    let mut pos = 0;
    while pos + 2 <= descriptors.len() {
        let tag = descriptors[pos];
        let len = descriptors[pos + 1] as usize;
        let desc_end = (pos + 2 + len).min(descriptors.len());

        match tag {
            0x6A => return (TrackKind::Audio, "ac3"),
            0x7A => return (TrackKind::Audio, "eac3"),
            0x7B => return (TrackKind::Audio, "dts"),
            0x7C => return (TrackKind::Audio, "aac"),
            0x59 => return (TrackKind::Subtitle, "dvb_subtitle"),
            0x56 => return (TrackKind::Subtitle, "dvb_teletext"),
            _ => {}
        }

        pos = desc_end;
    }
    (TrackKind::Audio, "unknown")
}

fn extract_language(descriptors: &[u8]) -> Option<String> {
    let mut pos = 0;
    while pos + 2 <= descriptors.len() {
        let tag = descriptors[pos];
        let len = descriptors[pos + 1] as usize;
        let desc_end = (pos + 2 + len).min(descriptors.len());

        if tag == 0x0A && len >= 4 && pos + 5 <= descriptors.len() {
            let lang = std::str::from_utf8(&descriptors[pos + 2..pos + 5])
                .ok()
                .map(|s| s.trim_end_matches('\0').to_owned())
                .filter(|s| !s.is_empty());
            if lang.is_some() {
                return lang;
            }
        }

        pos = desc_end;
    }
    None
}

fn extract_dovi_config(descriptors: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 0;
    while pos + 2 <= descriptors.len() {
        let tag = descriptors[pos];
        let len = descriptors[pos + 1] as usize;
        let data_start = pos + 2;
        let desc_end = (data_start + len).min(descriptors.len());

        if tag == DOVI_VIDEO_STREAM_DESCRIPTOR && desc_end.saturating_sub(data_start) >= 4 {
            let data = &descriptors[data_start..desc_end];
            let flags = u16::from_be_bytes([data[2], data[3]]);
            let bl_present_flag = (flags & 0x01) != 0;

            let mut cursor = 4;
            if !bl_present_flag && data.len() >= cursor + 2 {
                cursor += 2;
            }

            let compat_and_compression = data.get(cursor).copied().unwrap_or(0);
            return Some(vec![
                data[0],
                data[1],
                data[2],
                data[3],
                compat_and_compression,
            ]);
        }

        pos = desc_end;
    }
    None
}

// ---------------------------------------------------------------------------
// Stream probing
// ---------------------------------------------------------------------------

fn enrich_tracks_from_probe<T: Read + Seek>(
    stream: &mut T,
    es_entries: &[EsEntry],
    tracks: &mut [RawTrack],
) -> Result<(), MediaInfoError> {
    let mut buffers: HashMap<u16, Vec<u8>> = HashMap::new();
    let mut budgets: HashMap<u16, ProbeBudget> = HashMap::new();
    let mut pts_by_pid: HashMap<u16, Vec<u64>> = HashMap::new();
    for entry in es_entries {
        let kind = classify_stream_type(entry.stream_type, &entry.descriptors).0;
        if matches!(kind, TrackKind::Video | TrackKind::Audio) {
            buffers.insert(entry.pid, Vec::new());
            budgets.insert(entry.pid, ProbeBudget::new(STREAM_PROBE_MAX_BYTES_PER_PID));
        }
        if kind == TrackKind::Video {
            pts_by_pid.insert(entry.pid, Vec::new());
        }
    }
    if buffers.is_empty() {
        return Ok(());
    }

    stream
        .seek(SeekFrom::Start(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut pkt = [0_u8; TS_PACKET_SIZE];
    let mut packets_scanned = 0usize;

    while packets_scanned < STREAM_PROBE_PACKET_LIMIT
        && budgets.values().any(|budget| !budget.exhausted())
    {
        let n = read_full(stream, &mut pkt);
        if n < TS_PACKET_SIZE {
            break;
        }
        packets_scanned += 1;

        if pkt[0] != SYNC_BYTE && !resync(stream, &mut pkt)? {
            break;
        }

        let pid = ts_pid(&pkt);
        let Some(buffer) = buffers.get_mut(&pid) else {
            continue;
        };
        let Some(budget) = budgets.get_mut(&pid) else {
            continue;
        };
        if budget.exhausted() {
            continue;
        }

        let payload = ts_payload(&pkt);
        if payload.is_empty() {
            continue;
        }

        let payload = if (pkt[1] & 0x40) != 0 {
            if let Some(pts_values) = pts_by_pid.get_mut(&pid)
                && pts_values.len() < 8
                && let Some(pts) = extract_pts_from_pes(payload)
                && pts_values.last().copied() != Some(pts)
            {
                pts_values.push(pts);
            }
            strip_pes_header(payload).unwrap_or(payload)
        } else {
            payload
        };
        if payload.is_empty() {
            continue;
        }

        let take = budget.consume(payload.len());
        if take > 0 {
            buffer.extend_from_slice(&payload[..take]);
        }
    }

    for (entry, track) in es_entries.iter().zip(tracks.iter_mut()) {
        let Some(buffer) = buffers.get(&entry.pid) else {
            continue;
        };

        match track.codec_name.as_deref() {
            Some("h264") => probe_h264_track(buffer, track),
            Some("hevc") => probe_hevc_track(buffer, track),
            Some("mpeg1video") | Some("mpeg2video") => probe_mpeg_video_track(buffer, track),
            Some("aac") => probe_aac_track(buffer, track),
            Some("aac_latm") => probe_latm_track(buffer, track),
            Some("mp2") => probe_mpeg_audio_track(buffer, track),
            Some("ac3") => probe_ac3_track(buffer, track),
            Some("eac3") => probe_eac3_track(buffer, track),
            Some("dts") => probe_dts_track(buffer, track),
            _ => {}
        }

        if track.kind == TrackKind::Video && !is_plausible_frame_rate(track.frame_rate_fps) {
            track.frame_rate_fps = pts_by_pid
                .get(&entry.pid)
                .and_then(|pts_values| estimate_frame_rate_from_pts(pts_values));
        }
    }

    Ok(())
}

fn probe_h264_track(data: &[u8], track: &mut RawTrack) {
    let Some(sps_nal) = find_annexb_nal(data, |nal| (nal[0] & 0x1F) == 7) else {
        return;
    };
    let Ok(sps) = scuffle_h264::Sps::parse(std::io::Cursor::new(sps_nal)) else {
        return;
    };

    track.width = Some(sps.width() as i32);
    track.height = Some(sps.height() as i32);
    track.frame_rate_fps = sps.frame_rate();
    track.codec_private = Some(sps_nal.to_vec());
    track.color_transfer = sps.color_config.as_ref().and_then(|color| {
        let transfer = color.transfer_characteristics as u32;
        if transfer > 0 && transfer != 2 {
            Some(transfer)
        } else {
            None
        }
    });
}

fn probe_hevc_track(data: &[u8], track: &mut RawTrack) {
    let Some(sps_nal) = find_annexb_nal(data, |nal| ((nal[0] >> 1) & 0x3F) == 33) else {
        return;
    };
    let Ok(sps) = scuffle_h265::SpsNALUnit::parse(std::io::Cursor::new(sps_nal)) else {
        return;
    };

    track.width = Some(sps.rbsp.cropped_width() as i32);
    track.height = Some(sps.rbsp.cropped_height() as i32);
    track.codec_private = Some(sps_nal.to_vec());
    track.color_transfer = sps.rbsp.vui_parameters.as_ref().and_then(|vui| {
        let transfer = vui.video_signal_type.transfer_characteristics;
        if transfer > 0 && transfer != 2 {
            Some(transfer as u32)
        } else {
            None
        }
    });
}

fn probe_aac_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_adts_header(data) else {
        return;
    };
    track.channels = Some(header.channels as i32);
    track.bit_rate_bps = header.bit_rate_bps.map(|bitrate| bitrate as i64);
}

fn probe_latm_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_latm_header(data) else {
        return;
    };
    track.channels = Some(header.channels as i32);
}

fn probe_mpeg_video_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_mpeg_video_sequence_header(data) else {
        return;
    };
    track.width = Some(header.width as i32);
    track.height = Some(header.height as i32);
    track.frame_rate_fps = header.frame_rate_fps;
    if track.bit_rate_bps.is_none() {
        track.bit_rate_bps = header.bit_rate_bps.map(i64::from);
    }
}

fn probe_mpeg_audio_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_mpeg_audio_header(data) else {
        return;
    };
    track.channels = Some(header.channels as i32);
    track.bit_rate_bps = header.bit_rate_bps.map(i64::from);
}

fn probe_ac3_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_ac3_header(data) else {
        return;
    };
    track.channels = Some(header.channels as i32);
    track.bit_rate_bps = header.bit_rate_bps.map(i64::from);
}

fn probe_eac3_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_eac3_header(data) else {
        return;
    };
    track.channels = Some(header.channels as i32);
    track.bit_rate_bps = header.bit_rate_bps.map(i64::from);
}

fn probe_dts_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_dts_header(data) else {
        return;
    };
    track.channels = Some(header.channels as i32);
    if header.bit_rate_bps > 3 {
        track.bit_rate_bps = Some(i64::from(header.bit_rate_bps));
    }
}

fn is_plausible_frame_rate(frame_rate_fps: Option<f64>) -> bool {
    frame_rate_fps.is_some_and(|fps| (1.0..=240.0).contains(&fps))
}

fn estimate_frame_rate_from_pts(pts_values: &[u64]) -> Option<f64> {
    let mut sorted_pts = pts_values.to_vec();
    sorted_pts.sort_unstable();
    sorted_pts.dedup();

    let mut deltas: Vec<u64> = sorted_pts
        .windows(2)
        .filter_map(|window| window[1].checked_sub(window[0]))
        .filter(|delta| *delta > 0)
        .collect();
    if deltas.is_empty() {
        return None;
    }

    deltas.sort_unstable();
    let median_delta = deltas[deltas.len() / 2] as f64;
    let fps = PTS_HZ / median_delta;
    if is_plausible_frame_rate(Some(fps)) {
        Some(fps)
    } else {
        None
    }
}

struct AdtsHeader {
    channels: u8,
    bit_rate_bps: Option<u32>,
}

struct LatmHeader {
    channels: u8,
}

struct MpegVideoSequenceHeader {
    width: u16,
    height: u16,
    frame_rate_fps: Option<f64>,
    bit_rate_bps: Option<u32>,
}

struct MpegAudioHeader {
    channels: u8,
    bit_rate_bps: Option<u32>,
}

struct Ac3Header {
    channels: u8,
    bit_rate_bps: Option<u32>,
}

struct DtsHeader {
    channels: u8,
    bit_rate_bps: u32,
}

fn find_adts_header(data: &[u8]) -> Option<AdtsHeader> {
    if data.len() < 7 {
        return None;
    }
    for i in 0..=data.len() - 7 {
        let hdr = &data[i..];
        if hdr[0] != 0xFF || (hdr[1] & 0xF0) != 0xF0 {
            continue;
        }

        let sampling_frequency_index = (hdr[2] >> 2) & 0x0F;
        let sample_rate = match sampling_frequency_index {
            0 => 96_000,
            1 => 88_200,
            2 => 64_000,
            3 => 48_000,
            4 => 44_100,
            5 => 32_000,
            6 => 24_000,
            7 => 22_050,
            8 => 16_000,
            9 => 12_000,
            10 => 11_025,
            11 => 8_000,
            12 => 7_350,
            _ => continue,
        };

        let channels = ((hdr[2] & 0x01) << 2) | ((hdr[3] >> 6) & 0x03);
        if channels == 0 {
            continue;
        }

        let frame_length = (((hdr[3] & 0x03) as u32) << 11)
            | ((hdr[4] as u32) << 3)
            | ((hdr[5] as u32 >> 5) & 0x07);
        if frame_length < 7 {
            continue;
        }

        let number_of_raw_data_blocks = hdr[6] & 0x03;
        let samples_per_frame = 1024_u32 * (u32::from(number_of_raw_data_blocks) + 1);
        let bit_rate_bps = if samples_per_frame > 0 {
            Some(frame_length * 8 * sample_rate / samples_per_frame)
        } else {
            None
        };

        return Some(AdtsHeader {
            channels,
            bit_rate_bps,
        });
    }
    None
}

fn find_latm_header(data: &[u8]) -> Option<LatmHeader> {
    for start in 0..data.len().saturating_sub(3) {
        if data[start] != 0x56 || (data[start + 1] & 0xE0) != 0xE0 {
            continue;
        }

        let mut bits = BitReader::new(&data[start..]);
        if bits.read_bits(11)? != 0x2B7 {
            continue;
        }
        let _mux_length = bits.read_bits(13)?;
        if bits.read_bit()? != 0 {
            continue;
        }
        if bits.read_bit()? != 0 {
            continue;
        }
        let _all_streams_same_time_framing = bits.read_bit()?;
        if bits.read_bits(6)? != 0 || bits.read_bits(4)? != 0 || bits.read_bits(3)? != 0 {
            continue;
        }
        let audio_object_type = read_aac_audio_object_type(&mut bits)?;
        let _sample_rate = read_aac_sample_rate(&mut bits)?;
        let mut channel_config = bits.read_bits(4)? as usize;

        if matches!(audio_object_type, 5 | 29) {
            let _ext_sample_rate = read_aac_sample_rate(&mut bits)?;
            let ext_audio_object_type = read_aac_audio_object_type(&mut bits)?;
            if ext_audio_object_type == 22 {
                channel_config = bits.read_bits(4)? as usize;
            }
        }

        let channels = *AAC_CHANNEL_CONFIGS.get(channel_config)?;
        if channels == 0 {
            continue;
        }
        return Some(LatmHeader { channels });
    }

    None
}

fn read_aac_audio_object_type(bits: &mut BitReader<'_>) -> Option<u8> {
    let object_type = bits.read_bits(5)? as u8;
    if object_type == 31 {
        Some(32 + bits.read_bits(6)? as u8)
    } else {
        Some(object_type)
    }
}

fn read_aac_sample_rate(bits: &mut BitReader<'_>) -> Option<u32> {
    let sample_rate_index = bits.read_bits(4)? as usize;
    if sample_rate_index == 0xF {
        bits.read_bits(24)
    } else {
        AAC_SAMPLE_RATES.get(sample_rate_index).copied()
    }
}

fn find_mpeg_video_sequence_header(data: &[u8]) -> Option<MpegVideoSequenceHeader> {
    let payload = find_start_code_payload(data, 0xB3)?;
    if payload.len() < 8 {
        return None;
    }

    let width = ((u16::from(payload[0])) << 4) | u16::from(payload[1] >> 4);
    let height = ((u16::from(payload[1] & 0x0F)) << 8) | u16::from(payload[2]);
    let frame_rate_code = (payload[3] & 0x0F) as usize;
    let bit_rate_value = ((u32::from(payload[4])) << 10)
        | ((u32::from(payload[5])) << 2)
        | (u32::from(payload[6]) >> 6);

    Some(MpegVideoSequenceHeader {
        width,
        height,
        frame_rate_fps: MPEG_VIDEO_FRAME_RATES
            .get(frame_rate_code)
            .copied()
            .flatten(),
        bit_rate_bps: if bit_rate_value == 0 || bit_rate_value == 0x3_FFFF {
            None
        } else {
            Some(bit_rate_value * 400)
        },
    })
}

fn find_mpeg_audio_header(data: &[u8]) -> Option<MpegAudioHeader> {
    if data.len() < 4 {
        return None;
    }

    for i in 0..=data.len() - 4 {
        let header = u32::from_be_bytes(data[i..i + 4].try_into().ok()?);
        if (header & 0xFFE0_0000) != 0xFFE0_0000 {
            continue;
        }

        let version_id = ((header >> 19) & 0x3) as usize;
        let layer_index = ((header >> 17) & 0x3) as usize;
        let bitrate_index = ((header >> 12) & 0xF) as usize;
        let sample_rate_index = ((header >> 10) & 0x3) as usize;
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

        return Some(MpegAudioHeader {
            channels: MPEG_AUDIO_CHANNELS[channel_mode],
            bit_rate_bps: Some(bit_rate_kbps * 1000),
        });
    }

    None
}

fn find_ac3_header(data: &[u8]) -> Option<Ac3Header> {
    if data.len() < 7 {
        return None;
    }
    for start in 0..=data.len() - 7 {
        if data[start] != 0x0B || data[start + 1] != 0x77 {
            continue;
        }

        let bsid = data[start + 5] >> 3;
        if bsid > 10 {
            continue;
        }

        let fscod = (data[start + 4] >> 6) as usize;
        let frame_size_code = (data[start + 4] & 0x3F) as usize;
        if fscod == 3 || frame_size_code > 37 {
            continue;
        }

        let mut bits = BitReader::new(&data[start + 6..]);
        let acmod = bits.read_bits(3)? as usize;
        if acmod == 2 {
            bits.skip_bits(2)?;
        } else {
            if (acmod & 1) != 0 && acmod != 1 {
                bits.skip_bits(2)?;
            }
            if (acmod & 4) != 0 {
                bits.skip_bits(2)?;
            }
        }
        let lfe_on = bits.read_bit()? != 0;
        let sr_shift = usize::from(bsid.saturating_sub(8));
        let bit_rate_code = frame_size_code >> 1;

        return Some(Ac3Header {
            channels: AC3_CHANNELS_BY_ACMOD[acmod] + u8::from(lfe_on),
            bit_rate_bps: Some((AC3_BITRATES_KBPS[bit_rate_code] * 1000) >> sr_shift),
        });
    }

    None
}

fn find_eac3_header(data: &[u8]) -> Option<Ac3Header> {
    if data.len() < 6 {
        return None;
    }
    for start in 0..=data.len() - 6 {
        if data[start] != 0x0B || data[start + 1] != 0x77 {
            continue;
        }

        let bsid = data[start + 5] >> 3;
        if bsid <= 10 {
            continue;
        }

        let mut bits = BitReader::new(&data[start + 2..]);
        let frame_type = bits.read_bits(2)?;
        if frame_type == 3 {
            continue;
        }
        bits.skip_bits(3)?; // substream id
        let frame_size = (bits.read_bits(11)? + 1) * 2;

        let fscod = bits.read_bits(2)? as usize;
        let (sample_rate, num_blocks) = if fscod == 3 {
            let sample_rate = match bits.read_bits(2)? {
                0 => 24_000,
                1 => 22_050,
                2 => 16_000,
                _ => continue,
            };
            (sample_rate, 6)
        } else {
            let sample_rate = *AC3_SAMPLE_RATES.get(fscod)?;
            if sample_rate == 0 {
                continue;
            }
            let num_blocks = *EAC3_BLOCKS.get(bits.read_bits(2)? as usize)?;
            (sample_rate, num_blocks)
        };

        let acmod = bits.read_bits(3)? as usize;
        let lfe_on = bits.read_bit()? != 0;

        return Some(Ac3Header {
            channels: AC3_CHANNELS_BY_ACMOD[acmod] + u8::from(lfe_on),
            bit_rate_bps: Some((8 * frame_size * sample_rate) / (num_blocks * 256)),
        });
    }

    None
}

fn find_dts_header(data: &[u8]) -> Option<DtsHeader> {
    if data.len() < 11 {
        return None;
    }
    for start in 0..=data.len() - 11 {
        let mut bits = BitReader::new(&data[start..]);
        if bits.read_bits(32)? != 0x7FFE_8001 {
            continue;
        }
        bits.read_bit()?; // normal frame flag
        let deficit_samples = bits.read_bits(5)? as u8 + 1;
        if deficit_samples != 32 {
            continue;
        }
        bits.skip_bits(1)?; // crc present
        let npcmblocks = bits.read_bits(7)? as u8 + 1;
        if (npcmblocks & 0x1F) != 0 {
            continue;
        }
        let frame_size = bits.read_bits(14)? + 1;
        if frame_size < 96 {
            continue;
        }
        let audio_mode = bits.read_bits(6)? as usize;
        if audio_mode >= DTS_CHANNELS.len() {
            continue;
        }
        let sample_rate_code = bits.read_bits(4)? as usize;
        if *DTS_SAMPLE_RATES.get(sample_rate_code)? == 0 {
            continue;
        }
        let bit_rate_code = bits.read_bits(5)? as usize;
        if bit_rate_code >= DTS_BIT_RATES.len() {
            continue;
        }
        if bits.read_bit()? != 0 {
            continue;
        }
        bits.skip_bits(1 + 1 + 1 + 1 + 3 + 1 + 1)?; // drc/timestamp/aux/hdcd/ext/syncssf
        let lfe_present = bits.read_bits(2)? as u8;
        if lfe_present == 0x3 {
            continue;
        }

        return Some(DtsHeader {
            channels: DTS_CHANNELS[audio_mode] + u8::from(lfe_present > 0),
            bit_rate_bps: DTS_BIT_RATES[bit_rate_code],
        });
    }

    None
}

fn find_start_code_payload(data: &[u8], code: u8) -> Option<&[u8]> {
    for i in 0..data.len().saturating_sub(4) {
        if data[i] == 0x00 && data[i + 1] == 0x00 && data[i + 2] == 0x01 && data[i + 3] == code {
            return data.get(i + 4..);
        }
    }
    None
}

struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn read_bit(&mut self) -> Option<u8> {
        Some(self.read_bits(1)? as u8)
    }

    fn read_bits(&mut self, count: usize) -> Option<u32> {
        if count == 0 || count > 32 || self.bit_pos + count > self.data.len() * 8 {
            return None;
        }

        let mut value = 0_u32;
        for _ in 0..count {
            let byte_index = self.bit_pos / 8;
            let bit_index = 7 - (self.bit_pos % 8);
            value = (value << 1) | u32::from((self.data[byte_index] >> bit_index) & 0x01);
            self.bit_pos += 1;
        }

        Some(value)
    }

    fn skip_bits(&mut self, count: usize) -> Option<()> {
        if self.bit_pos + count > self.data.len() * 8 {
            return None;
        }
        self.bit_pos += count;
        Some(())
    }
}

fn find_annexb_nal(data: &[u8], predicate: impl Fn(&[u8]) -> bool) -> Option<&[u8]> {
    let mut i = 0;
    while i + 4 <= data.len() {
        let start_code_len = if data[i..].starts_with(&[0, 0, 1]) {
            3
        } else if data[i..].starts_with(&[0, 0, 0, 1]) {
            4
        } else {
            i += 1;
            continue;
        };

        let nal_start = i + start_code_len;
        let mut nal_end = data.len();
        let mut j = nal_start;
        while j + 3 < data.len() {
            if data[j..].starts_with(&[0, 0, 1]) || data[j..].starts_with(&[0, 0, 0, 1]) {
                nal_end = j;
                break;
            }
            j += 1;
        }

        if nal_end > nal_start {
            let nal = &data[nal_start..nal_end];
            if !nal.is_empty() && predicate(nal) {
                return Some(nal);
            }
        }
        i = nal_end;
    }
    None
}

fn strip_pes_header(payload: &[u8]) -> Option<&[u8]> {
    if payload.len() < 9 {
        return None;
    }
    if payload[0] != 0x00 || payload[1] != 0x00 || payload[2] != 0x01 {
        return None;
    }
    let header_len = 9 + payload[8] as usize;
    payload.get(header_len..)
}

// ---------------------------------------------------------------------------
// Duration estimation via PTS
// ---------------------------------------------------------------------------

fn estimate_duration<T: Read + Seek>(
    stream: &mut T,
    file_size: u64,
    es_entries: &[EsEntry],
) -> Option<f64> {
    estimate_duration_with_limit(stream, file_size, es_entries, FAST_DURATION_PROBE_PACKETS)
        .or_else(|| {
            estimate_duration_with_limit(
                stream,
                file_size,
                es_entries,
                FALLBACK_DURATION_PROBE_PACKETS,
            )
        })
}

fn estimate_duration_with_limit<T: Read + Seek>(
    stream: &mut T,
    file_size: u64,
    es_entries: &[EsEntry],
    packet_limit: usize,
) -> Option<f64> {
    if es_entries.is_empty() || file_size < TS_PACKET_SIZE as u64 {
        return None;
    }

    let pes_pids: Vec<u16> = es_entries
        .iter()
        .filter(|entry| is_pes_stream_type(entry.stream_type))
        .map(|entry| entry.pid)
        .collect();
    if pes_pids.is_empty() {
        return None;
    }

    let first_pts = find_pts_near(stream, 0, true, &pes_pids, packet_limit);
    let tail_start = file_size.saturating_sub(TS_PACKET_SIZE as u64 * packet_limit as u64);
    let last_pts = find_pts_near(stream, tail_start, false, &pes_pids, packet_limit);

    match (first_pts, last_pts) {
        (Some(first), Some(last)) if last > first => Some((last - first) as f64 / PTS_HZ),
        (Some(first), Some(last)) if last <= first => {
            let wrapped = (1u64 << 33) - first + last;
            Some(wrapped as f64 / PTS_HZ)
        }
        _ => None,
    }
}

fn is_pes_stream_type(stream_type: u8) -> bool {
    matches!(
        stream_type,
        0x01 | 0x02 | 0x03 | 0x04 | 0x06 | 0x0F | 0x10 | 0x11 | 0x1B | 0x24 | 0x81 | 0x87
    )
}

fn find_pts_near<T: Read + Seek>(
    stream: &mut T,
    start_pos: u64,
    first_match: bool,
    pes_pids: &[u16],
    max_packets: usize,
) -> Option<u64> {
    stream.seek(SeekFrom::Start(start_pos)).ok()?;
    let read_size = max_packets * TS_PACKET_SIZE;
    let mut data = vec![0u8; read_size];
    let n = read_full(stream, &mut data);
    data.truncate(n);

    let mut result = None;
    let mut offset = 0;
    while offset < data.len() && data[offset] != SYNC_BYTE {
        offset += 1;
    }

    while offset + TS_PACKET_SIZE <= data.len() {
        if data[offset] != SYNC_BYTE {
            offset += 1;
            continue;
        }

        let pkt = &data[offset..offset + TS_PACKET_SIZE];
        let pid = ts_pid(pkt);
        if pes_pids.contains(&pid) && (pkt[1] & 0x40) != 0 {
            let payload = ts_payload(pkt);
            if let Some(pts) = extract_pts_from_pes(payload) {
                if first_match {
                    return Some(pts);
                }
                result = Some(pts);
            }
        }

        offset += TS_PACKET_SIZE;
    }

    result
}

fn extract_pts_from_pes(payload: &[u8]) -> Option<u64> {
    if payload.len() < 14 {
        return None;
    }
    if payload[0] != 0x00 || payload[1] != 0x00 || payload[2] != 0x01 {
        return None;
    }
    let pts_dts_flags = (payload[7] >> 6) & 0x03;
    if pts_dts_flags < 2 {
        return None;
    }
    parse_pts_bytes(&payload[9..14])
}

fn parse_pts_bytes(data: &[u8]) -> Option<u64> {
    if data[0] & 0x01 == 0 || data[2] & 0x01 == 0 || data[4] & 0x01 == 0 {
        return None;
    }

    Some(
        ((data[0] as u64 >> 1) & 0x07) << 30
            | (data[1] as u64) << 22
            | ((data[2] as u64 >> 1) & 0x7F) << 15
            | (data[3] as u64) << 7
            | (data[4] as u64 >> 1) & 0x7F,
    )
}

// ---------------------------------------------------------------------------
// TS packet helpers
// ---------------------------------------------------------------------------

fn ts_pid(pkt: &[u8]) -> u16 {
    ((pkt[1] as u16 & 0x1F) << 8) | pkt[2] as u16
}

fn ts_payload(pkt: &[u8]) -> &[u8] {
    let adaptation_field_control = (pkt[3] >> 4) & 0x03;
    let offset = match adaptation_field_control {
        0b01 => 4,
        0b11 => {
            let af_length = pkt[4] as usize;
            5 + af_length
        }
        _ => return &[],
    };

    if offset >= TS_PACKET_SIZE {
        &[]
    } else {
        &pkt[offset..]
    }
}

fn resync<T: Read + Seek>(
    stream: &mut T,
    first_packet: &mut [u8; TS_PACKET_SIZE],
) -> Result<bool, MediaInfoError> {
    let current = stream
        .stream_position()
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;
    let rewind = current.saturating_sub((TS_PACKET_SIZE - 1) as u64);
    stream
        .seek(SeekFrom::Start(rewind))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut byte = [0u8; 1];
    for _ in 0..TS_PACKET_SIZE {
        if read_full(stream, &mut byte) < 1 {
            return Ok(false);
        }
        if byte[0] == SYNC_BYTE {
            stream
                .seek(SeekFrom::Current(-1))
                .map_err(|e| MediaInfoError::Io(e.to_string()))?;
            if read_full(stream, first_packet) == TS_PACKET_SIZE && first_packet[0] == SYNC_BYTE {
                return Ok(true);
            }
            stream
                .seek(SeekFrom::Current(-(TS_PACKET_SIZE as i64) + 1))
                .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        }
    }

    Ok(false)
}

fn read_full<T: Read>(reader: &mut T, buf: &mut [u8]) -> usize {
    let mut total = 0;
    while total < buf.len() {
        match reader.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(_) => break,
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dovi_descriptor_payload() {
        let descriptors = [
            DOVI_VIDEO_STREAM_DESCRIPTOR,
            5,
            1,
            0,
            0b0000_1111,
            0b0001_1101,
            0b1011_0000,
        ];
        let dovi = extract_dovi_config(&descriptors).unwrap();
        assert_eq!(dovi, vec![1, 0, 0b0000_1111, 0b0001_1101, 0b1011_0000]);
    }

    #[test]
    fn parses_adts_header_channels_and_bitrate() {
        let data = [0xFF, 0xF1, 0x50, 0x80, 0x10, 0x1F, 0xFC];
        let header = find_adts_header(&data).unwrap();
        assert_eq!(header.channels, 2);
        assert!(header.bit_rate_bps.is_some());
    }

    #[test]
    fn parses_mpeg2_video_sequence_header() {
        let data = [
            0x00, 0x00, 0x01, 0xB3, 0x2D, 0x01, 0xE0, 0x34, 0x00, 0x40, 0x00, 0x00,
        ];
        let header = find_mpeg_video_sequence_header(&data).unwrap();
        assert_eq!(header.width, 720);
        assert_eq!(header.height, 480);
        assert_eq!(header.frame_rate_fps, Some(30000.0 / 1001.0));
        assert!(header.bit_rate_bps.is_some());
    }

    #[test]
    fn parses_mpeg_audio_header() {
        let data = [0xFF, 0xFD, 0x84, 0x80];
        let header = find_mpeg_audio_header(&data).unwrap();
        assert_eq!(header.channels, 2);
        assert_eq!(header.bit_rate_bps, Some(128_000));
    }

    #[test]
    fn parses_ac3_header_channels_and_bitrate() {
        let data = [0x0B, 0x77, 0x00, 0x00, 0x0A, 0x40, 0x50];
        let header = find_ac3_header(&data).unwrap();
        assert_eq!(header.channels, 2);
        assert_eq!(header.bit_rate_bps, Some(80_000));
    }

    #[test]
    fn parses_eac3_header_channels_and_bitrate() {
        let data = [0x0B, 0x77, 0x00, 0x0F, 0x34, 0x80];
        let header = find_eac3_header(&data).unwrap();
        assert_eq!(header.channels, 2);
        assert_eq!(header.bit_rate_bps, Some(8_000));
    }

    #[test]
    fn parses_dts_core_header_channels_and_bitrate() {
        let data = [
            0x7F, 0xFE, 0x80, 0x01, 0x7C, 0x7C, 0x05, 0xF2, 0xB7, 0x00, 0x00,
        ];
        let header = find_dts_header(&data).unwrap();
        assert_eq!(header.channels, 6);
        assert_eq!(header.bit_rate_bps, 1_536_000);
    }

    #[test]
    fn parses_latm_header_channels() {
        let data = [0x56, 0xE0, 0x06, 0x20, 0x00, 0x12, 0x10];
        let header = find_latm_header(&data).unwrap();
        assert_eq!(header.channels, 2);
    }
}
