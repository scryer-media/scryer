use super::{
    Args, Kind, Param, ToolContext, ToolFuture, ToolSpec, field, free_text_value, graphql,
    list_result, offset_page, pick, tally,
};
use serde_json::{Map, Value, json};

/// Download client state without config values or secrets.
pub(super) const OPERATION: &str = include_str!(
    "../../../../../skills/scryer-troubleshoot/operations/DownloadClientStatus.graphql"
);

const MAX_ITEMS: i64 = 50;

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "list_download_clients",
    title: "Download clients",
    description: "List download clients with their type, whether they are enabled, their \
status (healthy, error or failed), last error and when they were last seen; unhealthy counts \
enabled clients that are not healthy, without any configuration values. Nothing \
contacts the clients.",
    params: &[
        Param {
            name: "limit",
            description: "Clients to return.",
            required: false,
            kind: Kind::Int {
                default: Some(MAX_ITEMS),
                min: 1,
                max: MAX_ITEMS,
            },
        },
        Param {
            name: "offset",
            description: "Clients to skip, from next_offset.",
            required: false,
            kind: Kind::Int {
                default: Some(0),
                min: 0,
                max: 10_000,
            },
        },
    ],
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let data = match graphql(&context, OPERATION, Value::Null).await {
            Ok(data) => data,
            Err(error) => return Ok(error),
        };
        let clients = field(&data, "downloadClientConfigs")
            .as_array()
            .cloned()
            .unwrap_or_default();
        let unhealthy = clients
            .iter()
            .filter(|client| {
                field(client, "isEnabled").as_bool() == Some(true)
                    && field(client, "status").as_str()
                        != Some(scryer_domain::DownloadClientStatus::Healthy.as_str())
            })
            .count();
        let mut head = Map::new();
        head.insert("total".into(), json!(clients.len()));
        head.insert("unhealthy".into(), json!(unhealthy));
        head.insert(
            "by_status".into(),
            tally(clients.iter().map(|client| {
                field(client, "status")
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string()
            })),
        );
        let total = clients.len();
        let rows = clients.iter().map(|client| {
            let mut row = pick(
                client,
                &[
                    "id",
                    "name",
                    "clientType",
                    "isEnabled",
                    "status",
                    "lastError",
                    "lastSeenAt",
                    "disabledUntil",
                    "escalationLevel",
                    "proxyConfigId",
                ],
            );
            row.insert(
                "lastError".into(),
                free_text_value(field(client, "lastError")),
            );
            Value::Object(row)
        });
        let (items, next) = offset_page(rows, total, args.count("offset"), args.count("limit"));
        Ok(list_result(head, items, "next_offset", next))
    })
}
