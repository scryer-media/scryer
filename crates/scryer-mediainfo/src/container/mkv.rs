//! Matroska and WebM: header, tracks, chapters and the bounded cluster probes.

use scryer_media_model::warning;
use scryer_media_model::{ChromaLocation, ChromaSubsampling, Codec};
use std::io::{BufReader, Read, Seek, SeekFrom};
#[cfg(test)]
use std::path::Path;

use crate::ProbeDepth;
use crate::ProbeError;
use crate::codec::ac3::{find_ac3_header, find_eac3_header};
use crate::codec::dts::find_dts_header;
use crate::codec::{
    audio_profile::{
        audio_profile_probe_spec, detect_audio_profile_from_probe_bytes,
        detect_header_audio_profile, merge_audio_profile,
    },
    dolby_vision::parse_dovi_config,
    dts::detect_dts_channels_from_probe_bytes,
    hdr10plus::{scan_hevc_frame_for_hdr10plus, scan_itu_t35_payload_for_hdr10plus},
    hevc::{extract_h265_info, hevc_nal_length_size},
    normalize_codec_name, normalize_pcm_codec_name, normalize_vfw_codec_name,
};
use crate::raw::{RawContainer, RawTrack, TrackKind};
use crate::scan;

mod blocks;
mod ebml;
mod inventory;
use blocks::*;
use ebml::*;

const MKV_FPS_PROBE_MAX_SCAN_BYTES: u64 = 32 * 1024 * 1024;
const MKV_DEEP_PROBE_CANDIDATE_READ_BYTES: usize = 64 * 1024;
const MKV_FPS_PROBE_MAX_TIMESTAMPS: usize = 12;
const MKV_FPS_PROBE_MAX_BLOCKS: usize = 24;
const MKV_RICH_HDR10PLUS_SCAN_MAX_BYTES: u64 = 512 * 1024;
const MKV_RICH_HDR10PLUS_MAX_VIDEO_BLOCKS: usize = 4;
const MKV_FIRST_FRAME_HDR10PLUS_SCAN_MAX_BYTES: u64 = 512 * 1024;
const MKV_FIRST_FRAME_HDR10PLUS_MAX_VIDEO_BLOCKS: usize = 1;
const MKV_FIRST_FRAME_HDR10PLUS_MAX_FILE_SCAN_BYTES: u64 = 32 * 1024 * 1024;
const MKV_HDR10PLUS_BLOCKADDITIONAL_PEEK_BYTES: u64 = 16;
const MKV_CHAPTER_SCAN_MAX_BYTES: usize = 2 * 1024 * 1024;
const MKV_AUDIO_PROFILE_SCAN_MAX_BYTES: u64 = 8 * 1024;
const MKV_AUDIO_PROFILE_MAX_BLOCKS: usize = 48;
const MKV_EAC3_AUDIO_PROFILE_MAX_BLOCKS: usize = 8;
const MKV_AUDIO_PROFILE_MAX_FILE_SCAN_BYTES: u64 = 128 * 1024 * 1024;
const MATROSKA_BLOCK_ADD_ID_TYPE_ITU_T_T35: u64 = 4;
/// BlockAddIDType FourCCs for the Dolby Vision configuration record: 'dvcC' (profiles 0-7)
/// and 'dvvC' (profiles 8-10 and 20), per the Matroska Block Additional Mappings registry.
/// The registry also lists 'dvwC'; it is left out because FFmpeg's Matroska demuxer does
/// not read it.
const MATROSKA_BLOCK_ADD_ID_TYPE_DVCC: u64 = u32::from_be_bytes(*b"dvcC") as u64;
const MATROSKA_BLOCK_ADD_ID_TYPE_DVVC: u64 = u32::from_be_bytes(*b"dvvC") as u64;
const DOVI_BLOCK_ADD_ID_TYPES: [u64; 2] = [
    MATROSKA_BLOCK_ADD_ID_TYPE_DVCC,
    MATROSKA_BLOCK_ADD_ID_TYPE_DVVC,
];
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
    _kind: TrackKind,
    language_bcp47: Option<&str>,
    language: Option<&str>,
) -> Option<String> {
    if let Some(language) = language_bcp47.or(language) {
        return normalize_explicit_mkv_language_tag(language);
    }
    None
}

