use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use scryer_application::{
    AppError, AppResult, DownloadSourceKind, EstimatedCost, ExpectedValueHint, HashDomain,
    INDEXER_CAPS_REFRESH_ERROR_PREFIX, IndexerClient, IndexerConfigRepository,
    IndexerErrorClassification, IndexerErrorOperation, IndexerErrorRepository,
    IndexerPluginProvider, IndexerQueryOutcome, IndexerResponseAttributes, IndexerRoutingPlan,
    IndexerSearchCandidateWrite, IndexerSearchCompletion, IndexerSearchEligibility,
    IndexerSearchIncompleteReason, IndexerSearchLearningContext, IndexerSearchLearningKey,
    IndexerSearchLearningRecord, IndexerSearchLearningRepository, IndexerSearchOutcome,
    IndexerSearchPageSink, IndexerSearchPlanRequest, IndexerSearchResponse, IndexerSearchResult,
    IndexerSearchRunWrite, IndexerSearchStrategyEvent, IndexerSearchStrategyEventSink,
    IndexerSearchStrategyRequest, IndexerStatsTracker, IndexerSystemBackoff, NewIndexerError,
    NormalizedIndexerSearchCandidate, NullIndexerErrorRepository,
    NullIndexerSearchLearningRepository, NullProxyConfigRepository, NullUpstreamScheduler,
    ProxyConfigRepository, RateLimitCooldownAction, RateLimitSignal, ReleaseCandidateProvenance,
    ReleaseSearchSubjectKind, ReusableIndexerSearchCandidate, RssFreshnessContext,
    SchedulerAdmission, SchedulerBatchRequest, SchedulerCandidate, SchedulerCandidateId,
    SchedulerFeedback, SchedulerFeedbackOutcome, SchedulerIntent, SchedulerLease,
    SchedulerOperation, SchedulerPluginKind, SchedulerSnapshot, SearchLearningContext, SearchMode,
    UpstreamScheduler, blake3_identity_hex, indexer_search_eligibility, indexer_search_identity,
};
use scryer_domain::{
    IndexerCapsSearchNode, IndexerCapsSnapshot, IndexerConfig, IndexerProviderCapabilities,
    IndexerSearchInputCapability, NabTransportKind,
};
use serde::Deserialize;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use scryer_outbound_http::{
    DestinationKey, HostKey, HostRpsProfile, HostRpsProfileSource, LOCAL_MANAGED_HOST_RPS,
    LOCAL_MANAGED_HOST_RPS_BURST, RateLimitRegistry, effective_indexer_timeout,
};

/// A single effective query within a host-built indexer strategy tier.
#[derive(Clone, Debug)]
struct SearchStrategy {
    request_query: String,
    request_facet: String,
    ids: HashMap<String, String>,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
    generic_query_only: bool,
    /// Ask the provider's generic function instead of the facet-scoped one,
    /// while keeping the routed categories. Narrower than `generic_query_only`,
    /// which also strips the categories and the structured context.
    omit_request_facet: bool,
    label: String,
}

#[derive(Clone, Debug)]
struct PreparedSearchStrategy {
    strategy_id: String,
    labels: Vec<String>,
    request: IndexerSearchStrategyRequest,
    title_guard_mode: TitleGuardMode,
}

#[derive(Clone, Default)]
struct SchedulerRssActivity {
    last_successful_poll_at: Option<DateTime<Utc>>,
    last_attempt_at: Option<DateTime<Utc>>,
    target_interval: Option<std::time::Duration>,
    latest_safe_poll_at: Option<DateTime<Utc>>,
    estimated_feed_depth: Option<u32>,
    freshness_risk: Option<f64>,
    destination_recent_activity_at: Option<DateTime<Utc>>,
}

struct SchedulerEligibleIndexer<'a> {
    config: &'a IndexerConfig,
    had_persisted_system_backoff: bool,
    candidate_id: SchedulerCandidateId,
    category_request: Option<Vec<String>>,
    rss_request_key: Option<String>,
}

#[derive(Debug)]
struct StrategyExecutionOutcome {
    strategy_id: String,
    labels: Vec<String>,
    label: String,
    title_guard_mode: TitleGuardMode,
    response: AppResult<IndexerSearchResponse>,
    page_reservation: Option<scryer_application::IndexerSearchPageReservation>,
    request_fired: bool,
    elapsed: std::time::Duration,
    retry_after: Option<std::time::Duration>,
    rate_limited: bool,
    timed_out: bool,
}

enum StrategyTierOutcomes {
    Legacy(tokio::task::JoinSet<StrategyExecutionOutcome>),
    Plan(StrategyPlanOutcomeStream),
}

impl StrategyTierOutcomes {
    async fn join_next(&mut self) -> Option<Result<StrategyExecutionOutcome, String>> {
        match self {
            Self::Legacy(tasks) => tasks
                .join_next()
                .await
                .map(|result| result.map_err(|error| error.to_string())),
            Self::Plan(stream) => stream.receiver.recv().await.map(Ok),
        }
    }
}

struct StrategyPlanOutcomeStream {
    receiver: tokio::sync::mpsc::Receiver<StrategyExecutionOutcome>,
    controller: tokio::task::JoinHandle<()>,
    cancel_token: CancellationToken,
}

impl Drop for StrategyPlanOutcomeStream {
    fn drop(&mut self) {
        self.cancel_token.cancel();
        self.controller.abort();
    }
}

#[cfg(test)]
fn strategy_execution_is_complete(outcome: &StrategyExecutionOutcome) -> bool {
    outcome.request_fired
        && matches!(
            outcome.response.as_ref(),
            Ok(response) if response.completion == IndexerSearchCompletion::Complete
        )
}

#[derive(Clone, Copy, Debug, Default)]
struct IndexerQuotaObservation {
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
}

impl IndexerQuotaObservation {
    fn merge_response(&mut self, response: &IndexerSearchResponse) {
        merge_max_u32(&mut self.api_current, response.api_current);
        merge_max_u32(&mut self.api_max, response.api_max);
        merge_max_u32(&mut self.grab_current, response.grab_current);
        merge_max_u32(&mut self.grab_max, response.grab_max);
    }
}

fn merge_max_u32(target: &mut Option<u32>, candidate: Option<u32>) {
    if let Some(candidate) = candidate {
        *target = Some(target.map_or(candidate, |existing| existing.max(candidate)));
    }
}

