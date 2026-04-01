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
/// Each strategy carries the raw query/ID params to pass through to the plugin.
#[derive(Clone, Debug)]
struct SearchStrategy {
    query: String,
    ids: HashMap<String, String>,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
    label: String,
}

#[derive(Debug)]
struct StrategyExecutionOutcome {
    label: String,
    title_guard_mode: TitleGuardMode,
    response: AppResult<IndexerSearchResponse>,
    elapsed: std::time::Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TitleGuardMode {
    SkipTitleMatch,
    ExactTitleMatch,
}

#[derive(Default)]
struct StrategyBatchHealth {
    any_success: bool,
    any_error: bool,
}

impl StrategyBatchHealth {
    fn mark_success(&mut self) {
        self.any_success = true;
    }

    fn mark_error(&mut self) {
        self.any_error = true;
    }

    async fn apply(
        self,
        backoff_tracker: &IndexerBackoffTracker,
        indexer_id: &str,
        indexer_name: &str,
    ) {
        if self.any_success {
            backoff_tracker.record_success(indexer_id).await;
        } else if self.any_error {
            let until = backoff_tracker.record_failure(indexer_id).await;
            warn!(
                indexer = indexer_name,
                disabled_until = %until,
                "indexer backoff escalated"
            );
        }
    }
}

fn should_run_fallback_tier(
    collected_results: &[IndexerSearchResult],
    primary_had_success: bool,
    primary_had_error: bool,
    fallback_strategies: &[SearchStrategy],
) -> bool {
    collected_results.is_empty()
        && primary_had_success
        && !primary_had_error
        && !fallback_strategies.is_empty()
}

/// Records transport metrics per outbound indexer request.
///
/// A single high-level search can emit multiple request attempts when we fan
/// out across ID/episode strategies or run a freetext fallback tier. Keeping
/// these counters at request granularity makes the tier labels actionable in
/// dashboards and keeps latency tied to the specific outbound call.
fn record_strategy_metrics(
    indexer_name: &str,
    mode_label: &str,
    status: &str,
    elapsed: std::time::Duration,
    result_count: Option<usize>,
) {
    metrics::counter!(
        "scryer_indexer_queries_total",
        "indexer" => indexer_name.to_string(),
        "status" => status.to_string(),
        "mode" => mode_label.to_string()
    )
    .increment(1);
    metrics::histogram!(
        "scryer_indexer_query_duration_seconds",
        "indexer" => indexer_name.to_string(),
        "mode" => mode_label.to_string()
    )
    .record(elapsed.as_secs_f64());

    if let Some(result_count) = result_count {
        metrics::counter!(
            "scryer_indexer_query_results_total",
            "indexer" => indexer_name.to_string(),
            "mode" => mode_label.to_string()
        )
        .increment(result_count as u64);
    }
}

fn preferred_anime_alias_query(
    query: &str,
    tagged_aliases: &[scryer_domain::TaggedAlias],
) -> Option<String> {
    let canonical = strip_query_context(query);
    if canonical.is_empty() {
        return None;
    }

    let alias_candidates: Vec<(String, String, bool, bool)> = tagged_aliases
        .iter()
        .map(|alias| {
            let trimmed = alias.name.trim().to_string();
            let language_matches = alias.language.eq_ignore_ascii_case("jpn");
            let romanized = is_romanized_alias(&alias.name);
            (trimmed, alias.language.clone(), language_matches, romanized)
        })
        .collect();

    alias_candidates
        .iter()
        .find(|(name, _, language_matches, romanized)| {
            !name.is_empty()
                && *language_matches
                && *romanized
                && !canonical.eq_ignore_ascii_case(name)
        })
        .map(|(name, _, _, _)| name.clone())
}

fn is_freetext_strategy_label(label: &str) -> bool {
    matches!(label, "freetext" | "freetext_alias")
}

fn should_defer_freetext_to_fallback(_facet: &str, strategies: &[SearchStrategy]) -> bool {
    strategies.iter().any(|strategy| !strategy.ids.is_empty())
        && strategies
            .iter()
            .any(|strategy| is_freetext_strategy_label(&strategy.label))
}

fn split_strategy_tiers(
    facet: &str,
    strategies: Vec<SearchStrategy>,
) -> (Vec<SearchStrategy>, Vec<SearchStrategy>) {
    if !should_defer_freetext_to_fallback(facet, &strategies) {
        return (strategies, Vec::new());
    }

    let mut primary = Vec::new();
    let mut fallback = Vec::new();

    for strategy in strategies {
        if is_freetext_strategy_label(&strategy.label) {
            fallback.push(strategy);
        } else {
            primary.push(strategy);
        }
    }

    if primary.is_empty() || fallback.is_empty() {
        let mut merged = primary;
        merged.extend(fallback);
        return (merged, Vec::new());
    }

    (primary, fallback)
}

fn strip_query_context(query: &str) -> &str {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return query.trim();
    }

    let mut start = tokens.len();
    for index in (0..tokens.len()).rev() {
        if looks_like_context_token(tokens[index]) {
            start = index;
        } else if start != tokens.len() {
            break;
        }
    }

    if start == tokens.len() {
        query.trim()
    } else {
        query[..query.rfind(tokens[start]).unwrap_or(query.len())].trim()
    }
}

fn looks_like_context_token(token: &str) -> bool {
    let trimmed = token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    if trimmed.is_empty() {
        return false;
    }

    let upper = trimmed.to_ascii_uppercase();
    if upper == "OVA" || upper == "SPECIAL" {
        return true;
    }

    if let Some(rest) = upper.strip_prefix('S') {
        if rest.chars().all(|ch| ch.is_ascii_digit()) {
            return true;
        }
        if let Some((season_part, episode_part)) = rest.split_once('E') {
            return !season_part.is_empty()
                && !episode_part.is_empty()
                && season_part.chars().all(|ch| ch.is_ascii_digit())
                && episode_part.chars().all(|ch| ch.is_ascii_digit());
        }
    }

    trimmed.chars().all(|ch| ch.is_ascii_digit())
}

