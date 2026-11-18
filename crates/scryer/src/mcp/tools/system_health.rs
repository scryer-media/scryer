use super::{Args, ToolContext, ToolFuture, ToolSpec, graphql};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};

/// Readiness and title counts without the per-indexer statistics, which
/// `list_indexers` reports.
pub(super) const OPERATION: &str = include_str!("../operations/SystemReadiness.graphql");

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "get_system_health",
    title: "System health",
    description: "Show whether the service is ready, the datastore engine and its migration key, \
and title counts by kind. Indexer statistics are in list_indexers.",
    params: &[],
    run,
};

fn run(context: ToolContext, _args: Args) -> ToolFuture {
    Box::pin(async move {
        let mut data = match graphql(&context, OPERATION, Value::Null).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let health = data
            .get_mut("systemHealth")
            .map(Value::take)
            .unwrap_or(Value::Null);
        Ok(CallToolResult::structured(json!({ "health": health })))
    })
}
