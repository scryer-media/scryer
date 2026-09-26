//! Lists operations: the public chart catalog, chart items, proxied public
//! IMDb lists, and `resolveTitles` for any kind with alias sources.
//!
//! Chart reads are public and cacheable, so they go out as signed APQ GETs.
//! `listImdbUserList` is served only to an enrolled instance and must not be
//! cached at the edge, so it goes out as an authenticated APQ POST.

use std::sync::LazyLock;

use scryer_application::lists::gateway::{ListChartCatalogEntry, ListChartItem};
use scryer_application::{AppResult, TitleExternalRef, TitleResolution};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    METADATA_GATEWAY_MAX_TITLE_BULK_BATCH, MetadataExternalIdItem, MetadataGatewayClient,
    OP_RESOLVE_TITLES, ResolveTitlesResponse, apq_hash, external_ids_from_gateway, graphql_docs,
};

pub(super) const OP_LIST_CHART_CATALOG: &str = "ListChartCatalog";
pub(super) const OP_LIST_CHART_ITEMS: &str = "ListChartItems";
pub(super) const OP_LIST_IMDB_USER_LIST: &str = "ListImdbUserList";

static LIST_CHART_CATALOG_HASH: LazyLock<String> =
    LazyLock::new(|| apq_hash(graphql_docs::LIST_CHART_CATALOG_QUERY));
static LIST_CHART_ITEMS_HASH: LazyLock<String> =
    LazyLock::new(|| apq_hash(graphql_docs::LIST_CHART_ITEMS_QUERY));
static LIST_IMDB_USER_LIST_HASH: LazyLock<String> =
    LazyLock::new(|| apq_hash(graphql_docs::LIST_IMDB_USER_LIST_QUERY));

#[derive(Deserialize)]
struct ListChartCatalogResponse {
    #[serde(rename = "listChartCatalog")]
    list_chart_catalog: Vec<ListChartCatalogItem>,
}

#[derive(Deserialize)]
struct ListChartCatalogItem {
    provider: String,
    chart_key: String,
    scope: String,
    label: String,
    #[serde(default)]
    kinds: Vec<String>,
    refreshed_at: Option<String>,
    item_count: i64,
}

#[derive(Deserialize)]
struct ListChartItemsResponse {
    #[serde(rename = "listChartItems")]
    list_chart_items: Vec<ListChartEntry>,
}

#[derive(Deserialize)]
struct ListImdbUserListResponse {
    #[serde(rename = "listImdbUserList")]
    list_imdb_user_list: Vec<ListChartEntry>,
}

#[derive(Deserialize)]
struct ListChartEntry {
    rank: i64,
    title_id: Option<i64>,
    resolved: bool,
    kind: String,
    #[serde(default)]
    external_ids: Vec<MetadataExternalIdItem>,
    display_title: String,
    year: Option<i32>,
    poster_url: Option<String>,
}

