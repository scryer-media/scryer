use super::*;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn clear_title_image_cache_collapses_duplicate_requests_and_waits_for_scans() {
    let (app, admin) = bootstrap_with_user_repo(Arc::new(MockUserRepo::default()));
    let title_images = Arc::new(BlockingTitleImageRepo::default());
    let proxy_cache = Arc::new(RecordingImageProxyCacheControl::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_title_images(title_images.clone())
            .with_image_proxy_cache_control(proxy_cache.clone())
    });

    let scan = app
        .runtime
        .library
        .library_scan_tracker
        .start_session(MediaFacet::Movie)
        .await
        .expect("scan should start");

    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("queue reset")
    );
    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("collapse queued reset")
    );

    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        title_images.clear_calls.load(Ordering::SeqCst),
        0,
        "cache clear should wait behind active library scans"
    );

    app.runtime
        .library
        .library_scan_tracker
        .fail_session(&scan.session_id)
        .await
        .expect("scan should finish");
    wait_for_title_image_clear_calls(&title_images, 1).await;

    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("collapse running reset")
    );
    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        title_images.clear_calls.load(Ordering::SeqCst),
        1,
        "running cache clear should collapse duplicate requests"
    );

    title_images.release_clear.notify_waiters();
    wait_for_title_image_cache_clear_idle(&app).await;
    assert_eq!(proxy_cache.clear_calls.load(Ordering::SeqCst), 1);

    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("queue reset after previous reset completes")
    );
    wait_for_title_image_clear_calls(&title_images, 2).await;
    title_images.release_clear.notify_waiters();
    wait_for_title_image_cache_clear_idle(&app).await;
    assert_eq!(proxy_cache.clear_calls.load(Ordering::SeqCst), 2);
}

#[derive(Default)]
struct RecordingImageProxyCacheControl {
    clear_calls: AtomicUsize,
}

#[async_trait]
impl ImageProxyCacheControl for RecordingImageProxyCacheControl {
    async fn clear_cache(&self) -> AppResult<()> {
        self.clear_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn set_configured_max_bytes(&self, _value: u64) -> AppResult<()> {
        Ok(())
    }
}

struct ChunkedTitleImageRepo {
    pending: Mutex<Vec<TitleImageSyncTask>>,
    list_failures: AtomicUsize,
    list_calls: AtomicUsize,
}

impl ChunkedTitleImageRepo {
    fn new(count: usize) -> Self {
        Self {
            pending: Mutex::new(
                (0..count)
                    .map(|index| TitleImageSyncTask {
                        title_id: format!("title-{index}"),
                        kind: TitleImageKind::Poster,
                        source_url: format!("https://example.test/poster-{index}.jpg"),
                        variants: Vec::new(),
                    })
                    .collect(),
            ),
            list_failures: AtomicUsize::new(0),
            list_calls: AtomicUsize::new(0),
        }
    }

    async fn pending_count(&self) -> usize {
        self.pending.lock().await.len()
    }
}

#[async_trait]
impl TitleImageRepository for ChunkedTitleImageRepo {
    async fn list_title_image_refresh_work(
        &self,
        limit: usize,
        skipped: &[TitleImageSyncTask],
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        if self
            .list_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return Err(AppError::Repository(
                "temporary artwork lookup failure".into(),
            ));
        }
        Ok(self
            .pending
            .lock()
            .await
            .iter()
            .filter(|task| {
                !skipped.iter().any(|skip| {
                    skip.title_id == task.title_id
                        && skip.kind == task.kind
                        && skip.source_url == task.source_url
                })
            })
            .take(limit)
            .cloned()
            .collect())
    }

    async fn clear_title_image_cache(&self) -> AppResult<()> {
        self.pending.lock().await.clear();
        Ok(())
    }

    async fn upsert_title_image_source_result(
        &self,
        title_id: &str,
        _result: TitleImageSourceResult,
        _event: Option<NewDomainEvent>,
    ) -> AppResult<Option<DomainEvent>> {
        self.pending
            .lock()
            .await
            .retain(|task| task.title_id != title_id);
        Ok(None)
    }

