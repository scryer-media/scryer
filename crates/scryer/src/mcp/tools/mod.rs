//! The tool table. `list_operations` and `describe_operation` read the
//! operation catalog in [`super::catalog`]; `run_query` and `run_mutation`
//! run one of its operations through GraphQL as the key's account, so
//! permissions, MFA step-up and the audit trail match `/graphql`. The
//! diagnostics tools read the bundle's own sections and logs.
//!
//! Each tool checks its own arguments; a key the tool does not take is
//! refused without being echoed.

mod describe_operation;
mod diagnostics_section;
mod list_operations;
mod run_operation;
pub(super) mod service_logs;
mod whoami;

use std::sync::{Arc, LazyLock};

use futures_util::future::BoxFuture;
use rmcp::ErrorData;
use rmcp::model::{CallToolResult, JsonObject, Tool, ToolAnnotations};
use scryer_application::AppError;
use serde_json::{Map, Value, json};

use super::server::{json_error, tool_result};
use crate::diagnostics_routes::DiagnosticsHost;
use crate::error_reasons::{
    HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED, HTTP_DIAGNOSTICS_PERMISSION_DENIED,
    HTTP_MCP_ARGUMENTS_INVALID, HTTP_MCP_DIAGNOSTICS_FAILED,
};
use crate::middleware::{
    ApiKeyCaller, AuthState, charge_api_key_call, execute_graphql_as_api_key_caller,
};
use crate::rate_limit::GraphqlRateLimitClass;

/// Serialized bytes a `run_query` or `run_mutation` result may hold.
pub(crate) const RESULT_MAX_BYTES: usize = 32 * 1024;

/// What a tool runs with: the route's auth state, the admitted caller and the
/// binary's diagnostics facts.
#[derive(Clone)]
pub(crate) struct ToolContext {
    pub auth: AuthState,
    pub caller: ApiKeyCaller,
    pub diagnostics: Arc<DiagnosticsHost>,
    /// Raised when the call ends or its client goes away; blocking work for
    /// the call checks it and stops.
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
    /// The cap on a run tool's serialized result, [`RESULT_MAX_BYTES`] outside
    /// tests.
    pub result_max_bytes: usize,
    /// Pending mutation confirmations, shared by every request.
    pub confirmations: Arc<super::confirm::Confirmations>,
}

pub(super) type ToolFuture = BoxFuture<'static, Result<CallToolResult, ErrorData>>;

pub(crate) struct ToolEntry {
    pub tool: Tool,
    /// The argument names the tool takes; any other is refused.
    pub accepts: &'static [&'static str],
    pub run: fn(ToolContext, Arguments) -> ToolFuture,
    /// The rate-limit class one call is charged to, read from its arguments
    /// before it runs.
    pub class: fn(&Arguments) -> GraphqlRateLimitClass,
}

/// The class of a tool that is not a catalog run: one API request.
pub(super) fn api_class(_: &Arguments) -> GraphqlRateLimitClass {
    GraphqlRateLimitClass::Api
}

/// Every tool, in the order `tools/list` returns them.
pub(crate) static TOOLS: LazyLock<Vec<ToolEntry>> = LazyLock::new(|| {
    vec![
        whoami::entry(),
        list_operations::entry(),
        describe_operation::entry(),
        run_operation::query_entry(),
        run_operation::mutation_entry(),
        diagnostics_section::entry(),
        service_logs::entry(),
    ]
});

pub(crate) fn find(name: &str) -> Option<&'static ToolEntry> {
    TOOLS.iter().find(|entry| entry.tool.name == name)
}

/// A tool with its input schema; no title, so `tools/list` stays small.
pub(super) fn tool(
    name: &'static str,
    description: &'static str,
    schema: Value,
    annotations: ToolAnnotations,
) -> Tool {
    let Value::Object(schema) = schema else {
        unreachable!("a tool's input schema is an object");
    };
    Tool::new(name, description, Arc::new(schema)).with_annotations(annotations)
}

/// Annotations of a tool that reads only this instance.
pub(super) fn local_read() -> ToolAnnotations {
    ToolAnnotations::new().read_only(true).open_world(false)
}

/// A tool call's arguments, checked against the names the tool takes.
#[derive(Debug, Default)]
pub(crate) struct Arguments(Map<String, Value>);

impl Arguments {
    pub(crate) fn new(arguments: Option<JsonObject>, accepts: &[&str]) -> Result<Self, ErrorData> {
        let mut arguments = arguments.unwrap_or_default();
        // A null argument is an absent one.
        arguments.retain(|_, value| !value.is_null());
        if arguments.keys().any(|key| !accepts.contains(&key.as_str())) {
            return Err(ErrorData::invalid_params(
                "an argument is not one this tool takes",
                Some(json!({"reason": HTTP_MCP_ARGUMENTS_INVALID})),
            ));
        }
        Ok(Self(arguments))
    }

    pub(super) fn str(
        &self,
        name: &'static str,
        max_chars: usize,
    ) -> Result<Option<&str>, ErrorData> {
        match self.0.get(name) {
            None => Ok(None),
            Some(Value::String(text)) if text.chars().count() <= max_chars => Ok(Some(text)),
            Some(Value::String(_)) => Err(invalid_argument(name, "is too long")),
            Some(_) => Err(invalid_argument(name, "must be a string")),
        }
    }

    pub(super) fn required_str(
        &self,
        name: &'static str,
        max_chars: usize,
    ) -> Result<&str, ErrorData> {
        self.str(name, max_chars)?
            .ok_or_else(|| invalid_argument(name, "is required"))
    }

