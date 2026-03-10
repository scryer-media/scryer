use super::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Mutex;

// ── Mock: PluginInstallationRepository ───────────────────────────────────────

struct MockPluginInstallationRepo {
    installations: Arc<Mutex<Vec<PluginInstallation>>>,
    registry_cache: Arc<Mutex<Option<String>>>,
    seeded: Arc<Mutex<Vec<SeededPluginRecord>>>,
}

type SeededPluginRecord = (String, String, String, String, String);

impl MockPluginInstallationRepo {
    fn new() -> Self {
        Self {
            installations: Arc::new(Mutex::new(vec![])),
            registry_cache: Arc::new(Mutex::new(None)),
            seeded: Arc::new(Mutex::new(vec![])),
        }
    }
}

#[async_trait]
impl PluginInstallationRepository for MockPluginInstallationRepo {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>> {
        Ok(self.installations.lock().await.clone())
    }

    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>> {
        let list = self.installations.lock().await;
        Ok(list.iter().find(|i| i.plugin_id == plugin_id).cloned())
    }

    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        _wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let mut list = self.installations.lock().await;
        list.push(installation.clone());
        Ok(installation.clone())
    }

    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        _wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let mut list = self.installations.lock().await;
        if let Some(existing) = list
            .iter_mut()
            .find(|i| i.plugin_id == installation.plugin_id)
        {
            *existing = installation.clone();
        }
        Ok(installation.clone())
    }

    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()> {
        let mut list = self.installations.lock().await;
        list.retain(|i| i.plugin_id != plugin_id);
        Ok(())
    }

    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<Vec<u8>>)>> {
        let list = self.installations.lock().await;
        Ok(list
            .iter()
            .filter(|i| i.is_enabled)
            .map(|i| (i.clone(), None))
            .collect())
    }

    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        self.seeded.lock().await.push((
            plugin_id.to_string(),
            name.to_string(),
            description.to_string(),
            version.to_string(),
            provider_type.to_string(),
        ));
        Ok(())
    }

    async fn store_registry_cache(&self, json: &str) -> AppResult<()> {
        *self.registry_cache.lock().await = Some(json.to_string());
        Ok(())
    }

    async fn get_registry_cache(&self) -> AppResult<Option<String>> {
        Ok(self.registry_cache.lock().await.clone())
    }
}

// ── Mock: IndexerConfigRepository ────────────────────────────────────────────

struct MockIndexerConfigRepo {
    store: Arc<Mutex<Vec<IndexerConfig>>>,
}

impl MockIndexerConfigRepo {
    fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(vec![])),
        }
    }
}

#[async_trait]
impl IndexerConfigRepository for MockIndexerConfigRepo {
    async fn list(&self, provider_filter: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .filter(|e| {
                provider_filter
                    .as_ref()
                    .is_none_or(|pf| pf == &e.provider_type)
            })
            .cloned()
            .collect())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
        let entries = self.store.lock().await;
        Ok(entries.iter().find(|e| e.id == id).cloned())
    }

    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        self.store.lock().await.push(config.clone());
        Ok(config)
    }

    async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
        Ok(())
    }

    async fn update(
        &self,
        _id: &str,
        _name: Option<String>,
        _provider_type: Option<String>,
        _base_url: Option<String>,
        _api_key_encrypted: Option<String>,
        _rate_limit_seconds: Option<i64>,
        _rate_limit_burst: Option<i64>,
        _is_enabled: Option<bool>,
        _enable_interactive_search: Option<bool>,
        _enable_auto_search: Option<bool>,
        _config_json: Option<String>,
    ) -> AppResult<IndexerConfig> {
        Err(AppError::Repository("not implemented".into()))
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.store.lock().await.retain(|e| e.id != id);
        Ok(())
    }
}

// ── Mock: IndexerPluginProvider ──────────────────────────────────────────────

struct MockPluginProvider {
    types: Vec<String>,
    default_urls: HashMap<String, String>,
    plugin_names: HashMap<String, String>,
    reload_count: AtomicUsize,
}

impl MockPluginProvider {
    fn new() -> Self {
        Self {
            types: vec![],
            default_urls: HashMap::new(),
            plugin_names: HashMap::new(),
            reload_count: AtomicUsize::new(0),
        }
    }