    async fn get_title_image_blob(
        &self,
        _title_id: &str,
        _kind: TitleImageKind,
        _variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        Ok(None)
    }
}

struct ChunkGatedTitleImageProcessor {
    started: AtomicUsize,
    first_chunk_gate: tokio::sync::Semaphore,
    second_chunk_gate: tokio::sync::Semaphore,
}

impl ChunkGatedTitleImageProcessor {
    fn new() -> Self {
        Self {
            started: AtomicUsize::new(0),
            first_chunk_gate: tokio::sync::Semaphore::new(0),
            second_chunk_gate: tokio::sync::Semaphore::new(0),
        }
    }
}

#[async_trait]
impl TitleImageProcessor for ChunkGatedTitleImageProcessor {
    async fn fetch_and_process_image(
        &self,
        kind: TitleImageKind,
        source_url: &str,
        _variants: Vec<TitleImageVariantSpec>,
    ) -> AppResult<TitleImageSourceResult> {
        const CHUNK_SIZE: usize = 4;

        let index = self.started.fetch_add(1, Ordering::SeqCst);
        let gate = if index < CHUNK_SIZE {
            &self.first_chunk_gate
        } else {
            &self.second_chunk_gate
        };
        gate.acquire()
            .await
            .expect("test image gate should remain open")
            .forget();

        Ok(TitleImageSourceResult {
            kind,
            requested_source_url: source_url.to_string(),
            source_url: source_url.to_string(),
            source_etag: None,
            source_last_modified: None,
            source_format: "jpeg".to_string(),
            source_width: 1,
            source_height: 1,
            variants: Vec::new(),
        })
    }
}

