//! Bounded reads of sample payloads for video headers, HDR10+ and audio
//! profiles.

use super::*;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(super) struct Mp4SampleReadKey {
    pub(super) offset: u64,
    pub(super) size: usize,
}

pub(super) fn scan_mp4_sample_probes(
    file: &mut dyn crate::source::MediaSource,
    ctx: &MediaContext,
    tracks: &mut [ParsedMp4Track],
    profile: AnalysisProfile,
    report: &mut scryer_media_model::ProbeReport,
) -> Vec<scryer_media_model::CaptionService> {
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

pub(super) fn scan_mp4_video_metadata_with_cache(
    file: &mut dyn crate::source::MediaSource,
    sample_cache: &mut HashMap<Mp4SampleReadKey, Vec<u8>>,
    ctx: &MediaContext,
    tracks: &mut [ParsedMp4Track],
    report: &mut scryer_media_model::ProbeReport,
    captions: &mut Vec<scryer_media_model::CaptionService>,
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
                .map(crate::codec::hevc::hevc_nal_length_size)
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
                Some("av1") => crate::codec::av1::enrich(&mut parsed.raw, &bytes, report),
                Some("h264" | "hevc") => crate::codec::sei::length_prefixed(
                    &bytes,
                    nal_length_size,
                    &mut parsed.raw,
                    report,
                    captions,
                ),
                _ => {
                    crate::codec::picture_headers::enrich(&mut parsed.raw, &bytes);
                    crate::codec::sei::annex_b(&bytes, &mut parsed.raw, report, captions);
                }
            }
        }
    }
}

pub(super) fn sample_probe_warning(
    report: &mut scryer_media_model::ProbeReport,
    track: &RawTrack,
    code: &str,
    budget: bool,
) {
    report.status = scryer_media_model::ProbeStatus::Incomplete;
    report.budget_exhausted |= budget;
    if !report
        .warnings
        .iter()
        .any(|warning| warning.code == code && warning.stream_id == track.metadata.id)
    {
        report.warnings.push(scryer_media_model::ProbeWarning {
            code: code.into(),
            message: "MP4 sample enrichment is incomplete; undetected metadata remains unknown"
                .into(),
            stream_id: track.metadata.id.clone(),
            ..Default::default()
        });
    }
}

/// Follow chunk mappings, including interleaved tracks, without walking payloads.
pub(super) fn track_sample_ranges(track: &mp4parse::Track) -> Option<(Vec<(u64, usize)>, bool)> {
    track_sample_ranges_bounded(track, 16)
}

pub(super) fn track_sample_ranges_bounded(
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

pub(super) fn scan_mp4_audio_profiles_with_cache(
    file: &mut dyn crate::source::MediaSource,
    sample_cache: &mut HashMap<Mp4SampleReadKey, Vec<u8>>,
    ctx: &MediaContext,
    tracks: &mut [ParsedMp4Track],
    report: &mut scryer_media_model::ProbeReport,
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
                Some("ac3") => crate::codec::ac3::find_ac3_header(&prefix),
                Some("eac3") => crate::codec::ac3::find_eac3_header(&prefix),
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

pub(super) fn read_audio_profile_probe_bytes_cached(
    file: &mut dyn crate::source::MediaSource,
    sample_cache: &mut HashMap<Mp4SampleReadKey, Vec<u8>>,
    sample_offset: u64,
    sample_size: u64,
    spec: crate::codec::audio_profile::AudioProfileProbeSpec,
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

pub(super) fn read_cached_mp4_range(
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
