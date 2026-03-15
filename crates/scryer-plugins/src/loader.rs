use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use extism::Manifest;
use scryer_application::{
    DownloadClient, DownloadClientPluginProvider, IndexerClient, IndexerPluginProvider,
    NotificationClient, NotificationPluginProvider,
};
use scryer_domain::{DownloadClientConfig, IndexerConfig, NotificationChannelConfig};
use tracing::{info, warn};

use crate::download_client_adapter::WasmDownloadClient;
use crate::indexer_adapter::WasmIndexerClient;
use crate::notification_adapter::WasmNotificationClient;
use crate::types::PluginDescriptor;

const SUPPORTED_SDK_MAJOR: &str = "0";
const NZBGEEK_DEFAULT_BASE_URL: &str = "https://api.nzbgeek.info";
const SUPPORTED_PLUGIN_TYPES: &[&str] = &[
    "indexer",
    "usenet_indexer",
    "torrent_indexer",
    "notification",
    "download_client",
];
const INDEXER_PLUGIN_TYPES: &[&str] = &["indexer", "usenet_indexer", "torrent_indexer"];

struct LoadedPlugin {
    wasm_bytes: Vec<u8>,
    descriptor: PluginDescriptor,
}

pub struct WasmIndexerPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
}

impl WasmIndexerPluginProvider {
    /// Create an empty provider with no plugins loaded.
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Register an externally-installed plugin from WASM bytes.
    /// External plugins take priority over built-ins with the same provider_type.
    pub fn with_external_bytes(mut self, wasm_bytes: &[u8]) -> Self {
        match load_from_bytes(wasm_bytes) {
            Ok((descriptor, bytes)) => {
                if !validate_indexer_descriptor(&descriptor) {
                    return self;
                }

                let provider_type = descriptor.provider_type.trim().to_ascii_lowercase();
                let aliases: Vec<String> = descriptor
                    .provider_aliases
                    .iter()
                    .map(|a| a.trim().to_ascii_lowercase())
                    .collect();

                info!(
                    plugin = descriptor.name.as_str(),
                    version = descriptor.version.as_str(),
                    provider_type = provider_type.as_str(),
                    "registered external plugin"
                );
                self.plugins.insert(
                    provider_type.clone(),
                    LoadedPlugin {
                        wasm_bytes: bytes.clone(),
                        descriptor: descriptor.clone(),
                    },
                );

                for alias in &aliases {
                    self.plugins.insert(
                        alias.clone(),
                        LoadedPlugin {
                            wasm_bytes: bytes.clone(),
                            descriptor: descriptor.clone(),
                        },
                    );
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to load external plugin");
            }
        }
        self
    }

    /// Remove a provider_type (and its aliases) from the loaded set.
    /// Used to disable built-in plugins at runtime.
    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let key = provider_type.trim().to_ascii_lowercase();
        if let Some(loaded) = self.plugins.remove(&key) {
            info!(
                plugin = loaded.descriptor.name.as_str(),
                provider_type = key.as_str(),
                "removed plugin provider_type"
            );
            // Also remove any aliases that point to the same descriptor
            let aliases: Vec<String> = loaded
                .descriptor
                .provider_aliases
                .iter()
                .map(|a| a.trim().to_ascii_lowercase())
                .collect();
            for alias in &aliases {
                self.plugins.remove(alias);
            }
        }
        self
    }

