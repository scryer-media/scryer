use std::collections::HashMap;

use scryer_domain::{ListMembershipState, ListMode, MediaFacet, MediaRequestOrigin};

use super::*;
use crate::lists::evaluate::{EvaluatedItem, ItemDecision};
use crate::lists::test_support::{at, membership, resolved_item, route, subscription};

#[test]
fn public_lists_cannot_request_or_discover() {
    let routes = [route(MediaFacet::Movie, "library-movies")];
    for mode in [ListMode::Request, ListMode::Discover] {
        assert!(validate_public_settings(&[MediaFacet::Movie], None, mode, &routes, None).is_err());
    }
    for mode in [ListMode::Add, ListMode::Hold] {
        assert_eq!(
            validate_public_settings(&[MediaFacet::Movie], None, mode, &routes, None).unwrap(),
            vec![MediaFacet::Movie]
        );
    }
}

#[test]
fn kinds_narrow_the_declared_set_and_routes_must_fit_them() {
    let declared = [MediaFacet::Movie, MediaFacet::Series];
    let kinds = validate_public_settings(
        &declared,
        Some(&[MediaFacet::Series]),
        ListMode::Add,
        &[route(MediaFacet::Series, "library-series")],
        Some(5),
    )
    .unwrap();
    assert_eq!(kinds, vec![MediaFacet::Series]);

    // A kind the source never lists.
    assert!(
        validate_public_settings(
            &declared,
            Some(&[MediaFacet::Anime]),
            ListMode::Add,
            &[],
            None
        )
        .is_err()
    );
    // A route for a kind the follow dropped.
    assert!(
        validate_public_settings(
            &declared,
            Some(&[MediaFacet::Series]),
            ListMode::Add,
            &[route(MediaFacet::Movie, "library-movies")],
            None,
        )
        .is_err()
    );
    // Two routes for one kind.
    assert!(
        validate_public_settings(
            &declared,
            None,
            ListMode::Add,
            &[
                route(MediaFacet::Movie, "library-movies"),
                route(MediaFacet::Movie, "library-movies-two"),
            ],
            None,
        )
        .is_err()
    );
    // A zero cap.
    assert!(validate_public_settings(&declared, None, ListMode::Add, &[], Some(0)).is_err());
}

#[test]
fn viewers_without_manage_lists_do_not_see_failure_text() {
    let mut followed = subscription("public-a");
    followed.sync.error_message = Some("The list no longer exists or is private.".into());
    followed.sync.fetch_fingerprint = Some("fingerprint".into());

    let viewer = redact_for_viewer(followed.clone(), false);
    assert_eq!(viewer.sync.error_message, None);
    assert_eq!(viewer.sync.fetch_fingerprint, None);

    let manager = redact_for_viewer(followed, true);
    assert!(manager.sync.error_message.is_some());
    assert_eq!(manager.sync.fetch_fingerprint, None);
}

#[test]
fn viewers_without_manage_lists_do_not_see_source_credentials() {
    let mut followed = subscription("public-a");
    followed.provider_url =
        Some("https://feeduser:s3cret@lists.invalid/feed?token=abc&page=1".into());
    followed.source.params.insert(
        "url".into(),
        "https://lists.invalid/feed?apikey=live-key&page=1".into(),
    );
    followed
        .source
        .params
        .insert("list_id".into(), "synthetic-list".into());

    let viewer = redact_for_viewer(followed.clone(), false);
    assert_eq!(
        viewer.provider_url.as_deref(),
        Some("https://[redacted]@lists.invalid/feed?token=[redacted]&page=1")
    );
    assert_eq!(
        viewer.source.params.get("url").map(String::as_str),
        Some("https://lists.invalid/feed?apikey=[redacted]&page=1")
    );
    assert_eq!(
        viewer.source.params.get("list_id").map(String::as_str),
        Some("synthetic-list")
    );

    let manager = redact_for_viewer(followed.clone(), true);
    assert_eq!(manager.provider_url, followed.provider_url);
    assert_eq!(manager.source.params, followed.source.params);
}

