//! Source cursors and immutable aggregate positions shared by raw-search callers.

use super::memory::{
    MemoryCharge, RawSearchBudget, compact_json_objects, result_heap, table_storage,
};
use super::{dedupe_exact_duplicate_releases, published_instant};
use crate::error_reasons::raw_search as reasons;
use crate::{
    AppError, AppResult, IndexerClient, IndexerSearchOutcome, IndexerSearchPageMetadata,
    IndexerSearchResult, RawTextSearchPage, RawTextSearchRequest,
};
use std::collections::{HashSet, VecDeque};
use std::mem::size_of;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub const RAW_SEARCH_RESPONSE_DEADLINE: Duration = Duration::from_secs(75);
pub const RAW_SEARCH_IDLE_TTL: Duration = Duration::from_secs(300);
pub const RAW_SEARCH_PAGE_SIZE: usize = 100;

#[cfg(test)]
#[path = "raw_tests.rs"]
mod tests;

fn resource_refusal() -> AppError {
    AppError::validation_refused(
        reasons::RETAINED_STATE_FULL,
        "Raw search retained-state capacity is full; retry the search".to_owned(),
    )
}

fn compare_raw_results(
    left: &IndexerSearchResult,
    right: &IndexerSearchResult,
) -> std::cmp::Ordering {
    published_instant(right)
        .cmp(&published_instant(left))
        .then_with(|| left.indexer_id.cmp(&right.indexer_id))
        .then_with(|| left.guid.cmp(&right.guid))
        .then_with(|| left.download_url.cmp(&right.download_url))
        .then_with(|| left.title.cmp(&right.title))
}

/// An adapter supplies already-authorized source requests. A legacy provider
/// uses its existing first read; only a paging-capable provider receives page.
#[derive(Clone)]
pub(crate) struct RawSearchSource {
    pub id: String,
    pub request: RawTextSearchRequest,
    pub paged: bool,
    pub result_scope: super::scope_filter::RawSearchResultScope,
}

struct StoredPage {
    rows: Vec<IndexerSearchResult>,
    _charge: MemoryCharge,
}

/// A row keeps its complete source page's reservation alive even after the
/// generation is evicted. Neither cache hits nor serialization clone payloads.
#[derive(Clone)]
pub struct RawSearchRow {
    page: Arc<StoredPage>,
    position: usize,
}

impl RawSearchRow {
    pub fn result(&self) -> &IndexerSearchResult {
        &self.page.rows[self.position]
    }

    /// The same conservative owned-storage estimate used for pool admission.
    /// Serializers use this before allocating per-item working storage.
    pub fn storage_bytes(&self) -> usize {
        size_of::<IndexerSearchResult>().saturating_add(result_heap(self.result()))
    }
}

#[derive(Clone, Debug)]
pub struct RawSearchSourceProgress {
    pub indexer_id: String,
    pub pending: bool,
    pub exhausted: bool,
    pub stopped: bool,
    pub outcome: Option<IndexerSearchOutcome>,
    pub page: Option<IndexerSearchPageMetadata>,
    pub refusal_reason: Option<&'static str>,
    pub retry_after: Option<Duration>,
    pub elapsed_ms: Option<u64>,
    /// Rows this source has handed the generation so far, across every page
    /// it answered, whether or not they have been merged into an output page.
    pub delivered: usize,
}

pub struct RawSearchPage {
    pub offset: usize,
    pub rows: Vec<RawSearchRow>,
    /// A deadline-short page may be nonterminal. Never infer exhaustion from
    /// row count: a late source remains generation-owned and may append later.
    pub exhausted: bool,
    /// No source can advance this generation, including unavailable sources.
    /// This is distinct from every upstream actually being exhausted.
    pub terminal: bool,
    pub deadline_reached: bool,
    pub sources: Vec<RawSearchSourceProgress>,
    registry: Option<Weak<super::InteractiveReleaseSearchRegistry>>,
    _charge: MemoryCharge,
}

/// Output bytes retain their shared reservation even after their response and
/// search generation are dropped. Clones share one charge, never a new budget.
#[derive(Clone)]
pub struct RawSearchOutputLease {
    _charge: Arc<MemoryCharge>,
}

