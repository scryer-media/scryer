//! WASI Preview 2 sandbox and store limits for the component hosts.
//!
//! Per invocation: a memory cap enforced by a tracking `ResourceLimiter`, and a
//! WASI context whose only authority is the spec's preopened directories plus a
//! private rw scratch dir (`TMPDIR`). Stdio is host-owned and captured for
//! diagnostics — never inherited. No network, no env beyond `TMPDIR`.

use scryer_application::{AppError, AppResult};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder};

use crate::runtime_backing::PreopenSpec;

/// Provisional default memory cap for an archive instance: 1 GiB.
///
/// Open question: the real sizing driver is solid-RAR dictionaries;
/// WP2's benchmark against the real fixture finalises this. Operator-overridable
/// via `PluginInstanceSpec::memory_max_bytes`.
pub(crate) const DEFAULT_ARCHIVE_MEMORY_CAP_BYTES: usize = 1024 * 1024 * 1024;

/// Default memory cap for one indexer component Store: 128 MiB.
///
/// An indexer guest holds at most one upstream HTTP body at a time, and the
/// host caps each body at 50 MiB (`DEFAULT_MAX_HTTP_RESPONSE_BYTES` in
/// `component_host`). Parsing a feed into results and serializing the response
/// costs roughly another body's worth, so ~100 MiB covers the worst legitimate
/// single page plus allocator slack; anything beyond that is accumulation, not
/// a page. The archive 1 GiB cap is sized for solid-RAR dictionaries and must
/// not be inherited by indexers: one Store is retained per configured indexer,
/// so a 1 GiB ceiling let three indexers grow a 4 GiB host into the OOM killer.
/// `SCRYER_INDEXER_PLUGIN_MEMORY_MAX_MIB` overrides it.
pub(crate) const DEFAULT_INDEXER_MEMORY_CAP_BYTES: usize = 128 * 1024 * 1024;

/// Default retention watermark for an indexer component Store: 32 MiB.
///
/// Wasm linear memory never shrinks, so a retained Store keeps its peak
/// resident for the life of the process. A Store whose linear memory exceeds
/// this after an invocation is dropped and re-instantiated on the next call;
/// instantiation from the pre-linked component costs milliseconds.
/// `SCRYER_INDEXER_PLUGIN_RETAIN_WATERMARK_MIB` overrides it.
pub(crate) const DEFAULT_INDEXER_RETAIN_WATERMARK_BYTES: usize = 32 * 1024 * 1024;

pub(crate) const INDEXER_MEMORY_CAP_ENV: &str = "SCRYER_INDEXER_PLUGIN_MEMORY_MAX_MIB";
const INDEXER_RETAIN_WATERMARK_ENV: &str = "SCRYER_INDEXER_PLUGIN_RETAIN_WATERMARK_MIB";

/// Memory bounds for indexer component Stores: the hard per-Store cap enforced
/// by [`HostLimits`] and the watermark above which the adapter releases a
/// retained Store after an invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IndexerMemoryPolicy {
    pub(crate) cap_bytes: usize,
    pub(crate) retain_watermark_bytes: usize,
}

impl Default for IndexerMemoryPolicy {
    fn default() -> Self {
        Self {
            cap_bytes: DEFAULT_INDEXER_MEMORY_CAP_BYTES,
            retain_watermark_bytes: DEFAULT_INDEXER_RETAIN_WATERMARK_BYTES,
        }
    }
}

impl IndexerMemoryPolicy {
    /// The process-wide policy, read once from the environment.
    pub(crate) fn from_env() -> Self {
        static POLICY: std::sync::OnceLock<IndexerMemoryPolicy> = std::sync::OnceLock::new();
        *POLICY.get_or_init(|| {
            Self::from_values(
                std::env::var(INDEXER_MEMORY_CAP_ENV).ok().as_deref(),
                std::env::var(INDEXER_RETAIN_WATERMARK_ENV).ok().as_deref(),
            )
        })
    }

    /// Parse MiB overrides; unset, empty, zero or malformed values keep the
    /// default. The watermark never exceeds the cap.
    pub(crate) fn from_values(cap_mib: Option<&str>, watermark_mib: Option<&str>) -> Self {
        let parse = |value: Option<&str>| {
            value
                .and_then(|value| value.trim().parse::<usize>().ok())
                .filter(|mib| *mib > 0)
                .and_then(|mib| mib.checked_mul(1024 * 1024))
        };
        let cap_bytes = parse(cap_mib).unwrap_or(DEFAULT_INDEXER_MEMORY_CAP_BYTES);
        let retain_watermark_bytes = parse(watermark_mib)
            .unwrap_or(DEFAULT_INDEXER_RETAIN_WATERMARK_BYTES)
            .min(cap_bytes);
        Self {
            cap_bytes,
            retain_watermark_bytes,
        }
    }
}

