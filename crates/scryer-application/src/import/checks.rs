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
    pub import_mode: scryer_domain::ImportMode,
    #[expect(dead_code)]
    pub parsed: &'a ParsedReleaseMetadata,
    pub existing_files: &'a [TitleMediaFile],
    /// Set only when the manual import executor's own content probe qualified
    /// the source as video, so it may lack a known video extension. Every
    /// other check still runs.
    pub content_qualified_video: bool,
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

/// Reject files with extensions outside the known video set, unless the manual
/// import executor content-qualified the source as video.
pub fn check_valid_extension(ctx: &ImportCheckContext<'_>) -> ImportVerdict {
    if ctx.content_qualified_video || scryer_domain::is_video_file(ctx.source_path) {
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

/// Skip a same-sized destination, allowing move retries to finish persistence.
pub fn check_not_already_imported(ctx: &ImportCheckContext<'_>) -> ImportVerdict {
    // A failed catalog write can leave verified bytes at the destination. Let
    // the coordinated importer prove their content and claim catalog ownership
    // before it can remove the source on retry.
    if !ctx.dest_path.exists()
        || (ctx.import_mode == scryer_domain::ImportMode::Move
            && !ctx
                .existing_files
                .iter()
                .any(|file| Path::new(&file.file_path) == ctx.dest_path))
    {
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
fn measured_disk_space(ctx: &ImportCheckContext<'_>) -> (ImportVerdict, Option<(u64, String)>) {
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
    let identity = available.map(|available| {
        #[cfg(unix)]
        let volume = {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(stat_path)
                .ok()
                .map(|metadata| format!("device:{}", metadata.dev()))
        };
        #[cfg(not(unix))]
        let volume: Option<String> = None;
        (available, volume.unwrap_or_default())
    });
    (
        disk_space_verdict_for_measurement(available, ctx.source_size),
        identity,
    )
}

// ── Pipeline ─────────────────────────────────────────────────────────────────

/// Run all pre-import checks in order. Short-circuits on the first `Reject`.
#[cfg(test)]
pub fn run_import_checks(ctx: &ImportCheckContext<'_>) -> ImportVerdict {
    run_observed_checks(ctx).0
}

fn run_observed_checks(ctx: &ImportCheckContext<'_>) -> (ImportVerdict, Option<(u64, String)>) {
    run_checks_with_measurement(ctx, measured_disk_space)
}

fn run_checks_with_measurement(
    ctx: &ImportCheckContext<'_>,
    measure: impl FnOnce(&ImportCheckContext<'_>) -> (ImportVerdict, Option<(u64, String)>),
) -> (ImportVerdict, Option<(u64, String)>) {
    let checks: &[fn(&ImportCheckContext<'_>) -> ImportVerdict] = &[
        // Active-download suffixes change the apparent extension, so this
        // transient check must run before extension validation.
        check_not_unpacking,
        check_valid_extension,
        check_not_sample,
    ];

    for check in checks {
        let verdict = check(ctx);
        if !verdict.is_accept() {
            return (verdict, None);
        }
    }

    let (verdict, measurement) = measure(ctx);
    if !verdict.is_accept() {
        return (verdict, measurement);
    }
    (check_not_already_imported(ctx), measurement)
}

/// Observation and retirement read `client_type` from different records, so
/// fold its casing and padding here; otherwise a retire keyed `"NZBGet"` would
/// miss an incident observed as `"nzbget"`.
pub(crate) fn space_job_key(client_type: &str, client_id: &str, item_id: &str) -> String {
    let client_type = client_type.trim().to_ascii_lowercase();
    serde_json::to_string(&(client_type, client_id, item_id)).expect("string tuple serializes")
}

fn space_path_key(path: &std::path::Path) -> String {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        serde_json::to_string(&path.as_os_str().encode_wide().collect::<Vec<_>>())
            .expect("path units serialize")
    }
    #[cfg(not(windows))]
    {
        serde_json::to_string(path.as_os_str().as_encoded_bytes()).expect("path bytes serialize")
    }
}

fn normalized_space_root(root: &std::path::Path) -> String {
    use std::path::Component;
    let absolute = std::fs::canonicalize(root)
        .or_else(|_| std::path::absolute(root))
        .unwrap_or_else(|_| root.to_path_buf());
    let mut normalized = std::path::PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    let root = normalized.to_string_lossy().into_owned();
    #[cfg(windows)]
    let root = root.replace('/', "\\").to_lowercase();
    format!("root:{root}")
}

pub(crate) async fn retire_space_blocks(
    app: &crate::AppUseCase,
    source: &crate::ClientJobLocator,
    download_id: Option<&scryer_domain::download_identity::DownloadId>,
) {
    let Some(client_id) = source.client_id.as_deref() else {
        return;
    };
    let update = scryer_domain::import_space::SpaceIncidentUpdate::Retired {
        job_key: space_job_key(&source.client_type, client_id, &source.item_id),
        download_id: download_id.map(ToString::to_string),
    };
    if let Err(error) = app
        .services
        .events
        .domain_events
        .update_import_space_incident(update)
        .await
    {
        tracing::warn!(%error, "unable to retire import disk-space incident membership");
    }
}

/// Observability is best effort and never changes an admission decision.
pub(crate) async fn run_import_checks_and_report(
    app: &crate::AppUseCase,
    ctx: &ImportCheckContext<'_>,
    completed: Option<&scryer_domain::CompletedDownload>,
    title: &scryer_domain::Title,
) -> ImportVerdict {
    use scryer_domain::import_space::{SpaceIncidentUpdate, SpaceMeasurement};
    let (verdict, measured) = run_observed_checks(ctx);
    if let Some((available_bytes, volume)) = measured {
        let destination = destination_directory(ctx.dest_path)
            .to_string_lossy()
            .into_owned();
        let destination_key = if volume.is_empty() {
            let root = match app.title_root_folder_path_override(title).await {
                Ok(root) => root,
                Err(error) => {
                    tracing::warn!(%error, "unable to identify import space incident root");
                    return verdict;
                }
            };
            normalized_space_root(std::path::Path::new(&root))
        } else {
            volume
        };
        let job_key = completed
            .map(|job| {
                space_job_key(
                    &job.client_type,
                    &job.client_id,
                    &job.download_client_item_id,
                )
            })
            .unwrap_or_else(|| format!("manual:{}", space_path_key(ctx.source_path)));
        let member_key = serde_json::to_string(&(&job_key, space_path_key(ctx.source_path)))
            .expect("string tuple serializes");
        let update = SpaceIncidentUpdate::Observed {
            member_key,
            job_key,
            download_id: completed.and_then(|job| job.download_id.clone()),
            measurement: SpaceMeasurement {
                destination_key,
                destination,
                available_bytes,
                required_bytes: u128::from(ctx.source_size) + u128::from(DISK_SPACE_RESERVE_BYTES),
            },
            blocked: matches!(
                verdict,
                ImportVerdict::Reject {
                    code: ImportCheckCode::InsufficientDiskSpace,
                    ..
                }
            ),
        };
        match app
            .services
            .events
            .domain_events
            .update_import_space_incident(update)
            .await
        {
            Ok(events) => {
                for event in events {
                    app.publish_stored_domain_event(&event).await;
                }
            }
            Err(error) => tracing::warn!(%error, "unable to persist import disk-space observation"),
        }
    }
    verdict
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release_parser::parse_release_metadata;
    use std::path::PathBuf;

    #[test]
    fn space_job_key_folds_client_type_case_and_padding() {
        let observed = space_job_key("nzbget", "client-1", "item-7");
        assert_eq!(space_job_key(" NZBGet ", "client-1", "item-7"), observed);
        assert_ne!(space_job_key("nzbget", "client-1", "ITEM-7"), observed);
        assert_ne!(space_job_key("nzbget", "Client-1", "item-7"), observed);
    }

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
            import_mode: scryer_domain::ImportMode::HardlinkOrCopy,
            parsed,
            existing_files,
            content_qualified_video: false,
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
    fn valid_extension_accepts_extensionless_source_only_when_content_qualified() {
        let parsed = parse_release_metadata("dXRUKoYEAJ58jradJdxMKKxgczVTvt");
        let src = PathBuf::from("/tmp/dXRUKoYEAJ58jradJdxMKKxgczVTvt");
        let dst = PathBuf::from("/data/Show/Season 01/Show - S01E01.mkv");
        let mut ctx = dummy_ctx(&src, &dst, 32 * 1024 * 1024, &parsed, &[]);
        assert_eq!(
            run_import_checks(&ctx),
            ImportVerdict::Reject {
                reason: "unsupported extension: <none>".into(),
                code: ImportCheckCode::InvalidExtension,
            }
        );

        ctx.content_qualified_video = true;
        assert!(check_valid_extension(&ctx).is_accept());
        let sample_src = PathBuf::from("/tmp/sample/dXRUKoYEAJ58jradJdxMKKxgczVTvt");
        ctx.source_path = &sample_src;
        assert!(matches!(
            run_import_checks(&ctx),
            ImportVerdict::Reject {
                code: ImportCheckCode::SampleDirectory,
                ..
            }
        ));
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
    fn uncataloged_move_destination_reaches_content_verification() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("destination.mkv");
        std::fs::write(&source, b"video").unwrap();
        std::fs::write(&dest, b"video").unwrap();
        let parsed = parse_release_metadata("movie");
        let mut ctx = dummy_ctx(&source, &dest, 5, &parsed, &[]);
        assert!(matches!(
            check_not_already_imported(&ctx),
            ImportVerdict::Reject {
                code: ImportCheckCode::DuplicateFile,
                ..
            }
        ));
        ctx.import_mode = scryer_domain::ImportMode::Move;
        assert!(check_not_already_imported(&ctx).is_accept());
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
    #[cfg(unix)]
    fn import_space_members_preserve_non_utf8_path_identity() {
        use std::os::unix::ffi::OsStrExt;
        let first = std::path::Path::new(std::ffi::OsStr::from_bytes(b"episode-\xff.mkv"));
        let second = std::path::Path::new(std::ffi::OsStr::from_bytes(b"episode-\xfe.mkv"));
        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        assert_ne!(space_path_key(first), space_path_key(second));
    }

    #[test]
    fn import_space_root_groups_equivalent_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("missing-library");
        assert_eq!(
            normalized_space_root(&root),
            normalized_space_root(&root.join("nested").join("..").join("."))
        );
        assert_ne!(
            normalized_space_root(&root),
            normalized_space_root(&dir.path().join("other"))
        );
    }

    #[test]
    fn import_space_unknown_measurement_cannot_report_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let parsed = parse_release_metadata("Show.S01E01");
        let src = dir.path().join("episode.mkv");
        let dest = dir.path().join("destination.mkv");
        let ctx = dummy_ctx(&src, &dest, 5, &parsed, &[]);
        let (verdict, measurement) =
            run_checks_with_measurement(&ctx, |_| (ImportVerdict::Accept, None));
        assert!(verdict.is_accept());
        assert!(measurement.is_none());
    }

    #[test]
    fn import_space_pass_is_observed_even_when_a_later_check_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let parsed = parse_release_metadata("Show.S01E01");
        let src = dir.path().join("episode.mkv");
        let dest = dir.path().join("destination.mkv");
        std::fs::write(&dest, b"video").unwrap();
        let ctx = dummy_ctx(&src, &dest, 5, &parsed, &[]);
        let (verdict, measurement) = run_checks_with_measurement(&ctx, |_| {
            (ImportVerdict::Accept, Some((123, "device:fixture".into())))
        });
        assert!(matches!(
            verdict,
            ImportVerdict::Reject {
                code: ImportCheckCode::DuplicateFile,
                ..
            }
        ));
        assert_eq!(measurement, Some((123, "device:fixture".into())));
        assert_eq!(std::fs::read(&dest).unwrap(), b"video");
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