    fn with_provider(mut self, pt: &str, name: &str, default_url: Option<&str>) -> Self {
        self.types.push(pt.to_string());
        self.plugin_names.insert(pt.to_string(), name.to_string());
        if let Some(url) = default_url {
            self.default_urls.insert(pt.to_string(), url.to_string());
        }
        self
    }
}

impl IndexerPluginProvider for MockPluginProvider {
    fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        None
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.types.clone()
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        vec![]
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[&[u8]],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        self.plugin_names.get(provider_type).cloned()
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        self.default_urls.get(provider_type).cloned()
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn admin() -> User {
    User::new_admin("admin")
}

fn viewer() -> User {
    User {
        id: scryer_domain::Id::new().0,
        username: "viewer".to_string(),
        password_hash: None,
        entitlements: vec![scryer_domain::Entitlement::ViewCatalog],
    }
}

fn make_installation(
    plugin_id: &str,
    version: &str,
    builtin: bool,
    enabled: bool,
) -> PluginInstallation {
    let now = Utc::now();
    PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: plugin_id.to_string(),
        name: format!("{plugin_id} Plugin"),
        description: format!("Description for {plugin_id}"),
        version: version.to_string(),
        plugin_type: "indexer".to_string(),
        provider_type: plugin_id.to_string(),
        is_enabled: enabled,
        is_builtin: builtin,
        wasm_sha256: None,
        source_url: None,
        installed_at: now,
        updated_at: now,
    }
}

fn make_registry_json(entries: &[serde_json::Value]) -> String {
    serde_json::json!({
        "schema_version": 1,
        "plugins": entries
    })
    .to_string()
}

fn registry_entry(
    id: &str,
    version: &str,
    builtin: bool,
    wasm_url: Option<&str>,
) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "id": id,
        "name": format!("{id} Plugin"),
        "description": format!("Description for {id}"),
        "plugin_type": "indexer",
        "provider_type": id,
        "version": version,
        "official": true,
        "builtin": builtin,
    });
    if let Some(url) = wasm_url {
        entry["wasm_url"] = serde_json::json!(url);
        entry["wasm_sha256"] = serde_json::json!("abc123");
    }
    entry
}

fn make_indexer_config(provider_type: &str) -> IndexerConfig {
    let now = Utc::now();
    IndexerConfig {
        id: scryer_domain::Id::new().0,
        name: format!("{provider_type} config"),
        provider_type: provider_type.to_string(),
        base_url: "https://example.com".to_string(),
        api_key_encrypted: None,
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        disabled_until: None,
        last_health_status: None,
        last_error_at: None,
        config_json: None,
        created_at: now,
        updated_at: now,
    }
}

struct TestHarness {
    app: AppUseCase,
    plugin_repo: Arc<MockPluginInstallationRepo>,
    indexer_config_repo: Arc<MockIndexerConfigRepo>,
}

fn bootstrap_plugins(provider: Option<MockPluginProvider>) -> TestHarness {
    use crate::null_repositories::test_nulls::*;
    use crate::null_repositories::NullSettingsRepository;
    use crate::types::JwtAuthConfig;

    let plugin_repo = Arc::new(MockPluginInstallationRepo::new());
    let indexer_config_repo = Arc::new(MockIndexerConfigRepo::new());

    let mut services = AppServices::with_default_channels(
        Arc::new(NullTitleRepository),
        Arc::new(NullShowRepository),
        Arc::new(NullUserRepository),
        Arc::new(NullEventRepository),
        indexer_config_repo.clone() as Arc<dyn IndexerConfigRepository>,
        Arc::new(NullIndexerClient),
        Arc::new(NullDownloadClient),
        Arc::new(NullDownloadClientConfigRepository),
        Arc::new(NullReleaseAttemptRepository),
        Arc::new(NullSettingsRepository),
        Arc::new(NullQualityProfileRepository),
        String::new(),
    );
    services.plugin_installations = plugin_repo.clone();
    if let Some(p) = provider {
        services.plugin_provider = Some(Arc::new(p));
    }

    let registry = FacetRegistry::new();
    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "test".to_string(),
            access_ttl_seconds: 3600,
            jwt_hmac_secret: "dGVzdC1zZWNyZXQtZm9yLXVuaXQtdGVzdHMtb25seS0zMmJ5dGVzISE=".to_string(),
        },
        Arc::new(registry),
    );

    TestHarness {
        app,
        plugin_repo,
        indexer_config_repo,
    }
}

