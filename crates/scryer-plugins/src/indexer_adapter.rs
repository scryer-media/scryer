use async_trait::async_trait;
#[cfg(test)]
use scryer_application::NullIndexerErrorRecorder;
use scryer_application::{
    AppError, AppResult, DownloadSourceKind, IndexerClient, IndexerErrorOperation,
    IndexerErrorRecorder, IndexerResponseAttributes, IndexerRoutingPlan, IndexerSearchCompletion,
    IndexerSearchIncompleteReason as HostIncompleteReason, IndexerSearchPlanCapability,
    IndexerSearchPlanRequest, IndexerSearchPlanSummary, IndexerSearchResponse, IndexerSearchResult,
    IndexerSearchStrategyEvent, IndexerSearchStrategyEventSink, SearchMode, is_valid_magnet_uri,
    normalize_release_password,
};
use scryer_domain::{IndexerConfig, IndexerProxyConfig, TaggedAlias};
use scryer_plugin_sdk::command::{
    PluginActionRequest, PluginActionResponse, PluginCommand, PluginCommandRequest,
    PluginCommandResult, PluginIndexerCommand, PluginIndexerCommandResult,
};
use scryer_plugin_sdk::{
    IndexerSearchIncompleteReason as PluginIncompleteReason, IndexerSearchPluginError, PluginError,
    PluginErrorDetails, PluginResult, PluginSearchPlanRequest, PluginSearchPlanSummary,
    PluginSearchStrategyEvent, PluginSearchStrategyRequest, ProviderDescriptor,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, mpsc},
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::legacy_runtime::{LegacyPlugin, LegacyPluginSpec};
use crate::loader::{allowed_hosts_for_descriptor, parse_config_json_entries};
use crate::plugin_http_host::{IndexerErrorCaptureContext, IndexerProxyPolicy};
use crate::runtime_backing::PluginInstanceSpec;
use crate::types::{
    ConfigFieldRole, EXPORT_INDEXER_ACTION, EXPORT_INDEXER_SEARCH, IndexerProtocol,
    IndexerSourceKind, PluginDescriptor, PluginSearchContext, PluginSearchOrigin,
    PluginSearchQueryKind, PluginSearchRequest, PluginSearchRequestKind, PluginSearchResponse,
    PluginSearchSubjectKind, decode_plugin_result, normalize_external_ids,
    normalize_indexer_info_hash, tagged_alias_to_sdk,
};
use crate::wasmtime_host::command_host::CommandHost;
use crate::wasmtime_host::component_host::{
    ComponentActor, ComponentContractVersion, ComponentHost, ComponentRuntime,
    component_strategy_event_channel,
};
use crate::wasmtime_host::engine;
use crate::wasmtime_host::{CommandInvocation, process_command};

/// One configured indexer, backed by either runtime.
///
/// Exactly one of `worker`/`command`/`component` is populated. Legacy artifacts keep the
/// dedicated worker thread that owns a long-lived Extism instance; command-ABI
/// artifacts are instantiated per invocation by [`process_command`], so they
/// need no thread of their own. Keeping both here — rather than behind a trait —
/// is what lets a single Scryer install serve both plugin generations while the
/// catalog is mid-migration.
pub struct WasmIndexerClient {
    descriptor: PluginDescriptor,
    indexer_id: String,
    indexer_name: String,
    indexer_error_recorder: Arc<dyn IndexerErrorRecorder>,
    worker: Option<IndexerPluginWorker>,
    command: Option<Arc<CommandIndexer>>,
    component: Option<Arc<ComponentIndexer>>,
}

struct CommandIndexer {
    wasm: Arc<Vec<u8>>,
    command_host: CommandHost,
    timeout: std::time::Duration,
    /// Serializes invocations against one configured indexer.
    ///
    /// The command runtime instantiates a fresh module per call, so this is not
    /// a reentrancy requirement the way the legacy worker thread was. It is
    /// deliberate parity: the legacy path could only ever run one search at a
    /// time per indexer, several plugins keep cursor/token state across calls
    /// through the host `state` service, and per-indexer rate limits assume
    /// requests leave in order. Dropping the lock would change upstream
    /// request patterns as a side effect of a runtime migration.
    invocation_lock: tokio::sync::Mutex<()>,
}

/// A retained WASI Preview 2 instance for one configured indexer. Calls are
/// serialized at this boundary, while the component may drive as much
/// upstream HTTP fanout as its own policy allows.
struct ComponentIndexer {
    runtime: Arc<ComponentRuntime>,
    host: ComponentHost,
    timeout: std::time::Duration,
    actor: tokio::sync::Mutex<Option<ComponentActor>>,
}

struct PluginSearchCallResponse {
    response: PluginSearchResponse,
    completion: IndexerSearchCompletion,
}

struct IndexerPluginWorker {
    tx: mpsc::Sender<IndexerPluginCommand>,
}

struct IndexerPluginCommand {
    export: &'static str,
    input: String,
    optional: bool,
    indexer_error_capture: Option<IndexerErrorCaptureContext>,
    response: tokio::sync::oneshot::Sender<AppResult<Option<String>>>,
}

impl IndexerPluginWorker {
    fn start(
        spec: LegacyPluginSpec,
        descriptor: &PluginDescriptor,
        indexer_name: &str,
    ) -> AppResult<Self> {
        let (tx, rx) = mpsc::channel::<IndexerPluginCommand>();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let plugin_name = descriptor.name.clone();
        let indexer_label = indexer_name.to_string();
        let thread_name = format!("scryer-wasm-indexer-{indexer_name}");

        std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let mut plugin = match LegacyPlugin::instantiate(spec) {
                    Ok(plugin) => {
                        let _ = ready_tx.send(Ok(()));
                        plugin
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };

                while let Ok(command) = rx.recv() {
                    let start = std::time::Instant::now();
                    if let Some(capture) = command.indexer_error_capture.clone() {
                        plugin.begin_indexer_error_capture(capture);
                    }
                    let result = if command.optional && !plugin.function_exists(command.export) {
                        Ok(None)
                    } else {
                        plugin.call_string(command.export, &command.input).map(Some)
                    };
                    if command.indexer_error_capture.is_some() {
                        let operation_failed =
                            result.as_ref().map_or(true, |output| {
                                output.as_ref().is_none_or(|output| match command.export {
                                    EXPORT_INDEXER_SEARCH => decode_plugin_result::<
                                        PluginSearchResponse,
                                    >(
                                        output, EXPORT_INDEXER_SEARCH
                                    )
                                    .is_err(),
                                    EXPORT_INDEXER_ACTION => decode_plugin_result::<
                                        PluginActionResponse,
                                    >(
                                        output, EXPORT_INDEXER_ACTION
                                    )
                                    .is_err(),
                                    _ => true,
                                })
                            });
                        plugin.finish_indexer_error_capture(operation_failed);
                    }
                    let elapsed = start.elapsed();

                    tracing::debug!(
                        plugin = plugin_name.as_str(),
                        indexer = indexer_label.as_str(),
                        elapsed_ms = elapsed.as_millis() as u64,
                        export = command.export,
                        "WASM plugin call completed"
                    );

                    let _ = command.response.send(result);
                }
            })
            .map_err(|e| AppError::Repository(format!("failed to start plugin worker: {e}")))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { tx }),
            Ok(Err(error)) => Err(AppError::Repository(format!(
                "failed to compile WASM plugin for {indexer_name}: {error}"
            ))),
            Err(error) => Err(AppError::Repository(format!(
                "plugin worker stopped during startup: {error}"
            ))),
        }
    }

    async fn call_search(
        &self,
        input: String,
        cancel_token: CancellationToken,
        indexer_error_capture: IndexerErrorCaptureContext,
    ) -> AppResult<String> {
        let (response, result) = tokio::sync::oneshot::channel();
        self.tx
            .send(IndexerPluginCommand {
                export: EXPORT_INDEXER_SEARCH,
                input,
                optional: false,
                indexer_error_capture: Some(indexer_error_capture),
                response,
            })
            .map_err(|_| AppError::Repository("plugin worker stopped".into()))?;
        let output = tokio::select! {
            _ = cancel_token.cancelled() => {
                return Err(AppError::canceled("plugin indexer search canceled"));
            }
            output = result => output,
        };
        output
            .map_err(|_| AppError::Repository("plugin worker stopped".into()))?
            .and_then(|output| {
                output.ok_or_else(|| {
                    AppError::Repository(format!(
                        "plugin {EXPORT_INDEXER_SEARCH}() returned no output"
                    ))
                })
            })
    }

    async fn call_action(
        &self,
        input: String,
        indexer_error_capture: IndexerErrorCaptureContext,
    ) -> AppResult<Option<String>> {
        let (response, result) = tokio::sync::oneshot::channel();
        self.tx
            .send(IndexerPluginCommand {
                export: EXPORT_INDEXER_ACTION,
                input,
                optional: true,
                indexer_error_capture: Some(indexer_error_capture),
                response,
            })
            .map_err(|_| AppError::Repository("plugin worker stopped".into()))?;
        result
            .await
            .map_err(|_| AppError::Repository("plugin worker stopped".into()))?
    }
}

impl WasmIndexerClient {
    pub fn new_with_indexer_error_recorder(
        wasm_bytes: Vec<u8>,
        descriptor: PluginDescriptor,
        indexer_name: String,
        config: IndexerConfig,
        indexer_proxy_config: Option<IndexerProxyConfig>,
        indexer_error_recorder: Arc<dyn IndexerErrorRecorder>,
    ) -> Result<Self, AppError> {
        let spec = build_legacy_spec(
            wasm_bytes,
            &descriptor,
            &indexer_name,
            &config,
            indexer_proxy_config,
        );
        let worker = IndexerPluginWorker::start(spec, &descriptor, &indexer_name)?;

        info!(
            indexer = indexer_name.as_str(),
            plugin = descriptor.name.as_str(),
            "WASM plugin compiled and cached"
        );

        Ok(Self {
            descriptor,
            indexer_id: config.id,
            indexer_name,
            indexer_error_recorder,
            worker: Some(worker),
            command: None,
            component: None,
        })
    }

