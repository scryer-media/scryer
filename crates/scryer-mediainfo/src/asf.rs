use crate::MediaInfoError;
use crate::types::{RawContainer, RawTrack, TrackKind};
use isolang::Language;
use std::collections::{BTreeMap, BTreeSet};
use std::io::SeekFrom;

pub(crate) const ASF_HEADER_GUID: [u8; 16] = [
    0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0x00, 0xaa, 0x00, 0x62, 0xce, 0x6c,
];
const ASF_DATA_GUID: [u8; 16] = [
    0x36, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0x00, 0xaa, 0x00, 0x62, 0xce, 0x6c,
];

const FILE_PROPERTIES_GUID: [u8; 16] = [
    0xa1, 0xdc, 0xab, 0x8c, 0x47, 0xa9, 0xcf, 0x11, 0x8e, 0xe4, 0x00, 0xc0, 0x0c, 0x20, 0x53, 0x65,
];
const STREAM_PROPERTIES_GUID: [u8; 16] = [
    0x91, 0x07, 0xdc, 0xb7, 0xb7, 0xa9, 0xcf, 0x11, 0x8e, 0xe6, 0x00, 0xc0, 0x0c, 0x20, 0x53, 0x65,
];
const HEADER_EXTENSION_GUID: [u8; 16] = [
    0xb5, 0x03, 0xbf, 0x5f, 0x2e, 0xa9, 0xcf, 0x11, 0x8e, 0xe3, 0x00, 0xc0, 0x0c, 0x20, 0x53, 0x65,
];
const EXTENDED_STREAM_PROPERTIES_GUID: [u8; 16] = [
    0xcb, 0xa5, 0xe6, 0x14, 0x72, 0xc6, 0x32, 0x43, 0x83, 0x99, 0xa9, 0x69, 0x52, 0x06, 0x5b, 0x5a,
];
const STREAM_BITRATE_PROPERTIES_GUID: [u8; 16] = [
    0xce, 0x75, 0xf8, 0x7b, 0x8d, 0x46, 0xd1, 0x11, 0x8d, 0x82, 0x00, 0x60, 0x97, 0xc9, 0xa2, 0xb2,
];
const LANGUAGE_LIST_GUID: [u8; 16] = [
    0xa9, 0x46, 0x43, 0x7c, 0xe0, 0xef, 0xfc, 0x4b, 0xb2, 0x29, 0x39, 0x3e, 0xde, 0x41, 0x5c, 0x85,
];
const MARKER_GUID: [u8; 16] = [
    0x01, 0xcd, 0x87, 0xf4, 0x51, 0xa9, 0xcf, 0x11, 0x8e, 0xe6, 0x00, 0xc0, 0x0c, 0x20, 0x53, 0x65,
];
const AUDIO_MEDIA_GUID: [u8; 16] = [
    0x40, 0x9e, 0x69, 0xf8, 0x4d, 0x5b, 0xcf, 0x11, 0xa8, 0xfd, 0x00, 0x80, 0x5f, 0x5c, 0x44, 0x2b,
];
const VIDEO_MEDIA_GUID: [u8; 16] = [
    0xc0, 0xef, 0x19, 0xbc, 0x4d, 0x5b, 0xcf, 0x11, 0xa8, 0xfd, 0x00, 0x80, 0x5f, 0x5c, 0x44, 0x2b,
];

const MAX_HEADER_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OBJECTS: usize = 4_096;
const MAX_LANGUAGES: usize = 65_536;
const MAX_FRAME_RATE_SCAN_BYTES: u64 = 1024 * 1024;
const MAX_FRAME_RATE_SCAN_PACKETS: usize = 256;

#[derive(Default)]
struct AsfState {
    duration_seconds: Option<f64>,
    chapters: Option<i32>,
    streams: BTreeMap<u16, StreamState>,
    languages: Vec<String>,
    object_count: usize,
    max_packet_size: Option<u32>,
}

#[derive(Default)]
struct StreamState {
    track: Option<RawTrack>,
    bit_rate_bps: Option<i64>,
    frame_rate_fps: Option<f64>,
    language_index: Option<usize>,
}

