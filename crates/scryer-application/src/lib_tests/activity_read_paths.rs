//! Read-path equivalence for title history and the activity feed: the narrowed
//! reads must return exactly what the full walk returns.

use super::*;

use crate::domain_events::{
    new_global_domain_event, new_title_domain_event, title_context_snapshot,
};
use scryer_domain::{
    AppPermissionMask, ImportCompletedEventData, ImportRejectedEventData, ImportStatus,
    LibraryPermissionMask, MediaFileAnalyzedEventData, MediaPathUpdate,
    MediaRequestSubmittedEventData, MediaUpdateType,
};

struct HistoryFixture {
    app: AppUseCase,
    admin: User,
    emberfall: Title,
    cinderwake: Title,
    ashgrove: Title,
    domain_events: Arc<MockDomainEventRepo>,
}

async fn seed_title(app: &AppUseCase, id: &str, name: &str, facet: MediaFacet, tvdb: i64) -> Title {
    let mut title = make_due_hydration_title(id, facet, tvdb);
    title.name = name.to_string();
    app.services
        .catalog
        .titles
        .create(title)
        .await
        .expect("create title")
}

fn analyzed(title: &Title, n: usize, episode_ids: Vec<String>) -> DomainEventPayload {
    DomainEventPayload::MediaFileAnalyzed(MediaFileAnalyzedEventData {
        title: title_context_snapshot(title),
        media_updates: vec![MediaPathUpdate {
            path: format!("/library/{}/file-{n}.mkv", title.name),
            update_type: MediaUpdateType::Modified,
        }],
        file_id: format!("file-{n}"),
        analysis_status: "scanned".to_string(),
        episode_ids,
    })
}

fn requested(title: &Title, n: usize, library_id: &str, tvdb: i64) -> DomainEventPayload {
    DomainEventPayload::MediaRequestSubmitted(MediaRequestSubmittedEventData {
        request_id: format!("request-{n}"),
        library_id: library_id.to_string(),
        facet: title.facet.clone(),
        title_name: title.name.clone(),
        external_ids: vec![ExternalId::new("TVDB".to_string(), tvdb.to_string())],
        poster_url: None,
        year: Some(2026),
        requested_quality_profile_id: None,
        requested_quality_profile_name: None,
        requested_monitor_type: None,
        requested_lease_days: None,
        rule_notifications: Vec::new(),
    })
}

/// More events than one 500-row batch, interleaving Emberfall's own events,
/// request submissions that do and do not match it, and another title's
/// events the narrowed read has to skip.
async fn seed_history() -> HistoryFixture {
    let harness = bootstrap_media_request_app();
    let (app, admin, domain_events) = (harness.app, harness.manager, harness.domain_events);
    let emberfall = seed_title(
        &app,
        "title-emberfall",
        "Emberfall",
        MediaFacet::Movie,
        9001,
    )
    .await;
    let cinderwake = seed_title(
        &app,
        "title-cinderwake",
        "Cinderwake",
        MediaFacet::Movie,
        9002,
    )
    .await;
    let ashgrove = seed_title(&app, "title-ashgrove", "Ashgrove", MediaFacet::Movie, 9003).await;
    let movie_library = emberfall.library_id.clone();

    for n in 0..1_230 {
        let event = match n % 6 {
            0 => new_global_domain_event(&admin, requested(&emberfall, n, &movie_library, 9001)),
            1 => new_title_domain_event(
                &admin,
                &emberfall,
                analyzed(
                    &emberfall,
                    n,
                    vec![
                        format!("ep-{n}-a"),
                        format!("ep-{n}-b"),
                        format!("ep-{n}-c"),
                    ],
                ),
            ),
            2 => new_title_domain_event(&admin, &cinderwake, analyzed(&cinderwake, n, vec![])),
            3 => new_title_domain_event(&admin, &emberfall, analyzed(&emberfall, n, vec![])),
            // Same external id, another library: never Emberfall's request.
            4 => new_global_domain_event(&admin, requested(&emberfall, n, "library-other", 9001)),
            _ => new_global_domain_event(&admin, requested(&cinderwake, n, &movie_library, 9002)),
        };
        app.append_domain_event(event)
            .await
            .expect("append history event");
    }

    HistoryFixture {
        app,
        admin,
        emberfall,
        cinderwake,
        ashgrove,
        domain_events,
    }
}

