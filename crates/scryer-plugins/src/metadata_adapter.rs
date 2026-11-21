//! Metadata-provider plugins: the client over one loaded
//! component and the provider that holds every installed one.
//!
//! A metadata provider has no per-instance configs. It sees the installation
//! settings its descriptor declares (an OMDb API key, a daily request limit)
//! and keeps its own cache and budget in plugin state.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use scryer_application::metadata_plugins::{MetadataPluginProvider, MetadataProviderClient};
use scryer_application::{AppError, AppResult, RuntimePluginLoad};
use scryer_plugin_sdk::command::{
    PluginCommand, PluginCommandRequest, PluginCommandResult, PluginMetadataCommand,
    PluginMetadataCommandResult,
};
use scryer_plugin_sdk::{
    MetadataMediaKind, MetadataPluginHealthRequest, MetadataPluginHealthResponse,
    MetadataPluginLookupRequest, MetadataPluginLookupResponse, MetadataPluginSearchRequest,
    MetadataPluginSearchResponse, PluginDescriptor, PluginResult,
};
use tracing::warn;

use crate::loader::{
    LoadedPlugin, LoadedPluginRecord, PluginLoadSource, allowed_hosts_for_descriptor,
    insert_loaded_plugin, operator_egress_policy_for_descriptor, remove_loaded_plugin,
    validate_descriptor_for_type,
};
use crate::runtime_backing::{PluginInstanceSpec, PluginRuntimeBacking};
use crate::wasmtime_host::command_host::CommandHost;
use crate::wasmtime_host::{MetadataComponentInvocation, process_metadata_component};

const METADATA_PLUGIN_TYPE: &str = "metadata_provider";

/// Wall-clock budget for one metadata invocation: one or two HTTP requests.
const METADATA_PLUGIN_TIMEOUT: Duration = Duration::from_secs(30);

/// One `scryer:metadata/metadata-provider@1.0.0` component with its
/// installation settings bound.
pub struct WasmMetadataClient {
    spec: PluginInstanceSpec,
    descriptor: PluginDescriptor,
    /// What the client was built from, to tell an unchanged plugin on reload.
    descriptor_json: Vec<u8>,
    plugin_config: BTreeMap<String, String>,
}

impl WasmMetadataClient {
    async fn invoke(
        &self,
        command: PluginMetadataCommand,
        operation: &'static str,
    ) -> AppResult<PluginMetadataCommandResult> {
        let response = process_metadata_component(
            &self.spec,
            &PluginCommandRequest::new(PluginCommand::Metadata(command)),
            MetadataComponentInvocation {
                plugin_id: &self.descriptor.id,
                plugin_version: &self.descriptor.version,
                operation,
            },
        )
        .await?;
        match response.response {
            PluginCommandResult::Metadata(result) => Ok(result),
            _ => Err(AppError::Repository(format!(
                "metadata plugin {} returned a response for another plugin family",
                self.descriptor.id
            ))),
        }
    }

    fn wrong_operation(&self, operation: &str) -> AppError {
        AppError::Repository(format!(
            "metadata plugin {} answered a {operation} command with another operation",
            self.descriptor.id
        ))
    }
}

#[async_trait]
impl MetadataProviderClient for WasmMetadataClient {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    async fn lookup(
        &self,
        kind: MetadataMediaKind,
        imdb_id: &str,
    ) -> AppResult<PluginResult<MetadataPluginLookupResponse>> {
        match self
            .invoke(
                PluginMetadataCommand::Lookup(MetadataPluginLookupRequest {
                    kind,
                    imdb_id: imdb_id.to_string(),
                }),
                "metadata_lookup",
            )
            .await?
        {
            PluginMetadataCommandResult::Lookup(result) => Ok(result),
            _ => Err(self.wrong_operation("lookup")),
        }
    }

    async fn search(
        &self,
        kind: MetadataMediaKind,
        query: &str,
        year: Option<i32>,
        limit: u32,
    ) -> AppResult<PluginResult<MetadataPluginSearchResponse>> {
        match self
            .invoke(
                PluginMetadataCommand::Search(MetadataPluginSearchRequest {
                    kind,
                    query: query.to_string(),
                    year,
                    limit,
                }),
                "metadata_search",
            )
            .await?
        {
            PluginMetadataCommandResult::Search(result) => Ok(result),
            _ => Err(self.wrong_operation("search")),
        }
    }

    async fn health(&self) -> AppResult<PluginResult<MetadataPluginHealthResponse>> {
        match self
            .invoke(
                PluginMetadataCommand::Health(MetadataPluginHealthRequest {}),
                "metadata_health",
            )
            .await?
        {
            PluginMetadataCommandResult::Health(result) => Ok(result),
            _ => Err(self.wrong_operation("health")),
        }
    }
}

