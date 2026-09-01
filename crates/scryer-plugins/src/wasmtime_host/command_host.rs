//! Binary `scryer:host/v1` imports for native command guests.
//!
//! A host call creates a bounded response handle. Guests can obtain its length,
//! copy it into their own memory, then drop it. The command host starts
//! fail-closed: adapters opt services in only with their existing descriptor
//! policy and configuration rather than command artifacts inheriting ambient
//! WASI authority.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use scryer_application::{AppError, ArchiveExtractorPluginProvider};
use scryer_plugin_sdk::host::{
    HOST_ABI_MODULE, PluginArchiveExtractRequest, PluginArchiveExtractResponse,
    PluginArchiveExtractedFile, PluginConfigGetResponse, PluginHostRequest, PluginHostResponse,
    PluginHttpResponse, PluginStateGetResponse, PluginStateMutationResponse,
};
use scryer_plugin_sdk::{
    ArchivePluginFormat, ArchivePluginOperation, ArchivePluginProcessRequest, ArchivePluginStatus,
    PluginError, PluginErrorCode, PluginResult,
};
use wasmtime::{Caller, Linker, Memory};

use crate::plugin_http_host::{
    IndexerErrorCaptureContext, IndexerProxyPolicy, PluginHttpHost, PluginHttpRequest,
};
use crate::wasmtime_host::sandbox::HostCtx;

const MAX_RESPONSE_HANDLES: usize = 32;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
/// The encoded request may include a small postcard envelope in addition to a
/// bounded artifact payload. Reject it before copying guest memory.
pub(crate) const MAX_HOST_REQUEST_BYTES: usize = 17 * 1024 * 1024;
const MAX_STATE_BYTES: usize = 1024 * 1024;
/// The largest archive a guest may hand the host-owned extraction service.
const MAX_ARCHIVE_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;
/// The largest total payload the service returns, summed across members. This
/// sits under `MAX_RESPONSE_BYTES` so the postcard envelope still fits.
const MAX_ARCHIVE_RESPONSE_BYTES: usize = 15 * 1024 * 1024;
/// A bound on member count, so a zip bomb of empty entries cannot exhaust the
/// host through the per-member envelope alone.
const MAX_ARCHIVE_RESPONSE_FILES: usize = 4096;
const STAGED_ARCHIVE_OUTPUT_DIR: &str = "output";
#[derive(Clone)]
pub(crate) struct CommandHost {
    state: Arc<Mutex<CommandHostState>>,
    services: Option<Arc<CommandHostServices>>,
    request_deadline: Option<Instant>,
}

struct CommandHostState {
    next_handle: u32,
    responses: HashMap<u32, Vec<u8>>,
}

struct CommandHostServices {
    plugin_id: String,
    config: BTreeMap<String, String>,
    state: Mutex<CommandState>,
    http: PluginHttpHost,
    timeout: Duration,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
    runtime: Option<tokio::runtime::Handle>,
}

#[derive(Default)]
struct CommandState {
    values: BTreeMap<String, Vec<u8>>,
    bytes: usize,
}

impl CommandHost {
    pub(crate) fn disabled() -> Self {
        Self {
            state: Arc::new(Mutex::new(CommandHostState {
                next_handle: 1,
                responses: HashMap::new(),
            })),
            services: None,
            request_deadline: None,
        }
    }

