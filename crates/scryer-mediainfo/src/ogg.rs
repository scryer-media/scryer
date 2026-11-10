use crate::MediaInfoError;
use crate::types::{RawContainer, RawTrack, TrackKind};
use ogg::reading::PacketReader;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Seek, SeekFrom};

const MAX_HEADER_BYTES: usize = 16 * 1024 * 1024;
// PacketReader assembles continued packets internally before returning them.
// Bound the raw input too so a never-terminated packet cannot grow without limit.
const MAX_HEADER_READ_BYTES: u64 = MAX_HEADER_BYTES as u64;
const MAX_HEADER_PACKETS: usize = 8_192;
const MAX_COMMENT_BYTES: usize = 1024 * 1024;
const MAX_COMMENTS: usize = 65_536;
const MAX_TAIL_BYTES: u64 = 1024 * 1024;
const MAX_TAIL_CAPTURE_CANDIDATES: usize = 128;

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
    Ogm {
        time_unit: u64,
        samples_per_unit: u64,
        default_duration: u32,
    },
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
        // All BOS pages precede secondary headers in a physical Ogg section.
        // Stop before buffering media packets once every codec header is known.
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
    let tail = read_stream_ends(file, file_len).unwrap_or_default();
    let is_ogm = streams
        .values()
        .any(|stream| matches!(stream.codec, OggCodec::Ogm { .. }));
    let mut duration_seconds = None::<f64>;
    let mut earlier_end_outside_tail = false;
    for (serial, stream) in &streams {
        let Some(end) = tail.ends.get(serial) else {
            // Ogg EOS pages need not be adjacent. A sparse subtitle or a
            // shorter audio stream can finish long before the terminal page.
            // Page ordering bounds streams absent from this tail, but a
            // continuous stream still present without EOS remains ambiguous.
            if stream.track.kind != TrackKind::Subtitle && tail.seen.contains(serial) {
                duration_seconds = None;
                break;
            }
            earlier_end_outside_tail = true;
            continue;
        };
        let granule = end.granule;
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
            OggCodec::Ogm {
                time_unit,
                samples_per_unit,
                default_duration,
            } => {
                let packet_duration = match end.packet_duration {
                    Ok(Some(duration)) => Some(duration),
                    Ok(None) => (default_duration > 0).then_some(u64::from(default_duration)),
                    Err(()) => None,
                };
                let Some(packet_duration) = packet_duration else {
                    duration_seconds = None;
                    break;
                };
                let Some(end_granule) = granule
                    .checked_add(packet_duration)
                    .filter(|end| *end <= i64::MAX as u64)
                else {
                    duration_seconds = None;
                    break;
                };
                end_granule as f64 * time_unit as f64 / (samples_per_unit as f64 * 10_000_000.0)
            }
        };
        duration_seconds = Some(duration_seconds.map_or(duration, |current| current.max(duration)));
    }
    if tail
        .last_packet
        .is_none_or(|(serial, eos)| !eos || !streams.contains_key(&serial))
    {
        duration_seconds = None;
    }
    let mut details = scryer_media_types::AnalysisDetails::default();
    if earlier_end_outside_tail && duration_seconds.is_some() {
        details.duration_provenance = scryer_media_types::Provenance::Estimated;
        details.report.status = scryer_media_types::ProbeStatus::Incomplete;
        details.report.warnings.push(scryer_media_types::ProbeWarning {
            code: "ogg_earlier_stream_end_outside_tail".into(),
            message: "Duration uses terminal stream timing; earlier stream ends are outside the bounded tail probe".into(),
            ..Default::default()
        });
    }

    let mut video_tracks = Vec::new();
    let mut audio_tracks = Vec::new();
    let mut subtitle_tracks = Vec::new();
    for serial in stream_order {
        let Some(stream) = streams.remove(&serial) else {
            continue;
        };
        match stream.track.kind {
            TrackKind::Video => video_tracks.push(stream.track),
            TrackKind::Audio => audio_tracks.push(stream.track),
            TrackKind::Subtitle => subtitle_tracks.push(stream.track),
        }
    }
    video_tracks.extend(audio_tracks);
    video_tracks.extend(subtitle_tracks);

    Ok(RawContainer {
        details,
        format_name: if is_ogm { "ogm" } else { "ogg" }.into(),
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
    if data.starts_with(b"\x01video")
        || data.starts_with(b"\x01audio")
        || data.starts_with(b"\x01text")
    {
        return parse_ogm_identification(data).map(Some);
    }
    if data.starts_with(b"\x01Direct Show Samples embedded in Ogg") {
        return parse_ogm_legacy_identification(data).map(Some);
    }
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
                apply_comments(&mut stream.track, &data[7..])?;
            }
            2 if data.starts_with(b"\x82theora") => {}
            _ => return Err(parse_error("invalid Theora header sequence")),
        },
        OggCodec::Vorbis { .. } => match stream.headers_seen {
            1 if data.starts_with(b"\x03vorbis") => {
                apply_comments(&mut stream.track, &data[7..])?;
            }
            2 if data.starts_with(b"\x05vorbis") => {}
            _ => return Err(parse_error("invalid Vorbis header sequence")),
        },
        OggCodec::Opus => {
            if data.starts_with(b"OpusTags") {
                apply_comments(&mut stream.track, &data[8..])?;
            }
            // A missing OpusTags packet is tolerated: the first audio packet
            // completes header discovery without being interpreted as comments.
        }
        OggCodec::Ogm { .. } => {
            if !data.starts_with(b"\x03vorbis") {
                return Err(parse_error("invalid OGM comment header"));
            }
            apply_comments(&mut stream.track, &data[7..])?;
        }
    }
    stream.headers_seen += 1;
    Ok(())
}