// ── RegistryManifest serde ───────────────────────────────────────────────────

#[test]
fn registry_manifest_deserialize_with_defaults() {
    let json = r#"{
        "schema_version": 1,
        "plugins": [{
            "id": "test",
            "name": "Test",
            "description": "A test plugin",
            "plugin_type": "indexer",
            "provider_type": "test",
            "version": "0.1.0"
        }]
    }"#;
    let manifest: RegistryManifest = serde_json::from_str(json).unwrap();
    assert_eq!(manifest.plugins.len(), 1);
    let entry = &manifest.plugins[0];
    assert_eq!(entry.id, "test");
    assert_eq!(entry.author, ""); // default
    assert!(!entry.official); // default false
    assert!(!entry.builtin); // default false
    assert!(entry.source_url.is_none());
    assert!(entry.wasm_url.is_none());
    assert!(entry.wasm_sha256.is_none());
    assert!(entry.min_scryer_version.is_none());
}

// ── list_available_plugins ───────────────────────────────────────────────────

#[tokio::test]
async fn list_empty_registry_empty_installations() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn list_registry_entries_not_installed() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_registry_json(&[
        registry_entry("alpha", "1.0.0", false, Some("https://example.com/a.wasm")),
        registry_entry("beta", "2.0.0", false, Some("https://example.com/b.wasm")),
    ]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(result.len(), 2);
    for p in &result {
        assert!(!p.is_installed);
        assert!(!p.is_enabled);
        assert!(p.installed_version.is_none());
        assert!(!p.update_available);
    }
}

#[tokio::test]
async fn list_installed_and_in_registry() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_registry_json(&[registry_entry(
        "alpha",
        "0.2.0",
        false,
        Some("https://example.com/a.wasm"),
    )]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(result.len(), 1);
    let p = &result[0];
    assert!(p.is_installed);
    assert!(p.is_enabled);
    assert_eq!(p.installed_version.as_deref(), Some("0.1.0"));
    assert!(p.update_available);
}

#[tokio::test]
async fn list_installed_at_latest() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_registry_json(&[registry_entry("alpha", "0.1.0", false, None)]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert!(!result[0].update_available);
}

#[tokio::test]
async fn list_installed_ahead_of_registry() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_registry_json(&[registry_entry("alpha", "0.1.0", false, None)]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.2.0", false, true));

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert!(!result[0].update_available);
}

#[tokio::test]
async fn list_installed_not_in_registry() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("manual", "1.0.0", false, true));

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(result.len(), 1);
    let p = &result[0];
    assert!(p.is_installed);
    assert!(!p.official);
    assert!(p.wasm_url.is_none());
    assert!(!p.update_available);
}

#[tokio::test]
async fn list_merge_both_sources() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_registry_json(&[
        registry_entry("alpha", "1.0.0", false, None),
        registry_entry("beta", "1.0.0", false, None),
    ]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();
    {
        let mut list = h.plugin_repo.installations.lock().await;
        list.push(make_installation("alpha", "1.0.0", false, true));
        list.push(make_installation("gamma", "1.0.0", false, false));
    }

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(result.len(), 3);
    let alpha = result.iter().find(|p| p.id == "alpha").unwrap();
    let beta = result.iter().find(|p| p.id == "beta").unwrap();
    let gamma = result.iter().find(|p| p.id == "gamma").unwrap();
    assert!(alpha.is_installed);
    assert!(!beta.is_installed);
    assert!(gamma.is_installed);
    assert!(!gamma.official);
}

#[tokio::test]
async fn list_invalid_registry_json_fallback() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .store_registry_cache("not valid json!!!")
        .await
        .unwrap();

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn list_invalid_semver_no_update() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_registry_json(&[registry_entry("alpha", "not-a-version", false, None)]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert!(!result[0].update_available);
}

