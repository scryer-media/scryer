use std::sync::Arc;

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, IndexerClient, IndexerConfigRepository, IndexerPluginProvider,
    IndexerRoutingPlan, IndexerSearchResult, IndexerStatsTracker, SearchMode,
};
use scryer_domain::IndexerConfig;
use tracing::{info, warn};

#[derive(Clone)]
pub struct MultiIndexerSearchClient {
    indexer_configs: Arc<dyn IndexerConfigRepository>,
    stats_tracker: Arc<dyn IndexerStatsTracker>,
    plugin_provider: Arc<dyn IndexerPluginProvider>,
}

impl MultiIndexerSearchClient {
    pub fn new(
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        stats_tracker: Arc<dyn IndexerStatsTracker>,
        plugin_provider: Arc<dyn IndexerPluginProvider>,
    ) -> Self {
        Self {
            indexer_configs,
            stats_tracker,
            plugin_provider,
        }
    }

    fn client_from_config(
        config: &IndexerConfig,
        plugin_provider: &Arc<dyn IndexerPluginProvider>,
    ) -> AppResult<Arc<dyn IndexerClient>> {
        let provider = config.provider_type.trim().to_ascii_lowercase();

        if let Some(client) = plugin_provider.client_for_provider(config) {
            return Ok(client);
        }

        Err(AppError::Validation(format!(
            "unsupported indexer provider: '{provider}'"
        )))
    }
}

#[async_trait]
impl IndexerClient for MultiIndexerSearchClient {
    async fn search(
        &self,
        query: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        category: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        limit: usize,
        mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let configs = self.indexer_configs.list(None).await.unwrap_or_else(|err| {
            warn!(error = %err, "failed to load indexer configs");
            vec![]
        });

        // Filter by is_enabled AND the appropriate search mode flag
        let enabled: Vec<&IndexerConfig> = configs
            .iter()
            .filter(|c| {
                c.is_enabled
                    && match mode {
                        SearchMode::Interactive => c.enable_interactive_search,
                        SearchMode::Auto => c.enable_auto_search,
                    }
            })
            .collect();

        if enabled.is_empty() {
            info!(mode = ?mode, "no enabled indexer configs found");
            return Ok(vec![]);
        }

        info!(
            mode = ?mode,
            count = enabled.len(),
            indexers = ?enabled.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            "dispatching search to indexers"
        );

        // Spawn parallel searches across enabled indexers, applying per-indexer routing
        let mut set = tokio::task::JoinSet::new();
        for config in enabled {
            // Apply per-indexer facet scoping: if routing is configured and this
            // indexer is disabled for the current scope, skip it entirely.
            let routing_entry = indexer_routing
                .as_ref()
                .and_then(|plan| plan.entries.get(&config.id));

            if let Some(entry) = routing_entry {
                if !entry.enabled {
                    info!(
                        indexer = config.name.as_str(),
                        "skipping indexer: disabled for scope via routing config"
                    );
                    continue;
                }
            }

            // Use per-indexer categories from routing if available, otherwise fall
            // back to the caller-provided newznab_categories.
            let per_indexer_categories = routing_entry
                .map(|entry| {
                    if entry.categories.is_empty() {
                        newznab_categories.clone()
                    } else {
                        Some(entry.categories.clone())
                    }
                })
                .unwrap_or_else(|| newznab_categories.clone());

            let client = match Self::client_from_config(
                config,
                &self.plugin_provider,
            ) {
                Ok(c) => c,
                Err(err) => {
                    warn!(
                        indexer = config.name.as_str(),
                        error = %err,
                        "skipping indexer: unsupported provider"
                    );
                    continue;
                }
            };
            let query = query.clone();
            let imdb_id = imdb_id.clone();
            let tvdb_id = tvdb_id.clone();
            let category = category.clone();
            let indexer_id = config.id.clone();
            let indexer_name = config.name.clone();

            set.spawn(async move {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    client.search(query, imdb_id, tvdb_id, category, per_indexer_categories, None, limit, mode, season, episode),
                )
                .await;

                match result {
                    Ok(inner) => (indexer_id, indexer_name, inner),
                    Err(_) => (
                        indexer_id,
                        indexer_name,
                        Err(AppError::Repository("indexer search timed out".into())),
                    ),
                }
            });
        }

        let mut all_results: Vec<IndexerSearchResult> = Vec::new();
        while let Some(join_result) = set.join_next().await {
            match join_result {
                Ok((id, name, Ok(mut items))) => {
                    info!(indexer = name.as_str(), count = items.len(), "indexer returned results");
                    self.stats_tracker.record_query(&id, &name, true);
                    all_results.append(&mut items);
                }
                Ok((id, name, Err(err))) => {
                    warn!(indexer = name.as_str(), error = %err, "indexer search failed");
                    self.stats_tracker.record_query(&id, &name, false);
                }
                Err(err) => {
                    warn!(error = %err, "indexer search task panicked");
                }
            }
        }

        Ok(all_results)
    }
}
