use super::{Arguments, ToolContext, ToolEntry, ToolFuture, api_class, execute, local_read, tool};
use crate::mcp::catalog::CATALOG;
use crate::mcp::server::tool_result;
use serde_json::json;

/// The catalog operation this tool runs, shared with the scryer-connect
/// skill.
const OPERATION: &str = "Me";

pub(super) fn entry() -> ToolEntry {
    ToolEntry {
        tool: tool(
            "whoami",
            "The account this API key acts as, with its permissions.",
            json!({"type": "object", "properties": {}}),
            local_read(),
        ),
        accepts: &[],
        run,
        class: api_class,
    }
}

fn run(context: ToolContext, _arguments: Arguments) -> ToolFuture {
    Box::pin(async move {
        let operation = CATALOG
            .get(OPERATION)
            .expect("the catalog serves one Me operation");
        let request = async_graphql::Request::new(operation.text).operation_name(OPERATION);
        let response = execute(&context, request).await;
        Ok(tool_result(response))
    })
}