    /// Register a built-in plugin from WASM bytes. The plugin is loaded,
    /// validated, and registered under its `provider_type` (and any
    /// `provider_aliases`). If an external plugin already claims the same
    /// provider_type, the external one wins and the built-in is skipped
    /// for that key.
    pub fn with_builtin(mut self, wasm_bytes: &[u8]) -> Self {
        match load_from_bytes(wasm_bytes) {
            Ok((descriptor, bytes)) => {
                let descriptor = apply_builtin_indexer_overrides(descriptor);
                if !validate_indexer_descriptor(&descriptor) {
                    return self;
                }

                let provider_type = descriptor.provider_type.trim().to_ascii_lowercase();
                let aliases: Vec<String> = descriptor
                    .provider_aliases
                    .iter()
                    .map(|a| a.trim().to_ascii_lowercase())
                    .collect();

                // Register primary provider_type (external overrides built-in)
                if self.plugins.contains_key(&provider_type) {
                    info!(
                        provider_type = provider_type.as_str(),
                        "external plugin overrides built-in"
                    );
                } else {
                    info!(
                        plugin = descriptor.name.as_str(),
                        version = descriptor.version.as_str(),
                        provider_type = provider_type.as_str(),
                        "registered built-in plugin"
                    );
                    self.plugins.insert(
                        provider_type.clone(),
                        LoadedPlugin {
                            wasm_bytes: bytes.clone(),
                            descriptor: descriptor.clone(),
                        },
                    );
                }

                // Register aliases (external overrides built-in)
                for alias in &aliases {
                    if self.plugins.contains_key(alias) {
                        info!(
                            alias = alias.as_str(),
                            provider_type = provider_type.as_str(),
                            "external plugin overrides built-in alias"
                        );
                    } else {
                        self.plugins.insert(
                            alias.clone(),
                            LoadedPlugin {
                                wasm_bytes: bytes.clone(),
                                descriptor: descriptor.clone(),
                            },
                        );
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to load built-in plugin");
            }
        }
        self
    }
}

fn apply_builtin_indexer_overrides(mut descriptor: PluginDescriptor) -> PluginDescriptor {
    if descriptor.provider_type.eq_ignore_ascii_case("nzbgeek") {
        descriptor.default_base_url = Some(NZBGEEK_DEFAULT_BASE_URL.to_string());
    }

    descriptor
}

impl IndexerPluginProvider for WasmIndexerPluginProvider {
    fn available_provider_types(&self) -> Vec<String> {
        // Only return primary provider_types, not aliases (which map to the same plugin)
        self.plugins
            .iter()
            .filter(|(key, loaded)| {
                **key == loaded.descriptor.provider_type.trim().to_ascii_lowercase()
            })
            .map(|(key, _)| key.clone())
            .collect()
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        // Deduplicate: multiple keys may point to the same plugin. Use the
        // primary provider_type as the canonical source for scoring policies.
        let mut seen = std::collections::HashSet::new();
        self.plugins
            .values()
            .filter(|loaded| seen.insert(loaded.descriptor.provider_type.clone()))
            .flat_map(|loaded| {
                loaded.descriptor.scoring_policies.iter().map(|sp| {
                    // ID must be a valid Rego path segment (letters, digits, underscores).
                    let safe_provider = loaded
                        .descriptor
                        .provider_type
                        .replace(['-', ':', '.'], "_");
                    let safe_name = sp.name.replace(['-', ':', '.'], "_");
                    let id = format!("plugin_{safe_provider}_{safe_name}");
                    scryer_rules::UserPolicy {
                        id,
                        rego_source: sp.rego_source.clone(),
                        applied_facets: sp.applied_facets.clone(),
                    }
                })
            })
            .collect()
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.config_fields.clone())
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .and_then(|loaded| loaded.descriptor.default_base_url.clone())
    }

    fn rate_limit_seconds_for_provider(&self, provider_type: &str) -> Option<i64> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .and_then(|loaded| loaded.descriptor.rate_limit_seconds)
    }

