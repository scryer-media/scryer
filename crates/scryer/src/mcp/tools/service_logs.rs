use super::{
    Args, Kind, Param, ToolContext, ToolFuture, ToolSpec, admit_diagnostics, coded_error,
    diagnostics_error,
};
use crate::diagnostics_routes::{buffered_log_lines, host_identifying};
use crate::error_reasons::{HTTP_MCP_ARGUMENTS_INVALID, HTTP_MCP_LOG_PATTERN_INVALID};
use rmcp::model::CallToolResult;
use scryer_application::AppError;
use scryer_application::diagnostics::{
    LOG_PATTERN_MAX_CHARS, LogCursor, LogFileRead, LogLevel, LogPattern, LogPatternRefusal,
    LogSearch, LogSource, read_log_files_unless_cancelled, search_log_lines,
};
use serde_json::json;

/// Characters of matching lines one result holds. With the line count cap
/// this keeps a result near 5,000 tokens.
const LINES_BUDGET_CHARS: usize = 16_000;

/// Lines one call reads and sanitizes at most. The description states it.
pub(crate) const MAX_LINES_SCANNED: usize = 20_000;

fn levels() -> Vec<&'static str> {
    LogLevel::ALL.iter().map(|level| level.as_str()).collect()
}

fn sources() -> Vec<&'static str> {
    vec!["buffer", "file", "both"]
}

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "search_service_logs",
    title: "Search service logs",
    description: "Search the in-memory log buffer and the log file newest first, as the \
troubleshooting log search does: an optional case-insensitive regular expression over each \
structured line's message and target (or a plain line's text), an optional minimum level, and a \
before cursor for older pages. Lines are sanitized before they are matched or returned. One call \
reads at most 20,000 lines; when it stops there before the page is full it returns scan_capped \
true and a next_cursor that continues the search. The log keeps moving while you page, so a later \
page can repeat or skip lines; a fresh search without a cursor gives the current newest lines. \
Needs the system settings permission and a full session.",
    params: &[
        Param {
            name: "pattern",
            description: "A regular expression, matched case-insensitively.",
            required: false,
            kind: Kind::Str {
                max_chars: LOG_PATTERN_MAX_CHARS,
            },
        },
        Param {
            name: "level",
            description: "Only lines at this level or above.",
            required: false,
            kind: Kind::Enum {
                values: levels,
                default: None,
            },
        },
        Param {
            name: "source",
            description: "Which logs to read; with both, the buffer is read first.",
            required: false,
            kind: Kind::Enum {
                values: sources,
                default: Some("both"),
            },
        },
        Param {
            name: "max_lines",
            description: "Matching lines to return.",
            required: false,
            kind: Kind::Int {
                default: Some(40),
                min: 1,
                max: 100,
            },
        },
        Param {
            name: "before",
            description: "Continue from next_cursor.",
            required: false,
            kind: Kind::Str { max_chars: 40 },
        },
    ],
    run,
};

fn invalid(argument: &str, message: &str) -> rmcp::ErrorData {
    rmcp::ErrorData::invalid_params(
        message.to_string(),
        Some(json!({"reason": HTTP_MCP_ARGUMENTS_INVALID, "argument": argument})),
    )
}

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let source = match args.str("source") {
            Some("buffer") => LogSource::Buffer,
            Some("file") => LogSource::File,
            _ => LogSource::Both,
        };
        let before = match args.str("before") {
            None => None,
            Some(cursor) => match LogCursor::parse(cursor) {
                Some(cursor) => Some(cursor),
                None => {
                    return Err(invalid(
                        "before",
                        "argument `before` must be a next_cursor value",
                    ));
                }
            },
        };
        if let Some(refused) = admit_diagnostics(&context).await {
            return Ok(refused);
        }
        let pattern = match args.str("pattern").map(LogPattern::new) {
            None => None,
            Some(Ok(pattern)) => Some(pattern),
            Some(Err(refusal)) => {
                return Ok(coded_error(
                    "BAD_USER_INPUT",
                    HTTP_MCP_LOG_PATTERN_INVALID,
                    match refusal {
                        LogPatternRefusal::TooLong => "The pattern is too long.",
                        LogPatternRefusal::Invalid => {
                            "The pattern is not a valid regular expression, or it is too large \
once compiled."
                        }
                    },
                ));
            }
        };
        let search = LogSearch {
            pattern,
            min_level: args.str("level").and_then(LogLevel::parse),
            source,
            max_lines: args.count("max_lines"),
            max_chars: LINES_BUDGET_CHARS,
            max_scanned: MAX_LINES_SCANNED,
            before,
        };

        // One harvest per call, with the bundle's inputs, so pseudonyms in
        // the lines match the bundle's.
        let host = context.diagnostics.clone();
        let Ok((values, paths)) =
            tokio::task::spawn_blocking(move || host_identifying(&host)).await
        else {
            return Ok(diagnostics_error(AppError::Repository(
                "diagnostics identifying values task failed".into(),
            )));
        };
        let harvest = match context
            .auth
            .app
            .diagnostics_harvest(context.caller.user(), &values, &paths)
            .await
        {
            Ok(harvest) => harvest,
            Err(error) => return Ok(diagnostics_error(error)),
        };
        if let Some(reason) = harvest.incomplete_reason() {
            // A value the harvest missed would go unreplaced.
            return Ok(CallToolResult::structured(json!({
                "logs_unavailable": reason,
                "returned": 0,
                "lines": [],
                "next_cursor": null,
            })));
        }

        let host = context.diagnostics.clone();
        let cancelled = context.cancelled.clone();
        let searched = tokio::task::spawn_blocking(move || {
            let buffer = if matches!(source, LogSource::Buffer | LogSource::Both) {
                buffered_log_lines(&host)
            } else {
                Vec::new()
            };
            let file = match (source, host.process.log_file.as_deref()) {
                (LogSource::File | LogSource::Both, Some(path)) => {
                    read_log_files_unless_cancelled(path, &cancelled)
                }
                _ => LogFileRead::absent(),
            };
            let result = search_log_lines(
                &harvest.sanitizer,
                &buffer,
                &file.lines,
                &search,
                &cancelled,
            );
            (result, file)
        })
        .await;
        let Ok((result, file)) = searched else {
            return Ok(diagnostics_error(AppError::Repository(
                "log search task failed".into(),
            )));
        };
        let lines = result
            .lines
            .iter()
            .map(|hit| json!({"source": hit.source, "line": hit.line}))
            .collect::<Vec<_>>();
        Ok(CallToolResult::structured(json!({
            "scanned": result.scanned,
            "file": {
                "configured": file.configured,
                "failures": file
                    .failures
                    .iter()
                    .map(|(source, reason)| json!({"source": source, "reason": reason}))
                    .collect::<Vec<_>>(),
                "read_capped": file.read_capped,
            },
            "scan_capped": result.scan_capped,
            "returned": lines.len(),
            "lines": lines,
            "next_cursor": result.next_cursor.map(LogCursor::render),
        })))
    })
}