fn history_filter(title_ids: Vec<String>, limit: usize, offset: usize) -> TitleHistoryFilter {
    TitleHistoryFilter {
        event_types: Some(crate::supported_title_history_event_types().to_vec()),
        title_ids: Some(title_ids),
        limit,
        offset,
        ..TitleHistoryFilter::default()
    }
}

/// Every record a filter matches, read page by page until a page comes back
/// empty. It never consults `has_more`, so it is the reference for it.
async fn whole_history(app: &AppUseCase, actor: &User, base: &TitleHistoryFilter) -> Vec<String> {
    let mut all = Vec::new();
    loop {
        let page = app
            .list_title_history(
                actor,
                &TitleHistoryFilter {
                    limit: 500,
                    offset: all.len(),
                    ..base.clone()
                },
            )
            .await
            .expect("history page");
        if page.records.is_empty() {
            return all;
        }
        all.extend(page_shape(&page));
    }
}

fn page_shape(page: &TitleHistoryPage) -> Vec<String> {
    page.records
        .iter()
        .map(|record| format!("{record:?}"))
        .collect()
}

#[tokio::test]
async fn single_title_request_history_merge_matches_the_full_walk() {
    let fixture = seed_history().await;
    let app = &fixture.app;
    let emberfall = fixture.emberfall.id.clone();
    // Ashgrove has no events and no matching request, so naming it changes no
    // record but keeps the read on the single full walk.
    let oracle_ids = vec![emberfall.clone(), fixture.ashgrove.id.clone()];

    let full = app
        .list_title_history(
            &fixture.admin,
            &history_filter(oracle_ids.clone(), 100_000, 0),
        )
        .await
        .expect("full walk");
    let total = full.records.len();
    assert!(total > 500);
    assert!(!full.has_more);
    assert!(
        full.records
            .iter()
            .any(|record| record.event_type == TitleHistoryEventType::Requested)
    );

    for (limit, offset) in [
        (1, 0),
        (3, 0),
        (3, 1),
        (50, 2),
        (500, 0),
        (501, 499),
        (50, total - 1),
        (50, total),
        (50, total + 7),
        (total - 1, 0),
        (total, 0),
        (total + 1, 0),
    ] {
        let oracle = app
            .list_title_history(
                &fixture.admin,
                &history_filter(oracle_ids.clone(), limit, offset),
            )
            .await
            .expect("oracle page");
        assert_eq!(
            oracle.has_more,
            offset + oracle.records.len() < total,
            "{limit}/{offset}"
        );
        fixture.domain_events.list_filters.lock().await.clear();
        let merged = app
            .list_title_history(
                &fixture.admin,
                &history_filter(vec![emberfall.clone()], limit, offset),
            )
            .await
            .expect("merged page");
        assert_eq!(page_shape(&merged), page_shape(&oracle), "{limit}/{offset}");
        assert_eq!(merged.has_more, oracle.has_more, "{limit}/{offset}");
        // The merged read never walks every title's history, and reads only
        // the submissions of the requests that share an id with the title.
        assert!(
            fixture
                .domain_events
                .list_filters
                .lock()
                .await
                .iter()
                .all(|filter| filter.title_id.is_some()
                    || (filter.event_types.as_deref()
                        == Some(&[DomainEventType::MediaRequestSubmitted][..])
                        && filter.media_request_ids.as_ref().is_some_and(|keys| {
                            keys.and_removed
                                && keys.request_ids.len() == 205
                                && keys.request_ids.contains(&"request-0".to_string())
                        })))
        );
    }

    // A title with no history: an empty page with nothing after it.
    let empty = history_filter(vec![fixture.ashgrove.id.clone()], 10, 0);
    let page = app
        .list_title_history(&fixture.admin, &empty)
        .await
        .expect("empty page");
    assert!(page.records.is_empty());
    assert!(!page.has_more);
}

