use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::AnalysisProfile;
use crate::MediaInfoError;
use crate::codec::{
    audio_profile_probe_spec, detect_audio_profile_from_probe_bytes,
    detect_dts_channels_from_probe_bytes, detect_header_audio_profile, extract_h265_info,
    hevc_nal_length_size, merge_audio_profile, normalize_codec_name, normalize_pcm_codec_name,
    normalize_vfw_codec_name, scan_hevc_frame_for_hdr10plus, scan_itu_t35_payload_for_hdr10plus,
};
use crate::scan;
use crate::ts::{find_ac3_header, find_dts_header, find_eac3_header};
use crate::types::{RawContainer, RawTrack, TrackKind};

const MKV_FPS_PROBE_MAX_SCAN_BYTES: u64 = 32 * 1024 * 1024;
const MKV_DEEP_PROBE_CANDIDATE_READ_BYTES: usize = 64 * 1024;
const MKV_FPS_PROBE_MAX_TIMESTAMPS: usize = 12;
const MKV_FPS_PROBE_MAX_BLOCKS: usize = 24;
const MKV_RICH_HDR10PLUS_SCAN_MAX_BYTES: u64 = 512 * 1024;
const MKV_RICH_HDR10PLUS_MAX_VIDEO_BLOCKS: usize = 4;
const MKV_SONARR_HDR10PLUS_SCAN_MAX_BYTES: u64 = 512 * 1024;
const MKV_SONARR_HDR10PLUS_MAX_VIDEO_BLOCKS: usize = 1;
const MKV_SONARR_HDR10PLUS_MAX_FILE_SCAN_BYTES: u64 = 32 * 1024 * 1024;
const MKV_HDR10PLUS_BLOCKADDITIONAL_PEEK_BYTES: u64 = 16;
const MKV_CHAPTER_SCAN_MAX_BYTES: usize = 2 * 1024 * 1024;
const MKV_AUDIO_PROFILE_SCAN_MAX_BYTES: u64 = 8 * 1024;
const MKV_AUDIO_PROFILE_MAX_BLOCKS: usize = 48;
const MKV_EAC3_AUDIO_PROFILE_MAX_BLOCKS: usize = 1;
const MKV_AUDIO_PROFILE_MAX_FILE_SCAN_BYTES: u64 = 128 * 1024 * 1024;
const EBML_ID_EBML: u32 = 0x1A45_DFA3;
const EBML_ID_DOC_TYPE: u32 = 0x4282;
const EBML_ID_SEGMENT: u32 = 0x1853_8067;
const EBML_ID_SEEK_HEAD: u32 = 0x114D_9B74;
const EBML_ID_SEEK: u32 = 0x4DBB;
const EBML_ID_SEEK_ID: u32 = 0x53AB;
const EBML_ID_SEEK_POSITION: u32 = 0x53AC;
const EBML_ID_INFO: u32 = 0x1549_A966;
const EBML_ID_TIMESTAMP_SCALE: u32 = 0x002A_D7B1;
const EBML_ID_DURATION: u32 = 0x4489;
const EBML_ID_TRACKS: u32 = 0x1654_AE6B;
const EBML_ID_TRACK_ENTRY: u32 = 0xAE;
const EBML_ID_TRACK_NUMBER: u32 = 0xD7;
const EBML_ID_TRACK_TYPE: u32 = 0x83;
const EBML_ID_FLAG_DEFAULT: u32 = 0x88;
const EBML_ID_FLAG_FORCED: u32 = 0x55AA;
const EBML_ID_DEFAULT_DURATION: u32 = 0x0023_E383;
const EBML_ID_NAME: u32 = 0x536E;
const EBML_ID_LANGUAGE: u32 = 0x0022_B59C;
const EBML_ID_LANGUAGE_BCP47: u32 = 0x0022_B59D;
const EBML_ID_CODEC_ID: u32 = 0x86;
const EBML_ID_CODEC_PRIVATE: u32 = 0x63A2;
const EBML_ID_VIDEO: u32 = 0xE0;
const EBML_ID_PIXEL_WIDTH: u32 = 0xB0;
const EBML_ID_PIXEL_HEIGHT: u32 = 0xBA;
const EBML_ID_COLOUR: u32 = 0x55B0;
const EBML_ID_TRANSFER_CHARACTERISTICS: u32 = 0x55BA;
const EBML_ID_AUDIO: u32 = 0xE1;
const EBML_ID_CHANNELS: u32 = 0x9F;
const EBML_ID_BIT_DEPTH: u32 = 0x6264;
const EBML_ID_TRACK_CONTENT_ENCODINGS: u32 = 0x6D80;
const EBML_ID_TRACK_CONTENT_ENCODING: u32 = 0x6240;
const EBML_ID_ENCODING_ORDER: u32 = 0x5031;
const EBML_ID_ENCODING_SCOPE: u32 = 0x5032;
const EBML_ID_ENCODING_TYPE: u32 = 0x5033;
const EBML_ID_ENCODING_COMPRESSION: u32 = 0x5034;
const EBML_ID_ENCODING_COMP_ALGO: u32 = 0x4254;
const EBML_ID_ENCODING_COMP_SETTINGS: u32 = 0x4255;
const EBML_ID_CLUSTER: u32 = 0x1F43_B675;
const EBML_ID_TIMESTAMP: u32 = 0xE7;
const EBML_ID_SIMPLE_BLOCK: u32 = 0xA3;
const EBML_ID_BLOCK_GROUP: u32 = 0xA0;
const EBML_ID_BLOCK: u32 = 0xA1;
const EBML_ID_BLOCK_ADDITIONS: u32 = 0x75A1;
const EBML_ID_BLOCK_MORE: u32 = 0xA6;
const EBML_ID_BLOCK_ADDITIONAL: u32 = 0xA5;
const EBML_ID_CHAPTERS: u32 = 0x1043_A770;
const EBML_ID_EDITION_ENTRY: u32 = 0x45B9;
const EBML_ID_CHAPTER_ATOM: u32 = 0xB6;
const EBML_ID_CHAPTER_TIME_START: u32 = 0x91;
const EBML_ID_BLOCK_ADDITION_MAPPING: u32 = 0x41E4;
const EBML_ID_BLOCK_ADD_ID_TYPE: u32 = 0x41E7;
const EBML_ID_BLOCK_ADD_ID_EXTRA_DATA: u32 = 0x41ED;
const MATROSKA_BLOCK_ADD_ID_TYPE_ITU_T_T35: u64 = 4;
const DOVI_BLOCK_ADD_ID_TYPE: u64 = 0x6476;
const MATROSKA_TRACK_ENCODING_SCOPE_FRAME_CONTENTS: u64 = 1;
const MATROSKA_TRACK_ENCODING_TYPE_COMPRESSION: u64 = 0;
const MATROSKA_TRACK_ENCODING_COMP_HEADERSTRIP: u64 = 3;
const MKV_KEEP_ELEMENT_MAX_BYTES: u64 = 256 * 1024 * 1024;
const MKV_METADATA_AGGREGATE_MAX_BYTES: u64 = 512 * 1024 * 1024;
const MKV_DEEP_PROBE_MAX_DEPTH: usize = 10;

fn next_mkv_segment_probe_depth(depth: usize) -> Option<usize> {
    (depth < MKV_DEEP_PROBE_MAX_DEPTH).then(|| depth.saturating_add(1))
}

fn next_mkv_deep_probe_depth(depth: usize) -> Option<usize> {
    (depth <= MKV_DEEP_PROBE_MAX_DEPTH).then(|| depth.saturating_add(1))
}

fn normalize_mkv_track_language(
    kind: TrackKind,
    _language_bcp47: Option<&str>,
    language: Option<&str>,
) -> Option<String> {
    if let Some(language) = language {
        return normalize_explicit_mkv_language_tag(language);
    }
    (kind != TrackKind::Video).then_some("eng".to_owned())
}

/// Parse an MKV/WebM file into a [`RawContainer`].
pub(crate) fn parse_mkv(
    path: &Path,
    profile: AnalysisProfile,
) -> Result<RawContainer, MediaInfoError> {
    let file = open_buffered_file(path)?;
    let mut scanner = MkvRawScanner::new(file)?;
    let header = parse_mkv_header(&mut scanner)?;
    let format_name = header.format_name;
    let duration_seconds = header.duration_seconds;
    let num_chapters = header
        .num_chapters
        .or_else(|| {
            (!profile.skips_deep_probes() && !header.chapters_known_absent)
                .then(|| {
                    scanner
                        .scan_prefix_for_chapter_count(MKV_CHAPTER_SCAN_MAX_BYTES as u64)
                        .ok()
                        .flatten()
                })
                .flatten()
        })
        .or(Some(0));
    let file_size_hint = header.file_size_hint.unwrap_or(0);
    let timestamp_scale_ns = header.timestamp_scale_ns;
    let mut tracks = Vec::with_capacity(header.tracks.len());
    let mut primary_video_track_num = None;
    let mut primary_video_index = None;
    let mut primary_video_signals = MkvTrackSignals::default();
    let mut audio_track_refs = Vec::new();

    for parsed_track in header.tracks {
        if parsed_track.raw.kind == TrackKind::Video && primary_video_track_num.is_none() {
            primary_video_track_num = Some(parsed_track.track_number);
            primary_video_index = Some(tracks.len());
            primary_video_signals = parsed_track.signals.clone();
        }
        if parsed_track.raw.kind == TrackKind::Audio {
            audio_track_refs.push((
                parsed_track.track_number,
                tracks.len(),
                parsed_track.signals.header_strip_prefix.clone(),
            ));
        }
        tracks.push(parsed_track.raw);
    }

    // Use a fast overall bitrate estimate for the primary video stream instead
    // of walking large frame ranges over networked filesystems.
    if file_size_hint > 0
        && let Some(duration_seconds) = duration_seconds
        && duration_seconds > 0.0
        && let Some(video_idx) = primary_video_index
    {
        tracks[video_idx].bit_rate_bps =
            Some((file_size_hint as f64 * 8.0 / duration_seconds) as i64);
    }

    let mut frame_rate_probe = None;
    let mut hdr10plus_probe = None;

    if let Some(video_idx) = primary_video_index {
        if let Some(dovi_config) = primary_video_signals.dovi_config.clone() {
            tracks[video_idx].dovi_config = Some(dovi_config);
        }

        if !profile.skips_deep_probes() {
            if !is_plausible_frame_rate(tracks[video_idx].frame_rate_fps)
                && let Some(track_num) = primary_video_track_num
            {
                frame_rate_probe = Some(MkvFrameRateProbeRequest {
                    target_track_num: track_num,
                    timestamp_scale_ns,
                });
            }

            if let Some(track_num) = primary_video_track_num {
                let nal_length_size = tracks[video_idx]
                    .codec_private
                    .as_deref()
                    .map(hevc_nal_length_size)
                    .unwrap_or(4);

                if profile == AnalysisProfile::FfprobeParity
                    && should_probe_sonarr_hdr10plus(&tracks[video_idx])
                {
                    hdr10plus_probe = Some(MkvHdr10PlusProbeRequest::new(
                        track_num,
                        nal_length_size,
                        primary_video_signals.has_itu_t_t35_mapping,
                        Hdr10PlusProbeLimits::sonarr(),
                    ));
                } else if profile == AnalysisProfile::DefaultRich
                    && should_confirm_mkv_hdr10plus(&tracks[video_idx], &primary_video_signals)
                {
                    hdr10plus_probe = Some(MkvHdr10PlusProbeRequest::new(
                        track_num,
                        nal_length_size,
                        primary_video_signals.has_itu_t_t35_mapping,
                        Hdr10PlusProbeLimits::rich(),
                    ));
                }
            }
        }
    }

    let audio_probe_requests = if profile.skips_deep_probes() {
        Vec::new()
    } else {
        audio_track_refs
            .into_iter()
            .filter_map(|(track_num, track_idx, header_strip_prefix)| {
                let codec_name = tracks[track_idx].codec_name.clone()?;
                matches!(codec_name.as_str(), "ac3" | "eac3" | "truehd" | "dts").then_some(
                    MkvAudioProbeRequest {
                        target_track_num: track_num,
                        track_idx,
                        codec_name,
                        header_strip_prefix,
                    },
                )
            })
            .collect()
    };

    let deep_probe = probe_mkv_deep_metadata(
        &mut scanner,
        frame_rate_probe,
        hdr10plus_probe,
        audio_probe_requests,
    );

    if let Some(video_idx) = primary_video_index {
        if let Some(fps) = deep_probe.frame_rate_fps
            && should_replace_frame_rate(tracks[video_idx].frame_rate_fps, fps)
        {
            tracks[video_idx].frame_rate_fps = Some(fps);
        }
        if let Some(has_hdr10plus) = deep_probe.has_hdr10plus {
            tracks[video_idx].has_hdr10plus = has_hdr10plus;
        }
        if tracks[video_idx].frame_rate_fps.is_none() {
            tracks[video_idx].frame_rate_fps =
                fallback_frame_rate_from_timestamp_scale(timestamp_scale_ns);
        }
    }

    for (track_idx, scanned) in deep_probe.audio {
        merge_audio_profile(&mut tracks[track_idx].audio_profile, scanned.profile);
        if let Some(channels) = scanned.channels {
            tracks[track_idx].channels =
                merge_scanned_audio_channels(tracks[track_idx].channels, Some(channels));
        }
        if tracks[track_idx].audio_profile.as_deref() == Some("DTS-ES")
            && tracks[track_idx].channels == Some(6)
        {
            tracks[track_idx].channels = Some(7);
        }
    }
    apply_unique_audio_prefix_channel_fallback(&mut scanner, &mut tracks);

    Ok(RawContainer {
        format_name,
        duration_seconds,
        num_chapters,
        tracks,
    })
}

