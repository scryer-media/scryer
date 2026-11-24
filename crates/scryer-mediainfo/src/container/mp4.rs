//! ISO base media files (MP4, MOV and relatives): the box walk, tracks, timing
//! and chapters.

use std::collections::{HashMap, HashSet, hash_map::Entry};
mod boxes;
mod chapters;
mod moov;
mod samples;
mod timing;
use boxes::*;
use moov::*;
use samples::*;
use std::io::{Cursor, Read, Seek, SeekFrom};

use core::range::Range;

use mp4parse::{
    AudioCodecSpecific, CodecType, MediaContext, MediaTimeScale, SampleEntry, TrackTimeScale,
    TrackType, VideoCodecSpecific,
};

use crate::AnalysisProfile;
use crate::MediaInfoError;
use crate::codec::{
    audio_profile::{
        audio_profile_probe_spec, detect_audio_profile_from_probe_bytes,
        detect_header_audio_profile, merge_audio_profile,
    },
    normalize_codec_name,
};
use crate::raw::{RawContainer, RawTrack, TrackKind};
use crate::scan;

const HDR10PLUS_SAMPLE_LIMIT_BYTES: u64 = 4 * 1024 * 1024;
/// Dolby Vision configuration boxes: 'dvcC' (profiles 0-7), 'dvvC' (8-10), 'dvwC' (above 10).
const MP4_DOVI_TYPES: [&str; 3] = ["dvcC", "dvvC", "dvwC"];
const MOV_TKHD_FLAG_ENABLED: u32 = 0x000001;
const MP4_BOX_MAX_DEPTH: usize = 10;
const MP4_KEEP_BOX_MAX_BYTES: u64 = 256 * 1024 * 1024;
const MP4_METADATA_OUTPUT_MAX_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug)]
struct PreparedMp4 {
    metadata: Vec<u8>,
    file_len_hint: u64,
    budget_exhausted: bool,
}

#[derive(Debug, Clone)]
struct ParsedMp4Track {
    track_id: Option<u32>,
    raw: RawTrack,
}

