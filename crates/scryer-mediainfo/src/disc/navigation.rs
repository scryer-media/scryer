use scryer_media_types::{Chapter, DiscSegment, DiscTitle, ProbeReport, ProbeStatus, ProbeWarning};

use super::image::ImageError;

type Result<T> = std::result::Result<T, ImageError>;
fn bounded(data: &[u8], at: usize, length: usize) -> Result<&[u8]> {
    let end = at
        .checked_add(length)
        .ok_or(ImageError::Malformed("navigation offset overflow"))?;
    data.get(at..end)
        .ok_or(ImageError::Malformed("navigation field outside section"))
}
pub(super) fn u16be(data: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(
        bounded(data, at, 2)?.try_into().unwrap(),
    ))
}
pub(super) fn u32be(data: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        bounded(data, at, 4)?.try_into().unwrap(),
    ))
}
fn section(data: &[u8], at: usize) -> Result<&[u8]> {
    let length = u32be(data, at)? as usize;
    bounded(
        data,
        at.checked_add(4)
            .ok_or(ImageError::Malformed("navigation section overflow"))?,
        length,
    )
}
fn dvd_table(data: &[u8], pointer: usize) -> Result<&[u8]> {
    let at = (u32be(data, pointer)? as usize)
        .checked_mul(2048)
        .filter(|at| *at >= 2048)
        .ok_or(ImageError::Malformed("DVD table sector offset"))?;
    let header = bounded(data, at, 8)?;
    let length = (u32be(header, 4)? as usize)
        .checked_add(1)
        .filter(|length| *length >= 8)
        .ok_or(ImageError::Malformed("DVD table length"))?;
    bounded(data, at, length)
}
pub(super) fn warning(report: &mut ProbeReport, code: &str, message: &str) {
    report.warnings.push(ProbeWarning {
        code: code.into(),
        message: message.into(),
        ..Default::default()
    });
}

pub(super) fn playlist(data: &[u8], id: String) -> Result<DiscTitle> {
    if data.get(..4) != Some(b"MPLS")
        || !matches!(data.get(4..8), Some(b"0100" | b"0200" | b"0300"))
    {
        return Err(ImageError::Unsupported(
            "Blu-ray playlist signature or version",
        ));
    }
    let list = section(data, u32be(data, 8)? as usize)?;
    let count = usize::from(u16be(list, 2)?);
    if count == 0 || count > 4096 {
        return Err(ImageError::Unsupported("Blu-ray play item count"));
    }
    let mut title = DiscTitle {
        id,
        angle_count: 1,
        report: ProbeReport {
            status: ProbeStatus::Incomplete,
            ..Default::default()
        },
        ..Default::default()
    };
    if u16be(list, 4)? != 0 {
        warning(
            &mut title.report,
            "bluray_subpaths",
            "Supplementary Blu-ray subpaths are inventoried only through primary playback streams",
        );
    }
    let mut at = 6;
    let mut starts = Vec::with_capacity(count);
    let mut duration = 0.0;
    for _ in 0..count {
        let length = usize::from(u16be(list, at)?);
        let item = list
            .get(at + 2..at + 2 + length)
            .filter(|item| item.len() >= 32)
            .ok_or(ImageError::Malformed("short Blu-ray play item"))?;
        at += 2 + length;
        let clip = std::str::from_utf8(&item[..5])
            .map_err(|_| ImageError::Malformed("invalid Blu-ray clip ID"))?;
        if !clip.bytes().all(|byte| byte.is_ascii_digit()) || &item[5..9] != b"M2TS" {
            return Err(ImageError::Unsupported("Blu-ray clip identifier"));
        }
        let input = u32be(item, 12)?;
        let output = u32be(item, 16)?;
        if output <= input {
            return Err(ImageError::Malformed("Blu-ray play item has invalid trim"));
        }
        if item[29] != 0 {
            title.report.status = ProbeStatus::Unsupported;
            warning(
                &mut title.report,
                "bluray_still",
                "Playlist contains a still or interactive hold",
            );
        }
        if item[10] & 0x10 != 0 {
            let angles = *item
                .get(32)
                .ok_or(ImageError::Malformed("missing Blu-ray angle count"))?;
            if angles == 0 || 34 + usize::from(angles - 1) * 10 > item.len() {
                return Err(ImageError::Malformed("short Blu-ray angle inventory"));
            }
            title.angle_count = title.angle_count.max(u32::from(angles));
        }
        starts.push(duration);
        duration += f64::from(output - input) / 45_000.0;
        title.segments.push(DiscSegment {
            path: format!("BDMV/STREAM/{clip}.M2TS"),
            in_seconds: f64::from(input) / 45_000.0,
            out_seconds: f64::from(output) / 45_000.0,
            angle: 1,
            sequence_id: Some(item[11]),
        });
    }
    title.duration_seconds = Some(duration);
    let mark_at = u32be(data, 12)? as usize;
    if mark_at != 0 {
        let marks = section(data, mark_at)?;
        let count = usize::from(u16be(marks, 0)?);
        if count > 8192 || 2 + count * 14 > marks.len() {
            return Err(ImageError::Malformed("Blu-ray mark table bounds"));
        }
        for (index, mark) in marks[2..2 + count * 14].chunks_exact(14).enumerate() {
            let reference = usize::from(u16be(mark, 2)?);
            let segment = title.segments.get(reference).ok_or(ImageError::Malformed(
                "Blu-ray mark references missing play item",
            ))?;
            let time = f64::from(u32be(mark, 4)?) / 45_000.0;
            if time < segment.in_seconds || time > segment.out_seconds {
                return Err(ImageError::Malformed("Blu-ray mark outside play item"));
            }
            if mark[1] == 1 {
                title.chapters.push(Chapter {
                    id: index.to_string(),
                    start_seconds: starts[reference] + time - segment.in_seconds,
                    ..Default::default()
                });
            }
        }
        title
            .chapters
            .sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
        for index in 0..title.chapters.len() {
            title.chapters[index].end_seconds = Some(
                title
                    .chapters
                    .get(index + 1)
                    .map_or(duration, |chapter| chapter.start_seconds),
            );
        }
    }
    Ok(title)
}

