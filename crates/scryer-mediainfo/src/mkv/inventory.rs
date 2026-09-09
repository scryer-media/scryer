//! Supplemental container inventory. Attachment payloads and media blocks are skipped.
use super::*;
use scryer_media_types::{
    AnalysisDetails, Attachment, Chapter, ProbeStatus, ProbeWarning, Provenance,
};

const MAX_HEADERS: usize = 65_536;
const MAX_METADATA: u64 = 4 * 1024 * 1024;

pub(super) fn read<R: Read + Seek>(
    scanner: &mut MkvRawScanner<R>,
    tracks: &mut [RawTrack],
    track_uids: &[(u64, usize)],
    scale: f64,
    recover_duration: bool,
) -> AnalysisDetails {
    let mut details = AnalysisDetails::default();
    let mut remaining = MAX_HEADERS;
    let mut bytes_left = MAX_METADATA;
    let result = inventory(
        scanner,
        tracks,
        track_uids,
        scale,
        recover_duration,
        &mut details,
        &mut remaining,
        &mut bytes_left,
    );
    if let Err(error) = result {
        details.report.status = ProbeStatus::Incomplete;
        details.report.budget_exhausted |= remaining == 0 || bytes_left == 0;
        details.report.warnings.push(ProbeWarning {
            code: "mkv_inventory_incomplete".into(),
            message: error.to_string(),
            ..Default::default()
        });
    }
    details
}

fn bounded_end(header: EbmlElementHeader, end: u64) -> Result<u64, MediaInfoError> {
    header
        .size
        .and_then(|size| header.data_offset.checked_add(size))
        .filter(|child_end| *child_end <= end)
        .ok_or_else(|| {
            MediaInfoError::Parse("unknown or invalid supplemental MKV element length".into())
        })
}

fn inventory<R: Read + Seek>(
    scanner: &mut MkvRawScanner<R>,
    tracks: &mut [RawTrack],
    track_uids: &[(u64, usize)],
    scale: f64,
    recover_duration: bool,
    details: &mut AnalysisDetails,
    remaining: &mut usize,
    bytes_left: &mut u64,
) -> Result<(), MediaInfoError> {
    scanner.seek_to(0)?;
    let segment = scanner.read_next_segment_header()?;
    let end = segment
        .size
        .and_then(|size| segment.data_offset.checked_add(size))
        .unwrap_or(scanner.file_len);
    if end > scanner.file_len {
        return Err(MediaInfoError::Parse("truncated MKV segment".into()));
    }
    scanner.seek_to(segment.data_offset)?;
    let mut last_cluster = None;
    let mut bitrates = std::collections::BTreeMap::new();
    while scanner.position()? < end {
        spend(remaining)?;
        let header = scanner
            .read_element_header()?
            .ok_or_else(|| MediaInfoError::Parse("truncated MKV inventory".into()))?;
        let child_end = bounded_end(header, end)?;
        match header.id {
            EBML_ID_CLUSTER => last_cluster = Some((header.data_offset, child_end)),
            EBML_ID_CHAPTERS => {
                let size = child_end - header.data_offset;
                if size > *bytes_left {
                    *bytes_left = 0;
                    return Err(MediaInfoError::Parse(
                        "MKV chapter inventory budget exhausted".into(),
                    ));
                }
                *bytes_left -= size;
                let bytes = scanner.read_bytes(size)?;
                chapters(&bytes, 0, remaining, &mut details.chapters)?;
            }
            0x1941_a469 => attachments(
                scanner,
                child_end,
                remaining,
                bytes_left,
                &mut details.attachments,
            )?,
            0x1254_c367 => {
                let size = child_end - header.data_offset;
                if size > *bytes_left {
                    *bytes_left = 0;
                    return Err(MediaInfoError::Parse(
                        "MKV statistics inventory budget exhausted".into(),
                    ));
                }
                *bytes_left -= size;
                statistics(&scanner.read_bytes(size)?, remaining, &mut bitrates)?;
            }
            _ => {}
        }
        scanner.seek_to(child_end)?;
    }
    let mut uid_index = std::collections::BTreeMap::new();
    for &(uid, index) in track_uids {
        uid_index
            .entry(uid)
            .and_modify(|index| *index = None)
            .or_insert(Some(index));
    }
    for (uid, bitrate) in bitrates {
        let Some(index) = uid_index.get(&uid) else {
            continue;
        };
        if index.is_none() || bitrate.is_none() {
            details.report.warnings.push(ProbeWarning {
                code: "mkv_statistics_conflict".into(),
                message: format!("Ambiguous bitrate statistics for track UID {uid}"),
                ..Default::default()
            });
            continue;
        }
        if let Some(track) = index.and_then(|index| tracks.get_mut(index)) {
            if track.bit_rate_bps.is_none() {
                track.bit_rate_bps = bitrate;
                track.metadata.bitrate_provenance = Provenance::Container;
            }
        }
    }
    if recover_duration && let Some((start, end)) = last_cluster {
        let videos: Vec<_> = tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .collect();
        if let Some(video) = crate::select_primary_video_track(&videos) {
            scanner.seek_to(start)?;
            details.duration_seconds =
                last_cluster_duration(scanner, end, video, scale, remaining)?;
            if details.duration_seconds.is_some() {
                details.duration_provenance = Provenance::Observed;
            }
        }
    }
    Ok(())
}

