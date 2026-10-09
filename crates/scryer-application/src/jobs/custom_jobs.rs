//! User-defined scheduled scripts run as jobs ("custom jobs").
//!
//! Every scheduled script shares [`JobKey::CustomJob`]; its runs are told
//! apart by the operation type `custom_job:<script id>`. The in-memory
//! schedule lives in [`crate::scripts::scheduler`]; this module starts runs
//! for it and executes them.

use super::*;
use crate::import::post_processing::{script_run_record, script_source};
use crate::scripts::runner::{ScriptInvocation, ScriptOutcome, run_script};
use crate::scripts::scheduler::{
    CustomJobDispatcher, CustomJobSource, DueCustomJob, run_custom_job_scheduler,
};
use async_trait::async_trait;
use scryer_domain::{
    ExecutionMode, PostProcessingScript, PostProcessingScriptRun, ScriptRunStatus, ScriptTrigger,
};
use std::time::Duration;

/// `SCRYER_EVENT` for a scheduled script.
const CUSTOM_JOB_EVENT: &str = "scheduled_job";

fn ensure_runnable_custom_job(script: &PostProcessingScript) -> AppResult<()> {
    if script.trigger != ScriptTrigger::Schedule {
        return Err(AppError::Validation(format!(
            "script \"{}\" is not a scheduled script",
            script.name
        )));
    }
    if !script.enabled {
        return Err(AppError::Validation(format!(
            "scheduled script \"{}\" is disabled",
            script.name
        )));
    }
    Ok(())
}

/// `SCRYER_TRIGGER_SOURCE` for a run.
fn custom_job_trigger_label(trigger_source: JobTriggerSource) -> &'static str {
    match trigger_source {
        JobTriggerSource::Manual => "manual",
        JobTriggerSource::ScheduledStartup => "startup",
        JobTriggerSource::ScheduledInterval
        | JobTriggerSource::ScheduledDaily
        | JobTriggerSource::SystemInternal => "schedule",
    }
}

fn custom_job_schedule_info(entry: &crate::scripts::scheduler::CustomJobEntry) -> JobScheduleInfo {
    use scryer_domain::ScriptSchedule;
    let (kind, interval_seconds) = match &entry.schedule {
        ScriptSchedule::Manual => (JobScheduleKind::Manual, None),
        ScriptSchedule::Interval { every_seconds } => (
            if entry.run_on_startup {
                JobScheduleKind::StartupAndInterval
            } else {
                JobScheduleKind::Interval
            },
            Some(*every_seconds),
        ),
        ScriptSchedule::Daily { .. } | ScriptSchedule::Weekly { .. } => {
            (JobScheduleKind::DailyAtTime, None)
        }
        ScriptSchedule::Cron { .. } => (JobScheduleKind::Interval, None),
    };
    JobScheduleInfo {
        kind,
        description: entry.schedule_description.clone(),
        interval_seconds,
        initial_delay_seconds: None,
        next_run_at: entry.next_run_at(),
    }
}

fn custom_job_summary_text(outcome: &ScriptOutcome) -> String {
    match outcome {
        ScriptOutcome::Exited {
            code: Some(code), ..
        } => format!("Exited with code {code}"),
        ScriptOutcome::Exited { code: None, .. } => "Exited without a code".to_string(),
        ScriptOutcome::TimedOut => "Timed out".to_string(),
        ScriptOutcome::SpawnFailed { reason } => format!("Failed to start: {reason}"),
        ScriptOutcome::IoError { reason } => format!("I/O error: {reason}"),
    }
}

impl AppUseCase {
    /// Reload the in-memory schedule from the script store. Called when a
    /// scheduled script is created, updated, toggled or deleted.
    pub(crate) async fn reload_custom_job_schedule(&self) {
        match self
            .services
            .customization
            .pp_scripts
            .list_enabled_scheduled()
            .await
        {
            Ok(scripts) => self.runtime.jobs.custom_job_scheduler.apply(scripts),
            Err(error) => warn!(error = %error, "failed to reload scheduled scripts"),
        }
    }

    /// One job definition per enabled scheduled script, read from the
    /// in-memory schedule.
    pub(super) fn custom_job_definitions(&self) -> Vec<JobDefinition> {
        self.runtime
            .jobs
            .custom_job_scheduler
            .entries()
            .into_iter()
            .map(|entry| {
                let mut definition = JobDefinition::from_key(JobKey::CustomJob, None);
                definition.custom_job_id = Some(entry.script_id.clone());
                definition.display_name = entry.name.clone();
                if !entry.description.trim().is_empty() {
                    definition.description = entry.description.clone();
                }
                definition.schedule = custom_job_schedule_info(&entry);
                definition
            })
            .collect()
    }