/// The installed metadata providers, keyed by provider type with aliases.
///
/// Each provider's client owns the command host that holds the plugin's
/// state, so its answer cache and request budget live as long as the client.
/// A client is rebuilt only when its own plugin changes (bytes, descriptor or
/// settings); installing, updating, removing or reloading any other plugin
/// keeps it. State is in memory, so a restart starts it afresh.
pub struct WasmMetadataPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
    /// One client per loaded provider, by primary key (sorted).
    clients: BTreeMap<String, Arc<WasmMetadataClient>>,
}

impl WasmMetadataPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
            clients: BTreeMap::new(),
        }
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        if !validate_descriptor_for_type(
            &plugin.descriptor,
            Some(METADATA_PLUGIN_TYPE),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("metadata provider descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(
            LoadedPlugin::from_owned(plugin.descriptor, plugin.wasm_bytes)
                .with_settings(plugin.settings),
        ))
    }

    /// Load one plugin without building clients. Returns the primary keys
    /// whose client must be refreshed.
    fn insert_runtime_plugin(&mut self, plugin: RuntimePluginLoad) -> Result<Vec<String>, String> {
        let record = Self::prepare_runtime_plugin_record(plugin)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            true,
            true,
        ))
    }

    /// Install or replace one plugin, refreshing only its own client.
    pub fn try_with_runtime_plugin(
        mut self,
        plugin: RuntimePluginLoad,
    ) -> (Self, Result<(), String>) {
        match self.insert_runtime_plugin(plugin) {
            Ok(affected) => {
                self.refresh_clients(&affected);
                (self, Ok(()))
            }
            Err(error) => (self, Err(error)),
        }
    }

    pub fn with_runtime_plugin(self, plugin: RuntimePluginLoad) -> Self {
        let (provider, result) = self.try_with_runtime_plugin(plugin);
        if let Err(error) = result {
            warn!(error = %error, "failed to load runtime metadata provider plugin");
        }
        provider
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let affected = remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type);
        self.refresh_clients(&affected);
        self
    }

    /// Bring the clients for `keys` in line with the loaded plugins: build or
    /// keep one for each key still loaded, drop the rest.
    fn refresh_clients(&mut self, keys: &[String]) {
        for key in keys {
            let previous = self.clients.remove(key);
            if let Some(loaded) = self.plugins.get(key)
                && let Some(client) = Self::build_or_keep(loaded, previous)
            {
                self.clients.insert(key.clone(), client);
            }
        }
    }

    /// Load every plugin in `runtime_plugins`, keeping the client from
    /// `previous` for each plugin that is unchanged, and building clients
    /// once, after all plugins are loaded.
    fn from_runtime_plugins(
        runtime_plugins: &[RuntimePluginLoad],
        mut previous: BTreeMap<String, Arc<WasmMetadataClient>>,
    ) -> Self {
        let mut provider = Self::empty();
        for plugin in runtime_plugins.iter().cloned() {
            if let Err(error) = provider.insert_runtime_plugin(plugin) {
                warn!(error = %error, "failed to load runtime metadata provider plugin");
            }
        }
        let mut keys = provider.plugins.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        for key in keys {
            let kept = previous.remove(&key);
            if let Some(loaded) = provider.plugins.get(&key)
                && let Some(client) = Self::build_or_keep(loaded, kept)
            {
                provider.clients.insert(key, client);
            }
        }
        provider
    }

    /// The client for `loaded`: `previous` when it was built from the same
    /// bytes, descriptor and settings, otherwise a new one.
    fn build_or_keep(
        loaded: &LoadedPlugin,
        previous: Option<Arc<WasmMetadataClient>>,
    ) -> Option<Arc<WasmMetadataClient>> {
        let provider_type = loaded.descriptor.provider_type().to_string();
        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                warn!(
                    provider_type = provider_type.as_str(),
                    error = %error,
                    "failed to materialize metadata provider plugin bytes"
                );
                return None;
            }
        };
        let mut plugin_config = BTreeMap::new();
        loaded.bind_settings(&mut plugin_config);
        let descriptor_json = serde_json::to_vec(&loaded.descriptor).unwrap_or_default();
        if let Some(previous) = previous
            && previous.descriptor_json == descriptor_json
            && previous.plugin_config == plugin_config
            && previous.spec.wasm.as_slice() == wasm_bytes.as_slice()
        {
            return Some(previous);
        }
        Self::build_client(loaded, wasm_bytes, plugin_config, descriptor_json).map(Arc::new)
    }

    fn build_client(
        loaded: &LoadedPlugin,
        wasm_bytes: Vec<u8>,
        plugin_config: BTreeMap<String, String>,
        descriptor_json: Vec<u8>,
    ) -> Option<WasmMetadataClient> {
        let provider_type = loaded.descriptor.provider_type().to_string();
        match PluginRuntimeBacking::for_artifact(&loaded.descriptor, &wasm_bytes) {
            Ok(PluginRuntimeBacking::Metadata) => {}
            Ok(_) => {
                warn!(
                    provider_type = provider_type.as_str(),
                    "metadata provider selected a runtime that is not valid for this descriptor family"
                );
                return None;
            }
            Err(error) => {
                warn!(
                    provider_type = provider_type.as_str(),
                    error = %error,
                    "metadata provider has an invalid runtime marker"
                );
                return None;
            }
        }

        let base_url = loaded
            .descriptor
            .metadata_provider()
            .and_then(|provider| provider.default_base_url.as_deref());
        let allowed_hosts = allowed_hosts_for_descriptor(&loaded.descriptor, base_url, Some("{}"));
        let egress_policy = operator_egress_policy_for_descriptor(None, Some("{}"));
        let command_host = CommandHost::with_archive_provider(
            loaded.descriptor.id.clone(),
            plugin_config.clone(),
            allowed_hosts,
            egress_policy,
            METADATA_PLUGIN_TIMEOUT,
            None,
            None,
        );
        Some(WasmMetadataClient {
            spec: PluginInstanceSpec {
                wasm: Arc::new(wasm_bytes),
                // Metadata providers have no filesystem authority.
                preopens: Vec::new(),
                timeout: METADATA_PLUGIN_TIMEOUT,
                memory_max_bytes: None,
                command_host,
            },
            descriptor: loaded.descriptor.clone(),
            descriptor_json,
            plugin_config,
        })
    }
}

