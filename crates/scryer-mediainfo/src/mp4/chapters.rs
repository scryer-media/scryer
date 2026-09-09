use scryer_media_types::Chapter;

/// Nero chapter timestamps use 100 ns units, independently of movie timescale.
/// The one-byte count and title lengths bound this inventory to 255 entries.
pub(super) fn nero(data: &[u8]) -> Option<Vec<Chapter>> {
    let mut at = match *data.first()? {
        0 => 4,
        1 => 8,
        _ => return None,
    };
    if data.get(1..4)? != [0, 0, 0] {
        return None;
    }
    let count = usize::from(*data.get(at)?);
    at += 1;
    let mut output: Vec<Chapter> = Vec::with_capacity(count);
    for index in 0..count {
        let ticks = u64::from_be_bytes(data.get(at..at + 8)?.try_into().ok()?);
        let length = usize::from(*data.get(at + 8)?);
        at += 9;
        let text = std::str::from_utf8(data.get(at..at + length)?).ok()?;
        at += length;
        let start = ticks as f64 / 10_000_000.0;
        if let Some(previous) = output.last_mut() {
            if start < previous.start_seconds {
                return None;
            }
            previous.end_seconds = Some(start);
        }
        output.push(Chapter {
            id: index.to_string(),
            title: (!text.is_empty()).then(|| text.to_owned()),
            start_seconds: start,
            end_seconds: None,
        });
    }
    Some(output)
}

pub(super) fn quicktime(
    source: &mut dyn crate::source::MediaSource,
    track: &mp4parse::Track,
    prepared: &[u8],
    remaining: &mut usize,
    bytes_left: &mut usize,
) -> Result<Vec<Chapter>, (&'static str, bool)> {
    use std::io::SeekFrom;
    let invalid = (
        "Invalid or unavailable QuickTime chapter sample references",
        false,
    );
    let budget = ("QuickTime chapter inventory budget exhausted", true);
    let id = track.track_id.ok_or(invalid)?;
    let scale = track
        .timescale
        .map(|scale| scale.0)
        .filter(|scale| *scale > 0)
        .ok_or(invalid)?;
    let (ranges, complete) =
        super::track_sample_ranges_bounded(track, u32::try_from(*remaining).map_err(|_| budget)?)
            .ok_or(invalid)?;
    if !complete {
        return Err(budget);
    }
    *remaining = remaining.checked_sub(ranges.len()).ok_or(budget)?;
    let timing = sample_times(track, ranges.len()).ok_or(invalid)?;
    let mut chapters = Vec::with_capacity(ranges.len());
    for (index, ((offset, size), (start, end))) in ranges.into_iter().zip(timing).enumerate() {
        if size < 2
            || offset
                .checked_add(size as u64)
                .is_none_or(|end| end > source.len())
        {
            return Err(invalid);
        }
        *bytes_left = bytes_left.checked_sub(2).ok_or(budget)?;
        source.seek(SeekFrom::Start(offset)).map_err(|_| invalid)?;
        let mut length = [0; 2];
        source.read_exact(&mut length).map_err(|_| invalid)?;
        let length = usize::from(u16::from_be_bytes(length));
        if length > size - 2 {
            return Err(invalid);
        }
        *bytes_left = bytes_left.checked_sub(length).ok_or(budget)?;
        let mut text = vec![0; length];
        source.read_exact(&mut text).map_err(|_| invalid)?;
        let title =
            decode_title(&text).ok_or(("Invalid QuickTime chapter text encoding", false))?;
        chapters.push(Chapter {
            id: format!("{id}:{index}"),
            title: (!title.is_empty()).then_some(title),
            start_seconds: start as f64 / scale as f64,
            end_seconds: Some(end as f64 / scale as f64),
        });
    }
    let (movie_scale, edits) = super::timing::track_edits(prepared, id).ok_or(invalid)?;
    if edits.is_empty() {
        return Ok(chapters);
    }
    apply_edits(
        &chapters,
        &edits,
        u64::from(movie_scale.ok_or(invalid)?),
        scale,
        remaining,
    )
}

fn sample_times(track: &mp4parse::Track, count: usize) -> Option<Vec<(i64, i64)>> {
    let entries = &track.stts.as_ref()?.samples;
    if entries.iter().try_fold(0_usize, |sum, entry| {
        sum.checked_add(entry.sample_count as usize)
    })? != count
    {
        return None;
    }
    let mut times = Vec::with_capacity(count);
    let mut decode = 0_i64;
    for entry in entries {
        if entry.sample_count == 0 || entry.sample_delta == 0 {
            return None;
        }
        for _ in 0..entry.sample_count {
            let end = decode.checked_add(i64::from(entry.sample_delta))?;
            times.push((decode, end));
            decode = end;
        }
    }
    if let Some(offsets) = &track.ctts {
        if offsets.samples.iter().try_fold(0_usize, |sum, entry| {
            sum.checked_add(entry.sample_count as usize)
        })? != count
        {
            return None;
        }
        let mut index = 0;
        for entry in &offsets.samples {
            let offset = match entry.time_offset {
                mp4parse::TimeOffsetVersion::Version0(value) => i64::from(value),
                mp4parse::TimeOffsetVersion::Version1(value) => i64::from(value),
            };
            for _ in 0..entry.sample_count {
                let time = times.get_mut(index)?;
                *time = (time.0.checked_add(offset)?, time.1.checked_add(offset)?);
                index += 1;
            }
        }
    }
    Some(times)
}