#[derive(Clone)]
struct StrategyTierContext {
    client: Arc<dyn IndexerClient>,
    search_limit: Arc<Semaphore>,
    rate_limiter: IndexerRateLimiter,
    indexer_id: String,
    search_timeout: std::time::Duration,
    rate_limit_seconds: Option<i64>,
    category: Option<String>,
    per_indexer_categories: Option<Vec<String>>,
    mode: SearchMode,
    operation: IndexerErrorOperation,
    /// The request's known release year. Constant across the tier, like the
    /// categories and aliases, so it rides the context rather than each
    /// strategy.
    year: Option<i32>,
    tagged_aliases: Vec<scryer_domain::TaggedAlias>,
    cancel_token: CancellationToken,
    deadline_at: Option<tokio::time::Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchWindowError {
    Cancelled,
    DeadlineExpired,
}

#[derive(Debug, PartialEq, Eq)]
enum SearchPermitError {
    Cancelled,
    DeadlineExpired,
    Closed(String),
}

async fn within_search_window<T>(
    future: impl Future<Output = T>,
    cancel_token: &CancellationToken,
    deadline_at: Option<tokio::time::Instant>,
) -> Result<T, SearchWindowError> {
    if cancel_token.is_cancelled() {
        return Err(SearchWindowError::Cancelled);
    }
    if deadline_at.is_some_and(|deadline| deadline <= tokio::time::Instant::now()) {
        return Err(SearchWindowError::DeadlineExpired);
    }

    tokio::select! {
        biased;
        _ = cancel_token.cancelled() => Err(SearchWindowError::Cancelled),
        _ = async {
            match deadline_at {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        } => Err(SearchWindowError::DeadlineExpired),
        result = future => Ok(result),
    }
}

async fn acquire_search_permit(
    search_limit: Arc<Semaphore>,
    cancel_token: &CancellationToken,
    deadline_at: Option<tokio::time::Instant>,
) -> Result<OwnedSemaphorePermit, SearchPermitError> {
    match within_search_window(search_limit.acquire_owned(), cancel_token, deadline_at).await {
        Ok(Ok(permit)) => Ok(permit),
        Ok(Err(error)) => Err(SearchPermitError::Closed(error.to_string())),
        Err(SearchWindowError::Cancelled) => Err(SearchPermitError::Cancelled),
        Err(SearchWindowError::DeadlineExpired) => Err(SearchPermitError::DeadlineExpired),
    }
}

fn effective_request_deadline(
    search_timeout: std::time::Duration,
    deadline_at: Option<tokio::time::Instant>,
) -> tokio::time::Instant {
    let request_deadline = tokio::time::Instant::now() + search_timeout;
    deadline_at.map_or(request_deadline, |deadline| deadline.min(request_deadline))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TitleGuardMode {
    SkipTitleMatch,
    ExactTitleMatch,
}

fn title_guard_mode_for_strategy(strategy: &SearchStrategy) -> TitleGuardMode {
    if !strategy.ids.is_empty() || strategy.request_query.trim().is_empty() {
        TitleGuardMode::SkipTitleMatch
    } else {
        TitleGuardMode::ExactTitleMatch
    }
}

fn prepare_search_strategies(
    context: &StrategyTierContext,
    strategies: Vec<SearchStrategy>,
) -> Vec<PreparedSearchStrategy> {
    let mut prepared = Vec::<PreparedSearchStrategy>::new();
    let mut by_identity = HashMap::<String, usize>::new();

    for strategy in strategies {
        let title_guard_mode = title_guard_mode_for_strategy(&strategy);
        let category = (!strategy.generic_query_only)
            .then(|| context.category.clone())
            .flatten();
        let facet = (!strategy.generic_query_only && !strategy.omit_request_facet)
            .then(|| strategy.request_facet.clone());
        let newznab_categories = (!strategy.generic_query_only)
            .then(|| context.per_indexer_categories.clone())
            .flatten();
        let season = (!strategy.generic_query_only)
            .then_some(strategy.season)
            .flatten();
        let episode = (!strategy.generic_query_only)
            .then_some(strategy.episode)
            .flatten();
        let absolute_episode = (!strategy.generic_query_only)
            .then_some(strategy.absolute_episode)
            .flatten();

        let sorted_ids = strategy
            .ids
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut sorted_categories = newznab_categories.clone().unwrap_or_default();
        sorted_categories.sort();
        sorted_categories.dedup();
        let mut sorted_aliases = context.tagged_aliases.clone();
        sorted_aliases.sort_by(|left, right| {
            (&left.name, &left.language).cmp(&(&right.name, &right.language))
        });
        let strategy_id = digest_json(
            HashDomain::IndexerQuerySignature,
            &serde_json::json!({
                "query": strategy.request_query,
                "ids": sorted_ids,
                "category": category,
                "facet": facet,
                "newznab_categories": sorted_categories,
                "season": season,
                "episode": episode,
                "absolute_episode": absolute_episode,
                "year": context.year,
                "tagged_aliases": sorted_aliases,
            }),
        );

        if let Some(existing) = by_identity.get(&strategy_id).copied() {
            if !prepared[existing].labels.contains(&strategy.label) {
                prepared[existing].labels.push(strategy.label.clone());
                prepared[existing].request.labels.push(strategy.label);
            }
            continue;
        }

        let request = IndexerSearchStrategyRequest {
            strategy_id: strategy_id.clone(),
            labels: vec![strategy.label.clone()],
            query: strategy.request_query,
            ids: strategy.ids,
            category,
            facet,
            id_search_facet: None,
            newznab_categories,
            season,
            episode,
            absolute_episode,
            year: context.year,
            tagged_aliases: context.tagged_aliases.clone(),
        };
        by_identity.insert(strategy_id.clone(), prepared.len());
        prepared.push(PreparedSearchStrategy {
            strategy_id,
            labels: vec![strategy.label],
            request,
            title_guard_mode,
        });
    }

    prepared
}

struct ReusableStrategySelection {
    live: Vec<PreparedSearchStrategy>,
    complete_count: usize,
    deferred_count: usize,
    replayed_result_count: usize,
}

async fn select_reusable_strategies(
    strategies: Vec<PreparedSearchStrategy>,
    reusable: &mut HashMap<String, ReusableStrategyState>,
    indexer_id: &str,
    page_sink: &IndexerSearchPageSink,
) -> AppResult<ReusableStrategySelection> {
    let now = Utc::now();
    let mut selection = ReusableStrategySelection {
        live: Vec::new(),
        complete_count: 0,
        deferred_count: 0,
        replayed_result_count: 0,
    };

    for strategy in strategies {
        let Some(mut state) = reusable.remove(&strategy.strategy_id) else {
            selection.live.push(strategy);
            continue;
        };
        selection.replayed_result_count = selection
            .replayed_result_count
            .saturating_add(state.candidates.len());
        for result in &mut state.candidates {
            result.indexer_id = Some(indexer_id.to_string());
        }
        if !state.candidates.is_empty() {
            page_sink
                .send(state.candidates)
                .await
                .map_err(|_| AppError::canceled("indexer scoring pipeline closed"))?;
        }

        match state.completion_state.as_str() {
            "complete" => selection.complete_count += 1,
            "deferred" if state.retry_at.is_some_and(|retry_at| retry_at > now) => {
                selection.deferred_count += 1;
            }
            _ => selection.live.push(strategy),
        }
    }

    Ok(selection)
}

#[derive(Debug, Default, Deserialize)]
struct ManagedIndexerMetadata {
    enable_rss: Option<bool>,
    enable_automatic_search: Option<bool>,
    #[serde(default)]
    caps_snapshot: Option<IndexerCapsSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdDispatchMode {
    LegacyAggregate,
    Aggregate,
    QueryOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextDispatchMode {
    None,
    FacetScoped,
    GenericOnly,
}

impl TextDispatchMode {
    fn can_dispatch(self) -> bool {
        !matches!(self, Self::None)
    }

    fn is_generic_only(self) -> bool {
        matches!(self, Self::GenericOnly)
    }
}

#[derive(Clone, Debug)]
struct ResolvedSearchCapabilities {
    caps: IndexerProviderCapabilities,
    id_dispatch_mode: IdDispatchMode,
    text_dispatch_mode: TextDispatchMode,
    query_only_reason: Option<&'static str>,
    transport_kind: Option<NabTransportKind>,
    caps_source: &'static str,
}

struct FilterStrategyContext<'a> {
    query: &'a str,
    season: Option<u32>,
    episode: Option<u32>,
    tagged_aliases: &'a [scryer_domain::TaggedAlias],
    title_guard_mode: TitleGuardMode,
    strategy_label: &'a str,
    is_rss_request: bool,
}

const MAX_INDEXER_ERROR_MESSAGE_BYTES: usize = 1024;

fn sanitize_indexer_error_message(message: &str) -> String {
    let redacted = scryer_application::challenge_solver::sanitize_proxy_error(message);
    let mut sanitized = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if sanitized.len() > MAX_INDEXER_ERROR_MESSAGE_BYTES {
        let mut end = MAX_INDEXER_ERROR_MESSAGE_BYTES;
        while !sanitized.is_char_boundary(end) {
            end -= 1;
        }
        sanitized.truncate(end);
    }
    sanitized
}

const SEARCH_CANDIDATE_REUSE_HOURS: i64 = 24;

/// Whether this search pass may be served from the persisted candidate corpus.
///
/// Reuse requires all three: Auto mode (Interactive never reused), a learning
/// context (no context means no diagnostics to rehydrate from), and the
/// context's own consent — which only the background convergence lanes give.
/// Operator-triggered Auto searches leave `candidate_reuse_allowed` false and
/// always fire the indexer live.
fn candidate_reuse_permitted(
    mode: SearchMode,
    learning_context: Option<&IndexerSearchLearningContext>,
) -> bool {
    mode == SearchMode::Auto
        && learning_context.is_some_and(|context| context.candidate_reuse_allowed)
}
const SEARCH_CANDIDATE_RETENTION_DAYS: i64 = 1;
const SEARCH_RUN_RETENTION_DAYS: i64 = 90;
const SEARCH_DIAGNOSTIC_CLEANUP_LIMIT: u32 = 500;
const SEARCH_DIAGNOSTIC_CLEANUP_RETRY_SECONDS: i64 = 3_600;
static LAST_SEARCH_DIAGNOSTIC_CLEANUP_DAY: AtomicI64 = AtomicI64::new(i64::MIN);
static NEXT_SEARCH_DIAGNOSTIC_CLEANUP_RETRY_AT: AtomicI64 = AtomicI64::new(i64::MIN);
static SEARCH_DIAGNOSTIC_CLEANUP_RUNNING: AtomicBool = AtomicBool::new(false);

struct SearchDiagnosticCleanupGuard;

impl Drop for SearchDiagnosticCleanupGuard {
    fn drop(&mut self) {
        SEARCH_DIAGNOSTIC_CLEANUP_RUNNING.store(false, Ordering::Release);
    }
}

struct ReusableStrategyState {
    completion_state: String,
    retry_at: Option<DateTime<Utc>>,
    candidates: Vec<IndexerSearchResult>,
}

fn reusable_strategy_provenance(branch: &str) -> ReleaseCandidateProvenance {
    let labels = branch.split('|').filter(|label| !label.is_empty());
    let primary_label = labels.clone().next().unwrap_or("fallback");
    ReleaseCandidateProvenance {
        search_subject_kind: ReleaseSearchSubjectKind::Freetext,
        strategy_kind: scryer_application::release_strategy_kind_for_label(primary_label, false),
        title_validated_upstream: labels.into_iter().any(|label| label.starts_with("ids")),
    }
}

#[derive(Clone)]
struct SearchDiagnosticsContext {
    repository: Arc<dyn IndexerSearchLearningRepository>,
    indexer_id: String,
    provider_type: String,
    search_session_id: String,
    scope_key: String,
    indexer_fingerprint: String,
}

impl SearchDiagnosticsContext {
    fn new(
        repository: Arc<dyn IndexerSearchLearningRepository>,
        config: &IndexerConfig,
        search_semantics_version: Option<u32>,
        learning_context: Option<&IndexerSearchLearningContext>,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
    ) -> Option<Self> {
        let learning_context = learning_context?;
        if learning_context.title_id.trim().is_empty()
            || learning_context.subject_kind == ReleaseSearchSubjectKind::Rss
        {
            return None;
        }

        let scope_key = format!(
            "{}:{}:{}:{}:{}:{}",
            learning_context.title_id.trim(),
            learning_context.facet.trim().to_ascii_lowercase(),
            learning_context.subject_kind.as_str(),
            season.map_or_else(|| "-".to_string(), |value| value.to_string()),
            episode.map_or_else(|| "-".to_string(), |value| value.to_string()),
            absolute_episode.map_or_else(|| "-".to_string(), |value| value.to_string()),
        );
        let indexer_fingerprint = digest_json(
            HashDomain::IndexerSearchIdentity,
            &indexer_search_identity(config, search_semantics_version),
        );

        Some(Self {
            repository,
            indexer_id: config.id.clone(),
            provider_type: config.provider_type.clone(),
            search_session_id: learning_context.search_session_id.clone(),
            scope_key,
            indexer_fingerprint,
        })
    }

    async fn persist_response(
        &self,
        query_signature: &str,
        branch: &str,
        raw_result_count: usize,
        response: &IndexerSearchResponse,
    ) -> AppResult<String> {
        let now = Utc::now();
        let run_id = uuid::Uuid::new_v4().to_string();
        let (completion_state, incomplete_reason, retry_after) = match response.completion {
            IndexerSearchCompletion::Complete => ("received_complete", None, None),
            IndexerSearchCompletion::Partial {
                reason,
                retry_after,
            } => ("received_partial", reason, retry_after),
        };
        let candidates = response
            .results
            .iter()
            .map(|candidate| IndexerSearchCandidateWrite {
                id: uuid::Uuid::new_v4().to_string(),
                run_id: run_id.clone(),
                search_session_id: self.search_session_id.clone(),
                indexer_id: self.indexer_id.clone(),
                scope_key: self.scope_key.clone(),
                query_signature: query_signature.to_string(),
                session_identity_hash: candidate_session_identity_hash(candidate),
                normalized: normalized_candidate(
                    candidate,
                    response.grab_current,
                    response.grab_max,
                ),
                created_at: now,
                reusable_until: now + Duration::hours(SEARCH_CANDIDATE_REUSE_HOURS),
                expires_at: now + Duration::days(SEARCH_CANDIDATE_RETENTION_DAYS),
            })
            .collect::<Vec<_>>();
        let run = IndexerSearchRunWrite {
            id: run_id.clone(),
            indexer_id: self.indexer_id.clone(),
            provider_type: self.provider_type.clone(),
            search_session_id: self.search_session_id.clone(),
            scope_key: self.scope_key.clone(),
            query_signature: query_signature.to_string(),
            branch: branch.to_string(),
            page: None,
            range_min_size: None,
            range_max_size: None,
            result_count: raw_result_count.min(u32::MAX as usize) as u32,
            completion_state: completion_state.to_string(),
            retry_at: retry_after
                .and_then(|delay| Duration::from_std(delay).ok())
                .map(|delay| now + delay),
            error_summary: incomplete_reason
                .map(|reason| format!("incomplete indexer search: {reason:?}"))
                .or_else(|| {
                    (!response.completion.is_complete())
                        .then(|| "incomplete indexer search response".to_string())
                }),
            indexer_fingerprint: self.indexer_fingerprint.clone(),
            created_at: now,
        };
        self.repository
            .record_search_diagnostics(&run, &candidates)
            .await?;
        maybe_cleanup_search_diagnostics(&self.repository, run.created_at);
        Ok(run_id)
    }

    async fn record_response(
        &self,
        query_signature: &str,
        branch: &str,
        raw_result_count: usize,
        response: &IndexerSearchResponse,
    ) {
        if let Err(error) = self
            .persist_response(query_signature, branch, raw_result_count, response)
            .await
        {
            warn!(
                indexer_id = self.indexer_id.as_str(),
                error = %error,
                "failed to persist indexer search diagnostics"
            );
        }
    }

    async fn record_error(
        &self,
        query_signature: &str,
        branch: &str,
        error: &AppError,
        retry_after: Option<std::time::Duration>,
    ) {
        let now = Utc::now();
        let run = IndexerSearchRunWrite {
            id: uuid::Uuid::new_v4().to_string(),
            indexer_id: self.indexer_id.clone(),
            provider_type: self.provider_type.clone(),
            search_session_id: self.search_session_id.clone(),
            scope_key: self.scope_key.clone(),
            query_signature: query_signature.to_string(),
            branch: branch.to_string(),
            page: None,
            range_min_size: None,
            range_max_size: None,
            result_count: 0,
            completion_state: if retry_after.is_some() {
                "deferred".to_string()
            } else {
                "errored".to_string()
            },
            retry_at: retry_after
                .and_then(|duration| Duration::from_std(duration).ok())
                .map(|duration| now + duration),
            error_summary: Some(sanitize_indexer_error_message(&error.to_string())),
            indexer_fingerprint: self.indexer_fingerprint.clone(),
            created_at: now,
        };
        self.persist(&run, &[]).await;
    }

    async fn reusable_strategies(
        &self,
        config: &IndexerConfig,
    ) -> HashMap<String, ReusableStrategyState> {
        let now = Utc::now();
        let created_after = now - Duration::hours(SEARCH_CANDIDATE_REUSE_HOURS);
        let states = match self
            .repository
            .list_reusable_search_strategies(
                &self.indexer_id,
                &self.scope_key,
                &self.indexer_fingerprint,
                created_after,
                now,
            )
            .await
        {
            Ok(states) => states,
            Err(error) => {
                warn!(
                    indexer_id = self.indexer_id.as_str(),
                    error = %error,
                    "failed to load reusable indexer strategy state"
                );
                return HashMap::new();
            }
        };

        let mut reusable = HashMap::with_capacity(states.len());
        for state in states {
            let mut candidates = match state.candidate_run_id.as_deref() {
                Some(run_id) => match self.persisted_candidates(run_id, config).await {
                    Ok(candidates) => candidates,
                    Err(error) => {
                        warn!(
                            indexer_id = self.indexer_id.as_str(),
                            query_signature = state.query_signature.as_str(),
                            error = %error,
                            "failed to rehydrate reusable indexer strategy candidates"
                        );
                        continue;
                    }
                },
                None => Vec::new(),
            };
            let provenance = reusable_strategy_provenance(&state.branch);
            for candidate in &mut candidates {
                candidate.provenance = Some(provenance.clone());
            }
            reusable.insert(
                state.query_signature,
                ReusableStrategyState {
                    completion_state: state.completion_state,
                    retry_at: state.retry_at,
                    candidates,
                },
            );
        }
        reusable
    }

    async fn persist(
        &self,
        run: &IndexerSearchRunWrite,
        candidates: &[IndexerSearchCandidateWrite],
    ) {
        if let Err(error) = self
            .repository
            .record_search_diagnostics(run, candidates)
            .await
        {
            warn!(
                indexer_id = self.indexer_id.as_str(),
                error = %error,
                "failed to persist indexer search diagnostics"
            );
        }
        maybe_cleanup_search_diagnostics(&self.repository, run.created_at);
    }

    async fn persisted_candidates(
        &self,
        run_id: &str,
        config: &IndexerConfig,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        self.repository
            .list_search_run_candidates(run_id)
            .await?
            .into_iter()
            .map(|record| reusable_candidate_from_record(record, config))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                AppError::Repository(
                    "persisted indexer candidate could not be rehydrated".to_string(),
                )
            })
    }
}

/// Domain-separated BLAKE3 digest of a JSON value.
///
/// Note the serialization is `serde_json::to_vec`, which with the workspace's
/// `preserve_order` feature is key-order sensitive. That is tolerable here: a
/// reordered value yields a different digest, which only costs a reuse cache
/// miss and a live search. The convergence ledger, where an order flap would
/// force a re-sweep, canonicalizes instead.
fn digest_json(domain: HashDomain, value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    blake3_identity_hex(domain, String::from_utf8_lossy(&bytes))
}

/// Delegates to the one canonical fingerprint. Staging writes under this value
/// and `finalize_search_session` retains by it, so this must be the same
/// function the discovery layer computes admissible fingerprints with — a
/// local reimplementation that drifted would silently discard every session's
/// corpus at finalize.
fn candidate_session_identity_hash(candidate: &IndexerSearchResult) -> String {
    scryer_application::release_candidate_fingerprint(candidate)
}

fn candidate_extra_string(candidate: &IndexerSearchResult, key: &str) -> Option<String> {
    candidate
        .extra
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn candidate_extra_strings(candidate: &IndexerSearchResult, key: &str) -> Vec<String> {
    candidate
        .extra
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn candidate_extra_i64(candidate: &IndexerSearchResult, key: &str) -> Option<i64> {
    candidate.extra.get(key).and_then(serde_json::Value::as_i64)
}

fn candidate_extra_f64(candidate: &IndexerSearchResult, key: &str) -> Option<f64> {
    candidate.extra.get(key).and_then(serde_json::Value::as_f64)
}

fn candidate_extra_bool(candidate: &IndexerSearchResult, key: &str) -> Option<bool> {
    candidate
        .extra
        .get(key)
        .and_then(serde_json::Value::as_bool)
}

fn normalized_candidate(
    candidate: &IndexerSearchResult,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
) -> NormalizedIndexerSearchCandidate {
    NormalizedIndexerSearchCandidate {
        provider_ref: candidate.guid.clone(),
        source: candidate.source.clone(),
        title: candidate.title.clone(),
        download_url: candidate.download_url.clone(),
        download_url_credential_keys: Vec::new(),
        link_url: candidate.link.clone(),
        link_url_credential_keys: Vec::new(),
        size_bytes: candidate.size_bytes,
        published_at: candidate.published_at.clone(),
        source_kind: candidate.source_kind.map(|kind| kind.as_str().to_string()),
        thumbs_up: candidate.thumbs_up,
        thumbs_down: candidate.thumbs_down,
        grabs: candidate.indexer_grabs,
        grab_current: grab_current.map(i64::from),
        grab_max: grab_max.map(i64::from),
        languages: candidate.indexer_languages.clone().unwrap_or_default(),
        subtitles: candidate.indexer_subtitles.clone().unwrap_or_default(),
        response_tvdb_id: candidate.response_attributes.tvdb_id.clone(),
        response_tmdb_id: candidate.response_attributes.tmdb_id.clone(),
        response_imdb_id: candidate.response_attributes.imdb_id.clone(),
        response_categories: candidate.response_attributes.categories.clone(),
        extra_categories: candidate_extra_strings(candidate, "categories"),
        season: candidate_extra_i64(candidate, "season"),
        episode: candidate_extra_i64(candidate, "episode"),
        absolute_episode: candidate_extra_i64(candidate, "absolute_episode"),
        series_names: candidate_extra_strings(candidate, "series_names"),
        release_group: candidate_extra_string(candidate, "group"),
        provider_source: candidate_extra_string(candidate, "source"),
        info_hash: candidate_extra_string(candidate, "info_hash"),
        seeders: candidate_extra_i64(candidate, "seeders"),
        peers: candidate_extra_i64(candidate, "peers"),
        download_volume_factor: candidate_extra_f64(candidate, "download_volume_factor"),
        upload_volume_factor: candidate_extra_f64(candidate, "upload_volume_factor"),
        protected: candidate_extra_bool(candidate, "protected"),
        tags: candidate_extra_strings(candidate, "tags"),
        provider_categories: candidate_extra_strings(candidate, "provider_categories"),
    }
}

fn reusable_candidate_from_record(
    record: ReusableIndexerSearchCandidate,
    config: &IndexerConfig,
) -> Option<IndexerSearchResult> {
    let normalized = record.normalized;
    let title = normalized.title.trim().to_string();
    if title.is_empty() {
        return None;
    }
    let download_url = normalized.download_url.clone();
    let link = normalized.link_url.clone();
    if download_url.is_none() && link.is_none() {
        return None;
    }
    let source_kind = normalized
        .source_kind
        .as_deref()
        .and_then(DownloadSourceKind::parse);
    let mut extra = HashMap::new();
    for (key, value) in [
        ("season", normalized.season),
        ("episode", normalized.episode),
        ("absolute_episode", normalized.absolute_episode),
        ("seeders", normalized.seeders),
        ("peers", normalized.peers),
        ("grab_current", normalized.grab_current),
        ("grab_max", normalized.grab_max),
    ] {
        if let Some(value) = value {
            extra.insert(key.to_string(), serde_json::json!(value));
        }
    }
    for (key, value) in [
        ("group", normalized.release_group.as_ref()),
        ("source", normalized.provider_source.as_ref()),
        ("info_hash", normalized.info_hash.as_ref()),
    ] {
        if let Some(value) = value {
            extra.insert(key.to_string(), serde_json::json!(value));
        }
    }
    for (key, values) in [
        ("series_names", &normalized.series_names),
        ("categories", &normalized.extra_categories),
        ("tags", &normalized.tags),
        ("provider_categories", &normalized.provider_categories),
    ] {
        if !values.is_empty() {
            extra.insert(key.to_string(), serde_json::json!(values));
        }
    }
    for (key, value) in [
        ("download_volume_factor", normalized.download_volume_factor),
        ("upload_volume_factor", normalized.upload_volume_factor),
    ] {
        if let Some(value) = value {
            extra.insert(key.to_string(), serde_json::json!(value));
        }
    }
    if let Some(value) = normalized.protected {
        extra.insert("protected".to_string(), serde_json::json!(value));
    }

    Some(IndexerSearchResult {
        indexer_id: Some(config.id.clone()),
        source: if normalized.source.trim().is_empty() {
            config.name.clone()
        } else {
            normalized.source
        },
        title,
        link,
        download_url,
        source_kind,
        size_bytes: normalized.size_bytes,
        published_at: normalized.published_at,
        thumbs_up: normalized.thumbs_up,
        thumbs_down: normalized.thumbs_down,
        indexer_languages: (!normalized.languages.is_empty()).then_some(normalized.languages),
        indexer_subtitles: (!normalized.subtitles.is_empty()).then_some(normalized.subtitles),
        indexer_grabs: normalized.grabs,
        password_hint: None,
        parsed_release_metadata: None,
        quality_profile_decision: None,
        extra,
        response_attributes: IndexerResponseAttributes {
            tvdb_id: normalized.response_tvdb_id,
            tmdb_id: normalized.response_tmdb_id,
            imdb_id: normalized.response_imdb_id,
            categories: normalized.response_categories,
        },
        guid: normalized.provider_ref,
        info_url: None,
        provenance: None,
        candidate_token: None,
        queue_scope: None,
        coverage_scope: None,
        auto_eligible: None,
        auto_decision_code: None,
        auto_decision_summary: None,
    })
}

async fn drain_search_diagnostics(
    repository: &Arc<dyn IndexerSearchLearningRepository>,
    now: DateTime<Utc>,
) -> AppResult<u32> {
    let mut deleted_total = 0_u32;
    loop {
        let deleted = repository
            .cleanup_search_diagnostics(
                now - Duration::days(SEARCH_CANDIDATE_RETENTION_DAYS),
                now - Duration::days(SEARCH_RUN_RETENTION_DAYS),
                SEARCH_DIAGNOSTIC_CLEANUP_LIMIT,
            )
            .await?;
        deleted_total = deleted_total.saturating_add(deleted);
        if deleted == 0 {
            return Ok(deleted_total);
        }
        tokio::task::yield_now().await;
    }
}

fn maybe_cleanup_search_diagnostics(
    repository: &Arc<dyn IndexerSearchLearningRepository>,
    now: DateTime<Utc>,
) {
    let day = now.timestamp().div_euclid(86_400);
    if LAST_SEARCH_DIAGNOSTIC_CLEANUP_DAY.load(Ordering::Relaxed) == day
        || now.timestamp() < NEXT_SEARCH_DIAGNOSTIC_CLEANUP_RETRY_AT.load(Ordering::Relaxed)
        || SEARCH_DIAGNOSTIC_CLEANUP_RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    let repository = repository.clone();
    tokio::spawn(async move {
        let _running_guard = SearchDiagnosticCleanupGuard;
        match drain_search_diagnostics(&repository, now).await {
            Ok(deleted) => {
                LAST_SEARCH_DIAGNOSTIC_CLEANUP_DAY.store(day, Ordering::Relaxed);
                NEXT_SEARCH_DIAGNOSTIC_CLEANUP_RETRY_AT.store(i64::MIN, Ordering::Relaxed);
                debug!(deleted, "cleaned up expired indexer search diagnostics");
            }
            Err(error) => {
                NEXT_SEARCH_DIAGNOSTIC_CLEANUP_RETRY_AT.store(
                    now.timestamp() + SEARCH_DIAGNOSTIC_CLEANUP_RETRY_SECONDS,
                    Ordering::Relaxed,
                );
                warn!(error = %error, "failed to clean up indexer search diagnostics");
            }
        }
    });
}

#[derive(Default)]
struct StrategyBatchHealth {
    any_success: bool,
    any_error: bool,
    retry_after: Option<std::time::Duration>,
    had_rate_limit: bool,
    had_solver_failure: bool,
    representative_error: Option<String>,
    rate_limit_error: Option<String>,
}

impl StrategyBatchHealth {
    fn mark_success(&mut self) {
        self.any_success = true;
    }

    fn mark_error(
        &mut self,
        error: &AppError,
        retry_after: Option<std::time::Duration>,
        rate_limited: bool,
    ) {
        self.any_error = true;
        self.had_rate_limit |= rate_limited;
        let message = sanitize_indexer_error_message(&error.to_string());
        if rate_limited {
            self.rate_limit_error.get_or_insert(message);
        } else {
            self.representative_error.get_or_insert(message);
        }
        if let Some(retry_after) = retry_after
            && self.retry_after.is_none_or(|current| retry_after > current)
        {
            self.retry_after = Some(retry_after);
        }
    }

    fn mark_solver_failure(&mut self) {
        self.had_solver_failure = true;
    }

    async fn apply(
        self,
        backoff_tracker: &IndexerBackoffTracker,
        indexer_configs: &Arc<dyn IndexerConfigRepository>,
        indexer_id: &str,
        indexer_name: &str,
        had_persisted_system_backoff: bool,
    ) {
        if self.any_success {
            MultiIndexerSearchClient::clear_indexer_last_error(
                indexer_configs,
                indexer_id,
                indexer_name,
            )
            .await;
            let had_in_memory_backoff = backoff_tracker.record_success(indexer_id).await;
            if had_in_memory_backoff || had_persisted_system_backoff {
                MultiIndexerSearchClient::clear_indexer_system_backoff(
                    indexer_configs,
                    indexer_id,
                    indexer_name,
                )
                .await;
            }
        } else if self.any_error {
            MultiIndexerSearchClient::record_indexer_last_error(
                indexer_configs,
                indexer_id,
                indexer_name,
                self.rate_limit_error
                    .clone()
                    .or_else(|| self.representative_error.clone()),
            )
            .await;
        }

        if self.any_error && !self.any_success && !self.had_rate_limit && !self.had_solver_failure {
            let backoff = backoff_tracker
                .record_failure(indexer_id, self.retry_after)
                .await;
            MultiIndexerSearchClient::record_indexer_system_backoff(
                indexer_configs,
                indexer_id,
                indexer_name,
                backoff.clone(),
            )
            .await;
            warn!(
                indexer = indexer_name,
                disabled_until = %backoff.disabled_until,
                escalation_level = backoff.escalation_level,
                "indexer backoff escalated"
            );
        } else if self.any_error
            && !self.any_success
            && self.had_solver_failure
            && !self.had_rate_limit
        {
            // The challenge solver failed, not the indexer: keep the indexer
            // out of operational backoff and let proxy health carry the blame.
            warn!(
                indexer = indexer_name,
                "proxy solver failure recorded without operational backoff"
            );
        } else if self.any_error && !self.any_success {
            warn!(
                indexer = indexer_name,
                retry_after_secs = self.retry_after.map(|delay| delay.as_secs()),
                "indexer rate-limit failure recorded without operational backoff"
            );
        }
    }
}

/// Global admission limit for complete automatic indexer strategies. This is
/// intentionally independent of the number of configured indexers: callers
/// share one bounded background-search lane across cloned clients.
const BACKGROUND_INDEXER_SEARCH_CONCURRENCY_LIMIT: usize = 4;
const INTERACTIVE_INDEXER_SEARCH_CONCURRENCY_LIMIT: usize = 24;
const LEARNED_EMPTY_SUPPRESSION_THRESHOLD: u32 = 3;
const LEARNED_SUPPRESSION_REPROBE_INTERVAL_DAYS: i64 = 7;

fn log_indexer_skip(
    mode: SearchMode,
    indexer_name: &str,
    reason: &str,
    disabled_until: Option<chrono::DateTime<chrono::Utc>>,
) {
    if matches!(mode, SearchMode::Interactive) {
        if let Some(disabled_until) = disabled_until {
            info!(
                indexer = indexer_name,
                reason,
                disabled_until = %disabled_until,
                "skipping indexer before dispatch"
            );
        } else {
            info!(
                indexer = indexer_name,
                reason, "skipping indexer before dispatch"
            );
        }
    } else if let Some(disabled_until) = disabled_until {
        debug!(
            indexer = indexer_name,
            reason,
            disabled_until = %disabled_until,
            "skipping indexer before dispatch"
        );
    } else {
        debug!(
            indexer = indexer_name,
            reason, "skipping indexer before dispatch"
        );
    }
}

fn stable_phase_seconds(key: &str, interval_seconds: u64) -> u64 {
    if interval_seconds == 0 {
        return 0;
    }
    let mut hash = 0xcbf29ce484222325u64;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash % interval_seconds
}

fn should_run_fallback_tier(
    mode: SearchMode,
    primary_usable_result_count: usize,
    primary_attempted: bool,
    primary_had_error: bool,
    fallback_strategies: &[SearchStrategy],
) -> bool {
    if mode == SearchMode::Auto && primary_had_error {
        return false;
    }

    primary_usable_result_count == 0 && primary_attempted && !fallback_strategies.is_empty()
}

fn rate_limit_signal_from_error(error: &AppError) -> Option<RateLimitSignal> {
    RateLimitSignal::from_error(error)
}

fn indexer_rss_feedback_summary(
    lease: &SchedulerLease,
    response: &IndexerSearchResponse,
) -> (
    Option<String>,
    Option<DateTime<Utc>>,
    Option<u32>,
    Vec<String>,
) {
    if lease.operation != SchedulerOperation::Rss && lease.intent != SchedulerIntent::BackgroundRss
    {
        return (None, None, None, Vec::new());
    }

    let mut newest_identity = None;
    let mut newest_published_at = None;
    let mut fallback_identity = None;
    let mut seen_identities = Vec::with_capacity(response.results.len());
    for result in &response.results {
        let identity = result
            .guid
            .clone()
            .or_else(|| result.link.clone())
            .or_else(|| result.download_url.clone())
            .unwrap_or_else(|| result.title.clone());
        fallback_identity.get_or_insert_with(|| identity.clone());
        seen_identities.push(identity.clone());
        let Some(published_at) = result
            .published_at
            .as_deref()
            .and_then(parse_indexer_published_at)
        else {
            continue;
        };
        if newest_published_at.is_none_or(|current| published_at > current) {
            newest_published_at = Some(published_at);
            newest_identity = Some(identity);
        }
    }

    (
        newest_identity.or(fallback_identity),
        newest_published_at,
        Some(response.results.len().min(u32::MAX as usize) as u32),
        seen_identities,
    )
}

fn parse_indexer_published_at(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
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

fn record_auto_strategy_selection(
    indexer_name: &str,
    caps_source: &'static str,
    primary_strategies: &[SearchStrategy],
    fallback_strategies: &[SearchStrategy],
) {
    let strategy_count = primary_strategies.len() + fallback_strategies.len();
    let primary_labels = primary_strategies
        .iter()
        .map(|strategy| strategy.label.as_str())
        .collect::<Vec<_>>();
    let fallback_labels = fallback_strategies
        .iter()
        .map(|strategy| strategy.label.as_str())
        .collect::<Vec<_>>();

    metrics::histogram!(
        "scryer_indexer_auto_strategy_count",
        "indexer" => indexer_name.to_string(),
        "caps_source" => caps_source.to_string()
    )
    .record(strategy_count as f64);

    debug!(
        indexer = indexer_name,
        mode = "auto",
        caps_source,
        auto_strategy_count = strategy_count,
        primary_strategy_count = primary_strategies.len(),
        fallback_strategy_count = fallback_strategies.len(),
        primary_strategies = ?primary_labels,
        fallback_strategies = ?fallback_labels,
        "selected automatic indexer search strategies"
    );
}

async fn record_strategy_learning_outcome(
    search_learning: &Arc<dyn IndexerSearchLearningRepository>,
    learning_context: Option<&IndexerSearchLearningContext>,
    mode: SearchMode,
    indexer_id: &str,
    indexer_name: &str,
    strategy_label: &str,
    usable_hits: usize,
) {
    if mode != SearchMode::Auto {
        return;
    }
    let Some(learning_context) = learning_context else {
        return;
    };
    if learning_context.title_id.trim().is_empty()
        || learning_context.subject_kind == ReleaseSearchSubjectKind::Rss
    {
        return;
    }
    let Some(strategy_key) = learning_strategy_key(strategy_label) else {
        return;
    };

    let key = IndexerSearchLearningKey {
        indexer_id: indexer_id.to_string(),
        title_id: learning_context.title_id.clone(),
        facet: learning_context.facet.clone(),
        strategy_key: strategy_key.to_string(),
    };
    let usable_hits = usable_hits.min(u32::MAX as usize) as u32;

    if let Err(error) = search_learning.record_outcome(&key, usable_hits).await {
        warn!(
            indexer = indexer_name,
            strategy = strategy_key,
            error = %error,
            "failed to record indexer search learning outcome"
        );
        return;
    }

    let records = match search_learning
        .list_for_title(
            indexer_id,
            &learning_context.title_id,
            &learning_context.facet,
        )
        .await
    {
        Ok(records) => records,
        Err(error) => {
            warn!(
                indexer = indexer_name,
                error = %error,
                "failed to load indexer search learning outcomes"
            );
            return;
        }
    };

    for record in records
        .iter()
        .filter(|record| is_learning_id_strategy_key(&record.key.strategy_key))
        .filter(|record| !record.suppressed)
        .filter(|record| record.empty_successes >= LEARNED_EMPTY_SUPPRESSION_THRESHOLD)
        .filter(|record| record.usable_successes == 0)
    {
        let has_working_alternative = records.iter().any(|candidate| {
            candidate.key.strategy_key != record.key.strategy_key && candidate.usable_successes > 0
        });
        if !has_working_alternative {
            continue;
        }

        if let Err(error) = search_learning.set_suppressed(&record.key, true).await {
            warn!(
                indexer = indexer_name,
                strategy = record.key.strategy_key.as_str(),
                error = %error,
                "failed to suppress learned-empty indexer search strategy"
            );
            continue;
        }

        info!(
            indexer = indexer_name,
            title_id = learning_context.title_id.as_str(),
            facet = learning_context.facet.as_str(),
            strategy = record.key.strategy_key.as_str(),
            empty_successes = record.empty_successes,
            "suppressing learned-empty automatic indexer search strategy"
        );
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

fn is_title_query_strategy_label(label: &str) -> bool {
    is_freetext_strategy_label(label) || label == "fallback"
}

fn learning_strategy_key(label: &str) -> Option<&'static str> {
    match label {
        "ids_abs" => Some("v2:ids_abs"),
        "ids_sxex" => Some("v2:ids_sxex"),
        "ids" => Some("v2:ids"),
        "freetext" | "freetext_alias" | "fallback" => Some("v2:freetext"),
        _ => None,
    }
}

fn is_learning_id_strategy_key(strategy_key: &str) -> bool {
    matches!(strategy_key, "v2:ids_abs" | "v2:ids_sxex" | "v2:ids")
}

fn learning_record_updated_at(record: &IndexerSearchLearningRecord) -> Option<DateTime<Utc>> {
    record
        .updated_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn learned_suppression_is_active(record: &IndexerSearchLearningRecord, now: DateTime<Utc>) -> bool {
    if !record.suppressed || !is_learning_id_strategy_key(&record.key.strategy_key) {
        return false;
    }

    let Some(updated_at) = learning_record_updated_at(record) else {
        return false;
    };

    updated_at >= now - Duration::days(LEARNED_SUPPRESSION_REPROBE_INTERVAL_DAYS)
}

async fn suppress_learned_strategies(
    search_learning: &Arc<dyn IndexerSearchLearningRepository>,
    indexer_name: &str,
    mode: SearchMode,
    strategies: Vec<SearchStrategy>,
    learned_records: &[IndexerSearchLearningRecord],
    now: DateTime<Utc>,
) -> Vec<SearchStrategy> {
    if mode != SearchMode::Auto || learned_records.is_empty() {
        return strategies;
    }

    let stale_before = now - Duration::days(LEARNED_SUPPRESSION_REPROBE_INTERVAL_DAYS);
    let mut suppressed_keys = HashSet::new();
    for record in learned_records {
        if !record.suppressed || !is_learning_id_strategy_key(&record.key.strategy_key) {
            continue;
        }

        if learned_suppression_is_active(record, now) {
            suppressed_keys.insert(record.key.strategy_key.as_str());
            continue;
        }

        match search_learning
            .try_claim_suppressed_reprobe(&record.key, stale_before)
            .await
        {
            Ok(true) => {
                debug!(
                    indexer = indexer_name,
                    strategy = record.key.strategy_key.as_str(),
                    "claimed learned-empty indexer strategy re-probe"
                );
            }
            Ok(false) => {
                suppressed_keys.insert(record.key.strategy_key.as_str());
            }
            Err(error) => {
                warn!(
                    indexer = indexer_name,
                    strategy = record.key.strategy_key.as_str(),
                    error = %error,
                    "failed to claim learned-empty indexer strategy re-probe"
                );
                suppressed_keys.insert(record.key.strategy_key.as_str());
            }
        }
    }

    if suppressed_keys.is_empty() {
        return strategies;
    }

    strategies
        .into_iter()
        .filter(|strategy| {
            let Some(strategy_key) = learning_strategy_key(&strategy.label) else {
                return true;
            };
            let suppressed =
                is_learning_id_strategy_key(strategy_key) && suppressed_keys.contains(strategy_key);
            if suppressed {
                debug!(
                    indexer = indexer_name,
                    strategy = strategy_key,
                    "skipping learned-empty indexer strategy for automatic search"
                );
            }
            !suppressed
        })
        .collect()
}

fn should_defer_freetext_to_fallback(_facet: &str, strategies: &[SearchStrategy]) -> bool {
    strategies
        .iter()
        .any(|strategy| !is_freetext_strategy_label(&strategy.label))
        && strategies
            .iter()
            .any(|strategy| is_freetext_strategy_label(&strategy.label))
}

fn split_strategy_tiers(
    mode: SearchMode,
    facet: &str,
    strategies: Vec<SearchStrategy>,
) -> (Vec<SearchStrategy>, Vec<SearchStrategy>) {
    if mode == SearchMode::Auto {
        return split_auto_strategy_tiers(strategies);
    }

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

fn split_auto_strategy_tiers(
    strategies: Vec<SearchStrategy>,
) -> (Vec<SearchStrategy>, Vec<SearchStrategy>) {
    if strategies.len() <= 1 {
        return (strategies, Vec::new());
    }

    let mut primary_candidates = Vec::new();
    let mut fallback_candidates = Vec::new();

    for strategy in strategies {
        if is_title_query_strategy_label(&strategy.label) {
            fallback_candidates.push(strategy);
        } else {
            primary_candidates.push(strategy);
        }
    }

    if primary_candidates.is_empty() {
        return (
            take_best_auto_strategy(&mut fallback_candidates)
                .into_iter()
                .collect(),
            Vec::new(),
        );
    }

    let primary = take_best_auto_strategy(&mut primary_candidates)
        .into_iter()
        .collect();
    let fallback = take_best_auto_strategy(&mut fallback_candidates)
        .into_iter()
        .collect();

    (primary, fallback)
}

fn take_best_auto_strategy(strategies: &mut Vec<SearchStrategy>) -> Option<SearchStrategy> {
    let index = strategies
        .iter()
        .enumerate()
        .min_by_key(|(_, strategy)| auto_strategy_rank(strategy))
        .map(|(index, _)| index)?;
    Some(strategies.swap_remove(index))
}

fn auto_strategy_rank(strategy: &SearchStrategy) -> (u8, u8) {
    match strategy.label.as_str() {
        "ids_abs" => (0, 0),
        "ids_sxex" => (0, 1),
        "ids" => (0, 2),
        "rss" => (0, 3),
        "freetext" => (1, 0),
        "freetext_alias" => (1, 1),
        "fallback" => (1, 2),
        _ if !strategy.ids.is_empty() => (0, 4),
        _ => (1, 3),
    }
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

/// Compatibility per-indexer limiter for explicit provider config.
///
/// Host-level default pacing is owned by scryer-outbound-http. This limiter
/// only applies when an indexer config/plugin declares a positive interval.
#[derive(Clone)]
struct IndexerRateLimiter {
    next_request: Arc<Mutex<HashMap<String, tokio::time::Instant>>>,
}

impl IndexerRateLimiter {
    fn new() -> Self {
        Self {
            next_request: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Wait until the configured rate limit period has elapsed for this
    /// indexer. Missing/zero values are ignored so the shared outbound host RPS
    /// limiter is the sole default pacing owner.
    async fn acquire(&self, indexer_id: &str, rate_limit_seconds: Option<i64>) {
        let interval_secs = rate_limit_seconds.unwrap_or_default().max(0) as u64;
        if interval_secs == 0 {
            return;
        }

        let interval = std::time::Duration::from_secs(interval_secs);
        let scheduled_at = {
            let now = tokio::time::Instant::now();
            let mut map = self.next_request.lock().await;
            let scheduled_at = map.get(indexer_id).copied().unwrap_or(now).max(now);
            map.insert(indexer_id.to_string(), scheduled_at + interval);
            scheduled_at
        };
        tokio::time::sleep_until(scheduled_at).await;
    }
}

/// Short escalating system backoff periods. Provider `Retry-After` handling can
/// choose longer when explicitly supplied, but generic storm containment caps at
/// one hour to avoid stranding every indexer after one transient burst.
const BACKOFF_PERIODS_SECS: &[u64] = &[
    5 * 60,  // 5 minutes
    10 * 60, // 10 minutes
    15 * 60, // 15 minutes
    30 * 60, // 30 minutes
    60 * 60, // 1 hour
];

#[derive(Clone, Debug)]
struct IndexerBackoffState {
    escalation_level: usize,
    disabled_until: Option<chrono::DateTime<chrono::Utc>>,
}

/// In-memory indexer backoff tracker. Persistent system backoffs seed this
/// state on startup/search so escalation survives process restarts.
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

    async fn seed_persisted(&self, indexer_id: &str, backoff: &IndexerSystemBackoff) {
        let mut map = self.state.lock().await;
        let state = map
            .entry(indexer_id.to_string())
            .or_insert(IndexerBackoffState {
                escalation_level: 0,
                disabled_until: None,
            });
        state.escalation_level = state.escalation_level.max(backoff.escalation_level);
        if backoff.disabled_until > chrono::Utc::now()
            && state
                .disabled_until
                .is_none_or(|current| backoff.disabled_until > current)
        {
            state.disabled_until = Some(backoff.disabled_until);
        }
    }

    /// Record a failure and escalate the backoff level. Returns the persisted row.
    async fn record_failure(
        &self,
        indexer_id: &str,
        retry_after: Option<std::time::Duration>,
    ) -> IndexerSystemBackoff {
        let mut map = self.state.lock().await;
        let state = map
            .entry(indexer_id.to_string())
            .or_insert(IndexerBackoffState {
                escalation_level: 0,
                disabled_until: None,
            });

        if let Some(until) = state.disabled_until
            && until > chrono::Utc::now()
        {
            return IndexerSystemBackoff {
                disabled_until: until,
                escalation_level: state.escalation_level,
            };
        }

        let period_index = state.escalation_level.min(BACKOFF_PERIODS_SECS.len() - 1);
        let backoff_secs = retry_after
            .map(|duration| duration.as_secs())
            .unwrap_or(BACKOFF_PERIODS_SECS[period_index]);
        let backoff_secs = backoff_secs.min(i64::MAX as u64) as i64;
        let until = chrono::Utc::now() + chrono::Duration::seconds(backoff_secs);

        state.escalation_level = (state.escalation_level + 1).min(BACKOFF_PERIODS_SECS.len());
        state.disabled_until = Some(until);

        IndexerSystemBackoff {
            disabled_until: until,
            escalation_level: state.escalation_level,
        }
    }

    /// Record a success and de-escalate by one level. Returns true when local
    /// backoff state existed and may need persistent cleanup.
    async fn record_success(&self, indexer_id: &str) -> bool {
        let mut map = self.state.lock().await;
        if let Some(state) = map.get_mut(indexer_id) {
            state.escalation_level = state.escalation_level.saturating_sub(1);
            if state.escalation_level == 0 {
                state.disabled_until = None;
            }
            true
        } else {
            false
        }
    }

    /// Forget everything held for one indexer: escalation level and disabled
    /// window alike. Returns true when there was state to drop.
    async fn clear(&self, indexer_id: &str) -> bool {
        self.state.lock().await.remove(indexer_id).is_some()
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
type RssFeedCache = Arc<Mutex<HashMap<String, Arc<RssFeedCacheEntry>>>>;

/// Indexer ids observed answering the facet-scoped RSS form with "function not
/// available". They keep sweeping — the next cycle just asks the bare-query way.
type RssBareQueryIndexers = Arc<Mutex<HashSet<String>>>;

/// Which shape the RSS "latest releases" sweep takes for one indexer. Both
/// forms participate in RSS; only the request differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RssRequestForm {
    /// The facet-scoped nab function (`tvsearch`/`movie`), carrying the routed
    /// categories.
    Nab,
    /// The bare "latest releases" query every newznab-shaped endpoint answers.
    BareQuery,
}

struct RssFeedCacheEntry {
    cell: tokio::sync::OnceCell<Result<Vec<IndexerSearchResult>, String>>,
    initialization_lock: Arc<Mutex<()>>,
    feedback_claimed: AtomicBool,
}

impl RssFeedCacheEntry {
    fn new() -> Self {
        Self {
            cell: tokio::sync::OnceCell::new(),
            initialization_lock: Arc::new(Mutex::new(())),
            feedback_claimed: AtomicBool::new(false),
        }
    }

    fn claim_feedback(&self) -> bool {
        !self.feedback_claimed.swap(true, Ordering::AcqRel)
    }
}

#[derive(Clone)]
pub struct MultiIndexerSearchClient {
    indexer_configs: Arc<dyn IndexerConfigRepository>,
    proxy_configs: Arc<dyn ProxyConfigRepository>,
    stats_tracker: Arc<dyn IndexerStatsTracker>,
    search_learning: Arc<dyn IndexerSearchLearningRepository>,
    indexer_errors: Arc<dyn IndexerErrorRepository>,
    plugin_provider: Arc<dyn IndexerPluginProvider>,
    upstream_scheduler: Arc<dyn UpstreamScheduler>,
    rate_limiter: IndexerRateLimiter,
    backoff_tracker: IndexerBackoffTracker,
    rss_feed_cache: RssFeedCache,
    rss_bare_query_indexers: RssBareQueryIndexers,
    background_search_limit: Arc<Semaphore>,
    interactive_search_limit: Arc<Semaphore>,
}

impl MultiIndexerSearchClient {
    fn effective_indexer_search_timeout(
        proxy_config: Option<&scryer_domain::ProxyConfig>,
    ) -> std::time::Duration {
        effective_indexer_timeout(proxy_config.map(|config| config.request_timeout_seconds))
    }

    pub fn new(
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        stats_tracker: Arc<dyn IndexerStatsTracker>,
        plugin_provider: Arc<dyn IndexerPluginProvider>,
    ) -> Self {
        Self {
            indexer_configs,
            proxy_configs: Arc::new(NullProxyConfigRepository),
            stats_tracker,
            search_learning: Arc::new(NullIndexerSearchLearningRepository),
            indexer_errors: Arc::new(NullIndexerErrorRepository),
            plugin_provider,
            upstream_scheduler: Arc::new(NullUpstreamScheduler),
            rate_limiter: IndexerRateLimiter::new(),
            backoff_tracker: IndexerBackoffTracker::new(),
            rss_feed_cache: Arc::new(Mutex::new(HashMap::new())),
            rss_bare_query_indexers: Arc::new(Mutex::new(HashSet::new())),
            background_search_limit: Arc::new(Semaphore::new(
                BACKGROUND_INDEXER_SEARCH_CONCURRENCY_LIMIT,
            )),
            interactive_search_limit: Arc::new(Semaphore::new(
                INTERACTIVE_INDEXER_SEARCH_CONCURRENCY_LIMIT,
            )),
        }
    }

    pub fn with_proxy_config_repository(
        mut self,
        proxy_configs: Arc<dyn ProxyConfigRepository>,
    ) -> Self {
        self.proxy_configs = proxy_configs;
        self
    }

    pub fn with_search_learning_repository(
        mut self,
        search_learning: Arc<dyn IndexerSearchLearningRepository>,
    ) -> Self {
        self.search_learning = search_learning;
        self
    }

    pub fn with_indexer_error_repository(
        mut self,
        indexer_errors: Arc<dyn IndexerErrorRepository>,
    ) -> Self {
        self.indexer_errors = indexer_errors;
        self
    }

    pub fn with_upstream_scheduler(
        mut self,
        upstream_scheduler: Arc<dyn UpstreamScheduler>,
    ) -> Self {
        self.upstream_scheduler = upstream_scheduler;
        self
    }

    fn scheduler_intent(mode: SearchMode, is_rss_request: bool) -> SchedulerIntent {
        if is_rss_request {
            SchedulerIntent::BackgroundRss
        } else {
            match mode {
                SearchMode::Interactive => SchedulerIntent::InteractiveSearch,
                SearchMode::Auto => SchedulerIntent::BackgroundAcquisition,
            }
        }
    }

    fn scheduler_keys_for_indexer(config: &IndexerConfig) -> (HostKey, DestinationKey) {
        let host_key = reqwest::Url::parse(config.base_url.as_str())
            .ok()
            .and_then(|url| url.host_str().map(HostKey::from))
            .unwrap_or_else(|| {
                let fallback = config
                    .base_url
                    .trim()
                    .trim_end_matches('/')
                    .trim()
                    .to_string();
                let fallback = if fallback.is_empty() {
                    config.id.clone()
                } else {
                    fallback
                };
                HostKey::from(fallback)
            });
        (
            host_key,
            DestinationKey::from(config.rate_limit_domain_key()),
        )
    }

    fn register_managed_proxy_host_profile(config: &IndexerConfig, host_key: &HostKey) {
        let is_managed_proxy =
            config.managed_parent_config_id.is_some() || config.provider_type.trim() == "prowlarr";
        if !is_managed_proxy {
            return;
        }
        RateLimitRegistry::new().register_host_profile(
            host_key.clone(),
            HostRpsProfile::limited(LOCAL_MANAGED_HOST_RPS, LOCAL_MANAGED_HOST_RPS_BURST),
            HostRpsProfileSource::ExplicitRegistration,
        );
    }

    fn scheduler_admission_candidate_id(admission: &SchedulerAdmission) -> &SchedulerCandidateId {
        match admission {
            SchedulerAdmission::Admit { candidate_id, .. }
            | SchedulerAdmission::Defer { candidate_id, .. }
            | SchedulerAdmission::Skip { candidate_id, .. } => candidate_id,
        }
    }

    fn scheduler_rss_activity(
        snapshot: Option<&SchedulerSnapshot>,
        host_key: &HostKey,
        destination_key: &DestinationKey,
        account_quota_key: Option<&scryer_application::AccountQuotaKey>,
        rss_request_key: Option<&str>,
    ) -> SchedulerRssActivity {
        let Some(snapshot) = snapshot else {
            return SchedulerRssActivity::default();
        };
        snapshot
            .entries
            .iter()
            .filter(|entry| {
                &entry.host_key == host_key
                    && &entry.destination_key == destination_key
                    && entry.account_quota_key.as_ref() == account_quota_key
                    && entry.rss_request_key.as_deref() == rss_request_key
            })
            .fold(SchedulerRssActivity::default(), |activity, entry| {
                SchedulerRssActivity {
                    last_successful_poll_at: activity.last_successful_poll_at.max(
                        entry
                            .rss_last_successful_poll_at
                            .or(entry.last_successful_at),
                    ),
                    last_attempt_at: activity
                        .last_attempt_at
                        .max(entry.rss_last_attempt_at.or(entry.last_attempt_at)),
                    target_interval: activity.target_interval.or(entry.rss_target_interval),
                    latest_safe_poll_at: activity
                        .latest_safe_poll_at
                        .max(entry.rss_latest_safe_poll_at),
                    estimated_feed_depth: activity
                        .estimated_feed_depth
                        .or(entry.rss_estimated_feed_depth),
                    freshness_risk: activity.freshness_risk.or(entry.rss_freshness_risk),
                    destination_recent_activity_at: activity
                        .destination_recent_activity_at
                        .max(entry.rss_destination_recent_activity_at),
                }
            })
    }

    fn rss_freshness_context(
        config: &IndexerConfig,
        now: DateTime<Utc>,
        activity: SchedulerRssActivity,
    ) -> RssFreshnessContext {
        // The first poll of an indexer has no persisted cadence entry yet, so
        // the phased window, the safe-poll time and the freshness risk are all
        // derived from this interval. It has to honour
        // `SCRYER_RSS_TARGET_INTERVAL_SECS` too, or the override would not take
        // effect until after a full default-length window had already elapsed.
        let rss_target_interval_secs = crate::upstream_scheduler::rss_target_interval()
            .as_secs()
            .clamp(1, i64::MAX as u64) as i64;
        let last_successful_poll_at = activity.last_successful_poll_at;
        let last_attempt_at = activity.last_attempt_at;
        let phase = stable_phase_seconds(&config.id, rss_target_interval_secs as u64) as i64;
        let timestamp = now.timestamp();
        let window_start = timestamp - timestamp.rem_euclid(rss_target_interval_secs);
        let phased_safe_poll_at =
            DateTime::<Utc>::from_timestamp(window_start + phase, 0).unwrap_or(now);
        let target_interval = activity
            .target_interval
            .and_then(|duration| chrono::Duration::from_std(duration).ok())
            .unwrap_or_else(|| Duration::seconds(rss_target_interval_secs));
        let latest_safe_poll_at = [
            last_successful_poll_at.map(|last_activity| last_activity + target_interval),
            last_attempt_at.map(|last_activity| last_activity + target_interval),
            activity.latest_safe_poll_at,
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(phased_safe_poll_at);
        let freshness_risk = activity.freshness_risk.unwrap_or_else(|| {
            last_successful_poll_at
                .map(|last_activity| {
                    let elapsed = (now - last_activity).num_seconds().max(0) as f64;
                    (elapsed / rss_target_interval_secs as f64).clamp(0.0, 1.0)
                })
                .unwrap_or_else(|| {
                    if now >= latest_safe_poll_at {
                        1.0
                    } else {
                        let elapsed = timestamp - window_start;
                        (elapsed as f64 / rss_target_interval_secs as f64).clamp(0.0, 1.0)
                    }
                })
        });

        RssFreshnessContext {
            last_successful_poll_at,
            last_attempt_at,
            target_interval: activity
                .target_interval
                .unwrap_or_else(|| std::time::Duration::from_secs(rss_target_interval_secs as u64)),
            latest_safe_poll_at,
            estimated_feed_depth: activity.estimated_feed_depth,
            freshness_risk,
            destination_recent_activity_at: activity
                .destination_recent_activity_at
                .or(last_attempt_at)
                .or(last_successful_poll_at),
            account_quota_budget: None,
        }
    }

    async fn record_indexer_scheduler_feedback(
        &self,
        lease: Option<SchedulerLease>,
        response: &IndexerSearchResponse,
        outcome: SchedulerFeedbackOutcome,
        retry_after: Option<std::time::Duration>,
        cooldown_action: RateLimitCooldownAction,
    ) {
        let Some(lease) = lease else {
            return;
        };
        let (
            rss_last_seen_release_identity,
            rss_last_seen_release_published_at,
            rss_feed_result_count,
            rss_seen_release_identities,
        ) = indexer_rss_feedback_summary(&lease, response);
        if let Err(error) = self
            .upstream_scheduler
            .record_feedback(SchedulerFeedback {
                host_key: lease.host_key.clone(),
                destination_key: lease.destination_key.clone(),
                account_quota_key: lease.account_quota_key.clone(),
                lease: Some(lease),
                outcome,
                observed_api_current: response.api_current.map(u64::from),
                observed_api_max: response.api_max.map(u64::from),
                observed_grab_current: response.grab_current.map(u64::from),
                observed_grab_max: response.grab_max.map(u64::from),
                retry_after,
                cooldown_action,
                rss_last_seen_release_identity,
                rss_last_seen_release_published_at,
                rss_feed_result_count,
                rss_seen_release_identities,
                observed_at: chrono::Utc::now(),
            })
            .await
        {
            warn!(error = %error, "failed to record indexer scheduler feedback");
        }
    }

    async fn record_indexer_scheduler_error(
        &self,
        lease: Option<SchedulerLease>,
        error: &AppError,
    ) {
        let response = IndexerSearchResponse {
            results: vec![],

            completion: IndexerSearchCompletion::Partial {
                reason: Some(IndexerSearchIncompleteReason::UpstreamFailure),
                retry_after: None,
            },
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
            indexer_outcomes: Vec::new(),
        };
        // Solver-service failures never reached the indexer: report transport
        // trouble to the scheduler instead of blaming the provider.
        if scryer_application::challenge_solver::is_solver_service_error_message(&error.to_string())
        {
            self.record_indexer_scheduler_feedback(
                lease,
                &response,
                SchedulerFeedbackOutcome::TransportFailure,
                None,
                RateLimitCooldownAction::None,
            )
            .await;
            return;
        }
        let rate_limit_signal = rate_limit_signal_from_error(error);
        let retry_after = rate_limit_signal
            .as_ref()
            .and_then(|signal| signal.retry_after);
        let cooldown_action = rate_limit_signal
            .as_ref()
            .map(|signal| signal.cooldown_action)
            .unwrap_or(RateLimitCooldownAction::None);
        let outcome = if rate_limit_signal.is_some() {
            SchedulerFeedbackOutcome::RateLimited
        } else {
            SchedulerFeedbackOutcome::ProviderFailure
        };
        self.record_indexer_scheduler_feedback(
            lease,
            &response,
            outcome,
            retry_after,
            cooldown_action,
        )
        .await;
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "test and direct callers use the same search envelope as the IndexerClient trait"
    )]
    pub async fn search(
        &self,
        query: String,
        ids: HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<scryer_domain::TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        <Self as IndexerClient>::search(
            self,
            query,
            ids,
            category,
            facet,
            id_search_facet,
            newznab_categories,
            indexer_routing,
            mode,
            match mode {
                SearchMode::Interactive => IndexerErrorOperation::InteractiveSearch,
                SearchMode::Auto => IndexerErrorOperation::AutomaticSearch,
            },
            season,
            episode,
            absolute_episode,
            // Direct callers search without a resolved subject, so there is no
            // year they can vouch for; the trait surface carries it instead.
            None,
            tagged_aliases,
            None,
            CancellationToken::new(),
        )
        .await
    }

    /// The background lane's budget bounds machine-initiated sweeps: RSS and
    /// the convergence lanes that consent to corpus reuse. An operator's
    /// Auto-mode search — queue-best-release, the UI search buttons, and the
    /// acquisition-search job they start — admits through the interactive lane
    /// so it is never queued behind that sweep.
    fn search_limit_for_mode(
        &self,
        mode: SearchMode,
        is_rss_request: bool,
        learning_context: Option<&IndexerSearchLearningContext>,
    ) -> Arc<Semaphore> {
        let background_pass = mode == SearchMode::Auto
            && (is_rss_request
                || learning_context.is_some_and(|context| context.candidate_reuse_allowed));
        if background_pass {
            self.background_search_limit.clone()
        } else {
            self.interactive_search_limit.clone()
        }
    }

    fn client_from_config(
        config: &IndexerConfig,
        plugin_provider: &Arc<dyn IndexerPluginProvider>,
        proxy_configs_by_id: &HashMap<String, Option<scryer_domain::ProxyConfig>>,
    ) -> AppResult<(Arc<dyn IndexerClient>, Option<String>, std::time::Duration)> {
        let provider = config.provider_type.trim().to_ascii_lowercase();
        let (proxy_config, proxy_cache_key) = if let Some(proxy_config_id) =
            config.proxy_config_id.as_deref()
        {
            let proxy_config = proxy_configs_by_id
                .get(proxy_config_id)
                .cloned()
                .flatten()
                .ok_or_else(|| AppError::Validation("Proxy configuration was not found.".into()))?;
            if !proxy_config.is_enabled {
                return Err(AppError::Validation(
                    "Proxy is disabled for this indexer.".into(),
                ));
            }
            let proxy_cache_key = format!("{}:{}", proxy_config.id, proxy_config.updated_at);
            (Some(proxy_config), Some(proxy_cache_key))
        } else {
            (None, None)
        };

        if let Some(client) =
            plugin_provider.client_for_provider_with_proxy(config, proxy_config.as_ref())
        {
            let search_timeout = Self::effective_indexer_search_timeout(proxy_config.as_ref());
            return Ok((client, proxy_cache_key, search_timeout));
        }

        Err(AppError::Validation(format!(
            "unsupported indexer provider: '{provider}'"
        )))
    }

    async fn record_indexer_last_error(
        indexer_configs: &Arc<dyn IndexerConfigRepository>,
        indexer_id: &str,
        indexer_name: &str,
        message: Option<String>,
    ) {
        if let Err(error) = indexer_configs.record_last_error(indexer_id, message).await {
            warn!(
                indexer = indexer_name,
                error = %error,
                "failed to update indexer last_error_at"
            );
        }
    }

    async fn clear_indexer_last_error(
        indexer_configs: &Arc<dyn IndexerConfigRepository>,
        indexer_id: &str,
        indexer_name: &str,
    ) {
        if let Err(error) = indexer_configs.clear_last_error(indexer_id).await {
            warn!(
                indexer = indexer_name,
                error = %error,
                "failed to clear indexer last_error_at"
            );
        }
    }

    async fn record_indexer_system_backoff(
        indexer_configs: &Arc<dyn IndexerConfigRepository>,
        indexer_id: &str,
        indexer_name: &str,
        backoff: IndexerSystemBackoff,
    ) {
        if let Err(error) = indexer_configs
            .set_system_backoff(indexer_id, backoff)
            .await
        {
            warn!(
                indexer = indexer_name,
                error = %error,
                "failed to persist indexer system backoff"
            );
        }
    }

    async fn clear_indexer_system_backoff(
        indexer_configs: &Arc<dyn IndexerConfigRepository>,
        indexer_id: &str,
        indexer_name: &str,
    ) {
        if let Err(error) = indexer_configs.clear_system_backoff(indexer_id).await {
            warn!(
                indexer = indexer_name,
                error = %error,
                "failed to clear indexer system backoff"
            );
        }
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

    fn auto_mode_enabled(config: &IndexerConfig, is_rss_request: bool) -> bool {
        if !config.enable_auto_search {
            return false;
        }
        if config
            .last_error_message
            .as_deref()
            .is_some_and(|message| message.starts_with(INDEXER_CAPS_REFRESH_ERROR_PREFIX))
        {
            return false;
        }

        let Some(raw) = config.managed_metadata_json.as_deref() else {
            return true;
        };
        let Ok(metadata) = serde_json::from_str::<ManagedIndexerMetadata>(raw) else {
            return true;
        };

        if is_rss_request {
            metadata.enable_rss.unwrap_or(true)
        } else {
            metadata.enable_automatic_search.unwrap_or(true)
        }
    }

    fn resolve_search_capabilities(
        config: &IndexerConfig,
        static_caps: &IndexerProviderCapabilities,
        query_facet: &str,
        id_facet: &str,
    ) -> ResolvedSearchCapabilities {
        let transport_kind = config.nab_transport_kind();
        if transport_kind.is_none() {
            return ResolvedSearchCapabilities {
                caps: static_caps.clone(),
                id_dispatch_mode: IdDispatchMode::LegacyAggregate,
                text_dispatch_mode: text_dispatch_mode_for_static(static_caps, query_facet),
                query_only_reason: None,
                transport_kind: None,
                caps_source: "static",
            };
        }

        let snapshot = stored_caps_snapshot(config);
        match transport_kind {
            Some(NabTransportKind::DirectNab) if snapshot.is_none() => {
                return ResolvedSearchCapabilities {
                    caps: static_caps.clone(),
                    id_dispatch_mode: IdDispatchMode::LegacyAggregate,
                    text_dispatch_mode: text_dispatch_mode_for_static(static_caps, query_facet),
                    query_only_reason: None,
                    transport_kind,
                    caps_source: "legacy_static",
                };
            }
            Some(NabTransportKind::ProwlarrNabProxy) if snapshot.is_none() => {
                return ResolvedSearchCapabilities {
                    caps: IndexerProviderCapabilities {
                        supported_ids: HashMap::new(),
                        search_inputs: static_caps.search_inputs.clone(),
                        supported_external_ids: Vec::new(),
                        query_param: static_caps.query_param.clone(),
                        ..static_caps.clone()
                    },
                    id_dispatch_mode: IdDispatchMode::QueryOnly,
                    text_dispatch_mode: if static_caps.query_param.is_some() {
                        TextDispatchMode::GenericOnly
                    } else {
                        TextDispatchMode::None
                    },
                    query_only_reason: Some("caps snapshot unavailable"),
                    transport_kind,
                    caps_source: "query_only_fallback",
                };
            }
            _ => {}
        }

        let Some(snapshot) = snapshot.as_ref() else {
            return ResolvedSearchCapabilities {
                caps: static_caps.clone(),
                id_dispatch_mode: IdDispatchMode::LegacyAggregate,
                text_dispatch_mode: text_dispatch_mode_for_static(static_caps, query_facet),
                query_only_reason: None,
                transport_kind,
                caps_source: "static",
            };
        };

        let mut caps = static_caps.clone();
        caps.supported_ids = supported_ids_from_caps_snapshot(snapshot);
        let text_dispatch_mode = caps_snapshot_text_dispatch_mode(snapshot, query_facet);
        caps.query_param = text_dispatch_mode.can_dispatch().then_some("q".to_string());
        caps.supported_query_facets = if matches!(text_dispatch_mode, TextDispatchMode::FacetScoped)
        {
            vec![query_facet.to_string()]
        } else {
            Vec::new()
        };
        caps.search_inputs = caps_search_inputs(snapshot, query_facet);
        caps.supported_external_ids = supported_external_ids_from_caps_snapshot(snapshot);
        caps.season_param = node_supports_param(snapshot.tv_search.as_ref(), "season")
            .then_some("season".to_string());
        caps.episode_param =
            node_supports_param(snapshot.tv_search.as_ref(), "ep").then_some("ep".to_string());
        if matches!(transport_kind, Some(NabTransportKind::DirectNab)) {
            preserve_direct_nab_native_capabilities(&mut caps, static_caps, id_facet);
        }

        let id_dispatch_mode = if caps.has_facet(id_facet) {
            IdDispatchMode::Aggregate
        } else {
            IdDispatchMode::QueryOnly
        };
        let query_only_reason = (id_dispatch_mode == IdDispatchMode::QueryOnly)
            .then_some("no actionable IDs in caps snapshot");

        ResolvedSearchCapabilities {
            caps,
            id_dispatch_mode,
            text_dispatch_mode,
            query_only_reason,
            transport_kind,
            caps_source: "snapshot",
        }
    }

    fn is_prowlarr_nab_proxy(config: &IndexerConfig) -> bool {
        config.is_prowlarr_nab_proxy()
    }

    fn default_newznab_categories_for_facet(facet: &str) -> Option<Vec<String>> {
        let categories = match facet {
            "movie" => &["2000"][..],
            "series" => &["5000"][..],
            "anime" => &["5070"][..],
            _ => &[][..],
        };
        (!categories.is_empty()).then(|| {
            categories
                .iter()
                .map(|value| (*value).to_string())
                .collect()
        })
    }

    fn split_rss_category_requests(categories: Option<Vec<String>>) -> Vec<Option<Vec<String>>> {
        let mut normalized: Vec<String> = categories
            .unwrap_or_default()
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();
        normalized.sort();
        normalized.dedup();

        if normalized.is_empty() {
            vec![None]
        } else if normalized.len() == 1 {
            vec![Some(normalized)]
        } else {
            normalized
                .into_iter()
                .map(|value| Some(vec![value]))
                .collect()
        }
    }

    fn rss_request_key(categories: Option<&[String]>) -> String {
        let mut normalized: Vec<String> = categories
            .unwrap_or_default()
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();
        normalized.sort();
        normalized.dedup();
        if normalized.is_empty() {
            "rss:*".to_string()
        } else {
            format!("rss:{}", normalized.join(","))
        }
    }

    fn rss_feed_cache_key(indexer_id: &str, categories: Option<&[String]>) -> String {
        format!("{indexer_id}:{}", Self::rss_request_key(categories))
    }

    fn execute_legacy_strategy_tier(
        context: StrategyTierContext,
        strategies: Vec<PreparedSearchStrategy>,
        initial_permit: Option<OwnedSemaphorePermit>,
        page_sink: IndexerSearchPageSink,
    ) -> tokio::task::JoinSet<StrategyExecutionOutcome> {
        let mut set = tokio::task::JoinSet::<StrategyExecutionOutcome>::new();
        let mut initial_permit = initial_permit;

        for (strategy_index, strategy) in strategies.into_iter().enumerate() {
            let context = context.clone();
            let initial_permit = if strategy_index == 0 {
                initial_permit.take()
            } else {
                None
            };
            let strategy_id = strategy.strategy_id.clone();
            let strategy_labels = strategy.labels.clone();
            let strategy_label = strategy.labels.first().cloned().unwrap_or_default();
            let title_guard_mode = strategy.title_guard_mode;
            let strategy = strategy.request;
            let page_sink = page_sink.clone();

            set.spawn(async move {
                let StrategyTierContext {
                    client,
                    search_limit,
                    rate_limiter,
                    indexer_id,
                    search_timeout,
                    rate_limit_seconds,
                    category: _,
                    per_indexer_categories: _,
                    mode,
                    operation,
                    year: _,
                    tagged_aliases: _,
                    cancel_token,
                    deadline_at,
                } = context;
                let permit = match initial_permit {
                    Some(permit) => Ok(permit),
                    None => acquire_search_permit(search_limit, &cancel_token, deadline_at).await,
                };
                let response = match permit {
                    Ok(_permit) => {
                        match within_search_window(
                            rate_limiter.acquire(&indexer_id, rate_limit_seconds),
                            &cancel_token,
                            deadline_at,
                        )
                        .await
                        {
                            Ok(()) => {}
                            Err(SearchWindowError::Cancelled) => {
                                return StrategyExecutionOutcome {
                                    strategy_id: strategy_id.clone(),
                                    labels: strategy_labels.clone(),
                                    label: strategy_label,
                                    title_guard_mode,
                                    request_fired: false,
                                    response: Err(AppError::canceled("indexer strategy canceled")),
                                    page_reservation: None,
                                    elapsed: std::time::Duration::ZERO,
                                    retry_after: None,
                                    rate_limited: false,
                                    timed_out: false,
                                };
                            }
                            Err(SearchWindowError::DeadlineExpired) => {
                                return StrategyExecutionOutcome {
                                    strategy_id: strategy_id.clone(),
                                    labels: strategy_labels.clone(),
                                    label: strategy_label,
                                    title_guard_mode,
                                    request_fired: false,
                                    response: Err(AppError::Repository(
                                        "indexer search timed out before dispatch".into(),
                                    )),
                                    page_reservation: None,
                                    elapsed: std::time::Duration::ZERO,
                                    retry_after: None,
                                    rate_limited: false,
                                    timed_out: false,
                                };
                            }
                        }
                        let start = std::time::Instant::now();
                        let request_cancel_token = cancel_token.child_token();
                        let request_deadline =
                            effective_request_deadline(search_timeout, deadline_at);
                        let mut request_fired = false;
                        let (response, timed_out) = match within_search_window(
                            async {
                                request_fired = true;
                                client
                                    .search(
                                        strategy.query,
                                        strategy.ids,
                                        strategy.category,
                                        strategy.facet,
                                        strategy.id_search_facet,
                                        strategy.newznab_categories,
                                        None,
                                        mode,
                                        operation,
                                        strategy.season,
                                        strategy.episode,
                                        strategy.absolute_episode,
                                        strategy.year,
                                        strategy.tagged_aliases,
                                        None,
                                        request_cancel_token,
                                    )
                                    .await
                            },
                            &cancel_token,
                            Some(request_deadline),
                        )
                        .await
                        {
                            Ok(response) => (response, false),
                            Err(SearchWindowError::Cancelled) => {
                                (Err(AppError::canceled("indexer strategy canceled")), false)
                            }
                            Err(SearchWindowError::DeadlineExpired) => (
                                Err(AppError::Repository("indexer search timed out".into())),
                                true,
                            ),
                        };
                        let rate_limit_signal = response
                            .as_ref()
                            .err()
                            .and_then(rate_limit_signal_from_error);
                        let retry_after = rate_limit_signal
                            .as_ref()
                            .and_then(|signal| signal.retry_after);
                        let rate_limited = rate_limit_signal.is_some();
                        let page_reservation = if response
                            .as_ref()
                            .is_ok_and(|response| !response.results.is_empty())
                        {
                            let reservation = tokio::select! {
                                _ = cancel_token.cancelled() => {
                                    return StrategyExecutionOutcome {
                                        strategy_id,
                                        labels: strategy_labels,
                                        label: strategy_label,
                                        title_guard_mode,
                                        request_fired,
                                        response: Err(AppError::canceled(
                                            "indexer strategy canceled",
                                        )),
                                        page_reservation: None,
                                        elapsed: start.elapsed(),
                                        retry_after: None,
                                        rate_limited: false,
                                        timed_out: false,
                                    };
                                }
                                reservation = page_sink.reserve() => reservation,
                            };
                            let Some(reservation) = reservation else {
                                return StrategyExecutionOutcome {
                                    strategy_id,
                                    labels: strategy_labels,
                                    label: strategy_label,
                                    title_guard_mode,
                                    request_fired,
                                    response: Err(AppError::canceled(
                                        "indexer scoring pipeline closed",
                                    )),
                                    page_reservation: None,
                                    elapsed: start.elapsed(),
                                    retry_after: None,
                                    rate_limited: false,
                                    timed_out: false,
                                };
                            };
                            Some(reservation)
                        } else {
                            None
                        };

                        return StrategyExecutionOutcome {
                            strategy_id: strategy_id.clone(),
                            labels: strategy_labels.clone(),
                            label: strategy_label,
                            title_guard_mode,
                            request_fired,
                            response,
                            page_reservation,
                            elapsed: start.elapsed(),
                            retry_after,
                            rate_limited,
                            timed_out,
                        };
                    }
                    Err(SearchPermitError::Cancelled) => {
                        Err(AppError::canceled("indexer strategy canceled"))
                    }
                    Err(SearchPermitError::DeadlineExpired) => Err(AppError::Repository(
                        "indexer search timed out before dispatch".into(),
                    )),
                    Err(SearchPermitError::Closed(error)) => Err(AppError::Repository(format!(
                        "indexer search limiter closed: {error}"
                    ))),
                };

                StrategyExecutionOutcome {
                    strategy_id,
                    labels: strategy_labels,
                    label: strategy_label,
                    title_guard_mode,
                    request_fired: false,
                    response,
                    page_reservation: None,
                    elapsed: std::time::Duration::ZERO,
                    retry_after: None,
                    rate_limited: false,
                    timed_out: false,
                }
            });
        }

        set
    }

    fn execute_strategy_tier(
        context: StrategyTierContext,
        strategies: Vec<PreparedSearchStrategy>,
        initial_permit: Option<OwnedSemaphorePermit>,
        page_sink: IndexerSearchPageSink,
    ) -> StrategyTierOutcomes {
        if context
            .client
            .search_plan_capability()
            .is_some_and(|capability| capability.version == 1)
        {
            Self::execute_plan_strategy_tier(context, strategies, initial_permit, page_sink)
        } else {
            StrategyTierOutcomes::Legacy(Self::execute_legacy_strategy_tier(
                context,
                strategies,
                initial_permit,
                page_sink,
            ))
        }
    }

    fn execute_plan_strategy_tier(
        context: StrategyTierContext,
        strategies: Vec<PreparedSearchStrategy>,
        initial_permit: Option<OwnedSemaphorePermit>,
        page_sink: IndexerSearchPageSink,
    ) -> StrategyTierOutcomes {
        let (outcome_tx, outcome_rx) = tokio::sync::mpsc::channel(16);
        let plan_cancel_token = context.cancel_token.child_token();
        let cancel_on_drop = plan_cancel_token.clone();
        let controller = tokio::spawn(async move {
            let plan_id = uuid::Uuid::new_v4().to_string();
            let mut expected = strategies
                .iter()
                .map(|strategy| (strategy.strategy_id.clone(), strategy.clone()))
                .collect::<HashMap<_, _>>();
            let requests = strategies
                .iter()
                .map(|strategy| strategy.request.clone())
                .collect();

            let permit = match initial_permit {
                Some(permit) => Ok(permit),
                None => {
                    acquire_search_permit(
                        context.search_limit.clone(),
                        &plan_cancel_token,
                        context.deadline_at,
                    )
                    .await
                }
            };
            let permit = match permit {
                Ok(permit) => permit,
                Err(error) => {
                    let timed_out = matches!(error, SearchPermitError::DeadlineExpired);
                    let message = match error {
                        SearchPermitError::Cancelled => {
                            "indexer strategy plan canceled".to_string()
                        }
                        SearchPermitError::DeadlineExpired => {
                            "indexer search timed out before plan dispatch".to_string()
                        }
                        SearchPermitError::Closed(error) => {
                            format!("indexer search limiter closed: {error}")
                        }
                    };
                    for strategy in expected.into_values() {
                        if outcome_tx
                            .send(StrategyExecutionOutcome {
                                strategy_id: strategy.strategy_id,
                                label: strategy.labels.first().cloned().unwrap_or_default(),
                                labels: strategy.labels,
                                title_guard_mode: strategy.title_guard_mode,
                                response: Err(AppError::Repository(message.clone())),
                                page_reservation: None,
                                request_fired: false,
                                elapsed: std::time::Duration::ZERO,
                                retry_after: None,
                                rate_limited: false,
                                timed_out,
                            })
                            .await
                            .is_err()
                        {
                            plan_cancel_token.cancel();
                            break;
                        }
                    }
                    return;
                }
            };

            let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(16);
            let event_sink = IndexerSearchStrategyEventSink::new(event_tx);
            let request = IndexerSearchPlanRequest {
                plan_id: plan_id.clone(),
                strategies: requests,
            };
            let client = context.client.clone();
            let operation = context.operation;
            let mode = context.mode;
            let request_cancel = plan_cancel_token.child_token();
            let request_deadline =
                effective_request_deadline(context.search_timeout, context.deadline_at);
            let request_window_cancel = plan_cancel_token.clone();
            let started_at = std::time::Instant::now();
            let invocation = tokio::spawn(async move {
                let _permit = permit;
                match within_search_window(
                    client.search_plan(request, mode, operation, request_cancel, event_sink),
                    &request_window_cancel,
                    Some(request_deadline),
                )
                .await
                {
                    Ok(result) => (result, false),
                    Err(SearchWindowError::Cancelled) => (
                        Err(AppError::canceled("indexer strategy plan canceled")),
                        false,
                    ),
                    Err(SearchWindowError::DeadlineExpired) => (
                        Err(AppError::Repository("indexer search plan timed out".into())),
                        true,
                    ),
                }
            });

            let mut invocation = Some(invocation);
            let mut invocation_result = None;
            let mut emitted = HashSet::new();
            let mut protocol_error = None::<String>;
            while invocation.is_some() || !event_rx.is_closed() || !event_rx.is_empty() {
                tokio::select! {
                    event = event_rx.recv(), if !event_rx.is_closed() || !event_rx.is_empty() => {
                        let Some(IndexerSearchStrategyEvent { strategy_id, response }) = event else {
                            continue;
                        };
                        let Some(strategy) = expected.get(&strategy_id).cloned() else {
                            protocol_error = Some(format!("indexer strategy plan emitted unknown strategy {strategy_id}"));
                            continue;
                        };
                        if !emitted.insert(strategy_id.clone()) {
                            protocol_error = Some(format!("indexer strategy plan emitted duplicate strategy {strategy_id}"));
                            continue;
                        }
                        let rate_limit_signal = response
                            .as_ref()
                            .err()
                            .and_then(rate_limit_signal_from_error);
                        let retry_after = rate_limit_signal
                            .as_ref()
                            .and_then(|signal| signal.retry_after);
                        let page_reservation = if response
                            .as_ref()
                            .is_ok_and(|response| !response.results.is_empty())
                        {
                            tokio::select! {
                                _ = plan_cancel_token.cancelled() => None,
                                reservation = page_sink.reserve() => reservation,
                            }
                        } else {
                            None
                        };
                        if outcome_tx
                            .send(StrategyExecutionOutcome {
                                strategy_id,
                                label: strategy.labels.first().cloned().unwrap_or_default(),
                                labels: strategy.labels,
                                title_guard_mode: strategy.title_guard_mode,
                                request_fired: true,
                                response,
                                page_reservation,
                                elapsed: started_at.elapsed(),
                                retry_after,
                                rate_limited: rate_limit_signal.is_some(),
                                timed_out: false,
                            })
                            .await
                            .is_err()
                        {
                            plan_cancel_token.cancel();
                            break;
                        }
                    }
                    joined = async { invocation.as_mut().expect("guarded invocation").await }, if invocation.is_some() => {
                        invocation_result = Some(match joined {
                            Ok(result) => result,
                            Err(error) => (
                                Err(AppError::Repository(format!("indexer strategy plan task failed: {error}"))),
                                false,
                            ),
                        });
                        invocation = None;
                    }
                }
                if invocation.is_none() && event_rx.is_empty() {
                    break;
                }
            }

            let (summary, timed_out) = invocation_result.unwrap_or_else(|| {
                (
                    Err(AppError::Repository(
                        "indexer strategy plan ended without a summary".to_string(),
                    )),
                    false,
                )
            });
            let invocation_error = match summary {
                Ok(summary) => {
                    if summary.plan_id != plan_id {
                        protocol_error =
                            Some("indexer strategy plan summary ID mismatch".to_string());
                    }
                    let summary_id_count = summary.emitted_strategy_ids.len();
                    let summary_ids = summary
                        .emitted_strategy_ids
                        .into_iter()
                        .collect::<HashSet<_>>();
                    if summary_ids.len() != summary_id_count || summary_ids != emitted {
                        protocol_error = Some(
                            "indexer strategy plan summary did not match emitted events"
                                .to_string(),
                        );
                    } else if emitted.len() != expected.len()
                        || expected
                            .keys()
                            .any(|strategy_id| !emitted.contains(strategy_id))
                    {
                        protocol_error =
                            Some("indexer strategy plan omitted a submitted strategy".to_string());
                    }
                    None
                }
                Err(error) => Some(error.to_string()),
            };

            if let Some(error) = protocol_error.as_deref() {
                if outcome_tx
                    .send(StrategyExecutionOutcome {
                        strategy_id: format!("protocol:{plan_id}"),
                        label: "protocol".to_string(),
                        labels: vec!["protocol".to_string()],
                        title_guard_mode: TitleGuardMode::SkipTitleMatch,
                        response: Err(AppError::Repository(error.to_string())),
                        page_reservation: None,
                        request_fired: true,
                        elapsed: started_at.elapsed(),
                        retry_after: None,
                        rate_limited: false,
                        timed_out,
                    })
                    .await
                    .is_err()
                {
                    plan_cancel_token.cancel();
                    return;
                }
                for strategy in expected.values() {
                    if outcome_tx
                        .send(StrategyExecutionOutcome {
                            strategy_id: strategy.strategy_id.clone(),
                            label: strategy.labels.first().cloned().unwrap_or_default(),
                            labels: strategy.labels.clone(),
                            title_guard_mode: strategy.title_guard_mode,
                            response: Err(AppError::Repository(error.to_string())),
                            page_reservation: None,
                            request_fired: true,
                            elapsed: started_at.elapsed(),
                            retry_after: None,
                            rate_limited: false,
                            timed_out,
                        })
                        .await
                        .is_err()
                    {
                        plan_cancel_token.cancel();
                        return;
                    }
                }
            } else {
                for strategy_id in &emitted {
                    expected.remove(strategy_id);
                }
                let missing_error = invocation_error
                    .as_deref()
                    .unwrap_or("indexer strategy plan omitted a strategy result");
                for strategy in expected.into_values() {
                    if outcome_tx
                        .send(StrategyExecutionOutcome {
                            strategy_id: strategy.strategy_id,
                            label: strategy.labels.first().cloned().unwrap_or_default(),
                            labels: strategy.labels,
                            title_guard_mode: strategy.title_guard_mode,
                            response: Err(AppError::Repository(missing_error.to_string())),
                            page_reservation: None,
                            request_fired: true,
                            elapsed: started_at.elapsed(),
                            retry_after: None,
                            rate_limited: false,
                            timed_out,
                        })
                        .await
                        .is_err()
                    {
                        plan_cancel_token.cancel();
                        return;
                    }
                }
            }
        });

        StrategyTierOutcomes::Plan(StrategyPlanOutcomeStream {
            receiver: outcome_rx,
            controller,
            cancel_token: cancel_on_drop,
        })
    }
}

#[async_trait]
impl IndexerClient for MultiIndexerSearchClient {
    async fn reset_indexer_backoff(&self, indexer_id: &str) {
        if self.backoff_tracker.clear(indexer_id).await {
            tracing::info!(
                indexer_id = %indexer_id,
                "cleared in-memory indexer backoff after a validated config save"
            );
        }
    }

    async fn search_stream(
        &self,
        query: String,
        ids: HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        operation: IndexerErrorOperation,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        year: Option<i32>,
        tagged_aliases: Vec<scryer_domain::TaggedAlias>,
        learning_context: Option<IndexerSearchLearningContext>,
        cancel_token: CancellationToken,
        page_sink: IndexerSearchPageSink,
    ) -> AppResult<IndexerSearchResponse> {
        self.search_queries_stream(
            vec![query],
            ids,
            category,
            facet,
            id_search_facet,
            newznab_categories,
            indexer_routing,
            mode,
            operation,
            season,
            episode,
            absolute_episode,
            year,
            tagged_aliases,
            learning_context,
            cancel_token,
            page_sink,
        )
        .await
    }

    async fn search_queries_stream(
        &self,
        queries: Vec<String>,
        ids: HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        operation: IndexerErrorOperation,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        year: Option<i32>,
        tagged_aliases: Vec<scryer_domain::TaggedAlias>,
        learning_context: Option<IndexerSearchLearningContext>,
        cancel_token: CancellationToken,
        page_sink: IndexerSearchPageSink,
    ) -> AppResult<IndexerSearchResponse> {
        if cancel_token.is_cancelled() {
            return Err(AppError::canceled("indexer search canceled"));
        }
        let query = queries.first().cloned().unwrap_or_default();
        let is_rss_request = Self::is_rss_sync_request(
            &query,
            !ids.is_empty(),
            category
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty()),
            mode,
            season,
            episode,
        );

        let configs = self.indexer_configs.list(None).await.unwrap_or_else(|err| {
            warn!(error = %err, "failed to load indexer configs");
            vec![]
        });
        let now = chrono::Utc::now();
        let system_backoffs = self
            .indexer_configs
            .list_system_backoffs()
            .await
            .unwrap_or_else(|err| {
                warn!(error = %err, "failed to load persisted indexer system backoffs");
                HashMap::new()
            });

        // Filter by is_enabled, search mode flag, disabled_until (config), and backoff state
        let mut enabled: Vec<(&IndexerConfig, bool)> = Vec::new();
        for c in &configs {
            if !c.is_enabled {
                log_indexer_skip(mode, c.name.as_str(), "disabled", None);
                continue;
            }
            // Check persistent disabled_until from config
            if let Some(until) = c.disabled_until
                && until > now
            {
                log_indexer_skip(
                    mode,
                    c.name.as_str(),
                    "temporarily disabled (config)",
                    Some(until),
                );
                continue;
            }
            let persisted_system_backoff = system_backoffs.get(&c.id).cloned();
            if let Some(backoff) = persisted_system_backoff.as_ref() {
                self.backoff_tracker.seed_persisted(&c.id, backoff).await;
            }
            let had_persisted_system_backoff = persisted_system_backoff.is_some();
            // Every mode respects operational backoff, interactive included —
            // Sonarr parity (`IndexerFactory.InteractiveSearchEnabled()` filters
            // blocked indexers and logs "Temporarily ignoring indexer … due to
            // recent failures"). Querying a backed-off indexer from the UI would
            // extend the very ban the backoff is protecting against; the skip is
            // logged at info for interactive so the reason stays visible.
            if let Some(backoff) = persisted_system_backoff.as_ref()
                && backoff.disabled_until > now
            {
                log_indexer_skip(
                    mode,
                    c.name.as_str(),
                    "temporarily disabled (system backoff)",
                    Some(backoff.disabled_until),
                );
                continue;
            }
            // Check in-memory backoff escalation
            if let Some(until) = self.backoff_tracker.is_disabled(&c.id).await {
                log_indexer_skip(
                    mode,
                    c.name.as_str(),
                    "temporarily disabled (backoff)",
                    Some(until),
                );
                continue;
            }
            let mode_ok = match mode {
                SearchMode::Interactive => c.enable_interactive_search,
                SearchMode::Auto => Self::auto_mode_enabled(c, is_rss_request),
            };
            if mode_ok {
                enabled.push((c, had_persisted_system_backoff));
            } else {
                log_indexer_skip(mode, c.name.as_str(), "disabled for search mode", None);
            }
        }

        if enabled.is_empty() {
            info!(mode = ?mode, "no enabled indexer configs found");
            return Ok(IndexerSearchResponse {
                results: vec![],

                indexer_outcomes: Vec::new(),
                completion: IndexerSearchCompletion::Complete,
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            });
        }

        debug!(
            mode = ?mode,
            count = enabled.len(),
            indexers = ?enabled.iter().map(|(c, _)| c.name.as_str()).collect::<Vec<_>>(),
            "dispatching search to indexers"
        );

        // A title-less operator search (the Indexers page's raw kind) carries a
        // query and no facet. Capability resolution still needs one to decide
        // whether the endpoint answers freetext at all, so it borrows the movie
        // facet, but the strategy omits the facet on the wire so plugins issue
        // a plain `q=` text query rather than a facet-scoped nab function.
        let facet_omitted = !is_rss_request
            && facet
                .as_deref()
                .map(str::trim)
                .is_none_or(|value| value.is_empty());
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
            None if !query.trim().is_empty() => "movie".to_string(),
            None => {
                return Err(AppError::Validation("search facet is required".to_string()));
            }
        };
        let id_search_facet = match id_search_facet
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value @ ("movie" | "series" | "anime")) => value.to_string(),
            Some(other) => {
                return Err(AppError::Validation(format!(
                    "unsupported ID search facet: {other}"
                )));
            }
            None => facet.clone(),
        };

        tracing::debug!(
            %facet,
            %id_search_facet,
            ?category,
            ?ids,
            ?season,
            ?episode,
            ?absolute_episode,
            %query,
            "search context"
        );
        let available_ids = ids;

        let scheduler_now = chrono::Utc::now();
        let scheduler_intent = Self::scheduler_intent(mode, is_rss_request);
        let scheduler_snapshot = if is_rss_request {
            self.upstream_scheduler
                .snapshot(scryer_application::SchedulerSnapshotFilter::default())
                .await
                .ok()
        } else {
            None
        };
        // Resolve each distinct proxy config once per search pass; both the
        // scheduler deadline calculation and client construction read from
        // this map. A missing entry value means the config row is gone.
        let mut proxy_configs_by_id: HashMap<String, Option<scryer_domain::ProxyConfig>> =
            HashMap::new();
        for (config, _) in &enabled {
            let Some(proxy_config_id) = config.proxy_config_id.as_deref() else {
                continue;
            };
            if proxy_configs_by_id.contains_key(proxy_config_id) {
                continue;
            }
            let fetched = self.proxy_configs.get_by_id(proxy_config_id).await?;
            proxy_configs_by_id.insert(proxy_config_id.to_string(), fetched);
        }
        let mut scheduler_candidates = Vec::new();
        let mut scheduler_eligible = Vec::new();
        for (config, had_persisted_system_backoff) in &enabled {
            let routing_entry = indexer_routing
                .as_ref()
                .and_then(|plan| plan.entries.get(&config.id));
            match indexer_search_eligibility(
                indexer_routing.as_ref(),
                page_sink.indexer_restriction(),
                &config.id,
            ) {
                IndexerSearchEligibility::Eligible => {}
                IndexerSearchEligibility::ExcludedBySearchRestriction => {
                    debug!(
                        indexer = config.name.as_str(),
                        "skipping indexer: excluded by per-search restriction"
                    );
                    continue;
                }
                IndexerSearchEligibility::DisabledForScope => {
                    info!(
                        indexer = config.name.as_str(),
                        "skipping indexer: disabled for scope via routing config"
                    );
                    continue;
                }
            }

            let static_caps = self
                .plugin_provider
                .capabilities_for_provider(&config.provider_type);
            let resolved_caps =
                Self::resolve_search_capabilities(config, &static_caps, &facet, &id_search_facet);
            let caps = resolved_caps.caps.clone();

            if is_rss_request && !caps.rss {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: does not support RSS sync"
                );
                continue;
            }

            let eligible_ids =
                filter_ids_for_types(&available_ids, caps.id_types_for_facet(&id_search_facet));
            let can_dispatch_id = !eligible_ids.is_empty()
                && caps.has_facet(&id_search_facet)
                && !matches!(resolved_caps.id_dispatch_mode, IdDispatchMode::QueryOnly);
            let can_dispatch_text =
                !query.trim().is_empty() && resolved_caps.text_dispatch_mode.can_dispatch();
            if !is_rss_request && !can_dispatch_id && !can_dispatch_text {
                info!(
                    indexer = config.name.as_str(),
                    facet, "skipping indexer: no supported IDs for facet and no freetext"
                );
                continue;
            }

            let (host_key, destination_key) = Self::scheduler_keys_for_indexer(config);
            Self::register_managed_proxy_host_profile(config, &host_key);
            let account_quota_key = Some(config.id.clone().into());
            let scheduler_learning_context = if mode == SearchMode::Auto && !is_rss_request {
                if let Some(learning_context) = learning_context.as_ref() {
                    match self
                        .search_learning
                        .list_for_title(
                            &config.id,
                            &learning_context.title_id,
                            &learning_context.facet,
                        )
                        .await
                    {
                        Ok(records) => {
                            let historically_useful = records.iter().any(|record| {
                                record.usable_successes > 0
                                    && !learned_suppression_is_active(record, now)
                            });
                            Some(SearchLearningContext {
                                indexer_id: config.id.clone(),
                                facet: learning_context.facet.clone(),
                                strategy: "provider_history".to_string(),
                                suppressed: false,
                                historically_useful,
                            })
                        }
                        Err(error) => {
                            warn!(
                                indexer = config.name.as_str(),
                                title_id = learning_context.title_id.as_str(),
                                facet = learning_context.facet.as_str(),
                                error = %error,
                                "failed to load scheduler learning hint"
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };
            // Candidate value: the
            // convergence cursor's per-scope lane hint takes precedence — hot
            // scopes carry the high value that keeps admitting under quota
            // pressure, cold scopes the low value the quota gate sheds first. When no
            // hint is supplied (interactive/RSS, or a background scope with no
            // hint), fall back to the historically-useful boost, then neutral.
            let expected_value = if let Some(score) = learning_context
                .as_ref()
                .and_then(|context| context.background_value)
            {
                ExpectedValueHint { score }
            } else if scheduler_learning_context
                .as_ref()
                .is_some_and(|context| context.historically_useful)
            {
                ExpectedValueHint { score: 1.0 }
            } else {
                ExpectedValueHint::default()
            };
            // Use per-indexer categories from routing if available. Prowlarr
            // proxy children may fall back to per-facet defaults when no
            // routed categories exist yet; direct *nab indexers stay broad.
            let per_indexer_categories = routing_entry
                .map(|entry| {
                    if entry.categories.is_empty() {
                        if Self::is_prowlarr_nab_proxy(config) {
                            newznab_categories
                                .clone()
                                .or_else(|| Self::default_newznab_categories_for_facet(&facet))
                        } else {
                            newznab_categories.clone()
                        }
                    } else {
                        Some(entry.categories.clone())
                    }
                })
                .unwrap_or_else(|| {
                    if Self::is_prowlarr_nab_proxy(config) {
                        newznab_categories
                            .clone()
                            .or_else(|| Self::default_newznab_categories_for_facet(&facet))
                    } else {
                        newznab_categories.clone()
                    }
                });
            let category_requests = if is_rss_request {
                Self::split_rss_category_requests(per_indexer_categories)
            } else {
                vec![per_indexer_categories]
            };
            for category_request in category_requests {
                let scheduler_candidate_id = SchedulerCandidateId::new();
                let rss_request_key =
                    is_rss_request.then(|| Self::rss_request_key(category_request.as_deref()));
                let rss_activity = if is_rss_request {
                    Self::scheduler_rss_activity(
                        scheduler_snapshot.as_ref(),
                        &host_key,
                        &destination_key,
                        account_quota_key.as_ref(),
                        rss_request_key.as_deref(),
                    )
                } else {
                    SchedulerRssActivity::default()
                };
                scheduler_candidates.push(SchedulerCandidate {
                    candidate_id: scheduler_candidate_id.clone(),
                    plugin_config_id: Some(config.id.clone()),
                    plugin_kind: SchedulerPluginKind::Indexer,
                    operation: if is_rss_request {
                        SchedulerOperation::Rss
                    } else {
                        SchedulerOperation::Search
                    },
                    intent: scheduler_intent,
                    host_key: host_key.clone(),
                    destination_key: destination_key.clone(),
                    account_quota_key: account_quota_key.clone(),
                    rss_request_key: rss_request_key.clone(),
                    estimated_cost: EstimatedCost::ONE_API_CALL,
                    expected_value,
                    learning_context: scheduler_learning_context.clone(),
                    deadline_at: None,
                    freshness: is_rss_request.then(|| {
                        Self::rss_freshness_context(config, scheduler_now, rss_activity.clone())
                    }),
                    cancel_token: cancel_token.child_token(),
                });
                scheduler_eligible.push(SchedulerEligibleIndexer {
                    config,
                    had_persisted_system_backoff: *had_persisted_system_backoff,
                    candidate_id: scheduler_candidate_id,
                    category_request,
                    rss_request_key,
                });
            }
        }

        if scheduler_candidates.is_empty() {
            info!(mode = ?mode, "no scheduler-eligible indexer configs found");
            return Ok(IndexerSearchResponse {
                results: vec![],

                indexer_outcomes: Vec::new(),
                completion: IndexerSearchCompletion::Complete,
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            });
        }

        let scheduler_decision = self
            .upstream_scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: format!("indexer-search:{mode:?}:{}", uuid::Uuid::new_v4()),
                now: scheduler_now,
                candidates: scheduler_candidates,
            })
            .await?;
        let mut scheduler_dispatches = scheduler_eligible
            .into_iter()
            .map(|dispatch| (dispatch.candidate_id.as_str().to_string(), dispatch))
            .collect::<HashMap<_, _>>();

        // Spawn parallel searches across admitted indexers, applying per-indexer routing.
        // Each indexer may still execute multiple strategies internally, but for
        // TV/series searches we run ID searches first and only fall back to a
        // broad freetext query if that indexer returned no releases.
        //
        // Per-indexer outcome for convergence coverage. Deferred/skipped
        // indexers (never queried) are recorded below; fired/errored are recorded as
        // their tasks complete in the join loop.
        let mut indexer_outcomes: Vec<IndexerQueryOutcome> = Vec::new();
        let mut set = tokio::task::JoinSet::<(
            String,
            String,
            Option<SchedulerLease>,
            AppResult<IndexerSearchResponse>,
            bool,
        )>::new();
        let search_limit =
            self.search_limit_for_mode(mode, is_rss_request, learning_context.as_ref());
        for scheduler_admission in scheduler_decision.decisions {
            let candidate_id = Self::scheduler_admission_candidate_id(&scheduler_admission);
            let Some(dispatch) = scheduler_dispatches.remove(candidate_id.as_str()) else {
                warn!(
                    candidate_id = candidate_id.as_str(),
                    "scheduler returned a decision for an unknown indexer search candidate"
                );
                continue;
            };
            let config = dispatch.config;
            let had_persisted_system_backoff = dispatch.had_persisted_system_backoff;
            let request_categories = dispatch.category_request.clone();
            let rss_request_key = dispatch.rss_request_key.clone();
            let (scheduler_lease, scheduler_blocked_outcome, scheduler_retry_after) =
                match scheduler_admission {
                    SchedulerAdmission::Admit { reason, lease, .. } => {
                        debug!(
                            indexer = config.name.as_str(),
                            scheduler_reason = ?reason,
                            "scheduler admitted indexer search candidate"
                        );
                        (Some(lease), None, None)
                    }
                    SchedulerAdmission::Defer {
                        reason,
                        retry_after,
                        ..
                    } => {
                        info!(
                            indexer = config.name.as_str(),
                            scheduler_reason = ?reason,
                            retry_after_secs = retry_after.map(|delay| delay.as_secs()),
                            "scheduler deferred indexer search candidate"
                        );
                        (
                            None,
                            Some(IndexerSearchOutcome::Deferred { retry_after }),
                            retry_after,
                        )
                    }
                    SchedulerAdmission::Skip {
                        reason,
                        retry_after,
                        ..
                    } => {
                        info!(
                            indexer = config.name.as_str(),
                            scheduler_reason = ?reason,
                            retry_after_secs = retry_after.map(|delay| delay.as_secs()),
                            "scheduler skipped indexer search candidate"
                        );
                        (
                            None,
                            Some(IndexerSearchOutcome::Skipped { retry_after }),
                            retry_after,
                        )
                    }
                };

            let static_caps = self
                .plugin_provider
                .capabilities_for_provider(&config.provider_type);
            let resolved_caps =
                Self::resolve_search_capabilities(config, &static_caps, &facet, &id_search_facet);
            let caps = resolved_caps.caps.clone();
            debug!(
                indexer = config.name.as_str(),
                transport = resolved_caps
                    .transport_kind
                    .map(|kind| kind.as_str())
                    .unwrap_or("other"),
                caps_source = resolved_caps.caps_source,
                "resolved effective indexer search capabilities"
            );

            // RSS-only check: skip non-RSS indexers for RSS sync requests
            if is_rss_request && !caps.rss {
                info!(
                    indexer = config.name.as_str(),
                    "skipping indexer: does not support RSS sync"
                );
                continue;
            }

            let eligible_ids =
                filter_ids_for_types(&available_ids, caps.id_types_for_facet(&id_search_facet));
            let can_dispatch_id = !eligible_ids.is_empty()
                && caps.has_facet(&id_search_facet)
                && !matches!(resolved_caps.id_dispatch_mode, IdDispatchMode::QueryOnly);
            let can_dispatch_text =
                !query.trim().is_empty() && resolved_caps.text_dispatch_mode.can_dispatch();
            if !is_rss_request && !can_dispatch_id && !can_dispatch_text {
                info!(
                    indexer = config.name.as_str(),
                    facet, "skipping indexer: no supported IDs for facet and no freetext"
                );
                continue;
            }

            if matches!(resolved_caps.id_dispatch_mode, IdDispatchMode::QueryOnly)
                && let Some(reason) = resolved_caps.query_only_reason
            {
                info!(
                    indexer = config.name.as_str(),
                    transport = resolved_caps
                        .transport_kind
                        .map(NabTransportKind::as_str)
                        .unwrap_or("unknown"),
                    reason,
                    "NAB indexer running in query-only fallback mode"
                );
            }

            if matches!(
                resolved_caps.id_dispatch_mode,
                IdDispatchMode::Aggregate | IdDispatchMode::QueryOnly
            ) {
                let extra_ids = available_ids
                    .keys()
                    .filter(|id_type| !eligible_ids.contains_key(*id_type))
                    .cloned()
                    .collect::<Vec<_>>();
                if !available_ids.is_empty() {
                    debug!(
                        indexer = config.name.as_str(),
                        facet,
                        id_search_facet,
                        eligible_ids = ?eligible_ids.keys().collect::<Vec<_>>(),
                        carried_ids = ?available_ids.keys().collect::<Vec<_>>(),
                        extra_ids = ?extra_ids,
                        "ID strategy capability resolved; carrying full ID envelope when strategy runs"
                    );
                }
            }

            let (client, proxy_cache_key, search_timeout) =
                match Self::client_from_config(config, &self.plugin_provider, &proxy_configs_by_id)
                {
                    Ok(c) => c,
                    Err(err) => {
                        warn!(
                            indexer = config.name.as_str(),
                            error = %err,
                            "skipping indexer: client setup failed"
                        );
                        continue;
                    }
                };
            // Queue admission is cancellation-bound. Each dispatched plugin call
            // receives its own per-indexer timeout below.
            let deadline_at = None;

            // RSS-only indexers: fetch the feed once, cache it, return cached
            // results for all concurrent callers. The feed content is the same
            // regardless of query — the caller matches results downstream.
            let is_rss_only = !caps.supports_any_search() && caps.rss;
            if is_rss_only && let Some(outcome) = scheduler_blocked_outcome {
                indexer_outcomes.push(IndexerQueryOutcome {
                    indexer_id: config.id.clone(),
                    outcome,
                });
                continue;
            }
            if is_rss_only {
                let rss_category_request = request_categories.clone();
                let rss_cache_key = rss_request_key
                    .as_ref()
                    .map(|key| {
                        format!(
                            "{}:{}:{key}",
                            config.id,
                            proxy_cache_key.as_deref().unwrap_or("direct")
                        )
                    })
                    .unwrap_or_else(|| {
                        let base =
                            Self::rss_feed_cache_key(&config.id, rss_category_request.as_deref());
                        format!("{base}:{}", proxy_cache_key.as_deref().unwrap_or("direct"))
                    });
                let cache_entry = {
                    let mut cache = self.rss_feed_cache.lock().await;
                    cache
                        .entry(rss_cache_key)
                        .or_insert_with(|| Arc::new(RssFeedCacheEntry::new()))
                        .clone()
                };
                let initialization_guard = cache_entry
                    .initialization_lock
                    .clone()
                    .try_lock_owned()
                    .ok();
                let initial_permit = if initialization_guard.is_some()
                    && cache_entry.cell.get().is_none()
                {
                    match acquire_search_permit(search_limit.clone(), &cancel_token, deadline_at)
                        .await
                    {
                        Ok(permit) => Some(permit),
                        Err(SearchPermitError::Cancelled) => {
                            set.abort_all();
                            while set.join_next().await.is_some() {}
                            self.rss_feed_cache.lock().await.clear();
                            return Err(AppError::canceled("indexer search canceled while queued"));
                        }
                        Err(SearchPermitError::DeadlineExpired) => {
                            debug!(
                                indexer = config.name.as_str(),
                                "skipping indexer: candidate deadline expired while queued"
                            );
                            indexer_outcomes.push(IndexerQueryOutcome {
                                indexer_id: config.id.clone(),
                                outcome: IndexerSearchOutcome::Skipped { retry_after: None },
                            });
                            continue;
                        }
                        Err(SearchPermitError::Closed(error)) => {
                            set.abort_all();
                            while set.join_next().await.is_some() {}
                            self.rss_feed_cache.lock().await.clear();
                            return Err(AppError::Repository(format!(
                                "indexer search concurrency limiter closed: {error}"
                            )));
                        }
                    }
                } else {
                    None
                };
                let client = client.clone();
                let query = query.clone();
                let category = category.clone();
                let tagged_aliases = tagged_aliases.clone();
                let indexer_id = config.id.clone();
                let indexer_name = config.name.clone();
                let rate_limiter = self.rate_limiter.clone();
                let rate_limit_seconds = config.rate_limit_seconds;
                let stats_tracker = self.stats_tracker.clone();
                let backoff_tracker = self.backoff_tracker.clone();
                let indexer_configs = self.indexer_configs.clone();
                let facet = facet.clone();
                let search_limit = search_limit.clone();
                let task_cancel_token = cancel_token.child_token();
                let scheduler_lease_for_task = scheduler_lease.clone();

                set.spawn(async move {
                        let _initialization_guard = tokio::select! {
                            _ = task_cancel_token.cancelled() => {
                                return (
                                    indexer_id,
                                    indexer_name,
                                    scheduler_lease_for_task.clone(),
                                    Err(AppError::canceled("RSS indexer search canceled")),
                                    false,
                                );
                            }
                            guard = async {
                                match initialization_guard {
                                    Some(guard) => guard,
                                    None => cache_entry
                                        .initialization_lock
                                        .clone()
                                        .lock_owned()
                                        .await,
                                }
                            } => guard,
                        };
                        let cached_results = tokio::select! {
                            _ = task_cancel_token.cancelled() => {
                                return (
                                    indexer_id,
                                    indexer_name,
                                    scheduler_lease_for_task.clone(),
                                    Err(AppError::canceled("RSS indexer search canceled")),
                                    false,
                                );
                            }
                            results = cache_entry.cell.get_or_init(|| async {
                                    let _permit = match initial_permit {
                                        Some(permit) => permit,
                                        None => match acquire_search_permit(
                                            search_limit,
                                            &task_cancel_token,
                                            deadline_at,
                                        )
                                        .await
                                        {
                                            Ok(permit) => permit,
                                            Err(SearchPermitError::Cancelled) => {
                                                return Err(
                                                    "RSS indexer search canceled".to_string()
                                                );
                                            }
                                            Err(SearchPermitError::DeadlineExpired) => {
                                                return Err(
                                                    "RSS indexer search timed out before dispatch"
                                                        .to_string(),
                                                );
                                            }
                                            Err(SearchPermitError::Closed(error)) => {
                                                return Err(format!(
                                                    "RSS indexer search limiter closed: {error}"
                                                ));
                                            }
                                        },
                                    };
                                match within_search_window(
                                    rate_limiter.acquire(&indexer_id, rate_limit_seconds),
                                    &task_cancel_token,
                                    deadline_at,
                                )
                                .await
                                {
                                    Ok(()) => {}
                                    Err(SearchWindowError::Cancelled) => {
                                        return Err("RSS indexer search canceled".to_string());
                                    }
                                    Err(SearchWindowError::DeadlineExpired) => {
                                        return Err(
                                            "RSS indexer search timed out before dispatch".to_string()
                                        );
                                    }
                                }
                                let start = std::time::Instant::now();
                                let request_cancel_token = task_cancel_token.child_token();
                                let request_deadline =
                                    effective_request_deadline(search_timeout, deadline_at);
                                let search_response = within_search_window(
                                    client.search(
                                        query,
                                        HashMap::new(),
                                        category,
                                        Some(facet),
                                        None,
                                        rss_category_request.clone(),
                                        None,
                                        mode,
                                        IndexerErrorOperation::RssSync,
                                        season,
                                        episode,
                                        absolute_episode,
                                        // An RSS poll has no subject, so no year.
                                        None,
                                        tagged_aliases,
                                        None,
                                        request_cancel_token,
                                    ),
                                    &task_cancel_token,
                                    Some(request_deadline),
                                )
                                .await;
                                let elapsed = start.elapsed();

                                match search_response {
                                    Ok(Ok(mut response)) => {
                                        info!(indexer = indexer_name.as_str(), count = response.results.len(), "RSS feed cached");
                                        stats_tracker.record_query(&indexer_id, &indexer_name, true);
                                        let had_in_memory_backoff = backoff_tracker.record_success(&indexer_id).await;
                                        if had_in_memory_backoff || had_persisted_system_backoff {
                                            Self::clear_indexer_system_backoff(
                                                &indexer_configs,
                                                &indexer_id,
                                                &indexer_name,
                                            )
                                            .await;
                                        }
                                        Self::clear_indexer_last_error(
                                            &indexer_configs,
                                            &indexer_id,
                                            &indexer_name,
                                        )
                                        .await;
                                        metrics::counter!("scryer_indexer_queries_total", "indexer" => indexer_name.clone(), "status" => "success", "mode" => "rss_cached").increment(1);
                                        metrics::histogram!("scryer_indexer_query_duration_seconds", "indexer" => indexer_name.clone(), "mode" => "rss_cached").record(elapsed.as_secs_f64());
                                        for result in &mut response.results {
                                            result.indexer_id = Some(indexer_id.clone());
                                        }
                                        Ok(response.results)
                                    }
                                    Ok(Err(err)) => {
                                        if err.is_canceled() {
                                            return Err("RSS indexer search canceled".to_string());
                                        }
                                        warn!(indexer = indexer_name.as_str(), error = %err, "RSS feed fetch failed");
                                        stats_tracker.record_query(&indexer_id, &indexer_name, false);
                                        if rate_limit_signal_from_error(&err).is_none()
                                            && !scryer_application::challenge_solver::is_solver_service_error_message(
                                                &err.to_string(),
                                            )
                                        {
                                            let backoff = backoff_tracker
                                                .record_failure(&indexer_id, None)
                                                .await;
                                            Self::record_indexer_system_backoff(
                                                &indexer_configs,
                                                &indexer_id,
                                                &indexer_name,
                                                backoff,
                                            )
                                            .await;
                                        }
                                        Self::record_indexer_last_error(
                                            &indexer_configs,
                                            &indexer_id,
                                            &indexer_name,
                                            Some(sanitize_indexer_error_message(&err.to_string())),
                                        )
                                        .await;
                                        Err(format!("RSS feed fetch failed: {err}"))
                                    }
                                    Err(SearchWindowError::Cancelled) => {
                                        Err("RSS indexer search canceled".to_string())
                                    }
                                    Err(SearchWindowError::DeadlineExpired) => {
                                        warn!(indexer = indexer_name.as_str(), "RSS feed fetch timed out");
                                        stats_tracker.record_query(&indexer_id, &indexer_name, false);
                                        let backoff =
                                            backoff_tracker.record_failure(&indexer_id, None).await;
                                        Self::record_indexer_system_backoff(
                                            &indexer_configs,
                                            &indexer_id,
                                            &indexer_name,
                                            backoff,
                                        )
                                        .await;
                                        Self::record_indexer_last_error(
                                            &indexer_configs,
                                            &indexer_id,
                                            &indexer_name,
                                            Some("RSS feed fetch timed out".to_string()),
                                        )
                                        .await;
                                        Err("RSS feed fetch timed out".to_string())
                                    }
                                }
                            }) => results.clone(),
                        };
                        if task_cancel_token.is_cancelled() {
                            return (
                                indexer_id,
                                indexer_name,
                                scheduler_lease_for_task.clone(),
                                Err(AppError::canceled("RSS indexer search canceled")),
                                false,
                            );
                        }
                        let should_record_feedback = cache_entry.claim_feedback();
                        let results = match cached_results {
                            Ok(mut results) => {
                                for result in &mut results {
                                    result.indexer_id.get_or_insert_with(|| indexer_id.clone());
                                }
                                results
                            }
                            Err(error) => {
                                return (
                                    indexer_id,
                                    indexer_name,
                                    scheduler_lease_for_task.clone(),
                                    Err(AppError::Repository(error)),
                                    should_record_feedback,
                                );
                            }
                        };

                        let response = IndexerSearchResponse {
                            results,

                            indexer_outcomes: Vec::new(),
                            completion: IndexerSearchCompletion::Complete,
                            api_current: None,
                            api_max: None,
                            grab_current: None,
                            grab_max: None,
                        };
                        (
                            indexer_id,
                            indexer_name,
                            scheduler_lease_for_task.clone(),
                            Ok(response),
                            should_record_feedback,
                        )
                });
                continue;
            }

            let mut strategies = Vec::new();
            for strategy_query in &queries {
                strategies.extend(build_strategies(&StrategyParams {
                    query: strategy_query,
                    query_facet: &facet,
                    id_facet: &id_search_facet,
                    ids: &available_ids,
                    season,
                    episode,
                    absolute_episode,
                    caps: &caps,
                    id_dispatch_mode: resolved_caps.id_dispatch_mode,
                    text_dispatch_mode: resolved_caps.text_dispatch_mode,
                    is_alias_query: false,
                    facet_omitted,
                }));

                if facet == "anime"
                    && let Some(alias_query) =
                        preferred_anime_alias_query(strategy_query, &tagged_aliases)
                {
                    strategies.extend(build_strategies(&StrategyParams {
                        query: &alias_query,
                        query_facet: &facet,
                        id_facet: &id_search_facet,
                        ids: &available_ids,
                        season,
                        episode,
                        absolute_episode,
                        caps: &caps,
                        id_dispatch_mode: resolved_caps.id_dispatch_mode,
                        text_dispatch_mode: resolved_caps.text_dispatch_mode,
                        is_alias_query: true,
                        facet_omitted,
                    }));
                }
            }
            let mut rss_form = None;
            if is_rss_request && strategies.is_empty() {
                let function_unavailable_observed = self
                    .rss_bare_query_indexers
                    .lock()
                    .await
                    .contains(&config.id);
                let form = rss_request_form(
                    &caps,
                    resolved_caps.text_dispatch_mode,
                    &facet,
                    function_unavailable_observed,
                );
                rss_form = Some(form);
                strategies.push(SearchStrategy {
                    request_query: String::new(),
                    request_facet: facet.clone(),
                    ids: HashMap::new(),
                    season: None,
                    episode: None,
                    absolute_episode: None,
                    generic_query_only: false,
                    omit_request_facet: form == RssRequestForm::BareQuery,
                    label: "rss".into(),
                });
            }
            let learned_records = if mode == SearchMode::Auto {
                if let Some(learning_context) = learning_context.as_ref() {
                    match self
                        .search_learning
                        .list_for_title(
                            &config.id,
                            &learning_context.title_id,
                            &learning_context.facet,
                        )
                        .await
                    {
                        Ok(records) => records,
                        Err(error) => {
                            warn!(
                                indexer = config.name.as_str(),
                                title_id = learning_context.title_id.as_str(),
                                facet = learning_context.facet.as_str(),
                                error = %error,
                                "failed to load indexer search learning records"
                            );
                            Vec::new()
                        }
                    }
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            let strategies = suppress_learned_strategies(
                &self.search_learning,
                config.name.as_str(),
                mode,
                strategies,
                &learned_records,
                Utc::now(),
            )
            .await;
            if strategies.is_empty() && !is_rss_request {
                debug!(
                    indexer = config.name.as_str(),
                    title_id = learning_context
                        .as_ref()
                        .map(|context| context.title_id.as_str())
                        .unwrap_or(""),
                    "skipping indexer: all automatic search strategies are learned-suppressed"
                );
                continue;
            }
            let (primary_strategies, fallback_strategies) =
                split_strategy_tiers(mode, &facet, strategies);
            if mode == SearchMode::Auto {
                record_auto_strategy_selection(
                    config.name.as_str(),
                    resolved_caps.caps_source,
                    &primary_strategies,
                    &fallback_strategies,
                );
            }

            let search_diagnostics = SearchDiagnosticsContext::new(
                self.search_learning.clone(),
                config,
                self.plugin_provider
                    .search_semantics_version_for_provider(&config.provider_type),
                learning_context.as_ref(),
                season,
                episode,
                absolute_episode,
            );
            // Corpus reuse is opt-in per pass: only the background convergence
            // lanes mark their learning context reusable. An operator-triggered
            // Auto search (queue-best-release, the UI search buttons) fires the
            // indexer live — a corpus snapshot persisted before a new release
            // appeared would hide it for the whole reuse window, which is how
            // an explicit upgrade search stopped seeing a just-registered
            // PROPER. Interactive searches never reused.
            let reusable_strategies = if candidate_reuse_permitted(mode, learning_context.as_ref())
            {
                match search_diagnostics.as_ref() {
                    Some(diagnostics) => diagnostics.reusable_strategies(config).await,
                    None => HashMap::new(),
                }
            } else {
                HashMap::new()
            };
            let rss_category_request = request_categories.clone();
            let indexer_id = config.id.clone();
            let indexer_name = config.name.clone();
            let facet = facet.clone();
            let search_query = query.clone();
            let category_for_indexer = category.clone();
            let tagged_aliases_for_indexer = tagged_aliases.clone();
            let stats_tracker = self.stats_tracker.clone();
            let search_learning = self.search_learning.clone();
            let learning_context = learning_context.clone();
            let backoff_tracker = self.backoff_tracker.clone();
            let indexer_configs = self.indexer_configs.clone();
            let indexer_errors = self.indexer_errors.clone();
            let rss_bare_query_indexers = self.rss_bare_query_indexers.clone();
            let client = client.clone();
            let primary_strategies = primary_strategies.clone();
            let fallback_strategies = fallback_strategies.clone();
            let search_limit = search_limit.clone();
            let rate_limiter = self.rate_limiter.clone();
            let rate_limit_seconds = config.rate_limit_seconds;
            let task_cancel_token = cancel_token.child_token();
            let scheduler_lease_for_task = scheduler_lease.clone();
            let live_search_admitted = scheduler_lease_for_task.is_some();
            let page_sink = Some(page_sink.clone());

            set.spawn(async move {
                if task_cancel_token.is_cancelled() {
                    return (
                        indexer_id,
                        indexer_name,
                        scheduler_lease_for_task.clone(),
                        Err(AppError::canceled("indexer search canceled")),
                        false,
                    );
                }
                let mut collected_results = Vec::new();
                let mut reusable_strategies = reusable_strategies;
                let mut any_strategy_fired = false;
                let mut all_strategies_complete = true;
                let mut only_unattested_incompleteness = true;
                let mut primary_attempted;
                let mut primary_had_error;
                let mut primary_usable_result_count = 0_usize;
                let mut batch_had_timeout = false;
                let mut batch_health = StrategyBatchHealth::default();
                let mut quota_observation = IndexerQuotaObservation::default();

                let primary_context = StrategyTierContext {
                        client: client.clone(),
                        search_limit: search_limit.clone(),
                        rate_limiter: rate_limiter.clone(),
                        indexer_id: indexer_id.clone(),
                        search_timeout,
                        rate_limit_seconds,
                        category: category_for_indexer.clone(),
                        per_indexer_categories: rss_category_request.clone(),
                        mode,
                        operation,
                        year,
                        tagged_aliases: tagged_aliases_for_indexer.clone(),
                        cancel_token: task_cancel_token.child_token(),
                        deadline_at,
                    };
                let primary_strategies = prepare_search_strategies(
                    &primary_context,
                    primary_strategies,
                );
                let primary_selection = match select_reusable_strategies(
                    primary_strategies,
                    &mut reusable_strategies,
                    &indexer_id,
                    page_sink.as_ref().expect("stream sink is present"),
                )
                .await
                {
                    Ok(selection) => selection,
                    Err(error) => {
                        return (
                            indexer_id,
                            indexer_name,
                            scheduler_lease_for_task.clone(),
                            Err(error),
                            false,
                        );
                    }
                };
                primary_attempted = primary_selection.complete_count > 0
                    || primary_selection.deferred_count > 0
                    || !primary_selection.live.is_empty();
                primary_had_error = primary_selection.deferred_count > 0;
                if primary_selection.deferred_count > 0 {
                    all_strategies_complete = false;
                    only_unattested_incompleteness = false;
                }
                primary_usable_result_count = primary_usable_result_count
                    .saturating_add(primary_selection.replayed_result_count);
                let primary_live = primary_selection.live;
                let mut primary_outcomes = if primary_live.is_empty() {
                    StrategyTierOutcomes::Legacy(tokio::task::JoinSet::new())
                } else if live_search_admitted {
                    Self::execute_strategy_tier(
                        primary_context,
                        primary_live,
                        None,
                        page_sink.as_ref().expect("stream sink is present").clone(),
                    )
                } else {
                    primary_had_error = true;
                    all_strategies_complete = false;
                    only_unattested_incompleteness = false;
                    StrategyTierOutcomes::Legacy(tokio::task::JoinSet::new())
                };

                while let Some(join_result) = primary_outcomes.join_next().await {
                    let mut outcome = match join_result {
                        Ok(outcome) => outcome,
                        Err(error) => StrategyExecutionOutcome {
                            strategy_id: "join".into(),
                            labels: vec!["join".into()],
                            label: "join".into(),
                            title_guard_mode: TitleGuardMode::SkipTitleMatch,
                            response: Err(AppError::Repository(format!(
                                "indexer search task panicked: {error}"
                            ))),
                            page_reservation: None,
                            request_fired: true,
                            elapsed: std::time::Duration::ZERO,
                            retry_after: None,
                            rate_limited: false,
                            timed_out: false,
                        },
                    };
                    batch_had_timeout |= outcome.timed_out;
                    if !outcome.request_fired {
                        all_strategies_complete = false;
                        only_unattested_incompleteness = false;
                        if outcome.response.as_ref().is_err_and(|err| err.is_canceled()) {
                            return (
                                indexer_id,
                                indexer_name,
                                scheduler_lease_for_task.clone(),
                                Err(AppError::canceled("indexer search canceled")),
                                false,
                            );
                        }
                        debug!(
                            indexer = indexer_name.as_str(),
                            strategy = outcome.label.as_str(),
                            "skipping strategy: request was not dispatched"
                        );
                        continue;
                    }
                    any_strategy_fired = true;
                    primary_attempted = true;
                    let diagnostic_labels = outcome.labels.join("|");
                    match outcome.response {
                        Ok(mut response) => {
                            if response.completion != IndexerSearchCompletion::Complete {
                                all_strategies_complete = false;
                                only_unattested_incompleteness &= matches!(
                                    response.completion,
                                    IndexerSearchCompletion::Partial {
                                        reason: Some(IndexerSearchIncompleteReason::Unattested),
                                        ..
                                    }
                                );
                            }
                            let raw_result_count = response.results.len();
                            batch_health.mark_success();
                            debug!(
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
                            quota_observation.merge_response(&response);

                            record_strategy_metrics(
                                &indexer_name,
                                &outcome.label,
                                "success",
                                outcome.elapsed,
                                Some(response.results.len()),
                            );

                            for result in &mut response.results {
                                if let Some(current) = response.grab_current {
                                    result.extra.insert(
                                        "grab_current".to_string(),
                                        serde_json::json!(current),
                                    );
                                }
                                if let Some(max) = response.grab_max {
                                    result.extra.insert(
                                        "grab_max".to_string(),
                                        serde_json::json!(max),
                                    );
                                }
                            }

                            filter_strategy_results(
                                &mut response.results,
                                &FilterStrategyContext {
                                    query: &search_query,
                                    season,
                                    episode,
                                    tagged_aliases: &tagged_aliases_for_indexer,
                                    title_guard_mode: outcome.title_guard_mode,
                                    strategy_label: &outcome.label,
                                    is_rss_request,
                                },
                            );
                            primary_usable_result_count = primary_usable_result_count
                                .saturating_add(response.results.len());
                            if response.completion == IndexerSearchCompletion::Complete {
                                for label in &outcome.labels {
                                    record_strategy_learning_outcome(
                                        &search_learning,
                                        learning_context.as_ref(),
                                        mode,
                                        &indexer_id,
                                        &indexer_name,
                                        label,
                                        response.results.len(),
                                    )
                                    .await;
                                }
                            }
                            if page_sink.is_some() {
                                if let Some(diagnostics) = search_diagnostics.as_ref()
                                    && let Err(error) = diagnostics
                                        .persist_response(
                                            &outcome.strategy_id,
                                            &diagnostic_labels,
                                            raw_result_count,
                                            &response,
                                        )
                                        .await
                                {
                                    return (
                                        indexer_id,
                                        indexer_name,
                                        scheduler_lease_for_task.clone(),
                                        Err(error),
                                        true,
                                    );
                                }
                                let mut persisted = std::mem::take(&mut response.results);
                                for result in &mut persisted {
                                    result.indexer_id = Some(indexer_id.clone());
                                }
                                if !persisted.is_empty() {
                                    let Some(reservation) = outcome.page_reservation.take() else {
                                        return (
                                            indexer_id,
                                            indexer_name,
                                            scheduler_lease_for_task.clone(),
                                            Err(AppError::Repository(
                                                "indexer result page was not reserved".to_string(),
                                            )),
                                            true,
                                        );
                                    };
                                    if reservation.send(persisted).await.is_err() {
                                        return (
                                            indexer_id,
                                            indexer_name,
                                            scheduler_lease_for_task.clone(),
                                            Err(AppError::canceled("indexer scoring pipeline closed")),
                                            true,
                                        );
                                    }
                                }
                            } else {
                                if let Some(diagnostics) = search_diagnostics.as_ref() {
                                    diagnostics
                                        .record_response(
                                            &outcome.strategy_id,
                                            &diagnostic_labels,
                                            raw_result_count,
                                            &response,
                                        )
                                        .await;
                                }
                                for result in &mut response.results {
                                    result.indexer_id = Some(indexer_id.clone());
                                }
                                collected_results.append(&mut response.results);
                            }
                        }
                        Err(err) => {
                            if err.is_canceled() {
                                return (
                                    indexer_id,
                                    indexer_name,
                                    scheduler_lease_for_task.clone(),
                                    Err(AppError::canceled("indexer search canceled")),
                                    true,
                                );
                            }
                            primary_had_error = true;
                            all_strategies_complete = false;
                            only_unattested_incompleteness = false;
                            // The endpoint does not implement the facet-scoped
                            // function it was asked for. That is a wrong request
                            // form, not a broken indexer, so it is remembered
                            // rather than held against the indexer's health: the
                            // next sweep asks the bare-query way, and only that
                            // form's failures are health events.
                            let wrong_rss_request_form = rss_form
                                == Some(RssRequestForm::Nab)
                                && newznab_function_is_unavailable(&err);
                            if wrong_rss_request_form {
                                rss_bare_query_indexers
                                    .lock()
                                    .await
                                    .insert(indexer_id.clone());
                            }
                            if let Some(diagnostics) = search_diagnostics.as_ref() {
                                diagnostics
                                    .record_error(
                                        &outcome.strategy_id,
                                        &diagnostic_labels,
                                        &err,
                                        outcome.retry_after,
                                    )
                                    .await;
                            }
                            if !wrong_rss_request_form {
                                batch_health.mark_error(
                                    &err,
                                    outcome.retry_after,
                                    outcome.rate_limited,
                                );
                                if scryer_application::challenge_solver::is_solver_service_error_message(
                                    &err.to_string(),
                                ) {
                                    batch_health.mark_solver_failure();
                                }
                            }
                            debug!(
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
                    mode,
                    primary_usable_result_count,
                    primary_attempted,
                    primary_had_error,
                    &fallback_strategies,
                ) {
                    debug!(
                        indexer = indexer_name.as_str(),
                        facet = facet.as_str(),
                        query = search_query.as_str(),
                        reason = "zero_usable_results",
                        "indexer search falling back to title tier"
                    );

                    let fallback_context = StrategyTierContext {
                            client,
                            search_limit,
                            rate_limiter,
                            indexer_id: indexer_id.clone(),
                            search_timeout,
                            rate_limit_seconds,
                            category: category_for_indexer,
                            per_indexer_categories: rss_category_request,
                            mode,
                            operation,
                            year,
                            tagged_aliases: tagged_aliases_for_indexer.clone(),
                            cancel_token: task_cancel_token.child_token(),
                            deadline_at,
                        };
                    let fallback_strategies = prepare_search_strategies(
                        &fallback_context,
                        fallback_strategies,
                    );
                    let fallback_selection = match select_reusable_strategies(
                        fallback_strategies,
                        &mut reusable_strategies,
                        &indexer_id,
                        page_sink.as_ref().expect("stream sink is present"),
                    )
                    .await
                    {
                        Ok(selection) => selection,
                        Err(error) => {
                            return (
                                indexer_id,
                                indexer_name,
                                scheduler_lease_for_task.clone(),
                                Err(error),
                                false,
                            );
                        }
                    };
                    if fallback_selection.deferred_count > 0 {
                        all_strategies_complete = false;
                        only_unattested_incompleteness = false;
                    }
                    let fallback_live = fallback_selection.live;
                    let mut fallback_outcomes = if fallback_live.is_empty() {
                        StrategyTierOutcomes::Legacy(tokio::task::JoinSet::new())
                    } else if live_search_admitted {
                        Self::execute_strategy_tier(
                            fallback_context,
                            fallback_live,
                            None,
                            page_sink.as_ref().expect("stream sink is present").clone(),
                        )
                    } else {
                        all_strategies_complete = false;
                        only_unattested_incompleteness = false;
                        StrategyTierOutcomes::Legacy(tokio::task::JoinSet::new())
                    };

                    while let Some(join_result) = fallback_outcomes.join_next().await {
                        let mut outcome = match join_result {
                            Ok(outcome) => outcome,
                            Err(error) => StrategyExecutionOutcome {
                                strategy_id: "join".into(),
                                labels: vec!["join".into()],
                                label: "join".into(),
                                title_guard_mode: TitleGuardMode::SkipTitleMatch,
                                response: Err(AppError::Repository(format!(
                                    "indexer search task panicked: {error}"
                                ))),
                                page_reservation: None,
                                request_fired: true,
                                elapsed: std::time::Duration::ZERO,
                                retry_after: None,
                                rate_limited: false,
                                timed_out: false,
                            },
                        };
                        batch_had_timeout |= outcome.timed_out;
                        if !outcome.request_fired {
                            all_strategies_complete = false;
                            only_unattested_incompleteness = false;
                            if outcome.response.as_ref().is_err_and(|err| err.is_canceled()) {
                                return (
                                    indexer_id,
                                    indexer_name,
                                    scheduler_lease_for_task.clone(),
                                    Err(AppError::canceled("indexer search canceled")),
                                    false,
                                );
                            }
                            debug!(
                                indexer = indexer_name.as_str(),
                                strategy = outcome.label.as_str(),
                                "skipping fallback strategy: request was not dispatched"
                            );
                            continue;
                        }
                        any_strategy_fired = true;
                        let diagnostic_labels = outcome.labels.join("|");
                        match outcome.response {
                            Ok(mut response) => {
                                if response.completion != IndexerSearchCompletion::Complete {
                                    all_strategies_complete = false;
                                    only_unattested_incompleteness &= matches!(
                                        response.completion,
                                        IndexerSearchCompletion::Partial {
                                            reason: Some(
                                                IndexerSearchIncompleteReason::Unattested
                                            ),
                                            ..
                                        }
                                    );
                                }
                                let raw_result_count = response.results.len();
                                batch_health.mark_success();
                                debug!(
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
                                quota_observation.merge_response(&response);

                                record_strategy_metrics(
                                    &indexer_name,
                                    &outcome.label,
                                    "success",
                                    outcome.elapsed,
                                    Some(response.results.len()),
                                );

                                filter_strategy_results(
                                    &mut response.results,
                                    &FilterStrategyContext {
                                        query: &search_query,
                                        season,
                                        episode,
                                        tagged_aliases: &tagged_aliases_for_indexer,
                                        title_guard_mode: outcome.title_guard_mode,
                                        strategy_label: &outcome.label,
                                        is_rss_request,
                                    },
                                );
                                if response.completion == IndexerSearchCompletion::Complete {
                                    for label in &outcome.labels {
                                        record_strategy_learning_outcome(
                                            &search_learning,
                                            learning_context.as_ref(),
                                            mode,
                                            &indexer_id,
                                            &indexer_name,
                                            label,
                                            response.results.len(),
                                        )
                                        .await;
                                    }
                                }
                                if page_sink.is_some() {
                                    if let Some(diagnostics) = search_diagnostics.as_ref()
                                        && let Err(error) = diagnostics
                                            .persist_response(
                                                &outcome.strategy_id,
                                                &diagnostic_labels,
                                                raw_result_count,
                                                &response,
                                            )
                                            .await
                                    {
                                        return (
                                            indexer_id,
                                            indexer_name,
                                            scheduler_lease_for_task.clone(),
                                            Err(error),
                                            true,
                                        );
                                    }
                                    let mut persisted = std::mem::take(&mut response.results);
                                    for result in &mut persisted {
                                        result.indexer_id = Some(indexer_id.clone());
                                    }
                                    if !persisted.is_empty() {
                                        let Some(reservation) = outcome.page_reservation.take() else {
                                            return (
                                                indexer_id,
                                                indexer_name,
                                                scheduler_lease_for_task.clone(),
                                                Err(AppError::Repository(
                                                    "indexer result page was not reserved".to_string(),
                                                )),
                                                true,
                                            );
                                        };
                                        if reservation.send(persisted).await.is_err() {
                                            return (
                                                indexer_id,
                                                indexer_name,
                                                scheduler_lease_for_task.clone(),
                                                Err(AppError::canceled("indexer scoring pipeline closed")),
                                                true,
                                            );
                                        }
                                    }
                                } else {
                                    if let Some(diagnostics) = search_diagnostics.as_ref() {
                                        diagnostics
                                            .record_response(
                                                &outcome.strategy_id,
                                                &diagnostic_labels,
                                                raw_result_count,
                                                &response,
                                            )
                                            .await;
                                    }
                                    for result in &mut response.results {
                                        result.indexer_id = Some(indexer_id.clone());
                                    }
                                    collected_results.append(&mut response.results);
                                }
                            }
                            Err(err) => {
                                if err.is_canceled() {
                                    return (
                                        indexer_id,
                                        indexer_name,
                                        scheduler_lease_for_task.clone(),
                                        Err(AppError::canceled("indexer search canceled")),
                                        true,
                                    );
                                }
                                all_strategies_complete = false;
                                only_unattested_incompleteness = false;
                                if let Some(diagnostics) = search_diagnostics.as_ref() {
                                    diagnostics
                                        .record_error(
                                            &outcome.strategy_id,
                                            &diagnostic_labels,
                                            &err,
                                            outcome.retry_after,
                                        )
                                        .await;
                                }
                                batch_health.mark_error(
                                    &err,
                                    outcome.retry_after,
                                    outcome.rate_limited,
                                );
                                if scryer_application::challenge_solver::is_solver_service_error_message(
                                    &err.to_string(),
                                ) {
                                    batch_health.mark_solver_failure();
                                }
                                debug!(
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

                let batch_had_success = batch_health.any_success;
                let batch_had_error = batch_health.any_error;
                let batch_had_rate_limit = batch_health.had_rate_limit;
                let batch_had_solver_failure = batch_health.had_solver_failure;
                let batch_retry_after = batch_health.retry_after;
                let batch_rate_limit_error = batch_health.rate_limit_error.clone();
                batch_health
                    .apply(
                        &backoff_tracker,
                        &indexer_configs,
                        &indexer_id,
                        &indexer_name,
                        had_persisted_system_backoff,
                    )
                    .await;

                if batch_had_timeout && !batch_had_success {
                    let error = NewIndexerError {
                        id: scryer_domain::Id::new().0,
                        indexer_id: indexer_id.clone(),
                        indexer_name: indexer_name.clone(),
                        operation,
                        classification: IndexerErrorClassification::HttpRequestTimeout,
                        provider_error_code: None,
                        message: "Indexer search timed out".to_string(),
                        content_type: None,
                        response: None,
                        occurred_at: Utc::now(),
                    };
                    if let Err(error) = indexer_errors.record(error).await {
                        warn!(
                            indexer = indexer_name.as_str(),
                            error = %error,
                            "failed to persist indexer transport error"
                        );
                    }
                }

                if mode == SearchMode::Interactive
                    && collected_results.is_empty()
                    && !batch_had_success
                    && batch_had_error
                {
                    return (
                        indexer_id,
                        indexer_name,
                        scheduler_lease_for_task.clone(),
                        Err(if batch_had_rate_limit {
                            AppError::TemporaryUnavailable {
                                // Carry the upstream rate-limit text (status and
                                // Retry-After) with the aggregate: the interactive
                                // per-indexer status, the logs, and text-based
                                // rate-limit classification all read this
                                // message, and "all strategies failed" alone
                                // hides *why*.
                                message: match batch_rate_limit_error.as_deref() {
                                    Some(detail) => format!(
                                        "repository: all attempted indexer strategies failed: {detail}"
                                    ),
                                    None => "repository: all attempted indexer strategies failed"
                                        .to_string(),
                                },
                                retry_after: batch_retry_after,
                                rate_limit_cooldown: RateLimitCooldownAction::AlreadyRecorded,
                            }
                        } else if batch_had_solver_failure {
                            // Keep the solver-side marker so scheduler feedback
                            // classifies this as transport trouble, not a
                            // provider failure.
                            AppError::Repository(format!(
                                "all attempted indexer strategies failed: {}",
                                scryer_application::challenge_solver::BYPARR_UNAVAILABLE_MESSAGE
                            ))
                        } else {
                            AppError::Repository(
                                "all attempted indexer strategies failed".to_string(),
                            )
                        }),
                        any_strategy_fired,
                    );
                }

                let task_indexer_outcomes = scheduler_blocked_outcome
                    .filter(|_| !all_strategies_complete)
                    .map(|outcome| IndexerQueryOutcome {
                        indexer_id: indexer_id.clone(),
                        outcome,
                    })
                    .into_iter()
                    .collect();
                (
                    indexer_id,
                    indexer_name,
                    scheduler_lease_for_task.clone(),
                    Ok(IndexerSearchResponse {
                        results: collected_results,

                        indexer_outcomes: task_indexer_outcomes,
                        completion: if all_strategies_complete {
                            IndexerSearchCompletion::Complete
                        } else {
                            IndexerSearchCompletion::Partial {
                                reason: only_unattested_incompleteness
                                    .then_some(IndexerSearchIncompleteReason::Unattested),
                                retry_after: scheduler_retry_after,
                            }
                        },
                        api_current: quota_observation.api_current,
                        api_max: quota_observation.api_max,
                        grab_current: quota_observation.grab_current,
                        grab_max: quota_observation.grab_max,
                    }),
                    any_strategy_fired,
                )
            });
        }

        for (_, dispatch) in scheduler_dispatches {
            warn!(
                indexer = dispatch.config.name.as_str(),
                candidate_id = dispatch.candidate_id.as_str(),
                "scheduler returned no decision for indexer search candidate"
            );
            indexer_outcomes.push(IndexerQueryOutcome {
                indexer_id: dispatch.config.id.clone(),
                outcome: IndexerSearchOutcome::Skipped { retry_after: None },
            });
        }

        let mut all_results: Vec<IndexerSearchResult> = Vec::new();
        let mut successful_searches = 0usize;
        let mut failed_searches = 0usize;
        let mut first_failure: Option<String> = None;
        let mut first_rate_limit: Option<(Option<std::time::Duration>, RateLimitCooldownAction)> =
            None;
        loop {
            let join_result = tokio::select! {
                _ = cancel_token.cancelled() => {
                    set.abort_all();
                    while set.join_next().await.is_some() {}
                    self.rss_feed_cache.lock().await.clear();
                    return Err(AppError::canceled("indexer search canceled"));
                }
                join_result = set.join_next() => join_result,
            };

            let Some(join_result) = join_result else {
                break;
            };

            match join_result {
                Ok((id, name, scheduler_lease, Ok(mut response), should_record_feedback)) => {
                    let empty = response.results.is_empty();
                    if should_record_feedback {
                        // A fired query that returned nothing is an
                        // EmptySuccess, distinct from a hitful Success. Plan 112
                        // treats both as a successful observation for quota and
                        // cadence; the convergence ledger reads emptiness from
                        // the Fired{empty} outcome below.
                        self.record_indexer_scheduler_feedback(
                            scheduler_lease,
                            &response,
                            if empty {
                                SchedulerFeedbackOutcome::EmptySuccess
                            } else {
                                SchedulerFeedbackOutcome::Success
                            },
                            None,
                            RateLimitCooldownAction::None,
                        )
                        .await;
                    }
                    successful_searches += 1;
                    debug!(
                        indexer = name.as_str(),
                        count = response.results.len(),
                        "indexer returned aggregated results"
                    );
                    let explicit_outcomes = std::mem::take(&mut response.indexer_outcomes);
                    let outcome = match response.completion {
                        IndexerSearchCompletion::Complete => {
                            IndexerSearchOutcome::Complete { empty }
                        }
                        IndexerSearchCompletion::Partial {
                            reason,
                            retry_after,
                        } => IndexerSearchOutcome::Partial {
                            empty,
                            reason,
                            retry_after,
                        },
                    };
                    all_results.append(&mut response.results);
                    if explicit_outcomes.is_empty() {
                        indexer_outcomes.push(IndexerQueryOutcome {
                            indexer_id: id,
                            outcome,
                        });
                    } else {
                        indexer_outcomes.extend(explicit_outcomes);
                    }
                }
                Ok((id, name, scheduler_lease, Err(err), should_record_feedback)) => {
                    if err.is_canceled() {
                        set.abort_all();
                        while set.join_next().await.is_some() {}
                        self.rss_feed_cache.lock().await.clear();
                        return Err(err);
                    }
                    let was_fired = should_record_feedback
                        || scheduler_lease
                            .as_ref()
                            .is_some_and(|lease| lease.operation == SchedulerOperation::Rss);
                    if !was_fired {
                        indexer_outcomes.push(IndexerQueryOutcome {
                            indexer_id: id,
                            outcome: IndexerSearchOutcome::Skipped { retry_after: None },
                        });
                        continue;
                    }
                    failed_searches += 1;
                    first_failure = first_failure.or_else(|| Some(err.to_string()));
                    if first_rate_limit.is_none() {
                        first_rate_limit = rate_limit_signal_from_error(&err)
                            .map(|signal| (signal.retry_after, signal.cooldown_action));
                    }
                    if should_record_feedback {
                        self.record_indexer_scheduler_error(scheduler_lease, &err)
                            .await;
                    }
                    let retry_after =
                        rate_limit_signal_from_error(&err).and_then(|signal| signal.retry_after);
                    warn!(indexer = name.as_str(), error = %err, "indexer search failed");
                    indexer_outcomes.push(IndexerQueryOutcome {
                        indexer_id: id,
                        outcome: if retry_after.is_some() {
                            IndexerSearchOutcome::Deferred { retry_after }
                        } else {
                            IndexerSearchOutcome::Errored
                        },
                    });
                }
                Err(err) => {
                    failed_searches += 1;
                    first_failure = first_failure.or_else(|| Some(err.to_string()));
                    warn!(error = %err, "indexer search task panicked");
                }
            }
        }

        // Clear the RSS feed cache after all tasks complete so the next
        // search session gets fresh feeds.
        self.rss_feed_cache.lock().await.clear();

        // Persist any solver-health observations the plugin HTTP host queued
        // while this pass ran.
        scryer_application::challenge_solver::flush_solver_health(self.proxy_configs.as_ref())
            .await;

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
                debug!(
                    before,
                    after = all_results.len(),
                    deduped,
                    "deduplicated search results by URL"
                );
            }
        }

        if all_results.is_empty()
            && successful_searches == 0
            && failed_searches > 0
            && mode == SearchMode::Interactive
        {
            let failure =
                first_failure.unwrap_or_else(|| "all indexer search attempts failed".to_string());
            if let Some((retry_after, cooldown_action)) = first_rate_limit {
                return Err(AppError::TemporaryUnavailable {
                    message: format!("repository: {failure}"),
                    retry_after,
                    rate_limit_cooldown: cooldown_action,
                });
            }
            return Err(AppError::Repository(failure));
        }

        for result in &mut all_results {
            if result.parsed_release_metadata.is_none() {
                result.parsed_release_metadata =
                    Some(scryer_application::parse_release_metadata(&result.title));
            }
        }
        let completion = if indexer_outcomes
            .iter()
            .all(|outcome| outcome.outcome.coverage_eligible())
        {
            IndexerSearchCompletion::Complete
        } else {
            IndexerSearchCompletion::Partial {
                reason: None,
                retry_after: None,
            }
        };

        Ok(IndexerSearchResponse {
            results: all_results,

            completion,
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
            indexer_outcomes,
        })
    }

    async fn prune_search_learning(&self, indexer_id: &str) -> AppResult<()> {
        self.search_learning.prune_indexer(indexer_id).await
    }

    async fn finalize_search_session(
        &self,
        search_session_id: &str,
        admissible_fingerprints: &[String],
    ) -> AppResult<()> {
        self.search_learning
            .finalize_search_session(search_session_id, admissible_fingerprints)
            .await
    }
}

/// Build parallel search strategies for interactive mode.
///
/// Uses the plugin's facet-scoped `supported_ids` to determine which ID-based
/// strategies to generate. Each strategy targets one ID type so the host can
/// dispatch them all in parallel.
struct StrategyParams<'a> {
    query: &'a str,
    query_facet: &'a str,
    id_facet: &'a str,
    ids: &'a HashMap<String, String>,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
    caps: &'a scryer_domain::IndexerProviderCapabilities,
    id_dispatch_mode: IdDispatchMode,
    text_dispatch_mode: TextDispatchMode,
    is_alias_query: bool,
    /// The caller asked for a facet-less search: text strategies keep the
    /// borrowed facet for capability resolution but do not send it.
    facet_omitted: bool,
}

/// The query facet controls text-search endpoint shape. The ID facet controls
/// which provider IDs are valid for ID-backed strategies.
fn build_strategies(p: &StrategyParams<'_>) -> Vec<SearchStrategy> {
    let query = p.query;
    let query_facet = p.query_facet;
    let id_facet = p.id_facet;
    let ids = p.ids;
    let season = p.season;
    let episode = p.episode;
    let absolute_episode = p.absolute_episode;
    let caps = p.caps;
    let id_dispatch_mode = p.id_dispatch_mode;
    let text_dispatch_mode = p.text_dispatch_mode;
    let is_alias_query = p.is_alias_query;
    let structured_season = season.filter(|_| caps.season_param.is_some());
    let structured_episode = episode.filter(|_| caps.episode_param.is_some());
    let supports_absolute_episode = caps.episode_param.is_some()
        || caps
            .search_inputs
            .contains(&IndexerSearchInputCapability::AbsoluteEpisode);
    let structured_absolute_episode = absolute_episode.filter(|_| supports_absolute_episode);
    // Alias queries skip indexers that deduplicate aliases internally
    if is_alias_query && caps.deduplicates_aliases {
        return vec![];
    }

    let mut strategies = Vec::with_capacity(4);

    let eligible_ids = filter_ids_for_types(ids, caps.id_types_for_facet(id_facet));
    if !eligible_ids.is_empty() && !is_alias_query {
        let selected_ids = match id_dispatch_mode {
            IdDispatchMode::LegacyAggregate | IdDispatchMode::Aggregate => eligible_ids.clone(),
            IdDispatchMode::QueryOnly => HashMap::new(),
        };
        if id_facet == "anime" && !selected_ids.is_empty() {
            if let Some(absolute_episode) = structured_absolute_episode {
                strategies.push(SearchStrategy {
                    request_query: String::new(),
                    request_facet: id_facet.to_string(),
                    ids: selected_ids.clone(),
                    season: None,
                    episode: None,
                    absolute_episode: Some(absolute_episode),
                    generic_query_only: false,
                    omit_request_facet: false,
                    label: "ids_abs".into(),
                });
            }

            if structured_episode.is_some() {
                strategies.push(SearchStrategy {
                    request_query: String::new(),
                    request_facet: id_facet.to_string(),
                    ids: selected_ids.clone(),
                    season: structured_season,
                    episode: structured_episode,
                    absolute_episode: None,
                    generic_query_only: false,
                    omit_request_facet: false,
                    label: "ids_sxex".into(),
                });
            }
        }

        if strategies.is_empty() && !selected_ids.is_empty() {
            strategies.push(SearchStrategy {
                request_query: String::new(),
                request_facet: id_facet.to_string(),
                ids: selected_ids,
                season: structured_season,
                episode: structured_episode,
                absolute_episode: structured_absolute_episode,
                generic_query_only: false,
                omit_request_facet: false,
                label: "ids".into(),
            });
        }
    }

    let generic_query_only = text_dispatch_mode.is_generic_only();
    let text_season = text_strategy_season(caps, text_dispatch_mode, season);
    let text_episode = text_strategy_episode(caps, text_dispatch_mode, episode);
    let text_absolute_episode =
        text_strategy_absolute_episode(caps, text_dispatch_mode, absolute_episode);
    if text_dispatch_mode.can_dispatch() && caps.query_param.is_some() && !query.is_empty() {
        strategies.push(SearchStrategy {
            request_query: query.to_string(),
            request_facet: query_facet.to_string(),
            ids: HashMap::new(),
            season: text_season,
            episode: text_episode,
            absolute_episode: text_absolute_episode,
            generic_query_only,
            omit_request_facet: p.facet_omitted,
            label: if is_alias_query {
                "freetext_alias".into()
            } else {
                "freetext".into()
            },
        });
    }

    // If no strategies were generated, fall back to a single combined call
    if strategies.is_empty()
        && !query.is_empty()
        && caps.query_param.is_some()
        && text_dispatch_mode.can_dispatch()
    {
        strategies.push(SearchStrategy {
            request_query: query.to_string(),
            request_facet: query_facet.to_string(),
            ids: HashMap::new(),
            season: text_season,
            episode: text_episode,
            absolute_episode: text_absolute_episode,
            generic_query_only,
            omit_request_facet: p.facet_omitted,
            label: "fallback".into(),
        });
    }

    strategies
}

/// Whether a failed strategy says the endpoint does not implement the newznab
/// function that was asked for.
fn newznab_function_is_unavailable(error: &AppError) -> bool {
    matches!(
        scryer_application::classify_newznab_error_message(&error.to_string())
            .map(|classified| classified.classification),
        Some(
            IndexerErrorClassification::NewznabFunctionNotAvailable
                | IndexerErrorClassification::NewznabNoSuchFunction
        )
    )
}

/// Pick the request form for one indexer's RSS sweep. The facet-scoped nab
/// function is used only where the caps advertise it and the endpoint has not
/// already answered "function not available"; everything else asks the bare
/// "latest releases" query, which is a request-form choice and never a reason
/// to drop the indexer from the sweep.
fn rss_request_form(
    caps: &IndexerProviderCapabilities,
    text_dispatch_mode: TextDispatchMode,
    facet: &str,
    function_unavailable_observed: bool,
) -> RssRequestForm {
    if function_unavailable_observed {
        return RssRequestForm::BareQuery;
    }
    if matches!(text_dispatch_mode, TextDispatchMode::FacetScoped)
        && caps.supports_query_for_facet(facet)
    {
        RssRequestForm::Nab
    } else {
        RssRequestForm::BareQuery
    }
}

fn stored_caps_snapshot(config: &IndexerConfig) -> Option<IndexerCapsSnapshot> {
    if let Some(raw) = config.caps_snapshot_json.as_deref()
        && let Ok(snapshot) = serde_json::from_str::<IndexerCapsSnapshot>(raw)
    {
        return Some(snapshot);
    }

    config
        .managed_metadata_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<ManagedIndexerMetadata>(raw).ok())
        .and_then(|metadata| metadata.caps_snapshot)
}

fn supported_ids_from_caps_snapshot(
    snapshot: &IndexerCapsSnapshot,
) -> HashMap<String, Vec<String>> {
    let mut supported_ids = HashMap::new();

    let movie_ids = actionable_ids_for_node(snapshot.movie_search.as_ref(), "movie");
    if !movie_ids.is_empty() {
        supported_ids.insert("movie".to_string(), movie_ids);
    }

    let tv_ids = actionable_ids_for_node(snapshot.tv_search.as_ref(), "tv");
    if !tv_ids.is_empty() {
        supported_ids.insert("series".to_string(), tv_ids.clone());
        supported_ids.insert("anime".to_string(), tv_ids);
    }

    supported_ids
}

fn supported_external_ids_from_caps_snapshot(snapshot: &IndexerCapsSnapshot) -> Vec<String> {
    let mut ids = actionable_ids_for_node(snapshot.movie_search.as_ref(), "movie");
    ids.extend(actionable_ids_for_node(snapshot.tv_search.as_ref(), "tv"));
    ids.sort();
    ids.dedup();
    ids
}

fn preserve_direct_nab_native_capabilities(
    caps: &mut IndexerProviderCapabilities,
    static_caps: &IndexerProviderCapabilities,
    facet: &str,
) {
    let native_ids = static_caps
        .supported_ids
        .get(facet)
        .into_iter()
        .flatten()
        .filter(|id| !matches!(id.as_str(), "imdb_id" | "tvdb_id" | "tmdb_id"))
        .cloned()
        .collect::<Vec<_>>();
    if native_ids.is_empty() {
        return;
    }

    let supported_ids = caps.supported_ids.entry(facet.to_string()).or_default();
    for id in native_ids {
        if !supported_ids.contains(&id) {
            supported_ids.push(id.clone());
        }
        if !caps.supported_external_ids.contains(&id) {
            caps.supported_external_ids.push(id);
        }
    }

    for input in [
        IndexerSearchInputCapability::IdQuery,
        IndexerSearchInputCapability::AggregateIdQuery,
        IndexerSearchInputCapability::Season,
        IndexerSearchInputCapability::Episode,
        IndexerSearchInputCapability::AbsoluteEpisode,
    ] {
        if static_caps.search_inputs.contains(&input) && !caps.search_inputs.contains(&input) {
            caps.search_inputs.push(input);
        }
    }

    if caps.season_param.is_none() {
        caps.season_param.clone_from(&static_caps.season_param);
    }
    if caps.episode_param.is_none() {
        caps.episode_param.clone_from(&static_caps.episode_param);
    }
}

fn text_dispatch_mode_for_static(
    caps: &IndexerProviderCapabilities,
    facet: &str,
) -> TextDispatchMode {
    if caps.supports_query_for_facet(facet) {
        TextDispatchMode::FacetScoped
    } else {
        TextDispatchMode::None
    }
}

fn actionable_ids_for_node(node: Option<&IndexerCapsSearchNode>, search_kind: &str) -> Vec<String> {
    let Some(node) = node else {
        return Vec::new();
    };
    if !node.available {
        return Vec::new();
    }

    actionable_ids_for_params(&node.supported_params, search_kind)
}

fn actionable_ids_for_params(params: &[String], search_kind: &str) -> Vec<String> {
    let mut ids = Vec::new();
    if params.iter().any(|param| param == "imdbid") {
        ids.push("imdb_id".to_string());
    }
    if params.iter().any(|param| param == "tvdbid") {
        ids.push("tvdb_id".to_string());
    }
    if params.iter().any(|param| param == "tmdbid") {
        ids.push("tmdb_id".to_string());
    }

    if search_kind == "movie" {
        ids.sort_by_key(|value| match value.as_str() {
            "tmdb_id" => 0,
            "imdb_id" => 1,
            _ => 2,
        });
    } else {
        ids.sort_by_key(|value| match value.as_str() {
            "tvdb_id" => 0,
            "imdb_id" => 1,
            "tmdb_id" => 2,
            _ => 3,
        });
    }

    ids.dedup();
    ids
}

fn node_supports_param(node: Option<&IndexerCapsSearchNode>, param: &str) -> bool {
    node.is_some_and(|node| {
        node.available
            && node
                .supported_params
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(param))
    })
}

fn caps_snapshot_has_facet_query(snapshot: &IndexerCapsSnapshot, facet: &str) -> bool {
    match facet {
        "movie" => node_supports_param(snapshot.movie_search.as_ref(), "q"),
        "series" | "anime" => node_supports_param(snapshot.tv_search.as_ref(), "q"),
        _ => false,
    }
}

fn caps_snapshot_has_generic_query(snapshot: &IndexerCapsSnapshot) -> bool {
    node_supports_param(snapshot.search.as_ref(), "q")
}

fn caps_snapshot_text_dispatch_mode(
    snapshot: &IndexerCapsSnapshot,
    facet: &str,
) -> TextDispatchMode {
    if caps_snapshot_has_facet_query(snapshot, facet) {
        TextDispatchMode::FacetScoped
    } else if caps_snapshot_has_generic_query(snapshot) {
        TextDispatchMode::GenericOnly
    } else {
        TextDispatchMode::None
    }
}

fn caps_search_inputs(
    snapshot: &IndexerCapsSnapshot,
    facet: &str,
) -> Vec<scryer_domain::IndexerSearchInputCapability> {
    let mut inputs = Vec::new();
    if caps_snapshot_text_dispatch_mode(snapshot, facet).can_dispatch() {
        inputs.push(scryer_domain::IndexerSearchInputCapability::TitleQuery);
    }

    let facet_ids = supported_ids_from_caps_snapshot(snapshot);
    if facet_ids.get(facet).is_some_and(|ids| !ids.is_empty()) {
        inputs.push(scryer_domain::IndexerSearchInputCapability::IdQuery);
        inputs.push(scryer_domain::IndexerSearchInputCapability::AggregateIdQuery);
    }

    if node_supports_param(snapshot.tv_search.as_ref(), "season") {
        inputs.push(scryer_domain::IndexerSearchInputCapability::Season);
    }
    if node_supports_param(snapshot.tv_search.as_ref(), "ep") {
        inputs.push(scryer_domain::IndexerSearchInputCapability::Episode);
    }

    inputs
}

fn supports_search_input_or_legacy(
    caps: &IndexerProviderCapabilities,
    input: scryer_domain::IndexerSearchInputCapability,
) -> bool {
    caps.search_inputs.is_empty() || caps.search_inputs.contains(&input)
}

fn text_strategy_season(
    caps: &IndexerProviderCapabilities,
    text_dispatch_mode: TextDispatchMode,
    season: Option<u32>,
) -> Option<u32> {
    if matches!(text_dispatch_mode, TextDispatchMode::FacetScoped)
        && supports_search_input_or_legacy(
            caps,
            scryer_domain::IndexerSearchInputCapability::Season,
        )
    {
        season
    } else {
        None
    }
}

fn text_strategy_episode(
    caps: &IndexerProviderCapabilities,
    text_dispatch_mode: TextDispatchMode,
    episode: Option<u32>,
) -> Option<u32> {
    if matches!(text_dispatch_mode, TextDispatchMode::FacetScoped)
        && supports_search_input_or_legacy(
            caps,
            scryer_domain::IndexerSearchInputCapability::Episode,
        )
    {
        episode
    } else {
        None
    }
}

fn text_strategy_absolute_episode(
    caps: &IndexerProviderCapabilities,
    text_dispatch_mode: TextDispatchMode,
    absolute_episode: Option<u32>,
) -> Option<u32> {
    if matches!(text_dispatch_mode, TextDispatchMode::FacetScoped)
        && caps
            .search_inputs
            .contains(&scryer_domain::IndexerSearchInputCapability::AbsoluteEpisode)
    {
        absolute_episode
    } else {
        None
    }
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
/// nearby-but-wrong releases like "Signal Road" for a "Signal Run" search.
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
    context: &FilterStrategyContext<'_>,
) {
    if results.is_empty() {
        return;
    }

    for result in results.iter_mut() {
        result.provenance = Some(ReleaseCandidateProvenance {
            search_subject_kind: if context.is_rss_request {
                ReleaseSearchSubjectKind::Rss
            } else {
                ReleaseSearchSubjectKind::Freetext
            },
            strategy_kind: scryer_application::release_strategy_kind_for_label(
                context.strategy_label,
                context.is_rss_request,
            ),
            title_validated_upstream: context.title_guard_mode == TitleGuardMode::SkipTitleMatch,
        });
        if result.parsed_release_metadata.is_none() {
            result.parsed_release_metadata =
                Some(scryer_application::parse_release_metadata(&result.title));
        }
    }

    if context.query.is_empty() && context.season.is_none() && context.episode.is_none() {
        return;
    }

    let mut expected_titles = if context.query.is_empty() {
        Vec::new()
    } else {
        parsed_title_candidates(&scryer_application::parse_release_metadata(context.query))
    };
    expected_titles.extend(
        context
            .tagged_aliases
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

        if context.title_guard_mode == TitleGuardMode::ExactTitleMatch
            && !expected_titles.is_empty()
        {
            let release_titles = parsed_title_candidates(parsed);
            let title_ok = release_titles.iter().any(|release_title| {
                expected_titles
                    .iter()
                    .any(|expected| expected == release_title)
            });
            if !title_ok {
                tracing::debug!(
                    strategy = context.strategy_label,
                    query = %context.query,
                    expected = ?expected_titles,
                    got = ?release_titles,
                    "title guard: title mismatch"
                );
                return false;
            }
        }

        if let Some(expected_s) = context.season
            && let Some(ref res_ep) = parsed.episode
            && let Some(rs) = res_ep.season
            && rs != expected_s
        {
            tracing::debug!(
                strategy = context.strategy_label,
                query = %context.query,
                expected_season = expected_s,
                got_season = rs,
                "title guard: season mismatch"
            );
            return false;
        }

        if let Some(expected_e) = context.episode
            && let Some(ref res_ep) = parsed.episode
            && !res_ep.episode_numbers.is_empty()
            && !res_ep.episode_numbers.contains(&expected_e)
        {
            tracing::debug!(
                strategy = context.strategy_label,
                query = %context.query,
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
        debug!(
            strategy = context.strategy_label,
            before,
            after = results.len(),
            filtered,
            "title guard: removed irrelevant results"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc as StdArc, Mutex as StdMutex};

    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use scryer_application::{
        IndexerQueryStats, IndexerSearchPlanCapability, IndexerSearchPlanSummary,
        IndexerSearchResponse, ReleaseStrategyKind, SchedulerBatchDecision,
    };
    use scryer_domain::IndexerProviderCapabilities;

    use super::*;

    #[test]
    fn rss_feed_cache_entry_allows_one_feedback_claim() {
        let entry = RssFeedCacheEntry::new();

        assert!(entry.claim_feedback());
        assert!(!entry.claim_feedback());
    }

    #[test]
    fn direct_indexer_search_deadline_is_two_minutes() {
        assert_eq!(
            MultiIndexerSearchClient::effective_indexer_search_timeout(None),
            std::time::Duration::from_secs(120)
        );
    }

    #[test]
    fn only_fired_complete_strategy_executions_are_complete() {
        let response = |completion| {
            Ok(IndexerSearchResponse {
                results: Vec::new(),

                completion,
                indexer_outcomes: Vec::new(),
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        };
        let execution = |request_fired, response| StrategyExecutionOutcome {
            strategy_id: "strategy".into(),
            labels: vec!["synthetic".into()],
            label: "synthetic".into(),
            title_guard_mode: TitleGuardMode::SkipTitleMatch,
            response,
            page_reservation: None,
            request_fired,
            elapsed: std::time::Duration::ZERO,
            retry_after: None,
            rate_limited: false,
            timed_out: false,
        };

        assert!(strategy_execution_is_complete(&execution(
            true,
            response(IndexerSearchCompletion::Complete),
        )));
        assert!(!strategy_execution_is_complete(&execution(
            false,
            response(IndexerSearchCompletion::Complete),
        )));
        assert!(!strategy_execution_is_complete(&execution(
            true,
            response(IndexerSearchCompletion::Partial {
                reason: Some(IndexerSearchIncompleteReason::UpstreamFailure),
                retry_after: None,
            }),
        )));
        assert!(!strategy_execution_is_complete(&execution(
            true,
            Err(AppError::Repository("synthetic failure".into())),
        )));
    }

    #[test]
    fn strategy_batch_health_prefers_the_exact_rate_limit_error() {
        let generic = AppError::Repository("upstream status 503".to_string());
        let rate_limit = AppError::TemporaryUnavailable {
            message: "HTTP 429: User configurable Indexer Query Limit reached; retry after 60s"
                .to_string(),
            retry_after: Some(std::time::Duration::from_secs(60)),
            rate_limit_cooldown: RateLimitCooldownAction::AlreadyRecorded,
        };
        let mut health = StrategyBatchHealth::default();

        health.mark_error(&generic, None, false);
        health.mark_error(&rate_limit, Some(std::time::Duration::from_secs(60)), true);

        assert_eq!(
            health.rate_limit_error.as_deref(),
            Some(sanitize_indexer_error_message(&rate_limit.to_string()).as_str())
        );
        assert_eq!(health.retry_after, Some(std::time::Duration::from_secs(60)));
    }

    #[test]
    fn indexer_error_diagnostics_redact_credentials_and_cap_length() {
        let message = format!(
            "HTTP 429 from https://prowlarr.example/api?t=search&apikey=SECRET&token=OTHER {}",
            "x".repeat(MAX_INDEXER_ERROR_MESSAGE_BYTES * 2)
        );

        let sanitized = sanitize_indexer_error_message(&message);

        assert!(!sanitized.contains("SECRET"));
        assert!(!sanitized.contains("OTHER"));
        assert!(sanitized.contains("apikey=REDACTED"));
        assert!(sanitized.contains("token=REDACTED"));
        assert!(sanitized.len() <= MAX_INDEXER_ERROR_MESSAGE_BYTES);
    }

    #[test]
    fn id_strategies_skip_the_text_title_guard() {
        let mut ids = HashMap::new();
        ids.insert("tmdb".to_string(), "123".to_string());
        let id_strategy = SearchStrategy {
            request_query: "Amber Circuit 2026".to_string(),
            request_facet: "movie".to_string(),
            ids,
            season: None,
            episode: None,
            absolute_episode: None,
            generic_query_only: false,
            omit_request_facet: false,
            label: "ids_tmdb".to_string(),
        };
        let text_strategy = SearchStrategy {
            request_query: "Amber Circuit 2026".to_string(),
            request_facet: "movie".to_string(),
            ids: HashMap::new(),
            season: None,
            episode: None,
            absolute_episode: None,
            generic_query_only: false,
            omit_request_facet: false,
            label: "freetext".to_string(),
        };

        assert_eq!(
            title_guard_mode_for_strategy(&id_strategy),
            TitleGuardMode::SkipTitleMatch
        );
        assert_eq!(
            title_guard_mode_for_strategy(&text_strategy),
            TitleGuardMode::ExactTitleMatch
        );
    }

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
            _update: scryer_application::IndexerConfigUpdate,
        ) -> AppResult<IndexerConfig> {
            Err(AppError::Validation("not implemented in test".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct RecordingTouchIndexerConfigRepository {
        configs: Vec<IndexerConfig>,
        touched_ids: StdArc<StdMutex<Vec<String>>>,
        recorded_messages: StdArc<StdMutex<Vec<Option<String>>>>,
        cleared_ids: StdArc<StdMutex<Vec<String>>>,
    }

    #[async_trait]
    impl IndexerConfigRepository for RecordingTouchIndexerConfigRepository {
        async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(self.configs.clone())
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, id: &str) -> AppResult<()> {
            self.touched_ids
                .lock()
                .expect("touched ids mutex")
                .push(id.to_string());
            Ok(())
        }

        async fn record_last_error(&self, id: &str, message: Option<String>) -> AppResult<()> {
            self.touch_last_error(id).await?;
            self.recorded_messages
                .lock()
                .expect("recorded messages mutex")
                .push(message);
            Ok(())
        }

        async fn clear_last_error(&self, id: &str) -> AppResult<()> {
            self.cleared_ids
                .lock()
                .expect("cleared ids mutex")
                .push(id.to_string());
            Ok(())
        }

        async fn update(
            &self,
            _update: scryer_application::IndexerConfigUpdate,
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

        fn record_grab(&self, _indexer_id: &str, _indexer_name: &str) {}

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

        fn record_grab(&self, _indexer_id: &str, _indexer_name: &str) {}

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

    type CandidateDeadlineLog = StdArc<StdMutex<Vec<Vec<Option<chrono::DateTime<Utc>>>>>>;

    #[derive(Default)]
    struct RecordingScheduler {
        candidate_ids: StdArc<StdMutex<Vec<Vec<String>>>>,
        candidate_deadlines: CandidateDeadlineLog,
        feedback_candidate_ids: StdArc<StdMutex<Vec<String>>>,
        reverse_decisions: bool,
        skip_retry_after: Option<std::time::Duration>,
    }

    #[async_trait]
    impl UpstreamScheduler for RecordingScheduler {
        async fn admit_batch(
            &self,
            request: SchedulerBatchRequest,
        ) -> AppResult<SchedulerBatchDecision> {
            self.candidate_ids
                .lock()
                .expect("scheduler candidates")
                .push(
                    request
                        .candidates
                        .iter()
                        .filter_map(|candidate| candidate.plugin_config_id.clone())
                        .collect(),
                );
            self.candidate_deadlines
                .lock()
                .expect("scheduler candidate deadlines")
                .push(
                    request
                        .candidates
                        .iter()
                        .map(|candidate| candidate.deadline_at)
                        .collect(),
                );
            let mut decisions = request
                .candidates
                .into_iter()
                .map(|candidate| {
                    if let Some(retry_after) = self.skip_retry_after {
                        SchedulerAdmission::Skip {
                            candidate_id: candidate.candidate_id,
                            reason: scryer_application::SkipReason::DestinationCooldown,
                            retry_after: Some(retry_after),
                        }
                    } else {
                        SchedulerAdmission::Admit {
                            candidate_id: candidate.candidate_id.clone(),
                            lease: SchedulerLease {
                                lease_id: format!("lease-{}", candidate.candidate_id),
                                candidate_id: candidate.candidate_id,
                                host_key: candidate.host_key,
                                destination_key: candidate.destination_key,
                                account_quota_key: candidate.account_quota_key,
                                rss_request_key: candidate.rss_request_key,
                                operation: candidate.operation,
                                intent: candidate.intent,
                                issued_at: request.now,
                            },
                            reason: scryer_application::AdmissionReason::BackgroundValue,
                        }
                    }
                })
                .collect::<Vec<_>>();
            if self.reverse_decisions {
                decisions.reverse();
            }
            Ok(SchedulerBatchDecision {
                batch_id: request.batch_id,
                decisions,
            })
        }

        async fn record_feedback(&self, feedback: SchedulerFeedback) -> AppResult<()> {
            if let Some(lease) = feedback.lease {
                self.feedback_candidate_ids
                    .lock()
                    .expect("scheduler feedback")
                    .push(lease.candidate_id.to_string());
            }
            Ok(())
        }

        async fn snapshot(
            &self,
            _filter: scryer_application::SchedulerSnapshotFilter,
        ) -> AppResult<SchedulerSnapshot> {
            Ok(SchedulerSnapshot::default())
        }
    }

    #[derive(Clone, Copy)]
    enum PlanFailureMode {
        InvocationError,
        DuplicateEvent,
        MissingEvent,
    }

    struct ProtocolPlanIndexerClient {
        mode: PlanFailureMode,
    }

    #[async_trait]
    impl IndexerClient for ProtocolPlanIndexerClient {
        fn search_plan_capability(&self) -> Option<IndexerSearchPlanCapability> {
            Some(IndexerSearchPlanCapability {
                version: 1,
                max_parallel_strategies: 4,
            })
        }

        async fn search_plan(
            &self,
            request: IndexerSearchPlanRequest,
            _mode: SearchMode,
            _operation: IndexerErrorOperation,
            _cancel_token: CancellationToken,
            event_sink: IndexerSearchStrategyEventSink,
        ) -> AppResult<IndexerSearchPlanSummary> {
            let strategy_id = request.strategies[0].strategy_id.clone();
            let response = IndexerSearchResponse {
                completion: IndexerSearchCompletion::Complete,
                indexer_outcomes: Vec::new(),
                results: vec![search_result("Synthetic.Plan.Result")],
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            };
            event_sink
                .send(IndexerSearchStrategyEvent {
                    strategy_id: strategy_id.clone(),
                    response: Ok(response.clone()),
                })
                .await
                .expect("plan event receiver should remain open");

            match self.mode {
                PlanFailureMode::InvocationError => {
                    Err(AppError::Repository("synthetic plan interruption".into()))
                }
                PlanFailureMode::MissingEvent => Ok(IndexerSearchPlanSummary {
                    plan_id: request.plan_id,
                    emitted_strategy_ids: vec![strategy_id],
                }),
                PlanFailureMode::DuplicateEvent => {
                    event_sink
                        .send(IndexerSearchStrategyEvent {
                            strategy_id: strategy_id.clone(),
                            response: Ok(response),
                        })
                        .await
                        .expect("plan event receiver should remain open");
                    Ok(IndexerSearchPlanSummary {
                        plan_id: request.plan_id,
                        emitted_strategy_ids: vec![strategy_id],
                    })
                }
            }
        }

        async fn search(
            &self,
            _query: String,
            _ids: HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            _operation: IndexerErrorOperation,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _year: Option<i32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            _learning_context: Option<IndexerSearchLearningContext>,
            _cancel_token: CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            Err(AppError::Repository("unary search was not expected".into()))
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
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            _operation: IndexerErrorOperation,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _year: Option<i32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            _learning_context: Option<IndexerSearchLearningContext>,
            _cancel_token: CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(IndexerSearchResponse {
                completion: IndexerSearchCompletion::Complete,
                indexer_outcomes: Vec::new(),
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
                ..Default::default()
            }
        }
    }

    struct CapabilityByProviderPluginProvider {
        calls: Arc<AtomicUsize>,
        capabilities: HashMap<String, IndexerProviderCapabilities>,
    }

    impl IndexerPluginProvider for CapabilityByProviderPluginProvider {
        fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
            Some(Arc::new(MockIndexerClient {
                calls: self.calls.clone(),
            }))
        }

        fn available_provider_types(&self) -> Vec<String> {
            self.capabilities.keys().cloned().collect()
        }

        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            vec![]
        }

        fn capabilities_for_provider(&self, provider_type: &str) -> IndexerProviderCapabilities {
            self.capabilities
                .get(provider_type)
                .cloned()
                .unwrap_or_default()
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
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn failed_rss_attempt_defers_until_its_cadence_boundary() {
        let now = Utc::now();
        let scheduler = crate::upstream_scheduler::InMemoryUpstreamScheduler::new();
        let host_key = HostKey::from("failed-rss-cadence.example.test");
        let destination_key = DestinationKey::from(host_key.to_string());
        let rss_request_key = "rss:*".to_string();
        let target_interval = crate::upstream_scheduler::rss_target_interval();
        let target_interval_chrono =
            Duration::from_std(target_interval).expect("RSS target interval should fit chrono");
        let last_successful_poll_at = now - target_interval_chrono - Duration::minutes(5);
        let expected_latest_safe_poll_at = now + target_interval_chrono;
        let lease = |issued_at| SchedulerLease {
            lease_id: uuid::Uuid::new_v4().to_string(),
            candidate_id: SchedulerCandidateId::new(),
            host_key: host_key.clone(),
            destination_key: destination_key.clone(),
            account_quota_key: None,
            rss_request_key: Some(rss_request_key.clone()),
            operation: SchedulerOperation::Rss,
            intent: SchedulerIntent::BackgroundRss,
            issued_at,
        };

        scheduler
            .record_feedback(SchedulerFeedback {
                lease: Some(lease(last_successful_poll_at)),
                host_key: host_key.clone(),
                destination_key: destination_key.clone(),
                account_quota_key: None,
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: None,
                observed_api_max: None,
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: None,
                rss_last_seen_release_published_at: None,
                rss_feed_result_count: Some(0),
                rss_seen_release_identities: Vec::new(),
                observed_at: last_successful_poll_at,
            })
            .await
            .expect("successful RSS feedback should be recorded");
        scheduler
            .record_feedback(SchedulerFeedback {
                lease: Some(lease(now)),
                host_key: host_key.clone(),
                destination_key: destination_key.clone(),
                account_quota_key: None,
                outcome: SchedulerFeedbackOutcome::RateLimited,
                observed_api_current: None,
                observed_api_max: None,
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::AlreadyRecorded,
                rss_last_seen_release_identity: None,
                rss_last_seen_release_published_at: None,
                rss_feed_result_count: None,
                rss_seen_release_identities: Vec::new(),
                observed_at: now,
            })
            .await
            .expect("rate-limited RSS feedback should be recorded");

        let snapshot = scheduler
            .snapshot(scryer_application::SchedulerSnapshotFilter::default())
            .await
            .expect("scheduler snapshot should succeed");
        let activity = MultiIndexerSearchClient::scheduler_rss_activity(
            Some(&snapshot),
            &host_key,
            &destination_key,
            None,
            Some(&rss_request_key),
        );
        assert_eq!(
            activity.last_successful_poll_at,
            Some(last_successful_poll_at)
        );
        assert_eq!(activity.last_attempt_at, Some(now));
        assert_eq!(
            activity.latest_safe_poll_at,
            Some(expected_latest_safe_poll_at)
        );

        let freshness =
            MultiIndexerSearchClient::rss_freshness_context(&mock_indexer_config(), now, activity);
        assert_eq!(freshness.latest_safe_poll_at, expected_latest_safe_poll_at);

        let candidate = |candidate_id| SchedulerCandidate {
            candidate_id,
            plugin_config_id: Some("idx-1".to_string()),
            plugin_kind: SchedulerPluginKind::Indexer,
            operation: SchedulerOperation::Rss,
            intent: SchedulerIntent::BackgroundRss,
            host_key: host_key.clone(),
            destination_key: destination_key.clone(),
            account_quota_key: None,
            rss_request_key: Some(rss_request_key.clone()),
            estimated_cost: EstimatedCost::ONE_API_CALL,
            expected_value: ExpectedValueHint::default(),
            learning_context: None,
            deadline_at: None,
            freshness: Some(freshness.clone()),
            cancel_token: CancellationToken::new(),
        };

        let next_tick_delay =
            std::cmp::min(std::time::Duration::from_secs(60), target_interval / 2);
        let deferred = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "failed-rss-next-tick".to_string(),
                now: now
                    + Duration::from_std(next_tick_delay)
                        .expect("next RSS tick delay should fit chrono"),
                candidates: vec![candidate(SchedulerCandidateId::new())],
            })
            .await
            .expect("scheduler should evaluate the next RSS tick");
        assert!(matches!(
            deferred.decisions.as_slice(),
            [SchedulerAdmission::Defer {
                reason: scryer_application::DeferralReason::RssCadence,
                ..
            }]
        ));

        let admitted = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "failed-rss-boundary".to_string(),
                now: expected_latest_safe_poll_at,
                candidates: vec![candidate(SchedulerCandidateId::new())],
            })
            .await
            .expect("scheduler should evaluate the RSS cadence boundary");
        assert!(matches!(
            admitted.decisions.as_slice(),
            [SchedulerAdmission::Admit {
                reason: scryer_application::AdmissionReason::RssFreshness,
                ..
            }]
        ));
    }

    #[test]
    fn persisted_rss_boundary_is_not_shortened_by_activity_timestamps() {
        let now = Utc::now();
        let persisted_latest_safe_poll_at = now + Duration::minutes(30);
        let freshness = MultiIndexerSearchClient::rss_freshness_context(
            &mock_indexer_config(),
            now,
            SchedulerRssActivity {
                last_successful_poll_at: Some(now - Duration::minutes(20)),
                last_attempt_at: Some(now),
                target_interval: Some(std::time::Duration::from_secs(15 * 60)),
                latest_safe_poll_at: Some(persisted_latest_safe_poll_at),
                freshness_risk: Some(0.5),
                ..SchedulerRssActivity::default()
            },
        );

        assert_eq!(freshness.latest_safe_poll_at, persisted_latest_safe_poll_at);
    }

    #[test]
    fn successful_rss_poll_keeps_the_normal_target_interval() {
        let now = Utc::now();
        let target_interval = std::time::Duration::from_secs(15 * 60);
        let freshness = MultiIndexerSearchClient::rss_freshness_context(
            &mock_indexer_config(),
            now,
            SchedulerRssActivity {
                last_successful_poll_at: Some(now),
                last_attempt_at: Some(now),
                target_interval: Some(target_interval),
                latest_safe_poll_at: Some(now + Duration::minutes(15)),
                freshness_risk: Some(0.0),
                ..SchedulerRssActivity::default()
            },
        );

        assert_eq!(freshness.target_interval, target_interval);
        assert_eq!(freshness.latest_safe_poll_at, now + Duration::minutes(15));
    }

    #[test]
    fn first_rss_poll_keeps_its_stable_phase() {
        let now = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("valid timestamp");
        let target_interval = crate::upstream_scheduler::rss_target_interval();
        let interval_seconds = target_interval.as_secs() as i64;
        let window_start = now.timestamp() - now.timestamp().rem_euclid(interval_seconds);
        let phase = stable_phase_seconds("idx-1", target_interval.as_secs()) as i64;
        let expected =
            DateTime::<Utc>::from_timestamp(window_start + phase, 0).expect("valid phase");

        let freshness = MultiIndexerSearchClient::rss_freshness_context(
            &mock_indexer_config(),
            now,
            SchedulerRssActivity::default(),
        );

        assert_eq!(freshness.latest_safe_poll_at, expected);
    }

    fn managed_auto_mode_metadata(enable_rss: bool, enable_automatic_search: bool) -> String {
        serde_json::json!({
            "enable_rss": enable_rss,
            "enable_automatic_search": enable_automatic_search,
        })
        .to_string()
    }

    fn managed_metadata_with_caps(snapshot: Option<IndexerCapsSnapshot>) -> String {
        serde_json::json!({
            "enable_rss": true,
            "enable_automatic_search": true,
            "caps_snapshot": snapshot,
        })
        .to_string()
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RecordedCall {
        query: String,
        ids: HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        categories: Vec<String>,
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
            category: Option<String>,
            facet: Option<String>,
            _id_search_facet: Option<String>,
            newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            _operation: IndexerErrorOperation,
            season: Option<u32>,
            episode: Option<u32>,
            absolute_episode: Option<u32>,
            _year: Option<i32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            _learning_context: Option<IndexerSearchLearningContext>,
            _cancel_token: CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            let call = RecordedCall {
                query,
                ids,
                category,
                facet,
                categories: newznab_categories.unwrap_or_default(),
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

    struct OrderedStartIndexerClient {
        indexer_id: String,
        starts: StdArc<StdMutex<Vec<String>>>,
    }

    #[async_trait]
    impl IndexerClient for OrderedStartIndexerClient {
        async fn search(
            &self,
            _query: String,
            _ids: HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            _operation: IndexerErrorOperation,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _year: Option<i32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            _learning_context: Option<IndexerSearchLearningContext>,
            _cancel_token: CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            self.starts
                .lock()
                .expect("start order mutex")
                .push(self.indexer_id.clone());
            Ok(IndexerSearchResponse {
                completion: IndexerSearchCompletion::Complete,
                indexer_outcomes: Vec::new(),
                results: vec![],

                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        }
    }

    struct ConfigBoundPluginProvider {
        starts: StdArc<StdMutex<Vec<String>>>,
        caps: IndexerProviderCapabilities,
    }

    impl IndexerPluginProvider for ConfigBoundPluginProvider {
        fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
            Some(Arc::new(OrderedStartIndexerClient {
                indexer_id: config.id.clone(),
                starts: self.starts.clone(),
            }))
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

    #[derive(Default)]
    struct SearchConcurrencyProbe {
        active: AtomicUsize,
        max_active: AtomicUsize,
        started: AtomicUsize,
        released: AtomicBool,
        release: tokio::sync::Notify,
    }

    impl SearchConcurrencyProbe {
        fn mark_started(&self) {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.started.fetch_add(1, Ordering::SeqCst);

            let mut max_active = self.max_active.load(Ordering::SeqCst);
            while active > max_active {
                match self.max_active.compare_exchange(
                    max_active,
                    active,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(observed) => max_active = observed,
                }
            }
        }

        async fn wait_until_released(&self) {
            while !self.released.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
        }

        fn mark_finished(&self) {
            self.active.fetch_sub(1, Ordering::SeqCst);
        }

        fn release_all(&self) {
            self.released.store(true, Ordering::SeqCst);
            self.release.notify_waiters();
        }
    }

    struct BlockingIndexerClient {
        probe: StdArc<SearchConcurrencyProbe>,
    }

    #[async_trait]
    impl IndexerClient for BlockingIndexerClient {
        async fn search(
            &self,
            _query: String,
            _ids: HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            _operation: IndexerErrorOperation,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _year: Option<i32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            _learning_context: Option<IndexerSearchLearningContext>,
            cancel_token: CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            self.probe.mark_started();
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    self.probe.mark_finished();
                    return Err(AppError::canceled("blocking indexer search canceled"));
                }
                _ = self.probe.wait_until_released() => {}
            }
            self.probe.mark_finished();
            Ok(IndexerSearchResponse {
                completion: IndexerSearchCompletion::Complete,
                indexer_outcomes: Vec::new(),
                results: vec![],

                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        }
    }

    async fn wait_for_started(probe: &SearchConcurrencyProbe, expected: usize) {
        for _ in 0..100 {
            if probe.started.load(Ordering::SeqCst) >= expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!(
            "timed out waiting for {expected} searches to start; saw {}",
            probe.started.load(Ordering::SeqCst)
        );
    }

    fn indexed_mock_configs(count: usize) -> Vec<IndexerConfig> {
        (0..count)
            .map(|idx| {
                let mut config = mock_indexer_config();
                config.id = format!("idx-{idx}");
                config.name = format!("Mock Indexer {idx}");
                config
            })
            .collect()
    }

    /// A consenting background context: the lane a convergence sweep's
    /// searches run in. Auto-mode admission caps only apply to these passes —
    /// a context-less Auto search is operator-shaped and uses the interactive
    /// lane.
    fn background_pass_context() -> IndexerSearchLearningContext {
        IndexerSearchLearningContext {
            title_id: "title-1".into(),
            facet: "movie".into(),
            subject_kind: ReleaseSearchSubjectKind::Title,
            search_session_id: "session".into(),
            background_value: Some(0.5),
            candidate_reuse_allowed: true,
        }
    }

    async fn assert_leaf_search_limit_shared_across_clones(mode: SearchMode, limit: usize) {
        let config_count = limit + 4;
        let probe = StdArc::new(SearchConcurrencyProbe::default());
        let client = Arc::new(BlockingIndexerClient {
            probe: probe.clone(),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: indexed_mock_configs(config_count),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );
        let first = multi.clone();
        let second = multi.clone();
        let operation = match mode {
            SearchMode::Interactive => IndexerErrorOperation::InteractiveSearch,
            SearchMode::Auto => IndexerErrorOperation::AutomaticSearch,
        };
        // Auto-mode admission caps apply to background passes; a context-less
        // Auto search would take the operator's interactive lane instead.
        let learning_context = Some(background_pass_context());
        let first_context = learning_context.clone();
        let first_search = tokio::spawn(async move {
            <MultiIndexerSearchClient as IndexerClient>::search(
                &first,
                "Search Limit".to_string(),
                HashMap::new(),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                mode,
                operation,
                None,
                None,
                None,
                None,
                vec![],
                first_context,
                CancellationToken::new(),
            )
            .await
        });
        let second_context = learning_context.clone();
        let second_search = tokio::spawn(async move {
            <MultiIndexerSearchClient as IndexerClient>::search(
                &second,
                "Search Limit".to_string(),
                HashMap::new(),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                mode,
                operation,
                None,
                None,
                None,
                None,
                vec![],
                second_context,
                CancellationToken::new(),
            )
            .await
        });

        wait_for_started(&probe, limit).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(probe.max_active.load(Ordering::SeqCst), limit);
        assert_eq!(probe.started.load(Ordering::SeqCst), limit);

        probe.release_all();
        tokio::time::timeout(std::time::Duration::from_secs(2), first_search)
            .await
            .expect("first search should finish")
            .expect("first search task should join")
            .expect("first search should succeed");
        tokio::time::timeout(std::time::Duration::from_secs(2), second_search)
            .await
            .expect("second search should finish")
            .expect("second search task should join")
            .expect("second search should succeed");

        assert_eq!(probe.started.load(Ordering::SeqCst), config_count * 2);
        assert!(probe.max_active.load(Ordering::SeqCst) <= limit);
    }

    #[tokio::test]
    async fn interactive_search_cancellation_returns_promptly() {
        let probe = StdArc::new(SearchConcurrencyProbe::default());
        let client = Arc::new(BlockingIndexerClient {
            probe: probe.clone(),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: indexed_mock_configs(3),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );
        let cancel_token = CancellationToken::new();
        let search_cancel_token = cancel_token.clone();
        let search = tokio::spawn(async move {
            <MultiIndexerSearchClient as IndexerClient>::search(
                &multi,
                "Cancel Me".to_string(),
                HashMap::new(),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                IndexerErrorOperation::InteractiveSearch,
                None,
                None,
                None,
                None,
                vec![],
                None,
                search_cancel_token,
            )
            .await
        });

        wait_for_started(&probe, 1).await;
        cancel_token.cancel();
        let error = tokio::time::timeout(std::time::Duration::from_secs(2), search)
            .await
            .expect("search should return promptly")
            .expect("search task should join")
            .expect_err("search should be canceled");
        assert!(error.is_canceled(), "unexpected error: {error}");
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
            indexer_id: None,
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
            response_attributes: Default::default(),
            guid: None,
            info_url: None,
            provenance: None,
            candidate_token: None,
            queue_scope: None,
            coverage_scope: None,
            auto_eligible: None,
            auto_decision_code: None,
            auto_decision_summary: None,
        }
    }

    fn prepared_strategy(strategy_id: &str) -> PreparedSearchStrategy {
        PreparedSearchStrategy {
            strategy_id: strategy_id.to_string(),
            labels: vec![format!("strategy:{strategy_id}")],
            request: IndexerSearchStrategyRequest {
                strategy_id: strategy_id.to_string(),
                labels: vec![format!("strategy:{strategy_id}")],
                query: format!("synthetic-{strategy_id}"),
                ids: HashMap::new(),
                category: None,
                facet: None,
                id_search_facet: None,
                newznab_categories: None,
                season: None,
                episode: None,
                absolute_episode: None,
                year: None,
                tagged_aliases: Vec::new(),
            },
            title_guard_mode: TitleGuardMode::SkipTitleMatch,
        }
    }

    async fn protocol_plan_outcomes(mode: PlanFailureMode) -> Vec<StrategyExecutionOutcome> {
        let (page_tx, _page_rx) = tokio::sync::mpsc::channel(4);
        let mut outcomes = MultiIndexerSearchClient::execute_plan_strategy_tier(
            StrategyTierContext {
                client: Arc::new(ProtocolPlanIndexerClient { mode }),
                search_limit: Arc::new(Semaphore::new(1)),
                rate_limiter: IndexerRateLimiter::new(),
                indexer_id: "indexer-1".into(),
                search_timeout: std::time::Duration::from_secs(5),
                rate_limit_seconds: None,
                category: None,
                per_indexer_categories: None,
                mode: SearchMode::Auto,
                operation: IndexerErrorOperation::AutomaticSearch,
                year: None,
                tagged_aliases: Vec::new(),
                cancel_token: CancellationToken::new(),
                deadline_at: None,
            },
            vec![prepared_strategy("first"), prepared_strategy("second")],
            None,
            IndexerSearchPageSink::new(page_tx, 4),
        );
        let mut collected = Vec::new();
        while let Some(outcome) = outcomes.join_next().await {
            collected.push(outcome.expect("plan controller does not use join tasks"));
        }
        collected
    }

    fn year_tier_context(year: Option<i32>) -> StrategyTierContext {
        StrategyTierContext {
            client: Arc::new(ProtocolPlanIndexerClient {
                mode: PlanFailureMode::MissingEvent,
            }),
            search_limit: Arc::new(Semaphore::new(1)),
            rate_limiter: IndexerRateLimiter::new(),
            indexer_id: "indexer-1".into(),
            search_timeout: std::time::Duration::from_secs(5),
            rate_limit_seconds: None,
            category: None,
            per_indexer_categories: None,
            mode: SearchMode::Interactive,
            operation: IndexerErrorOperation::InteractiveSearch,
            year,
            tagged_aliases: Vec::new(),
            cancel_token: CancellationToken::new(),
            deadline_at: None,
        }
    }

    fn freetext_strategy(query: &str) -> SearchStrategy {
        SearchStrategy {
            request_query: query.to_string(),
            request_facet: "movie".into(),
            ids: HashMap::new(),
            season: None,
            episode: None,
            absolute_episode: None,
            generic_query_only: false,
            omit_request_facet: false,
            label: "freetext".into(),
        }
    }

    #[test]
    fn prepared_strategies_carry_the_tier_year() {
        let prepared = prepare_search_strategies(
            &year_tier_context(Some(2026)),
            vec![freetext_strategy("Amber Circuit 2026")],
        );

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].request.year, Some(2026));
        // The year is a qualifier on the subject, not an id: it must not turn a
        // freetext strategy into an id-backed one.
        assert!(prepared[0].request.ids.is_empty());
        assert_eq!(
            prepared[0].title_guard_mode,
            TitleGuardMode::ExactTitleMatch
        );
    }

    #[test]
    fn prepared_strategies_omit_an_unknown_year() {
        let prepared = prepare_search_strategies(
            &year_tier_context(None),
            vec![freetext_strategy("Example Show S01")],
        );

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].request.year, None);
    }

    #[test]
    fn year_separates_otherwise_identical_strategy_identities() {
        let with_year = prepare_search_strategies(
            &year_tier_context(Some(2026)),
            vec![freetext_strategy("Amber Circuit")],
        );
        let without_year = prepare_search_strategies(
            &year_tier_context(None),
            vec![freetext_strategy("Amber Circuit")],
        );

        assert_ne!(
            with_year[0].strategy_id, without_year[0].strategy_id,
            "a year-qualified request is a different effective request"
        );
    }

    #[tokio::test]
    async fn interrupted_plan_keeps_emitted_strategy_complete() {
        let outcomes = protocol_plan_outcomes(PlanFailureMode::InvocationError).await;
        let first = outcomes
            .iter()
            .filter(|outcome| outcome.strategy_id == "first")
            .collect::<Vec<_>>();
        assert_eq!(first.len(), 1);
        assert!(
            strategy_execution_is_complete(first[0]),
            "emitted strategy outcome was not complete: {:?}",
            first[0]
        );
        assert!(
            outcomes
                .iter()
                .any(|outcome| { outcome.strategy_id == "second" && outcome.response.is_err() })
        );
    }

    #[tokio::test]
    async fn structurally_invalid_plan_reopens_every_strategy() {
        for mode in [
            PlanFailureMode::DuplicateEvent,
            PlanFailureMode::MissingEvent,
        ] {
            let outcomes = protocol_plan_outcomes(mode).await;
            for strategy_id in ["first", "second"] {
                assert!(outcomes.iter().any(|outcome| {
                    outcome.strategy_id == strategy_id && outcome.response.is_err()
                }));
            }
        }
    }

    #[tokio::test]
    async fn strategy_reuse_replays_candidates_and_retries_only_open_work() {
        let strategies = vec![
            prepared_strategy("complete"),
            prepared_strategy("deferred"),
            prepared_strategy("partial"),
        ];
        let mut reusable = HashMap::from([
            (
                "complete".to_string(),
                ReusableStrategyState {
                    completion_state: "complete".into(),
                    retry_at: None,
                    candidates: vec![search_result("Synthetic.Complete")],
                },
            ),
            (
                "deferred".to_string(),
                ReusableStrategyState {
                    completion_state: "deferred".into(),
                    retry_at: Some(Utc::now() + Duration::minutes(1)),
                    candidates: vec![search_result("Synthetic.Deferred")],
                },
            ),
            (
                "partial".to_string(),
                ReusableStrategyState {
                    completion_state: "partial".into(),
                    retry_at: Some(Utc::now() + Duration::minutes(1)),
                    candidates: vec![search_result("Synthetic.Partial")],
                },
            ),
        ]);
        let (page_tx, mut page_rx) = tokio::sync::mpsc::channel(4);
        let page_sink = IndexerSearchPageSink::new(page_tx, 4);

        let selection =
            select_reusable_strategies(strategies, &mut reusable, "indexer-1", &page_sink)
                .await
                .expect("strategy reuse should succeed");

        assert_eq!(selection.complete_count, 1);
        assert_eq!(selection.deferred_count, 1);
        assert_eq!(selection.replayed_result_count, 3);
        assert_eq!(selection.live.len(), 1);
        assert_eq!(selection.live[0].strategy_id, "partial");

        let mut replayed_pages = 0;
        while let Ok(page) = page_rx.try_recv() {
            replayed_pages += 1;
            assert_eq!(page.results.len(), 1);
            assert_eq!(page.results[0].indexer_id.as_deref(), Some("indexer-1"));
        }
        assert_eq!(replayed_pages, 3);
    }

    #[test]
    fn reusable_strategy_provenance_matches_live_strategy_labels() {
        let id_backed = reusable_strategy_provenance("ids_tvdb|ids");
        assert_eq!(id_backed.strategy_kind, ReleaseStrategyKind::IdBacked);
        assert!(id_backed.title_validated_upstream);

        let freetext = reusable_strategy_provenance("freetext_alias");
        assert_eq!(freetext.strategy_kind, ReleaseStrategyKind::Freetext);
        assert!(!freetext.title_validated_upstream);
    }

    fn response_with_titles(titles: &[&str]) -> AppResult<IndexerSearchResponse> {
        Ok(IndexerSearchResponse {
            completion: IndexerSearchCompletion::Complete,
            indexer_outcomes: Vec::new(),
            results: titles.iter().map(|title| search_result(title)).collect(),

            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }

    fn rss_only_caps() -> IndexerProviderCapabilities {
        IndexerProviderCapabilities {
            rss: true,
            ..Default::default()
        }
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn scheduler_priority_controls_indexer_request_start_order() {
        let starts = StdArc::new(StdMutex::new(Vec::new()));
        let scheduler = Arc::new(RecordingScheduler {
            reverse_decisions: true,
            ..RecordingScheduler::default()
        });
        let mut multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: indexed_mock_configs(3),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ConfigBoundPluginProvider {
                starts: starts.clone(),
                caps: movie_caps(),
            }),
        )
        .with_upstream_scheduler(scheduler.clone());
        multi.interactive_search_limit = Arc::new(Semaphore::new(1));

        multi
            .search(
                "Ranked Search".to_string(),
                HashMap::new(),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("ranked search should succeed");

        let mut expected = scheduler
            .candidate_ids
            .lock()
            .expect("scheduler candidates")
            .first()
            .expect("scheduler batch")
            .clone();
        expected.reverse();
        assert_eq!(*starts.lock().expect("start order mutex"), expected);
    }

    #[tokio::test]
    async fn rss_cache_followers_do_not_consume_search_permits() {
        let probe = Arc::new(SearchConcurrencyProbe::default());
        let scheduler = Arc::new(RecordingScheduler::default());
        let mut multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client: Arc::new(BlockingIndexerClient {
                    probe: probe.clone(),
                }),
                caps: rss_only_caps(),
            }),
        )
        .with_upstream_scheduler(scheduler);
        let background_limit = Arc::new(Semaphore::new(2));
        multi.background_search_limit = background_limit.clone();

        let first_client = multi.clone();
        let first = tokio::spawn(async move {
            first_client
                .search(
                    String::new(),
                    HashMap::new(),
                    None,
                    None,
                    None,
                    Some(vec!["2000".to_string()]),
                    None,
                    SearchMode::Auto,
                    None,
                    None,
                    None,
                    vec![],
                )
                .await
        });
        wait_for_started(&probe, 1).await;
        assert_eq!(background_limit.available_permits(), 1);

        let second_client = multi.clone();
        let second = tokio::spawn(async move {
            second_client
                .search(
                    String::new(),
                    HashMap::new(),
                    None,
                    None,
                    None,
                    Some(vec!["2000".to_string()]),
                    None,
                    SearchMode::Auto,
                    None,
                    None,
                    None,
                    vec![],
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(probe.started.load(Ordering::SeqCst), 1);
        assert_eq!(
            background_limit.available_permits(),
            1,
            "the cache follower must wait without holding a search permit"
        );

        probe.release_all();
        first
            .await
            .expect("first RSS task should join")
            .expect("first RSS search should succeed");
        second
            .await
            .expect("second RSS task should join")
            .expect("cached RSS search should succeed");
    }

    #[tokio::test]
    async fn interactive_candidates_wait_for_admission_without_a_deadline() {
        let calls = Arc::new(AtomicUsize::new(0));
        let scheduler = Arc::new(RecordingScheduler::default());
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: false,
                calls: calls.clone(),
            }),
        )
        .with_upstream_scheduler(scheduler.clone());

        multi
            .search(
                "Interactive Search".to_string(),
                HashMap::new(),
                None,
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("interactive search should complete");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            scheduler
                .candidate_deadlines
                .lock()
                .expect("scheduler candidate deadlines")
                .as_slice(),
            [vec![None]]
        );
    }

    #[tokio::test]
    async fn destination_cooldown_skip_reports_remaining_delay_per_indexer() {
        let calls = Arc::new(AtomicUsize::new(0));
        let retry_after = std::time::Duration::from_secs(60);
        let scheduler = Arc::new(RecordingScheduler {
            skip_retry_after: Some(retry_after),
            ..Default::default()
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: false,
                calls: calls.clone(),
            }),
        )
        .with_upstream_scheduler(scheduler);

        let response = multi
            .search(
                "Cooldown Search".to_string(),
                HashMap::new(),
                None,
                Some("movie".to_string()),
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
            .expect("destination cooldown should surface as a skipped outcome");

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            response.indexer_outcomes.as_slice(),
            [IndexerQueryOutcome {
                outcome: IndexerSearchOutcome::Skipped {
                    retry_after: Some(delay)
                },
                ..
            }] if *delay == retry_after
        ));
    }

    #[tokio::test]
    async fn queued_search_permit_stops_on_cancellation() {
        let search_limit = Arc::new(Semaphore::new(0));
        let cancel_token = CancellationToken::new();
        cancel_token.cancel();

        let result = acquire_search_permit(search_limit, &cancel_token, None).await;

        assert_eq!(result.unwrap_err(), SearchPermitError::Cancelled);
    }

    #[test]
    fn indexer_rate_limit_domains_are_config_scoped() {
        let mut first = mock_indexer_config();
        first.id = "config-a".to_string();
        let mut second = first.clone();
        second.id = "config-b".to_string();
        let mut managed_child = first.clone();
        managed_child.managed_parent_config_id = Some("parent-config".to_string());
        managed_child.managed_child_key = Some("child-42".to_string());

        let (first_host, first_domain) =
            MultiIndexerSearchClient::scheduler_keys_for_indexer(&first);
        let (second_host, second_domain) =
            MultiIndexerSearchClient::scheduler_keys_for_indexer(&second);
        let (_, child_domain) =
            MultiIndexerSearchClient::scheduler_keys_for_indexer(&managed_child);

        assert_eq!(first_host, second_host);
        assert_eq!(first_domain.as_str(), "config-a");
        assert_eq!(second_domain.as_str(), "config-b");
        assert_eq!(child_domain.as_str(), "parent-config:child-42");
    }

    #[test]
    fn automatic_strategy_capacity_is_fixed_at_four() {
        let client = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: Vec::new(),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: false,
                calls: Arc::new(AtomicUsize::new(0)),
            }),
        );

        assert_eq!(
            client.background_search_limit.available_permits(),
            BACKGROUND_INDEXER_SEARCH_CONCURRENCY_LIMIT
        );
        assert_eq!(BACKGROUND_INDEXER_SEARCH_CONCURRENCY_LIMIT, 4);
    }

    #[tokio::test]
    async fn automatic_strategy_concurrency_is_shared_across_cloned_clients() {
        assert_leaf_search_limit_shared_across_clones(
            SearchMode::Auto,
            BACKGROUND_INDEXER_SEARCH_CONCURRENCY_LIMIT,
        )
        .await;
    }

    #[tokio::test]
    async fn interactive_strategy_concurrency_is_shared_across_cloned_clients() {
        assert_leaf_search_limit_shared_across_clones(
            SearchMode::Interactive,
            INTERACTIVE_INDEXER_SEARCH_CONCURRENCY_LIMIT,
        )
        .await;
    }

    fn prowlarr_caps_snapshot(movie_params: &[&str], tv_params: &[&str]) -> IndexerCapsSnapshot {
        prowlarr_caps_snapshot_with_availability(true, movie_params, true, tv_params)
    }

    fn prowlarr_caps_snapshot_with_availability(
        movie_available: bool,
        movie_params: &[&str],
        tv_available: bool,
        tv_params: &[&str],
    ) -> IndexerCapsSnapshot {
        IndexerCapsSnapshot {
            search: Some(IndexerCapsSearchNode {
                available: true,
                supported_params: vec!["q".to_string()],
                search_engine: None,
            }),
            movie_search: Some(IndexerCapsSearchNode {
                available: movie_available,
                supported_params: movie_params.iter().map(|value| value.to_string()).collect(),
                search_engine: None,
            }),
            tv_search: Some(IndexerCapsSearchNode {
                available: tv_available,
                supported_params: tv_params.iter().map(|value| value.to_string()).collect(),
                search_engine: None,
            }),
            ..IndexerCapsSnapshot::default()
        }
    }

    #[tokio::test]
    async fn indexer_failure_records_last_error_for_config_id() {
        let touched_ids = StdArc::new(StdMutex::new(Vec::new()));
        let recorded_messages = StdArc::new(StdMutex::new(Vec::new()));
        let cleared_ids = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: StdArc::new(StdMutex::new(Vec::new())),
            responder: StdArc::new(|_| Err(AppError::Repository("upstream status 503".into()))),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(RecordingTouchIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
                touched_ids: touched_ids.clone(),
                recorded_messages: recorded_messages.clone(),
                cleared_ids: cleared_ids.clone(),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: IndexerProviderCapabilities {
                    rss: true,
                    search: false,
                    imdb_search: false,
                    tvdb_search: false,
                    anidb_search: false,
                    supported_ids: HashMap::new(),
                    ..Default::default()
                },
            }),
        );

        let response = multi
            .search(
                String::new(),
                HashMap::new(),
                None,
                Some("series".to_string()),
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
            .expect("RSS failure is isolated to the indexer");

        assert!(response.results.is_empty());
        assert_eq!(
            *touched_ids.lock().expect("touched ids mutex"),
            vec!["idx-1".to_string()]
        );
        assert_eq!(
            *recorded_messages.lock().expect("recorded messages mutex"),
            vec![Some("repository: upstream status 503".to_string())]
        );
        assert!(cleared_ids.lock().expect("cleared ids mutex").is_empty());
    }

    #[tokio::test]
    async fn indexer_success_clears_last_error_for_config_id() {
        let touched_ids = StdArc::new(StdMutex::new(Vec::new()));
        let recorded_messages = StdArc::new(StdMutex::new(Vec::new()));
        let cleared_ids = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: StdArc::new(StdMutex::new(Vec::new())),
            responder: StdArc::new(|_| {
                Ok(IndexerSearchResponse {
                    completion: IndexerSearchCompletion::Complete,
                    indexer_outcomes: Vec::new(),
                    results: vec![search_result("Recovered.Show.S01E01")],

                    api_current: None,
                    api_max: None,
                    grab_current: None,
                    grab_max: None,
                })
            }),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(RecordingTouchIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
                touched_ids: touched_ids.clone(),
                recorded_messages: recorded_messages.clone(),
                cleared_ids: cleared_ids.clone(),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: IndexerProviderCapabilities {
                    rss: true,
                    search: false,
                    imdb_search: false,
                    tvdb_search: false,
                    anidb_search: false,
                    supported_ids: HashMap::new(),
                    ..Default::default()
                },
            }),
        );

        let response = multi
            .search(
                String::new(),
                HashMap::new(),
                None,
                Some("series".to_string()),
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
            .expect("RSS success should succeed");

        assert_eq!(response.results.len(), 1);
        assert!(touched_ids.lock().expect("touched ids mutex").is_empty());
        assert!(
            recorded_messages
                .lock()
                .expect("recorded messages mutex")
                .is_empty()
        );
        assert_eq!(
            *cleared_ids.lock().expect("cleared ids mutex"),
            vec!["idx-1".to_string()]
        );
    }

    #[tokio::test]
    async fn indexer_failure_then_fallback_success_only_clears_last_error() {
        let touched_ids = StdArc::new(StdMutex::new(Vec::new()));
        let recorded_messages = StdArc::new(StdMutex::new(Vec::new()));
        let cleared_ids = StdArc::new(StdMutex::new(Vec::new()));
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let attempts = StdArc::new(AtomicUsize::new(0));
        let attempts_for_responder = attempts.clone();
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(move |call| {
                let attempt = attempts_for_responder.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    assert!(call.ids.contains_key("tvdb_id"));
                    return Err(AppError::Validation("id tier failed".into()));
                }

                assert!(call.ids.is_empty());
                response_with_titles(&["Signal.Run.S01E12.720p.WEB-DL"])
            }),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(RecordingTouchIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
                touched_ids: touched_ids.clone(),
                recorded_messages: recorded_messages.clone(),
                cleared_ids: cleared_ids.clone(),
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: series_caps(),
            }),
        );

        let response = multi
            .search(
                "Signal Run S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("fallback success should succeed");

        let recorded_calls = calls.lock().expect("calls").clone();
        assert_eq!(recorded_calls.len(), 2);
        assert_eq!(response.results.len(), 1);
        assert!(touched_ids.lock().expect("touched ids mutex").is_empty());
        assert!(
            recorded_messages
                .lock()
                .expect("recorded messages mutex")
                .is_empty()
        );
        assert_eq!(
            *cleared_ids.lock().expect("cleared ids mutex"),
            vec!["idx-1".to_string()]
        );
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
    async fn rss_sync_search_skips_managed_indexers_when_metadata_disables_rss() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut config = mock_indexer_config();
        config.managed_metadata_json = Some(managed_auto_mode_metadata(false, true));
        let client = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: true,
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
    async fn scheduler_candidates_exclude_indexers_without_dispatchable_capabilities() {
        let calls = Arc::new(AtomicUsize::new(0));
        let scheduler = Arc::new(RecordingScheduler::default());
        let recorded_candidates = scheduler.candidate_ids.clone();
        let mut executable = mock_indexer_config();
        executable.id = "idx-executable".to_string();
        executable.provider_type = "exec".to_string();
        let mut ineligible = mock_indexer_config();
        ineligible.id = "idx-ineligible".to_string();
        ineligible.provider_type = "ineligible".to_string();

        let executable_caps = IndexerProviderCapabilities {
            supported_ids: HashMap::from([("series".into(), vec!["tvdb_id".into()])]),
            query_param: Some("q".into()),
            search: true,
            tvdb_search: true,
            ..Default::default()
        };
        let client = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![executable, ineligible],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(CapabilityByProviderPluginProvider {
                calls,
                capabilities: HashMap::from([
                    ("exec".to_string(), executable_caps),
                    (
                        "ineligible".to_string(),
                        IndexerProviderCapabilities::default(),
                    ),
                ]),
            }),
        )
        .with_upstream_scheduler(scheduler);

        let _ = client
            .search(
                "Example Show".to_string(),
                HashMap::new(),
                None,
                Some("series".to_string()),
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
            .expect("automatic search should succeed");

        assert_eq!(
            recorded_candidates
                .lock()
                .expect("scheduler candidates")
                .as_slice(),
            &[vec!["idx-executable".to_string()]]
        );
    }

    #[tokio::test]
    async fn automatic_search_skips_managed_indexers_when_metadata_disables_automatic_search() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut config = mock_indexer_config();
        config.managed_metadata_json = Some(managed_auto_mode_metadata(true, false));
        let client = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: true,
                calls: calls.clone(),
            }),
        );

        let response = client
            .search(
                "Example Show".to_string(),
                HashMap::new(),
                None,
                Some("series".to_string()),
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
            .expect("automatic search should succeed");

        assert!(response.results.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn automatic_search_skips_caps_failed_indexers_until_health_recovers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut unhealthy = mock_indexer_config();
        unhealthy.last_error_message = Some("caps refresh failed: synthetic failure".into());
        let client = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![unhealthy],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: true,
                calls: calls.clone(),
            }),
        );

        client
            .search(
                "Synthetic Series".to_string(),
                HashMap::new(),
                None,
                Some("series".to_string()),
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
            .expect("automatic search should skip an unhealthy indexer");
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let recovered = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: true,
                calls: calls.clone(),
            }),
        );
        recovered
            .search(
                "Synthetic Series".to_string(),
                HashMap::new(),
                None,
                Some("series".to_string()),
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
            .expect("automatic search should resume after health recovery");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn managed_prowlarr_movie_caps_send_only_advertised_ids() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "imdbid", "genre"], &["q", "season", "ep", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["12.Lanterns.of.Winter.2013"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let response = multi
            .search(
                "12 Lanterns of Winter".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12004567".to_string()),
                    ("tmdb_id".to_string(), "120045".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
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

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([("imdb_id".to_string(), "tt12004567".to_string())])
        );
    }

    #[tokio::test]
    async fn direct_newznab_without_caps_snapshot_uses_static_caps_ids_only() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["12.Lanterns.of.Winter.2013"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let response = multi
            .search(
                "12 Lanterns of Winter".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12004567".to_string()),
                    ("tmdb_id".to_string(), "120045".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
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

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([("imdb_id".to_string(), "tt12004567".to_string())])
        );
    }

    #[tokio::test]
    async fn direct_newznab_caps_snapshot_can_widen_ids_when_live_caps_allow_it() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.caps_snapshot_json = Some(
            serde_json::to_string(&prowlarr_caps_snapshot(
                &["q", "tmdbid", "imdbid"],
                &["q", "season", "ep", "tvdbid"],
            ))
            .expect("serialize direct caps snapshot"),
        );

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Solar.Divide.Part.Two.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let response = multi
            .search(
                "Solar Divide Part Two".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt22006789".to_string()),
                    ("tmdb_id".to_string(), "220067".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
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

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([
                ("imdb_id".to_string(), "tt22006789".to_string()),
                ("tmdb_id".to_string(), "220067".to_string()),
            ])
        );
    }

    #[test]
    fn direct_nab_caps_preserve_provider_native_ids_and_structured_inputs() {
        let mut config = mock_indexer_config();
        config.provider_type = "animetosho-xyz".into();
        config.caps_snapshot_json = Some(
            serde_json::to_string(&prowlarr_caps_snapshot(&["q"], &["q"]))
                .expect("serialize direct caps snapshot"),
        );
        let mut static_caps = anime_caps();
        static_caps
            .supported_ids
            .insert("anime".into(), vec!["anidb_id".into(), "tvdb_id".into()]);
        static_caps.search_inputs = vec![
            IndexerSearchInputCapability::TitleQuery,
            IndexerSearchInputCapability::IdQuery,
            IndexerSearchInputCapability::AggregateIdQuery,
            IndexerSearchInputCapability::Season,
            IndexerSearchInputCapability::Episode,
            IndexerSearchInputCapability::AbsoluteEpisode,
        ];

        let resolved = MultiIndexerSearchClient::resolve_search_capabilities(
            &config,
            &static_caps,
            "anime",
            "anime",
        );

        assert_eq!(resolved.id_dispatch_mode, IdDispatchMode::Aggregate);
        assert_eq!(
            resolved.caps.supported_ids.get("anime"),
            Some(&vec!["anidb_id".to_string()])
        );
        assert_eq!(resolved.caps.season_param.as_deref(), Some("season"));
        assert_eq!(resolved.caps.episode_param.as_deref(), Some("ep"));
        assert!(
            resolved
                .caps
                .search_inputs
                .contains(&IndexerSearchInputCapability::AbsoluteEpisode)
        );

        let ids = HashMap::from([("anidb_id".to_string(), "1535".to_string())]);
        let strategies = build_strategies(&StrategyParams {
            query: "Synthetic Animation S02E03",
            query_facet: "anime",
            id_facet: "anime",
            ids: &ids,
            season: Some(2),
            episode: Some(3),
            absolute_episode: Some(21),
            caps: &resolved.caps,
            id_dispatch_mode: resolved.id_dispatch_mode,
            text_dispatch_mode: resolved.text_dispatch_mode,
            is_alias_query: false,
            facet_omitted: false,
        });
        assert!(strategies.iter().any(|strategy| {
            strategy.label == "ids_sxex"
                && strategy.ids == ids
                && strategy.season == Some(2)
                && strategy.episode == Some(3)
        }));
        assert!(strategies.iter().any(|strategy| {
            strategy.label == "ids_abs"
                && strategy.ids == ids
                && strategy.absolute_episode == Some(21)
        }));
    }

    #[test]
    fn direct_amenzb_caps_preserve_native_anidb_and_hash_ids() {
        let mut config = mock_indexer_config();
        config.provider_type = "amenzb".into();
        config.caps_snapshot_json = Some(
            serde_json::to_string(&prowlarr_caps_snapshot(&["q"], &["q"]))
                .expect("serialize direct caps snapshot"),
        );
        let mut static_caps = anime_caps();
        static_caps.supported_ids.insert(
            "anime".into(),
            vec![
                "anidb_id".into(),
                "anidb".into(),
                "tvdb_id".into(),
                "info_hash".into(),
                "info_hash_v1".into(),
                "btih".into(),
            ],
        );

        let resolved = MultiIndexerSearchClient::resolve_search_capabilities(
            &config,
            &static_caps,
            "anime",
            "anime",
        );

        assert_eq!(resolved.id_dispatch_mode, IdDispatchMode::Aggregate);
        assert_eq!(
            resolved.caps.supported_ids.get("anime"),
            Some(&vec![
                "anidb_id".to_string(),
                "anidb".to_string(),
                "info_hash".to_string(),
                "info_hash_v1".to_string(),
                "btih".to_string(),
            ])
        );
    }

    #[test]
    fn managed_nab_caps_do_not_preserve_provider_native_ids() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q"], &["q"]),
        )));
        let mut static_caps = anime_caps();
        static_caps.search_inputs = vec![
            IndexerSearchInputCapability::IdQuery,
            IndexerSearchInputCapability::Season,
            IndexerSearchInputCapability::Episode,
        ];

        let resolved = MultiIndexerSearchClient::resolve_search_capabilities(
            &config,
            &static_caps,
            "anime",
            "anime",
        );

        assert_eq!(resolved.id_dispatch_mode, IdDispatchMode::QueryOnly);
        assert!(resolved.caps.supported_ids.is_empty());
        assert_eq!(resolved.caps.season_param, None);
        assert_eq!(resolved.caps.episode_param, None);
    }

    #[tokio::test]
    async fn managed_prowlarr_caps_snapshot_can_aggregate_supported_ids() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "tmdbid", "imdbid"], &["q", "season", "ep", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Solar.Divide.Part.Two.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let response = multi
            .search(
                "Solar Divide Part Two".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt22006789".to_string()),
                    ("tmdb_id".to_string(), "220067".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
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

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([
                ("imdb_id".to_string(), "tt22006789".to_string()),
                ("tmdb_id".to_string(), "220067".to_string()),
            ])
        );
    }

    #[tokio::test]
    async fn managed_prowlarr_series_caps_drop_unadvertised_ids() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "imdbid"], &["q", "season", "ep", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Storm.Signal.S01E02.2026"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: series_caps(),
            }),
        );

        let response = multi
            .search(
                "Storm Signal".to_string(),
                HashMap::from([
                    ("tvdb_id".to_string(), "424242".to_string()),
                    ("imdb_id".to_string(), "tt42424242".to_string()),
                    ("tmdb_id".to_string(), "424242".to_string()),
                ]),
                Some("series".to_string()),
                Some("series".to_string()),
                None,
                Some(vec!["5000".to_string()]),
                None,
                SearchMode::Interactive,
                Some(1),
                Some(2),
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([("tvdb_id".to_string(), "424242".to_string())])
        );
        assert_eq!(recorded[0].season, Some(1));
        assert_eq!(recorded[0].episode, Some(2));
    }

    #[tokio::test]
    async fn managed_prowlarr_without_caps_snapshot_falls_back_to_query_only() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(None));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["12.Lanterns.of.Winter.2013"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "12 Lanterns of Winter".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12004567".to_string()),
                    ("tmdb_id".to_string(), "120045".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
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

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].ids.is_empty());
        assert_eq!(recorded[0].query, "12 Lanterns of Winter");
        assert_eq!(recorded[0].facet, None);
        assert!(recorded[0].categories.is_empty());
        assert_eq!(recorded[0].season, None);
        assert_eq!(recorded[0].episode, None);
        assert_eq!(recorded[0].absolute_episode, None);
    }

    #[tokio::test]
    async fn managed_prowlarr_prefers_supplied_newznab_categories_over_facet_defaults() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "imdbid"], &["q", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Ember.Saga.Iron.Rail.2020"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "Ember Saga Iron Rail 2020".to_string(),
                HashMap::from([("imdb_id".to_string(), "tt11032374".to_string())]),
                Some("anime".to_string()),
                Some("movie".to_string()),
                Some("movie".to_string()),
                Some(vec!["5070".to_string(), "2000".to_string()]),
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].categories,
            vec!["5070".to_string(), "2000".to_string()]
        );
    }

