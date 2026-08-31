use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use scryer_application::{AppError, AppResult};
use wasmtime::{Caller, ExternType, Instance, Linker, Store, ValType};
use wasmtime_wasi::{FsPerms, WasiCtxBuilder};

use crate::plugin_http_host::{
    HTTP_ENV_NAMESPACE, IndexerErrorCaptureContext, IndexerProxyPolicy, PluginHttpHost,
    PluginHttpRequest,
};
use crate::process_host::{PROCESS_HOST_NAMESPACE, ProcessHost};
use crate::runtime_backing::PreopenSpec;
use crate::socket_host::{SOCKET_HOST_NAMESPACE, SocketHost};
use crate::wasmtime_host::{engine, module_cache};

const DEFAULT_LEGACY_MEMORY_CAP_BYTES: usize = 512 * 1024 * 1024;
const DEFAULT_LEGACY_TABLE_ELEMENTS: usize = 1_000_000;
const GUEST_ARGV0: &str = "scryer-plugin";
const SCRYER_HTTP_NAMESPACE: &str = "scryer:host/http";

#[derive(Clone)]
pub(crate) struct LegacyPluginSpec {
    pub(crate) wasm: Arc<Vec<u8>>,
    pub(crate) timeout: Duration,
    pub(crate) memory_max_bytes: Option<usize>,
    pub(crate) exchange_memory_max_bytes: Option<usize>,
    pub(crate) var_store_max_bytes: Option<usize>,
    pub(crate) preopens: Vec<PreopenSpec>,
    pub(crate) allowed_hosts: Vec<String>,
    pub(crate) config: BTreeMap<String, String>,
    pub(crate) indexer_proxy_policy: Option<IndexerProxyPolicy>,
    pub(crate) destination_cooldown_key: Option<String>,
    pub(crate) socket_host: SocketHost,
    pub(crate) process_host: ProcessHost,
    pub(crate) plugin_id: String,
}

impl LegacyPluginSpec {
    pub(crate) fn new(wasm: Vec<u8>, plugin_id: impl Into<String>) -> Self {
        Self {
            wasm: Arc::new(wasm),
            timeout: Duration::from_secs(30),
            memory_max_bytes: None,
            exchange_memory_max_bytes: None,
            var_store_max_bytes: None,
            preopens: Vec::new(),
            allowed_hosts: Vec::new(),
            config: BTreeMap::new(),
            indexer_proxy_policy: None,
            destination_cooldown_key: None,
            socket_host: SocketHost::disabled(),
            process_host: ProcessHost::disabled(),
            plugin_id: plugin_id.into(),
        }
    }
}

pub(crate) struct LegacyPlugin {
    store: Store<LegacyHostState>,
    instance: Instance,
}

pub(crate) fn validate_legacy_module(wasm: &[u8], required_exports: &[&str]) -> Result<(), String> {
    let engine = engine::shared_engine();
    let module = module_cache::legacy_module(wasm)
        .map_err(|error| format!("failed to compile legacy plugin WASM: {error}"))?;

    let mut linker: Linker<LegacyHostState> = Linker::new(engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx: &mut LegacyHostState| &mut ctx.wasi)
        .map_err(|error| format!("failed to wire WASI preview1: {error:#}"))?;
    add_extism_compat_to_linker(&mut linker)
        .map_err(|error| format!("failed to wire Extism compatibility ABI: {error:#}"))?;
    linker
        .instantiate_pre(&module)
        .map_err(|error| format!("legacy plugin imports do not match the host ABI: {error:#}"))?;

    let invalid = required_exports
        .iter()
        .copied()
        .filter(|required| {
            !module.exports().any(|export| {
                export.name() == *required
                    && matches!(export.ty(), ExternType::Func(ref ty) if {
                        let mut params = ty.params();
                        let mut results = ty.results();
                        params.next().is_none()
                            && matches!(results.next(), Some(ValType::I32))
                            && results.next().is_none()
                    })
            })
        })
        .collect::<Vec<_>>();
    if invalid.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "legacy plugin is missing required () -> i32 export(s): {}",
            invalid.join(", ")
        ))
    }
}

impl LegacyPlugin {
    pub(crate) fn instantiate(spec: LegacyPluginSpec) -> AppResult<Self> {
        let engine = engine::shared_engine();
        let module = module_cache::legacy_module(&spec.wasm)
            .map_err(|error| AppError::Repository(format!("failed to compile WASM: {error}")))?;

        let mut linker: Linker<LegacyHostState> = Linker::new(engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx: &mut LegacyHostState| {
            &mut ctx.wasi
        })
        .map_err(|error| {
            AppError::Repository(format!("failed to wire WASI preview1: {error:#}"))
        })?;
        add_extism_compat_to_linker(&mut linker).map_err(|error| {
            AppError::Repository(format!(
                "failed to wire Extism compatibility ABI: {error:#}"
            ))
        })?;

        let wasi = build_legacy_wasi(&spec.preopens)?;
        let mut store = Store::new(engine, LegacyHostState::new(spec, wasi));
        store.limiter(|ctx: &mut LegacyHostState| &mut ctx.limits);
        let timeout = store.data().timeout;
        store.set_epoch_deadline(engine::deadline_ticks(timeout));

        let instance = linker.instantiate(&mut store, &module).map_err(|error| {
            AppError::Repository(format!("failed to instantiate WASM: {error:#}"))
        })?;

        initialize_reactor(&instance, &mut store)?;

        Ok(Self { store, instance })
    }

    pub(crate) fn function_exists(&mut self, export: &str) -> bool {
        self.instance.get_func(&mut self.store, export).is_some()
    }

    pub(crate) fn begin_indexer_error_capture(&mut self, context: IndexerErrorCaptureContext) {
        self.store.data().http.begin_indexer_error_capture(context);
    }

    pub(crate) fn finish_indexer_error_capture(&mut self, operation_failed: bool) {
        self.store
            .data()
            .http
            .finish_indexer_error_capture(operation_failed);
    }

