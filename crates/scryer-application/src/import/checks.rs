//! Pre-import validation pipeline.
//!
//! Each check is a pure function that returns [`ImportVerdict`].
//! `run_import_checks` executes them in order and short-circuits on the first
//! `Reject`.

use std::path::Path;

use crate::release_parser::ParsedReleaseMetadata;
use crate::types::TitleMediaFile;

/// Outcome of a single import check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportVerdict {
    Accept,
    Reject {
        reason: String,
        code: ImportCheckCode,
    },
}

impl ImportVerdict {
    pub fn is_accept(&self) -> bool {
        matches!(self, Self::Accept)
    }
}

/// Stable reason for an import-check rejection.
///
/// The string representation is persisted with import artifacts, but import
/// behavior must branch on this enum so additions remain exhaustive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportCheckCode {
    InvalidExtension,
    SampleFile,
    SampleDirectory,
    StillUnpacking,
    DuplicateFile,
    InsufficientDiskSpace,
}

impl ImportCheckCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidExtension => "invalid_extension",
            Self::SampleFile => "sample_file",
            Self::SampleDirectory => "sample_directory",
            Self::StillUnpacking => "still_unpacking",
            Self::DuplicateFile => "duplicate_file",
            Self::InsufficientDiskSpace => "insufficient_disk_space",
        }
    }

    pub const fn is_duplicate_file(self) -> bool {
        matches!(self, Self::DuplicateFile)
    }
}

impl std::fmt::Display for ImportCheckCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// All inputs needed by the check pipeline.
pub struct ImportCheckContext<'a> {
    pub source_path: &'a Path,
    pub dest_path: &'a Path,
    pub source_size: u64,
    #[expect(dead_code)]
    pub parsed: &'a ParsedReleaseMetadata,
    #[expect(dead_code)]
    pub existing_files: &'a [TitleMediaFile],
}

const DISK_SPACE_RESERVE_BYTES: u64 = 500 * 1024 * 1024;

fn disk_space_verdict(available: u64, source_size: u64) -> ImportVerdict {
    let required = u128::from(source_size) + u128::from(DISK_SPACE_RESERVE_BYTES);
    if u128::from(available) < required {
        ImportVerdict::Reject {
            reason: format!(
                "insufficient disk space: {:.1} GB available, need {:.1} GB",
                available as f64 / 1_073_741_824.0,
                required as f64 / 1_073_741_824.0,
            ),
            code: ImportCheckCode::InsufficientDiskSpace,
        }
    } else {
        ImportVerdict::Accept
    }
}

fn disk_space_verdict_for_measurement(available: Option<u64>, source_size: u64) -> ImportVerdict {
    available
        .map(|available| disk_space_verdict(available, source_size))
        .unwrap_or(ImportVerdict::Accept)
}

fn nearest_existing_ancestor(path: &Path) -> &Path {
    path.ancestors()
        .find(|candidate| candidate.exists())
        .unwrap_or(path)
}

fn destination_directory(dest_path: &Path) -> &Path {
    match dest_path.parent() {
        Some(parent) if parent.as_os_str().is_empty() => Path::new("."),
        Some(parent) => parent,
        None => dest_path,
    }
}

fn available_disk_space(path: &Path) -> std::io::Result<u64> {
    Ok(crate::filesystem_space_raw(path)?.available_bytes)
}

// ── Individual checks ────────────────────────────────────────────────────────

/// Reject files with extensions outside the known video set.
pub fn check_valid_extension(ctx: &ImportCheckContext<'_>) -> ImportVerdict {
    if scryer_domain::is_video_file(ctx.source_path) {
        ImportVerdict::Accept
    } else {
        let ext = ctx
            .source_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("<none>")
            .to_string();
        ImportVerdict::Reject {
            reason: format!("unsupported extension: {ext}"),
            code: ImportCheckCode::InvalidExtension,
        }
    }
}

/// Reject files that look like samples (name contains "sample" or parent dir
/// is "sample"/"samples").
pub fn check_not_sample(ctx: &ImportCheckContext<'_>) -> ImportVerdict {
    let filename = ctx
        .source_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if filename.contains("sample") {
        return ImportVerdict::Reject {
            reason: "filename contains 'sample'".into(),
            code: ImportCheckCode::SampleFile,
        };
    }

    // Parent directory named "sample" or "samples"
    if let Some(parent) = ctx.source_path.parent() {
        let dir_name = parent
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if dir_name == "sample" || dir_name == "samples" {
            return ImportVerdict::Reject {
                reason: "file is inside a sample directory".into(),
                code: ImportCheckCode::SampleDirectory,
            };
        }
    }

    ImportVerdict::Accept
}

