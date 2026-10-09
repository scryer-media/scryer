use crate::domain_events::DomainEventActor;
use crate::scripts::runner::{
    InterpreterConfig, ScriptExecution, ScriptInvocation, ScriptOutcome, ScriptSource, run_script,
};
use crate::stored_paths::path_to_stored_string;
use crate::{AppError, AppUseCase};
use chrono::Utc;
use scryer_domain::{
    AppPermission, ConfigurationChangeAction, DomainEventPayload, DomainEventStream,
    DomainExternalIds, ExecutionMode, Id, MediaFacet, NewDomainEvent,
    PostProcessingCompletedEventData, PostProcessingResult, PostProcessingScript,
    PostProcessingScriptRun, ScriptRunStatus, ScriptSchedule, ScriptTrigger, ScriptType,
    TitleContextSnapshot, User,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Context passed from the import pipeline into post-processing.
/// All fields that the caller already has are included here so the
/// execution engine does not need to re-query the database.
pub struct PostProcessingContext {
    /// Cheap clone of AppUseCase — all internal fields are Arc.
    pub app: AppUseCase,
    pub actor: DomainEventActor,
    pub title_id: String,
    pub title_name: String,
    pub facet: MediaFacet,
    pub dest_path: PathBuf,
    pub year: Option<i32>,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub quality: Option<String>,
}

impl AppUseCase {
    /// Catalog-settings permission covers import-triggered scripts. A
    /// scheduled script runs arbitrary code on the host on its own clock, so
    /// it also needs system-settings permission.
    async fn require_script_trigger_permission(
        &self,
        actor: &User,
        trigger: ScriptTrigger,
    ) -> crate::AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        if trigger == ScriptTrigger::Schedule {
            self.require_app_permission(actor, AppPermission::ManageSystemSettings)
                .await?;
        }
        Ok(())
    }

    /// Every script the caller may manage. Scheduled scripts are left out
    /// for callers without system-settings permission.
    pub async fn list_post_processing_scripts(
        &self,
        actor: &User,
    ) -> crate::AppResult<Vec<PostProcessingScript>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let scripts = self
            .services
            .customization
            .pp_scripts
            .list_scripts()
            .await?;
        if self
            .has_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?
        {
            return Ok(scripts);
        }
        Ok(scripts
            .into_iter()
            .filter(|script| script.trigger != ScriptTrigger::Schedule)
            .collect())
    }

    pub async fn list_post_processing_scripts_by_trigger(
        &self,
        actor: &User,
        trigger: ScriptTrigger,
    ) -> crate::AppResult<Vec<PostProcessingScript>> {
        self.require_script_trigger_permission(actor, trigger)
            .await?;
        self.services
            .customization
            .pp_scripts
            .list_scripts_by_trigger(trigger)
            .await
    }

    /// Checks a schedule being edited and previews when it would next fire.
    pub async fn validate_script_schedule(
        &self,
        actor: &User,
        schedule: &ScriptSchedule,
    ) -> crate::AppResult<crate::scripts::schedule::ScriptScheduleValidation> {
        self.require_script_trigger_permission(actor, ScriptTrigger::Schedule)
            .await?;
        Ok(crate::scripts::schedule::check_schedule(
            schedule,
            Utc::now(),
        ))
    }

    pub async fn list_post_processing_script_runs(
        &self,
        actor: &User,
        script_id: &str,
        limit: usize,
    ) -> crate::AppResult<Vec<PostProcessingScriptRun>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        self.services
            .customization
            .pp_scripts
            .list_runs_for_script(script_id, limit)
            .await
    }

    async fn finalize_post_processing_script_mutation(
        &self,
        actor: &User,
        script: &PostProcessingScript,
        action: ConfigurationChangeAction,
    ) {
        self.emit_configuration_changed_event(
            actor,
            post_processing_script_resource_type(script.script_type),
            Some(script.id.clone()),
            action,
        )
        .await;
    }

    pub async fn create_post_processing_script(
        &self,
        actor: &User,
        script: PostProcessingScript,
    ) -> crate::AppResult<PostProcessingScript> {
        self.require_script_trigger_permission(actor, script.trigger)
            .await?;
        let script = normalize_script_trigger(script)?;
        let created = self
            .services
            .customization
            .pp_scripts
            .create_script(script)
            .await?;
        self.finalize_post_processing_script_mutation(
            actor,
            &created,
            ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(created)
    }

    pub async fn get_post_processing_script(
        &self,
        actor: &User,
        id: &str,
    ) -> crate::AppResult<Option<PostProcessingScript>> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let script = self
            .services
            .customization
            .pp_scripts
            .get_script(id)
            .await?;
        if let Some(script) = &script {
            self.require_script_trigger_permission(actor, script.trigger)
                .await?;
        }
        Ok(script)
    }

    /// Replaces a script. The trigger is fixed at creation: an update whose
    /// trigger differs from the stored one is rejected, and permission is
    /// checked against the stored trigger.
    pub async fn update_post_processing_script(
        &self,
        actor: &User,
        script: PostProcessingScript,
    ) -> crate::AppResult<PostProcessingScript> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let stored = self.load_post_processing_script(&script.id).await?;
        self.require_script_trigger_permission(actor, stored.trigger)
            .await?;
        if script.trigger != stored.trigger {
            return Err(AppError::Validation(
                "a script's trigger cannot be changed after it is created".to_string(),
            ));
        }
        let script = normalize_script_trigger(script)?;
        let updated = self
            .services
            .customization
            .pp_scripts
            .update_script(script)
            .await?;
        self.finalize_post_processing_script_mutation(
            actor,
            &updated,
            ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(updated)
    }

    pub async fn delete_post_processing_script(
        &self,
        actor: &User,
        id: &str,
    ) -> crate::AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let existing = self
            .services
            .customization
            .pp_scripts
            .get_script(id)
            .await?;
        if let Some(script) = &existing {
            self.require_script_trigger_permission(actor, script.trigger)
                .await?;
        }
        self.services
            .customization
            .pp_scripts
            .delete_script(id)
            .await?;
        if let Some(script) = existing {
            self.finalize_post_processing_script_mutation(
                actor,
                &script,
                ConfigurationChangeAction::Deleted,
            )
            .await;
        }
        Ok(())
    }

    pub async fn toggle_post_processing_script(
        &self,
        actor: &User,
        id: &str,
    ) -> crate::AppResult<PostProcessingScript> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        let mut script = self.load_post_processing_script(id).await?;
        self.require_script_trigger_permission(actor, script.trigger)
            .await?;
        script.enabled = !script.enabled;
        script.updated_at = Utc::now();
        let updated = self
            .services
            .customization
            .pp_scripts
            .update_script(script)
            .await?;
        self.finalize_post_processing_script_mutation(
            actor,
            &updated,
            ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(updated)
    }

    async fn load_post_processing_script(
        &self,
        id: &str,
    ) -> crate::AppResult<PostProcessingScript> {
        self.services
            .customization
            .pp_scripts
            .get_script(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("script {id} not found")))
    }
}

