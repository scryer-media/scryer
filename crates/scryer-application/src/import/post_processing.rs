use crate::domain_events::DomainEventActor;
use crate::stored_paths::path_to_stored_string;
use crate::{AppError, AppUseCase};
use chrono::Utc;
use scryer_domain::{
    ConfigurationChangeAction, DomainEventPayload, DomainEventStream, DomainExternalIds,
    ExecutionMode, Id, MediaFacet, NewDomainEvent, PostProcessingCompletedEventData,
    PostProcessingResult, PostProcessingScript, PostProcessingScriptRun, ScriptRunStatus,
    ScriptType, TitleContextSnapshot, User,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// How much of each captured stream a run keeps. The store compresses the
/// tails (zstd) so a larger window costs little at rest.
const OUTPUT_TAIL_BYTES: usize = 32 * 1024;

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
    pub async fn list_post_processing_scripts(
        &self,
        actor: &User,
    ) -> crate::AppResult<Vec<PostProcessingScript>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.services.customization.pp_scripts.list_scripts().await
    }

    pub async fn list_post_processing_script_runs(
        &self,
        actor: &User,
        script_id: &str,
        limit: usize,
    ) -> crate::AppResult<Vec<PostProcessingScriptRun>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.services.customization.pp_scripts.get_script(id).await
    }

    pub async fn update_post_processing_script(
        &self,
        actor: &User,
        script: PostProcessingScript,
    ) -> crate::AppResult<PostProcessingScript> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let existing = self
            .services
            .customization
            .pp_scripts
            .get_script(id)
            .await?;
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let mut script = self
            .services
            .customization
            .pp_scripts
            .get_script(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("script {id} not found")))?;
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
        let run = execute_script(script, &ctx, facet_str, &env_json).await;
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
            let run = execute_script(&script, &ff_ctx, &facet_str_owned, &env_json).await;
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

fn validate_file_script_path(script_content: &str) -> Result<&str, &'static str> {
    let path = script_content.trim();
    if path.is_empty() {
        return Err("file script path is empty");
    }
    if !Path::new(path).is_absolute() {
        return Err("file script path must be absolute");
    }
    Ok(path)
}

fn build_post_processing_command(script: &PostProcessingScript) -> Result<Command, &'static str> {
    match script.script_type {
        ScriptType::Inline => Ok(build_inline_script_command(&script.script_content)),
        ScriptType::File => validate_file_script_path(&script.script_content).map(Command::new),
    }
}

#[cfg(windows)]
fn build_inline_script_command(script_content: &str) -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", script_content]);
    command
}

#[cfg(not(windows))]
fn build_inline_script_command(script_content: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script_content]);
    command
}

fn file_script_path_failure_run(
    script: &PostProcessingScript,
    ctx: &PostProcessingContext,
    facet_str: &str,
    env_json: &str,
    run_id: String,
    started_at: String,
    reason: &str,
) -> PostProcessingScriptRun {
    let completed_at = Utc::now().to_rfc3339();
    PostProcessingScriptRun {
        id: run_id,
        script_id: script.id.clone(),
        script_name: script.name.clone(),
        title_id: Some(ctx.title_id.clone()),
        title_name: Some(ctx.title_name.clone()),
        facet: Some(facet_str.to_string()),
        file_path: Some(path_to_stored_string(&ctx.dest_path)),
        status: ScriptRunStatus::Failed,
        exit_code: None,
        stdout_tail: None,
        stderr_tail: if script.debug {
            Some(format!("spawn error: {reason}"))
        } else {
            None
        },
        duration_ms: Some(0),
        env_payload_json: Some(env_json.to_string()),
        started_at,
        completed_at: Some(completed_at),
    }
}

