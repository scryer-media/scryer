use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, IndexerClient, IndexerConfigRepository, IndexerPluginProvider,
    IndexerRoutingPlan, IndexerSearchResponse, IndexerSearchResult, IndexerStatsTracker,
    SearchMode,
};
use scryer_domain::IndexerConfig;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// A single search strategy dispatched as an independent parallel task.
#[derive(Clone, Debug)]
struct SearchStrategy {
    query: String,
    imdb_id: Option<String>,
    tvdb_id: Option<String>,
    anidb_id: Option<String>,
    label: &'static str,
}

/// Per-indexer rate limiter tracking the last request time.
#[derive(Clone)]
struct IndexerRateLimiter {
    last_request: Arc<Mutex<HashMap<String, tokio::time::Instant>>>,
}

impl IndexerRateLimiter {
    fn new() -> Self {
        Self {
            last_request: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Wait until the rate limit period has elapsed for this indexer.
    /// When `rate_limit_seconds` is set (from config/plugin), that value wins.
    /// Otherwise the default depends on the search mode:
    ///   - Interactive: 1s (fast for end-user experience)
    ///   - Auto: 5s (gentle on indexer APIs during background acquisition)
    async fn acquire(&self, indexer_id: &str, rate_limit_seconds: Option<i64>, mode: SearchMode) {
        let default_secs = match mode {
            SearchMode::Interactive => 1,
            SearchMode::Auto => 5,
        };
        let interval_secs = rate_limit_seconds.unwrap_or(default_secs).max(0) as u64;
        if interval_secs == 0 {
            return;
        }

        let interval = std::time::Duration::from_secs(interval_secs);
        let now = tokio::time::Instant::now();

        let mut map = self.last_request.lock().await;
        if let Some(last) = map.get(indexer_id) {
            let elapsed = now.duration_since(*last);
            if elapsed < interval {
                let wait = interval - elapsed;
                drop(map); // Release lock while sleeping
                tokio::time::sleep(wait).await;
                let mut map = self.last_request.lock().await;
                map.insert(indexer_id.to_string(), tokio::time::Instant::now());
                return;
            }
        }
        map.insert(indexer_id.to_string(), now);
    }
}

/// Exponential backoff periods (in seconds), matching Sonarr's EscalationBackOff.Periods[].
const BACKOFF_PERIODS_SECS: &[u64] = &[
    5 * 60,       // 5 minutes
    10 * 60,      // 10 minutes
    15 * 60,      // 15 minutes
    30 * 60,      // 30 minutes
    60 * 60,      // 1 hour
    2 * 60 * 60,  // 2 hours
    4 * 60 * 60,  // 4 hours
    8 * 60 * 60,  // 8 hours
    24 * 60 * 60, // 24 hours
];

#[derive(Clone, Debug)]
struct IndexerBackoffState {
    escalation_level: usize,
    disabled_until: Option<chrono::DateTime<chrono::Utc>>,
}

/// In-memory indexer backoff tracker. Resets on restart, providing a natural
/// 15-minute startup grace period (matching Sonarr's behavior).
#[derive(Clone)]
struct IndexerBackoffTracker {
    state: Arc<Mutex<HashMap<String, IndexerBackoffState>>>,
}

impl IndexerBackoffTracker {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record a failure and escalate the backoff level. Returns the new disabled_until.
    async fn record_failure(&self, indexer_id: &str) -> chrono::DateTime<chrono::Utc> {
        let mut map = self.state.lock().await;
        let state = map
            .entry(indexer_id.to_string())
            .or_insert(IndexerBackoffState {
                escalation_level: 0,
                disabled_until: None,
            });

        let period_index = state.escalation_level.min(BACKOFF_PERIODS_SECS.len() - 1);
        let backoff_secs = BACKOFF_PERIODS_SECS[period_index];
        let until = chrono::Utc::now() + chrono::Duration::seconds(backoff_secs as i64);

        state.escalation_level = (state.escalation_level + 1).min(BACKOFF_PERIODS_SECS.len());
        state.disabled_until = Some(until);

        until
    }

    /// Record a success and de-escalate by one level.
    async fn record_success(&self, indexer_id: &str) {
        let mut map = self.state.lock().await;
        if let Some(state) = map.get_mut(indexer_id) {
            state.escalation_level = state.escalation_level.saturating_sub(1);
            if state.escalation_level == 0 {
                state.disabled_until = None;
            }
        }
    }

    /// Check if this indexer is currently in backoff.
    async fn is_disabled(&self, indexer_id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        let map = self.state.lock().await;
        map.get(indexer_id)
            .and_then(|s| s.disabled_until)
            .filter(|until| *until > chrono::Utc::now())
    }
}

/// Short-lived cache for RSS feed results. Multiple concurrent callers
/// awaiting the same indexer's feed will share a single HTTP fetch.
type RssFeedCache =
    Arc<Mutex<HashMap<String, Arc<tokio::sync::OnceCell<Vec<IndexerSearchResult>>>>>>;

#[derive(Clone)]
pub struct MultiIndexerSearchClient {
    indexer_configs: Arc<dyn IndexerConfigRepository>,
    stats_tracker: Arc<dyn IndexerStatsTracker>,
    plugin_provider: Arc<dyn IndexerPluginProvider>,
    rate_limiter: IndexerRateLimiter,
    backoff_tracker: IndexerBackoffTracker,
    rss_feed_cache: RssFeedCache,
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
            rate_limiter: IndexerRateLimiter::new(),
            backoff_tracker: IndexerBackoffTracker::new(),
            rss_feed_cache: Arc::new(Mutex::new(HashMap::new())),
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

    fn is_rss_sync_request(
        query: &str,
        ids_present: bool,
        filters_present: bool,
        mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> bool {
        matches!(mode, SearchMode::Auto)
            && query.trim().is_empty()
            && !ids_present
            && !filters_present
            && season.is_none()
            && episode.is_none()
    }
}

#[async_trait]
impl IndexerClient for MultiIndexerSearchClient {
    async fn search(
        &self,
        query: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        anidb_id: Option<String>,
        category: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        limit: usize,
        mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> AppResult<IndexerSearchResponse> {
        let is_rss_request = Self::is_rss_sync_request(
            &query,
            imdb_id.is_some() || tvdb_id.is_some() || anidb_id.is_some(),
            category
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty())
                || newznab_categories
                    .as_ref()
                    .is_some_and(|values| !values.is_empty()),
            mode,
            season,
            episode,
        );

        let configs = self.indexer_configs.list(None).await.unwrap_or_else(|err| {
            warn!(error = %err, "failed to load indexer configs");
            vec![]
        });

        let now = chrono::Utc::now();

        // Filter by is_enabled, search mode flag, disabled_until (config), and backoff state
        let mut enabled: Vec<&IndexerConfig> = Vec::new();
        for c in &configs {
            if !c.is_enabled {
                continue;
            }
            // Check persistent disabled_until from config
            if let Some(until) = c.disabled_until
                && until > now
            {
                info!(
                    indexer = c.name.as_str(),
                    disabled_until = %until,
                    "skipping indexer: temporarily disabled (config)"
                );
                continue;
            }
            // Check in-memory backoff escalation
            if let Some(until) = self.backoff_tracker.is_disabled(&c.id).await {
                info!(
                    indexer = c.name.as_str(),
                    disabled_until = %until,
                    "skipping indexer: temporarily disabled (backoff)"
                );
                continue;
            }
            let mode_ok = match mode {
                SearchMode::Interactive => c.enable_interactive_search,
                SearchMode::Auto => c.enable_auto_search,
            };
            if mode_ok {
                enabled.push(c);
            }
        }

        if enabled.is_empty() {
            info!(mode = ?mode, "no enabled indexer configs found");
            return Ok(IndexerSearchResponse {
                results: vec![],
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            });
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

            if let Some(entry) = routing_entry
                && !entry.enabled
            {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: disabled for scope via routing config"
                );
                continue;
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

            // Skip indexers that don't support the requested search type
            let caps = self
                .plugin_provider
                .capabilities_for_provider(&config.provider_type);
            if imdb_id.is_some() && !caps.imdb_search && !caps.search {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: does not support IMDB or freetext search"
                );
                continue;
            }
            if tvdb_id.is_some() && !caps.tvdb_search && !caps.search {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: does not support TVDB or freetext search"
                );
                continue;
            }
            if anidb_id.is_some() && !caps.anidb_search && !caps.search {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: does not support AniDB or freetext search"
                );
                continue;
            }
            if is_rss_request && !caps.rss {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: does not support RSS sync"
                );
                continue;
            }

            let client = match Self::client_from_config(config, &self.plugin_provider) {
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

            // RSS-only indexers: fetch the feed once, cache it, return cached
            // results for all concurrent callers. The feed content is the same
            // regardless of query — the caller matches results downstream.
            let is_rss_only = !caps.search && caps.rss;
            if is_rss_only {
                let cell = {
                    let mut cache = self.rss_feed_cache.lock().await;
                    cache
                        .entry(config.id.clone())
                        .or_insert_with(|| Arc::new(tokio::sync::OnceCell::new()))
                        .clone()
                };
                let client = client.clone();
                let query = query.clone();
                let category = category.clone();
                let per_indexer_categories = per_indexer_categories.clone();
                let indexer_id = config.id.clone();
                let indexer_name = config.name.clone();
                let rate_limiter = self.rate_limiter.clone();
                let rate_limit_seconds = config.rate_limit_seconds;
                let stats_tracker = self.stats_tracker.clone();
                let backoff_tracker = self.backoff_tracker.clone();

                set.spawn(async move {
                    let results = cell
                        .get_or_init(|| async {
                            rate_limiter
                                .acquire(&indexer_id, rate_limit_seconds, mode)
                                .await;
                            let start = std::time::Instant::now();
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(30),
                                client.search(query, None, None, None, category, per_indexer_categories, None, limit, mode, season, episode),
                            ).await {
                                Ok(Ok(response)) => {
                                    info!(indexer = indexer_name.as_str(), count = response.results.len(), "RSS feed cached");
                                    stats_tracker.record_query(&indexer_id, &indexer_name, true);
                                    backoff_tracker.record_success(&indexer_id).await;
                                    metrics::counter!("scryer_indexer_queries_total", "indexer" => indexer_name.clone(), "status" => "success", "mode" => "rss_cached").increment(1);
                                    metrics::histogram!("scryer_indexer_query_duration_seconds", "indexer" => indexer_name.clone(), "mode" => "rss_cached").record(start.elapsed().as_secs_f64());
                                    response.results
                                }
                                Ok(Err(err)) => {
                                    warn!(indexer = indexer_name.as_str(), error = %err, "RSS feed fetch failed");
                                    stats_tracker.record_query(&indexer_id, &indexer_name, false);
                                    vec![]
                                }
                                Err(_) => {
                                    warn!(indexer = indexer_name.as_str(), "RSS feed fetch timed out");
                                    stats_tracker.record_query(&indexer_id, &indexer_name, false);
                                    vec![]
                                }
                            }
                        })
                        .await;

                    let response = IndexerSearchResponse {
                        results: results.clone(),
                        api_current: None, api_max: None, grab_current: None, grab_max: None,
                    };
                    (indexer_id, indexer_name, Ok(response), std::time::Duration::ZERO, "rss_cached")
                });
                continue;
            }

            // For Interactive mode, fan out separate strategy tasks per ID type
            // so all HTTP calls happen in parallel. For Auto mode, send everything
            // in a single call (current behavior — no extra API pressure).
            let strategies: Vec<SearchStrategy> = if mode == SearchMode::Interactive {
                build_strategies(&query, &imdb_id, &tvdb_id, &anidb_id, &caps)
            } else {
                vec![SearchStrategy {
                    query: query.clone(),
                    imdb_id: imdb_id.clone(),
                    tvdb_id: tvdb_id.clone(),
                    anidb_id: anidb_id.clone(),
                    label: "auto",
                }]
            };

            for strategy in strategies {
                let client = client.clone();
                let category = category.clone();
                let per_indexer_categories = per_indexer_categories.clone();
                let indexer_id = config.id.clone();
                let indexer_name = config.name.clone();
                let rate_limiter = self.rate_limiter.clone();
                let rate_limit_seconds = config.rate_limit_seconds;
                let strategy_label = strategy.label;

                set.spawn(async move {
                    rate_limiter
                        .acquire(&indexer_id, rate_limit_seconds, mode)
                        .await;

                    let start = std::time::Instant::now();
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        client.search(
                            strategy.query,
                            strategy.imdb_id,
                            strategy.tvdb_id,
                            strategy.anidb_id,
                            category,
                            per_indexer_categories,
                            None,
                            limit,
                            mode,
                            season,
                            episode,
                        ),
                    )
                    .await;

                    let elapsed = start.elapsed();
                    match result {
                        Ok(inner) => (indexer_id, indexer_name, inner, elapsed, strategy_label),
                        Err(_) => (
                            indexer_id,
                            indexer_name,
                            Err(AppError::Repository("indexer search timed out".into())),
                            elapsed,
                            strategy_label,
                        ),
                    }
                });
            }
        }

        let mut all_results: Vec<IndexerSearchResult> = Vec::new();
        while let Some(join_result) = set.join_next().await {
            match join_result {
                Ok((id, name, Ok(mut response), elapsed, mode_label)) => {
                    info!(
                        indexer = name.as_str(),
                        count = response.results.len(),
                        "indexer returned results"
                    );
                    self.stats_tracker.record_query(&id, &name, true);
                    self.stats_tracker.record_api_limits(
                        &id,
                        response.api_current,
                        response.api_max,
                        response.grab_current,
                        response.grab_max,
                    );
                    // De-escalate on success
                    self.backoff_tracker.record_success(&id).await;

                    metrics::counter!("scryer_indexer_queries_total", "indexer" => name.clone(), "status" => "success", "mode" => mode_label).increment(1);
                    metrics::histogram!("scryer_indexer_query_duration_seconds", "indexer" => name.clone(), "mode" => mode_label).record(elapsed.as_secs_f64());
                    metrics::counter!("scryer_indexer_query_results_total", "indexer" => name.clone(), "mode" => mode_label).increment(response.results.len() as u64);

                    all_results.append(&mut response.results);
                }
                Ok((id, name, Err(err), elapsed, mode_label)) => {
                    warn!(indexer = name.as_str(), error = %err, "indexer search failed");
                    self.stats_tracker.record_query(&id, &name, false);
                    // Escalate backoff on failure
                    let until = self.backoff_tracker.record_failure(&id).await;
                    warn!(
                        indexer = name.as_str(),
                        disabled_until = %until,
                        "indexer backoff escalated"
                    );

                    metrics::counter!("scryer_indexer_queries_total", "indexer" => name.clone(), "status" => "error", "mode" => mode_label).increment(1);
                    metrics::histogram!("scryer_indexer_query_duration_seconds", "indexer" => name.clone(), "mode" => mode_label).record(elapsed.as_secs_f64());
                }
                Err(err) => {
                    warn!(error = %err, "indexer search task panicked");
                }
            }
        }

        // Clear the RSS feed cache after all tasks complete so the next
        // search session gets fresh feeds.
        self.rss_feed_cache.lock().await.clear();

        // Interactive mode fans out multiple strategies per indexer, so dedup
        // results by download_url to remove duplicates across strategies.
        if mode == SearchMode::Interactive {
            let before = all_results.len();
            let mut seen: HashSet<String> = HashSet::new();
            all_results.retain(|r| {
                if let Some(ref url) = r.download_url {
                    seen.insert(url.to_ascii_lowercase())
                } else {
                    true
                }
            });
            let deduped = before - all_results.len();
            if deduped > 0 {
                info!(
                    before,
                    after = all_results.len(),
                    deduped,
                    "deduplicated interactive search results"
                );
            }
        }

        Ok(IndexerSearchResponse {
            results: all_results,
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

/// Build parallel search strategies for interactive mode.
/// Each strategy targets one ID type so the host can dispatch them
/// all in parallel instead of the plugin calling endpoints sequentially.
fn build_strategies(
    query: &str,
    imdb_id: &Option<String>,
    tvdb_id: &Option<String>,
    anidb_id: &Option<String>,
    caps: &scryer_domain::IndexerProviderCapabilities,
) -> Vec<SearchStrategy> {
    let mut strategies = Vec::with_capacity(4);

    // Strategy per ID type (only if indexer supports it)
    if let Some(id) = anidb_id {
        if caps.anidb_search {
            strategies.push(SearchStrategy {
                query: String::new(),
                imdb_id: None,
                tvdb_id: None,
                anidb_id: Some(id.clone()),
                label: "anidb_id",
            });
        }
    }
    if let Some(id) = tvdb_id {
        if caps.tvdb_search {
            strategies.push(SearchStrategy {
                query: String::new(),
                imdb_id: None,
                tvdb_id: Some(id.clone()),
                anidb_id: None,
                label: "tvdb_id",
            });
        }
    }
    if let Some(id) = imdb_id {
        if caps.imdb_search {
            strategies.push(SearchStrategy {
                query: String::new(),
                imdb_id: Some(id.clone()),
                tvdb_id: None,
                anidb_id: None,
                label: "imdb_id",
            });
        }
    }

    // Freetext strategy (always, if query is non-empty and indexer supports search)
    if !query.is_empty() && caps.search {
        strategies.push(SearchStrategy {
            query: query.to_string(),
            imdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            label: "freetext",
        });
    }

    // If no strategies were generated (no IDs, no query), fall back to
    // a single combined call so the indexer can at least try RSS/empty search.
    if strategies.is_empty() {
        strategies.push(SearchStrategy {
            query: query.to_string(),
            imdb_id: imdb_id.clone(),
            tvdb_id: tvdb_id.clone(),
            anidb_id: anidb_id.clone(),
            label: "fallback",
        });
    }

    strategies
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use chrono::Utc;
    use scryer_application::{IndexerQueryStats, IndexerSearchResponse};
    use scryer_domain::IndexerProviderCapabilities;

    use super::*;

    struct MockIndexerConfigRepository {
        configs: Vec<IndexerConfig>,
    }

    #[async_trait]
    impl IndexerConfigRepository for MockIndexerConfigRepository {
        async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(self.configs.clone())
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
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
            Err(AppError::Validation("not implemented in test".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct MockIndexerStatsTracker;

    impl IndexerStatsTracker for MockIndexerStatsTracker {
        fn record_query(&self, _indexer_id: &str, _indexer_name: &str, _success: bool) {}

        fn record_api_limits(
            &self,
            _indexer_id: &str,
            _api_current: Option<u32>,
            _api_max: Option<u32>,
            _grab_current: Option<u32>,
            _grab_max: Option<u32>,
        ) {
        }

        fn all_stats(&self) -> Vec<IndexerQueryStats> {
            vec![]
        }
    }

    struct MockIndexerClient {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl IndexerClient for MockIndexerClient {
        async fn search(
            &self,
            _query: String,
            _imdb_id: Option<String>,
            _tvdb_id: Option<String>,
            _anidb_id: Option<String>,
            _category: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _limit: usize,
            _mode: SearchMode,
            _season: Option<u32>,
            _episode: Option<u32>,
        ) -> AppResult<IndexerSearchResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(IndexerSearchResponse {
                results: vec![],
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        }
    }

    struct MockIndexerPluginProvider {
        rss: bool,
        calls: Arc<AtomicUsize>,
    }

    impl IndexerPluginProvider for MockIndexerPluginProvider {
        fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
            Some(Arc::new(MockIndexerClient {
                calls: self.calls.clone(),
            }))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["mock".into()]
        }

        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            vec![]
        }

        fn capabilities_for_provider(&self, _provider_type: &str) -> IndexerProviderCapabilities {
            IndexerProviderCapabilities {
                rss: self.rss,
                search: true,
                imdb_search: true,
                tvdb_search: true,
                anidb_search: false,
            }
        }
    }

    fn mock_indexer_config() -> IndexerConfig {
        IndexerConfig {
            id: "idx-1".into(),
            name: "Mock Indexer".into(),
            provider_type: "mock".into(),
            base_url: "https://example.test".into(),
            api_key_encrypted: None,
            rate_limit_seconds: Some(0),
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            last_health_status: None,
            last_error_at: None,
            config_json: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn rss_sync_search_skips_providers_without_rss_capability() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: false,
                calls: calls.clone(),
            }),
        );

        let response = client
            .search(
                String::new(),
                None,
                None,
                None,
                None,
                None,
                None,
                500,
                SearchMode::Auto,
                None,
                None,
            )
            .await
            .expect("rss sync search should succeed");

        assert!(response.results.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
