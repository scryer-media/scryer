use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, DownloadSourceKind, IndexerClient, IndexerErrorOperation,
    IndexerErrorRecorder, IndexerLoginChallenge, IndexerResponseAttributes, IndexerRoutingPlan,
    IndexerSearchCompletion, IndexerSearchIncompleteReason as HostIncompleteReason,
    IndexerSearchPlanCapability, IndexerSearchPlanRequest, IndexerSearchPlanSummary,
    IndexerSearchResponse, IndexerSearchResult, IndexerSearchStrategyEvent,
    IndexerSearchStrategyEventSink, IndexerSearchStrategyRequest, IndexerStatsTracker,
    NullIndexerStatsTracker, PluginDynamicConfigFields, PluginFailureReason,
    ResolvedDownloadArtifact, SearchMode, extract_magnet_info_hash, is_valid_magnet_uri,
    normalize_release_password,
};
use scryer_domain::{IndexerConfig, ProxyConfig, TaggedAlias};
use scryer_plugin_sdk::command::{
    CONFIG_FIELDS_ACTION, ConfigFieldsRequest, ConfigFieldsResponse, LOGIN_CHALLENGE_ACTION,
    LoginChallengeResponse, PluginActionRequest, PluginActionResponse, PluginIndexerCommand,
    PluginIndexerCommandResult,
};
use scryer_plugin_sdk::{
    IndexerSearchIncompleteReason as PluginIncompleteReason, IndexerSearchPluginError, PluginError,
    PluginErrorCode, PluginErrorDetails, PluginResult, PluginSearchPlanRequest,
    PluginSearchPlanSummary, PluginSearchStrategyEvent, PluginSearchStrategyRequest,
    ProviderDescriptor,
};
use std::{collections::BTreeMap, sync::Arc};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::loader::{allowed_hosts_for_descriptor, parse_config_json_entries};
use crate::login_session_carry::{LoginSessionCarry, LoginSessionKey};
use crate::plugin_http_host::{IndexerErrorCaptureContext, ProxyPolicy};
use crate::types::{
    ConfigFieldRole, IndexerProtocol, IndexerSourceKind, PluginDescriptor, PluginSearchContext,
    PluginSearchOrigin, PluginSearchQueryKind, PluginSearchRequest, PluginSearchRequestKind,
    PluginSearchResponse, PluginSearchSubjectKind, normalize_external_ids,
    normalize_indexer_info_hash, tagged_alias_to_sdk,
};
use crate::wasmtime_host::component_host::{
    ComponentActor, ComponentContractVersion, ComponentHost, ComponentRuntime, GuestMemoryUsage,
    INDEXER_MEMORY_CAP_ENV, IndexerMemoryPolicy, component_strategy_event_channel,
};
use crate::wasmtime_host::engine;

/// One configured indexer, backed by its `scryer:indexer` component.
pub struct WasmIndexerClient {
    descriptor: PluginDescriptor,
    indexer_id: String,
    indexer_name: String,
    indexer_base_url: String,
    indexer_error_recorder: Arc<dyn IndexerErrorRecorder>,
    /// Counts outbound indexer requests. Defaults to the null tracker so the
    /// constructor and every existing call site keep their signatures.
    indexer_stats: Arc<dyn IndexerStatsTracker>,
    accounting: Option<scryer_application::IndexerAccountingContext>,
    /// Set when the plugin declares a login-challenge answer field: the
    /// settings under which this client shares its sign-in session.
    login_session: Option<LoginSessionKey>,
    component: Arc<ComponentIndexer>,
}

/// A retained WASI Preview 2 instance for one configured indexer. Calls are
/// serialized at this boundary, while the component may drive as much
/// upstream HTTP fanout as its own policy allows.
struct ComponentIndexer {
    runtime: Arc<ComponentRuntime>,
    host: ComponentHost,
    timeout: std::time::Duration,
    /// Store cap (already applied to `host`) and the retention watermark
    /// checked after every invocation.
    memory_policy: IndexerMemoryPolicy,
    invocation_lock: tokio::sync::Mutex<()>,
    actor: tokio::sync::Mutex<Option<ComponentActor>>,
}

/// What happened to the retained component actor after one invocation.
///
/// Wasm linear memory never shrinks, so a retained Store keeps whatever its
/// guest peaked at for the life of the process. Dropping the actor is the only
/// way to give that memory back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActorRelease {
    /// Kept for the next call: below the watermark and not an RSS call.
    Retained,
    /// Dropped because its linear memory exceeded the retention watermark.
    Watermark,
    /// Dropped unconditionally after an RSS call: RSS runs every few minutes,
    /// paging deep catch-up feeds, so re-instantiating costs far less than
    /// retaining a Store sized for the deepest page burst.
    Rss,
    /// Dropped because the call trapped, timed out, was cancelled, or hit
    /// the memory cap.
    Error,
}

impl ActorRelease {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Retained => "retained",
            Self::Watermark => "watermark",
            Self::Rss => "rss",
            Self::Error => "error",
        }
    }

    const fn drops_actor(self) -> bool {
        !matches!(self, Self::Retained)
    }

    /// The release decision for an invocation that returned to the host.
    fn after_invocation(
        operation: IndexerErrorOperation,
        usage: Option<GuestMemoryUsage>,
        policy: IndexerMemoryPolicy,
        failed: bool,
    ) -> Self {
        if failed || usage.is_some_and(|usage| usage.denied) {
            Self::Error
        } else if matches!(operation, IndexerErrorOperation::RssSync) {
            Self::Rss
        } else if usage.is_some_and(|usage| usage.resident_bytes > policy.retain_watermark_bytes) {
            Self::Watermark
        } else {
            Self::Retained
        }
    }
}

struct PluginSearchCallResponse {
    response: PluginSearchResponse,
    completion: IndexerSearchCompletion,
}

fn endpoint_resolution_error() -> AppError {
    AppError::Validation("network_endpoint_resolution_failed: Cannot resolve this indexer's configured endpoints. Check the selected definition and its endpoint settings; authorities must be HTTP(S) origins available from configuration.".into())
}

const MAX_NETWORK_ENDPOINTS_RESPONSE_BYTES: usize = 1024 * 1024;

fn validate_network_endpoints(bytes: &[u8]) -> AppResult<Vec<String>> {
    use scryer_plugin_sdk::command::ResolveNetworkEndpointsResponse;
    use std::collections::BTreeSet;
    // A size bound on the reply only, not a limit on how many hosts a
    // definition may name: every validated origin is declared.
    if bytes.len() > MAX_NETWORK_ENDPOINTS_RESPONSE_BYTES {
        return Err(endpoint_resolution_error());
    }
    let PluginResult::Ok(response) =
        serde_json::from_slice::<PluginResult<PluginActionResponse>>(bytes)
            .map_err(|_| endpoint_resolution_error())?
    else {
        return Err(endpoint_resolution_error());
    };
    let response: ResolveNetworkEndpointsResponse =
        serde_json::from_value(response.payload).map_err(|_| endpoint_resolution_error())?;
    let mut origins = BTreeSet::new();
    let mut hosts = BTreeSet::new();
    for origin in response.origins {
        let url = url::Url::parse(&origin).map_err(|_| endpoint_resolution_error())?;
        let authority = origin
            .split_once("://")
            .map(|(_, value)| value.strip_suffix('/').unwrap_or(value));
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || origin.contains(['*', '\\', '@'])
            || origin.chars().any(char::is_whitespace)
            || origin
                .split_once("://")
                .is_none_or(|(_, authority)| authority.starts_with('/'))
            || authority
                .is_none_or(|authority| authority.is_empty() || authority.contains(['/', '?', '#']))
            || !(origin.to_ascii_lowercase().starts_with("http://")
                || origin.to_ascii_lowercase().starts_with("https://"))
        {
            return Err(endpoint_resolution_error());
        }
        let host = url
            .host_str()
            .filter(|host| !host.is_empty() && host.len() <= 253)
            .ok_or_else(endpoint_resolution_error)?;
        origins.insert(url.origin().ascii_serialization());
        hosts.insert(host.to_string());
    }
    Ok(hosts.into_iter().collect())
}

impl WasmIndexerClient {
    /// Build an async WASI Preview 2 component indexer. The component is
    /// compiled once for this configured client and instantiated lazily on its
    /// first operation; its state then remains alive until a trap, timeout,
    /// cancellation, provider reload, or configuration change replaces this
    /// client.
    pub(crate) fn new_component_with_plugin_settings(
        wasm_bytes: Vec<u8>,
        descriptor: PluginDescriptor,
        indexer_name: String,
        config: IndexerConfig,
        proxy_config: Option<ProxyConfig>,
        indexer_error_recorder: Arc<dyn IndexerErrorRecorder>,
        settings: &BTreeMap<String, String>,
    ) -> Result<Self, AppError> {
        let inputs = build_runtime_inputs(&descriptor, &indexer_name, &config, proxy_config);
        let indexer_base_url = resolve_connection_url(&descriptor, Some(&inputs.config_entries))
            .unwrap_or_else(|| config.base_url.clone());
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
        let mut host_config = inputs
            .config_entries
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        crate::loader::bind_plugin_settings(&mut host_config, settings);
        let login_challenge_answer_keys = descriptor
            .config_fields()
            .iter()
            .filter(|field| field.role == Some(ConfigFieldRole::LoginChallengeAnswer))
            .map(|field| field.key.clone())
            .collect::<Vec<_>>();
        let login_session = (!login_challenge_answer_keys.is_empty()).then(|| {
            LoginSessionKey::new(
                &descriptor.id,
                inputs
                    .proxy_policy
                    .as_ref()
                    .map(|policy| policy.config.id.as_str()),
                &host_config,
                &login_challenge_answer_keys,
            )
        });
        let memory_policy = IndexerMemoryPolicy::from_env();
        let host = ComponentHost::for_indexer_with_provider_profile(
            host_config,
            inputs.allowed_hosts,
            inputs.egress_policy,
            inputs.proxy_policy,
            inputs.timeout,
            None,
            Some(memory_policy.cap_bytes),
            provider_profile,
        )
        .map_err(AppError::Repository)?;
        // A sign-in session left by a login challenge for these same settings
        // starts this client, so the answer meets the session that showed it.
        if let Some(carried) = login_session
            .as_ref()
            .and_then(|key| LoginSessionCarry::shared().carried(key, std::time::Instant::now()))
        {
            host.seed_state(carried);
        }
        let runtime = ComponentRuntime::new(engine::shared_async_engine(), &wasm_bytes)
            .map_err(AppError::Repository)?;

        tracing::debug!(
            indexer = indexer_name.as_str(),
            plugin = descriptor.name.as_str(),
            "WASI Preview 2 indexer component registered"
        );

        Ok(Self {
            descriptor,
            accounting: scryer_application::IndexerAccountingContext::for_config(&config),
            indexer_base_url,
            indexer_id: config.id,
            indexer_name,
            indexer_error_recorder,
            indexer_stats: Arc::new(NullIndexerStatsTracker),
            login_session,
            component: Arc::new(ComponentIndexer {
                runtime: Arc::new(runtime),
                host,
                timeout: inputs.timeout,
                memory_policy,
                invocation_lock: tokio::sync::Mutex::new(()),
                actor: tokio::sync::Mutex::new(None),
            }),
        })
    }

    /// Attach the tracker that counts outbound indexer API requests.
    pub fn with_indexer_stats_tracker(mut self, stats: Arc<dyn IndexerStatsTracker>) -> Self {
        self.indexer_stats = stats;
        self
    }

    pub fn with_accounting_context(
        mut self,
        accounting: Option<&scryer_application::IndexerAccountingContext>,
    ) -> Self {
        self.accounting = accounting.cloned();
        self
    }

    fn indexer_error_capture(
        &self,
        operation: IndexerErrorOperation,
    ) -> IndexerErrorCaptureContext {
        let accounting = self.indexer_request_accounting(operation);
        self.component
            .host
            .set_indexer_request_accounting(Some(accounting));
        IndexerErrorCaptureContext {
            indexer_id: self.indexer_id.clone(),
            indexer_name: self.indexer_name.clone(),
            operation,
            recorder: Arc::clone(&self.indexer_error_recorder),
            stats: Arc::clone(&self.indexer_stats),
        }
    }