#[tokio::test]
async fn has_more_is_exact_at_every_page_boundary_for_every_path() {
    let fixture = seed_history().await;
    let app = &fixture.app;
    let filters = [
        // Grouped page read.
        TitleHistoryFilter {
            group_by_event: true,
            ..TitleHistoryFilter::default()
        },
        // Episode-split walk, no request history.
        TitleHistoryFilter::default(),
        // Two titles, episode-split walk.
        TitleHistoryFilter {
            title_ids: Some(vec![
                fixture.emberfall.id.clone(),
                fixture.cinderwake.id.clone(),
            ]),
            ..TitleHistoryFilter::default()
        },
        // One title, request history on (merged read).
        TitleHistoryFilter {
            title_ids: Some(vec![fixture.emberfall.id.clone()]),
            ..TitleHistoryFilter::default()
        },
    ];
    for base in filters {
        let everything = whole_history(app, &fixture.admin, &base).await;
        let total = everything.len();
        assert!(total > 500, "{total}");
        // One grouped read returns at most 500 rows whatever limit it is given.
        let page_cap = if base.group_by_event { 500 } else { usize::MAX };
        for (limit, offset) in [
            (1, 0),
            (2, 1),
            (499, 0),
            (500, 0),
            (501, 0),
            // A full page that ends exactly on the last record.
            (25, total - 25),
            (499, total - 499),
            (500, total - 500),
            // The same pages with exactly one record after them.
            (25, total - 26),
            (499, total - 500),
            (500, total - 501),
            (25, total - 1),
            (25, total),
            (25, total + 3),
            (total, 0),
            (total - 1, 0),
            (total + 1, 0),
        ] {
            let page = app
                .list_title_history(
                    &fixture.admin,
                    &TitleHistoryFilter {
                        limit,
                        offset,
                        ..base.clone()
                    },
                )
                .await
                .expect("page");
            let expected = everything
                .iter()
                .skip(offset)
                .take(limit.min(page_cap))
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(
                page.records.len(),
                expected.len(),
                "{limit}/{offset} grouped={} titles={:?}",
                base.group_by_event,
                base.title_ids
            );
            assert_eq!(page_shape(&page), expected, "{limit}/{offset}");
            assert_eq!(
                page.has_more,
                offset + expected.len() < total,
                "{limit}/{offset} grouped={}",
                base.group_by_event
            );
        }
    }
}

#[tokio::test]
async fn chosen_titles_in_a_library_scope_keep_only_titles_in_scope() {
    let fixture = seed_history().await;
    let app = &fixture.app;
    let series = seed_title(
        app,
        "title-ashgrove-series",
        "Ashgrove",
        MediaFacet::Series,
        9101,
    )
    .await;
    app.append_domain_event(new_title_domain_event(
        &fixture.admin,
        &series,
        analyzed(&series, 9_999, vec![]),
    ))
    .await
    .expect("append series event");

    let mut grants = std::collections::BTreeMap::new();
    let mut mask = LibraryPermissionMask::VIEW;
    mask.insert(LibraryPermissionMask::MANAGE_TITLES);
    grants.insert(fixture.emberfall.library_id.clone(), mask);
    let mut movie_manager =
        test_user_with_app_permissions("movie-manager", AppPermissionMask::NONE);
    movie_manager.authorization.libraries = grants
        .into_iter()
        .map(|(library_id, mask)| (library_id, mask.normalized_for_storage()))
        .collect();

    let scoped = |title_ids: Vec<String>| TitleHistoryFilter {
        title_ids: Some(title_ids),
        limit: 100_000,
        ..TitleHistoryFilter::default()
    };
    let mixed = app
        .list_title_history(
            &movie_manager,
            &scoped(vec![
                series.id.clone(),
                fixture.cinderwake.id.clone(),
                "title-missing".to_string(),
                fixture.emberfall.id.clone(),
            ]),
        )
        .await
        .expect("mixed scope");
    let in_scope = app
        .list_title_history(
            &movie_manager,
            &scoped(vec![
                fixture.cinderwake.id.clone(),
                fixture.emberfall.id.clone(),
            ]),
        )
        .await
        .expect("in-scope titles");
    assert_eq!(page_shape(&mixed), page_shape(&in_scope));
    assert_eq!(mixed.has_more, in_scope.has_more);
    assert!(
        mixed
            .records
            .iter()
            .all(|record| record.title_id.as_deref() != Some(series.id.as_str()))
    );

    let none_in_scope = app
        .list_title_history(&movie_manager, &scoped(vec![series.id.clone()]))
        .await
        .expect("out-of-scope title");
    assert!(none_in_scope.records.is_empty());
    assert!(!none_in_scope.has_more);
}

