use crate::codec::{
    detect_audio_profile_from_payload, detect_dts_channels_from_probe_bytes, merge_audio_profile,
};
use crate::probe::ProbeBudget;
use crate::scan;
use crate::types::{RawContainer, RawTrack, TrackKind};
use crate::{AnalysisProfile, MediaInfoError};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Size of a transport payload packet without any outer framing.
const TS_PACKET_SIZE: usize = 188;
const TS_PID_COUNT: usize = 8192;
/// Blu-ray / DVHS transport packets carry a 4-byte prefix ahead of the TS sync byte.
const TS_DVHS_PACKET_SIZE: usize = 192;
/// Reed-Solomon protected transport packets carry 16 bytes of trailing FEC data.
const TS_FEC_PACKET_SIZE: usize = 204;
/// TS sync byte.
const SYNC_BYTE: u8 = 0x47;
/// PID of the Program Association Table.
const PAT_PID: u16 = 0x0000;
/// PTS clock rate (90 kHz).
const PTS_HZ: f64 = 90_000.0;
const FAST_DURATION_PROBE_PACKETS: usize = 10_000;
const FALLBACK_DURATION_PROBE_PACKETS: usize = 50_000;
const PROGRAM_MAP_PROBE_PACKETS: u32 = 200_000;
const STREAM_PROBE_PACKET_LIMIT: usize = 20_000;
const STREAM_PROBE_BATCH_PACKETS: usize = 256;
const STREAM_PROBE_MAX_BYTES_PER_PID: u64 = 256 * 1024;
const STREAM_PROBE_ROLLING_KEEP_BYTES: usize = 64 * 1024;
const DOVI_VIDEO_STREAM_DESCRIPTOR: u8 = 0xB0;
const ISO_639_LANGUAGE_DESCRIPTOR: u8 = 0x0A;
const TELETEXT_DESCRIPTOR: u8 = 0x56;
const SUBTITLING_DESCRIPTOR: u8 = 0x59;
const DVB_EXTENSION_DESCRIPTOR: u8 = 0x7F;
const SUPPLEMENTARY_AUDIO_DESCRIPTOR: u8 = 0x06;
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
const DTS_SYNCWORD_CORE_BE: u32 = 0x7FFE_8001;
const DTS_SYNCWORD_CORE_LE: u32 = 0xFE7F_0180;
const DTS_SYNCWORD_CORE_14B_BE: u32 = 0x1FFF_E800;
const DTS_SYNCWORD_CORE_14B_LE: u32 = 0xFF1F_00E8;
const DTS_HEADER_PROBE_BYTES: usize = 32;
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

#[derive(Clone, Copy)]
struct TsPacketLayout {
    raw_packet_size: usize,
    sync_offset: usize,
}

