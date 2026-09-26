use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use scryer_domain::{ExternalId, LIST_SOURCE_IMDB_LIST_ID_PARAM, ListSourceOrigin, MediaFacet};
use scryer_plugin_sdk::ListMediaKind;

use super::*;
use crate::lists::fetch::{ListFailureClass, fetch_list};
use crate::lists::test_support::{ScriptedCharts, ScriptedLists, ScriptedProvider, subscription};
use crate::{
    AppError, BulkMetadataResult, MetadataSearchItem, MetadataSearchQuery, MovieMetadata,
    MultiMetadataSearchResult, RichMetadataSearchItem, SeriesMetadata, TitleResolution,
};

fn id(source: &str, value: &str) -> ExternalId {
    ExternalId {
        source: source.to_string(),
        kind: Some("title".to_string()),
        value: value.to_string(),
    }
}

fn chart_item(rank: i64, title_id: Option<i64>, imdb: Option<&str>) -> ListChartItem {
    ListChartItem {
        rank,
        title_id,
        resolved: title_id.is_some(),
        kind: if title_id.is_some() {
            "movie"
        } else {
            "unknown"
        }
        .to_string(),
        external_ids: imdb
            .map(|value| vec![id("imdb", value)])
            .unwrap_or_default(),
        display_title: format!("Fixture Title {rank}"),
        year: Some(2030),
        poster_url: Some(format!("https://images.invalid/{rank}.jpg")),
    }
}

#[test]
fn chart_entries_are_keyed_by_gateway_title_and_carry_that_id() {
    let items = chart_items_to_plugin_items(
        vec![
            chart_item(1, Some(41), Some("tt0000041")),
            chart_item(2, None, Some("tt0000042")),
        ],
        ChartItemKey::GatewayTitle,
    );

    assert_eq!(items.len(), 1, "an entry with no gateway title has no key");
    let (item, poster) = &items[0];
    assert_eq!(item.item_key, "smg:41");
    assert_eq!(item.rank, Some(1));
    assert_eq!(item.kind_hint, Some(ListMediaKind::Movie));
    assert_eq!(item.title.as_deref(), Some("Fixture Title 1"));
    assert!(
        item.external_ids
            .iter()
            .any(|id| id.source == "smg" && id.id == "41")
    );
    assert_eq!(poster.as_deref(), Some("https://images.invalid/1.jpg"));
}

#[test]
fn imdb_entries_keep_their_imdb_key_whether_or_not_they_resolved() {
    let items = chart_items_to_plugin_items(
        vec![
            chart_item(1, Some(41), Some("tt0000041")),
            chart_item(2, None, Some("tt0000042")),
            chart_item(3, None, None),
        ],
        ChartItemKey::Imdb,
    );

    let keys = items
        .iter()
        .map(|(item, _)| item.item_key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["imdb:tt0000041", "imdb:tt0000042"]);
    assert_eq!(items[1].0.kind_hint, None, "an unknown kind stays unknown");
}

#[test]
fn a_gateway_title_id_becomes_the_ref_id() {
    let reference = title_ref(&[id("smg", "77"), id("trakt", "9001")]);
    assert_eq!(reference.smg_id, Some(77));
    assert_eq!(reference.external_ids, vec![id("trakt", "9001")]);
}

#[tokio::test]
async fn an_imdb_list_reads_through_the_gateway_by_its_list_id() {
    let charts = ScriptedCharts::default();
    charts.imdb_lists.lock().unwrap().insert(
        "ls000000001".to_string(),
        vec![
            chart_item(1, Some(41), Some("tt0000041")),
            chart_item(2, None, Some("tt0000042")),
        ],
    );
    let plugins = ScriptedProvider(ScriptedLists::new());
    let mut list = subscription("list-imdb");
    list.source.provider = "imdb".to_string();
    list.source.origin = ListSourceOrigin::SmgImdbList;
    list.source.params.insert(
        LIST_SOURCE_IMDB_LIST_ID_PARAM.to_string(),
        "ls000000001".to_string(),
    );

    let fetched = fetch_list(&list, &plugins, &charts, None, &Default::default())
        .await
        .expect("imdb list");

    assert_eq!(fetched.items.len(), 2);
    assert_eq!(
        fetched.posters.get("imdb:tt0000042").map(String::as_str),
        Some("https://images.invalid/2.jpg")
    );

    list.source.params.clear();
    let failure = fetch_list(&list, &plugins, &charts, None, &Default::default())
        .await
        .expect_err("no list id");
    assert_eq!(failure.class, ListFailureClass::NotFound);
}