    /// Build a command-ABI indexer.
    ///
    /// This derives its config, allowed hosts, timeout, proxy policy and
    /// cooldown key from exactly the same helpers the legacy path uses, so the
    /// two runtimes cannot drift on what a plugin observes — a search that
    /// worked before the migration sees byte-identical config after it.
    #[cfg(test)]
    pub fn new_command(
        wasm_bytes: Vec<u8>,
        descriptor: PluginDescriptor,
        indexer_name: String,
        config: IndexerConfig,
        indexer_proxy_config: Option<IndexerProxyConfig>,
    ) -> Result<Self, AppError> {
        Self::new_command_with_indexer_error_recorder(
            wasm_bytes,
            descriptor,
            indexer_name,
            config,
            indexer_proxy_config,
            Arc::new(NullIndexerErrorRecorder),
        )
    }

    pub fn new_command_with_indexer_error_recorder(
        wasm_bytes: Vec<u8>,
        descriptor: PluginDescriptor,
        indexer_name: String,
        config: IndexerConfig,
        indexer_proxy_config: Option<IndexerProxyConfig>,
        indexer_error_recorder: Arc<dyn IndexerErrorRecorder>,
    ) -> Result<Self, AppError> {
        let inputs =
            build_runtime_inputs(&descriptor, &indexer_name, &config, indexer_proxy_config);
        let command_host = CommandHost::for_indexer(
            descriptor.id.clone(),
            inputs
                .config_entries
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
            inputs.allowed_hosts,
            inputs.indexer_proxy_policy,
            inputs.destination_cooldown_key,
            inputs.timeout,
            None,
        );

        info!(
            indexer = indexer_name.as_str(),
            plugin = descriptor.name.as_str(),
            "command indexer plugin registered"
        );

        Ok(Self {
            descriptor,
            indexer_id: config.id,
            indexer_name,
            indexer_error_recorder,
            worker: None,
            command: Some(Arc::new(CommandIndexer {
                wasm: Arc::new(wasm_bytes),
                command_host,
                timeout: inputs.timeout,
                invocation_lock: tokio::sync::Mutex::new(()),
            })),
            component: None,
        })
    }

