//! WASI Preview 2 host for archive-extractor components.
//!
//! Archive extractors are components, and only components: the core-module
//! archive backing (wasip1 command + raw pointer crypto ABI) was removed in the
//! hard cut to `scryer:archive/archive-extractor@1.0.0`. What survives from it
//! is the shape of the sandbox — a read-only source preopen, a writable output
//! preopen, a private `TMPDIR` scratch dir, a memory cap, and an epoch deadline
//! — and the crypto/CRC cores, which now back the world's `crypto` import
//! instead of a hand-registered linker namespace.
//!
//! Instance-per-request, exactly as the command protocol was: one `process`
//! call per plugin invocation, then the whole `Store` is dropped.

use std::sync::Arc;
use std::time::{Duration, Instant};

use scryer_application::{AppError, AppResult};
use scryer_plugin_sdk::{ArchivePluginProcessResponse, PluginDescriptor};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::runtime_backing::PluginInstanceSpec;
use crate::wasmtime_host::sandbox::{self, HostLimits, PreparedComponentSandbox};
use crate::wasmtime_host::{crypto_host, engine, error, module_cache};

mod contract_v1_0 {
    wasmtime::component::bindgen!({
        world: "scryer:archive/archive-extractor@1.0.0",
        path: "wit/archive-v1.0.0",
        exports: { default: async },
    });
}

use self::contract_v1_0::InvocationError;
use self::contract_v1_0::scryer::archive::crypto::{AesError, Host as CryptoHost};

/// Describe runs reuse the 10s describe budget of every other backing.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Amount of guest stderr forwarded to tracing / attached to error messages.
const STDERR_TAIL_BYTES: usize = 8 * 1024;

/// The operator-facing diagnostic for a pre-component archive artifact.
///
/// This is the hard cut's whole user experience: an installed archive plugin
/// built against the removed core-module ABI must say what to do, not merely
/// fail to instantiate.
pub(crate) const ARCHIVE_CORE_MODULE_REJECTED: &str =
    "archive extractor plugins must be WASI Preview 2 components \
     (world scryer:archive/archive-extractor@1.0.0); this artifact is a legacy core wasm module. \
     Upgrade the plugin to a build that targets wasm32-wasip2.";

/// Identifying context for one archive component invocation.
pub(crate) struct ArchiveInvocation<'a> {
    pub(crate) plugin_id: &'a str,
    pub(crate) plugin_version: &'a str,
    pub(crate) operation: &'a str,
}

/// Compile-and-link validation for an archive component artifact, mirroring
/// `validate_indexer_component`.
pub(crate) fn validate_archive_component(wasm: &[u8]) -> Result<(), String> {
    ArchiveComponentRuntime::new(engine::shared_async_engine(), wasm).map(|_| ())
}

/// Store data for one archive component invocation.
pub(crate) struct ArchiveComponentCtx {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: HostLimits,
}

impl WasiView for ArchiveComponentCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl CryptoHost for ArchiveComponentCtx {
    fn aes_cbc_decrypt(
        &mut self,
        key: Vec<u8>,
        iv: Vec<u8>,
        data: Vec<u8>,
    ) -> Result<Vec<u8>, AesError> {
        crypto_host::aes_cbc_decrypt(&key, &iv, &data).map_err(|error| match error {
            crypto_host::AesDecryptError::KeyLength => AesError::BadKeyLength,
            crypto_host::AesDecryptError::BlockAlignment => AesError::BadBlockLength,
            crypto_host::AesDecryptError::IvLength => AesError::BadIvLength,
        })
    }

    fn crc32(&mut self, seed: u32, data: Vec<u8>) -> u32 {
        crypto_host::crc32(seed, &data)
    }
}

/// A compiled archive component plus its pre-instantiated world binding.
pub(crate) struct ArchiveComponentRuntime {
    component: Arc<Component>,
    instance_pre: contract_v1_0::ArchiveExtractorPre<ArchiveComponentCtx>,
}

