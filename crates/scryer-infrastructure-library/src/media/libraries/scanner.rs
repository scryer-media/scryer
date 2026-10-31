use std::collections::HashSet;
use std::fs as stdfs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use scryer_application::file_source_signature::file_source_signature_from_metadata;
use scryer_application::filesystem_walk::{FilesystemWalker, WalkedDirectory};
use scryer_application::{
    AppError, AppResult, LibraryDirectoryScanResult, LibraryFile, LibraryFileBatchReceiver,
    LibraryScanner, stored_paths::path_to_stored_string,
};
use scryer_domain::VIDEO_EXTENSIONS;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::sync::mpsc;

const LIBRARY_SCAN_DISCOVERY_CHANNEL_CAPACITY: usize = 16;

pub struct FileSystemLibraryScanner {
    allowed_extensions: HashSet<String>,
}

struct ScannedLibraryFile {
    path: PathBuf,
    size_bytes: Option<u64>,
}

impl Default for FileSystemLibraryScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystemLibraryScanner {
    pub fn new() -> Self {
        let allowed_extensions = VIDEO_EXTENSIONS.iter().map(|ext| ext.to_string()).collect();

        Self { allowed_extensions }
    }

    fn path_has_allowed_extension(allowed_extensions: &HashSet<String>, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .is_some_and(|ext| allowed_extensions.contains(&ext))
    }

    async fn validate_root(root: &str) -> AppResult<PathBuf> {
        let root_path = scryer_application::stored_paths::stored_path_to_path_buf(root);
        let metadata = fs::metadata(&root_path).await.map_err(|err| {
            AppError::validation_refused(
                crate::error_reasons::MEDIA_LIBRARY_PATH_UNAVAILABLE,
                format!("library path error: {err}"),
            )
        })?;

        if !metadata.is_dir() {
            return Err(AppError::validation_refused(
                crate::error_reasons::MEDIA_LIBRARY_PATH_NOT_DIRECTORY,
                "library path must be a directory",
            ));
        }

        Ok(root_path)
    }

    async fn scan_with_options(
        &self,
        root: &str,
        discover_movie_nfo: bool,
    ) -> AppResult<Vec<LibraryFile>> {
        let mut receiver = self
            .scan_with_options_batched(root, discover_movie_nfo, usize::MAX)
            .await?;
        let mut results = Vec::new();
        while let Some(batch) = receiver.recv().await {
            results.extend(batch?);
        }
        Ok(results)
    }

    async fn scan_with_options_batched(
        &self,
        root: &str,
        discover_movie_nfo: bool,
        batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        if batch_size == 0 {
            return Err(AppError::validation_refused(
                crate::error_reasons::MEDIA_SCAN_BATCH_SIZE_INVALID,
                "batch size must be greater than 0",
            ));
        }

        let root_path = Self::validate_root(root).await?;
        let allowed_extensions = self.allowed_extensions.clone();
        let (sender, receiver) = mpsc::channel(LIBRARY_SCAN_DISCOVERY_CHANNEL_CAPACITY);

        tokio::spawn(async move {
            if let Err(error) = walk_scan_batches(
                allowed_extensions,
                root_path,
                discover_movie_nfo,
                batch_size,
                sender.clone(),
            )
            .await
            {
                let _ = sender.send(Err(error)).await;
            }
        });

        Ok(receiver)
    }

    async fn scan_directory_with_metrics_internal(
        &self,
        root: &str,
        include_source_snapshot: bool,
    ) -> AppResult<LibraryDirectoryScanResult> {
        let root_path = Self::validate_root(root).await?;
        let allowed_extensions = self.allowed_extensions.clone();

        tokio::task::spawn_blocking(move || {
            scan_directory_with_metrics_blocking(
                allowed_extensions,
                root_path,
                include_source_snapshot,
            )
        })
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?
    }
}

async fn walk_scan_batches(
    allowed_extensions: HashSet<String>,
    root_path: PathBuf,
    discover_movie_nfo: bool,
    batch_size: usize,
    sender: mpsc::Sender<AppResult<Vec<LibraryFile>>>,
) -> AppResult<()> {
    tokio::task::spawn_blocking({
        move || {
            walk_scan_batches_blocking(
                allowed_extensions,
                root_path,
                discover_movie_nfo,
                batch_size,
                sender,
            )
        }
    })
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?
}