    /// Build an async WASI Preview 2 component indexer. The component is
    /// compiled once for this configured client and instantiated lazily on its
    /// first operation; its state then remains alive until a trap, timeout,
    /// cancellation, provider reload, or configuration change replaces this
    /// client.
    pub fn new_component_with_indexer_error_recorder(
        wasm_bytes: Vec<u8>,
        descriptor: PluginDescriptor,
        indexer_name: String,
        config: IndexerConfig,
        indexer_proxy_config: Option<IndexerProxyConfig>,
        indexer_error_recorder: Arc<dyn IndexerErrorRecorder>,
    ) -> Result<Self, AppError> {
        let inputs =
            build_runtime_inputs(&descriptor, &indexer_name, &config, indexer_proxy_config);
        let provider_profile = if descriptor.provider_type() == "newznab" {
            let normalized_config_json =
                serde_json::to_string(&inputs.config_entries).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to serialize normalized Newznab configuration: {error}"
                    ))
                })?;
            Some(
                crate::newznab_profiles::resolve_newznab_profile_bytes(
                    descriptor.indexer().ok_or_else(|| {
                        AppError::Repository(
                            "Newznab plugin descriptor is not an indexer".to_string(),
                        )
                    })?,
                    &config.provider_type,
                    Some(&normalized_config_json),
                )
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to resolve Newznab provider profile: {error}"
                    ))
                })?,
            )
        } else {
            None
        };
        let host = ComponentHost::for_indexer_with_provider_profile(
            inputs
                .config_entries
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
            inputs.allowed_hosts,
            inputs.indexer_proxy_policy,
            inputs.timeout,
            None,
            provider_profile,
        )
        .map_err(AppError::Repository)?;
        let runtime = ComponentRuntime::new(engine::shared_async_engine(), &wasm_bytes)
            .map_err(AppError::Repository)?;

        info!(
            indexer = indexer_name.as_str(),
            plugin = descriptor.name.as_str(),
            "WASI Preview 2 indexer component registered"
        );

        Ok(Self {
            descriptor,
            indexer_id: config.id,
            indexer_name,
            indexer_error_recorder,
            worker: None,
            command: None,
            component: Some(Arc::new(ComponentIndexer {
                runtime: Arc::new(runtime),
                host,
                timeout: inputs.timeout,
                actor: tokio::sync::Mutex::new(None),
            })),
        })
    }

    fn legacy_worker(&self) -> AppResult<&IndexerPluginWorker> {
        self.worker.as_ref().ok_or_else(|| {
            AppError::Repository(format!(
                "command indexer {} cannot use a legacy export",
                self.descriptor.id
            ))
        })
    }

    fn indexer_error_capture(
        &self,
        operation: IndexerErrorOperation,
    ) -> IndexerErrorCaptureContext {
        IndexerErrorCaptureContext {
            indexer_id: self.indexer_id.clone(),
            indexer_name: self.indexer_name.clone(),
            operation,
            recorder: Arc::clone(&self.indexer_error_recorder),
        }
    }

    /// Run one indexer command, or `Ok(None)` when this client is legacy-backed.
    ///
    /// Returning `None` rather than erroring is what keeps every call site a
    /// plain "try command, else legacy" pair.
    async fn invoke_command(
        &self,
        command: PluginIndexerCommand,
        operation: &'static str,
        cancel_token: Option<&CancellationToken>,
    ) -> AppResult<Option<PluginIndexerCommandResult>> {
        let Some(indexer) = self.command.as_ref() else {
            return Ok(None);
        };
        let _guard = indexer.invocation_lock.lock().await;
        let spec = PluginInstanceSpec {
            wasm: Arc::clone(&indexer.wasm),
            preopens: Vec::new(),
            timeout: indexer.timeout,
            memory_max_bytes: None,
            command_host: indexer.command_host.clone(),
        };
        let request = PluginCommandRequest::new(PluginCommand::Indexer(command));
        let invocation = process_command(
            &spec,
            &request,
            CommandInvocation {
                plugin_id: &self.descriptor.id,
                plugin_version: &self.descriptor.version,
                operation,
            },
        );
        let response = match cancel_token {
            Some(token) => tokio::select! {
                _ = token.cancelled() => {
                    return Err(AppError::canceled("plugin indexer search canceled"));
                }
                response = invocation => response?,
            },
            None => invocation.await?,
        };
        match response.response {
            PluginCommandResult::Indexer(result) => Ok(Some(result)),
            _ => Err(AppError::Repository(format!(
                "command plugin {} returned a response for another plugin family",
                self.descriptor.id
            ))),
        }
    }

    /// Invoke one retained component operation. Store ownership remains inside
    /// the actor for ordinary calls. Cancellation, timeout, and a component
    /// trap remove the actor while holding its serialization lock, which drops
    /// the Store and therefore every in-flight async host HTTP future before a
    /// subsequent request recreates the instance.
    async fn invoke_component(
        &self,
        command: PluginIndexerCommand,
        operation: &'static str,
        cancel_token: Option<&CancellationToken>,
    ) -> AppResult<Option<PluginIndexerCommandResult>> {
        let Some(indexer) = self.component.as_ref() else {
            return Ok(None);
        };
        let (request, is_search) = match command {
            PluginIndexerCommand::Search(request) => (serde_json::to_vec(&request), true),
            PluginIndexerCommand::Action(request) => (serde_json::to_vec(&request), false),
        };
        let request = request.map_err(|error| {
            AppError::Repository(format!(
                "failed to encode indexer component request: {error}"
            ))
        })?;
        let token = cancel_token.cloned().unwrap_or_else(CancellationToken::new);
        let _guard = indexer.actor.lock().await;
        indexer.host.bind_cancellation(token.clone());
        let mut actor = _guard;

        if actor.is_none() {
            *actor = Some(
                indexer
                    .runtime
                    .instantiate(&indexer.host)
                    .await
                    .map_err(|error| {
                        AppError::Repository(format!(
                            "indexer component {} could not start {operation}: {error}",
                            self.descriptor.id
                        ))
                    })?,
            );
        }

        let call = async {
            let Some(actor) = actor.as_mut() else {
                unreachable!("component actor was initialized above");
            };
            tokio::time::timeout(indexer.timeout, async move {
                if is_search {
                    actor.search(request).await
                } else {
                    actor.action(request).await
                }
            })
            .await
        };
        let response = tokio::select! {
            _ = token.cancelled() => {
                actor.take();
                return Err(AppError::canceled("plugin indexer component search canceled"));
            }
            result = call => match result {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    actor.take();
                    return Err(AppError::Repository(format!(
                        "indexer component {} {operation} failed: {error}",
                        self.descriptor.id
                    )));
                }
                Err(_) => {
                    actor.take();
                    return Err(AppError::Repository(format!(
                        "indexer component {} {operation} timed out after {} ms",
                        self.descriptor.id,
                        indexer.timeout.as_millis(),
                    )));
                }
            },
        };
        let bytes = match response {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "indexer component {} {operation} returned invocation error {error:?}",
                    self.descriptor.id
                )));
            }
        };
        let result = if is_search {
            serde_json::from_slice::<PluginResult<PluginSearchResponse>>(&bytes)
                .map(PluginIndexerCommandResult::Search)
        } else {
            serde_json::from_slice::<PluginResult<PluginActionResponse>>(&bytes)
                .map(PluginIndexerCommandResult::Action)
        }
        .map_err(|error| {
            AppError::Repository(format!(
                "indexer component {} {operation} returned invalid JSON response: {error}",
                self.descriptor.id
            ))
        })?;
        Ok(Some(result))
    }

    async fn invoke_component_search_plan(
        &self,
        request: &PluginSearchPlanRequest,
        operation: IndexerErrorOperation,
        cancel_token: CancellationToken,
        event_sink: &IndexerSearchStrategyEventSink,
    ) -> AppResult<PluginSearchPlanSummary> {
        let Some(indexer) = self.component.as_ref() else {
            return Err(AppError::Repository(
                "strategy plans require a component indexer".to_string(),
            ));
        };
        let request = serde_json::to_vec(request).map_err(|error| {
            AppError::Repository(format!(
                "failed to encode indexer component strategy plan: {error}"
            ))
        })?;
        let attested = matches!(
            &self.descriptor.provider,
            ProviderDescriptor::Indexer(descriptor)
                if descriptor.search_semantics_version.is_some()
        );
        let mut actor = indexer.actor.lock().await;
        indexer.host.bind_cancellation(cancel_token.clone());
        indexer
            .host
            .begin_indexer_error_capture(self.indexer_error_capture(operation));
        if actor.is_none() {
            *actor = Some(
                indexer
                    .runtime
                    .instantiate(&indexer.host)
                    .await
                    .map_err(|error| {
                        AppError::Repository(format!(
                            "indexer component {} could not start strategy plan: {error}",
                            self.descriptor.id
                        ))
                    })?,
            );
        }

        let (raw_event_tx, mut raw_event_rx) = component_strategy_event_channel();
        let mut event_error = None;
        let call_result = {
            let Some(component_actor) = actor.as_mut() else {
                unreachable!("component actor was initialized above");
            };
            let call = tokio::time::timeout(
                indexer.timeout,
                component_actor.search_plan(request, raw_event_tx),
            );
            tokio::pin!(call);
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        break Err(AppError::canceled("plugin indexer strategy plan canceled"));
                    }
                    event = raw_event_rx.recv(), if !raw_event_rx.is_closed() => {
                        let Some(event) = event else {
                            continue;
                        };
                        if let Err(error) = forward_component_strategy_event(
                            self,
                            &event,
                            attested,
                            event_sink,
                        )
                        .await
                        {
                            event_error = Some(error);
                            cancel_token.cancel();
                        }
                    }
                    result = &mut call => {
                        break match result {
                            Ok(Ok(response)) => Ok(response),
                            Ok(Err(error)) => Err(AppError::Repository(format!(
                                "indexer component {} strategy plan failed: {error}",
                                self.descriptor.id
                            ))),
                            Err(_) => Err(AppError::Repository(format!(
                                "indexer component {} strategy plan timed out after {} ms",
                                self.descriptor.id,
                                indexer.timeout.as_millis(),
                            ))),
                        };
                    }
                }
            }
        };

        if let Some(error) = event_error {
            actor.take();
            indexer.host.finish_indexer_error_capture(true);
            return Err(error);
        }
        let response = match call_result {
            Ok(response) => response,
            Err(error) => {
                actor.take();
                indexer.host.finish_indexer_error_capture(true);
                return Err(error);
            }
        };
        while let Ok(event) = raw_event_rx.try_recv() {
            if let Err(error) =
                forward_component_strategy_event(self, &event, attested, event_sink).await
            {
                actor.take();
                indexer.host.finish_indexer_error_capture(true);
                return Err(error);
            }
        }
        let bytes = response.map_err(|error| {
            AppError::Repository(format!(
                "indexer component {} strategy plan returned invocation error {error:?}",
                self.descriptor.id
            ))
        })?;
        let summary = serde_json::from_slice::<PluginSearchPlanSummary>(&bytes).map_err(|error| {
            AppError::Repository(format!(
                "indexer component {} returned an invalid strategy summary: {error}",
                self.descriptor.id
            ))
        });
        indexer.host.finish_indexer_error_capture(summary.is_err());
        summary
    }

    #[allow(dead_code)]
    pub async fn indexer_action(
        &self,
        action: &str,
        query: BTreeMap<String, String>,
    ) -> AppResult<Option<serde_json::Value>> {
        if let Some(indexer) = self.component.as_ref() {
            indexer.host.begin_indexer_error_capture(
                self.indexer_error_capture(IndexerErrorOperation::IndexerAction),
            );
            let result = match self
                .invoke_component(
                    PluginIndexerCommand::Action(PluginActionRequest {
                        action: action.to_string(),
                        payload: serde_json::json!({ "query": query }),
                    }),
                    "indexer_action",
                    None,
                )
                .await
            {
                Ok(Some(PluginIndexerCommandResult::Action(result))) => {
                    decode_command_result::<PluginActionResponse>(
                        result,
                        "indexer indexer_action component",
                    )
                    .map(|response| Some(response.payload))
                }
                Ok(Some(_)) => Err(AppError::Repository(
                    "indexer component returned the wrong result for indexer_action".to_string(),
                )),
                Ok(None) => Err(AppError::Repository(
                    "component indexer unexpectedly selected another runtime".to_string(),
                )),
                Err(error) => Err(error),
            };
            indexer.host.finish_indexer_error_capture(result.is_err());
            return result;
        }
        if let Some(indexer) = self.command.as_ref() {
            indexer.command_host.begin_indexer_error_capture(
                self.indexer_error_capture(IndexerErrorOperation::IndexerAction),
            );
            let result = match self
                .invoke_command(
                    PluginIndexerCommand::Action(PluginActionRequest {
                        action: action.to_string(),
                        payload: serde_json::json!({ "query": query }),
                    }),
                    "indexer_action",
                    None,
                )
                .await
            {
                Ok(Some(PluginIndexerCommandResult::Action(result))) => {
                    decode_command_result::<PluginActionResponse>(result, "indexer indexer_action")
                        .map(|response| Some(response.payload))
                }
                Ok(Some(_)) => Err(AppError::Repository(
                    "indexer command returned the wrong result for indexer_action".to_string(),
                )),
                Ok(None) => Err(AppError::Repository(
                    "command indexer unexpectedly selected the legacy runtime".to_string(),
                )),
                Err(error) => Err(error),
            };
            indexer
                .command_host
                .finish_indexer_error_capture(result.is_err());
            return result;
        }

        let request = serde_json::json!({
            "action": action,
            "query": query,
        });
        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize indexer action request: {e}"))
        })?;

        self.legacy_worker()?
            .call_action(
                input,
                self.indexer_error_capture(IndexerErrorOperation::IndexerAction),
            )
            .await?
            .map(|output| decode_plugin_result(&output, EXPORT_INDEXER_ACTION))
            .transpose()
    }

    async fn call_search_request(
        &self,
        request: &PluginSearchRequest,
        operation: IndexerErrorOperation,
        cancel_token: CancellationToken,
    ) -> AppResult<PluginSearchCallResponse> {
        let attested = matches!(
            &self.descriptor.provider,
            ProviderDescriptor::Indexer(descriptor)
                if descriptor.search_semantics_version.is_some()
        );
        if let Some(indexer) = self.component.as_ref() {
            indexer
                .host
                .begin_indexer_error_capture(self.indexer_error_capture(operation));
            let result = match self
                .invoke_component(
                    PluginIndexerCommand::Search(request.clone()),
                    "indexer_search",
                    Some(&cancel_token),
                )
                .await
            {
                Ok(Some(PluginIndexerCommandResult::Search(result))) => {
                    decode_search_result(result, attested, "indexer indexer_search component")
                }
                Ok(Some(_)) => Err(AppError::Repository(
                    "indexer component returned the wrong result for indexer_search".to_string(),
                )),
                Ok(None) => Err(AppError::Repository(
                    "component indexer unexpectedly selected another runtime".to_string(),
                )),
                Err(error) => Err(error),
            };
            indexer.host.finish_indexer_error_capture(result.is_err());
            return result;
        }
        if let Some(indexer) = self.command.as_ref() {
            indexer
                .command_host
                .begin_indexer_error_capture(self.indexer_error_capture(operation));
            let result = match self
                .invoke_command(
                    PluginIndexerCommand::Search(request.clone()),
                    "indexer_search",
                    Some(&cancel_token),
                )
                .await
            {
                Ok(Some(PluginIndexerCommandResult::Search(result))) => {
                    if matches!(
                        &result,
                        PluginResult::Err(error) if error.public_message == "indexer command failed"
                    ) && let Some(message) = indexer.command_host.rate_limit_message()
                    {
                        Err(AppError::Repository(format!(
                            "indexer indexer_search: plugin error RateLimited: {message}"
                        )))
                    } else {
                        decode_search_result(result, attested, "indexer indexer_search")
                    }
                }
                Ok(Some(_)) => Err(AppError::Repository(
                    "indexer command returned the wrong result for indexer_search".to_string(),
                )),
                Ok(None) => Err(AppError::Repository(
                    "command indexer unexpectedly selected the legacy runtime".to_string(),
                )),
                Err(error) => Err(error),
            };
            indexer
                .command_host
                .finish_indexer_error_capture(result.is_err());
            return result;
        }

        let input = serde_json::to_string(request).map_err(|e| {
            AppError::Repository(format!("failed to serialize plugin request: {e}"))
        })?;

        tracing::debug!(plugin = %self.descriptor.name, %input, "plugin search request");

        let output = self
            .legacy_worker()?
            .call_search(input, cancel_token, self.indexer_error_capture(operation))
            .await?;

        let result = serde_json::from_str::<PluginResult<PluginSearchResponse>>(&output).map_err(
            |error| {
                AppError::Repository(format!(
                    "{EXPORT_INDEXER_SEARCH}: plugin returned invalid result envelope: {error}"
                ))
            },
        )?;
        decode_search_result(result, attested, EXPORT_INDEXER_SEARCH)
    }
}