/// Reject files that are still being unpacked by a download client.
pub fn check_not_unpacking(ctx: &ImportCheckContext<'_>) -> ImportVerdict {
    let path_str = ctx.source_path.to_string_lossy();

    // Active-download markers
    for marker in &[".!qB", ".part", "._unpack"] {
        if path_str.ends_with(marker) {
            return ImportVerdict::Reject {
                reason: format!("file has active-download marker: {marker}"),
                code: ImportCheckCode::StillUnpacking,
            };
        }
    }

    // Check for sibling marker files (e.g. foo.mkv.!qB alongside foo.mkv)
    if let Some(file_name) = ctx.source_path.file_name()
        && let Some(parent) = ctx.source_path.parent()
    {
        for marker in &[".!qB", ".part", "._unpack"] {
            let mut marker_name = file_name.to_os_string();
            marker_name.push(marker);
            let marker_path = parent.join(marker_name);
            if marker_path.exists() {
                return ImportVerdict::Reject {
                    reason: format!("sibling marker file exists: {}", marker_path.display()),
                    code: ImportCheckCode::StillUnpacking,
                };
            }
        }
    }

    ImportVerdict::Accept
}

/// Reject if destination already exists with the same size (exact duplicate).
pub fn check_not_already_imported(ctx: &ImportCheckContext<'_>) -> ImportVerdict {
    if !ctx.dest_path.exists() {
        return ImportVerdict::Accept;
    }

    let existing_size = std::fs::metadata(ctx.dest_path)
        .map(|m| m.len())
        .unwrap_or(0);

    if existing_size == ctx.source_size {
        ImportVerdict::Reject {
            reason: format!(
                "destination exists with identical size ({} bytes)",
                ctx.source_size
            ),
            code: ImportCheckCode::DuplicateFile,
        }
    } else {
        // Different size → allow (will be handled as upgrade or overwrite)
        ImportVerdict::Accept
    }
}

/// Reject if available disk space is insufficient.
///
/// Requires at least `source_size + 500 MB` free on the destination volume.
pub fn check_disk_space(ctx: &ImportCheckContext<'_>) -> ImportVerdict {
    let target_dir = destination_directory(ctx.dest_path);
    let stat_path = nearest_existing_ancestor(target_dir);
    let available = match available_disk_space(stat_path) {
        Ok(available) => Some(available),
        Err(error) => {
            tracing::debug!(
                path = %stat_path.display(),
                %error,
                "unable to query available import destination space"
            );
            None
        }
    };
    disk_space_verdict_for_measurement(available, ctx.source_size)
}

// ── Pipeline ─────────────────────────────────────────────────────────────────

