use crate::MediaInfoError;
use crate::types::{RawContainer, RawTrack, TrackKind};
use ogg::reading::PacketReader;
use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom};

const MAX_HEADER_BYTES: usize = 16 * 1024 * 1024;
// PacketReader assembles continued packets internally before returning them.
// Bound the raw input too so a never-terminated packet cannot grow without limit.
const MAX_HEADER_READ_BYTES: u64 = MAX_HEADER_BYTES as u64;
const MAX_HEADER_PACKETS: usize = 8_192;
const MAX_COMMENT_BYTES: usize = 1024 * 1024;
const MAX_COMMENTS: usize = 65_536;
const MAX_OGG_PAGE_SIZE: u64 = 65_307;

#[derive(Clone, Copy)]
enum OggCodec {
    Theora {
        version: u32,
        granule_shift: u8,
        fps: f64,
    },
    Vorbis {
        sample_rate: u32,
    },
    Opus,
}

struct OggStream {
    codec: OggCodec,
    track: RawTrack,
    headers_seen: u8,
    headers_needed: u8,
}

pub(crate) fn parse_ogg_source(
    file: &mut dyn crate::source::MediaSource,
) -> Result<RawContainer, MediaInfoError> {
    let file_len = file.len();
    let header_limit = file_len.min(MAX_HEADER_READ_BYTES);
    let mut reader = PacketReader::new(HeaderReader::new(&mut *file, header_limit));
    let mut streams = BTreeMap::<u32, OggStream>::new();
    let mut stream_order = Vec::<u32>::new();
    let mut packet_count = 0_usize;
    let mut header_bytes = 0_usize;

    while packet_count < MAX_HEADER_PACKETS && header_bytes <= MAX_HEADER_BYTES {
        let Some(packet) = reader.read_packet().map_err(ogg_error)? else {
            break;
        };
        packet_count += 1;
        header_bytes = header_bytes
            .checked_add(packet.data.len())
            .ok_or_else(|| parse_error("Ogg header byte count overflow"))?;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(parse_error("Ogg codec headers exceed probe limit"));
        }

        let serial = packet.stream_serial();
        if packet.first_in_stream() {
            if let Some(stream) = identify_stream(&packet.data)?
                && streams.insert(serial, stream).is_none()
            {
                stream_order.push(serial);
            }
            continue;
        }

        if let Some(stream) = streams.get_mut(&serial) {
            consume_header(stream, &packet.data)?;
        }

        if !streams.is_empty()
            && streams
                .values()
                .all(|stream| stream.headers_seen >= stream.headers_needed)
        {
            break;
        }
    }

    if streams.is_empty() {
        return Err(parse_error("Ogg contains no supported logical streams"));
    }
    if streams
        .values()
        .any(|stream| stream.headers_seen < stream.headers_needed)
    {
        return Err(parse_error("truncated Ogg codec headers"));
    }

    drop(reader);
    let end_granules = read_end_granules(file, file_len).unwrap_or_default();
    let mut duration_seconds = None::<f64>;
    for (serial, stream) in &streams {
        let Some(granule) = end_granules.get(serial).copied() else {
            continue;
        };
        let duration = match stream.codec {
            OggCodec::Vorbis { sample_rate } => granule as f64 / f64::from(sample_rate),
            OggCodec::Opus => granule as f64 / 48_000.0,
            OggCodec::Theora {
                version,
                granule_shift,
                fps,
            } => {
                let mask = (1_u64 << granule_shift).saturating_sub(1);
                let mut iframe = granule >> granule_shift;
                if version < 0x030201 {
                    iframe = iframe.saturating_add(1);
                }
                let frame = iframe.saturating_add(granule & mask);
                frame as f64 / fps
            }
        };
        duration_seconds = Some(duration_seconds.map_or(duration, |current| current.max(duration)));
    }

    let mut video_tracks = Vec::new();
    let mut audio_tracks = Vec::new();
    for serial in stream_order {
        let Some(stream) = streams.remove(&serial) else {
            continue;
        };
        match stream.track.kind {
            TrackKind::Video => video_tracks.push(stream.track),
            TrackKind::Audio => audio_tracks.push(stream.track),
            TrackKind::Subtitle => {}
        }
    }
    video_tracks.extend(audio_tracks);

    Ok(RawContainer {
        details: Default::default(),
        format_name: "ogg".into(),
        duration_seconds,
        num_chapters: Some(0),
        tracks: video_tracks,
    })
}