    pub(crate) fn call_string(&mut self, export: &str, input: &str) -> AppResult<String> {
        self.call(export, Some(input.as_bytes()))
    }

    pub(crate) fn call_unit(&mut self, export: &str) -> AppResult<String> {
        self.call(export, None)
    }

    fn call(&mut self, export: &str, input: Option<&[u8]>) -> AppResult<String> {
        let func = self
            .instance
            .get_typed_func::<(), i32>(&mut self.store, export)
            .map_err(|error| {
                AppError::Repository(format!("plugin does not export {export}: {error:#}"))
            })?;

        self.store
            .data_mut()
            .reset_for_call(input.unwrap_or_default());
        let timeout = self.store.data().timeout;
        self.store
            .set_epoch_deadline(engine::deadline_ticks(timeout));

        let status = func.call(&mut self.store, ()).map_err(|error| {
            AppError::Repository(format!("plugin {export}() failed: {error:#}"))
        })?;

        if status != 0 {
            let state = self.store.data();
            let error = state
                .http
                .rate_limit_message(&state.plugin_id)
                .ok()
                .flatten()
                .or_else(|| state.error.clone())
                .unwrap_or_else(|| format!("plugin returned status {status}"));
            return Err(AppError::Repository(format!(
                "plugin {export}() failed: {error}"
            )));
        }

        let output = self.store.data().output_bytes().map_err(|error| {
            AppError::Repository(format!(
                "plugin {export}() produced invalid output: {error}"
            ))
        })?;
        String::from_utf8(output).map_err(|error| {
            AppError::Repository(format!(
                "plugin {export}() returned non-UTF-8 output: {error}"
            ))
        })
    }
}

fn initialize_reactor(instance: &Instance, store: &mut Store<LegacyHostState>) -> AppResult<()> {
    if let Ok(init) = instance.get_typed_func::<(), ()>(&mut *store, "__wasm_call_ctors") {
        init.call(store, ()).map_err(|error| {
            AppError::Repository(format!("failed to initialize WASM reactor: {error:#}"))
        })?;
        return Ok(());
    }
    if let Ok(init) = instance.get_typed_func::<(), ()>(&mut *store, "_initialize") {
        init.call(store, ()).map_err(|error| {
            AppError::Repository(format!("failed to initialize WASM reactor: {error:#}"))
        })?;
    }
    Ok(())
}

fn build_legacy_wasi(preopens: &[PreopenSpec]) -> AppResult<wasmtime_wasi::p1::WasiP1Ctx> {
    let mut builder = WasiCtxBuilder::new();
    builder.args(&[GUEST_ARGV0]);
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
                    "failed to preopen '{}' as '{}' for legacy plugin: {error}",
                    preopen.host_path.display(),
                    preopen.guest_path
                ))
            })?;
    }
    Ok(builder.build_p1())
}

struct LegacyHostState {
    wasi: wasmtime_wasi::p1::WasiP1Ctx,
    limits: LegacyLimits,
    memory: ExchangeMemory,
    input: Vec<u8>,
    output: Option<(u64, u64)>,
    error: Option<String>,
    vars: BTreeMap<String, Vec<u8>>,
    var_store_bytes: usize,
    var_store_max_bytes: usize,
    config: BTreeMap<String, String>,
    http: PluginHttpHost,
    socket_host: SocketHost,
    process_host: ProcessHost,
    plugin_id: String,
    timeout: Duration,
    call_started_at: Instant,
}

impl LegacyHostState {
    fn new(spec: LegacyPluginSpec, wasi: wasmtime_wasi::p1::WasiP1Ctx) -> Self {
        let exchange_memory_max_bytes = spec
            .exchange_memory_max_bytes
            .or(spec.memory_max_bytes)
            .unwrap_or(DEFAULT_LEGACY_MEMORY_CAP_BYTES);
        let var_store_max_bytes = spec
            .var_store_max_bytes
            .or(spec.exchange_memory_max_bytes)
            .or(spec.memory_max_bytes)
            .unwrap_or(DEFAULT_LEGACY_MEMORY_CAP_BYTES);
        Self {
            wasi,
            limits: LegacyLimits::new(spec.memory_max_bytes),
            memory: ExchangeMemory::new(exchange_memory_max_bytes),
            input: Vec::new(),
            output: None,
            error: None,
            vars: BTreeMap::new(),
            var_store_bytes: 0,
            var_store_max_bytes,
            config: spec.config,
            http: PluginHttpHost::new(
                spec.allowed_hosts,
                spec.indexer_proxy_policy,
                spec.destination_cooldown_key,
                spec.memory_max_bytes.map(|value| value as u64),
            ),
            socket_host: spec.socket_host,
            process_host: spec.process_host,
            plugin_id: spec.plugin_id,
            timeout: spec.timeout,
            call_started_at: Instant::now(),
        }
    }

    fn reset_for_call(&mut self, input: &[u8]) {
        self.memory.reset();
        self.input.clear();
        self.input.extend_from_slice(input);
        self.output = None;
        self.error = None;
        self.call_started_at = Instant::now();
    }

    fn time_remaining(&self) -> Duration {
        self.timeout
            .checked_sub(self.call_started_at.elapsed())
            .unwrap_or_default()
    }

    fn output_bytes(&self) -> Result<Vec<u8>, String> {
        let Some((offset, len)) = self.output else {
            return Ok(Vec::new());
        };
        self.memory.read(offset, len).map(|value| value.to_vec())
    }

    fn read_owned(&self, offset: i64) -> Result<Vec<u8>, String> {
        let offset = non_negative_offset(offset)?;
        let len = self.memory.length(offset);
        if len == 0 {
            return Ok(Vec::new());
        }
        self.memory.read(offset, len).map(|value| value.to_vec())
    }

