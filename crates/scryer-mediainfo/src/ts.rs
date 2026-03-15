use crate::MediaInfoError;
use crate::types::{RawContainer, RawTrack, TrackKind};
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

/// Parse an MPEG Transport Stream file and extract stream metadata.
pub(crate) fn parse_ts(path: &Path) -> Result<RawContainer, MediaInfoError> {
    let mut file = std::fs::File::open(path).map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let file_size = file
        .metadata()
        .map_err(|e| MediaInfoError::Io(e.to_string()))?
        .len();

    // Step 1: Locate PAT to find the PMT PID
    let pmt_pid = find_pmt_pid(&mut file)?;

    // Step 2: Parse PMT to enumerate elementary streams
    let es_entries = parse_pmt(&mut file, pmt_pid)?;

    // Step 3: Build tracks from PMT stream info
    let tracks: Vec<RawTrack> = es_entries.iter().map(build_track).collect();

    // Step 4: Estimate duration from first and last PTS values
    let duration_seconds = estimate_duration(&mut file, file_size, &es_entries);

    Ok(RawContainer {
        format_name: "mpegts".into(),
        duration_seconds,
        num_chapters: None,
        tracks,
    })
}

/// Elementary stream entry extracted from PMT.
struct EsEntry {
    stream_type: u8,
    pid: u16,
    descriptors: Vec<u8>,
}

// ---------------------------------------------------------------------------
// PAT parsing
// ---------------------------------------------------------------------------

/// Scan TS packets for PID 0 (PAT) and extract the first program's PMT PID.
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

        if buf[0] != SYNC_BYTE {
            // Try to resync: skip one byte and look for next sync
            if !resync(stream, &mut buf)? {
                return Err(MediaInfoError::Parse("could not sync to TS packets".into()));
            }
        }

        let pid = ts_pid(&buf);
        if pid != PAT_PID {
            continue;
        }

        // payload_unit_start_indicator must be set
        if buf[1] & 0x40 == 0 {
            continue;
        }

        let payload = ts_payload(&buf);
        if payload.is_empty() {
            continue;
        }

        // PAT payload starts with a pointer_field byte
        let pointer = payload[0] as usize;
        let section_start = 1 + pointer;
        if section_start >= payload.len() {
            continue;
        }
        let section = &payload[section_start..];

        // table_id should be 0x00 for PAT
        if section.is_empty() || section[0] != 0x00 {
            continue;
        }

        // section_length
        if section.len() < 8 {
            continue;
        }
        let section_length = ((section[1] as u16 & 0x0F) << 8 | section[2] as u16) as usize;
        let available = section.len().saturating_sub(3);
        let data_len = section_length.min(available);

        // Skip: transport_stream_id (2), reserved/version/current (1),
        //       section_number (1), last_section_number (1)
        // = 5 bytes, then program entries (4 bytes each), then CRC32 (4 bytes)
        if data_len < 9 {
            continue;
        }
        let program_data = &section[8..3 + data_len.saturating_sub(4)];

        // Each program entry is 4 bytes: program_number(2) + reserved/PID(2)
        for chunk in program_data.chunks_exact(4) {
            let program_number = (chunk[0] as u16) << 8 | chunk[1] as u16;
            let entry_pid = (chunk[2] as u16 & 0x1F) << 8 | chunk[3] as u16;

            if program_number == 0 {
                // Network PID, skip
                continue;
            }

            return Ok(entry_pid);
        }
    }
}

// ---------------------------------------------------------------------------
// PMT parsing
// ---------------------------------------------------------------------------