struct HeaderReader<R> {
    inner: R,
    position: u64,
    limit: u64,
}

impl<R> HeaderReader<R> {
    fn new(inner: R, limit: u64) -> Self {
        Self {
            inner,
            position: 0,
            limit,
        }
    }
}

impl<R: Read> Read for HeaderReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.position);
        let allowed = buffer
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        if allowed == 0 {
            return Ok(0);
        }
        let read = self.inner.read(&mut buffer[..allowed])?;
        self.position = self.position.saturating_add(read as u64);
        Ok(read)
    }
}

impl<R: Seek> Seek for HeaderReader<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(target) => Some(target),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self.limit.checked_add_signed(offset),
        }
        .filter(|target| *target <= self.limit)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "seek outside Ogg header limit")
        })?;
        self.position = self.inner.seek(SeekFrom::Start(target))?;
        Ok(self.position)
    }
}

fn identify_stream(data: &[u8]) -> Result<Option<OggStream>, MediaInfoError> {
    if data.starts_with(b"\x80theora") {
        let (track, version, granule_shift, fps) = parse_theora_identification(data)?;
        return Ok(Some(OggStream {
            codec: OggCodec::Theora {
                version,
                granule_shift,
                fps,
            },
            track,
            headers_seen: 1,
            headers_needed: 3,
        }));
    }
    if data.starts_with(b"\x01vorbis") {
        let (track, sample_rate) = parse_vorbis_identification(data)?;
        return Ok(Some(OggStream {
            codec: OggCodec::Vorbis { sample_rate },
            track,
            headers_seen: 1,
            headers_needed: 3,
        }));
    }
    if data.starts_with(b"OpusHead") {
        let track = parse_opus_identification(data)?;
        return Ok(Some(OggStream {
            codec: OggCodec::Opus,
            track,
            headers_seen: 1,
            headers_needed: 2,
        }));
    }
    Ok(None)
}

fn consume_header(stream: &mut OggStream, data: &[u8]) -> Result<(), MediaInfoError> {
    if stream.headers_seen >= stream.headers_needed {
        return Ok(());
    }
    match stream.codec {
        OggCodec::Theora { .. } => match stream.headers_seen {
            1 if data.starts_with(b"\x81theora") => {
                stream.track.language = parse_comments(&data[7..])?;
            }
            2 if data.starts_with(b"\x82theora") => {}
            _ => return Err(parse_error("invalid Theora header sequence")),
        },
        OggCodec::Vorbis { .. } => match stream.headers_seen {
            1 if data.starts_with(b"\x03vorbis") => {
                stream.track.language = parse_comments(&data[7..])?;
            }
            2 if data.starts_with(b"\x05vorbis") => {}
            _ => return Err(parse_error("invalid Vorbis header sequence")),
        },
        OggCodec::Opus => {
            if data.starts_with(b"OpusTags") {
                stream.track.language = parse_comments(&data[8..])?;
            }
            // A missing OpusTags packet is tolerated: the first audio packet
            // completes header discovery without being interpreted as comments.
        }
    }
    stream.headers_seen += 1;
    Ok(())
}

