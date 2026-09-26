//! List-provider plugins: the client over one loaded component and the
//! provider that holds every installed one.
//!
//! A list provider is stateless from the host's point of view. It sees the
//! server-wide config values it declared and, for a member's list, that
//! member's credential inside the one request that needs it. The credential is
//! never written into the plugin config map, so it cannot reach `config-get`
//! or plugin state.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, ExternalPluginWasm, ListPluginProvider, ListProviderClient,
    RuntimePluginLoad,
};
use scryer_plugin_sdk::command::{
    PluginCommand, PluginCommandRequest, PluginCommandResult, PluginListCommand,
    PluginListCommandResult,
};
use scryer_plugin_sdk::{
    ListCredential, ListPluginAccountRequest, ListPluginAccountResponse, ListPluginFetchRequest,
    ListPluginFetchResponse, ListPluginHealthRequest, ListPluginHealthResponse,
    ListSourceParamType, PluginDescriptor, PluginResult,
};
use tracing::{info, warn};

use crate::loader::{
    LoadedPlugin, LoadedPluginRecord, PluginLoadSource, allowed_hosts_for_descriptor,
    insert_loaded_plugin, load_from_bytes, operator_egress_policy_for_descriptor,
    parse_builtin_descriptor, remove_loaded_plugin, resolve_loaded_plugin,
    validate_descriptor_for_type,
};
use crate::runtime_backing::{PluginInstanceSpec, PluginRuntimeBacking};
use crate::wasmtime_host::command_host::CommandHost;
use crate::wasmtime_host::{ListComponentInvocation, process_list_component};

const LIST_PLUGIN_TYPE: &str = "list_provider";

/// Wall-clock budget for one list invocation. A fetch pages through a remote
/// API under that API's rate limit, so this is longer than a subtitle search.
pub(crate) const LIST_PLUGIN_TIMEOUT: Duration = Duration::from_secs(120);

/// Minimum spacing between fetch calls to one provider, shared by every
/// client of that provider so the descriptor's `rate_limit_seconds` holds
/// across lists, pages and plugin reloads.
#[derive(Clone, Default)]
pub(crate) struct ListFetchPacer {
    next_call: Arc<Mutex<HashMap<String, tokio::time::Instant>>>,
}

impl ListFetchPacer {
    /// Wait for this provider's next slot. A zero interval never waits.
    pub(crate) async fn acquire(&self, provider_type: &str, interval: Duration) {
        if let Some(slot) = self.reserve(provider_type, interval, tokio::time::Instant::now()) {
            tokio::time::sleep_until(slot).await;
        }
    }

    /// Claim this provider's next slot as of `now` and return when it opens,
    /// or `None` for a zero interval.
    pub(crate) fn reserve(
        &self,
        provider_type: &str,
        interval: Duration,
        now: tokio::time::Instant,
    ) -> Option<tokio::time::Instant> {
        if interval.is_zero() {
            return None;
        }
        let mut next_call = self
            .next_call
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = provider_type.to_ascii_lowercase();
        let slot = next_call.get(&key).copied().unwrap_or(now).max(now);
        next_call.insert(key, slot + interval);
        Some(slot)
    }
}

/// The spacing a descriptor asks for between fetch calls.
fn fetch_interval(descriptor: &PluginDescriptor) -> Duration {
    let seconds = descriptor
        .list_provider()
        .and_then(|provider| provider.rate_limit_seconds)
        .unwrap_or_default()
        .max(0);
    Duration::from_secs(u64::try_from(seconds).unwrap_or_default())
}

