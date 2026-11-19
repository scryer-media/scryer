use super::Effect;
use super::{
    Args, Kind, Param, ToolContext, ToolFuture, ToolSpec, field, fit_budget, graphql, list_result,
};
use serde_json::{Map, Value, json};

/// Why releases for one title were or were not grabbed.
pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-troubleshoot/operations/TitleDecisions.graphql");

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "get_title_decisions",
    title: "Title decisions",
    description: "Explain why releases for one title were or were not grabbed: decision codes \
counted over its newest 25 decisions, wanted and pending-release counts by status, and those \
decisions with their scores. Branch on decisionCode.",
    params: &[Param {
        name: "title_id",
        description: "The title, from find_titles.",
        required: true,
        kind: Kind::Str { max_chars: 100 },
    }],
    effect: Effect::Read,
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let data = match graphql(
            &context,
            OPERATION,
            json!({"titleId": args.value("title_id")}),
        )
        .await
        {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let diagnostics = field(&data, "titleAcquisitionDiagnostics");
        let mut head = Map::new();
        for name in [
            "latestDecisionAt",
            "latestWantedSearchAt",
            "mismatchRecoveryEligibleCount",
            "decisionCounts",
            "wantedStatusCounts",
            "pendingReleaseCounts",
        ] {
            head.insert(name.into(), field(diagnostics, name).clone());
        }
        head.insert("tally_covers".into(), json!("newest_25"));
        let items = fit_budget(
            field(diagnostics, "recentDecisions")
                .as_array()
                .cloned()
                .unwrap_or_default(),
        );
        Ok(list_result(head, items, "next_offset", Value::Null))
    })
}
