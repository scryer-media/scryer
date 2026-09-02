//! The shared host-service layer behind `scryer:host/services@1.0.0`.
//!
//! One postcard request in, one postcard response out — every family world's
//! `host-call` import binds straight to [`CommandHost::call_bytes`]. The host
//! starts fail-closed: adapters opt services in only with their existing
//! descriptor policy and configuration rather than artifacts inheriting ambient
//! WASI authority.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::plugin_http_host::{PluginHttpHost, PluginHttpRequest};
use crate::process_host::ProcessHost;
use crate::socket_host::{SocketCallError, SocketHost, socket_plugin_error};
use scryer_application::{AppError, ArchiveExtractorPluginProvider};
use scryer_plugin_sdk::host::{
    PluginArchiveExtractRequest, PluginArchiveExtractResponse, PluginArchiveExtractedFile,
    PluginConfigGetResponse, PluginHostRequest, PluginHostResponse, PluginHttpResponse,
    PluginStateGetResponse, PluginStateMutationResponse,
};
use scryer_plugin_sdk::{
    ArchivePluginFormat, ArchivePluginOperation, ArchivePluginProcessRequest, ArchivePluginStatus,
    PluginError, PluginErrorCode, PluginResult,
};

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
    services: Option<Arc<CommandHostServices>>,
    request_deadline: Option<Instant>,
}

struct CommandHostServices {
    plugin_id: String,
    config: BTreeMap<String, String>,
    state: Mutex<CommandState>,
    http: PluginHttpHost,
    timeout: Duration,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
    runtime: Option<tokio::runtime::Handle>,
    /// Raw sockets, held only by notification hosts.
    ///
    /// `None` — every other family — is not the same as a [`SocketHost`] with
    /// an empty permission set. `None` means this host has no socket service at
    /// all and answers `Unsupported`, exactly as it does for an uninstalled
    /// archive extractor; a present-but-empty host means the service exists and
    /// denied *this* request against the plugin's descriptor permissions. A
    /// guest can act on the difference, so the layer keeps it.
    sockets: Option<SocketHost>,
    /// Host process execution, held only by notification hosts, and gated a
    /// second time by the loader to first-party plugins. Same `None` reading as
    /// `sockets`.
    processes: Option<ProcessHost>,
}

#[derive(Default)]
struct CommandState {
    values: BTreeMap<String, Vec<u8>>,
    bytes: usize,
}

impl CommandHost {
    pub(crate) fn disabled() -> Self {
        Self {
            services: None,
            request_deadline: None,
        }
    }

