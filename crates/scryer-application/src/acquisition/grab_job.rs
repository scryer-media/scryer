//! One operator-chosen release handed to a download client as a background job.
//!
//! A grab waits on the indexer to hand over the release and on the download
//! client to accept it. Either can take a long time, so the request that asks
//! for a grab returns as soon as the job is recorded and the outcome is
//! reported on the job run.

use std::future::Future;

use super::*;

/// How a background grab ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrabJobOutcome {
    /// The download client accepted the release.
    Queued,
    /// Other downloads are in the way and the operator has to choose what
    /// happens to them. Nothing was queued and nothing failed.
    Conflict,
    /// The grab was refused or failed.
    Failed,
}

impl GrabJobOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Conflict => "CONFLICT",
            Self::Failed => "FAILED",
        }
    }
}

/// What a finished grab reports on its job run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrabJobReport {
    pub outcome: GrabJobOutcome,
    /// Reason code of a refusal, kept in a field of its own.
    pub reason: Option<String>,
    /// One line for the job history.
    pub summary_text: Option<String>,
    /// The failure sentence, when the grab failed.
    pub error_text: Option<String>,
}

impl AppUseCase {
    /// Record a grab job for `actor` and run `work` in the background.
    ///
    /// `work` receives the job run's id and performs the grab, including its
    /// own permission checks; nothing here waits for it. The returned run is
    /// still running.
    pub async fn start_grab_job<W, Fut>(
        &self,
        actor: &User,
        kind: &'static str,
        work: W,
    ) -> AppResult<JobRun>
    where
        W: FnOnce(String) -> Fut + Send + 'static,
        Fut: Future<Output = GrabJobReport> + Send + 'static,
    {
        let now = Utc::now();
        let mut run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::ReleaseGrab,
            operation_type: format!("release_grab:{kind}"),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: None,
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;
        let run_payload = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(run_payload.clone())
            .await;
        let actor_event = crate::domain_events::DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor_event.clone(),
                run.id.clone(),
                scryer_domain::DomainEventPayload::JobRunStarted(
                    scryer_domain::JobRunStartedEventData {
                        run_id: run.id.clone(),
                        job_key: run.job_key.as_str().to_string(),
                        operation_type: run.operation_type.clone(),
                        trigger_source: run.trigger_source.as_str().to_string(),
                    },
                ),
            ))
            .await;

        let app = self.clone();
        tokio::spawn(async move {
            // The grab runs in a task of its own so that a panic inside it
            // still ends the job run instead of leaving it running.
            let report = tokio::spawn(work(run.id.clone()))
                .await
                .unwrap_or_else(|_| GrabJobReport {
                    outcome: GrabJobOutcome::Failed,
                    reason: None,
                    summary_text: None,
                    error_text: Some("the grab stopped unexpectedly".to_string()),
                });
            app.finish_grab_job(run, actor_event, kind, report).await;
        });

        Ok(run_payload)
    }

    async fn finish_grab_job(
        &self,
        mut run: JobRunRecord,
        actor: crate::domain_events::DomainEventActor,
        kind: &'static str,
        report: GrabJobReport,
    ) {
        let completed_at = Utc::now();
        let failed = report.outcome == GrabJobOutcome::Failed;
        run.status = if failed {
            JobRunStatus::Failed
        } else {
            JobRunStatus::Completed
        };
        run.summary_json = Some(
            serde_json::json!({
                "kind": kind,
                "outcome": report.outcome.as_str(),
                "reason": report.reason,
            })
            .to_string(),
        );
        run.summary_text = report.summary_text;
        run.error_text = report.error_text;
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        // The live update goes out even when the row cannot be written, so the
        // operator who is waiting on this grab is still told how it ended.
        let finished = match self.services.events.job_runs.update_job_run(&run).await {
            Ok(updated) => updated,
            Err(error) => {
                tracing::warn!(run_id = %run.id, error = %error, "failed to record a finished grab job");
                run
            }
        };
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&finished, None))
            .await;
        let payload = if failed {
            scryer_domain::DomainEventPayload::JobRunFailed(scryer_domain::JobRunFailedEventData {
                run_id: finished.id.clone(),
                job_key: finished.job_key.as_str().to_string(),
                error_text: finished.error_text.clone(),
            })
        } else {
            scryer_domain::DomainEventPayload::JobRunCompleted(
                scryer_domain::JobRunCompletedEventData {
                    run_id: finished.id.clone(),
                    job_key: finished.job_key.as_str().to_string(),
                    summary_text: finished.summary_text.clone(),
                },
            )
        };
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor,
                finished.id.clone(),
                payload,
            ))
            .await;
    }
}
