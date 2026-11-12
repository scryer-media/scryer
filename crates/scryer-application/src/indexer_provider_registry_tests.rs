use super::*;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};

use crate::{AppError, IndexerValidationResult};

const PLUGIN_TYPE: &str = "newznab";
const NATIVE_TYPE: &str = "hostnative";

/// Plugin provider double that overrides every trait method, so no call can
/// reach a trait default, and records each call with its arguments.
#[derive(Default)]
struct RecordingPlugins {
    calls: Mutex<Vec<String>>,
    runtime_loads: Mutex<Vec<Vec<String>>>,
    disabled: Mutex<Vec<Vec<String>>>,
}

impl RecordingPlugins {
    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("calls").push(call.into());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }

    fn record_reload(&self, runtime_plugins: &[RuntimePluginLoad], disabled: &[String]) {
        self.runtime_loads.lock().expect("loads").push(
            runtime_plugins
                .iter()
                .map(|plugin| plugin.descriptor.provider_type().to_string())
                .collect(),
        );
        self.disabled
            .lock()
            .expect("disabled")
            .push(disabled.to_vec());
    }
}

fn proxy_id(proxy_config: Option<&scryer_domain::ProxyConfig>) -> String {
    proxy_config.map_or_else(|| "-".to_string(), |proxy| proxy.id.clone())
}

fn accounting_id(accounting: Option<&IndexerAccountingContext>) -> String {
    accounting.map_or_else(|| "-".to_string(), |context| context.indexer_id.clone())
}