    /// Build the host services for a command plugin whose egress needs no
    /// indexer-specific shaping — download clients, subtitle providers,
    /// notification channels, and every other non-indexer family.
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
            services: Some(Arc::new(CommandHostServices {
                plugin_id,
                config,
                state: Mutex::new(CommandState::default()),
                http: PluginHttpHost::new(allowed_hosts, None, None, max_http_response_bytes),
                timeout,
                archive_provider,
                runtime: tokio::runtime::Handle::try_current().ok(),
                sockets: None,
                processes: None,
            })),
            request_deadline: None,
        }
    }

    /// Build the host services for a download client, with the proxy the
    /// operator assigned to it (if any).
    ///
    /// This exists rather than a seventh positional argument on
    /// [`Self::with_archive_provider`] for the same reason
    /// [`Self::for_notification`] does: every other command family would pass
    /// `None`, and "who may have their egress rerouted" should be a question
    /// with one answer in the source — whoever calls this or
    /// [`Self::for_indexer`].
    ///
    /// The policy goes straight into [`PluginHttpHost`], which already knows
    /// how to branch a solver from a transport hop, so a download client gets
    /// the identical egress treatment an indexer does.
    pub(crate) fn for_download_client(
        plugin_id: String,
        config: BTreeMap<String, String>,
        allowed_hosts: Vec<String>,
        timeout: Duration,
        max_http_response_bytes: Option<u64>,
        archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
        proxy_policy: Option<crate::plugin_http_host::ProxyPolicy>,
    ) -> Self {
        Self {
            services: Some(Arc::new(CommandHostServices {
                plugin_id,
                config,
                state: Mutex::new(CommandState::default()),
                http: PluginHttpHost::new(
                    allowed_hosts,
                    proxy_policy,
                    None,
                    max_http_response_bytes,
                ),
                timeout,
                archive_provider,
                runtime: tokio::runtime::Handle::try_current().ok(),
                sockets: None,
                processes: None,
            })),
            request_deadline: None,
        }
    }

    /// Build the host services for a notification channel.
    ///
    /// Notifications are the one family that holds authority beyond HTTP: an
    /// SMTP notifier drives a raw TCP stream itself, and a first-party script
    /// notifier spawns an allowlisted executable. This constructor exists
    /// rather than two more `None` arguments on
    /// [`Self::with_archive_provider`] for the same reason [`Self::for_indexer`]
    /// does — every other caller would pass `None` — and so that "who can open a
    /// socket" is a question with exactly one answer in the source: whoever
    /// calls this.
    ///
    /// Both hosts are handed in already resolved: the loader decides what the
    /// descriptor's `socket_permissions` and process allowlist come to for this
    /// channel, and this layer only enforces that decision. Because both are
    /// cheap `Arc` clones over shared state, every invocation of the channel
    /// sees the same resolved permission set and the same socket handle table.
    // Pre-existing: eight arguments, one per authority this family holds. The
    // alternative is a parameter struct that exists only to satisfy the lint.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn for_notification(
        plugin_id: String,
        config: BTreeMap<String, String>,
        allowed_hosts: Vec<String>,
        timeout: Duration,
        max_http_response_bytes: Option<u64>,
        archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
        socket_host: SocketHost,
        process_host: ProcessHost,
    ) -> Self {
        Self {
            services: Some(Arc::new(CommandHostServices {
                plugin_id,
                config,
                state: Mutex::new(CommandState::default()),
                http: PluginHttpHost::new(allowed_hosts, None, None, max_http_response_bytes),
                timeout,
                archive_provider,
                runtime: tokio::runtime::Handle::try_current().ok(),
                sockets: Some(socket_host),
                processes: Some(process_host),
            })),
            request_deadline: None,
        }
    }

    /// Clone this host for one command invocation and bind HTTP calls to the
    /// invocation's remaining wall-clock budget.
    pub(crate) fn for_invocation(&self, timeout: Duration) -> Self {
        let now = Instant::now();
        Self {
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

    /// Service one encoded host request and return the encoded response.
    ///
    /// This is the whole service layer as a pure byte exchange. A component
    /// guest returns `list<u8>` by value, so `scryer:host/services@1.0.0`'s
    /// `host-call` binds straight to this — there is no response-handle
    /// bookkeeping left, because the raw pointer ABI that needed it is gone.
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

    /// Service one decoded host request against this host's configured services.
    ///
    /// The `match` is exhaustive on purpose — there is no catch-all arm. Every
    /// `PluginHostRequest` variant the SDK defines now has a service arm, so a
    /// new capability arriving in the SDK fails to compile here rather than
    /// silently reporting `Unsupported` for a service the host may well have.
    /// Fail-closed still holds for the services themselves: an arm whose
    /// service is absent answers `Unsupported` in-band.
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
            // The socket family. Each arm hands the request straight to the
            // `SocketHost` typed entry points, so the permission check, the
            // handle table, the 64 KiB per-call read/write bounds and the 2 MiB
            // total read bound live in one place. Those bounds sit far below the
            // host-call envelope's own caps, so no second limit is introduced
            // here.
            PluginHostRequest::SocketOpen(request) => PluginHostResponse::SocketOpen(
                socket_result(services.sockets.as_ref().map(|host| host.open(request))),
            ),
            PluginHostRequest::SocketRead(request) => PluginHostResponse::SocketRead(
                socket_result(services.sockets.as_ref().map(|host| host.read(request))),
            ),
            PluginHostRequest::SocketWrite(request) => PluginHostResponse::SocketWrite(
                socket_result(services.sockets.as_ref().map(|host| host.write(request))),
            ),
            PluginHostRequest::SocketStartTls(request) => PluginHostResponse::SocketStartTls(
                socket_result(services.sockets.as_ref().map(|host| host.starttls(request))),
            ),
            PluginHostRequest::SocketClose(request) => PluginHostResponse::SocketClose(
                socket_result(services.sockets.as_ref().map(|host| host.close(request))),
            ),
            PluginHostRequest::ProcessExec(request) => PluginHostResponse::ProcessExec(
                match services.processes.as_ref().map(|host| host.exec(request)) {
                    // No process service on this host at all.
                    None => unsupported(),
                    Some(Ok(result)) => result,
                    // A poisoned host lock is a host fault, not an answer about
                    // the command, so it reports the way every other service
                    // failure in this layer does.
                    Some(Err(message)) => service_error(message),
                },
            ),
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

/// Fold one typed socket call into the host-call response envelope.
///
/// Three outcomes, three different statements to the guest: `None` — this host
/// has no socket service — is the fail-closed `Unsupported` every unconfigured
/// service in this layer gives; a [`SocketError`] is the socket layer's own
/// answer, projected onto `PluginError` by [`socket_plugin_error`] with its
/// original code preserved in `debug_message`; and a poisoned lock is a host
/// fault reported as a temporary service failure.
fn socket_result<T>(outcome: Option<Result<T, SocketCallError>>) -> PluginResult<T> {
    match outcome {
        None => unsupported(),
        Some(Ok(value)) => PluginResult::Ok(value),
        Some(Err(SocketCallError::Socket(error))) => PluginResult::Err(socket_plugin_error(&error)),
        Some(Err(SocketCallError::Poisoned(message))) => service_error(message),
    }
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
        let response: PluginHostResponse = postcard::from_bytes(
            &host
                .call_bytes(&batch)
                .expect("reserved request receives a response"),
        )
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
        let response: PluginHostResponse = postcard::from_bytes(
            &host
                .call_bytes(&legacy_later_operation)
                .expect("pre-release later request receives a response"),
        )
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
        let response: PluginHostResponse =
            postcard::from_bytes(&host.call_bytes(&request).unwrap()).unwrap();
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
            .call_bytes(&request)
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
        assert_eq!(
            paths,
            vec!["Show.S01E17.eng.srt", "nested/Show.S01E17.spa.srt"]
        );
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

    // ------------------------------------------------------------------
    // Socket and process service arms.
    //
    // These are the arms that carry authority no other family holds, so the
    // assertions are about *parity with the legacy pointer ABI*, not merely
    // about the arms answering: the same denial, the same message, the same
    // handle table.
    // ------------------------------------------------------------------

    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

    use crate::process_host::ProcessHost;
    use crate::socket_host::SocketHost;
    use scryer_plugin_sdk::host::PluginProcessExecRequest;
    use scryer_plugin_sdk::{
        NotificationCapabilities, NotificationDescriptor, PluginDescriptor, ProviderDescriptor,
        SocketCloseRequest, SocketOpenRequest, SocketPermission, SocketReadRequest, SocketTlsMode,
        SocketWriteRequest,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const SOCKET_PROBE: &[u8] = b"EHLO scryer\r\n";
    const SOCKET_REPLY: &[u8] = b"250 OK\r\n";

    fn notification_descriptor(
        socket_permissions: Vec<SocketPermission>,
        requires_host_process: bool,
    ) -> PluginDescriptor {
        PluginDescriptor {
            id: "socket-host-test".to_string(),
            name: "Socket Host Test".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions,
            provider: ProviderDescriptor::Notification(NotificationDescriptor {
                provider_type: "socket-host-test".to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities: NotificationCapabilities {
                    requires_host_process,
                    ..Default::default()
                },
            }),
        }
    }

    fn loopback_permission(port: u16) -> SocketPermission {
        SocketPermission {
            host_pattern: "127.0.0.1".to_string(),
            ports: vec![port],
            tls_modes: vec![SocketTlsMode::Plain],
        }
    }

    fn echo_listener() -> (u16, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener must bind");
        let port = listener
            .local_addr()
            .expect("listener must report its address")
            .port();
        let handle = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut probe = vec![0_u8; SOCKET_PROBE.len()];
            let _ = stream.read_exact(&mut probe);
            let _ = stream.write_all(SOCKET_REPLY);
            let _ = stream.flush();
            std::thread::sleep(Duration::from_millis(250));
        });
        (port, handle)
    }

    fn notification_host(descriptor: &PluginDescriptor, config_json: Option<&str>) -> CommandHost {
        CommandHost::for_notification(
            descriptor.id.clone(),
            BTreeMap::new(),
            Vec::new(),
            Duration::from_secs(5),
            None,
            None,
            SocketHost::from_descriptor(descriptor, config_json),
            ProcessHost::from_descriptor(descriptor, config_json),
        )
    }

    fn open_request(port: u16) -> PluginHostRequest {
        PluginHostRequest::SocketOpen(SocketOpenRequest {
            host: "127.0.0.1".to_string(),
            port,
            tls_mode: SocketTlsMode::Plain,
            connect_timeout_ms: Some(5_000),
            read_timeout_ms: Some(5_000),
            write_timeout_ms: Some(5_000),
        })
    }

    /// The whole socket lifecycle through the one host-call door: open a real
    /// loopback listener, write to it, read its answer back, close.
    #[test]
    fn socket_service_arms_drive_a_real_loopback_connection() {
        let (port, listener) = echo_listener();
        let descriptor = notification_descriptor(vec![loopback_permission(port)], false);
        let host = notification_host(&descriptor, None);

        let PluginHostResponse::SocketOpen(PluginResult::Ok(opened)) =
            host.service_request(open_request(port))
        else {
            panic!("a granted socket must open");
        };

        let PluginHostResponse::SocketWrite(PluginResult::Ok(written)) =
            host.service_request(PluginHostRequest::SocketWrite(SocketWriteRequest {
                handle: opened.handle,
                data_base64: BASE64.encode(SOCKET_PROBE),
            }))
        else {
            panic!("an open socket must accept a bounded write");
        };
        assert_eq!(written.bytes_written, SOCKET_PROBE.len());

        let PluginHostResponse::SocketRead(PluginResult::Ok(read)) =
            host.service_request(PluginHostRequest::SocketRead(SocketReadRequest {
                handle: opened.handle,
                max_bytes: 64,
            }))
        else {
            panic!("an open socket must read the listener reply back");
        };
        assert_eq!(read.data_base64, BASE64.encode(SOCKET_REPLY));
        assert!(!read.eof);

        let PluginHostResponse::SocketClose(PluginResult::Ok(closed)) =
            host.service_request(PluginHostRequest::SocketClose(SocketCloseRequest {
                handle: opened.handle,
            }))
        else {
            panic!("an open socket must close");
        };
        assert!(closed.closed);
        listener.join().ok();
    }

    /// A descriptor with no socket permissions is denied on the host-call door
    /// with the socket layer's own message, and the socket layer's own
    /// `SocketErrorCode` carried through in `debug_message` because
    /// `PluginError` has nowhere else to put it.
    #[test]
    fn a_descriptor_without_socket_permissions_is_denied_with_the_socket_layers_own_message() {
        let descriptor = notification_descriptor(Vec::new(), false);
        let socket_host = SocketHost::from_descriptor(&descriptor, None);
        let host = CommandHost::for_notification(
            descriptor.id.clone(),
            BTreeMap::new(),
            Vec::new(),
            Duration::from_secs(5),
            None,
            None,
            socket_host.clone(),
            ProcessHost::disabled(),
        );

        // The typed entry point the door calls, invoked directly, is the
        // reference answer: the door must not reshape it.
        let denial = socket_host
            .open(SocketOpenRequest {
                host: "127.0.0.1".to_string(),
                port: 25,
                tls_mode: scryer_plugin_sdk::SocketTlsMode::Plain,
                connect_timeout_ms: Some(1),
                read_timeout_ms: Some(1),
                write_timeout_ms: Some(1),
            })
            .expect_err("an ungranted socket must be denied");
        let SocketCallError::Socket(denial) = denial else {
            panic!("a permission denial is a socket answer, not a host fault");
        };

        let PluginHostResponse::SocketOpen(PluginResult::Err(error)) =
            host.service_request(open_request(25))
        else {
            panic!("an ungranted socket must be denied");
        };

        assert_eq!(
            error.public_message, denial.message,
            "the host-call door must report the socket layer's own message verbatim",
        );
        assert_eq!(
            error.code,
            PluginErrorCode::Permanent,
            "a permission denial is not an absent capability",
        );
        assert!(
            error
                .debug_message
                .as_deref()
                .is_some_and(|debug| debug.contains("permission_denied")),
            "{:?}",
            error.debug_message,
        );
    }

    /// A denial and an absent service are different answers, and a guest can
    /// tell them apart: `Unsupported` means no host here has sockets at all.
    #[test]
    fn a_host_without_the_socket_service_reports_unsupported() {
        let host = CommandHost::with_archive_provider(
            "no-sockets".to_string(),
            BTreeMap::new(),
            Vec::new(),
            Duration::from_secs(5),
            None,
            None,
        );

        for request in [
            open_request(25),
            PluginHostRequest::SocketRead(SocketReadRequest {
                handle: 1,
                max_bytes: 16,
            }),
            PluginHostRequest::SocketWrite(SocketWriteRequest {
                handle: 1,
                data_base64: String::new(),
            }),
            PluginHostRequest::SocketStartTls(scryer_plugin_sdk::SocketStartTlsRequest {
                handle: 1,
                host: "127.0.0.1".to_string(),
            }),
            PluginHostRequest::SocketClose(SocketCloseRequest { handle: 1 }),
        ] {
            let code = match host.service_request(request) {
                PluginHostResponse::SocketOpen(PluginResult::Err(error))
                | PluginHostResponse::SocketRead(PluginResult::Err(error))
                | PluginHostResponse::SocketWrite(PluginResult::Err(error))
                | PluginHostResponse::SocketStartTls(PluginResult::Err(error))
                | PluginHostResponse::SocketClose(PluginResult::Err(error)) => error.code,
                other => panic!("a host with no socket service must refuse: {other:?}"),
            };
            assert_eq!(code, PluginErrorCode::Unsupported);
        }
    }

    #[test]
    fn a_disabled_host_reports_unsupported_for_sockets_and_processes() {
        let host = CommandHost::disabled();

        assert!(matches!(
            host.service_request(open_request(25)),
            PluginHostResponse::SocketOpen(PluginResult::Err(PluginError {
                code: PluginErrorCode::Unsupported,
                ..
            }))
        ));
        assert!(matches!(
            host.service_request(PluginHostRequest::ProcessExec(PluginProcessExecRequest {
                command: "/bin/echo".to_string(),
                args: Vec::new(),
                env: BTreeMap::new(),
                cwd: None,
                stdin: Vec::new(),
                timeout_ms: None,
            })),
            PluginHostResponse::ProcessExec(PluginResult::Err(PluginError {
                code: PluginErrorCode::Unsupported,
                ..
            }))
        ));
    }

    /// STARTTLS is refused for a socket that was not opened in STARTTLS mode,
    /// through the host-call door exactly as through the legacy one — the check
    /// lives in the socket host, not in either transport.
    #[test]
    fn starttls_on_a_plain_socket_is_refused() {
        let (port, listener) = echo_listener();
        let descriptor = notification_descriptor(vec![loopback_permission(port)], false);
        let host = notification_host(&descriptor, None);

        let PluginHostResponse::SocketOpen(PluginResult::Ok(opened)) =
            host.service_request(open_request(port))
        else {
            panic!("a granted socket must open");
        };
        let PluginHostResponse::SocketStartTls(PluginResult::Err(error)) = host.service_request(
            PluginHostRequest::SocketStartTls(scryer_plugin_sdk::SocketStartTlsRequest {
                handle: opened.handle,
                host: "127.0.0.1".to_string(),
            }),
        ) else {
            panic!("STARTTLS on a plain-mode socket must be refused");
        };
        assert_eq!(error.code, PluginErrorCode::Permanent);
        drop(listener);
    }

    /// The process arm spawns through the same allowlist the legacy
    /// registration uses, and projects the legacy response onto the SDK shape.
    #[test]
    fn the_process_arm_runs_an_allowlisted_command() {
        let descriptor = notification_descriptor(Vec::new(), true);
        let host = notification_host(&descriptor, Some(r#"{"path":"/bin/echo"}"#));

        let PluginHostResponse::ProcessExec(PluginResult::Ok(response)) =
            host.service_request(PluginHostRequest::ProcessExec(PluginProcessExecRequest {
                command: "/bin/echo".to_string(),
                args: vec!["scryer".to_string()],
                env: BTreeMap::new(),
                cwd: None,
                stdin: Vec::new(),
                timeout_ms: Some(5_000),
            }))
        else {
            panic!("an allowlisted command must run");
        };
        assert_eq!(response.exit_code, 0);
        assert_eq!(response.stdout, b"scryer\n".to_vec());
        assert!(response.stderr.is_empty());
    }

    /// A command outside the descriptor's allowlist is denied with the legacy
    /// `permission_denied` code carried through, and never spawns.
    #[test]
    fn the_process_arm_denies_a_command_outside_the_allowlist() {
        let descriptor = notification_descriptor(Vec::new(), true);
        let host = notification_host(&descriptor, Some(r#"{"path":"/bin/echo"}"#));

        let PluginHostResponse::ProcessExec(PluginResult::Err(error)) =
            host.service_request(PluginHostRequest::ProcessExec(PluginProcessExecRequest {
                command: "/bin/sh".to_string(),
                args: vec!["-c".to_string(), "echo pwned".to_string()],
                env: BTreeMap::new(),
                cwd: None,
                stdin: Vec::new(),
                timeout_ms: Some(5_000),
            }))
        else {
            panic!("a command outside the allowlist must be denied");
        };
        assert_eq!(error.code, PluginErrorCode::Permanent);
        assert!(
            error
                .debug_message
                .as_deref()
                .is_some_and(|debug| debug.contains("permission_denied")),
            "{:?}",
            error.debug_message,
        );
    }

    /// A notification plugin that never declared `requires_host_process` — and
    /// every non-first-party one, which the loader hands `ProcessHost::disabled`
    /// — has an empty allowlist, so the arm exists and denies rather than
    /// reporting the service missing.
    #[test]
    fn a_notification_host_without_a_process_allowlist_denies_rather_than_reporting_unsupported() {
        let descriptor = notification_descriptor(Vec::new(), false);
        let host = notification_host(&descriptor, Some(r#"{"path":"/bin/echo"}"#));

        let PluginHostResponse::ProcessExec(PluginResult::Err(error)) =
            host.service_request(PluginHostRequest::ProcessExec(PluginProcessExecRequest {
                command: "/bin/echo".to_string(),
                args: Vec::new(),
                env: BTreeMap::new(),
                cwd: None,
                stdin: Vec::new(),
                timeout_ms: Some(5_000),
            }))
        else {
            panic!("an empty allowlist must deny");
        };
        assert_eq!(error.code, PluginErrorCode::Permanent);
    }

    /// The socket layer's own read bound survives the trip through the
    /// host-call envelope: a guest asking for more than the per-call maximum is
    /// clamped by the socket host, not by anything this layer adds.
    #[test]
    fn socket_reads_stay_inside_the_socket_layers_own_bounds() {
        let (port, listener) = echo_listener();
        let descriptor = notification_descriptor(vec![loopback_permission(port)], false);
        let host = notification_host(&descriptor, None);

        let PluginHostResponse::SocketOpen(PluginResult::Ok(opened)) =
            host.service_request(open_request(port))
        else {
            panic!("a granted socket must open");
        };
        host.service_request(PluginHostRequest::SocketWrite(SocketWriteRequest {
            handle: opened.handle,
            data_base64: BASE64.encode(SOCKET_PROBE),
        }));
        let PluginHostResponse::SocketRead(PluginResult::Ok(read)) =
            host.service_request(PluginHostRequest::SocketRead(SocketReadRequest {
                handle: opened.handle,
                // Far beyond the socket host's 64 KiB per-call cap.
                max_bytes: MAX_HOST_REQUEST_BYTES,
            }))
        else {
            panic!("an oversized max_bytes must be clamped, not refused");
        };
        assert_eq!(
            BASE64
                .decode(read.data_base64.as_bytes())
                .expect("the host encodes its own base64"),
            SOCKET_REPLY,
        );
        listener.join().ok();
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
