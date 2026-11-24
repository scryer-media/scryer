//! Ogg pages and the stream headers carried in them, including OGM.

use crate::MediaInfoError;
use crate::raw::{RawContainer, RawTrack, TrackKind};
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
    let mut details = scryer_media_model::AnalysisDetails::default();
    if earlier_end_outside_tail && duration_seconds.is_some() {
        details.duration_provenance = scryer_media_model::Provenance::Estimated;
        details.report.status = scryer_media_model::ProbeStatus::Incomplete;
        details.report.warnings.push(scryer_media_model::ProbeWarning {
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
                .and_then(|tag| crate::container::asf::map_audio_format_tag(tag, bits_per_sample))
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
                codec_name: crate::container::asf::map_audio_format_tag(tag, 0).map(str::to_owned),
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
    let name = crate::container::avi::map_video_fourcc(tag);
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
    let rate = scryer_media_model::Rational::new(numerator, denominator)
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
    let mut track = RawTrack::for_codec(TrackKind::Video, "theora", None);
    track.width = Some(width);
    track.height = Some(height);
    track.frame_rate_fps = Some(fps);
    track.metadata.declared_frame_rate =
        scryer_media_model::Rational::new(i64::from(fps_numerator), u64::from(fps_denominator));
    if aspect_numerator > 0 && aspect_denominator > 0 {
        track.metadata.sample_aspect_ratio = scryer_media_model::Rational::new(
            i64::from(aspect_numerator),
            u64::from(aspect_denominator),
        );
        track.metadata.display_aspect_ratio = scryer_media_model::Rational::new(
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
        track.metadata.color.provenance = scryer_media_model::Provenance::Bitstream;
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
    let mut track = RawTrack::for_codec(TrackKind::Audio, "vorbis", Some(i32::from(channels)));
    track.bit_rate_bps = (nominal_bitrate > 0).then_some(i64::from(nominal_bitrate));
    track.metadata.sample_rate = Some(sample_rate);
    track.codec_private = Some(data.to_vec());
    Ok((track, sample_rate))
}

fn parse_opus_identification(data: &[u8]) -> Result<RawTrack, MediaInfoError> {
    if data.len() < 19 || !data.starts_with(b"OpusHead") || data[8] & 0xf0 != 0 || data[9] == 0 {
        return Err(parse_error("invalid Opus identification header"));
    }
    let mut track = RawTrack::for_codec(TrackKind::Audio, "opus", Some(i32::from(data[9])));
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
#[path = "../../tests/support/ogm_fixture.rs"]
mod ogm_fixture;

#[cfg(test)]
mod tests;
