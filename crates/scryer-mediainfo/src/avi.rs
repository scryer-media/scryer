use crate::scan::{self, AudioSyncKind};
use crate::types::{RawContainer, RawTrack, TrackKind};
use crate::{AnalysisProfile, MediaInfoError};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const AVI_IDX1_READ_BATCH_BYTES: usize = 64 * 1024;
const AVI_MP3_FALLBACK_SCAN_BYTES: usize = 1024 * 1024;
const AVI_HEADER_CHUNK_MAX_BYTES: u32 = 64 * 1024;

#[derive(Debug, Clone)]
struct AviTrack {
    raw: RawTrack,
    stream_number: usize,
    duration_seconds: Option<f64>,
    declared_payload_bytes: Option<u64>,
    index_bytes: u64,
}

struct ParsedAviStream {
    raw: RawTrack,
    duration_seconds: Option<f64>,
    declared_payload_bytes: Option<u64>,
}

fn chunk_id_matches(chunk_id: riff::ChunkId, expected: &[u8; 4]) -> bool {
    chunk_id.value == *expected
}

fn format_chunk_id(chunk_id: riff::ChunkId) -> String {
    std::str::from_utf8(&chunk_id.value)
        .map(|value| format!("'{value}'"))
        .unwrap_or_else(|_| format!("bytes {:02X?}", chunk_id.value))
}

/// Parse an AVI (RIFF) container and extract stream metadata.
pub(crate) fn parse_avi(
    path: &Path,
    profile: AnalysisProfile,
) -> Result<RawContainer, MediaInfoError> {
    let mut file = std::fs::File::open(path).map_err(|e| MediaInfoError::Io(e.to_string()))?;
    let file_len = file
        .metadata()
        .map_err(|e| MediaInfoError::Io(e.to_string()))?
        .len();

    let riff_chunk = riff::Chunk::read(&mut file, 0)
        .map_err(|e| MediaInfoError::Parse(format!("failed to read RIFF header: {e}")))?;

    if riff_chunk.id() != riff::RIFF_ID {
        return Err(MediaInfoError::Parse("not a RIFF file".into()));
    }

    let riff_type = riff_chunk
        .read_type(&mut file)
        .map_err(|e| MediaInfoError::Parse(format!("failed to read RIFF type: {e}")))?;

    if !chunk_id_matches(riff_type, b"AVI ") {
        return Err(MediaInfoError::Parse(format!(
            "RIFF type is {}, expected 'AVI '",
            format_chunk_id(riff_type)
        )));
    }

    // Collect top-level chunk offsets first to avoid borrow conflicts
    let top_chunks = collect_child_offsets(&riff_chunk, &mut file)?;

    let mut duration_seconds: Option<f64> = None;
    let mut tracks = Vec::new();
    let mut idx1_offset = None;
    let mut movi_payload_offset = None;

    for offset in top_chunks {
        let child = riff::Chunk::read(&mut file, offset)
            .map_err(|e| MediaInfoError::Parse(format!("error reading RIFF chunk: {e}")))?;

        if child.id() == riff::LIST_ID {
            let list_type = child
                .read_type(&mut file)
                .map_err(|e| MediaInfoError::Parse(format!("error reading LIST type: {e}")))?;

            if chunk_id_matches(list_type, b"hdrl") {
                parse_hdrl(
                    &child,
                    &mut file,
                    file_len,
                    &mut duration_seconds,
                    &mut tracks,
                )?;
            } else if chunk_id_matches(list_type, b"movi") {
                movi_payload_offset = Some(child.offset() + 12);
            }
        } else if chunk_id_matches(child.id(), b"idx1") {
            idx1_offset = Some(offset);
        }
    }

    if let Some(offset) = idx1_offset {
        let idx1 = riff::Chunk::read(&mut file, offset)
            .map_err(|e| MediaInfoError::Parse(format!("error reading idx1 chunk: {e}")))?;
        apply_idx1_stream_sizes(&idx1, &mut file, &mut tracks)?;
    }

    backfill_track_bitrates(&mut file, movi_payload_offset, &mut tracks, profile)?;

    Ok(RawContainer {
        format_name: "avi".into(),
        duration_seconds,
        num_chapters: None,
        tracks: tracks.into_iter().map(|track| track.raw).collect(),
    })
}