async fn execute_script(
    script: &PostProcessingScript,
    ctx: &PostProcessingContext,
    facet_str: &str,
    env_json: &str,
) -> PostProcessingScriptRun {
    let run_id = Id::new().0;
    let started_at = Utc::now().to_rfc3339();

    let cwd = ctx
        .dest_path
        .parent()
        .unwrap_or(Path::new("/"))
        .to_path_buf();

    let mut cmd = match build_post_processing_command(script) {
        Ok(command) => command,
        Err(reason) => {
            return file_script_path_failure_run(
                script, ctx, facet_str, env_json, run_id, started_at, reason,
            );
        }
    };
    #[cfg(not(windows))]
    {
        // Create a new process group so we can kill the entire tree on timeout,
        // not just the direct child process.
        unsafe {
            cmd.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }
    }

    cmd.env("SCRYER_METADATA", env_json)
        .env("SCRYER_EVENT", "post_import")
        .env(
            "SCRYER_FILE_PATH",
            ctx.dest_path.to_string_lossy().as_ref() as &str,
        )
        .env("SCRYER_FACET", facet_str)
        .env("SCRYER_TITLE_NAME", &ctx.title_name)
        .env("SCRYER_TITLE_ID", &ctx.title_id)
        .current_dir(&cwd);

    if script.debug {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }

    tracing::info!(
        script_name = %script.name,
        title = %ctx.title_name,
        facet = %facet_str,
        file = %ctx.dest_path.display(),
        "running post-processing script"
    );

    let start_instant = std::time::Instant::now();

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            let completed_at = Utc::now().to_rfc3339();
            let duration_ms = start_instant.elapsed().as_millis() as i64;
            tracing::warn!(
                script = %script.name,
                error = %err,
                "post-processing script failed to start"
            );
            return PostProcessingScriptRun {
                id: run_id,
                script_id: script.id.clone(),
                script_name: script.name.clone(),
                title_id: Some(ctx.title_id.clone()),
                title_name: Some(ctx.title_name.clone()),
                facet: Some(facet_str.to_string()),
                file_path: Some(path_to_stored_string(&ctx.dest_path)),
                status: ScriptRunStatus::Failed,
                exit_code: None,
                stdout_tail: None,
                stderr_tail: if script.debug {
                    Some(format!("spawn error: {err}"))
                } else {
                    None
                },
                duration_ms: Some(duration_ms),
                env_payload_json: Some(env_json.to_string()),
                started_at,
                completed_at: Some(completed_at),
            };
        }
    };

    let timeout = std::time::Duration::from_secs(script.timeout_secs.max(1) as u64);

    if script.debug {
        // Capture stdout/stderr (last OUTPUT_TAIL_BYTES of each).
        let stderr_pipe = child.stderr.take();
        let stdout_pipe = child.stdout.take();

        let drain_stderr = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut pipe) = stderr_pipe {
                let _ = pipe.read_to_end(&mut buf).await;
            }
            buf
        });
        let drain_stdout = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut pipe) = stdout_pipe {
                let _ = pipe.read_to_end(&mut buf).await;
            }
            buf
        });

        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => {
                let duration_ms = start_instant.elapsed().as_millis() as i64;
                let completed_at = Utc::now().to_rfc3339();
                let stdout_bytes = drain_stdout.await.unwrap_or_default();
                let stderr_bytes = drain_stderr.await.unwrap_or_default();
                PostProcessingScriptRun {
                    id: run_id,
                    script_id: script.id.clone(),
                    script_name: script.name.clone(),
                    title_id: Some(ctx.title_id.clone()),
                    title_name: Some(ctx.title_name.clone()),
                    facet: Some(facet_str.to_string()),
                    file_path: Some(path_to_stored_string(&ctx.dest_path)),
                    status: if status.success() {
                        ScriptRunStatus::Success
                    } else {
                        ScriptRunStatus::Failed
                    },
                    exit_code: status.code(),
                    stdout_tail: Some(last_bytes_utf8(&stdout_bytes, OUTPUT_TAIL_BYTES)),
                    stderr_tail: Some(last_bytes_utf8(&stderr_bytes, OUTPUT_TAIL_BYTES)),
                    duration_ms: Some(duration_ms),
                    env_payload_json: Some(env_json.to_string()),
                    started_at,
                    completed_at: Some(completed_at),
                }
            }
            Ok(Err(err)) => {
                let duration_ms = start_instant.elapsed().as_millis() as i64;
                let completed_at = Utc::now().to_rfc3339();
                PostProcessingScriptRun {
                    id: run_id,
                    script_id: script.id.clone(),
                    script_name: script.name.clone(),
                    title_id: Some(ctx.title_id.clone()),
                    title_name: Some(ctx.title_name.clone()),
                    facet: Some(facet_str.to_string()),
                    file_path: Some(path_to_stored_string(&ctx.dest_path)),
                    status: ScriptRunStatus::Failed,
                    exit_code: None,
                    stdout_tail: None,
                    stderr_tail: Some(format!("I/O error: {err}")),
                    duration_ms: Some(duration_ms),
                    env_payload_json: Some(env_json.to_string()),
                    started_at,
                    completed_at: Some(completed_at),
                }
            }
            Err(_elapsed) => {
                // Kill the entire process group (shell + children), not just the shell.
                #[cfg(unix)]
                if let Some(pid) = child.id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill().await;
                let duration_ms = start_instant.elapsed().as_millis() as i64;
                let completed_at = Utc::now().to_rfc3339();
                let stdout_bytes = drain_stdout.await.unwrap_or_default();
                let stderr_bytes = drain_stderr.await.unwrap_or_default();
                PostProcessingScriptRun {
                    id: run_id,
                    script_id: script.id.clone(),
                    script_name: script.name.clone(),
                    title_id: Some(ctx.title_id.clone()),
                    title_name: Some(ctx.title_name.clone()),
                    facet: Some(facet_str.to_string()),
                    file_path: Some(path_to_stored_string(&ctx.dest_path)),
                    status: ScriptRunStatus::Timeout,
                    exit_code: None,
                    stdout_tail: Some(last_bytes_utf8(&stdout_bytes, OUTPUT_TAIL_BYTES)),
                    stderr_tail: Some(last_bytes_utf8(&stderr_bytes, OUTPUT_TAIL_BYTES)),
                    duration_ms: Some(duration_ms),
                    env_payload_json: Some(env_json.to_string()),
                    started_at,
                    completed_at: Some(completed_at),
                }
            }
        }
    } else {
        // No debug — output piped to /dev/null, only record status.
        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => {
                let duration_ms = start_instant.elapsed().as_millis() as i64;
                let completed_at = Utc::now().to_rfc3339();
                PostProcessingScriptRun {
                    id: run_id,
                    script_id: script.id.clone(),
                    script_name: script.name.clone(),
                    title_id: Some(ctx.title_id.clone()),
                    title_name: Some(ctx.title_name.clone()),
                    facet: Some(facet_str.to_string()),
                    file_path: Some(path_to_stored_string(&ctx.dest_path)),
                    status: if status.success() {
                        ScriptRunStatus::Success
                    } else {
                        ScriptRunStatus::Failed
                    },
                    exit_code: status.code(),
                    stdout_tail: None,
                    stderr_tail: None,
                    duration_ms: Some(duration_ms),
                    env_payload_json: None,
                    started_at,
                    completed_at: Some(completed_at),
                }
            }
            Ok(Err(_err)) => {
                let duration_ms = start_instant.elapsed().as_millis() as i64;
                let completed_at = Utc::now().to_rfc3339();
                PostProcessingScriptRun {
                    id: run_id,
                    script_id: script.id.clone(),
                    script_name: script.name.clone(),
                    title_id: Some(ctx.title_id.clone()),
                    title_name: Some(ctx.title_name.clone()),
                    facet: Some(facet_str.to_string()),
                    file_path: Some(path_to_stored_string(&ctx.dest_path)),
                    status: ScriptRunStatus::Failed,
                    exit_code: None,
                    stdout_tail: None,
                    stderr_tail: None,
                    duration_ms: Some(duration_ms),
                    env_payload_json: None,
                    started_at,
                    completed_at: Some(completed_at),
                }
            }
            Err(_elapsed) => {
                #[cfg(unix)]
                if let Some(pid) = child.id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill().await;
                let duration_ms = start_instant.elapsed().as_millis() as i64;
                let completed_at = Utc::now().to_rfc3339();
                PostProcessingScriptRun {
                    id: run_id,
                    script_id: script.id.clone(),
                    script_name: script.name.clone(),
                    title_id: Some(ctx.title_id.clone()),
                    title_name: Some(ctx.title_name.clone()),
                    facet: Some(facet_str.to_string()),
                    file_path: Some(path_to_stored_string(&ctx.dest_path)),
                    status: ScriptRunStatus::Timeout,
                    exit_code: None,
                    stdout_tail: None,
                    stderr_tail: None,
                    duration_ms: Some(duration_ms),
                    env_payload_json: None,
                    started_at,
                    completed_at: Some(completed_at),
                }
            }
        }
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

/// Return the last `max_bytes` of `buf` as a trimmed UTF-8 string.
fn last_bytes_utf8(buf: &[u8], max_bytes: usize) -> String {
    let slice = if buf.len() > max_bytes {
        &buf[buf.len() - max_bytes..]
    } else {
        buf
    };
    String::from_utf8_lossy(slice).trim().to_string()
}
