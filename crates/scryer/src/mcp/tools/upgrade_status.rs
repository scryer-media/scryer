use super::Effect;
use super::{Args, ToolContext, ToolFuture, ToolSpec, free_text_value, graphql};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};

pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-troubleshoot/operations/UpgradeStatus.graphql");

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "get_upgrade_status",
    title: "Upgrade status",
    description: "Show the installed version, whether an update is available, how this \
installation is upgraded and by whom, whether a self-upgrade is allowed and why not, and the \
latest upgrade run.",
    params: &[],
    effect: Effect::Read,
    run,
};

fn run(context: ToolContext, _args: Args) -> ToolFuture {
    Box::pin(async move {
        let mut data = match graphql(&context, OPERATION, Value::Null).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        if let Some(run) = data
            .pointer_mut("/applicationUpgradeStatus/latestRun")
            .filter(|run| run.is_object())
        {
            run["errorText"] = free_text_value(&run["errorText"]);
        }
        let status = data
            .get_mut("applicationUpgradeStatus")
            .map(Value::take)
            .unwrap_or(Value::Null);
        Ok(CallToolResult::structured(json!({ "upgrade": status })))
    })
}