/// Collect child chunk byte-offsets from a parent chunk's iterator, so we can
/// re-read each child independently without holding a borrow on the stream.
fn collect_child_offsets<T: Read + Seek>(
    parent: &riff::Chunk,
    stream: &mut T,
) -> Result<Vec<u64>, MediaInfoError> {
    let mut offsets = Vec::new();
    for child_result in parent.iter(stream) {
        let child = child_result
            .map_err(|e| MediaInfoError::Parse(format!("error iterating chunks: {e}")))?;
        offsets.push(child.offset());
    }
    Ok(offsets)
}

/// Read a metadata chunk only after proving its declared content lies inside
/// both its parent LIST and the physical input, with a bounded allocation.
fn read_header_chunk_contents<T: Read + Seek>(
    parent: &riff::Chunk,
    child: &riff::Chunk,
    stream: &mut T,
    file_len: u64,
) -> Result<Vec<u8>, MediaInfoError> {
    let parent_contents_start = parent
        .offset()
        .checked_add(12)
        .ok_or_else(|| MediaInfoError::Parse("AVI parent chunk offset overflow".into()))?;
    let parent_contents_end = parent
        .offset()
        .checked_add(8)
        .and_then(|offset| offset.checked_add(u64::from(parent.len())))
        .ok_or_else(|| MediaInfoError::Parse("AVI parent chunk size overflow".into()))?;
    let contents_start = child
        .offset()
        .checked_add(8)
        .ok_or_else(|| MediaInfoError::Parse("AVI chunk offset overflow".into()))?;
    let contents_end = contents_start
        .checked_add(u64::from(child.len()))
        .ok_or_else(|| MediaInfoError::Parse("AVI chunk size overflow".into()))?;

    if child.offset() < parent_contents_start
        || contents_end > parent_contents_end
        || contents_end > file_len
    {
        return Err(MediaInfoError::Parse(format!(
            "AVI chunk {} exceeds enclosing or file bounds",
            format_chunk_id(child.id())
        )));
    }
    if child.len() > AVI_HEADER_CHUNK_MAX_BYTES {
        return Err(MediaInfoError::Parse(format!(
            "AVI chunk {} exceeds parser budget",
            format_chunk_id(child.id())
        )));
    }

    let mut data = vec![0; child.len() as usize];
    stream
        .seek(SeekFrom::Start(contents_start))
        .and_then(|_| stream.read_exact(&mut data))
        .map_err(|e| MediaInfoError::Parse(format!("error reading AVI header chunk: {e}")))?;
    Ok(data)
}

