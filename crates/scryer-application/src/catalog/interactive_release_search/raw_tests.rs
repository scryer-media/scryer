use super::*;
use crate::{IndexerResponseAttributes, IndexerSearchCompletion, IndexerSearchResponse};
use tokio::sync::{mpsc, oneshot};

type Read = (RawTextSearchRequest, oneshot::Sender<IndexerSearchResponse>);

struct ControlledClient(mpsc::UnboundedSender<Read>);

#[async_trait::async_trait]
impl IndexerClient for ControlledClient {
    async fn search_raw_text(
        &self,
        request: RawTextSearchRequest,
        _: CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        let (sender, receiver) = oneshot::channel();
        self.0.send((request, sender)).unwrap();
        Ok(tokio::time::timeout(Duration::from_secs(120), receiver)
            .await
            .unwrap()
            .unwrap())
    }
}

fn row(id: usize) -> IndexerSearchResult {
    IndexerSearchResult {
        indexer_id: None,
        source: "source".into(),
        title: id.to_string(),
        link: None,
        download_url: Some(format!("https://indexer.example/{id}")),
        source_kind: None,
        size_bytes: None,
        published_at: Some(
            chrono::DateTime::from_timestamp(2_000_000_000 - id as i64, 0)
                .unwrap()
                .to_rfc3339(),
        ),
        thumbs_up: None,
        thumbs_down: None,
        indexer_languages: None,
        indexer_subtitles: None,
        indexer_grabs: None,
        password_hint: None,
        parsed_release_metadata: None,
        quality_profile_decision: None,
        extra: Default::default(),
        response_attributes: IndexerResponseAttributes::default(),
        guid: Some(id.to_string()),
        info_url: None,
        provenance: None,
        candidate_token: None,
        queue_scope: None,
        coverage_scope: None,
        auto_eligible: None,
        auto_decision_code: None,
        auto_decision_summary: None,
        release_listing_json: None,
        http: None,
    }
}

fn response(ids: impl IntoIterator<Item = usize>, cursor: Option<&str>) -> IndexerSearchResponse {
    IndexerSearchResponse {
        results: ids.into_iter().map(row).collect(),
        completion: IndexerSearchCompletion::Complete,
        next_cursor: cursor.map(str::to_owned),
        api_current: None,
        api_max: None,
        grab_current: None,
        grab_max: None,
        indexer_outcomes: Vec::new(),
    }
}

async fn generation(
    count: usize,
    budget: Arc<RawSearchBudget>,
) -> (Arc<RawSearchGeneration>, mpsc::UnboundedReceiver<Read>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let coordinator = Arc::new(crate::RestoreCoordinator::default());
    let sources = (0..count)
        .map(|id| RawSearchSource {
            id: id.to_string(),
            paged: true,
            result_scope: Default::default(),
            request: RawTextSearchRequest {
                query: "query".into(),
                limit: Some(100),
                ..Default::default()
            },
        })
        .collect();
    (
        RawSearchGeneration::new(
            sources,
            Arc::new(ControlledClient(sender)),
            budget,
            coordinator.shutdown_workers.child_token(),
            None,
            coordinator,
        )
        .await
        .unwrap(),
        receiver,
    )
}

