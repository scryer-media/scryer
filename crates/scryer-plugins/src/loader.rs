use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use scryer_application::{
    AppError, AppResult, ArchiveExtractorClient, ArchiveExtractorPluginProvider, DownloadClient,
    DownloadClientPluginProvider, ExternalPluginWasm, IndexerClient, IndexerErrorRecorder,
    IndexerPluginProvider, NotificationClient, NotificationPluginProvider,
    NullIndexerErrorRecorder, PluginDescriptorLoader, RuntimePluginLoad, SubtitlePluginProvider,
    SubtitleProviderClient, SubtitleSyncClient,
};
use scryer_domain::{
    DownloadClientConfig, IndexerConfig, NotificationChannelConfig, PluginHostBindingId,
    ProxyConfig, SubtitleProviderConfig,
};
use tracing::{debug, info, warn};

use crate::archive_adapter::WasmArchiveExtractorClient;
use crate::download_client_adapter::WasmDownloadClient;
use crate::embedded_descriptor::embedded_descriptor_from_wasm;
use crate::indexer_adapter::WasmIndexerClient;
use crate::notification_adapter::WasmNotificationClient;
use crate::process_host::ProcessHost;
use crate::runtime_backing::{PluginInstanceSpec, PluginRuntimeBacking};
use crate::socket_host::SocketHost;
use crate::subtitle_adapter::WasmSubtitleClient;
use crate::subtitle_sync_adapter::WasmSubtitleSyncClient;
use crate::types::{
    ArchivePluginFormat, ConfigFieldRole, ConfigFieldValueSource, PluginDescriptor,
    PluginHostBindingId as SdkHostBinding, PluginKind, ProviderDescriptor, SDK_VERSION,
    SubtitleProviderMode, config_fields_to_domain, indexer_capabilities_to_domain,
    plugin_descriptor_sdk_constraint, validate_plugin_descriptor_sdk_contract,
};
use crate::wasmtime_host::module_cache::{self, ModuleFlavor, RehydrationArtifact};

const INDEXER_PLUGIN_TYPES: &[&str] = &["indexer", "usenet_indexer", "torrent_indexer"];

/// Schedule process-local module rehydration for all enabled persisted and
/// built-in plugins. Registration happens synchronously before the background
/// worker starts so a request can wait for its own shared task instead of
/// compiling an artifact independently.
pub fn schedule_plugin_rehydration(
    runtime_plugins: &[RuntimePluginLoad],
    disabled_builtins: &[String],
) {
    let mut artifacts = runtime_plugins
        .iter()
        .filter_map(|plugin| {
            let flavor = match module_flavor_for_artifact(&plugin.descriptor, &plugin.wasm_bytes) {
                Ok(flavor) => flavor,
                Err(error) => {
                    warn!(
                        plugin_id = plugin.descriptor.id.as_str(),
                        plugin_version = plugin.descriptor.version.as_str(),
                        error = %error,
                        "skipping persisted plugin artifact rehydration"
                    );
                    return None;
                }
            };
            Some(RehydrationArtifact {
                plugin_id: plugin.descriptor.id.clone(),
                plugin_version: plugin.descriptor.version.clone(),
                flavor,
                wasm: plugin.wasm_bytes.clone(),
            })
        })
        .collect::<Vec<_>>();

    let builtin_assets = crate::builtins::INDEXER_BUILTINS
        .iter()
        .chain(crate::builtins::SUBTITLE_BUILTINS.iter())
        .chain(crate::builtins::DOWNLOAD_CLIENT_BUILTINS.iter())
        .chain(crate::builtins::NOTIFICATION_BUILTINS.iter());
    for asset in builtin_assets {
        let descriptor = match parse_builtin_descriptor(*asset) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                warn!(error = %error, "skipping built-in plugin module rehydration");
                continue;
            }
        };
        if disabled_builtins
            .iter()
            .any(|provider_type| provider_type.eq_ignore_ascii_case(descriptor.provider_type()))
        {
            continue;
        }
        match crate::builtins::decode_builtin_wasm(*asset) {
            Ok(wasm) => match module_flavor_for_artifact(&descriptor, &wasm) {
                Ok(flavor) => artifacts.push(RehydrationArtifact {
                    plugin_id: descriptor.id,
                    plugin_version: descriptor.version,
                    flavor,
                    wasm,
                }),
                Err(error) => warn!(
                    plugin_id = descriptor.id.as_str(),
                    plugin_version = descriptor.version.as_str(),
                    error = %error,
                    "skipping built-in plugin component rehydration"
                ),
            },
            Err(error) => warn!(
                plugin_id = descriptor.id.as_str(),
                plugin_version = descriptor.version.as_str(),
                error = %error,
                "skipping built-in plugin module rehydration"
            ),
        }
    }

    module_cache::schedule_rehydration(artifacts);
}

fn module_flavor_for_artifact(
    descriptor: &PluginDescriptor,
    wasm: &[u8],
) -> Result<ModuleFlavor, String> {
    Ok(
        match PluginRuntimeBacking::for_artifact(descriptor, wasm)? {
            PluginRuntimeBacking::Indexer => ModuleFlavor::Indexer,
            PluginRuntimeBacking::Archive => ModuleFlavor::Archive,
            PluginRuntimeBacking::Subtitle => ModuleFlavor::Subtitle,
            PluginRuntimeBacking::DownloadClient => ModuleFlavor::DownloadClient,
            PluginRuntimeBacking::Notification => ModuleFlavor::Notification,
        },
    )
}

type IndexerClientCacheKey = (String, String, String, String, String);
type IndexerClientCache = std::sync::Mutex<HashMap<IndexerClientCacheKey, Arc<dyn IndexerClient>>>;
/// `(provider type, config id, config revision, proxy id, proxy revision)` —
/// the same shape as the indexer key, so editing an assigned proxy rebuilds the
/// client and unassigning one drops the proxied build.
type DownloadClientCacheKey = (String, String, String, String, String);
type DownloadClientCache =
    std::sync::Mutex<HashMap<DownloadClientCacheKey, Arc<dyn DownloadClient>>>;
type NotificationClientCacheKey = (String, String, String);
type NotificationClientCache =
    std::sync::Mutex<HashMap<NotificationClientCacheKey, Arc<dyn NotificationClient>>>;
type SubtitleClientCacheKey = (String, String, String, String, String);
type SubtitleClientCache =
    std::sync::Mutex<HashMap<SubtitleClientCacheKey, Arc<dyn SubtitleProviderClient>>>;
type ArchiveExtractorClientCache =
    std::sync::Mutex<HashMap<String, Arc<dyn ArchiveExtractorClient>>>;

fn log_stale_plugin_cache_eviction(
    plugin_family: &'static str,
    provider_type: &str,
    config_id: &str,
    evicted_count: usize,
) {
    if evicted_count > 0 {
        debug!(
            plugin_family = plugin_family,
            provider_type = provider_type,
            config_id = config_id,
            evicted_count = evicted_count,
            "evicted stale plugin client cache entries"
        );
    }
}

fn insert_plugin_client_cache<K, Client>(
    cache: &mut HashMap<K, Arc<Client>>,
    cache_key: K,
    client: Arc<Client>,
    plugin_family: &'static str,
    provider_type: &str,
    config_id: &str,
    same_identity: impl Fn(&K) -> bool,
) -> Arc<Client>
where
    K: Eq + Hash,
    Client: ?Sized,
{
    if let Some(existing) = cache.get(&cache_key).cloned() {
        let before_len = cache.len();
        cache.retain(|key, _| key == &cache_key || !same_identity(key));
        log_stale_plugin_cache_eviction(
            plugin_family,
            provider_type,
            config_id,
            before_len - cache.len(),
        );
        return existing;
    }

    let before_len = cache.len();
    cache.retain(|key, _| !same_identity(key));
    log_stale_plugin_cache_eviction(
        plugin_family,
        provider_type,
        config_id,
        before_len - cache.len(),
    );
    cache.insert(cache_key, Arc::clone(&client));
    client
}

fn insert_indexer_client_cache(
    cache: &mut HashMap<IndexerClientCacheKey, Arc<dyn IndexerClient>>,
    cache_key: IndexerClientCacheKey,
    client: Arc<dyn IndexerClient>,
) -> Arc<dyn IndexerClient> {
    let provider_type = cache_key.0.clone();
    let config_id = cache_key.1.clone();
    insert_plugin_client_cache(
        cache,
        cache_key,
        client,
        "indexer",
        &provider_type,
        &config_id,
        |key| key.0.as_str() == provider_type.as_str() && key.1.as_str() == config_id.as_str(),
    )
}

fn insert_download_client_cache(
    cache: &mut HashMap<DownloadClientCacheKey, Arc<dyn DownloadClient>>,
    cache_key: DownloadClientCacheKey,
    client: Arc<dyn DownloadClient>,
) -> Arc<dyn DownloadClient> {
    let provider_type = cache_key.0.clone();
    let config_id = cache_key.1.clone();
    insert_plugin_client_cache(
        cache,
        cache_key,
        client,
        "download",
        &provider_type,
        &config_id,
        |key| key.0.as_str() == provider_type.as_str() && key.1.as_str() == config_id.as_str(),
    )
}

fn insert_notification_client_cache(
    cache: &mut HashMap<NotificationClientCacheKey, Arc<dyn NotificationClient>>,
    cache_key: NotificationClientCacheKey,
    client: Arc<dyn NotificationClient>,
) -> Arc<dyn NotificationClient> {
    let provider_type = cache_key.0.clone();
    let config_id = cache_key.1.clone();
    insert_plugin_client_cache(
        cache,
        cache_key,
        client,
        "notification",
        &provider_type,
        &config_id,
        |key| key.0.as_str() == provider_type.as_str() && key.1.as_str() == config_id.as_str(),
    )
}

fn insert_subtitle_client_cache(
    cache: &mut HashMap<SubtitleClientCacheKey, Arc<dyn SubtitleProviderClient>>,
    cache_key: SubtitleClientCacheKey,
    client: Arc<dyn SubtitleProviderClient>,
) -> Arc<dyn SubtitleProviderClient> {
    let provider_type = cache_key.0.clone();
    let config_id = cache_key.1.clone();
    insert_plugin_client_cache(
        cache,
        cache_key,
        client,
        "subtitle",
        &provider_type,
        &config_id,
        |key| key.0.as_str() == provider_type.as_str() && key.1.as_str() == config_id.as_str(),
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PluginLoadSource {
    Builtin,
    External { first_party: bool },
}

impl PluginLoadSource {
    fn can_use_first_party_host_bindings(self) -> bool {
        matches!(self, Self::Builtin | Self::External { first_party: true })
    }
}

enum LoadedPluginBacking {
    Owned(Vec<u8>),
    Builtin(crate::builtins::BuiltinPluginAsset),
}

struct LoadedPlugin {
    wasm: LoadedPluginBacking,
    descriptor: PluginDescriptor,
    /// Trust provenance of this plugin. Drives host-capability gating (e.g. the
    /// host-process capability) via `can_use_first_party_host_bindings`.
    load_source: PluginLoadSource,
}

impl LoadedPlugin {
    fn from_owned(descriptor: PluginDescriptor, wasm_bytes: Vec<u8>) -> Self {
        Self {
            wasm: LoadedPluginBacking::Owned(wasm_bytes),
            descriptor,
            // Default to the least-privileged provenance; callers that know the
            // plugin is first-party override via `with_load_source`.
            load_source: PluginLoadSource::External { first_party: false },
        }
    }

    fn from_builtin(
        descriptor: PluginDescriptor,
        asset: crate::builtins::BuiltinPluginAsset,
    ) -> Self {
        Self {
            wasm: LoadedPluginBacking::Builtin(asset),
            descriptor,
            load_source: PluginLoadSource::Builtin,
        }
    }

    fn with_load_source(mut self, load_source: PluginLoadSource) -> Self {
        self.load_source = load_source;
        self
    }

    fn materialize_wasm(&self) -> Result<Vec<u8>, String> {
        match &self.wasm {
            LoadedPluginBacking::Owned(wasm_bytes) => Ok(wasm_bytes.clone()),
            LoadedPluginBacking::Builtin(asset) => crate::builtins::decode_builtin_wasm(*asset),
        }
    }

    #[cfg(test)]
    fn stores_builtin_asset(&self) -> bool {
        matches!(self.wasm, LoadedPluginBacking::Builtin(_))
    }
}

struct LoadedPluginRecord {
    primary_key: String,
    alias_keys: Vec<String>,
    loaded: LoadedPlugin,
}

impl LoadedPluginRecord {
    fn new(loaded: LoadedPlugin) -> Self {
        let primary_key = loaded
            .descriptor
            .provider_type()
            .trim()
            .to_ascii_lowercase();
        let alias_keys = loaded
            .descriptor
            .provider_aliases()
            .iter()
            .map(|alias| alias.trim().to_ascii_lowercase())
            .filter(|alias| !alias.is_empty() && alias != &primary_key)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        Self {
            primary_key,
            alias_keys,
            loaded,
        }
    }
}

fn resolve_loaded_plugin<'a>(
    plugins: &'a HashMap<String, LoadedPlugin>,
    aliases: &HashMap<String, String>,
    provider_type: &str,
) -> Option<&'a LoadedPlugin> {
    let key = provider_type.trim().to_ascii_lowercase();
    let primary = aliases
        .get(&key)
        .map(String::as_str)
        .unwrap_or(key.as_str());
    plugins.get(primary)
}

fn remove_loaded_plugin(
    plugins: &mut HashMap<String, LoadedPlugin>,
    aliases: &mut HashMap<String, String>,
    provider_type: &str,
) -> Vec<String> {
    let key = provider_type.trim().to_ascii_lowercase();
    let primary = aliases.get(&key).cloned().unwrap_or(key);
    let Some(_) = plugins.remove(&primary) else {
        return Vec::new();
    };

    let removed_aliases = aliases
        .iter()
        .filter(|(_, owner)| **owner == primary)
        .map(|(alias, _)| alias.clone())
        .collect::<Vec<_>>();
    for alias in &removed_aliases {
        aliases.remove(alias);
    }

    let mut affected = Vec::with_capacity(removed_aliases.len() + 1);
    affected.push(primary);
    affected.extend(removed_aliases);
    affected
}

fn insert_loaded_plugin(
    plugins: &mut HashMap<String, LoadedPlugin>,
    aliases: &mut HashMap<String, String>,
    record: LoadedPluginRecord,
    replace_existing_primary: bool,
    allow_alias_override: bool,
) -> Vec<String> {
    let mut affected = Vec::new();
    if plugins.contains_key(&record.primary_key) {
        if !replace_existing_primary {
            return affected;
        }
        affected.extend(remove_loaded_plugin(plugins, aliases, &record.primary_key));
    }

    let primary_key = record.primary_key.clone();
    let alias_keys = record.alias_keys.clone();
    plugins.insert(primary_key.clone(), record.loaded);
    affected.push(primary_key.clone());

    for alias in alias_keys {
        if plugins.contains_key(&alias) && alias != primary_key {
            continue;
        }
        if let Some(existing) = aliases.get(&alias)
            && existing != &primary_key
            && !allow_alias_override
        {
            continue;
        }
        aliases.insert(alias.clone(), primary_key.clone());
        affected.push(alias);
    }

    affected.sort();
    affected.dedup();
    affected
}