/// Parse the 'hdrl' LIST: extract the main AVI header and per-stream headers.
fn parse_hdrl<T: Read + Seek>(
    hdrl: &riff::Chunk,
    stream: &mut T,
    file_len: u64,
    duration_seconds: &mut Option<f64>,
    tracks: &mut Vec<AviTrack>,
) -> Result<(), MediaInfoError> {
    let child_offsets = collect_child_offsets(hdrl, stream)?;

    let mut micro_sec_per_frame: Option<u32> = None;
    let mut total_frames: Option<u32> = None;
    let mut stream_number = 0_u8;

    for offset in child_offsets {
        let child = riff::Chunk::read(stream, offset)
            .map_err(|e| MediaInfoError::Parse(format!("error in hdrl: {e}")))?;
        if chunk_id_matches(child.id(), b"avih") {
            let data = read_header_chunk_contents(hdrl, &child, stream, file_len)?;
            if data.len() >= 48 {
                micro_sec_per_frame = Some(read_u32_le(&data, 0));
                total_frames = Some(read_u32_le(&data, 16));
            }
        } else if child.id() == riff::LIST_ID {
            let list_type = child.read_type(stream).map_err(|e| {
                MediaInfoError::Parse(format!("error reading LIST type in hdrl: {e}"))
            })?;
            if chunk_id_matches(list_type, b"strl") {
                if let Some(parsed) = parse_strl(&child, stream, file_len)? {
                    tracks.push(AviTrack {
                        raw: parsed.raw,
                        stream_number: stream_number as usize,
                        duration_seconds: parsed.duration_seconds,
                        declared_payload_bytes: parsed.declared_payload_bytes,
                        index_bytes: 0,
                    });
                }
                stream_number = stream_number.saturating_add(1);
            }
        }
    }

    // Compute duration from avih fields
    if let (Some(usec), Some(frames)) = (micro_sec_per_frame, total_frames)
        && usec > 0
        && frames > 0
    {
        *duration_seconds = Some((usec as f64 * frames as f64) / 1_000_000.0);
    }

    Ok(())
}

/// Parse a single 'strl' LIST (one stream). Returns `None` if the stream type
/// is neither video nor audio.
fn parse_strl<T: Read + Seek>(
    strl: &riff::Chunk,
    stream: &mut T,
    file_len: u64,
) -> Result<Option<ParsedAviStream>, MediaInfoError> {
    let child_offsets = collect_child_offsets(strl, stream)?;

    let mut strh_data: Option<Vec<u8>> = None;
    let mut strf_data: Option<Vec<u8>> = None;

    for offset in child_offsets {
        let child = riff::Chunk::read(stream, offset)
            .map_err(|e| MediaInfoError::Parse(format!("error in strl: {e}")))?;
        if chunk_id_matches(child.id(), b"strh") {
            strh_data = Some(read_header_chunk_contents(strl, &child, stream, file_len)?);
        } else if chunk_id_matches(child.id(), b"strf") {
            strf_data = Some(read_header_chunk_contents(strl, &child, stream, file_len)?);
        }
    }

    let strh = match strh_data {
        Some(d) => d,
        None => return Ok(None),
    };
    let strf = match strf_data {
        Some(d) => d,
        None => return Ok(None),
    };

    if strh.len() < 56 {
        return Ok(None);
    }

    let fcc_type = &strh[0..4];
    let duration_seconds = parse_stream_duration_from_strh(&strh);
    let declared_payload_bytes = parse_declared_payload_bytes_from_strh(&strh);

    let raw = match fcc_type {
        b"vids" => parse_video_stream(&strh, &strf),
        b"auds" => parse_audio_stream(&strh, &strf),
        _ => return Ok(None),
    };

    Ok(Some(ParsedAviStream {
        raw,
        duration_seconds,
        declared_payload_bytes,
    }))
}