// OGM wraps bitmap/WAVE tags and timing declarations in Ogg packets. These
// layouts are documented by the OGM demuxer at ffmpeg.org/doxygen/9.0/oggparseogm_8c_source.html.
fn parse_ogm_identification(data: &[u8]) -> Result<OggStream, MediaInfoError> {
    let mut r = LeSliceReader::new(data);
    r.skip(1)?;
    let kind = match r.take(8)? {
        b"video\0\0\0" => TrackKind::Video,
        b"audio\0\0\0" => TrackKind::Audio,
        b"text\0\0\0\0" => TrackKind::Subtitle,
        _ => return Err(parse_error("invalid OGM stream type")),
    };
    let subtype = r.take(4)?;
    let declared_size = r.u32()? as usize;
    if declared_size < 52 || declared_size > data.len().saturating_sub(1) {
        return Err(parse_error("invalid OGM stream header size"));
    }
    let time_unit = r.u64()?;
    let samples_per_unit = r.u64()?;
    let timing_denominator = samples_per_unit
        .checked_mul(10_000_000)
        .filter(|value| *value > 0)
        .ok_or_else(|| parse_error("invalid OGM timing values"))?;
    if time_unit == 0 {
        return Err(parse_error("invalid OGM timing values"));
    }
    let default_duration = r.u32()?;
    r.skip(4)?; // Suggested buffer size.
    let bits_per_sample = r.u16()?;
    r.skip(2)?; // Alignment padding before the stream-specific union.
    let codec_id = String::from_utf8_lossy(subtype)
        .trim_end_matches('\0')
        .to_owned();
    let mut track = RawTrack {
        kind,
        codec_id,
        ..Default::default()
    };
    match kind {
        TrackKind::Video => {
            track.codec_name = ogm_video_codec(&track.codec_id);
            track.width = Some(positive_dimension(r.u32()?)?);
            track.height = Some(positive_dimension(r.u32()?)?);
            set_ogm_frame_rate(&mut track, timing_denominator, time_unit)?;
        }
        TrackKind::Audio => {
            let tag = std::str::from_utf8(subtype)
                .ok()
                .and_then(|tag| u16::from_str_radix(tag, 16).ok());
            track.codec_name = tag
                .and_then(|tag| crate::asf::map_audio_format_tag(tag, bits_per_sample))
                .map(str::to_owned);
            track.channels = Some(positive_channels(r.u16()?)?);
            r.skip(2)?; // Block alignment.
            set_ogm_audio_rate(&mut track, timing_denominator / time_unit, r.u32()?)?;
            if matches!(tag, Some(1 | 3)) && bits_per_sample > 0 {
                track.metadata.sample_bit_depth = Some(u32::from(bits_per_sample));
            }
            let mut extra_len = declared_size - 52;
            if tag == Some(0x00ff) && declared_size >= 56 {
                r.skip(4)?; // OGM's AAC-specific prefix is not AudioSpecificConfig.
                extra_len -= 4;
            }
            if extra_len > 0 {
                track.codec_private = Some(r.take(extra_len)?.to_vec());
            }
        }
        TrackKind::Subtitle => {
            track.codec_name = Some("text".into());
        }
    }
    Ok(OggStream {
        codec: OggCodec::Ogm {
            time_unit,
            samples_per_unit,
            default_duration,
        },
        track,
        headers_seen: 1,
        headers_needed: 2,
    })
}