impl ArchiveComponentRuntime {
    pub(crate) fn new(engine: &Engine, wasm: &[u8]) -> Result<Self, String> {
        let component = module_cache::archive_component(wasm)?;
        if !Engine::same(component.engine(), engine) {
            return Err(
                "archive component cache returned an artifact for a different engine".into(),
            );
        }
        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|error| format!("failed to register WASI Preview 2: {error:#}"))?;
        contract_v1_0::ArchiveExtractor::add_to_linker::<
            ArchiveComponentCtx,
            HasSelf<ArchiveComponentCtx>,
        >(&mut linker, |ctx| ctx)
        .map_err(|error| format!("failed to register archive component host: {error:#}"))?;
        let raw_instance_pre = linker
            .instantiate_pre(&component)
            .map_err(|error| format!("failed to preinstantiate archive component: {error:#}"))?;
        let instance_pre = contract_v1_0::ArchiveExtractorPre::new(raw_instance_pre).map_err(
            |error| {
                format!(
                    "archive component exports do not match scryer:archive/archive-extractor@1.0.0: {error:#}"
                )
            },
        )?;
        Ok(Self {
            component,
            instance_pre,
        })
    }

    async fn instantiate(
        &self,
        wasi: WasiCtx,
        memory_max_bytes: Option<usize>,
        timeout: Duration,
    ) -> Result<
        (
            Store<ArchiveComponentCtx>,
            contract_v1_0::ArchiveExtractor,
        ),
        wasmtime::Error,
    > {
        let mut store = Store::new(
            self.component.engine(),
            ArchiveComponentCtx {
                table: ResourceTable::new(),
                wasi,
                limits: HostLimits::new(memory_max_bytes),
            },
        );
        store.limiter(|ctx: &mut ArchiveComponentCtx| &mut ctx.limits);
        store.set_epoch_deadline(engine::deadline_ticks(timeout));
        let plugin = self.instance_pre.instantiate_async(&mut store).await?;
        Ok((store, plugin))
    }
}

/// Extract a descriptor from an archive component through the world's
/// `describe` export.
///
/// The loader's descriptor path is synchronous while component guests run on
/// the async engine, so the call is driven on a private current-thread runtime
/// on its own thread. That is safe from inside a Tokio worker (no nested
/// `block_on`) and from a plain thread alike; describe happens on install and
/// reload, never per invocation.
pub(crate) fn archive_component_describe(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| {
                        format!("failed to start archive component describe runtime: {error}")
                    })?;
                runtime.block_on(describe_async(wasm))
            })
            .join()
            .map_err(|_| "archive component describe thread panicked".to_string())?
    })
}

async fn describe_async(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    let runtime = ArchiveComponentRuntime::new(engine::shared_async_engine(), wasm)?;
    let (wasi, stderr) = sandbox::build_component_describe_sandbox();
    let (mut store, plugin) = runtime
        .instantiate(wasi, None, DESCRIBE_TIMEOUT)
        .await
        .map_err(|error| {
            format!("failed to instantiate archive component for describe: {error:#}")
        })?;
    let descriptor_json = plugin.call_describe(&mut store).await.map_err(|error| {
        let denied = store.data().limits.memory_denied;
        let failure = error::classify_error(&error, denied);
        let stderr_tail = tail_of(&stderr);
        format!(
            "archive component describe failed ({:?}): {}{}",
            failure.kind,
            failure.detail,
            stderr_suffix(&stderr_tail)
        )
    })?;
    serde_json::from_slice::<PluginDescriptor>(&descriptor_json).map_err(|error| {
        format!("archive component describe returned invalid PluginDescriptor JSON: {error}")
    })
}

