use std::collections::HashMap;
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

#[derive(Clone)]
pub struct MultiIndexerSearchClient {
    indexer_configs: Arc<dyn IndexerConfigRepository>,
    stats_tracker: Arc<dyn IndexerStatsTracker>,
    plugin_provider: Arc<dyn IndexerPluginProvider>,
    rate_limiter: IndexerRateLimiter,
    backoff_tracker: IndexerBackoffTracker,
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
    ) -> AppResult<IndexerSearchResponse> {
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
            if let Some(until) = c.disabled_until {
                if until > now {
                    info!(
                        indexer = c.name.as_str(),
                        disabled_until = %until,
                        "skipping indexer: temporarily disabled (config)"
                    );
                    continue;
                }
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

            // Skip indexers that don't support the requested search type
            let caps = self
                .plugin_provider
                .capabilities_for_provider(&config.provider_type);
            if imdb_id.is_some() && !caps.imdb_search {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: does not support IMDB search"
                );
                continue;
            }
            if tvdb_id.is_some() && !caps.tvdb_search {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: does not support TVDB search"
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
            let query = query.clone();
            let imdb_id = imdb_id.clone();
            let tvdb_id = tvdb_id.clone();
            let category = category.clone();
            let indexer_id = config.id.clone();
            let indexer_name = config.name.clone();
            let rate_limiter = self.rate_limiter.clone();
            let rate_limit_seconds = config.rate_limit_seconds;

            set.spawn(async move {
                // Enforce per-indexer rate limiting before dispatching
                rate_limiter
                    .acquire(&indexer_id, rate_limit_seconds, mode)
                    .await;

                let start = std::time::Instant::now();
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    client.search(
                        query,
                        imdb_id,
                        tvdb_id,
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
                let mode_label = match mode {
                    SearchMode::Interactive => "interactive",
                    SearchMode::Auto => "auto",
                };
                match result {
                    Ok(inner) => (indexer_id, indexer_name, inner, elapsed, mode_label),
                    Err(_) => (
                        indexer_id,
                        indexer_name,
                        Err(AppError::Repository("indexer search timed out".into())),
                        elapsed,
                        mode_label,
                    ),
                }
            });
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

        Ok(IndexerSearchResponse {
            results: all_results,
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}