pub(crate) fn parse_asf_source(
    mut file: &mut dyn crate::source::MediaSource,
) -> Result<RawContainer, MediaInfoError> {
    let file_len = file.len();
    let mut prefix = [0_u8; 24];
    file.read_exact(&mut prefix)
        .map_err(|_| parse_error("truncated ASF header"))?;
    if prefix[..16] != ASF_HEADER_GUID {
        return Err(parse_error("missing ASF header GUID"));
    }

    let header_size = u64::from_le_bytes(prefix[16..24].try_into().unwrap());
    if !(30..=MAX_HEADER_BYTES).contains(&header_size) || header_size > file_len {
        return Err(parse_error("invalid ASF header size"));
    }
    let header_len =
        usize::try_from(header_size).map_err(|_| parse_error("ASF header too large"))?;
    let mut header = vec![0_u8; header_len];
    header[..24].copy_from_slice(&prefix);
    file.seek(SeekFrom::Start(24))?;
    file.read_exact(&mut header[24..])
        .map_err(|_| parse_error("truncated ASF header object data"))?;

    let mut root = SliceReader::new(&header[24..]);
    let declared_objects = root.u32_le()? as usize;
    root.skip(2)?;
    if declared_objects > MAX_OBJECTS {
        return Err(parse_error("too many ASF header objects"));
    }

    let mut state = AsfState::default();
    parse_objects(&mut root, Some(declared_objects), &mut state)?;

    let video_streams = state
        .streams
        .iter()
        .filter_map(|(stream_number, stream)| {
            stream
                .track
                .as_ref()
                .is_some_and(|track| track.kind == TrackKind::Video)
                .then_some(*stream_number)
        })
        .collect::<BTreeSet<_>>();
    let probed_frame_rates = state
        .max_packet_size
        .and_then(|packet_size| {
            probe_asf_frame_rates(
                &mut file,
                file_len,
                header_size,
                packet_size,
                &video_streams,
            )
        })
        .unwrap_or_default();

    let languages = state.languages;
    let mut tracks = Vec::new();
    for (stream_number, stream) in state.streams {
        if let Some(mut track) = stream.track {
            track.bit_rate_bps = stream.bit_rate_bps.or(track.bit_rate_bps);
            track.frame_rate_fps = stream
                .frame_rate_fps
                .or(track.frame_rate_fps)
                .or_else(|| probed_frame_rates.get(&stream_number).copied());
            track.language = stream
                .language_index
                .and_then(|index| languages.get(index))
                .cloned();
            tracks.push(track);
        }
    }

    if tracks.is_empty() {
        return Err(parse_error("ASF header contains no supported streams"));
    }

    Ok(RawContainer {
        details: Default::default(),
        format_name: "asf".into(),
        duration_seconds: state.duration_seconds,
        num_chapters: state.chapters,
        tracks,
    })
}

fn parse_objects(
    reader: &mut SliceReader<'_>,
    expected: Option<usize>,
    state: &mut AsfState,
) -> Result<(), MediaInfoError> {
    let mut parsed = 0_usize;
    while reader.remaining() >= 24 && expected.is_none_or(|count| parsed < count) {
        state.object_count += 1;
        if state.object_count > MAX_OBJECTS {
            return Err(parse_error("too many ASF objects"));
        }
        let guid = reader.array_16()?;
        let size = reader.u64_le()?;
        if size < 24 {
            return Err(parse_error("ASF object smaller than its header"));
        }
        let payload_len = usize::try_from(size - 24)
            .map_err(|_| parse_error("ASF object size does not fit memory"))?;
        let payload = reader.take(payload_len)?;
        parse_object(guid, payload, state)?;
        parsed += 1;
    }
    if expected.is_some_and(|count| parsed != count) {
        return Err(parse_error(
            "ASF header object count exceeds header boundary",
        ));
    }
    Ok(())
}

fn parse_object(
    guid: [u8; 16],
    payload: &[u8],
    state: &mut AsfState,
) -> Result<(), MediaInfoError> {
    match guid {
        FILE_PROPERTIES_GUID => parse_file_properties(payload, state),
        STREAM_PROPERTIES_GUID => parse_stream_properties(payload, state),
        HEADER_EXTENSION_GUID => parse_header_extension(payload, state),
        EXTENDED_STREAM_PROPERTIES_GUID => parse_extended_stream_properties(payload, state),
        STREAM_BITRATE_PROPERTIES_GUID => parse_stream_bitrates(payload, state),
        LANGUAGE_LIST_GUID => parse_languages(payload, state),
        MARKER_GUID => parse_markers(payload, state),
        _ => Ok(()),
    }
}