#[tokio::test]
async fn list_default_base_url_from_provider() {
    let provider = MockPluginProvider::new().with_provider(
        "animetosho",
        "AnimeTosho",
        Some("https://feed.animetosho.org"),
    );
    let h = bootstrap_plugins(Some(provider));
    let json = make_registry_json(&[registry_entry("animetosho", "0.1.0", false, None)]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(
        result[0].default_base_url.as_deref(),
        Some("https://feed.animetosho.org")
    );
}

#[tokio::test]
async fn list_auth_rejects_viewer() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h.app.list_available_plugins(&viewer()).await.unwrap_err();
    assert!(matches!(err, AppError::Unauthorized(_)));
}

// ── toggle_plugin ────────────────────────────────────────────────────────────

#[tokio::test]
async fn toggle_enables_disabled_plugin() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "1.0.0", false, false));

    let result = h.app.toggle_plugin(&admin(), "alpha", true).await.unwrap();
    assert!(result.is_enabled);

    let stored = h
        .plugin_repo
        .get_plugin_installation("alpha")
        .await
        .unwrap()
        .unwrap();
    assert!(stored.is_enabled);
}

#[tokio::test]
async fn toggle_disables_enabled_plugin() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "1.0.0", false, true));

    let result = h.app.toggle_plugin(&admin(), "alpha", false).await.unwrap();
    assert!(!result.is_enabled);
}

#[tokio::test]
async fn toggle_not_found() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h
        .app
        .toggle_plugin(&admin(), "nonexistent", true)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn toggle_auth_rejects_viewer() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h
        .app
        .toggle_plugin(&viewer(), "alpha", true)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Unauthorized(_)));
}

// ── uninstall_plugin ─────────────────────────────────────────────────────────

#[tokio::test]
async fn uninstall_success() {
    let provider = MockPluginProvider::new();
    let h = bootstrap_plugins(Some(provider));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "1.0.0", false, true));

    h.app.uninstall_plugin(&admin(), "alpha").await.unwrap();

    let remaining = h.plugin_repo.list_plugin_installations().await.unwrap();
    assert!(remaining.is_empty());
}

#[tokio::test]
async fn uninstall_deletes_indexer_configs() {
    let provider = MockPluginProvider::new();
    let h = bootstrap_plugins(Some(provider));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "1.0.0", false, true));
    {
        let mut configs = h.indexer_config_repo.store.lock().await;
        configs.push(make_indexer_config("alpha"));
        configs.push(make_indexer_config("alpha"));
    }

    h.app.uninstall_plugin(&admin(), "alpha").await.unwrap();

    let configs = h.indexer_config_repo.store.lock().await;
    assert!(
        configs.is_empty(),
        "indexer configs should be deleted on uninstall"
    );
}

#[tokio::test]
async fn uninstall_not_found() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h
        .app
        .uninstall_plugin(&admin(), "nonexistent")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn uninstall_builtin_rejected() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("nzbgeek", "0.2.0", true, true));

    let err = h
        .app
        .uninstall_plugin(&admin(), "nzbgeek")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
    match err {
        AppError::Validation(msg) => {
            assert!(msg.contains("disable"), "expected 'disable' hint: {msg}")
        }
        _ => panic!("expected Validation error"),
    }
}

#[tokio::test]
async fn uninstall_auth_rejects_viewer() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h
        .app
        .uninstall_plugin(&viewer(), "alpha")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Unauthorized(_)));
}

// ── install_plugin error paths ───────────────────────────────────────────────

#[tokio::test]
async fn install_registry_not_loaded() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h.app.install_plugin(&admin(), "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
    match err {
        AppError::Validation(msg) => assert!(msg.contains("registry not loaded")),
        _ => panic!("expected Validation"),
    }
}

#[tokio::test]
async fn install_not_in_registry() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_registry_json(&[registry_entry(
        "beta",
        "1.0.0",
        false,
        Some("https://example.com/b.wasm"),
    )]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();

    let err = h.app.install_plugin(&admin(), "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn install_builtin_rejected() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_registry_json(&[registry_entry("nzbgeek", "0.2.0", true, None)]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();

    let err = h.app.install_plugin(&admin(), "nzbgeek").await.unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn install_no_wasm_url() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_registry_json(&[registry_entry("alpha", "1.0.0", false, None)]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();

    let err = h.app.install_plugin(&admin(), "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
    match err {
        AppError::Validation(msg) => assert!(msg.contains("no wasm_url")),
        _ => panic!("expected Validation"),
    }
}

#[tokio::test]
async fn install_auth_rejects_viewer() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h.app.install_plugin(&viewer(), "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::Unauthorized(_)));
}

// ── upgrade_plugin error paths ───────────────────────────────────────────────

#[tokio::test]
async fn upgrade_not_found() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h
        .app
        .upgrade_plugin(&admin(), "nonexistent")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn upgrade_builtin_rejected() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("nzbgeek", "0.2.0", true, true));

    let err = h.app.upgrade_plugin(&admin(), "nzbgeek").await.unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn upgrade_registry_not_loaded() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));

    let err = h.app.upgrade_plugin(&admin(), "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
    match err {
        AppError::Validation(msg) => assert!(msg.contains("registry not loaded")),
        _ => panic!("expected Validation"),
    }
}