    /// Build the host services for a command plugin whose egress needs no
    /// indexer-specific shaping — download clients, subtitle providers,
    /// notification channels, and the other legacy-ABI families.
    ///
    /// `archive_provider` backs the `ArchiveExtract` service. Every command
    /// family receives it when an extractor is installed: a container is a
    /// container regardless of which plugin found it, and the alternative is
    /// each family shipping its own decoder inside the sandbox. Archive
    /// extractors themselves stay on `disabled()` — they are the terminal
    /// delegation boundary.
    pub(crate) fn with_archive_provider(
        plugin_id: String,
        config: BTreeMap<String, String>,
        allowed_hosts: Vec<String>,
        timeout: Duration,
        max_http_response_bytes: Option<u64>,
        archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(CommandHostState {
                next_handle: 1,
                responses: HashMap::new(),
            })),
            services: Some(Arc::new(CommandHostServices {
                plugin_id,
                config,
                state: Mutex::new(CommandState::default()),
                http: PluginHttpHost::new(allowed_hosts, None, None, max_http_response_bytes),
                timeout,
                archive_provider,
                runtime: tokio::runtime::Handle::try_current().ok(),
            })),
            request_deadline: None,
        }
    }

    /// Build the host services for a command-ABI indexer.
    ///
    /// Indexers differ from download clients in exactly two ways, and both live
    /// in egress: a configured indexer proxy has to wrap every request, and the
    /// managed-destination cooldown key has to be carried so a shared upstream
    /// (a Prowlarr parent, say) throttles as one destination rather than once
    /// per child. Everything else — descriptor-bound config, plugin state, the
    /// timeout — is identical, so this mirrors `with_archive_provider` rather
    /// than growing it another two arguments that every caller passes `None` to.
    pub(crate) fn for_indexer(
        plugin_id: String,
        config: BTreeMap<String, String>,
        allowed_hosts: Vec<String>,
        indexer_proxy_policy: Option<IndexerProxyPolicy>,
        destination_cooldown_key: Option<String>,
        timeout: Duration,
        max_http_response_bytes: Option<u64>,
        archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(CommandHostState {
                next_handle: 1,
                responses: HashMap::new(),
            })),
            services: Some(Arc::new(CommandHostServices {
                plugin_id,
                config,
                state: Mutex::new(CommandState::default()),
                http: PluginHttpHost::new(
                    allowed_hosts,
                    indexer_proxy_policy,
                    destination_cooldown_key,
                    max_http_response_bytes,
                ),
                timeout,
                archive_provider,
                runtime: tokio::runtime::Handle::try_current().ok(),
            })),
            request_deadline: None,
        }
    }

    /// Clone this host for one command invocation and bind HTTP calls to the
    /// invocation's remaining wall-clock budget.
    pub(crate) fn for_invocation(&self, timeout: Duration) -> Self {
        let now = Instant::now();
        Self {
            state: Arc::clone(&self.state),
            services: self.services.clone(),
            request_deadline: Some(now.checked_add(timeout).unwrap_or(now)),
        }
    }

    fn remaining_http_timeout(&self, maximum: Duration) -> Result<Duration, String> {
        let Some(deadline) = self.request_deadline else {
            return Ok(maximum);
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("command plugin HTTP deadline exhausted".to_string());
        }
        Ok(remaining.min(maximum))
    }

    pub(crate) fn rate_limit_message(&self) -> Option<String> {
        let services = self.services.as_ref()?;
        services
            .http
            .rate_limit_message(&services.plugin_id)
            .ok()
            .flatten()
    }

    pub(crate) fn begin_indexer_error_capture(&self, context: IndexerErrorCaptureContext) {
        if let Some(services) = self.services.as_ref() {
            services.http.begin_indexer_error_capture(context);
        }
    }

    pub(crate) fn finish_indexer_error_capture(&self, operation_failed: bool) {
        if let Some(services) = self.services.as_ref() {
            services.http.finish_indexer_error_capture(operation_failed);
        }
    }

    /// Service one encoded host request and return the encoded response.
    ///
    /// This is the whole service layer as a pure byte exchange, with no
    /// response-handle bookkeeping. A core-module guest cannot receive a
    /// variable-length result across the raw pointer ABI and so goes through
    /// [`Self::call`]'s handle table; a component guest returns `list<u8>` by
    /// value, so `scryer:host/services@1.0.0`'s `host-call` binds straight to
    /// this. Both transports therefore speak the same postcard payload against
    /// the same services.
    pub(crate) fn call_bytes(&self, encoded_request: &[u8]) -> Result<Vec<u8>, String> {
        if encoded_request.len() > MAX_HOST_REQUEST_BYTES {
            return Err(format!(
                "encoded host request exceeds {MAX_HOST_REQUEST_BYTES} bytes"
            ));
        }
        let request: PluginHostRequest = postcard::from_bytes(encoded_request)
            .map_err(|error| format!("invalid postcard host request: {error}"))?;
        let response = self.service_request(request);
        let encoded = postcard::to_allocvec(&response)
            .map_err(|error| format!("failed to encode host response: {error}"))?;
        if encoded.len() > MAX_RESPONSE_BYTES {
            return Err(format!(
                "encoded host response exceeds {MAX_RESPONSE_BYTES} bytes"
            ));
        }
        Ok(encoded)
    }

    pub(crate) fn call(&self, encoded_request: &[u8]) -> Result<u32, String> {
        let encoded = self.call_bytes(encoded_request)?;
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("command host lock poisoned: {error}"))?;
        if state.responses.len() >= MAX_RESPONSE_HANDLES {
            return Err(format!(
                "command host response handle limit of {MAX_RESPONSE_HANDLES} reached"
            ));
        }
        let handle = state.next_handle;
        state.next_handle = state.next_handle.wrapping_add(1).max(1);
        state.responses.insert(handle, encoded);
        Ok(handle)
    }

    fn service_request(&self, request: PluginHostRequest) -> PluginHostResponse {
        let Some(services) = &self.services else {
            return unsupported_response(request);
        };
        match request {
            PluginHostRequest::ConfigGet(request) => {
                PluginHostResponse::ConfigGet(PluginResult::Ok(PluginConfigGetResponse {
                    value: services.config.get(&request.key).cloned(),
                }))
            }
            PluginHostRequest::StateGet(request) => {
                let result = services
                    .state
                    .lock()
                    .map_err(|error| error.to_string())
                    .map(|state| PluginStateGetResponse {
                        value: state.values.get(&request.key).cloned(),
                    });
                PluginHostResponse::StateGet(result.map_or_else(service_error, PluginResult::Ok))
            }
            PluginHostRequest::StateSet(request) => {
                let result = services
                    .state
                    .lock()
                    .map_err(|error| error.to_string())
                    .and_then(|mut state| set_state_value(&mut state, request.key, request.value));
                PluginHostResponse::StateSet(result.map_or_else(service_error, |changed| {
                    PluginResult::Ok(PluginStateMutationResponse { changed })
                }))
            }
            PluginHostRequest::StateDelete(request) => {
                let result = services
                    .state
                    .lock()
                    .map_err(|error| error.to_string())
                    .map(|mut state| {
                        let changed = state.values.remove(&request.key).is_some();
                        state.bytes = state
                            .values
                            .iter()
                            .map(|(key, value)| key.len() + value.len())
                            .sum();
                        changed
                    });
                PluginHostResponse::StateDelete(result.map_or_else(service_error, |changed| {
                    PluginResult::Ok(PluginStateMutationResponse { changed })
                }))
            }
            PluginHostRequest::Http(request) => {
                let response = self
                    .remaining_http_timeout(services.timeout)
                    .and_then(|timeout| {
                        services.http.request(
                            &services.plugin_id,
                            PluginHttpRequest {
                                url: request.url,
                                method: request.method,
                                headers: request.headers,
                            },
                            (!request.body.is_empty()).then_some(request.body),
                            timeout,
                        )
                    })
                    .and_then(|body| {
                        let status = services.http.status_code(&services.plugin_id)?;
                        Ok(PluginHttpResponse {
                            status,
                            headers: services
                                .http
                                .headers(&services.plugin_id)?
                                .unwrap_or_default(),
                            body,
                        })
                    });
                PluginHostResponse::Http(response.map_or_else(service_error, PluginResult::Ok))
            }
            PluginHostRequest::ReservedHttpBatch(_) => {
                PluginHostResponse::ReservedHttpBatch(unsupported())
            }
            PluginHostRequest::ArchiveExtract(request) => PluginHostResponse::ArchiveExtract(
                extract_archive(services, request)
                    .map_or_else(ArchiveExtractFailure::into_plugin_result, PluginResult::Ok),
            ),
            request => unsupported_response(request),
        }
    }

    pub(crate) fn response_len(&self, handle: u32) -> Option<usize> {
        self.state.lock().ok()?.responses.get(&handle).map(Vec::len)
    }

    pub(crate) fn response(&self, handle: u32) -> Option<Vec<u8>> {
        self.state.lock().ok()?.responses.get(&handle).cloned()
    }

    pub(crate) fn drop_response(&self, handle: u32) {
        if let Ok(mut state) = self.state.lock() {
            state.responses.remove(&handle);
        }
    }
}