/// Build a video `RawTrack` from strh + strf (BITMAPINFOHEADER).
fn parse_video_stream(strh: &[u8], strf: &[u8]) -> RawTrack {
    // strh: fccHandler at offset 4..8
    let fcc_handler = if strh.len() >= 8 {
        std::str::from_utf8(&strh[4..8]).ok().map(|s| s.to_owned())
    } else {
        None
    };

    // Frame rate from strh: dwScale at offset 20, dwRate at offset 24
    let frame_rate_fps = if strh.len() >= 28 {
        let dw_scale = read_u32_le(strh, 20);
        let dw_rate = read_u32_le(strh, 24);
        if dw_scale > 0 && dw_rate > 0 {
            Some(dw_rate as f64 / dw_scale as f64)
        } else {
            None
        }
    } else {
        None
    };

    // strf: BITMAPINFOHEADER
    // biWidth at offset 4 (i32 LE), biHeight at offset 8 (i32 LE),
    // biCompression at offset 16 (4 bytes = FourCC)
    let mut width: Option<i32> = None;
    let mut height: Option<i32> = None;
    let mut codec_id = String::from("unknown");
    let mut codec_name: Option<String> = None;

    if strf.len() >= 20 {
        let bi_width = read_i32_le(strf, 4);
        let bi_height = read_i32_le(strf, 8);
        width = Some(bi_width.unsigned_abs() as i32);
        height = Some(bi_height.unsigned_abs() as i32);

        let compression_fcc = &strf[16..20];
        let compression_str = std::str::from_utf8(compression_fcc)
            .ok()
            .map(|s| s.to_owned());

        // Prefer biCompression for codec identification; fall back to fccHandler
        let fourcc = compression_str
            .as_deref()
            .filter(|s| {
                let trimmed = s.trim_end_matches('\0');
                !trimmed.is_empty() && trimmed.bytes().any(|b| b != 0)
            })
            .or(fcc_handler.as_deref());

        if let Some(fcc) = fourcc {
            let fcc_trimmed = fcc.trim_end_matches('\0');
            codec_id = fcc_trimmed.to_owned();
            codec_name = Some(map_video_fourcc(fcc_trimmed).to_owned());
        }
    }

    RawTrack {
        kind: TrackKind::Video,
        codec_id,
        codec_name,
        audio_profile: None,
        codec_private: None,
        width,
        height,
        channels: None,
        bit_rate_bps: None,
        language: None,
        frame_rate_fps,
        color_transfer: None,
        dovi_config: None,
        has_hdr10plus: false,
        name: None,
        forced: false,
        default_track: false,
    }
}

/// Build an audio `RawTrack` from strh + strf (WAVEFORMATEX).
fn parse_audio_stream(_strh: &[u8], strf: &[u8]) -> RawTrack {
    let mut codec_id = String::from("unknown");
    let mut codec_name: Option<String> = None;
    let mut channels: Option<i32> = None;
    let mut bit_rate_bps: Option<i64> = None;

    if strf.len() >= 12 {
        let w_format_tag = read_u16_le(strf, 0);
        let codec_format_tag = wave_format_extensible_subformat(strf).unwrap_or(w_format_tag);
        let n_channels = read_u16_le(strf, 2);
        let n_avg_bytes_per_sec = read_u32_le(strf, 8);

        codec_id = if codec_format_tag == w_format_tag {
            format!("0x{w_format_tag:04X}")
        } else {
            format!("0x{w_format_tag:04X}/0x{codec_format_tag:04X}")
        };
        codec_name = Some(map_audio_format_tag(codec_format_tag).to_owned());
        channels = Some(n_channels as i32);
        bit_rate_bps = Some(n_avg_bytes_per_sec as i64 * 8);
    }

    RawTrack {
        kind: TrackKind::Audio,
        codec_id,
        codec_name,
        audio_profile: None,
        codec_private: None,
        width: None,
        height: None,
        channels,
        bit_rate_bps,
        language: None,
        frame_rate_fps: None,
        color_transfer: None,
        dovi_config: None,
        has_hdr10plus: false,
        name: None,
        forced: false,
        default_track: false,
    }
}

fn parse_stream_duration_from_strh(strh: &[u8]) -> Option<f64> {
    if strh.len() < 36 {
        return None;
    }

    let dw_scale = read_u32_le(strh, 20);
    let dw_rate = read_u32_le(strh, 24);
    let dw_length = read_u32_le(strh, 32);
    (dw_scale > 0 && dw_rate > 0 && dw_length > 0)
        .then_some(dw_length as f64 * dw_scale as f64 / dw_rate as f64)
}

fn parse_declared_payload_bytes_from_strh(strh: &[u8]) -> Option<u64> {
    if strh.len() < 48 {
        return None;
    }

    let dw_length = read_u32_le(strh, 32);
    let dw_sample_size = read_u32_le(strh, 44);
    (dw_length > 0 && dw_sample_size > 0)
        .then_some(u64::from(dw_length) * u64::from(dw_sample_size))
}