    fn indexer_request_accounting(
        &self,
        operation: IndexerErrorOperation,
    ) -> crate::plugin_http_host::IndexerRequestAccountingContext {
        crate::plugin_http_host::IndexerRequestAccountingContext {
            indexer_origin: Some(self.indexer_base_url.clone()),
            indexer_id: self
                .accounting
                .as_ref()
                .map(|value| value.indexer_id.clone())
                .unwrap_or_else(|| scryer_application::CONNECTION_TEST_INDEXER_ID.to_string()),
            indexer_name: self
                .accounting
                .as_ref()
                .map(|value| value.indexer_name.clone())
                .unwrap_or_else(|| self.indexer_name.clone()),
            operation,
            stats: self.indexer_stats.clone(),
        }
    }

    async fn initialize_component(&self, token: &CancellationToken) -> AppResult<ComponentActor> {
        let indexer = &self.component;
        let resolve = self
            .descriptor
            .indexer()
            .is_some_and(|value| value.capabilities.resolve_network_endpoints);
        let deadline = tokio::time::Instant::now() + indexer.timeout;
        let initialize = async {
            if resolve {
                tokio::time::timeout_at(deadline, async {
                    // Permissions are rebuilt from this configuration, never accumulated.
                    indexer.host.set_declared_hosts(Vec::new());
                    let resolver = indexer
                        .host
                        .endpoint_resolver()
                        .map_err(|_| endpoint_resolution_error())?;
                    resolver.bind_cancellation(token.clone());
                    let mut actor = indexer
                        .runtime
                        .instantiate(&resolver)
                        .await
                        .map_err(|_| endpoint_resolution_error())?;
                    let request = serde_json::to_vec(&PluginActionRequest {
                        action: "resolve_network_endpoints".into(),
                        payload: serde_json::Value::Null,
                    })
                    .expect("static action request");
                    let bytes = actor
                        .action(request)
                        .await
                        .map_err(|_| endpoint_resolution_error())?
                        .map_err(|_| endpoint_resolution_error())?;
                    let hosts = validate_network_endpoints(&bytes)?;
                    indexer.host.set_declared_hosts(hosts);
                    Ok::<_, AppError>(())
                })
                .await
                .map_err(|_| endpoint_resolution_error())??;
            }
            tokio::time::timeout_at(deadline, indexer.runtime.instantiate(&indexer.host))
                .await
                .map_err(|_| {
                    AppError::Repository("Indexer component initialization timed out".into())
                })?
                .map_err(|_| AppError::Repository("Indexer component initialization failed".into()))
        };
        tokio::select! {
            _ = token.cancelled() => Err(AppError::canceled("Indexer initialization canceled")),
            result = initialize => result,
        }
    }

    /// Decide whether the actor survives this invocation, drop it if not, and
    /// log what the guest cost. Returns the guest's memory usage so a failure
    /// can be attributed to the memory cap.
    fn finish_actor_invocation(
        &self,
        actor: &mut Option<ComponentActor>,
        operation: &'static str,
        context: IndexerErrorOperation,
        started: std::time::Instant,
        response_bytes: usize,
        failed: bool,
    ) -> Option<GuestMemoryUsage> {
        let usage = actor.as_ref().map(ComponentActor::memory_usage);
        let release =
            ActorRelease::after_invocation(context, usage, self.component.memory_policy, failed);
        if release.drops_actor() {
            actor.take();
        }
        tracing::info!(
            indexer_id = %self.indexer_id,
            indexer = %self.indexer_name,
            plugin = %self.descriptor.id,
            operation,
            context = context.as_str(),
            duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            response_bytes,
            guest_memory_bytes = usage.map_or(0, |usage| usage.resident_bytes),
            guest_largest_memory_bytes = usage.map_or(0, |usage| usage.largest_memory_bytes),
            guest_memory_cap_bytes = usage.map_or(0, |usage| usage.cap_bytes),
            guest_memory_denied = usage.is_some_and(|usage| usage.denied),
            retain_watermark_bytes = self.component.memory_policy.retain_watermark_bytes,
            actor = release.as_str(),
            "indexer component invocation finished"
        );
        usage
    }

    /// A failed call whose Store hit the memory cap is reported as that, by
    /// indexer and cap, instead of as the trap the refused growth produced.
    fn component_call_error(
        &self,
        operation: &str,
        usage: Option<GuestMemoryUsage>,
        reason: PluginFailureReason,
        message: String,
    ) -> AppError {
        match usage {
            Some(usage) if usage.denied => AppError::plugin_host_failed(
                PluginFailureReason::ResourceLimit,
                format!(
                    "indexer component {} for indexer '{}' ({}) {operation} exceeded its guest memory cap of {} MiB and was discarded; set {INDEXER_MEMORY_CAP_ENV} to raise the cap ({message})",
                    self.descriptor.id,
                    self.indexer_name,
                    self.indexer_id,
                    usage.cap_bytes / (1024 * 1024),
                ),
            ),
            _ => AppError::plugin_host_failed(reason, message),
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
        context: IndexerErrorOperation,
        cancel_token: Option<&CancellationToken>,
    ) -> AppResult<PluginIndexerCommandResult> {
        let indexer = &self.component;
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
            *actor = Some(self.initialize_component(&token).await?);
        }

        let started = std::time::Instant::now();
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
                self.finish_actor_invocation(&mut actor, operation, context, started, 0, true);
                return Err(AppError::canceled("plugin indexer component search canceled"));
            }
            result = call => match result {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    let usage =
                        self.finish_actor_invocation(&mut actor, operation, context, started, 0, true);
                    return Err(self.component_call_error(
                        operation,
                        usage,
                        PluginFailureReason::HostFailure,
                        format!("indexer component {} {operation} failed: {error}", self.descriptor.id),
                    ));
                }
                Err(_) => {
                    let usage =
                        self.finish_actor_invocation(&mut actor, operation, context, started, 0, true);
                    return Err(self.component_call_error(
                        operation,
                        usage,
                        PluginFailureReason::TimedOut,
                        format!(
                            "indexer component {} {operation} timed out after {} ms",
                            self.descriptor.id,
                            indexer.timeout.as_millis(),
                        ),
                    ));
                }
            },
        };
        let response_bytes = response.as_ref().map_or(0, Vec::len);
        let usage = self.finish_actor_invocation(
            &mut actor,
            operation,
            context,
            started,
            response_bytes,
            false,
        );
        let bytes = match response {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(self.component_call_error(
                    operation,
                    usage,
                    PluginFailureReason::HostFailure,
                    format!(
                        "indexer component {} {operation} returned invocation error {error:?}",
                        self.descriptor.id
                    ),
                ));
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
        // Where this call took a carried sign-in session (a connection test
        // signing in with the answer) is where the next client of the same
        // settings, the saved indexer, picks it up.
        if let Some(key) = self.login_session.as_ref() {
            LoginSessionCarry::shared().refresh(
                key,
                indexer.host.state_snapshot(),
                std::time::Instant::now(),
            );
        }
        Ok(result)
    }

    async fn invoke_component_search_plan(
        &self,
        request: &PluginSearchPlanRequest,
        operation: IndexerErrorOperation,
        cancel_token: CancellationToken,
        event_sink: &IndexerSearchStrategyEventSink,
    ) -> AppResult<PluginSearchPlanSummary> {
        let indexer = &self.component;
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
        let _operation_guard = indexer.invocation_lock.lock().await;
        let mut actor = indexer.actor.lock().await;
        indexer.host.bind_cancellation(cancel_token.clone());
        indexer
            .host
            .begin_indexer_error_capture(self.indexer_error_capture(operation));
        if actor.is_none() {
            match self.initialize_component(&cancel_token).await {
                Ok(initialized) => *actor = Some(initialized),
                Err(error) => {
                    indexer.host.finish_indexer_error_capture(true);
                    return Err(error);
                }
            }
        }

        const PLAN_OPERATION: &str = "strategy_plan";
        let started = std::time::Instant::now();
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

        if let Some(error) = indexer.host.captured_host_policy_error() {
            self.finish_actor_invocation(&mut actor, PLAN_OPERATION, operation, started, 0, true);
            indexer.host.finish_indexer_error_capture(true);
            return Err(error);
        }
        if let Some(error) = event_error {
            self.finish_actor_invocation(&mut actor, PLAN_OPERATION, operation, started, 0, true);
            indexer.host.finish_indexer_error_capture(true);
            return Err(error);
        }
        let response = match call_result {
            Ok(response) => response,
            Err(error) => {
                let usage = self.finish_actor_invocation(
                    &mut actor,
                    PLAN_OPERATION,
                    operation,
                    started,
                    0,
                    true,
                );
                indexer.host.finish_indexer_error_capture(true);
                return Err(match usage {
                    Some(usage) if usage.denied && !error.is_canceled() => self
                        .component_call_error(
                            PLAN_OPERATION,
                            Some(usage),
                            PluginFailureReason::HostFailure,
                            error.to_string(),
                        ),
                    _ => error,
                });
            }
        };
        while let Ok(event) = raw_event_rx.try_recv() {
            if let Err(error) =
                forward_component_strategy_event(self, &event, attested, event_sink).await
            {
                self.finish_actor_invocation(
                    &mut actor,
                    PLAN_OPERATION,
                    operation,
                    started,
                    0,
                    true,
                );
                indexer.host.finish_indexer_error_capture(true);
                return Err(error);
            }
        }
        let usage = self.finish_actor_invocation(
            &mut actor,
            PLAN_OPERATION,
            operation,
            started,
            response.as_ref().map_or(0, Vec::len),
            false,
        );
        let bytes = response.map_err(|error| {
            self.component_call_error(
                PLAN_OPERATION,
                usage,
                PluginFailureReason::HostFailure,
                format!(
                    "indexer component {} strategy plan returned invocation error {error:?}",
                    self.descriptor.id
                ),
            )
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
        self.indexer_action_with_operation(
            action,
            query,
            IndexerErrorOperation::IndexerAction,
            false,
        )
        .await
    }

    async fn indexer_action_with_operation(
        &self,
        action: &str,
        query: BTreeMap<String, String>,
        operation: IndexerErrorOperation,
        unsupported_is_absent: bool,
    ) -> AppResult<Option<serde_json::Value>> {
        let indexer = &self.component;
        // Capture state and request accounting are owned by the component
        // host, while actor serialization is owned by `invoke_component`. Hold
        // the operation lock across setup, invocation, and cleanup so another
        // caller cannot replace this operation's accounting/error capture
        // while it waits.
        let _operation_guard = indexer.invocation_lock.lock().await;
        indexer
            .host
            .begin_indexer_error_capture(self.indexer_error_capture(operation));
        let result = match self
            .invoke_component(
                PluginIndexerCommand::Action(PluginActionRequest {
                    action: action.to_string(),
                    payload: serde_json::json!({ "query": query }),
                }),
                "indexer_action",
                operation,
                None,
            )
            .await
        {
            // The connection probe is optional for installed components. An
            // explicit `Unsupported` reply therefore selects the established
            // generic-search fallback instead of failing the connection test.
            Ok(PluginIndexerCommandResult::Action(PluginResult::Err(error)))
                if unsupported_is_absent && error.code == PluginErrorCode::Unsupported =>
            {
                Ok(None)
            }
            Ok(PluginIndexerCommandResult::Action(result)) => {
                decode_command_result::<PluginActionResponse>(
                    result,
                    "indexer indexer_action component",
                )
                .map(|response| Some(response.payload))
            }
            Ok(_) => Err(AppError::Repository(
                "indexer component returned the wrong result for indexer_action".to_string(),
            )),
            Err(error) => Err(error),
        };
        let result = indexer
            .host
            .captured_host_policy_error()
            .map_or(result, Err);
        indexer.host.finish_indexer_error_capture(result.is_err());
        result
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
        let indexer = &self.component;
        // See the action path: host capture is mutable per component, so it
        // must share the operation's serialization boundary.
        let _operation_guard = indexer.invocation_lock.lock().await;
        indexer
            .host
            .begin_indexer_error_capture(self.indexer_error_capture(operation));
        let result = match self
            .invoke_component(
                PluginIndexerCommand::Search(request.clone()),
                "indexer_search",
                operation,
                Some(&cancel_token),
            )
            .await
        {
            Ok(PluginIndexerCommandResult::Search(result)) => {
                decode_search_result(result, attested, "indexer indexer_search component")
            }
            Ok(_) => Err(AppError::Repository(
                "indexer component returned the wrong result for indexer_search".to_string(),
            )),
            Err(error) => Err(error),
        };
        let result = match result {
            Err(error) => match indexer
                .host
                .captured_host_policy_error()
                .or_else(|| indexer.host.captured_newznab_quota_error())
            {
                Some(quota_error) => Err(quota_error),
                None => Err(error),
            },
            ok => ok,
        };
        let result = indexer
            .host
            .captured_host_policy_error()
            .map_or(result, Err);
        indexer.host.finish_indexer_error_capture(result.is_err());
        result
    }
}

/// The image a `checkCaptcha` reply carries, as `(content type, base64)`, or
/// `None` when the sign-in asks for nothing. Only raster images of a bounded
/// size are passed on to a browser.
fn decode_login_challenge(
    plugin_id: &str,
    payload: serde_json::Value,
) -> AppResult<Option<(String, String)>> {
    use base64::Engine as _;
    use scryer_application::login_challenge_refusal as refusal;

    let response: LoginChallengeResponse = serde_json::from_value(payload).map_err(|error| {
        warn!(plugin = plugin_id, error = %error, "login challenge reply was malformed");
        AppError::validation_refused(
            refusal::MALFORMED,
            "The indexer plugin's sign-in challenge could not be read.",
        )
    })?;
    let Some(request) = response.captcha_request else {
        return Ok(None);
    };
    if !request.challenge_type.trim().eq_ignore_ascii_case("image") {
        return Err(AppError::validation_refused(
            refusal::KIND_UNSUPPORTED,
            format!(
                "The indexer asks for a '{}' sign-in challenge, which Scryer cannot show.",
                request.challenge_type.trim()
            ),
        ));
    }
    let content_type = request
        .content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !refusal::IMAGE_CONTENT_TYPES.contains(&content_type.as_str()) {
        return Err(AppError::validation_refused(
            refusal::IMAGE_TYPE_UNSUPPORTED,
            format!(
                "The indexer's sign-in challenge is a '{content_type}' file, not an image Scryer shows."
            ),
        ));
    }
    let encoded = request.image_data.trim();
    // Base64 needs four characters for every three bytes, so a reply this
    // long is over the limit without decoding it.
    if encoded.len() > refusal::MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err(image_too_large());
    }
    let image = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| {
            AppError::validation_refused(
                refusal::MALFORMED,
                "The indexer plugin's sign-in challenge image could not be read.",
            )
        })?;
    if image.is_empty() {
        return Err(AppError::validation_refused(
            refusal::MALFORMED,
            "The indexer plugin's sign-in challenge image is empty.",
        ));
    }
    if image.len() > refusal::MAX_IMAGE_BYTES {
        return Err(image_too_large());
    }
    Ok(Some((
        content_type,
        base64::engine::general_purpose::STANDARD.encode(image),
    )))
}