fn set_state_value(state: &mut CommandState, key: String, value: Vec<u8>) -> Result<bool, String> {
    let prior = state
        .values
        .get(&key)
        .map(|value| key.len() + value.len())
        .unwrap_or(0);
    let next = state
        .bytes
        .checked_sub(prior)
        .and_then(|bytes| bytes.checked_add(key.len() + value.len()))
        .ok_or_else(|| "command plugin state byte accounting overflow".to_string())?;
    if next > MAX_STATE_BYTES {
        return Err(format!(
            "command plugin state exceeds {MAX_STATE_BYTES} bytes"
        ));
    }
    state.values.insert(key, value);
    state.bytes = next;
    Ok(true)
}

fn unsupported_error() -> PluginError {
    PluginError {
        code: PluginErrorCode::Unsupported,
        public_message: "this command plugin host service is not configured".to_string(),
        // `PluginError` predates the postcard host ABI and omits `None` fields
        // during serialization. Keep its optional fields present on this ABI.
        debug_message: Some(String::new()),
        retry_after_seconds: Some(0),
        details: None,
    }
}

fn unsupported<T>() -> PluginResult<T> {
    PluginResult::Err(unsupported_error())
}

fn service_error<T>(message: String) -> PluginResult<T> {
    PluginResult::Err(PluginError {
        code: PluginErrorCode::Temporary,
        public_message: "command plugin host service failed".to_string(),
        debug_message: Some(message),
        retry_after_seconds: Some(0),
        details: None,
    })
}

/// A typed failure of the host-owned archive-extraction service.
///
/// The service has three distinct outcomes a guest must be able to tell apart:
/// nothing installed can open this format (`Unsupported`, retrying is pointless
/// and the operator has to install an extractor), the request or the archive is
/// bad (`Permanent`, including a wrong or missing password), and everything
/// else (`Temporary`).
struct ArchiveExtractFailure {
    code: PluginErrorCode,
    message: String,
}

impl ArchiveExtractFailure {
    fn unsupported(message: impl Into<String>) -> Self {
        Self {
            code: PluginErrorCode::Unsupported,
            message: message.into(),
        }
    }

    fn permanent(message: impl Into<String>) -> Self {
        Self {
            code: PluginErrorCode::Permanent,
            message: message.into(),
        }
    }

    fn temporary(message: impl Into<String>) -> Self {
        Self {
            code: PluginErrorCode::Temporary,
            message: message.into(),
        }
    }

    fn from_app_error(error: AppError) -> Self {
        let code = match &error {
            AppError::ArchiveExtractionPluginRequired { .. } => PluginErrorCode::Unsupported,
            AppError::Validation(_) => PluginErrorCode::Permanent,
            _ => PluginErrorCode::Temporary,
        };
        Self {
            code,
            message: error.to_string(),
        }
    }