fn parse_ogm_legacy_identification(data: &[u8]) -> Result<OggStream, MediaInfoError> {
    let mut r = LeSliceReader::new(data);
    r.skip(96)?;
    let stream_type = r.u32()?;
    let (track, time_unit, samples_per_unit, default_duration) = match stream_type {
        0x0558_9f80 => {
            let mut r = LeSliceReader::new(data);
            r.skip(68)?;
            let codec_id = String::from_utf8_lossy(r.take(4)?)
                .trim_end_matches('\0')
                .to_owned();
            r.skip(92)?;
            let time_unit = r.u64()?;
            r.skip(4)?;
            let mut track = RawTrack {
                kind: TrackKind::Video,
                codec_name: ogm_video_codec(&codec_id),
                codec_id,
                ..Default::default()
            };
            track.width = Some(positive_dimension(r.u32()?)?);
            track.height = Some(positive_dimension(r.u32()?)?);
            set_ogm_frame_rate(&mut track, 10_000_000, time_unit)?;
            (track, time_unit, 1, 1)
        }
        0x0558_9f81 => {
            let mut r = LeSliceReader::new(data);
            r.skip(124)?;
            let tag = r.u16()?;
            let mut track = RawTrack {
                kind: TrackKind::Audio,
                codec_id: format!("{tag:04x}"),
                // Legacy headers do not declare a PCM sample representation.
                codec_name: crate::asf::map_audio_format_tag(tag, 0).map(str::to_owned),
                channels: Some(positive_channels(r.u16()?)?),
                ..Default::default()
            };
            let rate = r.u32()?;
            set_ogm_audio_rate(&mut track, u64::from(rate), r.u32()?)?;
            (track, 10_000_000, u64::from(rate), 0)
        }
        _ => return Err(parse_error("unsupported legacy OGM stream type")),
    };
    Ok(OggStream {
        codec: OggCodec::Ogm {
            time_unit,
            samples_per_unit,
            default_duration,
        },
        track,
        headers_seen: 1,
        headers_needed: 1,
    })
}

fn ogm_video_codec(tag: &str) -> Option<String> {
    let name = crate::avi::map_video_fourcc(tag);
    (name != "unknown").then(|| name.to_owned())
}

fn positive_dimension(value: u32) -> Result<i32, MediaInfoError> {
    i32::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| parse_error("invalid OGM video dimensions"))
}

fn positive_channels(value: u16) -> Result<i32, MediaInfoError> {
    (value > 0)
        .then_some(i32::from(value))
        .ok_or_else(|| parse_error("invalid OGM channel count"))
}

fn set_ogm_frame_rate(
    track: &mut RawTrack,
    numerator: u64,
    denominator: u64,
) -> Result<(), MediaInfoError> {
    let numerator =
        i64::try_from(numerator).map_err(|_| parse_error("OGM frame rate is too large"))?;
    let rate = scryer_media_types::Rational::new(numerator, denominator)
        .ok_or_else(|| parse_error("invalid OGM frame rate"))?;
    track.frame_rate_fps = Some(numerator as f64 / denominator as f64);
    track.metadata.declared_frame_rate = Some(rate);
    Ok(())
}

fn set_ogm_audio_rate(
    track: &mut RawTrack,
    rate: u64,
    average_bytes: u32,
) -> Result<(), MediaInfoError> {
    track.metadata.sample_rate = Some(
        u32::try_from(rate)
            .ok()
            .filter(|rate| *rate > 0)
            .ok_or_else(|| parse_error("invalid OGM sample rate"))?,
    );
    track.bit_rate_bps = (average_bytes > 0).then_some(i64::from(average_bytes) * 8);
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
    track.metadata.sample_rate = Some(sample_rate);
    track.codec_private = Some(data.to_vec());
    Ok((track, sample_rate))
}

fn parse_opus_identification(data: &[u8]) -> Result<RawTrack, MediaInfoError> {
    if data.len() < 19 || !data.starts_with(b"OpusHead") || data[8] & 0xf0 != 0 || data[9] == 0 {
        return Err(parse_error("invalid Opus identification header"));
    }
    let mut track = raw_track(TrackKind::Audio, "opus", Some(i32::from(data[9])));
    track.metadata.sample_rate = Some(48_000);
    track.codec_private = Some(data.to_vec());
    Ok(track)
}

#[derive(Default)]
struct OggComments {
    language: Option<String>,
    name: Option<String>,
}

fn apply_comments(track: &mut RawTrack, data: &[u8]) -> Result<(), MediaInfoError> {
    let comments = parse_comments(data)?;
    track.language = comments.language;
    track.name = comments.name;
    Ok(())
}

fn parse_comments(data: &[u8]) -> Result<OggComments, MediaInfoError> {
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
    let mut comments = OggComments::default();
    for _ in 0..count {
        let length = r.u32()? as usize;
        if length > MAX_COMMENT_BYTES {
            return Err(parse_error("Ogg comment exceeds limit"));
        }
        let value = r.take(length)?;
        if let Some(separator) = value.iter().position(|byte| *byte == b'=') {
            let (key, value) = (&value[..separator], &value[separator + 1..]);
            if key.eq_ignore_ascii_case(b"language") {
                comments.language = std::str::from_utf8(value)
                    .ok()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_ascii_lowercase);
            } else if key.eq_ignore_ascii_case(b"title") || key.eq_ignore_ascii_case(b"name") {
                comments.name = std::str::from_utf8(value)
                    .ok()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
            }
        }
    }
    Ok(comments)
}

struct OggStreamEnd {
    granule: u64,
    packet_duration: Result<Option<u64>, ()>,
}

#[derive(Default)]
struct OggTail {
    ends: BTreeMap<u32, OggStreamEnd>,
    seen: BTreeSet<u32>,
    last_packet: Option<(u32, bool)>,
    last_page_end: usize,
}