impl RawSearchPage {
    /// Reserve serialization scratch space and buffered output before creating
    /// either. The caller attaches this lease to every emitted byte chunk.
    pub async fn reserve_output(&self, bytes: usize) -> AppResult<RawSearchOutputLease> {
        let bytes = bytes
            .checked_add(size_of::<MemoryCharge>() + 2 * size_of::<usize>())
            .ok_or_else(resource_refusal)?;
        let charge = RawSearchGeneration::reserve_reclaiming(
            self._charge.pool(),
            bytes,
            self.registry.as_ref(),
        )
        .await
        .ok_or_else(resource_refusal)?;
        Ok(RawSearchOutputLease {
            _charge: Arc::new(charge),
        })
    }
}

impl crate::AppUseCase {
    /// Compatibility metadata uses the same pool as retained searches and feed
    /// output. This does not create a separate request or indexer quota.
    pub async fn reserve_raw_search_output(&self, bytes: usize) -> AppResult<RawSearchOutputLease> {
        let bytes = bytes
            .checked_add(size_of::<MemoryCharge>() + 2 * size_of::<usize>())
            .ok_or_else(resource_refusal)?;
        let registry = Arc::downgrade(&self.runtime.acquisition.interactive_release_searches);
        let charge = RawSearchGeneration::reserve_reclaiming(
            &self.runtime.acquisition.raw_search_budget,
            bytes,
            Some(&registry),
        )
        .await
        .ok_or_else(resource_refusal)?;
        Ok(RawSearchOutputLease {
            _charge: Arc::new(charge),
        })
    }
}

impl std::fmt::Debug for RawSearchPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RawSearchPage")
            .field("offset", &self.offset)
            .field("rows", &self.rows.len())
            .field("exhausted", &self.exhausted)
            .field("deadline_reached", &self.deadline_reached)
            .finish_non_exhaustive()
    }
}

struct SourceState {
    source: RawSearchSource,
    buffer: Option<Arc<StoredPage>>,
    position: usize,
    cursor: Option<String>,
    visited: HashSet<String>,
    page_fingerprints: HashSet<String>,
    pending: bool,
    exhausted: bool,
    stopped: bool,
    succeeded: bool,
    attempted: bool,
    elapsed_ms: Option<u64>,
    retry_after: Option<Duration>,
    outcome: Option<IndexerSearchOutcome>,
    page: Option<IndexerSearchPageMetadata>,
    refusal_reason: Option<&'static str>,
    delivered: usize,
}

struct State {
    sources: Vec<SourceState>,
    emitted: VecDeque<RawSearchRow>,
    position: usize,
    history_start: usize,
    active_offset: usize,
    charge: MemoryCharge,
}

/// This is a generation stored by the existing interactive-search registry,
/// not a separate registry, scheduler, timeout policy, or quota account.
pub(crate) struct RawSearchGeneration {
    state: Mutex<State>,
    advancement: Mutex<()>,
    touched: std::sync::Mutex<Instant>,
    rebuild: Mutex<Weak<Self>>,
    changed: Notify,
    budget: Arc<RawSearchBudget>,
    client: Arc<dyn IndexerClient>,
    cancel: CancellationToken,
    registry: Option<Weak<super::InteractiveReleaseSearchRegistry>>,
    coordinator: Arc<crate::RestoreCoordinator>,
}

fn request_heap(request: &RawTextSearchRequest) -> usize {
    let mut bytes = request.query.capacity()
        + request.categories.capacity() * size_of::<String>()
        + request
            .categories
            .iter()
            .map(String::capacity)
            .sum::<usize>();
    if let Some(ids) = &request.indexer_ids {
        bytes += table_storage::<String, ()>(ids.capacity())
            + ids.iter().map(String::capacity).sum::<usize>();
    }
    if let Some(nab) = &request.nab {
        bytes += super::memory::tree_storage::<String, String>(nab.parameters.len())
            + nab
                .parameters
                .iter()
                .map(|(key, value)| key.capacity() + value.capacity())
                .sum::<usize>();
    }
    bytes
        + request
            .page
            .as_ref()
            .and_then(|page| page.cursor.as_ref())
            .map_or(0, String::capacity)
}