impl IndexerPluginProvider for RecordingPlugins {
    fn validate_config_for_provider(
        &self,
        provider_type: &str,
        config_json: &str,
    ) -> AppResult<()> {
        self.record(format!(
            "validate_config_for_provider:{provider_type}:{config_json}"
        ));
        Err(AppError::Validation("plugin rejected config".to_string()))
    }

    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        self.record(format!("client_for_provider:{}", config.provider_type));
        None
    }

    fn client_for_provider_with_proxy(
        &self,
        config: &IndexerConfig,
        proxy_config: Option<&scryer_domain::ProxyConfig>,
    ) -> Option<Arc<dyn IndexerClient>> {
        self.record(format!(
            "client_for_provider_with_proxy:{}:{}",
            config.provider_type,
            proxy_id(proxy_config)
        ));
        None
    }

    fn management_client_for_provider(
        &self,
        config: &IndexerConfig,
    ) -> Option<Arc<dyn IndexerManagementClient>> {
        self.record(format!(
            "management_client_for_provider:{}",
            config.provider_type
        ));
        None
    }

    fn client_for_provider_with_accounting(
        &self,
        config: &IndexerConfig,
        proxy_config: Option<&scryer_domain::ProxyConfig>,
        accounting: Option<&IndexerAccountingContext>,
    ) -> Option<Arc<dyn IndexerClient>> {
        self.record(format!(
            "client_for_provider_with_accounting:{}:{}:{}",
            config.provider_type,
            proxy_id(proxy_config),
            accounting_id(accounting)
        ));
        None
    }

    fn management_client_for_provider_with_accounting(
        &self,
        config: &IndexerConfig,
        accounting: Option<&IndexerAccountingContext>,
    ) -> Option<Arc<dyn IndexerManagementClient>> {
        self.record(format!(
            "management_client_for_provider_with_accounting:{}:{}",
            config.provider_type,
            accounting_id(accounting)
        ));
        None
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.record("available_provider_types");
        vec![
            "torznab".to_string(),
            PLUGIN_TYPE.to_string(),
            NATIVE_TYPE.to_string(),
            PLUGIN_TYPE.to_string(),
        ]
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        self.record("builtin_provider_types");
        vec![PLUGIN_TYPE.to_string(), NATIVE_TYPE.to_uppercase()]
    }

    fn builtin_descriptor_for_provider(
        &self,
        provider_type: &str,
    ) -> Option<scryer_plugin_sdk::PluginDescriptor> {
        self.record(format!("builtin_descriptor_for_provider:{provider_type}"));
        Some(runtime_plugin(provider_type).descriptor)
    }

    fn restore_builtin_plugin_with_settings(
        &self,
        provider_type: &str,
        settings: BTreeMap<String, String>,
    ) -> Result<(), String> {
        let settings = settings
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(",");
        self.record(format!(
            "restore_builtin_plugin_with_settings:{provider_type}:{settings}"
        ));
        Ok(())
    }

    fn reload_runtime_plugins_with_builtin_settings(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
        settings: &BTreeMap<String, BTreeMap<String, String>>,
    ) -> Result<(), String> {
        self.record(format!(
            "reload_runtime_plugins_with_builtin_settings:{}",
            settings.keys().cloned().collect::<Vec<_>>().join(",")
        ));
        self.record_reload(runtime_plugins, disabled_builtins);
        Ok(())
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.record(format!("plugin_version_for_provider:{provider_type}"));
        Some("1.2.3".to_string())
    }

    fn search_semantics_version_for_provider(&self, provider_type: &str) -> Option<u32> {
        self.record(format!(
            "search_semantics_version_for_provider:{provider_type}"
        ));
        Some(7)
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.record(format!("plugin_sdk_version_for_provider:{provider_type}"));
        Some("sdk-version".to_string())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        self.record(format!(
            "plugin_sdk_constraint_for_provider:{provider_type}"
        ));
        Some("sdk-constraint".to_string())
    }

    fn plugin_type_for_provider(&self, provider_type: &str) -> Option<String> {
        self.record(format!("plugin_type_for_provider:{provider_type}"));
        Some("plugin-indexer".to_string())
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        self.record("scoring_policies");
        Vec::new()
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        self.record(format!(
            "upsert_runtime_plugin:{}",
            plugin.descriptor.provider_type()
        ));
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        self.record(format!("remove_runtime_plugin:{provider_type}"));
        Ok(())
    }

    fn prepare_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        self.record(format!("prepare_builtin_plugin:{provider_type}"));
        Ok(())
    }

    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        self.record(format!("restore_builtin_plugin:{provider_type}"));
        Ok(())
    }

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.record(format!("reload_plugins:{}", external_wasm_bytes.len()));
        self.record_reload(&[], disabled_builtins);
        Ok(())
    }

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.record("reload_runtime_plugins");
        self.record_reload(runtime_plugins, disabled_builtins);
        Ok(())
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.record(format!("config_fields_for_provider:{provider_type}"));
        vec![config_field("plugin_field")]
    }

    fn dynamic_config_trigger_keys_for_provider(&self, provider_type: &str) -> Vec<String> {
        self.record(format!(
            "dynamic_config_trigger_keys_for_provider:{provider_type}"
        ));
        vec!["plugin_trigger".to_string()]
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        self.record(format!("plugin_name_for_provider:{provider_type}"));
        Some("Plugin Name".to_string())
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        self.record(format!("plugin_description_for_provider:{provider_type}"));
        Some("Plugin description".to_string())
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        self.record(format!("default_base_url_for_provider:{provider_type}"));
        Some("https://plugin.example.test".to_string())
    }

    fn rate_limit_seconds_for_provider(&self, provider_type: &str) -> Option<i64> {
        self.record(format!("rate_limit_seconds_for_provider:{provider_type}"));
        Some(11)
    }

    fn management_capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerManagementCapabilities {
        self.record(format!(
            "management_capabilities_for_provider:{provider_type}"
        ));
        scryer_domain::IndexerManagementCapabilities::default()
    }

    fn capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        self.record(format!("capabilities_for_provider:{provider_type}"));
        scryer_domain::IndexerProviderCapabilities {
            rss: true,
            ..Default::default()
        }
    }
}

struct NullSearchClient;

#[async_trait]
impl IndexerClient for NullSearchClient {
    async fn search(
        &self,
        _query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<crate::IndexerRoutingPlan>,
        _mode: crate::SearchMode,
        _operation: crate::IndexerErrorOperation,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<crate::IndexerSearchResponse> {
        unreachable!("registry tests never search")
    }
}

struct NamedManagementClient;

#[async_trait]
impl IndexerManagementClient for NamedManagementClient {
    async fn validate_connection(&self) -> AppResult<IndexerValidationResult> {
        unreachable!("registry tests never validate connections")
    }

    fn name(&self) -> &str {
        NATIVE_TYPE
    }
}

/// Native provider double. It records what reached it so tests can tell it
/// apart from the plugin provider.
struct TestNative {
    provider_type: String,
    calls: Mutex<Vec<String>>,
    client: Arc<dyn IndexerClient>,
}

impl TestNative {
    fn new(provider_type: &str) -> Self {
        Self {
            provider_type: provider_type.to_string(),
            calls: Mutex::new(Vec::new()),
            client: Arc::new(NullSearchClient),
        }
    }

