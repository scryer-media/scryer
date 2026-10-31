use super::*;
use crate::{IndexerClient, IndexerSearchCompletion, IndexerSearchResponse, RawTextSearchRequest};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

struct ControlledClient(mpsc::UnboundedSender<oneshot::Sender<()>>);

#[async_trait::async_trait]
impl IndexerClient for ControlledClient {
    async fn search_raw_text(
        &self,
        _: RawTextSearchRequest,
        _: CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        let (send, receive) = oneshot::channel();
        self.0.send(send).unwrap();
        tokio::time::timeout(Duration::from_secs(120), receive)
            .await
            .unwrap()
            .unwrap();
        Ok(IndexerSearchResponse {
            results: Vec::new(),
            completion: IndexerSearchCompletion::Complete,
            next_cursor: None,
            indexer_outcomes: Vec::new(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

fn principal(id: &str) -> SearchPrincipal {
    SearchPrincipal::IndexerClient {
        client_id: id.into(),
        key_generation: 1,
        grants_generation: "1".into(),
    }
}

fn sources() -> Vec<RawSearchSource> {
    vec![RawSearchSource {
        id: "source".into(),
        paged: true,
        result_scope: Default::default(),
        request: RawTextSearchRequest {
            query: "query".into(),
            ..Default::default()
        },
    }]
}

fn fixture() -> (AppUseCase, mpsc::UnboundedReceiver<oneshot::Sender<()>>) {
    let (mut app, _) = crate::lib_tests::bootstrap();
    let (send, receive) = mpsc::unbounded_channel();
    app.services.integrations.indexer_client = Arc::new(ControlledClient(send));
    (app, receive)
}

#[tokio::test(start_paused = true)]
async fn initial_reads_join_then_new_offset_zero_replaces_without_cancelling_old_reads() {
    let (app, _) = fixture();
    let (id, first) = app
        .scoped_raw_generation(principal("a"), "query".into(), sources(), 0)
        .await
        .unwrap();
    let (same_id, joined) = app
        .scoped_raw_generation(principal("a"), "query".into(), sources(), 0)
        .await
        .unwrap();
    assert_eq!(id, same_id);
    assert!(Arc::ptr_eq(&first, &joined));
    app.runtime
        .acquisition
        .interactive_release_searches
        .lock()
        .await
        .get_mut(&id)
        .unwrap()
        .snapshot
        .state = InteractiveReleaseSearchState::Completed;
    let (continued_id, continued) = app
        .scoped_raw_generation(principal("a"), "query".into(), sources(), 100)
        .await
        .unwrap();
    assert_eq!(continued_id, id);
    assert!(Arc::ptr_eq(&continued, &first));
    let (new_id, fresh) = app
        .scoped_raw_generation(principal("a"), "query".into(), sources(), 0)
        .await
        .unwrap();
    assert_ne!(new_id, id);
    assert!(!Arc::ptr_eq(&first, &fresh));
    let entries = app
        .runtime
        .acquisition
        .interactive_release_searches
        .lock()
        .await;
    assert_eq!(entries.len(), 1);
    assert!(!entries[&new_id].cancel.is_cancelled());
    drop(entries);
    // A retained old reader remains usable after lookup replacement.
    assert!(first.source_progress().await[0].pending);
}

#[tokio::test(start_paused = true)]
async fn principal_key_grant_and_source_configuration_isolate_cached_generations() {
    let (app, _) = fixture();
    let (id, _) = app
        .scoped_raw_generation(principal("a"), "config-a".into(), sources(), 100)
        .await
        .unwrap();
    let mut rotated = principal("a");
    if let SearchPrincipal::IndexerClient { key_generation, .. } = &mut rotated {
        *key_generation += 1;
    }
    let mut regranted = principal("a");
    if let SearchPrincipal::IndexerClient {
        grants_generation, ..
    } = &mut regranted
    {
        *grants_generation = "2".into();
    }
    for (owner, key) in [
        (principal("b"), "config-a"),
        (rotated, "config-a"),
        (regranted, "config-a"),
        (principal("a"), "config-b"),
    ] {
        let (other, _) = app
            .scoped_raw_generation(owner, key.into(), sources(), 100)
            .await
            .unwrap();
        assert_ne!(id, other);
    }
    assert_eq!(
        app.runtime
            .acquisition
            .interactive_release_searches
            .lock()
            .await
            .len(),
        5
    );
}

#[tokio::test(start_paused = true)]
async fn expired_and_evicted_offsets_rebuild_and_completed_initial_reads_do_not_stay_joinable() {
    let (app, _) = fixture();
    let (first, _) = app
        .scoped_raw_generation(principal("a"), "query".into(), sources(), 100)
        .await
        .unwrap();
    tokio::time::advance(raw::RAW_SEARCH_IDLE_TTL).await;
    let (second, _) = app
        .scoped_raw_generation(principal("a"), "query".into(), sources(), 100)
        .await
        .unwrap();
    assert_ne!(first, second);
    app.runtime
        .acquisition
        .interactive_release_searches
        .lock()
        .await
        .clear();
    let (third, _) = app
        .scoped_raw_generation(principal("a"), "query".into(), sources(), 100)
        .await
        .unwrap();
    assert_ne!(second, third);
}

#[tokio::test(start_paused = true)]
async fn expiry_cancels_a_pending_source_and_releases_its_retained_budget() {
    struct PendingClient(mpsc::UnboundedSender<oneshot::Sender<()>>);
    #[async_trait::async_trait]
    impl IndexerClient for PendingClient {
        async fn search_raw_text(
            &self,
            _: RawTextSearchRequest,
            cancel: CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            let (send, receive) = oneshot::channel();
            self.0.send(send).unwrap();
            // A waiting source acknowledges cancellation before returning.
            tokio::select! {
                _ = cancel.cancelled() => Err(AppError::canceled("source cancelled")),
                _ = receive => unreachable!("this fixture only stops on cancellation"),
            }
        }
    }
    let (mut app, _) = crate::lib_tests::bootstrap();
    let (send, mut reads) = mpsc::unbounded_channel();
    app.services.integrations.indexer_client = Arc::new(PendingClient(send));
    let budget = app.runtime.acquisition.raw_search_budget.clone();
    let (id, generation) = app
        .scoped_raw_generation(principal("a"), "query".into(), sources(), 0)
        .await
        .unwrap();
    let weak = Arc::downgrade(&generation);
    let reader = tokio::spawn(async move { generation.page(0, 100).await });
    let mut pending = tokio::time::timeout(Duration::from_secs(60), reads.recv())
        .await
        .unwrap()
        .unwrap();
    tokio::time::advance(raw::RAW_SEARCH_RESPONSE_DEADLINE).await;
    let page = tokio::time::timeout(Duration::from_secs(60), reader)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(page);
    assert!(budget.used() > 0);
    assert!(!pending.is_closed());
    tokio::time::advance(raw::RAW_SEARCH_IDLE_TTL).await;
    {
        let mut entries = app
            .runtime
            .acquisition
            .interactive_release_searches
            .lock()
            .await;
        evict_stale_entries(&mut entries, app.runtime.environment.now());
        assert!(!entries.contains_key(&id));
    }
    // A continuously-ready yield loop prevents paused time from advancing.
    // Resume only the failure clock after the explicit expiry trigger.
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(60), pending.closed())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        while weak.upgrade().is_some() || budget.used() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(budget.used(), 0);
}

#[tokio::test(start_paused = true)]
async fn restore_tracks_disconnected_scoped_drivers_and_late_source_cleanup() {
    let (app, mut reads) = fixture();
    let worker = app.clone();
    let caller = tokio::spawn(async move {
        worker
            .search_scoped_raw_page(principal("a"), "query".into(), sources(), 0, 100)
            .await
    });
    let complete = tokio::time::timeout(Duration::from_secs(60), reads.recv())
        .await
        .unwrap()
        .unwrap();
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let deadline = crate::RestoreDeadline::new();
    let drain = app.restore_coordinator().drain(&deadline);
    tokio::pin!(drain);
    assert!(futures_util::poll!(&mut drain).is_pending());
    assert!(!complete.is_closed());
    complete.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(60), drain)
        .await
        .unwrap()
        .unwrap();
    let entries = app
        .runtime
        .acquisition
        .interactive_release_searches
        .lock()
        .await;
    assert!(entries.values().all(|entry| entry.cancel.is_cancelled()
        && entry.snapshot.state == InteractiveReleaseSearchState::Completed));
    assert!(reads.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn actual_page_completion_ends_join_window_and_later_offset_reuses_the_completed_source() {
    let (app, mut reads) = fixture();
    let worker = app.clone();
    let task = tokio::spawn(async move {
        worker
            .search_scoped_raw_page(principal("a"), "query".into(), sources(), 0, 100)
            .await
    });
    tokio::time::timeout(Duration::from_secs(60), reads.recv())
        .await
        .unwrap()
        .unwrap()
        .send(())
        .unwrap();
    let page = tokio::time::timeout(Duration::from_secs(90), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(page.terminal && page.exhausted);
    let entries = app
        .runtime
        .acquisition
        .interactive_release_searches
        .lock()
        .await;
    let (first_id, entry) = entries.iter().next().unwrap();
    let first_id = first_id.clone();
    assert_eq!(
        entry.snapshot.state,
        InteractiveReleaseSearchState::Completed
    );
    drop(entries);
    let later = app
        .search_scoped_raw_page(principal("a"), "query".into(), sources(), 100, 100)
        .await
        .unwrap();
    assert!(later.exhausted);
    assert!(reads.try_recv().is_err());
    let (fresh_id, _) = app
        .scoped_raw_generation(principal("a"), "query".into(), sources(), 0)
        .await
        .unwrap();
    assert_ne!(first_id, fresh_id);
}

#[tokio::test(start_paused = true)]
async fn page_driver_reclaims_idle_state_but_refuses_to_evict_an_active_generation() {
    const LIMIT: usize = 32 * 1024;
    for active in [false, true] {
        let (mut app, _) = fixture();
        let budget = Arc::new(memory::RawSearchBudget::new(LIMIT));
        app.runtime.acquisition.raw_search_budget = budget.clone();
        let (id, generation) = app
            .scoped_raw_generation(principal("old"), "x".repeat(8192), Vec::new(), 0)
            .await
            .unwrap();
        let protected = active.then_some(generation);
        let pressure = budget.reserve(LIMIT - budget.used()).unwrap();
        assert_eq!(budget.used(), LIMIT);
        let response = app
            .search_scoped_raw_page(principal("new"), "new-query".into(), Vec::new(), 0, 100)
            .await;
        if active {
            assert_eq!(
                response.unwrap_err().reason(),
                Some(reasons::RETAINED_STATE_FULL)
            );
            assert!(
                app.runtime
                    .acquisition
                    .interactive_release_searches
                    .lock()
                    .await
                    .contains_key(&id)
            );
        } else {
            assert!(response.unwrap().exhausted);
            assert!(
                !app.runtime
                    .acquisition
                    .interactive_release_searches
                    .lock()
                    .await
                    .contains_key(&id)
            );
        }
        assert!(budget.used() <= LIMIT);
        drop(protected);
        drop(pressure);
    }
}

#[tokio::test(start_paused = true)]
async fn internal_source_and_cached_page_admission_reclaim_idle_but_protect_active_state() {
    const LIMIT: usize = 32 * 1024;
    for cached in [false, true] {
        for active in [false, true] {
            let (mut app, mut reads) = fixture();
            let budget = Arc::new(memory::RawSearchBudget::new(LIMIT));
            app.runtime.acquisition.raw_search_budget = budget.clone();
            let (_, current) = app
                .scoped_raw_generation(principal("current"), "query".into(), sources(), 0)
                .await
                .unwrap();
            if cached {
                let generation = current.clone();
                let first = tokio::spawn(async move { generation.page(0, 100).await });
                tokio::time::timeout(Duration::from_secs(60), reads.recv())
                    .await
                    .unwrap()
                    .unwrap()
                    .send(())
                    .unwrap();
                let page = tokio::time::timeout(Duration::from_secs(90), first)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert!(page.exhausted);
                drop(page);
            }
            let (old_id, old) = app
                .scoped_raw_generation(principal("old"), "x".repeat(8192), Vec::new(), 0)
                .await
                .unwrap();
            let protected = active.then_some(old);
            // Both generations already exist. Pressure is first encountered
            // inside source dispatch or delivery of an already cached page.
            let pressure = budget.reserve(LIMIT - budget.used()).unwrap();
            let task = tokio::spawn(async move { current.page(0, 100).await });
            if !active && !cached {
                tokio::time::timeout(Duration::from_secs(60), reads.recv())
                    .await
                    .unwrap()
                    .unwrap()
                    .send(())
                    .unwrap();
            }
            let result = tokio::time::timeout(Duration::from_secs(90), task)
                .await
                .unwrap()
                .unwrap();
            if active {
                assert_eq!(
                    result.unwrap_err().reason(),
                    Some(reasons::RETAINED_STATE_FULL)
                );
            } else {
                assert!(result.unwrap().exhausted);
            }
            assert_eq!(
                app.runtime
                    .acquisition
                    .interactive_release_searches
                    .lock()
                    .await
                    .contains_key(&old_id),
                active
            );
            assert!(
                reads.try_recv().is_err(),
                "cached or refused reads cannot redispatch"
            );
            assert!(budget.used() <= LIMIT);
            drop(protected);
            drop(pressure);
        }
    }
}

#[tokio::test]
async fn initial_read_never_joins_a_running_offset_rebuild() {
    let (app, _reads) = fixture();
    let (rebuild_id, _) = app
        .scoped_raw_generation(principal("a"), "same".into(), sources(), 100)
        .await
        .unwrap();
    let (initial_id, _) = app
        .scoped_raw_generation(principal("a"), "same".into(), sources(), 0)
        .await
        .unwrap();
    assert_ne!(initial_id, rebuild_id);
    let (joined_id, _) = app
        .scoped_raw_generation(principal("a"), "same".into(), sources(), 0)
        .await
        .unwrap();
    assert_eq!(initial_id, joined_id);
}
