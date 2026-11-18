//! The platform's free-space call, and the figures it answers with.
//!
//! This is the only `statfs` / `statvfs` / `GetDiskFreeSpaceExW` for capacity.
//! Nothing here caches, bounds or schedules the call: the capacity owner that
//! does (sampling of library roots, the per-path cache and its staleness
//! rule, single-flight and the timeout) calls [`probe`] and is the one caller.

use std::io;
use std::path::Path;

/// A refreshing read answers from a figure taken this recently instead of
/// calling the disk again, so a burst of callers costs at most one call per
/// path per second. A guard reading a figure this old is no weaker than two
/// guards that each probed at the same moment, which is what a concurrent burst
/// did before.
pub const STORAGE_CAPACITY_REFRESH_REUSE: std::time::Duration = std::time::Duration::from_secs(1);

/// Bound on one background sample of one root.
pub const STORAGE_CAPACITY_SAMPLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Total and available bytes of the filesystem backing a path.
///
/// `available_bytes` is the space an unprivileged writer can use (the
/// `f_bavail` figure on unix, the caller-available figure on Windows), so
/// `total - available` counts reserved blocks as used, matching what an
/// operator expects a usage percentage to mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilesystemSpace {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// One capacity call's answer.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapacityProbeReading {
    pub space: FilesystemSpace,
    /// The filesystem's id as the call reported it, when the platform gives
    /// one and it is not zero. Roots whose last calls agree on it and on the
    /// total are sampled with one call.
    pub filesystem_id: Option<u64>,
}

#[cfg(unix)]
fn c_path(path: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("disk-space path contains an interior NUL: {error}"),
        )
    })
}

/// Widen a `statvfs` counter to `u64`; the field types differ by target.
#[cfg(all(unix, not(target_os = "macos")))]
fn widen<T: Into<u64>>(value: T) -> u64 {
    value.into()
}

fn nonzero(id: u64) -> Option<u64> {
    (id != 0).then_some(id)
}

/// `statfs`, not `statvfs`: Darwin's `statvfs` carries 32-bit block
/// counts, so a volume larger than `2^32 * f_frsize` bytes wraps (a 23 TB
/// SMB share reads back as about 1 TB). Darwin's `statfs` carries 64-bit
/// counts in `f_bsize` units.
#[cfg(target_os = "macos")]
#[doc(hidden)]
pub fn probe(path: &Path) -> io::Result<CapacityProbeReading> {
    let c_path = c_path(path)?;
    // SAFETY: `statfs` fills a zeroed struct we own for a valid,
    // NUL-terminated path that outlives the call.
    let buf = unsafe {
        let mut buf: libc::statfs = std::mem::zeroed();
        if libc::statfs(c_path.as_ptr(), &mut buf) != 0 {
            return Err(io::Error::last_os_error());
        }
        buf
    };
    // SAFETY: `fsid_t` is two initialized 32-bit integers.
    let fsid = unsafe {
        (&buf.f_fsid as *const libc::fsid_t)
            .cast::<[i32; 2]>()
            .read_unaligned()
    };
    let bsize = u64::from(buf.f_bsize);
    Ok(CapacityProbeReading {
        space: FilesystemSpace {
            total_bytes: buf.f_blocks.saturating_mul(bsize),
            available_bytes: buf.f_bavail.saturating_mul(bsize),
        },
        filesystem_id: nonzero((u64::from(fsid[0] as u32) << 32) | u64::from(fsid[1] as u32)),
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
#[doc(hidden)]
pub fn probe(path: &Path) -> io::Result<CapacityProbeReading> {
    let c_path = c_path(path)?;
    // SAFETY: `statvfs` fills a zeroed struct we own for a valid,
    // NUL-terminated path that outlives the call.
    let buf = unsafe {
        let mut buf: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut buf) != 0 {
            return Err(io::Error::last_os_error());
        }
        buf
    };
    // Some FUSE filesystems report only `f_bsize`; POSIX says counts are
    // in `f_frsize` units, so prefer it but fall back when it is zero.
    let mut frsize = widen(buf.f_frsize);
    if frsize == 0 {
        frsize = widen(buf.f_bsize);
    }
    #[allow(clippy::useless_conversion)]
    let fsid = u64::from(buf.f_fsid);
    Ok(CapacityProbeReading {
        space: FilesystemSpace {
            total_bytes: widen(buf.f_blocks).saturating_mul(frsize),
            available_bytes: widen(buf.f_bavail).saturating_mul(frsize),
        },
        filesystem_id: nonzero(fsid),
    })
}

#[cfg(windows)]
#[doc(hidden)]
pub fn probe(path: &Path) -> io::Result<CapacityProbeReading> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let mut available = 0_u64;
    let mut total = 0_u64;
    // SAFETY: the path is NUL-terminated and outlives the call; the out
    // pointers are valid for writes.
    let result = unsafe {
        GetDiskFreeSpaceExW(
            wide_path.as_ptr(),
            &mut available,
            &mut total,
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    let _ = nonzero;
    Ok(CapacityProbeReading {
        space: FilesystemSpace {
            total_bytes: total,
            available_bytes: available,
        },
        filesystem_id: None,
    })
}

#[cfg(not(any(unix, windows)))]
#[doc(hidden)]
pub fn probe(_path: &Path) -> io::Result<CapacityProbeReading> {
    let _ = nonzero;
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "disk-space queries are unsupported on this platform",
    ))
}
