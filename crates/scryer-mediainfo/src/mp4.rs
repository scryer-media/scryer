use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;

use mp4parse::{
    AudioCodecSpecific, CodecType, MediaContext, MediaTimeScale, SampleEntry, TrackTimeScale,
    TrackType, VideoCodecSpecific,
};

use crate::MediaInfoError;
use crate::codec::normalize_codec_name;
use crate::probe::{ProbeStats, TrackedReader};
use crate::types::{RawContainer, RawTrack, TrackKind};

const HDR10PLUS_SAMPLE_LIMIT_BYTES: u64 = 4 * 1024 * 1024;
const MP4_DOVI_TYPES: [&str; 2] = ["dvcC", "dvvC"];
const MOV_TKHD_FLAG_ENABLED: u32 = 0x000001;

#[derive(Debug)]
struct PreparedMp4 {
    metadata: Vec<u8>,
    #[allow(dead_code)]
    stats: ProbeStats,
}

#[derive(Debug, Clone)]
struct ParsedMp4Track {
    track_id: Option<u32>,
    raw: RawTrack,
}

#[derive(Debug, Clone, Default)]
struct Mp4TrackMetadata {
    track_id: Option<u32>,
    handler_type: Option<[u8; 4]>,
    language: Option<String>,
    sample_entry_fourcc: Option<String>,
    codec_private: Option<Vec<u8>>,
    dovi_config: Option<Vec<u8>>,
    name: Option<String>,
    forced: bool,
    default_track: bool,
}

#[derive(Debug, Clone, Copy)]
struct Mp4BoxHeader {
    name: [u8; 4],
    size: u64,
    header_size: usize,
}

/// Parse an MP4/MOV/M4V file into a [`RawContainer`].
pub(crate) fn parse_mp4(path: &Path) -> Result<RawContainer, MediaInfoError> {
    let file_len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let prepared = prepare_mp4_metadata(path)?;
    let metadata_by_track = parse_mp4_track_metadata(&prepared.metadata);

    let mut cursor = Cursor::new(prepared.metadata.as_slice());
    let ctx = mp4parse::read_mp4(&mut cursor)
        .map_err(|e| MediaInfoError::Parse(format!("mp4 parse: {e:?}")))?;

    // The movie-level timescale converts track-header durations to seconds.
    let movie_timescale = ctx.timescale.map(|MediaTimeScale(ts)| ts);
    let duration_seconds = movie_timescale.and_then(|ts| {
        if ts == 0 {
            return None;
        }
        ctx.tracks
            .iter()
            .filter_map(|t| t.tkhd.as_ref().map(|h| h.duration))
            .max()
            .map(|dur| dur as f64 / ts as f64)
    });

    let (mut tracks, seen_track_ids) = build_mp4_tracks(&ctx, &metadata_by_track, duration_seconds);
    append_metadata_only_tracks(&metadata_by_track, &seen_track_ids, &mut tracks);
    apply_fallback_video_bitrate(file_len, duration_seconds, &mut tracks);
    scan_mp4_hdr10plus(path, &ctx, &mut tracks);

    let format_name = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .map(|ext| match ext.as_str() {
            "mov" => "mov",
            _ => "mp4",
        })
        .unwrap_or("mp4")
        .to_owned();

    Ok(RawContainer {
        format_name,
        duration_seconds,
        num_chapters: None,
        tracks: tracks.into_iter().map(|track| track.raw).collect(),
    })
}

fn prepare_mp4_metadata(path: &Path) -> Result<PreparedMp4, MediaInfoError> {
    let file = std::fs::File::open(path).map_err(|e| MediaInfoError::Io(e.to_string()))?;
    let mut reader = TrackedReader::new(file);
    let metadata = prepare_mp4_metadata_from_reader(&mut reader)?;
    Ok(PreparedMp4 {
        metadata,
        stats: reader.stats(),
    })
}