#[derive(Debug, Clone, Default)]
struct Mp4TrackMetadata {
    details: scryer_media_model::StreamMetadata,
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

#[derive(Debug, Clone, Default)]
struct Mp4ChapterMetadata {
    chpl_count: Option<i32>,
    entries: Vec<scryer_media_model::Chapter>,
    incomplete: bool,
    chapter_track_ids: Vec<u32>,
}

#[derive(Debug, Default)]
struct ParsedMp4Metadata {
    tracks: HashMap<u32, Mp4TrackMetadata>,
    chapters: Mp4ChapterMetadata,
}

#[derive(Debug, Clone, Copy)]
struct Mp4BoxHeader {
    name: [u8; 4],
    size: u64,
    header_size: usize,
}

pub(crate) fn parse_mp4_source(
    source: &mut dyn crate::source::MediaSource,
    extension: &str,
    profile: AnalysisProfile,
) -> Result<RawContainer, MediaInfoError> {
    let mut prepared = prepare_mp4_metadata(source)?;
    sanitize_prepared_mp4_metadata(&mut prepared.metadata);
    let parsed_metadata = parse_mp4_metadata(&prepared.metadata);
    let metadata_by_track = parsed_metadata.tracks;

    let mut cursor = Cursor::new(prepared.metadata.as_slice());
    let ctx = match mp4parse::read_mp4(&mut cursor) {
        Ok(ctx) => ctx,
        Err(_) if prepared.budget_exhausted => {
            let mut details = scryer_media_model::AnalysisDetails::default();
            report_container_budget(&mut details.report);
            return Ok(RawContainer {
                format_name: extension.to_owned(),
                duration_seconds: None,
                num_chapters: None,
                tracks: Vec::new(),
                details,
            });
        }
        Err(error) => return Err(MediaInfoError::Parse(format!("mp4 parse: {error:?}"))),
    };
    let num_chapters = match count_mp4_chapters(&parsed_metadata.chapters, &ctx) {
        0 => None,
        count => Some(count),
    };

    let format_duration_seconds = mp4_format_duration_seconds(&ctx);
    let mut duration_seconds =
        mp4_runtime_seconds(&ctx, &metadata_by_track, format_duration_seconds);

    let (mut tracks, seen_track_ids) = build_mp4_tracks(&ctx, &metadata_by_track);
    let mut fragment_result = timing::fragments(&prepared.metadata);
    if prepared.budget_exhausted {
        // A prefix of the fragment sequence cannot establish full-file timing
        // or complete stream byte accounting.
        fragment_result.timelines.clear();
    }
    for track in &mut tracks {
        let Some(id) = track.track_id else {
            continue;
        };
        let Some(timeline) = fragment_result
            .timelines
            .get(&id)
            .filter(|timeline| !timeline.invalid)
        else {
            continue;
        };
        let scale = ctx
            .tracks
            .iter()
            .find(|track| track.track_id == Some(id))
            .and_then(|track| track.timescale)
            .map(|TrackTimeScale(scale, _)| scale)
            .filter(|scale| *scale > 0);
        if let Some(scale) = scale {
            let Some(duration) =
                timing::presentation_duration(&prepared.metadata, id, scale, timeline)
            else {
                continue;
            };
            if duration > 0.0 {
                track.raw.metadata.duration_seconds = Some(duration);
                // Encoded sample bitrate uses the complete media timeline, before edit trims.
                let media_duration = timeline
                    .start
                    .zip(timeline.end)
                    .map(|(start, end)| (end - start) as f64 / scale as f64);
                track.raw.bit_rate_bps = timeline
                    .bytes
                    .zip(media_duration)
                    .filter(|(_, duration)| *duration > 0.0)
                    .map(|(bytes, duration)| (bytes as f64 * 8.0 / duration) as i64);
                track.raw.metadata.bitrate_provenance = scryer_media_model::Provenance::Observed;
                if track.raw.kind == TrackKind::Video && duration_seconds.is_none() {
                    duration_seconds = Some(duration);
                }
            }
        }
    }
    append_metadata_only_tracks(&metadata_by_track, &seen_track_ids, &mut tracks);
    let overall_bitrate_bps = duration_seconds
        .filter(|duration| *duration > 0.0)
        .map(|duration| (prepared.file_len_hint as f64 * 8.0 / duration) as u64);
    let mut report = scryer_media_model::ProbeReport::default();
    report_fragment_budget(&mut report, fragment_result.budget_exhausted);
    if prepared.budget_exhausted {
        report_container_budget(&mut report);
    }
    for parsed in &tracks {
        if parsed.raw.kind == TrackKind::Video
            && ctx
                .tracks
                .iter()
                .find(|track| track.track_id == parsed.track_id)
                .and_then(|track| track.ctts.as_ref())
                .is_some_and(|offsets| {
                    offsets
                        .samples
                        .iter()
                        .map(|entry| u64::from(entry.sample_count))
                        .sum::<u64>()
                        > 4096
                })
        {
            sample_probe_warning(
                &mut report,
                &parsed.raw,
                "presentation_timing_sample_limit",
                true,
            );
        }
    }
    let caption_services = scan_mp4_sample_probes(source, &ctx, &mut tracks, profile, &mut report);
    let mut chapters = parsed_metadata.chapters.entries;
    if let Some(last) = chapters.last_mut() {
        last.end_seconds = duration_seconds.filter(|end| *end >= last.start_seconds);
    }
    if parsed_metadata.chapters.chpl_count.is_none() && !profile.skips_deep_probes() {
        let mut remaining = 4096;
        let mut bytes_left = 4 * 1024 * 1024;
        for id in &parsed_metadata.chapters.chapter_track_ids {
            let result = ctx
                .tracks
                .iter()
                .find(|track| track.track_id == Some(*id))
                .filter(|_| {
                    metadata_by_track
                        .get(id)
                        .and_then(|metadata| metadata.sample_entry_fourcc.as_deref())
                        .is_some_and(|format| matches!(format, "text" | "tx3g"))
                })
                .ok_or((
                    "Referenced chapter text track is unavailable or unsupported",
                    false,
                ))
                .and_then(|track| {
                    chapters::quicktime(
                        source,
                        track,
                        &prepared.metadata,
                        &mut remaining,
                        &mut bytes_left,
                    )
                });
            match result {
                Ok(entries) => chapters.extend(entries),
                Err((message, budget)) => {
                    report.status = scryer_media_model::ProbeStatus::Incomplete;
                    report.budget_exhausted |= budget;
                    report.warnings.push(scryer_media_model::ProbeWarning {
                        code: "mp4_chapter_track_incomplete".into(),
                        message: message.into(),
                        stream_id: Some(id.to_string()),
                        ..Default::default()
                    });
                }
            }
        }
        chapters.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    }
    if parsed_metadata.chapters.incomplete
        || (chapters.is_empty() && !parsed_metadata.chapters.chapter_track_ids.is_empty())
    {
        report.status = scryer_media_model::ProbeStatus::Incomplete;
        report.warnings.push(scryer_media_model::ProbeWarning {
            code: "mp4_chapters_incomplete".into(),
            message: "Chapter metadata could not be fully inventoried".into(),
            ..Default::default()
        });
    }
    // Referenced text samples carry chapter labels, not subtitle dialogue.
    tracks.retain(|track| {
        !(track.raw.kind == TrackKind::Subtitle
            && track
                .track_id
                .is_some_and(|id| parsed_metadata.chapters.chapter_track_ids.contains(&id)))
    });
    let num_chapters = if chapters.is_empty() {
        num_chapters
    } else {
        i32::try_from(chapters.len()).ok()
    };

    let format_name = if extension.eq_ignore_ascii_case("mov") {
        "mov"
    } else {
        "mp4"
    }
    .to_owned();

    Ok(RawContainer {
        details: scryer_media_model::AnalysisDetails {
            overall_bitrate_bps,
            report,
            caption_services,
            chapters,
            ..Default::default()
        },
        format_name,
        duration_seconds,
        num_chapters,
        tracks: tracks.into_iter().map(|track| track.raw).collect(),
    })
}

fn report_fragment_budget(report: &mut scryer_media_model::ProbeReport, exhausted: bool) {
    if !exhausted {
        return;
    }
    report.status = scryer_media_model::ProbeStatus::Incomplete;
    report.budget_exhausted = true;
    report.warnings.push(scryer_media_model::ProbeWarning {
        code: "mp4_fragment_inventory_budget".into(),
        message: "Fragment timing metadata exceeds the bounded header, track, or sample inventory"
            .into(),
        ..Default::default()
    });
}

fn mp4_format_duration_seconds(ctx: &MediaContext) -> Option<f64> {
    let movie_timescale = ctx.timescale.map(|MediaTimeScale(ts)| ts)?;
    if movie_timescale == 0 {
        return None;
    }
    ctx.tracks
        .iter()
        .filter_map(|t| t.tkhd.as_ref().map(|h| h.duration))
        .max()
        .map(|dur| dur as f64 / movie_timescale as f64)
}

fn mp4_runtime_seconds(
    ctx: &MediaContext,
    metadata_by_track: &HashMap<u32, Mp4TrackMetadata>,
    format_duration: Option<f64>,
) -> Option<f64> {
    let audio = first_mp4_track_duration_seconds(ctx, metadata_by_track, TrackKind::Audio);
    let video = primary_mp4_video_duration_seconds(ctx, metadata_by_track);
    preferred_runtime(audio, video, format_duration)
}

fn first_mp4_track_duration_seconds(
    ctx: &MediaContext,
    metadata_by_track: &HashMap<u32, Mp4TrackMetadata>,
    kind: TrackKind,
) -> Option<f64> {
    ctx.tracks.iter().find_map(|track| {
        let metadata = track.track_id.and_then(|id| metadata_by_track.get(&id));
        (track_kind_from_mp4_sources(track, metadata) == Some(kind))
            .then(|| track_duration_seconds(track))
            .flatten()
            .filter(|duration| *duration > 0.0)
    })
}

fn primary_mp4_video_duration_seconds(
    ctx: &MediaContext,
    metadata_by_track: &HashMap<u32, Mp4TrackMetadata>,
) -> Option<f64> {
    let video_tracks: Vec<_> = ctx
        .tracks
        .iter()
        .filter_map(|track| {
            let metadata = track.track_id.and_then(|id| metadata_by_track.get(&id));
            (track_kind_from_mp4_sources(track, metadata) == Some(TrackKind::Video))
                .then_some((track, metadata))
        })
        .collect();

    let selected = if video_tracks.len() <= 1 {
        video_tracks.first().copied()
    } else {
        let mut selected = None;
        for (track, metadata) in &video_tracks {
            if !matches!(
                mp4_video_codec_name(track, *metadata).as_deref(),
                Some("mjpeg" | "png")
            ) {
                selected = Some((*track, *metadata));
                break;
            }
        }
        selected.or_else(|| video_tracks.first().copied())
    };

    selected
        .and_then(|(track, _)| track_duration_seconds(track))
        .filter(|duration| *duration > 0.0)
}

fn mp4_video_codec_name(
    track: &mp4parse::Track,
    metadata: Option<&Mp4TrackMetadata>,
) -> Option<String> {
    let codec_id = metadata
        .and_then(|m| m.sample_entry_fourcc.clone())
        .or_else(|| {
            track
                .stsd
                .as_ref()
                .and_then(|stsd| stsd.descriptions.first())
                .and_then(|entry| match entry {
                    SampleEntry::Video(video) => video_codec_info(&video.codec_specific)
                        .0
                        .or_else(|| Some(codec_type_to_fourcc(video.codec_type))),
                    _ => None,
                })
        })?;
    normalize_codec_name(&codec_id)
}

fn preferred_runtime(audio: Option<f64>, video: Option<f64>, format: Option<f64>) -> Option<f64> {
    if video.unwrap_or_default() > 0.0 {
        video
    } else if audio.unwrap_or_default() > 0.0 {
        audio
    } else {
        format.filter(|duration| *duration > 0.0)
    }
}

fn report_container_budget(report: &mut scryer_media_model::ProbeReport) {
    report.status = scryer_media_model::ProbeStatus::Incomplete;
    report.budget_exhausted = true;
    report.warnings.push(scryer_media_model::ProbeWarning {
        code: "mp4_container_scan_budget".into(),
        message: "MP4 catalog inspection stops after 128 top-level boxes or 16 MiB of metadata; incomplete fragment timing and bitrate remain unknown".into(),
        ..Default::default()
    });
}

fn prepare_mp4_metadata(
    file: &mut dyn crate::source::MediaSource,
) -> Result<PreparedMp4, MediaInfoError> {
    let mut reader = file;
    let (metadata, file_len_hint, budget_exhausted) =
        prepare_mp4_metadata_from_reader(&mut reader)?;
    Ok(PreparedMp4 {
        metadata,
        file_len_hint,
        budget_exhausted,
    })
}

fn prepare_mp4_metadata_from_reader<R: Read + Seek>(
    reader: &mut R,
) -> Result<(Vec<u8>, u64, bool), MediaInfoError> {
    let mut output = Vec::new();
    let mut pos = 0_u64;
    let mut boxes_left = 128_usize;
    let mut budget_exhausted = false;
    let input_len = reader
        .seek(SeekFrom::End(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|e| MediaInfoError::Io(e.to_string()))?;

    loop {
        if pos == input_len {
            break;
        }
        if boxes_left == 0 {
            budget_exhausted = true;
            break;
        }
        boxes_left -= 1;
        let start = pos;
        let Some((header, header_bytes)) = read_top_level_box_header(reader)? else {
            break;
        };
        pos = pos.saturating_add(header.header_size as u64);
        let keep = should_copy_top_level_box(&header.name);

        if header.size != 0 {
            let box_end = start.checked_add(header.size).ok_or_else(|| {
                MediaInfoError::Parse(format!(
                    "MP4 box {} size overflow",
                    fourcc_to_string(header.name)
                ))
            })?;
            if box_end > input_len {
                return Err(MediaInfoError::Parse(format!(
                    "MP4 box {} extends past end of input",
                    fourcc_to_string(header.name)
                )));
            }
        }

        if keep {
            let retained_size = if header.size == 0 {
                input_len - start
            } else {
                header.size
            };
            if retained_size > (16 * 1024 * 1024_u64).saturating_sub(output.len() as u64) {
                budget_exhausted = true;
                break;
            }
            if header.size == 0 {
                output.extend_from_slice(&header_bytes);
                read_zero_sized_top_level_box(reader, &mut output, &mut pos)?;
                break;
            }
            if header.size > MP4_KEEP_BOX_MAX_BYTES {
                return Err(MediaInfoError::Parse(format!(
                    "MP4 metadata box {} exceeds parser budget",
                    fourcc_to_string(header.name)
                )));
            }
            let box_size = usize::try_from(header.size).map_err(|_| {
                MediaInfoError::Parse(format!(
                    "MP4 metadata box {} is too large for this platform",
                    fourcc_to_string(header.name)
                ))
            })?;
            if output.len().saturating_add(box_size) > MP4_METADATA_OUTPUT_MAX_BYTES {
                return Err(MediaInfoError::Parse(
                    "MP4 metadata output exceeds parser budget".into(),
                ));
            }
            output.extend_from_slice(&header_bytes);
            let payload_size = box_size.saturating_sub(header.header_size);
            let mut buf = vec![0_u8; payload_size];
            reader
                .read_exact(&mut buf)
                .map_err(|e| MediaInfoError::Io(e.to_string()))?;
            output.extend_from_slice(&buf);
            pos = start.saturating_add(header.size);
        } else if header.size == 0 {
            break;
        } else {
            let box_end = start.checked_add(header.size).ok_or_else(|| {
                MediaInfoError::Parse(format!(
                    "MP4 box {} size overflow",
                    fourcc_to_string(header.name)
                ))
            })?;
            reader
                .seek(SeekFrom::Start(box_end))
                .map_err(|e| MediaInfoError::Io(e.to_string()))?;
            pos = box_end;
        }

        if header.size == 0 {
            break;
        }
    }

    Ok((output, input_len, budget_exhausted))
}

fn should_copy_top_level_box(name: &[u8; 4]) -> bool {
    matches!(
        name,
        b"ftyp" | b"moov" | b"styp" | b"sidx" | b"moof" | b"mfra"
    )
}

fn sanitize_prepared_mp4_metadata(data: &mut Vec<u8>) {
    let range = Range {
        start: 0,
        end: data.len(),
    };
    sanitize_mp4_box_range(data, range, 0);
}

fn sanitize_mp4_box_range(data: &mut Vec<u8>, mut range: Range<usize>, depth: usize) -> usize {
    if !mp4_box_name_present(&data[range.start..range.end], &[*b"hdlr"]) {
        return 0;
    }

    let mut pos = range.start;
    let mut total_delta = 0;

    while pos < range.end {
        let Some(header) = read_box_header_from_bytes(&data[pos..range.end]) else {
            break;
        };
        let mut box_size = header.size as usize;
        if box_size < header.header_size || pos + box_size > range.end {
            break;
        }

        let box_delta = if &header.name == b"hdlr" {
            sanitize_hdlr_box(data, pos, header)
        } else if depth < MP4_BOX_MAX_DEPTH
            && let Some(child_range) = mp4_child_range(pos, header, box_size)
        {
            let child_delta = sanitize_mp4_box_range(data, child_range, depth + 1);
            if child_delta > 0 {
                box_size += child_delta;
                write_box_size_at(data, pos, header.header_size, box_size as u64);
            }
            child_delta
        } else {
            0
        };

        box_size += if &header.name == b"hdlr" {
            box_delta
        } else {
            0
        };
        pos += box_size;
        range.end += box_delta;
        total_delta += box_delta;
    }

    total_delta
}

fn mp4_child_range(
    box_start: usize,
    header: Mp4BoxHeader,
    box_size: usize,
) -> Option<Range<usize>> {
    let payload_start = match &header.name {
        b"meta" if box_size >= header.header_size + 4 => box_start + header.header_size + 4,
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"udta" | b"tref" | b"moof" | b"traf"
        | b"mfra" => box_start + header.header_size,
        _ => return None,
    };
    Some(Range {
        start: payload_start,
        end: box_start + box_size,
    })
}

fn sanitize_hdlr_box(data: &mut Vec<u8>, box_start: usize, header: Mp4BoxHeader) -> usize {
    let payload_start = box_start + header.header_size;
    let box_end = box_start + header.size as usize;
    if box_end > data.len() || box_end < payload_start {
        return 0;
    }

    if box_end >= payload_start + 8 {
        data[payload_start + 4..payload_start + 8].fill(0);
    }
    if box_end >= payload_start + 24 {
        data[payload_start + 12..payload_start + 24].fill(0);
    }

    let name_start = payload_start + 24;
    if box_end > name_start && data[name_start..box_end].contains(&0) {
        return 0;
    }

    data.insert(box_end, 0);
    write_box_size_at(data, box_start, header.header_size, header.size + 1);
    1
}

fn write_box_size_at(data: &mut [u8], box_start: usize, header_size: usize, new_size: u64) {
    match header_size {
        8 => {
            if let Ok(size32) = u32::try_from(new_size) {
                data[box_start..box_start + 4].copy_from_slice(&size32.to_be_bytes());
            }
        }
        16 => {
            data[box_start..box_start + 4].copy_from_slice(&1_u32.to_be_bytes());
            data[box_start + 8..box_start + 16].copy_from_slice(&new_size.to_be_bytes());
        }
        _ => {}
    }
}

fn build_mp4_tracks(
    ctx: &MediaContext,
    metadata_by_track: &HashMap<u32, Mp4TrackMetadata>,
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
            metadata: meta.map(|meta| meta.details.clone()).unwrap_or_default(),
            kind,
            codec_id: meta
                .and_then(|m| m.sample_entry_fourcc.clone())
                .unwrap_or_else(|| "unknown".into()),
            codec_name: None,
            audio_profile: None,
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
                raw.channels = mp4_audio_channels(
                    &raw.codec_id,
                    raw.codec_private.as_deref(),
                    Some(audio.channelcount),
                );
                if audio.samplerate.is_finite()
                    && audio.samplerate > 0.0
                    && audio.samplerate <= u32::MAX as f64
                {
                    raw.metadata.sample_rate = Some(audio.samplerate.round() as u32);
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

        if kind == TrackKind::Audio && raw.channels.is_none() {
            raw.channels = mp4_audio_channels(&raw.codec_id, raw.codec_private.as_deref(), None);
        }
        raw.metadata.duration_seconds =
            track_duration_seconds(track).filter(|duration| *duration > 0.0);
        if kind == TrackKind::Video {
            raw.metadata.declared_frame_rate = uniform_sample_frame_rate(track);
            let (rate, variable) = sample_timing_facts(track);
            raw.metadata.observed_frame_rate = rate;
            raw.metadata.variable_frame_rate = variable;
            raw.metadata.rotation_degrees = track
                .tkhd
                .as_ref()
                .and_then(|header| matrix_rotation(&header.matrix));
            if let (Some(width), Some(height), Some(sar)) =
                (raw.width, raw.height, raw.metadata.sample_aspect_ratio)
                && width > 0
                && height > 0
                && sar.numerator > 0
            {
                raw.metadata.display_aspect_ratio = i64::from(width)
                    .checked_mul(sar.numerator)
                    .zip((height as u64).checked_mul(sar.denominator))
                    .and_then(|(numerator, denominator)| {
                        scryer_media_model::Rational::new(numerator, denominator)
                    });
            }
        }

        if let Some(bit_rate_bps) = accounted_track_bitrate(track) {
            raw.bit_rate_bps = Some(bit_rate_bps);
            raw.metadata.bitrate_provenance = scryer_media_model::Provenance::Observed;
        }

        if raw.codec_name.is_none() {
            raw.codec_name = normalize_codec_name(&raw.codec_id);
        }
        raw.audio_profile = detect_header_audio_profile(
            &raw.codec_id,
            raw.codec_name.as_deref(),
            raw.codec_private.as_deref(),
        );

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
                metadata: metadata.details.clone(),
                kind: TrackKind::Subtitle,
                codec_name: normalize_codec_name(&codec_id),
                codec_id,
                audio_profile: None,
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
            },
        });
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
    matches!(sample_entry, "text" | "tx3g" | "wvtt" | "stpp" | "c608")
}