    fn capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| scryer_domain::IndexerProviderCapabilities {
                rss: loaded.descriptor.capabilities.rss,
                search: loaded.descriptor.capabilities.search,
                imdb_search: loaded.descriptor.capabilities.imdb_search,
                tvdb_search: loaded.descriptor.capabilities.tvdb_search,
            })
            .unwrap_or(scryer_domain::IndexerProviderCapabilities {
                rss: true,
                search: true,
                imdb_search: true,
                tvdb_search: true,
            })
    }

    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        let provider = config.provider_type.trim().to_ascii_lowercase();
        let loaded = self.plugins.get(&provider)?;

        let mut manifest = Manifest::new([extism::Wasm::data(loaded.wasm_bytes.clone())]);
        manifest = apply_allowed_hosts(
            manifest,
            &loaded.descriptor,
            Some(&config.base_url),
            config.config_json.as_deref(),
        );
        manifest = manifest.with_timeout(std::time::Duration::from_secs(30));

        // Inject standard config values the plugin can read via config::get()
        manifest = manifest.with_config_key("base_url", &config.base_url);
        if let Some(ref api_key) = config.api_key_encrypted {
            manifest = manifest.with_config_key("api_key", api_key);
        }

        // Inject any additional key-value pairs from config_json
        if let Some(ref json_str) = config.config_json {
            match parse_config_json_entries(json_str) {
                Ok(map) => {
                    for (k, v) in &map {
                        manifest = manifest.with_config_key(k, v);
                    }
                }
                Err(error) => {
                    warn!(
                        indexer = config.name.as_str(),
                        error = %error,
                        "failed to parse config_json; extra config keys will not be injected"
                    );
                }
            }
        }

        match build_plugin(manifest) {
            Ok(plugin) => {
                let client =
                    WasmIndexerClient::new(plugin, loaded.descriptor.clone(), config.name.clone());
                Some(Arc::new(client))
            }
            Err(e) => {
                warn!(
                    provider_type = provider.as_str(),
                    indexer = config.name.as_str(),
                    error = %e,
                    "failed to instantiate WASM plugin for indexer"
                );
                None
            }
        }
    }
}

/// A thread-safe wrapper around `WasmIndexerPluginProvider` that supports
/// runtime reload. All reads acquire a `RwLock` read lock; `reload()` acquires
/// a write lock to swap the inner provider.
///
/// Caches instantiated `IndexerClient`s by `(indexer_config_id, updated_at)` so
/// WASM compilation only happens once per config revision. The cache is cleared
/// on provider reload.
pub struct DynamicPluginProvider {
    inner: std::sync::RwLock<WasmIndexerPluginProvider>,
    client_cache: std::sync::Mutex<HashMap<(String, String), Arc<dyn IndexerClient>>>,
}

impl DynamicPluginProvider {
    pub fn new(provider: WasmIndexerPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Replace the inner provider. This is called after install/uninstall/toggle.
    pub fn reload(&self, new_provider: WasmIndexerPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .expect("DynamicPluginProvider lock poisoned");
        *guard = new_provider;
        // Clear the client cache — WASM bytes may have changed.
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        info!("plugin provider reloaded");
    }
}

impl IndexerPluginProvider for DynamicPluginProvider {
    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        let cache_key = (config.id.clone(), config.updated_at.to_rfc3339());

        // Fast path: check cache first
        if let Ok(cache) = self.client_cache.lock() {
            if let Some(client) = cache.get(&cache_key) {
                return Some(Arc::clone(client));
            }
        }

        // Slow path: compile WASM and cache the result
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        let client = guard.client_for_provider(config)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            cache.insert(cache_key, Arc::clone(&client));
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
        external_wasm_bytes: &[&[u8]],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let mut provider = WasmIndexerPluginProvider::empty();

        // Load user-installed (non-builtin) plugins first — they get priority
        for bytes in external_wasm_bytes {
            provider = provider.with_external_bytes(bytes);
        }

        // Layer builtins (skipped if external overrides same provider_type)
        provider = provider
            .with_builtin(crate::builtins::NZBGEEK_WASM)
            .with_builtin(crate::builtins::NEWZNAB_WASM);

        // Remove builtins the user has disabled
        for pt in disabled_builtins {
            provider = provider.without_provider_type(pt);
        }

        self.reload(provider);
        Ok(())
    }
}

// ── Download client plugin provider ────────────────────────────────────

pub struct WasmDownloadClientPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
}