fn prepare_mp4_metadata_from_reader<R: Read + Seek>(
    reader: &mut TrackedReader<R>,
) -> Result<Vec<u8>, MediaInfoError> {
    let file_len = reader
        .seek(SeekFrom::End(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let mut output = Vec::new();
    while reader
        .stream_position()
        .map_err(|e| MediaInfoError::Io(e.to_string()))?
        < file_len
    {
        let start = reader
            .stream_position()
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        let Some(header) = read_box_header(reader, file_len.saturating_sub(start))? else {
            break;
        };
        let keep = should_copy_top_level_box(&header.name);

        if keep {
            reader
                .seek(SeekFrom::Start(start))
                .map_err(|e| MediaInfoError::Io(e.to_string()))?;
            let mut buf = vec![0_u8; header.size as usize];
            reader
                .read_exact(&mut buf)
                .map_err(|e| MediaInfoError::Io(e.to_string()))?;
            output.extend_from_slice(&buf);
        } else if header.size == 0 {
            break;
        } else {
            reader
                .seek(SeekFrom::Start(start + header.size))
                .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        }

        if header.size == 0 {
            break;
        }
    }

    Ok(output)
}

fn should_copy_top_level_box(name: &[u8; 4]) -> bool {
    matches!(
        name,
        b"ftyp" | b"moov" | b"styp" | b"sidx" | b"moof" | b"mfra"
    )
}

fn build_mp4_tracks(
    ctx: &MediaContext,
    metadata_by_track: &HashMap<u32, Mp4TrackMetadata>,
    duration_seconds: Option<f64>,
) -> (Vec<ParsedMp4Track>, HashSet<u32>) {
    let mut parsed = Vec::new();
    let mut seen_track_ids = HashSet::new();

    for track in &ctx.tracks {
        let meta = track
            .track_id
            .and_then(|track_id| metadata_by_track.get(&track_id));
        let kind = match track_kind_from_mp4_sources(track, meta) {
            Some(kind) => kind,
            None => continue,
        };

        let mut raw = RawTrack {
            kind,
            codec_id: meta
                .and_then(|m| m.sample_entry_fourcc.clone())
                .unwrap_or_else(|| "unknown".into()),
            codec_name: None,
            codec_private: meta.and_then(|m| m.codec_private.clone()),
            width: None,
            height: None,
            channels: None,
            bit_rate_bps: None,
            language: meta.and_then(|m| m.language.clone()),
            frame_rate_fps: None,
            color_transfer: None,
            dovi_config: meta.and_then(|m| m.dovi_config.clone()),
            has_hdr10plus: false,
            name: meta.and_then(|m| m.name.clone()),
            forced: meta.is_some_and(|m| m.forced),
            default_track: meta.is_some_and(|m| m.default_track),
        };

        let first_entry = track
            .stsd
            .as_ref()
            .and_then(|stsd| stsd.descriptions.first());

        match (kind, first_entry) {
            (TrackKind::Video, Some(SampleEntry::Video(video))) => {
                raw.width = Some(i32::from(video.width));
                raw.height = Some(i32::from(video.height));
                let (codec_id, codec_private) = video_codec_info(&video.codec_specific);
                let fallback_codec_id =
                    codec_id.unwrap_or_else(|| codec_type_to_fourcc(video.codec_type));
                if raw.codec_id == "unknown" {
                    raw.codec_id = fallback_codec_id;
                }
                if raw.codec_private.is_none() {
                    raw.codec_private = codec_private;
                }
                raw.frame_rate_fps = estimate_frame_rate(track);
            }
            (TrackKind::Audio, Some(SampleEntry::Audio(audio))) => {
                raw.channels = Some(audio.channelcount as i32);
                let fallback_codec_id = audio_codec_id(&audio.codec_specific)
                    .unwrap_or_else(|| codec_type_to_fourcc(audio.codec_type));
                if raw.codec_id == "unknown" {
                    raw.codec_id = fallback_codec_id;
                }
                if raw.codec_private.is_none()
                    && let AudioCodecSpecific::ES_Descriptor(ref esds) = audio.codec_specific
                    && !esds.decoder_specific_data.is_empty()
                {
                    raw.codec_private = Some(esds.decoder_specific_data.iter().copied().collect());
                }
            }
            (TrackKind::Video, Some(SampleEntry::Unknown)) | (TrackKind::Video, None) => {
                if let Some(tkhd) = track.tkhd.as_ref() {
                    if tkhd.width > 0 {
                        raw.width = Some((tkhd.width >> 16) as i32);
                    }
                    if tkhd.height > 0 {
                        raw.height = Some((tkhd.height >> 16) as i32);
                    }
                }
                raw.frame_rate_fps = estimate_frame_rate(track);
            }
            _ => {}
        }

        if let Some(bit_rate_bps) = estimate_track_bitrate(track, duration_seconds) {
            raw.bit_rate_bps = Some(bit_rate_bps);
        }

        if raw.codec_name.is_none() {
            raw.codec_name = normalize_codec_name(&raw.codec_id);
        }

        if let Some(track_id) = track.track_id {
            seen_track_ids.insert(track_id);
        }
        parsed.push(ParsedMp4Track {
            track_id: track.track_id,
            raw,
        });
    }

    (parsed, seen_track_ids)
}

fn append_metadata_only_tracks(
    metadata_by_track: &HashMap<u32, Mp4TrackMetadata>,
    seen_track_ids: &HashSet<u32>,
    tracks: &mut Vec<ParsedMp4Track>,
) {
    for (&track_id, metadata) in metadata_by_track {
        if seen_track_ids.contains(&track_id) {
            continue;
        }
        if track_kind_from_metadata(metadata) != Some(TrackKind::Subtitle) {
            continue;
        }

        let codec_id = metadata
            .sample_entry_fourcc
            .clone()
            .unwrap_or_else(|| "unknown".into());
        tracks.push(ParsedMp4Track {
            track_id: Some(track_id),
            raw: RawTrack {
                kind: TrackKind::Subtitle,
                codec_name: normalize_codec_name(&codec_id),
                codec_id,
                codec_private: None,
                width: None,
                height: None,
                channels: None,
                bit_rate_bps: None,
                language: metadata.language.clone(),
                frame_rate_fps: None,
                color_transfer: None,
                dovi_config: None,
                has_hdr10plus: false,
                name: metadata.name.clone(),
                forced: metadata.forced,
                default_track: metadata.default_track,
            },
        });
    }
}

fn apply_fallback_video_bitrate(
    file_len: u64,
    duration_seconds: Option<f64>,
    tracks: &mut [ParsedMp4Track],
) {
    if file_len == 0 {
        return;
    }
    let Some(duration_seconds) = duration_seconds else {
        return;
    };
    if duration_seconds <= 0.0 {
        return;
    }

    let overall_bps = (file_len as f64 * 8.0 / duration_seconds) as i64;
    let has_any_video_bitrate = tracks
        .iter()
        .any(|track| track.raw.kind == TrackKind::Video && track.raw.bit_rate_bps.is_some());
    if !has_any_video_bitrate
        && let Some(video_track) = tracks
            .iter_mut()
            .find(|track| track.raw.kind == TrackKind::Video)
    {
        video_track.raw.bit_rate_bps = Some(overall_bps);
    }
}

fn track_kind_from_mp4_sources(
    track: &mp4parse::Track,
    metadata: Option<&Mp4TrackMetadata>,
) -> Option<TrackKind> {
    match track.track_type {
        TrackType::Video | TrackType::Picture | TrackType::AuxiliaryVideo => Some(TrackKind::Video),
        TrackType::Audio => Some(TrackKind::Audio),
        TrackType::Metadata | TrackType::Unknown => metadata.and_then(track_kind_from_metadata),
    }
}

fn track_kind_from_metadata(metadata: &Mp4TrackMetadata) -> Option<TrackKind> {
    if let Some(sample_entry) = metadata.sample_entry_fourcc.as_deref() {
        if is_video_sample_entry(sample_entry) {
            return Some(TrackKind::Video);
        }
        if is_audio_sample_entry(sample_entry) {
            return Some(TrackKind::Audio);
        }
        if is_subtitle_sample_entry(sample_entry) {
            return Some(TrackKind::Subtitle);
        }
    }

    match metadata.handler_type {
        Some([b'v', b'i', b'd', b'e']) => Some(TrackKind::Video),
        Some([b's', b'o', b'u', b'n']) => Some(TrackKind::Audio),
        Some([b't', b'e', b'x', b't'])
        | Some([b's', b'b', b't', b'l'])
        | Some([b's', b'u', b'b', b't'])
        | Some([b'c', b'l', b'c', b'p']) => Some(TrackKind::Subtitle),
        _ => None,
    }
}

fn is_video_sample_entry(sample_entry: &str) -> bool {
    matches!(
        sample_entry,
        "avc1"
            | "avc3"
            | "hvc1"
            | "hev1"
            | "dvh1"
            | "dvhe"
            | "dva1"
            | "dvav"
            | "av01"
            | "vp08"
            | "vp09"
            | "mp4v"
            | "s263"
    )
}

fn is_audio_sample_entry(sample_entry: &str) -> bool {
    matches!(
        sample_entry,
        "mp4a" | "ac-3" | "ec-3" | "Opus" | "fLaC" | "alac" | ".mp3" | "lpcm"
    )
}

fn is_subtitle_sample_entry(sample_entry: &str) -> bool {
    matches!(sample_entry, "tx3g" | "wvtt" | "stpp" | "c608")
}

fn parse_mp4_track_metadata(data: &[u8]) -> HashMap<u32, Mp4TrackMetadata> {
    let mut metadata = HashMap::new();
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"moov" {
            parse_moov(payload, &mut metadata);
        }
    });
    metadata
}