    fn record(&self, call: impl Into<String>) {
        self.calls.lock().expect("calls").push(call.into());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }
}

impl NativeIndexerProvider for TestNative {
    fn provider_type(&self) -> &str {
        &self.provider_type
    }

    fn display_name(&self) -> String {
        "Native Name".to_string()
    }

    fn description(&self) -> String {
        "Native description".to_string()
    }

    fn config_fields(&self) -> Vec<scryer_domain::ConfigFieldDef> {
        vec![config_field("native_field")]
    }

    fn management_capabilities(&self) -> scryer_domain::IndexerManagementCapabilities {
        scryer_domain::IndexerManagementCapabilities {
            supports_validate_config: true,
            supports_managed_children_sync: true,
        }
    }

    fn capabilities(&self) -> scryer_domain::IndexerProviderCapabilities {
        scryer_domain::IndexerProviderCapabilities::default()
    }

    fn client(
        &self,
        config: &IndexerConfig,
        proxy_config: Option<&scryer_domain::ProxyConfig>,
        accounting: Option<&IndexerAccountingContext>,
    ) -> Option<Arc<dyn IndexerClient>> {
        self.record(format!(
            "client:{}:{}:{}",
            config.provider_type,
            proxy_id(proxy_config),
            accounting_id(accounting)
        ));
        Some(Arc::clone(&self.client))
    }

    fn management_client(
        &self,
        config: &IndexerConfig,
        accounting: Option<&IndexerAccountingContext>,
    ) -> Option<Arc<dyn IndexerManagementClient>> {
        self.record(format!(
            "management_client:{}:{}",
            config.provider_type,
            accounting_id(accounting)
        ));
        Some(Arc::new(NamedManagementClient))
    }

    fn validate_config(&self, config_json: &str) -> AppResult<()> {
        self.record(format!("validate_config:{config_json}"));
        Ok(())
    }

    fn default_base_url(&self) -> Option<String> {
        Some("https://native.example.test".to_string())
    }

    fn rate_limit_seconds(&self) -> Option<i64> {
        Some(5)
    }