/// Scan TS packets for the given PMT PID and extract elementary stream entries.
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

        let pid = ts_pid(&buf);
        if pid != pmt_pid {
            continue;
        }

        if buf[1] & 0x40 == 0 {
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

        // table_id should be 0x02 for PMT
        if section.is_empty() || section[0] != 0x02 {
            continue;
        }

        if section.len() < 12 {
            continue;
        }

        let section_length = ((section[1] as u16 & 0x0F) << 8 | section[2] as u16) as usize;
        let available = section.len().saturating_sub(3);
        let data_len = section_length.min(available);

        // section[10..12] = reserved/program_info_length
        let program_info_length = ((section[10] as u16 & 0x0F) << 8 | section[11] as u16) as usize;

        let es_start = 12 + program_info_length;
        // CRC is last 4 bytes of section_length data
        let es_end = (3 + data_len).saturating_sub(4);

        if es_start > section.len() || es_end > section.len() || es_start > es_end {
            continue;
        }

        let es_data = &section[es_start..es_end];
        let mut entries = Vec::new();
        let mut pos = 0;

        while pos + 5 <= es_data.len() {
            let st = es_data[pos];
            let es_pid = ((es_data[pos + 1] as u16 & 0x1F) << 8) | es_data[pos + 2] as u16;
            let es_info_length =
                ((es_data[pos + 3] as u16 & 0x0F) << 8 | es_data[pos + 4] as u16) as usize;

            let desc_end = (pos + 5 + es_info_length).min(es_data.len());
            let descriptors = es_data[pos + 5..desc_end].to_vec();

            entries.push(EsEntry {
                stream_type: st,
                pid: es_pid,
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

/// Build a `RawTrack` from a PMT elementary stream entry.
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
        dovi_config: None,
        has_hdr10plus: false,
        name: None,
        forced: false,
        default_track: false,
    }
}

/// Map a PMT stream_type byte to a (TrackKind, codec name).
fn classify_stream_type(stream_type: u8, descriptors: &[u8]) -> (TrackKind, &'static str) {
    match stream_type {
        0x01 => (TrackKind::Video, "mpeg1video"),
        0x02 => (TrackKind::Video, "mpeg2video"),
        0x10 => (TrackKind::Video, "mpeg4"),
        0x1B => (TrackKind::Video, "h264"),
        0x24 => (TrackKind::Video, "hevc"),
        0x03 | 0x04 => (TrackKind::Audio, "mp2"),
        0x0F => (TrackKind::Audio, "aac"),
        0x11 => (TrackKind::Audio, "aac_latm"),
        0x81 => (TrackKind::Audio, "ac3"),
        0x87 => (TrackKind::Audio, "eac3"),
        0x06 => classify_private_pes(descriptors),
        _ if stream_type >= 0x80 => (TrackKind::Video, "unknown"),
        _ => (TrackKind::Video, "unknown"),
    }
}

/// For stream_type 0x06 (PES private data), inspect descriptors to identify codec.
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

/// Extract ISO 639 language code from descriptor tag 0x0A.
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

// ---------------------------------------------------------------------------
// Duration estimation via PTS
// ---------------------------------------------------------------------------

/// Estimate duration by reading the first and last PTS values from any PES stream.
fn estimate_duration<T: Read + Seek>(
    stream: &mut T,
    file_size: u64,
    es_entries: &[EsEntry],
) -> Option<f64> {
    if es_entries.is_empty() || file_size < TS_PACKET_SIZE as u64 {
        return None;
    }

    // Collect PIDs for PES streams (video/audio)
    let pes_pids: Vec<u16> = es_entries
        .iter()
        .filter(|e| is_pes_stream_type(e.stream_type))
        .map(|e| e.pid)
        .collect();

    if pes_pids.is_empty() {
        return None;
    }

    // Find first PTS from beginning of file
    let first_pts = find_pts_near(stream, 0, true, &pes_pids, 50_000);

    // Find last PTS from end of file
    let tail_start = file_size.saturating_sub(TS_PACKET_SIZE as u64 * 50_000);
    let last_pts = find_pts_near(stream, tail_start, false, &pes_pids, 50_000);

    match (first_pts, last_pts) {
        (Some(first), Some(last)) if last > first => Some((last - first) as f64 / PTS_HZ),
        (Some(first), Some(last)) if last <= first => {
            // PTS wrap-around (33-bit counter)
            let wrapped = (1u64 << 33) - first + last;
            Some(wrapped as f64 / PTS_HZ)
        }
        _ => None,
    }
}

/// Returns true if the stream_type represents PES data.
fn is_pes_stream_type(st: u8) -> bool {
    matches!(
        st,
        0x01 | 0x02 | 0x03 | 0x04 | 0x06 | 0x0F | 0x10 | 0x11 | 0x1B | 0x24 | 0x81 | 0x87
    )
}

/// Scan packets starting from `start_pos`, looking for a PES header with a PTS
/// value on one of the given PIDs. If `first_match` is true, return the first PTS
/// found; otherwise return the last PTS found in the scanned region.
fn find_pts_near<T: Read + Seek>(
    stream: &mut T,
    start_pos: u64,
    first_match: bool,
    pes_pids: &[u16],
    max_packets: usize,
) -> Option<u64> {
    stream.seek(SeekFrom::Start(start_pos)).ok()?;

    // Read a chunk of packets
    let read_size = max_packets * TS_PACKET_SIZE;
    let mut data = vec![0u8; read_size];
    let n = read_full(stream, &mut data);
    data.truncate(n);

    let mut result: Option<u64> = None;

    // Align to sync byte
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
            // payload_unit_start is set: this is the start of a PES packet
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

/// Extract the PTS value from the beginning of a PES packet payload.
/// PES header: 00 00 01 stream_id PES_packet_length(2) flags(2)
///             PES_header_data_length(1) ...
fn extract_pts_from_pes(payload: &[u8]) -> Option<u64> {
    // Minimum PES header: 3 (start code) + 1 (stream_id) + 2 (length) + 2 (flags)
    //                     + 1 (hdr_len) = 9
    if payload.len() < 9 {
        return None;
    }

    // Check PES start code: 0x00 0x00 0x01
    if payload[0] != 0x00 || payload[1] != 0x00 || payload[2] != 0x01 {
        return None;
    }

    // flags byte at offset 7: PTS_DTS_flags in bits 7-6
    let pts_dts_flags = (payload[7] >> 6) & 0x03;
    if pts_dts_flags < 2 {
        // No PTS present
        return None;
    }

    // PTS is 5 bytes starting at offset 9
    if payload.len() < 14 {
        return None;
    }

    parse_pts_bytes(&payload[9..14])
}

/// Parse 5 bytes of PTS/DTS timestamp.
/// Format: 4 bits marker | 3 bits PTS[32..30] | 1 bit marker |
///         15 bits PTS[29..15] | 1 bit marker |
///         15 bits PTS[14..0]  | 1 bit marker
fn parse_pts_bytes(data: &[u8]) -> Option<u64> {
    // Check marker bits (bit 0 of bytes 0, 2, 4 must be 1)
    if data[0] & 0x01 == 0 || data[2] & 0x01 == 0 || data[4] & 0x01 == 0 {
        return None;
    }

    let pts = ((data[0] as u64 >> 1) & 0x07) << 30
        | (data[1] as u64) << 22
        | ((data[2] as u64 >> 1) & 0x7F) << 15
        | (data[3] as u64) << 7
        | (data[4] as u64 >> 1) & 0x7F;

    Some(pts)
}

// ---------------------------------------------------------------------------
// TS packet helpers
// ---------------------------------------------------------------------------

/// Extract the 13-bit PID from a TS packet.
fn ts_pid(pkt: &[u8]) -> u16 {
    ((pkt[1] as u16 & 0x1F) << 8) | pkt[2] as u16
}

/// Extract the payload bytes from a TS packet, accounting for the adaptation field.
fn ts_payload(pkt: &[u8]) -> &[u8] {
    let adaptation_field_control = (pkt[3] >> 4) & 0x03;

    let offset = match adaptation_field_control {
        // 0b01: payload only
        0b01 => 4,
        // 0b11: adaptation field + payload
        0b11 => {
            let af_length = pkt[4] as usize;
            5 + af_length
        }
        // 0b10: adaptation field only, no payload
        // 0b00: reserved
        _ => return &[],
    };

    if offset >= TS_PACKET_SIZE {
        return &[];
    }
    &pkt[offset..]
}

/// Try to resync to the next TS packet by scanning for a sync byte.
fn resync<T: Read + Seek>(
    stream: &mut T,
    buf: &mut [u8; TS_PACKET_SIZE],
) -> Result<bool, MediaInfoError> {
    // Back up to just after the failed sync byte
    stream
        .seek(SeekFrom::Current(-(TS_PACKET_SIZE as i64 - 1)))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut single = [0u8; 1];
    for _ in 0..TS_PACKET_SIZE * 10 {
        if read_full(stream, &mut single) == 0 {
            return Ok(false);
        }
        if single[0] == SYNC_BYTE {
            // Read the rest of the packet
            buf[0] = SYNC_BYTE;
            let n = read_full(stream, &mut buf[1..]);
            return Ok(n == TS_PACKET_SIZE - 1);
        }
    }
    Ok(false)
}

/// Read exactly `buf.len()` bytes, returning the number actually read (may be less
/// at EOF).
fn read_full<T: Read>(stream: &mut T, buf: &mut [u8]) -> usize {
    let mut total = 0;
    while total < buf.len() {
        match stream.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(_) => break,
        }
    }
    total
}