/// Run all pre-import checks in order. Short-circuits on the first `Reject`.
pub fn run_import_checks(ctx: &ImportCheckContext<'_>) -> ImportVerdict {
    let checks: &[fn(&ImportCheckContext<'_>) -> ImportVerdict] = &[
        // Active-download suffixes change the apparent extension, so this
        // transient check must run before extension validation.
        check_not_unpacking,
        check_valid_extension,
        check_not_sample,
        check_disk_space,
        check_not_already_imported,
    ];

    for check in checks {
        let verdict = check(ctx);
        if !verdict.is_accept() {
            return verdict;
        }
    }

    ImportVerdict::Accept
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release_parser::parse_release_metadata;
    use std::path::PathBuf;

    fn dummy_ctx<'a>(
        source: &'a Path,
        dest: &'a Path,
        source_size: u64,
        parsed: &'a ParsedReleaseMetadata,
        existing_files: &'a [TitleMediaFile],
    ) -> ImportCheckContext<'a> {
        ImportCheckContext {
            source_path: source,
            dest_path: dest,
            source_size,
            parsed,
            existing_files,
        }
    }

    #[test]
    fn valid_extension_accepts_mkv() {
        let parsed = parse_release_metadata("Movie.2024.1080p.BluRay.x264");
        let src = PathBuf::from("/tmp/Movie.2024.1080p.BluRay.x264.mkv");
        let dst = PathBuf::from("/data/Movie (2024)/Movie.2024.1080p.BluRay.x264.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);
        assert!(check_valid_extension(&ctx).is_accept());
    }

    #[test]
    fn valid_extension_accepts_strm() {
        let parsed = parse_release_metadata("Movie.2024.1080p.BluRay.x264");
        let src = PathBuf::from("/tmp/Movie.2024.1080p.BluRay.x264.strm");
        let dst = PathBuf::from("/data/Movie (2024)/Movie.2024.1080p.BluRay.x264.strm");
        let ctx = dummy_ctx(&src, &dst, 1_024, &parsed, &[]);
        assert!(check_valid_extension(&ctx).is_accept());
    }

    #[test]
    fn valid_extension_rejects_txt() {
        let parsed = parse_release_metadata("readme");
        let src = PathBuf::from("/tmp/readme.txt");
        let dst = PathBuf::from("/data/readme.txt");
        let ctx = dummy_ctx(&src, &dst, 100, &parsed, &[]);
        assert!(!check_valid_extension(&ctx).is_accept());
    }

    #[test]
    fn sample_detected_in_filename() {
        let parsed = parse_release_metadata("sample-movie");
        let src = PathBuf::from("/tmp/sample-movie.mkv");
        let dst = PathBuf::from("/data/movie.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);
        assert!(!check_not_sample(&ctx).is_accept());
    }

    #[cfg(unix)]
    #[test]
    fn sample_detected_in_non_utf8_filename() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let parsed = parse_release_metadata("sample-movie");
        let src = PathBuf::from(OsStr::from_bytes(b"/tmp/\xFFsample-movie.mkv"));
        let dst = PathBuf::from("/data/movie.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);
        assert!(!check_not_sample(&ctx).is_accept());
    }

    #[test]
    fn sample_detected_in_parent_dir() {
        let parsed = parse_release_metadata("movie");
        let src = PathBuf::from("/tmp/Sample/movie.mkv");
        let dst = PathBuf::from("/data/movie.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);
        assert!(!check_not_sample(&ctx).is_accept());
    }

    #[test]
    fn unpacking_marker_rejects() {
        let parsed = parse_release_metadata("movie");
        let src = PathBuf::from("/tmp/movie.mkv.!qB");
        let dst = PathBuf::from("/data/movie.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);
        assert!(matches!(
            run_import_checks(&ctx),
            ImportVerdict::Reject {
                code: ImportCheckCode::StillUnpacking,
                ..
            }
        ));
    }

    #[test]
    fn sibling_unpacking_marker_rejects_with_the_same_typed_code() {
        let temp = tempfile::tempdir().expect("create temp directory");
        let parsed = parse_release_metadata("movie");
        let src = temp.path().join("movie.mkv");
        let marker = temp.path().join("movie.mkv.!qB");
        std::fs::write(&marker, "active").expect("write sibling marker");
        let dst = temp.path().join("destination.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);

        assert!(matches!(
            run_import_checks(&ctx),
            ImportVerdict::Reject {
                code: ImportCheckCode::StillUnpacking,
                ..
            }
        ));
    }

    #[test]
    fn clean_file_passes_unpacking() {
        let parsed = parse_release_metadata("movie");
        let src = PathBuf::from("/tmp/movie.mkv");
        let dst = PathBuf::from("/data/movie.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);
        assert!(check_not_unpacking(&ctx).is_accept());
    }

    #[test]
    fn pipeline_accepts_clean_file() {
        let parsed = parse_release_metadata("Movie.2024.1080p.BluRay.x264");
        let src = PathBuf::from("/tmp/Movie.2024.1080p.BluRay.x264.mkv");
        let dst = PathBuf::from("/nonexistent/Movie (2024)/Movie.2024.1080p.BluRay.x264.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);
        assert!(run_import_checks(&ctx).is_accept());
    }

    #[test]
    fn disk_space_accepts_exact_requirement() {
        let source_size = 1_000_000;
        let available = source_size + DISK_SPACE_RESERVE_BYTES;
        assert!(disk_space_verdict(available, source_size).is_accept());
    }

    #[test]
    fn disk_space_zero_length_source_still_requires_reserve() {
        assert!(disk_space_verdict(DISK_SPACE_RESERVE_BYTES, 0).is_accept());
        assert!(matches!(
            disk_space_verdict(DISK_SPACE_RESERVE_BYTES - 1, 0),
            ImportVerdict::Reject {
                code: ImportCheckCode::InsufficientDiskSpace,
                ..
            }
        ));
    }

    #[test]
    fn disk_space_query_failure_is_non_blocking() {
        assert!(disk_space_verdict_for_measurement(None, u64::MAX).is_accept());
    }

    #[test]
    fn disk_space_rejects_one_byte_below_requirement() {
        let source_size = 1_000_000;
        let available = source_size + DISK_SPACE_RESERVE_BYTES - 1;
        assert!(matches!(
            disk_space_verdict(available, source_size),
            ImportVerdict::Reject {
                code: ImportCheckCode::InsufficientDiskSpace,
                ..
            }
        ));
    }

    #[test]
    fn disk_space_requirement_does_not_overflow_for_large_sources() {
        assert!(matches!(
            disk_space_verdict(u64::MAX - 1, u64::MAX),
            ImportVerdict::Reject {
                code: ImportCheckCode::InsufficientDiskSpace,
                ..
            }
        ));
        assert!(matches!(
            disk_space_verdict(u64::MAX, u64::MAX),
            ImportVerdict::Reject {
                code: ImportCheckCode::InsufficientDiskSpace,
                ..
            }
        ));
    }

    #[test]
    fn disk_space_uses_nearest_existing_ancestor() {
        let temp = tempfile::tempdir().expect("create temp directory");
        let missing = temp.path().join("missing").join("destination");
        assert_eq!(nearest_existing_ancestor(&missing), temp.path());
    }

    #[test]
    fn disk_space_uses_current_directory_for_bare_destination_filename() {
        assert_eq!(
            destination_directory(Path::new("movie.mkv")),
            Path::new(".")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_disk_space_query_smoke_test() {
        let temp = tempfile::tempdir().expect("create temp directory");
        assert!(available_disk_space(temp.path()).expect("query available space") > 0);
    }
}