fn walk_scan_batches_blocking(
    allowed_extensions: HashSet<String>,
    root_path: PathBuf,
    discover_movie_nfo: bool,
    batch_size: usize,
    sender: mpsc::Sender<AppResult<Vec<LibraryFile>>>,
) -> AppResult<()> {
    let mut batch = Vec::with_capacity(batch_size.min(256));

    let walker = if discover_movie_nfo {
        FilesystemWalker::new()
            .skip_movie_scan_junk_and_extras()
            .supported_video_and_nfo_files()
            .max_depth(scryer_application::LIBRARY_SCAN_MAX_RECURSIVE_DEPTH)
    } else {
        FilesystemWalker::new()
            .skip_episodic_scan_junk_and_trailers()
            .supported_video_files_only()
            .max_depth(scryer_application::LIBRARY_SCAN_MAX_RECURSIVE_DEPTH)
    };

    walker.walk_with(&root_path, |walked_dir| {
        scan_walked_directory_blocking(
            &allowed_extensions,
            &root_path,
            discover_movie_nfo,
            walked_dir,
            batch_size,
            &sender,
            &mut batch,
        )
    })?;

    if !batch.is_empty() {
        let _ = sender.blocking_send(Ok(batch));
    }

    Ok(())
}

fn scan_walked_directory_blocking(
    allowed_extensions: &HashSet<String>,
    root_path: &Path,
    discover_movie_nfo: bool,
    walked_dir: WalkedDirectory,
    batch_size: usize,
    sender: &mpsc::Sender<AppResult<Vec<LibraryFile>>>,
    batch: &mut Vec<LibraryFile>,
) -> AppResult<bool> {
    let WalkedDirectory {
        path: dir_path,
        files,
        filenames_lower,
        ..
    } = walked_dir;

    let mut primary_movie_candidate: Option<PathBuf> = None;
    let movie_nfo_path = dir_path.join("movie.nfo");
    if discover_movie_nfo && dir_path != root_path && filenames_lower.contains("movie.nfo") {
        let mut non_sample_videos = Vec::new();
        let mut files = files
            .iter()
            .cloned()
            .map(|path| ScannedLibraryFile {
                path,
                size_bytes: None,
            })
            .collect::<Vec<_>>();
        for file in &mut files {
            if !FileSystemLibraryScanner::path_has_allowed_extension(allowed_extensions, &file.path)
            {
                continue;
            }
            if file.size_bytes.is_none() {
                file.size_bytes = stdfs::metadata(&file.path).ok().map(|meta| meta.len());
            }
            if is_sample_video_candidate(&file.path, file.size_bytes) {
                continue;
            }
            non_sample_videos.push(file.path.clone());
        }
        if non_sample_videos.len() == 1 {
            primary_movie_candidate = non_sample_videos.into_iter().next();
        }
    }

    for path in files {
        if !FileSystemLibraryScanner::path_has_allowed_extension(allowed_extensions, &path) {
            continue;
        }

        let display_name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();

        if display_name.trim().is_empty() {
            continue;
        }

        let nfo_path = if discover_movie_nfo {
            let same_stem_name = format!("{display_name}.nfo").to_ascii_lowercase();
            if filenames_lower.contains(&same_stem_name) {
                Some(path_to_stored_string(path.with_extension("nfo")))
            } else if primary_movie_candidate.as_ref() == Some(&path) {
                Some(path_to_stored_string(&movie_nfo_path))
            } else {
                None
            }
        } else {
            None
        };

        batch.push(LibraryFile {
            path: path_to_stored_string(&path),
            display_name,
            nfo_path,
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        });

        if batch.len() >= batch_size && sender.blocking_send(Ok(std::mem::take(batch))).is_err() {
            return Ok(false);
        }
    }

    Ok(true)
}

fn scan_directory_with_metrics_blocking(
    allowed_extensions: HashSet<String>,
    root_path: PathBuf,
    include_source_snapshot: bool,
) -> AppResult<LibraryDirectoryScanResult> {
    let started_at = Instant::now();
    let mut stat_elapsed = Duration::ZERO;
    let mut files = Vec::new();

    FilesystemWalker::new()
        .skip_episodic_scan_junk_and_trailers()
        .supported_video_files_only()
        .max_depth(scryer_application::LIBRARY_SCAN_MAX_RECURSIVE_DEPTH)
        .walk_with(&root_path, |walked_dir| {
            collect_directory_files_with_source_snapshot(
                &allowed_extensions,
                walked_dir,
                include_source_snapshot,
                &mut files,
                &mut stat_elapsed,
            )?;
            Ok(true)
        })?;

    let elapsed = started_at.elapsed();
    let walk_elapsed = elapsed.saturating_sub(stat_elapsed);

    Ok(LibraryDirectoryScanResult {
        files,
        walk_ms: u64::try_from(walk_elapsed.as_millis()).unwrap_or(u64::MAX),
        stat_ms: u64::try_from(stat_elapsed.as_millis()).unwrap_or(u64::MAX),
        elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
    })
}