/// Hosts of the request's parameters that the descriptor declares as links
/// for the requested source type. A custom feed may live on any host, so the
/// host the operator linked is allowed for the calls that carry that link and
/// for no other call.
pub(crate) fn url_param_hosts(
    descriptor: &PluginDescriptor,
    request: &ListPluginFetchRequest,
) -> Vec<String> {
    let Some(provider) = descriptor.list_provider() else {
        return Vec::new();
    };
    let mut hosts: Vec<String> = provider
        .groups
        .iter()
        .flat_map(|group| &group.items)
        .filter(|item| item.source_type == request.source_type)
        .flat_map(|item| &item.params)
        .filter(|param| param.param_type == ListSourceParamType::Url)
        .filter_map(|param| request.params.get(&param.key))
        .filter_map(|value| url::Url::parse(value.trim()).ok())
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .filter_map(|url| url.host_str().map(str::to_ascii_lowercase))
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

/// What a client needs to bind a host for one call: the declared config and
/// the hosts every call may reach.
struct ListHostBinding {
    plugin_config: BTreeMap<String, String>,
    allowed_hosts: Vec<String>,
    egress_policy: scryer_outbound_http::PluginEgressPolicy,
}

/// One `scryer:lists/list-provider@1.0.0` component with its server-wide
/// config bound.
pub struct WasmListClient {
    spec: PluginInstanceSpec,
    descriptor: PluginDescriptor,
    binding: Option<ListHostBinding>,
    pacer: ListFetchPacer,
    /// The host built for the last fetch that linked extra hosts, reused
    /// while later pages carry the same links.
    widened: Mutex<Option<(Vec<String>, CommandHost)>>,
}

impl WasmListClient {
    pub(crate) fn new(spec: PluginInstanceSpec, descriptor: PluginDescriptor) -> Self {
        Self {
            spec,
            descriptor,
            binding: None,
            pacer: ListFetchPacer::default(),
            widened: Mutex::new(None),
        }
    }

    /// The hosts one fetch may reach: every declared and configured host, and
    /// the hosts of the request's own links.
    pub(crate) fn allowed_hosts_for_fetch(&self, request: &ListPluginFetchRequest) -> Vec<String> {
        let mut hosts = self
            .binding
            .as_ref()
            .map(|binding| binding.allowed_hosts.clone())
            .unwrap_or_default();
        for host in url_param_hosts(&self.descriptor, request) {
            if !hosts.contains(&host) {
                hosts.push(host);
            }
        }
        hosts
    }

    /// The instance spec for one fetch: the client's own, or one whose host
    /// also reaches the request's linked hosts.
    fn spec_for_fetch(&self, request: &ListPluginFetchRequest) -> PluginInstanceSpec {
        let Some(binding) = &self.binding else {
            return self.spec.clone();
        };
        let extra: Vec<String> = url_param_hosts(&self.descriptor, request)
            .into_iter()
            .filter(|host| !binding.allowed_hosts.contains(host))
            .collect();
        if extra.is_empty() {
            return self.spec.clone();
        }
        let mut widened = self
            .widened
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let command_host = match widened.as_ref() {
            Some((hosts, host)) if *hosts == extra => host.clone(),
            _ => {
                let host = CommandHost::with_archive_provider(
                    self.descriptor.id.clone(),
                    binding.plugin_config.clone(),
                    self.allowed_hosts_for_fetch(request),
                    binding.egress_policy.clone(),
                    LIST_PLUGIN_TIMEOUT,
                    None,
                    None,
                );
                *widened = Some((extra, host.clone()));
                host
            }
        };
        PluginInstanceSpec {
            command_host,
            ..self.spec.clone()
        }
    }

    fn requires_member_credential(&self) -> bool {
        self.descriptor
            .list_provider()
            .is_some_and(|provider| provider.capabilities.requires_member_credential)
    }

    async fn invoke(
        &self,
        command: PluginListCommand,
        operation: &'static str,
    ) -> AppResult<PluginListCommandResult> {
        self.invoke_with(&self.spec, command, operation).await
    }

    async fn invoke_with(
        &self,
        spec: &PluginInstanceSpec,
        command: PluginListCommand,
        operation: &'static str,
    ) -> AppResult<PluginListCommandResult> {
        let response = process_list_component(
            spec,
            &PluginCommandRequest::new(PluginCommand::List(command)),
            ListComponentInvocation {
                plugin_id: &self.descriptor.id,
                plugin_version: &self.descriptor.version,
                operation,
            },
        )
        .await?;
        match response.response {
            PluginCommandResult::List(result) => Ok(result),
            _ => Err(AppError::Repository(format!(
                "list plugin {} returned a response for another plugin family",
                self.descriptor.id
            ))),
        }
    }

    fn wrong_operation(&self, operation: &str) -> AppError {
        AppError::Repository(format!(
            "list plugin {} answered a {operation} command with another operation",
            self.descriptor.id
        ))
    }
}

#[async_trait]
impl ListProviderClient for WasmListClient {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    async fn fetch(
        &self,
        request: ListPluginFetchRequest,
    ) -> AppResult<PluginResult<ListPluginFetchResponse>> {
        // Member-only providers are never invoked on the instance's behalf:
        // without the member's own token there is no call to make.
        if self.requires_member_credential() && request.credential.is_none() {
            return Err(AppError::Validation(format!(
                "list provider {} reads only a member's own lists and needs that member's linked account",
                self.descriptor.provider_type()
            )));
        }
        let spec = self.spec_for_fetch(&request);
        self.pacer
            .acquire(
                self.descriptor.provider_type(),
                fetch_interval(&self.descriptor),
            )
            .await;
        match self
            .invoke_with(&spec, PluginListCommand::Fetch(request), "list_fetch")
            .await?
        {
            PluginListCommandResult::Fetch(result) => Ok(result),
            _ => Err(self.wrong_operation("fetch")),
        }
    }

    async fn account(
        &self,
        credential: ListCredential,
    ) -> AppResult<PluginResult<ListPluginAccountResponse>> {
        match self
            .invoke(
                PluginListCommand::Account(ListPluginAccountRequest { credential }),
                "list_account",
            )
            .await?
        {
            PluginListCommandResult::Account(result) => Ok(result),
            _ => Err(self.wrong_operation("account")),
        }
    }

    async fn health(&self) -> AppResult<PluginResult<ListPluginHealthResponse>> {
        match self
            .invoke(
                PluginListCommand::Health(ListPluginHealthRequest {}),
                "list_health",
            )
            .await?
        {
            PluginListCommandResult::Health(result) => Ok(result),
            _ => Err(self.wrong_operation("health")),
        }
    }
}

/// The installed list providers, keyed by provider type with aliases.
pub struct WasmListPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
    pacer: ListFetchPacer,
}