#[derive(Debug)]
pub(super) struct DvdReference {
    pub id: String,
    pub set: u8,
    pub title: u8,
    pub angles: u8,
    pub dynamic: bool,
}
pub(super) fn dvd_titles(data: &[u8]) -> Result<Vec<DvdReference>> {
    if data.get(..12) != Some(b"DVDVIDEO-VMG") {
        return Err(ImageError::Malformed("DVD VMG signature"));
    }
    let table = dvd_table(data, 0xc4)?;
    let count = usize::from(u16be(table, 0)?);
    if count == 0 || count > 99 {
        return Err(ImageError::Malformed("DVD title count"));
    }
    let table = bounded(table, 8, count * 12)?;
    table
        .chunks_exact(12)
        .enumerate()
        .map(|(index, title)| {
            if title[6] == 0
                || title[6] > 99
                || title[7] == 0
                || title[7] > 99
                || !(1..=9).contains(&title[1])
            {
                return Err(ImageError::Malformed("DVD title reference"));
            }
            Ok(DvdReference {
                id: (index + 1).to_string(),
                set: title[6],
                title: title[7],
                angles: title[1],
                dynamic: title[0] & 0x40 != 0,
            })
        })
        .collect()
}

#[derive(Debug, Clone)]
pub(super) struct DvdCell {
    pub first_sector: u32,
    pub last_sector: u32,
}
pub(super) fn dvd_title(
    data: &[u8],
    reference: &DvdReference,
) -> Result<(DiscTitle, Vec<DvdCell>)> {
    if data.get(..12) != Some(b"DVDVIDEO-VTS") {
        return Err(ImageError::Malformed("DVD VTS signature"));
    }
    let mut title = DiscTitle {
        id: reference.id.clone(),
        angle_count: u32::from(reference.angles),
        report: ProbeReport {
            status: ProbeStatus::Incomplete,
            ..Default::default()
        },
        ..Default::default()
    };
    if reference.dynamic {
        title.report.status = ProbeStatus::Unsupported;
        warning(
            &mut title.report,
            "dvd_dynamic_navigation",
            "Title requires DVD navigation execution",
        );
        return Ok((title, vec![]));
    }
    let ptt = dvd_table(data, 0xc8)?;
    let title_count = usize::from(u16be(ptt, 0)?);
    if reference.title == 0 || title_count > 99 || usize::from(reference.title) > title_count {
        return Err(ImageError::Malformed("DVD title has no PTT entry"));
    }
    let offsets = bounded(ptt, 8, title_count * 4)?;
    let start = u32be(offsets, (usize::from(reference.title) - 1) * 4)? as usize;
    let end = if usize::from(reference.title) < title_count {
        u32be(offsets, usize::from(reference.title) * 4)? as usize
    } else {
        ptt.len()
    };
    let parts = ptt
        .get(start..end)
        .filter(|parts| start >= 8 + offsets.len() && !parts.is_empty() && parts.len() % 4 == 0)
        .ok_or(ImageError::Malformed("DVD PTT references"))?;
    let pgc_id = u16be(parts, 0)?;
    if parts
        .chunks_exact(4)
        .any(|part| u16be(part, 0).ok() != Some(pgc_id))
    {
        title.report.status = ProbeStatus::Unsupported;
        warning(
            &mut title.report,
            "dvd_multiple_pgcs",
            "Title spans program chains requiring navigation inspection",
        );
        return Ok((title, vec![]));
    }
    let table = dvd_table(data, 0xcc)?;
    let pgc_count = usize::from(u16be(table, 0)?);
    if pgc_id == 0 || usize::from(pgc_id) > pgc_count {
        return Err(ImageError::Malformed("DVD missing PGC"));
    }
    let entries = bounded(table, 8, pgc_count * 8)?;
    let pgc_at = u32be(entries, (usize::from(pgc_id) - 1) * 8 + 4)? as usize;
    let mut pgc_end = table.len();
    for entry in entries.chunks_exact(8) {
        let offset = u32be(entry, 4)? as usize;
        if offset < 8 + entries.len() || offset >= table.len() {
            return Err(ImageError::Malformed("DVD PGC reference bounds"));
        }
        if offset > pgc_at {
            pgc_end = pgc_end.min(offset);
        }
    }
    let pgc = table
        .get(pgc_at..pgc_end)
        .filter(|pgc| pgc.len() >= 236)
        .ok_or(ImageError::Malformed("short DVD PGC"))?;
    if pgc[163] != 0 || pgc[162] != 0 {
        title.report.status = ProbeStatus::Unsupported;
        warning(
            &mut title.report,
            "dvd_interactive_pgc",
            "PGC uses a still or non-sequential playback mode",
        );
        return Ok((title, vec![]));
    }
    let program_at = usize::from(u16be(pgc, 230)?);
    let cell_at = usize::from(u16be(pgc, 232)?);
    let programs = bounded(pgc, program_at, usize::from(pgc[2]))?;
    let cells = bounded(pgc, cell_at, usize::from(pgc[3]) * 24)?;
    let mut regions = vec![(program_at, programs.len()), (cell_at, cells.len())];
    let positions_at = usize::from(u16be(pgc, 234)?);
    if positions_at != 0 {
        let positions = bounded(pgc, positions_at, usize::from(pgc[3]) * 4)?;
        regions.push((positions_at, positions.len()));
    }
    if programs.is_empty()
        || programs.iter().any(|cell| *cell == 0 || *cell > pgc[3])
        || programs.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(ImageError::Malformed(
            "DVD program ordering or cell reference",
        ));
    }
    let first_program = u16be(parts, 2)?;
    let first_cell = programs
        .get(
            usize::from(first_program)
                .checked_sub(1)
                .ok_or(ImageError::Malformed("zero DVD starting program"))?,
        )
        .copied()
        .ok_or(ImageError::Malformed("DVD starting program outside chain"))?;
    let mut previous_program = None;
    for part in parts.chunks_exact(4) {
        let program = u16be(part, 2)?;
        if program == 0
            || usize::from(program) > programs.len()
            || previous_program.is_some_and(|previous| previous >= program)
        {
            return Err(ImageError::Malformed(
                "DVD chapter programs are not sequential",
            ));
        }
        previous_program = Some(program);
    }
    let command_at = usize::from(u16be(pgc, 228)?);
    if command_at != 0 {
        let header = bounded(pgc, command_at, 8)?;
        let pre = usize::from(u16be(header, 0)?);
        let post = usize::from(u16be(header, 2)?);
        let cell = usize::from(u16be(header, 4)?);
        let count = pre + post + cell;
        if count > 255 {
            return Err(ImageError::Malformed("DVD command count exceeds 255"));
        }
        let commands = bounded(pgc, command_at, 8 + count * 8)?;
        regions.push((command_at, commands.len()));
        let commands = &commands[8..];
        if commands[..pre * 8].iter().any(|byte| *byte != 0)
            || commands[(pre + post) * 8..].iter().any(|byte| *byte != 0)
            || commands[pre * 8..(pre + post) * 8]
                .chunks_exact(8)
                .any(|command| !dvd_terminal_post_command(command))
        {
            title.report.status = ProbeStatus::Unsupported;
            warning(
                &mut title.report,
                "dvd_pgc_commands",
                "Playback commands require DVD navigation interpretation",
            );
        }
    }
    regions.sort_unstable();
    let mut region_end = 236;
    for (start, length) in regions {
        if start < region_end {
            return Err(ImageError::Malformed("overlapping DVD PGC structures"));
        }
        region_end = start + length;
    }
    let mut duration = 0.0;
    let mut cell_starts = Vec::new();
    let mut selected = Vec::new();
    let mut in_angle_block = false;
    for (index, cell) in cells.chunks_exact(24).enumerate() {
        cell_starts.push(duration);
        if index + 1 < usize::from(first_cell) {
            continue;
        }
        let block_type = (cell[0] >> 4) & 3;
        let block_mode = cell[0] >> 6;
        if block_type == 1 {
            if block_mode == 1 {
                in_angle_block = true;
            } else if in_angle_block && matches!(block_mode, 2 | 3) {
                if block_mode == 3 {
                    in_angle_block = false;
                }
                continue;
            } else {
                return Err(ImageError::Malformed("DVD angle cell sequence"));
            }
        } else if block_type != 0 || in_angle_block {
            return Err(ImageError::Malformed("DVD cell block type"));
        }
        if cell[0] & 4 != 0 {
            title.report.status = ProbeStatus::Unsupported;
            warning(
                &mut title.report,
                "dvd_interleaved_angle",
                "Interleaved angle units require NAV packet mapping",
            );
        }
        if cell[2] != 0 || cell[3] != 0 || cell[1] & 0x40 != 0 {
            title.report.status = ProbeStatus::Unsupported;
            warning(
                &mut title.report,
                "dvd_cell_navigation",
                "Cell requires a command or interactive still",
            );
        }
        let first_sector = u32be(cell, 8)?;
        let last_sector = u32be(cell, 20)?;
        if last_sector < first_sector {
            return Err(ImageError::Malformed("DVD cell sector ordering"));
        }
        let seconds = dvd_time(&cell[4..8])?;
        title.segments.push(DiscSegment {
            path: format!(
                "VIDEO_TS/VTS_{:02}_TITLE.VOB#{}-{}",
                reference.set, first_sector, last_sector
            ),
            in_seconds: 0.0,
            out_seconds: seconds,
            angle: 1,
            sequence_id: None,
        });
        duration += seconds;
        selected.push(DvdCell {
            first_sector,
            last_sector,
        });
    }
    if in_angle_block {
        return Err(ImageError::Malformed("unfinished DVD angle block"));
    }
    for (index, part) in parts.chunks_exact(4).enumerate() {
        let program = usize::from(u16be(part, 2)?);
        let cell = *programs
            .get(
                program
                    .checked_sub(1)
                    .ok_or(ImageError::Malformed("zero DVD program reference"))?,
            )
            .ok_or(ImageError::Malformed("DVD chapter program reference"))?;
        let start = *cell_starts
            .get(
                usize::from(cell)
                    .checked_sub(1)
                    .ok_or(ImageError::Malformed("zero DVD cell reference"))?,
            )
            .ok_or(ImageError::Malformed("DVD chapter cell reference"))?;
        title.chapters.push(Chapter {
            id: (index + 1).to_string(),
            start_seconds: start,
            ..Default::default()
        });
    }
    title.duration_seconds = Some(duration);
    title.streams = dvd_streams(data, pgc)?;
    for index in 0..title.chapters.len() {
        title.chapters[index].end_seconds = Some(
            title
                .chapters
                .get(index + 1)
                .map_or(duration, |chapter| chapter.start_seconds),
        );
    }
    Ok((title, selected))
}

