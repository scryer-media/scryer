use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use scryer_domain::MediaFacet;
use crate::{ActivityChannel, ActivityKind, ActivitySeverity, AppUseCase};

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

/// Spawn a post-processing script for an imported file.
/// Returns immediately; the script runs in the background and emits an activity
/// event when it finishes (or fails or times out).
pub fn spawn_post_processing(ctx: PostProcessingContext) {
    tokio::spawn(async move {
        if let Err(err) = run_post_processing(ctx).await {
            tracing::warn!(error = %err, "post-processing task error");
        }
    });
}

/// Run a post-processing script for an imported file and await completion.
///
/// This is the same logic as [`spawn_post_processing`] but awaitable, which
/// makes it suitable for integration tests that need deterministic results.
pub async fn run_post_processing(ctx: PostProcessingContext) -> crate::AppResult<()> {
    let script_key = match ctx.facet {
        MediaFacet::Movie  => "post_processing.script.movie",
        MediaFacet::Tv     => "post_processing.script.series",
        MediaFacet::Anime  => "post_processing.script.anime",
        MediaFacet::Other  => return Ok(()),
    };

    let command = ctx
        .app
        .read_setting_string_value(script_key, None)
        .await
        .unwrap_or(None)
        .unwrap_or_default();

    if command.trim().is_empty() {
        return Ok(());
    }

    let timeout_secs: u64 = ctx
        .app
        .read_setting_string_value("post_processing.timeout_secs", None)
        .await
        .unwrap_or(None)
        .and_then(|v| v.parse().ok())
        .unwrap_or(1800);

    let facet_str = match ctx.facet {
        MediaFacet::Movie  => "movie",
        MediaFacet::Tv     => "series",
        MediaFacet::Anime  => "anime",
        MediaFacet::Other  => "other",
    };

    let envs: Vec<(&'static str, String)> = vec![
        ("SCRYER_EVENT",      "post_import".into()),
        ("SCRYER_FACET",      facet_str.into()),
        ("SCRYER_FILE_PATH",  ctx.dest_path.to_string_lossy().into_owned()),
        ("SCRYER_TITLE_NAME", ctx.title_name.clone()),
        ("SCRYER_TITLE_ID",   ctx.title_id.clone()),
        ("SCRYER_YEAR",       ctx.year.map(|y| y.to_string()).unwrap_or_default()),
        ("SCRYER_IMDB_ID",    ctx.imdb_id.unwrap_or_default()),
        ("SCRYER_TVDB_ID",    ctx.tvdb_id.unwrap_or_default()),
        ("SCRYER_SEASON",     ctx.season.map(|s| s.to_string()).unwrap_or_default()),
        ("SCRYER_EPISODE",    ctx.episode.map(|e| e.to_string()).unwrap_or_default()),
        ("SCRYER_QUALITY",    ctx.quality.unwrap_or_default()),
    ];

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
        c
    };

    cmd.envs(envs)
        .current_dir(&cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    tracing::info!(
        title = %ctx.title_name,
        facet = %facet_str,
        file = %ctx.dest_path.display(),
        "running post-processing script"
    );

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            let message = format!(
                "Post-processing script failed to start for '{}': {err}",
                ctx.title_name
            );
            tracing::warn!(%message);
            let _ = ctx.app.services
                .record_activity_event(
                    ctx.actor_id,
                    Some(ctx.title_id),
                    ActivityKind::PostProcessingCompleted,
                    message,
                    ActivitySeverity::Warning,
                    vec![ActivityChannel::WebUi],
                )
                .await;
            return Ok(());
        }
    };

    // Take pipe handles before waiting so we can drain them without consuming
    // the child — this lets us call child.kill() on timeout.
    let stderr_pipe = child.stderr.take();
    let stdout_pipe = child.stdout.take();

    let drain_stderr = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf).await;
        }
        buf
    });
    // Drain stdout to prevent pipe deadlock (output is discarded).
    tokio::spawn(async move {
        if let Some(mut pipe) = stdout_pipe {
            let _ = tokio::io::copy(&mut pipe, &mut tokio::io::sink()).await;
        }
    });

    match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait(),
    )
    .await
    {
        Ok(Ok(status)) => {
            if status.success() {
                let message = format!(
                    "Post-processing succeeded for '{}'",
                    ctx.title_name
                );
                tracing::info!(%message);
                let _ = ctx.app.services
                    .record_activity_event(
                        ctx.actor_id,
                        Some(ctx.title_id),
                        ActivityKind::PostProcessingCompleted,
                        message,
                        ActivitySeverity::Success,
                        vec![ActivityChannel::WebUi],
                    )
                    .await;
            } else {
                let code = status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".into());
                let stderr_bytes = drain_stderr.await.unwrap_or_default();
                let stderr_tail = last_bytes_utf8(&stderr_bytes, 2048);
                let message = format!(
                    "Post-processing failed (exit {code}) for '{}': {stderr_tail}",
                    ctx.title_name
                );
                tracing::warn!(%message);
                let _ = ctx.app.services
                    .record_activity_event(
                        ctx.actor_id,
                        Some(ctx.title_id),
                        ActivityKind::PostProcessingCompleted,
                        message,
                        ActivitySeverity::Warning,
                        vec![ActivityChannel::WebUi],
                    )
                    .await;
            }
        }
        Ok(Err(err)) => {
            let message = format!(
                "Post-processing I/O error for '{}': {err}",
                ctx.title_name
            );
            tracing::warn!(%message);
            let _ = ctx.app.services
                .record_activity_event(
                    ctx.actor_id,
                    Some(ctx.title_id),
                    ActivityKind::PostProcessingCompleted,
                    message,
                    ActivitySeverity::Warning,
                    vec![ActivityChannel::WebUi],
                )
                .await;
        }
        Err(_elapsed) => {
            let _ = child.kill().await;
            let message = format!(
                "Post-processing timed out after {timeout_secs}s for '{}'",
                ctx.title_name
            );
            tracing::warn!(%message);
            let _ = ctx.app.services
                .record_activity_event(
                    ctx.actor_id,
                    Some(ctx.title_id),
                    ActivityKind::PostProcessingCompleted,
                    message,
                    ActivitySeverity::Warning,
                    vec![ActivityChannel::WebUi],
                )
                .await;
        }
    }

    Ok(())
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