impl WasmListPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
            pacer: ListFetchPacer::default(),
        }
    }

    fn validate(descriptor: &PluginDescriptor, load_source: PluginLoadSource) -> bool {
        validate_descriptor_for_type(descriptor, Some(LIST_PLUGIN_TYPE), load_source)
    }

    fn prepare_external_plugin_record(
        plugin: ExternalPluginWasm<'_>,
    ) -> Result<LoadedPluginRecord, String> {
        let (descriptor, wasm_bytes) = load_from_bytes(plugin.bytes)?;
        if !Self::validate(
            &descriptor,
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("list provider descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            descriptor, wasm_bytes,
        )))
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        if !Self::validate(
            &plugin.descriptor,
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("list provider descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            plugin.descriptor,
            plugin.wasm_bytes,
        )))
    }

    fn prepare_builtin_asset_record(
        asset: crate::builtins::BuiltinPluginAsset,
    ) -> Result<LoadedPluginRecord, String> {
        let descriptor = parse_builtin_descriptor(asset)?;
        if !Self::validate(&descriptor, PluginLoadSource::Builtin) {
            return Err("built-in list provider descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_builtin(
            descriptor, asset,
        )))
    }

    pub fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match Self::prepare_external_plugin_record(plugin) {
            Ok(record) => {
                info!(
                    plugin = record.loaded().descriptor.name.as_str(),
                    version = record.loaded().descriptor.version.as_str(),
                    "registered external list provider plugin"
                );
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => warn!(error = %error, "failed to load external list provider plugin"),
        }
        self
    }

    pub fn with_runtime_plugin(mut self, plugin: RuntimePluginLoad) -> Self {
        match Self::prepare_runtime_plugin_record(plugin) {
            Ok(record) => {
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => warn!(error = %error, "failed to load runtime list provider plugin"),
        }
        self
    }

    /// Built-ins never displace an installed plugin of the same provider type.
    pub fn with_builtin_asset(mut self, asset: crate::builtins::BuiltinPluginAsset) -> Self {
        match Self::prepare_builtin_asset_record(asset) {
            Ok(record) => {
                let _ = insert_loaded_plugin(
                    &mut self.plugins,
                    &mut self.aliases,
                    record,
                    false,
                    false,
                );
            }
            Err(error) => warn!(error = %error, "failed to load built-in list provider plugin"),
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let _ = remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type);
        self
    }

    fn create_client(
        loaded: &LoadedPlugin,
        config: &BTreeMap<String, String>,
        pacer: &ListFetchPacer,
    ) -> Option<Arc<dyn ListProviderClient>> {
        Self::build_client(loaded, config, pacer)
            .map(|client| Arc::new(client) as Arc<dyn ListProviderClient>)
    }

    fn build_client(
        loaded: &LoadedPlugin,
        config: &BTreeMap<String, String>,
        pacer: &ListFetchPacer,
    ) -> Option<WasmListClient> {
        let provider_type = loaded.descriptor.provider_type().to_string();
        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                warn!(
                    provider_type = provider_type.as_str(),
                    error = %error,
                    "failed to materialize list provider plugin bytes"
                );
                return None;
            }
        };
        match PluginRuntimeBacking::for_artifact(&loaded.descriptor, &wasm_bytes) {
            Ok(PluginRuntimeBacking::List) => {}
            Ok(_) => {
                warn!(
                    provider_type = provider_type.as_str(),
                    "list provider selected a runtime that is not valid for this descriptor family"
                );
                return None;
            }
            Err(error) => {
                warn!(
                    provider_type = provider_type.as_str(),
                    error = %error,
                    "list provider has an invalid runtime marker"
                );
                return None;
            }
        }

        // Only the keys the descriptor declares reach the guest; anything
        // else a caller put in the map stays on the host.
        let declared = loaded
            .descriptor
            .config_fields()
            .iter()
            .map(|field| field.key.as_str())
            .collect::<std::collections::HashSet<_>>();
        let plugin_config = config
            .iter()
            .filter(|(key, _)| declared.contains(key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        let config_json = serde_json::to_string(&plugin_config).unwrap_or_default();
        let base_url = loaded
            .descriptor
            .list_provider()
            .and_then(|provider| provider.default_base_url.as_deref());
        let allowed_hosts =
            allowed_hosts_for_descriptor(&loaded.descriptor, base_url, Some(&config_json));
        let egress_policy = operator_egress_policy_for_descriptor(None, Some(&config_json));

        let command_host = CommandHost::with_archive_provider(
            loaded.descriptor.id.clone(),
            plugin_config.clone(),
            allowed_hosts.clone(),
            egress_policy.clone(),
            LIST_PLUGIN_TIMEOUT,
            None,
            None,
        );
        let mut client = WasmListClient::new(
            PluginInstanceSpec {
                wasm: Arc::new(wasm_bytes),
                // List providers have no filesystem authority.
                preopens: Vec::new(),
                timeout: LIST_PLUGIN_TIMEOUT,
                memory_max_bytes: None,
                command_host,
            },
            loaded.descriptor.clone(),
        );
        client.binding = Some(ListHostBinding {
            plugin_config,
            allowed_hosts,
            egress_policy,
        });
        client.pacer = pacer.clone();
        Some(client)
    }
}