fn parse_theora_identification(data: &[u8]) -> Result<(RawTrack, u32, u8, f64), MediaInfoError> {
    let mut bits = MsbBitReader::new(data);
    bits.skip(7 * 8)?;
    let version = bits.read(24)? as u32;
    if version < 0x030100 {
        return Err(parse_error("unsupported Theora version"));
    }
    let coded_width = (bits.read(16)? as u32) << 4;
    let coded_height = (bits.read(16)? as u32) << 4;
    if version >= 0x030400 {
        bits.skip(100)?;
    }
    let (width, height) = if version >= 0x030200 {
        let picture_width = bits.read(24)? as u32;
        let picture_height = bits.read(24)? as u32;
        bits.skip(16)?;
        let valid_picture = picture_width <= coded_width
            && picture_width > coded_width.saturating_sub(16)
            && picture_height <= coded_height
            && picture_height > coded_height.saturating_sub(16);
        if valid_picture {
            (picture_width, picture_height)
        } else {
            (coded_width, coded_height)
        }
    } else {
        (coded_width, coded_height)
    };
    let fps_numerator = bits.read(32)? as u32;
    let fps_denominator = bits.read(32)? as u32;
    if fps_numerator == 0 || fps_denominator == 0 {
        return Err(parse_error("invalid Theora frame rate"));
    }
    let fps = f64::from(fps_numerator) / f64::from(fps_denominator);
    let aspect_numerator = bits.read(24)? as u32;
    let aspect_denominator = bits.read(24)? as u32;
    let mut color_space = None;
    let mut nominal_bitrate = None;
    if version >= 0x030200 {
        color_space = Some(bits.read(8)? as u32);
        let nominal = bits.read(24)?;
        nominal_bitrate = (nominal > 0 && nominal < 0xff_ffff).then_some(nominal);
        bits.skip(6)?;
    }
    if version >= 0x030400 {
        bits.skip(2)?;
    }
    let granule_shift = bits.read(5)? as u8;
    if granule_shift >= 63 {
        return Err(parse_error("invalid Theora granule shift"));
    }
    let pixel_format = if (0x030200..0x030400).contains(&version) {
        let format = match bits.read(2)? {
            0 => "yuv420p",
            2 => "yuv422p",
            3 => "yuv444p",
            _ => return Err(parse_error("reserved Theora pixel format")),
        };
        if bits.read(3)? != 0 {
            return Err(parse_error("nonzero Theora reserved bits"));
        }
        Some(format)
    } else {
        None
    };
    let width = i32::try_from(width).map_err(|_| parse_error("Theora width is too large"))?;
    let height = i32::try_from(height).map_err(|_| parse_error("Theora height is too large"))?;
    let mut track = raw_track(TrackKind::Video, "theora", None);
    track.width = Some(width);
    track.height = Some(height);
    track.frame_rate_fps = Some(fps);
    track.metadata.declared_frame_rate =
        scryer_media_types::Rational::new(i64::from(fps_numerator), u64::from(fps_denominator));
    if aspect_numerator > 0 && aspect_denominator > 0 {
        track.metadata.sample_aspect_ratio = scryer_media_types::Rational::new(
            i64::from(aspect_numerator),
            u64::from(aspect_denominator),
        );
        track.metadata.display_aspect_ratio = scryer_media_types::Rational::new(
            i64::from(width) * i64::from(aspect_numerator),
            height as u64 * u64::from(aspect_denominator),
        );
    }
    if let Some(format) = pixel_format {
        track.metadata.pixel_format = Some(format.into());
        track.metadata.bit_depth = Some(8);
        track.metadata.field_order = Some("progressive".into());
    }
    if let Some(space @ (1 | 2)) = color_space {
        track.metadata.color.primaries = Some(if space == 1 { 4 } else { 5 });
        track.metadata.color.transfer = Some(if space == 1 { 4 } else { 5 });
        track.metadata.color.matrix = Some(if space == 1 { 6 } else { 5 });
        track.metadata.color.provenance = scryer_media_types::Provenance::Bitstream;
    }
    track.metadata.estimated_bitrate_bps = nominal_bitrate;
    track.codec_private = Some(data.to_vec());
    Ok((track, version, granule_shift, fps))
}

