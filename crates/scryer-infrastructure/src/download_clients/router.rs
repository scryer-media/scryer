use std::sync::Arc;

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest, DownloadClientConfigRepository,
    DownloadClientPluginProvider, DownloadGrabResult, DownloadSourceKind, SettingsRepository,
};
use scryer_domain::{DownloadClientConfig, DownloadQueueItem, MediaFacet};
use tracing::warn;

use super::nzbget::NzbgetDownloadClient;
use super::sabnzbd::SabnzbdDownloadClient;
use super::weaver::WeaverDownloadClient;
use super::{
    parse_download_client_config_json, read_config_string, resolve_download_client_base_url,
};

#[derive(Clone)]
pub struct PrioritizedDownloadClientRouter {
    download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    settings: Arc<dyn SettingsRepository>,
    fallback_client: Arc<dyn DownloadClient>,
    plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
}

impl PrioritizedDownloadClientRouter {
    pub fn new(
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        settings: Arc<dyn SettingsRepository>,
        fallback_client: Arc<dyn DownloadClient>,
        plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
    ) -> Self {
        Self {
            download_client_configs,
            settings,
            fallback_client,
            plugin_provider,
        }
    }

    async fn list_enabled_clients_by_priority(&self) -> AppResult<Vec<DownloadClientConfig>> {
        let mut clients = self
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .collect::<Vec<_>>();
        clients.sort_by_key(|config| config.client_priority);
        Ok(clients)
    }

    fn request_source_kind(request: &DownloadClientAddRequest) -> Option<DownloadSourceKind> {
        request
            .source_kind
            .or_else(|| DownloadSourceKind::infer_from_hint(request.source_hint.as_deref()))
            .or_else(|| {
                request
                    .info_hash_hint
                    .as_ref()
                    .map(|_| DownloadSourceKind::TorrentFile)
            })
    }

    fn source_kind_label(kind: DownloadSourceKind) -> &'static str {
        match kind {
            DownloadSourceKind::NzbUrl => "NZB",
            DownloadSourceKind::TorrentFile => "torrent file",
            DownloadSourceKind::MagnetUri => "magnet",
        }
    }

    fn config_accepts_source_kind(
        config: &DownloadClientConfig,
        source_kind: DownloadSourceKind,
        plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
    ) -> bool {
        let client_type = config.client_type.trim().to_ascii_lowercase();
        match client_type.as_str() {
            "nzbget" | "sabnzbd" | "weaver" => source_kind == DownloadSourceKind::NzbUrl,
            _ => {
                let accepted_inputs = plugin_provider
                    .map(|provider| provider.accepted_inputs_for_provider(&config.client_type))
                    .unwrap_or_default();
                if accepted_inputs.is_empty() {
                    return true;
                }
                accepted_inputs.iter().any(|input| {
                    DownloadSourceKind::parse(input)
                        .is_some_and(|accepted_kind| accepted_kind == source_kind)
                })
            }
        }
    }

    /// Return enabled clients ordered by per-facet routing priority.
    /// Falls back to global `client_priority` if the facet has no routing config.
    async fn list_clients_for_facet(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<Vec<DownloadClientConfig>> {
        let scope_id = match facet {
            MediaFacet::Movie => "movie",
            MediaFacet::Tv => "series",
            MediaFacet::Anime => "anime",
            _ => return self.list_enabled_clients_by_priority().await,
        };

        let routing_json = self
            .settings
            .get_setting_json(
                "system",
                "nzbget.client_routing",
                Some(scope_id.to_string()),
            )
            .await?;

        let mut clients = self
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .collect::<Vec<_>>();

        match routing_json {
            Some(json_str) => {
                // JSON key insertion order = priority (requires serde_json preserve_order)
                let ordered_ids: Vec<String> = serde_json::from_str::<serde_json::Value>(&json_str)
                    .ok()
                    .and_then(|v| v.as_object().map(|obj| obj.keys().cloned().collect()))
                    .unwrap_or_default();

                if ordered_ids.is_empty() {
                    clients.sort_by_key(|c| c.client_priority);
                } else {
                    clients.sort_by_key(|c| {
                        ordered_ids
                            .iter()
                            .position(|id| id == &c.id)
                            .unwrap_or(usize::MAX)
                    });
                }
            }
            None => {
                clients.sort_by_key(|c| c.client_priority);
            }
        }

        Ok(clients)
    }

    fn client_from_config(
        config: &DownloadClientConfig,
        plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
    ) -> AppResult<Arc<dyn DownloadClient>> {
        if let Some(provider) = plugin_provider {
            if let Some(client) = provider.client_for_config(config) {
                return Ok(client);
            }
        }

        let client_type = config.client_type.trim().to_ascii_lowercase();
        match client_type.as_str() {
            "nzbget" => {
                let parsed_config = parse_download_client_config_json(&config.config_json)?;
                let base_url = resolve_download_client_base_url(config, &parsed_config)
                    .ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} has no valid base URL",
                            config.id
                        ))
                    })?;
                let username = read_config_string(&parsed_config, &["username"]);
                let password = read_config_string(&parsed_config, &["password"]);
                let dupe_mode = read_config_string(&parsed_config, &["dupe_mode", "dupeMode"])
                    .unwrap_or_else(|| "SCORE".to_string());
                let client = NzbgetDownloadClient::new(base_url, username, password, dupe_mode);
                Ok(Arc::new(client))
            }
            "sabnzbd" => {
                let parsed_config = parse_download_client_config_json(&config.config_json)?;
                let base_url = resolve_download_client_base_url(config, &parsed_config)
                    .ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} has no valid base URL",
                            config.id
                        ))
                    })?;
                let api_key = read_config_string(&parsed_config, &["api_key", "apiKey", "apikey"])
                    .ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} (sabnzbd) requires an API key",
                            config.id
                        ))
                    })?;
                let client = SabnzbdDownloadClient::new(base_url, api_key);
                Ok(Arc::new(client))
            }
            "weaver" => {
                let parsed_config = parse_download_client_config_json(&config.config_json)?;
                let base_url = resolve_download_client_base_url(config, &parsed_config)
                    .ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} has no valid base URL",
                            config.id
                        ))
                    })?;
                let client = WeaverDownloadClient::new(base_url);
                Ok(Arc::new(client))
            }
            _ => Err(AppError::Validation(format!(
                "unsupported download client type '{}' for config {}",
                config.client_type, config.id
            ))),
        }
    }
}

