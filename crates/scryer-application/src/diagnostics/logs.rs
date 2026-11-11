//! Reads the configured log file and its newest rotated archives, read only.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Raw bytes read from the log files at most, newest content first. The
/// archive cap trims further; this bound keeps sanitizing time reasonable.
const LOG_FILE_READ_CAP_BYTES: u64 = 24 * 1024 * 1024;
/// Rotated archives read at most, newest first.
const ROTATED_ARCHIVES_READ: usize = 2;
/// Marker the log writer puts between the active file name and a rotation's
/// timestamp.
const ROTATION_MARKER: &str = ".scryer-rotate-";

/// What reading the log files produced.
#[derive(Debug, Default)]
pub(crate) struct LogFileRead {
    /// Lines, oldest first, across the archives and the active file.
    pub lines: Vec<String>,
    /// Sources read, as code-defined labels (`active`, `rotated-1`, ...).
    pub sources: Vec<String>,
    /// Sources that could not be read, as `(label, reason)`.
    pub failures: Vec<(String, String)>,
    /// Whether older content was skipped by the read cap.
    pub read_capped: bool,
    /// Whether file logging was configured at all.
    pub configured: bool,
}

impl LogFileRead {
    pub(crate) fn absent() -> Self {
        Self::default()
    }

    pub(crate) fn failed(reason: &str) -> Self {
        Self {
            configured: true,
            failures: vec![("log_file".to_string(), reason.to_string())],
            ..Self::default()
        }
    }
}

/// Reads the active file and up to [`ROTATED_ARCHIVES_READ`] rotated gzip
/// archives beside it, newest content first, until the read cap.
pub(crate) fn read_log_files(active: &Path) -> LogFileRead {
    let mut read = LogFileRead {
        configured: true,
        ..LogFileRead::default()
    };
    let mut budget = LOG_FILE_READ_CAP_BYTES;
    // Newest first: the active file, then archives newest to oldest.
    let mut chunks: Vec<Vec<String>> = Vec::new();
    match read_tail(active, budget) {
        Ok((lines, used, capped)) => {
            budget = budget.saturating_sub(used);
            read.read_capped |= capped;
            read.sources.push("active".to_string());
            chunks.push(lines);
        }
        Err(reason) => read.failures.push(("active".to_string(), reason)),
    }
    for (index, archive) in rotated_archives(active).into_iter().enumerate() {
        let label = format!("rotated-{}", index + 1);
        if budget == 0 {
            read.read_capped = true;
            break;
        }
        match read_gzip(&archive, budget) {
            Ok((lines, used, capped)) => {
                budget = budget.saturating_sub(used);
                read.read_capped |= capped;
                read.sources.push(label);
                chunks.push(lines);
            }
            Err(reason) => read.failures.push((label, reason)),
        }
    }
    for chunk in chunks.into_iter().rev() {
        read.lines.extend(chunk);
    }
    read
}

fn io_reason(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => "not_found".to_string(),
        std::io::ErrorKind::PermissionDenied => "permission_denied".to_string(),
        std::io::ErrorKind::InvalidData => "invalid_data".to_string(),
        _ => "io_error".to_string(),
    }
}

/// The last `budget` bytes of a plain file, cut to whole lines.
fn read_tail(path: &Path, budget: u64) -> Result<(Vec<String>, u64, bool), String> {
    let mut file = File::open(path).map_err(|error| io_reason(&error))?;
    let length = file.metadata().map_err(|error| io_reason(&error))?.len();
    let capped = length > budget;
    if capped {
        file.seek(SeekFrom::Start(length - budget))
            .map_err(|error| io_reason(&error))?;
    }
    let mut bytes = Vec::new();
    file.take(budget)
        .read_to_end(&mut bytes)
        .map_err(|error| io_reason(&error))?;
    let used = bytes.len() as u64;
    let mut lines = split_lines(&bytes);
    if capped && !lines.is_empty() {
        // The first line starts mid-way through a record.
        lines.remove(0);
    }
    Ok((lines, used, capped))
}

/// A gzip archive's lines, keeping the newest `budget` bytes.
fn read_gzip(path: &Path, budget: u64) -> Result<(Vec<String>, u64, bool), String> {
    let file = File::open(path).map_err(|error| io_reason(&error))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut lines = std::collections::VecDeque::new();
    let mut used = 0_u64;
    let mut capped = false;
    let mut reader = BufReader::new(decoder);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = reader
            .read_until(b'\n', &mut buffer)
            .map_err(|error| io_reason(&error))?;
        if read == 0 {
            break;
        }
        used += read as u64;
        lines.push_back(String::from_utf8_lossy(trim_newline(&buffer)).into_owned());
        while used > budget {
            let Some(dropped) = lines.pop_front() else {
                break;
            };
            used = used.saturating_sub(dropped.len() as u64 + 1);
            capped = true;
        }
    }
    Ok((lines.into_iter().collect(), used, capped))
}

fn trim_newline(bytes: &[u8]) -> &[u8] {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    bytes.strip_suffix(b"\r").unwrap_or(bytes)
}

fn split_lines(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == b'\n')
        .map(|line| String::from_utf8_lossy(trim_newline(line)).into_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Rotated gzip archives of `active`, newest first. Rotation names sort by
/// their timestamp and sequence, so a reverse name sort is newest first.
fn rotated_archives(active: &Path) -> Vec<std::path::PathBuf> {
    let (Some(directory), Some(file_name)) = (active.parent(), active.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}{ROTATION_MARKER}", file_name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut archives = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (name.starts_with(&prefix) && name.ends_with(".gz")).then(|| entry.path())
        })
        .collect::<Vec<_>>();
    archives.sort();
    archives.reverse();
    archives.truncate(ROTATED_ARCHIVES_READ);
    archives
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_active_file_and_newest_archives_oldest_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let active = dir.path().join("scryer.log");
        std::fs::write(&active, "active-1\nactive-2\n").expect("write active");
        for (stamp, line) in [
            ("20260101T000000Z-00000000000000000001", "oldest"),
            ("20260102T000000Z-00000000000000000002", "middle"),
            ("20260103T000000Z-00000000000000000003", "newest"),
        ] {
            let path = dir
                .path()
                .join(format!("scryer.log{ROTATION_MARKER}{stamp}.sealed.gz"));
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            writeln!(encoder, "{line}").expect("encode");
            std::fs::write(path, encoder.finish().expect("finish")).expect("write archive");
        }

        let read = read_log_files(&active);

        assert_eq!(read.lines, vec!["middle", "newest", "active-1", "active-2"]);
        assert_eq!(read.sources, vec!["active", "rotated-1", "rotated-2"]);
        assert!(read.failures.is_empty());
    }

    #[test]
    fn missing_active_file_is_reported_not_fatal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let read = read_log_files(&dir.path().join("absent.log"));
        assert!(read.lines.is_empty());
        assert_eq!(
            read.failures,
            vec![("active".to_string(), "not_found".to_string())]
        );
    }

    #[test]
    fn tail_read_drops_the_partial_first_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let active = dir.path().join("scryer.log");
        std::fs::write(&active, "first-line\nsecond\nthird\n").expect("write");
        let (lines, _, capped) = read_tail(&active, 12).expect("tail");
        assert!(capped);
        assert_eq!(lines, vec!["third"]);
    }
}