async fn read(receiver: &mut mpsc::UnboundedReceiver<Read>) -> Read {
    tokio::time::timeout(Duration::from_secs(60), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}

fn request_page(
    generation: &Arc<RawSearchGeneration>,
    offset: usize,
    limit: usize,
) -> tokio::task::JoinHandle<AppResult<RawSearchPage>> {
    let generation = generation.clone();
    tokio::spawn(async move { generation.page(offset, limit).await })
}

async fn finish(task: tokio::task::JoinHandle<AppResult<RawSearchPage>>) -> RawSearchPage {
    tokio::time::timeout(Duration::from_secs(90), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

fn titles(page: &RawSearchPage) -> Vec<usize> {
    page.rows
        .iter()
        .map(|row| row.result().title.parse().unwrap())
        .collect()
}

#[tokio::test(start_paused = true)]
async fn empty_eligible_scope_returns_an_exhausted_page_without_source_requests() {
    let (generation, mut reads) = generation(0, Arc::new(RawSearchBudget::default())).await;
    let page = finish(request_page(&generation, 0, 100)).await;
    assert!(page.rows.is_empty());
    assert!(page.sources.is_empty());
    assert!(page.exhausted && page.terminal);
    assert!(!page.deadline_reached);
    assert!(reads.try_recv().is_err());
}

#[tokio::test]
async fn output_reservations_share_the_raw_pool_and_survive_the_page() {
    let budget = Arc::new(RawSearchBudget::new(32 * 1024));
    let (generation, _) = generation(0, budget.clone()).await;
    let page = finish(request_page(&generation, 0, 100)).await;
    let before = budget.used();
    let lease = page.reserve_output(16 * 1024).await.unwrap();
    assert!(budget.used() >= before + 16 * 1024);
    assert!(matches!(page.reserve_output(32 * 1024).await,
        Err(error) if error.reason() == Some(reasons::RETAINED_STATE_FULL)));
    let retained_bytes = lease.clone();
    drop(lease);
    drop(page);
    drop(generation);
    assert!(budget.used() >= 16 * 1024);
    drop(retained_bytes);
    assert_eq!(budget.used(), 0);
}

#[tokio::test(start_paused = true)]
async fn dominant_source_refills_before_older_heads_and_cache_keeps_tail() {
    let budget = Arc::new(RawSearchBudget::default());
    let (generation, mut reads) = generation(2, budget.clone()).await;
    let first = request_page(&generation, 0, 3);
    for _ in 0..2 {
        let (request, sender) = read(&mut reads).await;
        assert_eq!(request.limit, Some(100));
        assert_eq!(request.page.unwrap().cursor, None);
        if request.indexer_ids.unwrap().contains("0") {
            sender.send(response([0, 1], Some("next"))).unwrap();
        } else {
            sender.send(response([10, 11], None)).unwrap();
        }
    }
    let (request, sender) = read(&mut reads).await;
    assert_eq!(request.page.unwrap().cursor.as_deref(), Some("next"));
    sender.send(response([2, 3], None)).unwrap();
    let first = finish(first).await;
    assert_eq!(titles(&first), vec![0, 1, 2]);
    assert!(!first.exhausted);
    let second = finish(request_page(&generation, 3, 3)).await;
    assert_eq!(titles(&second), vec![3, 10, 11]);
    assert!(second.exhausted);
    assert!(reads.try_recv().is_err());
    drop(generation);
    assert!(
        budget.used() > 0,
        "response still owns its page reservation"
    );
    drop(first);
    drop(second);
    assert_eq!(budget.used(), 0);
}

#[tokio::test(start_paused = true)]
async fn fifty_sources_keep_their_own_page_size_and_all_unemitted_rows() {
    let (generation, mut reads) = generation(50, Arc::new(RawSearchBudget::default())).await;
    let first = request_page(&generation, 0, 100);
    for _ in 0..50 {
        let (request, sender) = read(&mut reads).await;
        assert_eq!(request.limit, Some(100));
        let id: usize = request
            .indexer_ids
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        sender
            .send(response((0..100).map(|row| row * 50 + id), None))
            .unwrap();
    }
    assert_eq!(titles(&finish(first).await), (0..100).collect::<Vec<_>>());
    let last = finish(request_page(&generation, 4900, 100)).await;
    assert_eq!(titles(&last), (4900..5000).collect::<Vec<_>>());
    assert!(last.exhausted);
    assert!(reads.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn deadline_partial_does_not_exhaust_late_source_or_move_emitted_rows() {
    let (generation, mut reads) = generation(2, Arc::new(RawSearchBudget::default())).await;
    let first = request_page(&generation, 0, 2);
    let (_, late) = read(&mut reads).await;
    let (_, fast) = read(&mut reads).await;
    fast.send(response([100], None)).unwrap();
    tokio::time::advance(RAW_SEARCH_RESPONSE_DEADLINE).await;
    let first = finish(first).await;
    assert_eq!(titles(&first), vec![100]);
    assert!(first.deadline_reached);
    assert!(!first.exhausted);
    assert_eq!(
        first.sources.iter().filter(|source| source.pending).count(),
        1
    );
    late.send(response([0, 1], None)).unwrap();
    let next = finish(request_page(&generation, 1, 2)).await;
    assert_eq!(titles(&next), vec![0, 1]);
    assert!(next.exhausted);
    assert_eq!(
        titles(&finish(request_page(&generation, 0, 1)).await),
        vec![100]
    );
}

#[tokio::test(start_paused = true)]
async fn dropped_consumer_does_not_cancel_generation_owned_read() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let first = request_page(&generation, 0, 1);
    let (_, sender) = read(&mut reads).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    sender.send(response([7], None)).unwrap();
    assert_eq!(
        titles(&finish(request_page(&generation, 0, 1)).await),
        vec![7]
    );
    assert!(reads.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn restore_refuses_outstanding_source_work_and_drains_late_completion() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let caller = request_page(&generation, 0, 1);
    let (_, complete) = read(&mut reads).await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let deadline = crate::RestoreDeadline::new();
    let drain = generation.coordinator.drain(&deadline);
    tokio::pin!(drain);
    assert!(futures_util::poll!(&mut drain).is_pending());
    assert!(generation.cancel.is_cancelled());
    tokio::time::advance(crate::RESTORE_WORK_BUDGET).await;
    assert!(drain.await.is_err());
    // A source ignoring cancellation still owns its cleanup. The coordinator
    // cannot allow restore merely because its page consumer disconnected.
    complete.send(response([7], Some("more"))).unwrap();
    tokio::time::timeout(
        Duration::from_secs(60),
        generation.coordinator.drain(&crate::RestoreDeadline::new()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!generation.state.lock().await.sources[0].pending);
    assert!(generation.page(0, 1).await.is_err());
    assert!(reads.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn dominant_refill_has_its_own_response_bound_and_retains_pending_cursor() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let first = request_page(&generation, 0, 1);
    read(&mut reads)
        .await
        .1
        .send(response([0], Some("next")))
        .unwrap();
    assert_eq!(titles(&finish(first).await), vec![0]);
    let second = request_page(&generation, 1, 1);
    let (request, sender) = read(&mut reads).await;
    assert_eq!(request.page.unwrap().cursor.as_deref(), Some("next"));
    tokio::time::advance(RAW_SEARCH_RESPONSE_DEADLINE).await;
    let second = finish(second).await;
    assert!(second.rows.is_empty());
    assert!(!second.exhausted);
    assert!(second.deadline_reached && second.sources[0].pending);
    sender.send(response([1], None)).unwrap();
    assert_eq!(
        titles(&finish(request_page(&generation, 1, 1)).await),
        vec![1]
    );
}

#[tokio::test(start_paused = true)]
async fn whole_oversized_page_is_refused_without_claiming_source_exhaustion() {
    let budget = Arc::new(RawSearchBudget::new(32 * 1024));
    let (generation, mut reads) = generation(1, budget.clone()).await;
    let first = request_page(&generation, 0, 1);
    let (_, sender) = read(&mut reads).await;
    let mut answer = response([0, 1], None);
    answer.results[1].password_hint = Some("x".repeat(64 * 1024));
    sender.send(answer).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(90), first)
        .await
        .unwrap()
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(
        error.validation_reason(),
        Some(reasons::RETAINED_STATE_FULL)
    );
    assert!(!generation.state.lock().await.sources[0].exhausted);
    assert!(budget.used() <= 32 * 1024);
    drop(generation);
    assert_eq!(budget.used(), 0);
}

#[tokio::test(start_paused = true)]
async fn partial_page_ceiling_cursors_fill_the_same_page_and_deep_offsets() {
    let budget = Arc::new(RawSearchBudget::new(256 * 1024));
    let (generation, mut reads) = generation(1, budget.clone()).await;
    let page = request_page(&generation, 30, 2);
    for index in 0..32 {
        let (request, sender) = read(&mut reads).await;
        assert_eq!(
            request.page.unwrap().cursor,
            (index > 0).then(|| index.to_string())
        );
        let mut answer = response(
            [index],
            (index < 31).then(|| (index + 1).to_string()).as_deref(),
        );
        answer.results[0].password_hint = Some("x".repeat(64 * 1024));
        if index < 31 {
            answer.completion = IndexerSearchCompletion::Partial {
                reason: Some(crate::IndexerSearchIncompleteReason::PageCeilingReached),
                retry_after: None,
            };
        }
        sender.send(answer).unwrap();
    }
    let page = finish(page).await;
    assert_eq!(titles(&page), vec![30, 31]);
    assert!(page.terminal && page.exhausted);
    assert_eq!(generation.state.lock().await.emitted.len(), 2);
    assert!(budget.used() <= 256 * 1024);
    let old = generation.page(0, 1).await.err().unwrap();
    assert_eq!(old.validation_reason(), Some(reasons::HISTORY_EXPIRED));
}

#[tokio::test(start_paused = true)]
async fn unattested_sources_keep_their_existing_continuation_without_claiming_exhaustion() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let page = request_page(&generation, 0, 2);
    for index in 0..2 {
        let (_, sender) = read(&mut reads).await;
        let mut answer = response([index], (index == 0).then_some("next"));
        answer.completion = IndexerSearchCompletion::Partial {
            reason: Some(crate::IndexerSearchIncompleteReason::Unattested),
            retry_after: None,
        };
        sender.send(answer).unwrap();
    }
    let page = finish(page).await;
    assert_eq!(titles(&page), vec![0, 1]);
    assert!(page.terminal && !page.exhausted);
    assert!(matches!(
        page.sources[0].outcome,
        Some(IndexerSearchOutcome::Partial {
            reason: Some(crate::IndexerSearchIncompleteReason::Unattested),
            ..
        })
    ));
}

fn limited(retry: u64) -> IndexerSearchResponse {
    let mut answer = response([], None);
    answer.completion = IndexerSearchCompletion::Partial {
        reason: Some(crate::IndexerSearchIncompleteReason::RateLimited),
        retry_after: Some(Duration::from_secs(retry)),
    };
    answer
}

#[tokio::test(start_paused = true)]
async fn healthy_empty_source_makes_limited_partial_successful_without_retrying() {
    let (generation, mut reads) = generation(2, Arc::new(RawSearchBudget::default())).await;
    let page = request_page(&generation, 0, 2);
    read(&mut reads).await.1.send(limited(30)).unwrap();
    read(&mut reads).await.1.send(response([], None)).unwrap();
    let page = finish(page).await;
    assert!(page.rows.is_empty() && page.terminal && !page.exhausted);
    assert!(
        page.sources
            .iter()
            .any(|source| source.stopped && source.retry_after == Some(Duration::from_secs(30)))
    );
    assert!(finish(request_page(&generation, 2, 2)).await.terminal);
    assert!(reads.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn all_limited_is_coded_failure_with_earliest_retry() {
    let (generation, mut reads) = generation(2, Arc::new(RawSearchBudget::default())).await;
    let page = request_page(&generation, 0, 2);
    read(&mut reads).await.1.send(limited(50)).unwrap();
    read(&mut reads).await.1.send(limited(20)).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(90), page)
        .await
        .unwrap()
        .unwrap()
        .err()
        .unwrap();
    assert!(matches!(error, AppError::TemporaryUnavailable {
        reason: Some(reasons::ALL_SOURCES_LIMITED), retry_after: Some(retry), ..
    } if retry == Duration::from_secs(20)));
}

#[tokio::test(start_paused = true)]
async fn cached_results_survive_refill_quota_failure_and_end_the_generation() {
    let (generation, mut reads) = generation(2, Arc::new(RawSearchBudget::default())).await;
    let page = request_page(&generation, 0, 3);
    for _ in 0..2 {
        let (request, sender) = read(&mut reads).await;
        if request.indexer_ids.unwrap().contains("0") {
            sender.send(response([0], Some("next"))).unwrap();
        } else {
            sender.send(response([10, 11], None)).unwrap();
        }
    }
    read(&mut reads).await.1.send(limited(60)).unwrap();
    let page = finish(page).await;
    assert_eq!(titles(&page), vec![0, 10, 11]);
    assert!(page.terminal && !page.exhausted);
    assert!(finish(request_page(&generation, 3, 3)).await.terminal);
    assert!(reads.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn concurrent_same_page_readers_share_the_source_read_and_positions() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let first = request_page(&generation, 0, 2);
    let second = request_page(&generation, 0, 2);
    read(&mut reads)
        .await
        .1
        .send(response([1, 2, 3], None))
        .unwrap();
    assert_eq!(titles(&finish(first).await), vec![1, 2]);
    assert_eq!(titles(&finish(second).await), vec![1, 2]);
    assert!(reads.try_recv().is_err());
    assert_eq!(
        titles(&finish(request_page(&generation, 2, 1)).await),
        vec![3]
    );
}

#[tokio::test(start_paused = true)]
async fn raw_expiry_uses_idle_access_and_not_initial_job_age() {
    let (generation, _) = generation(0, Arc::new(RawSearchBudget::default())).await;
    tokio::time::advance(Duration::from_secs(299)).await;
    assert!(!generation.idle_expired(Instant::now()));
    generation.touch();
    tokio::time::advance(Duration::from_secs(299)).await;
    assert!(!generation.idle_expired(Instant::now()));
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(generation.idle_expired(Instant::now()));
}

#[tokio::test(start_paused = true)]
async fn registry_eviction_keeps_outstanding_response_allocations_charged() {
    use super::super::{
        InteractiveReleaseSearchJobEntry, InteractiveReleaseSearchSnapshot,
        InteractiveReleaseSearchState, SearchPrincipal,
    };
    let limit = 64 * 1024;
    let budget = Arc::new(RawSearchBudget::new(limit));
    let (generation, mut reads) = generation(1, budget.clone()).await;
    let first = request_page(&generation, 0, 1);
    read(&mut reads).await.1.send(response([0], None)).unwrap();
    let page = finish(first).await;
    let registry = Arc::new(super::super::InteractiveReleaseSearchRegistry::default());
    registry.lock().await.insert(
        "search".into(),
        InteractiveReleaseSearchJobEntry {
            snapshot: InteractiveReleaseSearchSnapshot {
                id: "search".into(),
                state: InteractiveReleaseSearchState::Completed,
                results: Vec::new(),
                raw_page: None,
                raw_failure_reason: None,
                indexers: Vec::new(),
                started_at: chrono::Utc::now(),
                completed_at: Some(chrono::Utc::now()),
                limit: None,
            },
            principal: SearchPrincipal::User {
                user_id: "operator".into(),
            },
            scope_key: "scope".into(),
            cancel: CancellationToken::new(),
            raw: Some(generation),
            raw_page_request: 0,
            raw_starts_at_zero: true,
            _raw_registry_charge: None,
        },
    );
    assert!(
        RawSearchGeneration::reserve_reclaiming(&budget, limit, Some(&Arc::downgrade(&registry)))
            .await
            .is_none()
    );
    assert!(registry.lock().await.is_empty());
    assert!(budget.used() > 0);
    assert_eq!(titles(&page), vec![0]);
    drop(page);
    assert_eq!(budget.used(), 0);
    assert!(budget.reserve(limit).is_some());
}

#[tokio::test(start_paused = true)]
async fn discarded_history_rebuilds_through_source_machinery() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let last = request_page(&generation, 300, 1);
    read(&mut reads)
        .await
        .1
        .send(response(0..400, None))
        .unwrap();
    assert_eq!(titles(&finish(last).await), vec![300]);
    let original = generation.clone();
    let rebuilt = tokio::spawn(async move { original.page_or_rebuild(0, 2).await });
    let original = generation.clone();
    let concurrent = tokio::spawn(async move { original.page_or_rebuild(0, 2).await });
    let (request, sender) = read(&mut reads).await;
    assert_eq!(request.page.unwrap().cursor, None);
    sender.send(response([10, 11], None)).unwrap();
    let (new_generation, page) = tokio::time::timeout(Duration::from_secs(90), rebuilt)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!Arc::ptr_eq(&generation, &new_generation));
    assert_eq!(
        titles(&page),
        vec![10, 11],
        "rebuild reflects the current feed"
    );
    let (joined, joined_page) = tokio::time::timeout(Duration::from_secs(90), concurrent)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&new_generation, &joined));
    assert_eq!(titles(&joined_page), vec![10, 11]);
    assert!(reads.try_recv().is_err());
    assert_eq!(
        titles(&finish(request_page(&generation, 300, 1)).await),
        vec![300]
    );
}

#[tokio::test(start_paused = true)]
async fn rebuild_deadline_before_offset_is_empty_pending_and_continuable() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let last = request_page(&generation, 300, 1);
    read(&mut reads)
        .await
        .1
        .send(response(0..400, None))
        .unwrap();
    finish(last).await;
    let rebuilt = tokio::spawn(async move { generation.page_or_rebuild(100, 2).await });
    let (_, sender) = read(&mut reads).await;
    tokio::time::advance(RAW_SEARCH_RESPONSE_DEADLINE).await;
    let (generation, page) = tokio::time::timeout(Duration::from_secs(1), rebuilt)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(page.rows.is_empty() && page.deadline_reached && !page.terminal);
    assert!(page.sources[0].pending);
    sender.send(response(0..102, None)).unwrap();
    assert_eq!(
        titles(&finish(request_page(&generation, 100, 2)).await),
        vec![100, 101]
    );
}

#[tokio::test(start_paused = true)]
async fn repeated_source_cursor_stops_only_that_source_without_refetching() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let page = request_page(&generation, 0, 3);
    read(&mut reads)
        .await
        .1
        .send(response([0], Some("same")))
        .unwrap();
    read(&mut reads)
        .await
        .1
        .send(response([1], Some("same")))
        .unwrap();
    let page = finish(page).await;
    assert_eq!(titles(&page), vec![0, 1]);
    assert!(page.terminal && !page.exhausted);
    assert_eq!(
        page.sources[0].refusal_reason,
        Some(reasons::CURSOR_REPEATED)
    );
    assert!(reads.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn all_silent_sources_return_nonterminal_page_and_explicit_cancel_ends_reads() {
    let (generation, mut reads) = generation(2, Arc::new(RawSearchBudget::default())).await;
    let first = request_page(&generation, 0, 2);
    let (_, mut first_source) = read(&mut reads).await;
    let (_, mut second_source) = read(&mut reads).await;
    tokio::time::advance(RAW_SEARCH_RESPONSE_DEADLINE).await;
    let page = finish(first).await;
    assert!(page.rows.is_empty() && page.deadline_reached && !page.terminal);
    assert!(page.sources.iter().all(|source| source.pending));
    generation.cancel.cancel();
    tokio::time::timeout(Duration::from_secs(60), async {
        first_source.closed().await;
        second_source.closed().await;
    })
    .await
    .unwrap();
    assert_eq!(
        generation.page(0, 2).await.unwrap_err().validation_reason(),
        Some(reasons::CANCELLED)
    );
}

#[tokio::test(start_paused = true)]
async fn equal_dates_use_stable_source_identity_and_missing_dates_sort_last() {
    let (generation, mut reads) = generation(2, Arc::new(RawSearchBudget::default())).await;
    let first = request_page(&generation, 0, 4);
    for _ in 0..2 {
        let (request, sender) = read(&mut reads).await;
        let id: usize = request
            .indexer_ids
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let mut response = response([id + 2, id], None);
        response.results[0].published_at = None;
        response.results[1].published_at = row(0).published_at;
        sender.send(response).unwrap();
    }
    let first = finish(first).await;
    assert_eq!(titles(&first), vec![0, 1, 2, 3]);
    assert_eq!(
        titles(&finish(request_page(&generation, 0, 4)).await),
        vec![0, 1, 2, 3]
    );
    assert!(reads.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn a_newer_later_chunk_never_reorders_already_emitted_positions() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let first = request_page(&generation, 0, 2);
    read(&mut reads)
        .await
        .1
        .send(response([2, 3], Some("later")))
        .unwrap();
    let first = finish(first).await;
    assert_eq!(titles(&first), vec![2, 3]);
    let next = request_page(&generation, 2, 2);
    read(&mut reads)
        .await
        .1
        .send(response([1, 0], None))
        .unwrap();
    let next = finish(next).await;
    assert_eq!(titles(&next), vec![0, 1]);
    assert_eq!(titles(&first), vec![2, 3]);
    assert_eq!(
        titles(&finish(request_page(&generation, 0, 4)).await),
        vec![2, 3, 0, 1]
    );
}

#[tokio::test(start_paused = true)]
async fn authorization_filter_refills_before_assigning_positions_and_preserves_source_metadata() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    generation.state.lock().await.sources[0].source.result_scope =
        super::super::scope_filter::RawSearchResultScope {
            protocol: Some(scryer_domain::IndexerProtocolCapability::Usenet),
            allow_untyped_protocol: false,
            allow_uncategorized: false,
            categories: Some(vec!["4010".into(), "100123".into()]),
        };
    let first = request_page(&generation, 0, 2);
    let mut excluded = response([0, 1, 2], Some("authorized-next"));
    excluded.results[0].source_kind = Some(crate::DownloadSourceKind::TorrentFile);
    excluded.results[0].response_attributes.categories = vec!["4010".into()];
    excluded.results[1].source_kind = Some(crate::DownloadSourceKind::NzbUrl);
    excluded.results[1].response_attributes.categories = vec!["6000".into()];
    excluded.results[2].source_kind = Some(crate::DownloadSourceKind::NzbUrl);
    excluded.results[2].response_attributes.categories = vec!["4020".into()];
    read(&mut reads).await.1.send(excluded).unwrap();
    let (request, sender) = read(&mut reads).await;
    assert_eq!(
        request.page.unwrap().cursor.as_deref(),
        Some("authorized-next")
    );
    let mut allowed = response([3, 4], None);
    for (result, category) in allowed.results.iter_mut().zip(["4010", "100123"]) {
        result.source_kind = Some(crate::DownloadSourceKind::NzbUrl);
        result.response_attributes.categories = vec![category.into()];
        result.password_hint = Some("literal password".into());
        result
            .response_attributes
            .original
            .push(scryer_plugin_sdk::PluginReleaseAttribute {
                name: "category".into(),
                value: category.into(),
                namespace: Some("newznab".into()),
                namespace_uri: None,
                origin: scryer_plugin_sdk::PluginReleaseAttributeOrigin::Attribute,
            });
    }
    sender.send(allowed).unwrap();
    let page = finish(first).await;
    assert_eq!(page.offset, 0);
    assert_eq!(titles(&page), vec![3, 4]);
    assert!(page.exhausted && page.terminal);
    assert_eq!(
        page.rows[0].result().response_attributes.categories,
        ["4010"]
    );
    assert_eq!(
        page.rows[0].result().password_hint.as_deref(),
        Some("literal password")
    );
    assert_eq!(
        page.rows[0].result().response_attributes.original[0].value,
        "4010"
    );
    assert!(reads.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn changing_cursors_with_identical_pages_stops_without_duplicate_rows_or_more_queries() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let first = request_page(&generation, 0, 4);
    read(&mut reads)
        .await
        .1
        .send(response([0, 1], Some("cursor-a")))
        .unwrap();
    let (request, answer) = read(&mut reads).await;
    assert_eq!(request.page.unwrap().cursor.as_deref(), Some("cursor-a"));
    answer.send(response([0, 1], Some("cursor-b"))).unwrap();
    let page = finish(first).await;
    assert_eq!(titles(&page), [0, 1]);
    assert!(page.exhausted && page.terminal);
    assert!(
        finish(request_page(&generation, 2, 4))
            .await
            .rows
            .is_empty()
    );
    assert!(reads.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn malformed_cursors_keep_rows_but_stop_without_claiming_exhaustion() {
    for cursor in [
        String::new(),
        "x".repeat(crate::INDEXER_PAGE_CURSOR_MAX_BYTES + 1),
    ] {
        let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
        let page = request_page(&generation, 0, 100);
        read(&mut reads)
            .await
            .1
            .send(response([0, 1], Some(&cursor)))
            .unwrap();
        let page = finish(page).await;
        assert_eq!(titles(&page), [0, 1]);
        assert!(page.terminal && !page.exhausted);
        assert_eq!(
            page.sources[0].refusal_reason,
            Some(reasons::CURSOR_INVALID)
        );
        let metadata = page.sources[0].page.as_ref().unwrap();
        assert!(metadata.next_cursor.is_none());
        assert!(!metadata.completion.is_complete());
        assert!(reads.try_recv().is_err());
    }
}

#[tokio::test(start_paused = true)]
async fn repeated_content_preserves_a_refill_rate_limit_and_retry_delay() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let task = request_page(&generation, 0, 100);
    read(&mut reads)
        .await
        .1
        .send(response([0, 1], Some("cursor-a")))
        .unwrap();
    let mut refill = response([0, 1], Some("cursor-b"));
    refill.completion = IndexerSearchCompletion::Partial {
        reason: Some(crate::IndexerSearchIncompleteReason::RateLimited),
        retry_after: Some(Duration::from_secs(31)),
    };
    read(&mut reads).await.1.send(refill).unwrap();
    let page = finish(task).await;
    assert_eq!(titles(&page), [0, 1]);
    assert!(page.terminal && !page.exhausted);
    assert_eq!(page.sources[0].retry_after, Some(Duration::from_secs(31)));
    assert!(matches!(
        page.sources[0].outcome,
        Some(IndexerSearchOutcome::Partial {
            reason: Some(crate::IndexerSearchIncompleteReason::RateLimited),
            ..
        })
    ));
    assert!(reads.try_recv().is_err());
}

async fn released(budget: &RawSearchBudget) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while budget.used() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dropped_merge_waiter_keeps_buffered_positions_and_releases_its_state() {
    let budget = Arc::new(RawSearchBudget::default());
    let (generation, mut reads) = generation(1, budget.clone()).await;
    let first = request_page(&generation, 0, 1);
    read(&mut reads).await.1.send(response(0..4, None)).unwrap();
    assert_eq!(titles(&finish(first).await), [0]);
    let merge_guard = generation.advancement.lock().await;
    let mut waiting = Box::pin(generation.page(1, 2));
    // Poll exactly to the held merge lock, then simulate consumer cancellation.
    // The merge itself has no preemption point between emitted positions.
    assert!(
        std::future::Future::poll(
            waiting.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop()),
        )
        .is_pending()
    );
    drop(waiting);
    drop(merge_guard);
    assert_eq!(
        titles(&finish(request_page(&generation, 1, 2)).await),
        [1, 2]
    );
    assert!(reads.try_recv().is_err());
    drop(generation);
    released(&budget).await;
}

#[tokio::test]
async fn dropped_refill_reader_preserves_cursor_and_generation_owned_results() {
    let budget = Arc::new(RawSearchBudget::default());
    let (generation, mut reads) = generation(1, budget.clone()).await;
    let first = request_page(&generation, 0, 1);
    read(&mut reads)
        .await
        .1
        .send(response([0], Some("next")))
        .unwrap();
    assert_eq!(titles(&finish(first).await), [0]);
    let refill = request_page(&generation, 1, 2);
    let (request, answer) = read(&mut reads).await;
    assert_eq!(request.page.unwrap().cursor.as_deref(), Some("next"));
    refill.abort();
    assert!(refill.await.unwrap_err().is_cancelled());
    assert!(!answer.is_closed());
    answer.send(response([1, 2], None)).unwrap();
    assert_eq!(
        titles(&finish(request_page(&generation, 1, 2)).await),
        [1, 2]
    );
    assert!(reads.try_recv().is_err());
    drop(generation);
    released(&budget).await;
}

#[tokio::test]
async fn dropped_rebuild_reader_keeps_successor_until_completion_or_explicit_cancel() {
    for cancel in [false, true] {
        let budget = Arc::new(RawSearchBudget::default());
        let (generation, mut reads) = generation(1, budget.clone()).await;
        let last = request_page(&generation, 300, 1);
        read(&mut reads)
            .await
            .1
            .send(response(0..400, None))
            .unwrap();
        assert_eq!(titles(&finish(last).await), [300]);
        let original = generation.clone();
        let rebuild = tokio::spawn(async move { original.page_or_rebuild(0, 2).await });
        let (_, answer) = read(&mut reads).await;
        rebuild.abort();
        assert!(matches!(rebuild.await, Err(error) if error.is_cancelled()));
        let successor = generation.rebuild.lock().await.upgrade().unwrap();
        assert!(!answer.is_closed());
        if cancel {
            generation.cancel.cancel();
            assert!(
                !answer.is_closed(),
                "cancellation must not drop admitted source cleanup"
            );
            assert_eq!(
                successor.page(0, 2).await.unwrap_err().reason(),
                Some(reasons::CANCELLED)
            );
            answer.send(response([10, 11], None)).unwrap();
        } else {
            answer.send(response([10, 11], None)).unwrap();
            let (joined, page) =
                tokio::time::timeout(Duration::from_secs(60), generation.page_or_rebuild(0, 2))
                    .await
                    .unwrap()
                    .unwrap();
            assert!(Arc::ptr_eq(&joined, &successor));
            assert_eq!(titles(&page), [10, 11]);
        }
        assert!(reads.try_recv().is_err());
        drop(successor);
        drop(generation);
        released(&budget).await;
    }
}

#[tokio::test]
async fn dropped_merge_during_capacity_reclamation_keeps_each_committed_position() {
    const LIMIT: usize = 512 * 1024;
    let budget = Arc::new(RawSearchBudget::new(LIMIT));
    let registry = Arc::new(super::super::InteractiveReleaseSearchRegistry::default());
    let (mut generation, mut reads) = generation(1, budget.clone()).await;
    Arc::get_mut(&mut generation).unwrap().registry = Some(Arc::downgrade(&registry));
    let first = request_page(&generation, 0, 1);
    read(&mut reads)
        .await
        .1
        .send(response(0..150, None))
        .unwrap();
    assert_eq!(titles(&finish(first).await), [0]);
    let pressure = budget.reserve(LIMIT - budget.used()).unwrap();
    let registry_guard = registry.lock().await;
    let mut merging = Box::pin(generation.page(1, 100));
    assert!(
        std::future::Future::poll(
            merging.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop()),
        )
        .is_pending()
    );
    // The first allocation boundary after cached positions is the 101st row.
    // Cancellation while reclamation waits must leave prior positions intact.
    drop(merging);
    {
        let state = generation.state.lock().await;
        assert_eq!(state.position, 100);
        assert_eq!(state.sources[0].position, 100);
        assert_eq!(state.emitted.len(), 100);
    }
    drop(registry_guard);
    drop(pressure);
    assert_eq!(
        titles(&finish(request_page(&generation, 1, 100)).await),
        (1..101).collect::<Vec<_>>()
    );
    assert!(reads.try_recv().is_err());
    drop(generation);
    released(&budget).await;
}

#[tokio::test(start_paused = true)]
async fn backed_off_source_is_limited_and_exposes_its_retry_delay() {
    let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
    let page = request_page(&generation, 0, 100);
    let mut blocked = response([], None);
    blocked.indexer_outcomes.push(crate::IndexerQueryOutcome {
        indexer_id: "0".into(),
        outcome: IndexerSearchOutcome::BackedOff {
            until: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        },
        page: None,
    });
    read(&mut reads).await.1.send(blocked).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(90), page)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.reason(), Some(reasons::ALL_SOURCES_LIMITED));
    assert!(matches!(
        error,
        AppError::TemporaryUnavailable {
            retry_after: Some(_),
            ..
        }
    ));
}

#[tokio::test(start_paused = true)]
async fn touch_survives_source_state_lock_contention() {
    let (generation, _) = generation(0, Arc::new(RawSearchBudget::default())).await;
    let state = generation.state.lock().await;
    tokio::time::advance(RAW_SEARCH_IDLE_TTL).await;
    generation.touch();
    drop(state);
    assert!(!generation.idle_expired(Instant::now()));
}

#[tokio::test(start_paused = true)]
async fn capacity_refusal_can_rebuild_the_same_offset_after_capacity_returns() {
    let limit = 64 * 1024;
    let budget = Arc::new(RawSearchBudget::new(limit));
    let (generation, mut reads) = generation(1, budget.clone()).await;
    generation.state.lock().await.sources[0].refusal_reason = Some(reasons::RETAINED_STATE_FULL);
    let held = budget.reserve(limit - budget.used()).unwrap();
    let error = generation.page_or_rebuild(1, 1).await.err().unwrap();
    assert_eq!(error.reason(), Some(reasons::RETAINED_STATE_FULL));
    drop(held);
    let reader = tokio::spawn(async move { generation.page_or_rebuild(1, 1).await });
    read(&mut reads)
        .await
        .1
        .send(response([0, 1], None))
        .unwrap();
    let (_, page) = tokio::time::timeout(Duration::from_secs(90), reader)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(titles(&page), vec![1]);
}

#[test]
fn missing_categories_are_allowed_only_without_a_category_grant() {
    let result = row(0);
    let mut scope = super::super::scope_filter::RawSearchResultScope {
        categories: Some(vec!["4000".into()]),
        allow_uncategorized: true,
        ..Default::default()
    };
    assert!(scope.allows(&result));
    scope.allow_uncategorized = false;
    assert!(!scope.allows(&result));
}

#[tokio::test(start_paused = true)]
async fn page_limits_clamp_and_filter_refills_keep_nonterminal_pages_full() {
    for (requested, expected) in [(0, 1), (200, RAW_SEARCH_PAGE_SIZE)] {
        let (generation, mut reads) = generation(1, Arc::new(RawSearchBudget::default())).await;
        generation.state.lock().await.sources[0]
            .source
            .result_scope
            .categories = Some(vec!["4000".into()]);
        let task = request_page(&generation, 0, requested);
        let mut excluded = response([0], Some("filtered-next"));
        excluded.results[0].response_attributes.categories = vec!["6000".into()];
        read(&mut reads).await.1.send(excluded).unwrap();
        let (request, answer) = read(&mut reads).await;
        assert_eq!(
            request.page.unwrap().cursor.as_deref(),
            Some("filtered-next")
        );
        let mut allowed = response(1..=RAW_SEARCH_PAGE_SIZE + 1, Some("more"));
        for result in &mut allowed.results {
            result.response_attributes.categories = vec!["4000".into()];
        }
        answer.send(allowed).unwrap();
        let page = finish(task).await;
        assert_eq!(page.rows.len(), expected);
        assert!(!page.terminal && !page.exhausted);
    }
}

#[tokio::test(start_paused = true)]
async fn source_progress_counts_every_delivered_row_not_only_the_page_shown() {
    let (generation, mut reads) = generation(2, Arc::new(RawSearchBudget::default())).await;
    let first = request_page(&generation, 0, 2);
    let (_, older) = read(&mut reads).await;
    let (_, newer) = read(&mut reads).await;
    // Fifty newest rows from one source fill the whole first page; the other
    // source's older rows are retained but not yet shown.
    newer.send(response(0..50, None)).unwrap();
    older.send(response(100..130, None)).unwrap();
    let first = finish(first).await;
    assert_eq!(titles(&first), vec![0, 1]);
    let mut delivered = first
        .sources
        .iter()
        .map(|source| source.delivered)
        .collect::<Vec<_>>();
    delivered.sort_unstable();
    assert_eq!(delivered, vec![30, 50]);
    assert_eq!(
        titles(&finish(request_page(&generation, 48, 4)).await),
        vec![48, 49, 100, 101]
    );
    let mut delivered = generation
        .source_progress()
        .await
        .iter()
        .map(|source| source.delivered)
        .collect::<Vec<_>>();
    delivered.sort_unstable();
    assert_eq!(delivered, vec![30, 50]);
}
