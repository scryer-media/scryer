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

        if base_url.trim().is_empty() {
            return Err(AppError::Validation("base_url is required".into()));
        }

        let provider = self
            .services
            .plugin_provider
            .as_ref()
            .ok_or_else(|| AppError::Repository("plugin provider not available".into()))?;

        let now = Utc::now();

        // Build a temporary IndexerConfig to get a client from the plugin
        let temp_config = IndexerConfig {
            id: "test-connection".to_string(),
            name: "Test Connection".to_string(),
            provider_type: provider_type.to_string(),
            base_url: base_url.trim().to_string(),
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
