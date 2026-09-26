//! The metadata gateway's side of Lists: public charts, proxied IMDb lists,
//! and identity resolution for every kind.
//!
//! Charts the gateway already ingests are read from it rather than from the
//! provider, so an instance never spends its own rate limit on a public chart.
//! IMDb has no API; the gateway proxies public IMDb lists for enrolled
//! instances. Every list item, whatever id scheme its provider speaks, is
//! resolved through the gateway's `resolveTitles`, which maps alias sources
//! (Plex GUIDs, Trakt, Simkl, Kitsu, MAL, AniList) itself.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use scryer_domain::{ExternalId, MediaFacet};
use scryer_plugin_sdk::{ListExternalId, ListMediaKind, ListPluginItem};

use super::fetch::{ListChartSource, ListFailure, ListFailureClass};
use super::resolve::{ListItemResolver, ResolveInput, ResolveOutput};
use crate::{AppResult, MetadataGateway, TitleExternalRef};

/// How many chart items one read asks for. Charts are ingested to about this
/// depth, and the largest (IMDb Top 250) is exactly this long.
pub const LIST_CHART_ITEM_LIMIT: i32 = 250;

/// The language chart labels and titles are read in.
pub const LIST_CHART_LANGUAGE: &str = "eng";

/// The most refs one `resolveTitles` call carries.
pub const RESOLVE_TITLES_BATCH: usize = 50;

/// One public chart the gateway serves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListChartCatalogEntry {
    pub provider: String,
    pub chart_key: String,
    pub scope: String,
    pub label: String,
    /// `movie`, `series`, `anime`.
    pub kinds: Vec<String>,
    pub refreshed_at: Option<String>,
    pub item_count: i64,
}

/// One entry of a chart or a proxied IMDb list, in list order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListChartItem {
    pub rank: i64,
    pub title_id: Option<i64>,
    pub resolved: bool,
    /// `movie`, `series`, `anime`, or `unknown` for an unresolved IMDb entry.
    pub kind: String,
    pub external_ids: Vec<ExternalId>,
    pub display_title: String,
    pub year: Option<i32>,
    pub poster_url: Option<String>,
}

/// The gateway's kind vocabulary for a facet.
pub fn gateway_kind(kind: &MediaFacet) -> &'static str {
    match kind {
        MediaFacet::Movie => "movie",
        MediaFacet::Series => "series",
        MediaFacet::Anime => "anime",
    }
}

fn list_kind(kind: &str) -> Option<ListMediaKind> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "movie" => Some(ListMediaKind::Movie),
        "series" | "tv" | "show" => Some(ListMediaKind::Series),
        "anime" => Some(ListMediaKind::Anime),
        _ => None,
    }
}

/// How an item of a gateway-served list is keyed across syncs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartItemKey {
    /// By gateway title id: chart entries are always resolved.
    GatewayTitle,
    /// By IMDb id: a proxied IMDb entry keeps its key when it resolves later.
    Imdb,
}

/// Turn gateway list entries into plugin-shaped items, keeping list order.
/// Entries with no usable key are dropped rather than invented.
pub fn chart_items_to_plugin_items(
    items: Vec<ListChartItem>,
    key: ChartItemKey,
) -> Vec<(ListPluginItem, Option<String>)> {
    items
        .into_iter()
        .filter_map(|item| {
            let imdb_id = item
                .external_ids
                .iter()
                .find(|id| id.source.eq_ignore_ascii_case("imdb"))
                .map(|id| id.value.clone());
            let item_key = match key {
                ChartItemKey::GatewayTitle => item.title_id.map(|id| format!("smg:{id}")),
                ChartItemKey::Imdb => imdb_id.map(|id| format!("imdb:{id}")),
            }?;
            let mut external_ids = item
                .external_ids
                .iter()
                .map(|id| ListExternalId {
                    source: id.source.clone(),
                    kind: id.kind.clone(),
                    id: id.value.clone(),
                })
                .collect::<Vec<_>>();
            if let Some(title_id) = item.title_id
                && !external_ids
                    .iter()
                    .any(|id| id.source.eq_ignore_ascii_case("smg"))
            {
                external_ids.push(ListExternalId {
                    source: "smg".to_string(),
                    kind: Some("title".to_string()),
                    id: title_id.to_string(),
                });
            }
            let title = Some(item.display_title.trim().to_string()).filter(|t| !t.is_empty());
            let plugin_item = ListPluginItem {
                item_key,
                rank: u32::try_from(item.rank).ok(),
                kind_hint: list_kind(&item.kind),
                title,
                year: item.year,
                external_ids,
                ..ListPluginItem::default()
            };
            Some((plugin_item, item.poster_url))
        })
        .collect()
}