    fn into_plugin_result<T>(self) -> PluginResult<T> {
        PluginResult::Err(PluginError {
            code: self.code,
            public_message: self.message.clone(),
            debug_message: Some(self.message),
            retry_after_seconds: Some(0),
            details: None,
        })
    }
}

fn parse_archive_format(format: &str) -> Result<ArchivePluginFormat, ArchiveExtractFailure> {
    match format.trim().to_ascii_lowercase().as_str() {
        "rar" => Ok(ArchivePluginFormat::Rar),
        "zip" => Ok(ArchivePluginFormat::Zip),
        "7z" => Ok(ArchivePluginFormat::SevenZip),
        "xz" => Ok(ArchivePluginFormat::Xz),
        other => Err(ArchiveExtractFailure::permanent(format!(
            "'{other}' is not an archive format the host recognizes"
        ))),
    }
}

/// Name the staged archive so extractors that sniff by extension still work,
/// without letting a guest-supplied name escape the workspace.
fn staged_archive_name(filename: Option<&str>, format: ArchivePluginFormat) -> String {
    let extension = match format {
        ArchivePluginFormat::Rar => "rar",
        ArchivePluginFormat::Zip => "zip",
        ArchivePluginFormat::SevenZip => "7z",
        ArchivePluginFormat::Xz => "xz",
    };
    let candidate = filename
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .and_then(|name| std::path::Path::new(name).file_name())
        .and_then(|name| name.to_str())
        .filter(|name| {
            !name.is_empty()
                && *name != "."
                && *name != ".."
                && !name.contains(|ch: char| ch.is_control())
        });
    match candidate {
        Some(name) if std::path::Path::new(name).extension().is_some() => name.to_string(),
        Some(name) => format!("{name}.{extension}"),
        None => format!("archive.{extension}"),
    }
}

fn extract_archive(
    services: &CommandHostServices,
    request: PluginArchiveExtractRequest,
) -> Result<PluginArchiveExtractResponse, ArchiveExtractFailure> {
    if request.content.len() > MAX_ARCHIVE_ARTIFACT_BYTES {
        return Err(ArchiveExtractFailure::permanent(format!(
            "archive artifact exceeds {MAX_ARCHIVE_ARTIFACT_BYTES} bytes"
        )));
    }
    let format = parse_archive_format(&request.format)?;
    let client = services
        .archive_provider
        .as_ref()
        .and_then(|provider| provider.client_for_format(format))
        .ok_or_else(|| {
            ArchiveExtractFailure::unsupported(format!(
                "no installed archive extractor plugin handles '{}'",
                request.format
            ))
        })?;
    // A captured handle is the normal case; falling back to the ambient one
    // keeps the service working on hosts that build their CommandHost outside a
    // runtime context.
    let runtime = services
        .runtime
        .clone()
        .or_else(|| tokio::runtime::Handle::try_current().ok())
        .ok_or_else(|| {
            ArchiveExtractFailure::temporary("archive extraction runtime is unavailable")
        })?;

    // `TempDir` removes the workspace when it drops, which happens on every
    // path out of this function including the `?` returns below.
    let workspace = tempfile::tempdir().map_err(|error| {
        ArchiveExtractFailure::temporary(format!(
            "failed to create archive extraction workspace: {error}"
        ))
    })?;
    let archive_path = workspace
        .path()
        .join(staged_archive_name(request.filename.as_deref(), format));
    std::fs::write(&archive_path, &request.content).map_err(|error| {
        ArchiveExtractFailure::temporary(format!("failed to stage archive artifact: {error}"))
    })?;
    let output_dir = workspace.path().join(STAGED_ARCHIVE_OUTPUT_DIR);
    std::fs::create_dir_all(&output_dir).map_err(|error| {
        ArchiveExtractFailure::temporary(format!(
            "failed to create archive extraction output directory: {error}"
        ))
    })?;

    let response = runtime
        .block_on(client.process(ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::ExtractArchive {
                archive_path: archive_path.to_string_lossy().into_owned(),
                output_dir: output_dir.to_string_lossy().into_owned(),
                format,
                password: request.password,
            },
        }))
        .map_err(ArchiveExtractFailure::from_app_error)?;

    match response.status {
        ArchivePluginStatus::Ok => {}
        ArchivePluginStatus::UnsupportedFormat => {
            return Err(ArchiveExtractFailure::unsupported(format!(
                "the installed archive extractor does not support '{}'",
                request.format
            )));
        }
        ArchivePluginStatus::PasswordRequired => {
            return Err(ArchiveExtractFailure::permanent(
                "archive requires a password",
            ));
        }
        ArchivePluginStatus::PasswordInvalid => {
            return Err(ArchiveExtractFailure::permanent(
                "archive password is invalid",
            ));
        }
        ArchivePluginStatus::Failed => {
            return Err(ArchiveExtractFailure::temporary(
                response
                    .message
                    .or(response.error_code)
                    .unwrap_or_else(|| "archive extraction failed".to_string()),
            ));
        }
    }

    collect_extracted_files(&output_dir)
}

