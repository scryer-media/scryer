use super::*;
use chrono::Utc;
use serde_json::json;
use tokio::sync::broadcast;
use tracing::warn;

#[derive(Clone, Debug, serde::Serialize)]
struct MetadataRefreshSummary {
    refreshed_titles: u32,
}

#[derive(Clone, Debug, serde::Serialize)]
struct HealthChecksSummary {
    total: usize,
    errors: usize,
    warnings: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
struct CountSummary {
    count: u32,
}

#[derive(Clone, Debug, serde::Serialize)]
struct LibraryScanRunSummary {
    scanned: usize,
    matched: usize,
    imported: usize,
    skipped: usize,
    unmatched: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
struct RssSyncRunSummary {
    releases_fetched: usize,
    releases_matched: usize,
    releases_grabbed: usize,
    releases_held: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
struct HousekeepingRunSummary {
    orphaned_media_files: u32,
    stale_release_decisions: u32,
    stale_release_attempts: u32,
    expired_event_outboxes: u32,
    stale_history_events: u32,
    staged_nzb_artifacts_pruned: u32,
    recycled_purged: u32,
}

#[derive(Clone, Debug)]
struct JobExecutionOutcome {
    summary_text: Option<String>,
    summary_json: Option<String>,
    library_scan_progress: Option<LibraryScanSession>,
}

impl JobExecutionOutcome {
    fn new(summary_text: Option<String>, summary_json: Option<String>) -> Self {
        Self {
            summary_text,
            summary_json,
            library_scan_progress: None,
        }
    }

    fn from_library_scan(summary: &LibraryScanSummary) -> Self {
        Self::new(
            Some(summary_text_from_library_scan(summary)),
            serde_json::to_string(&LibraryScanRunSummary {
                scanned: summary.scanned,
                matched: summary.matched,
                imported: summary.imported,
                skipped: summary.skipped,
                unmatched: summary.unmatched,
            })
            .ok(),
        )
    }
}

impl AppUseCase {
    pub async fn list_jobs(&self, actor: &User) -> AppResult<Vec<JobDefinition>> {
        require(actor, &Entitlement::ManageConfig)?;
        let next_runs = self.services.job_run_tracker.all_next_runs().await;
        Ok(crate::jobs::all_job_definitions(&next_runs))
    }

    pub async fn active_job_runs(&self, actor: &User) -> AppResult<Vec<JobRun>> {
        require(actor, &Entitlement::ManageConfig)?;
        Ok(self.services.job_run_tracker.list_active().await)
    }

    pub async fn list_job_runs(
        &self,
        actor: &User,
        job_key: JobKey,
        limit: usize,
    ) -> AppResult<Vec<JobRun>> {
        require(actor, &Entitlement::ManageConfig)?;
        let active_runs = self.services.job_run_tracker.list_active().await;
        let active_runs_by_id = active_runs
            .into_iter()
            .map(|run| (run.id.clone(), run))
            .collect::<HashMap<_, _>>();

        let records = self
            .services
            .job_runs
            .list_job_runs(Some(job_key), limit.max(1))
            .await?;

        Ok(records
            .into_iter()
            .map(|record| {
                active_runs_by_id
                    .get(&record.id)
                    .cloned()
                    .unwrap_or_else(|| JobRun::from_record(&record, None))
            })
            .collect())
    }

    pub async fn list_recent_job_runs(&self, actor: &User, limit: usize) -> AppResult<Vec<JobRun>> {
        require(actor, &Entitlement::ManageConfig)?;
        let active_runs = self.services.job_run_tracker.list_active().await;
        let active_runs_by_id = active_runs
            .into_iter()
            .map(|run| (run.id.clone(), run))
            .collect::<HashMap<_, _>>();

        let records = self
            .services
            .job_runs
            .list_job_runs(None, limit.max(1))
            .await?;

        Ok(records
            .into_iter()
            .map(|record| {
                active_runs_by_id
                    .get(&record.id)
                    .cloned()
                    .unwrap_or_else(|| JobRun::from_record(&record, None))
            })
            .collect())
    }