fn collect_directory_files_with_source_snapshot(
    allowed_extensions: &HashSet<String>,
    walked_dir: WalkedDirectory,
    include_source_snapshot: bool,
    files: &mut Vec<LibraryFile>,
    stat_elapsed: &mut Duration,
) -> AppResult<()> {
    let WalkedDirectory {
        files: dir_files, ..
    } = walked_dir;

    for path in dir_files {
        if !FileSystemLibraryScanner::path_has_allowed_extension(allowed_extensions, &path) {
            continue;
        }

        let display_name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();

        if display_name.trim().is_empty() {
            continue;
        }

        let (size_bytes, source_signature) = if include_source_snapshot {
            let stat_started = Instant::now();
            let metadata = stdfs::metadata(&path).ok();
            *stat_elapsed = stat_elapsed.saturating_add(stat_started.elapsed());

            let size_bytes = metadata
                .as_ref()
                .map(|metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX));
            let source_signature = metadata
                .as_ref()
                .and_then(|metadata| file_source_signature_from_metadata(metadata).ok());

            (size_bytes, source_signature)
        } else {
            (None, None)
        };
        let (source_signature_scheme, source_signature_value) = source_signature
            .map(|signature| (Some(signature.scheme), Some(signature.value)))
            .unwrap_or((None, None));

        files.push(LibraryFile {
            path: path_to_stored_string(&path),
            display_name,
            nfo_path: None,
            size_bytes,
            source_signature_scheme,
            source_signature_value,
        });
    }

    Ok(())
}

fn is_sample_video_candidate(path: &Path, size_bytes: Option<u64>) -> bool {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if stem.contains("sample") {
        return true;
    }

    size_bytes.is_some_and(|size| size < 50 * 1024 * 1024)
}

#[async_trait]
impl LibraryScanner for FileSystemLibraryScanner {
    async fn scan_library(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        self.scan_with_options(root, true).await
    }

    async fn scan_directory(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        Ok(self
            .scan_directory_with_metrics_internal(root, false)
            .await?
            .files)
    }

    async fn scan_directory_children(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        let root_path = Self::validate_root(root).await?;
        let allowed_extensions = self.allowed_extensions.clone();

        tokio::task::spawn_blocking(move || {
            let mut files = Vec::new();
            let entries = stdfs::read_dir(&root_path).map_err(|err| {
                AppError::validation_refused(
                    crate::error_reasons::MEDIA_LIBRARY_PATH_UNAVAILABLE,
                    format!("library path error: {err}"),
                )
            })?;
            for entry in entries.filter_map(|entry| entry.ok()) {
                let path = entry.path();
                let is_file = entry
                    .file_type()
                    .map(|file_type| file_type.is_file())
                    .unwrap_or_else(|_| path.is_file());
                if !is_file
                    || !FileSystemLibraryScanner::path_has_allowed_extension(
                        &allowed_extensions,
                        &path,
                    )
                {
                    continue;
                }
                files.push(LibraryFile {
                    path: path_to_stored_string(&path),
                    display_name: path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    nfo_path: None,
                    size_bytes: None,
                    source_signature_scheme: None,
                    source_signature_value: None,
                });
            }
            files.sort_by(|left, right| left.path.cmp(&right.path));
            Ok(files)
        })
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn scan_library_batched(
        &self,
        root: &str,
        batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        self.scan_with_options_batched(root, true, batch_size).await
    }

    async fn scan_directory_batched(
        &self,
        root: &str,
        batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        self.scan_with_options_batched(root, false, batch_size)
            .await
    }

    async fn scan_directory_with_metrics(
        &self,
        root: &str,
    ) -> AppResult<LibraryDirectoryScanResult> {
        self.scan_directory_with_metrics_internal(root, true).await
    }

    async fn scan_directory_for_progress_with_metrics(
        &self,
        root: &str,
    ) -> AppResult<LibraryDirectoryScanResult> {
        self.scan_directory_with_metrics_internal(root, false).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn scan_library_prefers_same_stem_nfo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let movie_path = dir.path().join("Movie.Title.2024.mkv");
        tokio::fs::write(&movie_path, b"video")
            .await
            .expect("write movie");
        tokio::fs::write(movie_path.with_extension("nfo"), b"<movie/>")
            .await
            .expect("write nfo");

        let scanner = FileSystemLibraryScanner::new();
        let files = scanner
            .scan_library(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan library");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].nfo_path.as_deref(),
            Some(movie_path.with_extension("nfo").to_string_lossy().as_ref())
        );
    }

    #[tokio::test]
    async fn scan_library_supports_movie_nfo_in_dedicated_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let movie_dir = dir.path().join("Movie Title (2024)");
        tokio::fs::create_dir_all(&movie_dir)
            .await
            .expect("create movie dir");
        let movie_path = movie_dir.join("Movie.Title.2024.mkv");
        let file = std::fs::File::create(&movie_path).expect("create movie");
        file.set_len(60 * 1024 * 1024).expect("set movie size");
        let movie_nfo_path = movie_dir.join("movie.nfo");
        tokio::fs::write(&movie_nfo_path, b"<movie/>")
            .await
            .expect("write movie nfo");

        let scanner = FileSystemLibraryScanner::new();
        let files = scanner
            .scan_library(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan library");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].nfo_path.as_deref(),
            Some(movie_nfo_path.to_string_lossy().as_ref())
        );
    }

