//! The MCP protocol surface: the tool table from [`super::tools`], argument
//! admission and the mapping of GraphQL responses to tool results.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_graphql::Response as GraphQLResponse;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
    Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};

use super::tools::{self, ToolContext};
use crate::diagnostics_routes::DiagnosticsHost;
use crate::error_reasons::{HTTP_MCP_GRAPHQL_ERROR, HTTP_MCP_TOOL_UNKNOWN};
use crate::middleware::{ApiKeyCaller, AuthState};

const INSTRUCTIONS: &str = "Calls act as the API key's account, with its permissions. \
list_operations, then describe_operation for variables, then run_query or run_mutation. \
Errors: isError with errors[{code, reason, message}]; decide on code and reason, never on message.";

const _: () = assert!(INSTRUCTIONS.len() < 400);

#[derive(Clone)]
pub(crate) struct ScryerMcpServer {
    auth: AuthState,
    diagnostics: Arc<DiagnosticsHost>,
}

impl ScryerMcpServer {
    pub(super) fn new(auth: AuthState, diagnostics: Arc<DiagnosticsHost>) -> Self {
        Self { auth, diagnostics }
    }
}

impl ServerHandler for ScryerMcpServer {
    fn get_info(&self) -> ServerConfig {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("scryer", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(
            tools::TOOLS
                .iter()
                .map(|entry| entry.tool.clone())
                .collect(),
        ))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools::find(name).map(|entry| entry.tool.clone())
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // The name the client sent is never echoed back.
        let Some(entry) = tools::find(&request.name) else {
            return Err(ErrorData::invalid_params(
                "unknown tool",
                Some(json!({"reason": HTTP_MCP_TOOL_UNKNOWN})),
            ));
        };
        let arguments = tools::Arguments::new(request.arguments, entry.accepts)?;
        // The route admits the caller and stores it on the HTTP request; rmcp
        // hands the request parts to the handler unchanged.
        let parts = context.extensions.get::<axum::http::request::Parts>();
        let Some(caller) = parts
            .and_then(|parts| parts.extensions.get::<ApiKeyCaller>())
            .cloned()
        else {
            return Err(ErrorData::internal_error(
                "the MCP request carries no admitted caller",
                None,
            ));
        };
        #[cfg(test)]
        let probe = parts
            .and_then(|parts| parts.extensions.get::<ExecutionProbe>())
            .cloned();
        #[cfg(test)]
        let result_max_bytes = parts
            .and_then(|parts| parts.extensions.get::<ResultCap>())
            .map_or(tools::RESULT_MAX_BYTES, |cap| cap.0);
        #[cfg(not(test))]
        let result_max_bytes = tools::RESULT_MAX_BYTES;
        let tool_context = ToolContext {
            auth: self.auth.clone(),
            caller,
            diagnostics: self.diagnostics.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
            result_max_bytes,
        };
        // One call is one charge, whatever the tool runs inside it.
        if let Some(refused) = tools::charge_call(&tool_context, entry, &arguments) {
            return Ok(refused.into());
        }
        // Blocking work cannot be dropped like a future; it polls this flag.
        let cancelled = tool_context.cancelled.clone();
        let _raise_on_drop = RaiseOnDrop(cancelled);
        let execution = async move {
            #[cfg(test)]
            if let Some(probe) = probe {
                probe.hold().await;
            }
            (entry.run)(tool_context, arguments).await
        };
        // A client that goes away cancels the request. The tool's future is
        // dropped at its next await, which ends its GraphQL and database
        // reads. Work already handed to `spawn_blocking` cannot be dropped,
        // so it checks the call's cancellation flag: the log search stops
        // between lines and between log files. These run to completion and
        // their results are discarded: one log file's read already under
        // way, the section tool's host facts and metrics render, which are
        // short, bounded reads, and the one-time schema index build. The
        // restore lease is held until rmcp drops the request, which waits
        // only for the handler, not for blocking work still finishing.
        tokio::select! {
            result = execution => result.map(Into::into),
            () = context.ct.cancelled() => Err(ErrorData::internal_error(
                "the request was cancelled",
                None,
            )),
        }
    }
}

/// Raises the call's cancellation flag when the call ends, however it ends,
/// so blocking work still running for it stops.
struct RaiseOnDrop(Arc<AtomicBool>);

impl Drop for RaiseOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// A test hook that holds a tool call at its start until the call is
/// dropped, recording when that happens.
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct ExecutionProbe {
    pub entered: Arc<tokio::sync::Notify>,
    pub dropped: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
impl ExecutionProbe {
    async fn hold(&self) {
        struct Flag(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Flag {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _flag = Flag(self.dropped.clone());
        self.entered.notify_one();
        std::future::pending::<()>().await;
    }
}

/// A test hook that sets the run tools' result cap for one request.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct ResultCap(pub usize);

/// A tool result carrying `text`, serialized JSON, as its one text block.
/// No tool declares an output schema, so no structured copy is sent: every
/// client reads the same block, and it travels once.
pub(crate) fn text_result(text: String, is_error: bool) -> CallToolResult {
    let content = vec![ContentBlock::text(text)];
    if is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

/// A successful tool result of `value`, as compact JSON text.
pub(crate) fn json_result(value: Value) -> CallToolResult {
    text_result(value.to_string(), false)
}

/// A failed tool result of `value`, carried the same way as [`json_result`].
pub(crate) fn json_error(value: Value) -> CallToolResult {
    text_result(value.to_string(), true)
}

/// A GraphQL response as a tool result; see [`graphql_value`].
pub(crate) fn tool_result(response: GraphQLResponse) -> CallToolResult {
    let (value, is_error) = graphql_value(response);
    text_result(value.to_string(), is_error)
}

/// A GraphQL response as a tool result's value and whether it failed: the
/// data on success, and on failure every error's code, reason, path and
/// other extension facts from its extensions, never parsed out of its
/// message, beside whatever data the operation still returned.
pub(crate) fn graphql_value(response: GraphQLResponse) -> (Value, bool) {
    let data = serde_json::to_value(&response.data).unwrap_or(Value::Null);
    if response.errors.is_empty() {
        return match data {
            Value::Object(_) => (data, false),
            _ => (json!({}), false),
        };
    }
    let errors = response
        .errors
        .into_iter()
        .map(|error| {
            let mut entry = Map::new();
            let mut facts = error
                .extensions
                .as_ref()
                .and_then(|extensions| serde_json::to_value(extensions).ok())
                .and_then(|value| match value {
                    Value::Object(map) => Some(map),
                    _ => None,
                })
                .unwrap_or_default();
            let code = facts
                .remove("code")
                .filter(Value::is_string)
                .unwrap_or_else(|| json!(HTTP_MCP_GRAPHQL_ERROR));
            entry.insert("code".into(), code);
            if let Some(reason) = facts.remove("reason").filter(Value::is_string) {
                entry.insert("reason".into(), reason);
            }
            entry.insert("message".into(), Value::String(error.message));
            if !error.path.is_empty() {
                entry.insert(
                    "path".into(),
                    serde_json::to_value(&error.path).unwrap_or(Value::Null),
                );
            }
            for (key, value) in facts {
                entry.entry(key).or_insert(value);
            }
            Value::Object(entry)
        })
        .collect::<Vec<_>>();
    let mut content = Map::new();
    content.insert("errors".into(), Value::Array(errors));
    // A partial result keeps the fields that resolved.
    if matches!(&data, Value::Object(fields) if fields.values().any(|value| !value.is_null())) {
        content.insert("data".into(), data);
    }
    (Value::Object(content), true)
}