fn parse_file_properties(data: &[u8], state: &mut AsfState) -> Result<(), MediaInfoError> {
    let mut r = SliceReader::new(data);
    r.skip(16 + 8 + 8 + 8)?;
    let play_time = r.u64_le()?;
    r.skip(8)?;
    let preroll_ms = r.u64_le()?;
    let flags = r.u32_le()?;
    r.skip(4)?;
    let max_packet_size = r.u32_le()?;
    r.skip(4)?;
    if max_packet_size != 0 {
        state.max_packet_size = Some(max_packet_size);
    }
    if flags & 1 == 0 {
        let play_ms = play_time / 10_000;
        state.duration_seconds = Some(play_ms.saturating_sub(preroll_ms) as f64 / 1_000.0);
    }
    Ok(())
}

fn parse_stream_properties(data: &[u8], state: &mut AsfState) -> Result<(), MediaInfoError> {
    let mut r = SliceReader::new(data);
    let stream_type = r.array_16()?;
    r.skip(16 + 8)?;
    let type_size = r.u32_le()? as usize;
    let error_size = r.u32_le()? as usize;
    let stream_number = r.u16_le()? & 0x7f;
    r.skip(4)?;
    let type_data = r.take(type_size)?;
    r.skip(error_size)?;

    let track = if stream_type == AUDIO_MEDIA_GUID {
        parse_audio_type_data(type_data)?
    } else if stream_type == VIDEO_MEDIA_GUID {
        parse_video_type_data(type_data)?
    } else {
        return Ok(());
    };
    state.streams.entry(stream_number).or_default().track = Some(track);
    Ok(())
}

fn parse_audio_type_data(data: &[u8]) -> Result<RawTrack, MediaInfoError> {
    let mut r = SliceReader::new(data);
    let format_tag = r.u16_le()?;
    let channels = r.u16_le()?;
    r.skip(4)?;
    let avg_bytes_per_second = r.u32_le()?;
    r.skip(2)?;
    let bits_per_sample = r.u16_le()?;
    let codec_private = if r.remaining() >= 2 {
        let extra_len = r.u16_le()? as usize;
        Some(r.take(extra_len)?.to_vec()).filter(|bytes| !bytes.is_empty())
    } else {
        None
    };
    let codec_name = resolve_audio_format(format_tag, bits_per_sample, codec_private.as_deref())
        .and_then(|(tag, bits)| map_audio_format_tag(tag, bits))
        .map(str::to_owned);
    let mut track = raw_track(TrackKind::Audio, format!("0x{format_tag:04x}"), codec_name);
    track.metadata = crate::audio_metadata::wave_format(data);
    track.codec_private = if format_tag == 0xfffe && track.codec_name.is_some() {
        // WAVEFORMATEXTENSIBLE's valid-bits, mask, and GUID precede the codec
        // configuration. They must not be interpreted as an AAC ASC.
        codec_private
            .and_then(|extra| Some(extra.get(22..)?.to_vec()).filter(|bytes| !bytes.is_empty()))
    } else {
        codec_private
    };
    track.audio_profile = crate::codec::detect_header_audio_profile(
        &track.codec_id,
        track.codec_name.as_deref(),
        track.codec_private.as_deref(),
    );
    track.channels = (channels > 0).then_some(i32::from(channels));
    track.bit_rate_bps = i64::from(avg_bytes_per_second)
        .checked_mul(8)
        .filter(|value| *value > 0);
    Ok(track)
}

fn parse_video_type_data(data: &[u8]) -> Result<RawTrack, MediaInfoError> {
    let mut r = SliceReader::new(data);
    let encoded_width = r.u32_le()?;
    let encoded_height = r.u32_le()?;
    r.skip(1)?;
    let format_size = r.u16_le()? as usize;
    let format = r.take(format_size)?;
    let mut bitmap = SliceReader::new(format);
    let bitmap_size = bitmap.u32_le()? as usize;
    if bitmap_size < 40 || bitmap_size > format.len() {
        return Err(parse_error("invalid ASF BITMAPINFOHEADER size"));
    }
    let bitmap_width = bitmap.i32_le()?.unsigned_abs();
    let bitmap_height = bitmap.i32_le()?.unsigned_abs();
    bitmap.skip(2 + 2)?;
    let fourcc = bitmap.array_4()?;
    bitmap.skip(20)?;
    let extra_len = bitmap_size - 40;
    let codec_private = Some(bitmap.take(extra_len)?.to_vec()).filter(|bytes| !bytes.is_empty());
    let width = if bitmap_width == 0 {
        encoded_width
    } else {
        bitmap_width
    };
    let height = if bitmap_height == 0 {
        encoded_height
    } else {
        bitmap_height
    };
    let width = i32::try_from(width).map_err(|_| parse_error("ASF video width is too large"))?;
    let height = i32::try_from(height).map_err(|_| parse_error("ASF video height is too large"))?;
    let codec_id = String::from_utf8_lossy(&fourcc).into_owned();
    let codec_name = map_video_fourcc(fourcc).map(str::to_owned);
    let mut track = raw_track(TrackKind::Video, codec_id, codec_name);
    track.codec_private = codec_private;
    track.width = Some(width);
    track.height = Some(height);
    Ok(track)
}

