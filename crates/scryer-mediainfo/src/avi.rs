use crate::MediaInfoError;
use crate::types::{RawContainer, RawTrack, TrackKind};
use std::io::{Read, Seek};
use std::path::Path;

#[derive(Debug, Clone)]
struct AviTrack {
    raw: RawTrack,
    stream_number: usize,
    duration_seconds: Option<f64>,
    declared_payload_bytes: Option<u64>,
    index_bytes: u64,
}

/// Parse an AVI (RIFF) container and extract stream metadata.
pub(crate) fn parse_avi(path: &Path) -> Result<RawContainer, MediaInfoError> {
    let mut file = std::fs::File::open(path).map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let riff_chunk = riff::Chunk::read(&mut file, 0)
        .map_err(|e| MediaInfoError::Parse(format!("failed to read RIFF header: {e}")))?;

    if riff_chunk.id() != riff::RIFF_ID {
        return Err(MediaInfoError::Parse("not a RIFF file".into()));
    }

    let riff_type = riff_chunk
        .read_type(&mut file)
        .map_err(|e| MediaInfoError::Parse(format!("failed to read RIFF type: {e}")))?;

    if riff_type.as_str() != "AVI " {
        return Err(MediaInfoError::Parse(format!(
            "RIFF type is '{}', expected 'AVI '",
            riff_type.as_str()
        )));
    }

    // Collect top-level chunk offsets first to avoid borrow conflicts
    let top_chunks = collect_child_offsets(&riff_chunk, &mut file)?;

    let mut duration_seconds: Option<f64> = None;
    let mut tracks = Vec::new();
    let mut idx1_offset = None;

    for offset in top_chunks {
        let child = riff::Chunk::read(&mut file, offset)
            .map_err(|e| MediaInfoError::Parse(format!("error reading RIFF chunk: {e}")))?;

        if child.id() == riff::LIST_ID {
            let list_type = child
                .read_type(&mut file)
                .map_err(|e| MediaInfoError::Parse(format!("error reading LIST type: {e}")))?;

            if list_type.as_str() == "hdrl" {
                parse_hdrl(&child, &mut file, &mut duration_seconds, &mut tracks)?;
            }
        } else if child.id().as_str() == "idx1" {
            idx1_offset = Some(offset);
        }
    }

    if let Some(offset) = idx1_offset {
        let idx1 = riff::Chunk::read(&mut file, offset)
            .map_err(|e| MediaInfoError::Parse(format!("error reading idx1 chunk: {e}")))?;
        apply_idx1_stream_sizes(&idx1, &mut file, &mut tracks)?;
    }

    backfill_track_bitrates(&mut file, &mut tracks)?;

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

/// Parse the 'hdrl' LIST: extract the main AVI header and per-stream headers.
fn parse_hdrl<T: Read + Seek>(
    hdrl: &riff::Chunk,
    stream: &mut T,
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

        let id_str = child.id().as_str().to_owned();

        if id_str == "avih" {
            let data = child
                .read_contents(stream)
                .map_err(|e| MediaInfoError::Parse(format!("error reading avih: {e}")))?;
            if data.len() >= 48 {
                micro_sec_per_frame = Some(read_u32_le(&data, 0));
                total_frames = Some(read_u32_le(&data, 16));
            }
        } else if child.id() == riff::LIST_ID {
            let list_type = child.read_type(stream).map_err(|e| {
                MediaInfoError::Parse(format!("error reading LIST type in hdrl: {e}"))
            })?;
            if list_type.as_str() == "strl" {
                if let Some(track) = parse_strl(&child, stream)? {
                    tracks.push(AviTrack {
                        raw: track,
                        stream_number: stream_number as usize,
                        duration_seconds: parse_stream_duration(&child, stream)?,
                        declared_payload_bytes: parse_declared_payload_bytes(&child, stream)?,
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
) -> Result<Option<RawTrack>, MediaInfoError> {
    let child_offsets = collect_child_offsets(strl, stream)?;

    let mut strh_data: Option<Vec<u8>> = None;
    let mut strf_data: Option<Vec<u8>> = None;

    for offset in child_offsets {
        let child = riff::Chunk::read(stream, offset)
            .map_err(|e| MediaInfoError::Parse(format!("error in strl: {e}")))?;

        let id_str = child.id().as_str().to_owned();

        if id_str == "strh" {
            strh_data = Some(
                child
                    .read_contents(stream)
                    .map_err(|e| MediaInfoError::Parse(format!("error reading strh: {e}")))?,
            );
        } else if id_str == "strf" {
            strf_data = Some(
                child
                    .read_contents(stream)
                    .map_err(|e| MediaInfoError::Parse(format!("error reading strf: {e}")))?,
            );
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

    match fcc_type {
        b"vids" => Ok(Some(parse_video_stream(&strh, &strf))),
        b"auds" => Ok(Some(parse_audio_stream(&strh, &strf))),
        _ => Ok(None),
    }
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
        let n_channels = read_u16_le(strf, 2);
        let n_avg_bytes_per_sec = read_u32_le(strf, 8);

        codec_id = format!("0x{w_format_tag:04X}");
        codec_name = Some(map_audio_format_tag(w_format_tag).to_owned());
        channels = Some(n_channels as i32);
        bit_rate_bps = Some(n_avg_bytes_per_sec as i64 * 8);
    }

    RawTrack {
        kind: TrackKind::Audio,
        codec_id,
        codec_name,
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

fn parse_stream_duration<T: Read + Seek>(
    strl: &riff::Chunk,
    stream: &mut T,
) -> Result<Option<f64>, MediaInfoError> {
    let child_offsets = collect_child_offsets(strl, stream)?;
    for offset in child_offsets {
        let child = riff::Chunk::read(stream, offset)
            .map_err(|e| MediaInfoError::Parse(format!("error in strl: {e}")))?;
        if child.id().as_str() != "strh" {
            continue;
        }
        let data = child
            .read_contents(stream)
            .map_err(|e| MediaInfoError::Parse(format!("error reading strh: {e}")))?;
        if data.len() < 36 {
            return Ok(None);
        }
        let dw_scale = read_u32_le(&data, 20);
        let dw_rate = read_u32_le(&data, 24);
        let dw_length = read_u32_le(&data, 32);
        if dw_scale > 0 && dw_rate > 0 && dw_length > 0 {
            return Ok(Some(dw_length as f64 * dw_scale as f64 / dw_rate as f64));
        }
        return Ok(None);
    }

    Ok(None)
}

fn parse_declared_payload_bytes<T: Read + Seek>(
    strl: &riff::Chunk,
    stream: &mut T,
) -> Result<Option<u64>, MediaInfoError> {
    let child_offsets = collect_child_offsets(strl, stream)?;
    for offset in child_offsets {
        let child = riff::Chunk::read(stream, offset)
            .map_err(|e| MediaInfoError::Parse(format!("error in strl: {e}")))?;
        if child.id().as_str() != "strh" {
            continue;
        }
        let data = child
            .read_contents(stream)
            .map_err(|e| MediaInfoError::Parse(format!("error reading strh: {e}")))?;
        if data.len() < 48 {
            return Ok(None);
        }
        let dw_length = read_u32_le(&data, 32);
        let dw_sample_size = read_u32_le(&data, 44);
        if dw_length > 0 && dw_sample_size > 0 {
            return Ok(Some(u64::from(dw_length) * u64::from(dw_sample_size)));
        }
        return Ok(None);
    }

    Ok(None)
}

fn apply_idx1_stream_sizes<T: Read + Seek>(
    idx1: &riff::Chunk,
    stream: &mut T,
    tracks: &mut [AviTrack],
) -> Result<(), MediaInfoError> {
    stream
        .seek(std::io::SeekFrom::Start(idx1.offset() + 8))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut remaining = idx1.len() as usize;
    let mut entry = [0_u8; 16];
    while remaining >= entry.len() {
        stream
            .read_exact(&mut entry)
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        remaining -= entry.len();

        let Some(stream_number) = parse_idx1_stream_number(&entry[..2]) else {
            continue;
        };
        let Some(track) = tracks
            .iter_mut()
            .find(|track| track.stream_number == stream_number)
        else {
            continue;
        };
        track.index_bytes += u64::from(read_u32_le(&entry, 12));
    }

    Ok(())
}

fn parse_idx1_stream_number(prefix: &[u8]) -> Option<usize> {
    let value = std::str::from_utf8(prefix).ok()?;
    usize::from_str_radix(value, 16).ok()
}

fn backfill_track_bitrates<T: Read + Seek>(
    stream: &mut T,
    tracks: &mut [AviTrack],
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

    stream
        .rewind()
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut buf = vec![0_u8; 1024 * 1024];
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

    for i in 0..=data.len() - 4 {
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