async fn wait_for_image_processor_starts(
    processor: &ChunkGatedTitleImageProcessor,
    expected: usize,
) {
    timeout(Duration::from_secs(5), async {
        while processor.started.load(Ordering::SeqCst) < expected {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("image processing should reach expected count");
}

#[tokio::test]
async fn artwork_job_yields_maintenance_lock_between_bounded_groups() {
    const CHUNK_SIZE: usize = 4;

    let (app, _) = bootstrap_with_user_repo(Arc::new(MockUserRepo::default()));
    let title_images = Arc::new(ChunkedTitleImageRepo::new(CHUNK_SIZE * 2));
    let processor = Arc::new(ChunkGatedTitleImageProcessor::new());
    let app = app.with_test_overrides(|services| {
        services
            .with_title_images(title_images.clone())
            .with_title_image_processor(processor.clone())
            .with_job_runs(Arc::new(RecordingJobRunRepo::default()))
    });
    let cancellation = tokio_util::sync::CancellationToken::new();
    let task_app = app.clone();
    let task_token = cancellation.clone();
    let image_loop = tokio::spawn(async move {
        task_app
            .run_artwork_encoding_with_workers(
                &artwork_test_run(JobTriggerSource::Manual),
                4,
                &task_token,
                &chrono::Local::now,
            )
            .await
    });
    wait_for_image_processor_starts(&processor, 4).await;

    let writer_acquired = Arc::new(Notify::new());
    let release_writer = Arc::new(Notify::new());
    let maintenance_lock = app.runtime.catalog.title_image_maintenance_lock.clone();
    let writer_acquired_for_task = writer_acquired.clone();
    let release_writer_for_task = release_writer.clone();
    let writer = tokio::spawn(async move {
        let _guard = maintenance_lock.write().await;
        writer_acquired_for_task.notify_one();
        release_writer_for_task.notified().await;
    });
    sleep(Duration::from_millis(25)).await;

    processor.first_chunk_gate.add_permits(CHUNK_SIZE);
    timeout(Duration::from_secs(5), writer_acquired.notified())
        .await
        .expect("maintenance writer should acquire after the first image chunk");
    assert_eq!(
        processor.started.load(Ordering::SeqCst),
        CHUNK_SIZE,
        "the second image chunk must wait behind the maintenance writer"
    );

    release_writer.notify_one();
    writer.await.expect("maintenance writer should finish");
    wait_for_image_processor_starts(&processor, CHUNK_SIZE + 4).await;
    processor.second_chunk_gate.add_permits(CHUNK_SIZE);
    timeout(Duration::from_secs(5), async {
        while title_images.pending_count().await != 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("both image chunks should finish");

    cancellation.cancel();
    timeout(Duration::from_secs(5), image_loop)
        .await
        .expect("image loop should stop after cancellation")
        .expect("image loop task should not panic")
        .expect("artwork job should succeed");
}

fn artwork_test_run(trigger_source: JobTriggerSource) -> JobRunRecord {
    let now = Utc::now();
    JobRunRecord {
        id: uuid::Uuid::new_v4().to_string(),
        job_key: JobKey::ArtworkEncoding,
        operation_type: "artwork_encoding".into(),
        trigger_source,
        actor_user_id: None,
        status: JobRunStatus::Running,
        progress_json: None,
        summary_json: None,
        summary_text: None,
        error_text: None,
        started_at: now,
        completed_at: None,
        created_at: now,
        updated_at: now,
    }
}

fn artwork_local_time(hour: u32, minute: u32) -> chrono::DateTime<Utc> {
    use chrono::TimeZone;
    chrono::Local
        .with_ymd_and_hms(2026, 9, 7, hour, minute, 0)
        .earliest()
        .unwrap()
        .with_timezone(&Utc)
}

#[tokio::test]
async fn artwork_cutoff_finishes_admitted_images_and_resumes_pending_work() {
    for workers in [2, 4] {
        let (app, _) = bootstrap();
        let images = Arc::new(ChunkedTitleImageRepo::new(8));
        let processor = Arc::new(ChunkGatedTitleImageProcessor::new());
        let app = app.with_test_overrides(|services| {
            services
                .with_title_images(images.clone())
                .with_title_image_processor(processor.clone())
                .with_job_runs(Arc::new(RecordingJobRunRepo::default()))
        });
        app.runtime
            .environment
            .set_fixed_now_for_tests(Some(artwork_local_time(8, 59)));
        let task_app = app.clone();
        let task = tokio::spawn(async move {
            task_app
                .run_artwork_encoding_with_workers(
                    &artwork_test_run(JobTriggerSource::ScheduledDaily),
                    workers,
                    &CancellationToken::new(),
                    &|| {
                        task_app
                            .runtime
                            .environment
                            .now()
                            .with_timezone(&chrono::Local)
                    },
                )
                .await
                .unwrap()
        });
        wait_for_image_processor_starts(&processor, workers).await;
        sleep(Duration::from_millis(25)).await;
        assert_eq!(processor.started.load(Ordering::SeqCst), workers);
        app.runtime
            .environment
            .set_fixed_now_for_tests(Some(artwork_local_time(9, 0)));
        processor.first_chunk_gate.add_permits(4);
        let summary = timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(summary.processed, workers);
        assert_eq!(images.pending_count().await, 8 - workers);
        assert!(summary.reason.contains("nightly"));

        app.runtime
            .environment
            .set_fixed_now_for_tests(Some(artwork_local_time(4, 0) + chrono::Duration::days(1)));
        processor.first_chunk_gate.add_permits(4);
        processor.second_chunk_gate.add_permits(4);
        let summary = app
            .run_artwork_encoding_with_workers(
                &artwork_test_run(JobTriggerSource::ScheduledDaily),
                workers,
                &CancellationToken::new(),
                &|| app.runtime.environment.now().with_timezone(&chrono::Local),
            )
            .await
            .unwrap();
        assert_eq!(summary.processed, 8 - workers);
        assert_eq!(processor.started.load(Ordering::SeqCst), 8);
        assert_eq!(images.pending_count().await, 0);
    }
}

#[tokio::test]
async fn artwork_manual_and_scheduled_triggers_share_one_run_even_at_noon() {
    let (app, admin) = bootstrap();
    let images = Arc::new(ChunkedTitleImageRepo::new(8));
    let processor = Arc::new(ChunkGatedTitleImageProcessor::new());
    let jobs = Arc::new(RecordingJobRunRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_title_images(images.clone())
            .with_title_image_processor(processor.clone())
            .with_job_runs(jobs.clone())
    });
    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(artwork_local_time(12, 0)));
    let (first, second) = tokio::join!(
        app.trigger_job(&admin, JobKey::ArtworkEncoding),
        app.trigger_job(&admin, JobKey::ArtworkEncoding),
    );
    assert_eq!(first.unwrap().id, second.unwrap().id);
    app.run_scheduled_job_now(JobKey::ArtworkEncoding, JobTriggerSource::ScheduledDaily)
        .await
        .unwrap();
    wait_for_image_processor_starts(&processor, 2).await;
    assert_eq!(jobs.runs.lock().await.len(), 1);
    processor.first_chunk_gate.add_permits(4);
    processor.second_chunk_gate.add_permits(4);
    timeout(Duration::from_secs(5), async {
        while app
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(JobKey::ArtworkEncoding)
            .await
        {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(images.pending_count().await, 0);
    let records = jobs.runs.lock().await;
    assert_eq!(records[0].status, JobRunStatus::Completed);
}

#[tokio::test]
async fn artwork_scheduler_does_not_turn_daytime_wakeups_into_encoding() {
    let (app, _) = bootstrap();
    let images = Arc::new(ChunkedTitleImageRepo::new(8));
    let processor = Arc::new(ChunkGatedTitleImageProcessor::new());
    let jobs = Arc::new(RecordingJobRunRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_title_images(images)
            .with_title_image_processor(processor.clone())
            .with_job_runs(jobs.clone())
    });
    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(artwork_local_time(12, 0)));
    let token = CancellationToken::new();
    let worker = tokio::spawn(
        crate::catalog::title_images::start_background_title_image_loop(app.clone(), token.clone()),
    );
    app.runtime.catalog.poster_wake.notify_one();
    app.runtime.catalog.fanart_wake.notify_one();
    sleep(Duration::from_millis(100)).await;
    assert_eq!(processor.started.load(Ordering::SeqCst), 0);
    assert!(jobs.runs.lock().await.is_empty());
    let next = app
        .runtime
        .jobs
        .job_run_tracker
        .next_run_at(JobKey::ArtworkEncoding)
        .await
        .unwrap();
    assert_eq!(next, artwork_local_time(3, 0) + chrono::Duration::days(1));
    token.cancel();
    worker.await.unwrap();
}

#[tokio::test]
async fn artwork_scheduler_retries_transient_errors_without_bypassing_cutoff() {
    for cutoff in [false, true] {
        let (app, _) = bootstrap();
        let images = Arc::new(ChunkedTitleImageRepo::new(8));
        images.list_failures.store(1, Ordering::SeqCst);
        let processor = Arc::new(ChunkGatedTitleImageProcessor::new());
        let app = app.with_test_overrides(|services| {
            services
                .with_title_images(images.clone())
                .with_title_image_processor(processor.clone())
                .with_job_runs(Arc::new(RecordingJobRunRepo::default()))
        });
        app.runtime
            .environment
            .set_fixed_now_for_tests(Some(artwork_local_time(8, 59)));
        tokio::time::pause();
        let token = CancellationToken::new();
        let worker = tokio::spawn(
            crate::catalog::title_images::start_background_title_image_loop(
                app.clone(),
                token.clone(),
            ),
        );
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(images.list_calls.load(Ordering::SeqCst), 1);
        tokio::time::advance(Duration::from_secs(59)).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            images.list_calls.load(Ordering::SeqCst),
            1,
            "retry must back off"
        );
        if cutoff {
            app.runtime
                .environment
                .set_fixed_now_for_tests(Some(artwork_local_time(9, 0)));
        }
        tokio::time::advance(Duration::from_secs(1)).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        tokio::time::resume();
        if cutoff {
            assert_eq!(images.list_calls.load(Ordering::SeqCst), 1);
            assert_eq!(processor.started.load(Ordering::SeqCst), 0);
        } else {
            wait_for_image_processor_starts(&processor, 2).await;
            assert!(
                images.list_calls.load(Ordering::SeqCst) >= 3,
                "retry should inspect backlog and start the job without another wake"
            );
        }
        token.cancel();
        timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn artwork_scheduler_bounds_retries_of_persistent_repository_errors() {
    let (app, _) = bootstrap();
    let images = Arc::new(ChunkedTitleImageRepo::new(8));
    images.list_failures.store(100, Ordering::SeqCst);
    let app = app.with_test_overrides(|services| services.with_title_images(images.clone()));
    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(artwork_local_time(4, 0)));
    let token = CancellationToken::new();
    let worker = tokio::spawn(
        crate::catalog::title_images::start_background_title_image_loop(app.clone(), token.clone()),
    );
    for attempt in 1..=3 {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(images.list_calls.load(Ordering::SeqCst), attempt);
        tokio::time::advance(Duration::from_secs(60)).await;
    }
    tokio::time::advance(Duration::from_secs(600)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(images.list_calls.load(Ordering::SeqCst), 3);
    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(artwork_local_time(4, 0) + chrono::Duration::days(1)));
    app.runtime.catalog.poster_wake.notify_one();
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        images.list_calls.load(Ordering::SeqCst),
        4,
        "the next local day gets another attempt"
    );
    token.cancel();
    worker.await.unwrap();
}

#[tokio::test]
async fn artwork_scan_pause_rechecks_cutoff_and_shutdown_abandons_active_images() {
    for shutdown in [false, true] {
        let (app, _) = bootstrap();
        let images = Arc::new(ChunkedTitleImageRepo::new(8));
        let processor = Arc::new(ChunkGatedTitleImageProcessor::new());
        let app = app.with_test_overrides(|services| {
            services
                .with_title_images(images.clone())
                .with_title_image_processor(processor.clone())
                .with_job_runs(Arc::new(RecordingJobRunRepo::default()))
        });
        app.runtime
            .environment
            .set_fixed_now_for_tests(Some(artwork_local_time(8, 59)));
        let token = CancellationToken::new();
        let task_token = token.clone();
        let task_app = app.clone();
        let task = tokio::spawn(async move {
            task_app
                .run_artwork_encoding_with_workers(
                    &artwork_test_run(JobTriggerSource::ScheduledDaily),
                    2,
                    &task_token,
                    &|| {
                        task_app
                            .runtime
                            .environment
                            .now()
                            .with_timezone(&chrono::Local)
                    },
                )
                .await
                .unwrap()
        });
        wait_for_image_processor_starts(&processor, 2).await;
        let scan = app
            .runtime
            .library
            .library_scan_tracker
            .start_session(MediaFacet::Movie)
            .await
            .unwrap();
        if shutdown {
            token.cancel();
        } else {
            processor.first_chunk_gate.add_permits(4);
            sleep(Duration::from_millis(50)).await;
            assert_eq!(processor.started.load(Ordering::SeqCst), 2);
            app.runtime
                .environment
                .set_fixed_now_for_tests(Some(artwork_local_time(9, 0)));
        }
        app.runtime
            .library
            .library_scan_tracker
            .fail_session(&scan.session_id)
            .await
            .unwrap();
        let summary = timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(summary.processed, if shutdown { 0 } else { 2 });
        assert_eq!(images.pending_count().await, if shutdown { 8 } else { 6 });
        assert_eq!(processor.started.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn artwork_scheduler_starts_inside_window_and_after_a_sleep_or_clock_jump() {
    for initially_open in [false, true] {
        let (app, _) = bootstrap();
        let images = Arc::new(ChunkedTitleImageRepo::new(8));
        let processor = Arc::new(ChunkGatedTitleImageProcessor::new());
        let app = app.with_test_overrides(|services| {
            services
                .with_title_images(images.clone())
                .with_title_image_processor(processor.clone())
                .with_job_runs(Arc::new(RecordingJobRunRepo::default()))
        });
        app.runtime
            .environment
            .set_fixed_now_for_tests(Some(artwork_local_time(
                if initially_open { 4 } else { 12 },
                0,
            )));
        let token = CancellationToken::new();
        let worker = tokio::spawn(
            crate::catalog::title_images::start_background_title_image_loop(
                app.clone(),
                token.clone(),
            ),
        );
        if !initially_open {
            sleep(Duration::from_millis(50)).await;
            assert_eq!(processor.started.load(Ordering::SeqCst), 0);
            app.runtime.environment.set_fixed_now_for_tests(Some(
                artwork_local_time(4, 0) + chrono::Duration::days(1),
            ));
            app.runtime.catalog.poster_wake.notify_one();
        }
        wait_for_image_processor_starts(&processor, 2).await;
        sleep(Duration::from_millis(25)).await;
        let admitted = processor.started.load(Ordering::SeqCst);
        assert!(matches!(admitted, 2 | 4));
        token.cancel();
        app.runtime.catalog.artwork_shutdown.cancelled().await;
        timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(processor.started.load(Ordering::SeqCst), admitted);
        assert_eq!(images.pending_count().await, 8);
    }
}

#[tokio::test]
async fn artwork_shutdown_abandons_in_flight_images_and_closes_future_admission() {
    let (app, admin) = bootstrap();
    let images = Arc::new(ChunkedTitleImageRepo::new(8));
    let processor = Arc::new(ChunkGatedTitleImageProcessor::new());
    let app = app.with_test_overrides(|services| {
        services
            .with_title_images(images.clone())
            .with_title_image_processor(processor.clone())
            .with_job_runs(Arc::new(RecordingJobRunRepo::default()))
    });
    app.trigger_job(&admin, JobKey::ArtworkEncoding)
        .await
        .unwrap();
    wait_for_image_processor_starts(&processor, 2).await;
    app.runtime.catalog.artwork_shutdown.cancel();
    let admitted = processor.started.load(Ordering::SeqCst);
    timeout(Duration::from_secs(5), async {
        while app
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(JobKey::ArtworkEncoding)
            .await
        {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // The processor gates stay closed: shutdown must not wait for encoding.
    assert_eq!(images.pending_count().await, 8);
    assert!(
        app.trigger_job(&admin, JobKey::ArtworkEncoding)
            .await
            .is_err()
    );
    app.run_scheduled_job_now(JobKey::ArtworkEncoding, JobTriggerSource::ScheduledDaily)
        .await
        .unwrap();
    assert_eq!(processor.started.load(Ordering::SeqCst), admitted);
}

struct FailingArtworkProcessor {
    startup_error: bool,
    attempts: AtomicUsize,
}

#[async_trait]
impl TitleImageProcessor for FailingArtworkProcessor {
    async fn configure_encoding_workers(&self, _: usize) -> AppResult<()> {
        if self.startup_error {
            Err(AppError::Repository("worker priority setup failed".into()))
        } else {
            Ok(())
        }
    }

    async fn fetch_and_process_image(
        &self,
        _: TitleImageKind,
        _: &str,
        _: Vec<TitleImageVariantSpec>,
    ) -> AppResult<TitleImageSourceResult> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Err(AppError::Repository("unavailable artwork".into()))
    }
}

#[tokio::test]
async fn artwork_failures_remain_pending_and_priority_failure_never_starts_images() {
    for startup_error in [false, true] {
        let (app, admin) = bootstrap();
        let images = Arc::new(ChunkedTitleImageRepo::new(8));
        let processor = Arc::new(FailingArtworkProcessor {
            startup_error,
            attempts: AtomicUsize::new(0),
        });
        let jobs = Arc::new(RecordingJobRunRepo::default());
        let app = app.with_test_overrides(|services| {
            services
                .with_title_images(images.clone())
                .with_title_image_processor(processor.clone())
                .with_job_runs(jobs.clone())
        });
        app.trigger_job(&admin, JobKey::ArtworkEncoding)
            .await
            .unwrap();
        timeout(Duration::from_secs(5), async {
            while app
                .runtime
                .jobs
                .job_run_tracker
                .has_active_job(JobKey::ArtworkEncoding)
                .await
            {
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(images.pending_count().await, 8);
        assert_eq!(
            processor.attempts.load(Ordering::SeqCst),
            if startup_error { 0 } else { 8 }
        );
        assert_eq!(
            jobs.runs.lock().await[0].status,
            if startup_error {
                JobRunStatus::Failed
            } else {
                JobRunStatus::Warning
            }
        );
    }
}
