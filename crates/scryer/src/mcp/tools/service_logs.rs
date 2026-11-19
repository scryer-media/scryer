use super::{
    Arguments, ToolContext, ToolEntry, ToolFuture, admit_diagnostics, coded_error,
    diagnostics_error, invalid_argument, local_read, omit_empty, tool,
};
use crate::diagnostics_routes::{buffered_log_lines, host_identifying};
use crate::error_reasons::HTTP_MCP_LOG_PATTERN_INVALID;
use crate::mcp::server::json_result;
use scryer_application::AppError;
use scryer_application::diagnostics::{
    LOG_PATTERN_MAX_CHARS, LogCursor, LogFileRead, LogLevel, LogPattern, LogPatternRefusal,
    LogSearch, LogSource, read_log_files_unless_cancelled, search_log_lines,
};
use serde_json::{Map, Value, json};

/// Characters of matching lines one result holds. With the line count cap
/// this keeps a result near 5,000 tokens.
const LINES_BUDGET_CHARS: usize = 16_000;

/// Lines one call reads and sanitizes at most.
pub(crate) const MAX_LINES_SCANNED: usize = 20_000;

const DEFAULT_MAX_LINES: i64 = 40;

pub(super) fn entry() -> ToolEntry {
    let levels = LogLevel::ALL
        .iter()
        .map(|level| level.as_str())
        .collect::<Vec<_>>();
    ToolEntry {
        tool: tool(
            "search_service_logs",
            "Searches sanitized service logs newest first by regex and level; page with before: next_cursor. Needs the system settings permission.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "level": {"enum": levels},
                    "source": {"enum": ["buffer", "file", "both"]},
                    "max_lines": {"type": "integer", "minimum": 1, "maximum": 100},
                    "before": {"type": "string"},
                },
            }),
            local_read(),
        ),
        accepts: &["pattern", "level", "source", "max_lines", "before"],
        run,
        class: super::api_class,
    }
}

fn run(context: ToolContext, arguments: Arguments) -> ToolFuture {
    Box::pin(async move {
        let source = match arguments.str("source", 10)? {
            None | Some("both") => LogSource::Both,
            Some("buffer") => LogSource::Buffer,
            Some("file") => LogSource::File,
            Some(_) => return Err(invalid_argument("source", "must be buffer, file or both")),
        };
        let min_level = match arguments.str("level", 20)? {
            None => None,
            Some(level) => match LogLevel::parse(level) {
                Some(level) => Some(level),
                None => {
                    return Err(invalid_argument(
                        "level",
                        "must be one of the listed levels",
                    ));
                }
            },
        };
        let max_lines = arguments
            .int("max_lines", 1, 100)?
            .unwrap_or(DEFAULT_MAX_LINES) as usize;
        let before = match arguments.str("before", 40)? {
            None => None,
            Some(cursor) => match LogCursor::parse(cursor) {
                Some(cursor) => Some(cursor),
                None => return Err(invalid_argument("before", "must be a next_cursor value")),
            },
        };
        let pattern = arguments
            .str("pattern", LOG_PATTERN_MAX_CHARS)?
            .map(str::to_string);
        if let Some(refused) = admit_diagnostics(&context).await {
            return Ok(refused);
        }
        let pattern = match pattern.as_deref().map(LogPattern::new) {
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
            min_level,
            source,
            max_lines,
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
            return Ok(json_result(
                json!({"logs_unavailable": reason, "returned": 0}),
            ));
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
        let mut file_facts = Map::new();
        file_facts.insert("configured".into(), json!(file.configured));
        file_facts.insert(
            "failures".into(),
            Value::Array(
                file.failures
                    .iter()
                    .map(|(source, reason)| json!({"source": source, "reason": reason}))
                    .collect(),
            ),
        );
        if file.read_capped {
            file_facts.insert("read_capped".into(), json!(true));
        }
        let mut found = Map::new();
        found.insert("scanned".into(), json!(result.scanned));
        found.insert("file".into(), Value::Object(omit_empty(file_facts)));
        if result.scan_capped {
            found.insert("scan_capped".into(), json!(true));
        }
        found.insert("returned".into(), json!(lines.len()));
        found.insert("lines".into(), Value::Array(lines));
        found.insert(
            "next_cursor".into(),
            json!(result.next_cursor.map(LogCursor::render)),
        );
        Ok(json_result(Value::Object(omit_empty(found))))
    })
}
