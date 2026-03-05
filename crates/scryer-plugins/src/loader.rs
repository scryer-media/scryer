use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use extism::Manifest;
use scryer_application::{IndexerClient, IndexerPluginProvider};
use scryer_domain::IndexerConfig;
use tracing::{info, warn};

use crate::indexer_adapter::WasmIndexerClient;
use crate::types::PluginDescriptor;

const SUPPORTED_SDK_MAJOR: &str = "0";
const SUPPORTED_PLUGIN_TYPE: &str = "indexer";

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
                if !validate_descriptor(&descriptor) {
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
                if !validate_descriptor(&descriptor) {
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

impl IndexerPluginProvider for WasmIndexerPluginProvider {
    fn available_provider_types(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
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
                    let safe_provider = loaded.descriptor.provider_type.replace(['-', ':', '.'], "_");
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

    fn config_fields_for_provider(&self, provider_type: &str) -> Vec<scryer_domain::ConfigFieldDef> {
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

    fn capabilities_for_provider(&self, provider_type: &str) -> scryer_domain::IndexerProviderCapabilities {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| scryer_domain::IndexerProviderCapabilities {
                search: loaded.descriptor.capabilities.search,
                imdb_search: loaded.descriptor.capabilities.imdb_search,
                tvdb_search: loaded.descriptor.capabilities.tvdb_search,
            })
            .unwrap_or(scryer_domain::IndexerProviderCapabilities {
                search: true,
                imdb_search: true,
                tvdb_search: true,
            })
    }

    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        let provider = config.provider_type.trim().to_ascii_lowercase();
        let loaded = self.plugins.get(&provider)?;

        let mut manifest = Manifest::new([extism::Wasm::data(loaded.wasm_bytes.clone())]);
        manifest = manifest
            .with_allowed_host("*")
            .with_timeout(std::time::Duration::from_secs(30));

        // Inject standard config values the plugin can read via config::get()
        manifest = manifest.with_config_key("base_url", &config.base_url);
        if let Some(ref api_key) = config.api_key_encrypted {
            manifest = manifest.with_config_key("api_key", api_key);
        }

        // Inject any additional key-value pairs from config_json
        if let Some(ref json_str) = config.config_json {
            match serde_json::from_str::<HashMap<String, String>>(json_str) {
                Ok(map) => {
                    for (k, v) in &map {
                        manifest = manifest.with_config_key(k, v);
                    }
                }
                Err(e) => {
                    warn!(
                        indexer = config.name.as_str(),
                        error = %e,
                        "failed to parse config_json as string map; extra config keys will not be injected"
                    );
                }
            }
        }

        match extism::Plugin::new(manifest, [], true) {
            Ok(plugin) => {
                let client = WasmIndexerClient::new(
                    plugin,
                    loaded.descriptor.clone(),
                    config.name.clone(),
                );
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
        let mut guard = self.inner.write().expect("DynamicPluginProvider lock poisoned");
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
        let guard = self.inner.read().expect("DynamicPluginProvider lock poisoned");
        let client = guard.client_for_provider(config)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            cache.insert(cache_key, Arc::clone(&client));
        }

        Some(client)
    }

    fn available_provider_types(&self) -> Vec<String> {
        let guard = self.inner.read().expect("DynamicPluginProvider lock poisoned");
        guard.available_provider_types()
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        let guard = self.inner.read().expect("DynamicPluginProvider lock poisoned");
        guard.scoring_policies()
    }

    fn config_fields_for_provider(&self, provider_type: &str) -> Vec<scryer_domain::ConfigFieldDef> {
        let guard = self.inner.read().expect("DynamicPluginProvider lock poisoned");
        guard.config_fields_for_provider(provider_type)
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self.inner.read().expect("DynamicPluginProvider lock poisoned");
        guard.plugin_name_for_provider(provider_type)
    }

    fn capabilities_for_provider(&self, provider_type: &str) -> scryer_domain::IndexerProviderCapabilities {
        let guard = self.inner.read().expect("DynamicPluginProvider lock poisoned");
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

/// Validate a plugin descriptor. Returns false (with log warnings) if the
/// plugin should be skipped.
fn validate_descriptor(descriptor: &PluginDescriptor) -> bool {
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

    if descriptor.plugin_type != SUPPORTED_PLUGIN_TYPE {
        info!(
            plugin = descriptor.name.as_str(),
            plugin_type = descriptor.plugin_type.as_str(),
            "skipping plugin: type '{}' not yet supported",
            descriptor.plugin_type
        );
        return false;
    }

    true
}

/// Scan `plugins_dir` for subdirectories containing `plugin.wasm`, load each,
/// call `describe()` to get the plugin descriptor, and return a provider that
/// can create indexer clients for any loaded plugin type.
pub fn load_indexer_plugins(plugins_dir: &Path) -> Result<WasmIndexerPluginProvider, String> {
    let mut plugins = HashMap::new();

    let entries = std::fs::read_dir(plugins_dir)
        .map_err(|e| format!("failed to read plugins directory {}: {e}", plugins_dir.display()))?;

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
                if !validate_descriptor(&descriptor) {
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

fn load_from_bytes(wasm_bytes: &[u8]) -> Result<(PluginDescriptor, Vec<u8>), String> {
    let bytes = wasm_bytes.to_vec();
    let manifest = Manifest::new([extism::Wasm::data(bytes.clone())])
        .with_allowed_host("*")
        .with_timeout(std::time::Duration::from_secs(10));

    let mut plugin = extism::Plugin::new(manifest, [], true)
        .map_err(|e| format!("failed to instantiate WASM: {e}"))?;

    let output: String = plugin
        .call::<&str, String>("describe", "")
        .map_err(|e| format!("describe() failed: {e}"))?;

    let descriptor: PluginDescriptor = serde_json::from_str(&output)
        .map_err(|e| format!("describe() returned invalid JSON: {e}"))?;

    Ok((descriptor, bytes))
}
