//! MPEG transport streams: program tables, PES payload probes and PTS timing.

use crate::codec::aac::find_adts_header_with_minimum;
use crate::codec::stream_probe::{
    probe_aac_track, probe_ac3_track, probe_dts_track, probe_eac3_track, probe_h264_track,
    probe_hevc_track, probe_latm_track, probe_mpeg_audio_track, probe_mpeg_video_track,
    probe_truehd_track, probe_unknown_audio_track, probe_vc1_track,
};
use crate::raw::{RawContainer, RawTrack, TrackKind};
use crate::scan;
use crate::source::ProbeBudget;
use crate::{ProbeDepth, ProbeError};
use scryer_media_model::Codec;
use scryer_media_model::warning;
use std::io::{Read, Seek, SeekFrom};

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

#[derive(Clone, Copy)]
struct TsPacketLayout {
    raw_packet_size: usize,
    sync_offset: usize,
}

pub(crate) fn parse_ts_source(
    source: &mut dyn crate::source::MediaSource,
    profile: ProbeDepth,
) -> Result<RawContainer, ProbeError> {
    // PAT/PMT and timestamp scans consume individual 188-byte packets. Coalesce
    // their reads before reaching a file or disc extent on network storage.
    let mut file = std::io::BufReader::with_capacity(64 * 1024, source);
    let file_size = file.seek(SeekFrom::End(0)).map_err(ProbeError::from)?;
    file.seek(SeekFrom::Start(0)).map_err(ProbeError::from)?;

    let layout = detect_ts_packet_layout(&mut file)?;
    let es_entries = parse_program_map(&mut file, layout)?;
    let mut tracks: Vec<RawTrack> = es_entries.iter().map(build_track).collect();
    let mut report = scryer_media_model::ProbeReport::default();
    let mut caption_services = Vec::new();

    if profile != ProbeDepth::Identify {
        // Stream probing only fills in detail for tracks the program map already
        // described. An exhausted budget leaves that detail unknown; it must not
        // discard the tracks themselves.
        if let Err(error) = enrich_tracks_from_probe(
            &mut file,
            &es_entries,
            &mut tracks,
            layout,
            &mut report,
            &mut caption_services,
        ) {
            if !crate::source::MediaSource::exhausted(file.get_ref()) {
                return Err(error);
            }
            report.status = scryer_media_model::ProbeStatus::Incomplete;
            report.budget_exhausted = true;
            report.warnings.push(scryer_media_model::ProbeWarning {
                code: warning::TS_PROBE_READ_BUDGET.into(),
                message:
                    "Stream probing stopped when the media byte or I/O-operation budget was exhausted"
                        .into(),
                ..Default::default()
            });
        }
    }
    let selected_video = tracks.iter().find(|track| track.kind == TrackKind::Video);
    let selected_program = selected_video.and_then(|track| track.metadata.program_id);
    let selected_pid = selected_video
        .and_then(|track| track.metadata.id.as_deref())
        .and_then(|id| id.parse::<u16>().ok());
    let selected_entries: Vec<_> = es_entries
        .iter()
        .filter(|entry| Some(entry.pid) == selected_pid)
        .cloned()
        .collect();
    let final_frame_seconds = selected_video
        .and_then(|track| track.frame_rate_fps)
        .filter(|fps| is_plausible_frame_rate(Some(*fps)))
        .map(|fps| 1.0 / fps);
    let duration_seconds = estimate_duration(&mut file, file_size, &selected_entries, layout, None)
        .and_then(|duration| {
            final_frame_seconds
                .map(|frame| duration + frame)
                .or_else(|| (profile == ProbeDepth::Identify).then_some(duration))
        });
    let duration_provenance = if profile == ProbeDepth::Identify && duration_seconds.is_some() {
        report.status = scryer_media_model::ProbeStatus::Incomplete;
        report.warnings.push(scryer_media_model::ProbeWarning {
            code: warning::CONTENT_PROBE_DURATION_ESTIMATE.into(),
            message: "Discovery uses the selected video's timestamp span; the final frame duration has not been inspected".into(),
            ..Default::default()
        });
        scryer_media_model::Provenance::Estimated
    } else if duration_seconds.is_some() {
        scryer_media_model::Provenance::Observed
    } else {
        scryer_media_model::Provenance::Unknown
    };
    let mut programs: Vec<scryer_media_model::Program> = Vec::new();
    for entry in &es_entries {
        let id = u32::from(entry.program_id);
        if let Some(program) = programs.iter_mut().find(|program| program.id == id) {
            program.stream_ids.push(entry.pid.to_string());
        } else {
            programs.push(scryer_media_model::Program {
                id,
                stream_ids: vec![entry.pid.to_string()],
                duration_seconds: (selected_program == Some(id))
                    .then_some(duration_seconds)
                    .flatten(),
                ..Default::default()
            });
        }
    }

    let overall_bitrate_bps = duration_seconds
        .filter(|duration| *duration > 0.0)
        .map(|duration| (file_size as f64 * 8.0 / duration) as u64);

    Ok(RawContainer {
        details: scryer_media_model::AnalysisDetails {
            duration_provenance,
            overall_bitrate_bps,
            programs,
            report,
            caption_services,
            ..Default::default()
        },
        format_name: "mpegts".into(),
        duration_seconds,
        num_chapters: None,
        tracks,
    })
}