impl WasmDownloadClientPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn with_external_bytes(mut self, wasm_bytes: &[u8]) -> Self {
        match load_from_bytes(wasm_bytes) {
            Ok((descriptor, bytes)) => {
                if !validate_descriptor_for_type(&descriptor, Some("download_client")) {
                    return self;
                }

                let provider_type = descriptor.provider_type.trim().to_ascii_lowercase();
                info!(
                    plugin = descriptor.name.as_str(),
                    version = descriptor.version.as_str(),
                    provider_type = provider_type.as_str(),
                    "registered external download client plugin"
                );
                self.plugins.insert(
                    provider_type,
                    LoadedPlugin {
                        wasm_bytes: bytes,
                        descriptor,
                    },
                );
            }
            Err(e) => {
                warn!(error = %e, "failed to load external download client plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins.remove(&key);
        self
    }

    fn create_download_client(
        loaded: &LoadedPlugin,
        config: &DownloadClientConfig,
    ) -> Option<Arc<dyn DownloadClient>> {
        let mut manifest = Manifest::new([extism::Wasm::data(loaded.wasm_bytes.clone())]);
        manifest = apply_allowed_hosts(
            manifest,
            &loaded.descriptor,
            config.base_url.as_deref(),
            Some(&config.config_json),
        );
        manifest = manifest.with_timeout(std::time::Duration::from_secs(30));

        if let Some(ref base_url) = config.base_url {
            manifest = manifest.with_config_key("base_url", base_url);
        }

        match parse_config_json_entries(&config.config_json) {
            Ok(map) => {
                for (k, v) in &map {
                    manifest = manifest.with_config_key(k, v);
                }
            }
            Err(error) => {
                warn!(
                    client = config.name.as_str(),
                    error = %error,
                    "failed to parse download client config_json"
                );
            }
        }

        match build_plugin(manifest) {
            Ok(plugin) => {
                let client = WasmDownloadClient::new(
                    plugin,
                    loaded.descriptor.clone(),
                    config.id.clone(),
                    config.name.clone(),
                );
                Some(Arc::new(client))
            }
            Err(e) => {
                warn!(
                    client = config.name.as_str(),
                    provider_type = config.client_type.as_str(),
                    error = %e,
                    "failed to instantiate WASM download client plugin"
                );
                None
            }
        }
    }
}

impl DownloadClientPluginProvider for WasmDownloadClientPluginProvider {
    fn client_for_config(&self, config: &DownloadClientConfig) -> Option<Arc<dyn DownloadClient>> {
        let provider = config.client_type.trim().to_ascii_lowercase();
        let loaded = self.plugins.get(&provider)?;
        Self::create_download_client(loaded, config)
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|(key, loaded)| {
                **key == loaded.descriptor.provider_type.trim().to_ascii_lowercase()
            })
            .map(|(key, _)| key.clone())
            .collect()
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.config_fields.clone())
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .and_then(|loaded| loaded.descriptor.default_base_url.clone())
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[&[u8]],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        Err("use DynamicDownloadClientPluginProvider for reload".to_string())
    }
}

pub struct DynamicDownloadClientPluginProvider {
    inner: std::sync::RwLock<WasmDownloadClientPluginProvider>,
    client_cache: std::sync::Mutex<HashMap<(String, String), Arc<dyn DownloadClient>>>,
}

