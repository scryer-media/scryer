//! The one indexer provider the application holds, composed from the plugin
//! runtime and any host-native providers.
//!
//! Host-native providers (Prowlarr today) are not plugins: they have no
//! descriptor, no SDK version and no lifecycle. They implement
//! [`NativeIndexerProvider`], and [`IndexerProviderRegistry`] routes each
//! [`IndexerPluginProvider`] call either to the native provider that claims
//! the provider type or to the plugin provider. Every trait method is
//! dispatched here explicitly so a new one cannot silently fall through to a
//! trait default.

use std::collections::BTreeMap;
use std::sync::Arc;

use scryer_domain::IndexerConfig;

use crate::{
    AppResult, ExternalPluginWasm, IndexerAccountingContext, IndexerClient,
    IndexerManagementClient, IndexerPluginProvider, RuntimePluginLoad,
};

/// What a host-native indexer provider owns: its identity, configuration
/// surface and clients. Nothing here concerns plugin installation or
/// lifecycle.
pub trait NativeIndexerProvider: Send + Sync {
    /// The provider type this provider claims. Matched case-insensitively
    /// after trimming.
    fn provider_type(&self) -> &str;
    fn display_name(&self) -> String;
    fn description(&self) -> String;
    fn config_fields(&self) -> Vec<scryer_domain::ConfigFieldDef>;
    fn management_capabilities(&self) -> scryer_domain::IndexerManagementCapabilities;
    fn capabilities(&self) -> scryer_domain::IndexerProviderCapabilities;
    fn client(
        &self,
        config: &IndexerConfig,
        proxy_config: Option<&scryer_domain::ProxyConfig>,
        accounting: Option<&IndexerAccountingContext>,
    ) -> Option<Arc<dyn IndexerClient>>;
    fn management_client(
        &self,
        config: &IndexerConfig,
        accounting: Option<&IndexerAccountingContext>,
    ) -> Option<Arc<dyn IndexerManagementClient>>;

