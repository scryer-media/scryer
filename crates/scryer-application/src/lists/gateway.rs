//! The metadata gateway's side of Lists: public charts and identity
//! resolution for every kind.
//!
//! Charts the gateway already ingests are read from it rather than from the
//! provider, so an instance never spends its own rate limit on a public chart.
//! Every list item, whatever id scheme its provider speaks, is
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

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
pub struct ListMovieTarget {
    pub movie_id: i64,
    pub parents: Vec<ListMovieParent>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
pub struct ListMovieParent {
    pub title_id: i64,
    pub tvdb_id: i64,
    pub name: String,
}

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

/// One entry of a chart, in chart order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListChartItem {
    pub rank: i64,
    pub title_id: Option<i64>,
    pub resolved: bool,
    /// `movie`, `series` or `anime`.
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

/// Turn chart entries into plugin-shaped items, keeping chart order. Each is
/// keyed by its gateway title id across syncs; an entry without one is
/// dropped rather than given an invented key.
pub fn chart_items_to_plugin_items(
    items: Vec<ListChartItem>,
) -> Vec<(ListPluginItem, Option<String>)> {
    items
        .into_iter()
        .filter_map(|item| {
            let item_key = item.title_id.map(|id| format!("smg:{id}"))?;
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
    let class = if text.contains("not found") {
        ListFailureClass::NotFound
    } else if text.contains("unknown chart") {
        ListFailureClass::Failed
    } else {
        ListFailureClass::Unavailable
    };
    ListFailure::new(class, provider)
}

/// Chart reads through the metadata gateway.
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

fn parse_release_date(value: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|date| date.date_naive())
        })
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
    vocabulary: Option<(
        Arc<super::vocabulary::VocabularyRuntime>,
        Arc<dyn super::ListSubscriptionRepository>,
    )>,
}

/// Finds a library title of a kind by external ids, in any library.
#[async_trait]
pub trait ListLibraryLookup: Send + Sync {
    async fn find_title(&self, kind: &MediaFacet, ids: &[ExternalId]) -> AppResult<Option<String>>;
    async fn find_titles(
        &self,
        targets: &[(MediaFacet, Vec<ExternalId>)],
    ) -> AppResult<Vec<Option<String>>> {
        let mut found = Vec::with_capacity(targets.len());
        for (kind, ids) in targets {
            found.push(self.find_title(kind, ids).await?);
        }
        Ok(found)
    }
    async fn find_series_movies(
        &self,
        _targets: &[(scryer_domain::ListSeriesMovieTarget, Vec<ExternalId>)],
    ) -> AppResult<Vec<Option<(String, String)>>> {
        Err(crate::AppError::Repository(
            "series movie lookup is not implemented".into(),
        ))
    }
}

impl<L: ListLibraryLookup> GatewayListItemResolver<L> {
    pub fn new(gateway: Arc<dyn MetadataGateway>, library: L) -> Self {
        Self {
            gateway,
            library,
            vocabulary: None,
        }
    }

    pub fn with_vocabulary(
        mut self,
        runtime: Arc<super::vocabulary::VocabularyRuntime>,
        store: Arc<dyn super::ListSubscriptionRepository>,
    ) -> Self {
        self.vocabulary = Some((runtime, store));
        self
    }
}

#[async_trait]
impl<L: ListLibraryLookup> ListItemResolver for GatewayListItemResolver<L> {
    async fn normalize_filters(
        &self,
        subscription: &scryer_domain::ListSubscription,
    ) -> AppResult<scryer_domain::ListSubscription> {
        let mut subscription = subscription.clone();
        let snapshot = if subscription.filters.iter().any(|filter| matches!(filter, scryer_domain::ListFilter::ExcludeGenres { genres } if !genres.is_empty())) {
            let (runtime, store) = self.vocabulary.as_ref().ok_or_else(|| crate::AppError::Repository("canonical vocabulary is not configured".into()))?;
            Some(runtime.get(store.clone(), self.gateway.clone(), false).await?)
        } else { None };
        subscription.filters = super::vocabulary::normalize_filters(
            &subscription.filters,
            &subscription.kinds,
            snapshot
                .as_ref()
                .map(|snapshot| snapshot.entries.as_slice())
                .unwrap_or_default(),
        );
        Ok(subscription)
    }