#[tokio::test]
async fn activity_visibility_with_prefetched_libraries_follows_each_title() {
    let fixture = seed_history().await;
    let app = &fixture.app;
    let series = seed_title(
        app,
        "title-ashgrove-series",
        "Ashgrove",
        MediaFacet::Series,
        9101,
    )
    .await;
    let mut deleted = fixture.ashgrove.clone();
    deleted.id = "title-deleted".to_string();
    for title in [&series, &deleted, &fixture.emberfall] {
        app.append_domain_event(new_title_domain_event(
            &fixture.admin,
            title,
            analyzed(title, 7_777, vec![]),
        ))
        .await
        .expect("append activity event");
    }

    let mut mask = LibraryPermissionMask::VIEW;
    mask.insert(LibraryPermissionMask::MANAGE_TITLES);
    let mut movie_manager =
        test_user_with_app_permissions("movie-manager", AppPermissionMask::NONE);
    movie_manager.authorization.libraries = [(
        fixture.emberfall.library_id.clone(),
        mask.normalized_for_storage(),
    )]
    .into_iter()
    .collect();

    let movie_titles = [
        fixture.emberfall.id.as_str(),
        fixture.cinderwake.id.as_str(),
    ];
    let events = app
        .list_domain_events(
            &movie_manager,
            &DomainEventFilter {
                limit: 200,
                ..DomainEventFilter::default()
            },
        )
        .await
        .expect("activity events");
    assert!(!events.is_empty());
    assert!(events.iter().all(|event| {
        event
            .title_id
            .as_deref()
            .is_none_or(|title_id| movie_titles.contains(&title_id))
    }));
    assert!(
        events
            .iter()
            .any(|event| event.title_id.as_deref() == Some(fixture.emberfall.id.as_str()))
    );

    let after = app
        .list_activity_events_after_sequence(&movie_manager, 0, 500)
        .await
        .expect("activity after sequence");
    assert!(!after.is_empty());
    let all_events = fixture.domain_events.events.lock().await.clone();
    for (sequence, _) in &after {
        let event = all_events
            .iter()
            .find(|event| event.sequence == *sequence)
            .expect("listed event exists");
        assert!(
            event
                .title_id
                .as_deref()
                .is_none_or(|title_id| movie_titles.contains(&title_id))
        );
    }
}

#[tokio::test]
async fn an_import_resolver_enters_the_feed_but_sees_no_other_library() {
    let fixture = seed_history().await;
    let app = &fixture.app;
    let series = seed_title(
        app,
        "title-ashgrove-series",
        "Ashgrove",
        MediaFacet::Series,
        9101,
    )
    .await;
    for title in [&series, &fixture.emberfall] {
        app.append_domain_event(new_title_domain_event(
            &fixture.admin,
            title,
            analyzed(title, 7_777, vec![]),
        ))
        .await
        .expect("append activity event");
    }

    let mut resolver = test_user_with_app_permissions("import-resolver", AppPermissionMask::NONE);
    resolver.authorization.libraries = [(
        series.library_id.clone(),
        LibraryPermissionMask::RESOLVE_IMPORTS,
    )]
    .into_iter()
    .collect();

    app.latest_domain_event_sequence(&resolver)
        .await
        .expect("resolver reads the feed tail");
    app.subscribe_activity_events(&resolver)
        .await
        .expect("resolver subscribes to activity");
    app.subscribe_domain_event_sequences(&resolver)
        .await
        .expect("resolver subscribes to feed sequences");
    app.subscribe_import_history(&resolver)
        .await
        .expect("resolver subscribes to import history");

    let movie_titles = [
        fixture.emberfall.id.as_str(),
        fixture.cinderwake.id.as_str(),
    ];
    let recent = app
        .recent_activity(&resolver, 200, 0)
        .await
        .expect("resolver reads recent activity");
    assert!(recent.iter().all(|event| {
        event
            .title_id
            .as_deref()
            .is_none_or(|title_id| !movie_titles.contains(&title_id))
    }));
    let after = app
        .list_activity_events_after_sequence(&resolver, 0, 500)
        .await
        .expect("resolver reads activity after a sequence");
    let all_events = fixture.domain_events.events.lock().await.clone();
    for (sequence, _) in &after {
        let event = all_events
            .iter()
            .find(|event| event.sequence == *sequence)
            .expect("listed event exists");
        assert!(
            event
                .title_id
                .as_deref()
                .is_none_or(|title_id| !movie_titles.contains(&title_id))
        );
    }

    // A member with no library grant at all is still refused.
    let outsider = test_user_with_app_permissions("no-grant-member", AppPermissionMask::NONE);
    let refused = app
        .recent_activity(&outsider, 10, 0)
        .await
        .expect_err("a member without a grant is refused");
    assert_eq!(
        refused.reason(),
        Some(crate::error_reasons::events::LIBRARY_ACCESS_DENIED)
    );
}

