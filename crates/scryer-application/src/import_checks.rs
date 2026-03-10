//! Pre-import validation pipeline.
//!
//! Each check is a pure function that returns [`ImportVerdict`].
//! `run_import_checks` executes them in order and short-circuits on the first
//! `Reject`.

#[cfg(unix)]
use nix::libc;
use std::path::Path;

use crate::release_parser::ParsedReleaseMetadata;
use crate::types::TitleMediaFile;

/// Outcome of a single import check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportVerdict {
    Accept,
    Reject { reason: String, code: &'static str },
}

impl ImportVerdict {
    pub fn is_accept(&self) -> bool {
        matches!(self, Self::Accept)
    }
}

/// All inputs needed by the check pipeline.
pub struct ImportCheckContext<'a> {
    pub source_path: &'a Path,
    pub dest_path: &'a Path,
    pub source_size: u64,
    #[allow(dead_code)]
    pub parsed: &'a ParsedReleaseMetadata,
    #[allow(dead_code)]
    pub existing_files: &'a [TitleMediaFile],
}

fn to_u64<T: Into<u64>>(value: T) -> u64 {
    value.into()
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
            code: "invalid_extension",
        }
    }
}

/// Reject files that look like samples (name contains "sample" or parent dir
/// is "sample"/"samples").
pub fn check_not_sample(ctx: &ImportCheckContext<'_>) -> ImportVerdict {
    let filename = ctx
        .source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if filename.contains("sample") {
        return ImportVerdict::Reject {
            reason: "filename contains 'sample'".into(),
            code: "sample_file",
        };
    }

    // Parent directory named "sample" or "samples"
    if let Some(parent) = ctx.source_path.parent() {
        let dir_name = parent
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if dir_name == "sample" || dir_name == "samples" {
            return ImportVerdict::Reject {
                reason: "file is inside a sample directory".into(),
                code: "sample_directory",
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
                code: "still_unpacking",
            };
        }
    }

    // Check for sibling marker files (e.g. foo.mkv.!qB alongside foo.mkv)
    if let Some(file_name) = ctx.source_path.file_name().and_then(|n| n.to_str()) {
        if let Some(parent) = ctx.source_path.parent() {
            for marker in &[".!qB", ".part", "._unpack"] {
                let marker_path = parent.join(format!("{file_name}{marker}"));
                if marker_path.exists() {
                    return ImportVerdict::Reject {
                        reason: format!("sibling marker file exists: {}", marker_path.display()),
                        code: "still_unpacking",
                    };
                }
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
            code: "duplicate_file",
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
    let target_dir = ctx.dest_path.parent().unwrap_or(ctx.dest_path);

    // Find an existing ancestor to stat (dest dir may not exist yet)
    let stat_path = {
        let mut p = target_dir.to_path_buf();
        while !p.exists() {
            if !p.pop() {
                break;
            }
        }
        p
    };

    #[cfg(unix)]
    {
        use std::ffi::CString;
        if let Ok(c_path) = CString::new(stat_path.to_string_lossy().as_bytes()) {
            let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
            let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
            if ret == 0 {
                let available = to_u64(stat.f_bavail) * to_u64(stat.f_frsize);
                let required = ctx.source_size + 500 * 1024 * 1024;
                if available < required {
                    return ImportVerdict::Reject {
                        reason: format!(
                            "insufficient disk space: {:.1} GB available, need {:.1} GB",
                            available as f64 / 1_073_741_824.0,
                            required as f64 / 1_073_741_824.0,
                        ),
                        code: "insufficient_disk_space",
                    };
                }
            }
        }
    }

    ImportVerdict::Accept
}

// ── Pipeline ─────────────────────────────────────────────────────────────────

/// Run all pre-import checks in order. Short-circuits on the first `Reject`.
pub fn run_import_checks(ctx: &ImportCheckContext<'_>) -> ImportVerdict {
    let checks: &[fn(&ImportCheckContext<'_>) -> ImportVerdict] = &[
        check_valid_extension,
        check_not_sample,
        check_not_unpacking,
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
        let dst = PathBuf::from("/media/Movie (2024)/Movie.2024.1080p.BluRay.x264.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);
        assert!(check_valid_extension(&ctx).is_accept());
    }

    #[test]
    fn valid_extension_rejects_txt() {
        let parsed = parse_release_metadata("readme");
        let src = PathBuf::from("/tmp/readme.txt");
        let dst = PathBuf::from("/media/readme.txt");
        let ctx = dummy_ctx(&src, &dst, 100, &parsed, &[]);
        assert!(!check_valid_extension(&ctx).is_accept());
    }

    #[test]
    fn sample_detected_in_filename() {
        let parsed = parse_release_metadata("sample-movie");
        let src = PathBuf::from("/tmp/sample-movie.mkv");
        let dst = PathBuf::from("/media/movie.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);
        assert!(!check_not_sample(&ctx).is_accept());
    }

    #[test]
    fn sample_detected_in_parent_dir() {
        let parsed = parse_release_metadata("movie");
        let src = PathBuf::from("/tmp/Sample/movie.mkv");
        let dst = PathBuf::from("/media/movie.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);
        assert!(!check_not_sample(&ctx).is_accept());
    }

    #[test]
    fn unpacking_marker_rejects() {
        let parsed = parse_release_metadata("movie");
        let src = PathBuf::from("/tmp/movie.mkv.!qB");
        let dst = PathBuf::from("/media/movie.mkv");
        let ctx = dummy_ctx(&src, &dst, 1_000_000, &parsed, &[]);
        assert!(!check_not_unpacking(&ctx).is_accept());
    }

    #[test]
    fn clean_file_passes_unpacking() {
        let parsed = parse_release_metadata("movie");
        let src = PathBuf::from("/tmp/movie.mkv");
        let dst = PathBuf::from("/media/movie.mkv");
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
}