fn image_too_large() -> AppError {
    AppError::validation_refused(
        scryer_application::login_challenge_refusal::IMAGE_TOO_LARGE,
        "The indexer's sign-in challenge image is too large to show.",
    )
}

fn decode_command_result<T>(result: PluginResult<T>, context: &str) -> AppResult<T> {
    match result {
        PluginResult::Ok(value) => Ok(value),
        PluginResult::Err(error) => Err(AppError::plugin_failed(&error, context)),
    }
}

/// The `grab` action's payload: the URL the provider finally resolved, plus the
/// artifact bytes it fetched behind its own authentication. Magnets carry no
/// body. Fields the provider does not know are simply absent.
#[derive(serde::Deserialize)]
struct IndexerGrabPayload {
    url: String,
    #[serde(default)]
    body: Vec<u8>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    info_hash_hint: Option<String>,
    #[serde(default)]
    source_kind: Option<String>,
    #[serde(default)]
    response_headers: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    password_candidates: Vec<String>,
}

/// Turn one `grab` action result into a router artifact.
///
/// An `Unsupported` plugin error is the provider saying it has no grab flow, so
/// it yields `None` and the router keeps its own direct/solver fallback. Every
/// other plugin error is a real transport or authentication failure and must
/// propagate: silently falling back would fetch the tracker's login page and
/// hand it to a download client as a torrent.
fn decode_grab_result(
    result: PluginResult<PluginActionResponse>,
) -> AppResult<Option<ResolvedDownloadArtifact>> {
    let payload = match result {
        PluginResult::Ok(response) => response.payload,
        PluginResult::Err(error) if error.code == PluginErrorCode::Unsupported => return Ok(None),
        PluginResult::Err(error) => {
            return Err(AppError::plugin_failed(
                &error,
                "indexer download resolution",
            ));
        }
    };
    // Shipped Newznab-family plugins report unknown actions with an empty
    // object. Treat only that legacy reply as Unsupported; nonempty malformed
    // replies must still fail instead of triggering another download.
    if payload
        .as_object()
        .is_some_and(|payload| payload.is_empty())
    {
        return Ok(None);
    }
    let payload: IndexerGrabPayload = serde_json::from_value(payload).map_err(|_| {
        AppError::Repository("indexer download resolution returned an invalid grab payload".into())
    })?;
    let url = payload.url.trim().to_string();
    if is_valid_magnet_uri(&url) {
        let info_hash_hint = extract_magnet_info_hash(&url)
            .and_then(|hash| normalize_indexer_info_hash(Some(&hash)).or(Some(hash)));
        return Ok(Some(ResolvedDownloadArtifact::Magnet {
            uri: url,
            info_hash_hint,
        }));
    }
    if payload.body.is_empty() {
        return Err(AppError::Repository(
            "indexer download resolution returned an empty grab payload".to_string(),
        ));
    }

    let is_nzb = payload
        .source_kind
        .as_deref()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("nzb"))
        || payload.content_type.as_deref().is_some_and(|kind| {
            kind.split(';')
                .next()
                .is_some_and(|kind| matches!(kind.trim(), "application/x-nzb" | "application/nzb"))
        })
        || payload
            .file_name
            .as_deref()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".nzb"));
    if is_nzb {
        let mut password_candidates = scryer_application::DownloadPasswordCandidates::default();
        for (name, value) in &payload.response_headers {
            if name.eq_ignore_ascii_case("x-dnzb-password") {
                match value {
                    serde_json::Value::String(value) => password_candidates.push(value),
                    serde_json::Value::Array(values) => {
                        for value in values.iter().filter_map(serde_json::Value::as_str) {
                            password_candidates.push(value);
                        }
                    }
                    _ => {}
                }
            }
        }
        password_candidates.extend(payload.password_candidates.iter().map(String::as_str));
        return Ok(Some(ResolvedDownloadArtifact::Nzb {
            bytes: payload.body,
            file_name: payload.file_name,
            content_type: payload.content_type,
            password_candidates,
        }));
    }

    Ok(Some(ResolvedDownloadArtifact::TorrentFile {
        bytes: payload.body,
        file_name: payload.file_name,
        content_type: payload.content_type,
        info_hash_hint: payload.info_hash_hint,
    }))
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
        PluginResult::Err(error) => Err(indexer_search_plugin_error(error, context)),
    }
}

/// A credential the indexer rejected is a stable reason the client translates
/// (`PLUGIN_INVALID_API_KEY` and friends), so a plugin that names the code
/// needs nothing more. Plugins built before the shared codes said only the
/// generic sentence and named the rejected key in their debug text; promote
/// those to the same reason without relaying the untrusted detail itself.
fn indexer_search_plugin_error(error: PluginError, context: &str) -> AppError {
    if is_legacy_api_key_rejection(&error) {
        let promoted = PluginError {
            code: PluginErrorCode::from("invalid_api_key"),
            ..error
        };
        return AppError::plugin_failed(&promoted, context);
    }
    AppError::plugin_failed(&error, context)
}