fn is_romanized_alias(alias: &str) -> bool {
    let trimmed = alias.trim();
    !trimmed.is_empty()
        && trimmed.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    ' ' | '-' | '_' | ':' | ';' | ',' | '.' | '\'' | '&' | '!' | '?'
                )
        })
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

    async fn execute_strategy_tier(
        client: Arc<dyn IndexerClient>,
        category: Option<String>,
        facet: String,
        per_indexer_categories: Option<Vec<String>>,
        mode: SearchMode,
        tagged_aliases: Vec<scryer_domain::TaggedAlias>,
        strategies: Vec<SearchStrategy>,
    ) -> Vec<StrategyExecutionOutcome> {
        let mut set = tokio::task::JoinSet::<StrategyExecutionOutcome>::new();

        for strategy in strategies {
            let client = client.clone();
            let category = category.clone();
            let per_indexer_categories = per_indexer_categories.clone();
            let tagged_aliases = tagged_aliases.clone();
            let facet = facet.clone();
            let strategy_label = strategy.label.clone();
            let title_guard_mode = if strategy.ids.is_empty() {
                TitleGuardMode::ExactTitleMatch
            } else {
                TitleGuardMode::SkipTitleMatch
            };

            set.spawn(async move {
                let start = std::time::Instant::now();
                let response = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    client.search(
                        strategy.query,
                        strategy.ids,
                        category,
                        Some(facet),
                        per_indexer_categories,
                        None,
                        mode,
                        strategy.season,
                        strategy.episode,
                        strategy.absolute_episode,
                        tagged_aliases,
                    ),
                )
                .await
                .unwrap_or_else(|_| Err(AppError::Repository("indexer search timed out".into())));

                StrategyExecutionOutcome {
                    label: strategy_label,
                    title_guard_mode,
                    response,
                    elapsed: start.elapsed(),
                }
            });
        }

        let mut outcomes = Vec::new();
        while let Some(join_result) = set.join_next().await {
            match join_result {
                Ok(outcome) => outcomes.push(outcome),
                Err(error) => outcomes.push(StrategyExecutionOutcome {
                    label: "join".into(),
                    title_guard_mode: TitleGuardMode::SkipTitleMatch,
                    response: Err(AppError::Repository(format!(
                        "indexer search task panicked: {error}"
                    ))),
                    elapsed: std::time::Duration::ZERO,
                }),
            }
        }

        outcomes
    }
}

#[async_trait]
impl IndexerClient for MultiIndexerSearchClient {
    async fn search(
        &self,
        query: String,
        ids: HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<scryer_domain::TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        let is_rss_request = Self::is_rss_sync_request(
            &query,
            !ids.is_empty(),
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

        let facet = match facet
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some("movie") | Some("series") | Some("anime") => facet.unwrap(),
            Some(other) => {
                return Err(AppError::Validation(format!(
                    "unsupported search facet: {other}"
                )));
            }
            None if is_rss_request => "series".to_string(),
            None => {
                return Err(AppError::Validation("search facet is required".to_string()));
            }
        };

        tracing::debug!(
            %facet,
            ?category,
            ?ids,
            ?season,
            ?episode,
            ?absolute_episode,
            %query,
            "search context"
        );
        let available_ids = ids;

        // Spawn parallel searches across enabled indexers, applying per-indexer routing.
        // Each indexer may still execute multiple strategies internally, but for
        // TV/series searches we run ID searches first and only fall back to a
        // broad freetext query if that indexer returned no releases.
        let mut set =
            tokio::task::JoinSet::<(String, String, AppResult<IndexerSearchResponse>)>::new();
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

            // Skip indexers at or near their API quota for auto searches.
            if mode == SearchMode::Auto && self.stats_tracker.is_at_quota(&config.id) {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: at API quota limit"
                );
                continue;
            }

            let caps = self
                .plugin_provider
                .capabilities_for_provider(&config.provider_type);

