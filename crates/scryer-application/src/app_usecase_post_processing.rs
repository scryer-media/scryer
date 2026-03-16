use crate::{ActivityChannel, ActivityKind, ActivitySeverity, AppUseCase};
use chrono::Utc;
use scryer_domain::{Id, MediaFacet, PostProcessingScript, PostProcessingScriptRun};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncReadExt;

/// Context passed from the import pipeline into post-processing.
/// All fields that the caller already has are included here so the
/// execution engine does not need to re-query the database.
pub struct PostProcessingContext {
    /// Cheap clone of AppUseCase — all internal fields are Arc.
    pub app: AppUseCase,
    pub actor_id: Option<String>,
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
    let facet_str = match ctx.facet {
        MediaFacet::Movie => "movie",
        MediaFacet::Tv => "tv",
        MediaFacet::Anime => "anime",
        MediaFacet::Other => return Ok(()),
    };

    let scripts = ctx
        .app
        .services
        .pp_scripts
        .list_enabled_for_facet(facet_str)
        .await
        .unwrap_or_default();

    if scripts.is_empty() {
        return Ok(());
    }

    // Build the JSON metadata payload once for all scripts.
    let env_payload = build_script_env_payload(&ctx, facet_str);
    let env_json = serde_json::to_string(&env_payload).unwrap_or_default();

    // Partition by execution mode.
    let mut blocking: Vec<&PostProcessingScript> = scripts
        .iter()
        .filter(|s| s.execution_mode == "blocking")
        .collect();
    blocking.sort_by_key(|s| s.priority);

    let fire_and_forget: Vec<&PostProcessingScript> = scripts
        .iter()
        .filter(|s| s.execution_mode == "fire_and_forget")
        .collect();

    // Run blocking scripts sequentially in priority order.
    for script in &blocking {
        let run = execute_script(script, &ctx, facet_str, &env_json).await;
        log_run_activity(&ctx, &run).await;
        ctx.app.services.pp_scripts.record_run(run).await.ok();
    }

    // Fire-and-forget scripts run in parallel.
    for script in &fire_and_forget {
        let app = ctx.app.clone();
        let actor_id = ctx.actor_id.clone();
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
                actor_id,
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
            app.services.pp_scripts.record_run(run).await.ok();
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

async fn execute_script(
    script: &PostProcessingScript,
    ctx: &PostProcessingContext,
    facet_str: &str,
    env_json: &str,
) -> PostProcessingScriptRun {
    let run_id = Id::new().0;
    let started_at = Utc::now().to_rfc3339();

    let command = match script.script_type.as_str() {
        "file" => format!("exec {}", shell_escape(&script.script_content)),
        _ => script.script_content.clone(),
    };

    let cwd = ctx
        .dest_path
        .parent()
        .unwrap_or(Path::new("/"))
        .to_path_buf();

    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", &command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", &command]);
        // Create a new process group so we can kill the entire tree on timeout,
        // not just the shell wrapper (which would orphan child processes).
        unsafe {
            c.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }
        c
    };

    cmd.env("SCRYER_METADATA", env_json)
        .env("SCRYER_EVENT", "post_import")
        .env("SCRYER_FILE_PATH", ctx.dest_path.to_string_lossy().as_ref())
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
                file_path: Some(ctx.dest_path.to_string_lossy().to_string()),
                status: "failed".to_string(),
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
        // Capture stdout/stderr (last 4KB each).
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
                    file_path: Some(ctx.dest_path.to_string_lossy().to_string()),
                    status: if status.success() {
                        "success".to_string()
                    } else {
                        "failed".to_string()
                    },
                    exit_code: status.code(),
                    stdout_tail: Some(last_bytes_utf8(&stdout_bytes, 4096)),
                    stderr_tail: Some(last_bytes_utf8(&stderr_bytes, 4096)),
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
                    file_path: Some(ctx.dest_path.to_string_lossy().to_string()),
                    status: "failed".to_string(),
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
                    file_path: Some(ctx.dest_path.to_string_lossy().to_string()),
                    status: "timeout".to_string(),
                    exit_code: None,
                    stdout_tail: Some(last_bytes_utf8(&stdout_bytes, 4096)),
                    stderr_tail: Some(last_bytes_utf8(&stderr_bytes, 4096)),
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
                    file_path: Some(ctx.dest_path.to_string_lossy().to_string()),
                    status: if status.success() {
                        "success".to_string()
                    } else {
                        "failed".to_string()
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
                    file_path: Some(ctx.dest_path.to_string_lossy().to_string()),
                    status: "failed".to_string(),
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
                    file_path: Some(ctx.dest_path.to_string_lossy().to_string()),
                    status: "timeout".to_string(),
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
    let (severity, message) = match run.status.as_str() {
        "success" => (
            ActivitySeverity::Success,
            format!(
                "Post-processing '{}' succeeded for '{}'",
                run.script_name, ctx.title_name
            ),
        ),
        "timeout" => (
            ActivitySeverity::Warning,
            format!(
                "Post-processing '{}' timed out for '{}'",
                run.script_name, ctx.title_name
            ),
        ),
        _ => (
            ActivitySeverity::Warning,
            format!(
                "Post-processing '{}' failed (exit {}) for '{}'",
                run.script_name,
                run.exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "n/a".into()),
                ctx.title_name
            ),
        ),
    };

    let _ = ctx
        .app
        .services
        .record_activity_event(
            ctx.actor_id.clone(),
            Some(ctx.title_id.clone()),
            ActivityKind::PostProcessingCompleted,
            message,
            severity,
            vec![ActivityChannel::WebUi],
        )
        .await;
}

/// Escape a path for use in a shell command.
fn shell_escape(s: &str) -> String {
    // Wrap in single quotes, escaping any existing single quotes.
    format!("'{}'", s.replace('\'', "'\\''"))
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