fn parse_builtin_descriptor(
    asset: crate::builtins::BuiltinPluginAsset,
) -> Result<PluginDescriptor, String> {
    serde_json::from_str(asset.descriptor_json)
        .map_err(|error| format!("built-in descriptor JSON is invalid: {error}"))
}

fn builtin_provider_types_from_assets(
    assets: &[crate::builtins::BuiltinPluginAsset],
    plugin_type_filter: impl Fn(&str) -> bool,
    apply_overrides: impl Fn(PluginDescriptor) -> PluginDescriptor,
) -> Vec<String> {
    assets
        .iter()
        .filter_map(|asset| parse_builtin_descriptor(*asset).ok())
        .map(apply_overrides)
        .filter(|descriptor| plugin_type_filter(descriptor.plugin_type()))
        .map(|descriptor| descriptor.provider_type().trim().to_ascii_lowercase())
        .collect()
}

fn builtin_indexer_provider_types() -> Vec<String> {
    static BUILTIN_INDEXER_PROVIDER_TYPES: LazyLock<Vec<String>> = LazyLock::new(|| {
        builtin_provider_types_from_assets(
            crate::builtins::INDEXER_BUILTINS,
            is_indexer_plugin_type,
            |descriptor| descriptor,
        )
    });

    BUILTIN_INDEXER_PROVIDER_TYPES.clone()
}

fn builtin_subtitle_provider_types() -> Vec<String> {
    static BUILTIN_SUBTITLE_PROVIDER_TYPES: LazyLock<Vec<String>> = LazyLock::new(|| {
        builtin_provider_types_from_assets(
            crate::builtins::SUBTITLE_BUILTINS,
            |plugin_type| plugin_type == "subtitle_provider",
            |descriptor| descriptor,
        )
    });

    BUILTIN_SUBTITLE_PROVIDER_TYPES.clone()
}

pub struct WasmIndexerPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
    indexer_error_recorder: Arc<dyn IndexerErrorRecorder>,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
}

impl WasmIndexerPluginProvider {
    /// Create an empty provider with no plugins loaded.
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
            indexer_error_recorder: Arc::new(NullIndexerErrorRecorder),
            archive_provider: None,
        }
    }

    pub fn with_indexer_error_recorder(
        mut self,
        indexer_error_recorder: Arc<dyn IndexerErrorRecorder>,
    ) -> Self {
        self.indexer_error_recorder = indexer_error_recorder;
        self
    }

    /// Give this provider's plugins the host-owned archive-extraction service.
    pub fn with_archive_extractor_provider(
        mut self,
        archive_provider: Arc<dyn ArchiveExtractorPluginProvider>,
    ) -> Self {
        self.archive_provider = Some(archive_provider);
        self
    }

    /// Register an externally-installed plugin from WASM bytes.
    /// External plugins take priority over built-ins with the same provider_type.
    pub fn with_external_bytes(self, wasm_bytes: &[u8]) -> Self {
        self.with_external_plugin(ExternalPluginWasm {
            bytes: wasm_bytes,
            first_party: false,
        })
    }

    fn prepare_external_plugin_record(
        plugin: ExternalPluginWasm<'_>,
    ) -> Result<LoadedPluginRecord, String> {
        let (descriptor, wasm_bytes) = load_from_bytes(plugin.bytes)?;
        if !validate_indexer_descriptor(
            &descriptor,
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("indexer descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            descriptor, wasm_bytes,
        )))
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        let descriptor = plugin.descriptor;
        if !validate_indexer_descriptor(
            &descriptor,
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("indexer descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            descriptor,
            plugin.wasm_bytes,
        )))
    }

    fn prepare_builtin_asset_record(
        asset: crate::builtins::BuiltinPluginAsset,
    ) -> Result<LoadedPluginRecord, String> {
        let descriptor = parse_builtin_descriptor(asset)?;
        if !validate_indexer_descriptor(&descriptor, PluginLoadSource::Builtin) {
            return Err("built-in indexer descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_builtin(
            descriptor, asset,
        )))
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match Self::prepare_external_plugin_record(plugin) {
            Ok(record) => {
                info!(
                    plugin = record.loaded.descriptor.name.as_str(),
                    version = record.loaded.descriptor.version.as_str(),
                    provider_type = record.primary_key.as_str(),
                    "registered external plugin"
                );
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load external plugin");
            }
        }
        self
    }

    fn with_runtime_plugin(mut self, plugin: RuntimePluginLoad) -> Self {
        match Self::prepare_runtime_plugin_record(plugin) {
            Ok(record) => {
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load runtime indexer plugin");
            }
        }
        self
    }

    fn restore_builtin_provider_type(
        &mut self,
        provider_type: &str,
    ) -> Result<Vec<String>, String> {
        let asset = builtin_indexer_asset_for_provider(provider_type).ok_or_else(|| {
            format!("no built-in indexer plugin is available for provider '{provider_type}'")
        })?;
        let record = Self::prepare_builtin_asset_record(asset)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            false,
            false,
        ))
    }

    fn prepare_builtin_provider_type(&self, provider_type: &str) -> Result<(), String> {
        let asset = builtin_indexer_asset_for_provider(provider_type).ok_or_else(|| {
            format!("no built-in indexer plugin is available for provider '{provider_type}'")
        })?;
        let wasm_bytes = crate::builtins::decode_builtin_wasm(asset)?;
        let (descriptor, _) = load_from_bytes(&wasm_bytes)?;
        if !validate_indexer_descriptor(&descriptor, PluginLoadSource::Builtin)
            || !descriptor
                .provider_type()
                .eq_ignore_ascii_case(provider_type)
        {
            return Err("built-in indexer descriptor rejected".to_string());
        }
        Ok(())
    }

    fn upsert_runtime_plugin_record(
        &mut self,
        plugin: RuntimePluginLoad,
    ) -> Result<Vec<String>, String> {
        let record = Self::prepare_runtime_plugin_record(plugin)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            true,
            true,
        ))
    }

    fn remove_provider_type(&mut self, provider_type: &str) -> Vec<String> {
        remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type)
    }

    fn get_loaded(&self, provider_type: &str) -> Option<&LoadedPlugin> {
        resolve_loaded_plugin(&self.plugins, &self.aliases, provider_type)
    }

    /// Remove a provider_type (and its aliases) from the loaded set.
    /// Used to disable built-in plugins at runtime.
    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let _ = self.remove_provider_type(provider_type);
        self
    }

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
            Err(error) => {
                warn!(error = %error, "failed to load built-in plugin");
            }
        }
        self
    }
}

fn builtin_indexer_asset_for_provider(
    provider_type: &str,
) -> Option<crate::builtins::BuiltinPluginAsset> {
    match provider_type.trim().to_ascii_lowercase().as_str() {
        "newznab" => Some(crate::builtins::NEWZNAB),
        "torznab" => Some(crate::builtins::TORZNAB),
        _ => None,
    }
}

impl IndexerPluginProvider for WasmIndexerPluginProvider {
    fn validate_config_for_provider(
        &self,
        provider_type: &str,
        config_json: &str,
    ) -> AppResult<()> {
        let Some(loaded) = self.get_loaded(provider_type) else {
            return Ok(());
        };
        let Some(indexer) = loaded.descriptor.indexer() else {
            return Ok(());
        };
        if indexer.provider_profiles.is_empty() {
            return Ok(());
        }
        crate::newznab_profiles::resolve_newznab_profile(indexer, provider_type, Some(config_json))
            .map(|_| ())
            .map_err(|error| AppError::Validation(error.to_string()))
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        builtin_indexer_provider_types()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn search_semantics_version_for_provider(&self, provider_type: &str) -> Option<u32> {
        self.get_loaded(provider_type)
            .and_then(|loaded| loaded.descriptor.indexer())
            .and_then(|indexer| indexer.search_semantics_version)
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn plugin_type_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.plugin_type().to_string())
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        // Deduplicate: multiple keys may point to the same plugin. Use the
        // primary provider_type as the canonical source for scoring policies.
        let mut seen = std::collections::HashSet::new();
        self.plugins
            .values()
            .filter(|loaded| seen.insert(loaded.descriptor.provider_type().to_string()))
            .flat_map(|loaded| {
                loaded.descriptor.indexer().into_iter().flat_map(|indexer| {
                    indexer.scoring_policies.iter().map(|sp| {
                        // ID must be a valid Rego path segment (letters, digits, underscores).
                        let safe_provider = loaded
                            .descriptor
                            .provider_type()
                            .replace(['-', ':', '.'], "_");
                        let safe_name = sp.name.replace(['-', ':', '.'], "_");
                        let id = format!("plugin_{safe_provider}_{safe_name}");
                        scryer_rules::UserPolicy {
                            id,
                            name: sp.name.clone(),
                            rego_source: sp.rego_source.clone(),
                            origin: scryer_rules::PolicyOrigin::User,
                            applied_facets: sp.applied_facets.clone(),
                        }
                    })
                })
            })
            .collect::<Vec<_>>()
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.get_loaded(provider_type)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        crate::builtins::builtin_description_for_provider(provider_type).map(str::to_string)
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .and_then(|loaded| default_indexer_connection_url(&loaded.descriptor))
    }

    fn rate_limit_seconds_for_provider(&self, provider_type: &str) -> Option<i64> {
        self.get_loaded(provider_type).and_then(|loaded| {
            loaded
                .descriptor
                .indexer()
                .and_then(|indexer| indexer.rate_limit_seconds)
        })
    }

    fn capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        self.get_loaded(provider_type)
            .map(|loaded| {
                loaded
                    .descriptor
                    .indexer()
                    .map(|indexer| indexer_capabilities_to_domain(&indexer.capabilities))
                    .unwrap_or_default()
            })
            .unwrap_or(scryer_domain::IndexerProviderCapabilities {
                rss: true,
                supported_ids: std::collections::HashMap::from([
                    ("movie".into(), vec!["imdb_id".into()]),
                    ("series".into(), vec!["tvdb_id".into()]),
                ]),
                deduplicates_aliases: false,
                season_param: Some("season".into()),
                episode_param: Some("ep".into()),
                query_param: Some("q".into()),
                supported_query_facets: vec!["movie".into(), "series".into(), "anime".into()],
                search: true,
                imdb_search: true,
                tvdb_search: true,
                anidb_search: false,
                ..Default::default()
            })
    }

    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        self.client_for_provider_with_proxy(config, None)
    }

    fn client_for_provider_with_proxy(
        &self,
        config: &IndexerConfig,
        proxy_config: Option<&ProxyConfig>,
    ) -> Option<Arc<dyn IndexerClient>> {
        let provider = config.provider_type.trim().to_ascii_lowercase();
        let loaded = self.get_loaded(&provider)?;

        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                tracing::warn!(
                    indexer = config.name.as_str(),
                    provider = provider.as_str(),
                    error = %error,
                    "failed to materialize WASM indexer plugin bytes"
                );
                return None;
            }
        };

        let backing = match indexer_runtime_backing(&loaded.descriptor, &wasm_bytes) {
            Ok(backing) => backing,
            Err(error) => {
                tracing::warn!(
                    indexer = config.name.as_str(),
                    provider = provider.as_str(),
                    error = %error,
                    "indexer plugin runtime is unusable, indexer will be unavailable"
                );
                return None;
            }
        };

        let built = match backing {
            PluginRuntimeBacking::Indexer => {
                WasmIndexerClient::new_component_with_indexer_error_recorder(
                    wasm_bytes,
                    loaded.descriptor.clone(),
                    config.name.clone(),
                    config.clone(),
                    proxy_config.cloned(),
                    Arc::clone(&self.indexer_error_recorder),
                )
            }
            other => Err(AppError::Repository(format!(
                "runtime {other:?} is not valid for an indexer descriptor"
            ))),
        };

        match built {
            Ok(client) => Some(Arc::new(client)),
            Err(e) => {
                tracing::warn!(
                    indexer = config.name.as_str(),
                    provider = provider.as_str(),
                    error = %e,
                    "failed to compile WASM plugin, indexer will be unavailable"
                );
                None
            }
        }
    }
}

/// Pick the runtime for one indexer artifact, or explain why it is unusable.
///
/// Indexers accept exactly one runtime: the `scryer:indexer` component world. A
/// pre-component artifact is refused here with the upgrade diagnostic, and any
/// other component family means the descriptor and the artifact disagree about
/// what this plugin is — running it would be a guess, so the loader drops the
/// indexer instead. Keeping the decision here (rather than inline) is what makes
/// the refusal a tested property rather than an incidental `else`.
fn indexer_runtime_backing(
    descriptor: &PluginDescriptor,
    wasm: &[u8],
) -> Result<PluginRuntimeBacking, String> {
    let backing = PluginRuntimeBacking::for_artifact(descriptor, wasm)?;
    match backing {
        PluginRuntimeBacking::Indexer => Ok(backing),
        other => Err(format!(
            "runtime {other:?} is not valid for an indexer descriptor"
        )),
    }
}

/// A thread-safe wrapper around `WasmIndexerPluginProvider` that supports
/// runtime reload. All reads acquire a `RwLock` read lock; `reload()` acquires
/// a write lock to swap the inner provider.
///
/// Caches instantiated `IndexerClient`s by provider/config revision while
/// retaining only one compiled generation for each stable provider/config id.
/// The cache is cleared on provider reload.
pub struct DynamicPluginProvider {
    inner: std::sync::RwLock<WasmIndexerPluginProvider>,
    client_cache: IndexerClientCache,
}

impl DynamicPluginProvider {
    pub fn new(provider: WasmIndexerPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn invalidate_provider_keys(&self, provider_keys: &[String]) {
        if provider_keys.is_empty() {
            return;
        }
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.retain(|(provider_type, _, _, _, _), _| !provider_keys.contains(provider_type));
        }
    }

    /// Replace the inner provider. This is called after install/uninstall/toggle.
    pub fn reload(&self, mut new_provider: WasmIndexerPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .expect("DynamicPluginProvider lock poisoned");
        // Reloads rebuild the provider from the plugin set alone; the host
        // services wired in at bootstrap have to survive them.
        new_provider.archive_provider = guard.archive_provider.clone();
        *guard = new_provider;
        // Clear the client cache — WASM bytes may have changed.
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        info!("plugin provider reloaded");
    }
}