#[tokio::test]
async fn a_chart_the_gateway_cannot_serve_is_a_plain_failure() {
    let charts = ScriptedCharts::default();
    let plugins = ScriptedProvider(ScriptedLists::new());
    let mut list = subscription("list-chart");
    list.source.origin = ListSourceOrigin::SmgChart {
        chart_key: "popular".to_string(),
        scope: "global".to_string(),
    };

    let failure = fetch_list(&list, &plugins, &charts, None, &Default::default())
        .await
        .expect_err("chart unavailable");
    assert_eq!(failure.class, ListFailureClass::Unavailable);
}

// ── Resolver ───────────────────────────────────────────────────────────────

#[derive(Default)]
struct RecordingResolveGateway {
    calls: Mutex<Vec<(String, Vec<TitleExternalRef>)>>,
}

#[async_trait]
impl MetadataGateway for RecordingResolveGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        unimplemented!("list resolver fixture only resolves titles")
    }

    async fn resolve_titles(
        &self,
        refs: &[TitleExternalRef],
        kind: &str,
        create_missing: bool,
    ) -> AppResult<Vec<TitleResolution>> {
        assert!(create_missing, "list items create missing titles");
        self.calls
            .lock()
            .unwrap()
            .push((kind.to_string(), refs.to_vec()));
        Ok(refs
            .iter()
            .enumerate()
            .filter_map(|(index, reference)| {
                let alias = reference.external_ids.first()?;
                let smg_id = reference.smg_id.unwrap_or(1000 + index as i64);
                Some(TitleResolution {
                    ref_index: index,
                    resolved: true,
                    smg_id: Some(smg_id),
                    kind: kind.to_string(),
                    primary_source: "tmdb".to_string(),
                    redirected_from: None,
                    created: false,
                    external_ids: vec![id("tmdb", &format!("{}{}", alias.value, 5))],
                    reason: String::new(),
                })
            })
            .collect())
    }
}

struct FixtureLibrary(HashMap<String, String>);

#[async_trait]
impl ListLibraryLookup for FixtureLibrary {
    async fn find_title(
        &self,
        _kind: &MediaFacet,
        ids: &[ExternalId],
    ) -> AppResult<Option<String>> {
        Ok(ids.iter().find_map(|id| self.0.get(&id.value).cloned()))
    }
}

#[tokio::test]
async fn items_resolve_per_kind_in_batches_and_find_their_library_title() {
    let gateway = Arc::new(RecordingResolveGateway::default());
    let library = FixtureLibrary(HashMap::from([(
        "trakt-25".to_string(),
        "title-in-library".to_string(),
    )]));
    let resolver = GatewayListItemResolver::new(gateway.clone(), library);
    let mut inputs = (0..(RESOLVE_TITLES_BATCH + 1))
        .map(|n| ResolveInput {
            kind: MediaFacet::Movie,
            external_ids: vec![id("trakt", &format!("trakt-{n}"))],
        })
        .collect::<Vec<_>>();
    inputs.insert(
        3,
        ResolveInput {
            kind: MediaFacet::Anime,
            external_ids: vec![id("anilist", "anilist-7")],
        },
    );
    inputs.push(ResolveInput {
        kind: MediaFacet::Series,
        external_ids: Vec::new(),
    });

    let outputs = resolver.resolve(&inputs).await.expect("resolve");

    let calls = gateway.calls.lock().unwrap().clone();
    let shapes = calls
        .iter()
        .map(|(kind, refs)| (kind.as_str(), refs.len()))
        .collect::<Vec<_>>();
    assert_eq!(
        shapes,
        vec![
            ("anime", 1),
            ("movie", RESOLVE_TITLES_BATCH),
            ("movie", 1),
            ("series", 1)
        ]
    );
    assert_eq!(outputs.len(), inputs.len());
    let anime = &outputs[3];
    assert!(anime.resolved);
    assert_eq!(anime.smg_title_id, Some(1000));
    assert!(anime.external_ids.contains(&id("anilist", "anilist-7")));
    assert!(anime.external_ids.contains(&id("tmdb", "anilist-75")));
    let last_movie = &outputs[RESOLVE_TITLES_BATCH + 1];
    assert_eq!(
        last_movie.smg_title_id,
        Some(1000),
        "second batch restarts its index"
    );
    assert_eq!(
        outputs[26].library_title_id.as_deref(),
        Some("title-in-library")
    );
    let series = outputs.last().unwrap();
    assert!(!series.resolved, "an item with no ids stays unresolved");
}

#[tokio::test]
async fn a_gateway_error_fails_the_whole_resolve() {
    let resolver = GatewayListItemResolver::new(
        Arc::new(crate::library_scan::NullMetadataGateway),
        FixtureLibrary(HashMap::new()),
    );
    let error = resolver
        .resolve(&[ResolveInput {
            kind: MediaFacet::Series,
            external_ids: vec![id("simkl", "simkl-1")],
        }])
        .await
        .expect_err("gateway down");
    assert!(matches!(
        error,
        AppError::Repository(_) | AppError::Validation(_)
    ));
}