    fn validate_config(&self, _config_json: &str) -> AppResult<()> {
        Ok(())
    }
    fn default_base_url(&self) -> Option<String> {
        None
    }
    fn rate_limit_seconds(&self) -> Option<i64> {
        None
    }
    fn dynamic_config_trigger_keys(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Routes indexer provider calls to the native provider claiming the type,
/// or to the plugin provider otherwise.
pub struct IndexerProviderRegistry {
    plugins: Arc<dyn IndexerPluginProvider>,
    native: Vec<Arc<dyn NativeIndexerProvider>>,
}

fn same_provider_type(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

impl IndexerProviderRegistry {
    pub fn new(plugins: Arc<dyn IndexerPluginProvider>) -> Self {
        Self {
            plugins,
            native: Vec::new(),
        }
    }

    /// Register a host-native provider. A provider whose type is already
    /// claimed by an earlier native provider is ignored with a warning: the
    /// first registration keeps the type.
    pub fn with_native(mut self, provider: Arc<dyn NativeIndexerProvider>) -> Self {
        if self.native_for(provider.provider_type()).is_some() {
            tracing::warn!(
                provider_type = provider.provider_type(),
                "ignoring a second native indexer provider for an already claimed provider type"
            );
            return self;
        }
        self.native.push(provider);
        self
    }

    fn native_for(&self, provider_type: &str) -> Option<&dyn NativeIndexerProvider> {
        self.native
            .iter()
            .find(|provider| same_provider_type(provider.provider_type(), provider_type))
            .map(|provider| provider.as_ref())
    }

    fn is_native(&self, provider_type: &str) -> bool {
        self.native_for(provider_type).is_some()
    }

    /// Native types are reserved: plugins never load or disable under them.
    fn without_native_types(&self, values: &[String]) -> Vec<String> {
        values
            .iter()
            .filter(|value| !self.is_native(value))
            .cloned()
            .collect()
    }

    fn without_native_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
    ) -> Vec<RuntimePluginLoad> {
        runtime_plugins
            .iter()
            .filter(|plugin| {
                let provider_type = plugin.descriptor.provider_type();
                if self.is_native(provider_type) {
                    tracing::warn!(
                        provider_type,
                        plugin_id = plugin.descriptor.id.as_str(),
                        "ignoring runtime plugin that tried to claim a reserved native provider type"
                    );
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    }
}

impl IndexerPluginProvider for IndexerProviderRegistry {
    fn validate_config_for_provider(
        &self,
        provider_type: &str,
        config_json: &str,
    ) -> AppResult<()> {
        match self.native_for(provider_type) {
            Some(native) => native.validate_config(config_json),
            None => self
                .plugins
                .validate_config_for_provider(provider_type, config_json),
        }
    }

    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        match self.native_for(&config.provider_type) {
            Some(native) => native.client(
                config,
                None,
                IndexerAccountingContext::for_config(config).as_ref(),
            ),
            None => self.plugins.client_for_provider(config),
        }
    }

    fn client_for_provider_with_proxy(
        &self,
        config: &IndexerConfig,
        proxy_config: Option<&scryer_domain::ProxyConfig>,
    ) -> Option<Arc<dyn IndexerClient>> {
        match self.native_for(&config.provider_type) {
            Some(native) => native.client(
                config,
                proxy_config,
                IndexerAccountingContext::for_config(config).as_ref(),
            ),
            None => self
                .plugins
                .client_for_provider_with_proxy(config, proxy_config),
        }
    }

    fn management_client_for_provider(
        &self,
        config: &IndexerConfig,
    ) -> Option<Arc<dyn IndexerManagementClient>> {
        match self.native_for(&config.provider_type) {
            Some(native) => native.management_client(
                config,
                IndexerAccountingContext::for_config(config).as_ref(),
            ),
            None => self.plugins.management_client_for_provider(config),
        }
    }

    fn client_for_provider_with_accounting(
        &self,
        config: &IndexerConfig,
        proxy_config: Option<&scryer_domain::ProxyConfig>,
        accounting: Option<&IndexerAccountingContext>,
    ) -> Option<Arc<dyn IndexerClient>> {
        match self.native_for(&config.provider_type) {
            Some(native) => native.client(config, proxy_config, accounting),
            None => {
                self.plugins
                    .client_for_provider_with_accounting(config, proxy_config, accounting)
            }
        }
    }

    fn management_client_for_provider_with_accounting(
        &self,
        config: &IndexerConfig,
        accounting: Option<&IndexerAccountingContext>,
    ) -> Option<Arc<dyn IndexerManagementClient>> {
        match self.native_for(&config.provider_type) {
            Some(native) => native.management_client(config, accounting),
            None => self
                .plugins
                .management_client_for_provider_with_accounting(config, accounting),
        }
    }

    fn available_provider_types(&self) -> Vec<String> {
        let mut providers = self.without_native_types(&self.plugins.available_provider_types());
        providers.extend(
            self.native
                .iter()
                .map(|provider| provider.provider_type().to_string()),
        );
        providers.sort();
        providers.dedup();
        providers
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        self.without_native_types(&self.plugins.builtin_provider_types())
    }

    fn builtin_descriptor_for_provider(
        &self,
        provider_type: &str,
    ) -> Option<scryer_plugin_sdk::PluginDescriptor> {
        if self.is_native(provider_type) {
            return None;
        }
        self.plugins.builtin_descriptor_for_provider(provider_type)
    }

    fn restore_builtin_plugin_with_settings(
        &self,
        provider_type: &str,
        settings: BTreeMap<String, String>,
    ) -> Result<(), String> {
        if self.is_native(provider_type) {
            return Ok(());
        }
        self.plugins
            .restore_builtin_plugin_with_settings(provider_type, settings)
    }

    fn reload_runtime_plugins_with_builtin_settings(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
        settings: &BTreeMap<String, BTreeMap<String, String>>,
    ) -> Result<(), String> {
        let runtime_plugins = self.without_native_plugins(runtime_plugins);
        let disabled_builtins = self.without_native_types(disabled_builtins);
        self.plugins.reload_runtime_plugins_with_builtin_settings(
            &runtime_plugins,
            &disabled_builtins,
            settings,
        )
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        if self.is_native(provider_type) {
            return None;
        }
        self.plugins.plugin_version_for_provider(provider_type)
    }

    fn search_semantics_version_for_provider(&self, provider_type: &str) -> Option<u32> {
        if self.is_native(provider_type) {
            return None;
        }
        self.plugins
            .search_semantics_version_for_provider(provider_type)
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        if self.is_native(provider_type) {
            return None;
        }
        self.plugins.plugin_sdk_version_for_provider(provider_type)
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        if self.is_native(provider_type) {
            return None;
        }
        self.plugins
            .plugin_sdk_constraint_for_provider(provider_type)
    }

    fn plugin_type_for_provider(&self, provider_type: &str) -> Option<String> {
        if self.is_native(provider_type) {
            return Some("indexer".to_string());
        }
        self.plugins.plugin_type_for_provider(provider_type)
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        self.plugins.scoring_policies()
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let provider_type = plugin.descriptor.provider_type();
        if self.is_native(provider_type) {
            tracing::warn!(
                provider_type,
                plugin_id = plugin.descriptor.id.as_str(),
                "ignoring runtime plugin that tried to claim a reserved native provider type"
            );
            return Ok(());
        }
        self.plugins.upsert_runtime_plugin(plugin)
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        if self.is_native(provider_type) {
            return Ok(());
        }
        self.plugins.remove_runtime_plugin(provider_type)
    }

    fn prepare_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        if self.is_native(provider_type) {
            return Ok(());
        }
        self.plugins.prepare_builtin_plugin(provider_type)
    }

    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        if self.is_native(provider_type) {
            return Ok(());
        }
        self.plugins.restore_builtin_plugin(provider_type)
    }

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let disabled_builtins = self.without_native_types(disabled_builtins);
        self.plugins
            .reload_plugins(external_wasm_bytes, &disabled_builtins)
    }

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let runtime_plugins = self.without_native_plugins(runtime_plugins);
        let disabled_builtins = self.without_native_types(disabled_builtins);
        self.plugins
            .reload_runtime_plugins(&runtime_plugins, &disabled_builtins)
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        match self.native_for(provider_type) {
            Some(native) => native.config_fields(),
            None => self.plugins.config_fields_for_provider(provider_type),
        }
    }