impl IndexerPluginProvider for DynamicPluginProvider {
    fn validate_config_for_provider(
        &self,
        provider_type: &str,
        config_json: &str,
    ) -> AppResult<()> {
        self.inner
            .read()
            .expect("DynamicPluginProvider lock poisoned")
            .validate_config_for_provider(provider_type, config_json)
    }

    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        self.client_for_provider_with_proxy(config, None)
    }

    fn client_for_provider_with_proxy(
        &self,
        config: &IndexerConfig,
        proxy_config: Option<&ProxyConfig>,
    ) -> Option<Arc<dyn IndexerClient>> {
        let provider_key = config.provider_type.trim().to_ascii_lowercase();
        let (proxy_id, proxy_revision) = proxy_config
            .map(|config| (config.id.clone(), config.updated_at.to_rfc3339()))
            .unwrap_or_else(|| (String::new(), String::new()));
        let cache_key = (
            provider_key.clone(),
            config.id.clone(),
            config.updated_at.to_rfc3339(),
            proxy_id,
            proxy_revision,
        );

        // Fast path: check cache first
        if let Ok(cache) = self.client_cache.lock()
            && let Some(client) = cache.get(&cache_key)
        {
            return Some(Arc::clone(client));
        }

        // Slow path: compile WASM and cache the result
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        let client = guard.client_for_provider_with_proxy(config, proxy_config)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            return Some(insert_indexer_client_cache(
                &mut cache,
                cache_key,
                Arc::clone(&client),
            ));
        }

        Some(client)
    }

    fn available_provider_types(&self) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.available_provider_types()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        builtin_indexer_provider_types()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_version_for_provider(provider_type)
    }

    fn search_semantics_version_for_provider(&self, provider_type: &str) -> Option<u32> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.search_semantics_version_for_provider(provider_type)
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_sdk_version_for_provider(provider_type)
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_sdk_constraint_for_provider(provider_type)
    }

    fn plugin_type_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_type_for_provider(provider_type)
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.scoring_policies()
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.config_fields_for_provider(provider_type)
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_name_for_provider(provider_type)
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_description_for_provider(provider_type)
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.default_base_url_for_provider(provider_type)
    }

    fn rate_limit_seconds_for_provider(&self, provider_type: &str) -> Option<i64> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.rate_limit_seconds_for_provider(provider_type)
    }

    fn capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.capabilities_for_provider(provider_type)
    }

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_indexer_plugin_provider(
            external_wasm_bytes,
            disabled_builtins,
        ));
        Ok(())
    }

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_indexer_plugin_provider_from_runtime_plugins(
            runtime_plugins,
            disabled_builtins,
        ));
        Ok(())
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicPluginProvider lock poisoned");
            guard.upsert_runtime_plugin_record(plugin)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicPluginProvider lock poisoned");
            guard.remove_provider_type(provider_type)
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicPluginProvider lock poisoned");
            guard.restore_builtin_provider_type(provider_type)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn prepare_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        self.inner
            .read()
            .expect("DynamicPluginProvider lock poisoned")
            .prepare_builtin_provider_type(provider_type)
    }
}

// ── Download client plugin provider ────────────────────────────────────

pub struct WasmDownloadClientPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
}

impl WasmDownloadClientPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
            archive_provider: None,
        }
    }

    /// Give this provider's plugins the host-owned archive-extraction service.
    pub fn with_archive_extractor_provider(
        mut self,
        archive_provider: Arc<dyn ArchiveExtractorPluginProvider>,
    ) -> Self {
        self.archive_provider = Some(archive_provider);
        self
    }

    pub fn with_external_bytes(self, wasm_bytes: &[u8]) -> Self {
        self.with_external_plugin(ExternalPluginWasm {
            bytes: wasm_bytes,
            first_party: false,
        })
    }

    fn prepare_external_plugin_record(
        plugin: ExternalPluginWasm<'_>,
    ) -> Result<LoadedPluginRecord, String> {
        let (descriptor, wasm_bytes) = load_from_bytes(plugin.bytes)?;
        if !validate_descriptor_for_type(
            &descriptor,
            Some("download_client"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("download client descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            descriptor, wasm_bytes,
        )))
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        if !validate_descriptor_for_type(
            &plugin.descriptor,
            Some("download_client"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("download client descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            plugin.descriptor,
            plugin.wasm_bytes,
        )))
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match Self::prepare_external_plugin_record(plugin) {
            Ok(record) => {
                info!(
                    plugin = record.loaded.descriptor.name.as_str(),
                    version = record.loaded.descriptor.version.as_str(),
                    provider_type = record.primary_key.as_str(),
                    "registered external download client plugin"
                );
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load external download client plugin");
            }
        }
        self
    }

    fn with_runtime_plugin(mut self, plugin: RuntimePluginLoad) -> Self {
        match Self::prepare_runtime_plugin_record(plugin) {
            Ok(record) => {
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load runtime download client plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let _ = remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type);
        self
    }

    fn upsert_runtime_plugin_record(
        &mut self,
        plugin: RuntimePluginLoad,
    ) -> Result<Vec<String>, String> {
        let record = Self::prepare_runtime_plugin_record(plugin)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            true,
            true,
        ))
    }

    fn remove_provider_type(&mut self, provider_type: &str) -> Vec<String> {
        remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type)
    }

    fn get_loaded(&self, provider_type: &str) -> Option<&LoadedPlugin> {
        resolve_loaded_plugin(&self.plugins, &self.aliases, provider_type)
    }

    fn create_download_client(
        loaded: &LoadedPlugin,
        config: &DownloadClientConfig,
        archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
        proxy_config: Option<&ProxyConfig>,
    ) -> Option<Arc<dyn DownloadClient>> {
        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                warn!(
                    client = config.name.as_str(),
                    provider_type = config.client_type.as_str(),
                    error = %error,
                    "failed to materialize WASM download client bytes"
                );
                return None;
            }
        };

        let computed_base_url = compute_base_url_from_config_json(&config.config_json);
        let backing = match PluginRuntimeBacking::for_artifact(&loaded.descriptor, &wasm_bytes) {
            Ok(backing) => backing,
            Err(error) => {
                warn!(
                    client = config.name.as_str(),
                    provider_type = config.client_type.as_str(),
                    error = %error,
                    "download client has an invalid runtime marker"
                );
                return None;
            }
        };
        if backing != PluginRuntimeBacking::DownloadClient {
            warn!(
                client = config.name.as_str(),
                provider_type = config.client_type.as_str(),
                "download client selected a runtime that is not valid for this descriptor family"
            );
            return None;
        }

        let mut command_config = std::collections::BTreeMap::new();
        if let Some(base_url) = &computed_base_url {
            command_config.insert("base_url".to_string(), base_url.to_string());
        }
        match parse_config_json_entries(&config.config_json) {
            Ok(map) => command_config.extend(map),
            Err(error) => {
                warn!(
                    client = config.name.as_str(),
                    error = %error,
                    "failed to parse download client config_json"
                );
                return None;
            }
        }
        let allowed_hosts = allowed_hosts_for_descriptor(
            &loaded.descriptor,
            computed_base_url.as_deref(),
            Some(&config.config_json),
        );
        let command_host = crate::wasmtime_host::command_host::CommandHost::for_download_client(
            loaded.descriptor.id.clone(),
            command_config,
            allowed_hosts,
            crate::download_client_adapter::DOWNLOAD_CLIENT_PLUGIN_TIMEOUT,
            None,
            archive_provider,
            proxy_config.map(|proxy_config| crate::plugin_http_host::ProxyPolicy {
                consumer_id: config.id.clone(),
                consumer_name: config.name.clone(),
                config: proxy_config.clone(),
            }),
        );
        Some(Arc::new(WasmDownloadClient::new_component(
            wasm_bytes,
            loaded.descriptor.clone(),
            config.id.clone(),
            config.name.clone(),
            command_host,
        )))
    }
}

impl DownloadClientPluginProvider for WasmDownloadClientPluginProvider {
    fn client_for_config(&self, config: &DownloadClientConfig) -> Option<Arc<dyn DownloadClient>> {
        self.client_for_config_with_proxy(config, None)
    }

    fn client_for_config_with_proxy(
        &self,
        config: &DownloadClientConfig,
        proxy_config: Option<&ProxyConfig>,
    ) -> Option<Arc<dyn DownloadClient>> {
        let provider = config.client_type.trim().to_ascii_lowercase();
        let loaded = self.get_loaded(&provider)?;
        Self::create_download_client(loaded, config, self.archive_provider.clone(), proxy_config)
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.get_loaded(provider_type)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        crate::builtins::builtin_description_for_provider(provider_type).map(str::to_string)
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .and_then(|loaded| loaded.descriptor.default_base_url().map(ToOwned::to_owned))
    }

    fn accepted_inputs_for_provider(&self, provider_type: &str) -> Vec<String> {
        self.get_loaded(provider_type)
            .and_then(|loaded| loaded.descriptor.download_client())
            .map(|download_client| {
                download_client
                    .accepted_inputs
                    .iter()
                    .map(|kind| serde_json::to_value(kind).unwrap_or_default())
                    .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[ExternalPluginWasm<'_>],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        Err("use DynamicDownloadClientPluginProvider for reload".to_string())
    }
}

pub struct DynamicDownloadClientPluginProvider {
    inner: std::sync::RwLock<WasmDownloadClientPluginProvider>,
    client_cache: DownloadClientCache,
}

impl DynamicDownloadClientPluginProvider {
    pub fn new(provider: WasmDownloadClientPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn invalidate_provider_keys(&self, provider_keys: &[String]) {
        if provider_keys.is_empty() {
            return;
        }
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.retain(|(provider_type, _, _, _, _), _| !provider_keys.contains(provider_type));
        }
    }

    pub fn reload(&self, mut new_provider: WasmDownloadClientPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        new_provider.archive_provider = guard.archive_provider.clone();
        *guard = new_provider;
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        info!("download client plugin provider reloaded");
    }
}

impl DownloadClientPluginProvider for DynamicDownloadClientPluginProvider {
    fn client_for_config(&self, config: &DownloadClientConfig) -> Option<Arc<dyn DownloadClient>> {
        self.client_for_config_with_proxy(config, None)
    }

    fn client_for_config_with_proxy(
        &self,
        config: &DownloadClientConfig,
        proxy_config: Option<&ProxyConfig>,
    ) -> Option<Arc<dyn DownloadClient>> {
        let provider_key = config.client_type.trim().to_ascii_lowercase();
        // The proxy revision is part of the key for exactly the reason it is on
        // the indexer side: editing the proxy has to rebuild the client, and
        // unassigning it has to stop serving the proxied build.
        let (proxy_id, proxy_revision) = proxy_config
            .map(|proxy| (proxy.id.clone(), proxy.updated_at.to_rfc3339()))
            .unwrap_or_else(|| (String::new(), String::new()));
        let cache_key = (
            provider_key.clone(),
            config.id.clone(),
            config.updated_at.to_rfc3339(),
            proxy_id,
            proxy_revision,
        );

        if let Ok(cache) = self.client_cache.lock()
            && let Some(client) = cache.get(&cache_key)
        {
            return Some(Arc::clone(client));
        }

        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        let client = guard.client_for_config_with_proxy(config, proxy_config)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            return Some(insert_download_client_cache(
                &mut cache,
                cache_key,
                Arc::clone(&client),
            ));
        }

        Some(client)
    }

    fn available_provider_types(&self) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.available_provider_types()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.plugin_version_for_provider(provider_type)
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.plugin_sdk_version_for_provider(provider_type)
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.plugin_sdk_constraint_for_provider(provider_type)
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.config_fields_for_provider(provider_type)
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.plugin_name_for_provider(provider_type)
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.plugin_description_for_provider(provider_type)
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.default_base_url_for_provider(provider_type)
    }

    fn accepted_inputs_for_provider(&self, provider_type: &str) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.accepted_inputs_for_provider(provider_type)
    }

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_download_client_plugin_provider(
            external_wasm_bytes,
            disabled_builtins,
        ));
        Ok(())
    }

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_download_client_plugin_provider_from_runtime_plugins(
            runtime_plugins,
            disabled_builtins,
        ));
        Ok(())
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicDownloadClientPluginProvider lock poisoned");
            guard.upsert_runtime_plugin_record(plugin)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicDownloadClientPluginProvider lock poisoned");
            guard.remove_provider_type(provider_type)
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }
}

/// Validate a plugin descriptor, optionally filtering by a specific plugin type.
/// If `expected_type` is None, any supported type passes.
fn validate_descriptor_for_type(
    descriptor: &PluginDescriptor,
    expected_type: Option<&str>,
    load_source: PluginLoadSource,
) -> bool {
    if let Err(error) = validate_plugin_descriptor_sdk_contract(descriptor, SDK_VERSION) {
        warn!(
            plugin = descriptor.name.as_str(),
            sdk_version = descriptor.sdk_version.as_str(),
            sdk_constraint = plugin_descriptor_sdk_constraint(descriptor),
            host_sdk_version = SDK_VERSION,
            error = error.as_str(),
            "skipping plugin: incompatible sdk contract"
        );
        return false;
    }

    if let Some(expected) = expected_type
        && descriptor.plugin_type() != expected
    {
        return false;
    }

    for host in descriptor.allowed_hosts() {
        if !allowed_host_pattern_is_valid(host) {
            warn!(
                plugin = descriptor.name.as_str(),
                provider_type = descriptor.provider_type(),
                host,
                "skipping plugin: invalid network permission pattern"
            );
            return false;
        }
    }

    let provider_matches_kind = matches!(
        (descriptor.kind(), &descriptor.provider),
        (PluginKind::Indexer, ProviderDescriptor::Indexer(_))
            | (
                PluginKind::Notification,
                ProviderDescriptor::Notification(_)
            )
            | (
                PluginKind::DownloadClient,
                ProviderDescriptor::DownloadClient(_)
            )
            | (
                PluginKind::SubtitleProvider,
                ProviderDescriptor::Subtitle(_)
            )
            | (
                PluginKind::ArchiveExtractor,
                ProviderDescriptor::ArchiveExtractor(_)
            )
    );
    if !provider_matches_kind {
        warn!(
            plugin = descriptor.name.as_str(),
            plugin_type = descriptor.plugin_type(),
            "skipping plugin: descriptor kind and provider block do not match"
        );
        return false;
    }

    for field in descriptor.config_fields() {
        match field.value_source {
            ConfigFieldValueSource::User => {
                if field.host_binding.is_some() {
                    warn!(
                        plugin = descriptor.name.as_str(),
                        provider_type = descriptor.provider_type(),
                        field_key = field.key.as_str(),
                        "skipping plugin: user-sourced config field must not declare host_binding"
                    );
                    return false;
                }
            }
            ConfigFieldValueSource::HostBinding => {
                let Some(binding) = field.host_binding else {
                    warn!(
                        plugin = descriptor.name.as_str(),
                        provider_type = descriptor.provider_type(),
                        field_key = field.key.as_str(),
                        "skipping plugin: host-binding field must declare host_binding"
                    );
                    return false;
                };

                if !binding_allowed_for_plugin(binding, descriptor, load_source) {
                    warn!(
                        plugin = descriptor.name.as_str(),
                        provider_type = descriptor.provider_type(),
                        binding = binding.as_str(),
                        "skipping plugin: host_binding is not permitted for this plugin"
                    );
                    return false;
                }
            }
        }
    }

    true
}

fn is_indexer_plugin_type(plugin_type: &str) -> bool {
    INDEXER_PLUGIN_TYPES.contains(&plugin_type)
}

fn validate_indexer_descriptor(
    descriptor: &PluginDescriptor,
    load_source: PluginLoadSource,
) -> bool {
    if descriptor.provider_type().eq_ignore_ascii_case("prowlarr") {
        warn!(
            plugin = descriptor.id.as_str(),
            provider_type = descriptor.provider_type(),
            "skipping plugin: prowlarr is reserved for the first-party provider"
        );
        return false;
    }
    validate_descriptor_for_type(descriptor, None, load_source)
        && is_indexer_plugin_type(descriptor.plugin_type())
        && validate_indexer_config_contract(descriptor)
}