fn parse_header_extension(data: &[u8], state: &mut AsfState) -> Result<(), MediaInfoError> {
    let mut r = SliceReader::new(data);
    r.skip(16 + 2)?;
    let extension_size = r.u32_le()? as usize;
    let extension = r.take(extension_size)?;
    parse_objects(&mut SliceReader::new(extension), None, state)
}

fn parse_extended_stream_properties(
    data: &[u8],
    state: &mut AsfState,
) -> Result<(), MediaInfoError> {
    let mut r = SliceReader::new(data);
    r.skip(16)?;
    let bitrate = r.u32_le()?;
    r.skip(28)?;
    let stream_number = r.u16_le()? & 0x7f;
    let language_index = r.u16_le()? as usize;
    let average_frame_time = r.u64_le()?;
    let stream_name_count = r.u16_le()? as usize;
    let payload_extension_count = r.u16_le()? as usize;

    let stream = state.streams.entry(stream_number).or_default();
    if bitrate != 0 {
        stream.bit_rate_bps = Some(i64::from(bitrate));
    }
    if average_frame_time != 0 {
        stream.frame_rate_fps = Some(10_000_000.0 / average_frame_time as f64);
    }
    stream.language_index = Some(language_index);

    for _ in 0..stream_name_count {
        r.skip(2)?;
        let length = r.u16_le()? as usize;
        r.skip(length)?;
    }
    for _ in 0..payload_extension_count {
        r.skip(16 + 2)?;
        let length = r.u32_le()? as usize;
        r.skip(length)?;
    }
    Ok(())
}

fn parse_stream_bitrates(data: &[u8], state: &mut AsfState) -> Result<(), MediaInfoError> {
    let mut r = SliceReader::new(data);
    let count = r.u16_le()? as usize;
    if count > 128 {
        return Err(parse_error("too many ASF stream bitrate records"));
    }
    for _ in 0..count {
        let stream_number = r.u16_le()? & 0x7f;
        let bitrate = r.u32_le()?;
        let stream = state.streams.entry(stream_number).or_default();
        if stream.bit_rate_bps.is_none() && bitrate != 0 {
            stream.bit_rate_bps = Some(i64::from(bitrate));
        }
    }
    Ok(())
}