fn parse_moov(data: &[u8], metadata: &mut HashMap<u32, Mp4TrackMetadata>) {
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"trak" {
            let track = parse_trak(payload);
            if let Some(track_id) = track.track_id {
                metadata.insert(track_id, track);
            }
        }
    });
}

fn parse_trak(data: &[u8]) -> Mp4TrackMetadata {
    let mut track = Mp4TrackMetadata::default();
    for_each_mp4_box(data, |header, payload| match &header.name {
        b"tkhd" => {
            apply_tkhd_metadata(payload, &mut track);
        }
        b"mdia" => parse_mdia(payload, &mut track),
        b"udta" => parse_udta(payload, &mut track),
        _ => {}
    });
    parse_kind_boxes(data, &mut track);
    track
}

fn parse_kind_boxes(data: &[u8], track: &mut Mp4TrackMetadata) {
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"kind" {
            apply_kind_metadata(payload, track);
        }
        parse_kind_boxes(payload, track);
    });
}

fn parse_mdia(data: &[u8], track: &mut Mp4TrackMetadata) {
    for_each_mp4_box(data, |header, payload| match &header.name {
        b"mdhd" => {
            track.language = parse_mdhd_language(payload);
        }
        b"hdlr" => {
            track.handler_type = parse_hdlr_type(payload);
        }
        b"minf" => parse_minf(payload, track),
        _ => {}
    });
}

