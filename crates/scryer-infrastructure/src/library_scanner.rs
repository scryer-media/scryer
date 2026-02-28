use std::collections::HashSet;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use scryer_application::{AppError, AppResult, LibraryFile, LibraryScanner};
use tokio::fs;

pub struct FileSystemLibraryScanner {
    allowed_extensions: HashSet<String>,
}

impl Default for FileSystemLibraryScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystemLibraryScanner {
    pub fn new() -> Self {
        let allowed_extensions = [
            "mkv", "mp4", "avi", "mov", "wmv", "m4v", "webm",
        ]
        .into_iter()
        .map(|ext| ext.to_string())
        .collect();

        Self { allowed_extensions }
    }

    fn is_allowed_extension(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .is_some_and(|ext| self.allowed_extensions.contains(&ext))
    }
}

#[async_trait]
impl LibraryScanner for FileSystemLibraryScanner {
    async fn scan_library(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        let root_path = PathBuf::from(root);
        let metadata = fs::metadata(&root_path)
            .await
            .map_err(|err| AppError::Validation(format!("library path error: {err}")))?;

        if !metadata.is_dir() {
            return Err(AppError::Validation(
                "library path must be a directory".into(),
            ));
        }

        let mut results = Vec::new();
        let mut stack = vec![root_path];

        while let Some(dir) = stack.pop() {
            let mut entries = fs::read_dir(&dir)
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?;

            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?
            {
                let path = entry.path();
                let entry_metadata = entry
                    .metadata()
                    .await
                    .map_err(|err| AppError::Repository(err.to_string()))?;

                if entry_metadata.is_dir() {
                    stack.push(path);
                    continue;
                }

                if !entry_metadata.is_file() || !self.is_allowed_extension(&path) {
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

                // Discover companion .nfo sidecar file.
                // Priority: <stem>.nfo, then movie.nfo in the same directory.
                let nfo_path = {
                    let nfo_same_stem = path.with_extension("nfo");
                    let nfo_canonical = path.parent().map(|p| p.join("movie.nfo"));

                    if nfo_same_stem.is_file() {
                        Some(nfo_same_stem.to_string_lossy().to_string())
                    } else if nfo_canonical.as_ref().is_some_and(|p| p.is_file()) {
                        Some(nfo_canonical.unwrap().to_string_lossy().to_string())
                    } else {
                        None
                    }
                };

                results.push(LibraryFile {
                    path: path.to_string_lossy().to_string(),
                    display_name,
                    nfo_path,
                });
            }
        }

        Ok(results)
    }
}