fn parse_vorbis_identification(data: &[u8]) -> Result<(RawTrack, u32), MediaInfoError> {
    if data.len() != 30 || !data.starts_with(b"\x01vorbis") {
        return Err(parse_error("invalid Vorbis identification header size"));
    }
    if le_u32(&data[7..11]) != 0 {
        return Err(parse_error("unsupported Vorbis version"));
    }
    let channels = data[11];
    let sample_rate = le_u32(&data[12..16]);
    let nominal_bitrate = i32::from_le_bytes(data[20..24].try_into().unwrap());
    let small_block = data[28] & 0x0f;
    let large_block = data[28] >> 4;
    if channels == 0
        || sample_rate == 0
        || !(6..=13).contains(&small_block)
        || small_block > large_block
        || large_block > 13
        || data[29] != 1
    {
        return Err(parse_error("invalid Vorbis identification header"));
    }
    let mut track = raw_track(TrackKind::Audio, "vorbis", Some(i32::from(channels)));
    track.bit_rate_bps = (nominal_bitrate > 0).then_some(i64::from(nominal_bitrate));
    track.codec_private = Some(data.to_vec());
    Ok((track, sample_rate))
}

fn parse_opus_identification(data: &[u8]) -> Result<RawTrack, MediaInfoError> {
    if data.len() < 19 || !data.starts_with(b"OpusHead") || data[8] & 0xf0 != 0 || data[9] == 0 {
        return Err(parse_error("invalid Opus identification header"));
    }
    let mut track = raw_track(TrackKind::Audio, "opus", Some(i32::from(data[9])));
    track.codec_private = Some(data.to_vec());
    Ok(track)
}

fn parse_comments(data: &[u8]) -> Result<Option<String>, MediaInfoError> {
    let mut r = LeSliceReader::new(data);
    let vendor_len = r.u32()? as usize;
    if vendor_len > MAX_COMMENT_BYTES {
        return Err(parse_error("Ogg comment vendor exceeds limit"));
    }
    r.skip(vendor_len)?;
    let count = r.u32()? as usize;
    if count > MAX_COMMENTS {
        return Err(parse_error("too many Ogg comments"));
    }
    let mut language = None;
    for _ in 0..count {
        let length = r.u32()? as usize;
        if length > MAX_COMMENT_BYTES {
            return Err(parse_error("Ogg comment exceeds limit"));
        }
        let value = r.take(length)?;
        if let Some(separator) = value.iter().position(|byte| *byte == b'=') {
            let (key, value) = (&value[..separator], &value[separator + 1..]);
            if key.eq_ignore_ascii_case(b"language") {
                language = std::str::from_utf8(value)
                    .ok()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_ascii_lowercase);
            }
        }
    }
    Ok(language)
}

fn read_end_granules(
    file: &mut dyn crate::source::MediaSource,
    file_len: u64,
) -> Result<BTreeMap<u32, u64>, MediaInfoError> {
    let mut reader = PacketReader::new(file);
    let start = file_len.saturating_sub(MAX_OGG_PAGE_SIZE);
    reader
        .seek_bytes(SeekFrom::Start(start))
        .map_err(|error| parse_error(format!("Ogg tail seek failed: {error}")))?;
    let mut granules = BTreeMap::new();
    while let Some(packet) = reader.read_packet().map_err(ogg_error)? {
        let granule = packet.absgp_page();
        if granule != u64::MAX {
            granules
                .entry(packet.stream_serial())
                .and_modify(|current: &mut u64| *current = (*current).max(granule))
                .or_insert(granule);
        }
    }
    Ok(granules)
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

fn le_u32(data: &[u8]) -> u32 {
    u32::from_le_bytes(data.try_into().unwrap())
}

struct LeSliceReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> LeSliceReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], MediaInfoError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| parse_error("Ogg comment offset overflow"))?;
        let value = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| parse_error("truncated Ogg comments"))?;
        self.pos = end;
        Ok(value)
    }

    fn skip(&mut self, len: usize) -> Result<(), MediaInfoError> {
        self.take(len).map(|_| ())
    }

    fn u32(&mut self) -> Result<u32, MediaInfoError> {
        Ok(le_u32(self.take(4)?))
    }
}

