//! Ending a job run releases it from the job run tracker even when the job
//! run store refuses the terminal write, so the job can start again and a
//! rerun requested meanwhile still starts.

use super::*;
use crate::domain_events::DomainEventActor;

const RUN_ID: &str = "list-sync-ending";

fn running_list_sync() -> JobRunRecord {
    let now = chrono::Utc::now();
    JobRunRecord {
        id: RUN_ID.to_string(),
        job_key: JobKey::ListSync,
        operation_type: JobKey::ListSync.as_str().to_string(),
        status: JobRunStatus::Running,
        trigger_source: JobTriggerSource::ScheduledInterval,
        actor_user_id: None,
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

struct Fixture {
    harness: MediaRequestTestHarness,
    app: AppUseCase,
    job_runs: Arc<RecordingJobRunRepo>,
}

/// A list sync run already active in the store and the tracker.
async fn fixture_with_active_run() -> Fixture {
    let harness = bootstrap_media_request_app();
    let job_runs = Arc::new(RecordingJobRunRepo::default());
    let app = harness
        .app
        .with_test_overrides(|services| services.with_job_runs(job_runs.clone()));
    let running = running_list_sync();
    job_runs.seed(running.clone()).await;
    app.runtime
        .jobs
        .job_run_tracker
        .upsert_active_run(JobRun::from_record(&running, None))
        .await;
    Fixture {
        harness,
        app,
        job_runs,
    }
}

/// Waits for a list sync run accepted by `matches` to be broadcast.
async fn next_list_sync_run(
    runs: &mut tokio::sync::broadcast::Receiver<JobRun>,
    what: &str,
    matches: impl Fn(&JobRun) -> bool,
) -> JobRun {
    within_deadline(what, async {
        loop {
            let run = runs.recv().await.expect("job run events stay open");
            if run.job_key == JobKey::ListSync && matches(&run) {
                return run;
            }
        }
    })
    .await
}

async fn stored_status(job_runs: &RecordingJobRunRepo, run_id: &str) -> Option<JobRunStatus> {
    job_runs
        .get_job_run(run_id)
        .await
        .unwrap()
        .map(|run| run.status)
}

async fn ended_event_recorded(harness: &MediaRequestTestHarness, run_id: &str) -> bool {
    harness
        .domain_events
        .events
        .lock()
        .await
        .iter()
        .any(|event| match &event.payload {
            DomainEventPayload::JobRunCompleted(data) => data.run_id == run_id,
            DomainEventPayload::JobRunFailed(data) => data.run_id == run_id,
            _ => false,
        })
}

#[derive(Clone, Copy)]
enum RunEnd {
    Finish,
    Fail,
}

async fn end_run(app: &AppUseCase, end: RunEnd) -> AppResult<()> {
    match end {
        RunEnd::Finish => {
            app.finish_job_run(
                running_list_sync(),
                DomainEventActor::system(),
                Some("done".to_string()),
                None,
                None,
                None,
            )
            .await
        }
        RunEnd::Fail => {
            app.fail_job_run(
                running_list_sync(),
                DomainEventActor::system(),
                "the job body failed".to_string(),
            )
            .await
        }
    }
}

async fn assert_refused_end_releases_the_job(end: RunEnd, expected_status: JobRunStatus) {
    let fixture = fixture_with_active_run().await;
    let app = &fixture.app;
    let tracker = &app.runtime.jobs.job_run_tracker;
    let blocked = app
        .start_manual_job_run(&fixture.harness.manager, JobKey::ListSync)
        .await
        .expect_err("a second run is refused while the first is active");
    assert!(blocked.to_string().contains("already running"), "{blocked}");

    fixture
        .job_runs
        .reject_terminal_updates
        .store(true, Ordering::SeqCst);
    let mut runs = tracker.subscribe();
    let error = end_run(app, end)
        .await
        .expect_err("the store error reaches the caller");

    assert!(
        matches!(&error, AppError::Repository(message) | AppError::RepositoryFailure { message, .. } | AppError::RepositoryFailed { message, .. } if message.contains("refused the terminal write")),
        "{error:?}"
    );
    assert_eq!(
        stored_status(&fixture.job_runs, RUN_ID).await,
        Some(JobRunStatus::Running),
        "the store kept the run unfinished"
    );
    assert!(
        !tracker.has_active_job(JobKey::ListSync).await,
        "the tracker no longer holds the run"
    );
    let ended = next_list_sync_run(&mut runs, "the ended run to be broadcast", |run| {
        run.id == RUN_ID
    })
    .await;
    assert_eq!(ended.status, expected_status);
    assert!(ended.completed_at.is_some());
    assert!(
        ended_event_recorded(&fixture.harness, RUN_ID).await,
        "the domain event is still appended"
    );

    fixture
        .job_runs
        .reject_terminal_updates
        .store(false, Ordering::SeqCst);
    let next = app
        .start_manual_job_run(&fixture.harness.manager, JobKey::ListSync)
        .await
        .expect("the job starts again");
    assert_ne!(next.id, RUN_ID);
    next_list_sync_run(&mut runs, "the new run to end", |run| {
        run.id == next.id && run.status.is_terminal()
    })
    .await;
    assert_eq!(
        stored_status(&fixture.job_runs, &next.id).await,
        Some(JobRunStatus::Completed)
    );
}

#[tokio::test]
async fn a_refused_finish_still_releases_the_job_for_its_next_start() {
    assert_refused_end_releases_the_job(RunEnd::Finish, JobRunStatus::Completed).await;
}

#[tokio::test]
async fn a_refused_failure_still_releases_the_job_for_its_next_start() {
    assert_refused_end_releases_the_job(RunEnd::Fail, JobRunStatus::Failed).await;
}

async fn assert_rerun_starts_after_a_refused_end(end: RunEnd) {
    let fixture = fixture_with_active_run().await;
    let app = &fixture.app;
    let tracker = &app.runtime.jobs.job_run_tracker;
    assert!(
        tracker
            .request_rerun_if_active(JobKey::ListSync, &fixture.harness.manager)
            .await,
        "the rerun request is recorded against the active run"
    );
    fixture
        .job_runs
        .reject_terminal_updates
        .store(true, Ordering::SeqCst);
    let mut runs = tracker.subscribe();

    end_run(app, end)
        .await
        .expect_err("the store error reaches the caller");

    let rerun = next_list_sync_run(&mut runs, "the requested rerun to start", |run| {
        run.id != RUN_ID
    })
    .await;
    assert_eq!(
        stored_status(&fixture.job_runs, &rerun.id).await,
        Some(JobRunStatus::Running),
        "the rerun was created in the store"
    );
    // The store still refuses terminal writes, so the rerun ends the same
    // way and must not be left behind in the tracker either.
    next_list_sync_run(&mut runs, "the rerun to end", |run| {
        run.id == rerun.id && run.status.is_terminal()
    })
    .await;
    assert!(!tracker.has_active_job(JobKey::ListSync).await);
}

#[tokio::test]
async fn a_rerun_requested_during_a_run_starts_after_a_refused_finish() {
    assert_rerun_starts_after_a_refused_end(RunEnd::Finish).await;
}

#[tokio::test]
async fn a_rerun_requested_during_a_run_starts_after_a_refused_failure() {
    assert_rerun_starts_after_a_refused_end(RunEnd::Fail).await;
}

#[tokio::test]
async fn a_stored_finish_ends_the_run_and_starts_the_requested_rerun() {
    let fixture = fixture_with_active_run().await;
    let app = &fixture.app;
    let tracker = &app.runtime.jobs.job_run_tracker;
    assert!(
        tracker
            .request_rerun_if_active(JobKey::ListSync, &fixture.harness.manager)
            .await
    );
    let mut runs = tracker.subscribe();

    end_run(app, RunEnd::Finish)
        .await
        .expect("the run ends cleanly");

    assert_eq!(
        stored_status(&fixture.job_runs, RUN_ID).await,
        Some(JobRunStatus::Completed)
    );
    let ended = next_list_sync_run(&mut runs, "the ended run to be broadcast", |run| {
        run.id == RUN_ID
    })
    .await;
    assert_eq!(ended.status, JobRunStatus::Completed);
    assert!(ended_event_recorded(&fixture.harness, RUN_ID).await);
    let rerun = next_list_sync_run(&mut runs, "the requested rerun to start", |run| {
        run.id != RUN_ID
    })
    .await;
    next_list_sync_run(&mut runs, "the rerun to end", |run| {
        run.id == rerun.id && run.status.is_terminal()
    })
    .await;
    assert_eq!(
        stored_status(&fixture.job_runs, &rerun.id).await,
        Some(JobRunStatus::Completed)
    );
}