    fn read_string_and_free(&mut self, offset: i64) -> Result<String, String> {
        let offset = non_negative_offset(offset)?;
        let bytes = self.read_owned(offset as i64)?;
        self.memory.free(offset);
        String::from_utf8(bytes).map_err(|error| error.to_string())
    }

    fn alloc_bytes(&mut self, bytes: &[u8]) -> Result<i64, String> {
        if bytes.is_empty() {
            return Ok(0);
        }
        self.memory.alloc_with(bytes).and_then(|offset| {
            i64::try_from(offset).map_err(|_| "exchange memory offset exceeds i64".to_string())
        })
    }

    fn remove_var(&mut self, key: &str) {
        if let Some(value) = self.vars.remove(key) {
            self.var_store_bytes = self
                .var_store_bytes
                .saturating_sub(var_entry_bytes(key, &value));
        }
    }

    fn set_var(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        let existing = self
            .vars
            .get(&key)
            .map(|value| var_entry_bytes(&key, value))
            .unwrap_or(0);
        let new_entry = var_entry_bytes(&key, &value);
        let next_bytes = self
            .var_store_bytes
            .checked_sub(existing)
            .and_then(|bytes| bytes.checked_add(new_entry))
            .ok_or_else(|| "legacy var store byte accounting overflow".to_string())?;
        if next_bytes > self.var_store_max_bytes {
            return Err(format!(
                "legacy var store exceeds budget: requested {next_bytes} bytes, limit {} bytes",
                self.var_store_max_bytes
            ));
        }
        self.vars.insert(key, value);
        self.var_store_bytes = next_bytes;
        Ok(())
    }
}

fn var_entry_bytes(key: &str, value: &[u8]) -> usize {
    key.len().saturating_add(value.len())
}

struct LegacyLimits {
    max_memory_bytes: usize,
    memory_denied: bool,
}

impl LegacyLimits {
    fn new(max_memory_bytes: Option<usize>) -> Self {
        Self {
            max_memory_bytes: max_memory_bytes.unwrap_or(DEFAULT_LEGACY_MEMORY_CAP_BYTES),
            memory_denied: false,
        }
    }
}

impl wasmtime::ResourceLimiter for LegacyLimits {
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
        Ok(desired <= DEFAULT_LEGACY_TABLE_ELEMENTS)
    }
}

struct ExchangeMemory {
    bytes: Vec<u8>,
    lengths: HashMap<u64, u64>,
    max_bytes: usize,
}

impl ExchangeMemory {
    fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::new(),
            lengths: HashMap::new(),
            max_bytes,
        }
    }

    fn reset(&mut self) {
        self.bytes.clear();
        self.lengths.clear();
        self.bytes.push(0);
    }

    fn alloc(&mut self, len: u64) -> Result<u64, String> {
        if len == 0 {
            return Ok(0);
        }
        let len = usize::try_from(len)
            .map_err(|_| "exchange memory allocation length exceeds usize".to_string())?;
        if self.bytes.is_empty() {
            self.bytes.push(0);
        }
        let offset = self.bytes.len() as u64;
        let new_len = self
            .bytes
            .len()
            .checked_add(len)
            .ok_or_else(|| "exchange memory allocation overflow".to_string())?;
        if new_len > self.max_bytes {
            return Err(format!(
                "exchange memory allocation exceeds budget: requested {new_len} bytes, limit {} bytes",
                self.max_bytes
            ));
        }
        self.bytes.resize(new_len, 0);
        self.lengths.insert(offset, len as u64);
        Ok(offset)
    }

    fn alloc_with(&mut self, data: &[u8]) -> Result<u64, String> {
        let offset = self.alloc(data.len() as u64)?;
        if offset != 0 {
            let start = usize::try_from(offset)
                .map_err(|_| "exchange memory offset exceeds usize".to_string())?;
            self.bytes[start..start + data.len()].copy_from_slice(data);
        }
        Ok(offset)
    }

    fn free(&mut self, offset: u64) {
        self.lengths.remove(&offset);
    }

    fn length(&self, offset: u64) -> u64 {
        self.lengths.get(&offset).copied().unwrap_or(0)
    }

    fn read(&self, offset: u64, len: u64) -> Result<&[u8], String> {
        let start = usize::try_from(offset)
            .map_err(|_| "exchange memory offset exceeds usize".to_string())?;
        let len =
            usize::try_from(len).map_err(|_| "exchange memory length exceeds usize".to_string())?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| "exchange memory offset overflow".to_string())?;
        self.bytes
            .get(start..end)
            .ok_or_else(|| format!("exchange memory range out of bounds: {offset}..{end}"))
    }

    fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), String> {
        let start = usize::try_from(offset)
            .map_err(|_| "exchange memory offset exceeds usize".to_string())?;
        let end = start
            .checked_add(data.len())
            .ok_or_else(|| "exchange memory offset overflow".to_string())?;
        let target = self
            .bytes
            .get_mut(start..end)
            .ok_or_else(|| format!("exchange memory range out of bounds: {offset}..{end}"))?;
        target.copy_from_slice(data);
        Ok(())
    }

    fn load_u8(&self, offset: u64) -> i32 {
        usize::try_from(offset)
            .ok()
            .and_then(|offset| self.bytes.get(offset).copied())
            .unwrap_or(0) as i32
    }

    fn load_u64(&self, offset: u64) -> i64 {
        let mut buf = [0_u8; 8];
        if let Ok(bytes) = self.read(offset, 8) {
            buf.copy_from_slice(bytes);
        }
        u64::from_le_bytes(buf) as i64
    }

    fn store_u8(&mut self, offset: u64, value: i32) {
        if let Ok(offset) = usize::try_from(offset)
            && let Some(slot) = self.bytes.get_mut(offset)
        {
            *slot = value as u8;
        }
    }

    fn store_u64(&mut self, offset: u64, value: i64) {
        let _ = self.write(offset, &(value as u64).to_le_bytes());
    }
}