/// Parse an MPEG Transport Stream file and extract stream metadata.
pub(crate) fn parse_ts(
    path: &Path,
    profile: AnalysisProfile,
) -> Result<RawContainer, MediaInfoError> {
    let mut file = std::fs::File::open(path).map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let file_size = file
        .seek(SeekFrom::End(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let layout = detect_ts_packet_layout(&mut file)?;
    let es_entries = parse_program_map(&mut file, layout)?;
    let mut tracks: Vec<RawTrack> = es_entries.iter().map(build_track).collect();

    let first_probe_pts = if profile == AnalysisProfile::ContentProbe {
        None
    } else {
        enrich_tracks_from_probe(&mut file, &es_entries, &mut tracks, layout)?
    };
    let duration_seconds =
        estimate_duration(&mut file, file_size, &es_entries, layout, first_probe_pts);

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
// Program map parsing
// ---------------------------------------------------------------------------

fn parse_program_map<T: Read + Seek>(
    stream: &mut T,
    layout: TsPacketLayout,
) -> Result<Vec<EsEntry>, MediaInfoError> {
    stream
        .seek(SeekFrom::Start(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut raw_packet = vec![0u8; layout.raw_packet_size];
    let mut packet = [0u8; TS_PACKET_SIZE];
    let mut packets_scanned = 0u32;
    let mut pat = PsiSectionAssembler::new(0x00);
    let mut pmt = PsiSectionAssembler::new(0x02);
    let mut pmt_pid = None;

    loop {
        if packets_scanned >= PROGRAM_MAP_PROBE_PACKETS {
            return Err(MediaInfoError::Parse(format!(
                "program map not found within first {PROGRAM_MAP_PROBE_PACKETS} packets"
            )));
        }

        if !read_ts_packet(stream, layout, &mut raw_packet, &mut packet)? {
            return Err(MediaInfoError::Parse(
                "program map not found before end of file".into(),
            ));
        }
        packets_scanned += 1;

        let pid = ts_pid(&packet);
        if pid == PAT_PID {
            if let Some(section) = pat.push_packet(&packet)? {
                let next_pid = parse_pat_pmt_pid(&section)?;
                if pmt_pid != Some(next_pid) {
                    pmt.reset();
                    pmt_pid = Some(next_pid);
                }
            }
        } else if Some(pid) == pmt_pid
            && let Some(section) = pmt.push_packet(&packet)?
        {
            return parse_pmt_section(&section);
        }
    }
}

fn parse_pat_pmt_pid(section: &[u8]) -> Result<u16, MediaInfoError> {
    if section.len() < 12 {
        return Err(MediaInfoError::Parse("PAT section too short".into()));
    }

    let section_length = ((section[1] as u16 & 0x0F) << 8 | section[2] as u16) as usize;
    if section_length < 9 {
        return Err(MediaInfoError::Parse("PAT section length too short".into()));
    }

    let program_end = 3 + section_length.saturating_sub(4);
    if program_end > section.len() || program_end < 8 {
        return Err(MediaInfoError::Parse("PAT program table truncated".into()));
    }

    let program_data = &section[8..program_end];
    for chunk in program_data.chunks_exact(4) {
        let program_number = (chunk[0] as u16) << 8 | chunk[1] as u16;
        let entry_pid = (chunk[2] as u16 & 0x1F) << 8 | chunk[3] as u16;
        if program_number != 0 {
            return Ok(entry_pid);
        }
    }

    Err(MediaInfoError::Parse(
        "PAT did not contain a program map PID".into(),
    ))
}

fn parse_pmt_section(section: &[u8]) -> Result<Vec<EsEntry>, MediaInfoError> {
    if section.len() < 16 {
        return Err(MediaInfoError::Parse("PMT section too short".into()));
    }

    let section_length = ((section[1] as u16 & 0x0F) << 8 | section[2] as u16) as usize;
    if section_length < 13 {
        return Err(MediaInfoError::Parse("PMT section length too short".into()));
    }

    let program_info_length = ((section[10] as u16 & 0x0F) << 8 | section[11] as u16) as usize;
    let es_start = 12 + program_info_length;
    let es_end = 3 + section_length.saturating_sub(4);
    if es_start > section.len() || es_end > section.len() || es_start > es_end {
        return Err(MediaInfoError::Parse(
            "PMT elementary stream table truncated".into(),
        ));
    }

    let es_data = &section[es_start..es_end];
    let mut entries = Vec::new();
    let mut pos = 0;

    while pos + 5 <= es_data.len() {
        let stream_type = es_data[pos];
        let es_pid = ((es_data[pos + 1] as u16 & 0x1F) << 8) | es_data[pos + 2] as u16;
        let es_info_length =
            ((es_data[pos + 3] as u16 & 0x0F) << 8 | es_data[pos + 4] as u16) as usize;
        let desc_end = pos + 5 + es_info_length;
        if desc_end > es_data.len() {
            break;
        }
        let descriptors = es_data[pos + 5..desc_end].to_vec();

        entries.push(EsEntry {
            stream_type,
            pid: es_pid,
            dovi_config: extract_dovi_config(&descriptors),
            descriptors,
        });

        pos = desc_end;
    }

    Ok(entries)
}

struct PsiSectionAssembler {
    table_id: u8,
    section: Vec<u8>,
    expected_len: Option<usize>,
    assembling: bool,
}

impl PsiSectionAssembler {
    fn new(table_id: u8) -> Self {
        Self {
            table_id,
            section: Vec::new(),
            expected_len: None,
            assembling: false,
        }
    }

    fn reset(&mut self) {
        self.section.clear();
        self.expected_len = None;
        self.assembling = false;
    }

    fn push_packet(&mut self, packet: &[u8]) -> Result<Option<Vec<u8>>, MediaInfoError> {
        let payload = ts_payload(packet);
        if payload.is_empty() {
            return Ok(None);
        }

        let payload_unit_start = packet[1] & 0x40 != 0;
        let mut data = payload;

        if payload_unit_start {
            let pointer = data[0] as usize;
            let section_start = 1 + pointer;
            if section_start > data.len() {
                self.reset();
                return Ok(None);
            }
            data = &data[section_start..];
            self.section.clear();
            self.expected_len = None;
            self.assembling = true;
        } else if !self.assembling {
            return Ok(None);
        }

        if data.is_empty() {
            return Ok(None);
        }

        if self.section.is_empty() && data[0] != self.table_id {
            self.reset();
            return Ok(None);
        }

        self.section.extend_from_slice(data);

        if self.expected_len.is_none() && self.section.len() >= 3 {
            let section_length =
                ((self.section[1] as usize & 0x0F) << 8) | self.section[2] as usize;
            let total_len = section_length + 3;
            if !(3..=4096).contains(&total_len) {
                self.reset();
                return Ok(None);
            }
            self.expected_len = Some(total_len);
        }

        if let Some(total_len) = self.expected_len
            && self.section.len() >= total_len
        {
            let mut section = std::mem::take(&mut self.section);
            section.truncate(total_len);
            self.expected_len = None;
            self.assembling = false;
            return Ok(Some(section));
        }

        Ok(None)
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
        audio_profile: stream_type_audio_profile(es.stream_type).map(str::to_owned),
        codec_private: None,
        width: None,
        height: None,
        channels: None,
        bit_rate_bps: None,
        language: extract_language(kind, &es.descriptors),
        frame_rate_fps: None,
        color_transfer: None,
        dovi_config: es.dovi_config.clone(),
        has_hdr10plus: false,
        name: None,
        forced: false,
        default_track: false,
    }
}

fn stream_type_audio_profile(stream_type: u8) -> Option<&'static str> {
    match stream_type {
        0x85 => Some("DTS-HD HRA"),
        0x86 => Some("DTS-HD MA"),
        _ => None,
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
        0x82 | 0x85 | 0x86 | 0xA2 => (TrackKind::Audio, "dts"),
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

fn extract_language(kind: TrackKind, descriptors: &[u8]) -> Option<String> {
    let mut best: Option<(u8, String)> = None;
    let mut pos = 0;
    while pos + 2 <= descriptors.len() {
        let tag = descriptors[pos];
        let len = descriptors[pos + 1] as usize;
        let data_start = pos + 2;
        let desc_end = (pos + 2 + len).min(descriptors.len());

        let candidate = match (kind, tag) {
            (TrackKind::Subtitle, SUBTITLING_DESCRIPTOR) if len >= 8 => descriptors
                .get(data_start..data_start + 3)
                .and_then(parse_descriptor_language)
                .map(|lang| (2, lang)),
            (TrackKind::Subtitle, TELETEXT_DESCRIPTOR) if len >= 5 => descriptors
                .get(data_start..data_start + 3)
                .and_then(parse_descriptor_language)
                .map(|lang| (1, lang)),
            (TrackKind::Audio, DVB_EXTENSION_DESCRIPTOR)
                if len >= 5
                    && descriptors.get(data_start).copied()
                        == Some(SUPPLEMENTARY_AUDIO_DESCRIPTOR) =>
            {
                let flags = descriptors.get(data_start + 1).copied().unwrap_or(0);
                if (flags & 0x01) != 0 {
                    descriptors
                        .get(data_start + 2..data_start + 5)
                        .and_then(parse_descriptor_language)
                        .map(|lang| (3, lang))
                } else {
                    None
                }
            }
            (_, ISO_639_LANGUAGE_DESCRIPTOR) if len >= 4 => descriptors
                .get(data_start..data_start + 3)
                .and_then(parse_descriptor_language)
                .map(|lang| (0, lang)),
            _ => None,
        };

        if let Some((priority, language)) = candidate
            && best
                .as_ref()
                .is_none_or(|(best_priority, _)| priority > *best_priority)
        {
            best = Some((priority, language));
        }

        pos = desc_end;
    }

    best.map(|(_, language)| language)
}

fn parse_descriptor_language(data: &[u8]) -> Option<String> {
    std::str::from_utf8(data)
        .ok()
        .map(|value| value.trim_end_matches('\0').to_owned())
        .filter(|value| !value.is_empty())
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
    layout: TsPacketLayout,
) -> Result<Option<u64>, MediaInfoError> {
    let mut states = Vec::new();
    let mut state_by_pid = [None; TS_PID_COUNT];
    for (track_index, entry) in es_entries.iter().enumerate() {
        let kind = classify_stream_type(entry.stream_type, &entry.descriptors).0;
        if matches!(kind, TrackKind::Video | TrackKind::Audio) {
            let state_index = states.len();
            states.push(TsStreamProbeState::new(
                track_index,
                kind,
                tracks[track_index].codec_name.clone(),
            ));
            state_by_pid[usize::from(entry.pid)] = Some(state_index);
        }
    }
    if states.is_empty() {
        return Ok(None);
    }

    stream
        .seek(SeekFrom::Start(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut batch = vec![0_u8; layout.raw_packet_size * STREAM_PROBE_BATCH_PACKETS];
    let mut raw_packet = vec![0_u8; layout.raw_packet_size];
    let mut pkt = [0_u8; TS_PACKET_SIZE];
    let mut packets_scanned = 0usize;
    let mut batch_mode = true;
    let mut active_states = states.iter().filter(|state| !state.done()).count();
    let mut first_pts = None;

    while packets_scanned < STREAM_PROBE_PACKET_LIMIT && active_states > 0 {
        if batch_mode {
            let batch_start = stream
                .stream_position()
                .map_err(|e| MediaInfoError::Io(e.to_string()))?;
            let remaining_packets = STREAM_PROBE_PACKET_LIMIT - packets_scanned;
            let packet_count = remaining_packets.min(STREAM_PROBE_BATCH_PACKETS);
            let read_len = packet_count * layout.raw_packet_size;
            let n = read_full(stream, &mut batch[..read_len]);
            if n < layout.raw_packet_size {
                break;
            }

            let usable_len = n - (n % layout.raw_packet_size);
            let mut offset = 0usize;
            while offset + layout.raw_packet_size <= usable_len
                && packets_scanned < STREAM_PROBE_PACKET_LIMIT
                && active_states > 0
            {
                let sync = offset + layout.sync_offset;
                if batch[sync] != SYNC_BYTE {
                    stream
                        .seek(SeekFrom::Start(batch_start + offset as u64))
                        .map_err(|e| MediaInfoError::Io(e.to_string()))?;
                    batch_mode = false;
                    break;
                }

                let packet_start = sync;
                let packet_end = packet_start + TS_PACKET_SIZE;
                if process_stream_probe_packet(
                    &batch[packet_start..packet_end],
                    &state_by_pid,
                    &mut states,
                    tracks,
                    &mut first_pts,
                ) {
                    active_states = active_states.saturating_sub(1);
                }
                packets_scanned += 1;
                offset += layout.raw_packet_size;
            }

            if n < read_len || !batch_mode {
                continue;
            }
        } else {
            if !read_ts_packet(stream, layout, &mut raw_packet, &mut pkt)? {
                break;
            }
            packets_scanned += 1;
            if process_stream_probe_packet(&pkt, &state_by_pid, &mut states, tracks, &mut first_pts)
            {
                active_states = active_states.saturating_sub(1);
            }
        }
    }

    for state in &mut states {
        let track_index = state.track_index;
        state.finish(&mut tracks[track_index]);
    }

    Ok(first_pts)
}

fn process_stream_probe_packet(
    pkt: &[u8],
    state_by_pid: &[Option<usize>; TS_PID_COUNT],
    states: &mut [TsStreamProbeState],
    tracks: &mut [RawTrack],
    first_pts: &mut Option<u64>,
) -> bool {
    let pid = ts_pid(pkt);
    let Some(state_index) = state_by_pid[usize::from(pid)] else {
        return false;
    };
    let Some(state) = states.get_mut(state_index) else {
        return false;
    };
    if state.done() {
        return false;
    }

    let payload = ts_payload(pkt);
    if payload.is_empty() {
        return false;
    }

    let payload = if (pkt[1] & 0x40) != 0 {
        let pts = extract_pts_from_pes(payload);
        if first_pts.is_none() {
            *first_pts = pts;
        }
        state.record_pts(pts);
        strip_pes_header(payload).unwrap_or(payload)
    } else {
        payload
    };
    if payload.is_empty() {
        return false;
    }

    let was_done = state.done();
    let track_index = state.track_index;
    state.push_payload(payload, &mut tracks[track_index]);
    !was_done && state.done()
}

struct TsStreamProbeState {
    track_index: usize,
    kind: TrackKind,
    codec_name: Option<String>,
    budget: ProbeBudget,
    buffer: Vec<u8>,
    pts_values: Vec<u64>,
    complete: bool,
}

impl TsStreamProbeState {
    fn new(track_index: usize, kind: TrackKind, codec_name: Option<String>) -> Self {
        Self {
            track_index,
            kind,
            codec_name,
            budget: ProbeBudget::new(STREAM_PROBE_MAX_BYTES_PER_PID),
            buffer: Vec::new(),
            pts_values: Vec::new(),
            complete: false,
        }
    }

    fn done(&self) -> bool {
        self.complete || self.budget.exhausted()
    }

    fn record_pts(&mut self, pts: Option<u64>) {
        if self.kind != TrackKind::Video || self.pts_values.len() >= 8 {
            return;
        }
        if let Some(pts) = pts
            && self.pts_values.last().copied() != Some(pts)
        {
            self.pts_values.push(pts);
        }
    }

    fn push_payload(&mut self, payload: &[u8], track: &mut RawTrack) {
        let take = self.budget.consume(payload.len());
        if take > 0 {
            let before_len = self.buffer.len();
            self.buffer.extend_from_slice(&payload[..take]);
            let after_len = self.buffer.len();
            if self.should_probe_after_push(before_len, after_len, track) {
                self.probe(track);
            }
        }
    }

    fn finish(&mut self, track: &mut RawTrack) {
        self.probe(track);
        if self.kind == TrackKind::Video
            && let Some(observed_fps) = estimate_frame_rate_from_pts(&self.pts_values)
            && should_use_pts_frame_rate(
                track.codec_name.as_deref(),
                track.frame_rate_fps,
                observed_fps,
            )
        {
            track.frame_rate_fps = Some(observed_fps);
        }
    }

    fn probe(&mut self, track: &mut RawTrack) {
        if self.complete || self.buffer.is_empty() {
            return;
        }
        if !self.track_probe_needed(track) {
            self.complete = self.probe_complete(track);
            return;
        }

        match self.codec_name.as_deref() {
            Some("h264") => probe_h264_track(&self.buffer, track),
            Some("hevc") => probe_hevc_track(&self.buffer, track),
            Some("vc1") => probe_vc1_track(&self.buffer, track),
            Some("mpeg1video") | Some("mpeg2video") => probe_mpeg_video_track(&self.buffer, track),
            Some("aac") => probe_aac_track(&self.buffer, track),
            Some("aac_latm") => probe_latm_track(&self.buffer, track),
            Some("mp2") => probe_mpeg_audio_track(&self.buffer, track),
            Some("ac3") => probe_ac3_track(&self.buffer, track),
            Some("eac3") => probe_eac3_track(&self.buffer, track),
            Some("truehd") => probe_truehd_track(&self.buffer, track),
            Some("dts") => probe_dts_track(&self.buffer, track),
            Some("unknown") if self.kind == TrackKind::Audio => {
                probe_unknown_audio_track(&self.buffer, track)
            }
            _ => {}
        }

        self.complete = self.probe_complete(track);
        if self.complete {
            self.buffer.clear();
        } else if self.buffer.len() > STREAM_PROBE_ROLLING_KEEP_BYTES * 2 {
            let keep_from = self.buffer.len() - STREAM_PROBE_ROLLING_KEEP_BYTES;
            self.buffer.drain(..keep_from);
        }
    }

    fn probe_complete(&self, track: &RawTrack) -> bool {
        match self.codec_name.as_deref() {
            Some("h264") | Some("hevc") | Some("vc1") | Some("mpeg1video") | Some("mpeg2video") => {
                track.width.is_some() && track.height.is_some() && self.pts_values.len() >= 8
            }
            Some("aac") => {
                track.channels.is_some()
                    && track.audio_profile.is_some()
                    && track.bit_rate_bps.is_some()
            }
            Some("aac_latm") | Some("eac3") => {
                track.channels.is_some() && track.audio_profile.is_some()
            }
            Some("mp2") | Some("ac3") => track.channels.is_some(),
            Some("truehd") => track.audio_profile.is_some(),
            Some("dts") => {
                track.channels.is_some()
                    && track.audio_profile.as_deref().is_some_and(|profile| {
                        profile != "DTS"
                            && (!profile.starts_with("DTS-HD") || track.channels.unwrap_or(0) > 6)
                    })
            }
            Some("unknown") if self.kind == TrackKind::Audio => {
                track.codec_name.as_deref() != Some("unknown") && track.channels.is_some()
            }
            _ => false,
        }
    }

    fn track_probe_needed(&self, track: &RawTrack) -> bool {
        match self.codec_name.as_deref() {
            Some("h264") | Some("hevc") | Some("vc1") | Some("mpeg1video") | Some("mpeg2video") => {
                track.width.is_none() || track.height.is_none()
            }
            Some("aac") => {
                track.channels.is_none()
                    || track.audio_profile.is_none()
                    || track.bit_rate_bps.is_none()
            }
            Some("aac_latm") | Some("eac3") => {
                track.channels.is_none() || track.audio_profile.is_none()
            }
            Some("mp2") | Some("ac3") => track.channels.is_none(),
            Some("truehd") => track.audio_profile.is_none(),
            Some("dts") => !self.probe_complete(track),
            Some("unknown") if self.kind == TrackKind::Audio => {
                track.codec_name.as_deref() == Some("unknown") || track.channels.is_none()
            }
            _ => true,
        }
    }

    fn should_probe_after_push(
        &self,
        before_len: usize,
        after_len: usize,
        track: &RawTrack,
    ) -> bool {
        if after_len == 0 || !self.track_probe_needed(track) {
            return false;
        }
        if before_len == 0 || self.budget.exhausted() {
            return true;
        }

        match self.codec_name.as_deref() {
            Some("aac") => crossed_probe_boundary(before_len, after_len, 64 * 1024, 16 * 1024),
            Some("h264") | Some("hevc") | Some("vc1") | Some("mpeg1video") | Some("mpeg2video") => {
                crossed_probe_boundary(before_len, after_len, 4 * 1024, 4 * 1024)
            }
            _ => crossed_probe_boundary(before_len, after_len, 2 * 1024, 2 * 1024),
        }
    }
}

fn crossed_probe_boundary(before_len: usize, after_len: usize, floor: usize, step: usize) -> bool {
    after_len >= floor && (before_len < floor || before_len / step != after_len / step)
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

fn probe_vc1_track(data: &[u8], track: &mut RawTrack) {
    let Some(sequence) = find_start_code_payload(data, 0x0F) else {
        return;
    };
    let Some((width, height)) = parse_vc1_advanced_sequence_dimensions(sequence) else {
        return;
    };

    track.width = Some(width as i32);
    track.height = Some(height as i32);
    track.codec_private = Some(sequence[..sequence.len().min(64)].to_vec());
}

fn parse_vc1_advanced_sequence_dimensions(data: &[u8]) -> Option<(u16, u16)> {
    let mut bits = BitReader::new(data);
    let profile = bits.read_bits(2)?;
    if profile != 3 {
        return None;
    }

    bits.skip_bits(3)?; // level
    bits.skip_bits(2)?; // chroma format
    bits.skip_bits(3)?; // frame-rate post-processing quality
    bits.skip_bits(5)?; // bit-rate post-processing quality
    bits.skip_bits(1)?; // post-processing flag
    let width = ((bits.read_bits(12)? + 1) * 2) as u16;
    let height = ((bits.read_bits(12)? + 1) * 2) as u16;

    (width > 0 && height > 0).then_some((width, height))
}

fn probe_aac_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_adts_header(data) else {
        return;
    };
    track.channels = Some(header.channels as i32);
    if let Some(bit_rate_bps) = header.bit_rate_bps {
        track.bit_rate_bps = Some(bit_rate_bps as i64);
    }
    merge_audio_profile(
        &mut track.audio_profile,
        detect_audio_profile_from_payload(track.codec_name.as_deref(), data),
    );
}

fn probe_latm_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_latm_header(data) else {
        return;
    };
    track.channels = Some(header.channels as i32);
    merge_audio_profile(
        &mut track.audio_profile,
        detect_audio_profile_from_payload(track.codec_name.as_deref(), data),
    );
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
    if header.layer == 3 {
        track.codec_name = Some("mp3".to_owned());
    } else if header.layer == 2 {
        track.codec_name = Some("mp2".to_owned());
    }
    track.channels = Some(header.channels as i32);
    track.bit_rate_bps = header.bit_rate_bps.map(i64::from);
}

fn probe_unknown_audio_track(data: &[u8], track: &mut RawTrack) {
    if find_adts_header(data).is_some() {
        track.codec_name = Some("aac".to_owned());
        probe_aac_track(data, track);
        return;
    }
    if find_latm_header(data).is_some() {
        track.codec_name = Some("aac_latm".to_owned());
        probe_latm_track(data, track);
        return;
    }
    if find_mpeg_audio_header(data).is_some() {
        probe_mpeg_audio_track(data, track);
        return;
    }
    if find_ac3_header(data).is_some() {
        track.codec_name = Some("ac3".to_owned());
        probe_ac3_track(data, track);
        return;
    }
    if find_eac3_header(data).is_some() {
        track.codec_name = Some("eac3".to_owned());
        probe_eac3_track(data, track);
    }
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
    merge_audio_profile(
        &mut track.audio_profile,
        detect_audio_profile_from_payload(track.codec_name.as_deref(), data),
    );
}

fn probe_truehd_track(data: &[u8], track: &mut RawTrack) {
    merge_audio_profile(
        &mut track.audio_profile,
        detect_audio_profile_from_payload(track.codec_name.as_deref(), data),
    );
}

fn probe_dts_track(data: &[u8], track: &mut RawTrack) {
    let Some(header) = find_dts_header(data) else {
        return;
    };
    track.channels = Some(header.channels as i32);
    if let Some(channels) = detect_dts_channels_from_probe_bytes(data) {
        track.channels = Some(
            track
                .channels
                .map_or(channels, |existing| existing.max(channels)),
        );
    }
    if header.bit_rate_bps > 3 {
        track.bit_rate_bps = Some(i64::from(header.bit_rate_bps));
    }
    merge_audio_profile(
        &mut track.audio_profile,
        detect_audio_profile_from_payload(track.codec_name.as_deref(), data),
    );
    if track.audio_profile.as_deref() == Some("DTS-ES") && track.channels == Some(6) {
        track.channels = Some(7);
    }
}

fn is_plausible_frame_rate(frame_rate_fps: Option<f64>) -> bool {
    frame_rate_fps.is_some_and(|fps| (1.0..=240.0).contains(&fps))
}

fn should_use_pts_frame_rate(
    codec_name: Option<&str>,
    existing: Option<f64>,
    observed: f64,
) -> bool {
    if !is_plausible_frame_rate(Some(observed)) {
        return false;
    }
    if !is_plausible_frame_rate(existing) {
        return true;
    }
    if matches!(codec_name, Some("h264" | "hevc"))
        && existing.is_some_and(|existing| observed < existing * 0.75)
    {
        return false;
    }
    true
}

fn estimate_frame_rate_from_pts(pts_values: &[u64]) -> Option<f64> {
    let mut sorted_pts = pts_values.to_vec();
    sorted_pts.sort_unstable();
    sorted_pts.dedup();

    let deltas: Vec<u64> = sorted_pts
        .windows(2)
        .filter_map(|window| window[1].checked_sub(window[0]))
        .filter(|delta| *delta > 0)
        .filter(|delta| is_plausible_frame_rate(Some(PTS_HZ / *delta as f64)))
        .collect();
    if deltas.is_empty() {
        return None;
    }

    let cadence_delta = choose_frame_cadence_delta(&deltas)?;
    let fps = PTS_HZ / cadence_delta as f64;
    if is_plausible_frame_rate(Some(fps)) {
        Some(fps)
    } else {
        None
    }
}

fn choose_frame_cadence_delta(deltas: &[u64]) -> Option<u64> {
    let mut sorted = deltas.to_vec();
    sorted.sort_unstable();

    let mut best_delta = None;
    let mut best_count = 0usize;
    let mut current_delta = sorted[0];
    let mut current_count = 0usize;
    for delta in sorted.iter().copied().chain(std::iter::once(u64::MAX)) {
        if delta == current_delta {
            current_count += 1;
            continue;
        }
        if current_count > best_count
            || (current_count == best_count && best_delta.is_none_or(|best| current_delta < best))
        {
            best_delta = Some(current_delta);
            best_count = current_count;
        }
        current_delta = delta;
        current_count = 1;
    }

    let reliable_count = if deltas.len() >= 4 { 2 } else { 1 };
    for candidate in sorted.iter().copied() {
        let count = sorted.iter().filter(|delta| **delta == candidate).count();
        if count >= reliable_count {
            return Some(candidate);
        }
    }
    best_delta
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
    layer: u8,
    channels: u8,
    bit_rate_bps: Option<u32>,
}

pub(crate) struct Ac3Header {
    pub channels: u8,
    pub bit_rate_bps: Option<u32>,
}

pub(crate) struct DtsHeader {
    pub channels: u8,
    pub bit_rate_bps: u32,
}

fn find_adts_header(data: &[u8]) -> Option<AdtsHeader> {
    const ADTS_BITRATE_SAMPLE_FRAMES: usize = 128;

    if data.len() < 7 {
        return None;
    }
    let mut start = 0;
    let mut detected_channels = None;
    let mut detected_sample_rate = None;
    let mut total_frame_bytes = 0_u64;
    let mut total_samples = 0_u64;
    let mut frames = 0usize;

    while let Some(candidate) = scan::find_audio_sync_candidate(data, start) {
        if candidate.kind != scan::AudioSyncKind::Adts {
            start = candidate.offset + 1;
            continue;
        }
        let i = candidate.offset;
        if i + 7 > data.len() {
            break;
        }
        start = i + 1;
        let hdr = &data[i..];
        if (hdr[1] & 0xF0) != 0xF0 {
            continue;
        }

        let sampling_frequency_index = (hdr[2] >> 2) & 0x0F;
        let frame_sample_rate: u32 = match sampling_frequency_index {
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

        let frame_channels = ((hdr[2] & 0x01) << 2) | ((hdr[3] >> 6) & 0x03);
        if frame_channels == 0 {
            continue;
        }

        let frame_length = (((hdr[3] & 0x03) as usize) << 11)
            | ((hdr[4] as usize) << 3)
            | (((hdr[5] >> 5) & 0x07) as usize);
        if frame_length < 7 {
            continue;
        }
        if i + frame_length > data.len() {
            break;
        }
        if i + frame_length + 1 < data.len()
            && (data[i + frame_length] != 0xFF || data[i + frame_length + 1] & 0xF0 != 0xF0)
        {
            continue;
        }

        let number_of_raw_data_blocks = hdr[6] & 0x03;
        let samples_per_frame = 1024_u32 * (u32::from(number_of_raw_data_blocks) + 1);
        detected_channels = Some(frame_channels);
        detected_sample_rate = Some(frame_sample_rate);
        total_frame_bytes += frame_length as u64;
        total_samples += u64::from(samples_per_frame);
        frames += 1;
        start = i + frame_length;
        if frames >= ADTS_BITRATE_SAMPLE_FRAMES {
            break;
        }
    }

    let channels = detected_channels?;
    let bit_rate_bps = detected_sample_rate.and_then(|sample_rate| {
        (frames >= ADTS_BITRATE_SAMPLE_FRAMES && total_samples > 0)
            .then(|| (total_frame_bytes * 8 * u64::from(sample_rate)) / total_samples)
            .and_then(|bitrate| u32::try_from(bitrate).ok())
    });

    Some(AdtsHeader {
        channels,
        bit_rate_bps,
    })
}

fn find_latm_header(data: &[u8]) -> Option<LatmHeader> {
    let mut cursor = 0;
    while let Some(candidate) = scan::find_audio_sync_candidate(data, cursor) {
        if candidate.kind != scan::AudioSyncKind::Latm {
            cursor = candidate.offset + 1;
            continue;
        }
        let start = candidate.offset;
        if start + 3 > data.len() {
            return None;
        }
        cursor = start + 1;

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
            (true, 2 | 3) => ((144 * bit_rate_bps) / sample_rate) + padding,
            (false, 2 | 3) => ((72 * bit_rate_bps) / sample_rate) + padding,
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
        if i + frame_size + 1 >= data.len() && data.len().saturating_sub(i) > 4 {
            return None;
        }

        return Some(MpegAudioHeader {
            layer,
            channels: MPEG_AUDIO_CHANNELS[channel_mode],
            bit_rate_bps: Some(bit_rate_bps),
        });
    }

    None
}

pub(crate) fn find_ac3_header(data: &[u8]) -> Option<Ac3Header> {
    if data.len() < 7 {
        return None;
    }
    let mut cursor = 0;
    while let Some(candidate) = scan::find_audio_sync_candidate(data, cursor) {
        if candidate.kind != scan::AudioSyncKind::Ac3 {
            cursor = candidate.offset + 1;
            continue;
        }
        let start = candidate.offset;
        if start + 7 > data.len() {
            return None;
        }
        cursor = start + 1;

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

pub(crate) fn find_eac3_header(data: &[u8]) -> Option<Ac3Header> {
    if data.len() < 6 {
        return None;
    }
    let mut cursor = 0;
    while let Some(candidate) = scan::find_audio_sync_candidate(data, cursor) {
        if candidate.kind != scan::AudioSyncKind::Ac3 {
            cursor = candidate.offset + 1;
            continue;
        }
        let start = candidate.offset;
        if start + 6 > data.len() {
            return None;
        }
        cursor = start + 1;

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

pub(crate) fn find_dts_header(data: &[u8]) -> Option<DtsHeader> {
    if data.len() < 11 {
        return None;
    }
    let mut cursor = 0;
    while let Some(candidate) = scan::find_audio_sync_candidate(data, cursor) {
        if candidate.kind != scan::AudioSyncKind::Dts {
            cursor = candidate.offset + 1;
            continue;
        }
        let start = candidate.offset;
        if start + 11 > data.len() {
            return None;
        }
        cursor = start + 1;

        let probe_end = (start + DTS_HEADER_PROBE_BYTES).min(data.len());
        let normalized = normalize_dts_core_prefix(&data[start..probe_end])?;
        let mut bits = BitReader::new(&normalized);
        if bits.read_bits(32)? != DTS_SYNCWORD_CORE_BE {
            continue;
        }
        bits.read_bit()?; // normal frame flag
        let deficit_samples = bits.read_bits(5)? as u8 + 1;
        if deficit_samples != 32 {
            continue;
        }
        bits.skip_bits(1)?; // crc present
        let npcmblocks = bits.read_bits(7)? as u8 + 1;
        if (npcmblocks & 0x07) != 0 {
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

fn normalize_dts_core_prefix(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 4 {
        return None;
    }

    let marker = u32::from_be_bytes(data[..4].try_into().ok()?);
    match marker {
        DTS_SYNCWORD_CORE_BE => Some(data.to_vec()),
        DTS_SYNCWORD_CORE_LE => {
            let mut normalized = Vec::with_capacity(data.len());
            for chunk in data.chunks(2) {
                if chunk.len() == 2 {
                    normalized.push(chunk[1]);
                    normalized.push(chunk[0]);
                } else {
                    normalized.push(chunk[0]);
                }
            }
            Some(normalized)
        }
        DTS_SYNCWORD_CORE_14B_BE | DTS_SYNCWORD_CORE_14B_LE => {
            let mut normalized = Vec::with_capacity((data.len() * 14) / 16 + 2);
            let mut bit_buffer = 0u32;
            let mut bits_in_buffer = 0usize;

            for chunk in data.chunks_exact(2) {
                let word = if marker == DTS_SYNCWORD_CORE_14B_BE {
                    u16::from_be_bytes([chunk[0], chunk[1]])
                } else {
                    u16::from_le_bytes([chunk[0], chunk[1]])
                } & 0x3FFF;

                bit_buffer = (bit_buffer << 14) | u32::from(word);
                bits_in_buffer += 14;

                while bits_in_buffer >= 8 {
                    bits_in_buffer -= 8;
                    normalized.push(((bit_buffer >> bits_in_buffer) & 0xFF) as u8);
                    bit_buffer &= (1u32 << bits_in_buffer).saturating_sub(1);
                }
            }

            Some(normalized)
        }
        _ => None,
    }
}

fn find_start_code_payload(data: &[u8], code: u8) -> Option<&[u8]> {
    scan::find_mpeg_start_code(data, code).and_then(|i| data.get(i + 4..))
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
    while let Some(start_code) = scan::find_annexb_start_code(data, i) {
        let nal_start = start_code.end;
        let nal_end = scan::find_annexb_start_code(data, nal_start)
            .map(|next_start_code| next_start_code.start)
            .unwrap_or(data.len());

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
    layout: TsPacketLayout,
    first_pts_hint: Option<u64>,
) -> Option<f64> {
    if es_entries.is_empty() || file_size < layout.raw_packet_size as u64 {
        return None;
    }

    let mut pes_pid_lookup = [false; TS_PID_COUNT];
    let mut has_pes_pid = false;
    for entry in es_entries
        .iter()
        .filter(|entry| is_pes_stream_type(entry.stream_type))
    {
        pes_pid_lookup[usize::from(entry.pid)] = true;
        has_pes_pid = true;
    }
    if !has_pes_pid {
        return None;
    }

    let first_pts = first_pts_hint
        .or_else(|| {
            find_pts_near(
                stream,
                0,
                true,
                &pes_pid_lookup,
                FAST_DURATION_PROBE_PACKETS,
                layout,
            )
        })
        .or_else(|| {
            find_pts_near(
                stream,
                0,
                true,
                &pes_pid_lookup,
                FALLBACK_DURATION_PROBE_PACKETS,
                layout,
            )
        })?;

    estimate_duration_from_first_pts(
        stream,
        file_size,
        &pes_pid_lookup,
        layout,
        first_pts,
        FAST_DURATION_PROBE_PACKETS,
    )
    .or_else(|| {
        estimate_duration_from_first_pts(
            stream,
            file_size,
            &pes_pid_lookup,
            layout,
            first_pts,
            FALLBACK_DURATION_PROBE_PACKETS,
        )
    })
}

fn estimate_duration_from_first_pts<T: Read + Seek>(
    stream: &mut T,
    file_size: u64,
    pes_pid_lookup: &[bool; TS_PID_COUNT],
    layout: TsPacketLayout,
    first_pts: u64,
    packet_limit: usize,
) -> Option<f64> {
    let tail_start = file_size.saturating_sub(layout.raw_packet_size as u64 * packet_limit as u64);
    let last_pts = find_pts_near(
        stream,
        tail_start,
        false,
        pes_pid_lookup,
        packet_limit,
        layout,
    );

    match last_pts {
        Some(last) if last > first_pts => Some((last - first_pts) as f64 / PTS_HZ),
        Some(last) if last <= first_pts => {
            let wrapped = (1u64 << 33) - first_pts + last;
            Some(wrapped as f64 / PTS_HZ)
        }
        _ => None,
    }
}

fn is_pes_stream_type(stream_type: u8) -> bool {
    matches!(
        stream_type,
        0x01 | 0x02
            | 0x03
            | 0x04
            | 0x06
            | 0x0F
            | 0x10
            | 0x11
            | 0x1B
            | 0x24
            | 0x42
            | 0x81
            | 0x82
            | 0x83
            | 0x84
            | 0x85
            | 0x86
            | 0x87
            | 0x90
            | 0x92
            | 0xA1
            | 0xA2
            | 0xC1
            | 0xC2
            | 0xCF
            | 0xD2
            | 0xD4
            | 0xDB
            | 0xEA
    )
}

fn find_pts_near<T: Read + Seek>(
    stream: &mut T,
    start_pos: u64,
    first_match: bool,
    pes_pid_lookup: &[bool; TS_PID_COUNT],
    max_packets: usize,
    layout: TsPacketLayout,
) -> Option<u64> {
    let aligned_start = start_pos - (start_pos % layout.raw_packet_size as u64);
    stream.seek(SeekFrom::Start(aligned_start)).ok()?;
    let read_size = max_packets * layout.raw_packet_size;
    let mut data = vec![0u8; read_size];
    let n = read_full(stream, &mut data);
    data.truncate(n);

    let mut result = None;
    let mut offset = first_packet_offset(&data, layout);

    while offset + layout.raw_packet_size <= data.len() {
        if data[offset + layout.sync_offset] != SYNC_BYTE {
            offset += 1;
            continue;
        }

        let packet_start = offset + layout.sync_offset;
        let pkt = &data[packet_start..packet_start + TS_PACKET_SIZE];
        let pid = ts_pid(pkt);
        if pes_pid_lookup[usize::from(pid)] && (pkt[1] & 0x40) != 0 {
            let payload = ts_payload(pkt);
            if let Some(pts) = extract_pts_from_pes(payload) {
                if first_match {
                    return Some(pts);
                }
                result = Some(pts);
            }
        }

        offset += layout.raw_packet_size;
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

fn read_ts_packet<T: Read + Seek>(
    stream: &mut T,
    layout: TsPacketLayout,
    raw_packet: &mut [u8],
    packet: &mut [u8; TS_PACKET_SIZE],
) -> Result<bool, MediaInfoError> {
    let n = read_full(stream, raw_packet);
    if n < layout.raw_packet_size {
        return Ok(false);
    }
    if raw_packet[layout.sync_offset] != SYNC_BYTE && !resync(stream, layout, raw_packet)? {
        return Ok(false);
    }
    packet.copy_from_slice(&raw_packet[layout.sync_offset..layout.sync_offset + TS_PACKET_SIZE]);
    Ok(true)
}

fn resync<T: Read + Seek>(
    stream: &mut T,
    layout: TsPacketLayout,
    raw_packet: &mut [u8],
) -> Result<bool, MediaInfoError> {
    let current = stream
        .stream_position()
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;
    let rewind = current.saturating_sub((layout.raw_packet_size - 1) as u64);

    for offset in 0..layout.raw_packet_size {
        let candidate_start = rewind + offset as u64;
        stream
            .seek(SeekFrom::Start(candidate_start))
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        if read_full(stream, raw_packet) < layout.raw_packet_size {
            return Ok(false);
        }
        if raw_packet[layout.sync_offset] == SYNC_BYTE {
            return Ok(true);
        }
    }

    Ok(false)
}

fn detect_ts_packet_layout<T: Read + Seek>(
    stream: &mut T,
) -> Result<TsPacketLayout, MediaInfoError> {
    let current = stream
        .stream_position()
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;
    stream
        .seek(SeekFrom::Start(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let probe_len = TS_FEC_PACKET_SIZE * 100;
    let mut probe = vec![0u8; probe_len];
    let n = read_full(stream, &mut probe);
    probe.truncate(n);

    stream
        .seek(SeekFrom::Start(current))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let candidates = [
        TsPacketLayout {
            raw_packet_size: TS_PACKET_SIZE,
            sync_offset: 0,
        },
        TsPacketLayout {
            raw_packet_size: TS_DVHS_PACKET_SIZE,
            sync_offset: 4,
        },
        TsPacketLayout {
            raw_packet_size: TS_FEC_PACKET_SIZE,
            sync_offset: 0,
        },
    ];

    candidates
        .into_iter()
        .max_by_key(|layout| packet_layout_score(&probe, *layout))
        .ok_or_else(|| MediaInfoError::Parse("unable to determine TS packet layout".into()))
}

fn packet_layout_score(data: &[u8], layout: TsPacketLayout) -> usize {
    if data.len() < layout.raw_packet_size
        || layout.sync_offset + TS_PACKET_SIZE > layout.raw_packet_size
    {
        return 0;
    }

    scan::score_ts_packet_layout(data, layout.raw_packet_size, layout.sync_offset, SYNC_BYTE)
}

fn first_packet_offset(data: &[u8], layout: TsPacketLayout) -> usize {
    let max_offset = layout.raw_packet_size.min(data.len());
    for offset in 0..max_offset {
        if offset + layout.sync_offset >= data.len() {
            break;
        }
        if data[offset + layout.sync_offset] != SYNC_BYTE {
            continue;
        }
        let next = offset + layout.raw_packet_size + layout.sync_offset;
        if next >= data.len() || data[next] == SYNC_BYTE {
            return offset;
        }
    }
    0
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
    fn detects_bluray_dvhs_packet_layout() {
        let mut data = vec![0u8; TS_DVHS_PACKET_SIZE * 3];
        for packet_idx in 0..3 {
            data[packet_idx * TS_DVHS_PACKET_SIZE + 4] = SYNC_BYTE;
        }

        let layout = detect_ts_packet_layout(&mut std::io::Cursor::new(data)).unwrap();
        assert_eq!(layout.raw_packet_size, TS_DVHS_PACKET_SIZE);
        assert_eq!(layout.sync_offset, 4);
    }

    #[test]
    fn parses_real_dts_hd_ma_core_header_prefix() {
        let data = [
            0x7f, 0xfe, 0x80, 0x01, 0xfc, 0x3c, 0x7d, 0xb2, 0x77, 0x00, 0x0d, 0x3b, 0x80, 0x09,
            0xef, 0x7b,
        ];
        let header = find_dts_header(&data).unwrap();
        assert_eq!(header.channels, 6);
    }

    #[test]
    fn parses_adts_header_channels_and_bitrate() {
        let mut data = Vec::new();
        for _ in 0..128 {
            data.extend_from_slice(&[0xFF, 0xF1, 0x50, 0x80, 0x00, 0xFF, 0xFC]);
        }
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
    fn pts_frame_rate_estimate_prefers_dense_cadence_over_repeated_field_gaps() {
        let pts = [0, 3003, 12012, 15015, 24024, 27027, 36036, 39039];
        let fps = estimate_frame_rate_from_pts(&pts).unwrap();
        assert!((fps - (30000.0 / 1001.0)).abs() < 0.001);
    }

    #[test]
    fn h264_ts_keeps_plausible_sps_frame_rate_over_sparse_pts_cadence() {
        assert!(!should_use_pts_frame_rate(
            Some("h264"),
            Some(30000.0 / 1001.0),
            30000.0 / 3003.0
        ));
        assert!(should_use_pts_frame_rate(
            Some("mpeg2video"),
            Some(24.0),
            12.0
        ));
    }

    #[test]
    fn parses_mpeg_audio_header() {
        let mut data = vec![0_u8; 388];
        data[..4].copy_from_slice(&[0xFF, 0xFD, 0x84, 0x80]);
        data[384..388].copy_from_slice(&[0xFF, 0xFD, 0x84, 0x80]);
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

    #[test]
    fn subtitle_language_prefers_subtitling_descriptor() {
        let descriptors = [
            ISO_639_LANGUAGE_DESCRIPTOR,
            4,
            b'e',
            b'n',
            b'g',
            0,
            SUBTITLING_DESCRIPTOR,
            8,
            b'j',
            b'p',
            b'n',
            0x10,
            0,
            1,
            0,
            2,
        ];
        assert_eq!(
            extract_language(TrackKind::Subtitle, &descriptors),
            Some("jpn".to_string())
        );
    }

    #[test]
    fn audio_language_prefers_supplementary_audio_override() {
        let descriptors = [
            ISO_639_LANGUAGE_DESCRIPTOR,
            4,
            b'e',
            b'n',
            b'g',
            0,
            DVB_EXTENSION_DESCRIPTOR,
            5,
            SUPPLEMENTARY_AUDIO_DESCRIPTOR,
            0x01,
            b'j',
            b'p',
            b'n',
        ];
        assert_eq!(
            extract_language(TrackKind::Audio, &descriptors),
            Some("jpn".to_string())
        );
    }

    #[test]
    fn truncated_supplementary_audio_descriptor_falls_back_without_panicking() {
        let descriptors = [
            ISO_639_LANGUAGE_DESCRIPTOR,
            4,
            b'e',
            b'n',
            b'g',
            0,
            DVB_EXTENSION_DESCRIPTOR,
            5,
            SUPPLEMENTARY_AUDIO_DESCRIPTOR,
            0x01,
            b'j',
        ];
        assert_eq!(
            extract_language(TrackKind::Audio, &descriptors),
            Some("eng".to_string())
        );
    }
}