    pub fn subscribe_job_run_events(&self, actor: &User) -> AppResult<broadcast::Receiver<JobRun>> {
        require(actor, &Entitlement::ManageConfig)?;
        Ok(self.services.job_run_tracker.subscribe())
    }

    pub async fn trigger_job(&self, actor: &User, job_key: JobKey) -> AppResult<JobRun> {
        require(actor, &Entitlement::ManageConfig)?;
        self.ensure_job_can_start(job_key).await?;

        let run = self
            .create_job_run_record(job_key, JobTriggerSource::Manual, Some(actor.id.clone()))
            .await?;
        let run_payload = JobRun::from_record(&run, None);
        self.services
            .job_run_tracker
            .upsert_active_run(run_payload.clone())
            .await;

        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            if let Err(error) = app.run_job_run(run, Some(actor)).await {
                warn!(job_key = job_key.as_str(), error = %error, "manual job trigger failed");
            }
        });

        Ok(run_payload)
    }

    pub async fn run_scheduled_job_now(
        &self,
        job_key: JobKey,
        trigger_source: JobTriggerSource,
    ) -> AppResult<()> {
        self.ensure_job_can_start(job_key).await?;
        let run = self
            .create_job_run_record(job_key, trigger_source, None)
            .await?;
        let run_payload = JobRun::from_record(&run, None);
        self.services
            .job_run_tracker
            .upsert_active_run(run_payload)
            .await;
        self.run_job_run(run, None).await
    }

    pub async fn set_job_next_run_at(&self, job_key: JobKey, next_run_at: chrono::DateTime<Utc>) {
        self.services
            .job_run_tracker
            .set_next_run_at(job_key, next_run_at)
            .await;
    }

    async fn ensure_job_can_start(&self, job_key: JobKey) -> AppResult<()> {
        if self.services.job_run_tracker.has_active_job(job_key).await {
            return Err(AppError::Validation(format!(
                "{} is already running",
                job_key.display_name()
            )));
        }

        if let Some(facet) = job_key_library_facet(job_key) {
            let active_scans = self.services.library_scan_tracker.list_active().await;
            if active_scans
                .into_iter()
                .any(|session| session.facet == facet)
            {
                return Err(AppError::Validation(format!(
                    "{} library scan is already running",
                    facet.as_str()
                )));
            }
        }

        Ok(())
    }

    async fn create_job_run_record(
        &self,
        job_key: JobKey,
        trigger_source: JobTriggerSource,
        actor_user_id: Option<String>,
    ) -> AppResult<JobRunRecord> {
        let now = Utc::now();
        let initial_status = if job_key.uses_library_scan_progress() {
            JobRunStatus::Discovering
        } else {
            JobRunStatus::Running
        };

        self.services
            .job_runs
            .create_job_run(&JobRunRecord {
                id: Id::new().0,
                job_key,
                operation_type: job_key.as_str().to_string(),
                status: initial_status,
                trigger_source,
                actor_user_id,
                progress_json: Some(json!({ "status": initial_status.as_str() }).to_string()),
                summary_json: None,
                summary_text: None,
                error_text: None,
                started_at: now,
                completed_at: None,
                created_at: now,
                updated_at: now,
            })
            .await
    }

    async fn run_job_run(&self, run: JobRunRecord, actor: Option<User>) -> AppResult<()> {
        match self.execute_job_body(run.job_key, &run.id, actor).await {
            Ok(outcome) => {
                self.finish_job_run(
                    run,
                    outcome.summary_text,
                    outcome.summary_json,
                    outcome.library_scan_progress,
                )
                .await
            }
            Err(error) => {
                self.fail_job_run(run, error.to_string()).await?;
                Err(error)
            }
        }
    }

