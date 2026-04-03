use std::collections::HashSet;
use std::fs as stdfs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use scryer_application::filesystem_walk::{FilesystemWalker, WalkedDirectory};
use scryer_application::{
    AppError, AppResult, LibraryDirectoryScanResult, LibraryFile, LibraryFileBatchReceiver,
    LibraryScanner, source_signature_from_std_metadata,
};
use scryer_domain::VIDEO_EXTENSIONS;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::sync::mpsc;

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
        let root_path = PathBuf::from(root);
        let metadata = fs::metadata(&root_path)
            .await
            .map_err(|err| AppError::Validation(format!("library path error: {err}")))?;

        if !metadata.is_dir() {
            return Err(AppError::Validation(
                "library path must be a directory".into(),
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
            return Err(AppError::Validation(
                "batch size must be greater than 0".into(),
            ));
        }

        let root_path = Self::validate_root(root).await?;
        let allowed_extensions = self.allowed_extensions.clone();
        let (sender, receiver) = mpsc::channel(1);

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

    FilesystemWalker::new().walk_with(&root_path, |walked_dir| {
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
                Some(path.with_extension("nfo").to_string_lossy().to_string())
            } else if primary_movie_candidate.as_ref() == Some(&path) {
                Some(movie_nfo_path.to_string_lossy().to_string())
            } else {
                None
            }
        } else {
            None
        };

        batch.push(LibraryFile {
            path: path.to_string_lossy().to_string(),
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

    FilesystemWalker::new().walk_with(&root_path, |walked_dir| {
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

        let (size_bytes, source_signature_scheme, source_signature_value) =
            if include_source_snapshot {
                let stat_started = Instant::now();
                let metadata = stdfs::metadata(&path).ok();
                *stat_elapsed = stat_elapsed.saturating_add(stat_started.elapsed());

                let size_bytes = metadata
                    .as_ref()
                    .map(|metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX));
                let (source_signature_scheme, source_signature_value) = metadata
                    .as_ref()
                    .and_then(source_signature_from_std_metadata)
                    .map_or((None, None), |(scheme, value)| (Some(scheme), Some(value)));
                (size_bytes, source_signature_scheme, source_signature_value)
            } else {
                (None, None, None)
            };

        files.push(LibraryFile {
            path: path.to_string_lossy().to_string(),
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
}