#[async_trait]
impl DownloadClient for PrioritizedDownloadClientRouter {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let mut clients = match self.list_clients_for_facet(&request.title.facet).await {
            Ok(configs) => configs,
            Err(error) => {
                warn!(
                    error = %error,
                    title = request.title.name.as_str(),
                    facet = ?request.title.facet,
                    "failed to load prioritized download clients; falling back to default client"
                );
                return self.fallback_client.submit_download(request).await;
            }
        };

        if clients.is_empty() {
            return self.fallback_client.submit_download(request).await;
        }

        if let Some(source_kind) = Self::request_source_kind(request) {
            clients.retain(|config| {
                let compatible = Self::config_accepts_source_kind(
                    config,
                    source_kind,
                    self.plugin_provider.as_ref(),
                );
                if !compatible {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        source_kind = source_kind.as_str(),
                        "download client skipped because it cannot handle this release type"
                    );
                }
                compatible
            });

            if clients.is_empty() {
                return Err(AppError::Validation(format!(
                    "no enabled download client can handle {} releases",
                    Self::source_kind_label(source_kind)
                )));
            }
        }

        let mut last_error: Option<AppError> = None;
        for config in clients {
            let client = match Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                Ok(client) => client,
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped due to invalid configuration"
                    );
                    last_error = Some(error);
                    continue;
                }
            };

            match client.submit_download(request).await {
                Ok(result) => {
                    return Ok(DownloadGrabResult {
                        job_id: result.job_id,
                        client_type: config.client_type.trim().to_ascii_lowercase(),
                    });
                }
                Err(error) => {
                    let should_failover = matches!(error, AppError::Repository(_));
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        failover = should_failover,
                        "download client enqueue failed"
                    );
                    if should_failover {
                        last_error = Some(error);
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::Repository(
                "all prioritized download clients failed to enqueue this release".to_string(),
            )
        }))
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_queue().await;
        }
        let mut all_items = Vec::new();
        for config in clients {
            let client = match Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for queue listing");
                    continue;
                }
            };
            match client.list_queue().await {
                Ok(mut items) => {
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list queue");
                }
            }
        }
        Ok(all_items)
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_history().await;
        }
        let mut all_items = Vec::new();
        for config in clients {
            let client = match Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for history listing");
                    continue;
                }
            };
            match client.list_history().await {
                Ok(mut items) => {
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list history");
                }
            }
        }
        Ok(all_items)
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_completed_downloads().await;
        }
        let mut all_items = Vec::new();
        for config in clients {
            let client = match Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for completed downloads");
                    continue;
                }
            };
            match client.list_completed_downloads().await {
                Ok(mut items) => {
                    for item in &mut items {
                        item.client_id = config.id.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list completed downloads");
                }
            }
        }
        Ok(all_items)
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        let clients = self.list_enabled_clients_by_priority().await?;
        for config in clients {
            if let Ok(client) = Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                return client.pause_queue_item(id).await;
            }
        }
        self.fallback_client.pause_queue_item(id).await
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        let clients = self.list_enabled_clients_by_priority().await?;
        for config in clients {
            if let Ok(client) = Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                return client.resume_queue_item(id).await;
            }
        }
        self.fallback_client.resume_queue_item(id).await
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        let clients = self.list_enabled_clients_by_priority().await?;
        for config in clients {
            if let Ok(client) = Self::client_from_config(&config, self.plugin_provider.as_ref()) {
                return client.delete_queue_item(id, is_history).await;
            }
        }
        self.fallback_client.delete_queue_item(id, is_history).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::sync::{Arc, Mutex};

    struct MockDownloadClientConfigRepository {
        configs: Vec<DownloadClientConfig>,
    }

    #[async_trait]
    impl DownloadClientConfigRepository for MockDownloadClientConfigRepository {
        async fn list(
            &self,
            _provider_type: Option<String>,
        ) -> AppResult<Vec<DownloadClientConfig>> {
            Ok(self.configs.clone())
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<DownloadClientConfig>> {
            Ok(None)
        }

        async fn create(&self, _config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
            unreachable!("not used in router tests")
        }

        async fn update(
            &self,
            _id: &str,
            _name: Option<String>,
            _client_type: Option<String>,
            _base_url: Option<String>,
            _config_json: Option<String>,
            _is_enabled: Option<bool>,
        ) -> AppResult<DownloadClientConfig> {
            unreachable!("not used in router tests")
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            unreachable!("not used in router tests")
        }

        async fn reorder(&self, _ordered_ids: Vec<String>) -> AppResult<()> {
            unreachable!("not used in router tests")
        }
    }

    struct MockSettingsRepository;

    #[async_trait]
    impl SettingsRepository for MockSettingsRepository {
        async fn get_setting_json(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct MockDownloadClient {
        submissions: Mutex<Vec<DownloadClientAddRequest>>,
    }

    #[async_trait]
    impl DownloadClient for MockDownloadClient {
        async fn submit_download(
            &self,
            request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            self.submissions.lock().unwrap().push(request.clone());
            Ok(DownloadGrabResult {
                job_id: "job-1".to_string(),
                client_type: "mock".to_string(),
            })
        }
    }

    struct MockDownloadClientPluginProvider {
        accepted_inputs: Vec<String>,
        client: Arc<dyn DownloadClient>,
    }

    impl DownloadClientPluginProvider for MockDownloadClientPluginProvider {
        fn client_for_config(
            &self,
            _config: &DownloadClientConfig,
        ) -> Option<Arc<dyn DownloadClient>> {
            Some(Arc::clone(&self.client))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["qbittorrent".to_string()]
        }

        fn accepted_inputs_for_provider(&self, _provider_type: &str) -> Vec<String> {
            self.accepted_inputs.clone()
        }
    }

    fn test_title() -> scryer_domain::Title {
        scryer_domain::Title {
            id: "title-1".to_string(),
            name: "Test Title".to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
        }
    }

    fn test_config(id: &str, name: &str, client_type: &str, priority: i64) -> DownloadClientConfig {
        DownloadClientConfig {
            id: id.to_string(),
            name: name.to_string(),
            client_type: client_type.to_string(),
            base_url: Some("http://localhost".to_string()),
            config_json: "{}".to_string(),
            is_enabled: true,
            status: "healthy".to_string(),
            last_error: None,
            last_seen_at: None,
            client_priority: priority,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn submit_download_skips_incompatible_clients_by_source_kind() {
        let torrent_client = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string(), "magnet_uri".to_string()],
                client: torrent_client.clone(),
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("nzb", "NZBGet", "nzbget", 0),
                    test_config("torrent", "qBittorrent", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository),
            Arc::new(MockDownloadClient::default()),
            Some(plugin_provider),
        );

        let result = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                source_hint: Some("https://tracker.example/file.torrent".to_string()),
                source_kind: Some(DownloadSourceKind::TorrentFile),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("torrent request should route to torrent client");

        assert_eq!(result.client_type, "qbittorrent");
        assert_eq!(torrent_client.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_errors_when_no_enabled_client_can_handle_source_kind() {
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("nzb", "NZBGet", "nzbget", 0)],
            }),
            Arc::new(MockSettingsRepository),
            Arc::new(MockDownloadClient::default()),
            None,
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                source_hint: Some("magnet:?xt=urn:btih:abcdef".to_string()),
                source_kind: Some(DownloadSourceKind::MagnetUri),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("magnet request should fail when only nzb clients are enabled");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("magnet"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }
}