    async fn execute_job_body(
        &self,
        job_key: JobKey,
        run_id: &str,
        actor: Option<User>,
    ) -> AppResult<JobExecutionOutcome> {
        match job_key {
            JobKey::LibraryScanMovies | JobKey::LibraryScanSeries | JobKey::LibraryScanAnime => {
                let actor = match actor {
                    Some(actor) => actor,
                    None => self.find_or_create_default_user().await?,
                };
                let facet = job_key_library_facet(job_key).expect("library scan facet");
                let summary = self
                    .scan_library_with_tracking(
                        &actor,
                        facet,
                        Some(run_id.to_string()),
                        LibraryScanMode::Full,
                    )
                    .await?;
                Ok(JobExecutionOutcome::from_library_scan(&summary))
            }
            JobKey::BackgroundLibraryRefreshMovies
            | JobKey::BackgroundLibraryRefreshSeries
            | JobKey::BackgroundLibraryRefreshAnime => {
                let actor = match actor {
                    Some(actor) => actor,
                    None => self.find_or_create_default_user().await?,
                };
                let facet = job_key_library_facet(job_key).expect("background refresh facet");
                let summary = self
                    .background_library_refresh_with_tracking(&actor, facet, run_id)
                    .await?;
                Ok(JobExecutionOutcome::from_library_scan(&summary))
            }
            JobKey::RssSync => {
                let report = self.run_rss_sync().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!(
                        "Fetched {}, matched {}, grabbed {}",
                        report.releases_fetched, report.releases_matched, report.releases_grabbed
                    )),
                    serde_json::to_string(&RssSyncRunSummary {
                        releases_fetched: report.releases_fetched,
                        releases_matched: report.releases_matched,
                        releases_grabbed: report.releases_grabbed,
                        releases_held: report.releases_held,
                    })
                    .ok(),
                ))
            }
            JobKey::SubtitleSearch => Ok(JobExecutionOutcome::new(
                Some(self.run_subtitle_search_job().await?),
                None,
            )),
            JobKey::MetadataRefresh => {
                let refreshed_titles = self.run_metadata_refresh_job().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!("Refreshed metadata for {refreshed_titles} titles")),
                    serde_json::to_string(&MetadataRefreshSummary { refreshed_titles }).ok(),
                ))
            }
            JobKey::PluginRegistryRefresh => {
                self.refresh_plugin_registry_internal().await?;
                Ok(JobExecutionOutcome::new(
                    Some("Plugin registry refreshed".to_string()),
                    None,
                ))
            }
            JobKey::Housekeeping => {
                let report = self.run_housekeeping().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!(
                        "Removed {} orphaned media files and {} stale release decisions",
                        report.orphaned_media_files, report.stale_release_decisions
                    )),
                    serde_json::to_string(&HousekeepingRunSummary {
                        orphaned_media_files: report.orphaned_media_files,
                        stale_release_decisions: report.stale_release_decisions,
                        stale_release_attempts: report.stale_release_attempts,
                        expired_event_outboxes: report.expired_event_outboxes,
                        stale_history_events: report.stale_history_events,
                        staged_nzb_artifacts_pruned: report.staged_nzb_artifacts_pruned,
                        recycled_purged: report.recycled_purged,
                    })
                    .ok(),
                ))
            }
            JobKey::HealthChecks => {
                let results = self.run_health_checks().await;
                *self.services.health_check_results.write().await = results.clone();
                let errors = results
                    .iter()
                    .filter(|result| matches!(result.status, HealthCheckStatus::Error))
                    .count();
                let warnings = results
                    .iter()
                    .filter(|result| matches!(result.status, HealthCheckStatus::Warning))
                    .count();
                Ok(JobExecutionOutcome::new(
                    Some(format!(
                        "Completed {} health checks ({} errors, {} warnings)",
                        results.len(),
                        errors,
                        warnings
                    )),
                    serde_json::to_string(&HealthChecksSummary {
                        total: results.len(),
                        errors,
                        warnings,
                    })
                    .ok(),
                ))
            }
            JobKey::WantedSync => {
                self.sync_wanted_state().await?;
                Ok(JobExecutionOutcome::new(
                    Some("Wanted state synchronized".to_string()),
                    None,
                ))
            }
            JobKey::PendingReleaseProcessing => {
                let count = self.process_expired_pending_releases().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!("Processed {count} pending releases")),
                    serde_json::to_string(&CountSummary { count }).ok(),
                ))
            }
            JobKey::StagedNzbPrune => {
                let count = self
                    .services
                    .staged_nzb_store
                    .prune_staged_nzbs_older_than(Utc::now() - chrono::Duration::hours(1))
                    .await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!("Pruned {count} staged NZB artifacts")),
                    serde_json::to_string(&CountSummary { count }).ok(),
                ))
            }
        }
    }

    async fn finish_job_run(
        &self,
        mut run: JobRunRecord,
        summary_text: Option<String>,
        summary_json: Option<String>,
        library_scan_progress: Option<LibraryScanSession>,
    ) -> AppResult<()> {
        let completed_at = Utc::now();
        run.status = match library_scan_progress
            .as_ref()
            .map(|session| &session.status)
        {
            Some(LibraryScanStatus::Warning) => JobRunStatus::Warning,
            Some(LibraryScanStatus::Failed) => JobRunStatus::Failed,
            _ => JobRunStatus::Completed,
        };
        run.progress_json = Some(json!({ "status": run.status.as_str() }).to_string());
        run.summary_text = summary_text;
        run.summary_json = summary_json;
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.job_runs.update_job_run(&run).await?;
        self.services
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, library_scan_progress))
            .await;
        Ok(())
    }

    async fn fail_job_run(&self, mut run: JobRunRecord, error_text: String) -> AppResult<()> {
        let completed_at = Utc::now();
        run.status = JobRunStatus::Failed;
        run.progress_json = Some(json!({ "status": run.status.as_str() }).to_string());
        run.error_text = Some(error_text.clone());
        run.summary_text = Some(format!("Failed: {error_text}"));
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.job_runs.update_job_run(&run).await?;
        self.services
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        Ok(())
    }
}