/// Read the extraction output back into a bounded response.
///
/// The walk stays inside `output_dir` by construction: it only descends into
/// entries whose own metadata says they are directories, and it refuses
/// symlinks outright rather than following them out of the workspace.
fn collect_extracted_files(
    output_dir: &std::path::Path,
) -> Result<PluginArchiveExtractResponse, ArchiveExtractFailure> {
    let mut files = Vec::new();
    let mut total_bytes = 0usize;
    let mut pending = vec![(output_dir.to_path_buf(), String::new())];

    while let Some((dir, prefix)) = pending.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|error| {
            ArchiveExtractFailure::temporary(format!(
                "failed to read archive extraction output: {error}"
            ))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                ArchiveExtractFailure::temporary(format!(
                    "failed to read archive extraction output entry: {error}"
                ))
            })?;
            let metadata = entry.metadata().map_err(|error| {
                ArchiveExtractFailure::temporary(format!(
                    "failed to inspect archive extraction output entry: {error}"
                ))
            })?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative_path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if metadata.is_dir() {
                pending.push((entry.path(), relative_path));
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            if files.len() >= MAX_ARCHIVE_RESPONSE_FILES {
                return Err(ArchiveExtractFailure::permanent(format!(
                    "archive expands to more than {MAX_ARCHIVE_RESPONSE_FILES} files"
                )));
            }
            let next_total = total_bytes.saturating_add(metadata.len() as usize);
            if next_total > MAX_ARCHIVE_RESPONSE_BYTES {
                return Err(ArchiveExtractFailure::permanent(format!(
                    "archive expands beyond {MAX_ARCHIVE_RESPONSE_BYTES} bytes"
                )));
            }
            let content = std::fs::read(entry.path()).map_err(|error| {
                ArchiveExtractFailure::temporary(format!(
                    "failed to read extracted file '{relative_path}': {error}"
                ))
            })?;
            total_bytes = total_bytes.saturating_add(content.len());
            if total_bytes > MAX_ARCHIVE_RESPONSE_BYTES {
                return Err(ArchiveExtractFailure::permanent(format!(
                    "archive expands beyond {MAX_ARCHIVE_RESPONSE_BYTES} bytes"
                )));
            }
            files.push(PluginArchiveExtractedFile {
                relative_path,
                content,
            });
        }
    }

    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(PluginArchiveExtractResponse { files })
}

fn unsupported_response(request: PluginHostRequest) -> PluginHostResponse {
    match request {
        PluginHostRequest::ConfigGet(_) => PluginHostResponse::ConfigGet(unsupported()),
        PluginHostRequest::StateGet(_) => PluginHostResponse::StateGet(unsupported()),
        PluginHostRequest::StateSet(_) => PluginHostResponse::StateSet(unsupported()),
        PluginHostRequest::StateDelete(_) => PluginHostResponse::StateDelete(unsupported()),
        PluginHostRequest::Http(_) => PluginHostResponse::Http(unsupported()),
        PluginHostRequest::ReservedHttpBatch(_) => {
            PluginHostResponse::ReservedHttpBatch(unsupported())
        }
        PluginHostRequest::SocketOpen(_) => PluginHostResponse::SocketOpen(unsupported()),
        PluginHostRequest::SocketRead(_) => PluginHostResponse::SocketRead(unsupported()),
        PluginHostRequest::SocketWrite(_) => PluginHostResponse::SocketWrite(unsupported()),
        PluginHostRequest::SocketStartTls(_) => PluginHostResponse::SocketStartTls(unsupported()),
        PluginHostRequest::SocketClose(_) => PluginHostResponse::SocketClose(unsupported()),
        PluginHostRequest::ProcessExec(_) => PluginHostResponse::ProcessExec(unsupported()),
        PluginHostRequest::ArchiveExtract(_) => PluginHostResponse::ArchiveExtract(unsupported()),
    }
}

pub(crate) fn add_to_linker(linker: &mut Linker<HostCtx>) -> wasmtime::Result<()> {
    linker.func_wrap_async(HOST_ABI_MODULE, "scryer_host_call", host_call)?;
    linker.func_wrap(
        HOST_ABI_MODULE,
        "scryer_host_response_len",
        host_response_len,
    )?;
    linker.func_wrap(
        HOST_ABI_MODULE,
        "scryer_host_response_read",
        host_response_read,
    )?;
    linker.func_wrap(
        HOST_ABI_MODULE,
        "scryer_host_response_drop",
        host_response_drop,
    )?;
    Ok(())
}

fn host_call(
    mut caller: Caller<'_, HostCtx>,
    (request_ptr, request_len): (i32, i32),
) -> Box<dyn std::future::Future<Output = i32> + Send + '_> {
    if usize::try_from(request_len)
        .ok()
        .is_none_or(|len| len > MAX_HOST_REQUEST_BYTES)
    {
        return Box::new(std::future::ready(0));
    }
    let request = read_memory(&mut caller, request_ptr, request_len);
    let command_host = caller.data().command_host.clone();
    Box::new(async move {
        let Ok(request) = request else {
            return 0;
        };
        tokio::task::spawn_blocking(move || command_host.call(&request))
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|handle| i32::try_from(handle).ok())
            .unwrap_or(0)
    })
}