fn parse_mp4_metadata(data: &[u8]) -> ParsedMp4Metadata {
    let mut metadata = ParsedMp4Metadata::default();
    if !mp4_box_name_present(data, &[*b"moov"]) {
        return metadata;
    }
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"moov" {
            parse_moov(payload, &mut metadata.tracks);
            metadata
                .chapters
                .merge(parse_mp4_chapter_metadata_from_moov_payload(payload));
        }
    });
    metadata
}

fn count_mp4_chapters(metadata: &Mp4ChapterMetadata, ctx: &MediaContext) -> i32 {
    if let Some(count) = metadata.chpl_count {
        return count;
    }

    metadata
        .chapter_track_ids
        .iter()
        .filter_map(|track_id| {
            ctx.tracks
                .iter()
                .find(|track| track.track_id == Some(*track_id))
                .and_then(mp4_track_sample_count)
        })
        .sum()
}

#[cfg(test)]
fn parse_mp4_chapter_metadata(data: &[u8]) -> Mp4ChapterMetadata {
    let mut metadata = Mp4ChapterMetadata::default();
    if !mp4_box_name_present(data, &[*b"chpl", *b"chap"]) {
        return metadata;
    }
    walk_mp4_boxes(
        data,
        MP4_BOX_MAX_DEPTH,
        |header, payload, _depth| match &header.name {
            b"moov" | b"udta" | b"trak" | b"tref" => Some(payload),
            b"meta" => payload.get(4..),
            _ => None,
        },
        |header, payload, _depth| match &header.name {
            b"chpl" if metadata.chpl_count.is_none() => {
                metadata.read_chpl(payload);
            }
            b"chap" => {
                metadata
                    .chapter_track_ids
                    .extend(parse_chap_track_ids(payload));
            }
            _ => {}
        },
    );
    metadata.chapter_track_ids.sort_unstable();
    metadata.chapter_track_ids.dedup();
    metadata
}