fn decode_command_result<T>(result: PluginResult<T>, context: &str) -> AppResult<T> {
    match result {
        PluginResult::Ok(value) => Ok(value),
        PluginResult::Err(PluginError {
            code,
            public_message,
            ..
        }) => Err(AppError::Repository(format!(
            "{context}: plugin error {code:?}: {public_message}"
        ))),
    }
}

fn decode_search_result(
    result: PluginResult<PluginSearchResponse>,
    attested: bool,
    context: &str,
) -> AppResult<PluginSearchCallResponse> {
    match result {
        PluginResult::Ok(response) => Ok(PluginSearchCallResponse {
            response,
            completion: if attested {
                IndexerSearchCompletion::Complete
            } else {
                IndexerSearchCompletion::Partial {
                    reason: Some(HostIncompleteReason::Unattested),
                    retry_after: None,
                }
            },
        }),
        PluginResult::Err(PluginError {
            details:
                Some(PluginErrorDetails::IndexerSearch(IndexerSearchPluginError::PartialResults {
                    response,
                    reason,
                    retry_after_seconds,
                })),
            ..
        }) => Ok(PluginSearchCallResponse {
            response: *response,
            completion: IndexerSearchCompletion::Partial {
                reason: Some(host_incomplete_reason(reason)),
                retry_after: retry_after_seconds
                    .and_then(|seconds| u64::try_from(seconds).ok())
                    .map(std::time::Duration::from_secs),
            },
        }),
        PluginResult::Err(PluginError {
            public_message,
            details:
                Some(PluginErrorDetails::IndexerSearch(IndexerSearchPluginError::Deferred {
                    reason,
                    retry_after_seconds,
                })),
            ..
        }) => {
            let retry_after = retry_after_seconds
                .and_then(|seconds| u64::try_from(seconds).ok())
                .map(std::time::Duration::from_secs);
            let message = match retry_after {
                Some(retry_after) => format!(
                    "{public_message}; indexer search deferred: {reason:?}; retry after {}s",
                    retry_after.as_secs()
                ),
                None => format!("{public_message}; indexer search deferred: {reason:?}"),
            };
            Err(AppError::temporary_unavailable(message, retry_after))
        }
        PluginResult::Err(error) => Err(AppError::Repository(format!(
            "{context}: plugin error {:?}: {}",
            error.code, error.public_message
        ))),
    }
}

fn host_incomplete_reason(reason: PluginIncompleteReason) -> HostIncompleteReason {
    match reason {
        PluginIncompleteReason::UpstreamFailure => HostIncompleteReason::UpstreamFailure,
        PluginIncompleteReason::RateLimited => HostIncompleteReason::RateLimited,
        PluginIncompleteReason::MalformedContent => HostIncompleteReason::MalformedContent,
        PluginIncompleteReason::PageCeilingReached => HostIncompleteReason::PageCeilingReached,
        PluginIncompleteReason::FanoutBranchFailed => HostIncompleteReason::FanoutBranchFailed,
        PluginIncompleteReason::SaturatedPartition => HostIncompleteReason::SaturatedPartition,
        PluginIncompleteReason::Unattested => HostIncompleteReason::Unattested,
    }
}

fn build_legacy_spec(
    wasm_bytes: Vec<u8>,
    descriptor: &PluginDescriptor,
    indexer_name: &str,
    config: &IndexerConfig,
    indexer_proxy_config: Option<IndexerProxyConfig>,
) -> LegacyPluginSpec {
    let inputs = build_runtime_inputs(descriptor, indexer_name, config, indexer_proxy_config);
    let mut spec = LegacyPluginSpec::new(wasm_bytes, descriptor.id.clone());
    spec.allowed_hosts = inputs.allowed_hosts;
    spec.timeout = inputs.timeout;
    for (key, value) in inputs.config_entries {
        spec.config.insert(key, value);
    }
    spec.indexer_proxy_policy = inputs.indexer_proxy_policy;
    spec.destination_cooldown_key = inputs.destination_cooldown_key;
    spec
}

/// Everything a configured indexer needs from its runtime, independent of which
/// runtime that is.
///
/// Both constructors go through here so config normalization, host allowlisting,
/// the proxy timeout bump and the cooldown key stay defined once. A plugin that
/// migrates to the command ABI must not see different config as a side effect.
struct IndexerRuntimeInputs {
    config_entries: std::collections::HashMap<String, String>,
    allowed_hosts: Vec<String>,
    timeout: std::time::Duration,
    indexer_proxy_policy: Option<IndexerProxyPolicy>,
    destination_cooldown_key: Option<String>,
}

fn build_runtime_inputs(
    descriptor: &PluginDescriptor,
    indexer_name: &str,
    config: &IndexerConfig,
    indexer_proxy_config: Option<IndexerProxyConfig>,
) -> IndexerRuntimeInputs {
    let config_entries = build_config_entries(descriptor, indexer_name, config);
    let connection_url = resolve_connection_url(descriptor, config_entries.as_ref());
    let allowed_hosts = allowed_hosts_for_descriptor(
        descriptor,
        connection_url.as_deref(),
        config.config_json.as_deref(),
    );
    let timeout = scryer_outbound_http::effective_indexer_timeout(
        indexer_proxy_config
            .as_ref()
            .map(|config| config.request_timeout_seconds),
    );
    IndexerRuntimeInputs {
        config_entries: config_entries.unwrap_or_default(),
        allowed_hosts,
        timeout,
        indexer_proxy_policy: indexer_proxy_config.map(|proxy_config| IndexerProxyPolicy {
            indexer_id: config.id.clone(),
            indexer_name: indexer_name.to_string(),
            config: proxy_config,
        }),
        destination_cooldown_key: Some(config.rate_limit_domain_key()),
    }
}

fn build_config_entries(
    descriptor: &PluginDescriptor,
    indexer_name: &str,
    config: &IndexerConfig,
) -> Option<std::collections::HashMap<String, String>> {
    match config.config_json.as_deref() {
        Some(json_str) => match parse_config_json_entries(json_str) {
            Ok(map) => Some(normalize_indexer_config_entries(descriptor, config, map)),
            Err(error) => {
                warn!(
                    indexer = indexer_name,
                    error = %error,
                    "failed to parse config_json; config keys will not be injected"
                );
                None
            }
        },
        None => None,
    }
}

fn normalize_indexer_config_entries(
    descriptor: &PluginDescriptor,
    config: &IndexerConfig,
    mut entries: std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    let mut extracted_api_path: Option<String> = None;
    let mut extracted_additional_params: Option<String> = None;
    let normalize_as_direct_nab = config.is_direct_nab();

    if let Some(connection_url_key) = descriptor
        .config_fields()
        .iter()
        .find(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))
        .map(|field| field.key.as_str())
    {
        let normalized_connection_url = if normalize_as_direct_nab {
            entries
                .get(connection_url_key)
                .and_then(|value| normalize_direct_nab_connection_url(value))
                .map(|parts| {
                    extracted_api_path = parts.api_path;
                    extracted_additional_params = parts.additional_params;
                    parts.base_url
                })
        } else {
            entries
                .get(connection_url_key)
                .and_then(|value| normalize_connection_url(value))
        };

        match normalized_connection_url {
            Some(value) => {
                entries.insert(connection_url_key.to_string(), value);
            }
            None => {
                entries.remove(connection_url_key);
            }
        }
    }

    let normalized_api_path = extracted_api_path.or_else(|| {
        entries
            .get("api_path")
            .and_then(|value| normalize_api_path(value))
    });
    match normalized_api_path {
        Some(value) => {
            entries.insert("api_path".to_string(), value);
        }
        None => {
            entries.remove("api_path");
        }
    }

    let normalized_additional_params = merge_additional_params(
        extracted_additional_params.as_deref(),
        entries.get("additional_params").map(String::as_str),
    );
    let normalized_additional_params = if normalize_as_direct_nab {
        Some(normalize_direct_nab_additional_params(
            normalized_additional_params.as_deref(),
        ))
    } else {
        normalized_additional_params
    };

    match normalized_additional_params {
        Some(value) => {
            entries.insert("additional_params".to_string(), value);
        }
        None => {
            entries.remove("additional_params");
        }
    }

    entries
}

fn normalize_connection_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    (!trimmed.is_empty()).then_some(trimmed.to_string())
}

#[derive(Debug, PartialEq, Eq)]
struct NormalizedDirectNabConnection {
    base_url: String,
    api_path: Option<String>,
    additional_params: Option<String>,
}

fn normalize_direct_nab_connection_url(raw: &str) -> Option<NormalizedDirectNabConnection> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let Ok(url) = url::Url::parse(trimmed) else {
        return normalize_connection_url(raw).map(|base_url| NormalizedDirectNabConnection {
            base_url,
            api_path: None,
            additional_params: None,
        });
    };

    let mut normalized = url.clone();
    normalized.set_query(None);
    normalized.set_fragment(None);
    normalized.set_path("");

    let origin = normalized.to_string().trim_end_matches('/').to_string();
    if origin.is_empty() {
        return None;
    }

    let raw_path = url.path().trim();
    let api_path = normalize_api_path(raw_path);

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in url.query_pairs() {
        let key = key.trim();
        if key.is_empty() || is_direct_nab_control_query_key(key) {
            continue;
        }
        serializer.append_pair(key, value.trim());
    }
    let serialized_params = serializer.finish();
    let additional_params = (!serialized_params.is_empty()).then_some(serialized_params);

    Some(NormalizedDirectNabConnection {
        base_url: origin,
        api_path,
        additional_params,
    })
}

