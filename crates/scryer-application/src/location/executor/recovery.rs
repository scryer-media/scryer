use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(super) const STORAGE_RECOVERY_INTERVAL: Duration = Duration::from_secs(120);

/// Only read-only probes run on the two-minute timer. Cancellation remains
/// responsive without starting another transfer or abandoning in-flight writes.
pub(super) async fn wait_for_reconnection<A, AF, C, CF>(
    mut available: A,
    mut canceled: C,
) -> crate::AppResult<bool>
where
    A: FnMut() -> AF,
    AF: std::future::Future<Output = bool>,
    C: FnMut() -> CF,
    CF: std::future::Future<Output = crate::AppResult<bool>>,
{
    loop {
        let delay = tokio::time::sleep(STORAGE_RECOVERY_INTERVAL);
        tokio::pin!(delay);
        loop {
            if canceled().await? {
                return Ok(false);
            }
            tokio::select! {
                _ = &mut delay => break,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {},
            }
        }
        if available().await {
            return Ok(!canceled().await?);
        }
    }
}

/// A read-only anchor captured before placement. A disconnected mount must not
/// be replaced by the local filesystem underneath its mount point on retry.
#[derive(Clone)]
pub(super) struct StorageWatch(Vec<(PathBuf, String)>);

impl StorageWatch {
    pub async fn capture(source: &Path, destination: &Path) -> Option<Self> {
        let paths = [source.to_path_buf(), destination.to_path_buf()];
        tokio::task::spawn_blocking(move || {
            let mut anchors = Vec::new();
            for path in paths {
                let mut parent = path.parent()?;
                loop {
                    match std::fs::metadata(parent) {
                        Ok(metadata) if metadata.is_dir() => break,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {
                            parent = parent.parent()?;
                        }
                        _ => return None,
                    }
                }
                anchors.push((parent.to_path_buf(), volume_identity(parent).ok()?));
            }
            Some(Self(anchors))
        })
        .await
        .ok()
        .flatten()
    }

    pub async fn available(&self) -> bool {
        let anchors = self.0.clone();
        tokio::task::spawn_blocking(move || {
            anchors.iter().all(|(path, expected)| {
                volume_identity(path).is_ok_and(|actual| &actual == expected)
                    && std::fs::read_dir(path)
                        .is_ok_and(|mut entries| entries.next().transpose().is_ok())
            })
        })
        .await
        .unwrap_or(false)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn volume_identity(path: &Path) -> io::Result<String> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: path is NUL-terminated and info is writable storage for statfs.
    if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful statfs initialized the structure.
    let info = unsafe { info.assume_init() };
    #[cfg(target_os = "macos")]
    {
        // Mount source/name survive an SMB reconnect; st_dev does not. Include
        // the filesystem type so a bare local mount-point directory never fits.
        Ok(format!(
            "{:?}:{:?}:{:?}",
            info.f_fstypename, info.f_mntfromname, info.f_mntonname
        ))
    }
    #[cfg(target_os = "linux")]
    {
        // libc keeps fsid_t's two integer fields private on some Linux targets.
        // SAFETY: Linux fsid_t consists of two initialized 32-bit integers.
        let fsid = unsafe {
            (&info.f_fsid as *const libc::fsid_t)
                .cast::<[i32; 2]>()
                .read_unaligned()
        };
        Ok(format!("{}:{fsid:?}", info.f_type))
    }
}

#[cfg(windows)]
fn volume_identity(path: &Path) -> io::Result<String> {
    use std::os::windows::fs::MetadataExt;
    let metadata = std::fs::metadata(path)?;
    metadata
        .volume_serial_number()
        .map(|id| id.to_string())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "volume identity unavailable"))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn volume_identity(_path: &Path) -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "volume identity unavailable",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn storage_recovery_polls_every_two_minutes_until_reconnected() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let probes = Arc::new(AtomicUsize::new(0));
        let count = probes.clone();
        let started = tokio::time::Instant::now();
        let recovered = wait_for_reconnection(
            move || std::future::ready(count.fetch_add(1, Ordering::SeqCst) == 2),
            || std::future::ready(Ok(false)),
        )
        .await
        .unwrap();
        assert!(recovered);
        assert_eq!(probes.load(Ordering::SeqCst), 3);
        assert_eq!(started.elapsed(), Duration::from_secs(360));
    }

    #[tokio::test(start_paused = true)]
    async fn storage_recovery_cancels_before_next_mount_probe() {
        let started = tokio::time::Instant::now();
        let recovered = wait_for_reconnection(
            || async { panic!("must not probe before two minutes") },
            || std::future::ready(Ok(started.elapsed() >= Duration::from_secs(3))),
        )
        .await
        .unwrap();
        assert!(!recovered);
        assert_eq!(started.elapsed(), Duration::from_secs(3));
    }

    #[tokio::test(start_paused = true)]
    async fn storage_recovery_checks_cancellation_after_probe_before_retry() {
        use std::cell::Cell;
        let canceled = Cell::new(false);
        assert!(
            !wait_for_reconnection(
                || {
                    canceled.set(true);
                    std::future::ready(true)
                },
                || std::future::ready(Ok(canceled.get())),
            )
            .await
            .unwrap()
        );
    }

    #[tokio::test]
    async fn storage_watch_waits_for_original_directory_to_return() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        let watch = StorageWatch::capture(&source.join("file"), &destination.join("file"))
            .await
            .unwrap();
        assert!(watch.available().await);
        let disconnected = temp.path().join("disconnected");
        std::fs::rename(&destination, &disconnected).unwrap();
        assert!(!watch.available().await);
        std::fs::rename(&disconnected, &destination).unwrap();
        assert!(watch.available().await);
        let mut wrong_volume = watch.clone();
        wrong_volume.0[1].1 = "another volume".into();
        assert!(!wrong_volume.available().await);
    }
}