    pub(super) fn bool(&self, name: &'static str) -> Result<Option<bool>, ErrorData> {
        match self.0.get(name) {
            None => Ok(None),
            Some(Value::Bool(value)) => Ok(Some(*value)),
            Some(_) => Err(invalid_argument(name, "must be a boolean")),
        }
    }

    pub(super) fn int(
        &self,
        name: &'static str,
        min: i64,
        max: i64,
    ) -> Result<Option<i64>, ErrorData> {
        match self.0.get(name) {
            None => Ok(None),
            Some(value) => match value.as_i64() {
                Some(number) if (min..=max).contains(&number) => Ok(Some(number)),
                _ => Err(invalid_argument(name, "must be an integer in range")),
            },
        }
    }

    /// A list of distinct strings.
    pub(super) fn str_list(
        &self,
        name: &'static str,
        max_items: usize,
        max_chars: usize,
    ) -> Result<Option<Vec<String>>, ErrorData> {
        let Some(value) = self.0.get(name) else {
            return Ok(None);
        };
        let invalid = || invalid_argument(name, "must be a short list of distinct strings");
        let items = value.as_array().ok_or_else(invalid)?;
        if items.is_empty() || items.len() > max_items {
            return Err(invalid());
        }
        let mut strings = Vec::with_capacity(items.len());
        for item in items {
            match item.as_str() {
                Some(text)
                    if text.chars().count() <= max_chars
                        && !strings.iter().any(|seen| seen == text) =>
                {
                    strings.push(text.to_string());
                }
                _ => return Err(invalid()),
            }
        }
        Ok(Some(strings))
    }

    /// The named argument, taken out unchanged.
    pub(super) fn take(&mut self, name: &str) -> Option<Value> {
        self.0.remove(name)
    }
}

/// A refused argument. The message names the declared argument, never the
/// value sent.
pub(super) fn invalid_argument(argument: &'static str, problem: &str) -> ErrorData {
    ErrorData::invalid_params(
        format!("argument `{argument}` {problem}"),
        Some(json!({"reason": HTTP_MCP_ARGUMENTS_INVALID, "argument": argument})),
    )
}

/// Runs one GraphQL request as the caller.
pub(super) async fn execute(
    context: &ToolContext,
    request: async_graphql::Request,
) -> async_graphql::Response {
    execute_graphql_as_api_key_caller(&context.auth, &context.caller, request).await
}

/// A tool error built here: a code, a reason, a short message and any facts
/// in their own fields.
pub(super) fn coded_error(code: &str, reason: &str, message: &str) -> CallToolResult {
    coded_error_with(code, reason, message, Map::new())
}

pub(super) fn coded_error_with(
    code: &str,
    reason: &str,
    message: &str,
    facts: Map<String, Value>,
) -> CallToolResult {
    json_error(json!({"errors": [error_entry(code, reason, message, facts)]}))
}

/// One entry of a tool error's `errors` list.
pub(super) fn error_entry(
    code: &str,
    reason: &str,
    message: &str,
    facts: Map<String, Value>,
) -> Value {
    let mut error = Map::new();
    error.insert("code".into(), json!(code));
    error.insert("reason".into(), json!(reason));
    error.insert("message".into(), json!(message));
    error.extend(facts);
    Value::Object(error)
}

/// Charges one tool call to the GraphQL rate limits in the tool's class for
/// these arguments. `call_tool` runs this once before the tool; nothing
/// inside a tool charges again.
pub(crate) fn charge_call(
    context: &ToolContext,
    entry: &ToolEntry,
    arguments: &Arguments,
) -> Option<CallToolResult> {
    charge_api_key_call(&context.auth, &context.caller, (entry.class)(arguments))
        .err()
        .map(tool_result)
}

/// The diagnostics tools' admission, the same as the bundle route's: a full
/// session and the system-settings permission, settled before anything is
/// read.
pub(super) async fn admit_diagnostics(context: &ToolContext) -> Option<CallToolResult> {
    if !context.caller.full_session() {
        return Some(coded_error(
            "UNAUTHORIZED",
            HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED,
            "Diagnostics need a full session.",
        ));
    }
    match context
        .auth
        .app
        .require_app_permission(
            context.caller.user(),
            scryer_domain::AppPermission::ManageSystemSettings,
        )
        .await
    {
        Ok(()) => None,
        Err(error) => Some(diagnostics_error(error)),
    }
}

/// A diagnostics read error: a permission refusal carries the bundle route's
/// reason, anything else the diagnostics failure reason.
pub(super) fn diagnostics_error(error: AppError) -> CallToolResult {
    match error {
        AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. } => coded_error(
            "UNAUTHORIZED",
            HTTP_DIAGNOSTICS_PERMISSION_DENIED,
            "Diagnostics require the system settings permission.",
        ),
        error => {
            tracing::warn!(%error, "MCP diagnostics read failed");
            coded_error(
                "INTERNAL_SERVER_ERROR",
                HTTP_MCP_DIAGNOSTICS_FAILED,
                "The diagnostics could not be read.",
            )
        }
    }
}

/// Drops the null, empty-array and empty-object fields of a tool's own
/// result envelope. Values it carries from elsewhere, such as a diagnostics
/// section, are left as they are.
pub(super) fn omit_empty(mut fields: Map<String, Value>) -> Map<String, Value> {
    fields.retain(|_, field| match field {
        Value::Null => false,
        Value::Array(items) => !items.is_empty(),
        Value::Object(inner) => !inner.is_empty(),
        _ => true,
    });
    fields
}