fn parse_mp4_chapter_metadata_from_moov_payload(data: &[u8]) -> Mp4ChapterMetadata {
    let mut metadata = Mp4ChapterMetadata::default();
    if !mp4_box_name_present(data, &[*b"chpl", *b"chap"]) {
        return metadata;
    }
    walk_mp4_boxes(
        data,
        MP4_BOX_MAX_DEPTH,
        |header, payload, _depth| match &header.name {
            b"udta" | b"trak" | b"tref" => Some(payload),
            b"meta" => payload.get(4..),
            _ => None,
        },
        |header, payload, _depth| match &header.name {
            b"chpl" if metadata.chpl_count.is_none() => {
                metadata.read_chpl(payload);
            }
            b"chap" => {
                metadata
                    .chapter_track_ids
                    .extend(parse_chap_track_ids(payload));
            }
            _ => {}
        },
    );
    metadata.chapter_track_ids.sort_unstable();
    metadata.chapter_track_ids.dedup();
    metadata
}

impl Mp4ChapterMetadata {
    fn read_chpl(&mut self, payload: &[u8]) {
        if let Some(entries) = chapters::nero(payload) {
            self.chpl_count = i32::try_from(entries.len()).ok();
            self.entries = entries;
        } else {
            self.incomplete = true;
        }
    }
    fn merge(&mut self, mut other: Self) {
        if self.chpl_count.is_none() {
            self.chpl_count = other.chpl_count;
            self.entries = other.entries;
        }
        self.incomplete |= other.incomplete;
        self.chapter_track_ids.append(&mut other.chapter_track_ids);
        self.chapter_track_ids.sort_unstable();
        self.chapter_track_ids.dedup();
    }
}