fn parse_languages(data: &[u8], state: &mut AsfState) -> Result<(), MediaInfoError> {
    let mut r = SliceReader::new(data);
    let count = r.u16_le()? as usize;
    if count > MAX_LANGUAGES {
        return Err(parse_error("too many ASF language records"));
    }
    let mut languages = Vec::with_capacity(count);
    for _ in 0..count {
        let byte_len = r.u8()? as usize;
        if !byte_len.is_multiple_of(2) {
            return Err(parse_error("invalid ASF language length"));
        }
        let raw = r.take(byte_len)?;
        let utf16 = raw
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();
        let language = String::from_utf16_lossy(&utf16)
            .trim_matches('\0')
            .split(['-', '_'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let language = Language::from_639_1(&language)
            .or_else(|| Language::from_639_3(&language))
            .map(|language| language.to_639_3().to_owned())
            .unwrap_or(language);
        languages.push(language);
    }
    state.languages = languages;
    Ok(())
}

fn parse_markers(data: &[u8], state: &mut AsfState) -> Result<(), MediaInfoError> {
    let mut r = SliceReader::new(data);
    r.skip(16)?;
    let count = r.u32_le()? as usize;
    r.skip(2)?;
    let name_len = r.u16_le()? as usize;
    r.skip(name_len)?;
    let mut valid = 0_i32;
    for _ in 0..count.min(i32::MAX as usize) {
        r.skip(8 + 8)?;
        let entry_len = r.u16_le()? as usize;
        let entry = r.take(entry_len)?;
        let mut marker = SliceReader::new(entry);
        marker.skip(4 + 4)?;
        let description_len = marker.u32_le()? as usize;
        marker.skip(description_len)?;
        valid += 1;
    }
    state.chapters = Some(valid);
    Ok(())
}

fn probe_asf_frame_rates(
    file: &mut dyn crate::source::MediaSource,
    file_len: u64,
    data_offset: u64,
    packet_size: u32,
    video_streams: &BTreeSet<u16>,
) -> Option<BTreeMap<u16, f64>> {
    if video_streams.is_empty() || packet_size == 0 {
        return None;
    }

    let mut data_header = [0_u8; 50];
    file.seek(SeekFrom::Start(data_offset)).ok()?;
    file.read_exact(&mut data_header).ok()?;
    if data_header[..16] != ASF_DATA_GUID {
        return None;
    }
    let data_size = u64::from_le_bytes(data_header[16..24].try_into().unwrap());
    if data_size < data_header.len() as u64 {
        return None;
    }
    let data_end = data_offset
        .checked_add(data_size)
        .unwrap_or(file_len)
        .min(file_len);
    let mut position = data_offset.checked_add(data_header.len() as u64)?;
    let scan_end = position
        .checked_add(MAX_FRAME_RATE_SCAN_BYTES)
        .unwrap_or(file_len)
        .min(data_end);
    let mut timestamps = BTreeMap::<u16, Vec<u32>>::new();

    for _ in 0..MAX_FRAME_RATE_SCAN_PACKETS {
        let available = scan_end.saturating_sub(position);
        if available < 8 {
            break;
        }
        let read_len = available.min(u64::from(packet_size)) as usize;
        let mut packet = vec![0_u8; read_len];
        file.seek(SeekFrom::Start(position)).ok()?;
        file.read_exact(&mut packet).ok()?;
        let Some(consumed) =
            collect_asf_packet_timestamps(&packet, packet_size, video_streams, &mut timestamps)
        else {
            break;
        };
        if consumed == 0 || consumed > read_len {
            break;
        }
        position = position.checked_add(consumed as u64)?;
    }

    Some(
        timestamps
            .into_iter()
            .filter_map(|(stream, timestamps)| {
                derive_frame_rate(&timestamps).map(|rate| (stream, rate))
            })
            .collect(),
    )
}

fn collect_asf_packet_timestamps(
    packet: &[u8],
    default_packet_size: u32,
    video_streams: &BTreeSet<u16>,
    timestamps: &mut BTreeMap<u16, Vec<u32>>,
) -> Option<usize> {
    let mut packet_reader = SliceReader::new(packet);
    let first = packet_reader.u8().ok()?;
    let packet_flags = if first & 0x80 != 0 {
        let error_correction_length = usize::from(first & 0x0f);
        if first & 0x70 != 0 || error_correction_length != 2 {
            return None;
        }
        packet_reader.skip(error_correction_length).ok()?;
        packet_reader.u8().ok()?
    } else {
        first
    };
    let packet_property = packet_reader.u8().ok()?;
    let packet_length =
        read_asf_packet_value(&mut packet_reader, packet_flags >> 5, default_packet_size)?;
    let _sequence = read_asf_packet_value(&mut packet_reader, packet_flags >> 1, 0)?;
    let padding = read_asf_packet_value(&mut packet_reader, packet_flags >> 3, 0)? as usize;
    packet_reader.skip(4 + 2).ok()?;

    let packet_length = packet_length as usize;
    if packet_length == 0 || packet_length > packet.len() || padding > packet_length {
        return None;
    }
    let payload_end = packet_length - padding;
    if packet_reader.pos > payload_end {
        return None;
    }
    let mut payload_reader = SliceReader::new(&packet[packet_reader.pos..payload_end]);
    let multiple_payloads = packet_flags & 1 != 0;
    let (payload_count, payload_length_type) = if multiple_payloads {
        let payload_flags = payload_reader.u8().ok()?;
        let count = usize::from(payload_flags & 0x3f);
        if count == 0 {
            return None;
        }
        (count, payload_flags >> 6)
    } else {
        (1, 0)
    };

    for _ in 0..payload_count {
        let stream = u16::from(payload_reader.u8().ok()? & 0x7f);
        let _object_number = read_asf_packet_value(&mut payload_reader, packet_property >> 4, 0)?;
        let fragment_offset = read_asf_packet_value(&mut payload_reader, packet_property >> 2, 0)?;
        let replicated_length =
            read_asf_packet_value(&mut payload_reader, packet_property, 0)? as usize;
        if replicated_length == 1 {
            return None;
        }
        let presentation_timestamp = if replicated_length >= 8 {
            let _object_size = payload_reader.u32_le().ok()?;
            let timestamp = payload_reader.u32_le().ok()?;
            payload_reader.skip(replicated_length - 8).ok()?;
            Some(timestamp)
        } else {
            payload_reader.skip(replicated_length).ok()?;
            None
        };

        let payload_length = if multiple_payloads {
            read_asf_packet_value(&mut payload_reader, payload_length_type, 0)? as usize
        } else {
            payload_reader.remaining()
        };
        if payload_length > payload_reader.remaining() {
            return None;
        }
        if fragment_offset == 0
            && video_streams.contains(&stream)
            && let Some(timestamp) = presentation_timestamp
        {
            let stream_timestamps = timestamps.entry(stream).or_default();
            if stream_timestamps
                .last()
                .is_none_or(|last| timestamp > *last)
            {
                stream_timestamps.push(timestamp);
            }
        }
        payload_reader.skip(payload_length).ok()?;
    }

    Some(packet_length)
}

fn read_asf_packet_value(
    reader: &mut SliceReader<'_>,
    length_type: u8,
    default: u32,
) -> Option<u32> {
    match length_type & 3 {
        0 => Some(default),
        1 => reader.u8().ok().map(u32::from),
        2 => reader.u16_le().ok().map(u32::from),
        3 => reader.u32_le().ok(),
        _ => unreachable!(),
    }
}

fn derive_frame_rate(timestamps: &[u32]) -> Option<f64> {
    if timestamps.len() < 3 {
        return None;
    }
    let mut deltas = timestamps
        .windows(2)
        .filter_map(|window| window[1].checked_sub(window[0]))
        .filter(|delta| *delta != 0)
        .collect::<Vec<_>>();
    if deltas.len() + 1 != timestamps.len() {
        return None;
    }
    deltas.sort_unstable();
    if deltas.last()?.saturating_sub(*deltas.first()?) > 1 {
        return None;
    }
    let span = timestamps.last()?.checked_sub(*timestamps.first()?)?;
    if span == 0 {
        return None;
    }
    let mut frame_rate = (timestamps.len() - 1) as f64 * 1_000.0 / f64::from(span);
    if !(1.0..=240.0).contains(&frame_rate) {
        return None;
    }
    let rounded = frame_rate.round();
    if (frame_rate - rounded).abs() <= rounded * 0.01 {
        frame_rate = rounded;
    }
    Some(frame_rate)
}

fn map_video_fourcc(fourcc: [u8; 4]) -> Option<&'static str> {
    let upper = fourcc.map(|byte| byte.to_ascii_uppercase());
    match &upper {
        b"WMV1" => Some("wmv1"),
        b"WMV2" | b"GXVE" => Some("wmv2"),
        b"WMV3" => Some("wmv3"),
        b"WVC1" | b"WMVA" => Some("vc1"),
        b"H264" | b"AVC1" => Some("h264"),
        _ => None,
    }
}

fn resolve_audio_format(
    tag: u16,
    bits_per_sample: u16,
    codec_private: Option<&[u8]>,
) -> Option<(u16, u16)> {
    if tag != 0xfffe {
        return Some((tag, bits_per_sample));
    }
    let extra = codec_private?;
    if extra.len() < 22
        || extra[10..22]
            != [
                0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
            ]
    {
        return None;
    }
    let subformat = u32::from_le_bytes(extra[6..10].try_into().unwrap());
    let subformat = u16::try_from(subformat).ok()?;
    let valid_bits = u16::from_le_bytes(extra[0..2].try_into().unwrap());
    if matches!(subformat, 1 | 3) && valid_bits > bits_per_sample {
        return None;
    }
    Some((subformat, bits_per_sample))
}

fn map_audio_format_tag(tag: u16, bits_per_sample: u16) -> Option<&'static str> {
    match tag {
        0x0001 => match bits_per_sample {
            8 => Some("pcm_u8"),
            16 => Some("pcm_s16le"),
            24 => Some("pcm_s24le"),
            32 => Some("pcm_s32le"),
            64 => Some("pcm_s64le"),
            _ => None,
        },
        0x0003 => match bits_per_sample {
            32 => Some("pcm_f32le"),
            64 => Some("pcm_f64le"),
            _ => None,
        },
        0x0050 | 0x0055 => Some("mp3"),
        0x00ff => Some("aac"),
        0x0160 => Some("wmav1"),
        0x0161 => Some("wmav2"),
        0x0162 => Some("wmapro"),
        0x0163 => Some("wmalossless"),
        0x000a => Some("wmavoice"),
        0x2000 => Some("ac3"),
        0x2001 => Some("dts"),
        _ => None,
    }
}