fn read_stream_ends(
    file: &mut dyn crate::source::MediaSource,
    file_len: u64,
) -> Result<OggTail, MediaInfoError> {
    let start = file_len.saturating_sub(MAX_TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; (file_len - start) as usize];
    file.read_exact(&mut bytes)?;
    // A clipped prefix may be codec payload, including false capture patterns.
    // Establish one CRC-validated boundary before either packet or page parsing.
    let first_page = first_valid_tail_page(&bytes, start != 0)?;
    let mut reader = PacketReader::new(std::io::Cursor::new(&bytes[first_page..]));
    reader
        .seek_bytes(SeekFrom::Start(0))
        .map_err(|error| parse_error(format!("Ogg tail seek failed: {error}")))?;
    let mut tail = OggTail::default();
    while let Some(packet) = reader.read_packet().map_err(ogg_error)? {
        let granule = packet.absgp_page();
        tail.seen.insert(packet.stream_serial());
        tail.last_packet = Some((packet.stream_serial(), packet.last_in_stream()));
        tail.last_page_end = first_page + reader.get_mut().position() as usize;
        if packet.last_in_stream() && granule <= i64::MAX as u64 {
            tail.ends.insert(
                packet.stream_serial(),
                OggStreamEnd {
                    granule,
                    packet_duration: ogm_packet_duration(&packet.data),
                },
            );
        }
    }
    // PacketReader deliberately yields packets, so an EOS page containing no
    // packets needs its own page-level observation. Reuse the same parser and
    // checksum verification; never recognize an unvalidated capture pattern.
    let mut offset = first_page;
    let mut pending = BTreeSet::new();
    while offset < bytes.len() {
        // After recapture, pages must be contiguous and valid. A later bad
        // checksum or truncated page invalidates duration instead of resyncing.
        let (header, table, page_end) = validate_tail_page(&bytes, offset)?;
        let segments = usize::from(header[26]);
        let serial = le_u32(&header[14..18]);
        if segments == 0 && header[5] == 4 && !pending.contains(&serial) {
            let granule = u64::from_le_bytes(header[6..14].try_into().unwrap());
            tail.seen.insert(serial);
            if granule <= i64::MAX as u64 {
                tail.ends.insert(
                    serial,
                    OggStreamEnd {
                        granule,
                        packet_duration: Ok(Some(0)),
                    },
                );
            }
            if page_end > tail.last_page_end {
                tail.last_page_end = page_end;
                tail.last_packet = Some((serial, true));
            }
        }
        if table.last() == Some(&255) {
            pending.insert(serial);
        } else if segments > 0 {
            pending.remove(&serial);
        }
        offset = page_end;
    }
    Ok(tail)
}

fn first_valid_tail_page(bytes: &[u8], clipped: bool) -> Result<usize, MediaInfoError> {
    if !clipped {
        validate_tail_page(bytes, 0)?;
        return Ok(0);
    }
    let mut offset = 0;
    for _ in 0..MAX_TAIL_CAPTURE_CANDIDATES {
        let Some(relative) = bytes[offset..]
            .windows(4)
            .position(|value| value == b"OggS")
        else {
            return Err(parse_error("no complete Ogg page in bounded tail"));
        };
        offset += relative;
        if validate_tail_page(bytes, offset).is_ok() {
            return Ok(offset);
        }
        offset += 4;
    }
    Err(parse_error("Ogg tail recapture exceeds probe limit"))
}

fn validate_tail_page(
    bytes: &[u8],
    offset: usize,
) -> Result<([u8; 27], &[u8], usize), MediaInfoError> {
    let header_end = offset
        .checked_add(27)
        .ok_or_else(|| parse_error("Ogg tail offset overflow"))?;
    let header: [u8; 27] = bytes
        .get(offset..header_end)
        .ok_or_else(|| parse_error("truncated Ogg tail page"))?
        .try_into()
        .unwrap();
    if !header.starts_with(b"OggS") || header[5] & !7 != 0 {
        return Err(parse_error("invalid Ogg tail page header"));
    }
    let (mut parser, segments) = ogg::reading::PageParser::new(header).map_err(ogg_error)?;
    let body_start = header_end
        .checked_add(segments)
        .ok_or_else(|| parse_error("Ogg tail offset overflow"))?;
    let table = bytes
        .get(header_end..body_start)
        .ok_or_else(|| parse_error("truncated Ogg tail lacing"))?;
    let body_size = parser.parse_segments(table.to_vec());
    let page_end = body_start
        .checked_add(body_size)
        .ok_or_else(|| parse_error("Ogg tail offset overflow"))?;
    let body = bytes
        .get(body_start..page_end)
        .ok_or_else(|| parse_error("truncated Ogg tail body"))?;
    parser.parse_packet_data(body.to_vec()).map_err(ogg_error)?;
    Ok((header, table, page_end))
}