    async fn enrich(&self, items: &mut [super::resolve::ResolvedItem]) -> AppResult<()> {
        use super::resolve::ListMetadataFacts;
        use std::collections::BTreeSet;
        let mut movies = BTreeSet::new();
        let mut series = BTreeSet::new();
        for item in items.iter() {
            if let Some(id) = item.smg_title_id {
                if item.kind == Some(MediaFacet::Movie) || item.series_movie.is_some() {
                    movies.insert(id);
                } else {
                    series.insert(id);
                }
            }
        }
        let mut facts = HashMap::new();
        for chunk in movies
            .into_iter()
            .collect::<Vec<_>>()
            .chunks(RESOLVE_TITLES_BATCH)
        {
            let refs = chunk
                .iter()
                .map(|id| crate::MovieTitleRef {
                    smg_id: Some(*id),
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            let result = self
                .gateway
                .get_movie_titles(&refs, LIST_CHART_LANGUAGE)
                .await?;
            for (index, metadata) in result.by_ref_index {
                if let Some(id) = chunk.get(index) {
                    facts.insert(
                        (true, *id),
                        ListMetadataFacts {
                            poster_url: Some(metadata.poster_url)
                                .filter(|url| !url.trim().is_empty()),
                            ratings: metadata.ratings.external_ratings,
                            canonical_names: metadata
                                .canonical_tags
                                .iter()
                                .map(|tag| tag.name.clone())
                                .collect(),
                            canonical_keys: metadata
                                .canonical_tags
                                .into_iter()
                                .map(|tag| tag.key)
                                .collect(),
                            original_language: metadata.original_language,
                            year: metadata.year,
                            release_date: metadata
                                .tmdb_release_date
                                .as_deref()
                                .and_then(parse_release_date),
                        },
                    );
                }
            }
        }
        for chunk in series
            .into_iter()
            .collect::<Vec<_>>()
            .chunks(RESOLVE_TITLES_BATCH)
        {
            let refs = chunk
                .iter()
                .map(|id| crate::SeriesTitleRef {
                    smg_id: Some(*id),
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            let result = self
                .gateway
                .get_series_titles(&refs, LIST_CHART_LANGUAGE, false, false)
                .await?;
            for (index, metadata) in result.by_ref_index {
                if let Some(id) = chunk.get(index) {
                    facts.insert(
                        (false, *id),
                        ListMetadataFacts {
                            poster_url: Some(metadata.poster_url)
                                .filter(|url| !url.trim().is_empty()),
                            ratings: metadata.ratings.external_ratings,
                            canonical_names: metadata
                                .canonical_tags
                                .iter()
                                .map(|tag| tag.name.clone())
                                .collect(),
                            canonical_keys: metadata
                                .canonical_tags
                                .into_iter()
                                .map(|tag| tag.key)
                                .collect(),
                            original_language: metadata.original_language,
                            year: metadata.year,
                            release_date: parse_release_date(&metadata.first_aired),
                        },
                    );
                }
            }
        }
        for item in items {
            item.facts = item.smg_title_id.and_then(|id| {
                facts
                    .get(&(
                        item.kind == Some(MediaFacet::Movie) || item.series_movie.is_some(),
                        id,
                    ))
                    .cloned()
            });
        }
        Ok(())
    }

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

        let movie_ids = inputs
            .iter()
            .zip(&outputs)
            .filter(|(input, output)| input.kind == MediaFacet::Movie && output.resolved)
            .filter_map(|(_, output)| output.smg_title_id)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut targets = HashMap::new();
        for chunk in movie_ids.chunks(RESOLVE_TITLES_BATCH) {
            let rows = self.gateway.list_movie_targets(chunk).await?;
            if rows.len() != chunk.len() {
                return Err(crate::AppError::Repository(
                    "incomplete movie relationship response".into(),
                ));
            }
            for row in rows {
                if !chunk.contains(&row.movie_id)
                    || targets.insert(row.movie_id, row.parents).is_some()
                {
                    return Err(crate::AppError::Repository(
                        "invalid movie relationship response".into(),
                    ));
                }
            }
        }
        let mut linked_positions = Vec::new();
        let mut linked_targets = Vec::new();
        let mut title_positions = Vec::new();
        let mut title_targets = Vec::new();
        for (position, (input, output)) in inputs.iter().zip(outputs.iter_mut()).enumerate() {
            if input.kind == MediaFacet::Movie
                && let Some(parents) = output.smg_title_id.and_then(|id| targets.get(&id))
            {
                match parents.as_slice() {
                    [] => {}
                    [parent] => {
                        let target = scryer_domain::ListSeriesMovieTarget {
                            parent_smg_id: parent.title_id,
                            parent_tvdb_id: parent.tvdb_id,
                            parent_name: parent.name.clone(),
                            link_id: None,
                        };
                        linked_positions.push(position);
                        linked_targets.push((target.clone(), output.external_ids.clone()));
                        output.series_movie = Some(target);
                        continue;
                    }
                    _ => {
                        output.resolved = false;
                        output.resolution_reason = Some("ambiguous_series_movie".into());
                        continue;
                    }
                }
            }
            title_positions.push(position);
            title_targets.push((input.kind.clone(), output.external_ids.clone()));
        }
        if !title_targets.is_empty() {
            let found = self.library.find_titles(&title_targets).await?;
            if found.len() != title_positions.len() {
                return Err(crate::AppError::Repository(
                    "incomplete library lookup".into(),
                ));
            }
            for (position, title_id) in title_positions.into_iter().zip(found) {
                outputs[position].library_title_id = title_id;
            }
        }
        if !linked_targets.is_empty() {
            let found = self.library.find_series_movies(&linked_targets).await?;
            if found.len() != linked_positions.len() {
                return Err(crate::AppError::Repository(
                    "incomplete series movie lookup".into(),
                ));
            }
            for (position, found) in linked_positions.into_iter().zip(found) {
                if let Some((title_id, link_id)) = found {
                    let output = &mut outputs[position];
                    output.library_title_id = Some(title_id);
                    if let Some(target) = &mut output.series_movie {
                        target.link_id = Some(link_id);
                    }
                }
            }
        }
        Ok(outputs)
    }
}

#[cfg(test)]
#[path = "gateway_tests.rs"]
mod tests;