/// Table-element ceiling for an archive instance. The shipped guest never grows
/// its function table; a finite cap stops a buggy/hostile (still-signed) artifact
/// from allocating host memory via `table.grow` in a loop — the dimension a
/// memory-only cap leaves open.
pub(crate) const DEFAULT_ARCHIVE_TABLE_ELEMENTS: usize = 1_000_000;

/// Guest path (and `TMPDIR` value) for the per-invocation scratch dir used for
/// large-member spill.
const SCRATCH_GUEST_PATH: &str = "/tmp";

/// argv[0] presented to the guest command for a process invocation.
const GUEST_ARGV0: &str = "scryer-archive-plugin";

/// WASI subcommand arg for a describe invocation. The command binary
/// self-describes when run as `<argv0> describe` — i.e. `"describe"` in argv[1],
/// which the guest reads via `std::env::args().nth(1)` (coordinated guest
/// contract) — printing its `PluginDescriptor` JSON to stdout and exiting 0.
const DESCRIBE_ARG: &str = "describe";

/// stdout capacity. Generous — the response is a JSON file listing; a write past
/// this bound traps and is reported as a plugin failure.
const STDOUT_CAPACITY_BYTES: usize = 64 * 1024 * 1024;
/// stderr capacity (diagnostic only; the logged/attached portion is tailed).
const STDERR_CAPACITY_BYTES: usize = 4 * 1024 * 1024;

/// Memory-cap limiter that also records the first denial, so the error mapper
/// can attribute an OOM/limit trap to the resource limit rather than a generic
/// fault. A superset of `StoreLimits::memory_size`.
///
/// It also records how large the Store's linear memory got: wasmtime reports
/// every memory's creation (`current == 0`) and growth here, and linear memory
/// never shrinks, so the sum of granted growth is the Store's resident linear
/// memory and also its peak. The cap bounds that sum as well as each single
/// memory, so a component with several core memories cannot multiply it.
pub(crate) struct HostLimits {
    max_memory_bytes: usize,
    pub(crate) memory_denied: bool,
    /// Largest single linear memory granted.
    largest_memory_bytes: usize,
    /// Linear memory granted across every memory in the Store.
    resident_memory_bytes: usize,
}

impl HostLimits {
    pub(crate) fn new(max_memory_bytes: Option<usize>) -> Self {
        Self::with_cap(max_memory_bytes.unwrap_or(DEFAULT_ARCHIVE_MEMORY_CAP_BYTES))
    }

    pub(crate) fn with_cap(max_memory_bytes: usize) -> Self {
        Self {
            max_memory_bytes,
            memory_denied: false,
            largest_memory_bytes: 0,
            resident_memory_bytes: 0,
        }
    }

    pub(crate) const fn max_memory_bytes(&self) -> usize {
        self.max_memory_bytes
    }

    pub(crate) const fn largest_memory_bytes(&self) -> usize {
        self.largest_memory_bytes
    }

    pub(crate) const fn resident_memory_bytes(&self) -> usize {
        self.resident_memory_bytes
    }
}

impl wasmtime::ResourceLimiter for HostLimits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        // The cap is per Store, not per memory: a component may instantiate
        // several core memories, and checking each on its own would let N of
        // them reach N times the cap. Deny growth that would take either one
        // memory or the Store's total past it.
        let growth = desired.saturating_sub(current);
        let projected_resident = self.resident_memory_bytes.saturating_add(growth);
        if desired > self.max_memory_bytes || projected_resident > self.max_memory_bytes {
            self.memory_denied = true;
            return Ok(false);
        }
        self.largest_memory_bytes = self.largest_memory_bytes.max(desired);
        self.resident_memory_bytes = projected_resident;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= DEFAULT_ARCHIVE_TABLE_ELEMENTS)
    }
}

/// A built WASI Preview 2 context for one archive component invocation.
///
/// The component protocol carries the request and the response through the
/// world's `process` export, so p2 stdio is diagnostics only: stdin is empty
/// and stdout/stderr are captured for the failure message.
pub(crate) struct PreparedComponentSandbox {
    pub(crate) wasi: WasiCtx,
    pub(crate) stdout: MemoryOutputPipe,
    pub(crate) stderr: MemoryOutputPipe,
    /// Kept alive for the invocation; dropped (and removed) afterwards.
    pub(crate) _scratch: tempfile::TempDir,
}