fn is_legacy_api_key_rejection(error: &PluginError) -> bool {
    error.public_message == "indexer component failed"
        && error.debug_message.as_deref().is_some_and(|message| {
            let normalized = message.to_ascii_lowercase();
            (normalized.contains("api key") || normalized.contains("apikey"))
                && (normalized.contains("invalid")
                    || normalized.contains("rejected")
                    || normalized.contains("error"))
        })
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

struct IndexerRuntimeInputs {
    config_entries: std::collections::HashMap<String, String>,
    allowed_hosts: Vec<String>,
    egress_policy: scryer_outbound_http::PluginEgressPolicy,
    timeout: std::time::Duration,
    proxy_policy: Option<ProxyPolicy>,
    /// Managed-destination cooldown key: a shared upstream (a Prowlarr parent,
    /// say) must throttle as ONE destination rather than once per child.
    ///
    /// Computed but not consumed: `CommandHost::for_indexer` carried it into
    /// the HTTP host, and that constructor died with the command-ABI indexer.
    /// `ComponentHost::for_indexer_with_provider_profile` has no equivalent
    /// parameter, so the component indexer has no cooldown-key seam at all.
    /// Kept — and computed, and covered by `build_runtime_inputs`'s tests — so
    /// the gap stays visible for whoever adds that seam to the component host.
    #[allow(dead_code)]
    destination_cooldown_key: Option<String>,
}

fn build_runtime_inputs(
    descriptor: &PluginDescriptor,
    indexer_name: &str,
    config: &IndexerConfig,
    proxy_config: Option<ProxyConfig>,
) -> IndexerRuntimeInputs {
    let proxy_config = proxy_config.filter(|_| {
        !config.is_prowlarr_nab_proxy()
            && !config.provider_type.trim().eq_ignore_ascii_case("prowlarr")
    });
    let config_entries = build_config_entries(descriptor, indexer_name, config);
    let connection_url = resolve_connection_url(descriptor, config_entries.as_ref());
    let allowed_hosts = allowed_hosts_for_descriptor(
        descriptor,
        connection_url.as_deref(),
        config.config_json.as_deref(),
    );
    let egress_policy = crate::loader::operator_egress_policy_for_descriptor(
        connection_url.as_deref(),
        config.config_json.as_deref(),
    );
    let timeout = scryer_outbound_http::effective_indexer_timeout(
        proxy_config
            .as_ref()
            .map(|config| config.request_timeout_seconds),
    );
    IndexerRuntimeInputs {
        config_entries: config_entries.unwrap_or_default(),
        allowed_hosts,
        egress_policy,
        timeout,
        proxy_policy: proxy_config.map(|proxy_config| ProxyPolicy {
            consumer_id: config.id.clone(),
            consumer_name: indexer_name.to_string(),
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

#[expect(
    clippy::too_many_arguments,
    reason = "the search context is derived from the full request envelope"
)]
fn build_search_context(
    query: &str,
    ids: &std::collections::HashMap<String, String>,
    facet: Option<&str>,
    mode: SearchMode,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
    year: Option<i32>,
) -> PluginSearchContext {
    // The year never participates in request-shape classification: it is a
    // qualifier on the subject, not evidence that a search was asked for.
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

    // A release year only describes a whole-title movie subject. A
    // season/episode-scoped search has no single release year, and a series
    // year is not one — so anything else is dropped here rather than sent to a
    // plugin as a qualifier the host cannot vouch for. The facet is absent on
    // the generic fallback form, which is still the same movie subject.
    let subject_year = year.filter(|_| {
        season.is_none()
            && episode.is_none()
            && absolute_episode.is_none()
            && matches!(normalized_facet.as_deref(), None | Some("movie"))
    });

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
        year: subject_year,
        ..PluginSearchContext::default()
    }
}

fn should_try_generic_search_fallback(request: &PluginSearchRequest) -> bool {
    !request.query.trim().is_empty() && !request.ids.is_empty()
}

/// The plugin request for one host search strategy, carrying the RSS
/// catch-up marker when the host set one.
fn plugin_search_request_from_strategy(
    strategy: IndexerSearchStrategyRequest,
    mode: SearchMode,
) -> PluginSearchRequest {
    let context = build_search_context(
        &strategy.query,
        &strategy.ids,
        strategy.facet.as_deref(),
        mode,
        strategy.season,
        strategy.episode,
        strategy.absolute_episode,
        strategy.year,
    );
    PluginSearchRequest {
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
        rss_catch_up: strategy
            .rss_catch_up
            .map(|marker| scryer_plugin_sdk::PluginRssCatchUp {
                last_seen_published_at: marker
                    .last_seen_published_at
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                last_seen_identity: marker.last_seen_identity,
            }),
        page_cursor: None,
    }
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
        request.context.as_ref().and_then(|context| context.year),
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
        Some(IndexerSourceKind::Http) => Some(DownloadSourceKind::HttpResource),
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
            Some(IndexerProtocol::Http) => Some(DownloadSourceKind::HttpResource),
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
                // This legacy provider-extra field mixes protection markers
                // with actual passwords. The typed password_hint is literal.
                .filter(|value| {
                    !matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes" | "protected" | "passworded" | "0" | "false" | "no"
                    )
                })
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
    // A successful page is not a completed search. This boundary also handles
    // strategy-plan events from artifacts whose embedded capability was lost.
    let completion = if response.next_cursor.is_some() && completion.is_complete() {
        IndexerSearchCompletion::Partial {
            reason: None,
            retry_after: None,
        }
    } else {
        completion
    };
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
                release_listing_json: None,
                http: result.http,
            }
        })
        .collect();

    IndexerSearchResponse {
        next_cursor: response.next_cursor,
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

/// Longest page cursor the host carries between pages. A cursor is a short
/// provider position; anything longer is treated as malformed.
const MAX_PAGE_CURSOR_BYTES: usize = 4096;

/// Pages one read follows when the caller set no page budget. A provider
/// whose cursors never run out cannot hold a search open past it.
const MAX_UNBUDGETED_PAGES: u32 = 100;

/// One paged search in progress: the pages read so far and where the next
/// one starts.
struct PagedRead {
    limit: usize,
    page_budget: Option<u32>,
    cursor: Option<String>,
    /// Every cursor this read started a page from; bounded by the page
    /// budget or [`MAX_UNBUDGETED_PAGES`].
    visited: std::collections::HashSet<String>,
    pages: u32,
    unattested: bool,
    merged: Option<PluginSearchResponse>,
}

/// What a paged search does after a page.
enum PagedStep {
    /// Read the next page from this cursor.
    Next(String),
    /// Stop; the read is answered.
    Done(PluginSearchCallResponse),
}

impl PagedRead {
    fn new(request: &PluginSearchRequest, page_budget: Option<u32>) -> Self {
        Self {
            limit: request.limit,
            page_budget,
            cursor: request.page_cursor.clone(),
            visited: request.page_cursor.iter().cloned().collect(),
            pages: 0,
            unattested: false,
            merged: None,
        }
    }

    fn finish(
        &mut self,
        completion: IndexerSearchCompletion,
        next_cursor: Option<String>,
    ) -> PluginSearchCallResponse {
        let mut response = self.merged.take().unwrap_or_default();
        response.next_cursor = next_cursor;
        // A provider cursor advances past the entire page, so cutting results
        // back to the soft limit would lose the ones a resume skips over. A
        // terminal page keeps its tail too: nothing will ask for it again.
        PluginSearchCallResponse {
            response,
            completion,
        }
    }

    fn stopped(&self, reason: Option<HostIncompleteReason>) -> IndexerSearchCompletion {
        IndexerSearchCompletion::Partial {
            reason: reason.or(self.unattested.then_some(HostIncompleteReason::Unattested)),
            retry_after: None,
        }
    }

    /// Takes in one page and decides whether to read on.
    fn absorb(&mut self, page: PluginSearchCallResponse) -> PagedStep {
        self.pages = self.pages.saturating_add(1);
        let PluginSearchCallResponse {
            response: mut page_response,
            completion: page_completion,
        } = page;
        let returned_cursor = page_response.next_cursor.take();
        match self.merged.as_mut() {
            Some(merged) => {
                merged.results.append(&mut page_response.results);
                merged.api_current = page_response.api_current.or(merged.api_current);
                merged.api_max = page_response.api_max.or(merged.api_max);
                merged.grab_current = page_response.grab_current.or(merged.grab_current);
                merged.grab_max = page_response.grab_max.or(merged.grab_max);
            }
            None => self.merged = Some(page_response),
        }
        match page_completion {
            IndexerSearchCompletion::Complete => {}
            IndexerSearchCompletion::Partial {
                reason: Some(HostIncompleteReason::Unattested),
                ..
            } => self.unattested = true,
            // The provider refused to go further; there is nothing to resume.
            IndexerSearchCompletion::Partial {
                reason:
                    Some(
                        HostIncompleteReason::PageCeilingReached
                        | HostIncompleteReason::SaturatedPartition,
                    ),
                ..
            } => {
                return PagedStep::Done(self.finish(page_completion, None));
            }
            // A page that ended partial is read again from its cursor.
            IndexerSearchCompletion::Partial { .. } => {
                let cursor = self.cursor.clone();
                return PagedStep::Done(self.finish(page_completion, cursor));
            }
        }
        let Some(next) = returned_cursor else {
            let completion = if self.unattested {
                self.stopped(None)
            } else {
                IndexerSearchCompletion::Complete
            };
            return PagedStep::Done(self.finish(completion, None));
        };
        if next.is_empty() || next.len() > MAX_PAGE_CURSOR_BYTES || self.visited.contains(&next) {
            // A cursor that does not move the read forward cannot be
            // followed: the search is as complete as the provider lets it be.
            let completion = self.stopped(Some(HostIncompleteReason::PageCeilingReached));
            return PagedStep::Done(self.finish(completion, None));
        }
        let read = self
            .merged
            .as_ref()
            .map_or(0, |merged| merged.results.len());
        if self.limit > 0 && read >= self.limit {
            let completion = self.stopped(Some(HostIncompleteReason::PageCeilingReached));
            return PagedStep::Done(self.finish(completion, Some(next)));
        }
        if self.page_budget.is_some_and(|budget| self.pages >= budget) {
            let completion = self.stopped(None);
            return PagedStep::Done(self.finish(completion, Some(next)));
        }
        if self.page_budget.is_none() && self.pages >= MAX_UNBUDGETED_PAGES {
            let completion = self.stopped(Some(HostIncompleteReason::PageCeilingReached));
            return PagedStep::Done(self.finish(completion, Some(next)));
        }
        self.visited.insert(next.clone());
        self.cursor = Some(next.clone());
        PagedStep::Next(next)
    }

    /// A page failed. With nothing read yet the error stands; otherwise the
    /// pages already read are kept and the failed one is asked again from the
    /// cursor that reached it.
    fn fail(mut self, error: AppError) -> AppResult<PluginSearchCallResponse> {
        if self.merged.is_none() || error.is_canceled() {
            return Err(error);
        }
        let cursor = self.cursor.clone();
        Ok(self.finish(
            IndexerSearchCompletion::Partial {
                reason: Some(HostIncompleteReason::UpstreamFailure),
                retry_after: None,
            },
            cursor,
        ))
    }
}

impl WasmIndexerClient {
    /// Whether the plugin answers searches one provider page at a time and
    /// resumes from a cursor it returns.
    fn paged_search(&self) -> bool {
        matches!(
            &self.descriptor.provider,
            ProviderDescriptor::Indexer(descriptor)
                if descriptor
                    .capabilities
                    .limits
                    .as_ref()
                    .is_some_and(|limits| limits.paged_search)
        )
    }

    /// The strategy-plan capability the plugin declares, whether or not the
    /// host routes searches through it.
    fn declared_search_plan_capability(&self) -> Option<IndexerSearchPlanCapability> {
        let component = &self.component;
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

    /// Reads a paged plugin's results one provider page per invocation,
    /// following its cursor until the provider is exhausted, the request's
    /// result limit is met, or `page_budget` pages were read. The response
    /// carries the cursor to resume from whenever the provider has more.
    async fn follow_paged_search(
        &self,
        request: PluginSearchRequest,
        operation: IndexerErrorOperation,
        cancel_token: CancellationToken,
        page_budget: Option<u32>,
    ) -> AppResult<PluginSearchCallResponse> {
        let mut read = PagedRead::new(&request, page_budget);
        let mut page_request = request;
        loop {
            if cancel_token.is_cancelled() {
                return Err(AppError::canceled("plugin indexer search canceled"));
            }
            let page = match self
                .call_search_request(&page_request, operation, cancel_token.child_token())
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    tracing::debug!(
                        plugin = %self.descriptor.name,
                        pages = read.pages,
                        %error,
                        "paged indexer search page failed"
                    );
                    return read.fail(error);
                }
            };
            match read.absorb(page) {
                PagedStep::Next(cursor) => page_request.page_cursor = Some(cursor),
                PagedStep::Done(response) => return Ok(response),
            }
        }
    }

    /// Sends one search request, retrying a legacy adapter's empty or failed
    /// identifier search once as a plain title search. A paged plugin is read
    /// page by page instead (see [`Self::follow_paged_search`]).
    async fn search_with_request(
        &self,
        request: PluginSearchRequest,
        mode: SearchMode,
        operation: IndexerErrorOperation,
        cancel_token: CancellationToken,
        page_budget: Option<u32>,
    ) -> AppResult<IndexerSearchResponse> {
        if self.paged_search() && (!request.query.trim().is_empty() || !request.ids.is_empty()) {
            let response = self
                .follow_paged_search(request, operation, cancel_token, page_budget)
                .await?;
            return Ok(host_search_response(self, response));
        }
        let legacy_adapter_fallback = self.declared_search_plan_capability().is_none();
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
        // Preserve unexpected cursors too: the boundary below must not mistake
        // a successful page from a misdeclared plugin for an exhausted search.
        Ok(host_search_response(self, response))
    }
}

/// Upper bound on one `config_fields` answer. A tracker definition carries a
/// few dozen settings; anything near this is a broken plugin, not a form.
const MAX_CONFIG_FIELDS_RESPONSE_BYTES: usize = 1024 * 1024;

impl WasmIndexerClient {
    /// Run the reserved `config_fields` action on a throwaway instance.
    ///
    /// The answer is a function of the trigger values alone, so the instance
    /// skips endpoint resolution (no network grants are derived from draft
    /// values) and is dropped afterwards instead of replacing the retained
    /// actor.
    async fn call_config_fields_action(&self, request: Vec<u8>) -> AppResult<Vec<u8>> {
        let indexer = &self.component;
        let _operation_guard = indexer.invocation_lock.lock().await;
        let _actor_guard = indexer.actor.lock().await;
        let token = CancellationToken::new();
        indexer.host.bind_cancellation(token);
        let plugin = self.descriptor.id.as_str();
        let call = async {
            let mut actor = indexer
                .runtime
                .instantiate(&indexer.host)
                .await
                .map_err(|error| {
                    AppError::Repository(format!(
                        "indexer component {plugin} config_fields initialization failed: {error}"
                    ))
                })?;
            actor
                .action(request)
                .await
                .map_err(|error| {
                    AppError::Repository(format!(
                        "indexer component {plugin} config_fields failed: {error}"
                    ))
                })?
                .map_err(|error| {
                    AppError::Repository(format!(
                        "indexer component {plugin} config_fields returned invocation error {error:?}"
                    ))
                })
        };
        tokio::time::timeout(indexer.timeout, call)
            .await
            .map_err(|_| {
                AppError::Repository(format!(
                    "indexer component {plugin} config_fields timed out after {} ms",
                    indexer.timeout.as_millis()
                ))
            })?
    }
}

/// Decode one `config_fields` answer. An `unsupported` error means the plugin
/// has no fields to add, not that the form is broken.
fn decode_config_fields_response(
    plugin: &str,
    bytes: &[u8],
) -> AppResult<Option<PluginDynamicConfigFields>> {
    if bytes.len() > MAX_CONFIG_FIELDS_RESPONSE_BYTES {
        return Err(AppError::Repository(format!(
            "indexer component {plugin} config_fields response exceeds {MAX_CONFIG_FIELDS_RESPONSE_BYTES} bytes"
        )));
    }
    let result =
        serde_json::from_slice::<PluginResult<PluginActionResponse>>(bytes).map_err(|error| {
            AppError::Repository(format!(
                "indexer component {plugin} config_fields returned invalid JSON response: {error}"
            ))
        })?;
    if let PluginResult::Err(error) = &result
        && error.code == PluginErrorCode::Unsupported
    {
        return Ok(None);
    }
    let response = decode_command_result(result, "indexer config_fields component")?;
    let response: ConfigFieldsResponse =
        serde_json::from_value(response.payload).map_err(|error| {
            AppError::Repository(format!(
                "indexer component {plugin} config_fields returned an invalid payload: {error}"
            ))
        })?;
    Ok(Some(PluginDynamicConfigFields {
        fields: crate::types::config_fields_to_domain(&response.fields),
        hidden_field_keys: response.hidden_field_keys,
    }))
}