    #[tokio::test]
    async fn scan_library_ignores_arbitrary_nfo_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let movie_path = dir.path().join("Movie.Title.2024.mkv");
        tokio::fs::write(&movie_path, b"video")
            .await
            .expect("write movie");
        tokio::fs::write(dir.path().join("random.nfo"), b"<movie/>")
            .await
            .expect("write random nfo");

        let scanner = FileSystemLibraryScanner::new();
        let files = scanner
            .scan_library(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan library");

        assert_eq!(files.len(), 1);
        assert!(files[0].nfo_path.is_none());
    }

    #[tokio::test]
    async fn scan_directory_skips_nfo_companion_lookup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let movie_path = dir.path().join("Movie.Title.2024.mkv");
        tokio::fs::write(&movie_path, b"video")
            .await
            .expect("write movie");
        tokio::fs::write(movie_path.with_extension("nfo"), b"<movie/>")
            .await
            .expect("write nfo");
        tokio::fs::write(dir.path().join("poster.jpg"), b"poster")
            .await
            .expect("write poster");

        let scanner = FileSystemLibraryScanner::new();
        let files = scanner
            .scan_directory(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan directory");

        assert_eq!(files.len(), 1);
        assert!(files[0].nfo_path.is_none());
        assert!(files[0].size_bytes.is_none());
        assert!(files[0].source_signature_scheme.is_none());
        assert!(files[0].source_signature_value.is_none());
    }

    #[tokio::test]
    async fn scan_directory_for_progress_with_metrics_ignores_non_video_sidecars() {
        let dir = tempfile::tempdir().expect("tempdir");
        let episode_path = dir.path().join("Episode.S01E01.mkv");
        tokio::fs::write(&episode_path, b"video")
            .await
            .expect("write episode");
        tokio::fs::write(dir.path().join("Episode.S01E01.nfo"), b"<episodedetails/>")
            .await
            .expect("write nfo");
        tokio::fs::write(dir.path().join("Episode.S01E01-thumb.jpg"), b"thumb")
            .await
            .expect("write thumb");
        tokio::fs::write(dir.path().join("poster.jpg"), b"poster")
            .await
            .expect("write poster");

        let scanner = FileSystemLibraryScanner::new();
        let result = scanner
            .scan_directory_for_progress_with_metrics(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan directory for progress");

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, episode_path.to_string_lossy());
    }

    #[tokio::test]
    async fn scan_directory_with_metrics_captures_source_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let episode_path = dir.path().join("Episode.S01E01.mkv");
        tokio::fs::write(&episode_path, b"video")
            .await
            .expect("write episode");

