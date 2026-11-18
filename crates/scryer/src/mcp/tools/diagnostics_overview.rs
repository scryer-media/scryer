use super::{
    Args, ToolContext, ToolFuture, ToolSpec, admit_diagnostics, diagnostics_error, free_text,
};
use crate::diagnostics_routes::buffered_log_lines;
use rmcp::model::CallToolResult;
use scryer_application::AppError;
use scryer_application::diagnostics::{
    LogFileRead, LogSummary, read_log_files_unless_cancelled, sanitization_facts,
    summarize_log_lines,
};
use serde_json::{Value, json};

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "get_diagnostics_overview",
    title: "Diagnostics overview",
    description: "Where to start a diagnosis: the version, log line counts by level for the \
in-memory buffer and the log file with the most frequent WARN and ERROR targets, whether metrics \
are on, and how values are sanitized. Host facts (build, installation, operating system, \
container and the rest) are in read_diagnostics_section, sanitized like the bundle; the \
installation kind and upgrade owner are also in get_upgrade_status. Needs the system settings \
permission and a full session.",
    params: &[],
    run,
};

/// The host sections the overview leaves to `read_diagnostics_section`,
/// which sanitizes them with the bundle's harvest. The overview runs no
/// harvest, so it shows no raw host value.
const HOST_SECTIONS: &[&str] = &["build", "installation", "os", "container"];

fn run(context: ToolContext, _args: Args) -> ToolFuture {
    Box::pin(async move {
        if let Some(refused) = admit_diagnostics(&context).await {
            return Ok(refused);
        }
        let host = context.diagnostics.clone();
        let cancelled = context.cancelled.clone();
        let read = tokio::task::spawn_blocking(move || {
            let buffer = summarize_log_lines(buffered_log_lines(&host).iter().map(String::as_str));
            let file = match host.process.log_file.as_deref() {
                Some(path) => read_log_files_unless_cancelled(path, &cancelled),
                None => LogFileRead::absent(),
            };
            let file_summary = summarize_log_lines(file.lines.iter().map(String::as_str));
            (buffer, file, file_summary, host.metrics.is_some())
        })
        .await;
        let Ok((buffer, file, file_summary, metrics_enabled)) = read else {
            return Ok(diagnostics_error(AppError::Repository(
                "diagnostics overview task failed".into(),
            )));
        };
        let result = json!({
            "version": env!("CARGO_PKG_VERSION"),
            "logs": {
                "buffer": log_summary(&buffer),
                "file": {
                    "configured": file.configured,
                    "sources": file.sources,
                    "failures": file
                        .failures
                        .iter()
                        .map(|(source, reason)| json!({"source": source, "reason": reason}))
                        .collect::<Vec<_>>(),
                    "read_capped": file.read_capped,
                    "summary": log_summary(&file_summary),
                },
            },
            "metrics_enabled": metrics_enabled,
            "sanitization": sanitization_facts(),
            "host_sections": HOST_SECTIONS,
        });
        Ok(CallToolResult::structured(result))
    })
}

/// Levels and targets come from log lines, so they pass the pattern
/// sanitizer like any other free text.
fn log_summary(summary: &LogSummary) -> Value {
    let by_level = summary
        .by_level
        .iter()
        .map(|(level, count)| (free_text(level), json!(count)))
        .collect::<serde_json::Map<_, _>>();
    json!({
        "lines": summary.lines,
        "by_level": by_level,
        "non_json_lines": summary.non_json_lines,
        "top_warn_error_targets": summary
            .top_warn_error_targets
            .iter()
            .map(|(target, count)| json!({"target": free_text(target), "count": count}))
            .collect::<Vec<_>>(),
    })
}