fn validate_indexer_config_contract(descriptor: &PluginDescriptor) -> bool {
    if let Some(indexer) = descriptor.indexer() {
        if let Err(error) = crate::newznab_profiles::validate_newznab_profiles(indexer) {
            warn!(
                plugin = descriptor.id.as_str(),
                provider_type = descriptor.provider_type(),
                error = %error,
                "indexer descriptor rejected: invalid provider profile"
            );
            return false;
        }
        let invalid_facets = indexer
            .capabilities
            .supported_query_facets
            .iter()
            .filter(|facet| {
                !scryer_domain::IndexerProviderCapabilities::QUERY_FACETS
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(facet.trim()))
            })
            .cloned()
            .collect::<Vec<_>>();
        if !invalid_facets.is_empty() {
            warn!(
                plugin = descriptor.id.as_str(),
                provider_type = descriptor.provider_type(),
                facets = ?invalid_facets,
                "indexer descriptor rejected: unsupported query facet"
            );
            return false;
        }
    }

    let connection_url_count = descriptor
        .config_fields()
        .iter()
        .filter(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))
        .count();

    let has_valid_connection_url_count = match connection_url_count {
        0 | 1 => true,
        _ => {
            warn!(
                plugin = descriptor.id.as_str(),
                provider_type = descriptor.provider_type(),
                "indexer descriptor rejected: multiple connection_url config field roles"
            );
            false
        }
    };
    if !has_valid_connection_url_count {
        return false;
    }

    true
}

fn default_indexer_connection_url(descriptor: &PluginDescriptor) -> Option<String> {
    descriptor
        .config_fields()
        .iter()
        .find(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))
        .and_then(|field| field.default_value.clone())
        .filter(|value| !value.trim().is_empty())
}

fn binding_allowed_for_plugin(
    binding: SdkHostBinding,
    descriptor: &PluginDescriptor,
    load_source: PluginLoadSource,
) -> bool {
    match binding {
        SdkHostBinding::SmgOpenSubtitlesApiKey => {
            load_source.can_use_first_party_host_bindings()
                && descriptor.plugin_type() == "subtitle_provider"
                && descriptor.id.eq_ignore_ascii_case("opensubtitles")
                && descriptor
                    .provider_type()
                    .eq_ignore_ascii_case("opensubtitles")
        }
    }
}

/// Wire the host-process host for a notification plugin.
///
/// The host-process capability lets a plugin spawn real OS processes on the host
/// and is therefore reserved for Scryer's own first-party plugins. Any plugin
/// that is not first-party (including community and operator-supplied notifiers)
/// receives a disabled host with an empty allowlist, so `scryer_process_exec`
/// always returns PermissionDenied regardless of a self-declared
/// `requires_host_process`. This reuses the same first-party trust gate as
/// first-party host bindings (`can_use_first_party_host_bindings`).
fn process_host_for_notification(loaded: &LoadedPlugin, config_json: &str) -> ProcessHost {
    if loaded.load_source.can_use_first_party_host_bindings() {
        ProcessHost::from_descriptor(&loaded.descriptor, Some(config_json))
    } else {
        ProcessHost::disabled()
    }
}

/// Wire raw socket permissions for first-party notification plugins.
///
/// SMTP-style notification providers need local/LAN sockets, but the grant must
/// come from Scryer's first-party descriptor plus the channel config. Community
/// and operator-supplied plugins receive a disabled host even if they
/// self-declare socket permissions.
fn socket_host_for_notification(loaded: &LoadedPlugin, config_json: &str) -> SocketHost {
    if loaded.load_source.can_use_first_party_host_bindings() {
        SocketHost::from_descriptor(&loaded.descriptor, Some(config_json))
    } else {
        SocketHost::disabled()
    }
}

fn allowed_host_pattern_is_valid(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty()
        || host == "*"
        || host.contains("://")
        || host.contains('/')
        || host.contains('?')
        || host.contains('#')
        || host.contains(':')
    {
        return false;
    }

    if let Some(suffix) = host.strip_prefix("*.") {
        return !suffix.is_empty() && !suffix.contains('*') && url::Host::parse(suffix).is_ok();
    }

    !host.contains('*') && url::Host::parse(host).is_ok()
}

pub fn build_indexer_plugin_provider(
    external_wasm_bytes: &[ExternalPluginWasm<'_>],
    disabled_builtins: &[String],
) -> WasmIndexerPluginProvider {
    let mut provider = WasmIndexerPluginProvider::empty();

    for plugin in external_wasm_bytes {
        provider = provider.with_external_plugin(*plugin);
    }

    for asset in crate::builtins::INDEXER_BUILTINS {
        provider = provider.with_builtin_asset(*asset);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

pub fn build_indexer_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
    disabled_builtins: &[String],
) -> WasmIndexerPluginProvider {
    let mut provider = WasmIndexerPluginProvider::empty();

    for plugin in runtime_plugins.iter().cloned() {
        provider = provider.with_runtime_plugin(plugin);
    }

    for asset in crate::builtins::INDEXER_BUILTINS {
        provider = provider.with_builtin_asset(*asset);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

pub fn build_download_client_plugin_provider(
    external_wasm_bytes: &[ExternalPluginWasm<'_>],
    disabled_builtins: &[String],
) -> WasmDownloadClientPluginProvider {
    let mut provider = WasmDownloadClientPluginProvider::empty();

    for plugin in external_wasm_bytes {
        provider = provider.with_external_plugin(*plugin);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

pub fn build_download_client_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
    disabled_builtins: &[String],
) -> WasmDownloadClientPluginProvider {
    let mut provider = WasmDownloadClientPluginProvider::empty();

    for plugin in runtime_plugins.iter().cloned() {
        provider = provider.with_runtime_plugin(plugin);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

// ── Subtitle provider plugin provider ────────────────────────────────

pub struct WasmSubtitlePluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
}

impl WasmSubtitlePluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
            archive_provider: None,
        }
    }

    pub fn with_archive_extractor_provider(
        mut self,
        archive_provider: Arc<dyn ArchiveExtractorPluginProvider>,
    ) -> Self {
        self.archive_provider = Some(archive_provider);
        self
    }

    pub fn with_external_bytes(self, wasm_bytes: &[u8]) -> Self {
        self.with_external_plugin(ExternalPluginWasm {
            bytes: wasm_bytes,
            first_party: false,
        })
    }

    fn prepare_external_plugin_record(
        plugin: ExternalPluginWasm<'_>,
    ) -> Result<LoadedPluginRecord, String> {
        let (descriptor, wasm_bytes) = load_from_bytes(plugin.bytes)?;
        if !validate_descriptor_for_type(
            &descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("subtitle provider descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            descriptor, wasm_bytes,
        )))
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        if !validate_descriptor_for_type(
            &plugin.descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("subtitle provider descriptor rejected".to_string());
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
        if !validate_descriptor_for_type(
            &descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::Builtin,
        ) {
            return Err("built-in subtitle provider descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_builtin(
            descriptor, asset,
        )))
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match Self::prepare_external_plugin_record(plugin) {
            Ok(record) => {
                info!(
                    plugin = record.loaded.descriptor.name.as_str(),
                    version = record.loaded.descriptor.version.as_str(),
                    provider_type = record.primary_key.as_str(),
                    "registered external subtitle provider plugin"
                );
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load external subtitle provider plugin");
            }
        }
        self
    }

    fn with_runtime_plugin(mut self, plugin: RuntimePluginLoad) -> Self {
        match Self::prepare_runtime_plugin_record(plugin) {
            Ok(record) => {
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load runtime subtitle provider plugin");
            }
        }
        self
    }

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
            Err(error) => {
                warn!(error = %error, "failed to load built-in subtitle provider plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let _ = remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type);
        self
    }

    fn restore_builtin_provider_type(
        &mut self,
        provider_type: &str,
    ) -> Result<Vec<String>, String> {
        let asset = builtin_subtitle_asset_for_provider(provider_type).ok_or_else(|| {
            format!("no built-in subtitle plugin is available for provider '{provider_type}'")
        })?;
        let record = Self::prepare_builtin_asset_record(asset)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            false,
            false,
        ))
    }

    fn prepare_builtin_provider_type(&self, provider_type: &str) -> Result<(), String> {
        let asset = builtin_subtitle_asset_for_provider(provider_type).ok_or_else(|| {
            format!("no built-in subtitle plugin is available for provider '{provider_type}'")
        })?;
        let wasm_bytes = crate::builtins::decode_builtin_wasm(asset)?;
        let (descriptor, _) = load_from_bytes(&wasm_bytes)?;
        if !validate_descriptor_for_type(
            &descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::Builtin,
        ) || !descriptor
            .provider_type()
            .eq_ignore_ascii_case(provider_type)
        {
            return Err("built-in subtitle provider descriptor rejected".to_string());
        }
        Ok(())
    }

    fn upsert_runtime_plugin_record(
        &mut self,
        plugin: RuntimePluginLoad,
    ) -> Result<Vec<String>, String> {
        let record = Self::prepare_runtime_plugin_record(plugin)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            true,
            true,
        ))
    }

    fn remove_provider_type(&mut self, provider_type: &str) -> Vec<String> {
        remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type)
    }

    fn get_loaded(&self, provider_type: &str) -> Option<&LoadedPlugin> {
        resolve_loaded_plugin(&self.plugins, &self.aliases, provider_type)
    }

    /// Build the align client for a loaded subtitle-*sync* plugin.
    ///
    /// Alignment rode its own `SubtitleSyncPluginProcessRequest` on the wasip1
    /// stdin/stdout transport, and when that transport was deleted the
    /// capability was orphaned. It now travels as
    /// `PluginSubtitleCommand::Sync` — the same request type, verbatim, inside
    /// the command envelope the `scryer:subtitle/subtitle-provider@1.0.0`
    /// world's opaque JSON payload already carries — so a sync plugin needs no
    /// world of its own and routes through the ordinary subtitle component
    /// host. See [`crate::subtitle_sync_adapter`].
    fn subtitle_sync_client_from_loaded(
        loaded: &LoadedPlugin,
    ) -> Option<Arc<dyn SubtitleSyncClient>> {
        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                warn!(
                    plugin_id = loaded.descriptor.id.as_str(),
                    provider_type = loaded.descriptor.provider_type(),
                    error = %error,
                    "failed to materialize WASM subtitle sync plugin bytes"
                );
                return None;
            }
        };
        // A pre-component artifact classifies here, so an operator running a
        // stale subtitle-sync build gets the fleet's shared "rebuild against
        // the component ABI" diagnostic at load rather than a missing-import
        // trap an hour into an align job.
        match WasmSubtitleSyncClient::new(wasm_bytes, &loaded.descriptor) {
            Ok(client) => Some(Arc::new(client)),
            Err(error) => {
                warn!(
                    plugin_id = loaded.descriptor.id.as_str(),
                    provider_type = loaded.descriptor.provider_type(),
                    error = %error,
                    "subtitle sync plugin rejected"
                );
                None
            }
        }
    }
}

impl SubtitlePluginProvider for WasmSubtitlePluginProvider {
    fn client_for_config(
        &self,
        config: &SubtitleProviderConfig,
        host_bindings: &HashMap<PluginHostBindingId, String>,
    ) -> Option<Arc<dyn SubtitleProviderClient>> {
        let provider = config.provider_type.trim().to_ascii_lowercase();
        let loaded = self.get_loaded(&provider)?;
        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                warn!(
                    subtitle_provider = config.name.as_str(),
                    provider_type = provider.as_str(),
                    error = %error,
                    "failed to materialize WASM subtitle provider bytes"
                );
                return None;
            }
        };
        match WasmSubtitleClient::new_with_archive_provider(
            wasm_bytes,
            loaded.descriptor.clone(),
            config.clone(),
            host_bindings.clone(),
            self.archive_provider.clone(),
        ) {
            Ok(client) => Some(Arc::new(client)),
            Err(error) => {
                warn!(
                    subtitle_provider = config.name.as_str(),
                    provider_type = provider.as_str(),
                    error = %error,
                    "failed to instantiate WASM subtitle provider plugin"
                );
                None
            }
        }
    }

    fn subtitle_sync_client(&self) -> Option<Arc<dyn SubtitleSyncClient>> {
        self.plugins
            .values()
            .find(|loaded| loaded.descriptor.provider_type() == "enhanced-subtitle-sync")
            .and_then(Self::subtitle_sync_client_from_loaded)
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        builtin_subtitle_provider_types()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn supports_catalog_search_for_provider(&self, provider_type: &str) -> bool {
        self.get_loaded(provider_type)
            .and_then(|loaded| loaded.descriptor.subtitle())
            .is_some_and(|subtitle| subtitle.capabilities.mode == SubtitleProviderMode::Catalog)
    }

    fn recommended_facets_for_provider(&self, provider_type: &str) -> Vec<String> {
        self.get_loaded(provider_type)
            .and_then(|loaded| loaded.descriptor.subtitle())
            .map(|subtitle| subtitle.capabilities.recommended_facets.clone())
            .unwrap_or_default()
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.get_loaded(provider_type)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        crate::builtins::builtin_description_for_provider(provider_type).map(str::to_string)
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[ExternalPluginWasm<'_>],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        Err("use DynamicSubtitlePluginProvider for reload".to_string())
    }
}

pub struct DynamicSubtitlePluginProvider {
    inner: std::sync::RwLock<WasmSubtitlePluginProvider>,
    client_cache: SubtitleClientCache,
}

impl DynamicSubtitlePluginProvider {
    pub fn new(provider: WasmSubtitlePluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn invalidate_provider_keys(&self, provider_keys: &[String]) {
        if provider_keys.is_empty() {
            return;
        }
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.retain(|(provider_type, _, _, _, _), _| !provider_keys.contains(provider_type));
        }
    }

    pub fn reload(&self, mut new_provider: WasmSubtitlePluginProvider) {
        let mut guard = self
            .inner
            .write()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        new_provider.archive_provider = guard.archive_provider.clone();
        *guard = new_provider;
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        info!("subtitle plugin provider reloaded");
    }
}

impl SubtitlePluginProvider for DynamicSubtitlePluginProvider {
    fn client_for_config(
        &self,
        config: &SubtitleProviderConfig,
        host_bindings: &HashMap<PluginHostBindingId, String>,
    ) -> Option<Arc<dyn SubtitleProviderClient>> {
        let provider_key = config.provider_type.trim().to_ascii_lowercase();
        let cache_key = (
            provider_key.clone(),
            config.id.clone(),
            config.updated_at.to_rfc3339(),
            cache_fingerprint(&config.config_json),
            host_binding_cache_key(host_bindings),
        );

        if let Ok(cache) = self.client_cache.lock()
            && let Some(client) = cache.get(&cache_key)
        {
            return Some(Arc::clone(client));
        }

        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        let client = guard.client_for_config(config, host_bindings)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            return Some(insert_subtitle_client_cache(
                &mut cache,
                cache_key,
                Arc::clone(&client),
            ));
        }

        Some(client)
    }

    fn subtitle_sync_client(&self) -> Option<Arc<dyn SubtitleSyncClient>> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.subtitle_sync_client()
    }

    fn available_provider_types(&self) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.available_provider_types()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        builtin_subtitle_provider_types()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.plugin_version_for_provider(provider_type)
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.plugin_sdk_version_for_provider(provider_type)
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.plugin_sdk_constraint_for_provider(provider_type)
    }

    fn supports_catalog_search_for_provider(&self, provider_type: &str) -> bool {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.supports_catalog_search_for_provider(provider_type)
    }

    fn recommended_facets_for_provider(&self, provider_type: &str) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.recommended_facets_for_provider(provider_type)
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.config_fields_for_provider(provider_type)
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.plugin_name_for_provider(provider_type)
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.plugin_description_for_provider(provider_type)
    }

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_subtitle_plugin_provider(
            external_wasm_bytes,
            disabled_builtins,
        ));
        Ok(())
    }

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_subtitle_plugin_provider_from_runtime_plugins(
            runtime_plugins,
            disabled_builtins,
        ));
        Ok(())
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicSubtitlePluginProvider lock poisoned");
            guard.upsert_runtime_plugin_record(plugin)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicSubtitlePluginProvider lock poisoned");
            guard.remove_provider_type(provider_type)
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicSubtitlePluginProvider lock poisoned");
            guard.restore_builtin_provider_type(provider_type)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn prepare_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        self.inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned")
            .prepare_builtin_provider_type(provider_type)
    }
}