fn raw_track(kind: TrackKind, codec_id: String, codec_name: Option<String>) -> RawTrack {
    RawTrack {
        metadata: Default::default(),
        kind,
        codec_id,
        codec_name,
        audio_profile: None,
        codec_private: None,
        width: None,
        height: None,
        channels: None,
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

struct SliceReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> SliceReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], MediaInfoError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| parse_error("ASF offset overflow"))?;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| parse_error("truncated ASF object"))?;
        self.pos = end;
        Ok(bytes)
    }

    fn skip(&mut self, len: usize) -> Result<(), MediaInfoError> {
        self.take(len).map(|_| ())
    }

    fn u8(&mut self) -> Result<u8, MediaInfoError> {
        Ok(self.take(1)?[0])
    }

    fn u16_le(&mut self) -> Result<u16, MediaInfoError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32_le(&mut self) -> Result<u32, MediaInfoError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32_le(&mut self) -> Result<i32, MediaInfoError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64_le(&mut self) -> Result<u64, MediaInfoError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn array_4(&mut self) -> Result<[u8; 4], MediaInfoError> {
        Ok(self.take(4)?.try_into().unwrap())
    }

    fn array_16(&mut self) -> Result<[u8; 16], MediaInfoError> {
        Ok(self.take(16)?.try_into().unwrap())
    }
}