fn parse_udta(data: &[u8], track: &mut Mp4TrackMetadata) {
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"name" {
            track.name = parse_name_box(payload);
        }
    });
}

fn parse_minf(data: &[u8], track: &mut Mp4TrackMetadata) {
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"stbl" {
            parse_stbl(payload, track);
        }
    });
}

fn parse_stbl(data: &[u8], track: &mut Mp4TrackMetadata) {
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"stsd" {
            parse_stsd(payload, track);
        }
    });
}

fn parse_stsd(data: &[u8], track: &mut Mp4TrackMetadata) {
    if data.len() < 8 {
        return;
    }
    let offset = 8; // version/flags + entry_count
    if let Some(header) = read_box_header_from_bytes(&data[offset..]) {
        let size = header.size as usize;
        if size < header.header_size || offset + size > data.len() {
            return;
        }
        let sample_entry = &data[offset..offset + size];
        let payload = &sample_entry[header.header_size..];
        let fourcc = fourcc_to_string(header.name);
        track.sample_entry_fourcc = Some(fourcc.clone());

        if is_video_sample_entry(&fourcc) {
            let child_offset = 78;
            if payload.len() >= child_offset {
                for_each_mp4_box(&payload[child_offset..], |child, child_payload| {
                    let child_name = fourcc_to_string(child.name);
                    match child_name.as_str() {
                        "avcC" | "hvcC" | "av1C" => {
                            track.codec_private = Some(child_payload.to_vec());
                        }
                        t if MP4_DOVI_TYPES.contains(&t) => {
                            track.dovi_config = Some(child_payload.to_vec());
                        }
                        _ => {}
                    }
                });
            }
        }
    }
}

