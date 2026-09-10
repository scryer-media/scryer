//! Process-local copy admission. Verification never holds a copy permit.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock, Weak};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub struct CopyCoordinator {
    volumes: Arc<Mutex<HashMap<String, Weak<Semaphore>>>>,
    capacity: usize,
}

impl CopyCoordinator {
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn new(capacity: usize) -> Self {
        Self {
            volumes: Arc::new(Mutex::new(HashMap::new())),
            capacity: capacity.clamp(1, 8),
        }
    }

    /// One budget for the single-node process, shared by import and location
    /// adapters. Only admission is transient; recovery never depends on it.
    pub fn shared() -> Self {
        static SHARED: OnceLock<CopyCoordinator> = OnceLock::new();
        SHARED
            .get_or_init(|| {
                let value = std::env::var("SCRYER_FILE_TRANSFER_WORKERS_PER_VOLUME")
                    .or_else(|_| std::env::var("SCRYER_IMPORT_COPY_WORKERS_PER_VOLUME"))
                    .ok();
                Self::new(worker_limit(value.as_deref()))
            })
            .clone()
    }

    pub async fn acquire(&self, key: &str, _lane: &'static str) -> OwnedSemaphorePermit {
        let semaphore = {
            let mut volumes = self.volumes.lock().await;
            volumes.retain(|_, value| value.strong_count() > 0);
            if let Some(semaphore) = volumes.get(key).and_then(Weak::upgrade) {
                semaphore
            } else {
                let semaphore = Arc::new(Semaphore::new(self.capacity));
                volumes.insert(key.to_owned(), Arc::downgrade(&semaphore));
                semaphore
            }
        };
        semaphore
            .acquire_owned()
            .await
            .expect("copy coordinator is never closed")
    }

    pub async fn acquire_destination(&self, path: &Path) -> OwnedSemaphorePermit {
        self.acquire(&destination_volume_key(path), "copy").await
    }
}

fn worker_limit(value: Option<&str>) -> usize {
    value
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(2)
        .min(8)
}

/// Match import's volume identity. File safety is checked by the placement
/// adapter; an unavailable identity conservatively shares the unknown bucket.
pub fn destination_volume_key(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        for ancestor in path.ancestors() {
            if let Ok(metadata) = std::fs::metadata(ancestor) {
                return format!("unix-dev:{}", metadata.dev());
            }
        }
    }
    #[cfg(windows)]
    {
        for ancestor in path.ancestors() {
            if let Some((volume, _)) = windows_file_identity(ancestor) {
                return format!("windows-volume:{volume}");
            }
        }
    }
    "unknown-volume".to_owned()
}

#[cfg(windows)]
pub fn windows_file_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        GetFileInformationByHandle,
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .access_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // The metadata handle stays alive throughout the OS call; no content read.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) } == 0 {
        return None;
    }
    let information = unsafe { information.assume_init() };
    Some((
        u64::from(information.dwVolumeSerialNumber),
        u64::from(information.nFileIndexHigh) << 32 | u64::from(information.nFileIndexLow),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn copies_share_volume_budget_and_release_before_verification() {
        let coordinator = CopyCoordinator::new(2);
        let a = coordinator.acquire("a", "copy").await;
        let b = coordinator.acquire("a", "copy").await;
        let waiting = coordinator.acquire("a", "copy");
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut waiting)
                .await
                .is_err()
        );
        let other = coordinator.acquire("b", "copy").await;
        drop(a);
        let next = waiting.await;
        drop((b, next, other));
        // CRC work owns no permit, regardless of how many files verify.
        let verifying = (0..10)
            .map(|_| tokio::spawn(async { tokio::task::yield_now().await }))
            .collect::<Vec<_>>();
        let _copy = coordinator.acquire("a", "copy").await;
        for task in verifying {
            task.await.unwrap();
        }
    }

    #[test]
    fn copy_worker_setting_is_bounded() {
        assert_eq!(worker_limit(None), 2);
        assert_eq!(worker_limit(Some("0")), 2);
        assert_eq!(worker_limit(Some("bad")), 2);
        assert_eq!(worker_limit(Some("1")), 1);
        assert_eq!(worker_limit(Some("100")), 8);
    }
}