/// Compile (or reuse) the component off the async worker, with the same
/// preparation timeout the core-module path used.
async fn prepare_archive_component(
    wasm: Arc<Vec<u8>>,
    timeout: Duration,
) -> Result<ArchiveComponentRuntime, String> {
    let prepare = tokio::task::spawn_blocking(move || {
        ArchiveComponentRuntime::new(engine::shared_async_engine(), &wasm)
    });
    match tokio::time::timeout(timeout, prepare).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!(
            "archive component preparation task failed: {error}"
        )),
        Err(_) => Err(format!(
            "timed out waiting for archive component rehydration after {} ms",
            timeout.as_millis()
        )),
    }
}

/// Instantiate the archive component and run one request→response exchange.
pub(crate) async fn process_archive_component(
    spec: &PluginInstanceSpec,
    request_json: &str,
    invocation: ArchiveInvocation<'_>,
) -> AppResult<ArchivePluginProcessResponse> {
    let span = tracing::info_span!(
        "archive_plugin_invoke",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
    );
    let _enter = span.enter();

    let started = Instant::now();
    let request_bytes = request_json.as_bytes().to_vec();
    let request_len = request_bytes.len();

    let runtime = prepare_archive_component(Arc::clone(&spec.wasm), spec.timeout)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "archive extractor plugin {}@{} failed to prepare: {error}",
                invocation.plugin_id, invocation.plugin_version
            ))
        })?;

    let PreparedComponentSandbox {
        wasi,
        stdout: _stdout,
        stderr,
        _scratch,
    } = sandbox::build_component_sandbox(&spec.preopens)?;

    let (mut store, plugin) = match runtime
        .instantiate(wasi, spec.memory_max_bytes, spec.timeout)
        .await
    {
        Ok(instantiated) => instantiated,
        Err(error) => {
            let failure = error::classify_error(&error, false);
            return Err(finish_error(
                &invocation,
                spec.timeout,
                &tail_of(&stderr),
                &failure,
                started,
                request_len,
            ));
        }
    };

    let call_result = plugin.call_process(&mut store, &request_bytes).await;
    let denied = store.data().limits.memory_denied;
    let stderr_tail = tail_of(&stderr);

    if !stderr_tail.is_empty() {
        tracing::debug!(
            target: "scryer_plugins::archive",
            plugin_id = invocation.plugin_id,
            stderr = stderr_tail.as_str(),
            "archive plugin stderr",
        );
    }

    let response_bytes = match call_result {
        Ok(Ok(response_bytes)) => response_bytes,
        Ok(Err(invocation_error)) => {
            let failure = error::protocol_failure(format!(
                "archive component reported {}",
                invocation_error_label(invocation_error)
            ));
            return Err(finish_error(
                &invocation,
                spec.timeout,
                &stderr_tail,
                &failure,
                started,
                request_len,
            ));
        }
        Err(error) => {
            let failure = error::classify_error(&error, denied);
            return Err(finish_error(
                &invocation,
                spec.timeout,
                &stderr_tail,
                &failure,
                started,
                request_len,
            ));
        }
    };

    if denied {
        let failure = error::classify_error(
            &wasmtime::Error::msg("guest exceeded the configured memory cap"),
            true,
        );
        return Err(finish_error(
            &invocation,
            spec.timeout,
            &stderr_tail,
            &failure,
            started,
            request_len,
        ));
    }

    let response: ArchivePluginProcessResponse = match serde_json::from_slice(&response_bytes) {
        Ok(response) => response,
        Err(error) => {
            let failure = error::protocol_failure(format!(
                "archive component returned invalid ArchivePluginProcessResponse JSON: {error}"
            ));
            return Err(finish_error(
                &invocation,
                spec.timeout,
                &stderr_tail,
                &failure,
                started,
                request_len,
            ));
        }
    };

    tracing::debug!(
        target: "scryer_plugins::archive",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        response_bytes = response_bytes.len(),
        disposition = "ok",
        "archive plugin invocation complete",
    );

    Ok(response)
}