fn add_extism_compat_to_linker(linker: &mut Linker<LegacyHostState>) -> wasmtime::Result<()> {
    linker.func_wrap(HTTP_ENV_NAMESPACE, "alloc", legacy_alloc)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "free", legacy_free)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "length", legacy_length)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "length_unsafe", legacy_length)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "load_u8", legacy_load_u8)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "load_u64", legacy_load_u64)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "store_u8", legacy_store_u8)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "store_u64", legacy_store_u64)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "input_length", legacy_input_length)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "input_load_u8", legacy_input_load_u8)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "input_load_u64", legacy_input_load_u64)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "output_set", legacy_output_set)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "error_set", legacy_error_set)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "config_get", legacy_config_get)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "var_get", legacy_var_get)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "var_set", legacy_var_set)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "http_request", legacy_http_request)?;
    linker.func_wrap(
        HTTP_ENV_NAMESPACE,
        "http_status_code",
        legacy_http_status_code,
    )?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "http_headers", legacy_http_headers)?;
    linker.func_wrap(
        SCRYER_HTTP_NAMESPACE,
        "scryer_http_request",
        legacy_http_request,
    )?;
    linker.func_wrap(
        SCRYER_HTTP_NAMESPACE,
        "scryer_http_status_code",
        legacy_http_status_code,
    )?;
    linker.func_wrap(
        SCRYER_HTTP_NAMESPACE,
        "scryer_http_headers",
        legacy_http_headers,
    )?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "log_trace", legacy_log_trace)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "log_debug", legacy_log_debug)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "log_info", legacy_log_info)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "log_warn", legacy_log_warn)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "log_error", legacy_log_error)?;
    linker.func_wrap(HTTP_ENV_NAMESPACE, "get_log_level", legacy_get_log_level)?;
    linker.func_wrap(
        SOCKET_HOST_NAMESPACE,
        "scryer_socket_open",
        legacy_socket_open,
    )?;
    linker.func_wrap(
        SOCKET_HOST_NAMESPACE,
        "scryer_socket_read",
        legacy_socket_read,
    )?;
    linker.func_wrap(
        SOCKET_HOST_NAMESPACE,
        "scryer_socket_write",
        legacy_socket_write,
    )?;
    linker.func_wrap(
        SOCKET_HOST_NAMESPACE,
        "scryer_socket_starttls",
        legacy_socket_starttls,
    )?;
    linker.func_wrap(
        SOCKET_HOST_NAMESPACE,
        "scryer_socket_close",
        legacy_socket_close,
    )?;
    linker.func_wrap(
        PROCESS_HOST_NAMESPACE,
        "scryer_process_exec",
        legacy_process_exec,
    )?;
    Ok(())
}

fn legacy_alloc(mut caller: Caller<'_, LegacyHostState>, len: i64) -> Result<i64, wasmtime::Error> {
    if len <= 0 {
        return Ok(0);
    }
    caller
        .data_mut()
        .memory
        .alloc(len as u64)
        .and_then(|offset| {
            i64::try_from(offset).map_err(|_| "exchange memory offset exceeds i64".to_string())
        })
        .map_err(wasmtime::Error::msg)
}

fn legacy_free(mut caller: Caller<'_, LegacyHostState>, offset: i64) {
    if let Ok(offset) = non_negative_offset(offset) {
        caller.data_mut().memory.free(offset);
    }
}

fn legacy_length(caller: Caller<'_, LegacyHostState>, offset: i64) -> i64 {
    non_negative_offset(offset)
        .map(|offset| caller.data().memory.length(offset) as i64)
        .unwrap_or(0)
}

fn legacy_load_u8(caller: Caller<'_, LegacyHostState>, offset: i64) -> i32 {
    non_negative_offset(offset)
        .map(|offset| caller.data().memory.load_u8(offset))
        .unwrap_or(0)
}

fn legacy_load_u64(caller: Caller<'_, LegacyHostState>, offset: i64) -> i64 {
    non_negative_offset(offset)
        .map(|offset| caller.data().memory.load_u64(offset))
        .unwrap_or(0)
}

fn legacy_store_u8(mut caller: Caller<'_, LegacyHostState>, offset: i64, value: i32) {
    if let Ok(offset) = non_negative_offset(offset) {
        caller.data_mut().memory.store_u8(offset, value);
    }
}

fn legacy_store_u64(mut caller: Caller<'_, LegacyHostState>, offset: i64, value: i64) {
    if let Ok(offset) = non_negative_offset(offset) {
        caller.data_mut().memory.store_u64(offset, value);
    }
}

fn legacy_input_length(caller: Caller<'_, LegacyHostState>) -> i64 {
    caller.data().input.len() as i64
}

fn legacy_input_load_u8(caller: Caller<'_, LegacyHostState>, offset: i64) -> i32 {
    non_negative_offset(offset)
        .ok()
        .and_then(|offset| usize::try_from(offset).ok())
        .and_then(|offset| caller.data().input.get(offset).copied())
        .unwrap_or(0) as i32
}

fn legacy_input_load_u64(caller: Caller<'_, LegacyHostState>, offset: i64) -> i64 {
    let mut buf = [0_u8; 8];
    if let Ok(offset) = non_negative_offset(offset).and_then(|offset| {
        usize::try_from(offset).map_err(|_| "input offset exceeds usize".to_string())
    }) {
        let start = offset;
        if let Some(bytes) = caller.data().input.get(start..start.saturating_add(8))
            && bytes.len() == 8
        {
            buf.copy_from_slice(bytes);
        }
    }
    u64::from_le_bytes(buf) as i64
}

fn legacy_output_set(
    mut caller: Caller<'_, LegacyHostState>,
    offset: i64,
    len: i64,
) -> Result<(), wasmtime::Error> {
    if let (Ok(offset), Ok(len)) = (non_negative_offset(offset), non_negative_offset(len)) {
        caller
            .data()
            .memory
            .read(offset, len)
            .map_err(wasmtime::Error::msg)?;
        caller.data_mut().output = Some((offset, len));
    }
    Ok(())
}

