//! The title-history page read scoped by library against real migrations.
//!
//! A library scope resolves its titles inside the history statement. It must
//! select exactly the rows the same read selects with those titles' ids
//! listed, and the id-only title read that decides between the two must
//! return the titles of a library in list order.

use super::*;
use crate::queries::sql_runtime::{SqlArg, SqlRuntime};
use scryer_application::TitleHistoryTitleScope;
use scryer_domain::{
    MediaFileAnalyzedEventData, MediaPathUpdate, MediaUpdateType, TitleHistoryEventType,
};
use scryer_infrastructure_workflow::workflow::stores::build_title_history_filter_sql;

fn library(facet: MediaFacet) -> String {
    scryer_domain::default_library_id_for_facet(&facet)
}

fn scoped_title(id: &str, name: &str, facet: MediaFacet) -> Title {
    let mut title = make_test_title(id, None);
    title.name = name.to_string();
    title.library_id = library(facet.clone());
    title.facet = facet;
    title
}

fn analyzed_event(title_id: Option<&str>, n: usize) -> NewDomainEvent {
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_kind: scryer_domain::DomainEventActorKind::System,
        actor_user_id: None,
        actor_display_name: "System".to_string(),
        title_id: title_id.map(str::to_string),
        facet: Some(MediaFacet::Movie),
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: match title_id {
            Some(title_id) => DomainEventStream::Title {
                title_id: title_id.to_string(),
            },
            None => DomainEventStream::Global,
        },
        payload: DomainEventPayload::MediaFileAnalyzed(MediaFileAnalyzedEventData {
            title: TitleContextSnapshot {
                title_name: title_id.unwrap_or("unlinked").to_string(),
                facet: MediaFacet::Movie,
                external_ids: Default::default(),
                poster_url: None,
                year: None,
            },
            media_updates: vec![MediaPathUpdate {
                path: format!("/library/file-{n}.mkv"),
                update_type: MediaUpdateType::Modified,
            }],
            file_id: format!("file-{n}"),
            analysis_status: "scanned".to_string(),
            episode_ids: vec![],
        }),
    }
}

fn event_ids(events: &[scryer_domain::DomainEvent]) -> Vec<String> {
    events.iter().map(|event| event.event_id.clone()).collect()
}