fn state_bytes(state: &State) -> usize {
    // Include one successor's Arc allocation while the weak rebuild link
    // survives its last strong owner; its dynamic payload is charged separately.
    2 * (size_of::<RawSearchGeneration>() + 2 * size_of::<usize>())
        + state.sources.capacity() * size_of::<SourceState>()
        + state.emitted.capacity() * size_of::<RawSearchRow>()
        + state
            .sources
            .iter()
            .map(|source| {
                source.source.id.capacity()
                    + request_heap(&source.source.request)
                    + source.source.result_scope.heap_bytes()
                    + source.cursor.as_ref().map_or(0, String::capacity)
                    + table_storage::<String, ()>(source.visited.capacity())
                    + source.visited.iter().map(String::capacity).sum::<usize>()
                    + table_storage::<String, ()>(source.page_fingerprints.capacity())
                    + source
                        .page_fingerprints
                        .iter()
                        .map(String::capacity)
                        .sum::<usize>()
                    + source
                        .page
                        .as_ref()
                        .and_then(|page| page.next_cursor.as_ref())
                        .map_or(0, String::capacity)
            })
            .sum::<usize>()
}

impl RawSearchGeneration {
    pub(crate) async fn new(
        sources: Vec<RawSearchSource>,
        client: Arc<dyn IndexerClient>,
        budget: Arc<RawSearchBudget>,
        cancel: CancellationToken,
        registry: Option<Weak<super::InteractiveReleaseSearchRegistry>>,
        coordinator: Arc<crate::RestoreCoordinator>,
    ) -> AppResult<Arc<Self>> {
        let mut state = State {
            sources: sources
                .into_iter()
                .map(|source| SourceState {
                    source,
                    buffer: None,
                    position: 0,
                    cursor: None,
                    visited: HashSet::new(),
                    page_fingerprints: HashSet::new(),
                    pending: false,
                    exhausted: false,
                    stopped: false,
                    succeeded: false,
                    attempted: false,
                    elapsed_ms: None,
                    retry_after: None,
                    outcome: None,
                    page: None,
                    refusal_reason: None,
                    delivered: 0,
                })
                .collect(),
            emitted: VecDeque::new(),
            position: 0,
            history_start: 0,
            active_offset: 0,
            charge: budget.reserve(0).ok_or_else(resource_refusal)?,
        };
        state.charge = Self::reserve_reclaiming(&budget, state_bytes(&state), registry.as_ref())
            .await
            .ok_or_else(resource_refusal)?;
        Ok(Arc::new(Self {
            state: Mutex::new(state),
            advancement: Mutex::new(()),
            touched: std::sync::Mutex::new(Instant::now()),
            rebuild: Mutex::new(Weak::new()),
            changed: Notify::new(),
            budget,
            client,
            cancel,
            registry,
            coordinator,
        }))
    }

    pub(super) async fn reserve_reclaiming(
        budget: &Arc<RawSearchBudget>,
        bytes: usize,
        registry: Option<&Weak<super::InteractiveReleaseSearchRegistry>>,
    ) -> Option<MemoryCharge> {
        Self::admit_reclaiming(registry, || budget.reserve(bytes)).await
    }

    async fn resize_reclaiming(&self, charge: &mut MemoryCharge, bytes: usize) -> bool {
        Self::admit_reclaiming(self.registry.as_ref(), || {
            charge.resize(bytes).then_some(())
        })
        .await
        .is_some()
    }

    async fn admit_reclaiming<T>(
        registry: Option<&Weak<super::InteractiveReleaseSearchRegistry>>,
        mut attempt: impl FnMut() -> Option<T>,
    ) -> Option<T> {
        if let Some(admitted) = attempt() {
            return Some(admitted);
        }
        let registry = registry?.upgrade()?;
        let mut entries = registry.lock().await;
        loop {
            // A source task or page reader owns another Arc. Evict only idle
            // generations; outstanding response payloads keep their own charge.
            // Access timestamps have their own lock, independent of source state.
            let oldest = entries
                .iter()
                .filter_map(|(id, entry)| {
                    let raw = entry.raw.as_ref()?;
                    if Arc::strong_count(raw) != 1 {
                        return None;
                    }
                    let touched = *raw
                        .touched
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    Some((id.clone(), touched))
                })
                .min_by_key(|(_, touched)| *touched)
                .map(|(id, _)| id)?;
            entries.remove(&oldest);
            entries.shrink_to_fit();
            if let Some(admitted) = attempt() {
                return Some(admitted);
            }
        }
    }

    pub(super) fn touch(&self) {
        *self
            .touched
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
    }

