use crate::MediaInfoError;
use crate::codec;
use crate::types::{RawContainer, RawTrack, TrackKind};
use std::io::{Read, SeekFrom};

const MAX_SCAN_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TAGS: usize = 8_192;
const MAX_TAG_PAYLOAD: usize = 16 * 1024 * 1024;
const MAX_RESYNC_BYTES: u64 = 1024 * 1024;
const MAX_AMF_DEPTH: usize = 16;
const MAX_AMF_ENTRIES: usize = 65_536;
const MAX_AMF_STRING: usize = 1024 * 1024;

#[derive(Default)]
struct FlvMetadata {
    duration_seconds: Option<f64>,
    width: Option<i32>,
    height: Option<i32>,
    frame_rate_fps: Option<f64>,
    video_bitrate_bps: Option<i64>,
    audio_bitrate_bps: Option<i64>,
}

pub(crate) fn parse_flv_source(
    mut file: &mut dyn crate::source::MediaSource,
) -> Result<RawContainer, MediaInfoError> {
    let file_len = file.len();
    let mut header = [0_u8; 9];
    file.read_exact(&mut header)
        .map_err(|_| parse_error("truncated FLV header"))?;
    if &header[..3] != b"FLV" || header[3] >= 5 {
        return Err(parse_error("invalid FLV header"));
    }
    let data_offset = u32::from_be_bytes(header[5..9].try_into().unwrap()) as u64;
    if data_offset < 9 || data_offset.checked_add(4).is_none_or(|end| end > file_len) {
        return Err(parse_error("invalid FLV data offset"));
    }
    file.seek(SeekFrom::Start(data_offset))?;
    let previous_tag_zero = read_u32_be(&mut file)?;
    if previous_tag_zero != 0 {
        return Err(parse_error("invalid FLV PreviousTagSize0"));
    }

    let mut metadata = FlvMetadata::default();
    let mut video_track = None;
    let mut audio_track = None;
    let mut max_timestamp_ms = 0_u32;
    let scan_end = data_offset.saturating_add(4).saturating_add(MAX_SCAN_BYTES);

    for _ in 0..MAX_TAGS {
        let tag_start = file.stream_position()?;
        if tag_start >= file_len || tag_start >= scan_end {
            break;
        }
        let mut tag_header = [0_u8; 11];
        match file.read_exact(&mut tag_header) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error.into()),
        }
        let tag_type = tag_header[0] & 0x1f;
        let payload_len = u24_be(&tag_header[1..4]) as usize;
        let timestamp = u24_be(&tag_header[4..7]) | (u32::from(tag_header[7]) << 24);
        if payload_len > MAX_TAG_PAYLOAD
            || file
                .stream_position()?
                .checked_add(payload_len as u64 + 4)
                .is_none_or(|end| end > file_len || end > scan_end)
        {
            break;
        }
        let mut payload = vec![0_u8; payload_len];
        file.read_exact(&mut payload)?;
        let previous_tag_size = read_u32_be(&mut file)?;
        if previous_tag_size != 11_u32.saturating_add(payload_len as u32) {
            if resynchronize_tags(&mut file, tag_start.saturating_add(1), file_len, scan_end)? {
                continue;
            }
            break;
        }
        max_timestamp_ms = max_timestamp_ms.max(timestamp);

        match tag_type {
            8 => parse_audio_tag(&payload, &mut audio_track)?,
            9 => parse_video_tag(&payload, &mut video_track)?,
            18 => {
                // Script metadata is advisory. A malformed script tag must not
                // discard valid media tags already found.
                let _ = parse_script_metadata(&payload, &mut metadata);
            }
            _ => {}
        }

        let have_video_details = video_track
            .as_ref()
            .is_some_and(|track: &RawTrack| track.width.is_some() || track.codec_private.is_some());
        let have_audio_details = audio_track.as_ref().is_some_and(|track: &RawTrack| {
            track.codec_name.as_deref() != Some("aac") || track.codec_private.is_some()
        });
        if have_video_details && have_audio_details && metadata.duration_seconds.is_some() {
            break;
        }
    }

    let mut tracks = Vec::new();
    if let Some(mut video) = video_track {
        video.width = video.width.or(metadata.width);
        video.height = video.height.or(metadata.height);
        video.frame_rate_fps = video.frame_rate_fps.or(metadata.frame_rate_fps);
        video.bit_rate_bps = video.bit_rate_bps.or(metadata.video_bitrate_bps);
        tracks.push(video);
    }
    if let Some(mut audio) = audio_track {
        audio.bit_rate_bps = audio.bit_rate_bps.or(metadata.audio_bitrate_bps);
        tracks.push(audio);
    }
    if tracks.is_empty() {
        return Err(parse_error("FLV contains no supported media tags"));
    }

    let duration_seconds = read_tail_timestamp(&mut file, file_len)
        .map(|timestamp| f64::from(timestamp) / 1_000.0)
        .or(metadata.duration_seconds)
        .or_else(|| (max_timestamp_ms != 0).then_some(f64::from(max_timestamp_ms) / 1_000.0));

    Ok(RawContainer {
        details: Default::default(),
        format_name: "flv".into(),
        duration_seconds,
        num_chapters: Some(0),
        tracks,
    })
}