fn import_completed(title: &Title, n: usize) -> DomainEventPayload {
    DomainEventPayload::ImportCompleted(ImportCompletedEventData {
        title: title_context_snapshot(title),
        media_updates: Vec::new(),
        imported_count: 1,
        import_id: Some(format!("import-{n}")),
        source_system: None,
        source_ref: None,
        source_title: Some(format!("Synthetic.Release.{n}")),
        source_path: Some(format!("/downloads/synthetic-{n}.mkv")),
        dest_path: Some(format!("/library/{}/file-{n}.mkv", title.name)),
        quality: None,
        episode_ids: Vec::new(),
        size_bytes: None,
        upgrade: false,
        file_placements: Vec::new(),
    })
}

fn import_rejected(title: &Title, n: usize) -> DomainEventPayload {
    DomainEventPayload::ImportRejected(ImportRejectedEventData {
        title: Some(title_context_snapshot(title)),
        status: ImportStatus::Failed,
        import_id: Some(format!("import-{n}")),
        source_system: None,
        source_ref: None,
        source_title: Some(format!("Synthetic.Release.{n}")),
        source_path: Some(format!("/downloads/synthetic-{n}.mkv")),
        dest_path: None,
        quality: None,
        reason: Some("synthetic refusal".to_string()),
        reason_code: None,
        skip_reason: None,
        episode_ids: Vec::new(),
    })
}

#[tokio::test]
async fn an_import_resolver_reads_only_their_librarys_import_events() {
    let harness = bootstrap_media_request_app();
    let (app, admin) = (harness.app, harness.manager);
    let series = seed_title(&app, "title-larkspur", "Larkspur", MediaFacet::Series, 9201).await;
    let movie = seed_title(
        &app,
        "title-quillmoor",
        "Quillmoor",
        MediaFacet::Movie,
        9202,
    )
    .await;
    assert_ne!(series.library_id, movie.library_id);

    let append = |title: &Title, payload| {
        let event = new_title_domain_event(&admin, title, payload);
        let app = app.clone();
        async move { app.append_domain_event(event).await.expect("append event") }
    };
    let own_completed = append(&series, import_completed(&series, 1)).await;
    let own_rejected = append(&series, import_rejected(&series, 2)).await;
    let own_analyzed = append(&series, analyzed(&series, 3, vec![])).await;
    let other_completed = append(&movie, import_completed(&movie, 4)).await;
    let own_request = app
        .append_domain_event(new_global_domain_event(
            &admin,
            requested(&series, 5, &series.library_id, 9201),
        ))
        .await
        .expect("append request");

    let member = |name: &str, mask: LibraryPermissionMask| {
        let mut user = test_user_with_app_permissions(name, AppPermissionMask::NONE);
        user.authorization.libraries = [(series.library_id.clone(), mask)].into_iter().collect();
        user
    };
    let resolver = member("import-resolver", LibraryPermissionMask::RESOLVE_IMPORTS);
    let viewer = member("library-viewer", LibraryPermissionMask::VIEW);
    let seeded = [
        own_completed.sequence,
        own_rejected.sequence,
        own_analyzed.sequence,
        other_completed.sequence,
        own_request.sequence,
    ];
    let feed = |actor: User| {
        let app = app.clone();
        async move {
            let mut sequences = app
                .list_domain_events(
                    &actor,
                    &DomainEventFilter {
                        limit: 500,
                        ..DomainEventFilter::default()
                    },
                )
                .await
                .expect("domain event feed")
                .into_iter()
                .map(|event| event.sequence)
                .filter(|sequence| seeded.contains(sequence))
                .collect::<Vec<_>>();
            sequences.sort_unstable();
            sequences
        }
    };

    assert_eq!(
        feed(resolver.clone()).await,
        vec![own_completed.sequence, own_rejected.sequence],
        "a resolver reads their library's import events and nothing else"
    );
    assert_eq!(
        feed(viewer.clone()).await,
        vec![own_request.sequence],
        "View still reads catalog-level events only"
    );

    // The live stream and the history list agree with the feed.
    let mut streamed = app
        .list_activity_events_after_sequence(&resolver, 0, 500)
        .await
        .expect("resolver reads the live stream")
        .into_iter()
        .map(|(sequence, _)| sequence)
        .filter(|sequence| seeded.contains(sequence))
        .collect::<Vec<_>>();
    streamed.sort_unstable();
    assert!(!streamed.is_empty());
    assert!(
        streamed
            .iter()
            .all(|sequence| [own_completed.sequence, own_rejected.sequence].contains(sequence))
    );
    let history = app
        .recent_events(&resolver, None, 100, 0)
        .await
        .expect("resolver reads recent events");
    assert!(!history.is_empty());
    assert!(
        history
            .iter()
            .all(|event| event.title_id.as_deref() == Some(series.id.as_str()))
    );
}