fn host_response_len(caller: Caller<'_, HostCtx>, handle: i32) -> i32 {
    let Ok(handle) = u32::try_from(handle) else {
        return -1;
    };
    caller
        .data()
        .command_host
        .response_len(handle)
        .and_then(|len| i32::try_from(len).ok())
        .unwrap_or(-1)
}

fn host_response_read(
    mut caller: Caller<'_, HostCtx>,
    handle: i32,
    destination_ptr: i32,
    destination_len: i32,
) -> i32 {
    let Ok(handle) = u32::try_from(handle) else {
        return -1;
    };
    let Some(response) = caller.data().command_host.response(handle) else {
        return -1;
    };
    if response.len() > usize::try_from(destination_len).unwrap_or(0) {
        return -1;
    }
    if write_memory(&mut caller, destination_ptr, &response).is_err() {
        return -1;
    }
    i32::try_from(response.len()).unwrap_or(-1)
}

fn host_response_drop(mut caller: Caller<'_, HostCtx>, handle: i32) {
    if let Ok(handle) = u32::try_from(handle) {
        caller.data_mut().command_host.drop_response(handle);
    }
}

fn memory(caller: &mut Caller<'_, HostCtx>) -> Result<Memory, String> {
    caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
        .ok_or_else(|| "command plugin did not export memory".to_string())
}

fn checked_range(pointer: i32, len: i32) -> Result<(usize, usize), String> {
    let start = usize::try_from(pointer).map_err(|_| "negative memory pointer".to_string())?;
    let len = usize::try_from(len).map_err(|_| "negative memory length".to_string())?;
    let end = start
        .checked_add(len)
        .ok_or_else(|| "memory range overflow".to_string())?;
    Ok((start, end))
}

fn read_memory(
    caller: &mut Caller<'_, HostCtx>,
    pointer: i32,
    len: i32,
) -> Result<Vec<u8>, String> {
    let (start, end) = checked_range(pointer, len)?;
    let memory = memory(caller)?;
    if end > memory.data_size(&*caller) {
        return Err("memory range is out of bounds".to_string());
    }
    let mut bytes = vec![0; end - start];
    memory
        .read(&*caller, start, &mut bytes)
        .map_err(|error| format!("failed to read guest memory: {error}"))?;
    Ok(bytes)
}

