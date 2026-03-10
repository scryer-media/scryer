use async_trait::async_trait;
use scryer_application::{AppError, AppResult, FileImporter};
use scryer_domain::{ImportFileResult, ImportStrategy};
use std::path::Path;

pub struct FsFileImporter;

impl Default for FsFileImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl FsFileImporter {
    pub fn new() -> Self {
        Self
    }
}

fn is_cross_device_error(err: &std::io::Error) -> bool {
    // EXDEV = errno 18 on both Linux and macOS
    // Windows: ERROR_NOT_SAME_DEVICE = 17
    matches!(err.raw_os_error(), Some(18) | Some(17))
}

#[async_trait]
impl FileImporter for FsFileImporter {
    async fn import_file(&self, source: &Path, dest: &Path) -> AppResult<ImportFileResult> {
        let source = source.to_path_buf();
        let dest = dest.to_path_buf();

        tokio::task::spawn_blocking(move || {
            // Validate source exists and is a regular file
            let source_meta = std::fs::metadata(&source).map_err(|e| {
                AppError::Repository(format!(
                    "import source not found or inaccessible: {}: {}",
                    source.display(),
                    e
                ))
            })?;

            if !source_meta.is_file() {
                return Err(AppError::Repository(format!(
                    "import source is not a regular file: {}",
                    source.display()
                )));
            }

            let size = source_meta.len();
            if size == 0 {
                return Err(AppError::Repository(format!(
                    "import source is zero bytes: {}",
                    source.display()
                )));
            }

            // Create destination parent directories
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    AppError::Repository(format!(
                        "failed to create destination directory {}: {}",
                        parent.display(),
                        e
                    ))
                })?;
            }

            // Attempt hard link first
            match std::fs::hard_link(&source, &dest) {
                Ok(()) => {
                    // Verify destination exists and size matches
                    match std::fs::metadata(&dest) {
                        Ok(dest_meta) if dest_meta.len() == size => {
                            return Ok(ImportFileResult {
                                strategy: ImportStrategy::HardLink,
                                source_path: source,
                                dest_path: dest,
                                size_bytes: size,
                            });
                        }
                        Ok(dest_meta) => {
                            let _ = std::fs::remove_file(&dest);
                            tracing::warn!(
                                "hard link size mismatch: source={} dest={}, falling back to copy",
                                size,
                                dest_meta.len()
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "hard link created but dest stat failed: {}, falling back to copy",
                                e
                            );
                        }
                    }
                }
                Err(e) if is_cross_device_error(&e) => {
                    tracing::info!(
                        "hard link failed (cross-device), falling back to copy: {} -> {}",
                        source.display(),
                        dest.display()
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "hard link failed: {}, falling back to copy: {} -> {}",
                        e,
                        source.display(),
                        dest.display()
                    );
                }
            }

            // Copy fallback: copy to temp file, fsync, rename atomically
            let temp_dest = dest.with_extension("tmp_import");

            let copy_result = (|| -> Result<(), std::io::Error> {
                // Copy to temp file
                std::fs::copy(&source, &temp_dest)?;

                // Fsync the temp file
                let file = std::fs::File::open(&temp_dest)?;
                file.sync_all()?;
                drop(file);

                // Atomic rename (same filesystem)
                std::fs::rename(&temp_dest, &dest)?;

                Ok(())
            })();

            match copy_result {
                Ok(()) => {
                    // Verify destination size matches
                    let dest_meta = std::fs::metadata(&dest).map_err(|e| {
                        AppError::Repository(format!("copy succeeded but dest stat failed: {}", e))
                    })?;

                    if dest_meta.len() != size {
                        let _ = std::fs::remove_file(&dest);
                        return Err(AppError::Repository(format!(
                            "copy size mismatch: source={} dest={}",
                            size,
                            dest_meta.len()
                        )));
                    }

                    Ok(ImportFileResult {
                        strategy: ImportStrategy::Copy,
                        source_path: source,
                        dest_path: dest,
                        size_bytes: size,
                    })
                }
                Err(e) => {
                    // Clean up partial temp file
                    let _ = std::fs::remove_file(&temp_dest);
                    Err(AppError::Repository(format!(
                        "import copy failed: {} -> {}: {}",
                        source.display(),
                        dest.display(),
                        e
                    )))
                }
            }
        })
        .await
        .map_err(|e| AppError::Repository(format!("import task panicked: {}", e)))?
    }
}
