use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use filetime::{FileTime, set_file_mtime};
use scryer_application::{AppError, AppResult, PendingStagedNzb, StagedNzbRef, StagedNzbStore};

#[derive(Clone)]
pub struct FileSystemStagedNzbStore {
    root_dir: PathBuf,
    active_artifacts: Arc<Mutex<HashMap<PathBuf, usize>>>,
}

impl FileSystemStagedNzbStore {
    pub async fn new(path: impl AsRef<Path>) -> AppResult<Self> {
        Self::new_with_startup_purge(path, false).await
    }

    pub async fn new_with_startup_purge(
        path: impl AsRef<Path>,
        purge_existing: bool,
    ) -> AppResult<Self> {
        let root_dir = path.as_ref().to_path_buf();
        if purge_existing {
            Self::purge_cache_dir(&root_dir)?;
        }
        std::fs::create_dir_all(&root_dir).map_err(|error| {
            AppError::Repository(format!(
                "failed to create staged nzb directory {}: {error}",
                root_dir.display()
            ))
        })?;
        Ok(Self {
            root_dir,
            active_artifacts: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn path_for_main_db(db_path: &str) -> PathBuf {
        let raw = db_path.strip_prefix("sqlite://").unwrap_or(db_path);
        let raw = raw.split('?').next().unwrap_or(raw).trim();
        let db_file = Path::new(raw);
        db_file
            .parent()
            .unwrap_or(Path::new("."))
            .join("cache")
            .join("nzb")
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    pub async fn count_staged_artifacts(&self) -> AppResult<u64> {
        let mut count = 0u64;
        let entries = std::fs::read_dir(&self.root_dir).map_err(|error| {
            AppError::Repository(format!(
                "failed to list staged nzb directory {}: {error}",
                self.root_dir.display()
            ))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                AppError::Repository(format!("failed to read staged nzb entry: {error}"))
            })?;
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "zst") {
                count += 1;
            }
        }
        Ok(count)
    }

    pub async fn stage_nzb_bytes_for_test(&self, nzb_bytes: &[u8]) -> AppResult<StagedNzbRef> {
        let pending = self
            .create_pending_staged_nzb("https://example.invalid/test.nzb", Some("test-title"))
            .await?;
        let compressed = zstd::bulk::compress(nzb_bytes, 3).map_err(|error| {
            AppError::Repository(format!(
                "failed to zstd-compress staged nzb fixture: {error}"
            ))
        })?;
        std::fs::write(&pending.partial_path, compressed).map_err(|error| {
            AppError::Repository(format!(
                "failed to write staged nzb fixture {}: {error}",
                pending.partial_path.display()
            ))
        })?;
        self.finalize_pending_staged_nzb(pending, nzb_bytes.len() as u64)
            .await
    }

    pub async fn set_staged_nzb_updated_at(
        &self,
        staged_nzb: &StagedNzbRef,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        set_file_mtime(
            &staged_nzb.compressed_path,
            FileTime::from_unix_time(updated_at.timestamp(), 0),
        )
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to set staged nzb mtime {}: {error}",
                staged_nzb.compressed_path.display()
            ))
        })?;
        Ok(())
    }

    fn purge_cache_dir(path: &Path) -> AppResult<()> {
        match std::fs::remove_dir_all(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "failed to purge staged nzb directory {}: {error}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    fn build_pending_paths(&self, id: &str) -> PendingStagedNzb {
        let compressed_path = self.root_dir.join(format!("{id}.nzb.zst"));
        let partial_path = self.root_dir.join(format!("{id}.nzb.zst.part"));
        PendingStagedNzb {
            id: id.to_string(),
            compressed_path,
            partial_path,
        }
    }

    fn with_active_artifacts<T>(
        &self,
        mut f: impl FnMut(&mut HashMap<PathBuf, usize>) -> T,
    ) -> AppResult<T> {
        let mut active_artifacts = self.active_artifacts.lock().map_err(|_| {
            AppError::Repository("staged nzb active-artifact registry was poisoned".to_string())
        })?;
        Ok(f(&mut active_artifacts))
    }
}

#[async_trait]
impl StagedNzbStore for FileSystemStagedNzbStore {
    async fn create_pending_staged_nzb(
        &self,
        _source_url: &str,
        _title_id: Option<&str>,
    ) -> AppResult<PendingStagedNzb> {
        std::fs::create_dir_all(&self.root_dir).map_err(|error| {
            AppError::Repository(format!(
                "failed to create staged nzb directory {}: {error}",
                self.root_dir.display()
            ))
        })?;
        Ok(self.build_pending_paths(&scryer_domain::Id::new().0))
    }

    async fn finalize_pending_staged_nzb(
        &self,
        pending: PendingStagedNzb,
        raw_size_bytes: u64,
    ) -> AppResult<StagedNzbRef> {
        std::fs::rename(&pending.partial_path, &pending.compressed_path).map_err(|error| {
            AppError::Repository(format!(
                "failed to finalize staged nzb {}: {error}",
                pending.compressed_path.display()
            ))
        })?;
        Ok(StagedNzbRef {
            id: pending.id,
            compressed_path: pending.compressed_path,
            raw_size_bytes,
        })
    }