    pub(crate) fn idle_expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(
            *self
                .touched
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        ) >= RAW_SEARCH_IDLE_TTL
    }

    pub(super) async fn source_progress(&self) -> Vec<RawSearchSourceProgress> {
        self.state
            .lock()
            .await
            .sources
            .iter()
            .map(|source| RawSearchSourceProgress {
                indexer_id: source.source.id.clone(),
                pending: source.pending || !source.attempted,
                exhausted: source.exhausted,
                stopped: source.stopped,
                retry_after: source.retry_after,
                outcome: source.outcome,
                page: source.page.clone(),
                refusal_reason: source.refusal_reason,
                elapsed_ms: source.elapsed_ms,
                delivered: source.delivered,
            })
            .collect()
    }

    /// Every invocation, including offset reconstruction, gets one response
    /// deadline. The spawned source calls belong to this Arc, not this future.
    #[cfg(test)]
    pub(crate) async fn page(
        self: &Arc<Self>,
        offset: usize,
        limit: usize,
    ) -> AppResult<RawSearchPage> {
        let deadline = Instant::now() + RAW_SEARCH_RESPONSE_DEADLINE;
        self.page_until(offset, limit, deadline).await
    }

    /// Reconstruct discarded history without giving the rebuild another
    /// response deadline or cancelling reads owned by the previous generation.
    pub(super) async fn page_or_rebuild(
        self: &Arc<Self>,
        offset: usize,
        limit: usize,
    ) -> AppResult<(Arc<Self>, RawSearchPage)> {
        let deadline = Instant::now() + RAW_SEARCH_RESPONSE_DEADLINE;
        let mut generation = self.clone();
        let mut retried_capacity = false;
        loop {
            match generation.page_until(offset, limit, deadline).await {
                Ok(page) => return Ok((generation, page)),
                Err(error)
                    if error.validation_reason() == Some(reasons::HISTORY_EXPIRED)
                        || (!retried_capacity
                            && error.validation_reason() == Some(reasons::RETAINED_STATE_FULL)) =>
                {
                    retried_capacity |=
                        error.validation_reason() == Some(reasons::RETAINED_STATE_FULL);
                    let mut successor = generation.rebuild.lock().await;
                    let rebuilt = if let Some(rebuilt) = successor.upgrade() {
                        rebuilt
                    } else {
                        let sources = generation
                            .state
                            .lock()
                            .await
                            .sources
                            .iter()
                            .map(|source| source.source.clone())
                            .collect();
                        let rebuilt = Self::new(
                            sources,
                            generation.client.clone(),
                            generation.budget.clone(),
                            generation.cancel.clone(),
                            generation.registry.clone(),
                            generation.coordinator.clone(),
                        )
                        .await?;
                        *successor = Arc::downgrade(&rebuilt);
                        rebuilt
                    };
                    drop(successor);
                    generation = rebuilt;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn page_until(
        self: &Arc<Self>,
        offset: usize,
        limit: usize,
        deadline: Instant,
    ) -> AppResult<RawSearchPage> {
        let limit = limit.clamp(1, RAW_SEARCH_PAGE_SIZE);
        let end = offset.checked_add(limit).ok_or_else(|| {
            AppError::validation_refused(
                reasons::OFFSET_INVALID,
                "Raw search offset is out of range".to_owned(),
            )
        })?;
        let _advance = match tokio::time::timeout_at(deadline, self.advancement.lock()).await {
            Ok(guard) => guard,
            Err(_) => {
                return self
                    .snapshot_page(&*self.state.lock().await, offset, end, true)
                    .await;
            }
        };
        {
            let mut state = self.state.lock().await;
            self.touch();
            state.active_offset = offset;
            if offset < state.history_start {
                return Err(AppError::validation_refused(
                    reasons::HISTORY_EXPIRED,
                    "This earlier raw search page must be rebuilt".to_owned(),
                ));
            }
        }
        loop {
            // Register before examining state: a fast provider cannot finish
            // between the check and waiter registration and lose its wakeup.
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let mut state = self.state.lock().await;
            if self.cancel.is_cancelled() {
                return Err(AppError::validation_refused(
                    reasons::CANCELLED,
                    "Raw search was cancelled".to_owned(),
                ));
            }
            let deadline_reached = Instant::now() >= deadline;
            if !deadline_reached && state.position < end {
                self.start_needed_reads(&mut state).await?;
            }
            self.merge_available(&mut state, offset, end, deadline_reached)
                .await?;
            let pending = state.sources.iter().any(|source| source.pending);
            let available = state.sources.iter().any(|source| source.buffer.is_some());
            let refill = state.sources.iter().any(|source| self.can_read(source));
            if state.position >= end || deadline_reached || (!pending && !available && !refill) {
                return self
                    .snapshot_page(&state, offset, end, deadline_reached)
                    .await;
            }
            // A consumed source may now need a refill. Schedule it before
            // waiting, while never launching a duplicate read of that source.
            if !deadline_reached {
                self.start_needed_reads(&mut state).await?;
            }
            if state.sources.iter().all(|source| !source.pending) && available {
                drop(state);
                continue;
            }
            drop(state);
            tokio::select! {
                _ = &mut changed => {}
                _ = tokio::time::sleep_until(deadline) => {}
                _ = self.cancel.cancelled() => {}
            }
        }
    }

    fn can_read(&self, source: &SourceState) -> bool {
        !self.cancel.is_cancelled()
            && !source.pending
            && !source.exhausted
            && !source.stopped
            && source.buffer.is_none()
            && source.refusal_reason.is_none()
            && (!source.attempted || source.cursor.is_some())
    }

    async fn start_needed_reads(self: &Arc<Self>, state: &mut State) -> AppResult<()> {
        for index in 0..state.sources.len() {
            let source = &mut state.sources[index];
            if !self.can_read(source) {
                continue;
            }
            let mut request = source.source.request.clone();
            request.indexer_ids = Some(HashSet::from([source.source.id.clone()]));
            request.page = source.source.paged.then(|| RawTextSearchPage {
                cursor: source.cursor.clone(),
            });
            let request_charge = Self::reserve_reclaiming(
                &self.budget,
                size_of::<RawTextSearchRequest>() + request_heap(&request) + 1024,
                self.registry.as_ref(),
            )
            .await
            .ok_or_else(resource_refusal)?;
            source.pending = true;
            source.attempted = true;
            let generation = self.clone();
            self.coordinator.spawn_task(async move {
                let started = Instant::now();
                // Cancellation belongs to the source call: keep tracking it
                // through persistence cleanup, even if it ignores cancellation.
                let response = if generation.cancel.is_cancelled() {
                    Err(AppError::canceled("raw source cancelled before admission"))
                } else {
                    generation
                        .client
                        .search_raw_text(request, generation.cancel.child_token())
                        .await
                };
                // The source's existing timeout, pacing and accounting stay
                // inside IndexerClient. No HTTP-request cancellation enters.
                let elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                generation
                    .accept_response(index, response, elapsed_ms)
                    .await;
                drop(request_charge);
            });
        }
        Ok(())
    }

    async fn accept_response(
        &self,
        index: usize,
        response: AppResult<crate::IndexerSearchResponse>,
        elapsed_ms: u64,
    ) {
        let mut state = self.state.lock().await;
        // Delivered history is evictable; an unconsumed source tail is not.
        // Outstanding responses retain their own Arc and budget reservation.
        let source = &mut state.sources[index];
        source.pending = false;
        source.elapsed_ms = Some(
            source
                .elapsed_ms
                .unwrap_or_default()
                .saturating_add(elapsed_ms),
        );
        match response {
            Err(error) => {
                source.stopped = true;
                source.refusal_reason = error.reason();
                source.outcome = Some(match error {
                    AppError::TemporaryUnavailable {
                        retry_after,
                        rate_limit_cooldown,
                        ..
                    } => {
                        source.retry_after = retry_after;
                        if matches!(rate_limit_cooldown, crate::RateLimitCooldownAction::None) {
                            IndexerSearchOutcome::Errored
                        } else {
                            IndexerSearchOutcome::Deferred { retry_after }
                        }
                    }
                    AppError::NewznabQuotaExceeded { .. } => {
                        IndexerSearchOutcome::Deferred { retry_after: None }
                    }
                    _ => IndexerSearchOutcome::Errored,
                });
            }
            Ok(mut response) => {
                // The single-page adapter deliberately leaves walk state to
                // this generation. Use the same ordered identity fingerprint
                // as the legacy accumulating PagedRead before any filtering,
                // sorting or normalization changes the received page.
                let fingerprint = source
                    .source
                    .paged
                    .then(|| {
                        crate::indexer_listing_page_fingerprint(response.results.iter().map(
                            |result| {
                                [
                                    result.guid.as_deref(),
                                    result.link.as_deref(),
                                    result.download_url.as_deref(),
                                    Some(result.title.as_str()),
                                ]
                            },
                        ))
                    })
                    .flatten();
                if fingerprint
                    .is_some_and(|fingerprint| !source.page_fingerprints.insert(fingerprint))
                {
                    response.results.clear();
                    response.next_cursor = None;
                    for outcome in &mut response.indexer_outcomes {
                        if outcome.indexer_id == source.source.id {
                            outcome.outcome = match outcome.outcome {
                                IndexerSearchOutcome::Complete { .. } => {
                                    IndexerSearchOutcome::Complete { empty: true }
                                }
                                IndexerSearchOutcome::Partial {
                                    reason,
                                    retry_after,
                                    ..
                                } => IndexerSearchOutcome::Partial {
                                    empty: true,
                                    reason,
                                    retry_after,
                                },
                                other => other,
                            };
                            if let Some(page) = &mut outcome.page {
                                page.next_cursor = None;
                            }
                        }
                    }
                }
                let outcome = response
                    .indexer_outcomes
                    .iter()
                    .find(|outcome| outcome.indexer_id == source.source.id);
                let mut page = outcome
                    .and_then(|outcome| outcome.page.clone())
                    .unwrap_or_else(|| response.page_metadata());
                source.outcome =
                    Some(outcome.map(|outcome| outcome.outcome).unwrap_or_else(|| {
                        match page.completion {
                            crate::IndexerSearchCompletion::Complete => {
                                IndexerSearchOutcome::Complete {
                                    empty: response.results.is_empty(),
                                }
                            }
                            crate::IndexerSearchCompletion::Partial {
                                reason,
                                retry_after,
                            } => IndexerSearchOutcome::Partial {
                                empty: response.results.is_empty(),
                                reason,
                                retry_after,
                            },
                        }
                    }));
                source.succeeded |= !response.results.is_empty()
                    || matches!(source.outcome, Some(IndexerSearchOutcome::Complete { .. }));
                source.retry_after = match source.outcome {
                    Some(
                        IndexerSearchOutcome::Partial { retry_after, .. }
                        | IndexerSearchOutcome::Deferred { retry_after }
                        | IndexerSearchOutcome::Skipped { retry_after },
                    ) => retry_after,
                    Some(IndexerSearchOutcome::BackedOff { until }) => {
                        until.map(|until| (until - chrono::Utc::now()).to_std().unwrap_or_default())
                    }
                    _ => None,
                };
                let continuable = matches!(
                    page.completion,
                    crate::IndexerSearchCompletion::Complete
                        | crate::IndexerSearchCompletion::Partial {
                            reason: None
                                | Some(
                                    crate::IndexerSearchIncompleteReason::PageCeilingReached
                                        | crate::IndexerSearchIncompleteReason::Unattested
                                ),
                            ..
                        }
                );
                source.stopped = !continuable
                    || matches!(
                        source.outcome,
                        Some(
                            IndexerSearchOutcome::Deferred { .. }
                                | IndexerSearchOutcome::Skipped { .. }
                                | IndexerSearchOutcome::BackedOff { .. }
                                | IndexerSearchOutcome::Unsupported
                                | IndexerSearchOutcome::Errored
                        )
                    );
                // Apply the caller's authorization scope before positions are
                // assigned. A filtered page still advances its source cursor;
                // the normal refill loop fills the aggregate page afterward.
                response
                    .results
                    .retain(|result| source.source.result_scope.allows(result));
                // Entire authorized pages and all attributes are admitted or refused.
                // Ordering is stable within equal publication timestamps.
                for result in &mut response.results {
                    if result.indexer_id.is_none() {
                        result.indexer_id = Some(source.source.id.clone());
                    }
                    for value in result.extra.values_mut() {
                        compact_json_objects(value);
                    }
                }
                // Keep the established exact-duplicate handling within a
                // source batch; never combine separate sources' attributes.
                response.results = dedupe_exact_duplicate_releases(response.results);
                response.results.sort_by(compare_raw_results);
                let bytes = size_of::<StoredPage>()
                    + 2 * size_of::<usize>()
                    + response.results.capacity() * size_of::<IndexerSearchResult>()
                    + response.results.iter().map(result_heap).sum::<usize>();
                if self.budget.reserve(bytes).is_none() {
                    while state.history_start < state.active_offset && !state.emitted.is_empty() {
                        state.emitted.pop_front();
                        state.history_start += 1;
                    }
                }
                let source = &mut state.sources[index];
                if let Some(charge) =
                    Self::reserve_reclaiming(&self.budget, bytes, self.registry.as_ref()).await
                {
                    source.position = 0;
                    source.delivered += response.results.len();
                    source.buffer = (!response.results.is_empty()).then(|| {
                        Arc::new(StoredPage {
                            rows: response.results,
                            _charge: charge,
                        })
                    });
                    if page.next_cursor.as_ref().is_some_and(|cursor| {
                        cursor.is_empty() || cursor.len() > crate::INDEXER_PAGE_CURSOR_MAX_BYTES
                    }) {
                        source.refusal_reason = Some(reasons::CURSOR_INVALID);
                        page.next_cursor = None;
                        if page.completion.is_complete() {
                            let reason =
                                Some(crate::IndexerSearchIncompleteReason::PageCeilingReached);
                            page.completion = crate::IndexerSearchCompletion::Partial {
                                reason,
                                retry_after: None,
                            };
                            source.outcome = Some(IndexerSearchOutcome::Partial {
                                empty: source.buffer.is_none(),
                                reason,
                                retry_after: None,
                            });
                        }
                    }
                    if let Some(cursor) = &page.next_cursor
                        && !source.visited.insert(cursor.clone())
                    {
                        source.refusal_reason = Some(reasons::CURSOR_REPEATED);
                    }
                    source.cursor = page.next_cursor.clone();
                    source.exhausted = page.completion.is_complete()
                        && source.cursor.is_none()
                        && matches!(source.outcome, Some(IndexerSearchOutcome::Complete { .. }));
                    // Legacy sources cannot expose a continuation. Their
                    // complete response is the terminal batch they already own.
                    if !source.source.paged && page.completion.is_complete() {
                        source.exhausted =
                            matches!(source.outcome, Some(IndexerSearchOutcome::Complete { .. }));
                    }
                    source.stopped |=
                        !source.exhausted && (!source.source.paged || source.cursor.is_none());
                    source.stopped |= source.refusal_reason.is_some();
                    source.page = Some(page);
                } else {
                    source.refusal_reason = Some(reasons::RETAINED_STATE_FULL);
                }
            }
        }
        let bytes = state_bytes(&state);
        if !self.resize_reclaiming(&mut state.charge, bytes).await {
            // Do not retain uncharged cursor storage. No terminal claim and
            // no truncated tail: refuse the generation on its next read.
            let source = &mut state.sources[index];
            source.refusal_reason = Some(reasons::RETAINED_STATE_FULL);
            source.cursor = None;
            source.visited = HashSet::new();
            source.page_fingerprints = HashSet::new();
            source.page = None;
            let bytes = state_bytes(&state);
            let _ = state.charge.resize(bytes);
        }
        drop(state);
        self.changed.notify_waiters();
    }

    async fn merge_available(
        &self,
        state: &mut State,
        offset: usize,
        end: usize,
        force: bool,
    ) -> AppResult<()> {
        if state
            .sources
            .iter()
            .any(|source| source.refusal_reason == Some(reasons::RETAINED_STATE_FULL))
        {
            return Err(resource_refusal());
        }
        while state.position < end {
            // Unknown or emptied pending sources may still outrank every
            // buffered head. Only the response deadline permits skipping them.
            if !force
                && state
                    .sources
                    .iter()
                    .any(|source| source.pending && source.buffer.is_none())
            {
                break;
            }
            let best = state
                .sources
                .iter()
                .enumerate()
                .filter_map(|(index, source)| {
                    source
                        .buffer
                        .as_ref()
                        .map(|page| (index, &page.rows[source.position]))
                })
                .min_by(|(left_index, left), (right_index, right)| {
                    compare_raw_results(left, right).then_with(|| {
                        state.sources[*left_index]
                            .source
                            .id
                            .cmp(&state.sources[*right_index].source.id)
                    })
                })
                .map(|(index, _)| index);
            let Some(best) = best else {
                break;
            };
            if state.position >= offset && state.emitted.len() == state.emitted.capacity() {
                let old = state_bytes(state);
                let additional = RAW_SEARCH_PAGE_SIZE * size_of::<RawSearchRow>();
                if !self
                    .resize_reclaiming(&mut state.charge, old.saturating_add(additional))
                    .await
                {
                    return Err(resource_refusal());
                }
                state
                    .emitted
                    .try_reserve_exact(RAW_SEARCH_PAGE_SIZE)
                    .map_err(|_| resource_refusal())?;
                let bytes = state_bytes(state);
                if !self.resize_reclaiming(&mut state.charge, bytes).await {
                    return Err(resource_refusal());
                }
            }
            let source = &mut state.sources[best];
            let page = source
                .buffer
                .as_ref()
                .expect("selected buffered source")
                .clone();
            if state.position >= offset {
                state.emitted.push_back(RawSearchRow {
                    page: page.clone(),
                    position: source.position,
                });
            } else {
                state.emitted.clear();
                state.history_start = state.position + 1;
            }
            state.position += 1;
            while state.emitted.len() > 2 * RAW_SEARCH_PAGE_SIZE {
                state.emitted.pop_front();
                state.history_start += 1;
            }
            source.position += 1;
            if source.position == page.rows.len() {
                source.buffer = None;
                // Refill the just-consumed source before committing older
                // heads. Late pages append to immutable emitted positions.
                if !force && !source.exhausted && !source.stopped && source.refusal_reason.is_none()
                {
                    break;
                }
            }
        }
        Ok(())
    }

    async fn snapshot_page(
        &self,
        state: &State,
        offset: usize,
        end: usize,
        deadline_reached: bool,
    ) -> AppResult<RawSearchPage> {
        if !state.sources.is_empty()
            && !state
                .sources
                .iter()
                .any(|source| source.succeeded || source.pending || !source.attempted)
        {
            let limited = !state.sources.is_empty() && state.sources.iter().all(|source| matches!(source.outcome,
                Some(IndexerSearchOutcome::Deferred { .. } | IndexerSearchOutcome::Skipped { .. }
                    | IndexerSearchOutcome::BackedOff { .. }
                    | IndexerSearchOutcome::Partial { reason: Some(crate::IndexerSearchIncompleteReason::RateLimited
                        | crate::IndexerSearchIncompleteReason::QueryBudgetExhausted), .. })));
            return Err(AppError::temporary_unavailable_reason(
                if limited {
                    reasons::ALL_SOURCES_LIMITED
                } else {
                    reasons::NO_SOURCE_SUCCEEDED
                },
                "No indexer could complete this raw search",
                state
                    .sources
                    .iter()
                    .filter_map(|source| source.retry_after)
                    .min(),
            ));
        }
        if offset < state.history_start {
            return Err(AppError::validation_refused(
                reasons::HISTORY_EXPIRED,
                "This earlier raw search page must be rebuilt".to_owned(),
            ));
        }
        let start = offset
            .saturating_sub(state.history_start)
            .min(state.emitted.len());
        let end = end
            .saturating_sub(state.history_start)
            .min(state.emitted.len());
        let header_bytes = size_of::<RawSearchPage>() + 2 * size_of::<usize>();
        let mut charge = Self::reserve_reclaiming(
            &self.budget,
            header_bytes
                + (end - start) * size_of::<RawSearchRow>()
                + state.sources.len() * size_of::<RawSearchSourceProgress>()
                + state
                    .sources
                    .iter()
                    .map(|source| {
                        source.source.id.len()
                            + source
                                .page
                                .as_ref()
                                .and_then(|page| page.next_cursor.as_ref())
                                .map_or(0, String::len)
                    })
                    .sum::<usize>(),
            self.registry.as_ref(),
        )
        .await
        .ok_or_else(resource_refusal)?;
        let rows = state.emitted.range(start..end).cloned().collect::<Vec<_>>();
        let sources = state
            .sources
            .iter()
            .map(|source| RawSearchSourceProgress {
                indexer_id: source.source.id.clone(),
                pending: source.pending || !source.attempted,
                exhausted: source.exhausted,
                stopped: source.stopped,
                retry_after: source.retry_after,
                outcome: source.outcome,
                page: source.page.clone(),
                refusal_reason: source.refusal_reason,
                elapsed_ms: source.elapsed_ms,
                delivered: source.delivered,
            })
            .collect::<Vec<_>>();
        let bytes = header_bytes
            + rows.capacity() * size_of::<RawSearchRow>()
            + sources.capacity() * size_of::<RawSearchSourceProgress>()
            + sources
                .iter()
                .map(|source| {
                    source.indexer_id.capacity()
                        + source
                            .page
                            .as_ref()
                            .and_then(|page| page.next_cursor.as_ref())
                            .map_or(0, String::capacity)
                })
                .sum::<usize>();
        if !self.resize_reclaiming(&mut charge, bytes).await {
            return Err(resource_refusal());
        }
        Ok(RawSearchPage {
            offset,
            rows,
            sources,
            deadline_reached,
            exhausted: state
                .sources
                .iter()
                .all(|source| source.exhausted && source.buffer.is_none())
                && end == state.emitted.len(),
            terminal: state
                .sources
                .iter()
                .all(|source| (source.exhausted || source.stopped) && source.buffer.is_none())
                && end == state.emitted.len(),
            registry: self.registry.clone(),
            _charge: charge,
        })
    }
}