fn parse_error(message: impl Into<String>) -> MediaInfoError {
    MediaInfoError::Parse(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensible_aac_uses_the_codec_configuration_after_the_wave_header() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/media/wmv_wmv1_aac_surround.wmv");
        let analysis = crate::analyze_catalog_file(&path).unwrap();
        let audio = analysis
            .details
            .streams
            .iter()
            .find(|stream| stream.kind == scryer_media_types::StreamKind::Audio)
            .unwrap();
        assert_eq!(audio.codec.as_deref(), Some("aac"));
        assert_eq!(audio.channels, Some(6));
        assert_eq!(audio.metadata.channel_layout.as_deref(), Some("5.1"));
        assert_eq!(audio.metadata.sample_rate, Some(48_000));
        assert_eq!(audio.metadata.profile.as_deref(), Some("LC"));
    }

    #[test]
    fn maps_windows_media_codecs() {
        assert_eq!(map_video_fourcc(*b"WMV3"), Some("wmv3"));
        assert_eq!(map_video_fourcc(*b"WVC1"), Some("vc1"));
        assert_eq!(map_audio_format_tag(0x0162, 0), Some("wmapro"));
        assert_eq!(map_audio_format_tag(0x0163, 0), Some("wmalossless"));
        assert_eq!(map_audio_format_tag(0x0001, 8), Some("pcm_u8"));
        assert_eq!(map_audio_format_tag(0x0001, 24), Some("pcm_s24le"));
        assert_eq!(map_audio_format_tag(0x0003, 32), Some("pcm_f32le"));
    }

    #[test]
    fn resolves_wave_format_extensible_subtype_and_valid_bits() {
        let mut extra = vec![0_u8; 22];
        extra[0..2].copy_from_slice(&32_u16.to_le_bytes());
        extra[6..10].copy_from_slice(&3_u32.to_le_bytes());
        extra[10..22].copy_from_slice(&[
            0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
        ]);

        assert_eq!(
            resolve_audio_format(0xfffe, 32, Some(&extra)),
            Some((3, 32))
        );
        assert_eq!(
            resolve_audio_format(0xfffe, 32, Some(&extra))
                .and_then(|(tag, bits)| map_audio_format_tag(tag, bits)),
            Some("pcm_f32le")
        );
        extra[0..2].copy_from_slice(&20_u16.to_le_bytes());
        extra[6..10].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            resolve_audio_format(0xfffe, 24, Some(&extra)),
            Some((1, 24))
        );
        let mut wave = vec![0_u8; 18];
        wave[0..2].copy_from_slice(&0xfffe_u16.to_le_bytes());
        wave[2..4].copy_from_slice(&2_u16.to_le_bytes());
        wave[4..8].copy_from_slice(&48_000_u32.to_le_bytes());
        wave[14..16].copy_from_slice(&24_u16.to_le_bytes());
        wave[16..18].copy_from_slice(&22_u16.to_le_bytes());
        wave.extend(extra);
        let track = parse_audio_type_data(&wave).unwrap();
        assert_eq!(track.codec_name.as_deref(), Some("pcm_s24le"));
        assert_eq!(track.metadata.sample_bit_depth, Some(20));
        assert_eq!(track.metadata.sample_rate, Some(48_000));
    }

    #[test]
    fn derives_frame_rate_from_replicated_video_timestamps() {
        fn packet(object_number: u8, timestamp: u32) -> Vec<u8> {
            let mut packet = vec![0x82, 0, 0, 0, 0x15];
            packet.extend_from_slice(&0_u32.to_le_bytes());
            packet.extend_from_slice(&0_u16.to_le_bytes());
            packet.extend_from_slice(&[0x81, object_number, 0, 8]);
            packet.extend_from_slice(&1_u32.to_le_bytes());
            packet.extend_from_slice(&timestamp.to_le_bytes());
            packet.push(0);
            assert_eq!(packet.len(), 24);
            packet
        }

        let video_streams = BTreeSet::from([1]);
        let mut timestamps = BTreeMap::new();
        for (object_number, timestamp) in [(1, 0), (2, 40), (3, 80)] {
            let packet = packet(object_number, timestamp);
            assert_eq!(
                collect_asf_packet_timestamps(&packet, 24, &video_streams, &mut timestamps),
                Some(24)
            );
        }
        assert_eq!(derive_frame_rate(&timestamps[&1]), Some(25.0));

        let truncated = packet(4, 120);
        assert!(
            collect_asf_packet_timestamps(
                &truncated[..truncated.len() - 1],
                24,
                &video_streams,
                &mut timestamps,
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_object_smaller_than_header() {
        let mut object = Vec::from(FILE_PROPERTIES_GUID);
        object.extend_from_slice(&23_u64.to_le_bytes());
        let error = parse_objects(
            &mut SliceReader::new(&object),
            None,
            &mut AsfState::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("smaller"));
    }

    #[test]
    fn subtracts_preroll_from_file_duration() {
        let mut properties = vec![0_u8; 80];
        properties[40..48].copy_from_slice(&30_000_000_u64.to_le_bytes());
        properties[56..64].copy_from_slice(&500_u64.to_le_bytes());
        let mut state = AsfState::default();
        parse_file_properties(&properties, &mut state).unwrap();
        assert_eq!(state.duration_seconds, Some(2.5));
    }

    #[test]
    fn extended_stream_properties_override_bitrate_records() {
        let mut bitrate = Vec::new();
        bitrate.extend_from_slice(&1_u16.to_le_bytes());
        bitrate.extend_from_slice(&7_u16.to_le_bytes());
        bitrate.extend_from_slice(&64_000_u32.to_le_bytes());
        let mut state = AsfState::default();
        parse_stream_bitrates(&bitrate, &mut state).unwrap();

        let mut extended = vec![0_u8; 64];
        extended[16..20].copy_from_slice(&128_000_u32.to_le_bytes());
        extended[48..50].copy_from_slice(&7_u16.to_le_bytes());
        extended[50..52].copy_from_slice(&1_u16.to_le_bytes());
        extended[52..60].copy_from_slice(&400_000_u64.to_le_bytes());
        parse_extended_stream_properties(&extended, &mut state).unwrap();

        let stream = state.streams.get(&7).unwrap();
        assert_eq!(stream.bit_rate_bps, Some(128_000));
        assert_eq!(stream.frame_rate_fps, Some(25.0));
        assert_eq!(stream.language_index, Some(1));
    }

    #[test]
    fn normalizes_rfc1766_languages_to_iso_639_3() {
        let mut languages = Vec::new();
        languages.extend_from_slice(&1_u16.to_le_bytes());
        languages.push(6);
        for value in ['j' as u16, 'a' as u16, 0] {
            languages.extend_from_slice(&value.to_le_bytes());
        }
        let mut state = AsfState::default();
        parse_languages(&languages, &mut state).unwrap();
        assert_eq!(state.languages, ["jpn"]);
    }
}
