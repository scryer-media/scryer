//! WASI preview1 sandbox and store limits for the archive host.
//!
//! Per invocation: a memory cap enforced by a tracking `ResourceLimiter`, and a
//! WASI p1 context whose only authority is the spec's preopened directories plus
//! a private rw scratch dir (`TMPDIR`). Stdio is host-owned (stdin carries the
//! request, stdout the response, stderr is captured) — never inherited. No
//! network, no env beyond `TMPDIR`.

use scryer_application::{AppError, AppResult};
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder};

use crate::runtime_backing::PreopenSpec;
use crate::wasmtime_host::command_host::CommandHost;

/// Provisional default memory cap for an archive instance: 1 GiB.
///
/// Open question: the real sizing driver is solid-RAR dictionaries;
/// WP2's benchmark against the real fixture finalises this. Operator-overridable
/// via `PluginInstanceSpec::memory_max_bytes`.
pub(crate) const DEFAULT_ARCHIVE_MEMORY_CAP_BYTES: usize = 1024 * 1024 * 1024;

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

/// Store data for an archive invocation: the WASI context plus the memory
/// limiter (`Store::limiter` borrows `&mut ctx.limits`).
pub(crate) struct HostCtx {
    pub(crate) wasi: WasiP1Ctx,
    pub(crate) limits: HostLimits,
    pub(crate) command_host: CommandHost,
}

impl HostCtx {
    pub(crate) fn new(wasi: WasiP1Ctx, limits: HostLimits) -> Self {
        Self::with_command_host(wasi, limits, CommandHost::disabled())
    }

    pub(crate) fn with_command_host(
        wasi: WasiP1Ctx,
        limits: HostLimits,
        command_host: CommandHost,
    ) -> Self {
        Self {
            wasi,
            limits,
            command_host,
        }
    }
}

/// Memory-cap limiter that also records the first denial, so the error mapper
/// can attribute an OOM/limit trap to the resource limit rather than a generic
/// fault. A superset of `StoreLimits::memory_size`.
pub(crate) struct HostLimits {
    max_memory_bytes: usize,
    pub(crate) memory_denied: bool,
}

impl HostLimits {
    pub(crate) fn new(max_memory_bytes: Option<usize>) -> Self {
        Self {
            max_memory_bytes: max_memory_bytes.unwrap_or(DEFAULT_ARCHIVE_MEMORY_CAP_BYTES),
            memory_denied: false,
        }
    }
}

impl wasmtime::ResourceLimiter for HostLimits {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.max_memory_bytes {
            self.memory_denied = true;
            return Ok(false);
        }
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

/// A built WASI context together with the output pipes and scratch dir that must
/// outlive the invocation. `stdout`/`stderr` are cheap clones sharing the guest
/// buffers, so their contents can be read after the run.
pub(crate) struct PreparedSandbox {
    pub(crate) wasi: WasiP1Ctx,
    pub(crate) stdout: MemoryOutputPipe,
    pub(crate) stderr: MemoryOutputPipe,
    /// Kept alive for the invocation; dropped (and removed) afterwards.
    pub(crate) _scratch: tempfile::TempDir,
}

/// A minimal WASI ctx (no filesystem/network) plus its captured output pipes.
pub(crate) struct BareSandbox {
    pub(crate) wasi: WasiP1Ctx,
    pub(crate) stdout: MemoryOutputPipe,
    pub(crate) stderr: MemoryOutputPipe,
}

/// WASI ctx for a describe invocation: empty stdin, argv
/// `[argv0, "describe"]`, captured stdio, and NO filesystem or network authority
/// — a describe call is a pure function of the artifact. The guest keys the
/// describe branch off argv[1] (`std::env::args().nth(1)`), so argv[0] MUST be
/// present ahead of the subcommand or the guest falls through to request mode.
pub(crate) fn build_describe_sandbox() -> BareSandbox {
    let stdout = MemoryOutputPipe::new(STDOUT_CAPACITY_BYTES);
    let stderr = MemoryOutputPipe::new(STDERR_CAPACITY_BYTES);
    let mut builder = WasiCtxBuilder::new();
    builder
        .allow_blocking_current_thread(false)
        .stdin(MemoryInputPipe::new(Vec::<u8>::new()))
        .stdout(stdout.clone())
        .stderr(stderr.clone())
        .args(&[GUEST_ARGV0, DESCRIBE_ARG]);
    BareSandbox {
        wasi: builder.build_p1(),
        stdout,
        stderr,
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

/// Build the WASI Preview 2 sandbox for one archive component invocation.
///
/// Directory authority is identical to the p1 sandbox: exactly the spec's
/// preopens (read-only source, writable output) plus a private rw scratch dir
/// exposed at `TMPDIR`. Nothing else is reachable — no network, no host env,
/// no inherited stdio.
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
/// artifact, exactly as it is on the p1 path.
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

/// Build the WASI p1 sandbox for one invocation: request on stdin, captured
/// stdout/stderr, the spec's preopens, and a private rw scratch at `TMPDIR`.
pub(crate) fn build_sandbox(
    preopens: &[PreopenSpec],
    request_bytes: Vec<u8>,
) -> AppResult<PreparedSandbox> {
    let scratch = tempfile::Builder::new()
        .prefix(".scryer-archive-scratch-")
        .tempdir()
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to create archive plugin scratch dir: {error}"
            ))
        })?;

    let stdin = MemoryInputPipe::new(request_bytes);
    let stdout = MemoryOutputPipe::new(STDOUT_CAPACITY_BYTES);
    let stderr = MemoryOutputPipe::new(STDERR_CAPACITY_BYTES);

    let mut builder = WasiCtxBuilder::new();
    builder
        .allow_blocking_current_thread(false)
        .stdin(stdin)
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

    // Per-invocation rw scratch, pointed at by TMPDIR for large-member spill.
    builder
        .preopened_dir(scratch.path(), SCRATCH_GUEST_PATH, FsPerms::ReadWrite)
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to preopen archive plugin scratch dir: {error}"
            ))
        })?;

    Ok(PreparedSandbox {
        wasi: builder.build_p1(),
        stdout,
        stderr,
        _scratch: scratch,
    })
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
    fn limiter_denies_growth_past_cap_and_records_it() {
        use wasmtime::ResourceLimiter;
        let mut limits = HostLimits::new(Some(4096));
        assert!(limits.memory_growing(0, 4096, None).unwrap());
        assert!(!limits.memory_denied);
        assert!(!limits.memory_growing(4096, 8192, None).unwrap());
        assert!(limits.memory_denied);
    }

    #[test]
    fn build_sandbox_wires_request_and_scratch() {
        let sandbox = build_sandbox(&[], b"{\"hello\":true}".to_vec()).expect("build sandbox");
        // stdout/stderr start empty; scratch dir exists.
        assert!(sandbox.stdout.contents().is_empty());
        assert!(sandbox._scratch.path().is_dir());
    }

    /// The component sandbox must grant exactly the p1 sandbox's authority:
    /// the spec's preopens plus a private scratch dir. A rejected preopen is a
    /// hard error rather than a silently unreachable directory.
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

        let Err(error) = build_component_sandbox(&[PreopenSpec::read_only(
            &missing,
            "/scryer/source",
        )]) else {
            panic!("a missing preopen must fail the invocation, not be skipped");
        };

        assert!(error.to_string().contains("failed to preopen"), "{error}");
    }
}