/// The live feed's budgeted reads, resumed from each read's resume point,
/// deliver exactly the events and order that unbounded reads deliver over a
/// long history the caller mostly cannot see, and no read examines more than
/// the page budget.
#[tokio::test]
async fn budgeted_domain_event_reads_match_unbounded_reads() {
    use crate::DomainEventRepository as _;
    let harness = bootstrap_media_request_app();
    let (app, admin, domain_events) = (harness.app, harness.manager, harness.domain_events);
    let visible_title = seed_title(
        &app,
        "title-emberfall",
        "Emberfall",
        MediaFacet::Movie,
        9001,
    )
    .await;
    let hidden_title = seed_title(
        &app,
        "title-ashgrove-series",
        "Ashgrove",
        MediaFacet::Series,
        9101,
    )
    .await;
    let viewer = manager_of("movie-viewer", &[visible_title.library_id.as_str()]);

    // 45 source pages of mostly hidden events, a few visible ones far apart,
    // then more visible events than one read returns, then a hidden tail.
    let hidden_run = 500 * 45;
    let sparse_visible = [3, 7, 15_000];
    let mut seeded = Vec::new();
    for n in 0..hidden_run + 150 + 10 {
        let visible = sparse_visible.contains(&n) || (hidden_run..hidden_run + 150).contains(&n);
        let title = if visible {
            &visible_title
        } else {
            &hidden_title
        };
        seeded.push(new_title_domain_event(
            &admin,
            title,
            analyzed(title, n, vec![]),
        ));
    }
    domain_events
        .append_many(seeded)
        .await
        .expect("seed domain events");
    let filter = |cursor| DomainEventFilter {
        after_sequence: Some(cursor),
        limit: 100,
        ..DomainEventFilter::default()
    };

    // The feed's reads before the budget: each resumes after its last event.
    let mut unbounded = Vec::new();
    let mut cursor = 0;
    loop {
        let events = app
            .list_domain_events(&viewer, &filter(cursor))
            .await
            .expect("unbounded read");
        let Some(last) = events.last() else {
            break;
        };
        cursor = last.sequence;
        unbounded.extend(events);
    }

    let mut budgeted = Vec::new();
    let mut cursor = 0;
    let mut stopped_at_budget = 0;
    loop {
        let calls_before = domain_events
            .list_calls
            .load(std::sync::atomic::Ordering::SeqCst);
        let page = app
            .list_domain_events_within_budget(&viewer, &filter(cursor))
            .await
            .expect("budgeted read");
        let source_pages = domain_events
            .list_calls
            .load(std::sync::atomic::Ordering::SeqCst)
            - calls_before;
        assert!(
            source_pages <= crate::DOMAIN_EVENT_FEED_SOURCE_PAGE_BUDGET,
            "one read examined {source_pages} source pages"
        );
        if page.events.is_empty() && page.resume_after_sequence.is_none() {
            break;
        }
        if let Some(last) = page.events.last() {
            cursor = last.sequence;
        }
        if let Some(resume_after) = page.resume_after_sequence {
            stopped_at_budget += 1;
            cursor = cursor.max(resume_after);
        }
        budgeted.extend(page.events);
    }

    let sequences = |events: &[DomainEvent]| {
        events
            .iter()
            .map(|event| (event.sequence, event.event_id.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(unbounded.len(), sparse_visible.len() + 150);
    assert_eq!(sequences(&budgeted), sequences(&unbounded));
    assert!(
        stopped_at_budget >= 2,
        "the history is long enough that reads stop at the budget"
    );
}

fn manager_of(name: &str, library_ids: &[&str]) -> User {
    let mut mask = LibraryPermissionMask::VIEW;
    mask.insert(LibraryPermissionMask::MANAGE_TITLES);
    let mut user = test_user_with_app_permissions(name, AppPermissionMask::NONE);
    user.authorization.libraries = library_ids
        .iter()
        .map(|library_id| (library_id.to_string(), mask.normalized_for_storage()))
        .collect();
    user
}

/// The library scope read in the history query returns, for every filter
/// shape, the page the read that first lists every title in scope returns:
/// a library with many titles, a library with exactly one, an empty one, a
/// search, chosen titles, and each request-history rule.
#[tokio::test]
async fn library_scoped_history_matches_the_listed_title_read() {
    let fixture = seed_history().await;
    let app = &fixture.app;
    let movies = fixture.emberfall.library_id.clone();
    let series = seed_title(
        app,
        "title-glimmerfen",
        "Glimmerfen",
        MediaFacet::Series,
        9201,
    )
    .await;
    let series_library = series.library_id.clone();
    let anime_library = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    assert_ne!(series_library, movies);
    for n in 0..40 {
        let event = match n % 3 {
            0 => new_title_domain_event(&fixture.admin, &series, analyzed(&series, n, vec![])),
            1 => new_global_domain_event(
                &fixture.admin,
                requested(&series, 5_000 + n, &series_library, 9201),
            ),
            _ => new_title_domain_event(
                &fixture.admin,
                &series,
                analyzed(&series, n, vec![format!("ep-glimmerfen-{n}")]),
            ),
        };
        app.append_domain_event(event)
            .await
            .expect("append series event");
    }

    let actors = [
        fixture.admin.clone(),
        manager_of("movies-and-series", &[&movies, &series_library]),
        // One title in scope: request history joins when no types are named.
        manager_of("series-only", &[&series_library]),
    ];
    let library_choices = [
        None,
        Some(vec![movies.clone()]),
        Some(vec![series_library.clone()]),
        Some(vec![anime_library.clone()]),
        Some(vec![movies.clone(), series_library.clone()]),
        Some(Vec::new()),
    ];
    let title_choices = [
        (None, None),
        (None, Some("e".to_string())),
        (None, Some("no-such-title".to_string())),
        (
            Some(vec![fixture.emberfall.id.clone(), series.id.clone()]),
            None,
        ),
        (
            Some(vec![fixture.cinderwake.id.clone(), series.id.clone()]),
            Some("w".to_string()),
        ),
    ];
    let without_requested = crate::supported_title_history_event_types()
        .iter()
        .copied()
        .filter(|event_type| *event_type != TitleHistoryEventType::Requested)
        .collect::<Vec<_>>();
    let event_type_choices = [
        None,
        Some(crate::supported_title_history_event_types().to_vec()),
        Some(without_requested),
    ];

    let mut compared = 0usize;
    let mut non_empty = 0usize;
    for (actor_index, actor) in actors.iter().enumerate() {
        for library_ids in &library_choices {
            for (title_ids, title_search) in &title_choices {
                for event_types in &event_type_choices {
                    for group_by_event in [true, false] {
                        for (limit, offset) in [(1, 0), (7, 3), (25, 1_000), (5_000, 0)] {
                            let filter = TitleHistoryFilter {
                                event_types: event_types.clone(),
                                title_ids: title_ids.clone(),
                                library_ids: library_ids.clone(),
                                title_search: title_search.clone(),
                                group_by_event,
                                limit,
                                offset,
                                ..TitleHistoryFilter::default()
                            };
                            let shape = |page: AppResult<TitleHistoryPage>| {
                                page.map(|page| (page_shape(&page), page.has_more))
                                    .map_err(|error| format!("{error:?}"))
                            };
                            let scoped = shape(app.list_title_history(actor, &filter).await);
                            let listed =
                                shape(app.list_title_history_with_title_ids(actor, &filter).await);
                            assert_eq!(scoped, listed, "actor {actor_index}: {filter:?}");
                            compared += 1;
                            if scoped.is_ok_and(|(records, _)| !records.is_empty()) {
                                non_empty += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(compared > 1_000, "{compared}");
    assert!(non_empty > compared / 4, "{non_empty} of {compared}");
}