fn parse_chap_track_ids(data: &[u8]) -> Vec<u32> {
    data.as_chunks::<4>()
        .0
        .iter()
        .filter_map(|chunk| read_be_u32(chunk))
        .collect()
}

fn mp4_track_sample_count(track: &mp4parse::Track) -> Option<i32> {
    if let Some(stsz) = track.stsz.as_ref() {
        if !stsz.sample_sizes.is_empty() {
            return i32::try_from(stsz.sample_sizes.len()).ok();
        }
        if stsz.sample_size > 0 {
            let count: u64 = track
                .stts
                .as_ref()
                .map(|stts| {
                    stts.samples
                        .iter()
                        .map(|sample| u64::from(sample.sample_count))
                        .sum()
                })
                .unwrap_or(0);
            return i32::try_from(count).ok();
        }
    }

    track.stts.as_ref().and_then(|stts| {
        let count: u64 = stts
            .samples
            .iter()
            .map(|sample| u64::from(sample.sample_count))
            .sum();
        i32::try_from(count).ok()
    })
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

/// Account for encoded samples using matching size and decode-time tables.
fn accounted_track_bitrate(track: &mp4parse::Track) -> Option<i64> {
    let stsz = track.stsz.as_ref()?;
    let scale = track.timescale?.0;
    let (sample_count, ticks) =
        track
            .stts
            .as_ref()?
            .samples
            .iter()
            .try_fold((0_u64, 0_u64), |(count, ticks), entry| {
                if entry.sample_count == 0 {
                    return None;
                }
                Some((
                    count.checked_add(u64::from(entry.sample_count))?,
                    ticks.checked_add(
                        u64::from(entry.sample_count).checked_mul(u64::from(entry.sample_delta))?,
                    )?,
                ))
            })?;
    let total_bytes = if stsz.sample_size > 0 {
        u64::from(stsz.sample_size).checked_mul(sample_count)?
    } else {
        if stsz.sample_sizes.len() as u64 != sample_count {
            return None;
        }
        stsz.sample_sizes
            .iter()
            .try_fold(0_u64, |sum, &size| sum.checked_add(u64::from(size)))?
    };
    if total_bytes == 0 || ticks == 0 || scale == 0 {
        return None;
    }
    let bitrate = u128::from(total_bytes)
        .checked_mul(8)?
        .checked_mul(u128::from(scale))?
        / u128::from(ticks);
    i64::try_from(bitrate).ok().filter(|bitrate| *bitrate > 0)
}

fn track_duration_seconds(track: &mp4parse::Track) -> Option<f64> {
    let ts = track.timescale.as_ref().map(|TrackTimeScale(t, _)| *t)?;
    if ts == 0 {
        return None;
    }
    let duration = track.duration.as_ref().map(|d| d.0)?;
    Some(duration as f64 / ts as f64)
}

fn matrix_rotation(matrix: &mp4parse::Matrix) -> Option<f64> {
    let (a, b, c, d) = (
        f64::from(matrix.a),
        f64::from(matrix.b),
        f64::from(matrix.c),
        f64::from(matrix.d),
    );
    if matrix.u != 0 || matrix.v != 0 || matrix.w != 1 << 30 || a * d - b * c <= 0.0 {
        return None;
    }
    // Shears and reflections do not have a single lossless rotation value.
    let scale = (a.hypot(b) * c.hypot(d)).max(1.0);
    if (a * c + b * d).abs() > scale * 0.0001 {
        return None;
    }
    Some(-b.atan2(a).to_degrees())
}

fn uniform_sample_frame_rate(track: &mp4parse::Track) -> Option<scryer_media_model::Rational> {
    let scale = track.timescale?.0;
    let samples = &track.stts.as_ref()?.samples;
    let delta = samples.first()?.sample_delta;
    if scale == 0
        || delta == 0
        || samples
            .iter()
            .any(|sample| sample.sample_count == 0 || sample.sample_delta != delta)
    {
        return None;
    }
    scryer_media_model::Rational::new(i64::try_from(scale).ok()?, u64::from(delta))
}

fn sample_timing_facts(
    track: &mp4parse::Track,
) -> (Option<scryer_media_model::Rational>, Option<bool>) {
    use scryer_media_model::Rational;
    let facts = (|| {
        let scale = track.timescale?.0;
        if scale == 0 {
            return None;
        }
        let timing = &track.stts.as_ref()?.samples;
        let count = timing.iter().try_fold(0_u64, |count, sample| {
            count.checked_add(u64::from(sample.sample_count))
        })?;
        if count < 2
            || timing
                .iter()
                .any(|entry| entry.sample_count == 0 || entry.sample_delta == 0)
        {
            return None;
        }
        if track.ctts.is_none() {
            let duration = timing.iter().try_fold(0_u64, |sum, entry| {
                sum.checked_add(u64::from(entry.sample_count) * u64::from(entry.sample_delta))
            })?;
            let rate = Rational::new(i64::try_from(count.checked_mul(scale)?).ok()?, duration)?;
            let variable = timing
                .iter()
                .any(|entry| entry.sample_delta != timing[0].sample_delta);
            return Some((Some(rate), Some(variable)));
        }
        let offsets = &track.ctts.as_ref()?.samples;
        let offset_count = offsets.iter().try_fold(0_u64, |count, entry| {
            count.checked_add(u64::from(entry.sample_count))
        })?;
        if count != offset_count || offsets.iter().any(|entry| entry.sample_count == 0) {
            return None;
        }
        let mut presentation = Vec::with_capacity(count.min(4096) as usize);
        let mut decode = 0_i64;
        let mut offset_index = 0;
        let mut offset_used = 0;
        'entries: for entry in timing {
            for _ in 0..entry.sample_count.min(4096) {
                if presentation.len() == 4096 {
                    break 'entries;
                }
                let offset = offsets.get(offset_index)?;
                let composition = match offset.time_offset {
                    mp4parse::TimeOffsetVersion::Version0(value) => i64::from(value),
                    mp4parse::TimeOffsetVersion::Version1(value) => i64::from(value),
                };
                presentation.push(decode.checked_add(composition)?);
                decode = decode.checked_add(i64::from(entry.sample_delta))?;
                offset_used += 1;
                if offset_used == offset.sample_count {
                    offset_index += 1;
                    offset_used = 0;
                }
            }
        }
        presentation.sort_unstable();
        let complete = presentation.len() as u64 == count;
        // Discard boundary samples when a bounded window can cut a reorder group.
        let times = if !complete && presentation.len() > 64 {
            &presentation[16..presentation.len() - 16]
        } else {
            &presentation[..]
        };
        let span = times.last()?.checked_sub(*times.first()?)?;
        if span <= 0 {
            return None;
        }
        let rate = Rational::new(
            i64::try_from((times.len() as u64 - 1).checked_mul(scale)?).ok()?,
            span as u64,
        )?;
        let mut deltas = times.windows(2).map(|pair| pair[1] - pair[0]);
        let first = deltas.next()?;
        if first <= 0 {
            return None;
        }
        let variable = deltas.any(|delta| delta != first);
        Some((
            Some(rate),
            if variable {
                Some(true)
            } else {
                complete.then_some(false)
            },
        ))
    })();
    facts.unwrap_or((None, None))
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

    let total_samples = stts.samples.iter().try_fold(0_u64, |total, sample| {
        total.checked_add(u64::from(sample.sample_count))
    })?;
    if total_samples == 0 {
        return None;
    }

    let dominant = stts.samples.iter().max_by_key(|s| s.sample_count)?;
    if u128::from(dominant.sample_count) * 10 >= u128::from(total_samples) * 9
        && dominant.sample_delta > 0
    {
        let fps = ts as f64 / f64::from(dominant.sample_delta);
        if fps > 0.0 && fps < 1000.0 {
            return Some(fps);
        }
    }

    let total_delta: u64 = stts.samples.iter().try_fold(0_u64, |total, sample| {
        total.checked_add(u64::from(sample.sample_count) * u64::from(sample.sample_delta))
    })?;
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

#[cfg(test)]
mod tests;