fn spend(remaining: &mut usize) -> Result<(), MediaInfoError> {
    *remaining = remaining
        .checked_sub(1)
        .ok_or_else(|| MediaInfoError::Parse("MKV metadata header budget exhausted".into()))?;
    Ok(())
}

fn statistics(
    bytes: &[u8],
    remaining: &mut usize,
    output: &mut std::collections::BTreeMap<u64, Option<i64>>,
) -> Result<(), MediaInfoError> {
    let mut bytes = bytes;
    while !bytes.is_empty() {
        spend(remaining)?;
        let (id, tag, consumed) = next_ebml_element(bytes)
            .ok_or_else(|| MediaInfoError::Parse("truncated MKV statistics tag".into()))?;
        if id == 0x7373 {
            let mut targets = Vec::new();
            let mut fields = tag;
            let mut bitrate_value = None;
            while !fields.is_empty() {
                spend(remaining)?;
                let (id, data, length) = next_ebml_element(fields).ok_or_else(|| {
                    MediaInfoError::Parse("truncated MKV statistics field".into())
                })?;
                if id == 0x63c0 {
                    let mut data = data;
                    while !data.is_empty() {
                        spend(remaining)?;
                        let (id, value, length) = next_ebml_element(data).ok_or_else(|| {
                            MediaInfoError::Parse("truncated MKV statistics target".into())
                        })?;
                        if id == 0x63c5
                            && let Some(uid) = parse_ebml_uint(value).filter(|uid| *uid != 0)
                        {
                            targets.push(uid);
                        }
                        data = &data[length..];
                    }
                } else if id == 0x67c8 {
                    // BPS is a top-level per-track statistic. Nested/localized
                    // tags and global tags do not establish a stream bitrate.
                    let mut children = data;
                    let mut name = None;
                    let mut value = None;
                    while !children.is_empty() {
                        spend(remaining)?;
                        let (id, data, length) = next_ebml_element(children).ok_or_else(|| {
                            MediaInfoError::Parse("truncated MKV simple statistic".into())
                        })?;
                        match id {
                            0x45a3 => name = Some(data),
                            0x4487 => value = Some(data),
                            _ => {}
                        }
                        children = &children[length..];
                    }
                    if name == Some(b"BPS".as_slice()) {
                        let bitrate = value
                            .and_then(|value| std::str::from_utf8(value).ok())
                            .and_then(|value| value.parse::<i64>().ok())
                            .filter(|value| *value > 0);
                        bitrate_value = Some(match bitrate_value {
                            Some(previous) if previous != bitrate => None,
                            Some(previous) => previous,
                            None => bitrate,
                        });
                    }
                }
                fields = &fields[length..];
            }
            if let Some(bitrate) = bitrate_value {
                for uid in targets {
                    output
                        .entry(uid)
                        .and_modify(|previous| {
                            if *previous != bitrate {
                                *previous = None;
                            }
                        })
                        .or_insert(bitrate);
                }
            }
        }
        bytes = &bytes[consumed..];
    }
    Ok(())
}