    /// Start a run of a scheduled script now.
    pub async fn trigger_custom_job(&self, actor: &User, script_id: &str) -> AppResult<JobRun> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let script = self
            .services
            .customization
            .pp_scripts
            .get_script(script_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("script {script_id} not found")))?;
        ensure_runnable_custom_job(&script)?;
        self.start_custom_job_run(
            &script.id,
            script.execution_mode,
            JobTriggerSource::Manual,
            Some(actor),
        )
        .await?
        .ok_or_else(|| AppError::Validation(format!("\"{}\" is already running", script.name)))
    }

    /// Runs of one scheduled script, newest first.
    pub async fn list_custom_job_runs(
        &self,
        actor: &User,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<JobRun>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let active_by_id = self
            .runtime
            .jobs
            .job_run_tracker
            .list_active()
            .await
            .into_iter()
            .map(|run| (run.id.clone(), run))
            .collect::<HashMap<_, _>>();
        let records = self
            .services
            .events
            .job_runs
            .list_job_runs_by_operation_type(
                JobKey::CustomJob,
                &custom_job_operation_type(script_id),
                limit.max(1),
            )
            .await?;
        Ok(records
            .into_iter()
            .map(|record| {
                active_by_id
                    .get(&record.id)
                    .cloned()
                    .unwrap_or_else(|| JobRun::from_record(&record, None))
            })
            .collect())
    }

    /// The latest run of each scheduled script, enabled or not.
    pub(super) async fn latest_custom_job_runs(
        &self,
        active_by_id: &HashMap<String, JobRun>,
    ) -> AppResult<Vec<JobRun>> {
        let scripts = self
            .services
            .customization
            .pp_scripts
            .list_scripts_by_trigger(ScriptTrigger::Schedule)
            .await?;
        let mut runs = Vec::new();
        for script in scripts {
            let records = self
                .services
                .events
                .job_runs
                .list_job_runs_by_operation_type(
                    JobKey::CustomJob,
                    &custom_job_operation_type(&script.id),
                    1,
                )
                .await?;
            runs.extend(records.into_iter().map(|record| {
                active_by_id
                    .get(&record.id)
                    .cloned()
                    .unwrap_or_else(|| JobRun::from_record(&record, None))
            }));
        }
        Ok(runs)
    }

    /// Start a run. A blocking script never overlaps itself: a manual start
    /// is refused and a scheduled fire is skipped (`Ok(None)`). A
    /// fire-and-forget script may overlap.
    async fn start_custom_job_run(
        &self,
        script_id: &str,
        execution_mode: ExecutionMode,
        trigger_source: JobTriggerSource,
        actor: Option<&User>,
    ) -> AppResult<Option<JobRun>> {
        let operation_type = custom_job_operation_type(script_id);
        let start_guard = self.runtime.jobs.custom_job_start_lock.lock().await;
        if execution_mode == ExecutionMode::Blocking
            && self
                .runtime
                .jobs
                .job_run_tracker
                .has_active_operation(JobKey::CustomJob, &operation_type)
                .await
        {
            if actor.is_some() {
                return Err(AppError::Validation(
                    "this scheduled script is already running".to_string(),
                ));
            }
            info!(
                script_id,
                trigger_source = trigger_source.as_str(),
                "skipping scheduled script fire: its previous run is still running"
            );
            return Ok(None);
        }
        let run = self
            .create_job_run_record(
                JobKey::CustomJob,
                trigger_source,
                actor.map(|actor| actor.id.clone()),
                Some(operation_type),
            )
            .await?;
        let run_payload = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(run_payload.clone())
            .await;
        drop(start_guard);

        let (principal, event_actor, log_actor) = match actor {
            Some(actor) => (
                JobExecutionPrincipal::User(actor.clone()),
                DomainEventActor::from(actor),
                actor.clone(),
            ),
            None => (
                JobExecutionPrincipal::System,
                DomainEventActor::system(),
                User::system_execution_actor(),
            ),
        };
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                event_actor.clone(),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;

        let log_span = job_log_span(&run, &log_actor);
        let app = self.clone();
        let script_id = script_id.to_string();
        tokio::spawn(
            async move {
                if let Err(error) = app.run_job_run(run, principal, event_actor).await {
                    warn!(script_id = %script_id, error = %error, "scheduled script run failed");
                }
            }
            .instrument(log_span),
        );
        Ok(Some(run_payload))
    }

    /// Job body for a scheduled script. Loads the script row and the
    /// interpreter settings once for this run.
    pub(super) async fn execute_custom_job(
        &self,
        run: &JobRunRecord,
    ) -> AppResult<JobExecutionOutcome> {
        let script_id =
            custom_job_id_from_operation_type(&run.operation_type).ok_or_else(|| {
                AppError::Validation("custom job run does not name a script".to_string())
            })?;
        let script = self
            .services
            .customization
            .pp_scripts
            .get_script(script_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!("scheduled script {script_id} no longer exists"))
            })?;
        ensure_runnable_custom_job(&script)?;

        let scripts_dir = self.services.config.scripts_dir.clone();
        tokio::fs::create_dir_all(&scripts_dir)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "could not create scripts directory {}: {error}",
                    scripts_dir.display()
                ))
            })?;

        let trigger = custom_job_trigger_label(run.trigger_source);
        let metadata = json!({
            "event": CUSTOM_JOB_EVENT,
            "job_id": script.id,
            "job_name": script.name,
            "run_id": run.id,
            "trigger_source": trigger,
        });
        let env_json = metadata.to_string();
        let invocation = ScriptInvocation {
            script_id: script.id.clone(),
            source: script_source(&script),
            env: vec![
                ("SCRYER_METADATA".to_string(), env_json.clone()),
                ("SCRYER_EVENT".to_string(), CUSTOM_JOB_EVENT.to_string()),
                ("SCRYER_JOB_ID".to_string(), script.id.clone()),
                ("SCRYER_JOB_NAME".to_string(), script.name.clone()),
                ("SCRYER_RUN_ID".to_string(), run.id.clone()),
                ("SCRYER_TRIGGER_SOURCE".to_string(), trigger.to_string()),
            ],
            cwd: scripts_dir.clone(),
            timeout: Duration::from_secs(script.timeout_secs.max(1) as u64),
            capture_output: true,
            interpreters: self.script_interpreter_config().await,
            materialize_root: scripts_dir,
        };
        info!(script_id = %script.id, script_name = %script.name, trigger, "running scheduled script");

        let script_run_id = Id::new().0;
        match script.execution_mode {
            ExecutionMode::Blocking => {
                let execution = run_script(invocation).await;
                let outcome = execution.outcome.clone();
                let record =
                    script_run_record(&script, true, &env_json, script_run_id.clone(), execution);
                let summary_text = custom_job_summary_text(&outcome);
                let succeeded = record.status == ScriptRunStatus::Success;
                let summary_json = json!({
                    "exit_code": record.exit_code,
                    "duration_ms": record.duration_ms,
                    "script_run_id": script_run_id,
                })
                .to_string();
                self.services
                    .customization
                    .pp_scripts
                    .record_run(record)
                    .await?;
                let mut outcome = JobExecutionOutcome::new(Some(summary_text), Some(summary_json));
                if !succeeded {
                    outcome.status_override = Some(JobRunStatus::Failed);
                }
                Ok(outcome)
            }
            ExecutionMode::FireAndForget => {
                let started_at = Utc::now().to_rfc3339();
                self.services
                    .customization
                    .pp_scripts
                    .record_run(PostProcessingScriptRun {
                        id: script_run_id.clone(),
                        script_id: script.id.clone(),
                        script_name: script.name.clone(),
                        title_id: None,
                        title_name: None,
                        facet: None,
                        file_path: None,
                        status: ScriptRunStatus::Running,
                        exit_code: None,
                        stdout_tail: None,
                        stderr_tail: None,
                        duration_ms: None,
                        env_payload_json: Some(env_json.clone()),
                        started_at: started_at.clone(),
                        completed_at: None,
                    })
                    .await?;
                let app = self.clone();
                let detached_run_id = script_run_id.clone();
                tokio::spawn(
                    async move {
                        let execution = run_script(invocation).await;
                        let record = PostProcessingScriptRun {
                            started_at,
                            ..script_run_record(
                                &script,
                                true,
                                &env_json,
                                detached_run_id,
                                execution,
                            )
                        };
                        if let Err(error) = app
                            .services
                            .customization
                            .pp_scripts
                            .update_run(record)
                            .await
                        {
                            warn!(
                                script_id = %script.id,
                                error = %error,
                                "failed to record the end of a scheduled script run"
                            );
                        }
                    }
                    .in_current_span(),
                );
                Ok(JobExecutionOutcome::new(
                    Some("Started".to_string()),
                    Some(json!({ "script_run_id": script_run_id }).to_string()),
                ))
            }
        }
    }
}

#[async_trait]
impl CustomJobSource for AppUseCase {
    async fn load_scheduled_scripts(&self) -> AppResult<Vec<PostProcessingScript>> {
        self.services
            .customization
            .pp_scripts
            .list_enabled_scheduled()
            .await
    }
}

#[async_trait]
impl CustomJobDispatcher for AppUseCase {
    async fn dispatch(&self, due: DueCustomJob) {
        if let Err(error) = self
            .start_custom_job_run(&due.script_id, due.execution_mode, due.trigger_source, None)
            .await
        {
            warn!(script_id = %due.script_id, error = %error, "could not start scheduled script");
        }
    }
}

/// Run the scheduler for user-defined scheduled scripts until `token` is
/// cancelled. One task serves every script.
pub async fn start_background_custom_job_scheduler(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    let scheduler = app.runtime.jobs.custom_job_scheduler.clone();
    run_custom_job_scheduler(scheduler, &app, &app, token).await;
}
