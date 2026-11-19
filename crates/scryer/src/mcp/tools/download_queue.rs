use super::Effect;
use super::{
    Args, Kind, Param, ToolContext, ToolFuture, ToolSpec, app_error, fit_budget, free_text,
    graphql_name, list_result, sorted_counts,
};
use scryer_interface::mappers::from_download_queue_item;
use scryer_interface::types::{DownloadDisplayStateValue, FromApplication, ImportErrorCodeValue};
use serde_json::{Map, Value, json};

const MAX_ITEMS: i64 = 50;
/// Tracked status messages kept per item.
const MAX_STATUS_MESSAGES: usize = 3;

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "list_download_queue",
    title: "Download queue",
    description: "Count the download queue by display state, attention and import error code, \
and return one page of it in status order with each item's client, states, progress, attention \
and import error. Read from the last client refresh; nothing contacts a download client.",
    params: &[
        Param {
            name: "title_id",
            description: "Only this title's downloads.",
            required: false,
            kind: Kind::Str { max_chars: 100 },
        },
        Param {
            name: "attention_only",
            description: "Page only the items that need attention. The counts cover every item.",
            required: false,
            kind: Kind::Bool {
                default: Some(false),
            },
        },
        Param {
            name: "limit",
            description: "Items to return.",
            required: false,
            kind: Kind::Int {
                default: Some(20),
                min: 1,
                max: MAX_ITEMS,
            },
        },
        Param {
            name: "offset",
            description: "Items to skip, from next_offset.",
            required: false,
            kind: Kind::Int {
                default: Some(0),
                min: 0,
                max: 100_000,
            },
        },
    ],
    effect: Effect::Read,
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let offset = args.count("offset");
        let diagnosis = match context
            .auth
            .app
            .diagnose_download_queue(
                context.caller.user(),
                args.str("title_id"),
                args.bool("attention_only"),
                args.count("limit"),
                offset,
            )
            .await
        {
            Ok(diagnosis) => diagnosis,
            Err(error) => return Ok(app_error(error)),
        };

        let mut head = Map::new();
        head.insert("total_count".into(), json!(diagnosis.total_count));
        head.insert(
            "by_display_state".into(),
            sorted_counts(diagnosis.by_display_state.iter().map(|(state, count)| {
                (
                    enum_name(&DownloadDisplayStateValue::from_application(*state)),
                    *count as u64,
                )
            })),
        );
        head.insert("attention_count".into(), json!(diagnosis.attention_count));
        head.insert(
            "by_import_error_code".into(),
            sorted_counts(diagnosis.by_import_error_code.iter().map(|(code, count)| {
                (
                    enum_name(&ImportErrorCodeValue::from_domain(*code)),
                    *count as u64,
                )
            })),
        );
        head.insert("matching_count".into(), json!(diagnosis.matching_count));
        head.insert("ready".into(), json!(diagnosis.ready));
        head.insert("stale".into(), json!(diagnosis.stale));
        head.insert("updated_at".into(), json!(diagnosis.updated_at));

        let items = fit_budget(diagnosis.items.into_iter().map(project).collect());
        let next = offset + items.len();
        let next = if next < diagnosis.matching_count {
            json!(next)
        } else {
            Value::Null
        };
        Ok(list_result(head, items, "next_offset", next))
    })
}

fn enum_name<T: async_graphql::InputType>(value: &T) -> String {
    graphql_name(value)
        .as_str()
        .unwrap_or("UNKNOWN")
        .to_string()
}

/// The fields the troubleshooting skill's queue page selects, in its order.
fn project(item: scryer_domain::DownloadQueueItem) -> Value {
    let item = from_download_queue_item(item);
    let messages = item.tracked_status_messages.len();
    json!({
        "id": item.id.as_str(),
        "titleId": item.title_id.as_ref().map(|id| id.as_str()),
        "titleName": item.title_name,
        "facet": graphql_name(&item.facet),
        "clientName": item.client_name,
        "clientType": item.client_type,
        "displayState": graphql_name(&item.display_state),
        "trackedState": graphql_name(&item.tracked_state),
        "progressPercent": item.progress_percent,
        "attentionRequired": item.attention_required,
        "attentionReasonCode": item.attention_reason_code,
        "attentionReason": item.attention_reason.as_deref().map(free_text),
        "passwordFailureCode": graphql_name(&item.password_failure_code),
        "importStatus": graphql_name(&item.import_status),
        "importErrorCode": graphql_name(&item.import_error_code),
        "importErrorMessage": item.import_error_message.as_deref().map(free_text),
        "trackedStatusMessages": item
            .tracked_status_messages
            .iter()
            .take(MAX_STATUS_MESSAGES)
            .map(|message| free_text(message))
            .collect::<Vec<_>>(),
        "trackedStatusMessagesOmitted": messages.saturating_sub(MAX_STATUS_MESSAGES),
        "queuedAt": item.queued_at,
        "lastUpdatedAt": item.last_updated_at,
    })
}
