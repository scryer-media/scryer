use super::Effect;
use super::{Args, ToolContext, ToolFuture, ToolSpec, graphql};
use rmcp::model::CallToolResult;

/// Shared with the scryer-connect skill.
pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-connect/operations/Me.graphql");

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "whoami",
    title: "Who am I",
    description: "Show the Scryer account this API key acts as: its username, account kind, app \
permissions and whether it has MFA.",
    params: &[],
    effect: Effect::Read,
    run,
};

fn run(context: ToolContext, _args: Args) -> ToolFuture {
    Box::pin(async move {
        Ok(
            match graphql(&context, OPERATION, serde_json::Value::Null).await {
                Ok(data) => CallToolResult::structured(data),
                Err(error) => error,
            },
        )
    })
}