pub async fn start_background_library_refresh_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    for job_key in [
        JobKey::BackgroundLibraryRefreshMovies,
        JobKey::BackgroundLibraryRefreshSeries,
        JobKey::BackgroundLibraryRefreshAnime,
    ] {
        let app = app.clone();
        let token = token.child_token();
        tokio::spawn(async move {
            run_background_library_refresh_worker(app, token, job_key).await;
        });
    }

    token.cancelled().await;
}

async fn run_background_library_refresh_worker(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
    job_key: JobKey,
) {
    let initial_delay = job_key.initial_delay_seconds().unwrap_or(60).max(1) as u64;
    let interval_seconds = job_key.interval_seconds().unwrap_or(3600).max(1) as u64;
    let initial_next_run_at = Utc::now() + chrono::Duration::seconds(initial_delay as i64);
    app.set_job_next_run_at(job_key, initial_next_run_at).await;

    tokio::select! {
        _ = token.cancelled() => return,
        _ = tokio::time::sleep(std::time::Duration::from_secs(initial_delay)) => {}
    }

    if let Err(error) = app
        .run_scheduled_job_now(job_key, JobTriggerSource::ScheduledStartup)
        .await
    {
        warn!(job_key = job_key.as_str(), error = %error, "startup background job failed");
    }

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_seconds));
    interval.tick().await;
    app.set_job_next_run_at(
        job_key,
        Utc::now() + chrono::Duration::seconds(interval_seconds as i64),
    )
    .await;

    loop {
        tokio::select! {
            _ = token.cancelled() => return,
            _ = interval.tick() => {
                app.set_job_next_run_at(
                    job_key,
                    Utc::now() + chrono::Duration::seconds(interval_seconds as i64),
                ).await;
                if let Err(error) = app
                    .run_scheduled_job_now(job_key, JobTriggerSource::ScheduledInterval)
                    .await
                {
                    warn!(job_key = job_key.as_str(), error = %error, "scheduled background job failed");
                }
            }
        }
    }
}

fn job_key_library_facet(job_key: JobKey) -> Option<MediaFacet> {
    match job_key {
        JobKey::LibraryScanMovies | JobKey::BackgroundLibraryRefreshMovies => {
            Some(MediaFacet::Movie)
        }
        JobKey::LibraryScanSeries | JobKey::BackgroundLibraryRefreshSeries => {
            Some(MediaFacet::Series)
        }
        JobKey::LibraryScanAnime | JobKey::BackgroundLibraryRefreshAnime => Some(MediaFacet::Anime),
        _ => None,
    }
}

fn summary_text_from_library_scan(summary: &LibraryScanSummary) -> String {
    format!(
        "Scanned {}, imported {}, skipped {}, unmatched {}",
        summary.scanned, summary.imported, summary.skipped, summary.unmatched
    )
}
