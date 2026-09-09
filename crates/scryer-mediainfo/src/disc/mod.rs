//! Native inspection of intact mastered optical-disc images.

mod clip_info;
mod cpi;
#[cfg(test)]
mod fixtures;
mod image;
mod navigation;
mod streams;
#[cfg(test)]
mod udf_fixtures;

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Instant;

use scryer_media_types::{
    ANALYSIS_REVISION, AnalysisDetails, DiscMetadata, DiscSelection, DiscTitle, ProbeReport,
    ProbeStatus, Provenance,
};

use crate::source::{BoundedSource, Extent, ExtentSource, FileSource, MediaSource};
use crate::{AnalysisProfile, MediaAnalysis, MediaInfoError};
use image::{Image, ImageError, ImageFile};
use navigation::warning;

const READ_BUDGET: u64 = 128 * 1024 * 1024;
const NAVIGATION_BUDGET: u64 = 32 * 1024 * 1024;

pub(crate) fn analyze(
    path: &Path,
    selection: DiscSelection,
    profile: AnalysisProfile,
) -> Result<MediaAnalysis, MediaInfoError> {
    let started = Instant::now();
    let mut source = BoundedSource::new(FileSource::open(path)?, READ_BUDGET);
    let result = inspect(&mut source, selection, profile);
    let mut analysis = match result {
        Ok(analysis) => analysis,
        Err(error) => MediaAnalysis {
            container_format: Some("iso".into()),
            details: AnalysisDetails {
                revision: ANALYSIS_REVISION,
                report: error_report(&error),
                ..Default::default()
            },
            ..Default::default()
        },
    };
    analysis.details.report.bytes_read = source.bytes_read;
    analysis.details.report.seeks = source.seeks;
    analysis.details.report.elapsed_ms =
        started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    analysis.details.report.budget_exhausted |= source.exhausted;
    if source.exhausted {
        analysis.details.report.status = ProbeStatus::Incomplete;
    }
    if !source.get_ref().unchanged(path)? {
        return Err(MediaInfoError::Io(
            "disc image changed during analysis".into(),
        ));
    }
    Ok(analysis)
}

pub(crate) fn diagnose_source(
    source: &mut dyn MediaSource,
    selection: DiscSelection,
) -> ProbeReport {
    match inspect(source, selection, AnalysisProfile::DefaultRich) {
        Ok(analysis) => analysis.details.report,
        Err(error) => error_report(&error),
    }
}