/// Applies the per-trigger rules. A scheduled script needs a valid
/// schedule; its facets and priority only matter on import, so they are
/// stored as given and ignored. An import-triggered script keeps the import
/// rules unchanged and carries no schedule or startup run.
fn normalize_script_trigger(
    mut script: PostProcessingScript,
) -> crate::AppResult<PostProcessingScript> {
    match script.trigger {
        ScriptTrigger::Schedule => {
            let schedule = script.schedule.as_mut().ok_or_else(|| {
                AppError::Validation("scheduled scripts require a schedule".to_string())
            })?;
            if let ScriptSchedule::Weekly { days, .. } = schedule {
                days.sort();
                days.dedup();
            }
            crate::scripts::schedule::validate_schedule(schedule)
                .map_err(crate::scripts::schedule::ScheduleError::into_app_error)?;
        }
        ScriptTrigger::PostImport => {
            script.schedule = None;
            script.run_on_startup = false;
        }
    }
    Ok(script)
}

/// Spawn the post-processing pipeline for an imported file.
/// Returns immediately; the pipeline runs in the background and records
/// results per-script.
pub fn spawn_post_processing(ctx: PostProcessingContext) {
    tokio::spawn(async move {
        if let Err(err) = run_post_processing(ctx).await {
            tracing::warn!(error = %err, "post-processing pipeline error");
        }
    });
}