fn parse_video_tag(data: &[u8], track: &mut Option<RawTrack>) -> Result<(), MediaInfoError> {
    let Some((&flags, payload)) = data.split_first() else {
        return Ok(());
    };
    let frame_type = flags >> 4;
    let codec_id = flags & 0x0f;
    let codec_name = match codec_id {
        2 => "flv1",
        4 => "vp6f",
        5 => "vp6a",
        7 => "h264",
        9 => "mpeg4",
        _ => return Ok(()),
    };
    let current = track.get_or_insert_with(|| raw_track(TrackKind::Video, codec_name, None));
    if current.codec_name.as_deref() != Some(codec_name) {
        return Ok(());
    }
    if frame_type != 1 {
        return Ok(());
    }
    match codec_id {
        2 if current.width.is_none() => {
            if let Some((width, height)) = parse_flv1_dimensions(payload) {
                current.width = Some(width);
                current.height = Some(height);
            }
        }
        4 if current.width.is_none() => {
            if let Some((width, height)) = parse_vp6_dimensions(payload, false) {
                current.width = Some(width);
                current.height = Some(height);
            }
        }
        5 if current.width.is_none() => {
            if let Some((width, height)) = parse_vp6_dimensions(payload, true) {
                current.width = Some(width);
                current.height = Some(height);
            }
        }
        7 | 9 if payload.first() == Some(&0) => {
            let config = payload
                .get(4..)
                .filter(|config| !config.is_empty())
                .ok_or_else(|| parse_error("truncated FLV video sequence header"))?;
            if codec_id == 7 && !codec::is_valid_h264_avcc(config) {
                return Err(parse_error("invalid FLV AVCDecoderConfigurationRecord"));
            }
            current.codec_private = Some(config.to_vec());
        }
        _ => {}
    }
    Ok(())
}

