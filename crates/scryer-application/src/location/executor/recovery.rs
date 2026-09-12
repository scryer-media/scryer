use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How often a waiting transfer probes its disconnected storage.
///
/// A brief hiccup (a switch reboot, an SMB session renegotiating) should
/// resume within seconds, so the first minute probes every five seconds. A
/// long outage should not keep hammering the mount, so the cadence then
/// doubles until it settles at one probe per minute.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct RecoverySchedule {
    probes: u32,
}

impl RecoverySchedule {
    pub(super) const QUICK: Duration = Duration::from_secs(5);
    const QUICK_PROBES: u32 = 12;
    pub(super) const CAP: Duration = Duration::from_secs(60);

    /// The delay to sleep before the next probe, advancing the schedule.
    pub(super) fn next_delay(&mut self) -> Duration {
        let delay = self.current_delay();
        self.probes = self.probes.saturating_add(1);
        delay
    }

    /// The delay the next call to [`Self::next_delay`] will return.
    pub(super) fn current_delay(&self) -> Duration {
        if self.probes < Self::QUICK_PROBES {
            return Self::QUICK;
        }
        let doublings = (self.probes - Self::QUICK_PROBES).saturating_add(1);
        Self::QUICK
            .checked_mul(1u32 << doublings.min(8))
            .unwrap_or(Self::CAP)
            .min(Self::CAP)
    }
}

/// Only read-only probes run on this timer. Cancellation remains responsive
/// without starting another transfer or abandoning in-flight writes.
///
/// `on_wait` is told the delay before each probe so the caller can report an
/// honest cadence to the user.
pub(super) async fn wait_for_reconnection<A, AF, C, CF, W>(
    mut available: A,
    mut canceled: C,
    mut on_wait: W,
) -> crate::AppResult<bool>
where
    A: FnMut() -> AF,
    AF: std::future::Future<Output = bool>,
    C: FnMut() -> CF,
    CF: std::future::Future<Output = crate::AppResult<bool>>,
    W: FnMut(Duration),
{
    let mut schedule = RecoverySchedule::default();
    loop {
        let interval = schedule.next_delay();
        on_wait(interval);
        let delay = tokio::time::sleep(interval);
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
pub(super) struct StorageWatch {
    anchors: Vec<(PathBuf, String)>,
    /// A probe that is still blocked inside the kernel (a hung network mount)
    /// must not be joined by another one every few seconds.
    probing: Arc<AtomicBool>,
}

impl StorageWatch {
    /// Longer than this and the mount is hung, which is as unavailable as
    /// missing; the blocked thread finishes on its own later.
    const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

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
            Some(Self {
                anchors,
                probing: Arc::new(AtomicBool::new(false)),
            })
        })
        .await
        .ok()
        .flatten()
    }

    pub async fn available(&self) -> bool {
        if self.probing.swap(true, Ordering::AcqRel) {
            return false;
        }
        let anchors = self.anchors.clone();
        let probing = self.probing.clone();
        let probe = tokio::task::spawn_blocking(move || {
            struct Done(Arc<AtomicBool>);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _done = Done(probing);
            anchors.iter().all(|(path, expected)| {
                volume_identity(path).is_ok_and(|actual| &actual == expected)
                    && std::fs::read_dir(path)
                        .is_ok_and(|mut entries| entries.next().transpose().is_ok())
            })
        });
        match tokio::time::timeout(Self::PROBE_TIMEOUT, probe).await {
            Ok(joined) => joined.unwrap_or(false),
            Err(_elapsed) => false,
        }
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
    // `MetadataExt::volume_serial_number` is still unstable (`windows_by_handle`);
    // the transfer module already reads the same serial through a stable call.
    crate::location::transfer::windows_file_identity(path)
        .map(|(volume_serial, _file_index)| volume_serial.to_string())
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

    #[test]
    fn recovery_schedule_probes_quickly_then_settles_at_one_minute() {
        let mut schedule = RecoverySchedule::default();
        let delays: Vec<u64> = (0..18).map(|_| schedule.next_delay().as_secs()).collect();
        assert_eq!(
            delays,
            [5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 10, 20, 40, 60, 60, 60]
        );
        assert_eq!(schedule.current_delay(), RecoverySchedule::CAP);
    }

    #[tokio::test(start_paused = true)]
    async fn storage_recovery_polls_quickly_then_backs_off_until_reconnected() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let probes = Arc::new(AtomicUsize::new(0));
        let count = probes.clone();
        let started = tokio::time::Instant::now();
        let mut reported = Vec::new();
        let recovered = wait_for_reconnection(
            move || std::future::ready(count.fetch_add(1, Ordering::SeqCst) == 15),
            || std::future::ready(Ok(false)),
            |delay| reported.push(delay.as_secs()),
        )
        .await
        .unwrap();
        assert!(recovered);
        assert_eq!(probes.load(Ordering::SeqCst), 16);
        // Twelve five-second probes, then 10s, 20s, 40s, and the one-minute cap.
        assert_eq!(
            started.elapsed(),
            Duration::from_secs(60 + 10 + 20 + 40 + 60)
        );
        assert_eq!(reported.len(), 16);
        assert_eq!(reported[0], 5);
        assert_eq!(reported[15], 60);
    }

    #[tokio::test(start_paused = true)]
    async fn storage_recovery_first_probe_lands_after_five_seconds() {
        let started = tokio::time::Instant::now();
        let recovered = wait_for_reconnection(
            || std::future::ready(true),
            || std::future::ready(Ok(false)),
            |_| {},
        )
        .await
        .unwrap();
        assert!(recovered);
        assert_eq!(started.elapsed(), Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn storage_recovery_cancels_before_next_mount_probe() {
        let started = tokio::time::Instant::now();
        let recovered = wait_for_reconnection(
            || async { panic!("must not probe before five seconds") },
            || std::future::ready(Ok(started.elapsed() >= Duration::from_secs(3))),
            |_| {},
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
                |_| {},
            )
            .await
            .unwrap()
        );
    }

    #[tokio::test]
    async fn storage_watch_reports_unavailable_while_a_probe_is_still_in_flight() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        let watch = StorageWatch::capture(&source.join("file"), &destination.join("file"))
            .await
            .unwrap();
        watch.probing.store(true, Ordering::Release);
        assert!(!watch.available().await);
        watch.probing.store(false, Ordering::Release);
        assert!(watch.available().await);
        assert!(!watch.probing.load(Ordering::Acquire));
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
        wrong_volume.anchors[1].1 = "another volume".into();
        assert!(!wrong_volume.available().await);
    }
}