#[test]
fn membership_pages_follow_list_order() {
    let mut rows = Vec::new();
    for (key, rank) in [("c", None), ("b", Some(2)), ("a", Some(1))] {
        let mut row = membership("public-a", key, ListMembershipState::InLibrary);
        row.rank = rank;
        rows.push(row);
    }
    let page = membership_page(rows.clone(), 2, 0);
    assert_eq!(page.total_count, 3);
    let keys = page
        .items
        .iter()
        .map(|row| row.item_key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["a", "b"]);

    let rest = membership_page(rows, 2, 2);
    assert_eq!(rest.items.len(), 1);
    assert_eq!(rest.items[0].item_key, "c");
}

#[test]
fn an_entry_in_a_library_goes_by_its_library_title() {
    let mut linked = membership("public-a", "linked", ListMembershipState::InLibrary);
    linked.display_title = Some("Listed Name".into());
    linked.title_id = Some("title-linked".into());
    let mut unlinked = membership("public-a", "unlinked", ListMembershipState::Unresolved);
    unlinked.display_title = Some("Listed Name".into());
    unlinked.title_id = None;
    // The title was since removed, so there is no library name to take.
    let mut gone = membership("public-a", "gone", ListMembershipState::InLibrary);
    gone.display_title = Some("Listed Name".into());
    gone.title_id = Some("title-gone".into());
    // A movie within a series is linked to the series, whose name is not the movie's.
    let mut within = membership("public-a", "within", ListMembershipState::InLibrary);
    within.display_title = Some("Listed Movie".into());
    within.title_id = Some("title-series".into());
    within.series_movie = Some(scryer_domain::ListSeriesMovieTarget {
        parent_smg_id: 1,
        parent_tvdb_id: 1,
        parent_name: "Fixture Series".into(),
        link_id: None,
    });

    let names = HashMap::from([
        ("title-linked".to_string(), " Library Name ".to_string()),
        ("title-series".to_string(), "Fixture Series".to_string()),
    ]);
    let mut rows = vec![linked, unlinked, gone, within];
    name_by_library_title(&mut rows, &names);
    let shown = rows
        .iter()
        .map(|row| row.display_title.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        shown,
        vec![
            Some("Library Name"),
            Some("Listed Name"),
            Some("Listed Name"),
            Some("Listed Movie"),
        ]
    );
}

#[test]
fn preview_with_ten_per_sync_only_adds_ten_of_ninety_eight_candidates() {
    for scope in [
        scryer_domain::ListScope::Public,
        scryer_domain::ListScope::Personal,
    ] {
        for (cap, expected) in [(Some(10), 10), (Some(1), 1), (None, 98)] {
            let mut list = subscription("preview-cap");
            list.scope = scope;
            list.max_per_sync = cap;
            let items = (0..98)
                .map(|n| resolved_item(&format!("item-{n}")))
                .collect();
            let evaluated = crate::lists::evaluate::evaluate(&list, items, &[], &HashMap::new());
            let preview = summarize_preview(&evaluated, &HashMap::new());
            assert_eq!(preview.total, 98);
            assert_eq!(preview.would_add.len(), expected);
            assert_eq!(preview.filtered, 0);
            assert_eq!(
                evaluated
                    .iter()
                    .filter(|row| row.decision == ItemDecision::Deferred)
                    .count(),
                98 - expected
            );
            assert_eq!(
                preview.would_add.last().unwrap().item_key,
                format!("item-{}", expected - 1)
            );
        }
    }
}

#[test]
fn a_preview_adds_exactly_the_candidates() {
    let decisions = vec![
        ("one", ItemDecision::Candidate),
        ("two", ItemDecision::Deferred),
        ("three", ItemDecision::Excluded),
        (
            "four",
            ItemDecision::Filtered {
                reason: "rating".into(),
            },
        ),
        (
            "five",
            ItemDecision::InLibrary {
                title_id: "title-5".into(),
            },
        ),
        ("six", ItemDecision::Unresolved),
        (
            "seven",
            ItemDecision::Keep {
                state: ListMembershipState::Added,
            },
        ),
    ];
    let evaluated = decisions
        .into_iter()
        .map(|(key, decision)| EvaluatedItem {
            item: resolved_item(key),
            decision,
        })
        .collect::<Vec<_>>();
    let posters = HashMap::from([(
        "one".to_string(),
        "https://img.example.test/one.jpg".to_string(),
    )]);

    let preview = summarize_preview(&evaluated, &posters);
    assert_eq!(preview.total, 7);
    assert_eq!(preview.excluded, 1);
    assert_eq!(preview.filtered, 1);
    assert_eq!(preview.in_library, 2);
    assert_eq!(preview.unresolved, 1);
    assert_eq!(preview.would_add.len(), 1);
    assert_eq!(preview.would_add[0].item_key, "one");
    assert_eq!(
        preview.would_add[0].display_title.as_deref(),
        Some("Fixture Title one")
    );
    assert_eq!(
        preview.would_add[0].poster_url.as_deref(),
        Some("https://img.example.test/one.jpg")
    );
}

