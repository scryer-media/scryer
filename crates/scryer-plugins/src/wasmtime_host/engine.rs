//! Process-wide wasmtime engine for the archive host.
//!
//! One lazily-initialised `Engine` is shared for the whole process. Its `Config`
//! turns on epoch interruption (for wall-clock cancellation) and pins the
//! safety-relevant knobs (wasm stack bound, linear-memory guard page, native
//! unwind info) so a future wasmtime bump cannot silently weaken them. Only the
//! default-on SIMD / relaxed-SIMD proposals are exposed to guests; threads and
//! exceptions are deliberately left off (see `archive_engine_config`). A single
//! background thread increments the engine epoch on a fixed tick so
//! per-invocation deadlines actually fire without a timer thread per call.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use wasmtime::{Cache, CacheConfig, Config, Engine};

/// Epoch tick interval. Per-invocation deadlines are expressed as a whole
/// number of ticks, so this bounds the timeout granularity (~100 ms).
pub(crate) const EPOCH_TICK: Duration = Duration::from_millis(100);

struct WasmRuntimeConfig {
    cache_dir: PathBuf,
    cache: Cache,
}

/// Configured exactly once by Scryer before any plugin engine is created. The
/// cache is per-instance and local to the machine, so portable database
/// backups never carry native code generated for another host.
static WASM_RUNTIME_CONFIG: OnceLock<WasmRuntimeConfig> = OnceLock::new();

/// The shared engine. Constructed only after the persistent cache has been
/// configured; the ticker thread then lives for the remainder of the process.
static SHARED_ENGINE: OnceLock<Engine> = OnceLock::new();

/// Async command-model engine. WASI `poll_oneoff` and friends yield through
/// Wasmtime async support, so adapter-level timeouts can cancel sleeps without
/// parking Tokio blocking threads.
static SHARED_ASYNC_ENGINE: OnceLock<Engine> = OnceLock::new();

/// Prepare Scryer's private persistent Wasmtime cache. This is deliberately a
/// startup prerequisite: silently falling back to an unconfigured engine would
/// reintroduce recompilation after every restart.
pub fn initialize_wasm_runtime(data_dir: impl AsRef<Path>) -> Result<(), String> {
    initialize_wasm_runtime_at(data_dir.as_ref().join("cache").join("wasmtime"))
}

/// Prepare Scryer's private persistent Wasmtime cache at an explicit,
/// platform-resolved directory. The caller must choose an instance-local path;
/// this function never falls back to Wasmtime's user-global cache.
pub fn initialize_wasm_runtime_at(cache_dir: impl AsRef<Path>) -> Result<(), String> {
    let cache_dir = cache_dir.as_ref().to_path_buf();
    fs::create_dir_all(&cache_dir).map_err(|error| {
        format!(
            "failed to create Wasmtime cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    let cache_dir = fs::canonicalize(&cache_dir).map_err(|error| {
        format!(
            "failed to resolve Wasmtime cache directory {}: {error}",
            cache_dir.display()
        )
    })?;
    restrict_cache_directory(&cache_dir)?;
    probe_cache_directory(&cache_dir)?;

    if let Some(configured) = WASM_RUNTIME_CONFIG.get() {
        return if configured.cache_dir == cache_dir {
            Ok(())
        } else {
            Err(format!(
                "WASM runtime cache is already configured for {}; cannot reconfigure it for {}",
                configured.cache_dir.display(),
                cache_dir.display()
            ))
        };
    }

    if SHARED_ENGINE.get().is_some() || SHARED_ASYNC_ENGINE.get().is_some() {
        return Err(
            "WASM runtime was initialized before its cache directory was configured".into(),
        );
    }

    let mut cache_config = CacheConfig::new();
    cache_config.with_directory(cache_dir.clone());
    let cache = Cache::new(cache_config).map_err(|error| {
        format!(
            "failed to initialize Wasmtime cache at {}: {error:#}",
            cache_dir.display()
        )
    })?;
    let config = WasmRuntimeConfig { cache_dir, cache };

    match WASM_RUNTIME_CONFIG.set(config) {
        Ok(()) => Ok(()),
        Err(config) if config.cache_dir == configured_cache_dir() => Ok(()),
        Err(config) => Err(format!(
            "WASM runtime cache is already configured for {}; cannot reconfigure it for {}",
            configured_cache_dir().display(),
            config.cache_dir.display()
        )),
    }
}

fn restrict_cache_directory(cache_dir: &Path) -> Result<(), String> {
    #[cfg(not(unix))]
    let _ = cache_dir;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(cache_dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
            format!(
                "failed to restrict Wasmtime cache directory {}: {error}",
                cache_dir.display()
            )
        })?;
    }
    Ok(())
}

