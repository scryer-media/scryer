//! What the host reports about memory: the machine's total, and the most this
//! process may use once a container memory limit is taken into account.

#[cfg(target_os = "linux")]
use crate::disk::{Disk, PathReading};
#[cfg(target_os = "linux")]
use std::path::Path;

/// The machine's physical memory, or `None` where it cannot be read.
#[cfg(target_os = "linux")]
pub fn total_memory_bytes() -> Option<u64> {
    let text = Disk::process()
        .read_to_string_blocking(Path::new("/proc/meminfo"))
        .ok()?;
    let kib = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(kib * 1024)
}

/// The machine's physical memory, or `None` where it cannot be read.
#[cfg(target_os = "macos")]
pub fn total_memory_bytes() -> Option<u64> {
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    // SAFETY: `hw.memsize` is a u64 and `value` is a writable u64.
    let result = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0).then_some(value)
}

/// The machine's physical memory, or `None` where it cannot be read.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn total_memory_bytes() -> Option<u64> {
    None
}

/// The most memory this process may use: the machine's total, lowered to the
/// container's cgroup memory limit when one is set. `None` when neither can be
/// read. Each call reads the host again, so callers read it once and keep it.
pub fn memory_available_to_process_bytes() -> Option<u64> {
    lower_of(total_memory_bytes(), cgroup_memory_limit_bytes())
}

fn lower_of(total: Option<u64>, limit: Option<u64>) -> Option<u64> {
    match (total, limit) {
        (Some(total), Some(limit)) => Some(total.min(limit)),
        (total, limit) => total.or(limit),
    }
}

#[cfg(target_os = "linux")]
fn cgroup_memory_limit_bytes() -> Option<u64> {
    // cgroup v2 first, then the v1 memory controller.
    [
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/memory/memory.limit_in_bytes",
    ]
    .into_iter()
    .find_map(|path| {
        Disk::process()
            .read_to_string_blocking(Path::new(path))
            .ok()
            .and_then(|text| parse_cgroup_memory_limit(&text))
    })
}

#[cfg(not(target_os = "linux"))]
fn cgroup_memory_limit_bytes() -> Option<u64> {
    None
}

/// A cgroup limit file holds a byte count, or `max` when there is no limit.
/// cgroup v1 reports "no limit" as a number near `i64::MAX`, which the
/// comparison with the machine's total discards on its own.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_cgroup_memory_limit(text: &str) -> Option<u64> {
    text.trim().parse::<u64>().ok().filter(|bytes| *bytes > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_container_limit_lowers_the_machine_total() {
        assert_eq!(lower_of(Some(16 << 30), Some(2 << 30)), Some(2 << 30));
        assert_eq!(lower_of(Some(4 << 30), Some(u64::MAX >> 1)), Some(4 << 30));
        assert_eq!(lower_of(None, Some(2 << 30)), Some(2 << 30));
        assert_eq!(lower_of(Some(8 << 30), None), Some(8 << 30));
        assert_eq!(lower_of(None, None), None);
    }

    #[test]
    fn cgroup_limit_files_parse_bytes_and_ignore_max() {
        assert_eq!(
            parse_cgroup_memory_limit("2147483648\n"),
            Some(2_147_483_648)
        );
        assert_eq!(parse_cgroup_memory_limit("max\n"), None);
        assert_eq!(parse_cgroup_memory_limit("0\n"), None);
        assert_eq!(parse_cgroup_memory_limit(""), None);
    }
}