#[tokio::test]
async fn preview_flyout_enrichment_is_bounded_reuses_facts_and_preserves_decisions() {
    use crate::lists::resolve::{
        ListItemResolver, ListMetadataFacts, ResolveInput, ResolveOutput, ResolvedItem,
    };
    use std::sync::Mutex;
    struct RecordingResolver(Mutex<Vec<Vec<String>>>);
    #[async_trait::async_trait]
    impl ListItemResolver for RecordingResolver {
        async fn resolve(&self, _: &[ResolveInput]) -> AppResult<Vec<ResolveOutput>> {
            unreachable!()
        }
        async fn enrich(&self, items: &mut [ResolvedItem]) -> AppResult<()> {
            self.0.lock().unwrap().push(
                items
                    .iter()
                    .map(|item| item.item.item_key.clone())
                    .collect(),
            );
            for item in items {
                item.facts = Some(ListMetadataFacts {
                    name: Some(" Looked Up Name ".into()),
                    original_language: Some("ja".into()),
                    poster_url: Some("https://images.example.test/enriched.jpg".into()),
                    canonical_names: vec!["Adventure".into()],
                    ..Default::default()
                });
            }
            Ok(())
        }
    }
    let resolver = RecordingResolver(Mutex::new(Vec::new()));
    let mut items = (0..LIST_PREVIEW_WOULD_ADD_MAX + 2)
        .map(|index| EvaluatedItem {
            item: resolved_item(&format!("item-{index}")),
            decision: ItemDecision::Candidate,
        })
        .collect::<Vec<_>>();
    items[0].item.facts = Some(ListMetadataFacts {
        original_language: Some("en".into()),
        ..Default::default()
    });
    items.insert(
        1,
        EvaluatedItem {
            item: resolved_item("deferred"),
            decision: ItemDecision::Deferred,
        },
    );
    let decisions = items
        .iter()
        .map(|item| item.decision.clone())
        .collect::<Vec<_>>();
    enrich_preview_candidates(&mut items, &resolver)
        .await
        .unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item.decision.clone())
            .collect::<Vec<_>>(),
        decisions
    );
    assert!(items[1].item.facts.is_none());
    assert!(items.last().unwrap().item.facts.is_none());
    let preview = summarize_preview(&items, &HashMap::new());
    assert_eq!(preview.would_add.len(), LIST_PREVIEW_WOULD_ADD_MAX);
    assert_eq!(
        preview.would_add[1].poster_url.as_deref(),
        Some("https://images.example.test/enriched.jpg")
    );
    assert_eq!(
        preview.would_add[0]
            .facts
            .as_ref()
            .unwrap()
            .original_language
            .as_deref(),
        Some("en")
    );
    assert_eq!(
        preview.would_add[1].facts.as_ref().unwrap().canonical_names,
        ["Adventure"]
    );
    // A looked-up title goes by the gateway's name; one without keeps the list's.
    assert_eq!(
        preview.would_add[1].display_title.as_deref(),
        Some("Looked Up Name")
    );
    assert_eq!(
        preview.would_add[0].display_title,
        items[0].item.item.title.clone()
    );
    enrich_preview_candidates(&mut items, &resolver)
        .await
        .unwrap();
    let calls = resolver.0.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].len(), LIST_PREVIEW_WOULD_ADD_MAX - 1);
    assert_eq!(calls[0][0], "item-1");
}