/// Run the full post-processing pipeline and await completion.
///
/// This is the same logic as [`spawn_post_processing`] but awaitable,
/// which makes it suitable for integration tests that need deterministic
/// results.
pub async fn run_post_processing(ctx: PostProcessingContext) -> crate::AppResult<()> {
    let facet_str = ctx.facet.as_str();

    let scripts = ctx
        .app
        .services
        .customization
        .pp_scripts
        .list_enabled_for_facet(facet_str)
        .await?;

    if scripts.is_empty() {
        return Ok(());
    }

    // Build the JSON metadata payload once for all scripts.
    let env_payload = build_script_env_payload(&ctx, facet_str);
    let env_json = serde_json::to_string(&env_payload).unwrap_or_default();
    let interpreters = ctx.app.script_interpreter_config().await;

    // Partition by execution mode.
    let mut blocking: Vec<&PostProcessingScript> = scripts
        .iter()
        .filter(|s| s.execution_mode == ExecutionMode::Blocking)
        .collect();
    blocking.sort_by_key(|s| s.priority);

    let fire_and_forget: Vec<&PostProcessingScript> = scripts
        .iter()
        .filter(|s| s.execution_mode == ExecutionMode::FireAndForget)
        .collect();

    // Run blocking scripts sequentially in priority order.
    for script in &blocking {
        let run = execute_script(script, &ctx, facet_str, &env_json, &interpreters).await;
        log_run_activity(&ctx, &run).await;
        persist_run_record(&ctx.app, run).await;
    }

    // Fire-and-forget scripts run in parallel.
    for script in &fire_and_forget {
        let app = ctx.app.clone();
        let actor = ctx.actor.clone();
        let title_id = ctx.title_id.clone();
        let title_name = ctx.title_name.clone();
        let dest_path = ctx.dest_path.clone();
        let facet = ctx.facet.clone();
        let env_json = env_json.clone();
        let interpreters = interpreters.clone();
        let script = (*script).clone();
        let facet_str_owned = facet_str.to_string();
        tokio::spawn(async move {
            let ff_ctx = PostProcessingContext {
                app: app.clone(),
                actor,
                title_id,
                title_name,
                facet,
                dest_path,
                year: None,
                imdb_id: None,
                tvdb_id: None,
                season: None,
                episode: None,
                quality: None,
            };
            let run =
                execute_script(&script, &ff_ctx, &facet_str_owned, &env_json, &interpreters).await;
            log_run_activity(&ff_ctx, &run).await;
            persist_run_record(&app, run).await;
        });
    }

    Ok(())
}

fn build_script_env_payload(ctx: &PostProcessingContext, facet_str: &str) -> serde_json::Value {
    json!({
        "event": "post_import",
        "facet": facet_str,
        "file_path": ctx.dest_path.to_string_lossy(),
        "title": {
            "id": ctx.title_id,
            "name": ctx.title_name,
            "year": ctx.year,
            "imdb_id": ctx.imdb_id,
            "tvdb_id": ctx.tvdb_id,
        },
        "episode": {
            "season": ctx.season,
            "episode": ctx.episode,
        },
        "release": {
            "quality": ctx.quality,
        },
    })
}

fn post_processing_script_resource_type(script_type: ScriptType) -> &'static str {
    match script_type {
        ScriptType::Inline => "post_processing_inline_script",
        ScriptType::File => "post_processing_script",
    }
}

fn script_source(script: &PostProcessingScript) -> ScriptSource {
    match script.script_type {
        ScriptType::Inline => ScriptSource::Inline {
            content: script.script_content.clone(),
            language: script.language,
        },
        ScriptType::File => ScriptSource::File {
            path: script.script_content.clone(),
        },
    }
}

async fn execute_script(
    script: &PostProcessingScript,
    ctx: &PostProcessingContext,
    facet_str: &str,
    env_json: &str,
    interpreters: &InterpreterConfig,
) -> PostProcessingScriptRun {
    let run_id = Id::new().0;

    let cwd = ctx
        .dest_path
        .parent()
        .unwrap_or(Path::new("/"))
        .to_path_buf();

    let invocation = ScriptInvocation {
        script_id: script.id.clone(),
        source: script_source(script),
        env: vec![
            ("SCRYER_METADATA".to_string(), env_json.to_string()),
            ("SCRYER_EVENT".to_string(), "post_import".to_string()),
            (
                "SCRYER_FILE_PATH".to_string(),
                ctx.dest_path.to_string_lossy().into_owned(),
            ),
            ("SCRYER_FACET".to_string(), facet_str.to_string()),
            ("SCRYER_TITLE_NAME".to_string(), ctx.title_name.clone()),
            ("SCRYER_TITLE_ID".to_string(), ctx.title_id.clone()),
        ],
        cwd,
        timeout: Duration::from_secs(script.timeout_secs.max(1) as u64),
        capture_output: script.debug,
        interpreters: interpreters.clone(),
        materialize_root: ctx.app.services.config.scripts_dir.clone(),
    };

    tracing::info!(
        script_name = %script.name,
        title = %ctx.title_name,
        facet = %facet_str,
        file = %ctx.dest_path.display(),
        "running post-processing script"
    );

    let execution = run_script(invocation).await;
    if let ScriptOutcome::SpawnFailed { reason } = &execution.outcome {
        tracing::warn!(
            script = %script.name,
            error = %reason,
            "post-processing script failed to start"
        );
    }
    script_run_record(script, ctx, facet_str, env_json, run_id, execution)
}