impl MetadataPluginProvider for WasmMetadataPluginProvider {
    fn clients(&self) -> Vec<Arc<dyn MetadataProviderClient>> {
        self.clients
            .values()
            .map(|client| client.clone() as Arc<dyn MetadataProviderClient>)
            .collect()
    }
}

/// The reloadable wrapper the application holds.
pub struct DynamicMetadataPluginProvider {
    inner: std::sync::RwLock<WasmMetadataPluginProvider>,
}

impl DynamicMetadataPluginProvider {
    pub fn new(provider: WasmMetadataPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
        }
    }

    fn replace_with<T>(
        &self,
        update: impl FnOnce(WasmMetadataPluginProvider) -> (WasmMetadataPluginProvider, T),
    ) -> T {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = std::mem::replace(&mut *guard, WasmMetadataPluginProvider::empty());
        let (next, result) = update(current);
        *guard = next;
        result
    }
}

impl MetadataPluginProvider for DynamicMetadataPluginProvider {
    fn clients(&self) -> Vec<Arc<dyn MetadataProviderClient>> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clients()
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        self.replace_with(|current| current.try_with_runtime_plugin(plugin))
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        self.replace_with(|current| (current.without_provider_type(provider_type), ()));
        Ok(())
    }

    fn reload_runtime_plugins(&self, runtime_plugins: &[RuntimePluginLoad]) -> Result<(), String> {
        self.replace_with(|current| {
            (
                WasmMetadataPluginProvider::from_runtime_plugins(runtime_plugins, current.clients),
                (),
            )
        });
        Ok(())
    }
}

pub fn build_metadata_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
) -> WasmMetadataPluginProvider {
    WasmMetadataPluginProvider::from_runtime_plugins(runtime_plugins, BTreeMap::new())
}

#[cfg(test)]
mod tests {
    use scryer_plugin_sdk::{
        MetadataProviderCapabilities, MetadataProviderDescriptor, ProviderDescriptor,
    };

    use super::*;

    fn runtime_plugin(allowed_hosts: Vec<String>) -> RuntimePluginLoad {
        RuntimePluginLoad {
            installation_id: Some("installation".to_string()),
            descriptor: PluginDescriptor {
                id: "example-metadata".to_string(),
                name: "Example metadata".to_string(),
                version: "0.1.0".to_string(),
                sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
                sdk_constraint: String::new(),
                socket_permissions: Vec::new(),
                settings: Vec::new(),
                provider: ProviderDescriptor::MetadataProvider(MetadataProviderDescriptor {
                    provider_type: "example".to_string(),
                    provider_aliases: Vec::new(),
                    display_name: "Example".to_string(),
                    summary: None,
                    coverage: vec![MetadataMediaKind::Movie],
                    capabilities: MetadataProviderCapabilities::default(),
                    config_fields: Vec::new(),
                    default_base_url: None,
                    allowed_hosts,
                }),
            },
            wasm_bytes: Vec::new(),
            first_party: false,
            settings: BTreeMap::new(),
        }
    }

    #[test]
    fn a_rejected_install_is_reported_and_loads_nothing() {
        let provider = DynamicMetadataPluginProvider::new(WasmMetadataPluginProvider::empty());
        let result =
            provider.upsert_runtime_plugin(runtime_plugin(vec!["not a host/".to_string()]));
        assert!(result.is_err());
        assert!(provider.clients().is_empty());
    }
}