    fn dynamic_config_trigger_keys_for_provider(&self, provider_type: &str) -> Vec<String> {
        match self.native_for(provider_type) {
            Some(native) => native.dynamic_config_trigger_keys(),
            None => self
                .plugins
                .dynamic_config_trigger_keys_for_provider(provider_type),
        }
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        match self.native_for(provider_type) {
            Some(native) => Some(native.display_name()),
            None => self.plugins.plugin_name_for_provider(provider_type),
        }
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        match self.native_for(provider_type) {
            Some(native) => Some(native.description()),
            None => self.plugins.plugin_description_for_provider(provider_type),
        }
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        match self.native_for(provider_type) {
            Some(native) => native.default_base_url(),
            None => self.plugins.default_base_url_for_provider(provider_type),
        }
    }

    fn rate_limit_seconds_for_provider(&self, provider_type: &str) -> Option<i64> {
        match self.native_for(provider_type) {
            Some(native) => native.rate_limit_seconds(),
            None => self.plugins.rate_limit_seconds_for_provider(provider_type),
        }
    }

    fn management_capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerManagementCapabilities {
        match self.native_for(provider_type) {
            Some(native) => native.management_capabilities(),
            None => self
                .plugins
                .management_capabilities_for_provider(provider_type),
        }
    }

    fn capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        match self.native_for(provider_type) {
            Some(native) => native.capabilities(),
            None => self.plugins.capabilities_for_provider(provider_type),
        }
    }
}

#[cfg(test)]
#[path = "indexer_provider_registry_tests.rs"]
mod tests;