fn normalize_api_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches('/');
    (!trimmed.is_empty()).then(|| format!("/{trimmed}"))
}

fn normalize_additional_params(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches(['?', '&']).trim();
    if trimmed.is_empty() {
        return None;
    }

    let pairs = url::form_urlencoded::parse(trimmed.as_bytes()).collect::<Vec<_>>();
    if pairs.is_empty() {
        return None;
    }

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(&key, &value);
    }

    let normalized = serializer.finish();
    (!normalized.is_empty()).then_some(normalized)
}

fn merge_additional_params(extracted: Option<&str>, existing: Option<&str>) -> Option<String> {
    if extracted.is_none() {
        return existing.and_then(normalize_additional_params);
    }
    if existing.is_none() {
        return extracted.and_then(normalize_additional_params);
    }

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    let mut any = false;

    for raw in [extracted, existing].into_iter().flatten() {
        let trimmed = raw.trim().trim_start_matches(['?', '&']).trim();
        if trimmed.is_empty() {
            continue;
        }

        for (key, value) in url::form_urlencoded::parse(trimmed.as_bytes()) {
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            serializer.append_pair(key, value.trim());
            any = true;
        }
    }

    any.then(|| serializer.finish())
}

fn normalize_direct_nab_additional_params(existing: Option<&str>) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("o", "json");
    serializer.append_pair("extended", "1");

    if let Some(raw) = existing {
        let trimmed = raw.trim().trim_start_matches(['?', '&']).trim();
        if !trimmed.is_empty() {
            for (key, value) in url::form_urlencoded::parse(trimmed.as_bytes()) {
                let key = key.trim();
                if key.is_empty() {
                    continue;
                }
                let normalized_key = key.to_ascii_lowercase();
                if normalized_key == "o" || normalized_key == "extended" {
                    continue;
                }
                serializer.append_pair(key, value.trim());
            }
        }
    }

    serializer.finish()
}

fn is_direct_nab_control_query_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "apikey"
            | "api_key"
            | "key"
            | "token"
            | "t"
            | "q"
            | "cat"
            | "o"
            | "extended"
            | "limit"
            | "offset"
            | "imdbid"
            | "tvdbid"
            | "tmdbid"
            | "season"
            | "ep"
            | "rid"
            | "tvmazeid"
            | "traktid"
            | "doubanid"
            | "imdbtitle"
            | "imdbyear"
            | "genre"
            | "year"
            | "group"
    )
}

fn resolve_connection_url(
    descriptor: &PluginDescriptor,
    config_entries: Option<&std::collections::HashMap<String, String>>,
) -> Option<String> {
    let field = descriptor
        .config_fields()
        .iter()
        .find(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))?;
    config_entries
        .and_then(|entries| entries.get(&field.key).map(String::as_str))
        .or(field.default_value.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn build_search_context(
    query: &str,
    ids: &std::collections::HashMap<String, String>,
    facet: Option<&str>,
    mode: SearchMode,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
) -> PluginSearchContext {
    let is_recent_request = matches!(mode, SearchMode::Auto)
        && query.trim().is_empty()
        && ids.is_empty()
        && season.is_none()
        && episode.is_none()
        && absolute_episode.is_none();

    let normalized_facet = facet
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());

    let subject_kind = match normalized_facet.as_deref() {
        Some("movie") => PluginSearchSubjectKind::Movie,
        Some("anime") if episode.is_some() || absolute_episode.is_some() => {
            PluginSearchSubjectKind::AnimeEpisode
        }
        Some("special") => PluginSearchSubjectKind::Special,
        _ if episode.is_some() || absolute_episode.is_some() => PluginSearchSubjectKind::Episode,
        _ if season.is_some() => PluginSearchSubjectKind::Season,
        Some("collection") => PluginSearchSubjectKind::Collection,
        Some("series") | Some("anime") | Some("title") => PluginSearchSubjectKind::Title,
        _ => PluginSearchSubjectKind::Unknown,
    };

    let query_kind = if !ids.is_empty() {
        if ids.len() > 1 {
            PluginSearchQueryKind::AggregateId
        } else {
            PluginSearchQueryKind::Id
        }
    } else if query.trim().is_empty() {
        PluginSearchQueryKind::Fallback
    } else if normalized_facet.is_some() {
        PluginSearchQueryKind::Title
    } else {
        PluginSearchQueryKind::Text
    };

    PluginSearchContext {
        request_kind: if is_recent_request {
            PluginSearchRequestKind::Recent
        } else {
            PluginSearchRequestKind::Search
        },
        search_origin: if is_recent_request {
            PluginSearchOrigin::Rss
        } else {
            match mode {
                SearchMode::Interactive => PluginSearchOrigin::Interactive,
                SearchMode::Auto => PluginSearchOrigin::Automatic,
            }
        },
        subject_kind,
        query_kind,
        ..PluginSearchContext::default()
    }
}

fn should_try_generic_search_fallback(request: &PluginSearchRequest) -> bool {
    !request.query.trim().is_empty() && !request.ids.is_empty()
}

fn generic_search_fallback_request(
    request: &PluginSearchRequest,
    mode: SearchMode,
) -> PluginSearchRequest {
    let mut fallback = request.clone();
    fallback.ids.clear();
    fallback.category = None;
    fallback.facet = None;
    fallback.context = Some(build_search_context(
        &fallback.query,
        &fallback.ids,
        None,
        mode,
        fallback.season,
        fallback.episode,
        fallback.absolute_episode,
    ));
    fallback
}

