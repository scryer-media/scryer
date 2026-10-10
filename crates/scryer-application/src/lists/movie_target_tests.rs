use super::*;
use crate::lists::evaluate::{ItemDecision, evaluate};
use crate::lists::resolve::resolve_items;
use crate::lists::test_support::{plugin_item, route};

fn movie(id_value: i64) -> ListPluginItem {
    let mut item = plugin_item(&format!("smg:{id_value}"));
    item.external_ids.clear();
    item.external_ids.push(ListExternalId {
        source: "smg".into(),
        kind: Some("movie".into()),
        id: id_value.to_string(),
    });
    let supplied: &[(&str, &str, &str)] = match id_value {
        2768085 => &[
            ("anidb", "anime", "18333"),
            ("anilist", "anime", "171952"),
            ("mal", "anime", "57584"),
            ("tmdb", "movie", "1220552"),
            ("trakt", "movie", "985120"),
            ("tvdb", "movie", "352365"),
        ],
        3021408 => &[
            ("anidb", "anime", "16259"),
            ("anilist", "anime", "195200"),
            ("mal", "anime", "62546"),
            ("imdb", "movie", "tt14888874"),
            ("tmdb", "movie", "802401"),
            ("trakt", "movie", "637062"),
            ("tvdb", "movie", "374512"),
            ("wikidata", "movie", "Q110118148"),
        ],
        _ => &[],
    };
    item.external_ids
        .extend(supplied.iter().map(|(source, kind, id)| ListExternalId {
            source: (*source).into(),
            kind: Some((*kind).into()),
            id: (*id).into(),
        }));
    item.kind_hint = Some(ListMediaKind::Movie);
    item
}

fn parent() -> ListMovieParent {
    ListMovieParent {
        title_id: 42,
        tvdb_id: 43,
        name: "Fixture parent".into(),
    }
}

#[tokio::test]
async fn excluded_movie_is_resolved_then_filtered_not_unmatched() {
    let gateway = Arc::new(RecordingResolveGateway::default());
    let resolver = GatewayListItemResolver::new(gateway.clone(), FixtureLibrary(HashMap::new()));
    let mut list = subscription("fixture-list");
    list.kinds = vec![MediaFacet::Anime];
    list.routes = vec![route(MediaFacet::Anime, "anime-library")];
    let items = resolve_items(&list, vec![movie(2768085)], &resolver)
        .await
        .unwrap();
    assert!(items[0].resolved);
    assert_eq!(items[0].kind, Some(MediaFacet::Movie));
    let result = evaluate(&list, items, &[], &HashMap::new());
    assert_eq!(
        result[0].decision,
        ItemDecision::Filtered {
            reason: "media_type_not_included".into()
        }
    );
    assert_eq!(
        *gateway.relationship_calls.lock().unwrap(),
        vec![vec![2768085]]
    );
}

#[tokio::test]
async fn series_movie_keeps_movie_identity_and_enrichment_in_anime_route() {
    let gateway = Arc::new(RecordingResolveGateway {
        movie_parents: HashMap::from([(3021408, vec![parent()])]),
        ..Default::default()
    });
    let resolver = GatewayListItemResolver::new(gateway.clone(), FixtureLibrary(HashMap::new()));
    let mut list = subscription("fixture-list");
    list.kinds = vec![MediaFacet::Anime];
    list.routes = vec![route(MediaFacet::Anime, "anime-library")];
    list.filters = vec![scryer_domain::ListFilter::Language {
        languages: vec!["en".into()],
    }];
    let items = resolve_items(&list, vec![movie(3021408)], &resolver)
        .await
        .unwrap();
    assert_eq!(items[0].kind, Some(MediaFacet::Anime));
    assert_eq!(items[0].smg_title_id, Some(3021408));
    assert_eq!(items[0].series_movie.as_ref().unwrap().parent_smg_id, 42);
    assert_eq!(*gateway.metadata_calls.lock().unwrap(), vec![(true, 1)]);
    assert_eq!(
        evaluate(&list, items, &[], &HashMap::new())[0].decision,
        ItemDecision::Candidate
    );
}