#[test]
fn preview_artwork_falls_back_to_metadata_for_every_facet() {
    use crate::lists::resolve::ListMetadataFacts;
    for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
        for (source, metadata, expected) in [
            (None, Some("metadata.jpg"), Some("metadata.jpg")),
            (Some("chart.jpg"), Some("metadata.jpg"), Some("chart.jpg")),
            (Some("  "), Some("metadata.jpg"), Some("metadata.jpg")),
            (Some("chart.jpg"), None, Some("chart.jpg")),
            (None, None, None),
        ] {
            let mut item = resolved_item("artwork");
            item.kind = Some(facet.clone());
            item.facts = Some(ListMetadataFacts {
                poster_url: metadata.map(str::to_string),
                ..Default::default()
            });
            let evaluated = vec![EvaluatedItem {
                item,
                decision: ItemDecision::Candidate,
            }];
            let posters = source
                .map(|url| ("artwork".into(), url.into()))
                .into_iter()
                .collect();
            let preview = summarize_preview(&evaluated, &posters);
            assert_eq!(preview.would_add[0].poster_url.as_deref(), expected);
        }
    }
}

#[tokio::test]
async fn preview_and_sync_evaluator_use_the_same_canonical_facts_for_both_scopes() {
    use crate::lists::resolve::{ListMetadataFacts, resolve_items};
    use crate::lists::test_support::{FixtureResolver, plugin_item};
    use scryer_domain::{ListFilter, ListScope};
    for scope in [ListScope::Public, ListScope::Personal] {
        let mut list = subscription("filtered-preview");
        list.scope = scope;
        list.filters = vec![ListFilter::Language {
            languages: vec!["eng".into()],
        }];
        let resolver = FixtureResolver::default();
        for (language, expected_candidates) in [(None, 0), (Some("ja"), 0), (Some("en"), 1)] {
            *resolver.facts.lock().unwrap() = Some(ListMetadataFacts {
                original_language: language.map(str::to_string),
                ..Default::default()
            });
            let resolved = resolve_items(&list, vec![plugin_item("one")], &resolver)
                .await
                .unwrap();
            let decisions = evaluate(&list, resolved, &[], &HashMap::new());
            let preview = summarize_preview(&decisions, &HashMap::new());
            assert_eq!(preview.would_add.len(), expected_candidates);
            assert_eq!(preview.filtered, 1 - expected_candidates as u64);
            assert_eq!(
                decisions
                    .iter()
                    .filter(|item| matches!(item.decision, ItemDecision::Candidate))
                    .count(),
                expected_candidates
            );
        }
    }
}

#[test]
fn list_request_counts_skip_manual_and_old_requests() {
    let request = |id: &str, owner: &str, origin: MediaRequestOrigin, minutes: i64| {
        let mut request = crate::lists::rejection::tests::rejected_request(id, origin);
        request.created_by_user_id = owner.to_string();
        request.created_at = at(minutes);
        request
    };
    let list = MediaRequestOrigin::PublicList {
        subscription_id: "public-a".to_string(),
    };
    let requests = vec![
        request("r1", "member-a", list.clone(), 100),
        request("r2", "member-a", MediaRequestOrigin::Manual, 100),
        request("r3", "member-a", list.clone(), 0),
        request(
            "r4",
            "member-b",
            MediaRequestOrigin::PersonalList {
                subscription_id: "personal-b".to_string(),
            },
            100,
        ),
    ];
    let counts = list_request_counts(&requests, at(50));
    assert_eq!(counts.get("member-a"), Some(&1));
    assert_eq!(counts.get("member-b"), Some(&1));
}

fn chart(chart_key: &str) -> ListChartCatalogEntry {
    ListChartCatalogEntry {
        provider: "emberfall".into(),
        chart_key: chart_key.into(),
        scope: "global".into(),
        label: format!("Emberfall {chart_key}"),
        kinds: vec!["movie".into()],
        refreshed_at: None,
        item_count: 10,
    }
}

fn gateway_failure() -> AppError {
    AppError::Repository("fixture gateway failure".into())
}