fn legacy_error_set(mut caller: Caller<'_, LegacyHostState>, offset: i64) {
    if let Ok(message) = caller.data_mut().read_string_and_free(offset) {
        caller.data_mut().error = Some(message);
    }
}

fn legacy_config_get(
    mut caller: Caller<'_, LegacyHostState>,
    offset: i64,
) -> Result<i64, wasmtime::Error> {
    let key = match caller.data_mut().read_string_and_free(offset) {
        Ok(key) => key,
        Err(_) => return Ok(0),
    };
    let value = caller.data().config.get(&key).cloned();
    match value {
        Some(value) => caller
            .data_mut()
            .alloc_bytes(value.as_bytes())
            .map_err(wasmtime::Error::msg),
        None => Ok(0),
    }
}

fn legacy_var_get(
    mut caller: Caller<'_, LegacyHostState>,
    offset: i64,
) -> Result<i64, wasmtime::Error> {
    let key = match caller.data_mut().read_string_and_free(offset) {
        Ok(key) => key,
        Err(_) => return Ok(0),
    };
    let value = caller.data().vars.get(&key).cloned();
    match value {
        Some(value) => caller
            .data_mut()
            .alloc_bytes(&value)
            .map_err(wasmtime::Error::msg),
        None => Ok(0),
    }
}

fn legacy_var_set(
    mut caller: Caller<'_, LegacyHostState>,
    key_offset: i64,
    value_offset: i64,
) -> Result<(), wasmtime::Error> {
    let key = match caller.data_mut().read_string_and_free(key_offset) {
        Ok(key) => key,
        Err(_) => return Ok(()),
    };
    if value_offset == 0 {
        caller.data_mut().remove_var(&key);
        return Ok(());
    }
    let value = match caller.data().read_owned(value_offset) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    if let Ok(offset) = non_negative_offset(value_offset) {
        caller.data_mut().memory.free(offset);
    }
    caller
        .data_mut()
        .set_var(key, value)
        .map_err(wasmtime::Error::msg)
}

fn legacy_http_request(
    mut caller: Caller<'_, LegacyHostState>,
    request_offset: i64,
    body_offset: i64,
) -> Result<i64, wasmtime::Error> {
    let request_bytes = caller
        .data()
        .read_owned(request_offset)
        .map_err(wasmtime::Error::msg)?;
    if let Ok(offset) = non_negative_offset(request_offset) {
        caller.data_mut().memory.free(offset);
    }
    let request: PluginHttpRequest =
        serde_json::from_slice(&request_bytes).map_err(wasmtime::Error::msg)?;
    let body = if body_offset > 0 {
        let body = caller
            .data()
            .read_owned(body_offset)
            .map_err(wasmtime::Error::msg)?;
        if let Ok(offset) = non_negative_offset(body_offset) {
            caller.data_mut().memory.free(offset);
        }
        Some(body)
    } else {
        None
    };
    let timeout = caller.data().time_remaining();
    let plugin_id = caller.data().plugin_id.clone();
    let output = caller
        .data()
        .http
        .request(&plugin_id, request, body, timeout)
        .map_err(wasmtime::Error::msg)?;
    caller
        .data_mut()
        .alloc_bytes(&output)
        .map_err(wasmtime::Error::msg)
}

fn legacy_http_status_code(caller: Caller<'_, LegacyHostState>) -> Result<i32, wasmtime::Error> {
    let plugin_id = caller.data().plugin_id.clone();
    caller
        .data()
        .http
        .status_code(&plugin_id)
        .map(|status| status as i32)
        .map_err(wasmtime::Error::msg)
}

fn legacy_http_headers(mut caller: Caller<'_, LegacyHostState>) -> Result<i64, wasmtime::Error> {
    let plugin_id = caller.data().plugin_id.clone();
    let Some(headers) = caller
        .data()
        .http
        .headers(&plugin_id)
        .map_err(wasmtime::Error::msg)?
    else {
        return Ok(0);
    };
    let json = serde_json::to_vec(&headers).map_err(wasmtime::Error::msg)?;
    caller
        .data_mut()
        .alloc_bytes(&json)
        .map_err(wasmtime::Error::msg)
}

fn legacy_log_trace(caller: Caller<'_, LegacyHostState>, offset: i64) {
    legacy_log(caller, offset, tracing::Level::TRACE);
}

fn legacy_log_debug(caller: Caller<'_, LegacyHostState>, offset: i64) {
    legacy_log(caller, offset, tracing::Level::DEBUG);
}

fn legacy_log_info(caller: Caller<'_, LegacyHostState>, offset: i64) {
    legacy_log(caller, offset, tracing::Level::INFO);
}

fn legacy_log_warn(caller: Caller<'_, LegacyHostState>, offset: i64) {
    legacy_log(caller, offset, tracing::Level::WARN);
}

fn legacy_log_error(caller: Caller<'_, LegacyHostState>, offset: i64) {
    legacy_log(caller, offset, tracing::Level::ERROR);
}

fn legacy_log(mut caller: Caller<'_, LegacyHostState>, offset: i64, level: tracing::Level) {
    let plugin_id = caller.data().plugin_id.clone();
    let Ok(message) = caller.data_mut().read_string_and_free(offset) else {
        return;
    };
    match level {
        tracing::Level::ERROR => tracing::error!(plugin = plugin_id, "{message}"),
        tracing::Level::WARN => tracing::warn!(plugin = plugin_id, "{message}"),
        tracing::Level::INFO => tracing::info!(plugin = plugin_id, "{message}"),
        tracing::Level::DEBUG => tracing::debug!(plugin = plugin_id, "{message}"),
        tracing::Level::TRACE => tracing::trace!(plugin = plugin_id, "{message}"),
    }
}

