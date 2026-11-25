use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::disk::{
    Access, Disk, DiskError, EntryKind, IfPresent, Listed, Parents, PathReading, Removal, Remove,
    Replace,
};
use scryer_application::{AppError, AppResult, PendingStagedNzb, StagedNzbRef, StagedNzbStore};

/// A failure to `what` at `path`, in the platform's own words.
fn failed(what: &str, path: &Path, error: DiskError) -> AppError {
    let (path, words) = (path.display(), error.words());
    AppError::Repository(format!("failed to {what} {path}: {words}"))
}

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
            Self::purge_cache_dir(&root_dir).await?;
        }
        let store = Self {
            root_dir,
            active_artifacts: Arc::new(Mutex::new(HashMap::new())),
        };
        store.create_root_dir().await?;
        Ok(store)
    }

    async fn create_root_dir(&self) -> AppResult<()> {
        Disk::process()
            .create_dir(&self.root_dir, Parents::Create)
            .await
            .map_err(|error| failed("create staged nzb directory", &self.root_dir, error))
    }

    fn list_failed(&self, error: DiskError) -> AppError {
        failed("list staged nzb directory", &self.root_dir, error)
    }

    /// Runs one pass over the directory in a single blocking task.
    async fn blocking_pass<T: Send + 'static>(
        &self,
        pass: impl FnOnce(&Self) -> AppResult<T> + Send + 'static,
    ) -> AppResult<T> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || pass(&store))
            .await
            .map_err(|error| {
                AppError::Repository(format!("staged nzb listing task failed: {error}"))
            })?
    }

    /// The entries a prune with `older_than` would remove, each judged only
    /// when it is reached. A missing directory has none.
    fn prunable(
        &self,
        older_than: DateTime<Utc>,
    ) -> AppResult<impl Iterator<Item = AppResult<PathBuf>> + '_> {
        let entries = Disk::process()
            .list_blocking(&self.root_dir)
            .if_present()
            .map_err(|error| self.list_failed(error))?;
        Ok(entries
            .into_iter()
            .flatten()
            .filter_map(move |entry| self.prunable_entry(entry, older_than).transpose()))
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
        self.blocking_pass(|store| {
            let disk = Disk::process();
            let entries = disk
                .list_blocking(&store.root_dir)
                .map_err(|error| store.list_failed(error))?;
            // A file counts as the listing says; a link counts when it leads
            // to a file.
            let is_file = |entry: &Listed| match entry.reading.kind() {
                Some(EntryKind::File) => true,
                Some(EntryKind::Directory | EntryKind::Other) => false,
                Some(EntryKind::Symlink) | None => {
                    disk.follow_blocking(&entry.path).kind() == Some(EntryKind::File)
                }
            };
            Ok(entries
                .iter()
                .filter(|entry| entry.path.extension().is_some_and(|ext| ext == "zst"))
                .filter(|entry| is_file(entry))
                .count() as u64)
        })
        .await
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
        Disk::process()
            .write(
                &pending.partial_path,
                compressed,
                Replace::Allowed,
                Access::Default,
            )
            .await
            .map_err(|error| failed("write staged nzb fixture", &pending.partial_path, error))?;
        self.finalize_pending_staged_nzb(pending, nzb_bytes.len() as u64)
            .await
    }

    pub async fn set_staged_nzb_updated_at(
        &self,
        staged_nzb: &StagedNzbRef,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        // Whole seconds, as the modification time has always been set.
        let at = DateTime::<Utc>::from_timestamp(updated_at.timestamp(), 0).unwrap_or(updated_at);
        Disk::process()
            .set_modified(&staged_nzb.compressed_path, at.into())
            .await
            .map_err(|error| failed("set staged nzb mtime", &staged_nzb.compressed_path, error))
    }

    /// A directory already gone is purged.
    async fn purge_cache_dir(path: &Path) -> AppResult<()> {
        Disk::process()
            .remove(path, Remove::Tree)
            .await
            .map(|_| ())
            .map_err(|error| failed("purge staged nzb directory", path, error))
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

    /// The path of one directory entry when a prune with `older_than` would
    /// remove it: a file, not in use, last written before the cutoff.
    ///
    /// Decided from fresh reads of the entry, never from the listing: a link
    /// counts as a file when it leads to one, and its age is the link's own.
    fn prunable_entry(
        &self,
        entry: Listed,
        older_than: DateTime<Utc>,
    ) -> AppResult<Option<PathBuf>> {
        let disk = Disk::process();
        let path = entry.path;
        let reading = disk.uncached_blocking(&path);
        let is_file = match reading.kind() {
            Some(EntryKind::File) => true,
            Some(EntryKind::Symlink) => disk.follow_blocking(&path).kind() == Some(EntryKind::File),
            _ => false,
        };
        if !is_file {
            return Ok(None);
        }
        if self.with_active_artifacts(|active| active.contains_key(&path))? {
            return Ok(None);
        }

        let PathReading::Known {
            modified: Some(modified),
            ..
        } = reading
        else {
            return Err(AppError::Repository(format!(
                "failed to read staged nzb mtime {}: the platform did not report it",
                path.display()
            )));
        };
        let modified = DateTime::<Utc>::from(modified);
        Ok((modified < older_than).then_some(path))
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
        self.create_root_dir().await?;
        Ok(self.build_pending_paths(&scryer_domain::Id::new().0))
    }

    async fn finalize_pending_staged_nzb(
        &self,
        pending: PendingStagedNzb,
        raw_size_bytes: u64,
    ) -> AppResult<StagedNzbRef> {
        // The destination is named by a new id, so nothing is there to keep.
        Disk::process()
            .move_exclusive(&pending.partial_path, &pending.compressed_path)
            .await
            .map_err(|error| {
                // The disk service removes the empty name it reserved when it
                // can tell it is its own; anything it could not is kept and
                // named here.
                if let Some(left) = error.left() {
                    tracing::warn!(
                        left = %left.at().display(),
                        error = %error.words(),
                        "a staged nzb whose rename failed left a file in place"
                    );
                }
                failed("finalize staged nzb", &pending.compressed_path, error)
            })?;
        Ok(StagedNzbRef {
            password_candidates: Default::default(),
            id: pending.id,
            compressed_path: pending.compressed_path,
            raw_size_bytes,
        })
    }

    async fn delete_staged_nzb(&self, staged_nzb: &StagedNzbRef) -> AppResult<bool> {
        Disk::process()
            .remove(&staged_nzb.compressed_path, Remove::File)
            .await
            .map(|removal| removal == Removal::Removed)
            .map_err(|error| failed("delete staged nzb", &staged_nzb.compressed_path, error))
    }

    async fn prune_staged_nzbs_older_than(&self, older_than: DateTime<Utc>) -> AppResult<u32> {
        self.blocking_pass(move |store| {
            let mut pruned = 0u32;
            for path in store.prunable(older_than)? {
                let path = path?;
                let removal = Disk::process()
                    .remove_blocking(&path, Remove::File)
                    .map_err(|error| failed("prune staged nzb artifact", &path, error))?;
                // One removed by someone else since it was judged is not counted.
                pruned += u32::from(removal == Removal::Removed);
            }
            Ok(pruned)
        })
        .await
    }

    async fn has_staged_nzbs_older_than(&self, older_than: DateTime<Utc>) -> AppResult<bool> {
        self.blocking_pass(move |store| {
            Ok(store.prunable(older_than)?.next().transpose()?.is_some())
        })
        .await
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
    async fn has_old_artifacts_answers_what_prune_would_remove() {
        let dir = temp_store_dir();
        let store = FileSystemStagedNzbStore::new(dir.path())
            .await
            .expect("store should initialize");
        let cutoff = || Utc::now() - Duration::hours(1);
        assert!(!store.has_staged_nzbs_older_than(cutoff()).await.unwrap());

        let _fresh = store
            .stage_nzb_bytes_for_test(b"<?xml version=\"1.0\"?><nzb></nzb>")
            .await
            .expect("fresh artifact should stage");
        assert!(!store.has_staged_nzbs_older_than(cutoff()).await.unwrap());

        let old = store
            .stage_nzb_bytes_for_test(b"<?xml version=\"1.0\"?><nzb></nzb>")
            .await
            .expect("old artifact should stage");
        store
            .set_staged_nzb_updated_at(&old, Utc::now() - Duration::hours(2))
            .await
            .expect("mtime update should succeed");
        store
            .mark_artifact_active(&old.compressed_path)
            .expect("active mark should succeed");
        assert!(
            !store.has_staged_nzbs_older_than(cutoff()).await.unwrap(),
            "an artifact in use is never pruned"
        );
        assert_eq!(
            store.prune_staged_nzbs_older_than(cutoff()).await.unwrap(),
            0
        );

        store
            .mark_artifact_inactive(&old.compressed_path)
            .expect("inactive mark should succeed");
        assert!(store.has_staged_nzbs_older_than(cutoff()).await.unwrap());
        assert_eq!(
            store.prune_staged_nzbs_older_than(cutoff()).await.unwrap(),
            1
        );
        assert!(!store.has_staged_nzbs_older_than(cutoff()).await.unwrap());
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