    #[tokio::test]
    async fn managed_prowlarr_caps_with_unavailable_movie_search_fall_back_to_generic_query() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot_with_availability(
                false,
                &["q", "imdbid", "tmdbid"],
                true,
                &["q", "season", "ep", "tvdbid"],
            ),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["12.Lanterns.of.Winter.2013"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "12 Lanterns of Winter".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12004567".to_string()),
                    ("tmdb_id".to_string(), "120045".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
                Some(vec!["2000".to_string()]),
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].ids.is_empty());
        assert_eq!(recorded[0].query, "12 Lanterns of Winter");
        assert_eq!(recorded[0].category, None);
        assert_eq!(recorded[0].facet, None);
        assert!(recorded[0].categories.is_empty());
    }

    #[tokio::test]
    async fn id_free_text_capable_movie_provider_receives_freetext() {
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Nebula.Circuit.0.2021.1080p"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: IndexerProviderCapabilities {
                    rss: false,
                    supported_ids: HashMap::new(),
                    query_param: Some("q".into()),
                    supported_query_facets: vec!["movie".into()],
                    search: true,
                    ..Default::default()
                },
            }),
        );

        let response = multi
            .search(
                "NEBULA CIRCUIT 0".to_string(),
                HashMap::from([("imdb_id".to_string(), "tt14331144".to_string())]),
                Some("movie".to_string()),
                Some("movie".to_string()),
                None,
                Some(vec!["2000".to_string()]),
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("movie freetext search should dispatch");

        assert_eq!(response.results.len(), 1);
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].ids.is_empty());
        assert_eq!(recorded[0].query, "NEBULA CIRCUIT 0");
        assert_eq!(recorded[0].category.as_deref(), Some("movie"));
        assert_eq!(recorded[0].facet.as_deref(), Some("movie"));
        assert_eq!(recorded[0].categories, vec!["2000".to_string()]);
    }

    #[tokio::test]
    async fn legacy_anime_id_provider_does_not_receive_movie_freetext() {
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Unexpected.Movie.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: anime_caps(),
            }),
        );

        let response = multi
            .search(
                "NEBULA CIRCUIT 0".to_string(),
                HashMap::from([("imdb_id".to_string(), "tt14331144".to_string())]),
                Some("movie".to_string()),
                Some("movie".to_string()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("unsupported facet should skip provider");

        assert!(response.results.is_empty());
        assert!(calls.lock().expect("calls").is_empty());
    }

    #[tokio::test]
    async fn generic_nab_query_only_fallback_strips_structured_context() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot_with_availability(false, &["q"], false, &["q"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Harbor.Tempest.09.1080p"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: anime_caps(),
            }),
        );

        let _response = multi
            .search(
                "Harbor Tempest 09".to_string(),
                HashMap::from([("anidb_id".to_string(), "1234".to_string())]),
                Some("anime".to_string()),
                Some("anime".to_string()),
                None,
                Some(vec!["5070".to_string()]),
                None,
                SearchMode::Interactive,
                Some(1),
                Some(9),
                Some(9),
                vec![],
            )
            .await
            .expect("generic fallback should search");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].ids.is_empty());
        assert_eq!(recorded[0].category, None);
        assert_eq!(recorded[0].facet, None);
        assert!(recorded[0].categories.is_empty());
        assert_eq!(recorded[0].season, None);
        assert_eq!(recorded[0].episode, None);
        assert_eq!(recorded[0].absolute_episode, None);
    }

    #[test]
    fn the_rss_form_uses_the_nab_function_only_where_the_caps_advertise_it() {
        let facet_capable = IndexerProviderCapabilities {
            rss: true,
            query_param: Some("q".to_string()),
            supported_query_facets: vec!["series".to_string()],
            ..Default::default()
        };
        assert_eq!(
            rss_request_form(
                &facet_capable,
                TextDispatchMode::FacetScoped,
                "series",
                false
            ),
            RssRequestForm::Nab
        );
        // An observed function-unavailable answer outranks the advertised caps.
        assert_eq!(
            rss_request_form(
                &facet_capable,
                TextDispatchMode::FacetScoped,
                "series",
                true
            ),
            RssRequestForm::BareQuery
        );

        let generic_only = IndexerProviderCapabilities {
            rss: true,
            query_param: Some("q".to_string()),
            ..Default::default()
        };
        assert_eq!(
            rss_request_form(
                &generic_only,
                TextDispatchMode::GenericOnly,
                "series",
                false
            ),
            RssRequestForm::BareQuery
        );
        assert_eq!(
            rss_request_form(
                &IndexerProviderCapabilities {
                    rss: true,
                    ..Default::default()
                },
                TextDispatchMode::None,
                "series",
                false
            ),
            RssRequestForm::BareQuery
        );
    }

    #[test]
    fn a_function_unavailable_answer_is_read_as_a_wrong_request_form() {
        assert!(newznab_function_is_unavailable(&AppError::Repository(
            "Newznab API error 203: Function not available".to_string()
        )));
        assert!(newznab_function_is_unavailable(&AppError::Repository(
            "Newznab API error 202: No such function".to_string()
        )));
        assert!(!newznab_function_is_unavailable(&AppError::Repository(
            "upstream status 503".to_string()
        )));
    }

    /// The sweep is a "latest releases" question, so an endpoint that does not
    /// implement the facet-scoped function must be asked the bare-query way
    /// rather than dropped from RSS.
    #[tokio::test]
    async fn an_rss_sweep_falls_back_to_the_bare_query_form_after_a_function_unavailable_answer() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "imdbid"], &["q", "season", "ep", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|call| {
                if call.facet.is_some() {
                    Err(AppError::Repository(
                        "Newznab API error 203: Function not available".to_string(),
                    ))
                } else {
                    response_with_titles(&["Quiet.Meridian.S13E01.1080p.WEB-DL"])
                }
            }),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: IndexerProviderCapabilities {
                    rss: true,
                    supported_ids: HashMap::from([("series".into(), vec!["tvdb_id".into()])]),
                    season_param: Some("season".into()),
                    episode_param: Some("ep".into()),
                    query_param: Some("q".into()),
                    search: true,
                    tvdb_search: true,
                    ..Default::default()
                },
            }),
        );

        let rss_sweep = || {
            multi.search(
                String::new(),
                HashMap::new(),
                None,
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
        };

        let _refused = rss_sweep().await;
        let _accepted = rss_sweep().await.expect("bare-query sweep should answer");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 2, "{recorded:?}");
        assert_eq!(recorded[0].facet.as_deref(), Some("series"), "{recorded:?}");
        assert_eq!(recorded[1].facet, None, "{recorded:?}");
        // Only the facet-scoped function is given up; the sweep keeps sweeping
        // the categories it was routed.
        assert_eq!(
            recorded[1].categories, recorded[0].categories,
            "{recorded:?}"
        );
    }

    #[tokio::test]
    async fn live_caps_basic_query_fallback_strips_facet_params_when_tvsearch_lacks_q() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "imdbid"], &["season", "ep", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|call| {
                if call.ids.is_empty() {
                    response_with_titles(&["Storm.Signal.S01E02.2026"])
                } else {
                    Ok(IndexerSearchResponse {
                        completion: IndexerSearchCompletion::Complete,
                        indexer_outcomes: Vec::new(),
                        results: vec![],

                        api_current: None,
                        api_max: None,
                        grab_current: None,
                        grab_max: None,
                    })
                }
            }),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "Storm Signal".to_string(),
                HashMap::from([("tvdb_id".to_string(), "424242".to_string())]),
                Some("series".to_string()),
                Some("series".to_string()),
                None,
                Some(vec!["5000".to_string()]),
                None,
                SearchMode::Interactive,
                Some(1),
                Some(2),
                None,
                vec![],
            )
            .await
            .expect("search should succeed");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 2);

        assert_eq!(recorded[0].ids.get("tvdb_id"), Some(&"424242".to_string()));
        assert_eq!(recorded[0].facet, Some("series".to_string()));
        assert_eq!(recorded[0].categories, vec!["5000".to_string()]);
        assert_eq!(recorded[0].season, Some(1));
        assert_eq!(recorded[0].episode, Some(2));

        assert!(recorded[1].ids.is_empty());
        assert_eq!(recorded[1].query, "Storm Signal");
        assert_eq!(recorded[1].facet, None);
        assert!(recorded[1].categories.is_empty());
        assert_eq!(recorded[1].season, None);
        assert_eq!(recorded[1].episode, None);
    }

    #[tokio::test]
    async fn facet_scoped_text_dispatch_preserves_advertised_anime_context() {
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Harbor.Tempest.09.1080p"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: IndexerProviderCapabilities {
                    rss: false,
                    supported_ids: HashMap::new(),
                    query_param: Some("q".into()),
                    supported_query_facets: vec!["anime".into()],
                    search_inputs: vec![
                        scryer_domain::IndexerSearchInputCapability::TitleQuery,
                        scryer_domain::IndexerSearchInputCapability::Category,
                        scryer_domain::IndexerSearchInputCapability::Season,
                        scryer_domain::IndexerSearchInputCapability::Episode,
                        scryer_domain::IndexerSearchInputCapability::AbsoluteEpisode,
                    ],
                    search: true,
                    ..Default::default()
                },
            }),
        );

        let _response = multi
            .search(
                "Harbor Tempest 09".to_string(),
                HashMap::from([("anidb_id".to_string(), "1234".to_string())]),
                Some("anime".to_string()),
                Some("anime".to_string()),
                None,
                Some(vec!["5070".to_string()]),
                None,
                SearchMode::Interactive,
                Some(1),
                Some(9),
                Some(9),
                vec![],
            )
            .await
            .expect("facet-scoped text search should dispatch");

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].ids.is_empty());
        assert_eq!(recorded[0].category.as_deref(), Some("anime"));
        assert_eq!(recorded[0].facet.as_deref(), Some("anime"));
        assert_eq!(recorded[0].categories, vec!["5070".to_string()]);
        assert_eq!(recorded[0].season, Some(1));
        assert_eq!(recorded[0].episode, Some(9));
        assert_eq!(recorded[0].absolute_episode, Some(9));
    }

    #[tokio::test]
    async fn managed_prowlarr_children_fall_back_to_default_categories_when_routing_is_empty() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();
        config.managed_parent_config_id = Some("parent".into());
        config.managed_metadata_json = Some(managed_metadata_with_caps(Some(
            prowlarr_caps_snapshot(&["q", "imdbid"], &["q", "season", "ep", "tvdbid"]),
        )));

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Category.Fallback.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "Category Fallback".to_string(),
                HashMap::from([("imdb_id".to_string(), "tt12345678".to_string())]),
                None,
                Some("movie".to_string()),
                None,
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

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].categories, vec!["2000".to_string()]);
    }

    #[tokio::test]
    async fn direct_newznab_searches_stay_uncategorized_when_routing_is_empty() {
        let mut config = mock_indexer_config();
        config.provider_type = "newznab".into();

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Category.Fallback.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "Category Fallback".to_string(),
                HashMap::from([("imdb_id".to_string(), "tt12345678".to_string())]),
                None,
                Some("movie".to_string()),
                None,
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

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].categories.is_empty());
    }

    #[tokio::test]
    async fn non_nab_managed_configs_do_not_inherit_prowlarr_proxy_behavior() {
        let mut config = mock_indexer_config();
        config.provider_type = "mock".into();
        config.managed_parent_config_id = Some("parent".into());

        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|_| response_with_titles(&["Proxy.Safe.Result.2024"])),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![config],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let _response = multi
            .search(
                "Proxy Safe Result".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12345678".to_string()),
                    ("tmdb_id".to_string(), "123456".to_string()),
                ]),
                None,
                Some("movie".to_string()),
                None,
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

        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].ids,
            HashMap::from([("imdb_id".to_string(), "tt12345678".to_string())])
        );
        assert_eq!(recorded[0].facet.as_deref(), Some("movie"));
        assert!(recorded[0].categories.is_empty());
    }

    #[tokio::test]
    async fn id_backed_movie_results_skip_freetext_title_guard() {
        let calls = StdArc::new(StdMutex::new(Vec::new()));
        let client = Arc::new(ScriptedIndexerClient {
            calls: calls.clone(),
            responder: StdArc::new(|call| {
                if call.ids.is_empty() {
                    response_with_titles(&["Should.Not.Fallback.2024"])
                } else {
                    response_with_titles(&["Completely.Different.Title.2024.1080p.BluRay"])
                }
            }),
        });
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(ScriptedIndexerPluginProvider {
                client,
                caps: movie_caps(),
            }),
        );

        let response = multi
            .search(
                "Expected Movie 2024".to_string(),
                HashMap::from([
                    ("imdb_id".to_string(), "tt12345678".to_string()),
                    ("tmdb_id".to_string(), "123456".to_string()),
                    ("tvdb_id".to_string(), "98765".to_string()),
                    ("anidb_id".to_string(), "54321".to_string()),
                    ("mal_id".to_string(), "67890".to_string()),
                ]),
                Some("movie".to_string()),
                Some("movie".to_string()),
                Some("movie".to_string()),
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

        assert_eq!(response.results.len(), 1);
        assert_eq!(
            response.results[0].title,
            "Completely.Different.Title.2024.1080p.BluRay"
        );
        let recorded = calls.lock().expect("calls").clone();
        assert_eq!(recorded.len(), 1, "ID results should suppress fallback");
        assert_eq!(
            recorded[0].ids,
            HashMap::from([("imdb_id".to_string(), "tt12345678".to_string())])
        );
    }

    #[tokio::test]
    async fn rss_sync_search_with_newznab_categories_still_uses_rss_mode() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: true,
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
                Some(vec!["2000".into(), "5030".into()]),
                None,
                SearchMode::Auto,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("rss sync search with categories should succeed");

        assert!(response.results.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn rss_sync_search_runs_each_newznab_category_in_a_separate_request() {
        let mut caps = movie_caps();
        caps.rss = true;
        let (client, calls) =
            scripted_search_client(caps, |call| match call.categories.as_slice() {
                [category] if category == "2000" => {
                    response_with_titles(&["Movies.Release.2000.1080p.WEB-DL"])
                }
                [category] if category == "5030" => {
                    response_with_titles(&["Series.Release.5030.720p.WEB-DL"])
                }
                other => Err(AppError::Validation(format!(
                    "unexpected rss categories: {:?}",
                    other
                ))),
            });
        let scheduler = Arc::new(RecordingScheduler::default());
        let recorded_candidates = scheduler.candidate_ids.clone();
        let recorded_feedback = scheduler.feedback_candidate_ids.clone();
        let client = client.with_upstream_scheduler(scheduler);

        let response = client
            .search(
                String::new(),
                HashMap::new(),
                None,
                None,
                None,
                Some(vec!["2000".into(), "5030".into()]),
                None,
                SearchMode::Auto,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("rss sync search should fan out per category");

        let calls = calls.lock().expect("call log mutex");
        let mut categories: Vec<Vec<String>> =
            calls.iter().map(|call| call.categories.clone()).collect();
        categories.sort();

        assert_eq!(
            categories,
            vec![vec!["2000".to_string()], vec!["5030".to_string()]]
        );
        let scheduler_batches = recorded_candidates.lock().expect("scheduler candidates");
        assert_eq!(scheduler_batches.len(), 1);
        assert_eq!(
            scheduler_batches[0],
            vec!["idx-1".to_string(), "idx-1".to_string()]
        );
        let feedback_candidate_ids = recorded_feedback.lock().expect("scheduler feedback");
        assert_eq!(feedback_candidate_ids.len(), 2);
        assert_eq!(response.results.len(), 2);
    }

    #[tokio::test]
    async fn series_search_with_tvdb_id_skips_freetext_when_id_tier_returns_results() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                response_with_titles(&["Signal.Run.S01E12.720p.WEB-DL"])
            } else {
                response_with_titles(&["Signal.Road.S01E12.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Signal Run S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
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
        assert_eq!(response.results[0].title, "Signal.Run.S01E12.720p.WEB-DL");
    }

    #[tokio::test]
    async fn series_search_with_tvdb_id_falls_back_only_after_empty_id_tier() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["Signal.Run.S01E12.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Signal Run S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
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
        assert_eq!(calls[1].query, "Signal Run S01E12");
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].title, "Signal.Run.S01E12.720p.WEB-DL");
    }

    #[tokio::test]
    async fn id_empty_then_fallback_still_rejects_false_positive_titles() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&[
                    "Signal.Run.S01E12.720p.WEB-DL",
                    "Signal.Road.2021.S01E12.2160p.WEB-DL",
                ])
            }
        });

        let response = client
            .search(
                "Signal Run S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
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
        assert_eq!(response.results[0].title, "Signal.Run.S01E12.720p.WEB-DL");
    }

    #[tokio::test]
    async fn movie_search_with_imdb_id_uses_tiered_fallback() {
        let (client, calls) = scripted_search_client(movie_caps(), |call| {
            if call.ids.contains_key("imdb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["Lattice.Zero.1999.1080p.BluRay"])
            }
        });

        let response = client
            .search(
                "Lattice Zero".into(),
                HashMap::from([("imdb_id".to_string(), "tt0133093".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
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
        assert_eq!(calls[1].query, "Lattice Zero");
        assert_eq!(response.results[0].title, "Lattice.Zero.1999.1080p.BluRay");
    }

    #[test]
    fn series_movie_anime_lane_builds_movie_id_strategy() {
        let caps = movie_caps();
        let ids = HashMap::from([("imdb_id".to_string(), "tt11032374".to_string())]);
        let strategies = build_strategies(&StrategyParams {
            query: "Iron Rail 2020",
            query_facet: "anime",
            id_facet: "movie",
            ids: &ids,
            season: None,
            episode: None,
            absolute_episode: None,
            caps: &caps,
            id_dispatch_mode: IdDispatchMode::LegacyAggregate,
            text_dispatch_mode: TextDispatchMode::None,
            is_alias_query: false,
            facet_omitted: false,
        });

        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies[0].label, "ids");
        assert_eq!(strategies[0].request_facet, "movie");
        assert!(strategies[0].ids.contains_key("imdb_id"));
    }

    #[test]
    fn legacy_aggregate_filters_outbound_ids_to_caps() {
        let caps = IndexerProviderCapabilities {
            supported_ids: HashMap::from([("series".into(), vec!["tvdb_id".into()])]),
            search: true,
            tvdb_search: true,
            imdb_search: false,
            ..Default::default()
        };
        let ids = HashMap::from([
            ("imdb_id".to_string(), "tt11032374".to_string()),
            ("tvdb_id".to_string(), "424536".to_string()),
        ]);

        let strategies = build_strategies(&StrategyParams {
            query: "Signal Run S01E12",
            query_facet: "series",
            id_facet: "series",
            ids: &ids,
            season: Some(1),
            episode: Some(12),
            absolute_episode: None,
            caps: &caps,
            id_dispatch_mode: IdDispatchMode::LegacyAggregate,
            text_dispatch_mode: TextDispatchMode::None,
            is_alias_query: false,
            facet_omitted: false,
        });

        assert_eq!(strategies.len(), 1);
        assert_eq!(
            strategies[0].ids,
            HashMap::from([("tvdb_id".to_string(), "424536".to_string())])
        );
    }

    #[tokio::test]
    async fn interactive_search_errors_when_every_strategy_fails() {
        let (client, _calls) = scripted_search_client(movie_caps(), |_call| {
            Err(AppError::Repository("forced indexer failure".into()))
        });

        let error = client
            .search(
                "Iron Rail 2020".into(),
                HashMap::from([("imdb_id".to_string(), "tt11032374".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect_err("interactive search should report all-failed attempts");

        assert!(
            error
                .to_string()
                .contains("all attempted indexer strategies failed"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn interactive_search_all_strategies_failed_preserves_rate_limit_signal() {
        let retry_after = std::time::Duration::from_secs(42);
        let (client, _calls) = scripted_search_client(movie_caps(), move |_call| {
            Err(AppError::TemporaryUnavailable {
                message: "upstream returned 429".to_string(),
                retry_after: Some(retry_after),
                rate_limit_cooldown: RateLimitCooldownAction::AlreadyRecorded,
            })
        });

        let error = client
            .search(
                "Iron Rail 2020".into(),
                HashMap::from([("imdb_id".to_string(), "tt11032374".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect_err("interactive search should report all-failed attempts");

        let rendered = error.to_string();
        assert!(
            rendered.contains("all attempted indexer strategies failed"),
            "unexpected error: {rendered}"
        );
        // The upstream rate-limit text travels with the aggregate so a reader
        // (the interactive per-indexer status, a log line) sees why it failed.
        assert!(
            rendered.contains("upstream returned 429"),
            "rate-limit detail should be carried: {rendered}"
        );
        let signal = scryer_application::RateLimitSignal::from_error(&error)
            .expect("aggregated failure should preserve rate-limit signal");
        assert_eq!(signal.retry_after, Some(retry_after));
        assert_eq!(
            signal.cooldown_action,
            RateLimitCooldownAction::AlreadyRecorded
        );
    }

    #[tokio::test]
    async fn movie_query_backed_id_search_keeps_synthetic_numeric_title_match() {
        let (client, calls) = scripted_search_client(movie_caps(), |_call| {
            response_with_titles(&["12.Lanterns.of.Winter.2013.1080p.BluRay.x264-GROUP"])
        });

        let response = client
            .search(
                "12 Lanterns of Winter".into(),
                HashMap::from([("imdb_id".to_string(), "tt12004567".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
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
        assert_eq!(calls.len(), 1);
        assert!(calls[0].ids.contains_key("imdb_id"));
        assert_eq!(response.results.len(), 1);
        assert_eq!(
            response.results[0].title,
            "12.Lanterns.of.Winter.2013.1080p.BluRay.x264-GROUP"
        );
    }

    #[tokio::test]
    async fn anime_search_keeps_id_variants_in_primary_tier_and_falls_back_after_empty_results() {
        let (client, calls) = scripted_search_client(anime_caps(), |call| {
            if call.ids.contains_key("anidb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["Blade.Summit.S02E03.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Blade Summit S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
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
        assert_eq!(calls[2].query, "Blade Summit S02E03");
        assert_eq!(response.results[0].title, "Blade.Summit.S02E03.720p.WEB-DL");
    }

    #[tokio::test]
    async fn id_tier_errors_trigger_title_fallback() {
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                Err(AppError::Repository("boom".into()))
            } else {
                response_with_titles(&["Signal.Run.S01E12.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Signal Run S01E12".into(),
                HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("ID-tier errors should fall back to freetext search");

        let calls = calls.lock().expect("call log mutex");
        assert_eq!(calls.len(), 2);
        assert!(calls[0].ids.contains_key("tvdb_id"));
        assert!(calls[0].query.is_empty());
        assert!(calls[1].ids.is_empty());
        assert_eq!(calls[1].query, "Signal Run S01E12");
        assert_eq!(response.results[0].title, "Signal.Run.S01E12.720p.WEB-DL");
    }

    #[tokio::test]
    async fn mixed_primary_outcomes_trigger_fallback_when_no_primary_results_are_usable() {
        let (client, calls) = scripted_search_client(anime_caps(), |call| {
            if call.ids.contains_key("anidb_id") && call.absolute_episode.is_some() {
                Err(AppError::Repository("abs lookup failed".into()))
            } else if call.ids.contains_key("anidb_id") {
                response_with_titles(&[])
            } else {
                response_with_titles(&["Ember.Saga.S02E03.720p.WEB-DL"])
            }
        });

        let response = client
            .search(
                "Blade Summit S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
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
        assert_eq!(calls.len(), 3);
        assert!(calls[0].ids.contains_key("anidb_id"));
        assert!(calls[1].ids.contains_key("anidb_id"));
        assert!(calls[0].query.is_empty());
        assert!(calls[1].query.is_empty());
        assert!(calls[2].ids.is_empty());
        assert_eq!(calls[2].query, "Blade Summit S02E03");
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
                    response_with_titles(&["Blade.Summit.S02E03.720p.WEB-DL"])
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
                "Blade Summit S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
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
            assert_eq!(calls.len(), 3);
            assert!(calls[2].ids.is_empty());
            assert_eq!(calls[2].query, "Blade Summit S02E03");
            assert_eq!(response.results[0].title, "Blade.Summit.S02E03.720p.WEB-DL");
        }
        assert!(client.backoff_tracker.is_disabled("idx-1").await.is_none());
        let state = backoff_state(&client, "idx-1")
            .await
            .expect("success should preserve a cleared backoff entry");
        assert_eq!(state.escalation_level, 0);
        assert!(state.disabled_until.is_none());

        let stats = stats.queries.lock().expect("stats log mutex");
        assert_eq!(stats.len(), 3);
        assert_eq!(stats.iter().filter(|success| **success).count(), 2);
        assert_eq!(stats.iter().filter(|success| !**success).count(), 1);
    }

    #[tokio::test]
    async fn all_primary_request_failures_fall_back_before_backoff() {
        let stats = Arc::new(RecordingIndexerStatsTracker::default());
        let (client, calls) =
            scripted_search_client_with_stats(anime_caps(), stats.clone(), |call| {
                if call.ids.contains_key("anidb_id") {
                    Err(AppError::Repository("lookup failed".into()))
                } else {
                    response_with_titles(&["Blade.Summit.S02E03.720p.WEB-DL"])
                }
            });

        let response = client
            .search(
                "Blade Summit S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(2),
                Some(3),
                Some(21),
                vec![],
            )
            .await
            .expect("all-failure primary outcomes should fall back to freetext");

        {
            let calls = calls.lock().expect("call log mutex");
            assert_eq!(calls.len(), 3);
            assert!(calls[0].ids.contains_key("anidb_id"));
            assert!(calls[1].ids.contains_key("anidb_id"));
            assert!(calls[2].ids.is_empty());
            assert_eq!(calls[2].query, "Blade Summit S02E03");
        }
        assert_eq!(response.results[0].title, "Blade.Summit.S02E03.720p.WEB-DL");
        assert!(client.backoff_tracker.is_disabled("idx-1").await.is_none());

        assert!(
            backoff_state(&client, "idx-1").await.is_none(),
            "fallback success should not create a new backoff entry"
        );

        let stats = stats.queries.lock().expect("stats log mutex");
        assert_eq!(stats.len(), 3);
        assert_eq!(stats.iter().filter(|success| **success).count(), 1);
        assert_eq!(stats.iter().filter(|success| !**success).count(), 2);
    }

    #[tokio::test]
    async fn solver_service_failure_does_not_create_operational_backoff() {
        let stats = Arc::new(RecordingIndexerStatsTracker::default());
        let (client, _calls) =
            scripted_search_client_with_stats(anime_caps(), stats.clone(), |_| {
                Err(AppError::Repository(format!(
                    "indexer request failed: {}",
                    scryer_application::challenge_solver::BYPARR_UNREACHABLE_MESSAGE
                )))
            });

        let _ = client
            .search(
                "Blade Summit S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(2),
                Some(3),
                Some(21),
                vec![],
            )
            .await;

        assert!(
            client.backoff_tracker.is_disabled("idx-1").await.is_none(),
            "solver-service failures must not disable the indexer"
        );
        assert!(
            backoff_state(&client, "idx-1").await.is_none(),
            "solver-service failures must not escalate indexer operational backoff"
        );
    }

    #[tokio::test]
    async fn non_solver_failures_still_create_operational_backoff() {
        let stats = Arc::new(RecordingIndexerStatsTracker::default());
        let (client, _calls) =
            scripted_search_client_with_stats(anime_caps(), stats.clone(), |_| {
                Err(AppError::Repository("origin exploded".into()))
            });

        let _ = client
            .search(
                "Blade Summit S02E03".into(),
                HashMap::from([("anidb_id".to_string(), "1535".to_string())]),
                Some("anime".into()),
                Some("anime".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(2),
                Some(3),
                Some(21),
                vec![],
            )
            .await;

        assert!(
            backoff_state(&client, "idx-1").await.is_some(),
            "plain provider failures must still escalate operational backoff"
        );
    }

    #[tokio::test]
    async fn clear_forgets_escalation_and_disabled_window() {
        let tracker = IndexerBackoffTracker::new();
        tracker.state.lock().await.insert(
            "idx-1".to_string(),
            IndexerBackoffState {
                escalation_level: 3,
                disabled_until: Some(chrono::Utc::now() + chrono::Duration::minutes(45)),
            },
        );
        assert!(tracker.is_disabled("idx-1").await.is_some());

        assert!(tracker.clear("idx-1").await, "state existed to drop");
        assert!(tracker.is_disabled("idx-1").await.is_none());
        assert!(!tracker.clear("idx-1").await, "nothing left to drop");

        // The next failure starts from level zero, not from where it left off.
        let backoff = tracker.record_failure("idx-1", None).await;
        assert_eq!(backoff.escalation_level, 1);
    }

    #[tokio::test]
    async fn record_failure_does_not_extend_active_backoff() {
        let tracker = IndexerBackoffTracker::new();
        let disabled_until = chrono::Utc::now() + chrono::Duration::minutes(45);

        tracker.state.lock().await.insert(
            "idx-1".to_string(),
            IndexerBackoffState {
                escalation_level: 3,
                disabled_until: Some(disabled_until),
            },
        );

        let returned = tracker.record_failure("idx-1", None).await;
        assert_eq!(returned.disabled_until, disabled_until);
        assert_eq!(returned.escalation_level, 3);

        let state = tracker
            .state
            .lock()
            .await
            .get("idx-1")
            .cloned()
            .expect("backoff state should remain present");
        assert_eq!(state.escalation_level, 3);
        assert_eq!(state.disabled_until, Some(disabled_until));
    }

    #[test]
    fn retry_after_parser_extracts_seconds_from_plugin_error_text() {
        let retry_after = rate_limit_signal_from_error(&AppError::Repository(
            "HTTP 429: rate limited; retry_after_seconds=900".to_string(),
        ))
        .and_then(|signal| signal.retry_after)
        .expect("retry after should parse");
        assert_eq!(retry_after, std::time::Duration::from_secs(900));
        let prowlarr_retry_after = rate_limit_signal_from_error(&AppError::Repository(
            "Prowlarr rate limited (retry after 120s)".to_string(),
        ))
        .and_then(|signal| signal.retry_after)
        .expect("Prowlarr retry after should parse");
        assert_eq!(prowlarr_retry_after, std::time::Duration::from_secs(120));
        assert!(
            rate_limit_signal_from_error(&AppError::Repository(
                "HTTP 429: rate limited".to_string()
            ))
            .and_then(|signal| signal.retry_after)
            .is_none()
        );
    }

    #[test]
    fn rate_limit_classifier_does_not_match_bare_429_substrings() {
        assert!(
            rate_limit_signal_from_error(&AppError::Repository(
                "release title contains 429 but no throttle signal".to_string()
            ))
            .is_none()
        );
        assert!(
            rate_limit_signal_from_error(&AppError::Repository(
                "HTTP 429: too many requests".to_string()
            ))
            .is_some()
        );
    }

    #[tokio::test]
    async fn record_failure_uses_retry_after_override() {
        let tracker = IndexerBackoffTracker::new();
        let before = chrono::Utc::now();
        let backoff = tracker
            .record_failure("idx-1", Some(std::time::Duration::from_secs(900)))
            .await;
        let after = chrono::Utc::now();

        assert_eq!(backoff.escalation_level, 1);
        assert!(backoff.disabled_until >= before + chrono::Duration::seconds(900));
        assert!(backoff.disabled_until <= after + chrono::Duration::seconds(901));
    }

    #[tokio::test]
    async fn persisted_backoff_seeds_next_escalation_after_restart() {
        let tracker = IndexerBackoffTracker::new();
        tracker
            .seed_persisted(
                "idx-1",
                &IndexerSystemBackoff {
                    disabled_until: chrono::Utc::now() - chrono::Duration::minutes(1),
                    escalation_level: 3,
                },
            )
            .await;

        let before = chrono::Utc::now();
        let backoff = tracker.record_failure("idx-1", None).await;
        let after = chrono::Utc::now();

        assert_eq!(backoff.escalation_level, 4);
        assert!(backoff.disabled_until >= before + chrono::Duration::minutes(30));
        assert!(backoff.disabled_until <= after + chrono::Duration::minutes(31));
    }

    #[tokio::test]
    async fn exact_title_guard_rejects_false_positive_series_matches_for_freetext_searches() {
        let (client, _calls) = scripted_search_client(series_caps(), |_call| {
            response_with_titles(&[
                "Signal.Run.S01E12.720p.WEB-DL",
                "Signal.Road.2021.S01E12.2160p.WEB-DL",
                "Pals.Like.These.S01E12.720p.WEB-DL",
                "Smiling.Pals.S01E12.1080p.WEB-DL",
            ])
        });

        let signal_run = client
            .search(
                "Signal Run S01E12".into(),
                HashMap::new(),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("signal_run search should succeed");
        assert_eq!(signal_run.results.len(), 1);
        assert_eq!(signal_run.results[0].title, "Signal.Run.S01E12.720p.WEB-DL");

        let pals = client
            .search(
                "Pals S01E12".into(),
                HashMap::new(),
                Some("series".into()),
                Some("series".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                Some(1),
                Some(12),
                None,
                vec![],
            )
            .await
            .expect("pals search should succeed");
        assert!(pals.results.is_empty());
    }

    #[tokio::test]
    async fn ids_only_searches_skip_title_guard() {
        let (client, _calls) = scripted_search_client(movie_caps(), |call| {
            if call.ids.contains_key("imdb_id") {
                response_with_titles(&["Lantern.Tide.Hidden.Current.2001.1080p.BluRay"])
            } else {
                response_with_titles(&[])
            }
        });

        let response = client
            .search(
                String::new(),
                HashMap::from([("imdb_id".to_string(), "tt0245429".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
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
            "Lantern.Tide.Hidden.Current.2001.1080p.BluRay"
        );
    }

    #[tokio::test]
    async fn query_backed_id_searches_skip_title_guard() {
        let (client, _calls) = scripted_search_client(movie_caps(), |call| {
            if call.ids.contains_key("imdb_id") {
                response_with_titles(&[
                    "Lantern.Tide.Hidden.Current.2001.1080p.BluRay",
                    "Lantern.Tide.2001.1080p.BluRay",
                ])
            } else {
                response_with_titles(&[])
            }
        });

        let response = client
            .search(
                "Lantern Tide".into(),
                HashMap::from([("imdb_id".to_string(), "tt0245429".to_string())]),
                Some("movie".into()),
                Some("movie".into()),
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .expect("query-backed ID search should succeed");

        assert_eq!(response.results.len(), 2);
        assert_eq!(
            response.results[0].title,
            "Lantern.Tide.Hidden.Current.2001.1080p.BluRay"
        );
        assert_eq!(response.results[1].title, "Lantern.Tide.2001.1080p.BluRay");
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
            ..Default::default()
        };

        let ids = HashMap::from([("anidb_id".to_string(), "18886".to_string())]);
        let strategies = build_strategies(&StrategyParams {
            query: "Silver Horizon: Beyond Harbor's End S02E05",
            query_facet: "anime",
            id_facet: "anime",
            ids: &ids,
            season: Some(2),
            episode: Some(5),
            absolute_episode: Some(33),
            caps: &caps,
            id_dispatch_mode: IdDispatchMode::LegacyAggregate,
            text_dispatch_mode: TextDispatchMode::FacetScoped,
            is_alias_query: false,
            facet_omitted: false,
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
    fn anime_strategies_strip_absolute_episode_when_not_supported() {
        let caps = IndexerProviderCapabilities {
            rss: false,
            supported_ids: HashMap::from([("anime".into(), vec!["anidb_id".into()])]),
            deduplicates_aliases: false,
            season_param: Some("s".into()),
            episode_param: None,
            query_param: Some("q".into()),
            search_inputs: vec![IndexerSearchInputCapability::TitleQuery],
            search: true,
            imdb_search: false,
            tvdb_search: false,
            anidb_search: true,
            ..Default::default()
        };

        let ids = HashMap::from([("anidb_id".to_string(), "18886".to_string())]);
        let strategies = build_strategies(&StrategyParams {
            query: "Silver Horizon: Beyond Harbor's End S02E05",
            query_facet: "anime",
            id_facet: "anime",
            ids: &ids,
            season: Some(2),
            episode: Some(5),
            absolute_episode: Some(33),
            caps: &caps,
            id_dispatch_mode: IdDispatchMode::Aggregate,
            text_dispatch_mode: TextDispatchMode::FacetScoped,
            is_alias_query: false,
            facet_omitted: false,
        });

        assert_eq!(strategies.len(), 2);
        assert_eq!(strategies[0].label, "ids");
        assert_eq!(strategies[0].absolute_episode, None);
        assert_eq!(strategies[0].episode, None);
        assert_eq!(strategies[1].label, "freetext");
        assert_eq!(strategies[1].absolute_episode, None);
        assert_eq!(strategies[1].episode, None);
    }

    fn strategy_with_label(label: &str) -> SearchStrategy {
        SearchStrategy {
            request_query: "Silver Horizon S02E05".into(),
            request_facet: "anime".into(),
            ids: if label.starts_with("ids") {
                HashMap::from([("anidb_id".to_string(), "18886".to_string())])
            } else {
                HashMap::new()
            },
            season: Some(2),
            episode: Some(5),
            absolute_episode: if label == "ids_abs" { Some(33) } else { None },
            generic_query_only: false,
            omit_request_facet: false,
            label: label.into(),
        }
    }

    fn learning_record(
        strategy_key: &str,
        empty_successes: u32,
        usable_successes: u32,
        suppressed: bool,
        updated_at: Option<String>,
    ) -> IndexerSearchLearningRecord {
        IndexerSearchLearningRecord {
            key: IndexerSearchLearningKey {
                indexer_id: "idx".into(),
                title_id: "title-1".into(),
                facet: "anime".into(),
                strategy_key: strategy_key.into(),
            },
            attempts: empty_successes + usable_successes,
            empty_successes,
            usable_successes,
            last_attempt_at: None,
            last_usable_at: None,
            suppressed,
            updated_at,
        }
    }

    #[tokio::test]
    async fn learned_suppression_skips_suppressed_id_strategy_for_auto() {
        let now = Utc::now();
        let repo: StdArc<dyn IndexerSearchLearningRepository> =
            StdArc::new(InMemorySearchLearningRepository::default());
        let strategies = vec![
            strategy_with_label("ids_abs"),
            strategy_with_label("ids_sxex"),
            strategy_with_label("freetext"),
        ];
        let records = vec![learning_record(
            "v2:ids_abs",
            3,
            0,
            true,
            Some(now.to_rfc3339()),
        )];

        let filtered = suppress_learned_strategies(
            &repo,
            "indexer",
            SearchMode::Auto,
            strategies,
            &records,
            now,
        )
        .await;
        let labels = filtered
            .iter()
            .map(|strategy| strategy.label.as_str())
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["ids_sxex", "freetext"]);
    }

    #[tokio::test]
    async fn learned_suppression_is_not_applied_to_interactive_search() {
        let now = Utc::now();
        let repo: StdArc<dyn IndexerSearchLearningRepository> =
            StdArc::new(InMemorySearchLearningRepository::default());
        let strategies = vec![
            strategy_with_label("ids_abs"),
            strategy_with_label("ids_sxex"),
        ];
        let records = vec![learning_record(
            "v2:ids_abs",
            3,
            0,
            true,
            Some(now.to_rfc3339()),
        )];

        let filtered = suppress_learned_strategies(
            &repo,
            "indexer",
            SearchMode::Interactive,
            strategies,
            &records,
            now,
        )
        .await;
        let labels = filtered
            .iter()
            .map(|strategy| strategy.label.as_str())
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["ids_abs", "ids_sxex"]);
    }

    #[tokio::test]
    async fn learned_suppression_claim_allows_only_one_stale_reprobe() {
        let now = Utc::now();
        let repo_impl = StdArc::new(InMemorySearchLearningRepository::default());
        let strategies = vec![
            strategy_with_label("ids_abs"),
            strategy_with_label("ids_sxex"),
            strategy_with_label("freetext"),
        ];
        let records = vec![learning_record(
            "v2:ids_abs",
            3,
            0,
            true,
            Some(
                (now - Duration::days(LEARNED_SUPPRESSION_REPROBE_INTERVAL_DAYS + 1)).to_rfc3339(),
            ),
        )];
        repo_impl
            .records
            .lock()
            .expect("learning records mutex")
            .insert(records[0].key.clone(), records[0].clone());
        let repo: StdArc<dyn IndexerSearchLearningRepository> = repo_impl;

        let filtered = suppress_learned_strategies(
            &repo,
            "indexer",
            SearchMode::Auto,
            strategies,
            &records,
            now,
        )
        .await;
        let labels = filtered
            .iter()
            .map(|strategy| strategy.label.as_str())
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["ids_abs", "ids_sxex", "freetext"]);

        let second_strategies = vec![
            strategy_with_label("ids_abs"),
            strategy_with_label("ids_sxex"),
            strategy_with_label("freetext"),
        ];
        let second_filtered = suppress_learned_strategies(
            &repo,
            "indexer",
            SearchMode::Auto,
            second_strategies,
            &records,
            now,
        )
        .await;
        let second_labels = second_filtered
            .iter()
            .map(|strategy| strategy.label.as_str())
            .collect::<Vec<_>>();

        assert_eq!(second_labels, vec!["ids_sxex", "freetext"]);
    }

    #[tokio::test]
    async fn learned_suppression_reprobes_missing_or_unparseable_updated_at() {
        let now = Utc::now();

        for updated_at in [None, Some("not-a-timestamp".to_string())] {
            let repo_impl = StdArc::new(InMemorySearchLearningRepository::default());
            let strategies = vec![strategy_with_label("ids_abs")];
            let records = vec![learning_record("v2:ids_abs", 3, 0, true, updated_at)];
            repo_impl
                .records
                .lock()
                .expect("learning records mutex")
                .insert(records[0].key.clone(), records[0].clone());
            let repo: StdArc<dyn IndexerSearchLearningRepository> = repo_impl;

            let filtered = suppress_learned_strategies(
                &repo,
                "indexer",
                SearchMode::Auto,
                strategies,
                &records,
                now,
            )
            .await;

            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].label, "ids_abs");
        }
    }

    #[derive(Default)]
    #[allow(clippy::type_complexity)]
    struct InMemorySearchLearningRepository {
        records: StdArc<StdMutex<HashMap<IndexerSearchLearningKey, IndexerSearchLearningRecord>>>,
        diagnostics:
            StdArc<StdMutex<Vec<(IndexerSearchRunWrite, Vec<IndexerSearchCandidateWrite>)>>>,
        cleanup_batches: StdArc<StdMutex<std::collections::VecDeque<u32>>>,
        cleanup_limits: StdArc<StdMutex<Vec<u32>>>,
    }

    #[async_trait]
    impl IndexerSearchLearningRepository for InMemorySearchLearningRepository {
        async fn list_for_title(
            &self,
            indexer_id: &str,
            title_id: &str,
            facet: &str,
        ) -> AppResult<Vec<IndexerSearchLearningRecord>> {
            Ok(self
                .records
                .lock()
                .expect("learning records mutex")
                .values()
                .filter(|record| {
                    record.key.indexer_id == indexer_id
                        && record.key.title_id == title_id
                        && record.key.facet == facet
                })
                .cloned()
                .collect())
        }

        async fn record_outcome(
            &self,
            key: &IndexerSearchLearningKey,
            usable_hits: u32,
        ) -> AppResult<IndexerSearchLearningRecord> {
            let now = Utc::now().to_rfc3339();
            let mut records = self.records.lock().expect("learning records mutex");
            let record =
                records
                    .entry(key.clone())
                    .or_insert_with(|| IndexerSearchLearningRecord {
                        key: key.clone(),
                        attempts: 0,
                        empty_successes: 0,
                        usable_successes: 0,
                        last_attempt_at: None,
                        last_usable_at: None,
                        suppressed: false,
                        updated_at: None,
                    });
            record.attempts += 1;
            record.last_attempt_at = Some(now.clone());
            record.updated_at = Some(now.clone());
            if usable_hits == 0 {
                record.empty_successes += 1;
            } else {
                record.usable_successes += 1;
                record.last_usable_at = Some(now);
                record.suppressed = false;
            }
            Ok(record.clone())
        }

        async fn record_search_diagnostics(
            &self,
            run: &IndexerSearchRunWrite,
            candidates: &[IndexerSearchCandidateWrite],
        ) -> AppResult<()> {
            self.diagnostics
                .lock()
                .expect("search diagnostics mutex")
                .push((run.clone(), candidates.to_vec()));
            Ok(())
        }

        async fn list_search_run_candidates(
            &self,
            run_id: &str,
        ) -> AppResult<Vec<ReusableIndexerSearchCandidate>> {
            Ok(self
                .diagnostics
                .lock()
                .expect("search diagnostics mutex")
                .iter()
                .find(|(run, _)| run.id == run_id)
                .map(|(_, candidates)| {
                    candidates
                        .iter()
                        .map(|candidate| ReusableIndexerSearchCandidate {
                            normalized: candidate.normalized.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default())
        }

        async fn cleanup_search_diagnostics(
            &self,
            _candidate_cutoff: DateTime<Utc>,
            _run_cutoff: DateTime<Utc>,
            limit: u32,
        ) -> AppResult<u32> {
            self.cleanup_limits
                .lock()
                .expect("cleanup limits mutex")
                .push(limit);
            Ok(self
                .cleanup_batches
                .lock()
                .expect("cleanup batches mutex")
                .pop_front()
                .unwrap_or(0))
        }

        async fn set_suppressed(
            &self,
            key: &IndexerSearchLearningKey,
            suppressed: bool,
        ) -> AppResult<()> {
            if let Some(record) = self
                .records
                .lock()
                .expect("learning records mutex")
                .get_mut(key)
            {
                record.suppressed = suppressed;
                record.updated_at = Some(Utc::now().to_rfc3339());
            }
            Ok(())
        }

        async fn try_claim_suppressed_reprobe(
            &self,
            key: &IndexerSearchLearningKey,
            stale_before: DateTime<Utc>,
        ) -> AppResult<bool> {
            let mut records = self.records.lock().expect("learning records mutex");
            let Some(record) = records.get_mut(key) else {
                return Ok(false);
            };
            if !record.suppressed {
                return Ok(false);
            }

            let stale = record
                .updated_at
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc) < stale_before)
                .unwrap_or(true);
            if !stale {
                return Ok(false);
            }

            record.updated_at = Some(Utc::now().to_rfc3339());
            Ok(true)
        }
    }

    #[tokio::test]
    async fn diagnostic_cleanup_drains_bounded_batches_and_reports_total() {
        let repo_impl = InMemorySearchLearningRepository::default();
        repo_impl
            .cleanup_batches
            .lock()
            .expect("cleanup batches mutex")
            .extend([500, 500, 37, 0]);
        let limits = repo_impl.cleanup_limits.clone();
        let repo: StdArc<dyn IndexerSearchLearningRepository> = StdArc::new(repo_impl);

        let deleted = drain_search_diagnostics(&repo, Utc::now())
            .await
            .expect("cleanup should drain all batches");

        assert_eq!(deleted, 1_037);
        assert_eq!(
            limits.lock().expect("cleanup limits mutex").as_slice(),
            &[500, 500, 500, 500]
        );
    }

    /// Corpus reuse needs Auto mode AND the context's explicit consent, which
    /// only the background convergence lanes give. An operator-triggered Auto
    /// search (no consent) and every Interactive search fire the indexer live.
    #[test]
    fn candidate_reuse_requires_auto_mode_and_a_consenting_context() {
        let background = IndexerSearchLearningContext {
            title_id: "title-1".into(),
            facet: "series".into(),
            subject_kind: ReleaseSearchSubjectKind::Episode,
            search_session_id: "session".into(),
            background_value: Some(0.5),
            candidate_reuse_allowed: true,
        };
        let operator = IndexerSearchLearningContext {
            candidate_reuse_allowed: false,
            background_value: None,
            ..background.clone()
        };

        assert!(candidate_reuse_permitted(
            SearchMode::Auto,
            Some(&background)
        ));
        assert!(
            !candidate_reuse_permitted(SearchMode::Auto, Some(&operator)),
            "an explicit operator search must fire live"
        );
        assert!(
            !candidate_reuse_permitted(SearchMode::Auto, None),
            "no context, nothing to rehydrate from"
        );
        assert!(
            !candidate_reuse_permitted(SearchMode::Interactive, Some(&background)),
            "interactive searches never reused"
        );
    }

    #[test]
    fn operator_auto_searches_admit_through_the_interactive_lane() {
        let multi = MultiIndexerSearchClient::new(
            Arc::new(MockIndexerConfigRepository {
                configs: vec![mock_indexer_config()],
            }),
            Arc::new(MockIndexerStatsTracker),
            Arc::new(MockIndexerPluginProvider {
                rss: false,
                calls: Arc::new(AtomicUsize::new(0)),
            }),
        );
        let background = IndexerSearchLearningContext {
            title_id: "title-1".into(),
            facet: "series".into(),
            subject_kind: ReleaseSearchSubjectKind::Episode,
            search_session_id: "session".into(),
            background_value: Some(0.5),
            candidate_reuse_allowed: true,
        };
        let operator = IndexerSearchLearningContext {
            candidate_reuse_allowed: false,
            background_value: None,
            ..background.clone()
        };

        let background_lane =
            |limit: &Arc<Semaphore>| Arc::ptr_eq(limit, &multi.background_search_limit);
        assert!(
            background_lane(&multi.search_limit_for_mode(SearchMode::Auto, true, None)),
            "RSS stays on the bounded background lane"
        );
        assert!(
            background_lane(&multi.search_limit_for_mode(
                SearchMode::Auto,
                false,
                Some(&background)
            )),
            "consenting convergence passes stay on the background lane"
        );
        assert!(
            !background_lane(&multi.search_limit_for_mode(
                SearchMode::Auto,
                false,
                Some(&operator)
            )),
            "an operator's Auto search must not queue behind the background sweep"
        );
        assert!(
            !background_lane(&multi.search_limit_for_mode(SearchMode::Auto, false, None)),
            "a context-less non-RSS Auto pass is operator-shaped"
        );
        assert!(
            !background_lane(&multi.search_limit_for_mode(
                SearchMode::Interactive,
                false,
                Some(&background)
            )),
            "interactive mode always uses the interactive lane"
        );
    }

    #[tokio::test]
    async fn learned_outcome_suppresses_empty_id_after_working_alternative() {
        let repo: StdArc<dyn IndexerSearchLearningRepository> =
            StdArc::new(InMemorySearchLearningRepository::default());
        let context = IndexerSearchLearningContext {
            title_id: "title-1".into(),
            facet: "anime".into(),
            subject_kind: ReleaseSearchSubjectKind::Episode,
            search_session_id: "test-session".into(),
            background_value: None,
            candidate_reuse_allowed: true,
        };

        record_strategy_learning_outcome(
            &repo,
            Some(&context),
            SearchMode::Auto,
            "idx",
            "Indexer",
            "ids_sxex",
            1,
        )
        .await;
        for _ in 0..LEARNED_EMPTY_SUPPRESSION_THRESHOLD {
            record_strategy_learning_outcome(
                &repo,
                Some(&context),
                SearchMode::Auto,
                "idx",
                "Indexer",
                "ids_abs",
                0,
            )
            .await;
        }

        let records = repo
            .list_for_title("idx", "title-1", "anime")
            .await
            .expect("learning records");
        let abs_record = records
            .iter()
            .find(|record| record.key.strategy_key == "v2:ids_abs")
            .expect("ids_abs record");

        assert_eq!(
            abs_record.empty_successes,
            LEARNED_EMPTY_SUPPRESSION_THRESHOLD
        );
        assert_eq!(abs_record.usable_successes, 0);
        assert!(abs_record.suppressed);
    }

    #[tokio::test]
    async fn learned_outcome_does_not_suppress_without_working_alternative() {
        let repo: StdArc<dyn IndexerSearchLearningRepository> =
            StdArc::new(InMemorySearchLearningRepository::default());
        let context = IndexerSearchLearningContext {
            title_id: "title-1".into(),
            facet: "anime".into(),
            subject_kind: ReleaseSearchSubjectKind::Episode,
            search_session_id: "test-session".into(),
            background_value: None,
            candidate_reuse_allowed: true,
        };

        for _ in 0..LEARNED_EMPTY_SUPPRESSION_THRESHOLD {
            record_strategy_learning_outcome(
                &repo,
                Some(&context),
                SearchMode::Auto,
                "idx",
                "Indexer",
                "ids_abs",
                0,
            )
            .await;
        }

        let records = repo
            .list_for_title("idx", "title-1", "anime")
            .await
            .expect("learning records");
        let abs_record = records
            .iter()
            .find(|record| record.key.strategy_key == "v2:ids_abs")
            .expect("ids_abs record");

        assert!(!abs_record.suppressed);
    }

    #[tokio::test]
    async fn learned_stale_reprobe_usable_outcome_clears_suppression() {
        let repo_impl = InMemorySearchLearningRepository::default();
        let key = IndexerSearchLearningKey {
            indexer_id: "idx".into(),
            title_id: "title-1".into(),
            facet: "anime".into(),
            strategy_key: "v2:ids_abs".into(),
        };
        repo_impl
            .records
            .lock()
            .expect("learning records mutex")
            .insert(
                key,
                learning_record(
                    "v2:ids_abs",
                    LEARNED_EMPTY_SUPPRESSION_THRESHOLD,
                    0,
                    true,
                    Some(
                        (Utc::now()
                            - Duration::days(LEARNED_SUPPRESSION_REPROBE_INTERVAL_DAYS + 1))
                        .to_rfc3339(),
                    ),
                ),
            );
        let repo: StdArc<dyn IndexerSearchLearningRepository> = StdArc::new(repo_impl);
        let context = IndexerSearchLearningContext {
            title_id: "title-1".into(),
            facet: "anime".into(),
            subject_kind: ReleaseSearchSubjectKind::Episode,
            search_session_id: "test-session".into(),
            background_value: None,
            candidate_reuse_allowed: true,
        };

        record_strategy_learning_outcome(
            &repo,
            Some(&context),
            SearchMode::Auto,
            "idx",
            "Indexer",
            "ids_abs",
            1,
        )
        .await;

        let records = repo
            .list_for_title("idx", "title-1", "anime")
            .await
            .expect("learning records");
        let abs_record = records
            .iter()
            .find(|record| record.key.strategy_key == "v2:ids_abs")
            .expect("ids_abs record");

        assert_eq!(abs_record.usable_successes, 1);
        assert!(!abs_record.suppressed);
    }

    #[tokio::test]
    async fn automatic_search_errors_do_not_record_empty_learning() {
        let repo: StdArc<dyn IndexerSearchLearningRepository> =
            StdArc::new(InMemorySearchLearningRepository::default());
        let (client, _calls) = scripted_search_client(movie_caps(), |_call| {
            Err(AppError::Repository("quota limited".into()))
        });
        let client = client.with_search_learning_repository(repo.clone());
        let context = IndexerSearchLearningContext {
            title_id: "title-1".into(),
            facet: "movie".into(),
            subject_kind: ReleaseSearchSubjectKind::Title,
            search_session_id: "test-session".into(),
            background_value: None,
            candidate_reuse_allowed: true,
        };

        let result = <MultiIndexerSearchClient as IndexerClient>::search(
            &client,
            "Lattice Zero".into(),
            HashMap::from([("imdb_id".to_string(), "tt0133093".to_string())]),
            Some("movie".into()),
            Some("movie".into()),
            None,
            None,
            None,
            SearchMode::Auto,
            IndexerErrorOperation::AutomaticSearch,
            None,
            None,
            None,
            None,
            vec![],
            Some(context),
            CancellationToken::new(),
        )
        .await;

        let _ = result.expect("automatic aggregate search should tolerate indexer errors");
        let records = repo
            .list_for_title("idx-1", "title-1", "movie")
            .await
            .expect("learning records");
        assert!(records.is_empty());
    }

    #[tokio::test]
    async fn automatic_search_partial_responses_do_not_record_empty_learning() {
        let repo: StdArc<dyn IndexerSearchLearningRepository> =
            StdArc::new(InMemorySearchLearningRepository::default());
        let (client, _calls) = scripted_search_client(movie_caps(), |_call| {
            Ok(IndexerSearchResponse {
                completion: IndexerSearchCompletion::Partial {
                    reason: Some(IndexerSearchIncompleteReason::UpstreamFailure),
                    retry_after: None,
                },
                indexer_outcomes: Vec::new(),
                results: Vec::new(),

                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        });
        let client = client.with_search_learning_repository(repo.clone());
        let context = IndexerSearchLearningContext {
            title_id: "title-1".into(),
            facet: "movie".into(),
            subject_kind: ReleaseSearchSubjectKind::Title,
            search_session_id: "test-session".into(),
            background_value: None,
            candidate_reuse_allowed: true,
        };

        let result = <MultiIndexerSearchClient as IndexerClient>::search(
            &client,
            "Lattice Zero".into(),
            HashMap::from([("imdb_id".to_string(), "tt0133093".to_string())]),
            Some("movie".into()),
            Some("movie".into()),
            None,
            None,
            None,
            SearchMode::Auto,
            IndexerErrorOperation::AutomaticSearch,
            None,
            None,
            None,
            None,
            vec![],
            Some(context),
            CancellationToken::new(),
        )
        .await;

        let response = result.expect("partial candidates remain usable by the aggregate search");
        assert!(matches!(
            response.completion,
            IndexerSearchCompletion::Partial { .. }
        ));
        let records = repo
            .list_for_title("idx-1", "title-1", "movie")
            .await
            .expect("learning records");
        assert!(records.is_empty());
    }

    #[tokio::test]
    async fn automatic_search_persists_only_strategy_admitted_candidates() {
        let repo_impl = StdArc::new(InMemorySearchLearningRepository::default());
        let diagnostics = repo_impl.diagnostics.clone();
        let repo: StdArc<dyn IndexerSearchLearningRepository> = repo_impl;
        let (client, calls) = scripted_search_client(series_caps(), |call| {
            if call.ids.contains_key("tvdb_id") {
                response_with_titles(&["Signal.Run.S02E12.720p.WEB-DL"])
            } else {
                response_with_titles(&[
                    "Signal.Run.S01E12.720p.WEB-DL",
                    "Signal.Road.S01E12.2160p.WEB-DL",
                ])
            }
        });
        let client = client.with_search_learning_repository(repo);
        let context = IndexerSearchLearningContext {
            title_id: "title-1".into(),
            facet: "series".into(),
            subject_kind: ReleaseSearchSubjectKind::Episode,
            search_session_id: "test-session".into(),
            background_value: None,
            candidate_reuse_allowed: true,
        };

        let response = <MultiIndexerSearchClient as IndexerClient>::search(
            &client,
            "Signal Run S01E12".into(),
            HashMap::from([("tvdb_id".to_string(), "78874".to_string())]),
            Some("series".into()),
            Some("series".into()),
            None,
            None,
            None,
            SearchMode::Auto,
            IndexerErrorOperation::AutomaticSearch,
            Some(1),
            Some(12),
            None,
            None,
            vec![],
            Some(context),
            CancellationToken::new(),
        )
        .await
        .expect("automatic search should succeed");

        assert_eq!(
            response
                .results
                .iter()
                .map(|result| result.title.as_str())
                .collect::<Vec<_>>(),
            ["Signal.Run.S01E12.720p.WEB-DL"]
        );
        assert_eq!(calls.lock().expect("call log mutex").len(), 2);

        let diagnostics = diagnostics.lock().expect("search diagnostics mutex");
        assert_eq!(diagnostics.len(), 2);

        let (primary_run, primary_candidates) = diagnostics
            .iter()
            .find(|(run, _)| run.branch != "freetext")
            .expect("primary diagnostics");
        assert_eq!(primary_run.result_count, 1);
        assert!(primary_candidates.is_empty());

        let (fallback_run, fallback_candidates) = diagnostics
            .iter()
            .find(|(run, _)| run.branch == "freetext")
            .expect("fallback diagnostics");
        assert_eq!(fallback_run.result_count, 2);
        assert_eq!(fallback_candidates.len(), 1);
        assert_eq!(
            fallback_candidates[0].normalized.title,
            "Signal.Run.S01E12.720p.WEB-DL"
        );
    }

    #[test]
    fn auto_strategy_tier_prefers_absolute_id_and_reserves_freetext() {
        let (primary, fallback) = split_strategy_tiers(
            SearchMode::Auto,
            "anime",
            vec![
                strategy_with_label("ids_sxex"),
                strategy_with_label("freetext"),
                strategy_with_label("ids_abs"),
            ],
        );

        assert_eq!(primary.len(), 1);
        assert_eq!(primary[0].label, "ids_abs");
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].label, "freetext");
    }

    #[test]
    fn auto_strategy_tier_uses_single_text_strategy_without_ids() {
        let (primary, fallback) = split_strategy_tiers(
            SearchMode::Auto,
            "anime",
            vec![
                strategy_with_label("freetext_alias"),
                strategy_with_label("freetext"),
            ],
        );

        assert_eq!(primary.len(), 1);
        assert_eq!(primary[0].label, "freetext");
        assert!(fallback.is_empty());
    }

    #[test]
    fn interactive_strategy_tier_keeps_parallel_id_strategies() {
        let (primary, fallback) = split_strategy_tiers(
            SearchMode::Interactive,
            "anime",
            vec![
                strategy_with_label("ids_abs"),
                strategy_with_label("ids_sxex"),
                strategy_with_label("freetext"),
            ],
        );

        assert_eq!(
            primary
                .iter()
                .map(|strategy| strategy.label.as_str())
                .collect::<Vec<_>>(),
            vec!["ids_abs", "ids_sxex"]
        );
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].label, "freetext");
    }

    #[test]
    fn auto_fallback_tier_is_not_spent_after_primary_error() {
        let fallback = vec![strategy_with_label("freetext")];

        assert!(!should_run_fallback_tier(
            SearchMode::Auto,
            0,
            true,
            true,
            &fallback
        ));
        assert!(should_run_fallback_tier(
            SearchMode::Interactive,
            0,
            true,
            true,
            &fallback
        ));
    }

    #[test]
    fn preferred_anime_alias_query_strips_episode_context() {
        let alias = preferred_anime_alias_query(
            "Silver Horizon: Beyond Harbor's End S02E05",
            &[scryer_domain::TaggedAlias {
                name: "Sora no Vale".into(),
                language: "jpn".into(),
            }],
        );

        assert_eq!(alias.as_deref(), Some("Sora no Vale"));
    }

    #[test]
    fn preferred_anime_alias_query_skips_canonical_alias_and_uses_distinct_romanized_alias() {
        let alias = preferred_anime_alias_query(
            "Silver Horizon: Beyond Harbor's End S02E05",
            &[
                scryer_domain::TaggedAlias {
                    name: "Silver Horizon: Beyond Harbor's End".into(),
                    language: "jpn".into(),
                },
                scryer_domain::TaggedAlias {
                    name: "Sora no Vale".into(),
                    language: "jpn".into(),
                },
            ],
        );

        assert_eq!(alias.as_deref(), Some("Sora no Vale"));
    }

    #[tokio::test]
    async fn indexer_rate_limiter_reserves_concurrent_slots_for_one_indexer() {
        let limiter = IndexerRateLimiter::new();
        let started_at = std::time::Instant::now();

        let (first, second, third) = tokio::join!(
            async {
                limiter.acquire("idx", Some(2)).await;
                started_at.elapsed()
            },
            async {
                limiter.acquire("idx", Some(2)).await;
                started_at.elapsed()
            },
            async {
                limiter.acquire("idx", Some(2)).await;
                started_at.elapsed()
            },
        );
        let mut dispatches = [first, second, third];
        dispatches.sort();

        assert!(
            dispatches[0] < std::time::Duration::from_millis(500),
            "the first request should dispatch immediately: {dispatches:?}"
        );
        assert!(
            dispatches[1] >= std::time::Duration::from_millis(1_900)
                && dispatches[1] < std::time::Duration::from_secs(3),
            "the second request should dispatch at roughly two seconds: {dispatches:?}"
        );
        assert!(
            dispatches[2] >= std::time::Duration::from_millis(3_900)
                && dispatches[2] < std::time::Duration::from_secs(6),
            "the third request should dispatch at roughly four seconds: {dispatches:?}"
        );
    }

    #[tokio::test]
    async fn indexer_rate_limiter_only_paces_explicit_intervals_per_indexer() {
        let limiter = IndexerRateLimiter::new();

        limiter.acquire("idx", None).await;

        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            limiter.acquire("idx", None),
        )
        .await
        .expect("missing per-indexer interval should not pace; host RPS owns default pacing");

        limiter.acquire("idx", Some(1)).await;

        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            limiter.acquire("other-idx", Some(1)),
        )
        .await
        .expect("a different indexer should have an independent pacing schedule");
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
            ..Default::default()
        };

        let ids = HashMap::from([("tvdb_id".to_string(), "424536".to_string())]);
        let strategies = build_strategies(&StrategyParams {
            query: "Sora no Vale",
            query_facet: "anime",
            id_facet: "anime",
            ids: &ids,
            season: Some(2),
            episode: Some(5),
            absolute_episode: Some(33),
            caps: &caps,
            id_dispatch_mode: IdDispatchMode::LegacyAggregate,
            text_dispatch_mode: TextDispatchMode::FacetScoped,
            is_alias_query: true,
            facet_omitted: false,
        });

        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies[0].label, "freetext_alias");
        assert!(strategies[0].ids.is_empty());
        assert_eq!(strategies[0].season, Some(2));
        assert_eq!(strategies[0].episode, Some(5));
        assert_eq!(strategies[0].absolute_episode, None);
    }
}