struct MsbBitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> MsbBitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    fn skip(&mut self, count: usize) -> Result<(), MediaInfoError> {
        self.bit = self
            .bit
            .checked_add(count)
            .filter(|end| *end <= self.data.len().saturating_mul(8))
            .ok_or_else(|| parse_error("truncated Theora identification header"))?;
        Ok(())
    }

    fn read(&mut self, count: usize) -> Result<u64, MediaInfoError> {
        if count > 64 {
            return Err(parse_error("invalid Theora bit-field width"));
        }
        let mut value = 0_u64;
        for _ in 0..count {
            let byte = *self
                .data
                .get(self.bit / 8)
                .ok_or_else(|| parse_error("truncated Theora identification header"))?;
            value = (value << 1) | u64::from((byte >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(value)
    }
}

fn ogg_error(error: impl std::fmt::Display) -> MediaInfoError {
    parse_error(format!("invalid Ogg stream: {error}"))
}

fn parse_error(message: impl Into<String>) -> MediaInfoError {
    MediaInfoError::Parse(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theora_header_preserves_rationals_and_rejects_reserved_fields() {
        let fixture = include_bytes!("../tests/media/ogv_theora_vorbis.ogv");
        let offset = fixture
            .windows(7)
            .position(|bytes| bytes == b"\x80theora")
            .unwrap();
        let mut header = fixture[offset..offset + 42].to_vec();
        let (track, _, _, _) = parse_theora_identification(&header).unwrap();
        assert_eq!(
            track.metadata.declared_frame_rate,
            scryer_media_types::Rational::new(15, 1)
        );
        assert_eq!(
            track.metadata.sample_aspect_ratio,
            scryer_media_types::Rational::new(1, 1)
        );
        assert_eq!(
            track.metadata.display_aspect_ratio,
            scryer_media_types::Rational::new(20, 11)
        );
        assert_eq!(track.metadata.bit_depth, Some(8));
        assert_eq!(track.metadata.pixel_format.as_deref(), Some("yuv420p"));
        header[30..36].fill(0);
        assert!(
            parse_theora_identification(&header)
                .unwrap()
                .0
                .metadata
                .sample_aspect_ratio
                .is_none()
        );
        header[41] = (header[41] & !0x18) | 0x10;
        assert_eq!(
            parse_theora_identification(&header)
                .unwrap()
                .0
                .metadata
                .pixel_format
                .as_deref(),
            Some("yuv422p")
        );
        header[41] = (header[41] & !0x18) | 0x08;
        assert!(parse_theora_identification(&header).is_err());
        header[41] &= !0x18;
        header[22..26].fill(0);
        assert!(
            parse_theora_identification(&header).is_err(),
            "invalid frame rate must not become 25 fps"
        );
    }

    #[test]
    fn parses_opus_identification() {
        let mut header = b"OpusHead\x01\x02\x38\x01\x80\xbb\0\0\0\0\0".to_vec();
        header.truncate(19);
        let track = parse_opus_identification(&header).unwrap();
        assert_eq!(track.codec_name.as_deref(), Some("opus"));
        assert_eq!(track.channels, Some(2));
    }

    #[test]
    fn parses_language_comment_case_insensitively() {
        let mut comments = Vec::new();
        comments.extend_from_slice(&3_u32.to_le_bytes());
        comments.extend_from_slice(b"lib");
        comments.extend_from_slice(&1_u32.to_le_bytes());
        comments.extend_from_slice(&12_u32.to_le_bytes());
        comments.extend_from_slice(b"Language=eng");
        assert_eq!(parse_comments(&comments).unwrap().as_deref(), Some("eng"));
    }

    #[test]
    fn bounds_continued_packets_before_completion() {
        use ogg::{PacketWriteEndInfo, PacketWriter};
        use std::io::Cursor;

        let packet = vec![0_u8; MAX_HEADER_BYTES + 1];
        let mut cursor = Cursor::new(Vec::new());
        PacketWriter::new(&mut cursor)
            .write_packet(&packet, 7, PacketWriteEndInfo::EndStream, 0)
            .unwrap();
        cursor.set_position(0);

        let mut reader = PacketReader::new(HeaderReader::new(cursor, MAX_HEADER_READ_BYTES));
        assert!(!matches!(reader.read_packet(), Ok(Some(_))));
    }
}