/// A gateway failure, in the list's plain words. Nothing about the gateway's
/// own error text reaches the subscription.
fn gateway_failure(provider: &str, error: &crate::AppError) -> ListFailure {
    let text = error.to_string().to_ascii_lowercase();
    let class = if text.contains("not found") || text.contains("invalid imdb list") {
        ListFailureClass::NotFound
    } else if text.contains("unknown chart") {
        ListFailureClass::Failed
    } else {
        ListFailureClass::Unavailable
    };
    ListFailure::new(class, provider)
}

/// Chart and IMDb list reads through the metadata gateway.
pub struct GatewayListChartSource {
    gateway: Arc<dyn MetadataGateway>,
}

impl GatewayListChartSource {
    pub fn new(gateway: Arc<dyn MetadataGateway>) -> Self {
        Self { gateway }
    }
}

#[async_trait]
impl ListChartSource for GatewayListChartSource {
    async fn chart_items(
        &self,
        provider: &str,
        chart_key: &str,
        scope: &str,
    ) -> Result<Vec<ListChartItem>, ListFailure> {
        self.gateway
            .list_chart_items(
                provider,
                chart_key,
                scope,
                LIST_CHART_ITEM_LIMIT,
                LIST_CHART_LANGUAGE,
            )
            .await
            .map_err(|error| gateway_failure(provider, &error))
    }

    async fn imdb_user_list(&self, list_id: &str) -> Result<Vec<ListChartItem>, ListFailure> {
        self.gateway
            .list_imdb_user_list(list_id)
            .await
            .map_err(|error| gateway_failure("IMDb", &error))
    }
}

/// The ref the gateway resolves one item by. A gateway title id the item
/// already carries is sent as the ref's id so the gateway skips the lookup.
pub fn title_ref(ids: &[ExternalId]) -> TitleExternalRef {
    let smg_id = ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case("smg"))
        .and_then(|id| id.value.trim().parse::<i64>().ok());
    TitleExternalRef {
        smg_id,
        external_ids: ids
            .iter()
            .filter(|id| !id.source.eq_ignore_ascii_case("smg"))
            .cloned()
            .collect(),
    }
}

fn merge_ids(target: &mut Vec<ExternalId>, extra: Vec<ExternalId>) {
    for id in extra {
        if !target.iter().any(|existing| {
            existing.source.eq_ignore_ascii_case(&id.source)
                && existing.value.eq_ignore_ascii_case(&id.value)
        }) {
            target.push(id);
        }
    }
}

/// Resolves every kind through the gateway, then looks each title up in the
/// local library by any id it now carries.
pub struct GatewayListItemResolver<L> {
    gateway: Arc<dyn MetadataGateway>,
    library: L,
}

/// Finds a library title of a kind by external ids, in any library.
#[async_trait]
pub trait ListLibraryLookup: Send + Sync {
    async fn find_title(&self, kind: &MediaFacet, ids: &[ExternalId]) -> AppResult<Option<String>>;
}

impl<L: ListLibraryLookup> GatewayListItemResolver<L> {
    pub fn new(gateway: Arc<dyn MetadataGateway>, library: L) -> Self {
        Self { gateway, library }
    }
}

#[async_trait]
impl<L: ListLibraryLookup> ListItemResolver for GatewayListItemResolver<L> {
    async fn resolve(&self, inputs: &[ResolveInput]) -> AppResult<Vec<ResolveOutput>> {
        let mut outputs = inputs
            .iter()
            .map(|input| ResolveOutput {
                external_ids: input.external_ids.clone(),
                ..ResolveOutput::default()
            })
            .collect::<Vec<_>>();

        // One gateway call per kind per batch; order within a kind is kept.
        let mut by_kind: HashMap<&'static str, Vec<usize>> = HashMap::new();
        for (index, input) in inputs.iter().enumerate() {
            by_kind
                .entry(gateway_kind(&input.kind))
                .or_default()
                .push(index);
        }
        let mut kinds = by_kind.into_iter().collect::<Vec<_>>();
        kinds.sort_by_key(|(kind, _)| *kind);
        for (kind, positions) in kinds {
            for chunk in positions.chunks(RESOLVE_TITLES_BATCH) {
                let refs = chunk
                    .iter()
                    .map(|&position| title_ref(&inputs[position].external_ids))
                    .collect::<Vec<_>>();
                let resolutions = self.gateway.resolve_titles(&refs, kind, true).await?;
                for resolution in resolutions {
                    let Some(&position) = chunk.get(resolution.ref_index) else {
                        continue;
                    };
                    let output = &mut outputs[position];
                    output.resolved = resolution.resolved && resolution.smg_id.is_some();
                    output.smg_title_id = resolution.smg_id;
                    merge_ids(&mut output.external_ids, resolution.external_ids);
                }
            }
        }

        for (input, output) in inputs.iter().zip(outputs.iter_mut()) {
            output.library_title_id = self
                .library
                .find_title(&input.kind, &output.external_ids)
                .await?;
        }
        Ok(outputs)
    }
}

#[cfg(test)]
#[path = "gateway_tests.rs"]
mod tests;