fn merge_result_extra(
    result: &scryer_plugin_sdk::PluginSearchResult,
) -> std::collections::HashMap<String, serde_json::Value> {
    let provider_extra = result.provider_extra.clone();
    let mut extra = std::collections::HashMap::new();

    insert_value(&mut extra, "source_kind", result.source_kind);
    insert_value(&mut extra, "protocol", result.protocol);

    let normalized_external_ids = normalize_external_ids(
        result
            .external_ids
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    if !normalized_external_ids.is_empty() {
        insert_json(&mut extra, "external_ids", &normalized_external_ids);
        if let Some(imdb_id) = normalized_external_ids.get("imdb_id") {
            insert_json(&mut extra, "response_imdbid", imdb_id);
        }
        if let Some(tvdb_id) = normalized_external_ids.get("tvdb_id") {
            insert_json(&mut extra, "response_tvdbid", tvdb_id);
        }
        if let Some(anidb_id) = normalized_external_ids.get("anidb_id") {
            insert_json(&mut extra, "response_anidbid", anidb_id);
        }
    }

    if !result.categories.is_empty() {
        insert_json(&mut extra, "categories", &result.categories);
    }
    if !result.provider_categories.is_empty() {
        insert_json(
            &mut extra,
            "provider_categories",
            &result.provider_categories,
        );
    }

    if let Some(magnet_url) = result.magnet_url.as_deref() {
        insert_json(&mut extra, "magnet_url", magnet_url);
        insert_json(&mut extra, "magnet_uri", magnet_url);
    }

    let info_hash_v1 = normalize_indexer_info_hash(result.info_hash_v1.as_deref())
        .filter(|value| value.len() == 40);
    let info_hash_v2 = normalize_indexer_info_hash(result.info_hash_v2.as_deref())
        .filter(|value| value.len() == 64);
    if let Some(info_hash_v1) = info_hash_v1.as_deref() {
        insert_json(&mut extra, "info_hash_v1", info_hash_v1);
        insert_json(&mut extra, "info_hash", info_hash_v1);
    }
    if let Some(info_hash_v2) = info_hash_v2.as_deref() {
        insert_json(&mut extra, "info_hash_v2", info_hash_v2);
    }

    insert_value(&mut extra, "seeders", result.seeders);
    insert_value(&mut extra, "peers", result.peers);
    insert_value(&mut extra, "leechers", result.leechers);
    insert_value(
        &mut extra,
        "download_volume_factor",
        result.download_volume_factor,
    );
    insert_value(
        &mut extra,
        "upload_volume_factor",
        result.upload_volume_factor,
    );
    insert_value(
        &mut extra,
        "downloadvolumefactor",
        result.download_volume_factor,
    );
    insert_value(
        &mut extra,
        "uploadvolumefactor",
        result.upload_volume_factor,
    );
    insert_value(&mut extra, "origin", result.origin.as_deref());
    insert_value(&mut extra, "source", result.source.as_deref());
    insert_value(&mut extra, "container", result.container.as_deref());
    insert_value(&mut extra, "codec", result.codec.as_deref());
    insert_value(&mut extra, "resolution", result.resolution.as_deref());

    if !result.indexer_flags.is_empty() {
        insert_json(&mut extra, "indexer_flags", &result.indexer_flags);
    }

    insert_value(&mut extra, "comment_url", result.comment_url.as_deref());
    insert_value(&mut extra, "minimum_seed_ratio", result.minimum_seed_ratio);
    insert_value(
        &mut extra,
        "minimum_seed_time_minutes",
        result.minimum_seed_time_minutes,
    );
    insert_value(
        &mut extra,
        "season_pack_seed_ratio",
        result.season_pack_seed_ratio,
    );
    insert_value(
        &mut extra,
        "season_pack_seed_time_minutes",
        result.season_pack_seed_time_minutes,
    );

    for (key, value) in provider_extra {
        extra.entry(key).or_insert(value);
    }

    extra
}

/// Lift the indexer-asserted response attrs the auto evaluator consumes (the
/// A2(2) id disambiguator and the D2 category veto) out of the merged extra map.
/// Both the SDK-typed `external_ids`/`categories` fields and the `response_*`
/// keys plugins set directly through `provider_extra` land there, so a single
/// read covers every plugin shape without touching the ABI.
fn response_attributes(
    extra: &std::collections::HashMap<String, serde_json::Value>,
) -> IndexerResponseAttributes {
    let external_ids = extra
        .get("external_ids")
        .and_then(|value| value.as_object());
    let response_id = |normalized_key: &str, response_key: &str| {
        external_ids
            .and_then(|ids| ids.get(normalized_key))
            .or_else(|| extra.get(response_key))
            .and_then(response_attribute_id)
    };

    IndexerResponseAttributes {
        tvdb_id: response_id("tvdb_id", "response_tvdbid"),
        tmdb_id: response_id("tmdb_id", "response_tmdbid"),
        imdb_id: response_id("imdb_id", "response_imdbid"),
        categories: response_attribute_categories(extra),
    }
}

/// Newznab writes `0` for "no id", so it is treated as absence rather than as a
/// disagreeing assertion.
fn response_attribute_id(value: &serde_json::Value) -> Option<String> {
    response_attribute_text(value).filter(|value| value != "0")
}

/// Categories from both the SDK-typed field and the provider-specific list, in
/// indexer order, deduped — a dual-categorized item keeps every value so the
/// set-level rule can find the compatible one.
fn response_attribute_categories(
    extra: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<String> {
    let mut categories: Vec<String> = Vec::new();
    for key in ["categories", "provider_categories"] {
        let Some(values) = extra.get(key).and_then(|value| value.as_array()) else {
            continue;
        };
        for category in values.iter().filter_map(response_attribute_text) {
            if !categories.contains(&category) {
                categories.push(category);
            }
        }
    }
    categories
}

fn response_attribute_text(value: &serde_json::Value) -> Option<String> {
    let text = match value {
        serde_json::Value::String(text) => text.trim().to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

fn explicit_source_kind(
    result: &scryer_plugin_sdk::PluginSearchResult,
    extra: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<DownloadSourceKind> {
    match result.source_kind {
        Some(IndexerSourceKind::Usenet) => Some(DownloadSourceKind::NzbUrl),
        Some(IndexerSourceKind::Torrent) => {
            if result
                .magnet_url
                .as_deref()
                .is_some_and(is_valid_magnet_uri)
                || extra
                    .get("magnet_uri")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(is_valid_magnet_uri)
            {
                Some(DownloadSourceKind::MagnetUri)
            } else {
                Some(DownloadSourceKind::TorrentFile)
            }
        }
        Some(IndexerSourceKind::Generic) | None => match result.protocol {
            Some(IndexerProtocol::Usenet) => Some(DownloadSourceKind::NzbUrl),
            Some(IndexerProtocol::Torrent) => {
                if result
                    .magnet_url
                    .as_deref()
                    .is_some_and(is_valid_magnet_uri)
                    || extra
                        .get("magnet_uri")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(is_valid_magnet_uri)
                {
                    Some(DownloadSourceKind::MagnetUri)
                } else {
                    Some(DownloadSourceKind::TorrentFile)
                }
            }
            _ => None,
        },
    }
}

fn plugin_password_hint(
    result: &scryer_plugin_sdk::PluginSearchResult,
    extra: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<String> {
    result
        .password_hint
        .as_deref()
        .and_then(|value| normalize_release_password(Some(value)))
        .or_else(|| {
            extra
                .get("password")
                .and_then(|value| value.as_str())
                .and_then(|value| normalize_release_password(Some(value)))
        })
}

fn insert_json<T: serde::Serialize>(
    extra: &mut std::collections::HashMap<String, serde_json::Value>,
    key: &str,
    value: T,
) {
    if !extra.contains_key(key)
        && let Ok(value) = serde_json::to_value(value)
    {
        if value.is_null() {
            return;
        }
        extra.insert(key.to_string(), value);
    }
}

fn insert_value<T: serde::Serialize>(
    extra: &mut std::collections::HashMap<String, serde_json::Value>,
    key: &str,
    value: T,
) {
    insert_json(extra, key, value);
}

fn host_search_response(
    client: &WasmIndexerClient,
    response: PluginSearchCallResponse,
) -> IndexerSearchResponse {
    let PluginSearchCallResponse {
        response,
        completion,
    } = response;
    let source = format!(
        "{} ({})",
        client.indexer_name,
        client.descriptor.provider_type()
    );
    let results = response
        .results
        .into_iter()
        .map(|result| {
            let extra = merge_result_extra(&result);
            let response_attributes = response_attributes(&extra);
            let password_hint = plugin_password_hint(&result, &extra);
            let source_kind = explicit_source_kind(&result, &extra).or_else(|| {
                DownloadSourceKind::infer_from_indexer_result(
                    Some(client.descriptor.plugin_type()),
                    result.download_url.as_deref(),
                    result.link.as_deref(),
                    &extra,
                )
            });

            IndexerSearchResult {
                indexer_id: None,
                source: source.clone(),
                title: result.title,
                link: result.link,
                download_url: result.download_url,
                source_kind,
                size_bytes: result.size_bytes,
                published_at: result.published_at,
                thumbs_up: result.thumbs_up,
                thumbs_down: result.thumbs_down,
                indexer_languages: (!result.languages.is_empty()).then_some(result.languages),
                indexer_subtitles: (!result.subtitles.is_empty()).then_some(result.subtitles),
                indexer_grabs: result.grabs,
                password_hint,
                candidate_token: None,
                parsed_release_metadata: None,
                quality_profile_decision: None,
                extra,
                response_attributes,
                guid: result.guid,
                info_url: result.info_url,
                provenance: None,
                queue_scope: None,
                coverage_scope: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
            }
        })
        .collect();

    IndexerSearchResponse {
        results,
        indexer_outcomes: Vec::new(),
        completion,
        api_current: response.api_current,
        api_max: response.api_max,
        grab_current: response.grab_current,
        grab_max: response.grab_max,
    }
}

async fn forward_component_strategy_event(
    client: &WasmIndexerClient,
    bytes: &[u8],
    attested: bool,
    sink: &IndexerSearchStrategyEventSink,
) -> AppResult<()> {
    let event = serde_json::from_slice::<PluginSearchStrategyEvent>(bytes).map_err(|error| {
        AppError::Repository(format!(
            "indexer component {} emitted an invalid strategy event: {error}",
            client.descriptor.id
        ))
    })?;
    let response = decode_search_result(
        event.result,
        attested,
        "indexer strategy-plan component event",
    )
    .map(|response| host_search_response(client, response));
    sink.send(IndexerSearchStrategyEvent {
        strategy_id: event.strategy_id,
        response,
    })
    .await
    .map_err(|_| AppError::canceled("indexer strategy result channel closed"))
}

#[async_trait]
impl IndexerClient for WasmIndexerClient {
    fn search_plan_capability(&self) -> Option<IndexerSearchPlanCapability> {
        let component = self.component.as_ref()?;
        if component.runtime.contract_version() != ComponentContractVersion::V1_1 {
            return None;
        }
        let ProviderDescriptor::Indexer(descriptor) = &self.descriptor.provider else {
            return None;
        };
        let capability = descriptor.strategy_plan?;
        (capability.version == 1).then_some(IndexerSearchPlanCapability {
            version: capability.version,
            max_parallel_strategies: capability.max_parallel_strategies,
        })
    }

    async fn search_plan(
        &self,
        request: IndexerSearchPlanRequest,
        mode: SearchMode,
        operation: IndexerErrorOperation,
        cancel_token: CancellationToken,
        event_sink: IndexerSearchStrategyEventSink,
    ) -> AppResult<IndexerSearchPlanSummary> {
        if self.search_plan_capability().is_none() {
            return Err(AppError::Repository(
                "indexer does not support strategy-plan search".to_string(),
            ));
        }
        if cancel_token.is_cancelled() {
            return Err(AppError::canceled("plugin indexer strategy plan canceled"));
        }
        let plan = PluginSearchPlanRequest {
            plan_id: request.plan_id,
            strategies: request
                .strategies
                .into_iter()
                .map(|strategy| {
                    let context = build_search_context(
                        &strategy.query,
                        &strategy.ids,
                        strategy.facet.as_deref(),
                        mode,
                        strategy.season,
                        strategy.episode,
                        strategy.absolute_episode,
                    );
                    PluginSearchStrategyRequest {
                        strategy_id: strategy.strategy_id,
                        labels: strategy.labels,
                        request: PluginSearchRequest {
                            query: strategy.query,
                            ids: strategy.ids,
                            facet: strategy.facet,
                            category: strategy.category,
                            categories: strategy.newznab_categories.unwrap_or_default(),
                            limit: 1000,
                            season: strategy.season,
                            episode: strategy.episode,
                            absolute_episode: strategy.absolute_episode,
                            tagged_aliases: strategy
                                .tagged_aliases
                                .into_iter()
                                .map(tagged_alias_to_sdk)
                                .collect(),
                            context: Some(context),
                        },
                    }
                })
                .collect(),
        };
        let summary = self
            .invoke_component_search_plan(&plan, operation, cancel_token, &event_sink)
            .await?;
        Ok(IndexerSearchPlanSummary {
            plan_id: summary.plan_id,
            emitted_strategy_ids: summary.emitted_strategy_ids,
        })
    }

    async fn search(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        _id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        operation: IndexerErrorOperation,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<scryer_application::IndexerSearchLearningContext>,
        cancel_token: CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        if cancel_token.is_cancelled() {
            return Err(AppError::canceled("plugin indexer search canceled"));
        }
        let context = build_search_context(
            &query,
            &ids,
            facet.as_deref(),
            mode,
            season,
            episode,
            absolute_episode,
        );
        let request = PluginSearchRequest {
            query,
            ids,
            facet,
            category,
            categories: newznab_categories.unwrap_or_default(),
            limit: 1000,
            season,
            episode,
            absolute_episode,
            tagged_aliases: tagged_aliases
                .into_iter()
                .map(tagged_alias_to_sdk)
                .collect(),
            context: Some(context),
        };

        let legacy_adapter_fallback = self.search_plan_capability().is_none();
        let response = match self
            .call_search_request(&request, operation, cancel_token.child_token())
            .await
        {
            Ok(response)
                if response.response.results.is_empty()
                    && legacy_adapter_fallback
                    && should_try_generic_search_fallback(&request) =>
            {
                let fallback_request = generic_search_fallback_request(&request, mode);
                self.call_search_request(&fallback_request, operation, cancel_token.child_token())
                    .await?
            }
            Ok(response) => response,
            Err(primary_error)
                if legacy_adapter_fallback && should_try_generic_search_fallback(&request) =>
            {
                if primary_error.is_canceled() {
                    return Err(primary_error);
                }
                tracing::debug!(
                    plugin = %self.descriptor.name,
                    error = %primary_error,
                    "plugin primary search failed; trying generic fallback"
                );
                let fallback_request = generic_search_fallback_request(&request, mode);
                self.call_search_request(&fallback_request, operation, cancel_token.child_token())
                    .await?
            }
            Err(error) => return Err(error),
        };
        Ok(host_search_response(self, response))
    }

    async fn search_stream(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        operation: IndexerErrorOperation,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<TaggedAlias>,
        learning_context: Option<scryer_application::IndexerSearchLearningContext>,
        cancel_token: CancellationToken,
        page_sink: scryer_application::IndexerSearchPageSink,
    ) -> AppResult<IndexerSearchResponse> {
        let mut response = self
            .search(
                query,
                ids,
                category,
                facet,
                id_search_facet,
                newznab_categories,
                indexer_routing,
                mode,
                operation,
                season,
                episode,
                absolute_episode,
                tagged_aliases,
                learning_context,
                cancel_token,
            )
            .await?;
        if !response.results.is_empty() {
            page_sink
                .send(std::mem::take(&mut response.results))
                .await
                .map_err(|_| AppError::canceled("indexer scoring pipeline closed"))?;
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_search_preserves_typed_reason_and_retry_delay() {
        let decoded = decode_search_result(
            PluginResult::Err(PluginError {
                code: scryer_plugin_sdk::PluginErrorCode::Temporary,
                public_message: "partial search".to_string(),
                debug_message: None,
                retry_after_seconds: None,
                details: Some(PluginErrorDetails::IndexerSearch(
                    IndexerSearchPluginError::PartialResults {
                        response: Box::new(PluginSearchResponse::default()),
                        reason: PluginIncompleteReason::PageCeilingReached,
                        retry_after_seconds: Some(45),
                    },
                )),
            }),
            true,
            "test search",
        )
        .expect("partial results remain usable");

        assert_eq!(
            decoded.completion,
            IndexerSearchCompletion::Partial {
                reason: Some(HostIncompleteReason::PageCeilingReached),
                retry_after: Some(std::time::Duration::from_secs(45)),
            }
        );
    }

    #[test]
    fn unattested_success_is_typed_as_incomplete() {
        let decoded = decode_search_result(
            PluginResult::Ok(PluginSearchResponse::default()),
            false,
            "test search",
        )
        .expect("legacy result remains usable");

        assert_eq!(
            decoded.completion,
            IndexerSearchCompletion::Partial {
                reason: Some(HostIncompleteReason::Unattested),
                retry_after: None,
            }
        );
    }

    #[test]
    fn builds_episode_id_context_for_auto_search() {
        let context = build_search_context(
            "Example Show S01E02",
            &std::collections::HashMap::from([("tvdb_id".to_string(), "123".to_string())]),
            Some("series"),
            SearchMode::Auto,
            Some(1),
            Some(2),
            None,
        );

        assert_eq!(context.request_kind, PluginSearchRequestKind::Search);
        assert_eq!(context.search_origin, PluginSearchOrigin::Automatic);
        assert_eq!(context.subject_kind, PluginSearchSubjectKind::Episode);
        assert_eq!(context.query_kind, PluginSearchQueryKind::Id);
    }

    #[test]
    fn builds_recent_context_for_category_only_auto_request() {
        let context = build_search_context(
            "",
            &std::collections::HashMap::new(),
            Some("series"),
            SearchMode::Auto,
            None,
            None,
            None,
        );

        assert_eq!(context.request_kind, PluginSearchRequestKind::Recent);
        assert_eq!(context.search_origin, PluginSearchOrigin::Rss);
        assert_eq!(context.subject_kind, PluginSearchSubjectKind::Title);
        assert_eq!(context.query_kind, PluginSearchQueryKind::Fallback);
    }

    #[test]
    fn merges_v13_result_fields_into_extra_with_top_level_precedence() {
        let result = scryer_plugin_sdk::PluginSearchResult {
            title: "Example".to_string(),
            source_kind: Some(IndexerSourceKind::Torrent),
            protocol: Some(IndexerProtocol::Torrent),
            external_ids: std::collections::HashMap::from([
                ("imdb".to_string(), "tt1234567".to_string()),
                ("tvdb".to_string(), "987".to_string()),
            ]),
            categories: vec!["TV".to_string()],
            magnet_url: Some("magnet:?xt=urn:btih:abcdef0123456789abcdef0123456789abcdef01".into()),
            info_hash_v1: Some("abcdef0123456789abcdef0123456789abcdef01".into()),
            seeders: Some(42),
            provider_extra: std::collections::HashMap::from([
                (
                    "magnet_uri".to_string(),
                    serde_json::Value::from("magnet:?existing"),
                ),
                (
                    "response_imdbid".to_string(),
                    serde_json::Value::from("tt0000000"),
                ),
                (
                    "provider_specific".to_string(),
                    serde_json::Value::from("kept"),
                ),
            ]),
            ..scryer_plugin_sdk::PluginSearchResult::default()
        };

        let extra = merge_result_extra(&result);
        assert_eq!(
            extra.get("response_imdbid"),
            Some(&serde_json::Value::from("tt1234567"))
        );
        assert_eq!(
            extra.get("response_tvdbid"),
            Some(&serde_json::Value::from("987"))
        );
        assert_eq!(extra.get("seeders"), Some(&serde_json::Value::from(42)));
        assert_eq!(
            extra.get("info_hash"),
            Some(&serde_json::Value::from(
                "abcdef0123456789abcdef0123456789abcdef01"
            ))
        );
        assert_eq!(
            extra.get("magnet_uri"),
            Some(&serde_json::Value::from(
                "magnet:?xt=urn:btih:abcdef0123456789abcdef0123456789abcdef01"
            ))
        );
        assert_eq!(
            extra.get("provider_specific"),
            Some(&serde_json::Value::from("kept"))
        );
    }

    /// A newznab item's `<attr name="tvdbid" value="393199"/>` reaches the
    /// adapter as `provider_extra["response_tvdbid"]`, and its `<category>`
    /// elements as the SDK-typed category lists. This is the shape both the
    /// search and RSS lanes deliver — they share the response parser.
    fn newznab_item(
        tvdbid: Option<&str>,
        categories: &[&str],
    ) -> scryer_plugin_sdk::PluginSearchResult {
        let mut provider_extra = std::collections::HashMap::new();
        if let Some(tvdbid) = tvdbid {
            provider_extra.insert(
                "response_tvdbid".to_string(),
                serde_json::Value::from(tvdbid),
            );
        }

        scryer_plugin_sdk::PluginSearchResult {
            title: "Tide.Chart.S02E01.1080p.WEB-DL.x264-GRP".to_string(),
            provider_categories: categories.iter().map(|value| value.to_string()).collect(),
            provider_extra,
            ..scryer_plugin_sdk::PluginSearchResult::default()
        }
    }

    #[test]
    fn captures_newznab_response_ids_and_categories() {
        let dual = newznab_item(Some("393199"), &["5000", "5070"]);
        let attributes = response_attributes(&merge_result_extra(&dual));
        assert_eq!(attributes.tvdb_id.as_deref(), Some("393199"));
        assert_eq!(attributes.categories, vec!["5000", "5070"]);

        let anime_only = newznab_item(None, &["5070"]);
        let attributes = response_attributes(&merge_result_extra(&anime_only));
        assert_eq!(attributes.tvdb_id, None);
        assert_eq!(attributes.categories, vec!["5070"]);
    }

    #[test]
    fn captures_sdk_typed_response_ids_and_dedupes_category_lists() {
        let result = scryer_plugin_sdk::PluginSearchResult {
            external_ids: std::collections::HashMap::from([
                ("tvdb".to_string(), "393199".to_string()),
                ("tmdb".to_string(), "111110".to_string()),
                ("imdb".to_string(), "tt14688458".to_string()),
            ]),
            categories: vec!["5000".to_string()],
            provider_categories: vec!["5000".to_string(), "TV > Anime".to_string()],
            ..scryer_plugin_sdk::PluginSearchResult::default()
        };

        let attributes = response_attributes(&merge_result_extra(&result));
        assert_eq!(attributes.tvdb_id.as_deref(), Some("393199"));
        assert_eq!(attributes.tmdb_id.as_deref(), Some("111110"));
        assert_eq!(attributes.imdb_id.as_deref(), Some("tt14688458"));
        assert_eq!(attributes.categories, vec!["5000", "TV > Anime"]);
    }

    #[test]
    fn response_attributes_are_empty_without_indexer_assertions() {
        let attributes = response_attributes(&merge_result_extra(
            &scryer_plugin_sdk::PluginSearchResult::default(),
        ));

        assert!(!attributes.has_external_ids());
        assert!(attributes.categories.is_empty());
    }

    #[test]
    fn response_attributes_treat_a_zero_id_as_absent() {
        // Newznab writes `0` for "no id"; it must not read as a disagreement.
        let attributes = response_attributes(&merge_result_extra(&newznab_item(Some("0"), &[])));

        assert_eq!(attributes.tvdb_id, None);
    }

    #[test]
    fn plugin_password_hint_rejects_provider_password_flags() {
        for marker in [
            "1",
            "true",
            "protected",
            "passworded",
            "0",
            "false",
            "no",
            "  ",
        ] {
            let result = scryer_plugin_sdk::PluginSearchResult {
                provider_extra: std::collections::HashMap::from([(
                    "password".to_string(),
                    serde_json::Value::from(marker),
                )]),
                ..scryer_plugin_sdk::PluginSearchResult::default()
            };
            let extra = merge_result_extra(&result);

            assert_eq!(
                plugin_password_hint(&result, &extra),
                None,
                "marker {marker:?} must not become a password hint"
            );
        }
    }

    #[test]
    fn plugin_password_hint_preserves_real_passwords() {
        let result = scryer_plugin_sdk::PluginSearchResult {
            provider_extra: std::collections::HashMap::from([(
                "password".to_string(),
                serde_json::Value::from(" archive-password "),
            )]),
            ..scryer_plugin_sdk::PluginSearchResult::default()
        };
        let extra = merge_result_extra(&result);
        assert_eq!(
            plugin_password_hint(&result, &extra).as_deref(),
            Some("archive-password")
        );

        let result = scryer_plugin_sdk::PluginSearchResult {
            password_hint: Some(" direct-password ".to_string()),
            ..scryer_plugin_sdk::PluginSearchResult::default()
        };
        let extra = merge_result_extra(&result);
        assert_eq!(
            plugin_password_hint(&result, &extra).as_deref(),
            Some("direct-password")
        );
    }

    #[test]
    fn normalizes_additional_params_for_safe_query_appending() {
        assert_eq!(
            normalize_additional_params(" ?foo=bar baz&zap=1 "),
            Some("foo=bar+baz&zap=1".to_string())
        );
        assert_eq!(
            normalize_additional_params(" &foo=bar%20baz&zap=1 "),
            Some("foo=bar+baz&zap=1".to_string())
        );
        assert_eq!(
            normalize_additional_params(" foo=bar%20baz&zap=1 "),
            Some("foo=bar+baz&zap=1".to_string())
        );
        assert_eq!(normalize_additional_params(" ? "), None);
    }

    #[test]
    fn normalizes_connection_url_and_api_path_for_sloppy_input() {
        assert_eq!(
            normalize_connection_url(" https://indexer.example.com/// "),
            Some("https://indexer.example.com".to_string())
        );
        assert_eq!(normalize_connection_url("   "), None);
        assert_eq!(
            normalize_api_path(" /api/v1/api// "),
            Some("/api/v1/api".to_string())
        );
        assert_eq!(normalize_api_path(" /// "), None);
    }

    #[test]
    fn normalizes_direct_nab_connection_urls_with_embedded_query_state() {
        assert_eq!(
            normalize_direct_nab_connection_url(
                " https://api.nzbgeek.info/api?t=search&q=legacy&cat=2000,2040&attrs=poster&apikey=secret "
            ),
            Some(NormalizedDirectNabConnection {
                base_url: "https://api.nzbgeek.info".to_string(),
                api_path: Some("/api".to_string()),
                additional_params: Some("attrs=poster".to_string()),
            })
        );
        assert_eq!(
            normalize_direct_nab_connection_url(" https://api.nzbgeek.info/nzbapi/ "),
            Some(NormalizedDirectNabConnection {
                base_url: "https://api.nzbgeek.info".to_string(),
                api_path: Some("/nzbapi".to_string()),
                additional_params: None,
            })
        );
    }

    #[test]
    fn merges_extracted_and_existing_additional_params() {
        assert_eq!(
            merge_additional_params(Some("attrs=poster&dl=1"), Some(" ?foo=bar baz&zap=1 "),),
            Some("attrs=poster&dl=1&foo=bar+baz&zap=1".to_string())
        );
    }

    #[test]
    fn direct_nab_config_forces_json_response_params() {
        let descriptor = descriptor_with_base_url_role("newznab");
        let config = sample_indexer_config("newznab", None);
        let mut entries = std::collections::HashMap::new();
        entries.insert(
            "base_url".to_string(),
            "https://api.nzbgeek.info/api?t=search&o=xml&attrs=poster".to_string(),
        );
        entries.insert(
            "additional_params".to_string(),
            " extended=0&foo=bar ".to_string(),
        );

        let normalized = normalize_indexer_config_entries(&descriptor, &config, entries);

        assert_eq!(
            normalized.get("base_url").map(String::as_str),
            Some("https://api.nzbgeek.info")
        );
        assert_eq!(normalized.get("api_path").map(String::as_str), Some("/api"));
        assert_eq!(
            normalized.get("additional_params").map(String::as_str),
            Some("o=json&extended=1&attrs=poster&foo=bar")
        );
    }

    fn descriptor_with_base_url_role(provider_type: &str) -> PluginDescriptor {
        PluginDescriptor {
            id: format!("{provider_type}_test"),
            name: "Test".to_string(),
            version: "0.0.0".to_string(),
            sdk_version: "0.0.0".to_string(),
            sdk_constraint: ">=0.0.0".to_string(),
            socket_permissions: vec![],
            provider: crate::types::ProviderDescriptor::Indexer(crate::types::IndexerDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: vec![],
                provider_profiles: vec![],
                source_kind: crate::types::IndexerSourceKind::Usenet,
                capabilities: crate::types::IndexerCapabilities::default(),
                scoring_policies: vec![],
                config_fields: vec![crate::types::ConfigFieldDef {
                    key: "base_url".to_string(),
                    label: "Base URL".to_string(),
                    field_type: crate::types::ConfigFieldType::String,
                    required: true,
                    default_value: None,
                    value_source: Default::default(),
                    role: Some(ConfigFieldRole::ConnectionUrl),
                    host_binding: None,
                    options: vec![],
                    help_text: None,
                }],
                allowed_hosts: vec![],
                rate_limit_seconds: None,
                search_semantics_version: Some(1),
                strategy_plan: None,
            }),
        }
    }

    fn sample_indexer_config(
        provider_type: &str,
        managed_parent_config_id: Option<&str>,
    ) -> IndexerConfig {
        IndexerConfig {
            id: "cfg".to_string(),
            name: "Test".to_string(),
            provider_type: provider_type.to_string(),
            base_url: String::new(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: managed_parent_config_id.map(ToString::to_string),
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn preserves_managed_prowlarr_child_proxy_path_for_newznab_provider() {
        let descriptor = descriptor_with_base_url_role("newznab");
        let config = sample_indexer_config("newznab", Some("parent"));
        let mut entries = std::collections::HashMap::new();
        entries.insert(
            "base_url".to_string(),
            "http://localhost:9696/1".to_string(),
        );
        entries.insert("api_path".to_string(), "/api".to_string());

        let normalized = normalize_indexer_config_entries(&descriptor, &config, entries);

        assert_eq!(
            normalized.get("base_url").map(String::as_str),
            Some("http://localhost:9696/1")
        );
        assert_eq!(normalized.get("api_path").map(String::as_str), Some("/api"));
        assert!(!normalized.contains_key("additional_params"));
    }

    /// Guards the invariant that makes the runtime migration safe to ship:
    /// whatever a legacy artifact observes, a command artifact for the same
    /// indexer observes too. Both constructors read from `build_runtime_inputs`
    /// today; this fails the moment either path grows its own normalization.
    #[test]
    fn both_runtimes_receive_identical_inputs() {
        let descriptor = descriptor_with_base_url_role("newznab");
        let mut config = sample_indexer_config("newznab", Some("parent"));
        config.managed_child_key = Some("child-42".to_string());
        config.config_json =
            Some(r#"{"base_url":"http://localhost:9696/1","api_path":"/api"}"#.to_string());

        let inputs = build_runtime_inputs(&descriptor, "Managed Child", &config, None);
        let spec = build_legacy_spec(Vec::new(), &descriptor, "Managed Child", &config, None);

        assert_eq!(spec.timeout, inputs.timeout);
        assert_eq!(inputs.timeout, scryer_outbound_http::INDEXER_HTTP_TIMEOUT);
        assert_eq!(spec.allowed_hosts, inputs.allowed_hosts);
        assert_eq!(
            spec.destination_cooldown_key,
            inputs.destination_cooldown_key
        );
        assert_eq!(
            inputs.destination_cooldown_key.as_deref(),
            Some("parent:child-42")
        );
        assert!(
            !inputs.config_entries.is_empty(),
            "the fixture must produce config for this comparison to mean anything"
        );
        for (key, value) in &inputs.config_entries {
            assert_eq!(
                spec.config.get(key),
                Some(value),
                "legacy spec lost normalized config key {key}"
            );
        }
    }

    #[test]
    fn command_clients_are_command_backed_and_refuse_legacy_exports() {
        let client = WasmIndexerClient::new_command(
            crate::command_abi::test_support::command_marked_wasm(),
            descriptor_with_base_url_role("newznab"),
            "Test".to_string(),
            sample_indexer_config("newznab", None),
            None,
        )
        .expect("command client builds");

        assert!(client.command.is_some());
        assert!(
            client.worker.is_none(),
            "a command client must not start a legacy worker thread"
        );

        let Err(error) = client.legacy_worker() else {
            panic!("a command client must not expose a legacy worker");
        };
        assert!(
            error.to_string().contains("cannot use a legacy export"),
            "got: {error}"
        );
    }
}
