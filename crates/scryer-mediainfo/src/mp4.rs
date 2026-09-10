use std::collections::{HashMap, HashSet, hash_map::Entry};
mod chapters;
mod timing;
use std::io::{Cursor, Read, Seek, SeekFrom};

use core::range::Range;

use mp4parse::{
    AudioCodecSpecific, CodecType, MediaContext, MediaTimeScale, SampleEntry, TrackTimeScale,
    TrackType, VideoCodecSpecific,
};

use crate::AnalysisProfile;
use crate::MediaInfoError;
use crate::codec::{
    audio_profile_probe_spec, detect_audio_profile_from_probe_bytes, detect_header_audio_profile,
    merge_audio_profile, normalize_codec_name,
};
use crate::probe::{ProbeStats, TrackedReader};
use crate::scan;
use crate::types::{RawContainer, RawTrack, TrackKind};

const HDR10PLUS_SAMPLE_LIMIT_BYTES: u64 = 4 * 1024 * 1024;
const MP4_DOVI_TYPES: [&str; 2] = ["dvcC", "dvvC"];
const MOV_TKHD_FLAG_ENABLED: u32 = 0x000001;
const MP4_BOX_MAX_DEPTH: usize = 10;
const MP4_KEEP_BOX_MAX_BYTES: u64 = 256 * 1024 * 1024;
const MP4_METADATA_OUTPUT_MAX_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug)]
struct PreparedMp4 {
    metadata: Vec<u8>,
    file_len_hint: u64,
    budget_exhausted: bool,
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
    details: scryer_media_types::StreamMetadata,
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
    entries: Vec<scryer_media_types::Chapter>,
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
            let mut details = scryer_media_types::AnalysisDetails::default();
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
        sonarr_mp4_duration_seconds(&ctx, &metadata_by_track, format_duration_seconds);

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
                track.raw.metadata.bitrate_provenance = scryer_media_types::Provenance::Observed;
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
    let mut report = scryer_media_types::ProbeReport::default();
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
                    report.status = scryer_media_types::ProbeStatus::Incomplete;
                    report.budget_exhausted |= budget;
                    report.warnings.push(scryer_media_types::ProbeWarning {
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
        report.status = scryer_media_types::ProbeStatus::Incomplete;
        report.warnings.push(scryer_media_types::ProbeWarning {
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
        details: scryer_media_types::AnalysisDetails {
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

fn report_fragment_budget(report: &mut scryer_media_types::ProbeReport, exhausted: bool) {
    if !exhausted {
        return;
    }
    report.status = scryer_media_types::ProbeStatus::Incomplete;
    report.budget_exhausted = true;
    report.warnings.push(scryer_media_types::ProbeWarning {
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

fn sonarr_mp4_duration_seconds(
    ctx: &MediaContext,
    metadata_by_track: &HashMap<u32, Mp4TrackMetadata>,
    format_duration: Option<f64>,
) -> Option<f64> {
    let audio = first_mp4_track_duration_seconds(ctx, metadata_by_track, TrackKind::Audio);
    let video = primary_mp4_video_duration_seconds(ctx, metadata_by_track);
    best_sonarr_runtime(audio, video, format_duration)
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

fn best_sonarr_runtime(audio: Option<f64>, video: Option<f64>, format: Option<f64>) -> Option<f64> {
    if video.unwrap_or_default() > 0.0 {
        video
    } else if audio.unwrap_or_default() > 0.0 {
        audio
    } else {
        format.filter(|duration| *duration > 0.0)
    }
}

fn report_container_budget(report: &mut scryer_media_types::ProbeReport) {
    report.status = scryer_media_types::ProbeStatus::Incomplete;
    report.budget_exhausted = true;
    report.warnings.push(scryer_media_types::ProbeWarning {
        code: "mp4_container_scan_budget".into(),
        message: "MP4 catalog inspection stops after 128 top-level boxes or 16 MiB of metadata; incomplete fragment timing and bitrate remain unknown".into(),
        ..Default::default()
    });
}

fn prepare_mp4_metadata(
    file: &mut dyn crate::source::MediaSource,
) -> Result<PreparedMp4, MediaInfoError> {
    let mut reader = TrackedReader::new(file);
    let (metadata, file_len_hint, budget_exhausted) = prepare_mp4_metadata_from_reader(&mut reader)?;
    Ok(PreparedMp4 {
        metadata,
        file_len_hint,
        budget_exhausted,
        stats: reader.stats(),
    })
}

fn prepare_mp4_metadata_from_reader<R: Read + Seek>(
    reader: &mut TrackedReader<R>,
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
            let retained_size = if header.size == 0 { input_len - start } else { header.size };
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
                        scryer_media_types::Rational::new(numerator, denominator)
                    });
            }
        }

        if let Some(bit_rate_bps) = accounted_track_bitrate(track) {
            raw.bit_rate_bps = Some(bit_rate_bps);
            raw.metadata.bitrate_provenance = scryer_media_types::Provenance::Observed;
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
                forced: metadata.forced,
                default_track: metadata.default_track,
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
    data.chunks_exact(4).filter_map(read_be_u32).collect()
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

fn mp4_audio_channels(
    codec_id: &str,
    codec_private: Option<&[u8]>,
    sample_entry_channels: Option<u32>,
) -> Option<i32> {
    let codec_specific_channels = codec_private.and_then(|codec_private| match codec_id {
        "mp4a" => parse_aac_audio_specific_config_channels(codec_private),
        "ac-3" => parse_dac3_channels(codec_private),
        "ec-3" => parse_dec3_channels(codec_private),
        _ => None,
    });
    let sample_entry_channels = sample_entry_channels
        .filter(|channels| *channels > 0)
        .map(|channels| channels as i32);

    match (codec_specific_channels, sample_entry_channels) {
        (Some(codec_specific_channels), Some(sample_entry_channels)) => {
            Some(codec_specific_channels.max(sample_entry_channels))
        }
        (Some(codec_specific_channels), None) => Some(codec_specific_channels),
        (None, Some(sample_entry_channels)) => Some(sample_entry_channels),
        (None, None) => None,
    }
}

fn parse_aac_audio_specific_config_channels(data: &[u8]) -> Option<i32> {
    const AAC_CHANNEL_CONFIGS: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 8, 0, 0, 0, 7, 8, 0, 8, 0];

    let mut bits = BitCursor::new(data);
    let audio_object_type = bits.read_aac_audio_object_type()?;
    bits.read_aac_sample_rate()?;
    let mut channel_config = bits.read_bits(4)? as usize;

    if matches!(audio_object_type, 5 | 29) {
        bits.read_aac_sample_rate()?;
        let ext_audio_object_type = bits.read_aac_audio_object_type()?;
        if ext_audio_object_type == 22 {
            channel_config = bits.read_bits(4)? as usize;
        }
    }

    let channels = *AAC_CHANNEL_CONFIGS.get(channel_config)?;
    (channels > 0).then_some(i32::from(channels))
}

fn parse_dac3_channels(data: &[u8]) -> Option<i32> {
    let mut bits = BitCursor::new(data);
    bits.read_bits(2)?;
    bits.read_bits(5)?;
    bits.read_bits(3)?;
    let acmod = bits.read_bits(3)? as usize;
    let lfeon = bits.read_bits(1)? as i32;
    Some(ac3_channel_count(acmod) + lfeon)
}

fn parse_dec3_channels(data: &[u8]) -> Option<i32> {
    let mut bits = BitCursor::new(data);
    bits.read_bits(13)?;
    let num_ind_sub = bits.read_bits(3)? as usize + 1;
    if num_ind_sub == 0 {
        return None;
    }

    bits.read_bits(2)?;
    bits.read_bits(5)?;
    bits.read_bits(1)?;
    bits.read_bits(1)?;
    bits.read_bits(3)?;
    let acmod = bits.read_bits(3)? as usize;
    let lfeon = bits.read_bits(1)? as i32;
    Some(ac3_channel_count(acmod) + lfeon)
}

fn ac3_channel_count(acmod: usize) -> i32 {
    const AC3_CHANNELS_BY_ACMOD: [i32; 8] = [2, 1, 2, 3, 3, 4, 4, 5];
    AC3_CHANNELS_BY_ACMOD.get(acmod).copied().unwrap_or(0)
}

struct BitCursor<'a> {
    data: &'a [u8],
    bit_offset: usize,
}

impl<'a> BitCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_offset: 0,
        }
    }

    fn read_bits(&mut self, count: usize) -> Option<u32> {
        if count == 0 || count > 32 {
            return None;
        }

        let mut value = 0_u32;
        for _ in 0..count {
            let byte = *self.data.get(self.bit_offset / 8)?;
            let shift = 7 - (self.bit_offset % 8);
            value = (value << 1) | u32::from((byte >> shift) & 1);
            self.bit_offset += 1;
        }
        Some(value)
    }

    fn read_aac_audio_object_type(&mut self) -> Option<u8> {
        let object_type = self.read_bits(5)? as u8;
        if object_type == 31 {
            Some(32 + self.read_bits(6)? as u8)
        } else {
            Some(object_type)
        }
    }

    fn read_aac_sample_rate(&mut self) -> Option<u32> {
        const AAC_SAMPLE_RATES: [u32; 13] = [
            96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
            8_000, 7_350,
        ];

        let sample_rate_index = self.read_bits(4)? as usize;
        if sample_rate_index == 0xF {
            self.read_bits(24)
        } else {
            AAC_SAMPLE_RATES.get(sample_rate_index).copied()
        }
    }
}

fn parse_moov(data: &[u8], metadata: &mut HashMap<u32, Mp4TrackMetadata>) {
    if !mp4_box_name_present(data, &[*b"trak"]) {
        return;
    }
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
    if mp4_box_name_present(data, &[*b"tkhd", *b"mdia", *b"udta"]) {
        for_each_mp4_box(data, |header, payload| match &header.name {
            b"tkhd" => {
                apply_tkhd_metadata(payload, &mut track);
            }
            b"mdia" => parse_mdia(payload, &mut track),
            b"udta" => parse_udta(payload, &mut track),
            _ => {}
        });
    }
    parse_kind_boxes(data, &mut track);
    track.details.id = track.track_id.map(|id| id.to_string());
    track.details.original_language = track.language.clone();
    if track.language.is_some() {
        track.details.language_provenance = scryer_media_types::Provenance::Container;
    }
    track.details.disposition.default = Some(track.default_track);
    track.details.disposition.forced = Some(track.forced);
    track
}

fn parse_kind_boxes(data: &[u8], track: &mut Mp4TrackMetadata) {
    if !mp4_box_name_present(data, &[*b"kind"]) {
        return;
    }
    walk_mp4_boxes(
        data,
        MP4_BOX_MAX_DEPTH,
        |header, payload, _depth| match &header.name {
            b"mdia" | b"minf" | b"stbl" | b"udta" => Some(payload),
            b"meta" => payload.get(4..),
            _ => None,
        },
        |header, payload, _depth| {
            if &header.name == b"kind" {
                apply_kind_metadata(payload, track);
            }
        },
    );
}

fn parse_mdia(data: &[u8], track: &mut Mp4TrackMetadata) {
    if !mp4_box_name_present(data, &[*b"mdhd", *b"hdlr", *b"minf", *b"elng"]) {
        return;
    }
    let mut extended_language = None;
    for_each_mp4_box(data, |header, payload| match &header.name {
        b"mdhd" => {
            track.language = parse_mdhd_language(payload);
        }
        b"hdlr" => {
            track.handler_type = parse_hdlr_type(payload);
        }
        b"minf" => parse_minf(payload, track),
        b"elng" => {
            extended_language = payload
                .get(..4)
                .filter(|flags| *flags == [0, 0, 0, 0])
                .and_then(|_| payload.get(4..))
                .and_then(|bytes| bytes.strip_suffix(&[0]))
                .filter(|bytes| {
                    !bytes.is_empty()
                        && bytes.len() <= 255
                        && bytes
                            .iter()
                            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
                })
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .map(str::to_owned);
        }
        _ => {}
    });
    if extended_language.is_some() {
        track.language = extended_language;
    }
}

fn parse_udta(data: &[u8], track: &mut Mp4TrackMetadata) {
    if !mp4_box_name_present(data, &[*b"name"]) {
        return;
    }
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"name" {
            track.name = parse_name_box(payload);
        }
    });
}

fn parse_minf(data: &[u8], track: &mut Mp4TrackMetadata) {
    if !mp4_box_name_present(data, &[*b"stbl"]) {
        return;
    }
    for_each_mp4_box(data, |header, payload| {
        if &header.name == b"stbl" {
            parse_stbl(payload, track);
        }
    });
}

fn parse_stbl(data: &[u8], track: &mut Mp4TrackMetadata) {
    if !mp4_box_name_present(data, &[*b"stsd"]) {
        return;
    }
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
                let children = &payload[child_offset..];
                if mp4_box_name_present(
                    children,
                    &[
                        *b"avcC", *b"hvcC", *b"av1C", *b"dvcC", *b"dvvC", *b"colr", *b"pasp",
                        *b"vpcC",
                    ],
                ) {
                    for_each_mp4_box(children, |child, child_payload| {
                        let child_name = fourcc_to_string(child.name);
                        match child_name.as_str() {
                            "avcC" | "hvcC" | "av1C" | "vpcC" => {
                                track.codec_private = Some(child_payload.to_vec());
                            }
                            "colr" => {
                                if child_payload.len() >= 10
                                    && matches!(&child_payload[..4], b"nclx" | b"nclc")
                                {
                                    track.details.color = scryer_media_types::ColorMetadata {
                                        primaries: read_be_u16(&child_payload[4..6])
                                            .map(u32::from)
                                            .filter(|value| *value != 2),
                                        transfer: read_be_u16(&child_payload[6..8])
                                            .map(u32::from)
                                            .filter(|value| *value != 2),
                                        matrix: read_be_u16(&child_payload[8..10])
                                            .map(u32::from)
                                            .filter(|value| *value != 2),
                                        full_range: (child_payload[..4] == *b"nclx")
                                            .then(|| {
                                                child_payload.get(10).map(|byte| byte & 128 != 0)
                                            })
                                            .flatten(),
                                        provenance: scryer_media_types::Provenance::Container,
                                        ..Default::default()
                                    };
                                }
                            }
                            "pasp" if child_payload.len() >= 8 => {
                                track.details.sample_aspect_ratio =
                                    read_be_u32(&child_payload[..4])
                                        .zip(read_be_u32(&child_payload[4..8]))
                                        .and_then(|(width, height)| {
                                            scryer_media_types::Rational::new(
                                                i64::from(width),
                                                u64::from(height),
                                            )
                                        });
                            }
                            t if MP4_DOVI_TYPES.contains(&t) => {
                                track.dovi_config = Some(child_payload.to_vec());
                            }
                            _ => {}
                        }
                    });
                }
            }
        } else if is_audio_sample_entry(&fourcc)
            && let Some(child_offset) = audio_sample_entry_child_offset(payload)
        {
            let children = &payload[child_offset..];
            if mp4_box_name_present(children, &[*b"dac3", *b"dec3", *b"dOps", *b"dfLa"]) {
                for_each_mp4_box(children, |child, child_payload| {
                    let child_name = fourcc_to_string(child.name);
                    if matches!(child_name.as_str(), "dac3" | "dec3" | "dOps") {
                        track.codec_private = Some(child_payload.to_vec());
                    } else if child_name == "dfLa" && child_payload.starts_with(&[0, 0, 0, 0]) {
                        let mut metadata = b"fLaC".to_vec();
                        metadata.extend_from_slice(&child_payload[4..]);
                        track.codec_private = Some(metadata);
                    }
                });
            }
        }
    }
}