fn builtin_subtitle_asset_for_provider(
    provider_type: &str,
) -> Option<crate::builtins::BuiltinPluginAsset> {
    let _ = provider_type;
    None
}

fn host_binding_cache_key(host_bindings: &HashMap<PluginHostBindingId, String>) -> String {
    let mut entries = host_bindings
        .iter()
        .map(|(binding, value)| (binding.as_str(), value))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    cache_fingerprint(
        &entries
            .into_iter()
            .map(|(binding, value)| format!("{binding}={value}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

// ── Archive extractor plugin provider ────────────────────────────────

pub struct WasmArchiveExtractorPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
}

impl WasmArchiveExtractorPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    fn prepare_external_plugin_record(
        plugin: ExternalPluginWasm<'_>,
    ) -> Result<LoadedPluginRecord, String> {
        let (descriptor, wasm_bytes) = load_from_bytes(plugin.bytes)?;
        if !validate_descriptor_for_type(
            &descriptor,
            Some("archive_extractor"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("archive extractor descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            descriptor, wasm_bytes,
        )))
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        if !validate_descriptor_for_type(
            &plugin.descriptor,
            Some("archive_extractor"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("archive extractor descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            plugin.descriptor,
            plugin.wasm_bytes,
        )))
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match Self::prepare_external_plugin_record(plugin) {
            Ok(record) => {
                info!(
                    plugin = record.loaded.descriptor.name.as_str(),
                    version = record.loaded.descriptor.version.as_str(),
                    provider_type = record.primary_key.as_str(),
                    "registered external archive extractor plugin"
                );
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load external archive extractor plugin");
            }
        }
        self
    }

    fn with_runtime_plugin(mut self, plugin: RuntimePluginLoad) -> Self {
        match Self::prepare_runtime_plugin_record(plugin) {
            Ok(record) => {
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load runtime archive extractor plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let _ = remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type);
        self
    }

    fn provider_supports_format(loaded: &LoadedPlugin, format: ArchivePluginFormat) -> bool {
        loaded
            .descriptor
            .archive_extractor()
            .map(|descriptor| descriptor.capabilities.formats.contains(&format))
            .unwrap_or(false)
    }

    fn provider_for_format(&self, format: ArchivePluginFormat) -> Option<&LoadedPlugin> {
        self.plugins
            .values()
            .find(|loaded| Self::provider_supports_format(loaded, format))
    }
}

impl ArchiveExtractorPluginProvider for WasmArchiveExtractorPluginProvider {
    fn client_for_format(
        &self,
        format: ArchivePluginFormat,
    ) -> Option<Arc<dyn ArchiveExtractorClient>> {
        let loaded = self.provider_for_format(format)?;
        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                warn!(
                    format = ?format,
                    error = %error,
                    "failed to materialize WASM archive extractor bytes"
                );
                return None;
            }
        };
        match WasmArchiveExtractorClient::new(wasm_bytes, loaded.descriptor.clone()) {
            Ok(client) => Some(Arc::new(client)),
            Err(error) => {
                warn!(
                    format = ?format,
                    error = %error,
                    "failed to instantiate WASM archive extractor plugin"
                );
                None
            }
        }
    }

    fn available_provider_types(&self) -> Vec<String> {
        let mut keys = self.plugins.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        keys
    }

    fn reload_runtime_plugins(
        &self,
        _runtime_plugins: &[RuntimePluginLoad],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        Err("use DynamicArchiveExtractorPluginProvider for reload".to_string())
    }
}

pub struct DynamicArchiveExtractorPluginProvider {
    inner: std::sync::RwLock<WasmArchiveExtractorPluginProvider>,
    client_cache: ArchiveExtractorClientCache,
}

impl DynamicArchiveExtractorPluginProvider {
    pub fn new(provider: WasmArchiveExtractorPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn reload(&self, new_provider: WasmArchiveExtractorPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = new_provider;
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
    }
}

impl ArchiveExtractorPluginProvider for DynamicArchiveExtractorPluginProvider {
    fn client_for_format(
        &self,
        format: ArchivePluginFormat,
    ) -> Option<Arc<dyn ArchiveExtractorClient>> {
        let cache_key = format!("extract:{format:?}");
        if let Ok(cache) = self.client_cache.lock()
            && let Some(client) = cache.get(&cache_key)
        {
            return Some(Arc::clone(client));
        }

        let client = {
            let guard = self
                .inner
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.client_for_format(format)?
        };

        if let Ok(mut cache) = self.client_cache.lock() {
            cache.insert(cache_key, Arc::clone(&client));
        }
        Some(client)
    }

    fn available_provider_types(&self) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.available_provider_types()
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = std::mem::replace(&mut *guard, WasmArchiveExtractorPluginProvider::empty());
        *guard = current.with_runtime_plugin(plugin);
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = std::mem::replace(&mut *guard, WasmArchiveExtractorPluginProvider::empty());
        *guard = current.without_provider_type(provider_type);
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        Ok(())
    }

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(
            build_archive_extractor_plugin_provider_from_runtime_plugins(
                runtime_plugins,
                disabled_builtins,
            ),
        );
        Ok(())
    }
}

fn cache_fingerprint(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn build_subtitle_plugin_provider(
    external_wasm_bytes: &[ExternalPluginWasm<'_>],
    disabled_builtins: &[String],
) -> WasmSubtitlePluginProvider {
    let mut provider = WasmSubtitlePluginProvider::empty();

    for plugin in external_wasm_bytes {
        provider = provider.with_external_plugin(*plugin);
    }

    for asset in crate::builtins::SUBTITLE_BUILTINS {
        provider = provider.with_builtin_asset(*asset);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

pub fn build_subtitle_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
    disabled_builtins: &[String],
) -> WasmSubtitlePluginProvider {
    let mut provider = WasmSubtitlePluginProvider::empty();

    for plugin in runtime_plugins.iter().cloned() {
        provider = provider.with_runtime_plugin(plugin);
    }

    for asset in crate::builtins::SUBTITLE_BUILTINS {
        provider = provider.with_builtin_asset(*asset);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

pub fn build_archive_extractor_plugin_provider(
    external_wasm_bytes: &[ExternalPluginWasm<'_>],
    disabled_builtins: &[String],
) -> WasmArchiveExtractorPluginProvider {
    let mut provider = WasmArchiveExtractorPluginProvider::empty();

    for plugin in external_wasm_bytes {
        provider = provider.with_external_plugin(*plugin);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

pub fn build_archive_extractor_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
    disabled_builtins: &[String],
) -> WasmArchiveExtractorPluginProvider {
    let mut provider = WasmArchiveExtractorPluginProvider::empty();

    for plugin in runtime_plugins.iter().cloned() {
        provider = provider.with_runtime_plugin(plugin);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

/// Scan `plugins_dir` for subdirectories containing `plugin.wasm`, load each,
/// call `describe()` to get the plugin descriptor, and return a provider that
/// can create indexer clients for any loaded plugin type.
pub fn load_indexer_plugins(plugins_dir: &Path) -> Result<WasmIndexerPluginProvider, String> {
    let mut provider = WasmIndexerPluginProvider::empty();

    let entries = std::fs::read_dir(plugins_dir).map_err(|e| {
        format!(
            "failed to read plugins directory {}: {e}",
            plugins_dir.display()
        )
    })?;

    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }

        let wasm_path = dir.join("plugin.wasm");
        if !wasm_path.exists() {
            continue;
        }

        match load_single_plugin(&wasm_path) {
            Ok((descriptor, wasm_bytes)) => {
                if !validate_indexer_descriptor(
                    &descriptor,
                    PluginLoadSource::External { first_party: false },
                ) {
                    continue;
                }

                let provider_type = descriptor.provider_type().trim().to_ascii_lowercase();

                // Check for duplicates
                if provider.plugins.contains_key(&provider_type) {
                    warn!(
                        plugin = descriptor.name.as_str(),
                        provider_type = provider_type.as_str(),
                        "skipping plugin: duplicate provider_type already loaded"
                    );
                    continue;
                }

                info!(
                    plugin = descriptor.name.as_str(),
                    version = descriptor.version.as_str(),
                    provider_type = provider_type.as_str(),
                    "loaded indexer plugin"
                );

                let record =
                    LoadedPluginRecord::new(LoadedPlugin::from_owned(descriptor, wasm_bytes));
                let _ = insert_loaded_plugin(
                    &mut provider.plugins,
                    &mut provider.aliases,
                    record,
                    true,
                    true,
                );
            }
            Err(e) => {
                warn!(
                    path = %wasm_path.display(),
                    error = %e,
                    "failed to load plugin"
                );
            }
        }
    }

    Ok(provider)
}

fn load_single_plugin(wasm_path: &Path) -> Result<(PluginDescriptor, Vec<u8>), String> {
    let wasm_bytes = std::fs::read(wasm_path)
        .map_err(|e| format!("failed to read {}: {e}", wasm_path.display()))?;

    load_from_bytes(&wasm_bytes)
}

pub(crate) fn parse_config_json_entries(json_str: &str) -> Result<HashMap<String, String>, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(json_str).map_err(|error| error.to_string())?;
    let object = parsed
        .as_object()
        .ok_or_else(|| "config_json must be a JSON object".to_string())?;

    let mut entries = HashMap::with_capacity(object.len());
    for (key, value) in object {
        if value.is_null() {
            continue;
        }

        let normalized = match value {
            serde_json::Value::String(value) => value.trim().to_string(),
            other => other.to_string(),
        };
        entries.insert(key.clone(), normalized);
    }

    Ok(entries)
}

/// Compute a base URL from host/port/use_ssl/url_base in config_json.
fn compute_base_url_from_config_json(json_str: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let host = parsed
        .get("host")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?;
    let port = parsed.get("port").and_then(|v| match v {
        serde_json::Value::String(s) => Some(s.as_str().to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    });
    let use_ssl = parsed
        .get("use_ssl")
        .or_else(|| parsed.get("useSsl"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let url_base = parsed
        .get("url_base")
        .or_else(|| parsed.get("urlBase"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    let protocol = if use_ssl { "https" } else { "http" };
    let mut url = format!("{protocol}://{host}");
    if let Some(p) = port.filter(|p| !p.is_empty()) {
        url.push(':');
        url.push_str(&p);
    }
    if let Some(base) = url_base {
        let normalized = base.trim_start_matches('/');
        if !normalized.is_empty() {
            url.push('/');
            url.push_str(normalized);
        }
    }
    Some(url)
}

/// Build the Extism allowed-hosts list for a plugin manifest.
///
/// The allowed hosts are derived from:
/// 1. The plugin's `allowed_hosts` descriptor field (static declarations).
/// 2. The hostname from `base_url` (indexer plugins).
/// 3. Hostnames from `config_json` values that parse as URLs (notification plugins).
///
/// If the resulting set is empty, no hosts are allowed (plugin has no network access).
pub(crate) fn allowed_hosts_for_descriptor(
    descriptor: &PluginDescriptor,
    base_url: Option<&str>,
    config_json: Option<&str>,
) -> Vec<String> {
    let mut hosts: Vec<String> = descriptor.allowed_hosts().to_vec();

    // Add hostname from base_url (indexer plugins)
    if let Some(url_str) = base_url
        && let Some(host) = host_from_url(url_str)
    {
        hosts.push(host);
    }

    // Add hostnames from config_json values that parse as URLs (notification plugins)
    if let Some(json_str) = config_json
        && let Ok(map) = parse_config_json_entries(json_str)
    {
        for value in map.values() {
            if let Some(host) = host_from_url(value) {
                hosts.push(host);
            }
        }
    }

    hosts
}

fn host_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }

    url::Url::parse(trimmed)
        .ok()
        .and_then(|parsed| parsed.host_str().map(ToOwned::to_owned))
}

/// The `descriptor_source` label for one loaded descriptor.
///
/// The five component worlds are structurally identical — `describe` /
/// `process` over `list<u8>` — so a component built for one family can link and
/// answer against another family's host. The *descriptor* is what discriminates,
/// so the label is derived from it rather than from which `describe` attempt
/// happened to return first; deriving it from the loop position mislabelled the
/// family in the load log.
fn descriptor_source_label(descriptor: &PluginDescriptor) -> &'static str {
    match descriptor.provider {
        ProviderDescriptor::Indexer(_) => "indexer_component",
        ProviderDescriptor::ArchiveExtractor(_) => "archive_component",
        ProviderDescriptor::Subtitle(_) => "subtitle_component",
        ProviderDescriptor::DownloadClient(_) => "download_client_component",
        ProviderDescriptor::Notification(_) => "notification_component",
    }
}

fn load_from_bytes(wasm_bytes: &[u8]) -> Result<(PluginDescriptor, Vec<u8>), String> {
    let started_at = Instant::now();
    module_cache::reset_failed_modules(wasm_bytes);
    let bytes = wasm_bytes.to_vec();

    if let Some(embedded) = embedded_descriptor_from_wasm(&bytes)? {
        // `for_artifact` refuses a pre-component artifact here with the
        // upgrade diagnostic, so a stale install fails at load with an
        // actionable message rather than being silently skipped.
        match PluginRuntimeBacking::for_artifact(&embedded.descriptor, &bytes)? {
            PluginRuntimeBacking::Archive => {
                crate::wasmtime_host::validate_archive_component(&bytes)?;
            }
            PluginRuntimeBacking::Indexer => {
                crate::wasmtime_host::validate_indexer_component(&bytes)?;
            }
            PluginRuntimeBacking::Subtitle => {
                crate::wasmtime_host::validate_subtitle_component(&bytes)?;
            }
            PluginRuntimeBacking::DownloadClient => {
                crate::wasmtime_host::validate_download_client_component(&bytes)?;
            }
            PluginRuntimeBacking::Notification => {
                crate::wasmtime_host::validate_notification_component(&bytes)?;
            }
        }
        let descriptor = embedded.descriptor;
        debug!(
            descriptor_source = "embedded",
            descriptor_load_ms = started_at.elapsed().as_millis() as u64,
            plugin_id = descriptor.id.as_str(),
            "loaded plugin descriptor"
        );
        return Ok((descriptor, bytes));
    }

    if !crate::wasmtime_host::component_host::is_component_binary(&bytes)? {
        // No embedded descriptor and not a component: nothing here can run it,
        // and the family cannot be named because the descriptor is what would
        // have named it. Say what to do anyway.
        return Err(
            "plugins must be WASI Preview 2 components; this artifact is a legacy core \
             wasm module. Upgrade the plugin to a build that targets wasm32-wasip2."
                .to_string(),
        );
    }

    // A component with no embedded descriptor can still self-describe through
    // its world's `describe` export. The worlds are structurally identical, so
    // an attempt against the wrong host can succeed — the returned descriptor,
    // not the attempt order, is what identifies the family.
    for describe in [
        crate::wasmtime_host::archive_component_describe
            as fn(&[u8]) -> Result<PluginDescriptor, String>,
        crate::wasmtime_host::subtitle_component_describe,
        crate::wasmtime_host::download_client_component_describe,
        crate::wasmtime_host::notification_component_describe,
    ] {
        if let Ok(descriptor) = describe(&bytes) {
            debug!(
                descriptor_source = descriptor_source_label(&descriptor),
                descriptor_load_ms = started_at.elapsed().as_millis() as u64,
                plugin_id = descriptor.id.as_str(),
                "loaded plugin descriptor"
            );
            return Ok((descriptor, bytes));
        }
    }
    Err("WASI Preview 2 indexer components must embed a top-level plugin descriptor".to_string())
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WasmPluginDescriptorLoader;

impl PluginDescriptorLoader for WasmPluginDescriptorLoader {
    fn load_descriptor_from_wasm_bytes(&self, wasm_bytes: &[u8]) -> AppResult<PluginDescriptor> {
        load_from_bytes(wasm_bytes)
            .map(|(descriptor, _)| descriptor)
            .map_err(AppError::Validation)
    }
}

// ── Notification plugin provider ───────────────────────────────────────

pub struct WasmNotificationPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
}

impl WasmNotificationPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
            archive_provider: None,
        }
    }

    /// Give this provider's plugins the host-owned archive-extraction service.
    pub fn with_archive_extractor_provider(
        mut self,
        archive_provider: Arc<dyn ArchiveExtractorPluginProvider>,
    ) -> Self {
        self.archive_provider = Some(archive_provider);
        self
    }

    pub fn with_external_bytes(self, wasm_bytes: &[u8]) -> Self {
        self.with_external_plugin(ExternalPluginWasm {
            bytes: wasm_bytes,
            first_party: false,
        })
    }

    fn prepare_external_plugin_record(
        plugin: ExternalPluginWasm<'_>,
    ) -> Result<LoadedPluginRecord, String> {
        let (descriptor, wasm_bytes) = load_from_bytes(plugin.bytes)?;
        let load_source = PluginLoadSource::External {
            first_party: plugin.first_party,
        };
        if !validate_descriptor_for_type(&descriptor, Some("notification"), load_source) {
            return Err("notification descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(
            LoadedPlugin::from_owned(descriptor, wasm_bytes).with_load_source(load_source),
        ))
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        let load_source = PluginLoadSource::External {
            first_party: plugin.first_party,
        };
        if !validate_descriptor_for_type(&plugin.descriptor, Some("notification"), load_source) {
            return Err("notification descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(
            LoadedPlugin::from_owned(plugin.descriptor, plugin.wasm_bytes)
                .with_load_source(load_source),
        ))
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match Self::prepare_external_plugin_record(plugin) {
            Ok(record) => {
                info!(
                    plugin = record.loaded.descriptor.name.as_str(),
                    version = record.loaded.descriptor.version.as_str(),
                    provider_type = record.primary_key.as_str(),
                    "registered external notification plugin"
                );
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load external notification plugin");
            }
        }
        self
    }

    fn with_runtime_plugin(mut self, plugin: RuntimePluginLoad) -> Self {
        match Self::prepare_runtime_plugin_record(plugin) {
            Ok(record) => {
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load runtime notification plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let _ = remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type);
        self
    }

    fn upsert_runtime_plugin_record(
        &mut self,
        plugin: RuntimePluginLoad,
    ) -> Result<Vec<String>, String> {
        let record = Self::prepare_runtime_plugin_record(plugin)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            true,
            true,
        ))
    }

    fn remove_provider_type(&mut self, provider_type: &str) -> Vec<String> {
        remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type)
    }

    fn get_loaded(&self, provider_type: &str) -> Option<&LoadedPlugin> {
        resolve_loaded_plugin(&self.plugins, &self.aliases, provider_type)
    }

    fn create_notification_client(
        loaded: &LoadedPlugin,
        config: &NotificationChannelConfig,
        archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
    ) -> Option<Arc<dyn NotificationClient>> {
        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                warn!(
                    channel = config.name.as_str(),
                    error = %error,
                    "failed to materialize WASM notification plugin bytes"
                );
                return None;
            }
        };

        // Classify from the artifact, not the descriptor: a notification
        // descriptor is identical whether the artifact is a legacy reactor or a
        // `scryer:notification/notification@1.0.0` component.
        let backing = match PluginRuntimeBacking::for_artifact(&loaded.descriptor, &wasm_bytes) {
            Ok(backing) => backing,
            Err(error) => {
                warn!(
                    channel = config.name.as_str(),
                    provider_type = config.channel_type.as_str(),
                    error = %error,
                    "notification channel has an invalid runtime marker"
                );
                return None;
            }
        };

        if backing != PluginRuntimeBacking::Notification {
            warn!(
                channel = config.name.as_str(),
                provider_type = config.channel_type.as_str(),
                "notification channel selected a runtime that is not valid for this descriptor family"
            );
            return None;
        }

        // ONE construction. The socket and process grants, the allowed hosts,
        // the timeout, the config map and the archive-provider `CommandHost`
        // are decided here, once; `socket_host` reaches both the service layer
        // (through the `CommandHost`) and the adapter's per-send cleanup, and
        // because it is an `Arc` clone over shared state, both see the same
        // resolved permission set and the same socket handle table.
        let allowed_hosts =
            allowed_hosts_for_descriptor(&loaded.descriptor, None, Some(&config.config_json));
        let timeout = crate::notification_adapter::NOTIFICATION_PLUGIN_TIMEOUT;
        let socket_host = socket_host_for_notification(loaded, &config.config_json);
        let process_host = process_host_for_notification(loaded, &config.config_json);

        let mut channel_config = std::collections::BTreeMap::new();
        match parse_config_json_entries(&config.config_json) {
            Ok(map) => channel_config.extend(map),
            Err(error) => {
                warn!(
                    channel = config.name.as_str(),
                    error = %error,
                    "failed to parse notification channel config_json"
                );
            }
        }

        let command_host = crate::wasmtime_host::command_host::CommandHost::for_notification(
            loaded.descriptor.id.clone(),
            channel_config,
            allowed_hosts,
            timeout,
            None,
            archive_provider,
            socket_host.clone(),
            process_host,
        );

        Some(Arc::new(WasmNotificationClient::new_component(
            PluginInstanceSpec {
                wasm: Arc::new(wasm_bytes),
                // This family has never been granted filesystem authority.
                preopens: Vec::new(),
                timeout,
                memory_max_bytes: None,
                command_host,
            },
            loaded.descriptor.clone(),
            config.name.clone(),
            Some(socket_host),
        )))
    }
}

impl NotificationPluginProvider for WasmNotificationPluginProvider {
    fn client_for_channel(
        &self,
        config: &NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        let provider = config.channel_type.as_str().to_ascii_lowercase();
        let loaded = self.get_loaded(&provider)?;
        Self::create_notification_client(loaded, config, self.archive_provider.clone())
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.get_loaded(provider_type)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        crate::builtins::builtin_description_for_provider(provider_type).map(str::to_string)
    }

    fn supported_events_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::NotificationEventType> {
        self.get_loaded(provider_type)
            .map(notification_supported_events_from_loaded)
            .unwrap_or_default()
    }

    fn supports_test_for_provider(&self, provider_type: &str) -> bool {
        self.get_loaded(provider_type)
            .map(notification_supports_test_from_loaded)
            .unwrap_or(false)
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[ExternalPluginWasm<'_>],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        Err("use DynamicNotificationPluginProvider for reload".to_string())
    }
}

/// Thread-safe wrapper around `WasmNotificationPluginProvider` that supports runtime reload.
pub struct DynamicNotificationPluginProvider {
    inner: std::sync::RwLock<WasmNotificationPluginProvider>,
    client_cache: NotificationClientCache,
}

impl DynamicNotificationPluginProvider {
    pub fn new(provider: WasmNotificationPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn invalidate_provider_keys(&self, provider_keys: &[String]) {
        if provider_keys.is_empty() {
            return;
        }
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.retain(|(provider_type, _, _), _| !provider_keys.contains(provider_type));
        }
    }

    pub fn reload(&self, mut new_provider: WasmNotificationPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        new_provider.archive_provider = guard.archive_provider.clone();
        *guard = new_provider;
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        info!("notification plugin provider reloaded");
    }
}

impl NotificationPluginProvider for DynamicNotificationPluginProvider {
    fn client_for_channel(
        &self,
        config: &NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        let provider_key = config.channel_type.as_str().to_ascii_lowercase();
        let cache_key = (
            provider_key.clone(),
            config.id.clone(),
            config.updated_at.to_rfc3339(),
        );

        if let Ok(cache) = self.client_cache.lock()
            && let Some(client) = cache.get(&cache_key)
        {
            return Some(Arc::clone(client));
        }

        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        let client = guard.client_for_channel(config)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            return Some(insert_notification_client_cache(
                &mut cache,
                cache_key,
                Arc::clone(&client),
            ));
        }

        Some(client)
    }

    fn available_provider_types(&self) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.available_provider_types()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.plugin_version_for_provider(provider_type)
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.plugin_sdk_version_for_provider(provider_type)
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.plugin_sdk_constraint_for_provider(provider_type)
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.config_fields_for_provider(provider_type)
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.plugin_name_for_provider(provider_type)
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.plugin_description_for_provider(provider_type)
    }

    fn supported_events_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::NotificationEventType> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.supported_events_for_provider(provider_type)
    }

    fn supports_test_for_provider(&self, provider_type: &str) -> bool {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.supports_test_for_provider(provider_type)
    }

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_notification_plugin_provider(
            external_wasm_bytes,
            disabled_builtins,
        ));
        Ok(())
    }

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_notification_plugin_provider_from_runtime_plugins(
            runtime_plugins,
            disabled_builtins,
        ));
        Ok(())
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicNotificationPluginProvider lock poisoned");
            guard.upsert_runtime_plugin_record(plugin)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicNotificationPluginProvider lock poisoned");
            guard.remove_provider_type(provider_type)
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }
}