            // RSS-only check: skip non-RSS indexers for RSS sync requests
            if is_rss_request && !caps.rss {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: does not support RSS sync"
                );
                continue;
            }

            // Skip indexers that can't contribute to this facet.
            // - Indexers with declared facets that don't include the current facet are skipped.
            // - Indexers that have the facet but only for ID-based search (deduplicates_aliases)
            //   are skipped when none of their supported IDs are available — freetext on
            //   AnimeTosho for "The Matrix" is pointless when there's no anidb_id.
            let has_facet_entry = caps.has_facet(&facet);
            let has_declared_facets = !caps.supported_ids.is_empty();
            let skip_no_facet = !has_facet_entry && has_declared_facets;
            let skip_no_matching_id = has_facet_entry && caps.deduplicates_aliases && {
                filter_ids_for_types(&available_ids, caps.id_types_for_facet(&facet)).is_empty()
            };
            if !is_rss_request && (skip_no_facet || skip_no_matching_id) {
                info!(
                    indexer = config.name.as_str(),
                    facet, "skipping indexer: no supported IDs for facet and no freetext"
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
            let is_rss_only = !caps.supports_any_search() && caps.rss;
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
                let tagged_aliases = tagged_aliases.clone();
                let indexer_id = config.id.clone();
                let indexer_name = config.name.clone();
                let rate_limiter = self.rate_limiter.clone();
                let rate_limit_seconds = config.rate_limit_seconds;
                let stats_tracker = self.stats_tracker.clone();
                let backoff_tracker = self.backoff_tracker.clone();
                let facet = facet.clone();

                set.spawn(async move {
                    let results = cell
                        .get_or_init(|| async {
                            rate_limiter
                                .acquire(&indexer_id, rate_limit_seconds, mode)
                                .await;
                            let start = std::time::Instant::now();
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(30),
                                client.search(
                                    query,
                                    HashMap::new(),
                                    category,
                                    Some(facet),
                                    per_indexer_categories,
                                    None,
                                    mode,
                                    season,
                                    episode,
                                    absolute_episode,
                                    tagged_aliases,
                                ),
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
                        api_current: None,
                        api_max: None,
                        grab_current: None,
                        grab_max: None,
                    };
                    (indexer_id, indexer_name, Ok(response))
                });
                continue;
            }

            let mut strategies: Vec<SearchStrategy> = build_strategies(&StrategyParams {
                query: &query,
                facet: &facet,
                ids: &available_ids,
                season,
                episode,
                absolute_episode,
                caps: &caps,
                is_alias_query: false,
            });

            if facet == "anime"
                && let Some(alias_query) = preferred_anime_alias_query(&query, &tagged_aliases)
            {
                let alias_strategies = build_strategies(&StrategyParams {
                    query: &alias_query,
                    facet: &facet,
                    ids: &available_ids,
                    season,
                    episode,
                    absolute_episode,
                    caps: &caps,
                    is_alias_query: true,
                });

                strategies.extend(alias_strategies);
            }

            let (primary_strategies, fallback_strategies) =
                split_strategy_tiers(&facet, strategies);

            self.rate_limiter
                .acquire(&config.id, config.rate_limit_seconds, mode)
                .await;

            let indexer_id = config.id.clone();
            let indexer_name = config.name.clone();
            let facet = facet.clone();
            let search_query = query.clone();
            let category_for_indexer = category.clone();
            let tagged_aliases_for_indexer = tagged_aliases.clone();
            let stats_tracker = self.stats_tracker.clone();
            let backoff_tracker = self.backoff_tracker.clone();

            set.spawn(async move {
                let mut collected_results = Vec::new();
                let mut primary_had_success = false;
                let mut primary_had_error = false;
                let mut batch_health = StrategyBatchHealth::default();

                let primary_outcomes = Self::execute_strategy_tier(
                    client.clone(),
                    category_for_indexer.clone(),
                    facet.clone(),
                    per_indexer_categories.clone(),
                    mode,
                    tagged_aliases_for_indexer.clone(),
                    primary_strategies,
                )
                .await;

                for outcome in primary_outcomes {
                    match outcome.response {
                        Ok(mut response) => {
                            primary_had_success = true;
                            batch_health.mark_success();
                            info!(
                                indexer = indexer_name.as_str(),
                                strategy = outcome.label.as_str(),
                                count = response.results.len(),
                                "indexer returned results"
                            );
                            stats_tracker.record_query(&indexer_id, &indexer_name, true);
                            stats_tracker.record_api_limits(
                                &indexer_id,
                                response.api_current,
                                response.api_max,
                                response.grab_current,
                                response.grab_max,
                            );
                            backoff_tracker.record_success(&indexer_id).await;

                            record_strategy_metrics(
                                &indexer_name,
                                &outcome.label,
                                "success",
                                outcome.elapsed,
                                Some(response.results.len()),
                            );

                            filter_strategy_results(
                                &mut response.results,
                                &search_query,
                                season,
                                episode,
                                &tagged_aliases_for_indexer,
                                outcome.title_guard_mode,
                                &outcome.label,
                            );
                            collected_results.append(&mut response.results);
                        }
                        Err(err) => {
                            primary_had_error = true;
                            batch_health.mark_error();
                            warn!(
                                indexer = indexer_name.as_str(),
                                strategy = outcome.label.as_str(),
                                error = %err,
                                "indexer search failed"
                            );
                            stats_tracker.record_query(&indexer_id, &indexer_name, false);

                            record_strategy_metrics(
                                &indexer_name,
                                &outcome.label,
                                "error",
                                outcome.elapsed,
                                None,
                            );
                        }
                    }
                }

                if should_run_fallback_tier(
                    &collected_results,
                    primary_had_success,
                    primary_had_error,
                    &fallback_strategies,
                ) {
                    info!(
                        indexer = indexer_name.as_str(),
                        facet = facet.as_str(),
                        query = search_query.as_str(),
                        reason = "zero_id_results",
                        "indexer search falling back to title tier"
                    );

                    let fallback_outcomes = Self::execute_strategy_tier(
                        client,
                        category_for_indexer,
                        facet,
                        per_indexer_categories,
                        mode,
                        tagged_aliases_for_indexer.clone(),
                        fallback_strategies,
                    )
                    .await;

                    for outcome in fallback_outcomes {
                        match outcome.response {
                            Ok(mut response) => {
                                batch_health.mark_success();
                                info!(
                                    indexer = indexer_name.as_str(),
                                    strategy = outcome.label.as_str(),
                                    count = response.results.len(),
                                    "indexer returned fallback results"
                                );
                                stats_tracker.record_query(&indexer_id, &indexer_name, true);
                                stats_tracker.record_api_limits(
                                    &indexer_id,
                                    response.api_current,
                                    response.api_max,
                                    response.grab_current,
                                    response.grab_max,
                                );
                                backoff_tracker.record_success(&indexer_id).await;

                                record_strategy_metrics(
                                    &indexer_name,
                                    &outcome.label,
                                    "success",
                                    outcome.elapsed,
                                    Some(response.results.len()),
                                );

                                filter_strategy_results(
                                    &mut response.results,
                                    &search_query,
                                    season,
                                    episode,
                                    &tagged_aliases_for_indexer,
                                    outcome.title_guard_mode,
                                    &outcome.label,
                                );
                                collected_results.append(&mut response.results);
                            }
                            Err(err) => {
                                batch_health.mark_error();
                                warn!(
                                    indexer = indexer_name.as_str(),
                                    strategy = outcome.label.as_str(),
                                    error = %err,
                                    "indexer fallback search failed"
                                );
                                stats_tracker.record_query(&indexer_id, &indexer_name, false);

                                record_strategy_metrics(
                                    &indexer_name,
                                    &outcome.label,
                                    "error",
                                    outcome.elapsed,
                                    None,
                                );
                            }
                        }
                    }
                }

                batch_health
                    .apply(&backoff_tracker, &indexer_id, &indexer_name)
                    .await;

                (
                    indexer_id,
                    indexer_name,
                    Ok(IndexerSearchResponse {
                        results: collected_results,
                        api_current: None,
                        api_max: None,
                        grab_current: None,
                        grab_max: None,
                    }),
                )
            });
        }

        let mut all_results: Vec<IndexerSearchResult> = Vec::new();
        while let Some(join_result) = set.join_next().await {
            match join_result {
                Ok((_id, name, Ok(mut response))) => {
                    info!(
                        indexer = name.as_str(),
                        count = response.results.len(),
                        "indexer returned aggregated results"
                    );
                    all_results.append(&mut response.results);
                }
                Ok((id, name, Err(err))) => {
                    warn!(indexer = name.as_str(), error = %err, "indexer search failed");
                    let _ = id;
                }
                Err(err) => {
                    warn!(error = %err, "indexer search task panicked");
                }
            }
        }

        // Clear the RSS feed cache after all tasks complete so the next
        // search session gets fresh feeds.
        self.rss_feed_cache.lock().await.clear();

        // Dedup by download_url (exact duplicates from parallel strategies).
        // Cross-indexer release-identity dedup happens in the discovery layer
        // where download client preferences are available.
        {
            let before = all_results.len();
            let mut seen_urls: HashSet<String> = HashSet::new();
            all_results.retain(|r| {
                if let Some(ref url) = r.download_url {
                    seen_urls.insert(url.to_ascii_lowercase())
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
                    "deduplicated search results by URL"
                );
            }
        }

        for result in &mut all_results {
            if result.parsed_release_metadata.is_none() {
                result.parsed_release_metadata =
                    Some(scryer_release_parser::parse_release_metadata(&result.title));
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
///
/// Uses the plugin's facet-scoped `supported_ids` to determine which ID-based
/// strategies to generate. Each strategy targets one ID type so the host can
/// dispatch them all in parallel.
struct StrategyParams<'a> {
    query: &'a str,
    facet: &'a str,
    ids: &'a HashMap<String, String>,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
    caps: &'a scryer_domain::IndexerProviderCapabilities,
    is_alias_query: bool,
}

/// The `facet` parameter is the current search facet ("movie", "series", "anime").
/// The orchestrator only builds ID strategies for facets the indexer declares
/// in `supported_ids`.
fn build_strategies(p: &StrategyParams<'_>) -> Vec<SearchStrategy> {
    let query = p.query;
    let facet = p.facet;
    let ids = p.ids;
    let season = p.season;
    let episode = p.episode;
    let absolute_episode = p.absolute_episode;
    let caps = p.caps;
    let is_alias_query = p.is_alias_query;
    // Alias queries skip indexers that deduplicate aliases internally
    if is_alias_query && caps.deduplicates_aliases {
        return vec![];
    }

    let mut strategies = Vec::with_capacity(4);

    let filtered_ids = filter_ids_for_types(ids, caps.id_types_for_facet(facet));
    if !filtered_ids.is_empty() && !is_alias_query {
        if facet == "anime" {
            if let Some(absolute_episode) = absolute_episode {
                strategies.push(SearchStrategy {
                    query: String::new(),
                    ids: filtered_ids.clone(),
                    season: None,
                    episode: None,
                    absolute_episode: Some(absolute_episode),
                    label: "ids_abs".into(),
                });
            }

            if episode.is_some() {
                strategies.push(SearchStrategy {
                    query: String::new(),
                    ids: filtered_ids.clone(),
                    season,
                    episode,
                    absolute_episode: None,
                    label: "ids_sxex".into(),
                });
            }
        }

        if strategies.is_empty() {
            strategies.push(SearchStrategy {
                query: String::new(),
                ids: filtered_ids,
                season,
                episode,
                absolute_episode,
                label: "ids".into(),
            });
        }
    }

    // Freetext strategy: skip if indexer has no capability for this facet at all.
    // An indexer that only declares "anime" should not get freetext for "series" searches.
    // For alias queries, indexers with deduplicates_aliases skip freetext (handled at top).
    let has_facet_entry = caps.has_facet(facet);
    let skip_no_facet = !has_facet_entry && !caps.supported_ids.is_empty();
    if caps.query_param.is_some() && !query.is_empty() && !skip_no_facet {
        strategies.push(SearchStrategy {
            query: query.to_string(),
            ids: HashMap::new(),
            season,
            episode,
            absolute_episode: None,
            label: if is_alias_query {
                "freetext_alias".into()
            } else {
                "freetext".into()
            },
        });
    }

    // If no strategies were generated, fall back to a single combined call
    if strategies.is_empty() {
        strategies.push(SearchStrategy {
            query: query.to_string(),
            ids: ids.clone(),
            season,
            episode,
            absolute_episode,
            label: "fallback".into(),
        });
    }

    strategies
}

fn filter_ids_for_types(
    ids: &HashMap<String, String>,
    supported_types: &[String],
) -> HashMap<String, String> {
    if supported_types.is_empty() {
        return HashMap::new();
    }

    let supported_types: HashSet<&str> = supported_types.iter().map(String::as_str).collect();
    ids.iter()
        .filter(|(id_type, value)| {
            supported_types.contains(id_type.as_str()) && !value.trim().is_empty()
        })
        .map(|(id_type, value)| (id_type.clone(), value.clone()))
        .collect()
}

/// Normalize a title for substring comparison: lowercase, alpha-only, no spaces.
fn normalize_for_comparison(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Returns the normalized titles that can legitimately identify this parsed
/// release. The title guard uses exact matches against this set to reject
/// nearby-but-wrong releases like "Firefly Lane" for a "Firefly" search.
fn parsed_title_candidates(parsed: &scryer_application::ParsedReleaseMetadata) -> Vec<String> {
    let mut titles = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };

    if titles.is_empty() {
        titles.push(parsed.normalized_title.clone());
    }

    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for title in titles {
        let candidate = normalize_for_comparison(&title);
        if !candidate.is_empty() && seen.insert(candidate.clone()) {
            normalized.push(candidate);
        }
    }

    normalized
}

fn filter_strategy_results(
    results: &mut Vec<IndexerSearchResult>,
    query: &str,
    season: Option<u32>,
    episode: Option<u32>,
    tagged_aliases: &[scryer_domain::TaggedAlias],
    title_guard_mode: TitleGuardMode,
    strategy_label: &str,
) {
    if results.is_empty() {
        return;
    }

    for result in results.iter_mut() {
        if result.parsed_release_metadata.is_none() {
            result.parsed_release_metadata =
                Some(scryer_release_parser::parse_release_metadata(&result.title));
        }
    }

    if query.is_empty() && season.is_none() && episode.is_none() {
        return;
    }

    let mut expected_titles = if query.is_empty() {
        Vec::new()
    } else {
        parsed_title_candidates(&scryer_release_parser::parse_release_metadata(query))
    };
    expected_titles.extend(
        tagged_aliases
            .iter()
            .map(|alias| normalize_for_comparison(&alias.name))
            .filter(|alias| !alias.is_empty()),
    );
    let mut seen_titles = HashSet::new();
    expected_titles.retain(|title| seen_titles.insert(title.clone()));

    let before = results.len();
    results.retain(|result| {
        let Some(ref parsed) = result.parsed_release_metadata else {
            return true;
        };

        if title_guard_mode == TitleGuardMode::ExactTitleMatch && !expected_titles.is_empty() {
            let release_titles = parsed_title_candidates(parsed);
            let title_ok = release_titles.iter().any(|release_title| {
                expected_titles
                    .iter()
                    .any(|expected| expected == release_title)
            });
            if !title_ok {
                tracing::debug!(
                    strategy = strategy_label,
                    query = %query,
                    expected = ?expected_titles,
                    got = ?release_titles,
                    "title guard: title mismatch"
                );
                return false;
            }
        }

        if let Some(expected_s) = season
            && let Some(ref res_ep) = parsed.episode
            && let Some(rs) = res_ep.season
            && rs != expected_s
        {
            tracing::debug!(
                strategy = strategy_label,
                query = %query,
                expected_season = expected_s,
                got_season = rs,
                "title guard: season mismatch"
            );
            return false;
        }

        if let Some(expected_e) = episode
            && let Some(ref res_ep) = parsed.episode
            && !res_ep.episode_numbers.is_empty()
            && !res_ep.episode_numbers.contains(&expected_e)
        {
            tracing::debug!(
                strategy = strategy_label,
                query = %query,
                expected_episode = expected_e,
                got_episodes = ?res_ep.episode_numbers,
                "title guard: episode mismatch"
            );
            return false;
        }

        true
    });

    let filtered = before - results.len();
    if filtered > 0 {
        info!(
            strategy = strategy_label,
            before,
            after = results.len(),
            filtered,
            "title guard: removed irrelevant results"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc as StdArc, Mutex as StdMutex};

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

    #[derive(Default)]
    struct RecordingIndexerStatsTracker {
        queries: StdArc<StdMutex<Vec<bool>>>,
    }

    impl IndexerStatsTracker for RecordingIndexerStatsTracker {
        fn record_query(&self, _indexer_id: &str, _indexer_name: &str, success: bool) {
            self.queries.lock().expect("stats log mutex").push(success);
        }

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
            _ids: HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
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
                supported_ids: HashMap::from([
                    ("movie".into(), vec!["imdb_id".into()]),
                    ("series".into(), vec!["tvdb_id".into()]),
                ]),
                deduplicates_aliases: false,
                season_param: Some("season".into()),
                episode_param: Some("ep".into()),
                query_param: Some("q".into()),
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

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RecordedCall {
        query: String,
        ids: HashMap<String, String>,
        facet: Option<String>,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
    }

    type ResponseFn = dyn Fn(&RecordedCall) -> AppResult<IndexerSearchResponse> + Send + Sync;

    struct ScriptedIndexerClient {
        calls: StdArc<StdMutex<Vec<RecordedCall>>>,
        responder: StdArc<ResponseFn>,
    }

    #[async_trait]
    impl IndexerClient for ScriptedIndexerClient {
        async fn search(
            &self,
            query: String,
            ids: HashMap<String, String>,
            _category: Option<String>,
            facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            season: Option<u32>,
            episode: Option<u32>,
            absolute_episode: Option<u32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
        ) -> AppResult<IndexerSearchResponse> {
            let call = RecordedCall {
                query,
                ids,
                facet,
                season,
                episode,
                absolute_episode,
            };
            self.calls
                .lock()
                .expect("call log mutex")
                .push(call.clone());
            (self.responder)(&call)
        }
    }

    struct ScriptedIndexerPluginProvider {
        client: Arc<dyn IndexerClient>,
        caps: IndexerProviderCapabilities,
    }

    impl IndexerPluginProvider for ScriptedIndexerPluginProvider {
        fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
            Some(self.client.clone())
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["mock".into()]
        }

        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            vec![]
        }

        fn capabilities_for_provider(&self, _provider_type: &str) -> IndexerProviderCapabilities {
            self.caps.clone()
        }
    }

    fn scripted_search_client(
        caps: IndexerProviderCapabilities,
        responder: impl Fn(&RecordedCall) -> AppResult<IndexerSearchResponse> + Send + Sync + 'static,
    ) -> (
        MultiIndexerSearchClient,
        StdArc<StdMutex<Vec<RecordedCall>>>,
    ) {
        scripted_search_client_with_stats(caps, Arc::new(MockIndexerStatsTracker), responder)
    }

    fn scripted_search_client_with_stats(
        caps: IndexerProviderCapabilities,
        stats_tracker: Arc<dyn IndexerStatsTracker>,
        responder: impl Fn(&RecordedCall) -> AppResult<IndexerSearchResponse> + Send + Sync + 'static,
    ) -> (
        MultiIndexerSearchClient,
        StdArc<StdMutex<Vec<RecordedCall>>>,
    ) {
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(responder),
        });

        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            stats_tracker,
            Arc::new(ScriptedIndexerPluginProvider { client, caps }),
        );

        (multi, calls)
    }

    async fn backoff_state(
        client: &MultiIndexerSearchClient,
        indexer_id: &str,
    ) -> Option<IndexerBackoffState> {
        client
            .backoff_tracker
            .state
            .lock()
            .await
            .get(indexer_id)
            .cloned()
    }

    fn search_result(title: &str) -> IndexerSearchResult {
        IndexerSearchResult {
            source: "mock".into(),
            title: title.into(),
            link: None,
            download_url: Some(format!(
                "https://example.test/download/{}",
                title.replace(' ', "_")
            )),
            source_kind: None,
            size_bytes: None,
            published_at: None,
            thumbs_up: None,
            thumbs_down: None,
            indexer_languages: None,
            indexer_subtitles: None,
            indexer_grabs: None,
            password_hint: None,
            parsed_release_metadata: None,
            quality_profile_decision: None,
            extra: HashMap::new(),
            guid: None,
            info_url: None,
        }
    }

    fn response_with_titles(titles: &[&str]) -> AppResult<IndexerSearchResponse> {
        Ok(IndexerSearchResponse {
            results: titles.iter().map(|title| search_result(title)).collect(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }

    fn movie_caps() -> IndexerProviderCapabilities {
        IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("movie".into(), vec!["imdb_id".into()])]),
            deduplicates_aliases: false,
            season_param: None,
            episode_param: None,
            query_param: Some("q".into()),
            search: true,
            imdb_search: true,
            tvdb_search: false,
            anidb_search: false,
        }
    }

    fn series_caps() -> IndexerProviderCapabilities {
        IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("series".into(), vec!["tvdb_id".into()])]),
            deduplicates_aliases: false,
            season_param: Some("season".into()),
            episode_param: Some("ep".into()),
            query_param: Some("q".into()),
            search: true,
            imdb_search: false,
            tvdb_search: true,
            anidb_search: false,
        }
    }

    fn anime_caps() -> IndexerProviderCapabilities {
        IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("anime".into(), vec!["anidb_id".into()])]),
            deduplicates_aliases: false,
            season_param: Some("season".into()),
            episode_param: Some("ep".into()),
            query_param: Some("q".into()),
            search: true,
            imdb_search: false,
            tvdb_search: false,
            anidb_search: true,
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
                HashMap::new(),
                None,
                None,
                None,
                None,
                SearchMode::Auto,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("rss sync search should succeed");

        assert!(response.results.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn series_search_with_tvdb_id_skips_freetext_when_id_tier_returns_results() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                response_with_titles(&["Firefly.S01E12.720p.WEB-DL"])
            } else {
                response_with_titles(&["Firefly.Lane.S01E12.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Firefly S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 1);
        assert!(calls[0].ids.contains_key("tvdb_id"));
        assert!(calls[0].query.is_empty());
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].title, "Firefly.S01E12.720p.WEB-DL");
    }

    #[tokio::test]
    async fn series_search_with_tvdb_id_falls_back_only_after_empty_id_tier() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["Firefly.S01E12.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Firefly S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 2);
        assert!(calls[0].ids.contains_key("tvdb_id"));
        assert!(calls[0].query.is_empty());
        assert!(calls[1].ids.is_empty());
        assert_eq!(calls[1].query, "Firefly S01E12");
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].title, "Firefly.S01E12.720p.WEB-DL");
    }

    #[tokio::test]
    async fn id_empty_then_fallback_still_rejects_false_positive_titles() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&[
                    "Firefly.S01E12.720p.WEB-DL",
                    "Firefly.Lane.2021.S01E12.2160p.WEB-DL",
                ])
            }
        });

        let response = client
            .search(
                "Firefly S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 2);
        assert!(calls[0].ids.contains_key("tvdb_id"));
        assert!(calls[1].ids.is_empty());
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].title, "Firefly.S01E12.720p.WEB-DL");
    }

    #[tokio::test]
    async fn movie_search_with_imdb_id_uses_tiered_fallback() {
        let (client, calls) = scripted_search_client(movie_caps(), |call| {
            if call.ids.contains_key("imdb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["The.Matrix.1999.1080p.BluRay"])
            }
        });

        let response = client
            .search(
                "The Matrix".into(),
                HashMap::from([("imdb_id".to_string(), "tt0133093".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 2);
        assert!(calls[0].ids.contains_key("imdb_id"));
        assert!(calls[0].query.is_empty());
        assert!(calls[1].ids.is_empty());
        assert_eq!(calls[1].query, "The Matrix");
        assert_eq!(response.results[0].title, "The.Matrix.1999.1080p.BluRay");
    }

    #[tokio::test]
    async fn anime_search_keeps_id_variants_in_primary_tier_and_falls_back_after_empty_results() {
        let (client, calls) = scripted_search_client(anime_caps(), |call| {
            if call.ids.contains_key("anidb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["Demon.Slayer.S02E03.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Demon Slayer S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
                None,
                SearchMode::Interactive,
                Some(2),
                Some(3),
                Some(21),
                vec![],
            )
            .await
            .expect("search should succeed");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 3);
        assert!(calls[0].ids.contains_key("anidb_id"));
        assert!(calls[1].ids.contains_key("anidb_id"));
        assert!(calls[0].query.is_empty());
        assert!(calls[1].query.is_empty());
        assert!(calls[0].absolute_episode == Some(21) || calls[1].absolute_episode == Some(21));
        assert!(calls[0].ids.is_empty() || calls[1].ids.is_empty() || calls[2].ids.is_empty());
        assert!(calls[2].ids.is_empty());
        assert_eq!(calls[2].query, "Demon Slayer S02E03");
        assert_eq!(response.results[0].title, "Demon.Slayer.S02E03.720p.WEB-DL");
    }

    #[tokio::test]
    async fn id_tier_errors_do_not_trigger_title_fallback() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                Err(AppError::Repository("boom".into()))
            } else {
                response_with_titles(&["Firefly.S01E12.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Firefly S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("search should still return an aggregate response");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 1);
        assert!(calls[0].ids.contains_key("tvdb_id"));
        assert!(calls[0].query.is_empty());
        assert!(response.results.is_empty());
    }

    #[tokio::test]
    async fn mixed_primary_outcomes_do_not_trigger_fallback() {
        let (client, calls) = scripted_search_client(anime_caps(), |call| {
            if call.ids.contains_key("anidb_id") && call.absolute_episode.is_some() {
                Err(AppError::Repository("abs lookup failed".into()))
            } else if call.ids.contains_key("anidb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["Demon.Slayer.S02E03.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Demon Slayer S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
                None,
                SearchMode::Interactive,
                Some(2),
                Some(3),
                Some(21),
                vec![],
            )
            .await
            .expect("mixed primary outcomes should still aggregate cleanly");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|call| call.ids.contains_key("anidb_id")));
        assert!(calls.iter().all(|call| call.query.is_empty()));
        assert!(response.results.is_empty());
    }

    #[tokio::test]
    async fn mixed_batch_does_not_back_off_when_any_request_succeeds() {
        let stats = Arc::new(RecordingIndexerStatsTracker::default());
        let (client, calls) =
            scripted_search_client_with_stats(anime_caps(), stats.clone(), |call| {
                if call.ids.contains_key("anidb_id") && call.absolute_episode.is_some() {
                    Err(AppError::Repository("abs lookup failed".into()))
                } else if call.ids.contains_key("anidb_id") {
                    response_with_titles(&[])
                } else {
                    response_with_titles(&["Demon.Slayer.S02E03.720p.WEB-DL"])
                }
            });

        client.backoff_tracker.state.lock().await.insert(
            "idx-1".into(),
            IndexerBackoffState {
                escalation_level: 1,
                disabled_until: None,
            },
        );

        let response = client
            .search(
                "Demon Slayer S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
                None,
                SearchMode::Interactive,
                Some(2),
                Some(3),
                Some(21),
                vec![],
            )
            .await
            .expect("mixed primary outcomes should still aggregate cleanly");

        {
            let calls = calls.lock().expect("call log mutex");
            assert_eq!(calls.len(), 2);
            assert!(response.results.is_empty());
        }
        assert!(client.backoff_tracker.is_disabled("idx-1").await.is_none());
        let state = backoff_state(&client, "idx-1")
            .await
            .expect("success should preserve a cleared backoff entry");
        assert_eq!(state.escalation_level, 0);
        assert!(state.disabled_until.is_none());

        let stats = stats.queries.lock().expect("stats log mutex");
        assert_eq!(stats.len(), 2);
        assert_eq!(stats.iter().filter(|success| **success).count(), 1);
        assert_eq!(stats.iter().filter(|success| !**success).count(), 1);
    }

    #[tokio::test]
    async fn all_request_failures_back_off_once() {
        let stats = Arc::new(RecordingIndexerStatsTracker::default());
        let (client, calls) =
            scripted_search_client_with_stats(anime_caps(), stats.clone(), |call| {
                if call.ids.contains_key("anidb_id") {
                    Err(AppError::Repository("lookup failed".into()))
                } else {
                    response_with_titles(&["Demon.Slayer.S02E03.720p.WEB-DL"])
                }
            });

        let response = client
            .search(
                "Demon Slayer S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
                None,
                SearchMode::Interactive,
                Some(2),
                Some(3),
                Some(21),
                vec![],
            )
            .await
            .expect("all-failure primary outcomes should still aggregate cleanly");

        {
            let calls = calls.lock().expect("call log mutex");
            assert_eq!(calls.len(), 2);
            assert!(calls.iter().all(|call| call.ids.contains_key("anidb_id")));
            assert!(response.results.is_empty());
        }
        assert!(client.backoff_tracker.is_disabled("idx-1").await.is_some());

        let state = backoff_state(&client, "idx-1")
            .await
            .expect("indexer should have backoff state");
        assert_eq!(state.escalation_level, 1);
        assert!(state.disabled_until.is_some());

        let stats = stats.queries.lock().expect("stats log mutex");
        assert_eq!(stats.len(), 2);
        assert!(stats.iter().all(|success| !*success));
    }

    #[tokio::test]
    async fn exact_title_guard_rejects_false_positive_series_matches_for_freetext_searches() {
        let (client, _calls) = scripted_search_client(series_caps(), |_call| {
            response_with_titles(&[
                "Firefly.S01E12.720p.WEB-DL",
                "Firefly.Lane.2021.S01E12.2160p.WEB-DL",
                "Friends.Like.These.S01E12.720p.WEB-DL",
                "Smiling.Friends.S01E12.1080p.WEB-DL",
            ])
        });

        let firefly = client
            .search(
                "Firefly S01E12".into(),
                HashMap::new(),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("firefly search should succeed");
        assert_eq!(firefly.results.len(), 1);
        assert_eq!(firefly.results[0].title, "Firefly.S01E12.720p.WEB-DL");

        let friends = client
            .search(
                "Friends S01E12".into(),
                HashMap::new(),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("friends search should succeed");
        assert!(friends.results.is_empty());
    }

    #[tokio::test]
    async fn id_backed_searches_skip_title_guard() {
        let (client, _calls) = scripted_search_client(movie_caps(), |call| {
            if call.ids.contains_key("imdb_id") {
                response_with_titles(&["Sen.to.Chihiro.no.Kamikakushi.2001.1080p.BluRay"])
            } else {
                response_with_titles(&[])
            }
        });

        let response = client
            .search(
                "Spirited Away".into(),
                HashMap::from([("imdb_id".to_string(), "tt0245429".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("ID-backed search should succeed");

        assert_eq!(response.results.len(), 1);
        assert_eq!(
            response.results[0].title,
            "Sen.to.Chihiro.no.Kamikakushi.2001.1080p.BluRay"
        );
    }
    #[test]
    fn anime_strategies_try_abs_and_sxex_in_parallel() {
        let caps = IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("anime".into(), vec!["anidb_id".into()])]),
            deduplicates_aliases: false,
            season_param: Some("s".into()),
            episode_param: Some("ep".into()),
            query_param: Some("q".into()),
            search: true,
            imdb_search: false,
            tvdb_search: false,
            anidb_search: true,
        };

        let ids = HashMap::from([("anidb_id".to_string(), "18886".to_string())]);
        let strategies = build_strategies(&StrategyParams {
            query: "Frieren: Beyond Journey's End S02E05",
            facet: "anime",
            ids: &ids,
            season: Some(2),
            episode: Some(5),
            absolute_episode: Some(33),
            caps: &caps,
            is_alias_query: false,
        });

        assert_eq!(strategies.len(), 3);

        assert_eq!(strategies[0].label, "ids_abs");
        assert_eq!(strategies[0].season, None);
        assert_eq!(strategies[0].episode, None);
        assert_eq!(strategies[0].absolute_episode, Some(33));

        assert_eq!(strategies[1].label, "ids_sxex");
        assert_eq!(strategies[1].season, Some(2));
        assert_eq!(strategies[1].episode, Some(5));
        assert_eq!(strategies[1].absolute_episode, None);

        assert_eq!(strategies[2].label, "freetext");
        assert_eq!(strategies[2].season, Some(2));
        assert_eq!(strategies[2].episode, Some(5));
        assert_eq!(strategies[2].absolute_episode, None);
    }

    #[test]
    fn preferred_anime_alias_query_strips_episode_context() {
        let alias = preferred_anime_alias_query(
            "Frieren: Beyond Journey's End S02E05",
            &[scryer_domain::TaggedAlias {
                name: "Sousou no Frieren".into(),
                language: "jpn".into(),
            }],
        );

        assert_eq!(alias.as_deref(), Some("Sousou no Frieren"));
    }

    #[test]
    fn preferred_anime_alias_query_skips_canonical_alias_and_uses_distinct_romanized_alias() {
        let alias = preferred_anime_alias_query(
            "Frieren: Beyond Journey's End S02E05",
            &[
                scryer_domain::TaggedAlias {
                    name: "Frieren: Beyond Journey's End".into(),
                    language: "jpn".into(),
                },
                scryer_domain::TaggedAlias {
                    name: "Sousou no Frieren".into(),
                    language: "jpn".into(),
                },
            ],
        );

        assert_eq!(alias.as_deref(), Some("Sousou no Frieren"));
    }

    #[test]
    fn anime_alias_strategy_is_freetext_only_and_skips_ids() {
        let caps = IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("anime".into(), vec!["tvdb_id".into()])]),
            deduplicates_aliases: false,
            season_param: Some("season".into()),
            episode_param: Some("ep".into()),
            query_param: Some("q".into()),
            search: true,
            imdb_search: false,
            tvdb_search: true,
            anidb_search: false,
        };

        let ids = HashMap::from([("tvdb_id".to_string(), "424536".to_string())]);
        let strategies = build_strategies(&StrategyParams {
            query: "Sousou no Frieren",
            facet: "anime",
            ids: &ids,
            season: Some(2),
            episode: Some(5),
            absolute_episode: Some(33),
            caps: &caps,
            is_alias_query: true,
        });

        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies[0].label, "freetext_alias");
        assert!(strategies[0].ids.is_empty());
        assert_eq!(strategies[0].season, Some(2));
        assert_eq!(strategies[0].episode, Some(5));
        assert_eq!(strategies[0].absolute_episode, None);
    }
}