// Recognize only unconditional post-title exits. No VM registers, conditions,
// calls, title jumps, or menu code are evaluated to construct the timeline.
fn dvd_terminal_post_command(command: &[u8]) -> bool {
    match command {
        [0, 0, 0, 0, 0, 0, 0, 0] | [0x30, 1, 0, 0, 0, 0, 0, 0] => true,
        [0x30, 6, 0, 0, 0, 0x42, 0, 0] => true, // VMGM title menu
        [0x30, 6, 0, title, set, menu, 0, 0] => {
            (1..=99).contains(title) && (1..=99).contains(set) && (0x83..=0x87).contains(menu)
        }
        _ => false,
    }
}

fn dvd_streams(data: &[u8], pgc: &[u8]) -> Result<Vec<scryer_media_types::StreamDetail>> {
    use scryer_media_types::{Provenance, Rational, StreamDetail, StreamKind};
    let attributes = data
        .get(0x200..0x318)
        .ok_or(ImageError::Malformed("short DVD stream attributes"))?;
    let mut streams = Vec::new();
    let pal = (attributes[0] >> 4) & 3 == 1;
    let picture = (attributes[1] >> 2) & 3;
    let mut video = StreamDetail {
        kind: StreamKind::Video,
        codec: match attributes[0] >> 6 {
            0 => Some("mpeg1video".into()),
            1 => Some("mpeg2video".into()),
            _ => None,
        },
        width: Some(match picture {
            0 => 720,
            1 => 704,
            _ => 352,
        }),
        height: Some(if pal {
            if picture == 3 { 288 } else { 576 }
        } else if picture == 3 {
            240
        } else {
            480
        }),
        ..Default::default()
    };
    video.metadata.id = Some("00e0".into());
    video.metadata.display_aspect_ratio = match (attributes[0] >> 2) & 3 {
        0 => Rational::new(4, 3),
        3 => Rational::new(16, 9),
        _ => None,
    };
    streams.push(video);
    let audio_count = usize::from(attributes[3]);
    if audio_count > 8 {
        return Err(ImageError::Malformed("DVD audio stream count"));
    }
    for index in 0..audio_count {
        let control = u16be(pgc, 12 + index * 2)?;
        if control & 0x8000 == 0 {
            continue;
        }
        let attr = &attributes[4 + index * 8..12 + index * 8];
        let format = attr[0] >> 5;
        let number = (control >> 8) & 7;
        let (codec, id) = match format {
            0 => ("ac3", 0xbd80 + number),
            2 | 3 => ("mp2", 0x00c0 + number),
            4 => ("pcm_dvd", 0xbda0 + number),
            6 => ("dts", 0xbd88 + number),
            _ => continue,
        };
        let language = ((attr[0] >> 2) & 3 == 1)
            .then(|| dvd_language(&attr[2..4]))
            .flatten();
        let mut stream = StreamDetail {
            kind: StreamKind::Audio,
            codec: Some(codec.into()),
            channels: Some(i32::from(attr[1] & 7) + 1),
            language: language.clone(),
            ..Default::default()
        };
        stream.metadata.id = Some(format!("{id:04x}"));
        stream.metadata.original_language = language;
        stream.metadata.language_provenance = if stream.language.is_some() {
            Provenance::Container
        } else {
            Provenance::Unknown
        };
        stream.metadata.sample_rate = match (attr[1] >> 4) & 3 {
            0 => Some(48_000),
            1 => Some(96_000),
            _ => None,
        };
        if format == 4 {
            stream.metadata.sample_bit_depth = match attr[1] >> 6 {
                0 => Some(16),
                1 => Some(20),
                2 => Some(24),
                _ => None,
            };
        }
        stream.metadata.disposition.commentary = match attr[5] {
            1 | 2 => Some(false),
            3 | 4 => Some(true),
            _ => None,
        };
        stream.metadata.disposition.visual_impaired = match attr[5] {
            1 | 3 | 4 => Some(false),
            2 => Some(true),
            _ => None,
        };
        streams.push(stream);
    }
    let count = usize::from(attributes[0x55]);
    if count > 32 {
        return Err(ImageError::Malformed("DVD subtitle stream count"));
    }
    for index in 0..count {
        let control = u32be(pgc, 28 + index * 4)?;
        if control & 0x8000_0000 == 0 {
            continue;
        }
        let attr = &attributes[0x56 + index * 6..0x5c + index * 6];
        let language = (attr[0] & 3 == 1)
            .then(|| dvd_language(&attr[2..4]))
            .flatten();
        let shifts: &[u32] = if (attributes[0] >> 2) & 3 == 3 {
            &[16, 8, 0]
        } else {
            &[24]
        };
        for shift in shifts {
            let id = format!("{:04x}", 0xbd20 + ((control >> shift) & 31));
            if streams
                .iter()
                .any(|stream| stream.metadata.id.as_ref() == Some(&id))
            {
                continue;
            }
            let mut stream = StreamDetail {
                kind: StreamKind::Subtitle,
                codec: Some("dvd_subtitle".into()),
                language: language.clone(),
                ..Default::default()
            };
            stream.metadata.id = Some(id);
            stream.metadata.original_language = language.clone();
            stream.metadata.language_provenance = if language.is_some() {
                Provenance::Container
            } else {
                Provenance::Unknown
            };
            if attr[5] != 0 {
                stream.metadata.disposition.forced = Some(attr[5] == 9);
                stream.metadata.disposition.hearing_impaired = Some(matches!(attr[5], 5..=7));
                stream.metadata.disposition.commentary = Some(matches!(attr[5], 13..=15));
            }
            streams.push(stream);
        }
    }
    Ok(streams)
}
fn dvd_language(bytes: &[u8]) -> Option<String> {
    bytes
        .iter()
        .all(u8::is_ascii_alphabetic)
        .then(|| String::from_utf8_lossy(bytes).to_ascii_lowercase())
}
fn dvd_time(data: &[u8]) -> Result<f64> {
    fn bcd(value: u8) -> Result<u32> {
        if value & 15 > 9 || value >> 4 > 9 {
            return Err(ImageError::Malformed("DVD BCD time"));
        }
        Ok(u32::from(value >> 4) * 10 + u32::from(value & 15))
    }
    let hours = bcd(data[0])?;
    let minutes = bcd(data[1])?;
    let seconds = bcd(data[2])?;
    let frames = bcd(data[3] & 63)?;
    let rate: f64 = match data[3] >> 6 {
        1 => 25.0,
        3 => 30000.0 / 1001.0,
        0 if frames == 0 => 1.0,
        _ => return Err(ImageError::Malformed("DVD time frame rate")),
    };
    if minutes >= 60 || seconds >= 60 || f64::from(frames) >= rate.ceil() {
        return Err(ImageError::Malformed("DVD time range"));
    }
    Ok(f64::from(hours * 3600 + minutes * 60 + seconds) + f64::from(frames) / rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dvd_time_preserves_fractional_frames_and_rejects_invalid_bcd() {
        assert!(
            (dvd_time(&[0x01, 0x23, 0x45, 0xc1]).unwrap() - (5025.0 + 1001.0 / 30000.0)).abs()
                < 1e-8
        );
        assert!(dvd_time(&[0, 0x6a, 0, 0]).is_err());
    }
    #[test]
    fn playlist_rejects_unbounded_and_truncated_tables() {
        let mut bytes = b"MPLS0200".to_vec();
        bytes.extend_from_slice(&u32::MAX.to_be_bytes());
        bytes.resize(40, 0);
        assert!(playlist(&bytes, "1".into()).is_err());
    }
}