    async fn delete_staged_nzb(&self, staged_nzb: &StagedNzbRef) -> AppResult<bool> {
        match std::fs::remove_file(&staged_nzb.compressed_path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(AppError::Repository(format!(
                "failed to delete staged nzb {}: {error}",
                staged_nzb.compressed_path.display()
            ))),
        }
    }

    async fn prune_staged_nzbs_older_than(&self, older_than: DateTime<Utc>) -> AppResult<u32> {
        let mut pruned = 0u32;
        let entries = match std::fs::read_dir(&self.root_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "failed to list staged nzb directory {}: {error}",
                    self.root_dir.display()
                )));
            }
        };

        for entry in entries {
            let entry = entry.map_err(|error| {
                AppError::Repository(format!("failed to read staged nzb entry: {error}"))
            })?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if self.with_active_artifacts(|active| active.contains_key(&path))? {
                continue;
            }

            let metadata = entry.metadata().map_err(|error| {
                AppError::Repository(format!(
                    "failed to read staged nzb metadata {}: {error}",
                    path.display()
                ))
            })?;
            let modified = metadata.modified().map_err(|error| {
                AppError::Repository(format!(
                    "failed to read staged nzb mtime {}: {error}",
                    path.display()
                ))
            })?;
            let modified = DateTime::<Utc>::from(modified);
            if modified < older_than {
                std::fs::remove_file(&path).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to prune staged nzb artifact {}: {error}",
                        path.display()
                    ))
                })?;
                pruned += 1;
            }
        }

        Ok(pruned)
    }

    fn mark_artifact_active(&self, path: &Path) -> AppResult<()> {
        self.with_active_artifacts(|active| {
            *active.entry(path.to_path_buf()).or_insert(0) += 1;
        })
    }

    fn mark_artifact_inactive(&self, path: &Path) -> AppResult<()> {
        self.with_active_artifacts(|active| {
            if let Some(count) = active.get_mut(path) {
                if *count > 1 {
                    *count -= 1;
                } else {
                    active.remove(path);
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use tempfile::TempDir;

    use super::*;

    fn temp_store_dir() -> TempDir {
        TempDir::new().expect("temp dir")
    }

    #[tokio::test]
    async fn creates_fresh_staging_directory() {
        let dir = temp_store_dir();
        let store = FileSystemStagedNzbStore::new(dir.path())
            .await
            .expect("store should initialize");

        assert_eq!(
            store
                .count_staged_artifacts()
                .await
                .expect("count should succeed"),
            0
        );
    }

    #[tokio::test]
    async fn startup_purge_removes_existing_artifacts() {
        let dir = temp_store_dir();
        let store = FileSystemStagedNzbStore::new(dir.path())
            .await
            .expect("store should initialize");
        let _artifact = store
            .stage_nzb_bytes_for_test(b"<?xml version=\"1.0\"?><nzb></nzb>")
            .await
            .expect("artifact should stage");
        assert_eq!(store.count_staged_artifacts().await.expect("count"), 1);

        let purged = FileSystemStagedNzbStore::new_with_startup_purge(dir.path(), true)
            .await
            .expect("startup purge should succeed");
        assert_eq!(purged.count_staged_artifacts().await.expect("count"), 0);
    }

    #[tokio::test]
    async fn prune_removes_only_old_artifacts() {
        let dir = temp_store_dir();
        let store = FileSystemStagedNzbStore::new(dir.path())
            .await
            .expect("store should initialize");
        let old_artifact = store
            .stage_nzb_bytes_for_test(b"<?xml version=\"1.0\"?><nzb></nzb>")
            .await
            .expect("old artifact should stage");
        let _fresh_artifact = store
            .stage_nzb_bytes_for_test(b"<?xml version=\"1.0\"?><nzb></nzb>")
            .await
            .expect("fresh artifact should stage");
        store
            .set_staged_nzb_updated_at(&old_artifact, Utc::now() - Duration::hours(2))
            .await
            .expect("mtime update should succeed");

        let pruned = store
            .prune_staged_nzbs_older_than(Utc::now() - Duration::hours(1))
            .await
            .expect("prune should succeed");

        assert_eq!(pruned, 1);
        assert_eq!(store.count_staged_artifacts().await.expect("count"), 1);
    }

    #[tokio::test]
    async fn prune_skips_active_artifacts() {
        let dir = temp_store_dir();
        let store = FileSystemStagedNzbStore::new(dir.path())
            .await
            .expect("store should initialize");
        let artifact = store
            .stage_nzb_bytes_for_test(b"<?xml version=\"1.0\"?><nzb></nzb>")
            .await
            .expect("artifact should stage");
        store
            .set_staged_nzb_updated_at(&artifact, Utc::now() - Duration::hours(2))
            .await
            .expect("mtime update should succeed");
        store
            .mark_artifact_active(&artifact.compressed_path)
            .expect("active mark should succeed");

        let pruned = store
            .prune_staged_nzbs_older_than(Utc::now() - Duration::hours(1))
            .await
            .expect("prune should succeed");

        assert_eq!(pruned, 0);
        assert_eq!(store.count_staged_artifacts().await.expect("count"), 1);

        store
            .mark_artifact_inactive(&artifact.compressed_path)
            .expect("inactive mark should succeed");
    }
}