fn configured_cache_dir() -> &'static Path {
    &WASM_RUNTIME_CONFIG
        .get()
        .expect("WASM runtime cache must be initialized before plugin use")
        .cache_dir
}

fn configured_cache() -> Cache {
    WASM_RUNTIME_CONFIG
        .get()
        .expect("WASM runtime cache must be initialized before plugin use")
        .cache
        .clone()
}

pub(crate) fn cache_statistics() -> (usize, usize) {
    let cache = configured_cache();
    (cache.cache_hits(), cache.cache_misses())
}

fn probe_cache_directory(cache_dir: &Path) -> Result<(), String> {
    let probe = cache_dir.join(format!(
        ".scryer-wasmtime-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|error| {
            format!(
                "Wasmtime cache directory {} is not writable: {error}",
                cache_dir.display()
            )
        })?;
    file.write_all(b"scryer-wasmtime-cache-probe")
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            format!(
                "failed to write Wasmtime cache probe {}: {error}",
                probe.display()
            )
        })?;
    drop(file);
    fs::read(&probe).map_err(|error| {
        format!(
            "Wasmtime cache directory {} is not readable: {error}",
            cache_dir.display()
        )
    })?;
    fs::remove_file(&probe).map_err(|error| {
        format!(
            "failed to remove Wasmtime cache probe {}: {error}",
            probe.display()
        )
    })
}

/// Borrow the process-wide archive engine, initialising it (and its epoch
/// ticker) on first call.
pub(crate) fn shared_engine() -> &'static Engine {
    #[cfg(test)]
    initialize_wasm_runtime_for_tests();
    SHARED_ENGINE.get_or_init(|| new_engine("archive host"))
}

/// Borrow the async command-model engine, initialising it (and its epoch ticker)
/// on first use.
pub(crate) fn shared_async_engine() -> &'static Engine {
    #[cfg(test)]
    initialize_wasm_runtime_for_tests();
    SHARED_ASYNC_ENGINE.get_or_init(|| new_engine("async command host"))
}

fn new_engine(kind: &str) -> Engine {
    let engine = Engine::new(&archive_engine_config())
        .unwrap_or_else(|error| panic!("wasmtime engine config for {kind} must be valid: {error}"));
    spawn_epoch_ticker(engine.clone());
    engine
}

/// Build the archive host `Config`.
///
/// This engine is process-wide and shared by every untrusted plugin module, so
/// its feature surface is kept to the minimum today's guests actually consume.
/// SIMD / relaxed SIMD are on by default in wasmtime and set explicitly for
/// intent.
///
/// `wasm_threads` and `wasm_exceptions` are deliberately NOT enabled. No
/// shipping guest (the archive + subtitle-command artifacts, or the legacy
/// Extism-compat reactors) uses either, and turning them on process-wide only
/// adds attack surface: threads bring a `memory.atomic.wait` blocking primitive
/// that epoch interruption cannot preempt (a worker-thread DoS), and exceptions
/// bring needless Cranelift EH codegen. They are dropped pending a real WP6
/// consumer. When one lands, re-enable them behind a PER-ARTIFACT declared
/// feature gate on a purpose-built engine — never by flipping them back on for
/// this shared, untrusted engine.
fn archive_engine_config() -> Config {
    let mut config = Config::new();
    config.epoch_interruption(true);
    config.wasm_simd(true);
    config.wasm_relaxed_simd(true);
    config.wasm_threads(false);
    // Pin the safety-relevant posture to wasmtime 46's current defaults so a
    // future bump cannot silently weaken it. This is a sync engine (no async
    // path), so there is no async-stack coupling to worry about.
    config.max_wasm_stack(512 * 1024); // 512 KiB wasm stack bound (wasmtime default).
    config.guard_before_linear_memory(true); // OOB guard page before linear memory.
    config.native_unwind_info(true); // Keep native unwind info for trap/backtrace fidelity.
    config.cache(Some(configured_cache()));
    config
}