/// Map a runner result onto the persisted run record. Output tails and the
/// metadata payload are kept only for debug scripts, except that a launch
/// failure always records its payload.
fn script_run_record(
    script: &PostProcessingScript,
    ctx: &PostProcessingContext,
    facet_str: &str,
    env_json: &str,
    run_id: String,
    execution: ScriptExecution,
) -> PostProcessingScriptRun {
    let debug_env_payload = || script.debug.then(|| env_json.to_string());
    let (status, exit_code, stdout_tail, stderr_tail, env_payload_json) = match execution.outcome {
        ScriptOutcome::Exited { code, success } => (
            if success {
                ScriptRunStatus::Success
            } else {
                ScriptRunStatus::Failed
            },
            code,
            execution.stdout_tail,
            execution.stderr_tail,
            debug_env_payload(),
        ),
        ScriptOutcome::TimedOut => (
            ScriptRunStatus::Timeout,
            None,
            execution.stdout_tail,
            execution.stderr_tail,
            debug_env_payload(),
        ),
        ScriptOutcome::IoError { reason } => (
            ScriptRunStatus::Failed,
            None,
            None,
            script.debug.then(|| format!("I/O error: {reason}")),
            debug_env_payload(),
        ),
        ScriptOutcome::SpawnFailed { reason } => (
            ScriptRunStatus::Failed,
            None,
            None,
            script.debug.then(|| format!("spawn error: {reason}")),
            Some(env_json.to_string()),
        ),
    };

    PostProcessingScriptRun {
        id: run_id,
        script_id: script.id.clone(),
        script_name: script.name.clone(),
        title_id: Some(ctx.title_id.clone()),
        title_name: Some(ctx.title_name.clone()),
        facet: Some(facet_str.to_string()),
        file_path: Some(path_to_stored_string(&ctx.dest_path)),
        status,
        exit_code,
        stdout_tail,
        stderr_tail,
        duration_ms: Some(execution.duration_ms),
        env_payload_json,
        started_at: execution.started_at.to_rfc3339(),
        completed_at: Some(execution.completed_at.to_rfc3339()),
    }
}

async fn log_run_activity(ctx: &PostProcessingContext, run: &PostProcessingScriptRun) {
    let result = match run.status {
        ScriptRunStatus::Success => PostProcessingResult::Succeeded,
        ScriptRunStatus::Timeout => PostProcessingResult::TimedOut,
        _ => PostProcessingResult::Failed,
    };

    if let Ok(Some(title)) = ctx
        .app
        .services
        .catalog
        .titles
        .get_by_id(&ctx.title_id)
        .await
    {
        ctx.app
            .emit_post_processing_completed_event(
                ctx.actor.clone(),
                &title,
                run.script_name.clone(),
                result,
                run.exit_code,
            )
            .await;
        return;
    }

    let external_ids = DomainExternalIds {
        imdb_id: ctx.imdb_id.clone(),
        tvdb_id: ctx.tvdb_id.clone(),
        ..DomainExternalIds::default()
    };

    let _ = ctx
        .app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_kind: ctx.actor.kind,
            actor_user_id: ctx.actor.user_id.clone(),
            actor_display_name: ctx.actor.display_name.clone(),
            title_id: Some(ctx.title_id.clone()),
            facet: Some(ctx.facet.clone()),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: ctx.title_id.clone(),
            },
            payload: DomainEventPayload::PostProcessingCompleted(
                PostProcessingCompletedEventData {
                    title: TitleContextSnapshot {
                        title_name: ctx.title_name.clone(),
                        facet: ctx.facet.clone(),
                        external_ids,
                        poster_url: None,
                        year: ctx.year,
                    },
                    script_name: run.script_name.clone(),
                    result,
                    exit_code: run.exit_code,
                },
            ),
        })
        .await;
}

async fn persist_run_record(app: &AppUseCase, run: PostProcessingScriptRun) {
    let script_id = run.script_id.clone();
    let script_name = run.script_name.clone();
    let title_id = run.title_id.clone();

    if let Err(error) = app.services.customization.pp_scripts.record_run(run).await {
        tracing::warn!(
            error = %error,
            script_id = %script_id,
            script_name = %script_name,
            title_id = ?title_id,
            "failed to record post-processing script run"
        );
    }
}