fn open_buffered_file(path: &Path) -> Result<BufReader<std::fs::File>, MediaInfoError> {
    let file = std::fs::File::open(path).map_err(|e| MediaInfoError::Io(e.to_string()))?;
    Ok(BufReader::new(file))
}

#[derive(Debug)]
struct ParsedMkvHeader {
    format_name: String,
    timestamp_scale_ns: f64,
    duration_seconds: Option<f64>,
    num_chapters: Option<i32>,
    chapters_known_absent: bool,
    file_size_hint: Option<u64>,
    tracks: Vec<ParsedMkvTrack>,
}

#[derive(Debug)]
struct ParsedMkvTrack {
    track_number: u64,
    raw: RawTrack,
    signals: MkvTrackSignals,
}

#[derive(Debug, Default)]
struct MkvSeekHeadOffsets {
    seen: bool,
    info: Option<u64>,
    tracks: Option<u64>,
    chapters: Option<u64>,
}

#[derive(Debug)]
struct ParsedMkvInfo {
    timestamp_scale_ns: f64,
    duration_seconds: Option<f64>,
}

impl Default for ParsedMkvInfo {
    fn default() -> Self {
        Self {
            timestamp_scale_ns: 1_000_000.0,
            duration_seconds: None,
        }
    }
}

fn parse_mkv_header<R: Read + Seek>(
    scanner: &mut MkvRawScanner<R>,
) -> Result<ParsedMkvHeader, MediaInfoError> {
    let format_name = parse_mkv_doc_type(scanner)?;
    let segment = scanner.read_next_segment_header()?;
    let segment_data_offset = segment.data_offset;
    let file_size_hint = segment
        .size
        .and_then(|size| segment_data_offset.checked_add(size));
    let segment_end = segment.end(scanner.file_len, scanner.file_len);

    let mut info = None;
    let mut tracks = None;
    let mut num_chapters = None;
    let mut offsets = MkvSeekHeadOffsets::default();

    scanner.seek_to(segment_data_offset)?;
    while scanner.position()? < segment_end {
        let Some(header) = scanner.read_element_header()? else {
            break;
        };
        let child_end = header.end(segment_end, scanner.file_len);
        match header.id {
            EBML_ID_CLUSTER => {
                break;
            }
            EBML_ID_SEEK_HEAD => {
                if let Some(payload) = scanner.read_sized_payload(header, child_end)? {
                    offsets.merge(parse_seek_head_offsets(&payload, segment_data_offset));
                }
            }
            EBML_ID_INFO => {
                if info.is_none()
                    && let Some(payload) = scanner.read_sized_payload(header, child_end)?
                {
                    info = Some(parse_mkv_info_payload(&payload));
                }
            }
            EBML_ID_TRACKS => {
                if tracks.is_none()
                    && let Some(payload) = scanner.read_sized_payload(header, child_end)?
                {
                    tracks = Some(parse_mkv_tracks_payload(&payload));
                }
            }
            EBML_ID_CHAPTERS => {
                if num_chapters.is_none()
                    && let Some(payload) = scanner.read_sized_payload(header, child_end)?
                {
                    num_chapters = parse_mkv_chapters_payload(&payload);
                }
            }
            _ => {}
        }
        scanner.seek_to(child_end)?;
        if info.is_some() && tracks.is_some() && num_chapters.is_some() {
            break;
        }
    }

    if info.is_none()
        && let Some(offset) = offsets.info
    {
        info = scanner
            .read_top_level_payload_at(offset, EBML_ID_INFO)?
            .map(|payload| parse_mkv_info_payload(&payload));
    }
    if tracks.is_none()
        && let Some(offset) = offsets.tracks
    {
        tracks = scanner
            .read_top_level_payload_at(offset, EBML_ID_TRACKS)?
            .map(|payload| parse_mkv_tracks_payload(&payload));
    }
    if num_chapters.is_none()
        && let Some(offset) = offsets.chapters
    {
        num_chapters = scanner
            .read_top_level_payload_at(offset, EBML_ID_CHAPTERS)?
            .and_then(|payload| parse_mkv_chapters_payload(&payload));
    }

    let info = info.unwrap_or_default();
    let tracks = tracks.ok_or_else(|| MediaInfoError::Parse("matroska tracks missing".into()))?;

    Ok(ParsedMkvHeader {
        format_name,
        timestamp_scale_ns: info.timestamp_scale_ns,
        duration_seconds: info.duration_seconds,
        num_chapters,
        chapters_known_absent: offsets.seen && offsets.chapters.is_none() && num_chapters.is_none(),
        file_size_hint,
        tracks,
    })
}

fn parse_mkv_doc_type<R: Read + Seek>(
    scanner: &mut MkvRawScanner<R>,
) -> Result<String, MediaInfoError> {
    let header = match scanner.read_element_header() {
        Ok(Some(header)) => header,
        Ok(None) => return Err(MediaInfoError::Parse("missing ebml header".into())),
        Err(MediaInfoError::Parse(_)) => {
            return Err(MediaInfoError::Parse("missing ebml header".into()));
        }
        Err(error) => return Err(error),
    };
    if header.id != EBML_ID_EBML {
        return Err(MediaInfoError::Parse("missing ebml header".into()));
    }
    let payload = scanner
        .read_sized_payload(header, scanner.file_len)?
        .ok_or_else(|| MediaInfoError::Parse("missing ebml header payload".into()))?;
    let doc_type = find_first_direct_ebml_child(&payload, EBML_ID_DOC_TYPE)
        .map(parse_ebml_string)
        .transpose()?
        .unwrap_or_else(|| "matroska".to_owned());
    Ok(if doc_type == "webm" {
        "webm".to_owned()
    } else {
        "matroska".to_owned()
    })
}