#[tokio::test(start_paused = true)]
async fn chart_catalog_is_served_from_the_cache_for_ten_minutes() {
    let cache = ListChartCatalogCache::default();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let fetch = |charts: Vec<ListChartCatalogEntry>| {
        let calls = &calls;
        async move {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(charts)
        }
    };

    assert_eq!(
        cache.get(fetch(vec![chart("trending")])).await.unwrap(),
        vec![chart("trending")]
    );
    tokio::time::advance(LIST_CHART_CATALOG_TTL - std::time::Duration::from_secs(1)).await;
    // Still fresh: the gateway's newer answer is not asked for.
    let served = cache.get(fetch(vec![chart("newer")])).await.unwrap();
    assert_eq!(served, vec![chart("trending")]);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    assert_eq!(
        cache.get(fetch(vec![chart("popular")])).await.unwrap(),
        vec![chart("popular")]
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn a_failed_chart_catalog_fetch_serves_the_last_good_answer_and_is_not_cached() {
    let cache = ListChartCatalogCache::default();
    // With nothing cached, the failure is the answer, and nothing is stored.
    assert!(cache.get(async { Err(gateway_failure()) }).await.is_err());
    // Within the hold the gateway is not asked again; after it, it is.
    assert!(cache.get(async { Ok(vec![chart("held")]) }).await.is_err());
    tokio::time::advance(LIST_CHART_CATALOG_FAILURE_HOLD).await;
    assert_eq!(
        cache
            .get(async { Ok(vec![chart("trending")]) })
            .await
            .unwrap(),
        vec![chart("trending")]
    );

    tokio::time::advance(LIST_CHART_CATALOG_TTL).await;
    assert_eq!(
        cache.get(async { Err(gateway_failure()) }).await.unwrap(),
        vec![chart("trending")],
        "the last good catalog is served when the gateway fails"
    );
    // Within the hold the last good catalog is served without a fetch.
    assert_eq!(
        cache.get(async { Ok(vec![chart("held")]) }).await.unwrap(),
        vec![chart("trending")]
    );
    // The failure did not refresh the cache: after the hold the next read
    // asks again.
    tokio::time::advance(LIST_CHART_CATALOG_FAILURE_HOLD).await;
    assert_eq!(
        cache
            .get(async { Ok(vec![chart("popular")]) })
            .await
            .unwrap(),
        vec![chart("popular")]
    );
}

#[tokio::test(start_paused = true)]
async fn concurrent_chart_catalog_readers_share_one_fetch() {
    let cache = std::sync::Arc::new(ListChartCatalogCache::default());
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let first = {
        let cache = cache.clone();
        let calls = calls.clone();
        tokio::spawn(async move {
            cache
                .get(async move {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    released.await.expect("release");
                    Ok(vec![chart("trending")])
                })
                .await
        })
    };
    // Wait until the first reader is inside its fetch.
    while calls.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let second = {
        let cache = cache.clone();
        let calls = calls.clone();
        tokio::spawn(async move {
            cache
                .get(async move {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(vec![chart("popular")])
                })
                .await
        })
    };
    // Let the second reader reach the cache while the first still fetches.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    release.send(()).expect("first reader waits");
    assert_eq!(first.await.unwrap().unwrap(), vec![chart("trending")]);
    assert_eq!(second.await.unwrap().unwrap(), vec![chart("trending")]);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn readers_queued_behind_a_failing_chart_catalog_fetch_do_not_fetch_again() {
    for seeded in [false, true] {
        let cache = std::sync::Arc::new(ListChartCatalogCache::default());
        if seeded {
            cache
                .get(async { Ok(vec![chart("trending")]) })
                .await
                .unwrap();
            tokio::time::advance(LIST_CHART_CATALOG_TTL).await;
        }
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let first = {
            let cache = cache.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                cache
                    .get(async move {
                        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        released.await.expect("release");
                        Err(gateway_failure())
                    })
                    .await
            })
        };
        while calls.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let waiters = (0..5)
            .map(|_| {
                let cache = cache.clone();
                let calls = calls.clone();
                tokio::spawn(async move {
                    cache
                        .get(async move {
                            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            Err(gateway_failure())
                        })
                        .await
                })
            })
            .collect::<Vec<_>>();
        release.send(()).expect("first reader waits");
        let mut answers = vec![first.await.unwrap()];
        for waiter in waiters {
            answers.push(waiter.await.unwrap());
        }
        for answer in answers {
            if seeded {
                assert_eq!(answer.unwrap(), vec![chart("trending")]);
            } else {
                assert!(matches!(answer, Err(AppError::Repository(_))));
            }
        }
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "seeded={seeded}"
        );
    }
}
