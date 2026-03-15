use super::*;
use chrono::Utc;

impl AppUseCase {
    /// Test an indexer connection by performing a minimal search through the plugin system.
    /// This validates: plugin availability, HTTP connectivity, API key, response parsing.
    pub async fn test_indexer_connection(
        &self,
        actor: &User,
        provider_type: &str,
        base_url: &str,
        api_key: Option<&str>,
        config_json: Option<&str>,
    ) -> AppResult<()> {
        require(actor, &Entitlement::ManageConfig)?;

        let base_url =
            crate::app_usecase_integration::resolve_indexer_base_url(base_url, config_json)?;

        let provider = self
            .services
            .plugin_provider
            .as_ref()
            .ok_or_else(|| AppError::Repository("plugin provider not available".into()))?;

        let now = Utc::now();

        // Build a temporary IndexerConfig to get a client from the plugin
        // Reject obviously invalid API keys (e.g. masked placeholders from
        // Sonarr/Radarr import that were stored before the masking fix).
        if let Some(key) = api_key {
            let trimmed = key.trim();
            if trimmed.chars().all(|c| c == '*') && !trimmed.is_empty() {
                return Err(AppError::Validation(
                    "API key appears to be a masked placeholder — enter the real key".into(),
                ));
            }
        }

        let temp_config = IndexerConfig {
            id: "test-connection".to_string(),
            name: "Test Connection".to_string(),
            provider_type: provider_type.to_string(),
            base_url,
            api_key_encrypted: api_key.map(|k| k.trim().to_string()),
            rate_limit_seconds: None,
            rate_limit_burst: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            disabled_until: None,
            last_health_status: None,
            last_error_at: None,
            config_json: config_json.map(|s| s.to_string()),
            created_at: now,
            updated_at: now,
        };

        let client = provider.client_for_provider(&temp_config).ok_or_else(|| {
            AppError::Validation(format!(
                "no indexer plugin available for provider type '{provider_type}'"
            ))
        })?;

        // Perform a minimal search to validate the full pipeline
        client
            .search(
                String::new(), // empty query
                None,
                None,
                None,
                None,
                None,
                None,
                1, // limit 1
                SearchMode::Interactive,
                None,
                None,
            )
            .await
            .map_err(|e| AppError::Repository(format!("indexer connection test failed: {e}")))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NullSettingsRepository;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullEventRepository,
        NullIndexerClient, NullQualityProfileRepository, NullReleaseAttemptRepository,
        NullShowRepository, NullTitleRepository, NullUserRepository,
    };
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    struct RecordingIndexerConfigRepo {
        created: Arc<Mutex<Vec<IndexerConfig>>>,
    }

    impl RecordingIndexerConfigRepo {
        fn new() -> Self {
            Self {
                created: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl IndexerConfigRepository for RecordingIndexerConfigRepo {
        async fn list(&self, _provider_filter: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(self.created.lock().await.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
            let created = self.created.lock().await;
            Ok(created.iter().find(|config| config.id == id).cloned())
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            self.created.lock().await.push(config.clone());
            Ok(config)
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
            self.created.lock().await.retain(|config| config.id != id);
            Ok(())
        }

        async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct RecordingPluginProvider {
        seen_configs: Arc<std::sync::Mutex<Vec<IndexerConfig>>>,
    }

    impl RecordingPluginProvider {
        fn new() -> Self {
            Self {
                seen_configs: Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }
    }

    impl IndexerPluginProvider for RecordingPluginProvider {
        fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
            self.seen_configs.lock().unwrap().push(config.clone());
            Some(Arc::new(NullIndexerClient))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["torrent_rss".to_string()]
        }

        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            vec![]
        }
    }

    fn test_app(
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        plugin_provider: Option<Arc<dyn IndexerPluginProvider>>,
    ) -> AppUseCase {
        let mut services = AppServices::with_default_channels(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(NullEventRepository),
            indexer_configs,
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        );
        services.plugin_provider = plugin_provider;

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".to_string(),
                access_ttl_seconds: 3600,
                jwt_hmac_secret: "dGVzdC1zZWNyZXQtZm9yLXVuaXQtdGVzdHMtb25seS0zMmJ5dGVzISE="
                    .to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    #[tokio::test]
    async fn create_indexer_config_derives_base_url_from_feed_url() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let app = test_app(indexer_repo.clone(), None);

        let created = app
            .create_indexer_config(
                &User::new_admin("admin"),
                NewIndexerConfig {
                    name: "RSS".to_string(),
                    provider_type: "torrent_rss".to_string(),
                    base_url: String::new(),
                    api_key_encrypted: None,
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    config_json: Some(
                        r#"{"feed_url":"https://ipt.beelyrics.net/t.rss?u=2203846"}"#.to_string(),
                    ),
                },
            )
            .await
            .unwrap();

        assert_eq!(created.base_url, "https://ipt.beelyrics.net");
    }

    #[tokio::test]
    async fn test_indexer_connection_derives_base_url_from_feed_url() {
        let provider = Arc::new(RecordingPluginProvider::new());
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider.clone()),
        );

        app.test_indexer_connection(
            &User::new_admin("admin"),
            "torrent_rss",
            "",
            None,
            Some(r#"{"feed_url":"https://ipt.beelyrics.net/t.rss?u=2203846"}"#),
        )
        .await
        .unwrap();

        let seen = provider.seen_configs.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].base_url, "https://ipt.beelyrics.net");
    }
}