/// Build the WASI Preview 2 sandbox for one component invocation.
///
/// Directory authority is exactly the spec's preopens (read-only source,
/// writable output) plus a private rw scratch dir exposed at `TMPDIR`. Nothing
/// else is reachable — no network, no host env, no inherited stdio.
pub(crate) fn build_component_sandbox(
    preopens: &[PreopenSpec],
) -> AppResult<PreparedComponentSandbox> {
    let scratch = tempfile::Builder::new()
        .prefix(".scryer-archive-scratch-")
        .tempdir()
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to create archive plugin scratch dir: {error}"
            ))
        })?;

    let stdout = MemoryOutputPipe::new(STDOUT_CAPACITY_BYTES);
    let stderr = MemoryOutputPipe::new(STDERR_CAPACITY_BYTES);

    let mut builder = WasiCtxBuilder::new();
    builder
        .allow_blocking_current_thread(false)
        .stdin(MemoryInputPipe::new(Vec::<u8>::new()))
        .stdout(stdout.clone())
        .stderr(stderr.clone())
        .args(&[GUEST_ARGV0])
        .env("TMPDIR", SCRATCH_GUEST_PATH);

    for preopen in preopens {
        let perms = if preopen.writable {
            FsPerms::ReadWrite
        } else {
            FsPerms::ReadOnly
        };
        builder
            .preopened_dir(&preopen.host_path, &preopen.guest_path, perms)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to preopen '{}' as '{}' for archive plugin: {error}",
                    preopen.host_path.display(),
                    preopen.guest_path
                ))
            })?;
    }

    builder
        .preopened_dir(scratch.path(), SCRATCH_GUEST_PATH, FsPerms::ReadWrite)
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to preopen archive plugin scratch dir: {error}"
            ))
        })?;

    Ok(PreparedComponentSandbox {
        wasi: builder.build(),
        stdout,
        stderr,
        _scratch: scratch,
    })
}