fn parse_seek_head_offsets(payload: &[u8], segment_data_offset: u64) -> MkvSeekHeadOffsets {
    let mut current = payload;
    let mut offsets = MkvSeekHeadOffsets {
        seen: true,
        ..Default::default()
    };
    while !current.is_empty() {
        let Some((id, child_payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        if id == EBML_ID_SEEK {
            let entry_id = find_first_direct_ebml_child(child_payload, EBML_ID_SEEK_ID)
                .and_then(parse_ebml_uint)
                .map(|value| value as u32);
            let position = find_first_direct_ebml_child(child_payload, EBML_ID_SEEK_POSITION)
                .and_then(parse_ebml_uint)
                .and_then(|pos| segment_data_offset.checked_add(pos));
            match (entry_id, position) {
                (Some(EBML_ID_INFO), Some(offset)) => offsets.info = Some(offset),
                (Some(EBML_ID_TRACKS), Some(offset)) => offsets.tracks = Some(offset),
                (Some(EBML_ID_CHAPTERS), Some(offset)) => offsets.chapters = Some(offset),
                _ => {}
            }
        }
        current = &current[consumed..];
    }
    offsets
}

impl MkvSeekHeadOffsets {
    fn merge(&mut self, other: Self) {
        self.seen |= other.seen;
        if self.info.is_none() {
            self.info = other.info;
        }
        if self.tracks.is_none() {
            self.tracks = other.tracks;
        }
        if self.chapters.is_none() {
            self.chapters = other.chapters;
        }
    }
}

fn parse_mkv_info_payload(payload: &[u8]) -> ParsedMkvInfo {
    let mut info = ParsedMkvInfo::default();
    let mut current = payload;
    let mut duration = None;

    while !current.is_empty() {
        let Some((id, child_payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        match id {
            EBML_ID_TIMESTAMP_SCALE => {
                if let Some(scale) = parse_ebml_uint(child_payload) {
                    info.timestamp_scale_ns = scale as f64;
                }
            }
            EBML_ID_DURATION => {
                duration = parse_ebml_float(child_payload)
                    .filter(|value| value.is_finite() && *value >= 0.0);
            }
            _ => {}
        }
        current = &current[consumed..];
    }

    info.duration_seconds = duration.map(|value| value * info.timestamp_scale_ns / 1e9);
    info
}

fn parse_mkv_tracks_payload(payload: &[u8]) -> Vec<ParsedMkvTrack> {
    let mut tracks = Vec::new();
    let mut current = payload;
    while !current.is_empty() {
        let Some((id, child_payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        if id == EBML_ID_TRACK_ENTRY
            && let Some(track) = parse_mkv_track_entry(child_payload)
        {
            tracks.push(track);
        }
        current = &current[consumed..];
    }
    tracks
}

fn parse_mkv_track_entry(payload: &[u8]) -> Option<ParsedMkvTrack> {
    let mut track_number = None;
    let mut kind = None;
    let mut codec_id = None;
    let mut codec_private = None;
    let mut name = None;
    let mut language = None;
    let mut language_bcp47 = None;
    let mut forced = false;
    let mut default_track = true;
    let mut frame_rate_fps = None;
    let mut width = None;
    let mut height = None;
    let mut channels = None;
    let mut audio_bit_depth = None;
    let mut color_transfer = None;

    let mut current = payload;
    while !current.is_empty() {
        let Some((id, child_payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        match id {
            EBML_ID_TRACK_NUMBER => track_number = parse_ebml_uint(child_payload),
            EBML_ID_TRACK_TYPE => {
                kind = parse_ebml_uint(child_payload).and_then(parse_mkv_track_type)
            }
            EBML_ID_FLAG_DEFAULT => {
                default_track = parse_ebml_uint(child_payload)
                    .map(|value| value != 0)
                    .unwrap_or(true)
            }
            EBML_ID_FLAG_FORCED => {
                forced = parse_ebml_uint(child_payload)
                    .map(|value| value != 0)
                    .unwrap_or(false)
            }
            EBML_ID_DEFAULT_DURATION => {
                frame_rate_fps = parse_ebml_uint(child_payload).and_then(|value| {
                    let ns = value as f64;
                    (ns > 0.0)
                        .then_some(1e9 / ns)
                        .filter(|fps| is_plausible_frame_rate(Some(*fps)))
                });
            }
            EBML_ID_NAME => name = parse_ebml_string(child_payload).ok(),
            EBML_ID_LANGUAGE => language = parse_ebml_string(child_payload).ok(),
            EBML_ID_LANGUAGE_BCP47 => language_bcp47 = parse_ebml_string(child_payload).ok(),
            EBML_ID_CODEC_ID => codec_id = parse_ebml_string(child_payload).ok(),
            EBML_ID_CODEC_PRIVATE => codec_private = Some(child_payload.to_vec()),
            EBML_ID_VIDEO => {
                let (parsed_width, parsed_height, parsed_transfer) =
                    parse_mkv_video_payload(child_payload);
                width = parsed_width;
                height = parsed_height;
                color_transfer = parsed_transfer;
            }
            EBML_ID_AUDIO => {
                let (parsed_channels, parsed_bit_depth) = parse_mkv_audio_payload(child_payload);
                channels = parsed_channels;
                audio_bit_depth = parsed_bit_depth;
            }
            _ => {}
        }
        current = &current[consumed..];
    }

    let track_number = track_number?;
    let kind = kind?;
    let codec_id = codec_id?;
    let signals = track_entry_signals(payload);
    let mut codec_name = normalize_pcm_codec_name(&codec_id, audio_bit_depth)
        .or_else(|| {
            (codec_id == "V_MS/VFW/FOURCC")
                .then(|| normalize_vfw_codec_name(codec_private.as_deref()))
                .flatten()
        })
        .or_else(|| normalize_codec_name(&codec_id));
    if codec_id == "S_TEXT/WEBVTT" {
        codec_name = None;
    }
    if channels.is_none() && codec_name.as_deref() == Some("flac") {
        channels = codec_private
            .as_deref()
            .and_then(parse_mkv_flac_codec_private_channels);
    }
    let audio_profile = if codec_name.as_deref() == Some("dts") {
        None
    } else {
        detect_header_audio_profile(&codec_id, codec_name.as_deref(), codec_private.as_deref())
    };

    Some(ParsedMkvTrack {
        track_number,
        raw: RawTrack {
            kind,
            codec_id,
            codec_name,
            audio_profile,
            codec_private,
            width,
            height,
            channels,
            bit_rate_bps: None,
            language: normalize_mkv_track_language(
                kind,
                language_bcp47.as_deref(),
                language.as_deref(),
            ),
            name,
            forced,
            default_track,
            frame_rate_fps,
            color_transfer,
            dovi_config: None,
            has_hdr10plus: false,
        },
        signals,
    })
}

fn parse_mkv_video_payload(payload: &[u8]) -> (Option<i32>, Option<i32>, Option<u32>) {
    let mut width = None;
    let mut height = None;
    let mut color_transfer = None;
    let mut current = payload;

    while !current.is_empty() {
        let Some((id, child_payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        match id {
            EBML_ID_PIXEL_WIDTH => width = parse_ebml_uint(child_payload).map(|value| value as i32),
            EBML_ID_PIXEL_HEIGHT => {
                height = parse_ebml_uint(child_payload).map(|value| value as i32)
            }
            EBML_ID_COLOUR => color_transfer = parse_mkv_colour_payload(child_payload),
            _ => {}
        }
        current = &current[consumed..];
    }

    (width, height, color_transfer)
}

fn parse_mkv_colour_payload(payload: &[u8]) -> Option<u32> {
    let mut current = payload;
    while !current.is_empty() {
        let Some((id, child_payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        if id == EBML_ID_TRANSFER_CHARACTERISTICS {
            return parse_ebml_uint(child_payload).map(|value| value as u32);
        }
        current = &current[consumed..];
    }
    None
}

fn parse_mkv_audio_payload(payload: &[u8]) -> (Option<i32>, Option<i32>) {
    let mut channels = None;
    let mut bit_depth = None;
    let mut current = payload;

    while !current.is_empty() {
        let Some((id, child_payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        match id {
            EBML_ID_CHANNELS => channels = parse_ebml_uint(child_payload).map(|value| value as i32),
            EBML_ID_BIT_DEPTH => {
                bit_depth = parse_ebml_uint(child_payload).map(|value| value as i32)
            }
            _ => {}
        }
        current = &current[consumed..];
    }

    (channels, bit_depth)
}

fn parse_mkv_flac_codec_private_channels(data: &[u8]) -> Option<i32> {
    let streaminfo = if data.starts_with(b"fLaC") {
        let metadata = data.get(4..)?;
        let block_header = metadata.get(..4)?;
        if (block_header[0] & 0x7F) != 0 {
            return None;
        }
        let block_len = ((u32::from(block_header[1])) << 16)
            | ((u32::from(block_header[2])) << 8)
            | u32::from(block_header[3]);
        if block_len < 34 {
            return None;
        }
        metadata.get(4..4 + 34)?
    } else {
        data.get(..34)?
    };

    // ff_flac_parse_streaminfo() reads the 3-bit channel count from
    // STREAMINFO byte 12 bits 3..1 after the 20-bit sample-rate field.
    let channel_bits = (streaminfo.get(12)? >> 1) & 0x07;
    Some(i32::from(channel_bits) + 1)
}

fn parse_mkv_chapters_payload(payload: &[u8]) -> Option<i32> {
    let first_edition_payload = find_first_direct_ebml_child(payload, EBML_ID_EDITION_ENTRY)?;
    Some(count_top_level_mkv_chapters_ffprobe_style(
        first_edition_payload,
    ))
}

fn parse_mkv_track_type(value: u64) -> Option<TrackKind> {
    match value {
        1 => Some(TrackKind::Video),
        2 => Some(TrackKind::Audio),
        17 => Some(TrackKind::Subtitle),
        _ => None,
    }
}

fn normalize_explicit_mkv_language_tag(language: &str) -> Option<String> {
    let trimmed = language.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("und") {
        return None;
    }
    Some(trimmed.to_owned())
}

fn merge_scanned_audio_channels(existing: Option<i32>, scanned: Option<i32>) -> Option<i32> {
    scanned.or(existing)
}

fn audio_channels_from_probe_bytes(codec_name: &str, data: &[u8]) -> Option<i32> {
    let mut cursor = 0usize;
    let mut channels = None;

    while let Some(candidate) = scan::find_audio_sync_candidate(data, cursor) {
        cursor = candidate.offset.saturating_add(1);
        let parsed =
            match (codec_name, candidate.kind) {
                ("ac3", scan::AudioSyncKind::Ac3) => find_ac3_header(&data[candidate.offset..])
                    .map(|header| i32::from(header.channels)),
                ("eac3", scan::AudioSyncKind::Ac3) => find_eac3_header(&data[candidate.offset..])
                    .map(|header| i32::from(header.channels)),
                ("dts", scan::AudioSyncKind::Dts) => {
                    detect_dts_channels_from_probe_bytes(&data[candidate.offset..]).or_else(|| {
                        find_dts_header(&data[candidate.offset..])
                            .map(|header| i32::from(header.channels))
                    })
                }
                _ => None,
            };
        if let Some(parsed) = parsed {
            channels = Some(channels.map_or(parsed, |existing: i32| existing.max(parsed)));
        }
    }

    channels
}

fn apply_unique_audio_prefix_channel_fallback<R: Read + Seek>(
    scanner: &mut MkvRawScanner<R>,
    tracks: &mut [RawTrack],
) {
    for track_idx in 0..tracks.len() {
        let Some(codec_name) = tracks[track_idx].codec_name.as_deref() else {
            continue;
        };
        if tracks[track_idx].kind != TrackKind::Audio
            || !matches!(codec_name, "ac3" | "eac3" | "dts")
            || tracks[track_idx].channels.is_some()
            || audio_codec_track_count(tracks, codec_name) != 1
        {
            continue;
        }

        if let Ok(Some(channels)) =
            scanner.scan_prefix_for_audio_channels(codec_name, MKV_CHAPTER_SCAN_MAX_BYTES as u64)
        {
            tracks[track_idx].channels =
                merge_scanned_audio_channels(tracks[track_idx].channels, Some(channels));
        }
    }
}

fn audio_codec_track_count(tracks: &[RawTrack], codec_name: &str) -> usize {
    tracks
        .iter()
        .filter(|track| {
            track.kind == TrackKind::Audio && track.codec_name.as_deref() == Some(codec_name)
        })
        .count()
}

#[cfg(test)]
fn count_mkv_chapters_ffprobe_style_from_bytes(data: &[u8]) -> Option<i32> {
    let root = find_ebml_element_payload(data, EBML_ID_SEGMENT).unwrap_or(data);
    let chapters_payload = find_first_direct_ebml_child(root, EBML_ID_CHAPTERS)?;
    let first_edition_payload =
        find_first_direct_ebml_child(chapters_payload, EBML_ID_EDITION_ENTRY)?;
    Some(count_top_level_mkv_chapters_ffprobe_style(
        first_edition_payload,
    ))
}

#[cfg(test)]
fn find_ebml_element_payload(mut data: &[u8], target_id: u32) -> Option<&[u8]> {
    while !data.is_empty() {
        let (id, payload, consumed) = next_ebml_element(data)?;
        if id == target_id {
            return Some(payload);
        }
        data = &data[consumed..];
    }
    None
}

fn find_first_direct_ebml_child(data: &[u8], target_id: u32) -> Option<&[u8]> {
    let mut current = data;
    while !current.is_empty() {
        let (id, payload, consumed) = next_ebml_element(current)?;
        if id == target_id {
            return Some(payload);
        }
        current = &current[consumed..];
    }
    None
}

fn count_top_level_mkv_chapters_ffprobe_style(data: &[u8]) -> i32 {
    let mut starts = Vec::new();
    let mut current = data;
    while !current.is_empty() {
        let Some((id, payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        if id == EBML_ID_CHAPTER_ATOM
            && let Some(start_payload) =
                find_first_direct_ebml_child(payload, EBML_ID_CHAPTER_TIME_START)
            && let Some(start) = parse_ebml_uint(start_payload)
        {
            starts.push(start);
        }
        current = &current[consumed..];
    }
    count_ffprobe_style_chapter_starts(starts)
}

fn count_ffprobe_style_chapter_starts(starts: impl IntoIterator<Item = u64>) -> i32 {
    let mut max_start = None;
    let mut count = 0_i32;
    for start in starts {
        if max_start.is_none_or(|max_start| start > max_start) {
            max_start = Some(start);
            count += 1;
        }
    }
    count
}

fn next_ebml_element(data: &[u8]) -> Option<(u32, &[u8], usize)> {
    let (id, id_len) = parse_ebml_id(data)?;
    let (size, size_len) = parse_ebml_vint(&data[id_len..])?;
    let payload_start = id_len + size_len;
    let payload_end = payload_start.checked_add(size)?;
    if payload_end > data.len() {
        return None;
    }
    Some((id, &data[payload_start..payload_end], payload_end))
}

fn parse_ebml_id(data: &[u8]) -> Option<(u32, usize)> {
    if data.is_empty() {
        return None;
    }
    let first = data[0];
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len == 0 || len > 4 || len > data.len() {
        return None;
    }

    let mut value = 0_u32;
    for &byte in &data[..len] {
        value = (value << 8) | u32::from(byte);
    }
    Some((value, len))
}

fn parse_ebml_uint(data: &[u8]) -> Option<u64> {
    if data.is_empty() || data.len() > 8 {
        return None;
    }

    let mut value = 0_u64;
    for &byte in data {
        value = (value << 8) | u64::from(byte);
    }
    Some(value)
}

fn parse_ebml_float(data: &[u8]) -> Option<f64> {
    match data.len() {
        4 => Some(f32::from_be_bytes(data.try_into().ok()?) as f64),
        8 => Some(f64::from_be_bytes(data.try_into().ok()?)),
        _ => None,
    }
}

fn parse_ebml_string(data: &[u8]) -> Result<String, MediaInfoError> {
    let value = std::str::from_utf8(data)
        .map_err(|e| MediaInfoError::Parse(format!("invalid utf-8 string: {e}")))?;
    Ok(value.trim_end_matches('\0').to_owned())
}

#[derive(Debug, Clone, Default)]
struct MkvTrackSignals {
    has_itu_t_t35_mapping: bool,
    dovi_config: Option<Vec<u8>>,
    header_strip_prefix: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy)]
struct EbmlElementHeader {
    id: u32,
    data_offset: u64,
    size: Option<u64>,
}

impl EbmlElementHeader {
    fn end(self, parent_end: u64, file_len: u64) -> u64 {
        self.size
            .and_then(|size| self.data_offset.checked_add(size))
            .map(|end| end.min(parent_end).min(file_len))
            .unwrap_or_else(|| parent_end.min(file_len))
    }
}

#[derive(Debug, Clone, Copy)]
struct BlockHeaderInfo {
    track_number: u64,
    timestamp: u64,
    payload_offset: u64,
    payload_size: u64,
    lacing_type: u8,
}

#[derive(Debug, Default)]
struct FrameRateProbeState {
    timestamps: Vec<u64>,
    matching_blocks: usize,
}

impl FrameRateProbeState {
    fn done(&self) -> bool {
        self.timestamps.len() >= MKV_FPS_PROBE_MAX_TIMESTAMPS
            || self.matching_blocks >= MKV_FPS_PROBE_MAX_BLOCKS
    }

    fn record_timestamp(&mut self, timestamp: u64) {
        self.matching_blocks += 1;
        if self.timestamps.last().copied() != Some(timestamp) {
            self.timestamps.push(timestamp);
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Hdr10PlusProbeLimits {
    max_bytes_read: u64,
    max_video_blocks: usize,
    max_file_scan_bytes: u64,
    block_additional_peek_bytes: u64,
}

impl Hdr10PlusProbeLimits {
    fn rich() -> Self {
        Self {
            max_bytes_read: MKV_RICH_HDR10PLUS_SCAN_MAX_BYTES,
            max_video_blocks: MKV_RICH_HDR10PLUS_MAX_VIDEO_BLOCKS,
            max_file_scan_bytes: u64::MAX,
            block_additional_peek_bytes: MKV_HDR10PLUS_BLOCKADDITIONAL_PEEK_BYTES,
        }
    }

    fn sonarr() -> Self {
        Self {
            max_bytes_read: MKV_SONARR_HDR10PLUS_SCAN_MAX_BYTES,
            max_video_blocks: MKV_SONARR_HDR10PLUS_MAX_VIDEO_BLOCKS,
            max_file_scan_bytes: MKV_SONARR_HDR10PLUS_MAX_FILE_SCAN_BYTES,
            block_additional_peek_bytes: MKV_HDR10PLUS_BLOCKADDITIONAL_PEEK_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Hdr10PlusProbeState {
    nal_length_size: usize,
    prefer_block_additional: bool,
    limits: Hdr10PlusProbeLimits,
    bytes_read: u64,
    inspected_blocks: usize,
    found: bool,
}

impl Hdr10PlusProbeState {
    fn new(
        nal_length_size: usize,
        prefer_block_additional: bool,
        limits: Hdr10PlusProbeLimits,
    ) -> Self {
        Self {
            nal_length_size,
            prefer_block_additional,
            limits,
            bytes_read: 0,
            inspected_blocks: 0,
            found: false,
        }
    }

    fn done(&self) -> bool {
        self.found
            || self.inspected_blocks >= self.limits.max_video_blocks
            || self.bytes_read >= self.limits.max_bytes_read
    }
}

#[derive(Debug, Default)]
struct AudioProfileProbeState {
    bytes_read: u64,
    inspected_blocks: usize,
    profile: Option<String>,
    channels: Option<i32>,
    dts_tentative_profile: Option<String>,
    dts_tentative_hits: usize,
    saw_dts_core: bool,
}

impl AudioProfileProbeState {
    fn done(&self, codec_name: &str) -> bool {
        audio_profile_probe_is_terminal(codec_name, self.profile.as_deref())
            || self.inspected_blocks >= audio_profile_max_blocks(codec_name)
            || self.bytes_read >= MKV_AUDIO_PROFILE_SCAN_MAX_BYTES
    }

    fn merge_profile(&mut self, codec_name: &str, candidate: Option<String>) {
        if codec_name != "dts" {
            merge_audio_profile(&mut self.profile, candidate);
            return;
        }

        match candidate.as_deref() {
            Some("DTS-HD MA") | Some("DTS-HD HRA") => {
                self.saw_dts_core = true;
                if self.dts_tentative_profile.as_deref() == candidate.as_deref() {
                    self.dts_tentative_hits += 1;
                } else {
                    self.dts_tentative_profile = candidate.clone();
                    self.dts_tentative_hits = 1;
                }
                if self.dts_tentative_hits >= 2 {
                    merge_audio_profile(&mut self.profile, candidate);
                }
            }
            Some("DTS")
            | Some("DTS-ES")
            | Some("DTS 96/24")
            | Some("DTS Express")
            | Some("DTS-HD MA + DTS:X")
            | Some("DTS-HD MA + DTS:X IMAX") => {
                self.saw_dts_core = true;
                merge_audio_profile(&mut self.profile, candidate);
            }
            Some(_) | None => {}
        }
    }

    fn finalize(self, codec_name: &str) -> ScannedAudioMetadata {
        let profile = if codec_name == "dts"
            && self.profile.as_deref().is_none_or(|profile| {
                matches!(profile, "DTS" | "DTS-ES" | "DTS 96/24" | "DTS Express")
            })
            && self.saw_dts_core
        {
            self.profile.or_else(|| Some("DTS".into()))
        } else {
            self.profile
        };

        ScannedAudioMetadata {
            profile,
            channels: self.channels,
        }
    }
}

fn audio_profile_max_blocks(codec_name: &str) -> usize {
    match codec_name {
        // Some AC-3 files start with stereo frames before the main 5.1 cadence.
        "ac3" => 8,
        // Raw ffprobe stream profiles for MKV E-AC-3 line up with the leading
        // independent syncframe on this corpus. Scanning deeper promotes false
        // positives from later dependent/converted frames.
        "eac3" => MKV_EAC3_AUDIO_PROFILE_MAX_BLOCKS,
        _ => MKV_AUDIO_PROFILE_MAX_BLOCKS,
    }
}

fn audio_profile_probe_is_terminal(codec_name: &str, profile: Option<&str>) -> bool {
    match (codec_name, profile) {
        ("dts", Some("DTS")) => false,
        (_, Some(_)) => true,
        _ => false,
    }
}

#[derive(Debug, Default)]
struct ScannedAudioMetadata {
    profile: Option<String>,
    channels: Option<i32>,
}

#[derive(Debug, Clone, Copy)]
struct MkvFrameRateProbeRequest {
    target_track_num: u64,
    timestamp_scale_ns: f64,
}

#[derive(Debug, Clone, Copy)]
struct MkvHdr10PlusProbeRequest {
    target_track_num: u64,
    nal_length_size: usize,
    prefer_block_additional: bool,
    limits: Hdr10PlusProbeLimits,
}

impl MkvHdr10PlusProbeRequest {
    fn new(
        target_track_num: u64,
        nal_length_size: usize,
        prefer_block_additional: bool,
        limits: Hdr10PlusProbeLimits,
    ) -> Self {
        Self {
            target_track_num,
            nal_length_size,
            prefer_block_additional,
            limits,
        }
    }
}

#[derive(Debug)]
struct MkvAudioProbeRequest {
    target_track_num: u64,
    track_idx: usize,
    codec_name: String,
    header_strip_prefix: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
struct MkvDeepProbeResult {
    frame_rate_fps: Option<f64>,
    has_hdr10plus: Option<bool>,
    audio: Vec<(usize, ScannedAudioMetadata)>,
}

struct MkvFrameRateProbeTarget {
    request: MkvFrameRateProbeRequest,
    state: FrameRateProbeState,
}

struct MkvHdr10PlusProbeTarget {
    target_track_num: u64,
    state: Hdr10PlusProbeState,
}

struct MkvAudioProbeTarget {
    target_track_num: u64,
    track_idx: usize,
    codec_name: String,
    header_strip_prefix: Option<Vec<u8>>,
    state: AudioProfileProbeState,
}

struct MkvDeepProbePlan {
    frame_rate: Option<MkvFrameRateProbeTarget>,
    hdr10plus: Option<MkvHdr10PlusProbeTarget>,
    audio: Vec<MkvAudioProbeTarget>,
}

impl MkvDeepProbePlan {
    fn new(
        frame_rate: Option<MkvFrameRateProbeRequest>,
        hdr10plus: Option<MkvHdr10PlusProbeRequest>,
        audio: Vec<MkvAudioProbeRequest>,
    ) -> Self {
        Self {
            frame_rate: frame_rate.map(|request| MkvFrameRateProbeTarget {
                request,
                state: FrameRateProbeState::default(),
            }),
            hdr10plus: hdr10plus.map(|request| MkvHdr10PlusProbeTarget {
                target_track_num: request.target_track_num,
                state: Hdr10PlusProbeState::new(
                    request.nal_length_size,
                    request.prefer_block_additional,
                    request.limits,
                ),
            }),
            audio: audio
                .into_iter()
                .map(|request| MkvAudioProbeTarget {
                    target_track_num: request.target_track_num,
                    track_idx: request.track_idx,
                    codec_name: request.codec_name,
                    header_strip_prefix: request.header_strip_prefix,
                    state: AudioProfileProbeState::default(),
                })
                .collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.frame_rate.is_none() && self.hdr10plus.is_none() && self.audio.is_empty()
    }

    fn done(&self) -> bool {
        self.frame_rate
            .as_ref()
            .is_none_or(|target| target.state.done())
            && self
                .hdr10plus
                .as_ref()
                .is_none_or(|target| target.state.done())
            && self
                .audio
                .iter()
                .all(|target| target.state.done(&target.codec_name))
    }

    fn max_scan_end(&self, file_len: u64) -> u64 {
        let mut end = 0_u64;
        if self.frame_rate.is_some() {
            end = end.max(MKV_FPS_PROBE_MAX_SCAN_BYTES);
        }
        if let Some(target) = self.hdr10plus.as_ref() {
            end = end.max(target.state.limits.max_file_scan_bytes);
        }
        if !self.audio.is_empty() {
            end = end.max(MKV_AUDIO_PROFILE_MAX_FILE_SCAN_BYTES);
        }
        end.min(file_len)
    }

    fn into_result(self) -> MkvDeepProbeResult {
        let frame_rate_fps = self.frame_rate.as_ref().and_then(|target| {
            estimate_frame_rate_from_timestamps(
                &target.state.timestamps,
                target.request.timestamp_scale_ns,
            )
        });
        let has_hdr10plus = self.hdr10plus.as_ref().map(|target| target.state.found);
        let audio = self
            .audio
            .into_iter()
            .filter_map(|target| {
                let scanned = target.state.finalize(&target.codec_name);
                (scanned.profile.is_some() || scanned.channels.is_some())
                    .then_some((target.track_idx, scanned))
            })
            .collect();

        MkvDeepProbeResult {
            frame_rate_fps,
            has_hdr10plus,
            audio,
        }
    }
}

struct MkvRawScanner<R> {
    reader: R,
    file_len: u64,
    metadata_payload_bytes: u64,
    pos: u64,
}

impl<R: Read + Seek> MkvRawScanner<R> {
    fn new(mut reader: R) -> Result<Self, MediaInfoError> {
        let file_len = reader
            .seek(SeekFrom::End(0))
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        Self::new_with_file_len(reader, file_len)
    }

    fn new_with_file_len(reader: R, file_len: u64) -> Result<Self, MediaInfoError> {
        Ok(Self {
            reader,
            file_len,
            metadata_payload_bytes: 0,
            pos: 0,
        })
    }

    fn read_next_segment_header(&mut self) -> Result<EbmlElementHeader, MediaInfoError> {
        while self.position()? < self.file_len {
            let Some(header) = self.read_element_header()? else {
                break;
            };
            let child_end = header.end(self.file_len, self.file_len);
            if header.id == EBML_ID_SEGMENT {
                return Ok(header);
            }
            self.seek_to(child_end)?;
        }
        Err(MediaInfoError::Parse("missing segment header".into()))
    }

    fn run_deep_probe_plan(&mut self, plan: &mut MkvDeepProbePlan) -> Result<(), MediaInfoError> {
        if plan.is_empty() {
            return Ok(());
        }
        let end = plan.max_scan_end(self.file_len);
        self.seek_to(0)?;
        self.scan_root_for_deep_probe(end, plan, 0)
    }

    fn scan_prefix_for_chapter_count(
        &mut self,
        max_bytes: u64,
    ) -> Result<Option<i32>, MediaInfoError> {
        let end = self.file_len.min(max_bytes);
        self.seek_to(0)?;
        while self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, &[EBML_ID_CHAPTERS])?
            else {
                return Ok(None);
            };
            let child_end = header.end(end, self.file_len);
            if header.id == EBML_ID_CHAPTERS
                && let Some(payload) = self.read_sized_payload(header, child_end)?
            {
                return Ok(parse_mkv_chapters_payload(&payload));
            }
            self.seek_to(child_end)?;
        }
        Ok(None)
    }

    fn scan_prefix_for_audio_channels(
        &mut self,
        codec_name: &str,
        max_bytes: u64,
    ) -> Result<Option<i32>, MediaInfoError> {
        let size = self.file_len.min(max_bytes);
        if size == 0 {
            return Ok(None);
        }
        self.seek_to(0)?;
        let bytes = self.read_bytes(size)?;
        Ok(audio_channels_from_probe_bytes(codec_name, &bytes))
    }

    #[cfg(test)]
    fn probe_frame_rate(
        &mut self,
        target_track_num: u64,
        timestamp_scale_ns: f64,
    ) -> Result<Option<f64>, MediaInfoError> {
        let mut plan = MkvDeepProbePlan::new(
            Some(MkvFrameRateProbeRequest {
                target_track_num,
                timestamp_scale_ns,
            }),
            None,
            Vec::new(),
        );
        self.run_deep_probe_plan(&mut plan)?;
        Ok(plan.into_result().frame_rate_fps)
    }

    fn scan_root_for_deep_probe(
        &mut self,
        end: u64,
        plan: &mut MkvDeepProbePlan,
        depth: usize,
    ) -> Result<(), MediaInfoError> {
        const ROOT_IDS: &[u32] = &[EBML_ID_SEGMENT, EBML_ID_CLUSTER];
        while !plan.done() && self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, ROOT_IDS)? else {
                break;
            };
            let child_end = header.end(end, self.file_len);
            match header.id {
                EBML_ID_SEGMENT => {
                    if let Some(child_depth) = next_mkv_segment_probe_depth(depth) {
                        self.scan_root_for_deep_probe(child_end, plan, child_depth)?;
                    }
                }
                EBML_ID_CLUSTER => {
                    if let Some(child_depth) = next_mkv_deep_probe_depth(depth) {
                        self.scan_cluster_for_deep_probe(child_end, plan, child_depth)?;
                    }
                }
                _ => {}
            }
            self.seek_to(child_end)?;
        }
        Ok(())
    }

    fn scan_cluster_for_deep_probe(
        &mut self,
        end: u64,
        plan: &mut MkvDeepProbePlan,
        depth: usize,
    ) -> Result<(), MediaInfoError> {
        const CLUSTER_IDS: &[u32] = &[EBML_ID_TIMESTAMP, EBML_ID_SIMPLE_BLOCK, EBML_ID_BLOCK_GROUP];
        let mut cluster_timestamp = 0_u64;
        while !plan.done() && self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, CLUSTER_IDS)? else {
                break;
            };
            let child_end = header.end(end, self.file_len);
            match header.id {
                EBML_ID_TIMESTAMP => {
                    if let Some(timestamp) =
                        self.read_unsigned_payload(child_end.saturating_sub(header.data_offset))?
                    {
                        cluster_timestamp = timestamp;
                    }
                }
                EBML_ID_SIMPLE_BLOCK => {
                    if let Some(block) = self.read_block_header(
                        child_end.saturating_sub(header.data_offset),
                        cluster_timestamp,
                    )? {
                        self.inspect_block_for_deep_probe(&block, plan)?;
                    }
                }
                EBML_ID_BLOCK_GROUP => {
                    if let Some(child_depth) = next_mkv_deep_probe_depth(depth) {
                        self.scan_block_group_for_deep_probe(
                            child_end,
                            cluster_timestamp,
                            plan,
                            child_depth,
                        )?;
                    }
                }
                _ => {}
            }
            self.seek_to(child_end)?;
        }
        Ok(())
    }

    fn scan_block_group_for_deep_probe(
        &mut self,
        end: u64,
        cluster_timestamp: u64,
        plan: &mut MkvDeepProbePlan,
        depth: usize,
    ) -> Result<(), MediaInfoError> {
        const BLOCK_GROUP_IDS: &[u32] = &[EBML_ID_BLOCK, EBML_ID_BLOCK_ADDITIONS];
        let mut hdr10plus_target_block_in_group = false;
        while !plan.done() && self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, BLOCK_GROUP_IDS)? else {
                break;
            };
            let child_end = header.end(end, self.file_len);
            match header.id {
                EBML_ID_BLOCK => {
                    if let Some(block) = self.read_block_header(
                        child_end.saturating_sub(header.data_offset),
                        cluster_timestamp,
                    )? {
                        if let Some(target) = plan.hdr10plus.as_ref()
                            && target.state.prefer_block_additional
                            && block.track_number == target.target_track_num
                        {
                            hdr10plus_target_block_in_group = true;
                        }
                        self.inspect_block_for_deep_probe(&block, plan)?;
                    }
                }
                EBML_ID_BLOCK_ADDITIONS if hdr10plus_target_block_in_group => {
                    if let Some(child_depth) = next_mkv_deep_probe_depth(depth)
                        && let Some(target) = plan.hdr10plus.as_mut()
                        && target.state.prefer_block_additional
                    {
                        self.scan_block_additions_for_hdr10plus(
                            child_end,
                            &mut target.state,
                            child_depth,
                        )?;
                    }
                }
                _ => {}
            }
            self.seek_to(child_end)?;
        }
        if hdr10plus_target_block_in_group
            && let Some(target) = plan.hdr10plus.as_mut()
            && target.state.prefer_block_additional
            && target.state.inspected_blocks < target.state.limits.max_video_blocks
        {
            target.state.inspected_blocks += 1;
        }
        Ok(())
    }

    fn inspect_block_for_deep_probe(
        &mut self,
        block: &BlockHeaderInfo,
        plan: &mut MkvDeepProbePlan,
    ) -> Result<(), MediaInfoError> {
        if let Some(target) = plan.frame_rate.as_mut()
            && !target.state.done()
            && block.track_number == target.request.target_track_num
        {
            target.state.record_timestamp(block.timestamp);
        }

        if let Some(target) = plan.hdr10plus.as_mut()
            && !target.state.done()
            && !target.state.prefer_block_additional
            && block.track_number == target.target_track_num
        {
            self.inspect_video_block_for_hdr10plus(block, &mut target.state)?;
        }

        for target in &mut plan.audio {
            if !target.state.done(&target.codec_name)
                && block.track_number == target.target_track_num
            {
                self.inspect_audio_block_for_profile(
                    block,
                    &target.codec_name,
                    target.header_strip_prefix.as_deref(),
                    &mut target.state,
                )?;
            }
        }

        Ok(())
    }

    fn scan_block_additions_for_hdr10plus(
        &mut self,
        end: u64,
        state: &mut Hdr10PlusProbeState,
        depth: usize,
    ) -> Result<(), MediaInfoError> {
        const BLOCK_ADDITION_IDS: &[u32] = &[EBML_ID_BLOCK_MORE];
        while !state.done() && self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, BLOCK_ADDITION_IDS)?
            else {
                break;
            };
            let child_end = header.end(end, self.file_len);
            if header.id == EBML_ID_BLOCK_MORE
                && let Some(child_depth) = next_mkv_deep_probe_depth(depth)
            {
                self.scan_block_more_for_hdr10plus(child_end, state, child_depth)?;
            }
            self.seek_to(child_end)?;
        }
        Ok(())
    }

    fn scan_block_more_for_hdr10plus(
        &mut self,
        end: u64,
        state: &mut Hdr10PlusProbeState,
        _depth: usize,
    ) -> Result<(), MediaInfoError> {
        const BLOCK_MORE_IDS: &[u32] = &[EBML_ID_BLOCK_ADDITIONAL];
        while !state.done() && self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, BLOCK_MORE_IDS)? else {
                break;
            };
            let child_end = header.end(end, self.file_len);
            if header.id == EBML_ID_BLOCK_ADDITIONAL {
                let remaining = state.limits.max_bytes_read.saturating_sub(state.bytes_read);
                let to_read = child_end
                    .saturating_sub(header.data_offset)
                    .min(state.limits.block_additional_peek_bytes)
                    .min(remaining);
                if to_read > 0 {
                    self.seek_to(header.data_offset)?;
                    let payload = self.read_bytes(to_read)?;
                    state.bytes_read += to_read;
                    if scan_itu_t35_payload_for_hdr10plus(&payload) {
                        state.found = true;
                    }
                }
            }
            self.seek_to(child_end)?;
        }
        Ok(())
    }

    fn inspect_video_block_for_hdr10plus(
        &mut self,
        block: &BlockHeaderInfo,
        state: &mut Hdr10PlusProbeState,
    ) -> Result<(), MediaInfoError> {
        if state.done() || block.lacing_type != 0 || block.payload_size == 0 {
            return Ok(());
        }
        let remaining = state.limits.max_bytes_read.saturating_sub(state.bytes_read);
        let to_read = block.payload_size.min(remaining);
        if to_read == 0 {
            return Ok(());
        }
        self.seek_to(block.payload_offset)?;
        let payload = self.read_bytes(to_read)?;
        state.bytes_read += to_read;
        state.inspected_blocks += 1;
        if scan_hevc_frame_for_hdr10plus(&payload, state.nal_length_size) {
            state.found = true;
        }
        Ok(())
    }

    fn inspect_audio_block_for_profile(
        &mut self,
        block: &BlockHeaderInfo,
        codec_name: &str,
        header_strip_prefix: Option<&[u8]>,
        state: &mut AudioProfileProbeState,
    ) -> Result<(), MediaInfoError> {
        if state.done(codec_name) || block.payload_size == 0 {
            return Ok(());
        }

        let probe_spec = audio_profile_probe_spec(Some(codec_name));
        let prefix_probe_bytes = audio_header_probe_bytes(codec_name).max(probe_spec.prefix_bytes);
        if prefix_probe_bytes == 0 {
            return Ok(());
        }

        self.seek_to(block.payload_offset)?;
        let Some((frame_header_size, frame_size)) =
            parse_first_laced_frame_from_reader(self, block.payload_size, block.lacing_type)?
        else {
            return Ok(());
        };
        let frame_offset = block.payload_offset.saturating_add(frame_header_size);
        let probe_payload_size = frame_size;
        let remaining = MKV_AUDIO_PROFILE_SCAN_MAX_BYTES.saturating_sub(state.bytes_read);
        let header_strip_len = header_strip_prefix.map_or(0, <[u8]>::len);
        let file_prefix_size = probe_payload_size
            .min(prefix_probe_bytes.saturating_sub(header_strip_len) as u64)
            .min(remaining);
        if file_prefix_size == 0 && header_strip_len == 0 {
            return Ok(());
        }

        self.seek_to(frame_offset)?;
        let mut prefix =
            Vec::with_capacity(header_strip_len.saturating_add(file_prefix_size as usize));
        if let Some(header_strip_prefix) = header_strip_prefix {
            prefix.extend_from_slice(header_strip_prefix);
        }
        if file_prefix_size > 0 {
            prefix.extend_from_slice(&self.read_bytes(file_prefix_size)?);
        }
        state.bytes_read += file_prefix_size;

        let suffix = if probe_spec.suffix_bytes > 0 {
            let remaining = MKV_AUDIO_PROFILE_SCAN_MAX_BYTES.saturating_sub(state.bytes_read);
            let suffix_size = probe_payload_size
                .min(probe_spec.suffix_bytes as u64)
                .min(remaining);
            let suffix_offset =
                frame_offset.saturating_add(probe_payload_size.saturating_sub(suffix_size));
            if suffix_size > 0 && suffix_offset >= frame_offset + file_prefix_size {
                self.seek_to(suffix_offset)?;
                let suffix = self.read_bytes(suffix_size)?;
                state.bytes_read += suffix_size;
                Some(suffix)
            } else {
                None
            }
        } else {
            None
        };

        state.inspected_blocks += 1;
        state.merge_profile(
            codec_name,
            detect_audio_profile_from_probe_bytes(Some(codec_name), &prefix, suffix.as_deref()),
        );
        let parsed_channels = audio_channels_from_probe_bytes(codec_name, &prefix);
        if parsed_channels.is_some_and(|channels| channels > state.channels.unwrap_or(0)) {
            state.channels = parsed_channels;
        }
        Ok(())
    }

    fn read_deep_probe_candidate_header(
        &mut self,
        end: u64,
        candidate_ids: &[u32],
    ) -> Result<Option<EbmlElementHeader>, MediaInfoError> {
        while self.position()? < end {
            let Some(candidate_pos) = self.find_next_deep_probe_candidate(end, candidate_ids)?
            else {
                return Ok(None);
            };
            self.seek_to(candidate_pos)?;
            let Some(header) = self.read_element_header()? else {
                return Ok(None);
            };
            let child_end = header.end(end, self.file_len);
            if candidate_ids.contains(&header.id)
                && header.data_offset <= child_end
                && child_end <= end.min(self.file_len)
            {
                return Ok(Some(header));
            }
            self.seek_to(candidate_pos.saturating_add(1))?;
        }

        Ok(None)
    }

    fn find_next_deep_probe_candidate(
        &mut self,
        end: u64,
        candidate_ids: &[u32],
    ) -> Result<Option<u64>, MediaInfoError> {
        let mut pos = self.position()?;
        let mut buf = vec![0_u8; MKV_DEEP_PROBE_CANDIDATE_READ_BYTES];
        while pos < end {
            let read_len = usize::try_from(
                end.saturating_sub(pos)
                    .min(MKV_DEEP_PROBE_CANDIDATE_READ_BYTES as u64),
            )
            .unwrap_or(MKV_DEEP_PROBE_CANDIDATE_READ_BYTES);
            if read_len == 0 {
                return Ok(None);
            }
            self.seek_to(pos)?;
            let mut bytes_read = 0;
            while bytes_read < read_len {
                let read = self
                    .read(&mut buf[bytes_read..read_len])
                    .map_err(|e| MediaInfoError::Io(e.to_string()))?;
                if read == 0 {
                    break;
                }
                bytes_read += read;
            }
            if bytes_read == 0 {
                return Ok(None);
            }
            if let Some(candidate) = scan::find_ebml_candidate(&buf[..bytes_read], 0, candidate_ids)
            {
                return Ok(Some(pos + candidate.offset as u64));
            }
            if bytes_read < read_len || read_len < buf.len() {
                return Ok(None);
            }
            pos = pos.saturating_add((read_len.saturating_sub(3)) as u64);
        }

        Ok(None)
    }

    fn read_element_header(&mut self) -> Result<Option<EbmlElementHeader>, MediaInfoError> {
        let Some((id, _)) = read_ebml_id_from_reader(self)? else {
            return Ok(None);
        };
        let (size, _) = read_ebml_size_from_reader(self)?;
        let data_offset = self.position()?;
        Ok(Some(EbmlElementHeader {
            id,
            data_offset,
            size,
        }))
    }

    fn read_sized_payload(
        &mut self,
        header: EbmlElementHeader,
        parent_end: u64,
    ) -> Result<Option<Vec<u8>>, MediaInfoError> {
        let child_end = header.end(parent_end, self.file_len);
        let Some(size) = header.size else {
            self.seek_to(child_end)?;
            return Ok(None);
        };
        let declared_end = header
            .data_offset
            .checked_add(size)
            .ok_or_else(|| MediaInfoError::Parse("MKV element size overflow".into()))?;
        if declared_end > parent_end || declared_end > self.file_len {
            return Err(MediaInfoError::Parse(format!(
                "MKV element 0x{:X} declares {} bytes beyond remaining input",
                header.id, size
            )));
        }
        if size > MKV_KEEP_ELEMENT_MAX_BYTES {
            return Err(MediaInfoError::Parse(format!(
                "MKV element 0x{:X} exceeds parser budget",
                header.id
            )));
        }
        self.metadata_payload_bytes = self
            .metadata_payload_bytes
            .checked_add(size)
            .ok_or_else(|| MediaInfoError::Parse("MKV metadata budget overflow".into()))?;
        if self.metadata_payload_bytes > MKV_METADATA_AGGREGATE_MAX_BYTES {
            return Err(MediaInfoError::Parse(
                "MKV metadata output exceeds parser budget".into(),
            ));
        }
        self.seek_to(header.data_offset)?;
        let payload = self.read_bytes(size)?;
        self.seek_to(child_end)?;
        Ok(Some(payload))
    }

    fn read_top_level_payload_at(
        &mut self,
        offset: u64,
        expected_id: u32,
    ) -> Result<Option<Vec<u8>>, MediaInfoError> {
        self.seek_to(offset)?;
        let Some(header) = self.read_element_header()? else {
            return Ok(None);
        };
        if header.id != expected_id {
            return Ok(None);
        }
        self.read_sized_payload(header, self.file_len)
    }

    fn read_block_header(
        &mut self,
        data_size: u64,
        cluster_timestamp: u64,
    ) -> Result<Option<BlockHeaderInfo>, MediaInfoError> {
        if data_size < 4 {
            return Ok(None);
        }

        let start = self.position()?;
        let peek_len = data_size.min(11) as usize;
        let mut buf = [0_u8; 11];
        self.read_exact(&mut buf[..peek_len])
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;

        let Some((track_number, track_len)) = parse_ebml_vint_value(&buf[..peek_len]) else {
            return Ok(None);
        };
        if data_size < track_len as u64 + 3 || peek_len < track_len + 3 {
            return Ok(None);
        }

        let relative_timestamp = i16::from_be_bytes([buf[track_len], buf[track_len + 1]]) as i64;
        let timestamp = cluster_timestamp as i64 + relative_timestamp;
        if timestamp < 0 {
            return Ok(None);
        }

        let flags = buf[track_len + 2];
        let payload_offset = start + track_len as u64 + 3;
        let payload_size = data_size - track_len as u64 - 3;
        self.seek_to(payload_offset)?;

        Ok(Some(BlockHeaderInfo {
            track_number,
            timestamp: timestamp as u64,
            payload_offset,
            payload_size,
            lacing_type: (flags & 0x06) >> 1,
        }))
    }

    fn read_unsigned_payload(&mut self, size: u64) -> Result<Option<u64>, MediaInfoError> {
        if size == 0 || size > 8 {
            return Ok(None);
        }
        let mut buf = [0_u8; 8];
        let size = size as usize;
        self.read_exact(&mut buf[..size])
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        Ok(parse_ebml_uint(&buf[..size]))
    }

    fn read_bytes(&mut self, size: u64) -> Result<Vec<u8>, MediaInfoError> {
        let mut buf = vec![0_u8; size as usize];
        self.read_exact(&mut buf)
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        Ok(buf)
    }

    fn position(&mut self) -> Result<u64, MediaInfoError> {
        Ok(self.pos)
    }

    fn seek_to(&mut self, pos: u64) -> Result<(), MediaInfoError> {
        if self.pos == pos {
            return Ok(());
        }
        self.reader
            .seek(SeekFrom::Start(pos))
            .map(|new_pos| {
                self.pos = new_pos;
            })
            .map_err(|e| MediaInfoError::Io(e.to_string()))
    }
}

impl<R: Read> Read for MkvRawScanner<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let bytes_read = self.reader.read(buf)?;
        self.pos = self.pos.saturating_add(bytes_read as u64);
        Ok(bytes_read)
    }
}

fn track_entry_signals(track_entry_payload: &[u8]) -> MkvTrackSignals {
    let mut current = track_entry_payload;
    let mut signals = MkvTrackSignals::default();
    while !current.is_empty() {
        let Some((id, payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        if id == EBML_ID_BLOCK_ADDITION_MAPPING {
            if block_addition_mapping_has_type(payload, MATROSKA_BLOCK_ADD_ID_TYPE_ITU_T_T35) {
                signals.has_itu_t_t35_mapping = true;
            }
            if signals.dovi_config.is_none() {
                signals.dovi_config =
                    block_addition_mapping_extra_data(payload, DOVI_BLOCK_ADD_ID_TYPE);
            }
        } else if id == EBML_ID_TRACK_CONTENT_ENCODINGS && signals.header_strip_prefix.is_none() {
            signals.header_strip_prefix = track_header_strip_prefix(payload);
        }
        current = &current[consumed..];
    }
    signals
}

fn track_header_strip_prefix(track_content_encodings_payload: &[u8]) -> Option<Vec<u8>> {
    let mut current = track_content_encodings_payload;
    while !current.is_empty() {
        let Some((id, payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        if id == EBML_ID_TRACK_CONTENT_ENCODING
            && let Some(prefix) = track_content_encoding_header_strip_prefix(payload)
        {
            return Some(prefix);
        }
        current = &current[consumed..];
    }
    None
}

fn track_content_encoding_header_strip_prefix(
    track_content_encoding_payload: &[u8],
) -> Option<Vec<u8>> {
    let mut scope = MATROSKA_TRACK_ENCODING_SCOPE_FRAME_CONTENTS;
    let mut encoding_type = MATROSKA_TRACK_ENCODING_TYPE_COMPRESSION;
    let mut compression_algo = None;
    let mut compression_settings = None;

    let mut current = track_content_encoding_payload;
    while !current.is_empty() {
        let Some((id, payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        match id {
            EBML_ID_ENCODING_SCOPE => scope = parse_ebml_uint(payload)?,
            EBML_ID_ENCODING_TYPE => encoding_type = parse_ebml_uint(payload)?,
            EBML_ID_ENCODING_COMPRESSION => {
                let (algo, settings) = track_content_compression(payload)?;
                compression_algo = Some(algo);
                compression_settings = settings;
            }
            EBML_ID_ENCODING_ORDER => {
                let _ = parse_ebml_uint(payload)?;
            }
            _ => {}
        }
        current = &current[consumed..];
    }

    (encoding_type == MATROSKA_TRACK_ENCODING_TYPE_COMPRESSION
        && (scope & MATROSKA_TRACK_ENCODING_SCOPE_FRAME_CONTENTS) != 0
        && compression_algo == Some(MATROSKA_TRACK_ENCODING_COMP_HEADERSTRIP))
    .then_some(compression_settings)
    .flatten()
    .filter(|settings| !settings.is_empty())
}

fn track_content_compression(
    track_content_compression_payload: &[u8],
) -> Option<(u64, Option<Vec<u8>>)> {
    let mut algorithm = None;
    let mut settings = None;

    let mut current = track_content_compression_payload;
    while !current.is_empty() {
        let Some((id, payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        match id {
            EBML_ID_ENCODING_COMP_ALGO => algorithm = parse_ebml_uint(payload),
            EBML_ID_ENCODING_COMP_SETTINGS => settings = Some(payload.to_vec()),
            _ => {}
        }
        current = &current[consumed..];
    }

    Some((algorithm?, settings))
}

fn block_addition_mapping_has_type(payload: &[u8], target_type: u64) -> bool {
    block_addition_mapping_type(payload) == Some(target_type)
}

fn block_addition_mapping_extra_data(payload: &[u8], target_type: u64) -> Option<Vec<u8>> {
    (block_addition_mapping_type(payload) == Some(target_type))
        .then(|| find_first_direct_ebml_child(payload, EBML_ID_BLOCK_ADD_ID_EXTRA_DATA))
        .flatten()
        .map(|payload| payload.to_vec())
}

fn block_addition_mapping_type(payload: &[u8]) -> Option<u64> {
    let mut current = payload;
    while !current.is_empty() {
        let Some((id, child_payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        if id == EBML_ID_BLOCK_ADD_ID_TYPE {
            return parse_ebml_uint(child_payload);
        }
        current = &current[consumed..];
    }
    None
}

fn should_confirm_mkv_hdr10plus(track: &RawTrack, signals: &MkvTrackSignals) -> bool {
    if track.codec_name.as_deref() != Some("hevc")
        || track.dovi_config.is_some()
        || track.color_transfer == Some(18)
    {
        return false;
    }

    if signals.has_itu_t_t35_mapping || track.color_transfer == Some(16) {
        return true;
    }

    track
        .codec_private
        .as_deref()
        .map(extract_h265_info)
        .and_then(|info| info.bit_depth)
        .is_some_and(|bit_depth| bit_depth >= 10)
}

fn should_probe_sonarr_hdr10plus(track: &RawTrack) -> bool {
    track.codec_name.as_deref() == Some("hevc")
        && track.dovi_config.is_none()
        && track.color_transfer == Some(16)
}

fn probe_mkv_deep_metadata<R: Read + Seek>(
    scanner: &mut MkvRawScanner<R>,
    frame_rate: Option<MkvFrameRateProbeRequest>,
    hdr10plus: Option<MkvHdr10PlusProbeRequest>,
    audio: Vec<MkvAudioProbeRequest>,
) -> MkvDeepProbeResult {
    let mut plan = MkvDeepProbePlan::new(frame_rate, hdr10plus, audio);
    if plan.is_empty() {
        return MkvDeepProbeResult::default();
    }

    if scanner.run_deep_probe_plan(&mut plan).is_err() {
        return MkvDeepProbeResult::default();
    }
    plan.into_result()
}

fn audio_header_probe_bytes(codec_name: &str) -> usize {
    match codec_name {
        "ac3" => 7,
        "eac3" => 6,
        "dts" => 32,
        _ => 0,
    }
}

fn estimate_frame_rate_from_timestamps(timestamps: &[u64], timestamp_scale_ns: f64) -> Option<f64> {
    if timestamps.len() < 4 {
        return None;
    }

    let mut deltas: Vec<u64> = timestamps
        .windows(2)
        .filter_map(|window| window[1].checked_sub(window[0]))
        .filter(|delta| *delta > 0)
        .collect();
    if deltas.is_empty() {
        return None;
    }

    deltas.sort_unstable();
    let median_delta = deltas[deltas.len() / 2] as f64;
    let delta_seconds = median_delta * timestamp_scale_ns / 1e9;
    if delta_seconds <= 0.0 {
        return None;
    }

    let fps = 1.0 / delta_seconds;
    if (1.0..=240.0).contains(&fps) {
        Some(fps)
    } else {
        None
    }
}

fn parse_ebml_vint_value(data: &[u8]) -> Option<(u64, usize)> {
    if data.is_empty() {
        return None;
    }
    let first = data[0];
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len == 0 || len > 8 || len > data.len() {
        return None;
    }

    let value_mask = if len == 8 { 0 } else { 0xFF >> len };
    let mut value = u64::from(first & value_mask);
    for &byte in &data[1..len] {
        value = (value << 8) | u64::from(byte);
    }
    Some((value, len))
}

fn parse_ebml_sint_value(data: &[u8]) -> Option<(i64, usize)> {
    let (unsigned, len) = parse_ebml_vint_value(data)?;
    let value_bits = len.checked_mul(7)?;
    let bias = (1_i64.checked_shl((value_bits.checked_sub(1)?) as u32)?).checked_sub(1)?;
    let unsigned = i64::try_from(unsigned).ok()?;
    Some((unsigned - bias, len))
}

fn read_u8_from_reader<R: Read>(reader: &mut R) -> Result<u8, MediaInfoError> {
    let mut buf = [0_u8; 1];
    reader
        .read_exact(&mut buf)
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;
    Ok(buf[0])
}

fn read_ebml_vint_value_from_reader<R: Read>(
    reader: &mut R,
) -> Result<(u64, usize), MediaInfoError> {
    let first = read_u8_from_reader(reader)?;
    if first == 0 {
        return Err(MediaInfoError::Parse("invalid ebml vint".into()));
    }

    let len = first.leading_zeros() as usize + 1;
    if len == 0 || len > 8 {
        return Err(MediaInfoError::Parse("invalid ebml vint".into()));
    }

    let mut buf = [0_u8; 8];
    buf[0] = first;
    if len > 1 {
        reader
            .read_exact(&mut buf[1..len])
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
    }

    parse_ebml_vint_value(&buf[..len])
        .ok_or_else(|| MediaInfoError::Parse("invalid ebml vint".into()))
}

fn read_ebml_sint_value_from_reader<R: Read>(
    reader: &mut R,
) -> Result<(i64, usize), MediaInfoError> {
    let first = read_u8_from_reader(reader)?;
    if first == 0 {
        return Err(MediaInfoError::Parse("invalid ebml signed vint".into()));
    }

    let len = first.leading_zeros() as usize + 1;
    if len == 0 || len > 8 {
        return Err(MediaInfoError::Parse("invalid ebml signed vint".into()));
    }

    let mut buf = [0_u8; 8];
    buf[0] = first;
    if len > 1 {
        reader
            .read_exact(&mut buf[1..len])
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
    }

    parse_ebml_sint_value(&buf[..len])
        .ok_or_else(|| MediaInfoError::Parse("invalid ebml signed vint".into()))
}

fn parse_first_laced_frame_from_reader<R: Read>(
    reader: &mut R,
    payload_size: u64,
    lacing_type: u8,
) -> Result<Option<(u64, u64)>, MediaInfoError> {
    if payload_size == 0 {
        return Ok(None);
    }
    if lacing_type == 0 {
        return Ok(Some((0, payload_size)));
    }
    if payload_size < 1 {
        return Ok(None);
    }

    let lace_count = u64::from(read_u8_from_reader(reader)?) + 1;
    let mut header_size = 1_u64;
    let mut remaining_payload = payload_size.saturating_sub(1);

    if lace_count <= 1 {
        return Ok(Some((header_size, remaining_payload)));
    }

    match lacing_type {
        1 => {
            let mut total_sizes = 0_u64;
            let mut first_frame_size = None;

            for index in 0..lace_count - 1 {
                let mut size = 0_u64;
                loop {
                    if remaining_payload == 0 {
                        return Ok(None);
                    }
                    let byte = u64::from(read_u8_from_reader(reader)?);
                    header_size += 1;
                    remaining_payload -= 1;
                    let Some(next_size) = size.checked_add(byte) else {
                        return Ok(None);
                    };
                    size = next_size;
                    let Some(next_total) = total_sizes.checked_add(byte) else {
                        return Ok(None);
                    };
                    total_sizes = next_total;
                    if byte != 0xFF {
                        break;
                    }
                }
                if index == 0 {
                    first_frame_size = Some(size);
                }
            }

            if remaining_payload < total_sizes {
                return Ok(None);
            }

            Ok(Some((
                header_size,
                first_frame_size.unwrap_or(remaining_payload),
            )))
        }
        2 => {
            if !remaining_payload.is_multiple_of(lace_count) {
                return Ok(None);
            }
            Ok(Some((header_size, remaining_payload / lace_count)))
        }
        3 => {
            let (first_frame_size, first_size_len) = read_ebml_vint_value_from_reader(reader)?;
            header_size += first_size_len as u64;
            let Some(next_remaining) = remaining_payload.checked_sub(first_size_len as u64) else {
                return Ok(None);
            };
            remaining_payload = next_remaining;

            let mut total_sizes = first_frame_size;
            let Ok(mut previous_size) = i64::try_from(first_frame_size) else {
                return Ok(None);
            };

            for _ in 1..lace_count - 1 {
                let (delta, delta_len) = read_ebml_sint_value_from_reader(reader)?;
                header_size += delta_len as u64;
                let Some(next_remaining) = remaining_payload.checked_sub(delta_len as u64) else {
                    return Ok(None);
                };
                remaining_payload = next_remaining;

                let Some(next_size) = previous_size.checked_add(delta) else {
                    return Ok(None);
                };
                previous_size = next_size;
                if previous_size < 0 {
                    return Ok(None);
                }
                let Some(next_total) = total_sizes.checked_add(previous_size as u64) else {
                    return Ok(None);
                };
                total_sizes = next_total;
            }

            if remaining_payload < total_sizes {
                return Ok(None);
            }

            Ok(Some((header_size, first_frame_size)))
        }
        _ => Ok(None),
    }
}

fn read_ebml_id_from_reader<R: Read>(
    reader: &mut R,
) -> Result<Option<(u32, usize)>, MediaInfoError> {
    let mut first = [0_u8; 1];
    let bytes_read = reader
        .read(&mut first)
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;
    if bytes_read == 0 {
        return Ok(None);
    }

    let len = first[0].leading_zeros() as usize + 1;
    if len == 0 || len > 4 {
        return Err(MediaInfoError::Parse("invalid ebml id".into()));
    }

    let mut value = u32::from(first[0]);
    let mut rest = [0_u8; 3];
    if len > 1 {
        reader
            .read_exact(&mut rest[..len - 1])
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        for &byte in &rest[..len - 1] {
            value = (value << 8) | u32::from(byte);
        }
    }

    Ok(Some((value, len)))
}

fn read_ebml_size_from_reader<R: Read>(
    reader: &mut R,
) -> Result<(Option<u64>, usize), MediaInfoError> {
    let mut first = [0_u8; 1];
    reader
        .read_exact(&mut first)
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    if first[0] == 0 {
        return Err(MediaInfoError::Parse("invalid ebml size".into()));
    }

    let len = first[0].leading_zeros() as usize + 1;
    if len > 8 {
        return Err(MediaInfoError::Parse("invalid ebml size".into()));
    }

    let mask = if len == 8 { 0 } else { 0xFF >> len };
    let mut value = u64::from(first[0] & mask);
    let mut unknown = value == u64::from(mask);

    let mut rest = [0_u8; 7];
    if len > 1 {
        reader
            .read_exact(&mut rest[..len - 1])
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        for &byte in &rest[..len - 1] {
            value = (value << 8) | u64::from(byte);
            unknown &= byte == 0xFF;
        }
    }

    Ok(((!unknown).then_some(value), len))
}

fn is_plausible_frame_rate(frame_rate_fps: Option<f64>) -> bool {
    frame_rate_fps.is_some_and(|fps| (1.0..=240.0).contains(&fps))
}

fn fallback_frame_rate_from_timestamp_scale(timestamp_scale_ns: f64) -> Option<f64> {
    if timestamp_scale_ns <= 0.0 {
        return None;
    }

    let fps = 1e9 / timestamp_scale_ns;
    (1.0..=1000.0).contains(&fps).then_some(fps)
}

fn should_replace_frame_rate(existing: Option<f64>, observed: f64) -> bool {
    match existing {
        None => true,
        Some(current) if current <= 0.0 => true,
        Some(current) => current < 10.0 && observed >= current * 10.0,
    }
}

/// Parse an EBML variable-size integer (VINT). Returns (value, bytes_consumed).
fn parse_ebml_vint(data: &[u8]) -> Option<(usize, usize)> {
    if data.is_empty() {
        return None;
    }
    let first = data[0];
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || len > data.len() {
        return None;
    }
    let value_mask = if len == 8 { 0 } else { 0xFF >> len };
    let mut value = (first & value_mask) as usize;
    for &b in &data[1..len] {
        value = (value << 8) | b as usize;
    }
    Some((value, len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MediaInfoError;
    use std::io::Cursor;

    fn make_ebml_size(size: u64) -> Vec<u8> {
        for len in 1..=8 {
            let value_bits = len * 7;
            let unknown_size = 1_u64 << value_bits;
            if size < unknown_size - 1 {
                let mut bytes = vec![0_u8; len];
                let mut value = size;
                for byte in bytes.iter_mut().rev() {
                    *byte = (value & 0xFF) as u8;
                    value >>= 8;
                }
                bytes[0] |= 1 << (8 - len);
                return bytes;
            }
        }

        panic!("EBML test size is too large");
    }

    fn make_ebml_element(id: &[u8], payload: &[u8]) -> Vec<u8> {
        make_ebml_element_with_declared_size(id, payload.len() as u64, payload)
    }

    fn make_ebml_element_with_declared_size(
        id: &[u8],
        declared_size: u64,
        payload_prefix: &[u8],
    ) -> Vec<u8> {
        let size = make_ebml_size(declared_size);
        let mut element = Vec::with_capacity(id.len() + size.len() + payload_prefix.len());
        element.extend_from_slice(id);
        element.extend_from_slice(&size);
        element.extend_from_slice(payload_prefix);
        element
    }

    fn make_chapter_atom(start: u64, nested: &[u8]) -> Vec<u8> {
        let mut payload = make_ebml_element(&[0x73, 0xC4], &[1]);
        payload.extend_from_slice(&make_ebml_element(&[0x91], &start.to_be_bytes()));
        payload.extend_from_slice(nested);
        make_ebml_element(&[0xB6], &payload)
    }

    fn make_uint_element(id: &[u8], value: u64) -> Vec<u8> {
        let bytes = if value <= u64::from(u8::MAX) {
            vec![value as u8]
        } else {
            value
                .to_be_bytes()
                .into_iter()
                .skip_while(|byte| *byte == 0)
                .collect()
        };
        make_ebml_element(id, &bytes)
    }

    fn make_simple_block(
        track_num: u8,
        relative_timestamp: i16,
        flags: u8,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut block = Vec::with_capacity(4 + payload.len());
        block.push(0x80 | track_num);
        block.extend_from_slice(&relative_timestamp.to_be_bytes());
        block.push(flags);
        block.extend_from_slice(payload);
        make_ebml_element(&[0xA3], &block)
    }

    fn audio_test_track(codec_name: &str, channels: Option<i32>) -> RawTrack {
        RawTrack {
            kind: TrackKind::Audio,
            codec_id: format!("A_{}", codec_name.to_ascii_uppercase()),
            codec_name: Some(codec_name.to_owned()),
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

    fn make_frame_rate_probe_cluster() -> Vec<u8> {
        make_ebml_element(
            &[0x1F, 0x43, 0xB6, 0x75],
            &[
                make_uint_element(&[0xE7], 0),
                make_simple_block(1, 0, 0, &[0x00]),
                make_simple_block(1, 40, 0, &[0x01]),
                make_simple_block(1, 80, 0, &[0x02]),
                make_simple_block(1, 120, 0, &[0x03]),
                make_simple_block(1, 160, 0, &[0x04]),
            ]
            .concat(),
        )
    }

    fn wrap_in_segments(mut payload: Vec<u8>, count: usize) -> Vec<u8> {
        for _ in 0..count {
            payload = make_ebml_element(&[0x18, 0x53, 0x80, 0x67], &payload);
        }
        payload
    }

    fn make_mkv_with_declared_info_size(declared_size: u64) -> Vec<u8> {
        let info =
            make_ebml_element_with_declared_size(&[0x15, 0x49, 0xA9, 0x66], declared_size, &[]);
        let segment_declared_size = declared_size + info.len() as u64;
        [
            make_ebml_element(&[0x1A, 0x45, 0xDF, 0xA3], &[]),
            make_ebml_element_with_declared_size(
                &[0x18, 0x53, 0x80, 0x67],
                segment_declared_size,
                &info,
            ),
        ]
        .concat()
    }

    fn unique_temp_mkv_path(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("scryer-{name}-{}-{nanos}.mkv", std::process::id()))
    }

    #[test]
    fn ebml_vint_parsing() {
        // 0x81 = 1 byte VINT, value 1
        assert_eq!(parse_ebml_vint(&[0x81]), Some((1, 1)));
        // 0x82 = 1 byte VINT, value 2
        assert_eq!(parse_ebml_vint(&[0x82]), Some((2, 1)));
        // 0x85 = 1 byte VINT, value 5
        assert_eq!(parse_ebml_vint(&[0x85]), Some((5, 1)));
        // 0x40 0x18 = 2 byte VINT, value 24
        assert_eq!(parse_ebml_vint(&[0x40, 0x18]), Some((24, 2)));
    }

    #[test]
    fn ebml_signed_vint_parsing_matches_matroska_lace_deltas() {
        assert_eq!(parse_ebml_sint_value(&[0xBE]), Some((-1, 1)));
        assert_eq!(parse_ebml_sint_value(&[0xBF]), Some((0, 1)));
        assert_eq!(parse_ebml_sint_value(&[0xC0]), Some((1, 1)));
    }

    #[test]
    fn first_laced_frame_parser_handles_non_laced_payloads() {
        let mut cursor = Cursor::new(vec![1, 2, 3, 4]);
        assert_eq!(
            parse_first_laced_frame_from_reader(&mut cursor, 4, 0).unwrap(),
            Some((0, 4))
        );
    }

    #[test]
    fn first_laced_frame_parser_handles_xiph_lacing() {
        let mut payload = vec![0x02, 0x04, 0x05];
        payload.extend_from_slice(&[0_u8; 15]);
        let mut cursor = Cursor::new(payload);
        assert_eq!(
            parse_first_laced_frame_from_reader(&mut cursor, 18, 1).unwrap(),
            Some((3, 4))
        );
    }

    #[test]
    fn first_laced_frame_parser_handles_fixed_lacing() {
        let mut payload = vec![0x02];
        payload.extend_from_slice(&[0_u8; 15]);
        let mut cursor = Cursor::new(payload);
        assert_eq!(
            parse_first_laced_frame_from_reader(&mut cursor, 16, 2).unwrap(),
            Some((1, 5))
        );
    }

    #[test]
    fn first_laced_frame_parser_handles_ebml_lacing() {
        let mut payload = vec![0x02, 0x84, 0xC0];
        payload.extend_from_slice(&[0_u8; 15]);
        let mut cursor = Cursor::new(payload);
        assert_eq!(
            parse_first_laced_frame_from_reader(&mut cursor, 18, 3).unwrap(),
            Some((3, 4))
        );
    }

    #[test]
    fn normalize_track_language_preserves_scryer_mkv_policy() {
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Video, None, None),
            None
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Audio, Some("en-US"), None),
            Some("eng".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Subtitle, None, Some("en-US")),
            Some("en-US".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Subtitle, Some("pt-BR"), Some("por")),
            Some("por".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Subtitle, Some("fil"), Some("fil")),
            Some("fil".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Subtitle, Some("jad"), Some("und")),
            None
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Subtitle, None, Some("zxx")),
            Some("zxx".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Audio, None, Some("???")),
            Some("???".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Audio, Some("ja-JP"), Some("eng")),
            Some("eng".to_string())
        );
        assert_eq!(
            normalize_mkv_track_language(TrackKind::Audio, None, None),
            Some("eng".to_string())
        );
    }

    #[test]
    fn matroska_webvtt_track_keeps_subtitle_stream_without_codec_name() {
        let track_entry = [
            make_uint_element(&[0xD7], 1),
            make_uint_element(&[0x83], 17),
            make_ebml_element(&[0x86], b"S_TEXT/WEBVTT"),
            make_ebml_element(&[0x22, 0xB5, 0x9C], b"eng"),
        ]
        .concat();

        let track = parse_mkv_track_entry(&track_entry).unwrap();
        assert_eq!(track.raw.kind, TrackKind::Subtitle);
        assert_eq!(track.raw.codec_id, "S_TEXT/WEBVTT");
        assert_eq!(track.raw.codec_name, None);
        assert_eq!(track.raw.language.as_deref(), Some("eng"));
    }

    #[test]
    fn scanned_audio_channels_replace_container_channels() {
        assert_eq!(merge_scanned_audio_channels(Some(2), Some(6)), Some(6));
        assert_eq!(merge_scanned_audio_channels(Some(6), Some(2)), Some(2));
        assert_eq!(merge_scanned_audio_channels(Some(2), None), Some(2));
    }

    #[test]
    fn unique_ac3_prefix_channel_fallback_fills_missing_channels() {
        let mut scanner = MkvRawScanner::new(Cursor::new(vec![
            0x1A, 0x45, 0xDF, 0xA3, 0x0B, 0x77, 0x0A, 0xA2, 0x1C, 0x30, 0x43, 0x0B, 0x77, 0x9A,
            0xE2, 0x1C, 0x30, 0xE1,
        ]))
        .unwrap();
        let mut tracks = vec![audio_test_track("ac3", None)];

        apply_unique_audio_prefix_channel_fallback(&mut scanner, &mut tracks);

        assert_eq!(tracks[0].channels, Some(6));
    }

    #[test]
    fn unique_audio_prefix_channel_fallback_skips_same_codec_multi_audio() {
        let mut scanner =
            MkvRawScanner::new(Cursor::new(vec![0x0B, 0x77, 0x0A, 0xA2, 0x1C, 0x30, 0x43]))
                .unwrap();
        let mut tracks = vec![
            audio_test_track("ac3", Some(2)),
            audio_test_track("ac3", Some(2)),
        ];

        apply_unique_audio_prefix_channel_fallback(&mut scanner, &mut tracks);

        assert_eq!(tracks[0].channels, Some(2));
        assert_eq!(tracks[1].channels, Some(2));
    }

    #[test]
    fn parses_flac_channels_from_matroska_codec_private() {
        let mut codec_private = Vec::new();
        codec_private.extend_from_slice(b"fLaC");
        codec_private.extend_from_slice(&[0x00, 0x00, 0x00, 0x22]);
        let mut streaminfo = [0_u8; 34];
        streaminfo[12] = 0;
        streaminfo[13] = 0;
        codec_private.extend_from_slice(&streaminfo);

        assert_eq!(
            parse_mkv_flac_codec_private_channels(&codec_private),
            Some(1)
        );
    }

    #[test]
    fn count_ffprobe_style_chapter_starts_matches_ffmpeg_guard() {
        assert_eq!(
            count_ffprobe_style_chapter_starts([0, 90, 1320, 6, 51, 1429]),
            4
        );
        assert_eq!(
            count_ffprobe_style_chapter_starts([0, 15, 105, 1226, 1315, 1409]),
            6
        );
    }

    #[test]
    fn chapter_scan_uses_first_edition_only() {
        let first_edition = [
            make_chapter_atom(0, &[]),
            make_chapter_atom(90, &[]),
            make_chapter_atom(742, &[]),
        ]
        .concat();
        let second_edition = [
            make_chapter_atom(33, &[]),
            make_chapter_atom(73, &[]),
            make_chapter_atom(164, &[]),
            make_chapter_atom(636, &[]),
        ]
        .concat();
        let chapters = make_ebml_element(
            &[0x10, 0x43, 0xA7, 0x70],
            &[
                make_ebml_element(&[0x45, 0xB9], &first_edition),
                make_ebml_element(&[0x45, 0xB9], &second_edition),
            ]
            .concat(),
        );

        assert_eq!(
            count_mkv_chapters_ffprobe_style_from_bytes(&chapters),
            Some(3)
        );
    }

    #[test]
    fn chapter_scan_ignores_nested_atoms_and_backwards_starts() {
        let nested = make_chapter_atom(105, &[]);
        let edition = [
            make_chapter_atom(0, &nested),
            make_chapter_atom(90, &[]),
            make_chapter_atom(15, &[]),
            make_chapter_atom(1409, &[]),
        ]
        .concat();
        let chapters = make_ebml_element(
            &[0x10, 0x43, 0xA7, 0x70],
            &make_ebml_element(&[0x45, 0xB9], &edition),
        );

        assert_eq!(
            count_mkv_chapters_ffprobe_style_from_bytes(&chapters),
            Some(3)
        );
    }

    #[test]
    fn estimate_frame_rate_from_timestamp_deltas_uses_millisecond_units() {
        let fps = estimate_frame_rate_from_timestamps(&[0, 40, 80, 120, 160], 1_000_000.0);
        assert_eq!(fps, Some(25.0));
    }

    #[test]
    fn estimate_frame_rate_requires_more_than_sparse_samples() {
        assert_eq!(
            estimate_frame_rate_from_timestamps(&[0, 1000], 1_000_000.0),
            None
        );
        assert_eq!(
            estimate_frame_rate_from_timestamps(&[0, 1000, 2000], 1_000_000.0),
            None
        );
    }

    #[test]
    fn fallback_frame_rate_uses_timestamp_scale_timebase() {
        assert_eq!(
            fallback_frame_rate_from_timestamp_scale(1_000_000.0),
            Some(1000.0)
        );
        let approx_24fps = fallback_frame_rate_from_timestamp_scale(41_666_667.0).unwrap();
        assert!((approx_24fps - 24.0).abs() < 0.001);
        assert_eq!(fallback_frame_rate_from_timestamp_scale(0.0), None);
    }

    #[test]
    fn plausible_frame_rate_guard_matches_expected_bounds() {
        assert!(is_plausible_frame_rate(Some(23.976)));
        assert!(is_plausible_frame_rate(Some(240.0)));
        assert!(!is_plausible_frame_rate(None));
        assert!(!is_plausible_frame_rate(Some(0.5)));
        assert!(!is_plausible_frame_rate(Some(1000.0)));
    }

    #[test]
    fn only_replaces_clearly_bogus_existing_frame_rates() {
        assert!(should_replace_frame_rate(None, 24.0));
        assert!(should_replace_frame_rate(Some(1.0), 1000.0));
        assert!(!should_replace_frame_rate(Some(24.0), 6.0));
        assert!(!should_replace_frame_rate(Some(23.976), 24.0));
    }

    #[test]
    fn track_entry_signals_detect_t35_mapping_and_dovi_config() {
        let dovi_config = vec![1, 0, 0x10, 0x00, 0x10];
        let track_entry = [
            make_uint_element(&[0xD7], 1),
            make_ebml_element(
                &[0x41, 0xE4],
                &make_uint_element(&[0x41, 0xE7], MATROSKA_BLOCK_ADD_ID_TYPE_ITU_T_T35),
            ),
            make_ebml_element(
                &[0x41, 0xE4],
                &[
                    make_uint_element(&[0x41, 0xE7], DOVI_BLOCK_ADD_ID_TYPE),
                    make_ebml_element(&[0x41, 0xED], &dovi_config),
                ]
                .concat(),
            ),
        ]
        .concat();

        let signals = track_entry_signals(&track_entry);
        assert!(signals.has_itu_t_t35_mapping);
        assert_eq!(signals.dovi_config, Some(dovi_config));
    }

    #[test]
    fn hdr10plus_confirmation_gate_short_circuits_dovi_hlg_and_non_hevc() {
        let hevc_track = RawTrack {
            kind: TrackKind::Video,
            codec_id: "V_MPEGH/ISO/HEVC".into(),
            codec_name: Some("hevc".into()),
            audio_profile: None,
            codec_private: None,
            width: None,
            height: None,
            channels: None,
            bit_rate_bps: None,
            language: None,
            name: None,
            forced: false,
            default_track: true,
            frame_rate_fps: None,
            color_transfer: Some(16),
            dovi_config: None,
            has_hdr10plus: false,
        };

        assert!(should_confirm_mkv_hdr10plus(
            &hevc_track,
            &MkvTrackSignals {
                has_itu_t_t35_mapping: true,
                dovi_config: None,
                header_strip_prefix: None,
            }
        ));

        let mut dovi_track = hevc_track.clone();
        dovi_track.dovi_config = Some(vec![1, 0, 0x10, 0x00, 0x10]);
        assert!(!should_confirm_mkv_hdr10plus(
            &dovi_track,
            &MkvTrackSignals {
                has_itu_t_t35_mapping: true,
                dovi_config: None,
                header_strip_prefix: None,
            }
        ));

        let mut hlg_track = hevc_track.clone();
        hlg_track.color_transfer = Some(18);
        assert!(!should_confirm_mkv_hdr10plus(
            &hlg_track,
            &MkvTrackSignals::default()
        ));

        let mut h264_track = hevc_track.clone();
        h264_track.codec_name = Some("h264".into());
        assert!(!should_confirm_mkv_hdr10plus(
            &h264_track,
            &MkvTrackSignals::default()
        ));
    }

    #[test]
    fn raw_block_timestamp_probe_derives_frame_rate_without_payload_scan() {
        let cluster = make_frame_rate_probe_cluster();
        let segment = make_ebml_element(&[0x18, 0x53, 0x80, 0x67], &cluster);
        let mut scanner = MkvRawScanner::new(Cursor::new(segment)).expect("scanner");

        let fps = scanner
            .probe_frame_rate(1, 1_000_000.0)
            .expect("probe should succeed");
        assert_eq!(fps, Some(25.0));
    }

    #[test]
    fn raw_block_timestamp_probe_respects_deep_probe_depth_cap() {
        let within_cap =
            wrap_in_segments(make_frame_rate_probe_cluster(), MKV_DEEP_PROBE_MAX_DEPTH);
        let mut scanner = MkvRawScanner::new(Cursor::new(within_cap)).expect("scanner");
        let fps = scanner
            .probe_frame_rate(1, 1_000_000.0)
            .expect("probe should complete within depth cap");
        assert_eq!(fps, Some(25.0));

        let beyond_cap = wrap_in_segments(
            make_frame_rate_probe_cluster(),
            MKV_DEEP_PROBE_MAX_DEPTH + 1,
        );
        let mut scanner = MkvRawScanner::new(Cursor::new(beyond_cap)).expect("scanner");
        let fps = scanner
            .probe_frame_rate(1, 1_000_000.0)
            .expect("probe should complete beyond depth cap");
        assert_eq!(fps, None);
    }

    #[test]
    fn sized_payload_rejects_declared_end_beyond_file_len_before_read() {
        let mut scanner =
            MkvRawScanner::new_with_file_len(Cursor::new(Vec::<u8>::new()), 16).expect("scanner");
        let header = EbmlElementHeader {
            id: EBML_ID_INFO,
            data_offset: 8,
            size: Some(MKV_KEEP_ELEMENT_MAX_BYTES),
        };

        let error = scanner
            .read_sized_payload(header, u64::MAX)
            .expect_err("oversized declared payload should fail before read");
        assert!(matches!(
            error,
            MediaInfoError::Parse(message) if message.contains("beyond remaining input")
        ));
    }

    #[test]
    fn parse_mkv_uses_real_file_length_for_declared_payload_limit() {
        let path = unique_temp_mkv_path("declared-payload");
        let data = make_mkv_with_declared_info_size(MKV_KEEP_ELEMENT_MAX_BYTES);
        assert!(data.len() < 1024, "test file should stay tiny");
        std::fs::write(&path, data).expect("write tiny mkv");

        let error = parse_mkv(&path, AnalysisProfile::DefaultRich)
            .expect_err("declared payload beyond file length should fail");
        let _ = std::fs::remove_file(&path);
        assert!(matches!(
            error,
            MediaInfoError::Parse(message) if message.contains("beyond remaining input")
        ));
    }

    #[test]
    fn parse_mkv_rejects_missing_ebml_header_immediately() {
        let path = std::env::temp_dir().join(format!("scryer-invalid-{}.mkv", std::process::id()));
        let file = std::fs::File::create(&path).expect("create invalid mkv");
        file.set_len(60 * 1024 * 1024).expect("set sparse size");

        let error = parse_mkv(&path, AnalysisProfile::DefaultRich)
            .expect_err("invalid mkv should fail fast");
        let _ = std::fs::remove_file(&path);
        assert!(matches!(error, MediaInfoError::Parse(message) if message.contains("ebml header")));
    }
}
