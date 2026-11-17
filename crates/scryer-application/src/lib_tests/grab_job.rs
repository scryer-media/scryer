use super::*;

fn bootstrap_with_job_runs() -> (AppUseCase, User) {
    let job_runs = Arc::new(RecordingJobRunRepo::default());
    let (app, actor) = bootstrap();
    let app = app.with_test_overrides(|services| services.with_job_runs(job_runs.clone()));
    (app, actor)
}

async fn wait_for_grab_run(app: &AppUseCase, actor: &User, run_id: &str) -> JobRun {
    wait_for("the grab job to reach a terminal status", || async {
        app.list_job_runs(actor, JobKey::ReleaseGrab, 10)
            .await
            .expect("list grab runs")
            .into_iter()
            .find(|run| run.id == run_id && run.status.is_terminal())
    })
    .await
}

#[tokio::test]
async fn a_grab_job_is_accepted_before_the_grab_has_finished() {
    let (app, actor) = bootstrap_with_job_runs();
    let (release, held) = tokio::sync::oneshot::channel::<()>();

    let accepted = app
        .start_grab_job(&actor, "existing", move |_run_id| async move {
            // Stands in for the indexer and the download client: the grab does
            // not finish until the test lets it.
            let _ = held.await;
            GrabJobReport {
                outcome: GrabJobOutcome::Queued,
                reason: None,
                summary_text: Some("Sent Example.Release to the download client.".to_string()),
                error_text: None,
            }
        })
        .await
        .expect("the grab job is accepted");

    assert_eq!(accepted.status, JobRunStatus::Running);
    assert_eq!(accepted.job_key, JobKey::ReleaseGrab);
    assert_eq!(accepted.actor_user_id.as_deref(), Some(actor.id.as_str()));

    release.send(()).expect("the grab is still waiting");
    let finished = wait_for_grab_run(&app, &actor, &accepted.id).await;

    assert_eq!(finished.status, JobRunStatus::Completed);
    assert_eq!(
        finished.summary_text.as_deref(),
        Some("Sent Example.Release to the download client.")
    );
    let summary: serde_json::Value =
        serde_json::from_str(finished.summary_json.as_deref().expect("summary json"))
            .expect("summary json parses");
    assert_eq!(summary["kind"], "existing");
    assert_eq!(summary["outcome"], "QUEUED");
    assert_eq!(summary["reason"], serde_json::Value::Null);
}

#[tokio::test]
async fn a_refused_grab_fails_its_job_with_the_reason_in_its_own_field() {
    let (app, actor) = bootstrap_with_job_runs();

    let accepted = app
        .start_grab_job(&actor, "pending", |_run_id| async {
            GrabJobReport {
                outcome: GrabJobOutcome::Failed,
                reason: Some("EXAMPLE_SOURCE_GONE".to_string()),
                summary_text: None,
                error_text: Some("the release is no longer on the indexer".to_string()),
            }
        })
        .await
        .expect("the grab job is accepted");
    let finished = wait_for_grab_run(&app, &actor, &accepted.id).await;

    assert_eq!(finished.status, JobRunStatus::Failed);
    assert_eq!(
        finished.error_text.as_deref(),
        Some("the release is no longer on the indexer")
    );
    let summary: serde_json::Value =
        serde_json::from_str(finished.summary_json.as_deref().expect("summary json"))
            .expect("summary json parses");
    assert_eq!(summary["outcome"], "FAILED");
    assert_eq!(summary["reason"], "EXAMPLE_SOURCE_GONE");
}

#[tokio::test]
async fn a_grab_that_panics_still_ends_its_job() {
    let (app, actor) = bootstrap_with_job_runs();

    let accepted = app
        .start_grab_job(&actor, "existing", |_run_id| async {
            if true {
                panic!("the grab stopped");
            }
            GrabJobReport {
                outcome: GrabJobOutcome::Queued,
                reason: None,
                summary_text: None,
                error_text: None,
            }
        })
        .await
        .expect("the grab job is accepted");
    let finished = wait_for_grab_run(&app, &actor, &accepted.id).await;

    assert_eq!(finished.status, JobRunStatus::Failed);
    assert!(finished.error_text.is_some());
}

#[tokio::test]
async fn a_grab_job_cannot_be_started_from_the_job_trigger() {
    assert!(!JobKey::ReleaseGrab.manual_trigger_allowed());
    assert_eq!(JobKey::parse("release_grab"), Some(JobKey::ReleaseGrab));
}
