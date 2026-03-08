use crate::types::{RawContainer, RawTrack, TrackKind};
use crate::MediaInfoError;
use std::io::{Read, Seek};
use std::path::Path;

/// Parse an AVI (RIFF) container and extract stream metadata.
pub(crate) fn parse_avi(path: &Path) -> Result<RawContainer, MediaInfoError> {
    let mut file =
        std::fs::File::open(path).map_err(|e| MediaInfoError::Io(e.to_string()))?;

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
        }
    }

    Ok(RawContainer {
        format_name: "avi".into(),
        duration_seconds,
        tracks,
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
    tracks: &mut Vec<RawTrack>,
) -> Result<(), MediaInfoError> {
    let child_offsets = collect_child_offsets(hdrl, stream)?;

    let mut micro_sec_per_frame: Option<u32> = None;
    let mut total_frames: Option<u32> = None;

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
                    tracks.push(track);
                }
            }
        }
    }

    // Compute duration from avih fields
    if let (Some(usec), Some(frames)) = (micro_sec_per_frame, total_frames) {
        if usec > 0 && frames > 0 {
            *duration_seconds = Some((usec as f64 * frames as f64) / 1_000_000.0);
        }
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
    }
}

/// Map a video FourCC to a canonical codec name.
fn map_video_fourcc(fcc: &str) -> &'static str {
    match fcc {
        "H264" | "h264" | "X264" | "x264" | "avc1" | "AVC1" => "h264",
        "HEVC" | "hevc" | "H265" | "h265" | "hvc1" | "HVC1" | "hev1" | "HEV1" => "hevc",
        "XVID" | "xvid" | "DX50" | "dx50" | "DIVX" | "divx" | "DIV3" | "div3" | "DIV4"
        | "div4" | "DIV5" | "div5" | "MP4V" | "mp4v" | "FMP4" | "fmp4" => "mpeg4",
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