#[derive(Clone)]
struct EsEntry {
    program_id: u16,
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
) -> Result<Vec<EsEntry>, ProbeError> {
    stream.seek(SeekFrom::Start(0)).map_err(ProbeError::from)?;

    let mut raw_packet = vec![0u8; layout.raw_packet_size];
    let mut packet = [0u8; TS_PACKET_SIZE];
    let mut packets_scanned = 0u32;
    let mut pat = PsiSectionAssembler::new(0x00);
    let mut programs = Vec::new();
    let mut assemblers = std::collections::HashMap::new();
    let mut entries = std::collections::HashMap::new();
    let mut pat_sections = std::collections::BTreeSet::new();
    let mut last_pat_section = None;

    loop {
        if packets_scanned >= PROGRAM_MAP_PROBE_PACKETS {
            return Err(ProbeError::Malformed(format!(
                "program map not found within first {PROGRAM_MAP_PROBE_PACKETS} packets"
            )));
        }

        if !read_ts_packet(stream, layout, &mut raw_packet, &mut packet)? {
            return Err(ProbeError::Malformed(
                "program map not found before end of file".into(),
            ));
        }
        packets_scanned += 1;

        let pid = ts_pid(&packet);
        if pid == PAT_PID {
            if let Some(section) = pat.push_packet(&packet)? {
                if section.len() < 12 || section[5] & 1 == 0 {
                    continue;
                }
                pat_sections.insert(section[6]);
                last_pat_section = Some(section[7]);
                let length =
                    3 + usize::from(((u16::from(section[1]) & 15) << 8) | u16::from(section[2]));
                let data = section
                    .get(8..length.saturating_sub(4))
                    .ok_or_else(|| ProbeError::Malformed("truncated PAT".into()))?;
                for entry in data.as_chunks::<4>().0 {
                    let program = u16::from_be_bytes([entry[0], entry[1]]);
                    let pid = u16::from_be_bytes([entry[2] & 31, entry[3]]);
                    if program == 0 {
                        continue;
                    }
                    if !programs.contains(&(program, pid)) {
                        programs.push((program, pid));
                    }
                    if programs.len() > 64 {
                        return Err(ProbeError::Unsupported(
                            "TS program inventory exceeds budget".into(),
                        ));
                    }
                    assemblers
                        .entry(pid)
                        .or_insert_with(|| PsiSectionAssembler::new(0x02));
                }
            }
        } else if let Some(assembler) = assemblers.get_mut(&pid)
            && let Some(section) = assembler.push_packet(&packet)?
        {
            let parsed = parse_pmt_section(&section)?;
            entries.insert(pid, parsed);
        }
        if !programs.is_empty()
            && programs.iter().all(|(_, pid)| entries.contains_key(pid))
            && last_pat_section
                .is_some_and(|last| (0..=last).all(|section| pat_sections.contains(&section)))
        {
            let mut ordered = Vec::new();
            for (program, pid) in programs {
                if let Some(mut tracks) = entries.remove(&pid) {
                    for track in &mut tracks {
                        track.program_id = program;
                    }
                    ordered.extend(tracks);
                }
            }
            return Ok(ordered);
        }
    }
}