fn apply_tkhd_metadata(data: &[u8], track: &mut Mp4TrackMetadata) {
    track.track_id = parse_tkhd_track_id(data);
    track.default_track =
        parse_full_box_flags(data).is_some_and(|flags| (flags & MOV_TKHD_FLAG_ENABLED) != 0);
}

fn parse_tkhd_track_id(data: &[u8]) -> Option<u32> {
    if data.len() < 24 {
        return None;
    }
    match data[0] {
        1 if data.len() >= 32 => Some(read_be_u32(&data[20..24])?),
        _ => Some(read_be_u32(&data[12..16])?),
    }
}

fn parse_full_box_flags(data: &[u8]) -> Option<u32> {
    if data.len() < 4 {
        return None;
    }
    Some(u32::from_be_bytes([0, data[1], data[2], data[3]]))
}

fn parse_mdhd_language(data: &[u8]) -> Option<String> {
    if data.len() < 24 {
        return None;
    }
    let language_offset = match data[0] {
        1 if data.len() >= 34 => 32,
        _ => 20,
    };
    let code = read_be_u16(&data[language_offset..language_offset + 2])?;
    decode_mdhd_language(code)
}

fn parse_hdlr_type(data: &[u8]) -> Option<[u8; 4]> {
    data.get(8..12)?.try_into().ok()
}