/// Parse an MKV/WebM file into a [`RawContainer`].
#[cfg(test)]
pub(crate) fn parse_mkv(path: &Path, profile: ProbeDepth) -> Result<RawContainer, ProbeError> {
    parse_mkv_source(&mut crate::source::FileSource::open(path)?, profile)
}

pub(crate) fn parse_mkv_source(
    source: &mut dyn crate::source::MediaSource,
    profile: ProbeDepth,
) -> Result<RawContainer, ProbeError> {
    let file = BufReader::new(source);
    let mut scanner = MkvRawScanner::new(file)?;
    let header = parse_mkv_header(&mut scanner)?;
    let format_name = header.format_name;
    let mut duration_seconds = header.duration_seconds;
    let num_chapters = header.num_chapters.or_else(|| {
        (!profile.skips_deep_probes() && !header.chapters_known_absent)
            .then(|| {
                scanner
                    .scan_prefix_for_chapter_count(MKV_CHAPTER_SCAN_MAX_BYTES as u64)
                    .ok()
                    .flatten()
            })
            .flatten()
    });
    let file_size_hint = header.file_size_hint.unwrap_or(scanner.file_len);
    let timestamp_scale_ns = header.timestamp_scale_ns;
    let mut tracks = Vec::with_capacity(header.tracks.len());
    let mut primary_video_track_num = None;
    let mut primary_video_index = None;
    let mut primary_video_signals = MkvTrackSignals::default();
    let mut audio_track_refs = Vec::new();
    let mut track_uids = Vec::new();

    for parsed_track in header.tracks {
        if let Some(uid) = parsed_track.uid {
            track_uids.push((uid, tracks.len()));
        }
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

    let mut frame_rate_probe = None;
    let mut hdr10plus_probe = None;

    if let Some(video_idx) = primary_video_index
        && !profile.skips_deep_probes()
        && let Some(track_num) = primary_video_track_num
    {
        if !primary_video_signals.nondefault_timestamp_scale {
            frame_rate_probe = Some(MkvFrameRateProbeRequest {
                target_track_num: track_num,
                timestamp_scale_ns,
            });
        }

        let nal_length_size = tracks[video_idx]
            .codec_private
            .as_deref()
            .map(hevc_nal_length_size)
            .unwrap_or(4);

        if profile == ProbeDepth::FfprobeParity
            && should_check_first_frame_hdr10plus(&tracks[video_idx])
        {
            hdr10plus_probe = Some(MkvHdr10PlusProbeRequest::new(
                track_num,
                nal_length_size,
                primary_video_signals.has_itu_t_t35_mapping,
                Hdr10PlusProbeLimits::first_frame(),
            ));
        } else if profile == ProbeDepth::Full
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

    let audio_probe_requests = if profile.skips_deep_probes() {
        Vec::new()
    } else {
        audio_track_refs
            .into_iter()
            .filter_map(|(track_num, track_idx, header_strip_prefix)| {
                let codec_name = tracks[track_idx].codec_name.clone()?;
                matches!(
                    codec_name,
                    Codec::Ac3
                        | Codec::Eac3
                        | Codec::TrueHd
                        | Codec::Dts
                        | Codec::Mp1
                        | Codec::Mp2
                        | Codec::Mp3
                )
                .then_some(MkvAudioProbeRequest {
                    target_track_num: track_num,
                    track_idx,
                    codec_name,
                    header_strip_prefix,
                })
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
        if let Some(timing) = deep_probe.frame_timing {
            tracks[video_idx].metadata.observed_frame_rate = Some(timing.rate);
            tracks[video_idx].metadata.variable_frame_rate = Some(timing.variable);
        }
        if let Some(fps) = deep_probe.frame_rate_fps
            && should_replace_frame_rate(tracks[video_idx].frame_rate_fps, fps)
        {
            tracks[video_idx].frame_rate_fps = Some(fps);
        }
        if let Some(has_hdr10plus) = deep_probe.has_hdr10plus {
            tracks[video_idx].has_hdr10plus = has_hdr10plus;
        }
    }

    for (track_idx, scanned) in deep_probe.audio {
        merge_audio_profile(&mut tracks[track_idx].audio_profile, scanned.profile);
        if let Some(channels) = scanned.channels {
            tracks[track_idx].channels =
                merge_scanned_audio_channels(tracks[track_idx].channels, Some(channels));
        }
        if let Some(header) = scanned.mpeg_header {
            header.apply(&mut tracks[track_idx]);
        }
        if let Some(header) = scanned.dolby_header {
            header.apply(&mut tracks[track_idx]);
        }
        if tracks[track_idx].audio_profile.as_deref() == Some("DTS-ES")
            && tracks[track_idx].channels == Some(6)
        {
            tracks[track_idx].channels = Some(7);
        }
    }
    apply_unique_audio_prefix_channel_fallback(&mut scanner, &mut tracks);
    let mut sample_report = scryer_media_model::ProbeReport::default();
    let mut caption_services = Vec::new();
    if !profile.skips_deep_probes()
        && tracks.iter().any(|track| {
            matches!(
                track.codec_name,
                Some(
                    Codec::H264
                        | Codec::Hevc
                        | Codec::Av1
                        | Codec::Vp8
                        | Codec::Vp9
                        | Codec::Mpeg1Video
                        | Codec::Mpeg2Video
                        | Codec::Mpeg4
                        | Codec::Vc1
                        | Codec::Mjpeg
                )
            )
        })
    {
        let end = scanner.file_len.min(8 * 1024 * 1024);
        let mut samples_left = 16;
        let enriched = scanner.seek_to(0).and_then(|()| {
            scanner.enrich_from_picture_header_samples(
                end,
                0,
                &mut tracks,
                &mut samples_left,
                &mut sample_report,
                &mut caption_services,
            )
        });
        // Enrichment runs after the deep probe, so it is the stage most likely to
        // meet an exhausted budget. Losing sample detail is a gap in the report;
        // it must never cost the track table parsed before it.
        let budget_stopped_enrichment =
            enriched.is_err() && crate::source::MediaSource::exhausted(scanner.reader.get_ref());
        if let Err(error) = enriched
            && !budget_stopped_enrichment
        {
            return Err(error);
        }
        if budget_stopped_enrichment {
            sample_report.status = scryer_media_model::ProbeStatus::Incomplete;
            sample_report.budget_exhausted = true;
            sample_report
                .warnings
                .push(scryer_media_model::ProbeWarning {
                    code: warning::MKV_VIDEO_ENRICHMENT_BUDGET.into(),
                    message:
                        "Video enrichment stopped when the media byte or I/O-operation budget was exhausted"
                            .into(),
                    ..Default::default()
                });
        } else if samples_left == 0 || end < scanner.file_len {
            sample_report.status = scryer_media_model::ProbeStatus::Incomplete;
            sample_report.budget_exhausted = true;
            sample_report
                .warnings
                .push(scryer_media_model::ProbeWarning {
                    code: warning::VIDEO_ENRICHMENT_SAMPLE_LIMIT.into(),
                    message: "Video enrichment sampled at most 16 blocks within an 8 MiB prefix"
                        .into(),
                    ..Default::default()
                });
        }
    }

    let mut details = if !profile.skips_deep_probes() {
        inventory::read(
            &mut scanner,
            &mut tracks,
            &track_uids,
            timestamp_scale_ns,
            duration_seconds.is_none(),
        )
    } else {
        Default::default()
    };
    if sample_report.status == scryer_media_model::ProbeStatus::Incomplete {
        details.report.status = sample_report.status;
    }
    details.report.budget_exhausted |= sample_report.budget_exhausted;
    details.report.warnings.extend(sample_report.warnings);
    if frame_rate_probe.is_some()
        || (!profile.skips_deep_probes() && primary_video_signals.nondefault_timestamp_scale)
    {
        details.report.status = scryer_media_model::ProbeStatus::Incomplete;
        details.report.budget_exhausted |= deep_probe.frame_timing_budget_exhausted;
        details.report.warnings.push(scryer_media_model::ProbeWarning {
            code: warning::MKV_FRAME_TIMING_SAMPLED.into(),
            message: "Frame timing observes at most 12 timestamps or 24 blocks within a 32 MiB prefix; laced, nondefault track-scale, or inconclusive samples remain unknown".into(),
            stream_id: primary_video_track_num.map(|number| number.to_string()),
            ..Default::default()
        });
    }
    details.caption_services = caption_services;
    duration_seconds = duration_seconds.or(details.duration_seconds);
    details.overall_bitrate_bps = duration_seconds
        .filter(|duration| *duration > 0.0)
        .map(|duration| (file_size_hint as f64 * 8.0 / duration) as u64);
    let num_chapters = if details.chapters.is_empty() {
        num_chapters
    } else {
        i32::try_from(details.chapters.len()).ok()
    };
    Ok(RawContainer {
        details,
        format_name,
        duration_seconds,
        num_chapters,
        tracks,
    })
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
    uid: Option<u64>,
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
) -> Result<ParsedMkvHeader, ProbeError> {
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
    let tracks = tracks.ok_or_else(|| ProbeError::Malformed("matroska tracks missing".into()))?;

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
) -> Result<String, ProbeError> {
    let header = match scanner.read_element_header() {
        Ok(Some(header)) => header,
        Ok(None) => return Err(ProbeError::Malformed("missing ebml header".into())),
        Err(ProbeError::Malformed(_)) => {
            return Err(ProbeError::Malformed("missing ebml header".into()));
        }
        Err(error) => return Err(error),
    };
    if header.id != EBML_ID_EBML {
        return Err(ProbeError::Malformed("missing ebml header".into()));
    }
    let payload = scanner
        .read_sized_payload(header, scanner.file_len)?
        .ok_or_else(|| ProbeError::Malformed("missing ebml header payload".into()))?;
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
    if channels.is_none() && codec_name == Some(Codec::Flac) {
        channels = codec_private
            .as_deref()
            .and_then(parse_mkv_flac_codec_private_channels);
    }
    let audio_profile = if codec_name == Some(Codec::Dts) {
        None
    } else {
        detect_header_audio_profile(&codec_id, codec_name.as_ref(), codec_private.as_deref())
    };

    let mut metadata = mkv_stream_metadata(payload, track_number);
    // An unreadable FlagDefault leaves the disposition unknown; the element's
    // own default (set) still applies.
    if default_track {
        metadata.disposition.default.get_or_insert(true);
    }
    if forced {
        metadata.disposition.forced.get_or_insert(true);
    }
    Some(ParsedMkvTrack {
        track_number,
        uid: find_first_direct_ebml_child(payload, 0x73c5)
            .and_then(parse_ebml_uint)
            .filter(|uid| *uid != 0),
        raw: RawTrack {
            metadata,
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
            frame_rate_fps,
            color_transfer,
            // Each track keeps its own record, so the primary video chosen later
            // (which skips cover-art tracks) still sees it.
            dovi_config: signals.dovi_config.clone(),
            has_hdr10plus: false,
        },
        signals,
    })
}

fn mkv_stream_metadata(payload: &[u8], track_number: u64) -> scryer_media_model::StreamMetadata {
    use scryer_media_model::{
        ContentLight, MasteringDisplay, Provenance, Rational, StreamMetadata,
    };
    fn children(mut bytes: &[u8], mut visit: impl FnMut(u32, &[u8])) {
        while let Some((id, data, consumed)) = next_ebml_element(bytes) {
            visit(id, data);
            bytes = &bytes[consumed..];
        }
    }
    let mut metadata = StreamMetadata {
        id: Some(track_number.to_string()),
        ..Default::default()
    };
    metadata.disposition.default = Some(true);
    metadata.disposition.forced = Some(false);
    let mut iso639_language = None;
    let mut bcp47_language = None;
    children(payload, |id, data| {
        let flag = || parse_ebml_uint(data).map(|value| value != 0);
        match id {
            EBML_ID_LANGUAGE => iso639_language = parse_ebml_string(data).ok(),
            EBML_ID_LANGUAGE_BCP47 => bcp47_language = parse_ebml_string(data).ok(),
            EBML_ID_FLAG_DEFAULT => metadata.disposition.default = flag(),
            EBML_ID_FLAG_FORCED => metadata.disposition.forced = flag(),
            0x55ab => metadata.disposition.hearing_impaired = flag(),
            0x55ac => metadata.disposition.visual_impaired = flag(),
            0x55ae => metadata.disposition.original = flag(),
            0x55af => metadata.disposition.commentary = flag(),
            EBML_ID_DEFAULT_DURATION => {
                metadata.declared_frame_rate = parse_ebml_uint(data)
                    .and_then(|duration| Rational::new(1_000_000_000, duration))
            }
            EBML_ID_AUDIO => children(data, |id, data| match id {
                0xb5 | 0x78b5 => {
                    metadata.sample_rate = parse_ebml_float(data)
                        .filter(|value| {
                            value.is_finite() && *value > 0.0 && *value <= f64::from(u32::MAX)
                        })
                        .map(|value| value as u32)
                }
                EBML_ID_BIT_DEPTH => {
                    metadata.sample_bit_depth =
                        parse_ebml_uint(data).and_then(|value| u32::try_from(value).ok())
                }
                _ => {}
            }),
            EBML_ID_VIDEO => {
                let mut display_width = None;
                let mut display_height = None;
                let mut display_unit = Some(0);
                let mut pixel_width = None;
                let mut pixel_height = None;
                let mut crop = [Some(0_u64); 4];
                children(data, |id, data| match id {
                    EBML_ID_PIXEL_WIDTH => pixel_width = parse_ebml_uint(data),
                    EBML_ID_PIXEL_HEIGHT => pixel_height = parse_ebml_uint(data),
                    // Matroska StereoMode: 0 is mono, 1..=14 are stereo layouts.
                    0x53b8 => metadata.is_3d |= matches!(parse_ebml_uint(data), Some(1..=14)),
                    0x54b2 => display_unit = parse_ebml_uint(data),
                    0x54cc => crop[0] = parse_ebml_uint(data),
                    0x54dd => crop[1] = parse_ebml_uint(data),
                    0x54bb => crop[2] = parse_ebml_uint(data),
                    0x54aa => crop[3] = parse_ebml_uint(data),
                    0x9a => {
                        metadata.field_order = parse_ebml_uint(data).and_then(|value| match value {
                            1 => Some("interlaced".into()),
                            2 => Some("progressive".into()),
                            _ => None,
                        })
                    }
                    0x9d => {
                        metadata.field_order = parse_ebml_uint(data)
                            .and_then(|value| match value {
                                0 => Some("progressive".into()),
                                1 => Some("top_first".into()),
                                6 => Some("bottom_first".into()),
                                _ => None,
                            })
                            .or(metadata.field_order.take())
                    }
                    0x54b0 => display_width = parse_ebml_uint(data),
                    0x54ba => display_height = parse_ebml_uint(data),
                    EBML_ID_COLOUR => {
                        metadata.color.provenance = Provenance::Container;
                        let mut light = ContentLight::default();
                        let mut subsampling = [None; 2];
                        let mut siting = [None; 2];
                        children(data, |id, data| {
                            let value =
                                parse_ebml_uint(data).and_then(|value| u32::try_from(value).ok());
                            match id {
                                0x55b1 => metadata.color.matrix = value.filter(|value| *value != 2),
                                0x55b2 => {
                                    metadata.bit_depth =
                                        value.and_then(|value| i32::try_from(value).ok())
                                }
                                0x55b9 => {
                                    metadata.color.full_range =
                                        value.and_then(|value| match value {
                                            1 => Some(false),
                                            2 => Some(true),
                                            _ => None,
                                        })
                                }
                                0x55ba => {
                                    metadata.color.transfer = value.filter(|value| *value != 2)
                                }
                                0x55bb => {
                                    metadata.color.primaries = value.filter(|value| *value != 2)
                                }
                                0x55b3 => subsampling[0] = value,
                                0x55b4 => subsampling[1] = value,
                                0x55b7 => siting[0] = value,
                                0x55b8 => siting[1] = value,
                                0x55bc => light.max_cll = value,
                                0x55bd => light.max_fall = value,
                                0x55d0 => {
                                    let mut mastering = MasteringDisplay::default();
                                    children(data, |id, data| {
                                        let value = parse_ebml_float(data)
                                            .filter(|value| value.is_finite() && *value >= 0.0);
                                        match id {
                                            0x55d1 => mastering.red_x = value,
                                            0x55d2 => mastering.red_y = value,
                                            0x55d3 => mastering.green_x = value,
                                            0x55d4 => mastering.green_y = value,
                                            0x55d5 => mastering.blue_x = value,
                                            0x55d6 => mastering.blue_y = value,
                                            0x55d7 => mastering.white_x = value,
                                            0x55d8 => mastering.white_y = value,
                                            0x55d9 => mastering.max_luminance = value,
                                            0x55da => mastering.min_luminance = value,
                                            _ => {}
                                        }
                                    });
                                    metadata.color.mastering_display = Some(mastering);
                                }
                                _ => {}
                            }
                        });
                        if light.max_cll.is_some() || light.max_fall.is_some() {
                            metadata.color.content_light = Some(light);
                        }
                        metadata.chroma_subsampling = colour_subsampling(subsampling);
                        metadata.chroma_location = colour_chroma_siting(siting);
                    }
                    _ => {}
                });
                let cropped_width = pixel_width
                    .and_then(|width| width.checked_sub(crop[0]?)?.checked_sub(crop[1]?))
                    .filter(|value| *value > 0);
                let cropped_height = pixel_height
                    .and_then(|height| height.checked_sub(crop[2]?)?.checked_sub(crop[3]?))
                    .filter(|value| *value > 0);
                if display_unit == Some(0) {
                    display_width = display_width.or(cropped_width);
                    display_height = display_height.or(cropped_height);
                }
                if matches!(display_unit, Some(0..=3)) {
                    metadata.display_aspect_ratio = display_width
                        .zip(display_height)
                        .filter(|(width, height)| *width > 0 && *height > 0)
                        .and_then(|(width, height)| {
                            Rational::new(i64::try_from(width).ok()?, height)
                        });
                    metadata.sample_aspect_ratio =
                        metadata.display_aspect_ratio.as_ref().and_then(|ratio| {
                            let numerator = u64::try_from(ratio.numerator)
                                .ok()?
                                .checked_mul(cropped_height?)?;
                            let denominator = ratio.denominator.checked_mul(cropped_width?)?;
                            Rational::new(i64::try_from(numerator).ok()?, denominator)
                        });
                }
            }
            _ => {}
        }
    });
    metadata.original_language = bcp47_language.or(iso639_language);
    if metadata.original_language.is_some() {
        metadata.language_provenance = Provenance::Container;
    }
    metadata
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

/// The sampling a Matroska Colour element's ChromaSubsamplingHorz and
/// ChromaSubsamplingVert state. The specification counts the chroma samples
/// removed for each one kept, which makes 4:1:1 a horizontal 3; common
/// muxers write the log2 of the decimation instead, a horizontal 2. Both
/// are read as 4:1:1, since a horizontal 2 means nothing else in practice
/// and the two conventions agree on every other sampling. Absent elements
/// state nothing; a monochrome picture cannot be stated this way.
fn colour_subsampling([horizontal, vertical]: [Option<u32>; 2]) -> Option<ChromaSubsampling> {
    Some(match (horizontal?, vertical?) {
        (1, 1) => ChromaSubsampling::Yuv420,
        (1, 0) => ChromaSubsampling::Yuv422,
        (0, 0) => ChromaSubsampling::Yuv444,
        (2 | 3, 0) => ChromaSubsampling::Yuv411,
        (0, 1) => ChromaSubsampling::Yuv440,
        _ => return None,
    })
}

/// The chroma location a Matroska Colour element's ChromaSitingHorz and
/// ChromaSitingVert state: 1 is co-sited with the left column or top row, 2
/// is halfway between samples, and 0 or absence leaves it unstated.
fn colour_chroma_siting([horizontal, vertical]: [Option<u32>; 2]) -> Option<ChromaLocation> {
    Some(match (horizontal?, vertical?) {
        (1, 1) => ChromaLocation::TopLeft,
        (1, 2) => ChromaLocation::Left,
        (2, 1) => ChromaLocation::Top,
        (2, 2) => ChromaLocation::Center,
        _ => return None,
    })
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
    Some(count_increasing_top_level_chapters(first_edition_payload))
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

fn audio_channels_from_probe_bytes(codec_name: &Codec, data: &[u8]) -> Option<i32> {
    let mut cursor = 0usize;
    let mut channels = None;

    while let Some(candidate) = scan::find_audio_sync_candidate(data, cursor) {
        cursor = candidate.offset.saturating_add(1);
        let parsed = match (codec_name, candidate.kind) {
            (Codec::Ac3, scan::AudioSyncKind::Ac3) => {
                find_ac3_header(&data[candidate.offset..]).map(|header| i32::from(header.channels))
            }
            (Codec::Eac3, scan::AudioSyncKind::Ac3) => {
                find_eac3_header(&data[candidate.offset..]).map(|header| i32::from(header.channels))
            }
            (Codec::Dts, scan::AudioSyncKind::Dts) => {
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
        let Some(codec_name) = tracks[track_idx].codec_name.as_ref() else {
            continue;
        };
        if tracks[track_idx].kind != TrackKind::Audio
            || !matches!(codec_name, Codec::Ac3 | Codec::Eac3 | Codec::Dts)
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

fn audio_codec_track_count(tracks: &[RawTrack], codec_name: &Codec) -> usize {
    tracks
        .iter()
        .filter(|track| {
            track.kind == TrackKind::Audio && track.codec_name.as_ref() == Some(codec_name)
        })
        .count()
}

#[cfg(test)]
fn count_mkv_chapters_from_bytes(data: &[u8]) -> Option<i32> {
    let root = find_ebml_element_payload(data, EBML_ID_SEGMENT).unwrap_or(data);
    let chapters_payload = find_first_direct_ebml_child(root, EBML_ID_CHAPTERS)?;
    let first_edition_payload =
        find_first_direct_ebml_child(chapters_payload, EBML_ID_EDITION_ENTRY)?;
    Some(count_increasing_top_level_chapters(first_edition_payload))
}

fn count_increasing_top_level_chapters(data: &[u8]) -> i32 {
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
    count_increasing_chapter_starts(starts)
}

fn count_increasing_chapter_starts(starts: impl IntoIterator<Item = u64>) -> i32 {
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

#[derive(Debug, Clone, Default)]
struct MkvTrackSignals {
    has_itu_t_t35_mapping: bool,
    dovi_config: Option<Vec<u8>>,
    header_strip_prefix: Option<Vec<u8>>,
    nondefault_timestamp_scale: bool,
}

fn track_entry_signals(track_entry_payload: &[u8]) -> MkvTrackSignals {
    let mut current = track_entry_payload;
    let mut signals = MkvTrackSignals::default();
    while !current.is_empty() {
        let Some((id, payload, consumed)) = next_ebml_element(current) else {
            break;
        };
        if id == 0x23314f {
            // Deprecated TrackTimestampScale changes block timing semantics.
            signals.nondefault_timestamp_scale |= parse_ebml_float(payload) != Some(1.0);
        } else if id == EBML_ID_BLOCK_ADDITION_MAPPING {
            if block_addition_mapping_has_type(payload, MATROSKA_BLOCK_ADD_ID_TYPE_ITU_T_T35) {
                signals.has_itu_t_t35_mapping = true;
            }
            // Like FFmpeg, a later record replaces an earlier one; a record that does
            // not parse is ignored rather than shadowing a valid one.
            if let Some(record) =
                block_addition_mapping_extra_data(payload, &DOVI_BLOCK_ADD_ID_TYPES)
                    .filter(|record| parse_dovi_config(record).is_some())
            {
                signals.dovi_config = Some(record);
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

fn block_addition_mapping_extra_data(payload: &[u8], target_types: &[u64]) -> Option<Vec<u8>> {
    block_addition_mapping_type(payload)
        .is_some_and(|mapping_type| target_types.contains(&mapping_type))
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
    if track.codec_name != Some(Codec::Hevc) {
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

/// PQ HEVC without Dolby Vision gets a first-frame HDR10+ check.
fn should_check_first_frame_hdr10plus(track: &RawTrack) -> bool {
    track.codec_name == Some(Codec::Hevc)
        && track.dovi_config.is_none()
        && track.color_transfer == Some(16)
}

#[cfg(test)]
mod tests;