        let scanner = FileSystemLibraryScanner::new();
        let result = scanner
            .scan_directory_with_metrics(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan directory with metrics");

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, episode_path.to_string_lossy());
        assert!(result.files[0].size_bytes.is_some());
        assert!(result.files[0].source_signature_scheme.is_some());
        assert!(result.files[0].source_signature_value.is_some());
        assert!(result.elapsed_ms >= result.stat_ms);
    }

    #[tokio::test]
    async fn scan_directory_for_progress_with_metrics_omits_source_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let episode_path = dir.path().join("Episode.S01E01.mkv");
        tokio::fs::write(&episode_path, b"video")
            .await
            .expect("write episode");

        let scanner = FileSystemLibraryScanner::new();
        let result = scanner
            .scan_directory_for_progress_with_metrics(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan directory for progress");

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, episode_path.to_string_lossy());
        assert!(result.files[0].size_bytes.is_none());
        assert!(result.files[0].source_signature_scheme.is_none());
        assert!(result.files[0].source_signature_value.is_none());
        assert_eq!(result.stat_ms, 0);
    }

    #[tokio::test]
    async fn scan_library_batched_preserves_sorted_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("B");
        tokio::fs::create_dir_all(&nested)
            .await
            .expect("create nested");
        tokio::fs::write(dir.path().join("A.mkv"), b"video")
            .await
            .expect("write a");
        tokio::fs::write(nested.join("C.mkv"), b"video")
            .await
            .expect("write c");
        tokio::fs::write(nested.join("D.mkv"), b"video")
            .await
            .expect("write d");

        let scanner = FileSystemLibraryScanner::new();
        let mut receiver = scanner
            .scan_library_batched(dir.path().to_string_lossy().as_ref(), 2)
            .await
            .expect("scan library batched");
        let mut files = Vec::new();
        while let Some(batch) = receiver.recv().await {
            files.extend(batch.expect("batch result"));
        }

        assert_eq!(files.len(), 3);
        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                dir.path().join("A.mkv").to_string_lossy().to_string(),
                nested.join("C.mkv").to_string_lossy().to_string(),
                nested.join("D.mkv").to_string_lossy().to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn scan_library_includes_transport_stream_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let episode_path = dir.path().join("Show - 4x01 - Episode.ts");
        tokio::fs::write(&episode_path, b"video")
            .await
            .expect("write episode");

        let scanner = FileSystemLibraryScanner::new();
        let files = scanner
            .scan_library(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan library");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, episode_path.to_string_lossy());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn scan_library_includes_stream_pointer_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let episode_path = dir.path().join("Show - S01E01.strm");
        tokio::fs::write(&episode_path, b"https://nzbdav.example/stream/Show.S01E01")
            .await
            .expect("write episode");

        let scanner = FileSystemLibraryScanner::new();
        let files = scanner
            .scan_library(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan library");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, episode_path.to_string_lossy());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn scan_library_follows_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("Season 1");
        tokio::fs::create_dir_all(&target)
            .await
            .expect("target dir");
        tokio::fs::write(target.join("Show - 1x01 - Episode.mkv"), b"video")
            .await
            .expect("write episode");
        symlink(&target, dir.path().join("Linked Season 1")).expect("symlink");

        let scanner = FileSystemLibraryScanner::new();
        let files = scanner
            .scan_library(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan library");

        assert_eq!(files.len(), 1);
        assert!(
            files[0]
                .path
                .ends_with("Linked Season 1/Show - 1x01 - Episode.mkv")
        );
    }

    #[tokio::test]
    async fn scan_directory_for_progress_with_metrics_skips_junk_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let show_dir = dir.path().join("Show");
        let junk_dir = show_dir.join("@eaDir");
        let trickplay_dir = show_dir.join("Episode.S01E01.trickplay");
        let recycle_dir = show_dir.join("$RECYCLE.BIN");
        let system_dir = show_dir.join("System Volume Information");
        let lost_found_dir = show_dir.join("lost+found");
        tokio::fs::create_dir_all(&junk_dir)
            .await
            .expect("junk dir");
        tokio::fs::create_dir_all(&trickplay_dir)
            .await
            .expect("trickplay dir");
        tokio::fs::create_dir_all(&recycle_dir)
            .await
            .expect("recycle dir");
        tokio::fs::create_dir_all(&system_dir)
            .await
            .expect("system dir");
        tokio::fs::create_dir_all(&lost_found_dir)
            .await
            .expect("lost+found dir");
        tokio::fs::write(show_dir.join("Episode.S01E01.mkv"), b"video")
            .await
            .expect("episode");
        tokio::fs::write(junk_dir.join("Episode.S01E02.mkv"), b"video")
            .await
            .expect("junk episode");
        tokio::fs::write(trickplay_dir.join("segment001.mkv"), b"video")
            .await
            .expect("trickplay segment");
        tokio::fs::write(recycle_dir.join("Episode.S01E03.mkv"), b"video")
            .await
            .expect("recycle episode");
        tokio::fs::write(system_dir.join("Episode.S01E04.mkv"), b"video")
            .await
            .expect("system episode");
        tokio::fs::write(lost_found_dir.join("Episode.S01E05.mkv"), b"video")
            .await
            .expect("lost+found episode");

        let scanner = FileSystemLibraryScanner::new();
        let result = scanner
            .scan_directory_for_progress_with_metrics(show_dir.to_string_lossy().as_ref())
            .await
            .expect("scan directory");

        assert_eq!(
            result
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                show_dir
                    .join("Episode.S01E01.mkv")
                    .to_string_lossy()
                    .to_string()
            ]
        );
    }

    #[tokio::test]
    async fn scan_directory_for_progress_with_metrics_skips_episodic_trailer_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let show_dir = dir.path().join("Anime Show");
        let season_dir = show_dir.join("Season 1");
        let extras_dir = show_dir.join("extras");
        let featurettes_dir = show_dir.join("Featurettes");
        let backdrops_dir = show_dir.join("backdrops");
        let theme_music_dir = show_dir.join("theme_music");
        let trailers_dir = show_dir.join("trailers");
        let titled_trailers_dir = show_dir.join("12 Tides a Shore (Trailers)");
        tokio::fs::create_dir_all(&season_dir)
            .await
            .expect("season dir");
        tokio::fs::create_dir_all(&extras_dir)
            .await
            .expect("extras dir");
        tokio::fs::create_dir_all(&featurettes_dir)
            .await
            .expect("featurettes dir");
        tokio::fs::create_dir_all(&backdrops_dir)
            .await
            .expect("backdrops dir");
        tokio::fs::create_dir_all(&theme_music_dir)
            .await
            .expect("theme music dir");
        tokio::fs::create_dir_all(&trailers_dir)
            .await
            .expect("trailers dir");
        tokio::fs::create_dir_all(&titled_trailers_dir)
            .await
            .expect("titled trailers dir");
        tokio::fs::write(season_dir.join("Episode.S01E01.mkv"), b"video")
            .await
            .expect("episode");
        tokio::fs::write(show_dir.join("Episode.S01E02-trailer.mkv"), b"video")
            .await
            .expect("trailer suffix");
        tokio::fs::write(extras_dir.join("Episode.S00E01.mkv"), b"video")
            .await
            .expect("extra");
        tokio::fs::write(featurettes_dir.join("Featurette.mkv"), b"video")
            .await
            .expect("featurette");
        tokio::fs::write(backdrops_dir.join("Backdrop.mkv"), b"video")
            .await
            .expect("backdrop");
        tokio::fs::write(theme_music_dir.join("Theme.Music.mkv"), b"video")
            .await
            .expect("theme music");
        tokio::fs::write(trailers_dir.join("Episode.S00E01.mkv"), b"video")
            .await
            .expect("trailer");
        tokio::fs::write(titled_trailers_dir.join("Feature.Trailer.mkv"), b"video")
            .await
            .expect("titled trailer");

        let scanner = FileSystemLibraryScanner::new();
        let result = scanner
            .scan_directory_for_progress_with_metrics(show_dir.to_string_lossy().as_ref())
            .await
            .expect("scan directory");

        assert_eq!(
            result
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                season_dir
                    .join("Episode.S01E01.mkv")
                    .to_string_lossy()
                    .to_string()
            ]
        );
    }

    #[tokio::test]
    async fn scan_library_skips_movie_extras_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let extras_dir = dir.path().join("extras");
        let trailers_dir = dir.path().join("trailers");
        let theme_music_dir = dir.path().join("theme-music");
        let recycle_dir = dir.path().join("#recycle");
        let trash_dir = dir.path().join("trash");
        tokio::fs::create_dir_all(&extras_dir)
            .await
            .expect("extras dir");
        tokio::fs::create_dir_all(&trailers_dir)
            .await
            .expect("trailers dir");
        tokio::fs::create_dir_all(&theme_music_dir)
            .await
            .expect("theme music dir");
        tokio::fs::create_dir_all(&recycle_dir)
            .await
            .expect("recycle dir");
        tokio::fs::create_dir_all(&trash_dir)
            .await
            .expect("trash dir");
        tokio::fs::write(dir.path().join("Movie.Title.2024.mkv"), b"video")
            .await
            .expect("movie");
        tokio::fs::write(dir.path().join("Movie.Title.2024-trailer.mkv"), b"video")
            .await
            .expect("trailer suffix");
        tokio::fs::write(extras_dir.join("Featurette.mkv"), b"video")
            .await
            .expect("featurette");
        tokio::fs::write(trailers_dir.join("Trailer.mkv"), b"video")
            .await
            .expect("trailer");
        tokio::fs::write(theme_music_dir.join("Theme.Music.mkv"), b"video")
            .await
            .expect("theme music");
        tokio::fs::write(recycle_dir.join("Deleted.Movie.mkv"), b"video")
            .await
            .expect("recycle movie");
        tokio::fs::write(trash_dir.join("Trash.Movie.mkv"), b"video")
            .await
            .expect("trash movie");

        let scanner = FileSystemLibraryScanner::new();
        let files = scanner
            .scan_library(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan library");

        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                dir.path()
                    .join("Movie.Title.2024.mkv")
                    .to_string_lossy()
                    .to_string()
            ]
        );
    }

    #[tokio::test]
    async fn scan_directory_with_metrics_stops_descending_after_depth_three() {
        let dir = tempfile::tempdir().expect("tempdir");
        let level1 = dir.path().join("Season 1");
        let level2 = level1.join("Disc 1");
        let level3 = level2.join("Nested");
        let level4 = level3.join("TooDeep");
        tokio::fs::create_dir_all(&level4).await.expect("level4");
        tokio::fs::write(level3.join("Episode.S01E01.mkv"), b"video")
            .await
            .expect("depth 3 file");
        tokio::fs::write(level4.join("Episode.S01E02.mkv"), b"video")
            .await
            .expect("depth 4 file");

        let scanner = FileSystemLibraryScanner::new();
        let result = scanner
            .scan_directory_with_metrics(dir.path().to_string_lossy().as_ref())
            .await
            .expect("scan directory with metrics");

        assert_eq!(
            result
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                level3
                    .join("Episode.S01E01.mkv")
                    .to_string_lossy()
                    .to_string()
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "diagnostic harness for local mounted media roots"]
    async fn profile_real_media_root_walks() {
        let roots = std::env::var("SCRYER_WALK_PROFILE_ROOTS").unwrap_or_else(|_| {
            "/Volumes/Media/Movies:/Volumes/Media/Anime:/Volumes/Media/TV".to_string()
        });
        let limit = std::env::var("SCRYER_WALK_PROFILE_LIMIT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok());
        let scanner = FileSystemLibraryScanner::new();

        for root in roots.split(':').filter(|root| !root.trim().is_empty()) {
            let root_path = PathBuf::from(root);
            if !root_path.is_dir() {
                eprintln!("ROOT\t{}\tmissing", root_path.display());
                continue;
            }

            let started = Instant::now();
            let mut entries = stdfs::read_dir(&root_path)
                .expect("read root")
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .collect::<Vec<_>>();
            entries.sort();
            let list_ms = started.elapsed().as_millis();
            let dirs = entries.iter().filter(|path| path.is_dir()).count();
            let files = entries.iter().filter(|path| path.is_file()).count();
            eprintln!(
                "ROOT\t{}\tentries={}\tdirs={}\tfiles={}\tlist_ms={}",
                root_path.display(),
                entries.len(),
                dirs,
                files,
                list_ms
            );

            if let Some(limit) = limit {
                entries.truncate(limit);
            }

            if root_path.ends_with("Movies") {
                profile_movie_entries(&scanner, &root_path, &entries).await;
            } else {
                profile_episodic_entries(&scanner, &root_path, &entries).await;
            }
        }
    }

    async fn profile_movie_entries(
        scanner: &FileSystemLibraryScanner,
        root: &Path,
        entries: &[PathBuf],
    ) {
        let mut rows = Vec::new();
        let mut total_scan_directory_ms = 0u128;
        let mut total_scan_library_ms = 0u128;

        for entry in entries.iter().filter(|path| path.is_dir()) {
            let nfo_started = Instant::now();
            let movie_nfo = entry.join("movie.nfo");
            let nfo_bytes = stdfs::read_to_string(&movie_nfo)
                .ok()
                .map(|content| content.len())
                .unwrap_or_default();
            let nfo_ms = nfo_started.elapsed().as_millis();

            let walked = count_walked_directories(entry, true);

            let scan_directory_started = Instant::now();
            let scan_directory_files = scanner
                .scan_directory(entry.to_string_lossy().as_ref())
                .await
                .expect("scan directory");
            let scan_directory_ms = scan_directory_started.elapsed().as_millis();
            total_scan_directory_ms = total_scan_directory_ms.saturating_add(scan_directory_ms);

            let scan_library_started = Instant::now();
            let scan_library_files = scanner
                .scan_library(entry.to_string_lossy().as_ref())
                .await
                .expect("scan library");
            let scan_library_ms = scan_library_started.elapsed().as_millis();
            total_scan_library_ms = total_scan_library_ms.saturating_add(scan_library_ms);

            rows.push(ProfileRow {
                path: entry.clone(),
                nfo_bytes,
                nfo_ms,
                walked_dirs: walked.walked_dirs,
                trickplay_dirs: walked.trickplay_dirs,
                skipped_like_dirs: walked.skipped_like_dirs,
                scan_directory_files: scan_directory_files.len(),
                scan_directory_ms,
                scan_library_files: scan_library_files.len(),
                scan_library_ms,
            });
        }

        eprintln!(
            "MOVIES_SUMMARY\troot={}\tfolders={}\tscan_directory_total_ms={}\tscan_library_total_ms={}",
            root.display(),
            rows.len(),
            total_scan_directory_ms,
            total_scan_library_ms
        );
        print_slowest_rows("MOVIES_SCAN_DIRECTORY_SLOW", &rows, |row| {
            row.scan_directory_ms
        });
        print_slowest_rows("MOVIES_SCAN_LIBRARY_SLOW", &rows, |row| row.scan_library_ms);
        print_slowest_rows("MOVIES_NFO_SLOW", &rows, |row| row.nfo_ms);
    }

    async fn profile_episodic_entries(
        scanner: &FileSystemLibraryScanner,
        root: &Path,
        entries: &[PathBuf],
    ) {
        let mut rows = Vec::new();
        let mut total_nfo_ms = 0u128;
        let mut total_progress_ms = 0u128;

        for entry in entries.iter().filter(|path| path.is_dir()) {
            let nfo_started = Instant::now();
            let tvshow_nfo = entry.join("tvshow.nfo");
            let nfo_bytes = stdfs::read_to_string(&tvshow_nfo)
                .ok()
                .map(|content| content.len())
                .unwrap_or_default();
            let nfo_ms = nfo_started.elapsed().as_millis();
            total_nfo_ms = total_nfo_ms.saturating_add(nfo_ms);

            let walked = count_walked_directories(entry, false);
            let progress_started = Instant::now();
            let progress = scanner
                .scan_directory_for_progress_with_metrics(entry.to_string_lossy().as_ref())
                .await
                .expect("scan directory for progress");
            let progress_ms = progress_started.elapsed().as_millis();
            total_progress_ms = total_progress_ms.saturating_add(progress_ms);

            rows.push(ProfileRow {
                path: entry.clone(),
                nfo_bytes,
                nfo_ms,
                walked_dirs: walked.walked_dirs,
                trickplay_dirs: walked.trickplay_dirs,
                skipped_like_dirs: walked.skipped_like_dirs,
                scan_directory_files: progress.files.len(),
                scan_directory_ms: progress_ms,
                scan_library_files: 0,
                scan_library_ms: 0,
            });
        }

        eprintln!(
            "EPISODIC_SUMMARY\troot={}\tfolders={}\tnfo_total_ms={}\tprogress_total_ms={}",
            root.display(),
            rows.len(),
            total_nfo_ms,
            total_progress_ms
        );
        print_slowest_rows("EPISODIC_PROGRESS_SLOW", &rows, |row| row.scan_directory_ms);
        print_slowest_rows("EPISODIC_NFO_SLOW", &rows, |row| row.nfo_ms);
    }

    #[derive(Default)]
    struct WalkCount {
        walked_dirs: usize,
        trickplay_dirs: usize,
        skipped_like_dirs: usize,
    }

    #[derive(Clone)]
    struct ProfileRow {
        path: PathBuf,
        nfo_bytes: usize,
        nfo_ms: u128,
        walked_dirs: usize,
        trickplay_dirs: usize,
        skipped_like_dirs: usize,
        scan_directory_files: usize,
        scan_directory_ms: u128,
        scan_library_files: usize,
        scan_library_ms: u128,
    }

    fn count_walked_directories(root: &Path, movie_policy: bool) -> WalkCount {
        let mut count = WalkCount::default();
        let walker = if movie_policy {
            FilesystemWalker::new()
                .skip_movie_scan_junk_and_extras()
                .supported_video_and_nfo_files()
        } else {
            FilesystemWalker::new()
                .skip_episodic_scan_junk_and_trailers()
                .supported_video_files_only()
        }
        .max_depth(scryer_application::LIBRARY_SCAN_MAX_RECURSIVE_DEPTH);

        walker
            .walk_with(root, |walked_dir| {
                count.walked_dirs = count.walked_dirs.saturating_add(1);
                let name = walked_dir
                    .path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if name.starts_with('.') || name.ends_with(".trickplay") {
                    count.trickplay_dirs = count.trickplay_dirs.saturating_add(1);
                }
                for subdir in &walked_dir.subdirs {
                    let name = subdir
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if name.contains("trailer")
                        || name == "extras"
                        || name == "featurettes"
                        || name.ends_with(".trickplay")
                    {
                        count.skipped_like_dirs = count.skipped_like_dirs.saturating_add(1);
                    }
                }
                Ok(true)
            })
            .expect("count walked directories");

        count
    }

    fn print_slowest_rows(label: &str, rows: &[ProfileRow], elapsed: impl Fn(&ProfileRow) -> u128) {
        let mut rows = rows.to_vec();
        rows.sort_by_key(|row| std::cmp::Reverse(elapsed(row)));
        for row in rows.into_iter().take(12) {
            eprintln!(
                "{}\tms={}\tnfo_ms={}\tnfo_bytes={}\twalked_dirs={}\ttrickplay_dirs={}\tskipped_like_dirs={}\tscan_directory_files={}\tscan_library_files={}\tpath={}",
                label,
                elapsed(&row),
                row.nfo_ms,
                row.nfo_bytes,
                row.walked_dirs,
                row.trickplay_dirs,
                row.skipped_like_dirs,
                row.scan_directory_files,
                row.scan_library_files,
                row.path.display()
            );
        }
    }
}