#[tokio::test]
async fn ambiguous_movie_stays_unresolved_and_outage_is_not_standalone() {
    let gateway = Arc::new(RecordingResolveGateway {
        movie_parents: HashMap::from([(
            3021408,
            vec![
                parent(),
                ListMovieParent {
                    title_id: 44,
                    ..parent()
                },
            ],
        )]),
        ..Default::default()
    });
    let resolver = GatewayListItemResolver::new(gateway, FixtureLibrary(HashMap::new()));
    let list = subscription("fixture-list");
    let items = resolve_items(&list, vec![movie(3021408)], &resolver)
        .await
        .unwrap();
    assert_eq!(
        items[0].resolution_reason.as_deref(),
        Some("ambiguous_series_movie")
    );
    assert_eq!(
        evaluate(&list, items, &[], &HashMap::new())[0].decision,
        ItemDecision::Unresolved
    );
    let resolver = GatewayListItemResolver::new(
        Arc::new(RecordingResolveGateway {
            relationship_failure: true,
            ..Default::default()
        }),
        FixtureLibrary(HashMap::new()),
    );
    assert!(
        resolve_items(&list, vec![movie(3021408)], &resolver)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn relationship_queries_are_deduplicated_and_batched() {
    let gateway = Arc::new(RecordingResolveGateway::default());
    let resolver = GatewayListItemResolver::new(gateway.clone(), FixtureLibrary(HashMap::new()));
    let list = subscription("fixture-list");
    let items = (1..=101).flat_map(|id| [movie(id), movie(id)]).collect();
    resolve_items(&list, items, &resolver).await.unwrap();
    assert_eq!(
        gateway
            .relationship_calls
            .lock()
            .unwrap()
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        vec![50, 50, 1]
    );
}

#[tokio::test]
async fn standalone_anime_film_uses_selected_movie_library_and_preview_cap() {
    for library in ["default-movies", "anime-movies"] {
        let gateway = Arc::new(RecordingResolveGateway::default());
        let resolver = GatewayListItemResolver::new(gateway, FixtureLibrary(HashMap::new()));
        let mut list = subscription("fixture-list");
        list.kinds = vec![MediaFacet::Anime, MediaFacet::Movie];
        list.routes = vec![
            route(MediaFacet::Anime, "anime"),
            route(MediaFacet::Movie, library),
        ];
        list.max_per_sync = Some(1);
        let mut film = movie(900001);
        film.title = Some("Spirited Away".into());
        film.external_ids.push(ListExternalId {
            source: "tmdb".into(),
            kind: Some("movie".into()),
            id: "129".into(),
        });
        let mut duplicate = film.clone();
        duplicate.item_key = "same-film-another-key".into();
        let resolved = resolve_items(&list, vec![film, duplicate, movie(900002)], &resolver)
            .await
            .unwrap();
        assert_eq!(resolved[0].kind, Some(MediaFacet::Movie));
        assert!(resolved[0].series_movie.is_none());
        assert_eq!(
            list.routes
                .iter()
                .find(|route| Some(&route.kind) == resolved[0].kind.as_ref())
                .unwrap()
                .library_id,
            library
        );
        let evaluated = evaluate(&list, resolved, &[], &HashMap::new());
        assert_eq!(evaluated[0].decision, ItemDecision::Candidate);
        assert_eq!(
            evaluated[1].decision,
            ItemDecision::Filtered {
                reason: "duplicate_target".into()
            }
        );
        assert_eq!(evaluated[2].decision, ItemDecision::Deferred);
        let preview = crate::lists::public::summarize_preview(&evaluated, &HashMap::new());
        assert_eq!(preview.would_add.len(), 1);
        assert_eq!(preview.would_add[0].canonical_smg_id, Some(900001));
    }
}