fn parse_audio_tag(data: &[u8], track: &mut Option<RawTrack>) -> Result<(), MediaInfoError> {
    let Some((&flags, payload)) = data.split_first() else {
        return Ok(());
    };
    let codec_id = flags >> 4;
    let codec_name = match codec_id {
        0 => "pcm_u8",
        1 => "adpcm_swf",
        2 => "mp3",
        3 => "pcm_s16le",
        4..=6 => "nellymoser",
        7 => "pcm_alaw",
        8 => "pcm_mulaw",
        10 => "aac",
        11 => "speex",
        _ => return Ok(()),
    };
    let channels = if codec_id == 11 {
        1
    } else {
        i32::from(flags & 1) + 1
    };
    let current =
        track.get_or_insert_with(|| raw_track(TrackKind::Audio, codec_name, Some(channels)));
    if current.codec_name.as_deref() != Some(codec_name) {
        return Ok(());
    }
    match codec_id {
        2 if current.bit_rate_bps.is_none() => {
            current.bit_rate_bps = find_mp3_bitrate(payload);
        }
        10 if payload.is_empty() => return Err(parse_error("truncated FLV AAC packet header")),
        10 if payload[0] == 0 => {
            let config = parse_aac_config(&payload[1..])
                .ok_or_else(|| parse_error("invalid FLV AudioSpecificConfig"))?;
            current.codec_private = Some(payload[1..].to_vec());
            current.audio_profile = codec::detect_header_audio_profile(
                "aac",
                Some("aac"),
                current.codec_private.as_deref(),
            );
            if let Some(channels) = config.channels {
                current.channels = Some(channels);
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_flv1_dimensions(data: &[u8]) -> Option<(i32, i32)> {
    let mut bits = MsbBitReader::new(data);
    if bits.read(17)? != 1 {
        return None;
    }
    let format = bits.read(5)?;
    if format > 1 {
        return None;
    }
    bits.read(8)?;
    match bits.read(3)? {
        0 => Some((bits.read(8)? as i32, bits.read(8)? as i32)),
        1 => Some((bits.read(16)? as i32, bits.read(16)? as i32)),
        2 => Some((352, 288)),
        3 => Some((176, 144)),
        4 => Some((128, 96)),
        5 => Some((320, 240)),
        6 => Some((160, 120)),
        _ => None,
    }
}

fn parse_vp6_dimensions(data: &[u8], alpha: bool) -> Option<(i32, i32)> {
    let (&crop, mut frame) = data.split_first()?;
    if alpha {
        frame = frame.get(3..)?;
    }
    if frame.len() < 6 || frame[0] & 0x80 != 0 {
        return None;
    }
    let separated_coefficients = frame[0] & 1 != 0;
    let filter_header = frame[1] & 0x06;
    let offset = if separated_coefficients || filter_header == 0 {
        2
    } else {
        0
    };
    let rows = *frame.get(offset + 2)? as i32;
    let columns = *frame.get(offset + 3)? as i32;
    if rows == 0 || columns == 0 {
        return None;
    }
    Some((
        columns * 16 - i32::from(crop >> 4),
        rows * 16 - i32::from(crop & 0x0f),
    ))
}

fn parse_script_metadata(data: &[u8], metadata: &mut FlvMetadata) -> Result<(), MediaInfoError> {
    let mut reader = AmfReader::new(data);
    let name = reader.string_value()?;
    if name != b"onMetaData" {
        return Ok(());
    }
    let marker = reader.u8()?;
    match marker {
        3 => parse_amf_object(&mut reader, metadata, 0, true),
        8 => {
            reader.u32()?;
            parse_amf_object(&mut reader, metadata, 0, true)
        }
        _ => Ok(()),
    }
}

fn parse_amf_object(
    reader: &mut AmfReader<'_>,
    metadata: &mut FlvMetadata,
    depth: usize,
    capture: bool,
) -> Result<(), MediaInfoError> {
    if depth >= MAX_AMF_DEPTH {
        return Err(parse_error("AMF nesting exceeds limit"));
    }
    for _ in 0..MAX_AMF_ENTRIES {
        let key_len = reader.u16()? as usize;
        if key_len == 0 && reader.peek() == Some(9) {
            reader.u8()?;
            return Ok(());
        }
        let key = reader.take_string(key_len)?.to_vec();
        let marker = reader.u8()?;
        if capture && marker == 0 {
            let value = reader.f64()?;
            capture_metadata_number(&key, value, metadata);
        } else {
            skip_amf_value(reader, marker, metadata, depth + 1)?;
        }
    }
    Err(parse_error("AMF object entry count exceeds limit"))
}

fn skip_amf_value(
    reader: &mut AmfReader<'_>,
    marker: u8,
    metadata: &mut FlvMetadata,
    depth: usize,
) -> Result<(), MediaInfoError> {
    match marker {
        0 => reader.skip(8),
        1 => reader.skip(1),
        2 => {
            let len = reader.u16()? as usize;
            reader.skip_string(len)
        }
        3 => parse_amf_object(reader, metadata, depth, false),
        5 | 6 | 13 => Ok(()),
        8 => {
            reader.u32()?;
            parse_amf_object(reader, metadata, depth, false)
        }
        10 => {
            if depth >= MAX_AMF_DEPTH {
                return Err(parse_error("AMF nesting exceeds limit"));
            }
            let count = reader.u32()? as usize;
            if count > MAX_AMF_ENTRIES {
                return Err(parse_error("AMF array entry count exceeds limit"));
            }
            for _ in 0..count {
                let marker = reader.u8()?;
                skip_amf_value(reader, marker, metadata, depth + 1)?;
            }
            Ok(())
        }
        11 => reader.skip(10),
        12 => {
            let len = reader.u32()? as usize;
            reader.skip_string(len)
        }
        _ => Err(parse_error("unsupported AMF value type")),
    }
}

fn capture_metadata_number(key: &[u8], value: f64, metadata: &mut FlvMetadata) {
    if !value.is_finite() || value < 0.0 {
        return;
    }
    if key.eq_ignore_ascii_case(b"duration") {
        metadata.duration_seconds = Some(value);
    } else if key.eq_ignore_ascii_case(b"width") {
        metadata.width = rounded_i32(value);
    } else if key.eq_ignore_ascii_case(b"height") {
        metadata.height = rounded_i32(value);
    } else if key.eq_ignore_ascii_case(b"framerate") {
        metadata.frame_rate_fps = (value > 0.0).then_some(value);
    } else if key.eq_ignore_ascii_case(b"videodatarate") {
        metadata.video_bitrate_bps = kbps_to_bps(value);
    } else if key.eq_ignore_ascii_case(b"audiodatarate") {
        metadata.audio_bitrate_bps = kbps_to_bps(value);
    }
}

fn rounded_i32(value: f64) -> Option<i32> {
    (value > 0.0 && value <= f64::from(i32::MAX)).then(|| value.round() as i32)
}

fn kbps_to_bps(value: f64) -> Option<i64> {
    let bps = value * 1_024.0;
    (bps > 0.0 && bps <= i64::MAX as f64).then(|| bps.round() as i64)
}

struct AacConfig {
    channels: Option<i32>,
}

fn parse_aac_config(data: &[u8]) -> Option<AacConfig> {
    let mut bits = MsbBitReader::new(data);
    let mut object_type = bits.read(5)?;
    if object_type == 31 {
        object_type = 32 + bits.read(6)?;
    }
    if object_type == 0 {
        return None;
    }
    let frequency_index = bits.read(4)?;
    if matches!(frequency_index, 13 | 14) {
        return None;
    }
    if frequency_index == 15 && bits.read(24)? == 0 {
        return None;
    }
    let channel_config = bits.read(4)? as u8;
    if matches!(object_type, 5 | 29) {
        let extension_frequency = bits.read(4)?;
        if matches!(extension_frequency, 13 | 14) {
            return None;
        }
        if extension_frequency == 15 && bits.read(24)? == 0 {
            return None;
        }
        object_type = bits.read(5)?;
        if object_type == 0 {
            return None;
        }
    }
    let channels = match channel_config {
        0 => None,
        1 => Some(1),
        2 => Some(2),
        3 => Some(3),
        4 => Some(4),
        5 => Some(5),
        6 => Some(6),
        7 => Some(8),
        _ => return None,
    };
    Some(AacConfig { channels })
}

fn find_mp3_bitrate(data: &[u8]) -> Option<i64> {
    const MPEG1_LAYER3: [u16; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const MPEG2_LAYER3: [u16; 16] = [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ];
    data.windows(4).find_map(|header| {
        if header[0] != 0xff || header[1] & 0xe0 != 0xe0 || header[1] & 0x06 != 0x02 {
            return None;
        }
        let version = (header[1] >> 3) & 0x03;
        if version == 1 {
            return None;
        }
        let index = (header[2] >> 4) as usize;
        let kbps = if version == 3 {
            MPEG1_LAYER3[index]
        } else {
            MPEG2_LAYER3[index]
        };
        (kbps != 0).then_some(i64::from(kbps) * 1_000)
    })
}

fn read_tail_timestamp(file: &mut dyn crate::source::MediaSource, file_len: u64) -> Option<u32> {
    if file_len < 15 {
        return None;
    }
    file.seek(SeekFrom::End(-4)).ok()?;
    let previous_size = read_u32_be(file).ok()? as u64;
    if previous_size < 11 || previous_size.checked_add(4)? > file_len {
        return None;
    }
    let tag_start = file_len.checked_sub(previous_size + 4)?;
    let timestamp = read_tag_timestamp_at(file, tag_start, previous_size)?;
    if timestamp != 0 || tag_start < 4 {
        return Some(timestamp);
    }

    file.seek(SeekFrom::Start(tag_start - 4)).ok()?;
    let preceding_size = u64::from(read_u32_be(file).ok()?);
    let preceding_start = tag_start.checked_sub(preceding_size.checked_add(4)?)?;
    read_tag_timestamp_at(file, preceding_start, preceding_size)
}

fn read_tag_timestamp_at(
    file: &mut dyn crate::source::MediaSource,
    start: u64,
    expected_size: u64,
) -> Option<u32> {
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut header = [0_u8; 11];
    file.read_exact(&mut header).ok()?;
    let payload_size = u24_be(&header[1..4]);
    if u64::from(payload_size) + 11 != expected_size {
        return None;
    }
    Some(u24_be(&header[4..7]) | (u32::from(header[7]) << 24))
}

fn resynchronize_tags(
    file: &mut dyn crate::source::MediaSource,
    start: u64,
    file_len: u64,
    scan_end: u64,
) -> Result<bool, MediaInfoError> {
    let end = file_len.min(scan_end);
    let length = end.saturating_sub(start).min(MAX_RESYNC_BYTES);
    if length < 30 {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(start))?;
    let mut data = Vec::with_capacity(length as usize);
    file.take(length).read_to_end(&mut data)?;
    for offset in 0..data.len() {
        let Some(next) = tag_layout_end(&data, offset) else {
            continue;
        };
        if tag_layout_end(&data, next).is_some() {
            file.seek(SeekFrom::Start(start + offset as u64))?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn tag_layout_end(data: &[u8], offset: usize) -> Option<usize> {
    let header = data.get(offset..offset.checked_add(11)?)?;
    if !matches!(header[0] & 0x1f, 8 | 9 | 18) {
        return None;
    }
    let payload_size = u24_be(&header[1..4]) as usize;
    let payload_end = offset.checked_add(11)?.checked_add(payload_size)?;
    let end = payload_end.checked_add(4)?;
    let trailer = data.get(payload_end..end)?;
    let previous_size = u32::from_be_bytes(trailer.try_into().ok()?);
    (previous_size == 11_u32.checked_add(payload_size as u32)?).then_some(end)
}

fn raw_track(kind: TrackKind, codec: &str, channels: Option<i32>) -> RawTrack {
    RawTrack {
        metadata: Default::default(),
        kind,
        codec_id: codec.into(),
        codec_name: Some(codec.into()),
        audio_profile: None,
        codec_private: None,
        width: None,
        height: None,
        channels,
        bit_rate_bps: None,
        language: None,
        name: None,
        forced: false,
        default_track: false,
        frame_rate_fps: None,
        color_transfer: None,
        dovi_config: None,
        has_hdr10plus: false,
    }
}

fn read_u32_be(reader: &mut (impl Read + ?Sized)) -> Result<u32, MediaInfoError> {
    let mut data = [0_u8; 4];
    reader.read_exact(&mut data)?;
    Ok(u32::from_be_bytes(data))
}

fn u24_be(data: &[u8]) -> u32 {
    (u32::from(data[0]) << 16) | (u32::from(data[1]) << 8) | u32::from(data[2])
}

struct MsbBitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> MsbBitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    fn read(&mut self, count: usize) -> Option<u64> {
        if count > 64 || self.bit.checked_add(count)? > self.data.len().checked_mul(8)? {
            return None;
        }
        let mut value = 0_u64;
        for _ in 0..count {
            value = (value << 1) | u64::from((self.data[self.bit / 8] >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Some(value)
    }
}

struct AmfReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> AmfReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], MediaInfoError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| parse_error("AMF offset overflow"))?;
        let value = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| parse_error("truncated AMF value"))?;
        self.pos = end;
        Ok(value)
    }

    fn take_string(&mut self, len: usize) -> Result<&'a [u8], MediaInfoError> {
        if len > MAX_AMF_STRING {
            return Err(parse_error("AMF string exceeds limit"));
        }
        self.take(len)
    }

    fn skip(&mut self, len: usize) -> Result<(), MediaInfoError> {
        self.take(len).map(|_| ())
    }

    fn skip_string(&mut self, len: usize) -> Result<(), MediaInfoError> {
        self.take_string(len).map(|_| ())
    }

    fn u8(&mut self) -> Result<u8, MediaInfoError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, MediaInfoError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, MediaInfoError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn f64(&mut self) -> Result<f64, MediaInfoError> {
        Ok(f64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn string_value(&mut self) -> Result<&'a [u8], MediaInfoError> {
        if self.u8()? != 2 {
            return Err(parse_error("AMF value is not a string"));
        }
        let len = self.u16()? as usize;
        self.take_string(len)
    }
}

fn parse_error(message: impl Into<String>) -> MediaInfoError {
    MediaInfoError::Parse(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_fixed_flv1_dimensions() {
        let fixed = [
            (2_u8, (352, 288)),
            (3, (176, 144)),
            (4, (128, 96)),
            (5, (320, 240)),
            (6, (160, 120)),
        ];
        for (code, expected) in fixed {
            let mut fields = vec![false; 16];
            fields.push(true);
            fields.extend([false; 5]);
            fields.extend([false; 8]);
            fields.extend((0..3).rev().map(|shift| code & (1 << shift) != 0));
            let mut bytes = vec![0_u8; fields.len().div_ceil(8)];
            for (index, value) in fields.into_iter().enumerate() {
                if value {
                    bytes[index / 8] |= 1 << (7 - index % 8);
                }
            }
            assert_eq!(parse_flv1_dimensions(&bytes), Some(expected));
        }
    }

    #[test]
    fn parses_aac_stereo_channel_configuration() {
        assert_eq!(parse_aac_config(&[0x12, 0x10]).unwrap().channels, Some(2));
    }

    #[test]
    fn rejects_malformed_avc_and_aac_sequence_headers() {
        let mut video = None;
        assert!(parse_video_tag(&[0x17, 0, 0, 0, 0, 1], &mut video).is_err());

        let mut audio = None;
        assert!(parse_audio_tag(&[0xaf, 0, 0x12], &mut audio).is_err());
    }

    #[test]
    fn recognizes_mpeg4_sequence_headers() {
        let mut video = None;
        parse_video_tag(&[0x19, 0, 0, 0, 0, 0, 0, 1, 0xb0], &mut video).unwrap();
        let track = video.unwrap();
        assert_eq!(track.codec_name.as_deref(), Some("mpeg4"));
        assert_eq!(track.codec_private.as_deref(), Some(&[0, 0, 1, 0xb0][..]));
    }

    #[test]
    fn parses_vp6_and_vp6_alpha_dimensions_with_crop() {
        let frame = [0, 0, 0, 0, 10, 20, 10, 20];
        let mut vp6 = vec![0x12];
        vp6.extend_from_slice(&frame);
        assert_eq!(parse_vp6_dimensions(&vp6, false), Some((319, 158)));

        let mut vp6a = vec![0x12, 0, 0, 8];
        vp6a.extend_from_slice(&frame);
        assert_eq!(parse_vp6_dimensions(&vp6a, true), Some((319, 158)));
    }

    #[test]
    fn captures_amf_metadata_numbers() {
        let mut metadata = FlvMetadata::default();
        capture_metadata_number(b"width", 320.0, &mut metadata);
        capture_metadata_number(b"videodatarate", 100.0, &mut metadata);
        assert_eq!(metadata.width, Some(320));
        assert_eq!(metadata.video_bitrate_bps, Some(102_400));
    }

    #[test]
    fn validates_two_consecutive_tag_layouts_for_resync() {
        fn tag(tag_type: u8, payload: &[u8]) -> Vec<u8> {
            let mut tag = vec![tag_type, 0, 0, payload.len() as u8, 0, 0, 0, 0, 0, 0, 0];
            tag.extend_from_slice(payload);
            tag.extend_from_slice(&(11_u32 + payload.len() as u32).to_be_bytes());
            tag
        }

        let first = tag(8, &[1, 2]);
        let second = tag(9, &[3]);
        let mut data = first.clone();
        data.extend_from_slice(&second);
        assert_eq!(tag_layout_end(&data, 0), Some(first.len()));
        assert_eq!(tag_layout_end(&data, first.len()), Some(data.len()));

        data[first.len() - 1] ^= 1;
        assert_eq!(tag_layout_end(&data, 0), None);
    }
}