fn decode_title(bytes: &[u8]) -> Option<String> {
    match bytes.get(..2) {
        Some([0xfe, 0xff] | [0xff, 0xfe]) => {
            if !bytes.len().is_multiple_of(2) {
                return None;
            }
            let little = bytes[0] == 0xff;
            let units: Vec<_> = bytes[2..]
                .chunks_exact(2)
                .map(|pair| {
                    if little {
                        u16::from_le_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_be_bytes([pair[0], pair[1]])
                    }
                })
                .collect();
            String::from_utf16(&units).ok()
        }
        _ => std::str::from_utf8(bytes).ok().map(str::to_owned),
    }
}

fn apply_edits(
    chapters: &[Chapter],
    edits: &[super::timing::Edit],
    movie_scale: u64,
    scale: u64,
    remaining: &mut usize,
) -> Result<Vec<Chapter>, (&'static str, bool)> {
    let invalid = (
        "QuickTime chapter edit timeline could not be resolved",
        false,
    );
    if movie_scale == 0 || scale == 0 {
        return Err(invalid);
    }
    let media_end = chapters
        .iter()
        .filter_map(|chapter| chapter.end_seconds)
        .max_by(f64::total_cmp)
        .ok_or(invalid)?;
    let mut output = Vec::new();
    let mut position = 0.0;
    for (edit_index, edit) in edits.iter().enumerate() {
        if edit.media_time < -1 || edit.rate != 65536 {
            return Err(invalid);
        }
        let duration = edit.duration as f64 / movie_scale as f64;
        if edit.media_time == -1 {
            position += duration;
            continue;
        }
        let start = edit.media_time as f64 / scale as f64;
        let end = if duration == 0.0 {
            media_end
        } else {
            start + duration
        };
        if start > end || end > media_end + 1.0 / scale as f64 {
            return Err(invalid);
        }
        for chapter in chapters {
            let input = chapter.start_seconds.max(start);
            let output_end = chapter.end_seconds.ok_or(invalid)?.min(end);
            if input >= output_end {
                continue;
            }
            // Repeated edits can expand a title's chapter inventory.
            if output.len() >= chapters.len() {
                *remaining = remaining
                    .checked_sub(1)
                    .ok_or(("QuickTime chapter edit expansion exceeds budget", true))?;
            }
            output.push(Chapter {
                id: format!("{}:{edit_index}", chapter.id),
                title: chapter.title.clone(),
                start_seconds: position + input - start,
                end_seconds: Some(position + output_end - start),
            });
        }
        position += end - start;
    }
    output.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quicktime_titles_and_edit_trims_keep_the_authored_timeline() {
        use super::super::timing::Edit;
        assert_eq!(
            decode_title(&[0xfe, 0xff, 0x5e, 0x55]).as_deref(),
            Some("幕")
        );
        assert!(decode_title(&[0xfe, 0xff, 0xd8, 0]).is_none());
        let chapters = vec![
            Chapter {
                id: "1".into(),
                title: Some("Opening".into()),
                start_seconds: 0.0,
                end_seconds: Some(2.0),
            },
            Chapter {
                id: "2".into(),
                title: Some("Ending".into()),
                start_seconds: 2.0,
                end_seconds: Some(4.0),
            },
        ];
        let edits = [
            Edit {
                duration: 500,
                media_time: -1,
                rate: 65536,
            },
            Edit {
                duration: 2000,
                media_time: 1000,
                rate: 65536,
            },
        ];
        let mapped = apply_edits(&chapters, &edits, 1000, 1000, &mut 10).unwrap();
        assert_eq!(
            (mapped[0].start_seconds, mapped[0].end_seconds),
            (0.5, Some(1.5))
        );
        assert_eq!(
            (mapped[1].start_seconds, mapped[1].end_seconds),
            (1.5, Some(2.5))
        );
        assert_eq!(mapped[1].title.as_deref(), Some("Ending"));
        let repeated = [Edit {
            duration: 0,
            media_time: 0,
            rate: 65536,
        }; 2];
        assert!(
            apply_edits(&chapters, &repeated, 1000, 1000, &mut 0)
                .unwrap_err()
                .1
        );
        assert!(
            apply_edits(
                &chapters,
                &[Edit {
                    duration: 1000,
                    media_time: 0,
                    rate: 0
                }],
                1000,
                1000,
                &mut 10
            )
            .is_err()
        );
    }

    #[test]
    fn nero_chapters_preserve_unicode_and_fractional_timestamps() {
        for version in [0, 1] {
            let mut bytes = vec![0; if version == 0 { 4 } else { 8 }];
            bytes[0] = version;
            bytes.push(2);
            for (ticks, title) in [(0_u64, "Opening"), (12_345_000, "幕間")] {
                bytes.extend(ticks.to_be_bytes());
                bytes.push(title.len() as u8);
                bytes.extend(title.as_bytes());
            }
            let chapters = nero(&bytes).unwrap();
            assert_eq!(chapters[0].end_seconds, Some(1.2345));
            assert_eq!(chapters[1].start_seconds, 1.2345);
            assert_eq!(chapters[1].title.as_deref(), Some("幕間"));
            assert!(chapters[1].end_seconds.is_none());
            bytes.pop();
            assert!(
                nero(&bytes).is_none(),
                "truncated titles are not complete chapter inventories"
            );
        }
        assert!(nero(&[2, 0, 0, 0, 0]).is_none());
        assert!(nero(&[0, 0, 0, 0, 255]).is_none());
    }
}