#[tokio::test]
async fn upgrade_not_in_registry() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));
    let json = make_registry_json(&[registry_entry("beta", "1.0.0", false, None)]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();

    let err = h.app.upgrade_plugin(&admin(), "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn upgrade_already_at_latest() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.2.0", false, true));
    let json = make_registry_json(&[registry_entry(
        "alpha",
        "0.2.0",
        false,
        Some("https://example.com/a.wasm"),
    )]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();

    let err = h.app.upgrade_plugin(&admin(), "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
    match err {
        AppError::Validation(msg) => assert!(msg.contains("already at version")),
        _ => panic!("expected Validation"),
    }
}

#[tokio::test]
async fn upgrade_no_wasm_url() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));
    let json = make_registry_json(&[registry_entry("alpha", "0.2.0", false, None)]);
    h.plugin_repo.store_registry_cache(&json).await.unwrap();

    let err = h.app.upgrade_plugin(&admin(), "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
    match err {
        AppError::Validation(msg) => assert!(msg.contains("no wasm_url")),
        _ => panic!("expected Validation"),
    }
}

// ── seed_builtin_plugins ─────────────────────────────────────────────────────

#[tokio::test]
async fn seed_calls_for_nzbgeek_and_newznab() {
    let h = bootstrap_plugins(None);
    h.app.seed_builtin_plugins().await.unwrap();

    let seeded = h.plugin_repo.seeded.lock().await;
    assert_eq!(seeded.len(), 2);

    let ids: Vec<&str> = seeded.iter().map(|(id, _, _, _, _)| id.as_str()).collect();
    assert!(ids.contains(&"nzbgeek"));
    assert!(ids.contains(&"newznab"));
}

// ── reconcile_indexer_configs ────────────────────────────────────────────────

#[tokio::test]
async fn reconcile_creates_config_for_default_url_plugin() {
    let provider = MockPluginProvider::new().with_provider(
        "animetosho",
        "AnimeTosho",
        Some("https://feed.animetosho.org"),
    );
    let h = bootstrap_plugins(Some(provider));

    h.app.reconcile_indexer_configs().await.unwrap();

    let configs = h.indexer_config_repo.store.lock().await;
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].provider_type, "animetosho");
    assert_eq!(configs[0].base_url, "https://feed.animetosho.org");
    assert!(configs[0].is_enabled);
}

#[tokio::test]
async fn reconcile_skips_when_config_exists() {
    let provider = MockPluginProvider::new().with_provider(
        "animetosho",
        "AnimeTosho",
        Some("https://feed.animetosho.org"),
    );
    let h = bootstrap_plugins(Some(provider));
    h.indexer_config_repo
        .store
        .lock()
        .await
        .push(make_indexer_config("animetosho"));

    h.app.reconcile_indexer_configs().await.unwrap();

    let configs = h.indexer_config_repo.store.lock().await;
    assert_eq!(configs.len(), 1, "should not create duplicate");
}

#[tokio::test]
async fn reconcile_skips_without_default_url() {
    let provider = MockPluginProvider::new().with_provider("newznab", "Newznab", None);
    let h = bootstrap_plugins(Some(provider));

    h.app.reconcile_indexer_configs().await.unwrap();

    let configs = h.indexer_config_repo.store.lock().await;
    assert!(configs.is_empty());
}

#[tokio::test]
async fn reconcile_noop_without_plugin_provider() {
    let h = bootstrap_plugins(None);

    h.app.reconcile_indexer_configs().await.unwrap();

    let configs = h.indexer_config_repo.store.lock().await;
    assert!(configs.is_empty());
}
