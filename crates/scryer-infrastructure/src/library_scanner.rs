use std::collections::HashSet;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, LibraryFile, LibraryFileBatchReceiver, LibraryScanner,
};
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
        let allowed_extensions = ["mkv", "mp4", "avi", "mov", "wmv", "m4v", "webm"]
            .into_iter()
            .map(|ext| ext.to_string())
            .collect();

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
}

async fn walk_scan_batches(
    allowed_extensions: HashSet<String>,
    root_path: PathBuf,
    discover_movie_nfo: bool,
    batch_size: usize,
    sender: mpsc::Sender<AppResult<Vec<LibraryFile>>>,
) -> AppResult<()> {
    let mut stack = vec![root_path.clone()];
    let mut batch = Vec::with_capacity(batch_size.min(256));

    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let mut subdirs = Vec::new();
        let mut files = Vec::new();
        let mut filenames = HashSet::new();

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
        {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?;

            if file_type.is_dir() {
                subdirs.push(path);
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
                filenames.insert(name.to_ascii_lowercase());
            }
            files.push(ScannedLibraryFile {
                path,
                size_bytes: None,
            });
        }

        subdirs.sort();
        stack.extend(subdirs.into_iter().rev());

        let mut primary_movie_candidate: Option<PathBuf> = None;
        let movie_nfo_path = dir.join("movie.nfo");
        if discover_movie_nfo && dir != root_path && filenames.contains("movie.nfo") {
            let mut non_sample_videos = Vec::new();
            for file in &mut files {
                if !FileSystemLibraryScanner::path_has_allowed_extension(
                    &allowed_extensions,
                    &file.path,
                ) {
                    continue;
                }
                if file.size_bytes.is_none() {
                    file.size_bytes = fs::metadata(&file.path).await.ok().map(|meta| meta.len());
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

        files.sort_by(|left, right| left.path.cmp(&right.path));

        for file in files {
            let path = file.path;
            if !FileSystemLibraryScanner::path_has_allowed_extension(&allowed_extensions, &path) {
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
                if filenames.contains(&same_stem_name) {
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
            });

            if batch.len() >= batch_size
                && sender.send(Ok(std::mem::take(&mut batch))).await.is_err()
            {
                return Ok(());
            }
        }
    }

    if !batch.is_empty() {
        let _ = sender.send(Ok(batch)).await;
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
        self.scan_with_options(root, false).await
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
}