#[cfg(test)]
fn initialize_wasm_runtime_for_tests() {
    if WASM_RUNTIME_CONFIG.get().is_some() {
        return;
    }
    // Unit tests also run in independent test processes under Nextest.
    let cache_dir = std::env::temp_dir().join("scryer-wasmtime-test-cache");
    initialize_wasm_runtime_at(&cache_dir).expect("test Wasmtime cache must initialize");
}

/// Translate a wall-clock timeout into an epoch-tick deadline for
/// `Store::set_epoch_deadline`. Always at least one tick so a zero/short budget
/// still terminates a wedged guest.
pub(crate) fn deadline_ticks(timeout: Duration) -> u64 {
    let tick = EPOCH_TICK.as_millis().max(1);
    let budget = timeout.as_millis();
    budget.div_ceil(tick).max(1) as u64
}

/// Spawn the single background epoch ticker for `engine`. Detached daemon
/// thread: it holds only a cheap `Engine` clone (an `Arc`) and loops for the
/// life of the process.
fn spawn_epoch_ticker(engine: Engine) {
    std::thread::Builder::new()
        .name("scryer-archive-epoch".to_string())
        .spawn(move || {
            loop {
                std::thread::sleep(EPOCH_TICK);
                engine.increment_epoch();
            }
        })
        .expect("spawn archive epoch ticker thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_for(directory: &Path) -> Cache {
        let mut config = CacheConfig::new();
        config.with_directory(directory.to_path_buf());
        Cache::new(config).expect("test Wasmtime cache must initialize")
    }

    fn engine_with_cache(cache: Cache) -> Engine {
        let mut config = Config::new();
        config.cache(Some(cache));
        Engine::new(&config).expect("test engine must initialize")
    }

    #[test]
    fn engine_config_is_accepted_by_wasmtime() {
        // Proves the pinned, minimized feature surface still yields a valid
        // Engine on the resolved wasmtime (46.0.1).
        initialize_wasm_runtime_for_tests();
        Engine::new(&archive_engine_config()).expect("archive engine config must build");
    }

    #[test]
    fn cache_directory_probe_requires_read_write_access() {
        let temp = tempfile::tempdir().expect("temporary directory");
        probe_cache_directory(temp.path()).expect("writable directory must pass probe");

        let file = temp.path().join("not-a-directory");
        fs::write(&file, b"file").expect("test file");
        assert!(probe_cache_directory(&file).is_err());
    }

    #[test]
    fn wasmtime_cache_reuses_compiled_module_with_a_new_engine() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let wasm = wat::parse_str("(module (func (export \"entry\")))").expect("WAT must parse");

        let writer_cache = cache_for(temp.path());
        let writer_engine = engine_with_cache(writer_cache.clone());
        wasmtime::Module::from_binary(&writer_engine, &wasm).expect("first module must compile");
        assert!(writer_cache.cache_misses() >= 1);

        let reader_cache = cache_for(temp.path());
        let reader_engine = engine_with_cache(reader_cache.clone());
        wasmtime::Module::from_binary(&reader_engine, &wasm).expect("cached module must load");
        assert_eq!(reader_cache.cache_hits(), 1);
        assert_eq!(reader_cache.cache_misses(), 0);
    }

    #[test]
    fn deadline_ticks_rounds_up_and_has_floor() {
        assert_eq!(deadline_ticks(Duration::from_millis(0)), 1);
        assert_eq!(deadline_ticks(Duration::from_millis(1)), 1);
        assert_eq!(deadline_ticks(EPOCH_TICK), 1);
        assert_eq!(deadline_ticks(EPOCH_TICK * 2), 2);
        // 1-hour archive budget at a 100ms tick.
        assert_eq!(deadline_ticks(Duration::from_secs(3600)), 36_000);
    }

    #[test]
    fn shared_engine_is_stable() {
        let a = shared_engine();
        let b = shared_engine();
        assert!(std::ptr::eq(a, b));

        let async_a = shared_async_engine();
        let async_b = shared_async_engine();
        assert!(std::ptr::eq(async_a, async_b));
        assert!(!std::ptr::eq(a, async_a));
    }
}