fn parse_pmt_section(section: &[u8]) -> Result<Vec<EsEntry>, ProbeError> {
    if section.len() < 16 {
        return Err(ProbeError::Malformed("PMT section too short".into()));
    }

    let section_length = ((section[1] as u16 & 0x0F) << 8 | section[2] as u16) as usize;
    if section_length < 13 {
        return Err(ProbeError::Malformed("PMT section length too short".into()));
    }

    let program_info_length = ((section[10] as u16 & 0x0F) << 8 | section[11] as u16) as usize;
    let es_start = 12 + program_info_length;
    let es_end = 3 + section_length.saturating_sub(4);
    if es_start > section.len() || es_end > section.len() || es_start > es_end {
        return Err(ProbeError::Malformed(
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
            program_id: u16::from_be_bytes([section[3], section[4]]),
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

    fn push_packet(&mut self, packet: &[u8]) -> Result<Option<Vec<u8>>, ProbeError> {
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
        metadata: scryer_media_model::StreamMetadata {
            id: Some(es.pid.to_string()),
            program_id: Some(u32::from(es.program_id)),
            language_provenance: scryer_media_model::Provenance::Container,
            ..Default::default()
        },
        kind,
        codec_id: format!("0x{:02X}", es.stream_type),
        codec_name: Some(codec_name),
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
    }
}

fn stream_type_audio_profile(stream_type: u8) -> Option<&'static str> {
    match stream_type {
        0x85 => Some("DTS-HD HRA"),
        0x86 => Some("DTS-HD MA"),
        _ => None,
    }
}

fn classify_stream_type(stream_type: u8, descriptors: &[u8]) -> (TrackKind, Codec) {
    match stream_type {
        0x01 => (TrackKind::Video, Codec::Mpeg1Video),
        0x02 => (TrackKind::Video, Codec::Mpeg2Video),
        0x10 => (TrackKind::Video, Codec::Mpeg4),
        0x1B => (TrackKind::Video, Codec::H264),
        0x24 => (TrackKind::Video, Codec::Hevc),
        0x42 => (TrackKind::Video, Codec::Cavs),
        0xD2 => (TrackKind::Video, Codec::Avs2),
        0xD4 => (TrackKind::Video, Codec::Avs3),
        0xDB => (TrackKind::Video, Codec::H264),
        0xEA => (TrackKind::Video, Codec::Vc1),
        0x03 | 0x04 => (TrackKind::Audio, Codec::Mp2),
        0x0F => (TrackKind::Audio, Codec::Aac),
        0x11 => (TrackKind::Audio, Codec::AacLatm),
        0x81 => (TrackKind::Audio, Codec::Ac3),
        0x82 | 0x85 | 0x86 | 0xA2 => (TrackKind::Audio, Codec::Dts),
        0x83 => (TrackKind::Audio, Codec::TrueHd),
        0x84 | 0x87 | 0xA1 | 0xC2 => (TrackKind::Audio, Codec::Eac3),
        0x90 => (TrackKind::Subtitle, Codec::HdmvPgsSubtitle),
        0x92 => (TrackKind::Subtitle, Codec::HdmvTextSubtitle),
        0xC1 => (TrackKind::Audio, Codec::Ac3),
        0xCF => (TrackKind::Audio, Codec::Aac),
        0x06 => classify_private_pes(descriptors),
        _ if stream_type >= 0x80 => (TrackKind::Video, Codec::Unknown),
        _ => (TrackKind::Video, Codec::Unknown),
    }
}

fn classify_private_pes(descriptors: &[u8]) -> (TrackKind, Codec) {
    let mut pos = 0;
    while pos + 2 <= descriptors.len() {
        let tag = descriptors[pos];
        let len = descriptors[pos + 1] as usize;
        let desc_end = (pos + 2 + len).min(descriptors.len());

        match tag {
            0x6A => return (TrackKind::Audio, Codec::Ac3),
            0x7A => return (TrackKind::Audio, Codec::Eac3),
            0x7B => return (TrackKind::Audio, Codec::Dts),
            0x7C => return (TrackKind::Audio, Codec::Aac),
            0x59 => return (TrackKind::Subtitle, Codec::DvbSubtitle),
            0x56 => return (TrackKind::Subtitle, Codec::DvbTeletext),
            _ => {}
        }

        pos = desc_end;
    }
    (TrackKind::Audio, Codec::Unknown)
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

/// Like FFmpeg, a later Dolby Vision descriptor replaces an earlier one.
fn extract_dovi_config(descriptors: &[u8]) -> Option<Vec<u8>> {
    let mut record = None;
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
            record = Some(vec![
                data[0],
                data[1],
                data[2],
                data[3],
                compat_and_compression,
            ]);
        }

        pos = desc_end;
    }
    record
}

// ---------------------------------------------------------------------------
// Stream probing
// ---------------------------------------------------------------------------

fn enrich_tracks_from_probe<T: Read + Seek>(
    stream: &mut T,
    es_entries: &[EsEntry],
    tracks: &mut [RawTrack],
    layout: TsPacketLayout,
    report: &mut scryer_media_model::ProbeReport,
    captions: &mut Vec<scryer_media_model::CaptionService>,
) -> Result<Option<u64>, ProbeError> {
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

    stream.seek(SeekFrom::Start(0)).map_err(ProbeError::from)?;

    let mut batch = vec![0_u8; layout.raw_packet_size * STREAM_PROBE_BATCH_PACKETS];
    let mut raw_packet = vec![0_u8; layout.raw_packet_size];
    let mut pkt = [0_u8; TS_PACKET_SIZE];
    let mut packets_scanned = 0usize;
    let mut batch_mode = true;
    let mut active_states = states.iter().filter(|state| !state.done()).count();
    let mut first_pts = None;

    while packets_scanned < STREAM_PROBE_PACKET_LIMIT && active_states > 0 {
        if batch_mode {
            let batch_start = stream.stream_position().map_err(ProbeError::from)?;
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
                        .map_err(ProbeError::from)?;
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
        state.finish(
            &mut tracks[track_index],
            packets_scanned >= STREAM_PROBE_PACKET_LIMIT,
        );
        report.budget_exhausted |= state.metadata_report.budget_exhausted;
        if state.metadata_report.status == scryer_media_model::ProbeStatus::Incomplete {
            report.status = state.metadata_report.status;
        }
        report.warnings.append(&mut state.metadata_report.warnings);
        for caption in state.caption_services.drain(..) {
            if captions.len() < 256 && !captions.contains(&caption) {
                captions.push(caption);
            }
        }
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
    codec_name: Option<Codec>,
    budget: ProbeBudget,
    buffer: Vec<u8>,
    pts_values: Vec<u64>,
    complete: bool,
    metadata_report: scryer_media_model::ProbeReport,
    caption_services: Vec<scryer_media_model::CaptionService>,
}

impl TsStreamProbeState {
    fn new(track_index: usize, kind: TrackKind, codec_name: Option<Codec>) -> Self {
        Self {
            track_index,
            kind,
            codec_name,
            budget: ProbeBudget::new(STREAM_PROBE_MAX_BYTES_PER_PID),
            buffer: Vec::new(),
            pts_values: Vec::new(),
            complete: false,
            metadata_report: Default::default(),
            caption_services: Vec::new(),
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

    fn finish(&mut self, track: &mut RawTrack, packet_budget_exhausted: bool) {
        let stopped_on_budget =
            !self.complete && (self.budget.exhausted() || packet_budget_exhausted);
        if !self.complete
            && self.codec_name == Some(Codec::Aac)
            && let Some(header) = find_adts_header_with_minimum(&self.buffer, 2)
            && let Some(bitrate) = header.bit_rate_bps
        {
            // Keep the normal 128-frame sampling target while reads remain;
            // expose a shorter final sample as an estimate, never a declaration.
            track.metadata.estimated_bitrate_bps = Some(u64::from(bitrate));
        }
        self.probe(track);
        if !self.complete || stopped_on_budget {
            let exhausted = self.budget.exhausted() || packet_budget_exhausted;
            self.metadata_report.budget_exhausted |= exhausted;
            self.metadata_report.status = scryer_media_model::ProbeStatus::Incomplete;
            self.metadata_report.warnings.push(scryer_media_model::ProbeWarning {
                code: if exhausted { warning::TS_STREAM_PROBE_BUDGET } else { warning::TS_STREAM_PROBE_INCOMPLETE }.into(),
                message: if exhausted {
                    "Transport stream enrichment reached its read budget before all requested metadata was established"
                } else {
                    "Transport stream enrichment ended before all requested metadata was established"
                }.into(),
                stream_id: track.metadata.id.clone(),
                ..Default::default()
            });
        }
        if self.kind == TrackKind::Video
            && let Some(observed_fps) = estimate_frame_rate_from_pts(&self.pts_values)
            && should_use_pts_frame_rate(
                track.codec_name.as_ref(),
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
        if self.kind == TrackKind::Video {
            crate::codec::sei::annex_b(
                &self.buffer,
                track,
                &mut self.metadata_report,
                &mut self.caption_services,
            );
        }
        if !self.track_probe_needed(track) {
            self.complete = self.probe_complete(track);
            return;
        }

        match self.codec_name {
            Some(Codec::H264) => probe_h264_track(&self.buffer, track),
            Some(Codec::Hevc) => probe_hevc_track(&self.buffer, track),
            Some(Codec::Vc1) => probe_vc1_track(&self.buffer, track),
            Some(Codec::Mpeg1Video) | Some(Codec::Mpeg2Video) => {
                probe_mpeg_video_track(&self.buffer, track)
            }
            Some(Codec::Aac) => probe_aac_track(&self.buffer, track),
            Some(Codec::AacLatm) => probe_latm_track(&self.buffer, track),
            Some(Codec::Mp2) => probe_mpeg_audio_track(&self.buffer, track),
            Some(Codec::Ac3) => probe_ac3_track(&self.buffer, track),
            Some(Codec::Eac3) => probe_eac3_track(&self.buffer, track),
            Some(Codec::TrueHd) => probe_truehd_track(&self.buffer, track),
            Some(Codec::Dts) => probe_dts_track(&self.buffer, track),
            Some(Codec::Unknown) if self.kind == TrackKind::Audio => {
                probe_unknown_audio_track(&self.buffer, track);
                // Identification establishes the codec, not completion of its
                // metadata. Continue with that codec's bounded sampling rules.
                if track
                    .codec_name
                    .as_ref()
                    .is_some_and(|name| *name != Codec::Unknown)
                {
                    self.codec_name.clone_from(&track.codec_name);
                }
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
        match self.codec_name {
            Some(Codec::H264)
            | Some(Codec::Hevc)
            | Some(Codec::Vc1)
            | Some(Codec::Mpeg1Video)
            | Some(Codec::Mpeg2Video) => {
                track.width.is_some() && track.height.is_some() && self.pts_values.len() >= 8
            }
            Some(Codec::Aac) => {
                track.channels.is_some()
                    && track.audio_profile.is_some()
                    && (track.bit_rate_bps.is_some()
                        || track.metadata.estimated_bitrate_bps.is_some())
            }
            Some(Codec::AacLatm) | Some(Codec::Eac3) => {
                track.channels.is_some() && track.audio_profile.is_some()
            }
            Some(Codec::Mp2) | Some(Codec::Ac3) => track.channels.is_some(),
            Some(Codec::TrueHd) => track.audio_profile.is_some(),
            Some(Codec::Dts) => {
                track.channels.is_some()
                    && track.audio_profile.as_deref().is_some_and(|profile| {
                        profile != "DTS"
                            && (!profile.starts_with("DTS-HD") || track.channels.unwrap_or(0) > 6)
                    })
            }
            Some(Codec::Unknown) if self.kind == TrackKind::Audio => {
                track.codec_name != Some(Codec::Unknown) && track.channels.is_some()
            }
            _ => false,
        }
    }

    fn track_probe_needed(&self, track: &RawTrack) -> bool {
        match self.codec_name {
            Some(Codec::H264)
            | Some(Codec::Hevc)
            | Some(Codec::Vc1)
            | Some(Codec::Mpeg1Video)
            | Some(Codec::Mpeg2Video) => track.width.is_none() || track.height.is_none(),
            Some(Codec::Aac) => {
                track.channels.is_none()
                    || track.audio_profile.is_none()
                    || (track.bit_rate_bps.is_none()
                        && track.metadata.estimated_bitrate_bps.is_none())
            }
            Some(Codec::AacLatm) | Some(Codec::Eac3) => {
                track.channels.is_none() || track.audio_profile.is_none()
            }
            Some(Codec::Mp2) | Some(Codec::Ac3) => track.channels.is_none(),
            Some(Codec::TrueHd) => track.audio_profile.is_none(),
            Some(Codec::Dts) => !self.probe_complete(track),
            Some(Codec::Unknown) if self.kind == TrackKind::Audio => {
                track.codec_name == Some(Codec::Unknown) || track.channels.is_none()
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

        match self.codec_name {
            Some(Codec::Aac) => crossed_probe_boundary(before_len, after_len, 64 * 1024, 16 * 1024),
            Some(Codec::H264)
            | Some(Codec::Hevc)
            | Some(Codec::Vc1)
            | Some(Codec::Mpeg1Video)
            | Some(Codec::Mpeg2Video) => {
                crossed_probe_boundary(before_len, after_len, 4 * 1024, 4 * 1024)
            }
            _ => crossed_probe_boundary(before_len, after_len, 2 * 1024, 2 * 1024),
        }
    }
}

fn crossed_probe_boundary(before_len: usize, after_len: usize, floor: usize, step: usize) -> bool {
    after_len >= floor && (before_len < floor || before_len / step != after_len / step)
}

fn is_plausible_frame_rate(frame_rate_fps: Option<f64>) -> bool {
    frame_rate_fps.is_some_and(|fps| (1.0..=240.0).contains(&fps))
}

fn should_use_pts_frame_rate(
    codec_name: Option<&Codec>,
    existing: Option<f64>,
    observed: f64,
) -> bool {
    if !is_plausible_frame_rate(Some(observed)) {
        return false;
    }
    if !is_plausible_frame_rate(existing) {
        return true;
    }
    if matches!(codec_name, Some(Codec::H264 | Codec::Hevc))
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

    let ticks = (last_pts? + (1_u64 << 33) - first_pts) % (1_u64 << 33);
    // Equal timestamps and backward resets do not establish a 26-hour movie.
    (ticks > 0 && ticks < (1_u64 << 32)).then_some(ticks as f64 / PTS_HZ)
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
            if let Some(pts) = extract_pts_from_pes(payload)
                && result.is_none_or(|previous| {
                    if first_match {
                        pts_after(previous, pts)
                    } else {
                        pts_after(pts, previous)
                    }
                })
            {
                result = Some(pts);
            }
        }

        offset += layout.raw_packet_size;
    }

    result
}

fn pts_after(candidate: u64, previous: u64) -> bool {
    let delta = (candidate + (1_u64 << 33) - previous) % (1_u64 << 33);
    delta > 0 && delta < (1_u64 << 32)
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
) -> Result<bool, ProbeError> {
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
) -> Result<bool, ProbeError> {
    let current = stream.stream_position().map_err(ProbeError::from)?;
    let rewind = current.saturating_sub((layout.raw_packet_size - 1) as u64);

    for offset in 0..layout.raw_packet_size {
        let candidate_start = rewind + offset as u64;
        stream
            .seek(SeekFrom::Start(candidate_start))
            .map_err(ProbeError::from)?;
        if read_full(stream, raw_packet) < layout.raw_packet_size {
            return Ok(false);
        }
        if raw_packet[layout.sync_offset] == SYNC_BYTE {
            return Ok(true);
        }
    }

    Ok(false)
}

fn detect_ts_packet_layout<T: Read + Seek>(stream: &mut T) -> Result<TsPacketLayout, ProbeError> {
    let current = stream.stream_position().map_err(ProbeError::from)?;
    stream.seek(SeekFrom::Start(0)).map_err(ProbeError::from)?;

    let probe_len = TS_FEC_PACKET_SIZE * 100;
    let mut probe = vec![0u8; probe_len];
    let n = read_full(stream, &mut probe);
    probe.truncate(n);

    stream
        .seek(SeekFrom::Start(current))
        .map_err(ProbeError::from)?;

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
        .ok_or_else(|| ProbeError::Malformed("unable to determine TS packet layout".into()))
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
mod tests;