impl DynamicDownloadClientPluginProvider {
    pub fn new(provider: WasmDownloadClientPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn reload(&self, new_provider: WasmDownloadClientPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        *guard = new_provider;
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        info!("download client plugin provider reloaded");
    }
}

impl DownloadClientPluginProvider for DynamicDownloadClientPluginProvider {
    fn client_for_config(&self, config: &DownloadClientConfig) -> Option<Arc<dyn DownloadClient>> {
        let cache_key = (config.id.clone(), config.updated_at.to_rfc3339());

        if let Ok(cache) = self.client_cache.lock() {
            if let Some(client) = cache.get(&cache_key) {
                return Some(Arc::clone(client));
            }
        }

        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        let client = guard.client_for_config(config)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            cache.insert(cache_key, Arc::clone(&client));
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
        external_wasm_bytes: &[&[u8]],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let mut provider = WasmDownloadClientPluginProvider::empty();

        for bytes in external_wasm_bytes {
            provider = provider.with_external_bytes(bytes);
        }

        for pt in disabled_builtins {
            provider = provider.without_provider_type(pt);
        }

        self.reload(provider);
        Ok(())
    }
}

/// Validate a plugin descriptor, optionally filtering by a specific plugin type.
/// If `expected_type` is None, any supported type passes.
fn validate_descriptor_for_type(
    descriptor: &PluginDescriptor,
    expected_type: Option<&str>,
) -> bool {
    let sdk_major = descriptor.sdk_version.split('.').next().unwrap_or("");
    if sdk_major != SUPPORTED_SDK_MAJOR {
        warn!(
            plugin = descriptor.name.as_str(),
            sdk_version = descriptor.sdk_version.as_str(),
            expected_major = SUPPORTED_SDK_MAJOR,
            "skipping plugin: incompatible sdk_version"
        );
        return false;
    }

    if !SUPPORTED_PLUGIN_TYPES.contains(&descriptor.plugin_type.as_str()) {
        info!(
            plugin = descriptor.name.as_str(),
            plugin_type = descriptor.plugin_type.as_str(),
            "skipping plugin: type '{}' not supported",
            descriptor.plugin_type
        );
        return false;
    }

    if let Some(expected) = expected_type {
        if descriptor.plugin_type != expected {
            return false;
        }
    }

    true
}

fn is_indexer_plugin_type(plugin_type: &str) -> bool {
    INDEXER_PLUGIN_TYPES.contains(&plugin_type)
}

fn validate_indexer_descriptor(descriptor: &PluginDescriptor) -> bool {
    validate_descriptor_for_type(descriptor, None)
        && is_indexer_plugin_type(&descriptor.plugin_type)
}

/// Scan `plugins_dir` for subdirectories containing `plugin.wasm`, load each,
/// call `describe()` to get the plugin descriptor, and return a provider that
/// can create indexer clients for any loaded plugin type.
pub fn load_indexer_plugins(plugins_dir: &Path) -> Result<WasmIndexerPluginProvider, String> {
    let mut plugins = HashMap::new();

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
                if !validate_indexer_descriptor(&descriptor) {
                    continue;
                }

                let provider_type = descriptor.provider_type.trim().to_ascii_lowercase();

                // Check for duplicates
                if plugins.contains_key(&provider_type) {
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

                // Register aliases
                let aliases: Vec<String> = descriptor
                    .provider_aliases
                    .iter()
                    .map(|a| a.trim().to_ascii_lowercase())
                    .collect();
                for alias in &aliases {
                    if !plugins.contains_key(alias) {
                        plugins.insert(
                            alias.clone(),
                            LoadedPlugin {
                                wasm_bytes: wasm_bytes.clone(),
                                descriptor: descriptor.clone(),
                            },
                        );
                    }
                }

                plugins.insert(
                    provider_type,
                    LoadedPlugin {
                        wasm_bytes,
                        descriptor,
                    },
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

    Ok(WasmIndexerPluginProvider { plugins })
}

fn load_single_plugin(wasm_path: &Path) -> Result<(PluginDescriptor, Vec<u8>), String> {
    let wasm_bytes = std::fs::read(wasm_path)
        .map_err(|e| format!("failed to read {}: {e}", wasm_path.display()))?;

    load_from_bytes(&wasm_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(plugin_type: &str) -> PluginDescriptor {
        PluginDescriptor {
            name: "Test".to_string(),
            version: "0.1.0".to_string(),
            sdk_version: "0.1".to_string(),
            plugin_type: plugin_type.to_string(),
            provider_type: "test".to_string(),
            provider_aliases: vec![],
            capabilities: crate::types::IndexerCapabilities::default(),
            scoring_policies: vec![],
            config_fields: vec![],
            default_base_url: None,
            allowed_hosts: vec![],
            rate_limit_seconds: None,
            notification_capabilities: None,
            accepted_inputs: vec![],
            isolation_modes: vec![],
            download_client_capabilities: None,
        }
    }

    #[test]
    fn indexer_family_types_are_accepted() {
        assert!(validate_indexer_descriptor(&descriptor("indexer")));
        assert!(validate_indexer_descriptor(&descriptor("usenet_indexer")));
        assert!(validate_indexer_descriptor(&descriptor("torrent_indexer")));
    }

    #[test]
    fn non_indexer_types_are_rejected_for_indexer_provider() {
        assert!(!validate_indexer_descriptor(&descriptor("notification")));
        assert!(!validate_indexer_descriptor(&descriptor("download_client")));
    }

    #[test]
    fn parse_config_json_entries_stringifies_scalar_values() {
        let entries = parse_config_json_entries(
            r#"{"username":"alice","password":"secret","use_ssl":false,"port":8080,"meta":{"tag":"tv"}}"#,
        )
        .unwrap();

        assert_eq!(entries.get("username"), Some(&"alice".to_string()));
        assert_eq!(entries.get("password"), Some(&"secret".to_string()));
        assert_eq!(entries.get("use_ssl"), Some(&"false".to_string()));
        assert_eq!(entries.get("port"), Some(&"8080".to_string()));
        assert_eq!(entries.get("meta"), Some(&r#"{"tag":"tv"}"#.to_string()));
    }

    #[test]
    fn parse_config_json_entries_requires_object_root() {
        let error = parse_config_json_entries(r#"["not","an","object"]"#).unwrap_err();
        assert_eq!(error, "config_json must be a JSON object");
    }
}

fn parse_config_json_entries(json_str: &str) -> Result<HashMap<String, String>, String> {
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
            serde_json::Value::String(value) => value.clone(),
            other => other.to_string(),
        };
        entries.insert(key.clone(), normalized);
    }

    Ok(entries)
}

/// Build the Extism allowed-hosts list for a plugin manifest.
///
/// The allowed hosts are derived from:
/// 1. The plugin's `allowed_hosts` descriptor field (static declarations).
///    Use `["*"]` for unrestricted access.
/// 2. The hostname from `base_url` (indexer plugins).
/// 3. Hostnames from `config_json` values that parse as URLs (notification plugins).
///
/// If the resulting set is empty, no hosts are allowed (plugin has no network access).
fn apply_allowed_hosts(
    mut manifest: Manifest,
    descriptor: &PluginDescriptor,
    base_url: Option<&str>,
    config_json: Option<&str>,
) -> Manifest {
    // Short-circuit: explicit wildcard in descriptor
    if descriptor.allowed_hosts.iter().any(|h| h == "*") {
        return manifest.with_allowed_host("*");
    }

    let mut hosts: Vec<String> = descriptor.allowed_hosts.clone();

    // Add hostname from base_url (indexer plugins)
    if let Some(url_str) = base_url {
        if let Some(host) = host_from_url(url_str) {
            hosts.push(host);
        }
    }

    // Add hostnames from config_json values that parse as URLs (notification plugins)
    if let Some(json_str) = config_json {
        if let Ok(map) = parse_config_json_entries(json_str) {
            for value in map.values() {
                if let Some(host) = host_from_url(value) {
                    hosts.push(host);
                }
            }
        }
    }

    for host in &hosts {
        manifest = manifest.with_allowed_host(host);
    }
    manifest
}

/// Extract hostname from a URL string without pulling in the `url` crate.
fn host_from_url(url: &str) -> Option<String> {
    // Expect "scheme://host..." — strip scheme, then take until '/' or ':'
    let after_scheme = url.split("://").nth(1)?;
    let host = after_scheme.split('/').next()?;
    // Strip port if present
    let host = if host.contains('[') {
        // IPv6: [::1]:8080 — take everything including brackets
        host.split(']')
            .next()
            .map(|h| format!("{}]", h))
            .unwrap_or_default()
    } else {
        host.split(':').next().unwrap_or(host).to_string()
    };
    if host.is_empty() { None } else { Some(host) }
}

fn build_plugin(manifest: Manifest) -> Result<extism::Plugin, extism::Error> {
    extism::PluginBuilder::new(manifest)
        .with_wasi(true)
        .with_http_response_headers(true)
        .build()
}

fn load_from_bytes(wasm_bytes: &[u8]) -> Result<(PluginDescriptor, Vec<u8>), String> {
    let bytes = wasm_bytes.to_vec();
    // No allowed hosts needed — describe() is a pure function that returns JSON.
    let manifest = Manifest::new([extism::Wasm::data(bytes.clone())])
        .with_timeout(std::time::Duration::from_secs(10));

    let mut plugin =
        build_plugin(manifest).map_err(|e| format!("failed to instantiate WASM: {e}"))?;

    let output: String = plugin
        .call::<&str, String>("describe", "")
        .map_err(|e| format!("describe() failed: {e}"))?;

    let descriptor: PluginDescriptor = serde_json::from_str(&output)
        .map_err(|e| format!("describe() returned invalid JSON: {e}"))?;

    Ok((descriptor, bytes))
}

// ── Notification plugin provider ───────────────────────────────────────

pub struct WasmNotificationPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
}

impl WasmNotificationPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn with_external_bytes(mut self, wasm_bytes: &[u8]) -> Self {
        match load_from_bytes(wasm_bytes) {
            Ok((descriptor, bytes)) => {
                if !validate_descriptor_for_type(&descriptor, Some("notification")) {
                    return self;
                }

                let provider_type = descriptor.provider_type.trim().to_ascii_lowercase();
                info!(
                    plugin = descriptor.name.as_str(),
                    version = descriptor.version.as_str(),
                    provider_type = provider_type.as_str(),
                    "registered external notification plugin"
                );
                self.plugins.insert(
                    provider_type,
                    LoadedPlugin {
                        wasm_bytes: bytes,
                        descriptor,
                    },
                );
            }
            Err(e) => {
                warn!(error = %e, "failed to load external notification plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins.remove(&key);
        self
    }

    fn create_notification_client(
        loaded: &LoadedPlugin,
        config: &NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        let mut manifest = Manifest::new([extism::Wasm::data(loaded.wasm_bytes.clone())]);
        manifest = apply_allowed_hosts(
            manifest,
            &loaded.descriptor,
            None,
            Some(&config.config_json),
        );
        manifest = manifest.with_timeout(std::time::Duration::from_secs(30));

        // Inject config_json key-value pairs
        match parse_config_json_entries(&config.config_json) {
            Ok(map) => {
                for (k, v) in &map {
                    manifest = manifest.with_config_key(k, v);
                }
            }
            Err(error) => {
                warn!(
                    channel = config.name.as_str(),
                    error = %error,
                    "failed to parse notification channel config_json"
                );
            }
        }

        match build_plugin(manifest) {
            Ok(plugin) => {
                let client = WasmNotificationClient::new(
                    plugin,
                    loaded.descriptor.clone(),
                    config.name.clone(),
                );
                Some(Arc::new(client))
            }
            Err(e) => {
                warn!(
                    channel = config.name.as_str(),
                    error = %e,
                    "failed to instantiate WASM notification plugin"
                );
                None
            }
        }
    }
}

impl NotificationPluginProvider for WasmNotificationPluginProvider {
    fn client_for_channel(
        &self,
        config: &NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        let provider = config.channel_type.trim().to_ascii_lowercase();
        let loaded = self.plugins.get(&provider)?;
        Self::create_notification_client(loaded, config)
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|(key, loaded)| {
                **key == loaded.descriptor.provider_type.trim().to_ascii_lowercase()
            })
            .map(|(key, _)| key.clone())
            .collect()
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.config_fields.clone())
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[&[u8]],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        Err("use DynamicNotificationPluginProvider for reload".to_string())
    }
}

/// Thread-safe wrapper around `WasmNotificationPluginProvider` that supports runtime reload.
pub struct DynamicNotificationPluginProvider {
    inner: std::sync::RwLock<WasmNotificationPluginProvider>,
    client_cache: std::sync::Mutex<HashMap<(String, String), Arc<dyn NotificationClient>>>,
}

impl DynamicNotificationPluginProvider {
    pub fn new(provider: WasmNotificationPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn reload(&self, new_provider: WasmNotificationPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .expect("DynamicNotificationPluginProvider lock poisoned");
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
        let cache_key = (config.id.clone(), config.updated_at.to_rfc3339());

        if let Ok(cache) = self.client_cache.lock() {
            if let Some(client) = cache.get(&cache_key) {
                return Some(Arc::clone(client));
            }
        }

        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        let client = guard.client_for_channel(config)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            cache.insert(cache_key, Arc::clone(&client));
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

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[&[u8]],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let mut provider = WasmNotificationPluginProvider::empty();

        for bytes in external_wasm_bytes {
            provider = provider.with_external_bytes(bytes);
        }

        for pt in disabled_builtins {
            provider = provider.without_provider_type(pt);
        }

        self.reload(provider);
        Ok(())
    }
}