fn legacy_get_log_level(_caller: Caller<'_, LegacyHostState>) -> i32 {
    let level = tracing::level_filters::LevelFilter::current();
    if level == tracing::level_filters::LevelFilter::OFF {
        i32::MAX
    } else {
        match level.into_level().unwrap_or(tracing::Level::ERROR) {
            tracing::Level::TRACE => 0,
            tracing::Level::DEBUG => 1,
            tracing::Level::INFO => 2,
            tracing::Level::WARN => 3,
            tracing::Level::ERROR => 4,
        }
    }
}

fn legacy_socket_open(
    caller: Caller<'_, LegacyHostState>,
    offset: i64,
) -> Result<i64, wasmtime::Error> {
    legacy_user_host_call(caller, offset, |state, input| {
        state.socket_host.call("scryer_socket_open", input)
    })
}

fn legacy_socket_read(
    caller: Caller<'_, LegacyHostState>,
    offset: i64,
) -> Result<i64, wasmtime::Error> {
    legacy_user_host_call(caller, offset, |state, input| {
        state.socket_host.call("scryer_socket_read", input)
    })
}

fn legacy_socket_write(
    caller: Caller<'_, LegacyHostState>,
    offset: i64,
) -> Result<i64, wasmtime::Error> {
    legacy_user_host_call(caller, offset, |state, input| {
        state.socket_host.call("scryer_socket_write", input)
    })
}

fn legacy_socket_starttls(
    caller: Caller<'_, LegacyHostState>,
    offset: i64,
) -> Result<i64, wasmtime::Error> {
    legacy_user_host_call(caller, offset, |state, input| {
        state.socket_host.call("scryer_socket_starttls", input)
    })
}

fn legacy_socket_close(
    caller: Caller<'_, LegacyHostState>,
    offset: i64,
) -> Result<i64, wasmtime::Error> {
    legacy_user_host_call(caller, offset, |state, input| {
        state.socket_host.call("scryer_socket_close", input)
    })
}

fn legacy_process_exec(
    caller: Caller<'_, LegacyHostState>,
    offset: i64,
) -> Result<i64, wasmtime::Error> {
    legacy_user_host_call(caller, offset, |state, input| {
        state.process_host.call("scryer_process_exec", input)
    })
}

fn legacy_user_host_call(
    mut caller: Caller<'_, LegacyHostState>,
    offset: i64,
    call: impl FnOnce(&LegacyHostState, String) -> Result<String, String>,
) -> Result<i64, wasmtime::Error> {
    let input = caller
        .data_mut()
        .read_string_and_free(offset)
        .map_err(wasmtime::Error::msg)?;
    let output = call(caller.data(), input).map_err(wasmtime::Error::msg)?;
    caller
        .data_mut()
        .alloc_bytes(output.as_bytes())
        .map_err(wasmtime::Error::msg)
}