fn chapters(
    bytes: &[u8],
    depth: usize,
    remaining: &mut usize,
    output: &mut Vec<Chapter>,
) -> Result<(), MediaInfoError> {
    if depth > 10 {
        *remaining = 0;
        return Err(MediaInfoError::Parse(
            "MKV chapter nesting exceeds budget".into(),
        ));
    }
    let mut bytes = bytes;
    while !bytes.is_empty() {
        spend(remaining)?;
        let (id, data, consumed) = next_ebml_element(bytes)
            .ok_or_else(|| MediaInfoError::Parse("truncated chapter metadata".into()))?;
        if id == EBML_ID_CHAPTER_ATOM {
            let get = |id| find_first_direct_ebml_child(data, id);
            let uid = get(0x73c4).and_then(parse_ebml_uint);
            let seconds = |id| {
                get(id)
                    .and_then(parse_ebml_uint)
                    .map(|value| value as f64 / 1e9)
            };
            let title = get(0x80)
                .and_then(|display| find_first_direct_ebml_child(display, 0x85))
                .and_then(|bytes| parse_ebml_string(bytes).ok());
            output.push(Chapter {
                id: uid
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| output.len().to_string()),
                title,
                start_seconds: seconds(0x91).ok_or_else(|| {
                    MediaInfoError::Parse("chapter start timestamp missing".into())
                })?,
                end_seconds: seconds(0x92),
            });
            chapters(data, depth + 1, remaining, output)?;
        } else if id == EBML_ID_EDITION_ENTRY {
            chapters(data, depth + 1, remaining, output)?;
        }
        bytes = &bytes[consumed..];
    }
    Ok(())
}

fn attachments<R: Read + Seek>(
    scanner: &mut MkvRawScanner<R>,
    end: u64,
    remaining: &mut usize,
    bytes_left: &mut u64,
    output: &mut Vec<Attachment>,
) -> Result<(), MediaInfoError> {
    while scanner.position()? < end {
        spend(remaining)?;
        let header = scanner
            .read_element_header()?
            .ok_or_else(|| MediaInfoError::Parse("truncated attachment".into()))?;
        let child_end = bounded_end(header, end)?;
        if header.id == 0x61a7 {
            let mut attachment = Attachment::default();
            while scanner.position()? < child_end {
                spend(remaining)?;
                let field = scanner
                    .read_element_header()?
                    .ok_or_else(|| MediaInfoError::Parse("truncated attachment field".into()))?;
                let field_end = bounded_end(field, child_end)?;
                let size = field_end - field.data_offset;
                match field.id {
                    0x465c => attachment.size_bytes = size,
                    0x46ae | 0x466e | 0x4660 => {
                        if size > 64 * 1024 || size > *bytes_left {
                            *bytes_left = 0;
                            return Err(MediaInfoError::Parse(
                                "MKV attachment metadata budget exhausted".into(),
                            ));
                        }
                        *bytes_left -= size;
                        let value = scanner.read_bytes(size)?;
                        match field.id {
                            0x46ae => {
                                attachment.id = parse_ebml_uint(&value)
                                    .map(|id| id.to_string())
                                    .unwrap_or_default()
                            }
                            0x466e => attachment.name = parse_ebml_string(&value).ok(),
                            _ => attachment.media_type = parse_ebml_string(&value).ok(),
                        }
                    }
                    _ => {}
                }
                scanner.seek_to(field_end)?;
            }
            output.push(attachment);
        }
        scanner.seek_to(child_end)?;
    }
    Ok(())
}