fn error_report(error: &ImageError) -> ProbeReport {
    let mut report = ProbeReport {
        status: match error {
            ImageError::Malformed(_) => ProbeStatus::Malformed,
            ImageError::Unsupported(_) => ProbeStatus::Unsupported,
            ImageError::Budget | ImageError::Io(_) => ProbeStatus::Incomplete,
        },
        budget_exhausted: matches!(error, ImageError::Budget),
        ..Default::default()
    };
    warning(&mut report, "disc_inspection", &error.to_string());
    report
}
fn read_navigation(
    source: &mut dyn MediaSource,
    file: &ImageFile,
    remaining: &mut u64,
) -> Result<Vec<u8>, ImageError> {
    if file.length > 4 * 1024 * 1024 || file.length > *remaining {
        return Err(ImageError::Budget);
    }
    *remaining -= file.length;
    let mut bytes = vec![0; file.length as usize];
    ExtentSource::new(source, file.extents.clone())?.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn inspect(
    source: &mut dyn MediaSource,
    selection: DiscSelection,
    profile: AnalysisProfile,
) -> Result<MediaAnalysis, ImageError> {
    let image = image::open(source)?;
    let bluray = image
        .files
        .keys()
        .any(|path| path.starts_with("BDMV/PLAYLIST/") && path.ends_with(".MPLS"));
    let dvd = image.files.contains_key("VIDEO_TS/VIDEO_TS.IFO");
    let mut disc = DiscMetadata {
        disc_type: if bluray {
            "bluray"
        } else if dvd {
            "dvd"
        } else {
            "unknown"
        }
        .into(),
        filesystem: image.filesystem.clone(),
        volume_label: image.volume_label.clone(),
        selection,
        ..Default::default()
    };
    let mut navigation_budget = NAVIGATION_BUDGET;
    let mut title_files: BTreeMap<String, Vec<(String, ImageFile)>> = BTreeMap::new();
    if bluray {
        let mut clip_information =
            BTreeMap::<String, std::result::Result<clip_info::ClipInfo, ProbeReport>>::new();
        let mut segment_budget = 65_536_usize;
        let mut stream_inventory_budget = 32_768_usize;
        let mut clip_inventory_budget = 32_768_usize;
        let mut clip_point_budget = 262_144_usize;
        if let Some(index) = image.files.get("BDMV/INDEX.BDMV") {
            let index = read_navigation(source, index, &mut navigation_budget)?;
            if index.get(..8) == Some(b"INDX0300") {
                disc.disc_type = "uhd_bluray".into();
            }
        }
        for (path, file) in &image.files {
            if !path.starts_with("BDMV/PLAYLIST/") || !path.ends_with(".MPLS") {
                continue;
            }
            if disc.titles.len() >= 2048 {
                return Err(ImageError::Budget);
            }
            let id = path
                .trim_start_matches("BDMV/PLAYLIST/")
                .trim_end_matches(".MPLS")
                .to_string();
            if id.len() != 5 || !id.bytes().all(|byte| byte.is_ascii_digit()) {
                continue;
            }
            let title = read_navigation(source, file, &mut navigation_budget)
                .and_then(|bytes| navigation::playlist(&bytes, id.clone()));
            match title {
                Ok(mut title) => {
                    segment_budget = segment_budget
                        .checked_sub(title.segments.len())
                        .ok_or(ImageError::Budget)?;
                    let mut files = Vec::new();
                    for segment in &title.segments {
                        if let Some(file) = image.files.get(&segment.path) {
                            let mut referenced = file.clone();
                            let mut cache_key = segment.path.clone();
                            let info_path = segment
                                .path
                                .replace("BDMV/STREAM/", "BDMV/CLIPINF/")
                                .replace(".M2TS", ".CLPI");
                            if let Some(info_file) = image.files.get(&info_path) {
                                if !clip_information.contains_key(&info_path) {
                                    let info =
                                        read_navigation(source, info_file, &mut navigation_budget)
                                            .and_then(|bytes| clip_info::parse(&bytes))
                                            .map_err(|error| error_report(&error));
                                    if let Ok(info) = &info {
                                        clip_inventory_budget = clip_inventory_budget
                                            .checked_sub(info.stream_count())
                                            .ok_or(ImageError::Budget)?;
                                        clip_point_budget = clip_point_budget
                                            .checked_sub(info.point_count())
                                            .ok_or(ImageError::Budget)?;
                                    }
                                    clip_information.insert(info_path.clone(), info);
                                }
                                let declared = match &clip_information[&info_path] {
                                    Ok(info)
                                        if u64::from(info.source_packets) * 192 <= file.length =>
                                    {
                                        (|| {
                                            let streams = info.streams_for(segment)?;
                                            let range = info.packet_range(segment)?;
                                            let length = u64::from(range.end - range.start) * 192;
                                            referenced = ImageFile {
                                                length,
                                                extents: image::range(
                                                    &file.extents,
                                                    u64::from(range.start) * 192,
                                                    length,
                                                )?,
                                            };
                                            cache_key = format!(
                                                "{}#{}-{}",
                                                segment.path, range.start, range.end
                                            );
                                            Ok(streams)
                                        })()
                                        .map_err(|error| error_report(&error))
                                    }
                                    Ok(_) => Err(error_report(&ImageError::Malformed(
                                        "CLPI source packet count exceeds referenced media extent",
                                    ))),
                                    Err(report) => Err(report.clone()),
                                };
                                match declared {
                                    Ok(streams) if title.streams.is_empty() => {
                                        title.streams = streams
                                    }
                                    Ok(streams) if title.streams != streams => {
                                        title.report.status = ProbeStatus::Unsupported;
                                        warning(
                                            &mut title.report,
                                            "clip_declaration_change",
                                            "CLPI declarations differ across referenced playback segments",
                                        );
                                    }
                                    Ok(_) => {}
                                    Err(report) => {
                                        if title.report.status != ProbeStatus::Malformed {
                                            title.report.status = report.status;
                                        }
                                        title.report.budget_exhausted |= report.budget_exhausted;
                                        title.report.warnings.extend(report.warnings);
                                    }
                                }
                            } else {
                                if !matches!(
                                    title.report.status,
                                    ProbeStatus::Malformed | ProbeStatus::Unsupported
                                ) {
                                    title.report.status = ProbeStatus::Incomplete;
                                }
                                warning(
                                    &mut title.report,
                                    "missing_clip_information",
                                    "CLPI is missing; the selected clock sequence and trim coverage could not be verified",
                                );
                            }
                            files.push((cache_key, referenced));
                        } else {
                            title.report.status = ProbeStatus::Malformed;
                            warning(
                                &mut title.report,
                                "missing_clip",
                                &format!("Referenced clip {} is missing", segment.path),
                            );
                        }
                    }
                    stream_inventory_budget = stream_inventory_budget
                        .checked_sub(title.streams.len())
                        .ok_or(ImageError::Budget)?;
                    title_files.insert(id, files);
                    disc.titles.push(title);
                }
                Err(error) => disc.titles.push(DiscTitle {
                    id,
                    report: error_report(&error),
                    ..Default::default()
                }),
            }
        }
    } else if dvd {
        let vmg = read_navigation(
            source,
            &image.files["VIDEO_TS/VIDEO_TS.IFO"],
            &mut navigation_budget,
        )?;
        let mut vts_cache = BTreeMap::new();
        for reference in navigation::dvd_titles(&vmg)? {
            let title_result = (|| {
                let path = format!("VIDEO_TS/VTS_{:02}_0.IFO", reference.set);
                if !vts_cache.contains_key(&reference.set) {
                    let file = image
                        .files
                        .get(&path)
                        .ok_or(ImageError::Malformed("missing DVD title set IFO"))?;
                    vts_cache.insert(
                        reference.set,
                        read_navigation(source, file, &mut navigation_budget)?,
                    );
                }
                let (title, cells) = navigation::dvd_title(&vts_cache[&reference.set], &reference)?;
                let extents = dvd_vob_extents(&image, reference.set)?;
                let mut files = Vec::new();
                for (cell, segment) in cells.iter().zip(&title.segments) {
                    let offset = u64::from(cell.first_sector) * 2048;
                    let length =
                        (u64::from(cell.last_sector) - u64::from(cell.first_sector) + 1) * 2048;
                    files.push((
                        segment.path.clone(),
                        ImageFile {
                            length,
                            extents: image::range(&extents, offset, length)?,
                        },
                    ));
                }
                title_files.insert(reference.id.clone(), files);
                Ok::<_, ImageError>(title)
            })();
            disc.titles
                .push(title_result.unwrap_or_else(|error| DiscTitle {
                    id: reference.id,
                    report: error_report(&error),
                    ..Default::default()
                }));
        }
    } else {
        return Err(ImageError::Unsupported(
            "image contains no recognized BDMV or DVD title structure",
        ));
    }

    deduplicate_titles(&mut disc.titles);
    select_title(&mut disc);
    let mut report = ProbeReport {
        status: ProbeStatus::Incomplete,
        ..Default::default()
    };
    if disc.selected_title_id.is_none() {
        warning(
            &mut report,
            "disc_selection_review",
            "No valid title is selected; a missing saved selection requires review",
        );
    }
    let mut cache: BTreeMap<String, MediaAnalysis> = BTreeMap::new();
    let mut failed_clips: BTreeMap<String, (ProbeStatus, String)> = BTreeMap::new();
    let mut selected_analysis = None;
    let mut resolved_title_id = None;
    // Probe the chosen title first so other titles cannot consume its budget.
    let mut order: Vec<usize> = (0..disc.titles.len()).collect();
    order.sort_by_key(|index| {
        let title = &disc.titles[*index];
        (
            disc.selected_title_id.as_ref() != Some(&title.id),
            !disc.selection.episode_mappings.iter().any(|mapping| {
                mapping.disc_title_id == title.id || title.aliases.contains(&mapping.disc_title_id)
            }),
        )
    });
    for index in order {
        let title = &mut disc.titles[index];
        if matches!(
            title.report.status,
            ProbeStatus::Malformed | ProbeStatus::Unsupported | ProbeStatus::Encrypted
        ) {
            if disc.selected_title_id.as_ref() == Some(&title.id) {
                report = title.report.clone();
            }
            continue;
        }
        let mut first: Option<MediaAnalysis> = None;
        for (key, file) in title_files.get(&title.id).into_iter().flatten() {
            if let Some((status, message)) = failed_clips.get(key) {
                title.report.status = *status;
                warning(&mut title.report, "clip_probe_failed", message);
                continue;
            }
            if !cache.contains_key(key) {
                if cache.len() + failed_clips.len() >= 64 {
                    title.report.budget_exhausted = true;
                    warning(
                        &mut title.report,
                        "clip_budget",
                        "Clip inspection budget exhausted",
                    );
                    break;
                }
                let probe = (|| {
                    let mut clip = ExtentSource::new(source, file.extents.clone())?;
                    if encrypted(&mut clip, bluray)? {
                        return Err(MediaInfoError::UnsupportedFormat(
                            "encrypted disc payload".into(),
                        ));
                    }
                    clip.seek(SeekFrom::Start(0))?;
                    crate::analyze_source(
                        &mut clip,
                        if bluray { "m2ts" } else { "vob" },
                        crate::AnalyzeOptions { profile },
                    )
                })();
                match probe {
                    Ok(analysis) => {
                        cache.insert(key.clone(), analysis);
                    }
                    Err(error) => {
                        title.report.status =
                            if error.to_string().contains("encrypted disc payload") {
                                ProbeStatus::Encrypted
                            } else {
                                ProbeStatus::Incomplete
                            };
                        failed_clips.insert(key.clone(), (title.report.status, error.to_string()));
                        warning(&mut title.report, "clip_probe_failed", &error.to_string());
                        continue;
                    }
                }
            }
            let analysis = &cache[key];
            if analysis.details.report.budget_exhausted {
                title.report.budget_exhausted = true;
                warning(
                    &mut title.report,
                    "clip_enrichment_budget",
                    &format!("Referenced clip {key} exhausted its analysis budget"),
                );
            }
            if matches!(
                analysis.details.report.status,
                ProbeStatus::Unsupported | ProbeStatus::Encrypted | ProbeStatus::Malformed
            ) {
                title.report.status = analysis.details.report.status;
                warning(
                    &mut title.report,
                    "clip_analysis_incomplete",
                    &format!(
                        "Referenced clip {key} reported {:?}",
                        analysis.details.report.status
                    ),
                );
            }
            if let Some(first) = &first {
                if !same_stream_formats(first, analysis) {
                    title.report.status = ProbeStatus::Unsupported;
                    warning(
                        &mut title.report,
                        "segment_format_change",
                        "Referenced segments have materially different stream formats",
                    );
                }
            } else {
                first = Some(analysis.clone());
            }
        }
        if let Some(mut analysis) = first {
            streams::apply_navigation(&mut analysis, &title.streams);
            title.streams = analysis.details.streams.clone();
            if title.report.warnings.is_empty() && analysis.video_codec.is_some() {
                title.report.status = ProbeStatus::Complete;
            }
            for warning in &analysis.details.report.warnings {
                if !title.report.warnings.contains(warning) {
                    title.report.warnings.push(warning.clone());
                }
            }
            let better_automatic_choice = disc.automatic_selection
                && selected_analysis
                    .as_ref()
                    .is_none_or(|previous: &MediaAnalysis| {
                        title.duration_seconds.unwrap_or(0.0)
                            > previous.details.duration_seconds.unwrap_or(0.0)
                            || (title.duration_seconds == previous.details.duration_seconds
                                && numeric_id(&title.id)
                                    < resolved_title_id
                                        .as_deref()
                                        .map(numeric_id)
                                        .unwrap_or(u32::MAX))
                    });
            if better_automatic_choice
                || (!disc.automatic_selection && disc.selected_title_id.as_ref() == Some(&title.id))
            {
                if title.report.status == ProbeStatus::Complete {
                    analysis.duration_seconds = title
                        .duration_seconds
                        .map(|duration| duration.round().min(f64::from(i32::MAX)) as i32);
                    analysis.details.duration_seconds = title.duration_seconds;
                    analysis.details.duration_provenance = Provenance::Container;
                    analysis.details.chapters = title.chapters.clone();
                    analysis.num_chapters = Some(title.chapters.len() as i32);
                    // Whole-image storage rate is unrelated to this title's video bitrate.
                    analysis.details.overall_bitrate_bps = None;
                    analysis.details.report = title.report.clone();
                    selected_analysis = Some(analysis);
                    resolved_title_id = Some(title.id.clone());
                }
            }
        }
        if disc.selected_title_id.as_ref() == Some(&title.id) {
            report = title.report.clone();
        }
    }
    if disc.automatic_selection {
        disc.selected_title_id = resolved_title_id;
    }
    if let Some(selected) = &selected_analysis {
        report = selected.details.report.clone();
    }
    for mapping in &disc.selection.episode_mappings {
        if disc
            .titles
            .iter()
            .find(|title| {
                title.id == mapping.disc_title_id || title.aliases.contains(&mapping.disc_title_id)
            })
            .is_none_or(|title| title.report.status != ProbeStatus::Complete)
        {
            if report.status == ProbeStatus::Complete {
                report.status = ProbeStatus::Incomplete;
            }
            warning(
                &mut report,
                "disc_episode_mapping_review",
                &format!(
                    "Mapped playback title {} is missing or could not be inspected; review the saved episode mapping",
                    mapping.disc_title_id
                ),
            );
        }
    }
    let mut analysis = selected_analysis.unwrap_or_default();
    analysis.container_format = Some("iso".into());
    analysis.details.revision = ANALYSIS_REVISION;
    analysis.details.disc = Some(disc);
    analysis.details.report = report;
    Ok(analysis)
}
fn dvd_vob_extents(image: &Image, set: u8) -> Result<Vec<Extent>, ImageError> {
    let mut extents = Vec::new();
    let mut missing = false;
    for index in 1..=9 {
        if let Some(file) = image
            .files
            .get(&format!("VIDEO_TS/VTS_{set:02}_{index}.VOB"))
        {
            if missing || file.length % 2048 != 0 {
                return Err(ImageError::Malformed("DVD VOB file sequence or size"));
            }
            extents.extend(file.extents.iter().copied());
        } else {
            missing = true;
        }
    }
    Ok(extents)
}
fn encrypted(source: &mut dyn MediaSource, bluray: bool) -> Result<bool, MediaInfoError> {
    let length = source.len().min(64 * 1024) as usize;
    let mut bytes = vec![0; length];
    source.read_exact(&mut bytes)?;
    if bluray {
        for packet in bytes.chunks_exact(192) {
            if packet[4] == 0x47 && packet[7] & 0xc0 != 0 {
                return Ok(true);
            }
        }
    } else {
        for at in 0..bytes.len().saturating_sub(7) {
            if bytes[at..at + 3] == [0, 0, 1]
                && matches!(bytes[at + 3], 0xbd | 0xc0..=0xef)
                && bytes[at + 6] & 0xb0 == 0xb0
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn same_stream_formats(a: &MediaAnalysis, b: &MediaAnalysis) -> bool {
    a.details.streams.len() == b.details.streams.len()
        && a.details
            .streams
            .iter()
            .zip(&b.details.streams)
            .all(|(a, b)| {
                a.kind == b.kind
                    && a.codec == b.codec
                    && a.width == b.width
                    && a.height == b.height
                    && a.channels == b.channels
                    && a.language == b.language
                    && a.metadata.profile == b.metadata.profile
                    && a.metadata.level == b.metadata.level
                    && a.metadata.bit_depth == b.metadata.bit_depth
                    && a.metadata.pixel_format == b.metadata.pixel_format
                    && a.metadata.field_order == b.metadata.field_order
                    && a.metadata.display_aspect_ratio == b.metadata.display_aspect_ratio
                    && a.metadata.sample_aspect_ratio == b.metadata.sample_aspect_ratio
                    && a.metadata.rotation_degrees == b.metadata.rotation_degrees
                    && a.metadata.declared_frame_rate == b.metadata.declared_frame_rate
                    && a.metadata.sample_rate == b.metadata.sample_rate
                    && a.metadata.sample_format == b.metadata.sample_format
                    && a.metadata.sample_bit_depth == b.metadata.sample_bit_depth
                    && a.metadata.channel_layout == b.metadata.channel_layout
                    && a.metadata.hdr == b.metadata.hdr
                    && a.metadata.color.primaries == b.metadata.color.primaries
                    && a.metadata.color.transfer == b.metadata.color.transfer
                    && a.metadata.color.matrix == b.metadata.color.matrix
                    && a.metadata.color.full_range == b.metadata.color.full_range
            })
}
fn numeric_id(id: &str) -> u32 {
    id.parse().unwrap_or(u32::MAX)
}
fn deduplicate_titles(titles: &mut Vec<DiscTitle>) {
    titles.sort_by_key(|title| numeric_id(&title.id));
    let mut distinct: Vec<DiscTitle> = Vec::new();
    for title in titles.drain(..) {
        if !title.segments.is_empty()
            && let Some(previous) = distinct
                .iter_mut()
                .find(|previous| previous.segments == title.segments)
        {
            previous.aliases.push(title.id);
        } else {
            distinct.push(title);
        }
    }
    *titles = distinct;
}
fn select_title(disc: &mut DiscMetadata) {
    disc.automatic_selection = disc.selection.title_id.is_none();
    disc.selected_title_id = if let Some(id) = &disc.selection.title_id {
        disc.titles
            .iter()
            .find(|title| &title.id == id || title.aliases.contains(id))
            .map(|title| title.id.clone())
    } else {
        disc.titles
            .iter()
            .filter(|title| {
                !matches!(
                    title.report.status,
                    ProbeStatus::Malformed | ProbeStatus::Unsupported | ProbeStatus::Encrypted
                ) && title
                    .duration_seconds
                    .is_some_and(|duration| duration.is_finite() && duration > 0.0)
            })
            .max_by(|a, b| {
                a.duration_seconds
                    .unwrap()
                    .total_cmp(&b.duration_seconds.unwrap())
                    .then_with(|| numeric_id(&b.id).cmp(&numeric_id(&a.id)))
            })
            .map(|title| title.id.clone())
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_media_types::DiscSegment;
    fn title(id: &str, seconds: f64, clip: &str) -> DiscTitle {
        DiscTitle {
            id: id.into(),
            duration_seconds: Some(seconds),
            segments: vec![DiscSegment {
                path: clip.into(),
                out_seconds: seconds,
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    #[test]
    fn segment_compatibility_preserves_layout_hdr_and_encoded_precision() {
        let mut first = MediaAnalysis::default();
        first
            .details
            .streams
            .push(scryer_media_types::StreamDetail {
                kind: scryer_media_types::StreamKind::Audio,
                codec: Some("pcm_bluray".into()),
                channels: Some(6),
                metadata: scryer_media_types::StreamMetadata {
                    channel_layout: Some("5.1".into()),
                    sample_bit_depth: Some(24),
                    ..Default::default()
                },
                ..Default::default()
            });
        let mut other = first.clone();
        other.details.streams[0].metadata.channel_layout = Some("6.0".into());
        assert!(!same_stream_formats(&first, &other));
        other = first.clone();
        other.details.streams[0].metadata.sample_bit_depth = Some(16);
        assert!(!same_stream_formats(&first, &other));
        first
            .details
            .streams
            .push(scryer_media_types::StreamDetail {
                codec: Some("hevc".into()),
                ..Default::default()
            });
        other = first.clone();
        other.details.streams[1].metadata.hdr.dolby_vision = Some(true);
        assert!(!same_stream_formats(&first, &other));
        other = first.clone();
        other.details.streams[1].metadata.color.transfer = Some(16);
        assert!(!same_stream_formats(&first, &other));
        other = first.clone();
        other.details.streams[1].metadata.bitrate_bps = Some(42_000_000);
        other.details.streams[1].metadata.duration_seconds = Some(10.0);
        assert!(
            same_stream_formats(&first, &other),
            "clip duration and variable bitrate do not change stream formats"
        );
    }
    #[test]
    fn selection_deduplicates_timelines_and_uses_numeric_ties() {
        let mut disc = DiscMetadata {
            titles: vec![
                title("10", 100.0, "a"),
                title("2", 100.0, "b"),
                title("3", 100.0, "a"),
            ],
            ..Default::default()
        };
        deduplicate_titles(&mut disc.titles);
        assert_eq!(disc.titles.len(), 2);
        assert_eq!(disc.titles[1].aliases, ["10"]);
        select_title(&mut disc);
        assert_eq!(disc.selected_title_id.as_deref(), Some("2"));
        disc.selection.title_id = Some("10".into());
        select_title(&mut disc);
        assert_eq!(disc.selected_title_id.as_deref(), Some("3"));
        disc.selection.title_id = Some("99".into());
        select_title(&mut disc);
        assert!(disc.selected_title_id.is_none());
        assert!(!disc.automatic_selection);
    }
}