impl ListPluginProvider for WasmListPluginProvider {
    fn client_for_provider(
        &self,
        provider_type: &str,
        config: &BTreeMap<String, String>,
    ) -> Option<Arc<dyn ListProviderClient>> {
        let loaded = resolve_loaded_plugin(&self.plugins, &self.aliases, provider_type)?;
        Self::create_client(loaded, config, &self.pacer)
    }

    fn descriptors(&self) -> Vec<PluginDescriptor> {
        let mut descriptors = self
            .plugins
            .values()
            .map(|loaded| loaded.descriptor.clone())
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.provider_type().cmp(right.provider_type()));
        descriptors
    }

    fn available_provider_types(&self) -> Vec<String> {
        let mut keys = self.plugins.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        keys
    }
}

/// The reloadable wrapper the application holds.
pub struct DynamicListPluginProvider {
    inner: std::sync::RwLock<WasmListPluginProvider>,
}

impl DynamicListPluginProvider {
    pub fn new(provider: WasmListPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
        }
    }

    /// Swap in a rebuilt provider. Fetch pacing carries over, so a reload
    /// never lets a provider be called sooner than it asked.
    pub fn reload(&self, mut provider: WasmListPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        provider.pacer = guard.pacer.clone();
        *guard = provider;
    }

    fn replace_with(&self, update: impl FnOnce(WasmListPluginProvider) -> WasmListPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = std::mem::replace(&mut *guard, WasmListPluginProvider::empty());
        *guard = update(current);
    }
}

impl ListPluginProvider for DynamicListPluginProvider {
    fn client_for_provider(
        &self,
        provider_type: &str,
        config: &BTreeMap<String, String>,
    ) -> Option<Arc<dyn ListProviderClient>> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .client_for_provider(provider_type, config)
    }

    fn descriptors(&self) -> Vec<PluginDescriptor> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .descriptors()
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .available_provider_types()
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        self.replace_with(|current| current.with_runtime_plugin(plugin));
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        self.replace_with(|current| current.without_provider_type(provider_type));
        Ok(())
    }

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_list_plugin_provider_from_runtime_plugins(
            runtime_plugins,
            disabled_builtins,
        ));
        Ok(())
    }
}

pub fn build_list_plugin_provider(
    external_wasm_bytes: &[ExternalPluginWasm<'_>],
    disabled_builtins: &[String],
) -> WasmListPluginProvider {
    let mut provider = WasmListPluginProvider::empty();
    for plugin in external_wasm_bytes {
        provider = provider.with_external_plugin(*plugin);
    }
    for asset in crate::builtins::LIST_BUILTINS {
        provider = provider.with_builtin_asset(*asset);
    }
    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }
    provider
}

pub fn build_list_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
    disabled_builtins: &[String],
) -> WasmListPluginProvider {
    let mut provider = WasmListPluginProvider::empty();
    for plugin in runtime_plugins.iter().cloned() {
        provider = provider.with_runtime_plugin(plugin);
    }
    for asset in crate::builtins::LIST_BUILTINS {
        provider = provider.with_builtin_asset(*asset);
    }
    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }
    provider
}

#[cfg(test)]
mod tests;
