//! Binary `scryer:host/v1` imports for native command guests.
//!
//! A host call creates a bounded response handle. Guests can obtain its length,
//! copy it into their own memory, then drop it. The command host starts
//! fail-closed: adapters opt services in only with their existing descriptor
//! policy and configuration rather than command artifacts inheriting ambient
//! WASI authority.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use scryer_plugin_sdk::host::{
    HOST_ABI_MODULE, PluginConfigGetResponse, PluginHostRequest, PluginHostResponse,
    PluginHttpResponse, PluginStateGetResponse, PluginStateMutationResponse,
};
use scryer_plugin_sdk::{PluginError, PluginErrorCode, PluginResult};
use wasmtime::{Caller, Linker, Memory};

use crate::plugin_http_host::{IndexerProxyPolicy, PluginHttpHost, PluginHttpRequest};
use crate::wasmtime_host::sandbox::HostCtx;

const MAX_RESPONSE_HANDLES: usize = 32;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_STATE_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub(crate) struct CommandHost {
    state: Arc<Mutex<CommandHostState>>,
    services: Option<Arc<CommandHostServices>>,
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
        }
    }

    pub(crate) fn for_download_client(
        plugin_id: String,
        config: BTreeMap<String, String>,
        allowed_hosts: Vec<String>,
        egress_policy: scryer_outbound_http::PluginEgressPolicy,
        timeout: Duration,
        max_http_response_bytes: Option<u64>,
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
                http: PluginHttpHost::new_with_egress_policy(
                    allowed_hosts,
                    egress_policy,
                    None,
                    None,
                    max_http_response_bytes,
                ),
                timeout,
            })),
        }
    }

    /// Build the host services for a command-ABI indexer.
    ///
    /// Indexers differ from download clients in exactly two ways, and both live
    /// in egress: a configured indexer proxy has to wrap every request, and the
    /// managed-destination cooldown key has to be carried so a shared upstream
    /// (a Prowlarr parent, say) throttles as one destination rather than once
    /// per child. Everything else — descriptor-bound config, plugin state, the
    /// timeout — is identical, so this mirrors `for_download_client` rather
    /// than growing it another two arguments that every caller passes `None` to.
    pub(crate) fn for_indexer(
        plugin_id: String,
        config: BTreeMap<String, String>,
        allowed_hosts: Vec<String>,
        indexer_proxy_policy: Option<IndexerProxyPolicy>,
        destination_cooldown_key: Option<String>,
        timeout: Duration,
        max_http_response_bytes: Option<u64>,
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
            })),
        }
    }

    pub(crate) fn rate_limit_message(&self) -> Option<String> {
        let services = self.services.as_ref()?;
        services
            .http
            .rate_limit_message(&services.plugin_id)
            .ok()
            .flatten()
    }

    fn call(&self, encoded_request: &[u8]) -> Result<u32, String> {
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
                let response = services
                    .http
                    .request(
                        &services.plugin_id,
                        PluginHttpRequest {
                            url: request.url,
                            method: request.method,
                            headers: request.headers,
                        },
                        (!request.body.is_empty()).then_some(request.body),
                        Some(services.timeout),
                    )
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
            request => unsupported_response(request),
        }
    }

    fn response_len(&self, handle: u32) -> Option<usize> {
        self.state.lock().ok()?.responses.get(&handle).map(Vec::len)
    }

    fn response(&self, handle: u32) -> Option<Vec<u8>> {
        self.state.lock().ok()?.responses.get(&handle).cloned()
    }

    fn drop_response(&self, handle: u32) {
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
    })
}

fn unsupported_response(request: PluginHostRequest) -> PluginHostResponse {
    match request {
        PluginHostRequest::ConfigGet(_) => PluginHostResponse::ConfigGet(unsupported()),
        PluginHostRequest::StateGet(_) => PluginHostResponse::StateGet(unsupported()),
        PluginHostRequest::StateSet(_) => PluginHostResponse::StateSet(unsupported()),
        PluginHostRequest::StateDelete(_) => PluginHostResponse::StateDelete(unsupported()),
        PluginHostRequest::Http(_) => PluginHostResponse::Http(unsupported()),
        PluginHostRequest::SocketOpen(_) => PluginHostResponse::SocketOpen(unsupported()),
        PluginHostRequest::SocketRead(_) => PluginHostResponse::SocketRead(unsupported()),
        PluginHostRequest::SocketWrite(_) => PluginHostResponse::SocketWrite(unsupported()),
        PluginHostRequest::SocketStartTls(_) => PluginHostResponse::SocketStartTls(unsupported()),
        PluginHostRequest::SocketClose(_) => PluginHostResponse::SocketClose(unsupported()),
        PluginHostRequest::ProcessExec(_) => PluginHostResponse::ProcessExec(unsupported()),
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
}