#[async_trait]
impl IndexerClient for WasmIndexerClient {
    async fn dynamic_config_fields(
        &self,
        values: &BTreeMap<String, String>,
    ) -> AppResult<Option<PluginDynamicConfigFields>> {
        if self
            .descriptor
            .indexer()
            .and_then(|indexer| indexer.capabilities.dynamic_config_fields.as_ref())
            .is_none()
        {
            return Ok(None);
        }
        let payload = serde_json::to_value(ConfigFieldsRequest {
            values: values.clone(),
        })
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let request = serde_json::to_vec(&PluginActionRequest {
            action: CONFIG_FIELDS_ACTION.to_string(),
            payload,
        })
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let bytes = self.call_config_fields_action(request).await?;
        decode_config_fields_response(&self.descriptor.id, &bytes)
    }

    async fn login_challenge(&self) -> AppResult<Option<IndexerLoginChallenge>> {
        let Some(answer_field_key) = self
            .descriptor
            .config_fields()
            .iter()
            .find(|field| field.role == Some(ConfigFieldRole::LoginChallengeAnswer))
            .map(|field| field.key.clone())
        else {
            return Ok(None);
        };
        let Some(payload) = self
            .indexer_action_with_operation(
                LOGIN_CHALLENGE_ACTION,
                BTreeMap::new(),
                IndexerErrorOperation::ConnectionTest,
                true,
            )
            .await?
        else {
            return Ok(None);
        };
        let Some(challenge) = decode_login_challenge(&self.descriptor.id, payload)? else {
            return Ok(None);
        };
        if let Some(key) = self.login_session.as_ref() {
            LoginSessionCarry::shared().remember(
                key.clone(),
                self.component.host.state_snapshot(),
                std::time::Instant::now(),
            );
        }
        Ok(Some(IndexerLoginChallenge {
            answer_field_key,
            content_type: challenge.0,
            image_base64: challenge.1,
        }))
    }

    async fn probe_connection(&self) -> AppResult<bool> {
        if !self
            .descriptor
            .provider_type()
            .eq_ignore_ascii_case("newznab")
        {
            return Ok(false);
        }

        let payload = self
            .indexer_action_with_operation(
                "newznabConnectionTest",
                BTreeMap::new(),
                IndexerErrorOperation::ConnectionTest,
                true,
            )
            .await?;
        let Some(payload) = payload else {
            return Ok(false);
        };
        let Some(values) = payload.as_object() else {
            return Err(AppError::Repository(
                "indexer connection probe returned an invalid response".to_string(),
            ));
        };
        if values.is_empty() {
            return Ok(false);
        }
        if values.get("version").and_then(serde_json::Value::as_u64) == Some(1)
            && values.get("validated").and_then(serde_json::Value::as_bool) == Some(true)
        {
            return Ok(true);
        }

        Err(AppError::Repository(
            "indexer connection probe returned an invalid response".to_string(),
        ))
    }