fn write_memory(
    caller: &mut Caller<'_, HostCtx>,
    pointer: i32,
    bytes: &[u8],
) -> Result<(), String> {
    let start = usize::try_from(pointer).map_err(|_| "negative memory pointer".to_string())?;
    let end = start
        .checked_add(bytes.len())
        .ok_or_else(|| "memory range overflow".to_string())?;
    let memory = memory(caller)?;
    if end > memory.data_size(&*caller) {
        return Err("memory range is out of bounds".to_string());
    }
    memory
        .write(&mut *caller, start, bytes)
        .map_err(|error| format!("failed to write guest memory: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_http_budget_uses_only_remaining_time() {
        let host = CommandHost::disabled();
        assert_eq!(
            host.remaining_http_timeout(Duration::from_secs(5))
                .expect("unbound host uses its configured maximum"),
            Duration::from_secs(5)
        );

        let active = host.for_invocation(Duration::from_secs(30));
        let remaining = active
            .remaining_http_timeout(Duration::from_secs(5))
            .expect("active invocation retains a request budget");
        assert!(!remaining.is_zero());
        assert!(remaining <= Duration::from_secs(5));

        let expired = host.for_invocation(Duration::ZERO);
        assert_eq!(
            expired
                .remaining_http_timeout(Duration::from_secs(5))
                .unwrap_err(),
            "command plugin HTTP deadline exhausted"
        );
    }

    #[test]
    fn reserved_batch_slot_is_rejected_without_shifting_later_operations() {
        use scryer_plugin_sdk::host::{
            PluginConfigGetRequest, PluginHttpRequest, PluginProcessExecRequest,
            PluginStateDeleteRequest, PluginStateGetRequest, PluginStateSetRequest,
        };
        use scryer_plugin_sdk::{
            SocketCloseRequest, SocketOpenRequest, SocketReadRequest, SocketStartTlsRequest,
            SocketWriteRequest,
        };
        use serde::Serialize;

        #[derive(Serialize)]
        struct PreReleaseHttpBatchStartRate {
            starts: u32,
            interval_ms: u64,
        }

        #[derive(Serialize)]
        struct PreReleaseHttpBatchRequest {
            requests: Vec<PluginHttpRequest>,
            desired_start_rate: PreReleaseHttpBatchStartRate,
        }

        #[allow(dead_code)]
        #[derive(Serialize)]
        enum PreReleasePluginHostRequest {
            ConfigGet(PluginConfigGetRequest),
            StateGet(PluginStateGetRequest),
            StateSet(PluginStateSetRequest),
            StateDelete(PluginStateDeleteRequest),
            Http(PluginHttpRequest),
            HttpBatch(PreReleaseHttpBatchRequest),
            SocketOpen(SocketOpenRequest),
            SocketRead(SocketReadRequest),
            SocketWrite(SocketWriteRequest),
            SocketStartTls(SocketStartTlsRequest),
            SocketClose(SocketCloseRequest),
            ProcessExec(PluginProcessExecRequest),
        }

        let host = CommandHost::disabled();
        let batch = postcard::to_allocvec(&PreReleasePluginHostRequest::HttpBatch(
            PreReleaseHttpBatchRequest {
                requests: Vec::new(),
                desired_start_rate: PreReleaseHttpBatchStartRate {
                    starts: 1,
                    interval_ms: 1_000,
                },
            },
        ))
        .expect("pre-release batch request serializes");
        let handle = host
            .call(&batch)
            .expect("reserved request receives a response");
        let response: PluginHostResponse =
            postcard::from_bytes(&host.response(handle).expect("response is retained"))
                .expect("reserved response decodes");
        assert!(matches!(
            response,
            PluginHostResponse::ReservedHttpBatch(PluginResult::Err(PluginError {
                code: PluginErrorCode::Unsupported,
                ..
            }))
        ));

        let legacy_later_operation = postcard::to_allocvec(
            &PreReleasePluginHostRequest::SocketClose(SocketCloseRequest { handle: 1 }),
        )
        .expect("pre-release later request serializes");
        let handle = host
            .call(&legacy_later_operation)
            .expect("pre-release later request receives a response");
        let response: PluginHostResponse =
            postcard::from_bytes(&host.response(handle).expect("response is retained"))
                .expect("later response decodes");
        assert!(matches!(
            response,
            PluginHostResponse::SocketClose(PluginResult::Err(PluginError {
                code: PluginErrorCode::Unsupported,
                ..
            }))
        ));
    }

    #[test]
    fn disabled_host_returns_typed_unsupported_response() {
        let host = CommandHost::disabled();
        let request = postcard::to_allocvec(&PluginHostRequest::ConfigGet(
            scryer_plugin_sdk::host::PluginConfigGetRequest {
                key: "base_url".to_string(),
            },
        ))
        .unwrap();
        let handle = host.call(&request).unwrap();
        let response: PluginHostResponse =
            postcard::from_bytes(&host.response(handle).unwrap()).unwrap();
        assert!(matches!(
            response,
            PluginHostResponse::ConfigGet(PluginResult::Err(PluginError {
                code: PluginErrorCode::Unsupported,
                ..
            }))
        ));
    }

    #[test]
    fn host_rejects_oversized_encoded_request_before_decoding() {
        let request = vec![0; MAX_HOST_REQUEST_BYTES + 1];
        let error = CommandHost::disabled()
            .call(&request)
            .expect_err("oversized encoded request must be rejected");
        assert!(error.contains("encoded host request exceeds"), "{error}");
    }

    struct StubArchiveProvider {
        formats: Vec<ArchivePluginFormat>,
        outputs: Vec<(String, Vec<u8>)>,
        status: ArchivePluginStatus,
    }

    struct StubArchiveClient {
        outputs: Vec<(String, Vec<u8>)>,
        status: ArchivePluginStatus,
    }

    #[async_trait::async_trait]
    impl scryer_application::ArchiveExtractorClient for StubArchiveClient {
        async fn process(
            &self,
            request: ArchivePluginProcessRequest,
        ) -> Result<scryer_plugin_sdk::ArchivePluginProcessResponse, AppError> {
            let ArchivePluginOperation::ExtractArchive { output_dir, .. } = &request.operation
            else {
                panic!("host archive extraction must issue an ExtractArchive operation");
            };
            if matches!(self.status, ArchivePluginStatus::Ok) {
                for (relative_path, content) in &self.outputs {
                    let path = std::path::Path::new(output_dir).join(relative_path);
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).unwrap();
                    }
                    std::fs::write(path, content).unwrap();
                }
            }
            Ok(scryer_plugin_sdk::ArchivePluginProcessResponse {
                status: self.status,
                files: Vec::new(),
                expanded_bytes: None,
                copied_bytes: None,
                staged_bytes: None,
                error_code: None,
                message: None,
            })
        }
    }

    impl ArchiveExtractorPluginProvider for StubArchiveProvider {
        fn client_for_format(
            &self,
            format: ArchivePluginFormat,
        ) -> Option<Arc<dyn scryer_application::ArchiveExtractorClient>> {
            self.formats.contains(&format).then(|| {
                Arc::new(StubArchiveClient {
                    outputs: self.outputs.clone(),
                    status: self.status,
                }) as Arc<dyn scryer_application::ArchiveExtractorClient>
            })
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["stub".to_string()]
        }
    }

    fn host_with_stub_provider(provider: StubArchiveProvider) -> CommandHost {
        CommandHost::with_archive_provider(
            "archive-host-test".to_string(),
            BTreeMap::new(),
            Vec::new(),
            Duration::from_secs(5),
            None,
            Some(Arc::new(provider)),
        )
    }

    fn archive_request(format: &str) -> PluginHostRequest {
        PluginHostRequest::ArchiveExtract(PluginArchiveExtractRequest {
            content: b"not really an archive".to_vec(),
            format: format.to_string(),
            filename: Some("subs.zip".to_string()),
            password: None,
        })
    }

    #[tokio::test]
    async fn archive_host_service_returns_bounded_extracted_files() {
        let host = host_with_stub_provider(StubArchiveProvider {
            formats: vec![ArchivePluginFormat::Zip],
            outputs: vec![
                ("Show.S01E17.eng.srt".to_string(), b"hello".to_vec()),
                ("nested/Show.S01E17.spa.srt".to_string(), b"hola".to_vec()),
            ],
            status: ArchivePluginStatus::Ok,
        });
        let response =
            tokio::task::spawn_blocking(move || host.service_request(archive_request("zip")))
                .await
                .expect("archive host task completes");

        let PluginHostResponse::ArchiveExtract(PluginResult::Ok(response)) = response else {
            panic!("archive host service did not return a successful typed response");
        };
        let paths = response
            .files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["Show.S01E17.eng.srt", "nested/Show.S01E17.spa.srt"]);
        assert_eq!(response.files[0].content, b"hello");
        assert_eq!(response.files[1].content, b"hola");
    }

    #[tokio::test]
    async fn archive_host_service_rejects_an_oversized_artifact() {
        let host = host_with_stub_provider(StubArchiveProvider {
            formats: vec![ArchivePluginFormat::Zip],
            outputs: Vec::new(),
            status: ArchivePluginStatus::Ok,
        });
        let response = tokio::task::spawn_blocking(move || {
            host.service_request(PluginHostRequest::ArchiveExtract(
                PluginArchiveExtractRequest {
                    content: vec![0; MAX_ARCHIVE_ARTIFACT_BYTES + 1],
                    format: "zip".to_string(),
                    filename: None,
                    password: None,
                },
            ))
        })
        .await
        .expect("archive host task completes");

        let PluginHostResponse::ArchiveExtract(PluginResult::Err(error)) = response else {
            panic!("oversized archive artifact must be rejected");
        };
        assert_eq!(error.code, PluginErrorCode::Permanent);
        assert!(
            error.public_message.contains("exceeds"),
            "{}",
            error.public_message
        );
    }

    #[tokio::test]
    async fn archive_host_service_reports_unsupported_formats_as_unsupported() {
        let host = host_with_stub_provider(StubArchiveProvider {
            formats: vec![ArchivePluginFormat::Zip],
            outputs: Vec::new(),
            status: ArchivePluginStatus::Ok,
        });
        let response =
            tokio::task::spawn_blocking(move || host.service_request(archive_request("rar")))
                .await
                .expect("archive host task completes");

        let PluginHostResponse::ArchiveExtract(PluginResult::Err(error)) = response else {
            panic!("an unhandled format must fail");
        };
        assert_eq!(error.code, PluginErrorCode::Unsupported);
    }

    #[tokio::test]
    async fn archive_host_service_surfaces_password_status_permanently() {
        let host = host_with_stub_provider(StubArchiveProvider {
            formats: vec![ArchivePluginFormat::Zip],
            outputs: Vec::new(),
            status: ArchivePluginStatus::PasswordRequired,
        });
        let response =
            tokio::task::spawn_blocking(move || host.service_request(archive_request("zip")))
                .await
                .expect("archive host task completes");

        let PluginHostResponse::ArchiveExtract(PluginResult::Err(error)) = response else {
            panic!("a password-required archive must fail");
        };
        assert_eq!(error.code, PluginErrorCode::Permanent);
        assert!(
            error.public_message.contains("password"),
            "{}",
            error.public_message
        );
    }

    #[test]
    fn host_without_an_archive_provider_reports_unsupported() {
        let host = CommandHost::with_archive_provider(
            "archive-host-test".to_string(),
            BTreeMap::new(),
            Vec::new(),
            Duration::from_secs(5),
            None,
            None,
        );
        let PluginHostResponse::ArchiveExtract(PluginResult::Err(error)) =
            host.service_request(archive_request("zip"))
        else {
            panic!("a host with no extractor installed must fail");
        };
        assert_eq!(error.code, PluginErrorCode::Unsupported);
    }

    #[test]
    fn disabled_host_rejects_archive_extraction() {
        let response = CommandHost::disabled().service_request(archive_request("xz"));

        assert!(matches!(
            response,
            PluginHostResponse::ArchiveExtract(PluginResult::Err(PluginError {
                code: PluginErrorCode::Unsupported,
                ..
            }))
        ));
    }

    #[test]
    fn staged_archive_name_never_escapes_the_workspace() {
        assert_eq!(
            staged_archive_name(Some("../../etc/passwd"), ArchivePluginFormat::Zip),
            "passwd.zip"
        );
        assert_eq!(
            staged_archive_name(Some("/tmp/subs.rar"), ArchivePluginFormat::Rar),
            "subs.rar"
        );
        assert_eq!(
            staged_archive_name(Some(".."), ArchivePluginFormat::Xz),
            "archive.xz"
        );
        assert_eq!(
            staged_archive_name(None, ArchivePluginFormat::SevenZip),
            "archive.7z"
        );
    }
}
