//! The MCP tool surface. Each tool runs one operation file from the operator
//! skills, so the tool and the documented GraphQL procedure cannot drift.

use std::sync::Arc;

use async_graphql::Response as GraphQLResponse;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, InitializeResult,
    JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};

use crate::middleware::{ApiKeyCaller, AuthState, execute_graphql_as_api_key_caller};

/// The operation behind `whoami`, shared with the scryer-connect skill.
const ME_OPERATION: &str = include_str!("../../../../skills/scryer-connect/operations/Me.graphql");

const WHOAMI: &str = "whoami";

const INSTRUCTIONS: &str = "Scryer tools act as the account that owns the API key, with exactly \
that account's permissions and library access. Results are bounded: list results return a \
limited page. A failed tool call has isError set and structured content {\"errors\": [...]}; each \
error carries a stable code, an optional reason and a message. Decide on code and reason, never \
on the message text.";

const _: () = assert!(INSTRUCTIONS.len() < 600);

/// A GraphQL error without a code of its own, such as a parse or validation
/// failure.
const GRAPHQL_ERROR_FALLBACK_CODE: &str = "GRAPHQL_ERROR";

#[derive(Clone)]
pub(crate) struct ScryerMcpServer {
    auth: AuthState,
}

impl ScryerMcpServer {
    pub(super) fn new(auth: AuthState) -> Self {
        Self { auth }
    }
}

fn whoami_tool() -> Tool {
    let mut input_schema = JsonObject::new();
    input_schema.insert("type".into(), json!("object"));
    input_schema.insert("additionalProperties".into(), json!(false));
    Tool::new(
        WHOAMI,
        "Show the Scryer account this API key acts as: its username, account kind, app \
         permissions and whether it has MFA.",
        Arc::new(input_schema),
    )
    .with_title("Who am I")
    .with_annotations(
        ToolAnnotations::with_title("Who am I")
            .read_only(true)
            .open_world(false),
    )
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
        Ok(ListToolsResult::with_all_items(vec![whoami_tool()]))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        (name == WHOAMI).then(whoami_tool)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if request.name != WHOAMI {
            return Err(ErrorData::invalid_params(
                format!("unknown tool: {}", request.name),
                None,
            ));
        }
        if request
            .arguments
            .as_ref()
            .is_some_and(|arguments| !arguments.is_empty())
        {
            return Err(ErrorData::invalid_params("whoami takes no arguments", None));
        }
        // The route admits the caller and stores it on the HTTP request; rmcp
        // hands the request parts to the handler unchanged.
        let Some(caller) = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<ApiKeyCaller>())
            .cloned()
        else {
            return Err(ErrorData::internal_error(
                "the MCP request carries no admitted caller",
                None,
            ));
        };
        let response = execute_graphql_as_api_key_caller(
            &self.auth,
            &caller,
            async_graphql::Request::new(ME_OPERATION),
        )
        .await;
        Ok(tool_result(response).into())
    }
}

/// A GraphQL response as a tool result: the data on success, and on failure
/// every error's code, reason and other extension facts from its extensions,
/// never parsed out of its message.
pub(super) fn tool_result(response: GraphQLResponse) -> CallToolResult {
    if response.errors.is_empty() {
        let data = serde_json::to_value(&response.data).unwrap_or(Value::Null);
        return match data {
            Value::Object(_) => CallToolResult::structured(data),
            _ => CallToolResult::structured(json!({})),
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
                .unwrap_or_else(|| json!(GRAPHQL_ERROR_FALLBACK_CODE));
            entry.insert("code".into(), code);
            if let Some(reason) = facts.remove("reason").filter(Value::is_string) {
                entry.insert("reason".into(), reason);
            }
            entry.insert("message".into(), Value::String(error.message));
            for (key, value) in facts {
                entry.entry(key).or_insert(value);
            }
            Value::Object(entry)
        })
        .collect::<Vec<_>>();
    CallToolResult::structured_error(json!({ "errors": errors }))
}