fn last_cluster_duration<R: Read + Seek>(
    scanner: &mut MkvRawScanner<R>,
    end: u64,
    video: &RawTrack,
    scale: f64,
    remaining: &mut usize,
) -> Result<Option<f64>, MediaInfoError> {
    if !scale.is_finite() || scale <= 0.0 {
        return Ok(None);
    }
    let track_id = video
        .metadata
        .id
        .as_deref()
        .and_then(|id| id.parse::<u64>().ok());
    let frame_duration = video
        .metadata
        .declared_frame_rate
        .and_then(|rate| rate.as_f64())
        .filter(|rate| *rate > 0.0)
        .map(|rate| 1.0 / rate);
    let mut timestamp = None;
    let mut duration = None::<f64>;
    while scanner.position()? < end {
        spend(remaining)?;
        let header = scanner
            .read_element_header()?
            .ok_or_else(|| MediaInfoError::Parse("truncated final cluster".into()))?;
        let child_end = bounded_end(header, end)?;
        match header.id {
            EBML_ID_TIMESTAMP => {
                timestamp = scanner.read_unsigned_payload(child_end - header.data_offset)?
            }
            EBML_ID_SIMPLE_BLOCK | EBML_ID_BLOCK_GROUP => {
                let Some(timestamp) = timestamp else {
                    return Ok(None);
                };
                let mut block = None;
                let mut block_duration = None;
                if header.id == EBML_ID_SIMPLE_BLOCK {
                    block = scanner.read_block_header(child_end - header.data_offset, timestamp)?;
                } else {
                    while scanner.position()? < child_end {
                        spend(remaining)?;
                        let part = scanner.read_element_header()?.ok_or_else(|| {
                            MediaInfoError::Parse("truncated final block group".into())
                        })?;
                        let part_end = bounded_end(part, child_end)?;
                        if part.id == EBML_ID_BLOCK {
                            block = scanner
                                .read_block_header(part_end - part.data_offset, timestamp)?;
                        }
                        if part.id == 0x9b {
                            block_duration = scanner
                                .read_unsigned_payload(part_end - part.data_offset)?
                                .map(|value| value as f64 * scale / 1e9);
                        }
                        scanner.seek_to(part_end)?;
                    }
                }
                if let Some(block) = block.filter(|block| Some(block.track_number) == track_id) {
                    // A laced block needs its authored frame count to establish its end.
                    if block.lacing_type != 0 {
                        return Ok(None);
                    }
                    let Some(length) = block_duration.or(frame_duration) else {
                        return Ok(None);
                    };
                    let last = block.timestamp as f64 * scale / 1e9 + length;
                    duration = Some(duration.map_or(last, |previous| previous.max(last)));
                }
            }
            _ => {}
        }
        scanner.seek_to(child_end)?;
    }
    Ok(duration.filter(|value| value.is_finite() && *value > 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn element(id: &[u8], data: &[u8]) -> Vec<u8> {
        assert!(data.len() < 16383);
        if data.len() < 127 {
            [id, &[0x80 | data.len() as u8], data].concat()
        } else {
            [id, &(0x4000 | data.len() as u16).to_be_bytes(), data].concat()
        }
    }
    fn video(with_rate: bool) -> RawTrack {
        RawTrack {
            kind: TrackKind::Video,
            metadata: scryer_media_types::StreamMetadata {
                id: Some("1".into()),
                declared_frame_rate: with_rate
                    .then(|| scryer_media_types::Rational::new(25, 1).unwrap()),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    #[test]
    fn inventory_budget_exhaustion_is_distinct_from_malformed_metadata() {
        let mut oversized = [0x18, 0x53, 0x80, 0x67, 0xff, 0x10, 0x43, 0xa7, 0x70].to_vec();
        oversized.extend((0x1000_0000_u32 | (MAX_METADATA as u32 + 1)).to_be_bytes());
        oversized.resize(oversized.len() + MAX_METADATA as usize + 1, 0);
        let mut source = crate::source::BoundedSource::new(Cursor::new(oversized), 64);
        let mut scanner = MkvRawScanner::new(&mut source).unwrap();
        let report = read(&mut scanner, &mut [], &[], 1_000_000.0, false).report;
        assert_eq!(report.status, ProbeStatus::Incomplete);
        assert!(report.budget_exhausted);
        assert!(
            source.bytes_read < 64,
            "oversized metadata is rejected before reading its payload"
        );
        let mut nested = Vec::new();
        for _ in 0..12 {
            nested = element(&[0x45, 0xb9], &nested);
        }
        let bytes = element(
            &[0x18, 0x53, 0x80, 0x67],
            &element(&[0x10, 0x43, 0xa7, 0x70], &nested),
        );
        let mut scanner = MkvRawScanner::new(Cursor::new(bytes)).unwrap();
        let report = read(&mut scanner, &mut [], &[], 1_000_000.0, false).report;
        assert_eq!(report.status, ProbeStatus::Incomplete);
        assert!(report.budget_exhausted);
        let mut scanner = MkvRawScanner::new(Cursor::new(vec![0])).unwrap();
        let report = read(&mut scanner, &mut [], &[], 1_000_000.0, false).report;
        assert_eq!(report.status, ProbeStatus::Incomplete);
        assert!(!report.budget_exhausted);
    }

    #[test]
    fn track_statistics_use_uids_and_skip_payloads_without_borrowing_global_bitrate() {
        let tag = |uid: u8, value: &[u8]| {
            element(
                &[0x73, 0x73],
                &[
                    element(&[0x63, 0xc0], &element(&[0x63, 0xc5], &[uid])),
                    element(
                        &[0x67, 0xc8],
                        &[
                            element(&[0x45, 0xa3], b"BPS"),
                            element(&[0x44, 0x87], value),
                        ]
                        .concat(),
                    ),
                ]
                .concat(),
            )
        };
        let tags = [
            tag(0, b"999999999"),
            tag(42, b"8000000"),
            tag(43, b"640000"),
            tag(44, b"1234"),
            tag(44, b"5678"),
        ]
        .concat();
        let bytes = element(
            &[0x18, 0x53, 0x80, 0x67],
            &[
                element(&[0xec], &[0x55; 8192]),
                element(&[0x12, 0x54, 0xc3, 0x67], &tags),
            ]
            .concat(),
        );
        let mut source = crate::source::BoundedSource::new(Cursor::new(bytes), 512);
        let mut scanner = MkvRawScanner::new(&mut source).unwrap();
        let mut tracks = vec![
            video(true),
            RawTrack::default(),
            RawTrack::default(),
            RawTrack::default(),
        ];
        let details = read(
            &mut scanner,
            &mut tracks,
            &[(42, 0), (43, 1), (44, 2)],
            1_000_000.0,
            false,
        );
        assert_ne!(
            details.report.status,
            ProbeStatus::Incomplete,
            "{:?}",
            details.report
        );
        assert_eq!(tracks[0].bit_rate_bps, Some(8_000_000));
        assert_eq!(tracks[1].bit_rate_bps, Some(640_000));
        assert_eq!(tracks[0].metadata.bitrate_provenance, Provenance::Container);
        assert!(
            tracks[2].bit_rate_bps.is_none(),
            "conflicting declarations remain unknown"
        );
        assert!(
            tracks[3].bit_rate_bps.is_none(),
            "a global tag cannot become a stream bitrate"
        );
        assert!(
            details
                .report
                .warnings
                .iter()
                .any(|warning| warning.code == "mkv_statistics_conflict")
        );
        assert!(source.bytes_read < 512);
        for value in [b"-1".as_slice(), b"0", b"9223372036854775808", b"NaN"] {
            let mut output = std::collections::BTreeMap::new();
            statistics(&tag(42, value), &mut MAX_HEADERS.clone(), &mut output).unwrap();
            assert_eq!(output.get(&42), Some(&None));
        }
        assert!(statistics(&tags, &mut 2, &mut Default::default()).is_err());
    }

    #[test]
    fn missing_container_duration_uses_final_block_end_and_keeps_uncertainty() {
        let mut header_budget = MAX_HEADERS;
        let timestamp = element(&[0xe7], &1000_u16.to_be_bytes());
        let block = element(&[0xa3], &[0x81, 0, 40, 0, 0x55]);
        let bytes = [timestamp, block].concat();
        let mut scanner = MkvRawScanner::new(Cursor::new(bytes.clone())).unwrap();
        let duration = last_cluster_duration(
            &mut scanner,
            bytes.len() as u64,
            &video(true),
            1_000_000.0,
            &mut header_budget,
        )
        .unwrap();
        assert_eq!(duration, Some(1.08));
        let mut scanner = MkvRawScanner::new(Cursor::new(bytes.clone())).unwrap();
        assert_eq!(
            last_cluster_duration(
                &mut scanner,
                bytes.len() as u64,
                &video(false),
                1_000_000.0,
                &mut header_budget
            )
            .unwrap(),
            None
        );
        let mut scanner = MkvRawScanner::new(Cursor::new(bytes.clone())).unwrap();
        assert!(
            last_cluster_duration(
                &mut scanner,
                bytes.len() as u64,
                &video(true),
                1_000_000.0,
                &mut 0
            )
            .is_err()
        );
    }
    #[test]
    fn attachment_payload_is_not_read_and_chapters_retain_titles() {
        let mut header_budget = MAX_HEADERS;
        let mut metadata_budget = MAX_METADATA;
        let file = [
            element(&[0x46, 0xae], &[7]),
            element(&[0x46, 0x6e], b"font.ttf"),
            element(&[0x46, 0x60], b"font/ttf"),
            element(&[0x46, 0x5c], &[0x55; 80]),
        ]
        .concat();
        let bytes = element(&[0x61, 0xa7], &file);
        let mut source = crate::source::BoundedSource::new(Cursor::new(bytes.clone()), 60);
        let mut scanner = MkvRawScanner::new(&mut source).unwrap();
        let mut output = Vec::new();
        attachments(
            &mut scanner,
            bytes.len() as u64,
            &mut header_budget,
            &mut metadata_budget,
            &mut output,
        )
        .unwrap();
        assert_eq!(output[0].id, "7");
        assert_eq!(output[0].size_bytes, 80);
        assert_eq!(output[0].name.as_deref(), Some("font.ttf"));
        assert!(source.bytes_read < 60);
        let atom = [
            element(&[0x73, 0xc4], &[4]),
            element(&[0x91], &1_500_000_000_u64.to_be_bytes()),
            element(&[0x80], &element(&[0x85], b"Opening")),
        ]
        .concat();
        let mut chapters_out = Vec::new();
        chapters(
            &element(&[0x45, 0xb9], &element(&[0xb6], &atom)),
            0,
            &mut header_budget,
            &mut chapters_out,
        )
        .unwrap();
        assert_eq!(chapters_out[0].title.as_deref(), Some("Opening"));
        assert_eq!(chapters_out[0].start_seconds, 1.5);
    }
}