fn apply_idx1_stream_sizes<T: Read + Seek>(
    idx1: &riff::Chunk,
    stream: &mut T,
    tracks: &mut [AviTrack],
) -> Result<(), MediaInfoError> {
    stream
        .seek(SeekFrom::Start(idx1.offset() + 8))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut remaining = idx1.len() as usize;
    let mut buf = vec![0_u8; AVI_IDX1_READ_BATCH_BYTES];
    while remaining >= 16 {
        let read_len = remaining.min(buf.len());
        let read_len = read_len - (read_len % 16);
        if read_len == 0 {
            break;
        }
        stream
            .read_exact(&mut buf[..read_len])
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        remaining -= read_len;

        if tracks.iter().all(|track| track.stream_number < 100) {
            let mut stream_sizes = [0_u64; 100];
            scan::accumulate_avi_idx1_stream_sizes(&buf[..read_len], &mut stream_sizes);
            for track in tracks.iter_mut() {
                track.index_bytes += stream_sizes[track.stream_number];
            }
        } else {
            for entry in buf[..read_len].chunks_exact(16) {
                let Some(stream_number) = parse_idx1_stream_number(&entry[..2]) else {
                    continue;
                };
                let Some(track) = tracks
                    .iter_mut()
                    .find(|track| track.stream_number == stream_number)
                else {
                    continue;
                };
                track.index_bytes += u64::from(read_u32_le(entry, 12));
            }
        }
    }

    Ok(())
}

fn parse_idx1_stream_number(prefix: &[u8]) -> Option<usize> {
    let first = prefix.first().copied()?;
    let second = prefix.get(1).copied()?;
    if first.is_ascii_digit() && second.is_ascii_digit() {
        return Some(usize::from(first - b'0') * 10 + usize::from(second - b'0'));
    }
    let value = std::str::from_utf8(prefix).ok()?;
    usize::from_str_radix(value, 16).ok()
}

fn backfill_track_bitrates<T: Read + Seek>(
    stream: &mut T,
    movi_payload_offset: Option<u64>,
    tracks: &mut [AviTrack],
    profile: AnalysisProfile,
) -> Result<(), MediaInfoError> {
    for track in tracks.iter_mut() {
        if track.raw.bit_rate_bps.unwrap_or_default() > 0 {
            continue;
        }

        let total_bytes = if track.index_bytes > 0 {
            Some(track.index_bytes)
        } else {
            track.declared_payload_bytes
        };
        if let (Some(total_bytes), Some(duration_seconds)) = (total_bytes, track.duration_seconds)
            && duration_seconds > 0.0
        {
            track.raw.bit_rate_bps = Some((total_bytes as f64 * 8.0 / duration_seconds) as i64);
        }
    }

    let needs_mp3_bitrate = tracks.iter().any(|track| {
        track.raw.kind == TrackKind::Audio
            && track.raw.codec_name.as_deref() == Some("mp3")
            && track.raw.bit_rate_bps.unwrap_or_default() <= 0
    });
    if !needs_mp3_bitrate {
        return Ok(());
    }
    if profile.skips_deep_probes() {
        return Ok(());
    }

    stream
        .seek(SeekFrom::Start(movi_payload_offset.unwrap_or(0)))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut buf = vec![0_u8; AVI_MP3_FALLBACK_SCAN_BYTES];
    let bytes_read = stream
        .read(&mut buf)
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;
    buf.truncate(bytes_read);

    let fallback_bitrate = find_mp3_bitrate(&buf);
    for track in tracks.iter_mut() {
        if track.raw.kind == TrackKind::Audio
            && track.raw.codec_name.as_deref() == Some("mp3")
            && track.raw.bit_rate_bps.unwrap_or_default() <= 0
        {
            track.raw.bit_rate_bps = fallback_bitrate;
        }
    }

    Ok(())
}