#[tokio::test]
async fn library_scope_selects_the_rows_of_its_listed_titles() {
    let (services, _db) = temp_services("scryer_title_history_library_scope").await;
    let catalog = title_store(&services);
    let domain_events = DomainEventStore::new(services.datastore());

    let titles = [
        scoped_title("title-quillmere", "quillmere", MediaFacet::Movie),
        scoped_title("title-bramblecourt", "Bramblecourt", MediaFacet::Movie),
        scoped_title("title-ashveil", "ashveil", MediaFacet::Movie),
        scoped_title("title-ashveil-b", "Ashveil", MediaFacet::Movie),
        scoped_title("title-lanternfold", "Lanternfold", MediaFacet::Series),
    ];
    for title in &titles {
        TitleRepository::create(&catalog, title.clone())
            .await
            .expect("title should insert");
    }
    let mut events = Vec::new();
    for n in 0..120 {
        let title_id = match n % 7 {
            // Rows with no catalog title, and rows of a title that is not in
            // the catalog at all.
            0 => None,
            1 => Some("title-not-in-catalog"),
            other => Some(titles[other % titles.len()].id.as_str()),
        };
        events.push(analyzed_event(title_id, n));
    }
    domain_events
        .append_many(events)
        .await
        .expect("events should append");

    let movie = library(MediaFacet::Movie);
    let series = library(MediaFacet::Series);
    let anime = library(MediaFacet::Anime);
    let scopes = [
        vec![movie.clone()],
        vec![series.clone()],
        vec![anime.clone()],
        vec![movie.clone(), series.clone(), anime.clone()],
        vec!["library-unknown".to_string()],
        Vec::new(),
    ];

    for library_ids in &scopes {
        let listed = catalog
            .list_title_ids_for_libraries(library_ids, None)
            .await
            .expect("id-only title read");
        let decoded = catalog
            .list_with_projection(
                None,
                Some(library_ids),
                None,
                TitleListProjection::without_canonical_tags(),
            )
            .await
            .expect("full title read")
            .into_iter()
            .map(|title| title.id)
            .collect::<Vec<_>>();
        assert_eq!(listed, decoded, "{library_ids:?}");
        for limit in [0, 1, 2, 3, 100] {
            let limited = catalog
                .list_title_ids_for_libraries(library_ids, Some(limit))
                .await
                .expect("limited id-only title read");
            assert_eq!(
                limited,
                decoded.iter().take(limit).cloned().collect::<Vec<_>>(),
                "{library_ids:?} limit {limit}"
            );
        }

        for event_types in [None, Some(&[TitleHistoryEventType::Scanned][..])] {
            for include_titleless in [false, true] {
                let by_library = Some(TitleHistoryTitleScope::Libraries(library_ids));
                let by_titles = Some(TitleHistoryTitleScope::Titles(&listed));
                let count = |scope| {
                    domain_events.count_title_history_page_events(
                        event_types,
                        scope,
                        include_titleless,
                        None,
                    )
                };
                let library_count = count(by_library).await.expect("library count");
                let title_count = count(by_titles).await.expect("title count");
                assert_eq!(
                    library_count, title_count,
                    "{library_ids:?} {include_titleless}"
                );
                for (limit, offset) in [(1, 0), (10, 5), (500, 0), (10, 1_000)] {
                    let page = |scope| {
                        domain_events.list_title_history_page_events(
                            event_types,
                            scope,
                            include_titleless,
                            None,
                            limit,
                            offset,
                        )
                    };
                    let library_page = page(by_library).await.expect("library page");
                    let title_page = page(by_titles).await.expect("title page");
                    assert_eq!(
                        event_ids(&library_page),
                        event_ids(&title_page),
                        "{library_ids:?} {include_titleless} {limit}/{offset}"
                    );
                }
            }
        }
    }

    // The movie library holds four titles' rows; nothing outside it leaks in.
    let movie_rows = domain_events
        .count_title_history_page_events(
            None,
            Some(TitleHistoryTitleScope::Libraries(&[movie.clone()])),
            false,
            None,
        )
        .await
        .expect("movie count");
    assert!(movie_rows > 0);
}

/// The library-scoped page is read newest first from the domain event
/// indexes, and the scope's titles come from the library index.
#[tokio::test]
async fn library_scoped_page_uses_the_event_and_title_indexes() {
    let (services, _db) = temp_services("scryer_title_history_library_plan").await;
    let library_ids = vec![library(MediaFacet::Movie), library(MediaFacet::Series)];
    for include_titleless in [false, true] {
        let (where_sql, mut args) = build_title_history_filter_sql(
            &services.datastore(),
            None,
            Some(TitleHistoryTitleScope::Libraries(&library_ids)),
            include_titleless,
            None,
        );
        args.push(SqlArg::I64(50));
        args.push(SqlArg::I64(0));
        let sql = format!(
            "EXPLAIN QUERY PLAN SELECT event_id FROM domain_events{where_sql} \
             ORDER BY sequence DESC LIMIT {{}} OFFSET {{}}"
        );
        let datastore = services.datastore();
        let plan = SqlRuntime::fetch_all(datastore.read_exec(), &sql, &args)
            .await
            .expect("query plan")
            .iter()
            .map(|row| row.text("detail").expect("plan detail"))
            .collect::<Vec<_>>();
        assert!(
            plan.iter()
                .any(|detail| detail.contains("idx_titles_library")),
            "{include_titleless}: {plan:?}"
        );
        assert!(
            plan.iter()
                .any(|detail| detail.starts_with("SEARCH domain_events")
                    || detail.contains("idx_domain_events_")),
            "{include_titleless}: {plan:?}"
        );
        assert!(
            !plan
                .iter()
                .any(|detail| detail == "SCAN domain_events" || detail.starts_with("SCAN titles")),
            "{include_titleless}: {plan:?}"
        );
    }
}