/// A minimal WASI Preview 2 context for a describe invocation: no filesystem,
/// no network, captured stdio. A describe call is a pure function of the
/// artifact.
pub(crate) fn build_component_describe_sandbox() -> (WasiCtx, MemoryOutputPipe) {
    let stderr = MemoryOutputPipe::new(STDERR_CAPACITY_BYTES);
    let mut builder = WasiCtxBuilder::new();
    builder
        .allow_blocking_current_thread(false)
        .stdin(MemoryInputPipe::new(Vec::<u8>::new()))
        .stdout(MemoryOutputPipe::new(STDOUT_CAPACITY_BYTES))
        .stderr(stderr.clone())
        .args(&[GUEST_ARGV0, DESCRIBE_ARG]);
    (builder.build(), stderr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cap_applies_when_unset() {
        let limits = HostLimits::new(None);
        assert_eq!(limits.max_memory_bytes, DEFAULT_ARCHIVE_MEMORY_CAP_BYTES);
        assert!(!limits.memory_denied);
    }

    #[test]
    fn indexer_cap_is_separate_from_and_smaller_than_the_archive_cap() {
        let policy = IndexerMemoryPolicy::default();
        assert_eq!(policy.cap_bytes, DEFAULT_INDEXER_MEMORY_CAP_BYTES);
        assert_eq!(policy.cap_bytes, 128 * 1024 * 1024);
        assert_eq!(policy.retain_watermark_bytes, 32 * 1024 * 1024);
        assert_ne!(policy.cap_bytes, DEFAULT_ARCHIVE_MEMORY_CAP_BYTES);
        assert!(policy.cap_bytes < DEFAULT_ARCHIVE_MEMORY_CAP_BYTES);
        assert!(policy.retain_watermark_bytes < policy.cap_bytes);
        assert_eq!(
            HostLimits::with_cap(policy.cap_bytes).max_memory_bytes(),
            DEFAULT_INDEXER_MEMORY_CAP_BYTES
        );
        // The archive default is untouched.
        assert_eq!(
            HostLimits::new(None).max_memory_bytes(),
            DEFAULT_ARCHIVE_MEMORY_CAP_BYTES
        );
    }

    #[test]
    fn indexer_memory_policy_parses_mib_overrides() {
        assert_eq!(
            IndexerMemoryPolicy::from_values(None, None),
            IndexerMemoryPolicy::default()
        );
        assert_eq!(
            IndexerMemoryPolicy::from_values(Some(" 256 "), Some("64")),
            IndexerMemoryPolicy {
                cap_bytes: 256 * 1024 * 1024,
                retain_watermark_bytes: 64 * 1024 * 1024,
            }
        );
        // Zero and garbage keep the defaults.
        assert_eq!(
            IndexerMemoryPolicy::from_values(Some("0"), Some("lots")),
            IndexerMemoryPolicy::default()
        );
        // The watermark is clamped to the cap.
        assert_eq!(
            IndexerMemoryPolicy::from_values(Some("16"), None),
            IndexerMemoryPolicy {
                cap_bytes: 16 * 1024 * 1024,
                retain_watermark_bytes: 16 * 1024 * 1024,
            }
        );
    }

    #[test]
    fn limiter_tracks_peak_and_resident_memory() {
        use wasmtime::ResourceLimiter;
        let mut limits = HostLimits::with_cap(1 << 20);
        assert_eq!(limits.resident_memory_bytes(), 0);
        // Creation of two memories, then growth of the first.
        assert!(limits.memory_growing(0, 65536, None).unwrap());
        assert!(limits.memory_growing(0, 4096, None).unwrap());
        assert!(limits.memory_growing(65536, 262_144, None).unwrap());
        assert_eq!(limits.largest_memory_bytes(), 262_144);
        assert_eq!(limits.resident_memory_bytes(), 262_144 + 4096);
        // A denied growth changes neither figure but is recorded.
        assert!(!limits.memory_growing(262_144, 2 << 20, None).unwrap());
        assert!(limits.memory_denied);
        assert_eq!(limits.largest_memory_bytes(), 262_144);
        assert_eq!(limits.resident_memory_bytes(), 262_144 + 4096);
    }

    #[test]
    fn limiter_denies_growth_past_cap_and_records_it() {
        use wasmtime::ResourceLimiter;
        let mut limits = HostLimits::new(Some(4096));
        assert!(limits.memory_growing(0, 4096, None).unwrap());
        assert!(!limits.memory_denied);
        assert!(!limits.memory_growing(4096, 8192, None).unwrap());
        assert!(limits.memory_denied);
    }

    #[test]
    fn limiter_caps_the_sum_of_every_memory_in_the_store() {
        use wasmtime::ResourceLimiter;
        const CAP: usize = 1 << 20;
        let mut limits = HostLimits::with_cap(CAP);
        // Two memories, each within the per-memory cap on its own.
        assert!(limits.memory_growing(0, 3 * CAP / 4, None).unwrap());
        assert!(limits.memory_growing(0, CAP / 8, None).unwrap());
        assert!(!limits.memory_denied);
        // A third memory whose creation would take the total past the cap is
        // denied although it alone is far below it.
        assert!(!limits.memory_growing(0, CAP / 4, None).unwrap());
        assert!(limits.memory_denied);
        // So is growing the second memory past the remaining headroom.
        assert!(!limits.memory_growing(CAP / 8, CAP / 4 + 1, None).unwrap());
        // Denials leave the accounting exactly at what was granted.
        assert_eq!(limits.resident_memory_bytes(), 3 * CAP / 4 + CAP / 8);
        assert_eq!(limits.largest_memory_bytes(), 3 * CAP / 4);
        // Growth up to exactly the cap is still granted.
        assert!(limits.memory_growing(CAP / 8, CAP / 4, None).unwrap());
        assert_eq!(limits.resident_memory_bytes(), CAP);
    }

    #[test]
    fn limiter_lets_a_single_memory_grow_to_exactly_the_cap() {
        use wasmtime::ResourceLimiter;
        const CAP: usize = 1 << 20;
        let mut limits = HostLimits::with_cap(CAP);
        assert!(limits.memory_growing(0, CAP / 2, None).unwrap());
        assert!(limits.memory_growing(CAP / 2, CAP, None).unwrap());
        assert!(!limits.memory_denied);
        assert_eq!(limits.resident_memory_bytes(), CAP);
        assert_eq!(limits.largest_memory_bytes(), CAP);
        assert!(!limits.memory_growing(CAP, CAP + 65536, None).unwrap());
        assert!(limits.memory_denied);
        assert_eq!(limits.resident_memory_bytes(), CAP);
    }

    /// The component sandbox grants exactly the spec's preopens plus a private
    /// scratch dir. A rejected preopen is a hard error rather than a silently
    /// unreachable directory.
    #[test]
    fn build_component_sandbox_accepts_the_archive_preopens() {
        let source = tempfile::tempdir().expect("source dir");
        let output = tempfile::tempdir().expect("output dir");
        let preopens = vec![
            PreopenSpec::read_only(source.path(), "/scryer/source"),
            PreopenSpec::writable(output.path(), "/scryer/output"),
        ];

        let sandbox = build_component_sandbox(&preopens).expect("build component sandbox");

        assert!(sandbox.stdout.contents().is_empty());
        assert!(sandbox.stderr.contents().is_empty());
        assert!(sandbox._scratch.path().is_dir());
    }

    #[test]
    fn build_component_sandbox_rejects_a_missing_preopen() {
        let missing = std::env::temp_dir().join("scryer-archive-component-missing-preopen");
        let _ = std::fs::remove_dir_all(&missing);

        let Err(error) =
            build_component_sandbox(&[PreopenSpec::read_only(&missing, "/scryer/source")])
        else {
            panic!("a missing preopen must fail the invocation, not be skipped");
        };

        assert!(error.to_string().contains("failed to preopen"), "{error}");
    }
}