fn ogm_packet_duration(data: &[u8]) -> Result<Option<u64>, ()> {
    let flags = *data.first().ok_or(())?;
    if flags & 1 != 0 {
        return Err(()); // A header is not timed media.
    }
    let bytes = usize::from(((flags & 2) << 1) | ((flags >> 6) & 3));
    if bytes == 0 {
        return Ok(None);
    }
    let duration = data
        .get(1..=bytes)
        .ok_or(())?
        .iter()
        .enumerate()
        .fold(0_u64, |value, (index, byte)| {
            value | (u64::from(*byte) << (8 * index))
        });
    Ok(Some(duration))
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

    fn u16(&mut self) -> Result<u16, MediaInfoError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, MediaInfoError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
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
#[path = "../tests/support/ogm_fixture.rs"]
mod ogm_fixture;

#[cfg(test)]
mod tests {
    use super::ogm_fixture as fixture;
    use super::*;
    use std::io::Cursor;

    fn parse(bytes: Vec<u8>) -> RawContainer {
        parse_ogg_source(&mut Cursor::new(bytes)).unwrap()
    }

    #[test]
    fn ogm_modern_multiplexed_tracks_preserve_metadata_and_end_time() {
        let raw = parse(fixture::video_audio_text());
        assert_eq!(raw.format_name, "ogm");
        assert_eq!(raw.duration_seconds, Some(3.0));
        assert_eq!(raw.tracks.len(), 3);
        let video = &raw.tracks[0];
        assert_eq!(video.kind, TrackKind::Video);
        assert_eq!(video.codec_id, "XVID");
        assert_eq!(video.codec_name.as_deref(), Some("mpeg4"));
        assert_eq!((video.width, video.height), (Some(1280), Some(720)));
        assert_eq!(video.frame_rate_fps, Some(25.0));
        assert_eq!(
            video.metadata.declared_frame_rate,
            scryer_media_types::Rational::new(25, 1)
        );
        assert_eq!(video.name.as_deref(), Some("Main video"));
        assert_eq!(raw.tracks[1].codec_name.as_deref(), Some("vorbis"));
        assert_eq!(raw.tracks[1].language.as_deref(), Some("eng"));
        assert_eq!(raw.tracks[1].name.as_deref(), Some("Stereo"));
        assert_eq!(raw.tracks[2].kind, TrackKind::Subtitle);
        assert_eq!(raw.tracks[2].codec_name.as_deref(), Some("text"));
        assert_eq!(raw.tracks[2].language.as_deref(), Some("jpn"));
        assert_eq!(raw.tracks[2].name.as_deref(), Some("Captions"));
        let analysis = crate::build_analysis(raw);
        assert_eq!(analysis.subtitle_codecs, ["text"]);
        assert_eq!(analysis.subtitle_streams[0].codec.as_deref(), Some("text"));
        assert_eq!(analysis.subtitle_languages, ["jpn"]);
    }

    #[test]
    fn ogm_audio_only_reports_wave_metadata_without_becoming_video() {
        let raw = parse(fixture::audio_only());
        assert_eq!(raw.duration_seconds, Some(3.0));
        let audio = &raw.tracks[0];
        assert_eq!(audio.kind, TrackKind::Audio);
        assert_eq!(audio.codec_id, "2000");
        assert_eq!(audio.codec_name.as_deref(), Some("ac3"));
        assert_eq!(audio.channels, Some(6));
        assert_eq!(audio.metadata.sample_rate, Some(48_000));
        assert_eq!(audio.metadata.sample_bit_depth, None);
        assert_eq!(audio.bit_rate_bps, Some(384_000));
        assert_eq!(audio.language.as_deref(), Some("deu"));
        assert!(!crate::is_valid_video(&crate::build_analysis(raw)));
    }

    #[test]
    fn ogm_modern_rational_video_and_unknown_tags_remain_distinct() {
        let mut header = fixture::modern_header(b"video\0\0\0", b"ZZZZ", 10_010_000, 24);
        let raw = parse(fixture::single_stream(
            &header,
            Some(&fixture::comments(&[])),
            &[0x48, 1, 0],
            23,
            true,
        ));
        assert_eq!(raw.tracks[0].codec_id, "ZZZZ");
        assert_eq!(raw.tracks[0].codec_name, None);
        assert_eq!(
            raw.tracks[0].metadata.declared_frame_rate,
            scryer_media_types::Rational::new(24_000, 1001)
        );
        assert_eq!(raw.duration_seconds, Some(1.001));
        assert_eq!(crate::build_analysis(raw).video_codec, None);
        header[9..13].copy_from_slice(b"H264");
        assert_eq!(
            parse_ogm_identification(&header)
                .unwrap()
                .track
                .codec_name
                .as_deref(),
            Some("h264")
        );
        let audio = fixture::modern_header(b"audio\0\0\0", b"dead", 10_000_000, 48_000);
        let stream = parse_ogm_identification(&audio).unwrap();
        assert_eq!(stream.track.kind, TrackKind::Audio);
        assert_eq!(stream.track.codec_id, "dead");
        assert_eq!(stream.track.codec_name, None);
    }

    #[test]
    fn ogm_legacy_video_and_audio_use_their_own_layouts() {
        let raw = parse(fixture::single_stream(
            &fixture::legacy_video(),
            None,
            &[0x48, 1, 0],
            74,
            true,
        ));
        assert_eq!(raw.format_name, "ogm");
        assert_eq!(raw.duration_seconds, Some(3.0));
        assert_eq!(raw.tracks[0].codec_name.as_deref(), Some("mpeg4"));
        assert_eq!(
            (raw.tracks[0].width, raw.tracks[0].height),
            (Some(1280), Some(720))
        );
        assert_eq!(
            raw.tracks[0].metadata.declared_frame_rate,
            scryer_media_types::Rational::new(25, 1)
        );
        let raw = parse(fixture::single_stream(
            &fixture::legacy_audio(),
            None,
            &[0x80, 0, 6, 0],
            142_464,
            true,
        ));
        assert_eq!(raw.duration_seconds, Some(3.0));
        assert_eq!(raw.tracks[0].codec_name.as_deref(), Some("ac3"));
        assert_eq!(raw.tracks[0].metadata.sample_rate, Some(48_000));
        assert_eq!(raw.tracks[0].channels, Some(6));
        assert_eq!(raw.tracks[0].bit_rate_bps, Some(384_000));
    }

    #[test]
    fn ogm_truncated_headers_invalid_counts_and_arithmetic_are_rejected() {
        let header = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
        for length in 6..header.len() {
            assert!(
                parse_ogm_identification(&header[..length]).is_err(),
                "length={length}"
            );
        }
        for (offset, bytes) in [
            (13, 51_u32.to_le_bytes().to_vec()),
            (13, u32::MAX.to_le_bytes().to_vec()),
            (17, 0_u64.to_le_bytes().to_vec()),
            (25, 0_u64.to_le_bytes().to_vec()),
            (25, u64::MAX.to_le_bytes().to_vec()),
            (45, u32::MAX.to_le_bytes().to_vec()),
            (49, 0_u32.to_le_bytes().to_vec()),
        ] {
            let mut bad = header.clone();
            bad[offset..offset + bytes.len()].copy_from_slice(&bytes);
            assert!(parse_ogm_identification(&bad).is_err(), "offset={offset}");
        }
        let legacy = fixture::legacy_video();
        for length in 35..legacy.len() {
            assert!(parse_ogm_legacy_identification(&legacy[..length]).is_err());
        }
        let mut comments = fixture::comments(&[])[7..].to_vec();
        comments[11..15].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_comments(&comments).is_err());
        assert!(parse_comments(&u32::MAX.to_le_bytes()).is_err());
        assert!(parse_ogg_source(&mut Cursor::new(b"OggS\0".to_vec())).is_err());
    }

    #[test]
    fn ogm_audio_private_config_excludes_aac_prefix_and_checks_declared_size() {
        let mut header = fixture::modern_header(b"audio\0\0\0", b"00ff", 10_000_000, 48_000);
        header[13..17].copy_from_slice(&58_u32.to_le_bytes());
        header.extend_from_slice(&[0, 0, 0, 0, 0x11, 0x90]);
        assert_eq!(
            parse_ogm_identification(&header)
                .unwrap()
                .track
                .codec_private,
            Some(vec![0x11, 0x90])
        );
        header.pop();
        assert!(parse_ogm_identification(&header).is_err());
        let mut header = fixture::modern_header(b"audio\0\0\0", b"0001", 10_000_000, 48_000);
        header[41..43].copy_from_slice(&24_u16.to_le_bytes());
        assert_eq!(
            parse_ogm_identification(&header)
                .unwrap()
                .track
                .codec_name
                .as_deref(),
            Some("pcm_s24le")
        );
        header[45..47].fill(0);
        assert!(parse_ogm_identification(&header).is_err());
    }

    #[test]
    fn ogm_duration_requires_eos_complete_packet_and_checked_end() {
        let header = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
        let comment = fixture::comments(&[]);
        for (packet, granule, eos) in [
            (&[0x48, 1, 0][..], 74, false),
            (&[0xc0][..], 74, true),
            (&[0x48, 1, 0][..], u64::MAX - 1, true),
            (&[0x48, 1, 0][..], i64::MAX as u64, true),
            (&[0x48, 1, 0][..], i64::MAX as u64 + 1, true),
        ] {
            assert_eq!(
                parse(fixture::single_stream(
                    &header,
                    Some(&comment),
                    packet,
                    granule,
                    eos
                ))
                .duration_seconds,
                None
            );
        }
        assert_eq!(
            parse(fixture::single_stream(
                &header,
                Some(&comment),
                &[0x08, 0],
                74,
                true
            ))
            .duration_seconds,
            Some(3.0)
        );
        assert_eq!(
            ogm_packet_duration(&[0x02, 0x78, 0x56, 0x34, 0x12]),
            Ok(Some(0x1234_5678))
        );
        assert!(
            parse(fixture::single_stream(
                &header,
                Some(&comment),
                &[0x48, 1, 0],
                i64::MAX as u64 - 1,
                true
            ))
            .duration_seconds
            .is_some()
        );
        assert_eq!(
            ogm_packet_duration(&[0xc2, 1, 2, 3, 4, 5, 6, 7]),
            Ok(Some(0x07_0605_0403_0201))
        );
    }

    #[test]
    fn ogm_clipped_tail_recaptures_past_false_magic_before_packet_and_nil_eos_parsing() {
        let header = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
        let packet_tail = fixture::single_stream(
            &header,
            Some(&fixture::comments(&[])),
            &[0x48, 1, 0],
            74,
            true,
        );
        for suffix in [packet_tail, fixture::nil_eos_page(7, 75, 3)] {
            let mut bytes = vec![0x55; MAX_TAIL_BYTES as usize];
            let mut fake = [0; 27];
            fake[..4].copy_from_slice(b"OggS");
            fake[4] = 1; // Invalid version before another false, CRC-invalid header.
            bytes.extend_from_slice(&fake);
            fake[4] = 0;
            bytes.extend_from_slice(&fake);
            bytes.extend_from_slice(&suffix);
            let length = bytes.len() as u64;
            let tail = read_stream_ends(&mut Cursor::new(bytes), length).unwrap();
            assert!(tail.ends.contains_key(&7));
            assert_eq!(tail.last_packet, Some((7, true)));
        }
    }

    #[test]
    fn ogm_tail_does_not_resync_after_valid_boundary_or_accept_embedded_nil_eos() {
        let header = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
        let embedded = fixture::nil_eos_page(99, 100, 0);
        let mut payload = vec![0x48, 1];
        payload.extend_from_slice(&embedded);
        let valid =
            fixture::single_stream(&header, Some(&fixture::comments(&[])), &payload, 74, true);
        let tail = read_stream_ends(&mut Cursor::new(valid.clone()), valid.len() as u64).unwrap();
        assert!(!tail.ends.contains_key(&99));
        let mut corrupt = valid;
        let mut bad = fixture::nil_eos_page(7, 75, 3);
        bad[22] ^= 1;
        corrupt.extend_from_slice(&bad);
        corrupt.extend_from_slice(&fixture::nil_eos_page(7, 75, 4));
        let length = corrupt.len() as u64;
        assert!(read_stream_ends(&mut Cursor::new(corrupt), length).is_err());
        let mut captures = Vec::new();
        for _ in 0..=MAX_TAIL_CAPTURE_CANDIDATES {
            captures.extend_from_slice(b"OggS");
        }
        assert!(first_valid_tail_page(&captures, true).is_err());
    }

    #[test]
    fn ogm_nil_eos_pages_use_validated_end_boundary_without_adding_a_frame() {
        let header = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
        let mut bytes = fixture::single_stream(
            &header,
            Some(&fixture::comments(&[])),
            &[0x48, 1, 0],
            74,
            false,
        );
        let nil = fixture::nil_eos_page(7, 75, 3);
        bytes.extend_from_slice(&nil);
        assert_eq!(parse(bytes.clone()).duration_seconds, Some(3.0));
        *bytes.last_mut().unwrap() ^= 1;
        assert_eq!(parse(bytes).duration_seconds, None);
    }

    #[test]
    fn ogm_tail_is_bounded_and_earlier_subtitle_end_keeps_terminal_runtime() {
        use ogg::{PacketWriteEndInfo, PacketWriter};
        let mut output = Cursor::new(Vec::new());
        let mut writer = PacketWriter::new(&mut output);
        let video = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
        let text = fixture::modern_header(b"text\0\0\0\0", &[0; 4], 10_000, 1);
        writer
            .write_packet(&video, 1, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
        writer
            .write_packet(&text, 2, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
        for serial in [1, 2] {
            writer
                .write_packet(
                    fixture::comments(&[]),
                    serial,
                    PacketWriteEndInfo::EndPage,
                    0,
                )
                .unwrap();
        }
        writer
            .write_packet(&[0x40, 1, b'a'], 2, PacketWriteEndInfo::EndStream, 1_000)
            .unwrap();
        writer
            .write_packet(
                vec![0; MAX_TAIL_BYTES as usize + 1],
                1,
                PacketWriteEndInfo::EndPage,
                50,
            )
            .unwrap();
        writer
            .write_packet(&[0x48, 1, 0], 1, PacketWriteEndInfo::EndStream, 74)
            .unwrap();
        drop(writer);
        let length = output.get_ref().len() as u64;
        let raw = parse(output.get_ref().clone());
        assert_eq!(raw.duration_seconds, Some(3.0));
        assert_eq!(
            crate::build_analysis(raw.clone())
                .details
                .duration_provenance,
            scryer_media_types::Provenance::Estimated
        );
        assert_eq!(
            raw.details.report.warnings[0].code,
            "ogg_earlier_stream_end_outside_tail"
        );
        assert!(length > MAX_TAIL_BYTES);
        struct Counted {
            cursor: Cursor<Vec<u8>>,
            bytes: u64,
        }
        impl Read for Counted {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                let size = self.cursor.read(bytes)?;
                self.bytes += size as u64;
                Ok(size)
            }
        }
        impl Seek for Counted {
            fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
                self.cursor.seek(offset)
            }
        }
        impl crate::source::MediaSource for Counted {
            fn len(&self) -> u64 {
                self.cursor.get_ref().len() as u64
            }
        }
        let mut counted = Counted {
            cursor: output,
            bytes: 0,
        };
        let ends = read_stream_ends(&mut counted, length).unwrap();
        assert!(ends.ends.contains_key(&1));
        assert!(!ends.ends.contains_key(&2));
        assert!(counted.bytes <= MAX_TAIL_BYTES);
    }

    #[test]
    fn ogm_sniffing_wins_over_extensions_and_audio_only_is_not_video() {
        for extension in ["ogm", "OGM", "", "mkv", "dat"] {
            let analysis = crate::analyze_source(
                &mut Cursor::new(fixture::video_audio_text()),
                extension,
                crate::AnalyzeOptions::default(),
            )
            .unwrap();
            assert_eq!(analysis.container_format.as_deref(), Some("ogm"));
            assert_eq!(analysis.video_codec.as_deref(), Some("mpeg4"));
            assert_eq!(analysis.subtitle_codecs, ["text"]);
            assert!(crate::is_valid_video(&analysis));
        }
        let analysis = crate::analyze_source(
            &mut Cursor::new(fixture::audio_only()),
            "ogm",
            crate::AnalyzeOptions::default(),
        )
        .unwrap();
        assert!(!crate::is_valid_video(&analysis));
    }

    #[test]
    fn ogm_multiple_audio_streams_with_interleaved_headers_are_discovered() {
        use ogg::{PacketWriteEndInfo, PacketWriter};
        let mut output = Cursor::new(Vec::new());
        let mut writer = PacketWriter::new(&mut output);
        let video = fixture::modern_header(b"video\0\0\0", b"XVID", 400_000, 1);
        let audio = fixture::modern_header(b"audio\0\0\0", b"2000", 10_000_000, 48_000);
        writer
            .write_packet(&video, 1, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
        for serial in [2, 3] {
            writer
                .write_packet(&audio, serial, PacketWriteEndInfo::EndPage, 0)
                .unwrap();
        }
        writer
            .write_packet(fixture::comments(&[]), 1, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
        for (serial, language) in [(2, "LANGUAGE=eng"), (3, "LANGUAGE=jpn")] {
            writer
                .write_packet(
                    fixture::comments(&[language]),
                    serial,
                    PacketWriteEndInfo::EndPage,
                    0,
                )
                .unwrap();
        }
        writer
            .write_packet(&[0x48, 1, 0], 1, PacketWriteEndInfo::EndStream, 74)
            .unwrap();
        for serial in [2, 3] {
            writer
                .write_packet(
                    &[0x80, 0, 6, 0],
                    serial,
                    PacketWriteEndInfo::EndStream,
                    142_464,
                )
                .unwrap();
        }
        drop(writer);
        let analysis = crate::build_analysis(parse(output.into_inner()));
        assert_eq!(analysis.audio_streams.len(), 2);
        assert!(analysis.has_multiaudio);
        assert_eq!(analysis.audio_languages, ["eng", "jpn"]);
    }

    #[test]
    fn ordinary_ogg_theora_and_opus_keep_their_granule_meaning() {
        let mut source =
            Cursor::new(include_bytes!("../tests/media/ogv_theora_vorbis.ogv").as_slice());
        let raw = parse_ogg_source(&mut source).unwrap();
        assert_eq!(raw.format_name, "ogg");
        assert_eq!(raw.tracks[0].codec_name.as_deref(), Some("theora"));
        assert_eq!(raw.tracks[1].codec_name.as_deref(), Some("vorbis"));
        assert!(
            raw.duration_seconds
                .is_some_and(|duration| (duration - 1.4).abs() < 0.001)
        );
        use ogg::{PacketWriteEndInfo, PacketWriter};
        let mut output = Cursor::new(Vec::new());
        let mut writer = PacketWriter::new(&mut output);
        writer
            .write_packet(
                b"OpusHead\x01\x02\0\0\x80\xbb\0\0\0\0\0",
                9,
                PacketWriteEndInfo::EndPage,
                0,
            )
            .unwrap();
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&[0; 8]);
        writer
            .write_packet(&tags, 9, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
        writer
            .write_packet(
                &[0xf8, 0xff, 0xfe],
                9,
                PacketWriteEndInfo::EndStream,
                144_000,
            )
            .unwrap();
        drop(writer);
        let raw = parse(output.into_inner());
        assert_eq!(raw.format_name, "ogg");
        assert_eq!(raw.duration_seconds, Some(3.0));
        assert_eq!(raw.tracks[0].codec_name.as_deref(), Some("opus"));
        assert_eq!(raw.tracks[0].channels, Some(2));
    }

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
        assert_eq!(
            parse_comments(&comments).unwrap().language.as_deref(),
            Some("eng")
        );
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
