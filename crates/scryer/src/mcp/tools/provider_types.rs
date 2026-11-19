use super::{
    Args, Effect, OFFSET_PARAM, ToolContext, ToolFuture, ToolSpec, field, graphql, limit_param,
    list_result, offset_page,
};
use serde_json::{Map, Value, json};

/// Installed indexer providers and their field declarations, without
/// default values, help text, options or base URLs.
pub(super) const INDEXER_OPERATION: &str =
    include_str!("../operations/IndexerProviderTypeFields.graphql");

/// Installed download-client providers and their field declarations,
/// without default values, help text, options or base URLs.
pub(super) const DOWNLOAD_CLIENT_OPERATION: &str =
    include_str!("../operations/DownloadClientProviderTypeFields.graphql");

pub(super) static INDEXER_TOOL: ToolSpec = ToolSpec {
    name: "list_indexer_provider_types",
    title: "Indexer provider types",
    description: "List the installed indexer providers: provider type, name, whether a \
connection test exists, recommended facets, and each declared config field's key, label, type, \
whether it is required and whether it is advanced. No default value, help text, option list or \
base URL is returned, and no config value of any indexer.",
    params: &[limit_param("Providers to return.", 50), OFFSET_PARAM],
    effect: Effect::Read,
    run: run_indexer,
};

pub(super) static DOWNLOAD_CLIENT_TOOL: ToolSpec = ToolSpec {
    name: "list_download_client_provider_types",
    title: "Download client provider types",
    description: "List the installed download-client providers: provider type, name, whether \
a connection test exists, and each declared config field's key, label, type, whether it is \
required and whether it is advanced. No default value, help text, option list or base URL is \
returned, and no config value of any client.",
    params: &[limit_param("Providers to return.", 50), OFFSET_PARAM],
    effect: Effect::Read,
    run: run_download_client,
};

async fn providers(
    context: &ToolContext,
    args: &Args,
    document: &'static str,
    path: &str,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    let data = match graphql(context, document, Value::Null).await {
        Ok(data) => data,
        Err(error) => return Ok(error),
    };
    let rows = field(&data, path).as_array().cloned().unwrap_or_default();
    let mut head = Map::new();
    head.insert("total".into(), json!(rows.len()));
    let total = rows.len();
    let (items, next) = offset_page(rows, total, args.count("offset"), args.count("limit"));
    Ok(list_result(head, items, "next_offset", next))
}

fn run_indexer(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(
        async move { providers(&context, &args, INDEXER_OPERATION, "indexerProviderTypes").await },
    )
}

fn run_download_client(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        providers(
            &context,
            &args,
            DOWNLOAD_CLIENT_OPERATION,
            "downloadClientProviderTypes",
        )
        .await
    })
}