    fn dynamic_config_trigger_keys(&self) -> Vec<String> {
        vec!["native_trigger".to_string()]
    }
}

fn config_field(key: &str) -> scryer_domain::ConfigFieldDef {
    scryer_domain::ConfigFieldDef {
        key: key.to_string(),
        label: key.to_string(),
        ..Default::default()
    }
}

fn indexer_config(provider_type: &str) -> IndexerConfig {
    let at = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
    IndexerConfig {
        id: "idx-1".to_string(),
        name: "Synthetic Indexer".to_string(),
        provider_type: provider_type.to_string(),
        base_url: "https://indexer.example.test".to_string(),
        api_key_encrypted: None,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        max_queries_per_minute: None,
        max_requests_per_month: None,
        max_grabs_per_month: None,
        disabled_until: None,
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        proxy_config_id: None,
        download_client_id: None,
        seeding_profile_id: None,
        managed_parent_config_id: None,
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        last_health_status: None,
        last_error_message: None,
        last_error_code: None,
        last_error_at: None,
        config_json: None,
        created_at: at,
        updated_at: at,
    }
}

fn proxy_config() -> scryer_domain::ProxyConfig {
    let at = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
    scryer_domain::ProxyConfig {
        id: "proxy-1".to_string(),
        name: "Synthetic Proxy".to_string(),
        provider_type: scryer_domain::ProxyProviderType::Http,
        protocol: None,
        base_url: "http://proxy.example.test:8080".to_string(),
        request_timeout_seconds: 30,
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
        created_at: at,
        updated_at: at,
    }
}

fn accounting() -> IndexerAccountingContext {
    IndexerAccountingContext {
        indexer_id: "acct-1".to_string(),
        indexer_name: "Accounting Indexer".to_string(),
    }
}

fn runtime_plugin(provider_type: &str) -> RuntimePluginLoad {
    RuntimePluginLoad {
        installation_id: Some(format!("installation-{provider_type}")),
        settings: Default::default(),
        descriptor: scryer_plugin_sdk::PluginDescriptor {
            settings: Vec::new(),
            id: format!("{provider_type}-plugin"),
            name: format!("{provider_type} Plugin"),
            version: "0.1.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions: vec![],
            provider: scryer_plugin_sdk::ProviderDescriptor::Indexer(
                scryer_plugin_sdk::IndexerDescriptor {
                    provider_type: provider_type.to_string(),
                    provider_aliases: Vec::new(),
                    provider_profiles: Vec::new(),
                    search_semantics_version: None,
                    strategy_plan: None,
                    source_kind: scryer_plugin_sdk::IndexerSourceKind::Generic,
                    capabilities: Default::default(),
                    scoring_policies: vec![],
                    config_fields: Vec::new(),
                    allowed_hosts: vec![],
                    rate_limit_seconds: None,
                },
            ),
        },
        wasm_bytes: vec![1, 2, 3, 4],
        first_party: true,
    }
}

fn registry() -> (
    IndexerProviderRegistry,
    Arc<RecordingPlugins>,
    Arc<TestNative>,
) {
    let plugins = Arc::new(RecordingPlugins::default());
    let native = Arc::new(TestNative::new(NATIVE_TYPE));
    let registry = IndexerProviderRegistry::new(plugins.clone()).with_native(native.clone());
    (registry, plugins, native)
}

#[test]
fn plugin_type_queries_reach_the_plugin_provider() {
    let (registry, plugins, native) = registry();

    assert!(
        registry
            .validate_config_for_provider(PLUGIN_TYPE, "{\"k\":1}")
            .is_err(),
        "the plugin provider's validation verdict must come back"
    );
    assert_eq!(
        registry.config_fields_for_provider(PLUGIN_TYPE)[0].key,
        "plugin_field"
    );
    assert_eq!(
        registry.dynamic_config_trigger_keys_for_provider(PLUGIN_TYPE),
        vec!["plugin_trigger".to_string()]
    );
    assert_eq!(
        registry.plugin_name_for_provider(PLUGIN_TYPE).as_deref(),
        Some("Plugin Name")
    );
    assert_eq!(
        registry
            .plugin_description_for_provider(PLUGIN_TYPE)
            .as_deref(),
        Some("Plugin description")
    );
    assert_eq!(
        registry
            .default_base_url_for_provider(PLUGIN_TYPE)
            .as_deref(),
        Some("https://plugin.example.test")
    );
    assert_eq!(
        registry.rate_limit_seconds_for_provider(PLUGIN_TYPE),
        Some(11)
    );
    assert_eq!(
        registry.management_capabilities_for_provider(PLUGIN_TYPE),
        scryer_domain::IndexerManagementCapabilities::default()
    );
    assert!(registry.capabilities_for_provider(PLUGIN_TYPE).rss);
    assert_eq!(
        registry.plugin_version_for_provider(PLUGIN_TYPE).as_deref(),
        Some("1.2.3")
    );
    assert_eq!(
        registry.search_semantics_version_for_provider(PLUGIN_TYPE),
        Some(7)
    );
    assert_eq!(
        registry
            .plugin_sdk_version_for_provider(PLUGIN_TYPE)
            .as_deref(),
        Some("sdk-version")
    );
    assert_eq!(
        registry
            .plugin_sdk_constraint_for_provider(PLUGIN_TYPE)
            .as_deref(),
        Some("sdk-constraint")
    );
    assert_eq!(
        registry.plugin_type_for_provider(PLUGIN_TYPE).as_deref(),
        Some("plugin-indexer")
    );
    assert!(
        registry
            .builtin_descriptor_for_provider(PLUGIN_TYPE)
            .is_some()
    );
    assert!(registry.scoring_policies().is_empty());

    assert_eq!(
        plugins.calls(),
        vec![
            "validate_config_for_provider:newznab:{\"k\":1}",
            "config_fields_for_provider:newznab",
            "dynamic_config_trigger_keys_for_provider:newznab",
            "plugin_name_for_provider:newznab",
            "plugin_description_for_provider:newznab",
            "default_base_url_for_provider:newznab",
            "rate_limit_seconds_for_provider:newznab",
            "management_capabilities_for_provider:newznab",
            "capabilities_for_provider:newznab",
            "plugin_version_for_provider:newznab",
            "search_semantics_version_for_provider:newznab",
            "plugin_sdk_version_for_provider:newznab",
            "plugin_sdk_constraint_for_provider:newznab",
            "plugin_type_for_provider:newznab",
            "builtin_descriptor_for_provider:newznab",
            "scoring_policies",
        ]
    );
    assert!(native.calls().is_empty());
}

#[test]
fn plugin_type_clients_reach_the_same_named_plugin_method_with_arguments_intact() {
    let (registry, plugins, native) = registry();
    let config = indexer_config(PLUGIN_TYPE);
    let proxy = proxy_config();
    let accounting = accounting();

    assert!(registry.client_for_provider(&config).is_none());
    assert!(
        registry
            .client_for_provider_with_proxy(&config, Some(&proxy))
            .is_none()
    );
    assert!(
        registry
            .client_for_provider_with_accounting(&config, Some(&proxy), Some(&accounting))
            .is_none()
    );
    assert!(registry.management_client_for_provider(&config).is_none());
    assert!(
        registry
            .management_client_for_provider_with_accounting(&config, Some(&accounting))
            .is_none()
    );

    assert_eq!(
        plugins.calls(),
        vec![
            "client_for_provider:newznab",
            "client_for_provider_with_proxy:newznab:proxy-1",
            "client_for_provider_with_accounting:newznab:proxy-1:acct-1",
            "management_client_for_provider:newznab",
            "management_client_for_provider_with_accounting:newznab:acct-1",
        ]
    );
    assert!(native.calls().is_empty());
}

#[test]
fn native_type_queries_reach_the_native_provider_and_never_the_plugin_provider() {
    let (registry, plugins, native) = registry();
    // Matching trims and ignores case, like provider types everywhere else.
    let native_type = " HostNative ";

    registry
        .validate_config_for_provider(native_type, "{}")
        .expect("native validation accepts");
    assert_eq!(
        registry.config_fields_for_provider(native_type)[0].key,
        "native_field"
    );
    assert_eq!(
        registry.dynamic_config_trigger_keys_for_provider(native_type),
        vec!["native_trigger".to_string()]
    );
    assert_eq!(
        registry.plugin_name_for_provider(native_type).as_deref(),
        Some("Native Name")
    );
    assert_eq!(
        registry
            .plugin_description_for_provider(native_type)
            .as_deref(),
        Some("Native description")
    );
    assert_eq!(
        registry
            .default_base_url_for_provider(native_type)
            .as_deref(),
        Some("https://native.example.test")
    );
    assert_eq!(
        registry.rate_limit_seconds_for_provider(native_type),
        Some(5)
    );
    assert!(
        registry
            .management_capabilities_for_provider(native_type)
            .supports_managed_children_sync
    );
    assert_eq!(
        registry.capabilities_for_provider(native_type),
        scryer_domain::IndexerProviderCapabilities::default()
    );
    assert_eq!(registry.plugin_version_for_provider(native_type), None);
    assert_eq!(
        registry.search_semantics_version_for_provider(native_type),
        None
    );
    assert_eq!(registry.plugin_sdk_version_for_provider(native_type), None);
    assert_eq!(
        registry.plugin_sdk_constraint_for_provider(native_type),
        None
    );
    assert_eq!(
        registry.plugin_type_for_provider(native_type).as_deref(),
        Some("indexer")
    );
    assert!(
        registry
            .builtin_descriptor_for_provider(native_type)
            .is_none()
    );

    assert!(plugins.calls().is_empty());
    assert_eq!(native.calls(), vec!["validate_config:{}"]);
}

#[test]
fn native_type_clients_reach_the_native_provider_and_never_the_plugin_provider() {
    let (registry, plugins, native) = registry();
    let config = indexer_config(NATIVE_TYPE);
    let proxy = proxy_config();
    let accounting = accounting();

    let client = registry
        .client_for_provider(&config)
        .expect("native search client");
    assert!(Arc::ptr_eq(&client, &native.client));
    assert!(
        registry
            .client_for_provider_with_proxy(&config, Some(&proxy))
            .is_some()
    );
    assert!(
        registry
            .client_for_provider_with_accounting(&config, Some(&proxy), Some(&accounting))
            .is_some()
    );
    let management = registry
        .management_client_for_provider(&config)
        .expect("native management client");
    assert_eq!(management.name(), NATIVE_TYPE);
    assert!(
        registry
            .management_client_for_provider_with_accounting(&config, Some(&accounting))
            .is_some()
    );

    assert!(plugins.calls().is_empty());
    // Variants without an explicit accounting context derive it from the
    // config, as the plugin provider does.
    assert_eq!(
        native.calls(),
        vec![
            "client:hostnative:-:idx-1",
            "client:hostnative:proxy-1:idx-1",
            "client:hostnative:proxy-1:acct-1",
            "management_client:hostnative:idx-1",
            "management_client:hostnative:acct-1",
        ]
    );
}

#[test]
fn plugin_type_lifecycle_reaches_the_plugin_provider() {
    let (registry, plugins, _native) = registry();
    let settings = BTreeMap::from([("mode".to_string(), "strict".to_string())]);

    registry
        .upsert_runtime_plugin(runtime_plugin(PLUGIN_TYPE))
        .expect("upsert");
    registry.remove_runtime_plugin(PLUGIN_TYPE).expect("remove");
    registry
        .prepare_builtin_plugin(PLUGIN_TYPE)
        .expect("prepare");
    registry
        .restore_builtin_plugin(PLUGIN_TYPE)
        .expect("restore");
    registry
        .restore_builtin_plugin_with_settings(PLUGIN_TYPE, settings)
        .expect("restore with settings");

    assert_eq!(
        plugins.calls(),
        vec![
            "upsert_runtime_plugin:newznab",
            "remove_runtime_plugin:newznab",
            "prepare_builtin_plugin:newznab",
            "restore_builtin_plugin:newznab",
            "restore_builtin_plugin_with_settings:newznab:mode=strict",
        ]
    );
}

#[test]
fn native_type_lifecycle_succeeds_without_touching_the_plugin_provider() {
    let (registry, plugins, native) = registry();
    // Lifecycle dispatch trims and ignores case like every other decision.
    let native_type = " HostNative ";

    registry
        .upsert_runtime_plugin(runtime_plugin(native_type))
        .expect("a plugin claiming a native type is dropped, not refused");
    registry.remove_runtime_plugin(native_type).expect("remove");
    registry
        .prepare_builtin_plugin(native_type)
        .expect("prepare");
    registry
        .restore_builtin_plugin(native_type)
        .expect("restore");
    registry
        .restore_builtin_plugin_with_settings(
            native_type,
            BTreeMap::from([("mode".to_string(), "strict".to_string())]),
        )
        .expect("restore with settings");

    assert!(plugins.calls().is_empty());
    assert!(native.calls().is_empty());
}

#[test]
fn reloads_drop_runtime_plugins_and_disabled_builtins_claiming_a_native_type() {
    let (registry, plugins, _native) = registry();
    let runtime = vec![runtime_plugin(PLUGIN_TYPE), runtime_plugin(" HostNative ")];
    let disabled = vec!["torznab".to_string(), NATIVE_TYPE.to_uppercase()];

    registry
        .reload_runtime_plugins(&runtime, &disabled)
        .expect("reload");
    registry
        .reload_runtime_plugins_with_builtin_settings(
            &runtime,
            &disabled,
            &BTreeMap::from([(PLUGIN_TYPE.to_string(), BTreeMap::new())]),
        )
        .expect("reload with settings");
    registry
        .reload_plugins(&[], &disabled)
        .expect("reload plugins");

    assert_eq!(
        plugins.calls(),
        vec![
            "reload_runtime_plugins",
            "reload_runtime_plugins_with_builtin_settings:newznab",
            "reload_plugins:0",
        ]
    );
    assert_eq!(
        *plugins.runtime_loads.lock().expect("loads"),
        vec![
            vec![PLUGIN_TYPE.to_string()],
            vec![PLUGIN_TYPE.to_string()],
            Vec::new(),
        ]
    );
    assert_eq!(
        *plugins.disabled.lock().expect("disabled"),
        vec![vec!["torznab".to_string()]; 3]
    );
}

#[test]
fn provider_type_listings_merge_native_types_and_keep_builtins_plugin_only() {
    let (registry, _plugins, _native) = registry();

    assert_eq!(
        registry.available_provider_types(),
        vec![
            NATIVE_TYPE.to_string(),
            PLUGIN_TYPE.to_string(),
            "torznab".to_string(),
        ]
    );
    assert_eq!(
        registry.builtin_provider_types(),
        vec![PLUGIN_TYPE.to_string()]
    );
}

#[test]
fn a_second_native_provider_for_a_claimed_type_is_ignored() {
    let plugins = Arc::new(RecordingPlugins::default());
    let first = Arc::new(TestNative::new(NATIVE_TYPE));
    let second = Arc::new(TestNative::new(&NATIVE_TYPE.to_uppercase()));
    let registry = IndexerProviderRegistry::new(plugins)
        .with_native(first.clone())
        .with_native(second.clone());

    registry
        .validate_config_for_provider(NATIVE_TYPE, "{}")
        .expect("validate");

    assert_eq!(first.calls(), vec!["validate_config:{}"]);
    assert!(second.calls().is_empty());
}