/// Map a video FourCC to a canonical codec name.
fn map_video_fourcc(fcc: &str) -> &'static str {
    match fcc {
        "H264" | "h264" | "X264" | "x264" | "avc1" | "AVC1" => "h264",
        "HEVC" | "hevc" | "H265" | "h265" | "hvc1" | "HVC1" | "hev1" | "HEV1" => "hevc",
        "XVID" | "xvid" | "DX50" | "dx50" | "DIVX" | "divx" | "DIV3" | "div3" | "DIV4" | "div4"
        | "DIV5" | "div5" | "MP4V" | "mp4v" | "FMP4" | "fmp4" => "mpeg4",
        "MJPG" | "mjpg" => "mjpeg",
        "WVC1" | "wvc1" => "vc1",
        "WMV3" | "wmv3" => "wmv3",
        "MP2V" | "mp2v" | "mpg2" | "MPG2" => "mpeg2video",
        "MP1V" | "mp1v" | "mpg1" | "MPG1" => "mpeg1video",
        "VP80" | "vp80" => "vp8",
        "VP90" | "vp90" => "vp9",
        _ => "unknown",
    }
}

fn wave_format_extensible_subformat(strf: &[u8]) -> Option<u16> {
    if read_u16_le(strf, 0) != 0xFFFE || strf.len() < 40 {
        return None;
    }

    // WAVEFORMATEXTENSIBLE stores the actual subformat in the first little-endian
    // word of the GUID extension after WAVEFORMATEX.
    Some(read_u16_le(strf, 24))
}

/// Map a WAVEFORMATEX wFormatTag to a canonical codec name.
fn map_audio_format_tag(tag: u16) -> &'static str {
    match tag {
        0x0001 => "pcm_s16le",
        0x0003 => "pcm_f32le",
        0x0006 => "pcm_alaw",
        0x0007 => "pcm_mulaw",
        0x0055 => "mp3",
        0x00FF => "aac",
        0x0161 => "wmav2",
        0x0162 => "wmapro",
        0x2000 => "ac3",
        0x2001 => "dts",
        0xFFFE => "extensible",
        _ => "unknown",
    }
}