const fn invocation_error_label(error: InvocationError) -> &'static str {
    match error {
        InvocationError::Failed => "failed",
        InvocationError::Cancelled => "cancelled",
        InvocationError::InvalidResponse => "invalid-response",
    }
}

fn finish_error(
    invocation: &ArchiveInvocation<'_>,
    budget: Duration,
    stderr_tail: &str,
    failure: &error::RunFailure,
    started: Instant,
    request_len: usize,
) -> AppError {
    tracing::debug!(
        target: "scryer_plugins::archive",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        disposition = ?failure.kind,
        "archive plugin invocation failed",
    );
    error::to_app_error(
        failure,
        &error::InvocationContext {
            plugin_id: invocation.plugin_id,
            plugin_version: invocation.plugin_version,
            operation: invocation.operation,
            budget,
            stderr_tail,
        },
    )
}

fn stderr_suffix(stderr_tail: &str) -> String {
    if stderr_tail.is_empty() {
        String::new()
    } else {
        format!("; guest stderr: {stderr_tail}")
    }
}

/// Size-capped, lossy tail of a captured output pipe.
fn tail_of(pipe: &MemoryOutputPipe) -> String {
    let bytes = pipe.contents();
    let start = bytes.len().saturating_sub(STDERR_TAIL_BYTES);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_plugin_sdk::{
        ArchivePluginOperation, ArchivePluginProcessRequest, ArchivePluginStatus,
    };

    /// Guest memory layout for the hand-built fixture component below.
    const DESCRIPTOR_PTR: usize = 0;
    const OK_RESPONSE_PTR: usize = 8192;
    const FAIL_RESPONSE_PTR: usize = 12288;
    const CRC_INPUT_PTR: usize = 16384;
    const AES_KEY_PTR: usize = 16400;
    const AES_IV_PTR: usize = 16416;
    const AES_CIPHERTEXT_PTR: usize = 16432;
    const DESCRIBE_RETURN_PTR: usize = 20480;
    const PROCESS_RETURN_PTR: usize = 20496;
    const AES_RETURN_PTR: usize = 20512;

    /// NIST SP 800-38A AES-128-CBC vector — the same one `crypto_host`'s unit
    /// tests use, so a mismatch means the WIT binding mangled the buffers
    /// rather than that the core is wrong.
    const AES_KEY_HEX: &str = "2b7e151628aed2a6abf7158809cf4f3c";
    const AES_IV_HEX: &str = "000102030405060708090a0b0c0d0e0f";
    const AES_CIPHERTEXT_HEX: &str =
        "7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b2";
    /// First eight plaintext bytes (`6bc1bee22e409f96`) as a little-endian i64.
    const AES_PLAINTEXT_HEAD_LE: u64 = 0x969f_402e_e2be_c16b;
    const CRC_CHECK_INPUT: &str = "123456789";
    const CRC_CHECK_VALUE: u32 = 0xcbf4_3926;

    fn hex_bytes(input: &str) -> Vec<u8> {
        (0..input.len())
            .step_by(2)
            .map(|idx| u8::from_str_radix(&input[idx..idx + 2], 16).expect("hex digit"))
            .collect()
    }

    /// WAT data-string escaping: every byte as `\xx`.
    fn wat_bytes(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("\\{byte:02x}")).collect()
    }

    fn archive_descriptor_json() -> String {
        let descriptor = PluginDescriptor {
            id: "fixture-archive".to_string(),
            name: "Fixture Archive".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions: Vec::new(),
            provider: scryer_plugin_sdk::ProviderDescriptor::ArchiveExtractor(
                scryer_plugin_sdk::ArchiveExtractorDescriptor {
                    provider_type: "archive-extraction".to_string(),
                    provider_aliases: Vec::new(),
                    config_fields: Vec::new(),
                    default_base_url: None,
                    allowed_hosts: Vec::new(),
                    capabilities: scryer_plugin_sdk::ArchiveExtractorCapabilities::default(),
                },
            ),
        };
        serde_json::to_string(&descriptor).expect("fixture descriptor must serialize")
    }

    /// A minimal but real `scryer:archive/archive-extractor@1.0.0` component.
    ///
    /// `describe` returns a static descriptor document. `process` gates its
    /// response on three facts that only a correctly wired host can make true:
    /// the request bytes arrived (first byte is `{`), `crc32` returned the IEEE
    /// check value, and `aes-cbc-decrypt` returned the NIST plaintext. It
    /// answers `status: ok` when all three hold and `status: failed` otherwise,
    /// so the assertion is about the binding, not about a trap.
    ///
    /// `enable_crypto` builds the same guest with the crypto import replaced by
    /// a local stub, which is how the "a component that never calls the host
    /// crypto still runs" case is covered.
    fn fixture_component_wat(descriptor_json: &str, ok_json: &str, fail_json: &str) -> String {
        let key = hex_bytes(AES_KEY_HEX);
        let iv = hex_bytes(AES_IV_HEX);
        let ciphertext = hex_bytes(AES_CIPHERTEXT_HEX);
        format!(
            r#"(component
  (import "scryer:archive/crypto@1.0.0" (instance $crypto
    (type (enum "bad-key-length" "bad-block-length" "bad-iv-length"))
    (export "aes-error" (type (eq 0)))
    (export "aes-cbc-decrypt" (func
      (param "key" (list u8))
      (param "iv" (list u8))
      (param "data" (list u8))
      (result (result (list u8) (error 1)))))
    (export "crc32" (func (param "seed" u32) (param "data" (list u8)) (result u32)))
  ))

  (type $ie (enum "failed" "cancelled" "invalid-response"))
  (export $ieX "invocation-error" (type $ie))
  (type $describe-ty (func (result (list u8))))
  (type $process-ty (func (param "request" (list u8))
    (result (result (list u8) (error $ieX)))))

  (core module $libc
    (memory (export "memory") 2)
    (global $bump (mut i32) (i32.const 32768))
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (local $ptr i32)
      (global.set $bump
        (i32.and (i32.add (global.get $bump) (i32.const 7)) (i32.const -8)))
      (local.set $ptr (global.get $bump))
      (global.set $bump (i32.add (global.get $bump) (local.get 3)))
      (local.get $ptr))
  )
  (core instance $libci (instantiate $libc))
  (alias core export $libci "memory" (core memory $mem))
  (alias core export $libci "cabi_realloc" (core func $realloc))

  (core func $crc_low (canon lower (func $crypto "crc32") (memory $mem)))
  (core func $aes_low
    (canon lower (func $crypto "aes-cbc-decrypt") (memory $mem) (realloc $realloc)))

  (core module $main
    (import "libc" "memory" (memory 2))
    (import "crypto" "crc32" (func $crc (param i32 i32 i32) (result i32)))
    (import "crypto" "aes" (func $aes (param i32 i32 i32 i32 i32 i32 i32)))
    (data (i32.const {descriptor_ptr}) "{descriptor}")
    (data (i32.const {ok_ptr}) "{ok}")
    (data (i32.const {fail_ptr}) "{fail}")
    (data (i32.const {crc_ptr}) "{crc_input}")
    (data (i32.const {key_ptr}) "{key}")
    (data (i32.const {iv_ptr}) "{iv}")
    (data (i32.const {ct_ptr}) "{ciphertext}")

    (func $respond (param $ptr i32) (param $len i32) (result i32)
      (i32.store8 (i32.const {process_ret}) (i32.const 0))
      (i32.store (i32.const {process_ret_ptr}) (local.get $ptr))
      (i32.store (i32.const {process_ret_len}) (local.get $len))
      (i32.const {process_ret}))

    (func $fail (result i32)
      (call $respond (i32.const {fail_ptr}) (i32.const {fail_len})))

    (func (export "describe") (result i32)
      (i32.store (i32.const {describe_ret}) (i32.const {descriptor_ptr}))
      (i32.store (i32.const {describe_ret_len}) (i32.const {descriptor_len}))
      (i32.const {describe_ret}))

    (func (export "process") (param $ptr i32) (param $len i32) (result i32)
      ;; The request must have crossed the boundary as JSON.
      (if (i32.eqz (local.get $len)) (then (return (call $fail))))
      (if (i32.ne (i32.load8_u (local.get $ptr)) (i32.const 123))
        (then (return (call $fail))))
      ;; Host CRC-32 must produce the IEEE check value.
      (if (i32.ne
            (call $crc (i32.const 0) (i32.const {crc_ptr}) (i32.const {crc_len}))
            (i32.const {crc_check}))
        (then (return (call $fail))))
      ;; Host AES-CBC must produce the NIST plaintext.
      (call $aes
        (i32.const {key_ptr}) (i32.const {key_len})
        (i32.const {iv_ptr}) (i32.const {iv_len})
        (i32.const {ct_ptr}) (i32.const {ct_len})
        (i32.const {aes_ret}))
      (if (i32.ne (i32.load8_u (i32.const {aes_ret})) (i32.const 0))
        (then (return (call $fail))))
      (if (i32.ne (i32.load (i32.const {aes_ret_len})) (i32.const {ct_len}))
        (then (return (call $fail))))
      (if (i64.ne
            (i64.load (i32.load (i32.const {aes_ret_ptr})))
            (i64.const {plaintext_head}))
        (then (return (call $fail))))
      (call $respond (i32.const {ok_ptr}) (i32.const {ok_len})))
  )
  (core instance $maini (instantiate $main
    (with "libc" (instance $libci))
    (with "crypto" (instance
      (export "crc32" (func $crc_low))
      (export "aes" (func $aes_low))))))

  (func (export "describe") (type $describe-ty)
    (canon lift (core func $maini "describe") (memory $mem) (realloc $realloc)))
  (func (export "process") (type $process-ty)
    (canon lift (core func $maini "process") (memory $mem) (realloc $realloc)))
)"#,
            descriptor_ptr = DESCRIPTOR_PTR,
            descriptor = descriptor_json.replace('"', "\\\""),
            descriptor_len = descriptor_json.len(),
            ok_ptr = OK_RESPONSE_PTR,
            ok = ok_json.replace('"', "\\\""),
            ok_len = ok_json.len(),
            fail_ptr = FAIL_RESPONSE_PTR,
            fail = fail_json.replace('"', "\\\""),
            fail_len = fail_json.len(),
            crc_ptr = CRC_INPUT_PTR,
            crc_input = CRC_CHECK_INPUT,
            crc_len = CRC_CHECK_INPUT.len(),
            crc_check = CRC_CHECK_VALUE,
            key_ptr = AES_KEY_PTR,
            key = wat_bytes(&key),
            key_len = key.len(),
            iv_ptr = AES_IV_PTR,
            iv = wat_bytes(&iv),
            iv_len = iv.len(),
            ct_ptr = AES_CIPHERTEXT_PTR,
            ciphertext = wat_bytes(&ciphertext),
            ct_len = ciphertext.len(),
            plaintext_head = AES_PLAINTEXT_HEAD_LE as i64,
            describe_ret = DESCRIBE_RETURN_PTR,
            describe_ret_len = DESCRIBE_RETURN_PTR + 4,
            process_ret = PROCESS_RETURN_PTR,
            process_ret_ptr = PROCESS_RETURN_PTR + 4,
            process_ret_len = PROCESS_RETURN_PTR + 8,
            aes_ret = AES_RETURN_PTR,
            aes_ret_ptr = AES_RETURN_PTR + 4,
            aes_ret_len = AES_RETURN_PTR + 8,
        )
    }

    fn fixture_component() -> Vec<u8> {
        let ok = r#"{"status":"ok","files":[{"relative_path":"movie.mkv","size":7}]}"#;
        let fail = r#"{"status":"failed","message":"host crypto binding mismatch"}"#;
        wat::parse_str(fixture_component_wat(&archive_descriptor_json(), ok, fail))
            .expect("fixture archive component WAT must assemble")
    }

    fn test_spec(wasm: Vec<u8>) -> PluginInstanceSpec {
        PluginInstanceSpec {
            wasm: Arc::new(wasm),
            preopens: Vec::new(),
            timeout: Duration::from_secs(30),
            memory_max_bytes: None,
            command_host: crate::wasmtime_host::command_host::CommandHost::disabled(),
        }
    }

    fn inspect_request_json() -> String {
        serde_json::to_string(&ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::Inspect {
                source_dir: "/scryer/source".to_string(),
                archive_path: None,
            },
        })
        .expect("request must serialize")
    }

    #[test]
    fn a_core_module_archive_artifact_fails_world_validation() {
        let core_module = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "_start")))"#,
        )
        .expect("core module WAT must parse");

        let error = validate_archive_component(&core_module)
            .expect_err("a core module must not validate as an archive component");
        assert!(
            error.contains("component") || error.contains("compile"),
            "{error}"
        );
    }

    #[test]
    fn an_arbitrary_component_fails_world_validation() {
        let wasm = wat::parse_str("(component)").expect("component WAT must parse");

        let error = validate_archive_component(&wasm)
            .expect_err("an arbitrary component must not pass archive-world validation");
        assert!(error.contains("exports do not match"), "{error}");
    }

    #[test]
    fn the_fixture_component_passes_world_validation() {
        validate_archive_component(&fixture_component())
            .expect("the fixture must satisfy scryer:archive/archive-extractor@1.0.0");
    }

    #[test]
    fn describe_returns_the_guest_descriptor() {
        let descriptor = archive_component_describe(&fixture_component())
            .expect("the fixture must self-describe through the world's describe export");

        assert_eq!(descriptor.id, "fixture-archive");
        assert!(matches!(
            descriptor.provider,
            scryer_plugin_sdk::ProviderDescriptor::ArchiveExtractor(_)
        ));
    }

    /// The end-to-end host path: the request crosses as JSON, both crypto
    /// imports are callable and numerically correct, and the response
    /// deserializes into the SDK type.
    #[tokio::test(flavor = "multi_thread")]
    async fn process_round_trips_json_and_serves_the_crypto_imports() {
        let spec = test_spec(fixture_component());
        let response = process_archive_component(
            &spec,
            &inspect_request_json(),
            ArchiveInvocation {
                plugin_id: "fixture-archive",
                plugin_version: "1.0.0",
                operation: "Inspect",
            },
        )
        .await
        .expect("the fixture component must complete one process exchange");

        assert_eq!(
            response.status,
            ArchivePluginStatus::Ok,
            "a non-ok status means the request or a crypto import did not arrive intact: {:?}",
            response.message
        );
        assert_eq!(response.files.len(), 1);
        assert_eq!(response.files[0].relative_path, "movie.mkv");
    }

    /// A `process` response that is not a `ArchivePluginProcessResponse` is a
    /// protocol failure, not a silent empty result.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_json_process_response_is_a_protocol_failure() {
        let wasm = wat::parse_str(fixture_component_wat(
            &archive_descriptor_json(),
            "not json at all",
            "not json either",
        ))
        .expect("fixture archive component WAT must assemble");
        let spec = test_spec(wasm);

        let error = process_archive_component(
            &spec,
            &inspect_request_json(),
            ArchiveInvocation {
                plugin_id: "fixture-archive",
                plugin_version: "1.0.0",
                operation: "Inspect",
            },
        )
        .await
        .expect_err("a malformed response document must fail the invocation");

        assert!(
            error.to_string().contains("ArchivePluginProcessResponse"),
            "{error}"
        );
    }
}
