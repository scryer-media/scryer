//! Opt-in jemalloc heap profiling for load-test / benchmark images.
//!
//! jemalloc is the global allocator on every non-Windows target and its
//! sampling profiler and stats are compiled in unconditionally; this module,
//! gated behind the `jemalloc-prof` feature, adds the background thread that
//! dumps profiles on a timer. jemalloc itself is configured through the
//! `MALLOC_CONF` environment variable (the allocator crate is built with
//! `unprefixed_malloc_on_supported_platforms`, so the unprefixed name works),
//! e.g. `prof:true,prof_active:true,prof_accum:true,lg_prof_sample:19` — a
//! 512 KiB sampling interval. `prof_accum:true` matters: jemalloc defaults it
//! to false, and without it every dump's cumulative counts are zero, so
//! `jeprof --alloc_space` (allocation churn, as opposed to what is still live)
//! returns an empty profile.
//!
//! When `SCRYER_JEMALLOC_PROF_DUMP_SECS` is set, a background thread writes a
//! timestamped heap profile into `SCRYER_JEMALLOC_PROF_DIR` every N seconds and
//! logs one INFO line per dump, including jemalloc's own
//! `stats.allocated/active/resident/retained` gauges.

use std::ffi::CString;
use std::os::raw::c_char;
use std::time::{SystemTime, UNIX_EPOCH};

const DUMP_SECS_ENV: &str = "SCRYER_JEMALLOC_PROF_DUMP_SECS";
const DUMP_DIR_ENV: &str = "SCRYER_JEMALLOC_PROF_DIR";
const DEFAULT_DIR: &str = "/scryer-data/jemalloc-prof";

/// Reads jemalloc's aggregate gauges. The `epoch` write is what makes the
/// cached statistics refresh; without it every read returns the same numbers.
fn stats() -> (usize, usize, usize, usize) {
    let refreshed = unsafe { tikv_jemalloc_ctl::raw::write::<u64>(b"epoch\0", 1) };
    if refreshed.is_err() {
        return (0, 0, 0, 0);
    }
    let read = |name: &'static [u8]| -> usize {
        unsafe { tikv_jemalloc_ctl::raw::read::<usize>(name) }.unwrap_or(0)
    };
    (
        read(b"stats.allocated\0"),
        read(b"stats.active\0"),
        read(b"stats.resident\0"),
        read(b"stats.retained\0"),
    )
}

/// Writes one heap profile to `path`. jemalloc's `prof.dump` mallctl takes the
/// destination path as a NUL-terminated string value.
fn dump(path: &str) -> Result<(), String> {
    let c_path = CString::new(path).map_err(|error| error.to_string())?;
    unsafe { tikv_jemalloc_ctl::raw::write::<*const c_char>(b"prof.dump\0", c_path.as_ptr()) }
        .map_err(|error| error.to_string())
}

/// Starts the periodic dump thread when `SCRYER_JEMALLOC_PROF_DUMP_SECS` names
/// a positive number of seconds. A no-op otherwise, so the feature can stay
/// compiled into an image that is not currently profiling.
pub fn spawn_dump_thread() {
    let Some(secs) = std::env::var(DUMP_SECS_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
    else {
        return;
    };
    // Compose passes an unset variable through as an empty string, so an empty
    // value has to mean "unset" here too.
    let dir = std::env::var(DUMP_DIR_ENV)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
        .unwrap_or_else(|| DEFAULT_DIR.to_string());
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("jemalloc-prof: cannot create {dir}: {error}");
        return;
    }
    let interval = std::time::Duration::from_secs(secs);
    std::thread::Builder::new()
        .name("jemalloc-prof".to_string())
        .spawn(move || {
            let pid = std::process::id();
            loop {
                std::thread::sleep(interval);
                let stamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|since| since.as_secs())
                    .unwrap_or_default();
                let path = format!("{dir}/heap.{stamp}.{pid}.heap");
                let (allocated, active, resident, retained) = stats();
                match dump(&path) {
                    Ok(()) => {
                        let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                        tracing::info!(
                            target: "scryer::jemalloc_prof",
                            path = %path,
                            profile_bytes = bytes,
                            stats_allocated = allocated,
                            stats_active = active,
                            stats_resident = resident,
                            stats_retained = retained,
                            "jemalloc heap profile written"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "scryer::jemalloc_prof",
                            path = %path,
                            error = %error,
                            "jemalloc heap profile dump failed"
                        );
                    }
                }
            }
        })
        .map(|_| ())
        .unwrap_or_else(|error| eprintln!("jemalloc-prof: cannot spawn dump thread: {error}"));
}

#[cfg(test)]
mod tests {
    /// `prof.dump` is refused when the binary was not started with
    /// `MALLOC_CONF=prof:true`, so the unit test only pins the error path: the
    /// call must return, not abort, and must not panic.
    #[test]
    fn dump_without_profiling_enabled_returns_instead_of_aborting() {
        // An owned directory: whatever a dump writes goes away with it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("smoke.heap");
        let _ = super::dump(path.to_string_lossy().as_ref());
    }

    #[test]
    fn stats_are_readable() {
        let (allocated, active, resident, _retained) = super::stats();
        assert!(allocated > 0, "jemalloc reports no allocated bytes");
        assert!(active >= allocated);
        assert!(resident >= active);
    }
}