impl From<ListChartEntry> for ListChartItem {
    fn from(entry: ListChartEntry) -> Self {
        Self {
            rank: entry.rank,
            title_id: entry.title_id,
            resolved: entry.resolved,
            kind: entry.kind,
            external_ids: external_ids_from_gateway(entry.external_ids),
            display_title: entry.display_title,
            year: entry.year,
            poster_url: entry.poster_url.filter(|url| !url.trim().is_empty()),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AliasTitleRefInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
    external_ids: Vec<AliasExternalIdInput>,
}

#[derive(Serialize)]
struct AliasExternalIdInput {
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    id: String,
}

fn alias_ref_input(reference: &TitleExternalRef) -> AliasTitleRefInput {
    AliasTitleRefInput {
        id: reference.smg_id,
        external_ids: reference
            .external_ids
            .iter()
            .filter(|id| !id.source.trim().is_empty() && !id.value.trim().is_empty())
            .map(|id| AliasExternalIdInput {
                source: id.source.trim().to_ascii_lowercase(),
                kind: id
                    .kind
                    .as_deref()
                    .map(str::trim)
                    .filter(|kind| !kind.is_empty())
                    .map(str::to_string),
                id: id.value.trim().to_string(),
            })
            .collect(),
    }
}

impl MetadataGatewayClient {
    pub(super) async fn fetch_list_chart_catalog(
        &self,
        language: &str,
    ) -> AppResult<Vec<ListChartCatalogEntry>> {
        let data: ListChartCatalogResponse = self
            .execute_graphql_apq(
                OP_LIST_CHART_CATALOG,
                graphql_docs::LIST_CHART_CATALOG_QUERY,
                &LIST_CHART_CATALOG_HASH,
                json!({ "language": language }),
            )
            .await?;
        Ok(data
            .list_chart_catalog
            .into_iter()
            .map(|item| ListChartCatalogEntry {
                provider: item.provider,
                chart_key: item.chart_key,
                scope: item.scope,
                label: item.label,
                kinds: item.kinds,
                refreshed_at: item.refreshed_at,
                item_count: item.item_count,
            })
            .collect())
    }

    pub(super) async fn fetch_list_chart_items(
        &self,
        provider: &str,
        chart_key: &str,
        scope: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<Vec<ListChartItem>> {
        let data: ListChartItemsResponse = self
            .execute_graphql_apq(
                OP_LIST_CHART_ITEMS,
                graphql_docs::LIST_CHART_ITEMS_QUERY,
                &LIST_CHART_ITEMS_HASH,
                json!({
                    "provider": provider,
                    "chartKey": chart_key,
                    "scope": scope,
                    "limit": limit,
                    "language": language,
                }),
            )
            .await?;
        Ok(data
            .list_chart_items
            .into_iter()
            .map(ListChartItem::from)
            .collect())
    }

    pub(super) async fn fetch_list_imdb_user_list(
        &self,
        list_id: &str,
    ) -> AppResult<Vec<ListChartItem>> {
        let data: ListImdbUserListResponse = self
            .execute_authenticated_graphql_apq_post(
                OP_LIST_IMDB_USER_LIST,
                graphql_docs::LIST_IMDB_USER_LIST_QUERY,
                &LIST_IMDB_USER_LIST_HASH,
                json!({ "listId": list_id }),
            )
            .await?;
        Ok(data
            .list_imdb_user_list
            .into_iter()
            .map(ListChartItem::from)
            .collect())
    }

    /// `resolveTitles` for any kind, in gateway-sized batches. Result indexes
    /// are rebased onto `refs`.
    pub(super) async fn resolve_alias_title_refs(
        &self,
        refs: &[TitleExternalRef],
        kind: &str,
        create_missing: bool,
    ) -> AppResult<Vec<TitleResolution>> {
        let mut resolutions = Vec::with_capacity(refs.len());
        for (chunk_index, chunk) in refs
            .chunks(METADATA_GATEWAY_MAX_TITLE_BULK_BATCH)
            .enumerate()
        {
            let inputs = chunk.iter().map(alias_ref_input).collect::<Vec<_>>();
            let data: ResolveTitlesResponse = self
                .execute_graphql_apq(
                    OP_RESOLVE_TITLES,
                    graphql_docs::RESOLVE_TITLES_QUERY,
                    &self.resolve_titles_hash,
                    json!({
                        "refs": inputs,
                        "kind": kind,
                        "createMissing": create_missing,
                    }),
                )
                .await?;
            let offset = chunk_index * METADATA_GATEWAY_MAX_TITLE_BULK_BATCH;
            resolutions.extend(data.resolve_titles.into_iter().map(|item| {
                let mut resolution = TitleResolution::from(item);
                resolution.ref_index = resolution.ref_index.saturating_add(offset);
                resolution
            }));
        }
        Ok(resolutions)
    }
}

#[cfg(test)]
#[path = "client_lists_tests.rs"]
mod tests;