fn find_mp3_bitrate(data: &[u8]) -> Option<i64> {
    if data.len() < 4 {
        return None;
    }

    const MPEG_AUDIO_SAMPLE_RATES: [[u32; 4]; 4] = [
        [11_025, 12_000, 8_000, 0],
        [0, 0, 0, 0],
        [22_050, 24_000, 16_000, 0],
        [44_100, 48_000, 32_000, 0],
    ];
    const MPEG_AUDIO_BITRATES_MPEG1_LAYER3: [u32; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const MPEG_AUDIO_BITRATES_MPEG2_LAYER3: [u32; 16] = [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ];

    let mut cursor = 0;
    while let Some(candidate) = scan::find_audio_sync_candidate(data, cursor) {
        if !matches!(
            candidate.kind,
            AudioSyncKind::MpegAudio | AudioSyncKind::Adts
        ) {
            cursor = candidate.offset + 1;
            continue;
        }

        let i = candidate.offset;
        if i + 4 > data.len() {
            return None;
        }
        cursor = i + 1;

        let header = u32::from_be_bytes(data[i..i + 4].try_into().ok()?);
        if (header & 0xFFE0_0000) != 0xFFE0_0000 {
            continue;
        }

        let version_id = ((header >> 19) & 0x3) as usize;
        let layer_index = ((header >> 17) & 0x3) as usize;
        let bitrate_index = ((header >> 12) & 0xF) as usize;
        let sample_rate_index = ((header >> 10) & 0x3) as usize;

        if version_id == 1 || layer_index != 1 || bitrate_index == 0 || bitrate_index == 0xF {
            continue;
        }

        let sample_rate = *MPEG_AUDIO_SAMPLE_RATES
            .get(version_id)?
            .get(sample_rate_index)?;
        if sample_rate == 0 {
            continue;
        }

        let bitrate_kbps = if version_id == 3 {
            MPEG_AUDIO_BITRATES_MPEG1_LAYER3[bitrate_index]
        } else {
            MPEG_AUDIO_BITRATES_MPEG2_LAYER3[bitrate_index]
        };
        if bitrate_kbps == 0 {
            continue;
        }

        return Some(i64::from(bitrate_kbps) * 1000);
    }

    None
}

/// Read a little-endian u32 from a byte slice at the given offset.
fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Read a little-endian i32 from a byte slice at the given offset.
fn read_i32_le(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Read a little-endian u16 from a byte slice at the given offset.
fn read_u16_le(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

#[cfg(test)]
mod tests {
    use super::parse_avi;
    use crate::AnalysisProfile;
    use crate::MediaInfoError;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempAviFile {
        path: PathBuf,
    }

    impl TempAviFile {
        fn new(name: &str, bytes: &[u8]) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "scryer-mediainfo-{name}-{}-{unique}.avi",
                std::process::id()
            ));
            fs::write(&path, bytes).expect("write temp AVI fixture");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempAviFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn parse_avi_rejects_non_utf8_riff_type_without_panicking() {
        let fixture = TempAviFile::new(
            "invalid-riff-type",
            &[
                b"RIFF".as_slice(),
                &4_u32.to_le_bytes(),
                &[0xFF, 0xFE, 0xFD, 0xFC],
            ]
            .concat(),
        );

        let error = parse_avi(fixture.path(), AnalysisProfile::DefaultRich)
            .expect_err("invalid RIFF type should be rejected");
        let MediaInfoError::Parse(message) = error else {
            panic!("expected parse error for invalid RIFF type");
        };
        assert!(message.contains("expected 'AVI '"));
        assert!(message.contains("FF"));
    }

    #[test]
    fn parse_avi_ignores_non_utf8_top_level_chunk_ids_without_panicking() {
        let fixture = TempAviFile::new(
            "invalid-top-level-chunk-id",
            &[
                b"RIFF".as_slice(),
                &12_u32.to_le_bytes(),
                b"AVI ".as_slice(),
                &[0xFF, 0xFE, 0xFD, 0xFC],
                &0_u32.to_le_bytes(),
            ]
            .concat(),
        );

        let container = parse_avi(fixture.path(), AnalysisProfile::DefaultRich)
            .expect("malformed chunk ids should not crash");
        assert_eq!(container.format_name, "avi");
        assert_eq!(container.duration_seconds, None);
        assert!(container.tracks.is_empty());
    }

    #[test]
    fn parse_avi_rejects_truncated_oversized_avih_without_allocating() {
        let fixture = TempAviFile::new(
            "truncated-oversized-avih",
            &[
                b"RIFF".as_slice(),
                &24_u32.to_le_bytes(),
                b"AVI ".as_slice(),
                b"LIST".as_slice(),
                &12_u32.to_le_bytes(),
                b"hdrl".as_slice(),
                b"avih".as_slice(),
                &u32::MAX.to_le_bytes(),
            ]
            .concat(),
        );

        let error = parse_avi(fixture.path(), AnalysisProfile::DefaultRich)
            .expect_err("truncated avih must be rejected before allocation");
        let MediaInfoError::Parse(message) = error else {
            panic!("expected parse error for oversized avih");
        };
        assert!(message.contains("exceeds enclosing or file bounds"));
    }

    #[test]
    fn find_mp3_bitrate_uses_late_sync_candidate() {
        let mut data = vec![0x55; 4096];
        data.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);

        assert_eq!(super::find_mp3_bitrate(&data), Some(128_000));
    }
}