pub fn build_notification_plugin_provider(
    external_wasm_bytes: &[ExternalPluginWasm<'_>],
    disabled_builtins: &[String],
) -> WasmNotificationPluginProvider {
    let mut provider = WasmNotificationPluginProvider::empty();

    for plugin in external_wasm_bytes {
        provider = provider.with_external_plugin(*plugin);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

pub fn build_notification_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
    disabled_builtins: &[String],
) -> WasmNotificationPluginProvider {
    let mut provider = WasmNotificationPluginProvider::empty();

    for plugin in runtime_plugins.iter().cloned() {
        provider = provider.with_runtime_plugin(plugin);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

fn notification_supported_events_from_loaded(
    loaded: &LoadedPlugin,
) -> Vec<scryer_domain::NotificationEventType> {
    loaded
        .descriptor
        .notification()
        .map(|notification| {
            notification
                .capabilities
                .supported_events
                .iter()
                .filter_map(|event| scryer_domain::NotificationEventType::parse(event.as_str()))
                .collect()
        })
        .unwrap_or_default()
}

fn notification_supports_test_from_loaded(loaded: &LoadedPlugin) -> bool {
    loaded
        .descriptor
        .notification()
        .map(|notification| notification.capabilities.supports_test)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::{NEWZNAB, TORZNAB};
    use crate::types::{
        ArchiveExtractorCapabilities, ArchiveExtractorDescriptor, ArchivePluginFormat,
        ConfigFieldDef, ConfigFieldType, ConfigFieldValueSource, DownloadClientCapabilities,
        DownloadClientDescriptor, IndexerDescriptor, IndexerSourceKind, NotificationCapabilities,
        NotificationDescriptor, PluginHostBindingId, SubtitleCapabilities, SubtitleDescriptor,
    };

    struct DummyIndexerClient;

    #[async_trait::async_trait]
    impl IndexerClient for DummyIndexerClient {
        async fn search(
            &self,
            _query: String,
            _ids: std::collections::HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<scryer_application::IndexerRoutingPlan>,
            _mode: scryer_application::SearchMode,
            _operation: scryer_application::IndexerErrorOperation,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _year: Option<i32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            _learning_context: Option<scryer_application::IndexerSearchLearningContext>,
            _cancel_token: tokio_util::sync::CancellationToken,
        ) -> scryer_application::AppResult<scryer_application::IndexerSearchResponse> {
            unreachable!("dummy indexer client should not be called")
        }
    }

    fn indexer_cache_key(
        provider_type: &str,
        config_id: &str,
        revision: &str,
    ) -> IndexerClientCacheKey {
        (
            provider_type.to_string(),
            config_id.to_string(),
            revision.to_string(),
            String::new(),
            String::new(),
        )
    }

    fn download_cache_key(
        provider_type: &str,
        config_id: &str,
        revision: &str,
    ) -> DownloadClientCacheKey {
        (
            provider_type.to_string(),
            config_id.to_string(),
            revision.to_string(),
            String::new(),
            String::new(),
        )
    }

    struct DummyDownloadClient;

    #[async_trait::async_trait]
    impl DownloadClient for DummyDownloadClient {
        async fn submit_download(
            &self,
            _request: &scryer_application::DownloadClientAddRequest,
        ) -> scryer_application::AppResult<scryer_application::DownloadGrabResult> {
            unreachable!("dummy download client should not be called")
        }
    }

    struct DummyNotificationClient;

    #[async_trait::async_trait]
    impl NotificationClient for DummyNotificationClient {
        async fn send_notification(
            &self,
            _payload: &scryer_application::NotificationPayload,
        ) -> scryer_application::AppResult<()> {
            unreachable!("dummy notification client should not be called")
        }
    }

    struct DummySubtitleProviderClient;

    #[async_trait::async_trait]
    impl SubtitleProviderClient for DummySubtitleProviderClient {
        async fn search(
            &self,
            _query: &scryer_application::subtitles::SubtitleQuery,
        ) -> scryer_application::AppResult<Vec<scryer_application::subtitles::SubtitleMatch>>
        {
            unreachable!("dummy subtitle provider client should not be called")
        }

        async fn download(
            &self,
            _provider_file_id: &str,
        ) -> scryer_application::AppResult<scryer_application::subtitles::SubtitleFile> {
            unreachable!("dummy subtitle provider client should not be called")
        }

        async fn validate_connection(
            &self,
        ) -> scryer_application::AppResult<scryer_application::SubtitleProviderValidationResult>
        {
            unreachable!("dummy subtitle provider client should not be called")
        }

        fn name(&self) -> &str {
            "dummy-subtitle-provider"
        }
    }

    fn indexer_config_fields() -> Vec<ConfigFieldDef> {
        vec![ConfigFieldDef {
            key: "base_url".to_string(),
            label: "Base URL".to_string(),
            field_type: ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::User,
            role: Some(ConfigFieldRole::ConnectionUrl),
            host_binding: None,
            options: vec![],
            help_text: None,
        }]
    }

    fn descriptor(plugin_type: &str) -> PluginDescriptor {
        let provider = match plugin_type {
            "indexer" => ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                provider_profiles: vec![],
                source_kind: IndexerSourceKind::Generic,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: indexer_config_fields(),
                allowed_hosts: vec![],
                rate_limit_seconds: None,
                search_semantics_version: Some(1),
                strategy_plan: None,
            }),
            "usenet_indexer" => ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                provider_profiles: vec![],
                source_kind: IndexerSourceKind::Usenet,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: indexer_config_fields(),
                allowed_hosts: vec![],
                rate_limit_seconds: None,
                search_semantics_version: Some(1),
                strategy_plan: None,
            }),
            "torrent_indexer" => ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                provider_profiles: vec![],
                source_kind: IndexerSourceKind::Torrent,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: indexer_config_fields(),
                allowed_hosts: vec![],
                rate_limit_seconds: None,
                search_semantics_version: Some(1),
                strategy_plan: None,
            }),
            "notification" => ProviderDescriptor::Notification(NotificationDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                capabilities: NotificationCapabilities::default(),
            }),
            "download_client" => ProviderDescriptor::DownloadClient(DownloadClientDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                accepted_inputs: vec![],
                isolation_modes: vec![],
                capabilities: DownloadClientCapabilities::default(),
            }),
            "subtitle_provider" => ProviderDescriptor::Subtitle(SubtitleDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                capabilities: SubtitleCapabilities::default(),
            }),
            "archive_extractor" => {
                ProviderDescriptor::ArchiveExtractor(ArchiveExtractorDescriptor {
                    provider_type: "test".to_string(),
                    provider_aliases: vec![],
                    config_fields: vec![],
                    default_base_url: None,
                    allowed_hosts: vec![],
                    capabilities: ArchiveExtractorCapabilities {
                        formats: vec![
                            ArchivePluginFormat::Rar,
                            ArchivePluginFormat::SevenZip,
                            ArchivePluginFormat::Zip,
                        ],
                    },
                })
            }
            other => panic!("unsupported test plugin type: {other}"),
        };

        PluginDescriptor {
            id: "test".to_string(),
            name: "Test".to_string(),
            version: "0.1.0".to_string(),
            sdk_version: SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions: vec![],
            provider,
        }
    }

    fn set_provider_type(descriptor: &mut PluginDescriptor, provider_type: &str) {
        match &mut descriptor.provider {
            ProviderDescriptor::Indexer(provider) => {
                provider.provider_type = provider_type.to_string()
            }
            ProviderDescriptor::Notification(provider) => {
                provider.provider_type = provider_type.to_string()
            }
            ProviderDescriptor::DownloadClient(provider) => {
                provider.provider_type = provider_type.to_string()
            }
            ProviderDescriptor::Subtitle(provider) => {
                provider.provider_type = provider_type.to_string()
            }
            ProviderDescriptor::ArchiveExtractor(provider) => {
                provider.provider_type = provider_type.to_string()
            }
        }
    }

    fn set_provider_aliases(descriptor: &mut PluginDescriptor, aliases: Vec<String>) {
        match &mut descriptor.provider {
            ProviderDescriptor::Indexer(provider) => provider.provider_aliases = aliases,
            ProviderDescriptor::Notification(provider) => provider.provider_aliases = aliases,
            ProviderDescriptor::DownloadClient(provider) => provider.provider_aliases = aliases,
            ProviderDescriptor::Subtitle(provider) => provider.provider_aliases = aliases,
            ProviderDescriptor::ArchiveExtractor(provider) => provider.provider_aliases = aliases,
        }
    }

    fn set_allowed_hosts(descriptor: &mut PluginDescriptor, allowed_hosts: Vec<String>) {
        match &mut descriptor.provider {
            ProviderDescriptor::Indexer(provider) => provider.allowed_hosts = allowed_hosts,
            ProviderDescriptor::Notification(provider) => provider.allowed_hosts = allowed_hosts,
            ProviderDescriptor::DownloadClient(provider) => provider.allowed_hosts = allowed_hosts,
            ProviderDescriptor::Subtitle(provider) => provider.allowed_hosts = allowed_hosts,
            ProviderDescriptor::ArchiveExtractor(provider) => {
                provider.allowed_hosts = allowed_hosts
            }
        }
    }

    /// The embedded descriptor short-circuits guest describe execution: the
    /// fixture component self-describes as something else through its own
    /// `describe` export, so the loaded descriptor can only have come from the
    /// custom section.
    #[test]
    fn embedded_descriptor_avoids_guest_descriptor_execution() {
        fn encode_u32_leb(mut value: u32, output: &mut Vec<u8>) {
            loop {
                let mut byte = (value & 0x7f) as u8;
                value >>= 7;
                if value != 0 {
                    byte |= 0x80;
                }
                output.push(byte);
                if value == 0 {
                    break;
                }
            }
        }

        let mut wasm = crate::wasmtime_host::subtitle_component_host::tests::fixture_component();
        let embedded = descriptor("subtitle_provider");
        let descriptor_json = serde_json::to_vec(&embedded).unwrap();
        let section_name = crate::embedded_descriptor::PLUGIN_DESCRIPTOR_CUSTOM_SECTION_V1;
        let mut section = Vec::new();
        encode_u32_leb(section_name.len() as u32, &mut section);
        section.extend_from_slice(section_name.as_bytes());
        section.extend_from_slice(&descriptor_json);
        wasm.push(0);
        encode_u32_leb(section.len() as u32, &mut wasm);
        wasm.extend_from_slice(&section);

        let (loaded_descriptor, loaded_wasm) = load_from_bytes(&wasm).unwrap();

        assert_eq!(
            serde_json::to_value(loaded_descriptor).unwrap(),
            serde_json::to_value(embedded).unwrap()
        );
        assert_eq!(loaded_wasm, wasm);
    }

    /// The hard cut at the loader's own door: a pre-component artifact — even
    /// one carrying a valid embedded descriptor — is refused with the upgrade
    /// instruction rather than being loaded and failed later.
    #[test]
    fn a_core_module_artifact_is_refused_with_an_upgrade_diagnostic() {
        let core_module = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "scryer_describe") (result i32) unreachable))"#,
        )
        .unwrap();

        let error = load_from_bytes(&core_module).expect_err("a core module must not load");

        assert!(error.contains("wasm32-wasip2"), "got: {error}");
        assert!(error.contains("Upgrade the plugin"), "got: {error}");
    }

    fn runtime_plugin_load(
        plugin_type: &str,
        provider_type: &str,
        aliases: &[&str],
    ) -> RuntimePluginLoad {
        let mut descriptor = descriptor(plugin_type);
        descriptor.id = provider_type.to_string();
        descriptor.name = format!("{provider_type} plugin");
        descriptor.sdk_version = SDK_VERSION.to_string();
        descriptor.sdk_constraint = scryer_plugin_sdk::current_sdk_constraint();
        set_provider_type(&mut descriptor, provider_type);
        set_provider_aliases(
            &mut descriptor,
            aliases.iter().map(|alias| (*alias).to_string()).collect(),
        );

        RuntimePluginLoad {
            descriptor,
            wasm_bytes: provider_type.as_bytes().to_vec(),
            first_party: true,
        }
    }

    #[test]
    fn notification_process_host_gated_to_first_party() {
        let mut descriptor = descriptor("notification");
        if let ProviderDescriptor::Notification(provider) = &mut descriptor.provider {
            provider.capabilities.requires_host_process = true;
        }
        let config_json = r#"{"path":"/usr/bin/env"}"#;

        let first_party = LoadedPlugin::from_owned(descriptor.clone(), Vec::new())
            .with_load_source(PluginLoadSource::External { first_party: true });
        let builtin = LoadedPlugin::from_owned(descriptor.clone(), Vec::new())
            .with_load_source(PluginLoadSource::Builtin);
        let untrusted = LoadedPlugin::from_owned(descriptor, Vec::new())
            .with_load_source(PluginLoadSource::External { first_party: false });

        // First-party and builtin notifiers keep a working, non-empty allowlist.
        assert!(
            process_host_for_notification(&first_party, config_json).allowed_command_count() > 0
        );
        assert!(process_host_for_notification(&builtin, config_json).allowed_command_count() > 0);
        // Operator-supplied (Unverified) notifiers get a disabled host even though
        // the descriptor self-declares `requires_host_process`.
        assert_eq!(
            process_host_for_notification(&untrusted, config_json).allowed_command_count(),
            0
        );
    }

    #[test]
    fn embedded_builtin_descriptors_match_current_sdk_line() {
        for asset in [NEWZNAB, TORZNAB] {
            let descriptor: PluginDescriptor = serde_json::from_str(asset.descriptor_json)
                .expect("embedded builtin descriptor should parse");
            validate_plugin_descriptor_sdk_contract(&descriptor, SDK_VERSION)
                .expect("embedded builtin should match current SDK line");
        }
    }

    #[test]
    fn builtin_records_keep_embedded_assets_until_materialized() {
        let record = WasmIndexerPluginProvider::prepare_builtin_asset_record(NEWZNAB)
            .expect("builtin loads");
        assert!(record.loaded.stores_builtin_asset());

        let first = record
            .loaded
            .materialize_wasm()
            .expect("builtin should decode on demand");
        let second = record
            .loaded
            .materialize_wasm()
            .expect("builtin should decode on repeated access");

        assert!(!first.is_empty());
        assert_eq!(first, second);
    }

    #[test]
    fn builtin_providers_expose_embedded_plugins() {
        let indexers = build_indexer_plugin_provider(&[], &[]);
        for provider_type in ["newznab", "torznab"] {
            assert!(
                indexers.plugin_name_for_provider(provider_type).is_some(),
                "expected builtin indexer provider '{provider_type}' to be available"
            );
        }
        assert!(
            build_subtitle_plugin_provider(&[], &[])
                .builtin_provider_types()
                .is_empty()
        );
    }

    #[test]
    fn indexer_family_types_are_accepted() {
        assert!(validate_indexer_descriptor(
            &descriptor("indexer"),
            PluginLoadSource::External { first_party: false }
        ));
        assert!(validate_indexer_descriptor(
            &descriptor("usenet_indexer"),
            PluginLoadSource::External { first_party: false }
        ));
        assert!(validate_indexer_descriptor(
            &descriptor("torrent_indexer"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn indexer_query_facets_must_be_known_search_facets() {
        let mut descriptor = descriptor("indexer");
        if let ProviderDescriptor::Indexer(indexer) = &mut descriptor.provider {
            indexer.capabilities.supported_query_facets =
                vec!["movie".to_string(), "music".to_string()];
        } else {
            unreachable!("test descriptor should be an indexer");
        }

        assert!(!validate_indexer_descriptor(
            &descriptor,
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn non_indexer_types_are_rejected_for_indexer_provider() {
        assert!(!validate_indexer_descriptor(
            &descriptor("notification"),
            PluginLoadSource::External { first_party: false }
        ));
        assert!(!validate_indexer_descriptor(
            &descriptor("download_client"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn provider_type_collision_is_allowed_across_plugin_families() {
        let mut indexer = descriptor("indexer");
        set_provider_type(&mut indexer, "example_provider");

        let mut subtitle = descriptor("subtitle_provider");
        set_provider_type(&mut subtitle, "example_provider");

        assert!(validate_indexer_descriptor(
            &indexer,
            PluginLoadSource::External { first_party: false }
        ));
        assert!(validate_descriptor_for_type(
            &subtitle,
            Some("subtitle_provider"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn indexers_without_connection_fields_are_accepted() {
        let mut descriptor = descriptor("usenet_indexer");

        let ProviderDescriptor::Indexer(indexer) = &mut descriptor.provider else {
            panic!("expected indexer descriptor");
        };
        indexer.config_fields.clear();

        assert!(validate_indexer_descriptor(
            &descriptor,
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn indexers_with_multiple_connection_fields_are_rejected() {
        let mut descriptor = descriptor("usenet_indexer");

        let ProviderDescriptor::Indexer(indexer) = &mut descriptor.provider else {
            panic!("expected indexer descriptor");
        };
        indexer.config_fields.push(ConfigFieldDef {
            key: "alternate_url".to_string(),
            label: "Alternate URL".to_string(),
            field_type: ConfigFieldType::String,
            required: false,
            default_value: None,
            value_source: ConfigFieldValueSource::User,
            role: Some(ConfigFieldRole::ConnectionUrl),
            host_binding: None,
            options: vec![],
            help_text: None,
        });

        assert!(!validate_indexer_descriptor(
            &descriptor,
            PluginLoadSource::External { first_party: false }
        ));
    }

    fn indexer_config_field(key: &str, role: Option<ConfigFieldRole>) -> ConfigFieldDef {
        ConfigFieldDef {
            key: key.to_string(),
            label: key.to_string(),
            field_type: ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::User,
            role,
            host_binding: None,
            options: vec![],
            help_text: None,
        }
    }

    /// An indexer may own its endpoint internally and expose no
    /// `connection_url` field at all ("config-free" to the user); a URL-typed
    /// field without the role is not a contract violation.
    #[test]
    fn indexer_without_connection_url_role_is_accepted_as_url_less() {
        let mut descriptor = descriptor("torrent_indexer");

        let ProviderDescriptor::Indexer(indexer) = &mut descriptor.provider else {
            panic!("expected indexer descriptor");
        };
        indexer.config_fields = vec![indexer_config_field("feed_url", None)];

        assert!(validate_indexer_descriptor(
            &descriptor,
            PluginLoadSource::External { first_party: false }
        ));
    }

    /// Exactly one connection URL may drive host-side routing; two is ambiguous.
    #[test]
    fn indexer_with_multiple_connection_url_roles_is_rejected() {
        let mut descriptor = descriptor("torrent_indexer");

        let ProviderDescriptor::Indexer(indexer) = &mut descriptor.provider else {
            panic!("expected indexer descriptor");
        };
        indexer.config_fields = vec![
            indexer_config_field("base_url", Some(ConfigFieldRole::ConnectionUrl)),
            indexer_config_field("mirror_url", Some(ConfigFieldRole::ConnectionUrl)),
        ];

        assert!(!validate_indexer_descriptor(
            &descriptor,
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn subtitle_provider_rejects_notification_expected_type() {
        let descriptor = descriptor("subtitle_provider");
        assert!(!validate_descriptor_for_type(
            &descriptor,
            Some("notification"),
            PluginLoadSource::Builtin
        ));
    }

    #[test]
    fn constrained_allowed_host_glob_is_accepted() {
        let mut descriptor = descriptor("subtitle_provider");
        set_allowed_hosts(&mut descriptor, vec!["*.opensubtitles.com".to_string()]);

        assert!(validate_descriptor_for_type(
            &descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn malformed_allowed_host_patterns_are_rejected() {
        for pattern in [
            "*",
            "http://*.example.com",
            "*.*.example.com",
            "foo*bar.com",
            "example.com/path",
            "example.com:443",
        ] {
            let mut descriptor = descriptor("subtitle_provider");
            set_allowed_hosts(&mut descriptor, vec![pattern.to_string()]);
            assert!(
                !validate_descriptor_for_type(
                    &descriptor,
                    Some("subtitle_provider"),
                    PluginLoadSource::External { first_party: false }
                ),
                "pattern should be rejected: {pattern}"
            );
        }
    }

    #[test]
    fn official_external_opensubtitles_plugin_may_request_api_key_binding() {
        let mut descriptor = descriptor("subtitle_provider");
        descriptor.id = "opensubtitles".to_string();
        set_provider_type(&mut descriptor, "opensubtitles");
        let ProviderDescriptor::Subtitle(subtitle) = &mut descriptor.provider else {
            panic!("expected subtitle descriptor");
        };
        subtitle.config_fields = vec![ConfigFieldDef {
            key: "api_key".to_string(),
            label: "API Key".to_string(),
            field_type: ConfigFieldType::Password,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::HostBinding,
            role: None,
            host_binding: Some(PluginHostBindingId::SmgOpenSubtitlesApiKey),
            options: vec![],
            help_text: None,
        }];

        assert!(validate_descriptor_for_type(
            &descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::External { first_party: true }
        ));
    }

    #[test]
    fn non_official_external_plugins_cannot_request_opensubtitles_api_key_binding() {
        let mut descriptor = descriptor("subtitle_provider");
        descriptor.id = "opensubtitles".to_string();
        set_provider_type(&mut descriptor, "opensubtitles");
        let ProviderDescriptor::Subtitle(subtitle) = &mut descriptor.provider else {
            panic!("expected subtitle descriptor");
        };
        subtitle.config_fields = vec![ConfigFieldDef {
            key: "api_key".to_string(),
            label: "API Key".to_string(),
            field_type: ConfigFieldType::Password,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::HostBinding,
            role: None,
            host_binding: Some(PluginHostBindingId::SmgOpenSubtitlesApiKey),
            options: vec![],
            help_text: None,
        }];

        assert!(!validate_descriptor_for_type(
            &descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn non_subtitle_plugins_cannot_request_subtitle_host_bindings() {
        let mut descriptor = descriptor("notification");
        let ProviderDescriptor::Notification(notification) = &mut descriptor.provider else {
            panic!("expected notification descriptor");
        };
        notification.config_fields = vec![ConfigFieldDef {
            key: "api_key".to_string(),
            label: "API Key".to_string(),
            field_type: ConfigFieldType::Password,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::HostBinding,
            role: None,
            host_binding: Some(PluginHostBindingId::SmgOpenSubtitlesApiKey),
            options: vec![],
            help_text: None,
        }];

        assert!(!validate_descriptor_for_type(
            &descriptor,
            Some("notification"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn parse_config_json_entries_stringifies_scalar_values() {
        let entries = parse_config_json_entries(
            r#"{"username":" alice ","api_path":" /api ","use_ssl":false,"port":8080,"meta":{"tag":"series"}}"#,
        )
        .unwrap();

        assert_eq!(entries.get("username"), Some(&"alice".to_string()));
        assert_eq!(entries.get("api_path"), Some(&"/api".to_string()));
        assert_eq!(entries.get("use_ssl"), Some(&"false".to_string()));
        assert_eq!(entries.get("port"), Some(&"8080".to_string()));
        assert_eq!(
            entries.get("meta"),
            Some(&r#"{"tag":"series"}"#.to_string())
        );
    }

    #[test]
    fn parse_config_json_entries_requires_object_root() {
        let error = parse_config_json_entries(r#"["not","an","object"]"#).unwrap_err();
        assert_eq!(error, "config_json must be a JSON object");
    }

    #[test]
    fn subtitle_client_cache_fingerprint_changes_with_config_json() {
        assert_ne!(
            cache_fingerprint(r#"{"username":"alice"}"#),
            cache_fingerprint(r#"{"username":"bob"}"#)
        );
    }

    #[test]
    fn dynamic_client_cache_insert_returns_existing_for_duplicate_key() {
        let mut cache = HashMap::new();
        let cache_key = indexer_cache_key("newznab", "idx-1", "revision-1");

        let first = insert_indexer_client_cache(
            &mut cache,
            cache_key.clone(),
            Arc::new(DummyIndexerClient),
        );
        let duplicate =
            insert_indexer_client_cache(&mut cache, cache_key, Arc::new(DummyIndexerClient));

        assert!(Arc::ptr_eq(&first, &duplicate));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn dynamic_client_cache_insert_bounds_same_identity_for_all_plugin_families() {
        let mut indexer_cache = HashMap::new();
        let first_indexer = insert_indexer_client_cache(
            &mut indexer_cache,
            indexer_cache_key("newznab", "idx-1", "revision-1"),
            Arc::new(DummyIndexerClient),
        );
        let second_indexer = insert_indexer_client_cache(
            &mut indexer_cache,
            indexer_cache_key("newznab", "idx-1", "revision-2"),
            Arc::new(DummyIndexerClient),
        );
        assert!(!Arc::ptr_eq(&first_indexer, &second_indexer));
        assert_eq!(indexer_cache.len(), 1);
        assert!(indexer_cache.contains_key(&indexer_cache_key("newznab", "idx-1", "revision-2")));

        let mut download_cache = HashMap::new();
        let first_download = insert_download_client_cache(
            &mut download_cache,
            download_cache_key("sabnzbd", "download-1", "revision-1"),
            Arc::new(DummyDownloadClient),
        );
        let second_download = insert_download_client_cache(
            &mut download_cache,
            download_cache_key("sabnzbd", "download-1", "revision-2"),
            Arc::new(DummyDownloadClient),
        );
        assert!(!Arc::ptr_eq(&first_download, &second_download));
        assert_eq!(download_cache.len(), 1);
        assert!(download_cache.contains_key(&download_cache_key(
            "sabnzbd",
            "download-1",
            "revision-2"
        )));

        // Reassigning the proxy on an otherwise unchanged client is also a new
        // build, and the stale proxied entry is evicted rather than kept.
        let proxied_download = insert_download_client_cache(
            &mut download_cache,
            (
                "sabnzbd".to_string(),
                "download-1".to_string(),
                "revision-2".to_string(),
                "proxy-1".to_string(),
                "2026-09-01T00:00:00+00:00".to_string(),
            ),
            Arc::new(DummyDownloadClient),
        );
        assert!(!Arc::ptr_eq(&second_download, &proxied_download));
        assert_eq!(download_cache.len(), 1);

        let mut subtitle_cache = HashMap::new();
        let first_subtitle = insert_subtitle_client_cache(
            &mut subtitle_cache,
            (
                "opensubtitles".to_string(),
                "subtitle-1".to_string(),
                "revision-1".to_string(),
                "config-a".to_string(),
                "bindings-a".to_string(),
            ),
            Arc::new(DummySubtitleProviderClient),
        );
        let second_subtitle = insert_subtitle_client_cache(
            &mut subtitle_cache,
            (
                "opensubtitles".to_string(),
                "subtitle-1".to_string(),
                "revision-2".to_string(),
                "config-b".to_string(),
                "bindings-b".to_string(),
            ),
            Arc::new(DummySubtitleProviderClient),
        );
        assert!(!Arc::ptr_eq(&first_subtitle, &second_subtitle));
        assert_eq!(subtitle_cache.len(), 1);
        assert!(subtitle_cache.contains_key(&(
            "opensubtitles".to_string(),
            "subtitle-1".to_string(),
            "revision-2".to_string(),
            "config-b".to_string(),
            "bindings-b".to_string()
        )));

        let mut notification_cache = HashMap::new();
        let first_notification = insert_notification_client_cache(
            &mut notification_cache,
            (
                "webhook".to_string(),
                "channel-1".to_string(),
                "revision-1".to_string(),
            ),
            Arc::new(DummyNotificationClient),
        );
        let second_notification = insert_notification_client_cache(
            &mut notification_cache,
            (
                "webhook".to_string(),
                "channel-1".to_string(),
                "revision-2".to_string(),
            ),
            Arc::new(DummyNotificationClient),
        );
        assert!(!Arc::ptr_eq(&first_notification, &second_notification));
        assert_eq!(notification_cache.len(), 1);
        assert!(notification_cache.contains_key(&(
            "webhook".to_string(),
            "channel-1".to_string(),
            "revision-2".to_string()
        )));
    }

    #[test]
    fn archive_extractor_provider_routes_by_declared_format_capability() {
        let provider = WasmArchiveExtractorPluginProvider::empty().with_runtime_plugin(
            runtime_plugin_load("archive_extractor", "archive-tools", &[]),
        );

        assert_eq!(provider.available_provider_types(), vec!["archive-tools"]);
        assert!(
            provider
                .provider_for_format(ArchivePluginFormat::Rar)
                .is_some()
        );
        assert!(
            provider
                .provider_for_format(ArchivePluginFormat::Zip)
                .is_some()
        );
        assert!(
            provider
                .provider_for_format(ArchivePluginFormat::SevenZip)
                .is_some()
        );
    }

    #[test]
    fn archive_extractor_runtime_mutation_updates_dynamic_provider() {
        let provider =
            DynamicArchiveExtractorPluginProvider::new(WasmArchiveExtractorPluginProvider::empty());
        provider
            .upsert_runtime_plugin(runtime_plugin_load(
                "archive_extractor",
                "archive-tools",
                &[],
            ))
            .expect("upsert archive extractor");
        provider
            .upsert_runtime_plugin(runtime_plugin_load(
                "archive_extractor",
                "archive-tools-two",
                &[],
            ))
            .expect("upsert second archive extractor");

        assert_eq!(
            provider.available_provider_types(),
            vec!["archive-tools", "archive-tools-two"]
        );

        provider
            .remove_runtime_plugin("archive-tools")
            .expect("remove archive extractor");

        assert_eq!(
            provider.available_provider_types(),
            vec!["archive-tools-two"]
        );
    }

    #[test]
    fn indexer_runtime_mutation_invalidates_only_changed_provider_cache_entries() {
        let provider = DynamicPluginProvider::new(build_indexer_plugin_provider(&[], &[]));
        provider
            .upsert_runtime_plugin(runtime_plugin_load(
                "indexer",
                "example_indexer",
                &["example-indexer-alias"],
            ))
            .expect("upsert example indexer");
        provider
            .upsert_runtime_plugin(runtime_plugin_load("indexer", "newznab", &[]))
            .expect("upsert newznab");

        {
            let mut cache = provider.client_cache.lock().expect("indexer cache lock");
            cache.insert(
                indexer_cache_key("example_indexer", "cfg-a", "1"),
                Arc::new(DummyIndexerClient),
            );
            cache.insert(
                indexer_cache_key("example-indexer-alias", "cfg-b", "1"),
                Arc::new(DummyIndexerClient),
            );
            cache.insert(
                indexer_cache_key("newznab", "cfg-c", "1"),
                Arc::new(DummyIndexerClient),
            );
        }

        provider
            .remove_runtime_plugin("example_indexer")
            .expect("remove target provider");

        let cache = provider.client_cache.lock().expect("indexer cache lock");
        assert_eq!(cache.len(), 1);
        assert!(
            cache
                .keys()
                .all(|(provider_type, _, _, _, _)| provider_type == "newznab")
        );
        let providers = provider.available_provider_types();
        assert!(
            providers
                .iter()
                .any(|provider_type| provider_type == "newznab")
        );
        assert!(
            !providers
                .iter()
                .any(|provider_type| provider_type == "example_indexer")
        );
    }

    #[test]
    fn download_runtime_mutation_invalidates_only_changed_provider_cache_entries() {
        let provider = DynamicDownloadClientPluginProvider::new(
            build_download_client_plugin_provider(&[], &[]),
        );
        provider
            .upsert_runtime_plugin(runtime_plugin_load(
                "download_client",
                "qbittorrent",
                &["qbt"],
            ))
            .expect("upsert qbittorrent");
        provider
            .upsert_runtime_plugin(runtime_plugin_load("download_client", "rtorrent", &[]))
            .expect("upsert rtorrent");

        {
            let mut cache = provider.client_cache.lock().expect("download cache lock");
            cache.insert(
                download_cache_key("qbittorrent", "cfg-a", "1"),
                Arc::new(DummyDownloadClient),
            );
            cache.insert(
                download_cache_key("qbt", "cfg-b", "1"),
                Arc::new(DummyDownloadClient),
            );
            cache.insert(
                download_cache_key("rtorrent", "cfg-c", "1"),
                Arc::new(DummyDownloadClient),
            );
        }

        provider
            .remove_runtime_plugin("qbittorrent")
            .expect("remove target provider");

        let cache = provider.client_cache.lock().expect("download cache lock");
        assert_eq!(cache.len(), 1);
        assert!(
            cache
                .keys()
                .all(|(provider_type, _, _, _, _)| provider_type == "rtorrent")
        );
        assert_eq!(
            provider.available_provider_types(),
            vec!["rtorrent".to_string()]
        );
    }

    #[test]
    fn notification_runtime_mutation_invalidates_only_changed_provider_cache_entries() {
        let provider =
            DynamicNotificationPluginProvider::new(build_notification_plugin_provider(&[], &[]));
        provider
            .upsert_runtime_plugin(runtime_plugin_load(
                "notification",
                "email",
                &["smtp-email"],
            ))
            .expect("upsert email");
        provider
            .upsert_runtime_plugin(runtime_plugin_load("notification", "webhook", &[]))
            .expect("upsert webhook");

        {
            let mut cache = provider
                .client_cache
                .lock()
                .expect("notification cache lock");
            cache.insert(
                (
                    "email".to_string(),
                    "channel-a".to_string(),
                    "1".to_string(),
                ),
                Arc::new(DummyNotificationClient),
            );
            cache.insert(
                (
                    "smtp-email".to_string(),
                    "channel-b".to_string(),
                    "1".to_string(),
                ),
                Arc::new(DummyNotificationClient),
            );
            cache.insert(
                (
                    "webhook".to_string(),
                    "channel-c".to_string(),
                    "1".to_string(),
                ),
                Arc::new(DummyNotificationClient),
            );
        }

        provider
            .remove_runtime_plugin("email")
            .expect("remove target provider");

        let cache = provider
            .client_cache
            .lock()
            .expect("notification cache lock");
        assert_eq!(cache.len(), 1);
        assert!(
            cache
                .keys()
                .all(|(provider_type, _, _)| provider_type == "webhook")
        );
        assert_eq!(
            provider.available_provider_types(),
            vec!["webhook".to_string()]
        );
    }

    #[test]
    fn subtitle_builtin_restore_rejects_removed_builtin() {
        let provider = DynamicSubtitlePluginProvider::new(build_subtitle_plugin_provider(&[], &[]));
        provider
            .upsert_runtime_plugin(runtime_plugin_load(
                "subtitle_provider",
                "opensubtitles",
                &[],
            ))
            .expect("upsert opensubtitles");

        let providers = provider.available_provider_types();
        assert!(
            providers
                .iter()
                .any(|provider_type| provider_type == "opensubtitles")
        );
        assert!(
            !providers
                .iter()
                .any(|provider_type| provider_type == "jimaku")
        );

        assert!(provider.restore_builtin_plugin("jimaku").is_err());
        let providers = provider.available_provider_types();
        assert!(
            providers
                .iter()
                .any(|provider_type| provider_type == "opensubtitles")
        );
        assert!(
            providers
                .iter()
                .all(|provider_type| provider_type != "jimaku")
        );
    }

    /// Indexers accept exactly one runtime, and the two ways an artifact can
    /// fail to reach it are different messages: a pre-component artifact gets
    /// the upgrade instruction, while an artifact whose descriptor belongs to
    /// another family gets the runtime-mismatch message.
    #[test]
    fn indexer_runtime_backing_accepts_only_the_indexer_component() {
        let indexer = descriptor("indexer");
        let component = wat::parse_str("(component)").expect("component WAT must parse");
        let core_module = wat::parse_str("(module (memory (export \"memory\") 1))")
            .expect("core module WAT must parse");

        assert_eq!(
            indexer_runtime_backing(&indexer, &component).expect("an indexer component is usable"),
            PluginRuntimeBacking::Indexer
        );

        let error = indexer_runtime_backing(&indexer, &core_module)
            .expect_err("a pre-component indexer artifact is not usable");
        assert!(error.contains("wasm32-wasip2"), "got: {error}");
        assert!(
            error.contains("scryer:indexer/indexer-plugin@1.1.0"),
            "got: {error}"
        );

        let archive = descriptor("archive_extractor");
        let error = indexer_runtime_backing(&archive, &component)
            .expect_err("an archive runtime is not valid for an indexer");
        assert!(error.contains("not valid for an indexer"), "got: {error}");
    }
}