fn parse_name_box(data: &[u8]) -> Option<String> {
    let raw = if data.len() > 4 { &data[4..] } else { data };
    let name = std::str::from_utf8(raw).ok()?.trim_end_matches('\0').trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

fn apply_kind_metadata(data: &[u8], track: &mut Mp4TrackMetadata) {
    let Some(payload) = data.get(4..) else {
        return;
    };
    let Some((scheme, value)) = parse_kind_strings(payload) else {
        return;
    };
    if scheme == "urn:mpeg:dash:role:2011" && value.starts_with("forced-subtitle") {
        track.forced = true;
    }
}

fn parse_kind_strings(data: &[u8]) -> Option<(String, String)> {
    let scheme_end = data.iter().position(|&byte| byte == 0)?;
    let scheme = std::str::from_utf8(&data[..scheme_end]).ok()?.to_owned();
    let rest = data.get(scheme_end + 1..)?;
    let value_end = rest
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(rest.len());
    let value = std::str::from_utf8(&rest[..value_end]).ok()?.to_owned();
    Some((scheme, value))
}

fn decode_mdhd_language(code: u16) -> Option<String> {
    let chars = [
        (((code >> 10) & 0x1F) as u8).saturating_add(0x60),
        (((code >> 5) & 0x1F) as u8).saturating_add(0x60),
        ((code & 0x1F) as u8).saturating_add(0x60),
    ];
    if chars.iter().all(|c| c.is_ascii_lowercase()) {
        Some(String::from_utf8_lossy(&chars).into_owned())
    } else {
        None
    }
}

fn for_each_mp4_box(mut data: &[u8], mut f: impl FnMut(Mp4BoxHeader, &[u8])) {
    while let Some(header) = read_box_header_from_bytes(data) {
        let size = header.size as usize;
        if size < header.header_size || size > data.len() {
            break;
        }
        let payload = &data[header.header_size..size];
        f(header, payload);
        if size == data.len() {
            break;
        }
        data = &data[size..];
    }
}

fn read_box_header<R: Read>(
    reader: &mut R,
    available: u64,
) -> Result<Option<Mp4BoxHeader>, MediaInfoError> {
    if available < 8 {
        return Ok(None);
    }

    let mut header = [0_u8; 8];
    reader
        .read_exact(&mut header)
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    let size32 = u32::from_be_bytes(header[0..4].try_into().unwrap()) as u64;
    let name: [u8; 4] = header[4..8].try_into().unwrap();
    let mut size = size32;
    let mut header_size = 8;

    if size32 == 1 {
        if available < 16 {
            return Ok(None);
        }
        let mut extended = [0_u8; 8];
        reader
            .read_exact(&mut extended)
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        size = u64::from_be_bytes(extended);
        header_size = 16;
    } else if size32 == 0 {
        size = available;
    }

    if size < header_size as u64 {
        return Err(MediaInfoError::Parse(format!(
            "invalid MP4 box size {} for {}",
            size,
            fourcc_to_string(name)
        )));
    }

    Ok(Some(Mp4BoxHeader {
        name,
        size,
        header_size,
    }))
}

fn read_box_header_from_bytes(data: &[u8]) -> Option<Mp4BoxHeader> {
    if data.len() < 8 {
        return None;
    }
    let size32 = u32::from_be_bytes(data[0..4].try_into().ok()?) as u64;
    let name: [u8; 4] = data[4..8].try_into().ok()?;
    let (size, header_size) = if size32 == 1 {
        if data.len() < 16 {
            return None;
        }
        (u64::from_be_bytes(data[8..16].try_into().ok()?), 16)
    } else if size32 == 0 {
        (data.len() as u64, 8)
    } else {
        (size32, 8)
    };
    if size < header_size as u64 {
        return None;
    }
    Some(Mp4BoxHeader {
        name,
        size,
        header_size,
    })
}

fn read_be_u16(data: &[u8]) -> Option<u16> {
    let bytes: [u8; 2] = data.get(0..2)?.try_into().ok()?;
    Some(u16::from_be_bytes(bytes))
}

fn read_be_u32(data: &[u8]) -> Option<u32> {
    let bytes: [u8; 4] = data.get(0..4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn fourcc_to_string(fourcc: [u8; 4]) -> String {
    String::from_utf8_lossy(&fourcc).into_owned()
}

/// Extract codec identifier and codec-private bytes from a video sample entry.
fn video_codec_info(codec_specific: &VideoCodecSpecific) -> (Option<String>, Option<Vec<u8>>) {
    match codec_specific {
        VideoCodecSpecific::AVCConfig(data) => {
            let private: Vec<u8> = data.iter().copied().collect();
            (Some("avc1".into()), Some(private))
        }
        VideoCodecSpecific::VPxConfig(vpx) => {
            let private: Vec<u8> = vpx.codec_init.iter().copied().collect();
            let private = if private.is_empty() {
                None
            } else {
                Some(private)
            };
            (None, private)
        }
        VideoCodecSpecific::AV1Config(av1c) => {
            let private: Vec<u8> = av1c.raw_config.iter().copied().collect();
            (Some("av01".into()), Some(private))
        }
        VideoCodecSpecific::ESDSConfig(data) => {
            let private: Vec<u8> = data.iter().copied().collect();
            (Some("mp4v".into()), Some(private))
        }
        VideoCodecSpecific::H263Config(data) => {
            let private: Vec<u8> = data.iter().copied().collect();
            (Some("s263".into()), Some(private))
        }
    }
}

/// Map an `AudioCodecSpecific` variant to a FourCC / codec identifier string.
fn audio_codec_id(codec_specific: &AudioCodecSpecific) -> Option<String> {
    match codec_specific {
        AudioCodecSpecific::ES_Descriptor(esds) => match esds.audio_codec {
            CodecType::AAC => Some("mp4a".into()),
            CodecType::MP3 => Some(".mp3".into()),
            _ => None,
        },
        AudioCodecSpecific::FLACSpecificBox(_) => Some("fLaC".into()),
        AudioCodecSpecific::OpusSpecificBox(_) => Some("Opus".into()),
        AudioCodecSpecific::ALACSpecificBox(_) => Some("alac".into()),
        AudioCodecSpecific::MP3 => Some(".mp3".into()),
        AudioCodecSpecific::LPCM => Some("lpcm".into()),
    }
}

/// Fallback: derive a FourCC-style string from the mp4parse `CodecType` enum.
fn codec_type_to_fourcc(ct: CodecType) -> String {
    match ct {
        CodecType::H264 => "avc1".into(),
        CodecType::AV1 => "av01".into(),
        CodecType::VP9 => "vp09".into(),
        CodecType::VP8 => "vp08".into(),
        CodecType::MP4V => "mp4v".into(),
        CodecType::H263 => "s263".into(),
        CodecType::AAC => "mp4a".into(),
        CodecType::MP3 => ".mp3".into(),
        CodecType::FLAC => "fLaC".into(),
        CodecType::Opus => "Opus".into(),
        CodecType::ALAC => "alac".into(),
        CodecType::LPCM => "lpcm".into(),
        CodecType::EncryptedVideo => "encv".into(),
        CodecType::EncryptedAudio => "enca".into(),
        CodecType::Unknown => "unknown".into(),
    }
}

/// Estimate a track's bitrate from the `stsz` (sample size) box.
fn estimate_track_bitrate(track: &mp4parse::Track, container_duration: Option<f64>) -> Option<i64> {
    let stsz = track.stsz.as_ref()?;

    let total_bytes: u64 = if stsz.sample_size > 0 {
        let count = if !stsz.sample_sizes.is_empty() {
            stsz.sample_sizes.len() as u64
        } else {
            track
                .stts
                .as_ref()
                .map(|stts| stts.samples.iter().map(|s| u64::from(s.sample_count)).sum())
                .unwrap_or(0)
        };
        u64::from(stsz.sample_size) * count
    } else {
        stsz.sample_sizes.iter().map(|&s| u64::from(s)).sum()
    };

    if total_bytes == 0 {
        return None;
    }

    let duration = track_duration_seconds(track).or(container_duration)?;
    if duration <= 0.0 {
        return None;
    }

    Some((total_bytes as f64 * 8.0 / duration) as i64)
}

fn track_duration_seconds(track: &mp4parse::Track) -> Option<f64> {
    let ts = track.timescale.as_ref().map(|TrackTimeScale(t, _)| *t)?;
    if ts == 0 {
        return None;
    }
    let duration = track.duration.as_ref().map(|d| d.0)?;
    Some(duration as f64 / ts as f64)
}

fn estimate_frame_rate(track: &mp4parse::Track) -> Option<f64> {
    let ts = track.timescale.as_ref().map(|TrackTimeScale(t, _)| *t)?;
    if ts == 0 {
        return None;
    }
    let stts = track.stts.as_ref()?;
    if stts.samples.is_empty() {
        return None;
    }

    let total_samples: u64 = stts.samples.iter().map(|s| u64::from(s.sample_count)).sum();
    if total_samples == 0 {
        return None;
    }

    let dominant = stts.samples.iter().max_by_key(|s| s.sample_count)?;
    if u64::from(dominant.sample_count) * 10 >= total_samples * 9 && dominant.sample_delta > 0 {
        let fps = ts as f64 / f64::from(dominant.sample_delta);
        if fps > 0.0 && fps < 1000.0 {
            return Some(fps);
        }
    }

    let total_delta: u64 = stts
        .samples
        .iter()
        .map(|s| u64::from(s.sample_count) * u64::from(s.sample_delta))
        .sum();
    if total_delta == 0 {
        return None;
    }

    let fps = total_samples as f64 * ts as f64 / total_delta as f64;
    if fps > 0.0 && fps < 1000.0 {
        Some(fps)
    } else {
        None
    }
}

/// Scan the first sample of the primary HEVC-like video track for HDR10+ metadata.
fn scan_mp4_hdr10plus(path: &Path, ctx: &MediaContext, tracks: &mut [ParsedMp4Track]) {
    let Some(raw_idx) = tracks.iter().position(|track| {
        track.raw.kind == TrackKind::Video
            && matches!(track.raw.codec_name.as_deref(), Some("hevc"))
    }) else {
        return;
    };

    let nal_length_size = tracks[raw_idx]
        .raw
        .codec_private
        .as_deref()
        .map(crate::codec::hevc_nal_length_size)
        .unwrap_or(4);

    let mp4_track = tracks[raw_idx]
        .track_id
        .and_then(|track_id| {
            ctx.tracks
                .iter()
                .find(|track| track.track_id == Some(track_id))
        })
        .or_else(|| {
            ctx.tracks.iter().find(|track| {
                matches!(
                    track.track_type,
                    TrackType::Video | TrackType::Picture | TrackType::AuxiliaryVideo
                )
            })
        });
    let Some(mp4_track) = mp4_track else {
        return;
    };

    let first_offset = mp4_track
        .stco
        .as_ref()
        .and_then(|stco| stco.offsets.first().copied());
    let first_size = mp4_track.stsz.as_ref().and_then(|stsz| {
        if stsz.sample_size > 0 {
            Some(stsz.sample_size as u64)
        } else {
            stsz.sample_sizes.first().map(|&size| size as u64)
        }
    });

    let (offset, size) = match (first_offset, first_size) {
        (Some(offset), Some(size)) if size > 0 && size <= HDR10PLUS_SAMPLE_LIMIT_BYTES => {
            (offset, size as usize)
        }
        _ => return,
    };

    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return,
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return;
    }
    let mut buf = vec![0_u8; size];
    if file.read_exact(&mut buf).is_err() {
        return;
    }

    if crate::codec::scan_hevc_frame_for_hdr10plus(&buf, nal_length_size) {
        tracks[raw_idx].raw.has_hdr10plus = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_box(name: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (8 + payload.len()) as u32;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn codec_type_to_fourcc_roundtrips() {
        assert_eq!(codec_type_to_fourcc(CodecType::H264), "avc1");
        assert_eq!(codec_type_to_fourcc(CodecType::AV1), "av01");
        assert_eq!(codec_type_to_fourcc(CodecType::VP9), "vp09");
        assert_eq!(codec_type_to_fourcc(CodecType::AAC), "mp4a");
        assert_eq!(codec_type_to_fourcc(CodecType::FLAC), "fLaC");
        assert_eq!(codec_type_to_fourcc(CodecType::Opus), "Opus");
        assert_eq!(codec_type_to_fourcc(CodecType::Unknown), "unknown");
    }

    #[test]
    fn normalize_mp4_codecs() {
        assert_eq!(normalize_codec_name("avc1").as_deref(), Some("h264"));
        assert_eq!(normalize_codec_name("av01").as_deref(), Some("av1"));
        assert_eq!(normalize_codec_name("mp4a").as_deref(), Some("aac"));
        assert_eq!(normalize_codec_name("fLaC").as_deref(), Some("flac"));
        assert_eq!(normalize_codec_name("Opus").as_deref(), Some("opus"));
        assert_eq!(normalize_codec_name("unknown"), None);
    }

    #[test]
    fn metadata_copy_seeks_over_mdat_payload() {
        let ftyp = make_box(b"ftyp", b"isom\0\0\0\0isom");
        let mdat_payload = vec![0_u8; 1024 * 1024];
        let mdat = make_box(b"mdat", &mdat_payload);
        let moov = make_box(b"moov", &make_box(b"mvhd", &[0_u8; 32]));
        let file = [ftyp.clone(), mdat, moov.clone()].concat();

        let mut reader = TrackedReader::new(Cursor::new(file));
        let metadata = prepare_mp4_metadata_from_reader(&mut reader).unwrap();
        let stats = reader.stats();

        assert_eq!(metadata, [ftyp, moov].concat());
        assert!(
            stats.bytes_read < 512,
            "unexpected payload read: {:?}",
            stats
        );
        assert!(stats.seeks >= 3, "expected explicit skipping: {:?}", stats);
    }

    #[test]
    fn parse_trak_extracts_default_and_forced_subtitle_flags() {
        let mut tkhd_payload = vec![0_u8; 24];
        tkhd_payload[3] = MOV_TKHD_FLAG_ENABLED as u8;
        tkhd_payload[12..16].copy_from_slice(&7_u32.to_be_bytes());

        let kind_payload = [
            [0_u8, 0, 0, 0].as_slice(),
            b"urn:mpeg:dash:role:2011\0forced-subtitle\0".as_slice(),
        ]
        .concat();

        let trak_payload = [
            make_box(b"tkhd", &tkhd_payload),
            make_box(b"udta", &make_box(b"kind", &kind_payload)),
        ]
        .concat();

        let track = parse_trak(&trak_payload);

        assert_eq!(track.track_id, Some(7));
        assert!(track.default_track);
        assert!(track.forced);
    }
}