fn audio_sample_entry_child_offset(payload: &[u8]) -> Option<usize> {
    if payload.len() < 28 {
        return None;
    }
    let version = read_be_u16(&payload[8..10])?;
    let offset = match version {
        0 => 28,
        1 => 44,
        2 => 64,
        _ => return None,
    };
    payload.get(offset..).map(|_| offset)
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
    if data.len() < 6 {
        return;
    }
    if !matches!(data.get(0..4), Some([0, 0, 0, 0])) {
        return;
    }
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
    if code < 0x400 {
        return match code {
            0 => Some("eng".to_string()),
            6 => Some("spa".to_string()),
            11 => Some("jpn".to_string()),
            _ => None,
        };
    }

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

fn walk_mp4_boxes<'a, FDescend, FVisit>(
    data: &'a [u8],
    max_depth: usize,
    mut should_descend: FDescend,
    mut visit: FVisit,
) where
    FDescend: FnMut(Mp4BoxHeader, &'a [u8], usize) -> Option<&'a [u8]>,
    FVisit: FnMut(Mp4BoxHeader, &'a [u8], usize),
{
    let mut stack = vec![(data, 0_usize)];

    while let Some((mut current, depth)) = stack.pop() {
        let mut children = Vec::new();

        while let Some(header) = read_box_header_from_bytes(current) {
            let size = header.size as usize;
            if size < header.header_size || size > current.len() {
                break;
            }

            let payload = &current[header.header_size..size];
            visit(header, payload, depth);

            if depth < max_depth
                && let Some(child_payload) = should_descend(header, payload, depth)
                && !child_payload.is_empty()
            {
                children.push((child_payload, depth + 1));
            }

            if size == current.len() {
                break;
            }
            current = &current[size..];
        }

        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
}

fn read_top_level_box_header<R: Read>(
    reader: &mut R,
) -> Result<Option<(Mp4BoxHeader, Vec<u8>)>, MediaInfoError> {
    let mut header = [0_u8; 8];
    let mut read = 0;
    while read < header.len() {
        let bytes_read = reader
            .read(&mut header[read..])
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        if bytes_read == 0 {
            if read == 0 {
                return Ok(None);
            }
            return Err(MediaInfoError::Parse("truncated MP4 box header".into()));
        }
        read += bytes_read;
    }

    let size32 = u32::from_be_bytes(header[0..4].try_into().unwrap()) as u64;
    let name: [u8; 4] = header[4..8].try_into().unwrap();
    let mut size = size32;
    let mut header_size = 8;
    let mut raw = header.to_vec();

    if size32 == 1 {
        let mut extended = [0_u8; 8];
        reader
            .read_exact(&mut extended)
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        size = u64::from_be_bytes(extended);
        header_size = 16;
        raw.extend_from_slice(&extended);
    }

    if size != 0 && size < header_size as u64 {
        return Err(MediaInfoError::Parse(format!(
            "invalid MP4 box size {} for {}",
            size,
            fourcc_to_string(name)
        )));
    }

    Ok(Some((
        Mp4BoxHeader {
            name,
            size,
            header_size,
        },
        raw,
    )))
}

fn read_zero_sized_top_level_box<R: Read>(
    reader: &mut R,
    output: &mut Vec<u8>,
    pos: &mut u64,
) -> Result<(), MediaInfoError> {
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buf)
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        if read == 0 {
            return Ok(());
        }
        if output.len().saturating_add(read) > MP4_METADATA_OUTPUT_MAX_BYTES {
            return Err(MediaInfoError::Parse(
                "MP4 metadata output exceeds parser budget".into(),
            ));
        }
        output.extend_from_slice(&buf[..read]);
        *pos = (*pos).saturating_add(read as u64);
    }
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

fn mp4_box_name_present(data: &[u8], names: &[[u8; 4]]) -> bool {
    scan::find_mp4_box_name_candidate(data, 0, names).is_some()
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

fn uniform_sample_frame_rate(track: &mp4parse::Track) -> Option<scryer_media_types::Rational> {
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
    scryer_media_types::Rational::new(i64::try_from(scale).ok()?, u64::from(delta))
}

fn sample_timing_facts(
    track: &mp4parse::Track,
) -> (Option<scryer_media_types::Rational>, Option<bool>) {
    use scryer_media_types::Rational;
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

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct Mp4SampleReadKey {
    offset: u64,
    size: usize,
}

fn scan_mp4_sample_probes(
    file: &mut dyn crate::source::MediaSource,
    ctx: &MediaContext,
    tracks: &mut [ParsedMp4Track],
    profile: AnalysisProfile,
    report: &mut scryer_media_types::ProbeReport,
) -> Vec<scryer_media_types::CaptionService> {
    if profile.skips_deep_probes() {
        return Vec::new();
    }

    let audio_probe_needed = tracks.iter().any(|track| {
        track.raw.kind == TrackKind::Audio
            && matches!(
                track.raw.codec_name.as_deref(),
                Some("ac3" | "eac3" | "truehd" | "dts")
            )
            && audio_profile_probe_spec(track.raw.codec_name.as_deref()).prefix_bytes > 0
    });
    let hdr10plus_probe_needed = profile == AnalysisProfile::DefaultRich
        && tracks.iter().any(|track| {
            track.raw.kind == TrackKind::Video
                && matches!(
                    track.raw.codec_name.as_deref(),
                    Some(
                        "h264"
                            | "hevc"
                            | "av1"
                            | "vp8"
                            | "vp9"
                            | "mpeg1video"
                            | "mpeg2video"
                            | "mpeg4"
                            | "vc1"
                            | "mjpeg"
                    )
                )
        });
    if !audio_probe_needed && !hdr10plus_probe_needed {
        return Vec::new();
    }

    let mut sample_cache = HashMap::new();
    let mut captions = Vec::new();

    if audio_probe_needed {
        scan_mp4_audio_profiles_with_cache(file, &mut sample_cache, ctx, tracks, report);
    }
    if hdr10plus_probe_needed {
        scan_mp4_video_metadata_with_cache(
            file,
            &mut sample_cache,
            ctx,
            tracks,
            report,
            &mut captions,
        );
    }
    captions
}

fn scan_mp4_video_metadata_with_cache(
    file: &mut dyn crate::source::MediaSource,
    sample_cache: &mut HashMap<Mp4SampleReadKey, Vec<u8>>,
    ctx: &MediaContext,
    tracks: &mut [ParsedMp4Track],
    report: &mut scryer_media_types::ProbeReport,
    captions: &mut Vec<scryer_media_types::CaptionService>,
) {
    let mut remaining = 8 * 1024 * 1024_usize;
    for parsed in tracks
        .iter_mut()
        .filter(|track| track.raw.kind == TrackKind::Video)
    {
        if !matches!(
            parsed.raw.codec_name.as_deref(),
            Some(
                "h264"
                    | "hevc"
                    | "av1"
                    | "vp8"
                    | "vp9"
                    | "mpeg1video"
                    | "mpeg2video"
                    | "mpeg4"
                    | "vc1"
                    | "mjpeg"
            )
        ) {
            continue;
        }
        let ranges = parsed
            .track_id
            .and_then(|id| ctx.tracks.iter().find(|track| track.track_id == Some(id)))
            .and_then(track_sample_ranges);
        let Some((ranges, complete)) = ranges else {
            sample_probe_warning(report, &parsed.raw, "sample_references_unavailable", false);
            continue;
        };
        if !complete {
            sample_probe_warning(report, &parsed.raw, "video_enrichment_sample_limit", true);
        }
        let nal_length_size = if parsed.raw.codec_name.as_deref() == Some("h264") {
            parsed
                .raw
                .codec_private
                .as_deref()
                .and_then(|bytes| bytes.get(4))
                .map_or(4, |byte| usize::from(byte & 3) + 1)
        } else {
            parsed
                .raw
                .codec_private
                .as_deref()
                .map(crate::codec::hevc_nal_length_size)
                .unwrap_or(4)
        };
        for (offset, size) in ranges {
            let limit = remaining.min(HDR10PLUS_SAMPLE_LIMIT_BYTES as usize);
            if size > limit {
                sample_probe_warning(report, &parsed.raw, "video_enrichment_byte_limit", true);
                continue;
            }
            remaining -= size;
            let Some(bytes) = read_cached_mp4_range(file, sample_cache, offset, size) else {
                sample_probe_warning(report, &parsed.raw, "sample_read_incomplete", false);
                continue;
            };
            match parsed.raw.codec_name.as_deref() {
                Some("av1") => crate::av1::enrich(&mut parsed.raw, &bytes, report),
                Some("h264" | "hevc") => crate::video_metadata::length_prefixed(
                    &bytes,
                    nal_length_size,
                    &mut parsed.raw,
                    report,
                    captions,
                ),
                _ => {
                    crate::legacy_video::enrich(&mut parsed.raw, &bytes);
                    crate::video_metadata::annex_b(&bytes, &mut parsed.raw, report, captions);
                }
            }
        }
    }
}

fn sample_probe_warning(
    report: &mut scryer_media_types::ProbeReport,
    track: &RawTrack,
    code: &str,
    budget: bool,
) {
    report.status = scryer_media_types::ProbeStatus::Incomplete;
    report.budget_exhausted |= budget;
    if !report
        .warnings
        .iter()
        .any(|warning| warning.code == code && warning.stream_id == track.metadata.id)
    {
        report.warnings.push(scryer_media_types::ProbeWarning {
            code: code.into(),
            message: "MP4 sample enrichment is incomplete; undetected metadata remains unknown"
                .into(),
            stream_id: track.metadata.id.clone(),
            ..Default::default()
        });
    }
}

/// Follow chunk mappings, including interleaved tracks, without walking payloads.
fn track_sample_ranges(track: &mp4parse::Track) -> Option<(Vec<(u64, usize)>, bool)> {
    track_sample_ranges_bounded(track, 16)
}

fn track_sample_ranges_bounded(
    track: &mp4parse::Track,
    limit: u32,
) -> Option<(Vec<(u64, usize)>, bool)> {
    let sizes = track.stsz.as_ref()?;
    let chunks = &track.stco.as_ref()?.offsets;
    let mapping = &track.stsc.as_ref()?.samples;
    if mapping.first()?.first_chunk != 1 {
        return None;
    }
    let count = if sizes.sample_size == 0 {
        sizes.sample_sizes.len() as u64
    } else {
        track
            .stts
            .as_ref()?
            .samples
            .iter()
            .try_fold(0_u64, |sum, sample| {
                sum.checked_add(u64::from(sample.sample_count))
            })?
    };
    let mut ranges = Vec::new();
    let mut map_index = 0;
    let mut sample_index = 0;
    for (chunk_index, chunk_offset) in chunks.iter().enumerate().take(limit as usize) {
        while let Some(next) = mapping.get(map_index + 1) {
            if next.first_chunk <= mapping[map_index].first_chunk {
                return None;
            }
            if u64::from(next.first_chunk) > chunk_index as u64 + 1 {
                break;
            }
            map_index += 1;
        }
        let entry = &mapping[map_index];
        if entry.samples_per_chunk == 0 || entry.sample_description_index != 1 {
            return None;
        }
        let mut offset = *chunk_offset;
        for _ in 0..entry.samples_per_chunk.min(limit) {
            if ranges.len() == limit as usize || sample_index as u64 == count {
                return Some((ranges, sample_index as u64 == count));
            }
            let size = if sizes.sample_size == 0 {
                *sizes.sample_sizes.get(sample_index)?
            } else {
                sizes.sample_size
            } as usize;
            if size == 0 {
                return None;
            }
            ranges.push((offset, size));
            offset = offset.checked_add(size as u64)?;
            sample_index += 1;
        }
        if entry.samples_per_chunk >= limit {
            break;
        }
    }
    if ranges.is_empty() {
        None
    } else {
        Some((ranges, sample_index as u64 == count))
    }
}

fn scan_mp4_audio_profiles_with_cache(
    file: &mut dyn crate::source::MediaSource,
    sample_cache: &mut HashMap<Mp4SampleReadKey, Vec<u8>>,
    ctx: &MediaContext,
    tracks: &mut [ParsedMp4Track],
    report: &mut scryer_media_types::ProbeReport,
) {
    let mut remaining = 4 * 1024 * 1024_usize;
    for parsed_track in tracks
        .iter_mut()
        .filter(|track| track.raw.kind == TrackKind::Audio)
    {
        if !matches!(
            parsed_track.raw.codec_name.as_deref(),
            Some("ac3" | "eac3" | "truehd" | "dts")
        ) {
            continue;
        }
        let probe_spec = audio_profile_probe_spec(parsed_track.raw.codec_name.as_deref());
        if probe_spec.prefix_bytes == 0 {
            continue;
        }

        let Some(mp4_track) = parsed_track
            .track_id
            .and_then(|track_id| {
                ctx.tracks
                    .iter()
                    .find(|track| track.track_id == Some(track_id))
            })
            .or_else(|| {
                ctx.tracks
                    .iter()
                    .find(|track| matches!(track.track_type, TrackType::Audio))
            })
        else {
            continue;
        };

        let Some((ranges, complete)) = track_sample_ranges(mp4_track) else {
            sample_probe_warning(
                report,
                &parsed_track.raw,
                "sample_references_unavailable",
                false,
            );
            continue;
        };
        if !complete {
            sample_probe_warning(
                report,
                &parsed_track.raw,
                "audio_enrichment_sample_limit",
                true,
            );
        }
        for (offset, size) in ranges {
            let cost = size
                .min(probe_spec.prefix_bytes)
                .saturating_add(size.min(probe_spec.suffix_bytes));
            let Some(left) = remaining.checked_sub(cost) else {
                sample_probe_warning(
                    report,
                    &parsed_track.raw,
                    "audio_enrichment_byte_limit",
                    true,
                );
                break;
            };
            remaining = left;
            let Some((prefix, suffix)) = read_audio_profile_probe_bytes_cached(
                file,
                sample_cache,
                offset,
                size as u64,
                probe_spec,
            ) else {
                sample_probe_warning(report, &parsed_track.raw, "sample_read_incomplete", false);
                continue;
            };

            let dolby_header = match parsed_track.raw.codec_name.as_deref() {
                Some("ac3") => crate::ts::find_ac3_header(&prefix),
                Some("eac3") => crate::ts::find_eac3_header(&prefix),
                _ => None,
            };
            if let Some(header) = dolby_header {
                header.apply(&mut parsed_track.raw);
            }
            merge_audio_profile(
                &mut parsed_track.raw.audio_profile,
                detect_audio_profile_from_probe_bytes(
                    parsed_track.raw.codec_name.as_deref(),
                    &prefix,
                    suffix.as_deref(),
                ),
            );
        }
    }
}

fn read_audio_profile_probe_bytes_cached(
    file: &mut dyn crate::source::MediaSource,
    sample_cache: &mut HashMap<Mp4SampleReadKey, Vec<u8>>,
    sample_offset: u64,
    sample_size: u64,
    spec: crate::codec::AudioProfileProbeSpec,
) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
    let prefix_size = sample_size.min(spec.prefix_bytes as u64) as usize;
    if prefix_size == 0 {
        return None;
    }

    let prefix = read_cached_mp4_range(file, sample_cache, sample_offset, prefix_size)?;

    let suffix = if spec.suffix_bytes > 0 {
        let suffix_size = sample_size.min(spec.suffix_bytes as u64) as usize;
        let suffix_offset = sample_offset + sample_size.saturating_sub(suffix_size as u64);
        if suffix_offset >= sample_offset + prefix_size as u64 {
            read_cached_mp4_range(file, sample_cache, suffix_offset, suffix_size)
        } else {
            None
        }
    } else {
        None
    };

    Some((prefix, suffix))
}

fn read_cached_mp4_range(
    file: &mut dyn crate::source::MediaSource,
    sample_cache: &mut HashMap<Mp4SampleReadKey, Vec<u8>>,
    offset: u64,
    size: usize,
) -> Option<Vec<u8>> {
    let key = Mp4SampleReadKey { offset, size };
    match sample_cache.entry(key) {
        Entry::Occupied(entry) => Some(entry.get().clone()),
        Entry::Vacant(entry) => {
            file.seek(SeekFrom::Start(offset)).ok()?;
            let mut bytes = vec![0_u8; size];
            file.read_exact(&mut bytes).ok()?;
            Some(entry.insert(bytes).clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing_track(entries: &[(u32, u32)]) -> mp4parse::Track {
        let mut track = mp4parse::Track {
            timescale: Some(TrackTimeScale(1000, 0)),
            stts: Some(mp4parse::TimeToSampleBox {
                samples: Default::default(),
            }),
            ..Default::default()
        };
        for &(sample_count, sample_delta) in entries {
            track
                .stts
                .as_mut()
                .unwrap()
                .samples
                .push(mp4parse::Sample {
                    sample_count,
                    sample_delta,
                })
                .unwrap();
        }
        track
    }

    #[test]
    fn sample_bitrate_uses_matching_tables_and_ignores_presentation_duration() {
        let mut track = sample_track();
        assert_eq!(accounted_track_bitrate(&track), Some(800));
        track.duration = Some(mp4parse::TrackScaledTime(1, 0));
        assert_eq!(accounted_track_bitrate(&track), Some(800));
        track.stts.as_mut().unwrap().samples[0].sample_count = 2;
        assert_eq!(accounted_track_bitrate(&track), None);
        track.stts.as_mut().unwrap().samples[0].sample_count = 3;
        track.stts.as_mut().unwrap().samples[0].sample_delta = 0;
        assert_eq!(accounted_track_bitrate(&track), None);
        track
            .stts
            .as_mut()
            .unwrap()
            .samples
            .push(mp4parse::Sample {
                sample_count: 0,
                sample_delta: 40,
            })
            .unwrap();
        assert_eq!(accounted_track_bitrate(&track), None);
    }

    #[test]
    fn uniform_sample_timing_retains_declarations_without_claiming_observations() {
        use scryer_media_types::Rational;
        assert_eq!(
            uniform_sample_frame_rate(&timing_track(&[(1, 1000)])),
            Rational::new(1, 1)
        );
        assert_eq!(
            sample_timing_facts(&timing_track(&[(1, 1000)])),
            (None, None)
        );
        assert_eq!(
            uniform_sample_frame_rate(&timing_track(&[(3, 40), (2, 40)])),
            Rational::new(25, 1)
        );
        for entries in [
            &[][..],
            &[(0, 40)][..],
            &[(1, 0)][..],
            &[(1, 40), (1, 60)][..],
        ] {
            assert!(uniform_sample_frame_rate(&timing_track(entries)).is_none());
        }
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/hevc_hdr10plus.mp4");
        let analysis = crate::analyze_catalog_file(&path).unwrap();
        let video = analysis
            .details
            .streams
            .iter()
            .find(|stream| stream.kind == scryer_media_types::StreamKind::Video)
            .unwrap();
        assert_eq!(video.metadata.declared_frame_rate, Rational::new(1, 1));
        assert!(video.metadata.observed_frame_rate.is_none());
        assert!(video.metadata.variable_frame_rate.is_none());
    }

    #[test]
    fn sample_timing_handles_reordering_vfr_and_bounded_unknowns() {
        use scryer_media_types::Rational;
        let constant = timing_track(&[(4, 40)]);
        assert_eq!(
            sample_timing_facts(&constant),
            (Rational::new(25, 1), Some(false))
        );
        assert_eq!(
            sample_timing_facts(&timing_track(&[(3, 40), (2, 60)])).1,
            Some(true)
        );
        let mut reordered = timing_track(&[(4, 40)]);
        reordered.ctts = Some(mp4parse::CompositionOffsetBox {
            samples: Default::default(),
        });
        for offset in [0, 80, -40, -40] {
            reordered
                .ctts
                .as_mut()
                .unwrap()
                .samples
                .push(mp4parse::TimeOffset {
                    sample_count: 1,
                    time_offset: mp4parse::TimeOffsetVersion::Version1(offset),
                })
                .unwrap();
        }
        assert_eq!(
            sample_timing_facts(&reordered),
            (Rational::new(25, 1), Some(false))
        );
        let mut bounded = timing_track(&[(10_000, 40)]);
        bounded.ctts = Some(mp4parse::CompositionOffsetBox {
            samples: Default::default(),
        });
        bounded
            .ctts
            .as_mut()
            .unwrap()
            .samples
            .push(mp4parse::TimeOffset {
                sample_count: 10_000,
                time_offset: mp4parse::TimeOffsetVersion::Version0(0),
            })
            .unwrap();
        assert_eq!(sample_timing_facts(&bounded), (Rational::new(25, 1), None));
        assert_eq!(sample_timing_facts(&timing_track(&[(4, 0)])), (None, None));
    }

    #[test]
    fn partial_fragment_walk_preserves_headers_without_using_partial_timing() {
        let mut bytes = include_bytes!("../tests/media/hevc_hdr10plus.mp4").to_vec();
        let baseline = parse_mp4_source(&mut Cursor::new(bytes.clone()), "mp4", AnalysisProfile::DefaultRich).unwrap();
        for index in 0..200_u32 {
            let tfhd = [0x18_u32.to_be_bytes(), 1_u32.to_be_bytes(), 1_000_000_u32.to_be_bytes(), 10_000_u32.to_be_bytes()].concat();
            let tfdt = [0_u32.to_be_bytes(), (index * 1_000_000).to_be_bytes()].concat();
            let trun = [0_u32.to_be_bytes(), 1_u32.to_be_bytes()].concat();
            let traf = [make_box(b"tfhd", &tfhd), make_box(b"tfdt", &tfdt), make_box(b"trun", &trun)].concat();
            bytes.extend_from_slice(&make_box(b"moof", &make_box(b"traf", &traf)));
        }
        let raw = parse_mp4_source(&mut Cursor::new(bytes), "mp4", AnalysisProfile::DefaultRich).unwrap();
        assert!(raw.details.report.budget_exhausted);
        assert_eq!(raw.duration_seconds, baseline.duration_seconds);
        assert_eq!(raw.tracks[0].codec_name, baseline.tracks[0].codec_name);
        assert_eq!(raw.tracks[0].bit_rate_bps, baseline.tracks[0].bit_rate_bps);
    }

    #[test]
    fn fragment_budget_exhaustion_is_visible_in_outer_report() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/hevc_hdr10plus.mp4");
        let mut bytes = std::fs::read(path).unwrap();
        let mut tfhd = vec![0; 4];
        tfhd.extend_from_slice(&1_u32.to_be_bytes());
        let mut trun = vec![0; 4];
        trun.extend_from_slice(&4_000_001_u32.to_be_bytes());
        let mut traf = make_box(b"tfhd", &tfhd);
        traf.extend_from_slice(&make_box(b"trun", &trun));
        bytes.extend_from_slice(&make_box(b"moof", &make_box(b"traf", &traf)));

        let container =
            parse_mp4_source(&mut Cursor::new(bytes), "mp4", AnalysisProfile::DefaultRich).unwrap();
        let report = container.details.report;
        assert_eq!(report.status, scryer_media_types::ProbeStatus::Incomplete);
        assert!(report.budget_exhausted);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].code, "mp4_fragment_inventory_budget");
    }

    fn sample_track() -> mp4parse::Track {
        let mut track = timing_track(&[(3, 40)]);
        track.track_id = Some(1);
        track.stsz = Some(mp4parse::SampleSizeBox {
            sample_size: 0,
            sample_sizes: Default::default(),
        });
        for size in [3, 4, 5] {
            track
                .stsz
                .as_mut()
                .unwrap()
                .sample_sizes
                .push(size)
                .unwrap();
        }
        track.stco = Some(mp4parse::ChunkOffsetBox {
            offsets: Default::default(),
        });
        for offset in [100, 500] {
            track.stco.as_mut().unwrap().offsets.push(offset).unwrap();
        }
        track.stsc = Some(mp4parse::SampleToChunkBox {
            samples: Default::default(),
        });
        for (first_chunk, samples_per_chunk) in [(1, 2), (2, 1)] {
            track
                .stsc
                .as_mut()
                .unwrap()
                .samples
                .push(mp4parse::SampleToChunk {
                    first_chunk,
                    samples_per_chunk,
                    sample_description_index: 1,
                })
                .unwrap();
        }
        track
    }

    #[test]
    fn sample_ranges_follow_chunk_boundaries_and_reject_overflow() {
        let mut track = sample_track();
        assert_eq!(
            track_sample_ranges(&track),
            Some((vec![(100, 3), (103, 4), (500, 5)], true))
        );
        track.stco.as_mut().unwrap().offsets[0] = u64::MAX;
        assert!(track_sample_ranges(&track).is_none());
        let mut track = sample_track();
        track.stsc.as_mut().unwrap().samples[0].samples_per_chunk = 0;
        assert!(track_sample_ranges(&track).is_none());
    }

    #[test]
    fn av1_mp4_enrichment_reads_metadata_after_the_first_sample() {
        let mut track = sample_track();
        track.stsz.as_mut().unwrap().sample_sizes[0] = 2;
        track.stsz.as_mut().unwrap().sample_sizes[1] = 8;
        track.stsz.as_mut().unwrap().sample_sizes[2] = 2;
        let mut bytes = vec![0; 502];
        bytes[100..102].copy_from_slice(&[18, 0]);
        bytes[102..110].copy_from_slice(&[42, 6, 1, 3, 232, 1, 144, 128]);
        bytes[500..502].copy_from_slice(&[18, 0]);
        let mut context = MediaContext::default();
        context.tracks.push(track).unwrap();
        let mut tracks = vec![ParsedMp4Track {
            track_id: Some(1),
            raw: RawTrack {
                kind: TrackKind::Video,
                codec_name: Some("av1".into()),
                ..Default::default()
            },
        }];
        let mut report = scryer_media_types::ProbeReport::default();
        scan_mp4_sample_probes(
            &mut Cursor::new(bytes),
            &context,
            &mut tracks,
            AnalysisProfile::DefaultRich,
            &mut report,
        );
        assert_eq!(
            tracks[0]
                .raw
                .metadata
                .color
                .content_light
                .as_ref()
                .unwrap()
                .max_cll,
            Some(1000)
        );
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn rotation_rejects_reflections_and_preserves_clockwise_sign() {
        let matrix = mp4parse::Matrix {
            a: 0,
            b: 65536,
            c: -65536,
            d: 0,
            u: 0,
            v: 0,
            w: 1 << 30,
            x: 0,
            y: 0,
        };
        assert_eq!(matrix_rotation(&matrix), Some(-90.0));
        assert_eq!(
            matrix_rotation(&mp4parse::Matrix { c: 65536, ..matrix }),
            None
        );
    }

    fn make_box(name: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (8 + payload.len()) as u32;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(payload);
        out
    }

    fn make_meta_box(payload: &[u8]) -> Vec<u8> {
        let mut full_box = vec![0_u8; 4];
        full_box.extend_from_slice(payload);
        make_box(b"meta", &full_box)
    }

    fn make_hdlr_box(
        pre_defined: u32,
        handler_type: &[u8; 4],
        reserved: [u32; 3],
        name: &[u8],
    ) -> Vec<u8> {
        let mut payload = vec![0_u8; 4];
        payload.extend_from_slice(&pre_defined.to_be_bytes());
        payload.extend_from_slice(handler_type);
        for value in reserved {
            payload.extend_from_slice(&value.to_be_bytes());
        }
        payload.extend_from_slice(name);
        make_box(b"hdlr", &payload)
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
    fn catalog_box_walk_is_independent_of_fragment_count() {
        for count in [1_000, 100_000] {
            let file = make_box(b"free", &[]).repeat(count);
            let mut reader = TrackedReader::new(Cursor::new(file));
            let (_, length, exhausted) = prepare_mp4_metadata_from_reader(&mut reader).unwrap();
            assert!(exhausted);
            assert_eq!(length, (count * 8) as u64);
            assert!(reader.stats().bytes_read <= 128 * 8);
            assert!(reader.stats().seeks <= 260);
            let raw = parse_mp4_source(&mut Cursor::new(make_box(b"free", &[]).repeat(count)), "mp4", AnalysisProfile::DefaultRich).unwrap();
            assert!(raw.details.report.budget_exhausted);
            assert_eq!(raw.duration_seconds, None);
        }
    }

    #[test]
    fn metadata_copy_seeks_over_mdat_payload() {
        let ftyp = make_box(b"ftyp", b"isom\0\0\0\0isom");
        let mdat_payload = vec![0_u8; 1024 * 1024];
        let mdat = make_box(b"mdat", &mdat_payload);
        let moov = make_box(b"moov", &make_box(b"mvhd", &[0_u8; 32]));
        let file = [ftyp.clone(), mdat.clone(), moov.clone()].concat();

        let mut reader = TrackedReader::new(Cursor::new(file));
        let (metadata, file_len_hint, exhausted) = prepare_mp4_metadata_from_reader(&mut reader).unwrap();
        assert!(!exhausted);
        let stats = reader.stats();

        assert_eq!(metadata, [ftyp, moov].concat());
        assert_eq!(file_len_hint, metadata.len() as u64 + mdat.len() as u64);
        assert!(
            stats.bytes_read < 512,
            "unexpected payload read: {:?}",
            stats
        );
        assert!(stats.seeks >= 1, "expected explicit mdat skip: {:?}", stats);
    }

    #[test]
    fn metadata_copy_rejects_truncated_kept_box_before_allocating() {
        let file = [
            &(MP4_KEEP_BOX_MAX_BYTES as u32).to_be_bytes(),
            b"moov".as_slice(),
        ]
        .concat();
        let mut reader = TrackedReader::new(Cursor::new(file));

        let error = prepare_mp4_metadata_from_reader(&mut reader)
            .expect_err("truncated moov must be rejected before allocation");
        let MediaInfoError::Parse(message) = error else {
            panic!("expected parse error for truncated moov");
        };
        assert!(message.contains("extends past end of input"));
    }

    #[test]
    fn sanitize_prepared_metadata_stops_at_depth_limit() {
        const DEPTH: usize = 65_000;
        const FIXTURE_BYTES: usize = 2_665_000;

        let hdlr = make_box(b"hdlr", &[0_u8; 24]);
        let free_payload_len = FIXTURE_BYTES - DEPTH * 8 - hdlr.len() - 8;
        let mut metadata = vec![0_u8; FIXTURE_BYTES];

        for level in 0..DEPTH {
            let offset = level * 8;
            let size = u32::try_from(FIXTURE_BYTES - offset).unwrap();
            metadata[offset..offset + 4].copy_from_slice(&size.to_be_bytes());
            metadata[offset + 4..offset + 8].copy_from_slice(b"moov");
        }

        let hdlr_start = DEPTH * 8;
        metadata[hdlr_start..hdlr_start + hdlr.len()].copy_from_slice(&hdlr);
        let free_start = hdlr_start + hdlr.len();
        let free_size = u32::try_from(free_payload_len + 8).unwrap();
        metadata[free_start..free_start + 4].copy_from_slice(&free_size.to_be_bytes());
        metadata[free_start + 4..free_start + 8].copy_from_slice(b"free");

        sanitize_prepared_mp4_metadata(&mut metadata);

        assert_eq!(metadata.len(), FIXTURE_BYTES);
    }

    #[test]
    fn sanitize_prepared_metadata_repairs_malformed_hdlr_boxes() {
        let hdlr = make_hdlr_box(1, b"vide", [2, 3, 4], b"VideoHandler");
        let mdia = make_box(b"mdia", &hdlr);
        let trak = make_box(b"trak", &mdia);
        let mut moov = make_box(b"moov", &trak);

        let original_len = moov.len();
        let original_moov_size = read_box_header_from_bytes(&moov).unwrap().size;

        sanitize_prepared_mp4_metadata(&mut moov);

        assert_eq!(moov.len(), original_len + 1);
        assert_eq!(
            read_box_header_from_bytes(&moov).unwrap().size,
            original_moov_size + 1
        );

        let moov_header = read_box_header_from_bytes(&moov).unwrap();
        let trak = &moov[moov_header.header_size..moov_header.size as usize];
        let trak_header = read_box_header_from_bytes(trak).unwrap();
        let mdia = &trak[trak_header.header_size..trak_header.size as usize];
        let mdia_header = read_box_header_from_bytes(mdia).unwrap();
        let hdlr = &mdia[mdia_header.header_size..mdia_header.size as usize];
        let hdlr_header = read_box_header_from_bytes(hdlr).unwrap();
        let payload = &hdlr[hdlr_header.header_size..hdlr_header.size as usize];

        assert_eq!(&payload[4..8], &[0, 0, 0, 0]);
        assert_eq!(&payload[12..24], &[0; 12]);
        assert_eq!(payload.last().copied(), Some(0));
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

    #[test]
    fn parse_trak_ignores_kind_box_hidden_inside_leaf_payload() {
        let kind_payload = [
            [0_u8, 0, 0, 0].as_slice(),
            b"urn:mpeg:dash:role:2011\0forced-subtitle\0".as_slice(),
        ]
        .concat();

        let trak_payload = make_box(
            b"udta",
            &make_box(b"name", &make_box(b"kind", &kind_payload)),
        );

        let track = parse_trak(&trak_payload);

        assert!(!track.forced);
    }

    #[test]
    fn parse_trak_stops_descending_past_max_depth() {
        let kind_payload = [
            [0_u8, 0, 0, 0].as_slice(),
            b"urn:mpeg:dash:role:2011\0forced-subtitle\0".as_slice(),
        ]
        .concat();

        let mut nested = make_box(b"kind", &kind_payload);
        for _ in 0..=MP4_BOX_MAX_DEPTH {
            nested = make_meta_box(&nested);
        }

        let track = parse_trak(&make_box(b"udta", &nested));

        assert!(!track.forced);
    }

    #[test]
    fn extended_language_tags_override_legacy_tags_in_either_box_order() {
        let mut mdhd = vec![0; 24];
        mdhd[20..22].copy_from_slice(&((5_u16 << 10) | (14 << 5) | 7).to_be_bytes());
        let legacy = make_box(b"mdhd", &mdhd);
        let modern = make_box(b"elng", &[&[0, 0, 0, 0][..], b"zh-Hant-TW\0"].concat());
        for boxes in [[legacy.clone(), modern.clone()], [modern, legacy.clone()]] {
            let track = parse_trak(&make_box(b"mdia", &boxes.concat()));
            assert_eq!(track.language.as_deref(), Some("zh-Hant-TW"));
            assert_eq!(
                track.details.original_language.as_deref(),
                Some("zh-Hant-TW")
            );
            assert_eq!(
                track.details.language_provenance,
                scryer_media_types::Provenance::Container
            );
        }
        let invalid = make_box(b"elng", b"\0\0\0\0fr\0hidden");
        let track = parse_trak(&make_box(b"mdia", &[legacy, invalid].concat()));
        assert_eq!(track.language.as_deref(), Some("eng"));
    }

    #[test]
    fn parse_chpl_counts_chapters() {
        let chpl_payload = [[0_u8, 0, 0, 0].as_slice(), &[3], &[0_u8; 27]].concat();
        let metadata = parse_mp4_chapter_metadata(&make_box(
            b"moov",
            &make_box(b"udta", &make_box(b"chpl", &chpl_payload)),
        ));

        assert_eq!(metadata.chpl_count, Some(3));
    }

    #[test]
    fn parse_chpl_ignores_boxes_hidden_inside_ilst_metadata() {
        let chpl_payload = [[0_u8, 0, 0, 0].as_slice(), &[3], &[0_u8; 27]].concat();
        let metadata = parse_mp4_chapter_metadata(&make_box(
            b"moov",
            &make_box(
                b"udta",
                &make_meta_box(&make_box(b"ilst", &make_box(b"chpl", &chpl_payload))),
            ),
        ));

        assert_eq!(metadata.chpl_count, None);
    }

    #[test]
    fn parse_chap_track_ids_reads_references() {
        let ids = parse_chap_track_ids(&[0, 0, 0, 7, 0, 0, 0, 9]);
        assert_eq!(ids, vec![7, 9]);
    }

    #[test]
    fn parses_aac_audio_specific_config_channel_count() {
        assert_eq!(
            parse_aac_audio_specific_config_channels(&[0x11, 0xB0]),
            Some(6)
        );
    }

    #[test]
    fn parses_ac3_and_eac3_channel_counts_from_codec_private() {
        assert_eq!(parse_dac3_channels(&[0x00, 0x3C, 0x00]), Some(6));
        assert_eq!(
            parse_dec3_channels(&[0x00, 0x00, 0x00, 0x0F, 0x00]),
            Some(6)
        );
    }

    #[test]
    fn combines_codec_specific_audio_channels_with_sample_entry_defaults() {
        assert_eq!(
            mp4_audio_channels("mp4a", Some(&[0x11, 0xB0]), Some(2)),
            Some(6)
        );
        assert_eq!(
            mp4_audio_channels("mp4a", Some(&[0x13, 0x08]), Some(2)),
            Some(2)
        );
        assert_eq!(
            mp4_audio_channels("ac-3", Some(&[0x00, 0x3C, 0x00]), None),
            Some(6)
        );
    }

    #[test]
    fn sonarr_runtime_prefers_video_then_audio_then_format_duration() {
        assert_eq!(
            best_sonarr_runtime(Some(1920.96), Some(1919.167), Some(1920.96)),
            Some(1919.167)
        );
        assert_eq!(
            best_sonarr_runtime(Some(1517.0), Some(0.0), Some(1519.0)),
            Some(1517.0)
        );
        assert_eq!(best_sonarr_runtime(None, None, Some(42.0)), Some(42.0));
    }

    #[test]
    fn decodes_legacy_mdhd_english_language_code() {
        assert_eq!(decode_mdhd_language(0), Some("eng".to_string()));
        assert_eq!(decode_mdhd_language(6), Some("spa".to_string()));
        assert_eq!(decode_mdhd_language(11), Some("jpn".to_string()));
    }
}