fn non_negative_offset(value: i64) -> Result<u64, String> {
    u64::try_from(value).map_err(|_| format!("negative offset: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn embedded_legacy_validation_uses_exact_host_abi() {
        let valid = wat::parse_str(
            r#"(module
                (func (export "scryer_describe") (result i32) (i32.const 0)))"#,
        )
        .unwrap();
        validate_legacy_module(&valid, &["scryer_describe"]).expect("valid legacy module");

        let unknown_import = wat::parse_str(
            r#"(module
                (import "extism:host/env" "not_a_host_function" (func))
                (func (export "scryer_describe") (result i32) (i32.const 0)))"#,
        )
        .unwrap();
        let error = validate_legacy_module(&unknown_import, &["scryer_describe"])
            .expect_err("unknown host import must fail");
        assert!(error.contains("imports do not match"), "{error}");

        let wrong_export = wat::parse_str(
            r#"(module
                (func (export "scryer_describe") (result i64) (i64.const 0)))"#,
        )
        .unwrap();
        let error = validate_legacy_module(&wrong_export, &["scryer_describe"])
            .expect_err("wrong export signature must fail");
        assert!(error.contains("() -> i32"), "{error}");
    }

    #[test]
    fn embedded_legacy_validation_uses_host_engine_features() {
        let threads = wat::parse_str(
            r#"(module
                (memory 1 1 shared)
                (func (export "scryer_describe") (result i32) (i32.const 0)))"#,
        )
        .unwrap();
        let error = validate_legacy_module(&threads, &["scryer_describe"])
            .expect_err("threads are disabled by the host engine");
        assert!(error.contains("compile"), "{error}");
    }

    fn instantiate_wat(wat: &str) -> LegacyPlugin {
        let wasm = wat::parse_str(wat).expect("wat parses");
        LegacyPlugin::instantiate(LegacyPluginSpec::new(wasm, "test-plugin"))
            .expect("legacy plugin instantiates")
    }

    fn instantiate_wat_with_exchange_cap(wat: &str, cap: usize) -> LegacyPlugin {
        let wasm = wat::parse_str(wat).expect("wat parses");
        let mut spec = LegacyPluginSpec::new(wasm, "test-plugin");
        spec.exchange_memory_max_bytes = Some(cap);
        LegacyPlugin::instantiate(spec).expect("legacy plugin instantiates")
    }

    fn instantiate_wat_with_var_cap(wat: &str, cap: usize) -> LegacyPlugin {
        let wasm = wat::parse_str(wat).expect("wat parses");
        let mut spec = LegacyPluginSpec::new(wasm, "test-plugin");
        spec.var_store_max_bytes = Some(cap);
        LegacyPlugin::instantiate(spec).expect("legacy plugin instantiates")
    }

    fn store_bytes_wat(pointer: &str, bytes: &[u8]) -> String {
        bytes
            .iter()
            .enumerate()
            .map(|(index, byte)| {
                format!(
                    "local.get ${pointer}\n\
                     i64.const {index}\n\
                     i64.add\n\
                     i32.const {byte}\n\
                     call $store_u8\n"
                )
            })
            .collect()
    }

    #[test]
    fn call_string_round_trips_extism_pdk_input_and_output_memory() {
        let mut plugin = instantiate_wat(
            r#"
            (module
              (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
              (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
              (import "extism:host/env" "input_length" (func $input_length (result i64)))
              (import "extism:host/env" "input_load_u8" (func $input_load_u8 (param i64) (result i32)))
              (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
              (func (export "echo") (result i32)
                (local $ptr i64)
                (local $len i64)
                (local $i i64)
                call $input_length
                local.set $len
                local.get $len
                call $alloc
                local.set $ptr
                (block $done
                  (loop $loop
                    local.get $i
                    local.get $len
                    i64.ge_u
                    br_if $done
                    local.get $ptr
                    local.get $i
                    i64.add
                    local.get $i
                    call $input_load_u8
                    call $store_u8
                    local.get $i
                    i64.const 1
                    i64.add
                    local.set $i
                    br $loop))
                local.get $ptr
                local.get $len
                call $output_set
                i32.const 0))
            "#,
        );

        let output = plugin.call_string("echo", "legacy-input").unwrap();
        assert_eq!(output, "legacy-input");
    }

    #[test]
    fn config_get_returns_config_values_from_legacy_spec() {
        let wasm = wat::parse_str(
            r#"
            (module
              (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
              (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
              (import "extism:host/env" "length" (func $length (param i64) (result i64)))
              (import "extism:host/env" "config_get" (func $config_get (param i64) (result i64)))
              (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
              (func (export "read_config") (result i32)
                (local $key i64)
                (local $value i64)
                i64.const 7
                call $alloc
                local.set $key
                local.get $key
                i32.const 97 ;; a
                call $store_u8
                local.get $key
                i64.const 1
                i64.add
                i32.const 112 ;; p
                call $store_u8
                local.get $key
                i64.const 2
                i64.add
                i32.const 105 ;; i
                call $store_u8
                local.get $key
                i64.const 3
                i64.add
                i32.const 95 ;; _
                call $store_u8
                local.get $key
                i64.const 4
                i64.add
                i32.const 107 ;; k
                call $store_u8
                local.get $key
                i64.const 5
                i64.add
                i32.const 101 ;; e
                call $store_u8
                local.get $key
                i64.const 6
                i64.add
                i32.const 121 ;; y
                call $store_u8
                local.get $key
                call $config_get
                local.set $value
                local.get $value
                local.get $value
                call $length
                call $output_set
                i32.const 0))
            "#,
        )
        .expect("wat parses");
        let mut spec = LegacyPluginSpec::new(wasm, "test-plugin");
        spec.config
            .insert("api_key".to_string(), "swordfish".to_string());
        let mut plugin = LegacyPlugin::instantiate(spec).expect("legacy plugin instantiates");

        let output = plugin.call_unit("read_config").unwrap();
        assert_eq!(output, "swordfish");
    }

    #[test]
    fn scryer_http_namespace_is_available_for_new_pdk_guests() {
        let mut plugin = instantiate_wat(
            r#"
            (module
              (import "scryer:host/http" "scryer_http_status_code" (func $status (result i32)))
              (import "scryer:host/http" "scryer_http_headers" (func $headers (result i64)))
              (func (export "probe") (result i32)
                call $status
                drop
                call $headers
                drop
                i32.const 0))
            "#,
        );

        let output = plugin.call_unit("probe").unwrap();
        assert_eq!(output, "");
    }

    #[test]
    fn scryer_http_request_uses_shared_plugin_http_host() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test HTTP server");
        let addr = listener.local_addr().expect("test HTTP server address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test HTTP request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).expect("read test HTTP request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n")
                    && request.ends_with(b"hello")
                {
                    break;
                }
            }
            let rendered = String::from_utf8_lossy(&request);
            assert!(rendered.starts_with("POST /probe HTTP/1.1"));
            assert!(rendered.contains("x-test: one"));
            assert!(rendered.ends_with("hello"));
            stream
                .write_all(
                    b"HTTP/1.1 201 Created\r\nX-Reply: yes\r\nContent-Length: 7\r\n\r\ncreated",
                )
                .expect("write test HTTP response");
        });

        let request_json = format!(
            r#"{{"url":"http://{addr}/probe","method":"POST","headers":{{"X-Test":"one"}}}}"#
        );
        let request_bytes = request_json.as_bytes();
        let body_bytes = b"hello";
        let request_stores = store_bytes_wat("request", request_bytes);
        let body_stores = store_bytes_wat("body", body_bytes);
        let wat = format!(
            r#"
            (module
              (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
              (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
              (import "extism:host/env" "length" (func $length (param i64) (result i64)))
              (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
              (import "scryer:host/http" "scryer_http_request" (func $http_request (param i64 i64) (result i64)))
              (import "scryer:host/http" "scryer_http_status_code" (func $status (result i32)))
              (import "scryer:host/http" "scryer_http_headers" (func $headers (result i64)))
              (func (export "run") (result i32)
                (local $request i64)
                (local $body i64)
                (local $response i64)
                i64.const {request_len}
                call $alloc
                local.set $request
                {request_stores}
                i64.const {body_len}
                call $alloc
                local.set $body
                {body_stores}
                local.get $request
                local.get $body
                call $http_request
                local.set $response
                call $status
                i32.const 201
                i32.ne
                if
                  i32.const 1
                  return
                end
                call $headers
                drop
                local.get $response
                local.get $response
                call $length
                call $output_set
                i32.const 0))
            "#,
            request_len = request_bytes.len(),
            request_stores = request_stores,
            body_len = body_bytes.len(),
            body_stores = body_stores,
        );
        let wasm = wat::parse_str(&wat).expect("wat parses");
        let mut spec = LegacyPluginSpec::new(wasm, "test-plugin");
        spec.allowed_hosts = vec!["127.0.0.1".to_string()];
        let mut plugin = LegacyPlugin::instantiate(spec).expect("legacy plugin instantiates");

        let output = plugin.call_unit("run").unwrap();
        assert_eq!(output, "created");
        assert_eq!(
            plugin.store.data().http.status_code("test-plugin").unwrap(),
            201
        );
        assert_eq!(
            plugin
                .store
                .data()
                .http
                .headers("test-plugin")
                .unwrap()
                .unwrap()
                .get("x-reply")
                .map(String::as_str),
            Some("yes")
        );
        server.join().expect("test HTTP server exits");
    }

    #[test]
    fn huge_alloc_fails_safely_against_exchange_budget() {
        let mut plugin = instantiate_wat_with_exchange_cap(
            r#"
            (module
              (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
              (func (export "too_big") (result i32)
                i64.const 128
                call $alloc
                drop
                i32.const 0))
            "#,
            16,
        );

        let err = plugin.call_unit("too_big").unwrap_err().to_string();
        assert!(err.contains("exchange memory allocation exceeds budget"));
    }

    #[test]
    fn over_budget_host_returned_config_fails_safely() {
        let wasm = wat::parse_str(
            r#"
            (module
              (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
              (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
              (import "extism:host/env" "config_get" (func $config_get (param i64) (result i64)))
              (func (export "read_config") (result i32)
                (local $key i64)
                i64.const 7
                call $alloc
                local.set $key
                local.get $key
                i32.const 97 ;; a
                call $store_u8
                local.get $key
                i64.const 1
                i64.add
                i32.const 112 ;; p
                call $store_u8
                local.get $key
                i64.const 2
                i64.add
                i32.const 105 ;; i
                call $store_u8
                local.get $key
                i64.const 3
                i64.add
                i32.const 95 ;; _
                call $store_u8
                local.get $key
                i64.const 4
                i64.add
                i32.const 107 ;; k
                call $store_u8
                local.get $key
                i64.const 5
                i64.add
                i32.const 101 ;; e
                call $store_u8
                local.get $key
                i64.const 6
                i64.add
                i32.const 121 ;; y
                call $store_u8
                local.get $key
                call $config_get
                drop
                i32.const 0))
            "#,
        )
        .expect("wat parses");
        let mut spec = LegacyPluginSpec::new(wasm, "test-plugin");
        spec.exchange_memory_max_bytes = Some(12);
        spec.config
            .insert("api_key".to_string(), "swordfish".to_string());
        let mut plugin = LegacyPlugin::instantiate(spec).expect("legacy plugin instantiates");

        let err = plugin.call_unit("read_config").unwrap_err().to_string();
        assert!(err.contains("exchange memory allocation exceeds budget"));
    }

    #[test]
    fn output_set_invalid_range_fails_safely() {
        let mut plugin = instantiate_wat(
            r#"
            (module
              (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
              (func (export "bad_output") (result i32)
                i64.const 99
                i64.const 1
                call $output_set
                i32.const 0))
            "#,
        );

        let err = plugin.call_unit("bad_output").unwrap_err().to_string();
        assert!(err.contains("exchange memory range out of bounds"));
    }

    #[test]
    fn var_set_cannot_accumulate_past_store_budget() {
        let mut plugin = instantiate_wat_with_var_cap(
            r#"
            (module
              (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
              (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
              (import "extism:host/env" "var_set" (func $var_set (param i64 i64)))
              (func $store_key (param $ptr i64) (param $byte i32)
                local.get $ptr
                local.get $byte
                call $store_u8)
              (func $store_value (param $ptr i64)
                local.get $ptr
                i32.const 120
                call $store_u8
                local.get $ptr
                i64.const 1
                i64.add
                i32.const 120
                call $store_u8
                local.get $ptr
                i64.const 2
                i64.add
                i32.const 120
                call $store_u8
                local.get $ptr
                i64.const 3
                i64.add
                i32.const 120
                call $store_u8)
              (func (export "fill_vars") (result i32)
                (local $key_a i64)
                (local $key_b i64)
                (local $value_a i64)
                (local $value_b i64)
                i64.const 1
                call $alloc
                local.set $key_a
                local.get $key_a
                i32.const 97
                call $store_key
                i64.const 4
                call $alloc
                local.set $value_a
                local.get $value_a
                call $store_value
                local.get $key_a
                local.get $value_a
                call $var_set
                i64.const 1
                call $alloc
                local.set $key_b
                local.get $key_b
                i32.const 98
                call $store_key
                i64.const 4
                call $alloc
                local.set $value_b
                local.get $value_b
                call $store_value
                local.get $key_b
                local.get $value_b
                call $var_set
                i32.const 0))
            "#,
            8,
        );

        let err = plugin.call_unit("fill_vars").unwrap_err().to_string();
        assert!(err.contains("legacy var store exceeds budget"));
    }

    #[test]
    fn var_set_removal_releases_store_budget() {
        let mut plugin = instantiate_wat_with_var_cap(
            r#"
            (module
              (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
              (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
              (import "extism:host/env" "var_set" (func $var_set (param i64 i64)))
              (func $key (param $byte i32) (result i64)
                (local $ptr i64)
                i64.const 1
                call $alloc
                local.set $ptr
                local.get $ptr
                local.get $byte
                call $store_u8
                local.get $ptr)
              (func $value (result i64)
                (local $ptr i64)
                i64.const 4
                call $alloc
                local.set $ptr
                local.get $ptr
                i32.const 120
                call $store_u8
                local.get $ptr
                i64.const 1
                i64.add
                i32.const 120
                call $store_u8
                local.get $ptr
                i64.const 2
                i64.add
                i32.const 120
                call $store_u8
                local.get $ptr
                i64.const 3
                i64.add
                i32.const 120
                call $store_u8
                local.get $ptr)
              (func (export "replace_after_remove") (result i32)
                i32.const 97
                call $key
                call $value
                call $var_set
                i32.const 97
                call $key
                i64.const 0
                call $var_set
                i32.const 98
                call $key
                call $value
                call $var_set
                i32.const 0))
            "#,
            8,
        );

        let output = plugin.call_unit("replace_after_remove").unwrap();
        assert_eq!(output, "");
    }
}
