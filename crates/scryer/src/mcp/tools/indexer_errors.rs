use super::Effect;
use std::collections::BTreeMap;

use super::{
    Args, Kind, Param, ToolContext, ToolFuture, ToolSpec, field, fit_budget, free_text_value,
    graphql, list_result, pick,
};
use serde_json::{Map, Value, json};

/// Recent indexer failures, newest first.
pub(super) const OPERATION: &str =
    include_str!("../../../../../skills/scryer-troubleshoot/operations/IndexerErrors.graphql");

const MAX_ITEMS: i64 = 100;

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "list_indexer_errors",
    title: "Indexer errors",
    description: "List recent indexer failures newest first, grouped by classification with the \
operations each was seen on, then the failures themselves with HTTP status, provider error code \
and message. Page with after = next_cursor.",
    params: &[
        Param {
            name: "indexer_id",
            description: "Only this indexer's failures.",
            required: false,
            kind: Kind::Str { max_chars: 100 },
        },
        Param {
            name: "first",
            description: "Failures to read.",
            required: false,
            kind: Kind::Int {
                default: Some(25),
                min: 1,
                max: MAX_ITEMS,
            },
        },
        Param {
            name: "after",
            description: "The next_cursor of an earlier page.",
            required: false,
            kind: Kind::Str { max_chars: 512 },
        },
    ],
    effect: Effect::Read,
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let mut first = args.count("first");
        loop {
            let variables = json!({
                "indexerId": args.value("indexer_id"),
                "first": first,
                "after": args.value("after"),
            });
            let data = match graphql(&context, OPERATION, variables).await {
                Ok(data) => data,
                Err(error) => return Ok(error),
            };
            let page = field(&data, "indexerErrors");
            let errors = field(page, "items").as_array().cloned().unwrap_or_default();
            let rows = errors
                .iter()
                .map(|error| {
                    let mut row = pick(
                        error,
                        &[
                            "indexerId",
                            "indexerName",
                            "operation",
                            "occurredAt",
                            "httpStatus",
                            "classification",
                            "providerErrorCode",
                            "message",
                        ],
                    );
                    row.insert("message".into(), free_text_value(field(error, "message")));
                    Value::Object(row)
                })
                .collect::<Vec<_>>();
            let read = rows.len();
            let items = fit_budget(rows);
            if items.len() < read {
                // The page is larger than a result may be; read exactly what
                // fits so the cursor resumes after the last item returned.
                first = items.len();
                continue;
            }
            let mut groups: BTreeMap<String, (u64, Vec<String>)> = BTreeMap::new();
            for error in &errors {
                let classification = field(error, "classification")
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let entry = groups.entry(classification).or_default();
                entry.0 += 1;
                if let Some(operation) = field(error, "operation").as_str()
                    && !entry.1.iter().any(|seen| seen == operation)
                {
                    entry.1.push(operation.to_string());
                }
            }
            let mut groups = groups.into_iter().collect::<Vec<_>>();
            groups
                .sort_by(|left, right| right.1.0.cmp(&left.1.0).then_with(|| left.0.cmp(&right.0)));
            let mut head = Map::new();
            head.insert(
                "by_classification".into(),
                Value::Array(
                    groups
                        .into_iter()
                        .map(|(classification, (count, mut operations))| {
                            operations.sort();
                            json!({
                                "classification": classification,
                                "count": count,
                                "operations": operations,
                            })
                        })
                        .collect(),
                ),
            );
            return Ok(list_result(
                head,
                items,
                "next_cursor",
                field(page, "nextCursor").clone(),
            ));
        }
    })
}