    fn search_plan_capability(&self) -> Option<IndexerSearchPlanCapability> {
        // A plan answers each strategy with one event, so it cannot follow a
        // paged plugin's cursor; such a plugin is searched strategy by
        // strategy.
        if self.paged_search() {
            return None;
        }
        self.declared_search_plan_capability()
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
                .map(|mut strategy| PluginSearchStrategyRequest {
                    strategy_id: std::mem::take(&mut strategy.strategy_id),
                    labels: std::mem::take(&mut strategy.labels),
                    request: plugin_search_request_from_strategy(strategy, mode),
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
        year: Option<i32>,
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
            year,
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
            rss_catch_up: None,
            page_cursor: None,
        };
        self.search_with_request(request, mode, operation, cancel_token, None)
            .await
    }

    async fn search_strategy(
        &self,
        request: IndexerSearchStrategyRequest,
        mode: SearchMode,
        operation: IndexerErrorOperation,
        cancel_token: CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        if cancel_token.is_cancelled() {
            return Err(AppError::canceled("plugin indexer search canceled"));
        }
        let mut request = request;
        let page_cursor = request.page_cursor.take();
        let page_budget = request.page_budget;
        let mut request = plugin_search_request_from_strategy(request, mode);
        request.page_cursor = page_cursor;
        self.search_with_request(request, mode, operation, cancel_token, page_budget)
            .await
    }

    async fn search_raw_text(
        &self,
        request: scryer_application::RawTextSearchRequest,
        cancel_token: CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        if cancel_token.is_cancelled() {
            return Err(AppError::canceled("plugin indexer search canceled"));
        }
        let mode = SearchMode::Interactive;
        let context = build_search_context(
            &request.query,
            &Default::default(),
            None,
            mode,
            None,
            None,
            None,
            None,
        );
        let request = PluginSearchRequest {
            query: request.query,
            categories: request.categories,
            limit: request.limit.unwrap_or(100) as usize,
            context: Some(context),
            ..PluginSearchRequest::default()
        };
        self.search_with_request(
            request,
            mode,
            IndexerErrorOperation::InteractiveSearch,
            cancel_token,
            None,
        )
        .await
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
        year: Option<i32>,
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
                year,
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

    async fn resolve_download(
        &self,
        download_url: &str,
    ) -> AppResult<Option<ResolvedDownloadArtifact>> {
        let indexer = &self.component;
        let _operation_guard = indexer.invocation_lock.lock().await;
        indexer.host.begin_indexer_error_capture(
            self.indexer_error_capture(IndexerErrorOperation::IndexerAction),
        );
        let request = PluginActionRequest {
            action: "grab".to_string(),
            payload: serde_json::json!({ "url": download_url }),
        };
        let result = self
            .invoke_component(
                PluginIndexerCommand::Action(request),
                "indexer_grab",
                IndexerErrorOperation::IndexerAction,
                None,
            )
            .await;
        let result = match result {
            Ok(PluginIndexerCommandResult::Action(result)) => decode_grab_result(result),
            Ok(_) => Err(AppError::Repository(
                "Indexer component returned the wrong result for grab".into(),
            )),
            Err(error) => Err(error),
        };
        let result = indexer
            .host
            .captured_host_policy_error()
            .map_or(result, Err);
        indexer.host.finish_indexer_error_capture(result.is_err());
        result
    }

    async fn resolve_http_release(
        &self,
        release: &scryer_plugin_sdk::PluginHttpRelease,
    ) -> AppResult<Option<scryer_plugin_sdk::PluginHttpRelease>> {
        let Some(resolve) = release.resolve.as_ref() else {
            return Ok(None);
        };
        let payload = serde_json::to_value(release).map_err(|error| {
            AppError::Repository(format!(
                "failed to encode http release for resolve: {error}"
            ))
        })?;
        let indexer = &self.component;
        let _operation_guard = indexer.invocation_lock.lock().await;
        indexer.host.begin_indexer_error_capture(
            self.indexer_error_capture(IndexerErrorOperation::IndexerAction),
        );
        let request = PluginActionRequest {
            action: resolve.action.clone(),
            payload,
        };
        let result = self
            .invoke_component(
                PluginIndexerCommand::Action(request),
                "indexer_http_resolve",
                IndexerErrorOperation::IndexerAction,
                None,
            )
            .await;
        let result = match result {
            Ok(PluginIndexerCommandResult::Action(result)) => decode_http_resolve_result(result),
            Ok(_) => Err(AppError::Repository(
                "Indexer component returned the wrong result for http resolve".into(),
            )),
            Err(error) => Err(error),
        };
        let result = indexer
            .host
            .captured_host_policy_error()
            .map_or(result, Err);
        indexer.host.finish_indexer_error_capture(result.is_err());
        result
    }
}

/// The `resolve` action's reply: a fresh `PluginHttpRelease`, or `None` when
/// the plugin reports the action as unsupported. A malformed reply fails the
/// grab rather than submitting a release the indexer did not describe.
fn decode_http_resolve_result(
    result: PluginResult<PluginActionResponse>,
) -> AppResult<Option<scryer_plugin_sdk::PluginHttpRelease>> {
    let payload = match result {
        PluginResult::Ok(response) => response.payload,
        PluginResult::Err(error) if error.code == PluginErrorCode::Unsupported => return Ok(None),
        PluginResult::Err(error) => {
            return Err(AppError::plugin_failed(&error, "indexer http resolve"));
        }
    };
    serde_json::from_value(payload).map(Some).map_err(|error| {
        AppError::Repository(format!(
            "indexer http resolve returned an invalid release: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint_response(origins: Vec<String>) -> Vec<u8> {
        serde_json::to_vec(&PluginResult::Ok(PluginActionResponse {
            payload: serde_json::json!({"origins": origins}),
        }))
        .unwrap()
    }

    #[test]
    fn http_resolve_reply_decodes_a_release_and_treats_unsupported_as_none() {
        let release = scryer_plugin_sdk::PluginHttpRelease {
            provider: "mediathek".to_string(),
            item_id: "item-1".to_string(),
            page_url: "https://provider.invalid/watch/item-1".to_string(),
            ..Default::default()
        };
        let decoded = decode_http_resolve_result(PluginResult::Ok(PluginActionResponse {
            payload: serde_json::to_value(&release).unwrap(),
        }))
        .unwrap();
        assert_eq!(decoded, Some(release));

        let unsupported = decode_http_resolve_result(PluginResult::Err(PluginError {
            code: PluginErrorCode::Unsupported,
            public_message: "no resolve".to_string(),
            debug_message: None,
            details: None,
            retry_after_seconds: None,
        }))
        .unwrap();
        assert_eq!(unsupported, None);

        let malformed = decode_http_resolve_result(PluginResult::Ok(PluginActionResponse {
            payload: serde_json::json!({"provider": "mediathek"}),
        }));
        assert!(malformed.is_err());
    }

    /// A challenge reaches the browser only as a bounded raster image, and
    /// every refusal names its reason.
    #[test]
    fn login_challenge_reply_is_shown_only_as_a_bounded_raster_image() {
        use base64::Engine as _;
        use scryer_application::login_challenge_refusal as refusal;

        let reply = |kind: &str, content_type: &str, data: &str| {
            serde_json::json!({
                "captchaRequest": { "type": kind, "contentType": content_type, "imageData": data }
            })
        };
        assert_eq!(
            decode_login_challenge("synthetic", reply("image", "Image/PNG; q=1", "iVBORw=="))
                .unwrap(),
            Some(("image/png".to_string(), "iVBORw==".to_string()))
        );
        assert_eq!(
            decode_login_challenge("synthetic", serde_json::json!({})).unwrap(),
            None
        );
        let reason = |payload| {
            decode_login_challenge("synthetic", payload)
                .unwrap_err()
                .validation_reason()
        };
        assert_eq!(
            reason(serde_json::json!({"captchaRequest": {"type": "image"}})),
            Some(refusal::MALFORMED)
        );
        assert_eq!(
            reason(reply("image", "image/png", "not base64!")),
            Some(refusal::MALFORMED)
        );
        assert_eq!(
            reason(reply("image", "image/png", "")),
            Some(refusal::MALFORMED)
        );
        assert_eq!(
            reason(reply("text", "image/png", "iVBORw==")),
            Some(refusal::KIND_UNSUPPORTED)
        );
        assert_eq!(
            reason(reply("image", "image/svg+xml", "PHN2Zz4=")),
            Some(refusal::IMAGE_TYPE_UNSUPPORTED)
        );
        assert_eq!(
            reason(reply("image", "text/html", "PGh0bWw+")),
            Some(refusal::IMAGE_TYPE_UNSUPPORTED)
        );
        let at_limit =
            base64::engine::general_purpose::STANDARD.encode(vec![0u8; refusal::MAX_IMAGE_BYTES]);
        assert!(
            decode_login_challenge("synthetic", reply("image", "image/png", &at_limit))
                .unwrap()
                .is_some()
        );
        let over_limit =
            base64::engine::general_purpose::STANDARD
                .encode(vec![0u8; refusal::MAX_IMAGE_BYTES + 1]);
        assert_eq!(
            reason(reply("image", "image/png", &over_limit)),
            Some(refusal::IMAGE_TOO_LARGE)
        );
    }

    #[test]
    fn config_fields_reply_decodes_fields_and_treats_unsupported_as_none() {
        let response = scryer_plugin_sdk::command::ConfigFieldsResponse {
            fields: vec![scryer_plugin_sdk::ConfigFieldDef {
                key: "cookie".to_string(),
                label: "Cookie".to_string(),
                field_type: scryer_plugin_sdk::ConfigFieldType::Password,
                required: true,
                ..Default::default()
            }],
            hidden_field_keys: vec!["username".to_string()],
        };
        let bytes = serde_json::to_vec(&PluginResult::Ok(PluginActionResponse {
            payload: serde_json::to_value(&response).unwrap(),
        }))
        .unwrap();
        let decoded = decode_config_fields_response("cardigann", &bytes)
            .unwrap()
            .unwrap();
        assert_eq!(decoded.fields.len(), 1);
        assert_eq!(decoded.fields[0].key, "cookie");
        assert_eq!(
            decoded.fields[0].field_type,
            scryer_domain::ConfigFieldType::Password
        );
        assert_eq!(decoded.hidden_field_keys, vec!["username".to_string()]);

        let unsupported =
            serde_json::to_vec(&PluginResult::<PluginActionResponse>::Err(PluginError {
                code: PluginErrorCode::Unsupported,
                public_message: "no config fields".to_string(),
                debug_message: None,
                details: None,
                retry_after_seconds: None,
            }))
            .unwrap();
        assert_eq!(
            decode_config_fields_response("cardigann", &unsupported).unwrap(),
            None
        );

        let failed = serde_json::to_vec(&PluginResult::<PluginActionResponse>::Err(PluginError {
            code: PluginErrorCode::InvalidConfig,
            public_message: "unknown definition".to_string(),
            debug_message: None,
            details: None,
            retry_after_seconds: None,
        }))
        .unwrap();
        assert!(decode_config_fields_response("cardigann", &failed).is_err());

        let malformed = serde_json::to_vec(&PluginResult::Ok(PluginActionResponse {
            payload: serde_json::json!({"fields": "nope"}),
        }))
        .unwrap();
        assert!(decode_config_fields_response("cardigann", &malformed).is_err());
        assert!(
            decode_config_fields_response(
                "cardigann",
                &vec![b' '; MAX_CONFIG_FIELDS_RESPONSE_BYTES + 1]
            )
            .is_err()
        );
    }

    #[test]
    fn network_endpoints_validate_every_declared_origin() {
        assert_eq!(
            validate_network_endpoints(&endpoint_response(vec![
                "https://API.example:8443".into(),
                "https://api.example/".into()
            ]))
            .unwrap(),
            vec!["api.example"]
        );
        for invalid in [
            "https://user:secret@api.example",
            "https://@api.example",
            "https://*.example",
            "ftp://api.example",
            "https://api.example/path",
            "https://api.example?secret=value",
            "https://api.example#secret",
            "https:api.example",
            "https:///api.example",
            "https://api.example\\other",
            "https://api.example\n",
        ] {
            assert!(
                validate_network_endpoints(&endpoint_response(vec![invalid.into()])).is_err(),
                "{invalid}"
            );
        }
        // No cap on how many hosts a definition names.
        assert_eq!(
            validate_network_endpoints(&endpoint_response(
                (0..500)
                    .map(|n| format!("https://api{n}.example"))
                    .collect()
            ))
            .unwrap()
            .len(),
            500
        );
        assert!(
            validate_network_endpoints(&vec![b' '; MAX_NETWORK_ENDPOINTS_RESPONSE_BYTES + 1])
                .is_err()
        );
        assert!(validate_network_endpoints(b"{}").is_err());
    }

    fn page(titles: &[&str], next_cursor: Option<&str>) -> PluginSearchCallResponse {
        PluginSearchCallResponse {
            response: PluginSearchResponse {
                results: titles
                    .iter()
                    .map(|title| scryer_plugin_sdk::PluginSearchResult {
                        title: (*title).to_string(),
                        ..Default::default()
                    })
                    .collect(),
                next_cursor: next_cursor.map(str::to_string),
                ..Default::default()
            },
            completion: IndexerSearchCompletion::Complete,
        }
    }

    fn paged_read(limit: usize, cursor: Option<&str>, page_budget: Option<u32>) -> PagedRead {
        PagedRead::new(
            &PluginSearchRequest {
                query: "Synthetic Paged Title".into(),
                limit,
                page_cursor: cursor.map(str::to_string),
                ..PluginSearchRequest::default()
            },
            page_budget,
        )
    }

    fn done(step: PagedStep) -> PluginSearchCallResponse {
        match step {
            PagedStep::Done(response) => response,
            PagedStep::Next(cursor) => panic!("expected the read to stop, got cursor {cursor}"),
        }
    }

    fn next(step: PagedStep) -> String {
        match step {
            PagedStep::Next(cursor) => cursor,
            PagedStep::Done(_) => panic!("expected the read to continue"),
        }
    }

    fn titles(response: &PluginSearchCallResponse) -> Vec<&str> {
        response
            .response
            .results
            .iter()
            .map(|result| result.title.as_str())
            .collect()
    }

    #[test]
    fn a_paged_read_follows_the_cursor_until_the_provider_runs_out() {
        let mut read = paged_read(1000, None, None);
        assert_eq!(next(read.absorb(page(&["A", "B"], Some("p2")))), "p2");
        assert_eq!(next(read.absorb(page(&["C"], Some("p3")))), "p3");
        let response = done(read.absorb(page(&["D"], None)));
        assert_eq!(titles(&response), vec!["A", "B", "C", "D"]);
        assert_eq!(response.completion, IndexerSearchCompletion::Complete);
        assert_eq!(response.response.next_cursor, None);
    }

    #[test]
    fn a_paged_read_keeps_the_whole_page_when_crossing_the_limit() {
        let mut read = paged_read(3, None, None);
        assert_eq!(next(read.absorb(page(&["A", "B"], Some("p2")))), "p2");
        // The limit is crossed mid-page; the cursor resumes after the whole
        // page, so the page is kept whole rather than losing "D".
        let response = done(read.absorb(page(&["C", "D"], Some("p3"))));
        assert_eq!(titles(&response), vec!["A", "B", "C", "D"]);
        assert_eq!(response.response.next_cursor.as_deref(), Some("p3"));
        assert_eq!(
            response.completion,
            IndexerSearchCompletion::Partial {
                reason: Some(HostIncompleteReason::PageCeilingReached),
                retry_after: None,
            }
        );
    }

    #[test]
    fn a_paged_read_preserves_the_tail_of_a_terminal_page() {
        let mut read = paged_read(3, None, None);
        assert_eq!(next(read.absorb(page(&["A", "B"], Some("p2")))), "p2");
        let response = done(read.absorb(page(&["C", "D"], None)));
        assert_eq!(titles(&response), vec!["A", "B", "C", "D"]);
        assert_eq!(response.completion, IndexerSearchCompletion::Complete);
        assert_eq!(response.response.next_cursor, None);
    }

    #[test]
    fn a_paged_read_stops_at_its_page_budget_with_the_resume_cursor() {
        let mut read = paged_read(1000, Some("p4"), Some(2));
        assert_eq!(next(read.absorb(page(&["A"], Some("p5")))), "p5");
        let response = done(read.absorb(page(&["B"], Some("p6"))));
        assert_eq!(titles(&response), vec!["A", "B"]);
        assert_eq!(response.response.next_cursor.as_deref(), Some("p6"));
        assert_eq!(
            response.completion,
            IndexerSearchCompletion::Partial {
                reason: None,
                retry_after: None,
            }
        );
    }

    #[test]
    fn a_cursor_that_does_not_advance_or_is_malformed_ends_the_read_at_the_ceiling() {
        let ceiling = IndexerSearchCompletion::Partial {
            reason: Some(HostIncompleteReason::PageCeilingReached),
            retry_after: None,
        };
        for returned in [
            "p2".to_string(),
            String::new(),
            "x".repeat(MAX_PAGE_CURSOR_BYTES + 1),
        ] {
            let mut read = paged_read(1000, Some("p2"), None);
            let response = done(read.absorb(page(&["A"], Some(&returned))));
            assert_eq!(titles(&response), vec!["A"]);
            assert_eq!(response.completion, ceiling);
            assert_eq!(response.response.next_cursor, None);
        }
    }

    #[test]
    fn a_cursor_that_returns_to_an_earlier_page_ends_the_read_at_the_ceiling() {
        let mut read = paged_read(1000, Some("p1"), None);
        assert_eq!(next(read.absorb(page(&["A"], Some("p2")))), "p2");
        assert_eq!(next(read.absorb(page(&["B"], Some("p3")))), "p3");
        let response = done(read.absorb(page(&["C"], Some("p1"))));
        assert_eq!(titles(&response), vec!["A", "B", "C"]);
        assert_eq!(response.response.next_cursor, None);
        assert_eq!(
            response.completion,
            IndexerSearchCompletion::Partial {
                reason: Some(HostIncompleteReason::PageCeilingReached),
                retry_after: None,
            }
        );
    }

    #[test]
    fn an_unbudgeted_read_stops_at_the_page_ceiling_with_the_resume_cursor() {
        let mut read = paged_read(0, None, None);
        for page_number in 1..MAX_UNBUDGETED_PAGES {
            let cursor = format!("p{}", page_number + 1);
            assert_eq!(next(read.absorb(page(&["A"], Some(&cursor)))), cursor);
        }
        let response = done(read.absorb(page(&["A"], Some("beyond"))));
        assert_eq!(
            response.response.results.len(),
            MAX_UNBUDGETED_PAGES as usize
        );
        assert_eq!(response.response.next_cursor.as_deref(), Some("beyond"));
        assert_eq!(
            response.completion,
            IndexerSearchCompletion::Partial {
                reason: Some(HostIncompleteReason::PageCeilingReached),
                retry_after: None,
            }
        );
    }

    #[test]
    fn a_failed_page_keeps_earlier_pages_and_resumes_from_its_own_cursor() {
        let mut read = paged_read(1000, None, None);
        assert_eq!(next(read.absorb(page(&["A"], Some("p2")))), "p2");
        let response = read
            .fail(AppError::Repository("upstream reset".into()))
            .expect("earlier pages stand");
        assert_eq!(titles(&response), vec!["A"]);
        assert_eq!(response.response.next_cursor.as_deref(), Some("p2"));
        assert!(matches!(
            response.completion,
            IndexerSearchCompletion::Partial {
                reason: Some(HostIncompleteReason::UpstreamFailure),
                ..
            }
        ));

        // A first page that fails, or a cancelled read, is the error itself.
        assert!(
            paged_read(1000, None, None)
                .fail(AppError::Repository("upstream reset".into()))
                .is_err()
        );
        let mut read = paged_read(1000, None, None);
        next(read.absorb(page(&["A"], Some("p2"))));
        assert!(read.fail(AppError::canceled("canceled")).is_err());
    }

    #[test]
    fn a_provider_ceiling_page_ends_the_read_without_a_cursor_and_a_partial_page_is_reread() {
        let mut read = paged_read(1000, None, None);
        next(read.absorb(page(&["A"], Some("p2"))));
        let mut capped = page(&["B"], Some("p3"));
        capped.completion = IndexerSearchCompletion::Partial {
            reason: Some(HostIncompleteReason::PageCeilingReached),
            retry_after: None,
        };
        let response = done(read.absorb(capped));
        assert_eq!(titles(&response), vec!["A", "B"]);
        assert_eq!(response.response.next_cursor, None);

        let mut read = paged_read(1000, None, None);
        next(read.absorb(page(&["A"], Some("p2"))));
        let mut broken = page(&["B"], None);
        broken.completion = IndexerSearchCompletion::Partial {
            reason: Some(HostIncompleteReason::UpstreamFailure),
            retry_after: None,
        };
        let response = done(read.absorb(broken));
        assert_eq!(response.response.next_cursor.as_deref(), Some("p2"));
    }

    fn rss_strategy(
        rss_catch_up: Option<scryer_application::IndexerRssCatchUp>,
    ) -> IndexerSearchStrategyRequest {
        IndexerSearchStrategyRequest {
            page_cursor: None,
            page_budget: None,
            strategy_id: "rss-strategy".to_string(),
            labels: vec!["rss".to_string()],
            query: String::new(),
            ids: std::collections::HashMap::new(),
            category: None,
            facet: None,
            id_search_facet: None,
            newznab_categories: Some(vec!["2000".to_string()]),
            season: None,
            episode: None,
            absolute_episode: None,
            year: None,
            tagged_aliases: Vec::new(),
            rss_catch_up,
        }
    }

    #[test]
    fn strategy_request_carries_the_rss_marker_to_the_plugin() {
        let marker_at = chrono::DateTime::parse_from_rfc3339("2026-09-20T10:00:00Z")
            .expect("fixture time should parse")
            .with_timezone(&chrono::Utc);
        let request = plugin_search_request_from_strategy(
            rss_strategy(Some(scryer_application::IndexerRssCatchUp {
                last_seen_published_at: marker_at,
                last_seen_identity: Some("marker-guid".to_string()),
            })),
            SearchMode::Auto,
        );

        assert_eq!(
            request.rss_catch_up,
            Some(scryer_plugin_sdk::PluginRssCatchUp {
                last_seen_published_at: "2026-09-20T10:00:00Z".to_string(),
                last_seen_identity: Some("marker-guid".to_string()),
            })
        );
        assert_eq!(request.categories, vec!["2000".to_string()]);
        let wire = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(
            wire["rss_catch_up"]["last_seen_published_at"],
            "2026-09-20T10:00:00Z"
        );
    }

    #[test]
    fn strategy_request_without_a_marker_omits_rss_catch_up() {
        let request = plugin_search_request_from_strategy(rss_strategy(None), SearchMode::Auto);

        assert_eq!(request.rss_catch_up, None);
        let wire = serde_json::to_value(&request).expect("request should serialize");
        assert!(wire.get("rss_catch_up").is_none());
    }

    #[test]
    fn grab_nzb_payload_preserves_headers_and_opaque_bytes() {
        let bytes = b"<nzb><head><meta type=\"password\">untouched</meta></head></nzb>";
        let artifact = decode_grab_result(PluginResult::Ok(PluginActionResponse {
            payload: serde_json::json!({
                "url": "https://indexer.example/download/1",
                "body": bytes.to_vec(),
                "source_kind": "nzb",
                "response_headers": {"X-DNZB-Password": ["1", " synthetic ", "1"]},
                "password_candidates": ["true", " synthetic "]
            }),
        }))
        .unwrap()
        .unwrap();
        let ResolvedDownloadArtifact::Nzb {
            bytes: actual,
            password_candidates,
            ..
        } = artifact
        else {
            panic!("expected NZB artifact");
        };
        assert_eq!(actual, bytes);
        assert_eq!(
            password_candidates
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["1", " synthetic ", "true"]
        );
        assert!(!format!("{password_candidates:?}").contains("synthetic"));
    }

    #[test]
    fn grab_magnet_payload_resolves_without_torrent_bytes() {
        let uri = "magnet:?xt=urn:btih:ABCDEF0123456789ABCDEF0123456789ABCDEF01";
        let artifact = decode_grab_result(PluginResult::Ok(PluginActionResponse {
            payload: serde_json::json!({
                "url": uri,
                "body": [],
            }),
        }))
        .expect("magnet payload should decode")
        .expect("grab should resolve a magnet artifact");

        assert!(matches!(
            artifact,
            ResolvedDownloadArtifact::Magnet { uri: resolved_uri, info_hash_hint: Some(info_hash) }
                if resolved_uri == uri
                    && info_hash == "abcdef0123456789abcdef0123456789abcdef01"
        ));
    }

    #[test]
    fn grab_torrent_payload_carries_bytes_and_hints() {
        let artifact = decode_grab_result(PluginResult::Ok(PluginActionResponse {
            payload: serde_json::json!({
                "url": "https://tracker.example/download/1",
                "method": "GET",
                "headers": {"cookie": "session=one"},
                "body": [1, 2, 3],
                "file_name": "release.torrent",
            }),
        }))
        .expect("torrent payload should decode")
        .expect("grab should resolve a torrent artifact");

        assert!(matches!(
            artifact,
            ResolvedDownloadArtifact::TorrentFile { bytes, file_name: Some(name), .. }
                if bytes == vec![1, 2, 3] && name == "release.torrent"
        ));
    }

    #[test]
    fn grab_empty_legacy_reply_keeps_host_fetch_available() {
        // Newznab, Torznab, and AnimeTosho return {} for unknown actions.
        let resolved = decode_grab_result(PluginResult::Ok(PluginActionResponse {
            payload: serde_json::json!({}),
        }))
        .expect("a legacy unsupported grab must retain the host fetch path");

        assert!(resolved.is_none());
    }

    #[test]
    fn grab_nonempty_malformed_replies_do_not_fall_back() {
        for payload in [
            serde_json::Value::Null,
            serde_json::json!([]),
            serde_json::json!({"body": [100, 101]}),
            serde_json::json!({"url": null}),
            serde_json::json!({"error": "authentication failed"}),
        ] {
            assert!(
                decode_grab_result(PluginResult::Ok(PluginActionResponse { payload })).is_err(),
                "a malformed grab response must not trigger a second download"
            );
        }
    }

    #[test]
    fn grab_unsupported_keeps_router_fallback_available() {
        let resolved = decode_grab_result(PluginResult::Err(PluginError {
            code: PluginErrorCode::Unsupported,
            public_message: "grab is not implemented".to_string(),
            debug_message: None,
            retry_after_seconds: None,
            details: None,
        }))
        .expect("unsupported grab is an optional capability");

        assert!(resolved.is_none());
    }

    #[test]
    fn grab_transport_or_auth_failure_propagates() {
        let error = decode_grab_result(PluginResult::Err(PluginError {
            code: PluginErrorCode::AuthFailed,
            public_message: "login expired".to_string(),
            debug_message: None,
            retry_after_seconds: None,
            details: None,
        }))
        .expect_err("real grab failures must not silently fall back");

        assert!(error.to_string().contains("AuthFailed"));
        assert!(error.to_string().contains("login expired"));
    }

    #[test]
    fn grab_without_bytes_or_magnet_is_rejected() {
        let error = decode_grab_result(PluginResult::Ok(PluginActionResponse {
            payload: serde_json::json!({
                "url": "https://tracker.example/login",
                "body": [],
            }),
        }))
        .expect_err("an empty non-magnet grab must never reach a download client");

        assert!(error.to_string().contains("empty grab payload"));
    }

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

    /// The search client sizes the indexer's operational backoff from this
    /// error's own delay, so the plugin's `Retry-After` must survive the
    /// component boundary on it.
    #[test]
    fn deferred_rate_limit_keeps_the_plugin_retry_after_on_the_error() {
        let error = decode_search_result(
            PluginResult::Err(PluginError {
                code: PluginErrorCode::RateLimited,
                public_message: "Newznab upstream rate limit reached".to_string(),
                debug_message: Some("HTTP 429 after 1 attempt(s) and 0s total wait".to_string()),
                retry_after_seconds: Some(360),
                details: Some(PluginErrorDetails::IndexerSearch(
                    IndexerSearchPluginError::Deferred {
                        reason: PluginIncompleteReason::RateLimited,
                        retry_after_seconds: Some(360),
                    },
                )),
            }),
            true,
            "indexer search",
        )
        .err()
        .expect("a deferred search is a failed strategy");

        match error {
            AppError::TemporaryUnavailable { retry_after, .. } => assert_eq!(
                retry_after,
                Some(std::time::Duration::from_secs(360)),
                "the provider's Retry-After was lost at the component boundary"
            ),
            other => panic!("unexpected deferred-search error: {other:?}"),
        }
    }

    #[test]
    fn component_api_key_rejection_is_actionable_without_exposing_debug_detail() {
        let error = decode_search_result(
            PluginResult::Err(PluginError {
                code: PluginErrorCode::Temporary,
                public_message: "indexer component failed".to_string(),
                debug_message: Some(
                    "Newznab API key error 100: Invalid API Key (key=secret)".to_string(),
                ),
                retry_after_seconds: None,
                details: None,
            }),
            true,
            "indexer search",
        )
        .err()
        .expect("an API-key rejection must fail the search");

        let failure = error
            .plugin_failure()
            .expect("a legacy API-key rejection is promoted to the shared reason");
        assert_eq!(failure.reason.as_str(), "PLUGIN_INVALID_API_KEY");
        assert_eq!(failure.detail, None);
        assert!(!format!("{error:?}").contains("secret"));
    }

    #[test]
    fn a_plugin_code_the_sdk_never_named_reaches_the_client_as_its_own_reason() {
        // What a guest writes for a code newer than any host or SDK release.
        let bytes = br#"{"err":{"code":"geo_blocked","public_message":"The site refuses requests from this region","debug_message":"form answered 451 with key=example-secret"}}"#;
        let result = serde_json::from_slice::<PluginResult<PluginSearchResponse>>(bytes)
            .expect("an unknown code decodes unchanged");

        let error = decode_search_result(result, true, "indexer search")
            .err()
            .expect("a plugin error must fail the search");

        let failure = error.plugin_failure().expect("a typed plugin failure");
        assert_eq!(failure.reason.as_str(), "PLUGIN_GEO_BLOCKED");
        assert_eq!(
            failure.detail.as_deref(),
            Some("The site refuses requests from this region")
        );
        assert!(!format!("{error:?}").contains("example-secret"));
    }

    fn coded_search_error(code: &str, public_message: &str, debug_message: &str) -> AppError {
        decode_search_result(
            PluginResult::Err(PluginError {
                code: PluginErrorCode::from(code),
                public_message: public_message.to_string(),
                debug_message: Some(debug_message.to_string()),
                retry_after_seconds: None,
                details: None,
            }),
            true,
            "indexer search",
        )
        .err()
        .expect("a plugin error must fail the search")
    }

    #[test]
    fn a_rejected_credential_code_reaches_the_client_as_its_translated_reason() {
        for code in [
            "invalid_api_key",
            "invalid_passkey",
            "invalid_cookie",
            "invalid_username_password",
        ] {
            // The plugin's sentence does not matter: the code alone decides.
            let error = coded_search_error(
                code,
                "the plugin could not complete the request",
                "upstream answered 401 for key=example-secret",
            );
            let failure = error
                .plugin_failure()
                .unwrap_or_else(|| panic!("{code} stays a typed plugin failure"));
            assert_eq!(
                failure.reason.as_str(),
                format!("PLUGIN_{}", code.to_ascii_uppercase()),
                "{code}"
            );
            assert_eq!(failure.detail, None, "{code}");
            assert!(!format!("{error:?}").contains("example-secret"), "{code}");
        }
    }

    #[test]
    fn a_code_that_names_no_credential_stays_a_plugin_failure() {
        for code in ["captcha_required", "quota_exceeded", "tls_error"] {
            let error = coded_search_error(
                code,
                "the plugin could not complete the request",
                "api key error: rejected",
            );
            let failure = error
                .plugin_failure()
                .unwrap_or_else(|| panic!("{code} stays a typed plugin failure"));
            assert_eq!(
                failure.reason.as_str(),
                format!("PLUGIN_{}", code.to_ascii_uppercase()),
                "{code}"
            );
            // The PDK's generic sentence adds nothing beside the reason.
            assert_eq!(failure.detail, None, "{code}");
        }
    }

    #[test]
    fn a_plugin_search_error_names_its_reason_and_keeps_its_public_sentence() {
        let error = decode_search_result(
            PluginResult::Err(PluginError {
                code: PluginErrorCode::AuthFailed,
                public_message: "Tracker login failed: account disabled".to_string(),
                debug_message: Some("login form answered 200 with key=example-secret".to_string()),
                retry_after_seconds: None,
                details: None,
            }),
            true,
            "indexer search",
        )
        .err()
        .expect("a plugin error must fail the search");

        let failure = error.plugin_failure().expect("a typed plugin failure");
        assert_eq!(failure.reason, PluginFailureReason::AuthFailed);
        assert_eq!(
            failure.detail.as_deref(),
            Some("Tracker login failed: account disabled")
        );
        assert_eq!(
            error.to_string(),
            "indexer search: plugin error AuthFailed: Tracker login failed: account disabled"
        );
        assert!(!format!("{error:?}").contains("example-secret"));
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
            None,
        );

        assert_eq!(context.request_kind, PluginSearchRequestKind::Recent);
        assert_eq!(context.search_origin, PluginSearchOrigin::Rss);
        assert_eq!(context.subject_kind, PluginSearchSubjectKind::Title);
        assert_eq!(context.query_kind, PluginSearchQueryKind::Fallback);
    }

    #[test]
    fn carries_known_year_on_movie_search_context() {
        let context = build_search_context(
            "Amber Circuit 2026",
            &std::collections::HashMap::new(),
            Some("movie"),
            SearchMode::Interactive,
            None,
            None,
            None,
            Some(2026),
        );

        assert_eq!(context.subject_kind, PluginSearchSubjectKind::Movie);
        assert_eq!(context.year, Some(2026));
        // The year is a qualifier, not evidence of an id search.
        assert_eq!(context.query_kind, PluginSearchQueryKind::Title);
    }

    #[test]
    fn omits_year_on_movie_search_without_a_known_year() {
        let context = build_search_context(
            "Amber Circuit",
            &std::collections::HashMap::new(),
            Some("movie"),
            SearchMode::Interactive,
            None,
            None,
            None,
            None,
        );

        assert_eq!(context.subject_kind, PluginSearchSubjectKind::Movie);
        assert_eq!(context.year, None);
    }

    #[test]
    fn drops_series_year_on_episode_search_context() {
        let context = build_search_context(
            "Example Show S01E02",
            &std::collections::HashMap::new(),
            Some("series"),
            SearchMode::Auto,
            Some(1),
            Some(2),
            None,
            // A series year is not the release year of the searched episode.
            Some(2013),
        );

        assert_eq!(context.subject_kind, PluginSearchSubjectKind::Episode);
        assert_eq!(context.year, None);
    }

    #[test]
    fn drops_series_year_on_title_scoped_series_search_context() {
        let context = build_search_context(
            "Example Show",
            &std::collections::HashMap::new(),
            Some("series"),
            SearchMode::Auto,
            None,
            None,
            None,
            Some(2013),
        );

        assert_eq!(context.subject_kind, PluginSearchSubjectKind::Title);
        assert_eq!(context.year, None);
    }

    #[test]
    fn generic_fallback_keeps_the_movie_year() {
        let request = PluginSearchRequest {
            query: "Amber Circuit 2026".to_string(),
            ids: std::collections::HashMap::from([(
                "imdb_id".to_string(),
                "tt12004567".to_string(),
            )]),
            facet: Some("movie".to_string()),
            category: Some("movie".to_string()),
            limit: 1000,
            context: Some(build_search_context(
                "Amber Circuit 2026",
                &std::collections::HashMap::from([(
                    "imdb_id".to_string(),
                    "tt12004567".to_string(),
                )]),
                Some("movie"),
                SearchMode::Interactive,
                None,
                None,
                None,
                Some(2026),
            )),
            ..PluginSearchRequest::default()
        };

        let fallback = generic_search_fallback_request(&request, SearchMode::Interactive);

        assert!(fallback.ids.is_empty());
        assert_eq!(fallback.facet, None);
        assert_eq!(
            fallback.context.as_ref().expect("fallback context").year,
            Some(2026),
            "the facet-less fallback is the same movie subject"
        );
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
            Some(" archive-password ")
        );

        let result = scryer_plugin_sdk::PluginSearchResult {
            password_hint: Some(" direct-password ".to_string()),
            ..scryer_plugin_sdk::PluginSearchResult::default()
        };
        let extra = merge_result_extra(&result);
        assert_eq!(
            plugin_password_hint(&result, &extra).as_deref(),
            Some(" direct-password ")
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
            settings: Vec::new(),
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
                    ..Default::default()
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
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

    fn sample_solver_config() -> scryer_domain::ProxyConfig {
        scryer_domain::ProxyConfig {
            id: "solver".to_string(),
            name: "Solver".to_string(),
            provider_type: scryer_domain::ProxyProviderType::Byparr,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            base_url: "http://solver.local".to_string(),
            request_timeout_seconds: 90,
            is_enabled: true,
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn direct_indexer_runtime_keeps_solver_policy() {
        let descriptor = descriptor_with_base_url_role("newznab");
        let config = sample_indexer_config("newznab", None);

        let inputs = build_runtime_inputs(
            &descriptor,
            "Direct Indexer",
            &config,
            Some(sample_solver_config()),
        );

        assert!(inputs.proxy_policy.is_some());
        assert_eq!(inputs.timeout, std::time::Duration::from_secs(120));
    }

    #[test]
    fn prowlarr_runtime_strips_stale_solver_policy() {
        let descriptor = descriptor_with_base_url_role("newznab");
        let mut config = sample_indexer_config("newznab", Some("prowlarr-parent"));
        config.managed_child_key = Some("child-42".to_string());

        let inputs = build_runtime_inputs(
            &descriptor,
            "Managed Prowlarr Child",
            &config,
            Some(sample_solver_config()),
        );

        assert!(
            config.is_prowlarr_nab_proxy(),
            "fixture must model a Prowlarr child"
        );
        assert!(inputs.proxy_policy.is_none());
        assert_eq!(inputs.timeout, scryer_outbound_http::INDEXER_HTTP_TIMEOUT);
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

    /// `build_runtime_inputs` is the single source of what a configured
    /// indexer observes; this pins the normalization it performs so a future
    /// caller cannot quietly grow its own.
    #[test]
    fn runtime_inputs_normalize_a_managed_child_once() {
        let descriptor = descriptor_with_base_url_role("newznab");
        let mut config = sample_indexer_config("newznab", Some("parent"));
        config.managed_child_key = Some("child-42".to_string());
        config.config_json =
            Some(r#"{"base_url":"http://localhost:9696/1","api_path":"/api"}"#.to_string());

        let inputs = build_runtime_inputs(&descriptor, "Managed Child", &config, None);

        assert_eq!(inputs.timeout, scryer_outbound_http::INDEXER_HTTP_TIMEOUT);
        assert_eq!(
            inputs.destination_cooldown_key.as_deref(),
            Some("parent:child-42")
        );
        assert_eq!(
            inputs.config_entries.get("base_url").map(String::as_str),
            Some("http://localhost:9696/1")
        );
        assert_eq!(
            inputs.config_entries.get("api_path").map(String::as_str),
            Some("/api")
        );
    }

    fn usage(resident_bytes: usize, denied: bool) -> GuestMemoryUsage {
        GuestMemoryUsage {
            resident_bytes,
            largest_memory_bytes: resident_bytes,
            cap_bytes: 128 * 1024 * 1024,
            denied,
        }
    }

    #[test]
    fn actor_release_policy_drops_on_watermark_rss_and_error() {
        let policy = IndexerMemoryPolicy::default();
        let small = Some(usage(4 * 1024 * 1024, false));
        let large = Some(usage(policy.retain_watermark_bytes + 1, false));
        let interactive = IndexerErrorOperation::InteractiveSearch;

        assert_eq!(
            ActorRelease::after_invocation(interactive, small, policy, false),
            ActorRelease::Retained
        );
        assert_eq!(
            ActorRelease::after_invocation(interactive, large, policy, false),
            ActorRelease::Watermark
        );
        // RSS drops even a small Store.
        assert_eq!(
            ActorRelease::after_invocation(IndexerErrorOperation::RssSync, small, policy, false),
            ActorRelease::Rss
        );
        // A failed call or a cap denial always drops, whatever the operation.
        assert_eq!(
            ActorRelease::after_invocation(interactive, small, policy, true),
            ActorRelease::Error
        );
        assert_eq!(
            ActorRelease::after_invocation(
                IndexerErrorOperation::RssSync,
                Some(usage(1024, true)),
                policy,
                false
            ),
            ActorRelease::Error
        );
        assert!(!ActorRelease::Retained.drops_actor());
        for release in [
            ActorRelease::Watermark,
            ActorRelease::Rss,
            ActorRelease::Error,
        ] {
            assert!(release.drops_actor(), "{release:?}");
        }
    }

    #[test]
    fn memory_cap_failures_name_the_indexer_and_the_cap() {
        let client = builtin_newznab_client(IndexerMemoryPolicy::default());
        let error = client.component_call_error(
            "indexer_search",
            Some(usage(64 * 1024 * 1024, true)),
            PluginFailureReason::HostFailure,
            "indexer component newznab indexer_search failed: wasm trap".to_string(),
        );
        assert_eq!(
            error.plugin_failure().map(|failure| failure.reason.clone()),
            Some(PluginFailureReason::ResourceLimit)
        );
        let message = error.to_string();
        assert!(message.contains("'Test' (cfg)"), "{message}");
        assert!(message.contains("guest memory cap of 128 MiB"), "{message}");
        assert!(message.contains(INDEXER_MEMORY_CAP_ENV), "{message}");
        assert!(message.contains("wasm trap"), "{message}");

        let plain = client.component_call_error(
            "indexer_search",
            Some(usage(1024, false)),
            PluginFailureReason::TimedOut,
            "boom".into(),
        );
        assert_eq!(
            plain.plugin_failure().map(|failure| failure.reason.clone()),
            Some(PluginFailureReason::TimedOut)
        );
        let plain = plain.to_string();
        assert!(!plain.contains("memory cap"), "{plain}");
    }

    /// The bundled Newznab component, configured against a closed local port.
    /// The probes below are actions the component answers without HTTP.
    fn builtin_newznab_client(policy: IndexerMemoryPolicy) -> WasmIndexerClient {
        let asset = crate::builtins::NEWZNAB;
        let wasm = crate::builtins::decode_builtin_wasm(asset).expect("bundled Newznab wasm");
        let descriptor: PluginDescriptor =
            serde_json::from_str(asset.descriptor_json).expect("bundled Newznab descriptor");
        let mut config = sample_indexer_config("newznab", None);
        config.base_url = "http://127.0.0.1:9".to_string();
        let mut client = WasmIndexerClient::new_component_with_plugin_settings(
            wasm,
            descriptor,
            "Test".to_string(),
            config,
            None,
            Arc::new(scryer_application::NullIndexerErrorRecorder),
            &BTreeMap::new(),
        )
        .expect("bundled Newznab client");
        Arc::get_mut(&mut client.component)
            .expect("fresh component")
            .memory_policy = policy;
        client
    }

    async fn run_probe_action(client: &WasmIndexerClient, operation: IndexerErrorOperation) {
        // Unknown actions are answered in-guest; the reply itself is irrelevant
        // here, only that the invocation returned to the host.
        let _ = client
            .indexer_action_with_operation(
                "scryer_memory_release_probe",
                BTreeMap::new(),
                operation,
                true,
            )
            .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn small_interactive_store_is_retained_between_calls() {
        let client = builtin_newznab_client(IndexerMemoryPolicy::default());
        run_probe_action(&client, IndexerErrorOperation::InteractiveSearch).await;
        let actor = client.component.actor.lock().await;
        let usage = actor
            .as_ref()
            .map(ComponentActor::memory_usage)
            .expect("a Store below the watermark is retained");
        assert!(usage.resident_bytes > 0, "{usage:?}");
        assert!(
            usage.resident_bytes <= IndexerMemoryPolicy::default().retain_watermark_bytes,
            "{usage:?}"
        );
        assert_eq!(usage.cap_bytes, IndexerMemoryPolicy::default().cap_bytes);
        assert!(!usage.denied);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn store_above_the_watermark_is_dropped_after_the_call() {
        let client = builtin_newznab_client(IndexerMemoryPolicy {
            retain_watermark_bytes: 1,
            ..IndexerMemoryPolicy::default()
        });
        run_probe_action(&client, IndexerErrorOperation::InteractiveSearch).await;
        assert!(
            client.component.actor.lock().await.is_none(),
            "a Store past the watermark must be released"
        );
        // The next call re-instantiates a fresh Store.
        run_probe_action(&client, IndexerErrorOperation::InteractiveSearch).await;
        assert!(client.component.actor.lock().await.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rss_invocation_always_drops_the_store() {
        let client = builtin_newznab_client(IndexerMemoryPolicy::default());
        run_probe_action(&client, IndexerErrorOperation::InteractiveSearch).await;
        assert!(client.component.actor.lock().await.is_some());
        run_probe_action(&client, IndexerErrorOperation::RssSync).await;
        assert!(
            client.component.actor.lock().await.is_none(),
            "an RSS call must not leave a retained Store behind"
        );
    }
}
