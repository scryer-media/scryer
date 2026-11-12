//! Round-trips for the policy columns 0223 added to `media_requests` and for
//! the three history reads the request-fact builder needs (spec 0003 FR-030,
//! FR-040, FR-050).

use super::media_request_requesters::{
    new_request, request_event, request_store, seed_library, seed_user, user,
};
use super::*;
use scryer_application::{MediaRequestRepository, MediaRequestResolution};
use scryer_domain::{
    MediaRequestResolvedEventData, MediaRequestStatus, MonitorSelection, MonitorSelectionMovie,
};
use scryer_infrastructure_sql::runtime::{SqlArg, SqlRuntime, StoreDatastore};

type RejectionFixture = (&'static str, Option<MonitorSelection>, MediaRequestStatus);

#[tokio::test]
async fn media_request_rejection_keeps_distinct_series_seasons_pending() {
    assert_rejection_scope(
        "series_season_rejection",
        MediaFacet::Series,
        &series_rejection_fixtures(),
    )
    .await;
}

fn series_rejection_fixtures() -> [RejectionFixture; 5] {
    let seasons = |seasons| {
        Some(MonitorSelection {
            seasons,
            series_movies: Vec::new(),
        })
    };
    [
        ("season-one", seasons(vec![1]), MediaRequestStatus::Rejected),
        (
            "season-one-duplicate",
            seasons(vec![1, 1]),
            MediaRequestStatus::Rejected,
        ),
        ("season-five", seasons(vec![5]), MediaRequestStatus::Pending),
        (
            "seasons-one-and-five",
            seasons(vec![5, 1]),
            MediaRequestStatus::Pending,
        ),
        ("whole-series", None, MediaRequestStatus::Pending),
    ]
}

#[tokio::test]
async fn media_request_rejection_keeps_distinct_anime_series_movies_pending() {
    assert_rejection_scope(
        "anime_series_movie_rejection",
        MediaFacet::Anime,
        &anime_rejection_fixtures(),
    )
    .await;
}

fn anime_rejection_fixtures() -> [RejectionFixture; 4] {
    let movie = |name: &str, id: &str| {
        Some(MonitorSelection {
            seasons: vec![1],
            series_movies: vec![MonitorSelectionMovie {
                name: name.into(),
                external_ids: vec![ExternalId::new("tmdb", id)],
            }],
        })
    };
    [
        (
            "first-movie",
            movie("First movie", "802401"),
            MediaRequestStatus::Rejected,
        ),
        (
            "same-movie-renamed",
            movie("First movie after refresh", "802401"),
            MediaRequestStatus::Rejected,
        ),
        (
            "second-movie",
            movie("Second movie", "802402"),
            MediaRequestStatus::Pending,
        ),
        (
            "season-only",
            Some(MonitorSelection {
                seasons: vec![1],
                series_movies: Vec::new(),
            }),
            MediaRequestStatus::Pending,
        ),
    ]
}

#[tokio::test]
async fn media_request_rejection_keeps_scoped_requests_pending_when_dismissing_whole_series() {
    assert_rejection_scope(
        "whole_series_rejection",
        MediaFacet::Series,
        &[
            ("whole-series", None, MediaRequestStatus::Rejected),
            ("whole-series-duplicate", None, MediaRequestStatus::Rejected),
            (
                "season-five",
                Some(MonitorSelection {
                    seasons: vec![5],
                    series_movies: Vec::new(),
                }),
                MediaRequestStatus::Pending,
            ),
        ],
    )
    .await;
}

#[tokio::test]
async fn media_request_rejection_still_resolves_duplicate_movie_requests() {
    assert_rejection_scope(
        "movie_duplicate_rejection",
        MediaFacet::Movie,
        &[
            ("movie", None, MediaRequestStatus::Rejected),
            ("movie-duplicate", None, MediaRequestStatus::Rejected),
        ],
    )
    .await;
}

async fn assert_rejection_scope(db_prefix: &str, facet: MediaFacet, fixtures: &[RejectionFixture]) {
    let (services, db) = temp_services(db_prefix).await;
    let result = assert_rejection_scope_in_store(services.datastore(), facet, fixtures).await;
    services.pool().close().await;
    let _ = std::fs::remove_file(db);
    result.unwrap();
}

#[tokio::test]
async fn media_request_rejection_keeps_distinct_scopes_pending_on_postgres() -> AppResult<()> {
    let Ok(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL rejection test; SCRYER_TEST_POSTGRES_URL is not set");
        return Ok(());
    };
    if raw_url.trim().is_empty() {
        return Ok(());
    }

    let admin_pool = sqlx::PgPool::connect(raw_url.trim())
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!("scryer_test_{}", Id::new().0.replace('-', "_"));
    let mut url = url::Url::parse(raw_url.trim())
        .map_err(|error| AppError::Validation(format!("invalid postgres test URL: {error}")))?;
    url.query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema}"));
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| AppError::Repository(format!("failed to create schema: {error}")))?;

    let result = async {
        let services =
            crate::PostgresServices::new_with_mode(url.to_string(), crate::MigrationMode::Apply)
                .await?;
        let result = async {
            assert_rejection_scope_in_store(
                services.datastore(),
                MediaFacet::Series,
                &series_rejection_fixtures(),
            )
            .await?;
            assert_rejection_scope_in_store(
                services.datastore(),
                MediaFacet::Anime,
                &anime_rejection_fixtures(),
            )
            .await
        }
        .await;
        services.pool().close().await;
        result
    }
    .await;

    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await;
    admin_pool.close().await;
    cleanup.map_err(|error| AppError::Repository(format!("failed to drop schema: {error}")))?;
    result
}

async fn assert_rejection_scope_in_store(
    datastore: StoreDatastore,
    facet: MediaFacet,
    fixtures: &[RejectionFixture],
) -> AppResult<()> {
    let library = format!("rejection-{}", facet.as_str());
    SqlRuntime::execute_write(
        &datastore,
        "seed_rejection_library",
        "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {})",
        vec![
            SqlArg::Text(library.clone()),
            SqlArg::Text(facet.as_str().into()),
            SqlArg::Text(library.clone()),
            SqlArg::Text(library.clone()),
            SqlArg::Bool(false),
            SqlArg::Timestamp(Utc::now()),
            SqlArg::Timestamp(Utc::now()),
        ],
    )
    .await?;
    let users = UserStore::new(datastore.clone());
    let store = crate::MediaRequestStore::new(datastore.clone());
    let resolver = format!("{library}-resolver");
    users.create(user(&resolver)).await?;
    for (index, (id, selection, _)) in fixtures.iter().enumerate() {
        let requester = user(&format!("{library}-requester-{index}"));
        users.create(requester.clone()).await?;
        let mut request = new_request(id, &library, &requester.id);
        request.facet = facet.clone();
        request.external_ids = vec![ExternalId::new("tvdb", "9000001")];
        request.requested_monitor_type = selection.as_ref().map(|_| "advanced".into());
        request.requested_monitor_selection = selection.clone();
        let mut event = request_event(id);
        event.facet = Some(facet.clone());
        if let DomainEventPayload::MediaRequestSubmitted(data) = &mut event.payload {
            data.library_id = library.clone();
            data.facet = facet.clone();
        }
        let stored = store.submit(request, &requester, event).await?.request;
        assert_eq!(stored.id, *id, "expected an independent pending request");
    }

    let target = store.get(fixtures[0].0).await?.unwrap();
    let mut event = request_event(&target.id);
    event.facet = Some(facet.clone());
    event.payload = DomainEventPayload::MediaRequestRejected(MediaRequestResolvedEventData {
        request_id: target.id.clone(),
        library_id: target.library_id.clone(),
        facet: facet.clone(),
        title_name: target.title.clone(),
        external_ids: target.external_ids.clone(),
        created_title_id: None,
        requested_quality_profile_id: None,
        requested_quality_profile_name: None,
        requested_monitor_type: target.requested_monitor_type.clone(),
        approved_quality_profile_id: None,
        approved_quality_profile_name: None,
        decided_by_rule_set_ids: Vec::new(),
        decision_reason_codes: Vec::new(),
        approved_lease_days: None,
        policy_tags: Vec::new(),
    });
    let expected_count = fixtures
        .iter()
        .filter(|(_, _, status)| *status == MediaRequestStatus::Rejected)
        .count();
    let resolution = store
        .resolve_pending_overlapping(
            &target,
            MediaRequestResolution {
                status: MediaRequestStatus::Rejected,
                resolved_by_user_id: Some(resolver.clone()),
                resolved_at: Utc::now(),
                created_title_id: None,
                approved_quality_profile_id: None,
                approved_quality_profile_name: None,
                approved_lease_days: None,
                decision_id: None,
                decided_by_rule_set_ids: Vec::new(),
                policy_tags: Vec::new(),
                event,
            },
        )
        .await?;
    for &(id, _, status) in fixtures {
        let request = store.get(id).await?.unwrap();
        assert_eq!(request.status, status, "unexpected state for {id}");
        let rejected = status == MediaRequestStatus::Rejected;
        assert_eq!(request.resolved_at.is_some(), rejected);
        assert_eq!(
            request.resolved_by_user_id.as_deref(),
            rejected.then_some(resolver.as_str())
        );
        assert!(request.created_title_id.is_none());
    }
    assert_eq!(resolution.updated, expected_count as u64);
    let emitted = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT COUNT(*) AS count FROM domain_events
         WHERE event_type = 'media_request_rejected' AND facet = {}",
        &[SqlArg::Text(facet.as_str().into())],
    )
    .await?
    .unwrap()
    .i64("count")?;
    assert_eq!(emitted, expected_count as i64);
    Ok(())
}

#[tokio::test]
async fn list_movie_approval_preserves_parent_and_scopes_request_ownership() {
    use scryer_domain::MediaRequestOrigin;
    for existing in [false, true] {
        let (services, _db) = temp_services("list_movie_approval").await;
        seed_library(&services, "library-1").await;
        sqlx::query("UPDATE libraries SET facet = 'anime' WHERE id = 'library-1'")
            .execute(services.pool())
            .await
            .unwrap();
        seed_user(&services, "requester-1").await;
        let selection = MonitorSelection {
            seasons: vec![],
            series_movies: vec![MonitorSelectionMovie {
                name: "Selected movie".into(),
                external_ids: vec![ExternalId::new("tmdb", "802401")],
            }],
        };
        let mut title = make_test_title("parent-title", None);
        title.library_id = "library-1".into();
        title.facet = MediaFacet::Anime;
        title.external_ids = vec![ExternalId::new("tvdb", "9000001")];
        title.tags = vec!["scryer:monitor-type:advanced".into()];
        let titles = title_store(&services);
        if existing {
            title.monitored = false;
            titles.create(title.clone()).await.unwrap();
        }
        let mut requested = new_request("movie-request", "library-1", "requester-1");
        requested.facet = MediaFacet::Anime;
        requested.origin = MediaRequestOrigin::PublicList {
            subscription_id: "fixture-list".into(),
        };
        requested.identity_fingerprint = "list-movie:fixture-movie".into();
        requested.external_ids = title.external_ids.clone();
        requested.requested_monitor_type = Some("advanced".into());
        requested.requested_monitor_selection = Some(selection.clone());
        let store = request_store(&services);
        let mut sibling = requested.clone();
        sibling.id = "sibling-request".into();
        sibling.identity_fingerprint = "list-movie:sibling".into();
        sibling
            .requested_monitor_selection
            .as_mut()
            .unwrap()
            .series_movies[0]
            .external_ids = vec![ExternalId::new("tmdb", "802402")];
        store
            .submit(
                sibling,
                &user("requester-1"),
                request_event("sibling-request"),
            )
            .await
            .unwrap();
        let request = store
            .submit(
                requested,
                &user("requester-1"),
                request_event("movie-request"),
            )
            .await
            .unwrap()
            .request;
        let before = titles.get_by_id(&title.id).await.unwrap();
        title.monitored = true;
        let (created, resolved, _) = store
            .approve_with_title(
                &request,
                title.clone(),
                scryer_application::TitleOptionsPatch {
                    monitor_selection: Some(Some(selection.clone())),
                    ..Default::default()
                },
                MediaRequestResolution {
                    status: MediaRequestStatus::Approved,
                    resolved_by_user_id: Some("requester-1".into()),
                    resolved_at: Utc::now(),
                    created_title_id: Some(title.id.clone()),
                    approved_quality_profile_id: None,
                    approved_quality_profile_name: None,
                    approved_lease_days: None,
                    decision_id: None,
                    decided_by_rule_set_ids: vec![],
                    policy_tags: vec![],
                    event: request_event("approved-movie"),
                },
                request_event("added-parent"),
            )
            .await
            .unwrap();
        assert_eq!(resolved.updated, 1);
        assert_eq!(created.reused_existing, existing);
        assert_eq!(
            store.get("sibling-request").await.unwrap().unwrap().status,
            MediaRequestStatus::Pending
        );
        if let Some(before) = before {
            assert_eq!(created.title.monitored, before.monitored);
            assert_eq!(created.title.tags, before.tags);
            assert!(
                titles
                    .get_title_monitor_selection(&title.id)
                    .await
                    .unwrap()
                    .is_none()
            );
        } else {
            assert_eq!(
                titles.get_title_monitor_selection(&title.id).await.unwrap(),
                Some(selection)
            );
        }
        let claims: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM lifecycle_claims WHERE title_id = ?")
                .bind(&title.id)
                .fetch_one(services.pool())
                .await
                .unwrap();
        assert_eq!(
            claims, 0,
            "movie approval must not establish parent ownership"
        );
        assert!(
            store
                .requester_user_ids_by_title_ids(&[title.id])
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn list_movie_admission_retains_each_selected_movie_before_hydration() {
    use scryer_domain::{MonitorSelection, MonitorSelectionMovie};
    let (services, _db) = temp_services("list_movie_selections").await;
    seed_library(&services, "library-1").await;
    sqlx::query("UPDATE libraries SET facet = 'anime' WHERE id = 'library-1'")
        .execute(services.pool())
        .await
        .unwrap();
    let titles = title_store(&services);
    let mut parent = make_test_title("container", None);
    parent.library_id = "library-1".into();
    parent.facet = MediaFacet::Anime;
    parent.monitored = true;
    parent.tags = vec!["scryer:monitor-type:advanced".into()];
    parent.external_ids = vec![ExternalId::new("tvdb", "9000001")];
    for (index, id) in ["9000002", "9000003"].into_iter().enumerate() {
        let selection = MonitorSelection {
            seasons: vec![],
            series_movies: vec![MonitorSelectionMovie {
                name: format!("Movie {index}"),
                external_ids: vec![ExternalId::new("tmdb", id)],
            }],
        };
        let outcome = titles
            .create_or_get_existing_preserving_options(parent.clone(), selection)
            .await
            .unwrap();
        assert_eq!(outcome.reused_existing, index > 0);
    }
    let selection = titles
        .get_title_monitor_selection(&parent.id)
        .await
        .unwrap()
        .unwrap();
    assert!(selection.seasons.is_empty());
    assert_eq!(selection.series_movies.len(), 2);
    assert_eq!(
        titles.get_by_id(&parent.id).await.unwrap().unwrap().tags,
        parent.tags
    );
}

#[tokio::test]
async fn approval_rolls_back_title_events_and_claims_together() {
    let (services, _db) = temp_services("atomic_request_approval").await;
    seed_library(&services, "library-1").await;
    seed_user(&services, "requester-1").await;
    let store = request_store(&services);
    let request = store
        .submit(
            new_request("atomic-request", "library-1", "requester-1"),
            &user("requester-1"),
            request_event("atomic-request"),
        )
        .await
        .unwrap()
        .request;
    let mut title = make_test_title("atomic-title", None);
    title.library_id = "library-1".into();
    let resolution = MediaRequestResolution {
        status: MediaRequestStatus::Approved,
        resolved_by_user_id: Some("requester-1".into()),
        resolved_at: Utc::now(),
        created_title_id: Some(title.id.clone()),
        approved_quality_profile_id: None,
        approved_quality_profile_name: None,
        approved_lease_days: Some(14),
        decision_id: None,
        decided_by_rule_set_ids: vec![],
        policy_tags: vec![],
        event: request_event("atomic-approved"),
    };
    let added = request_event("atomic-added");
    sqlx::query(
        "CREATE TRIGGER fail_claim BEFORE INSERT ON lifecycle_claims
        BEGIN SELECT RAISE(ABORT, 'injected claim failure'); END",
    )
    .execute(services.pool())
    .await
    .unwrap();
    assert!(
        store
            .approve_with_title(
                &request,
                title.clone(),
                Default::default(),
                resolution.clone(),
                added.clone()
            )
            .await
            .is_err()
    );
    assert!(
        title_store(&services)
            .get_by_id(&title.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.get(&request.id).await.unwrap().unwrap().status,
        MediaRequestStatus::Pending
    );
    let events: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM domain_events WHERE event_id IN (?, ?)")
            .bind(&added.event_id)
            .bind(&resolution.event.event_id)
            .fetch_one(services.pool())
            .await
            .unwrap();
    assert_eq!(events, 0);
    sqlx::query("DROP TRIGGER fail_claim")
        .execute(services.pool())
        .await
        .unwrap();
    let (created, result, added) = store
        .approve_with_title(
            &request,
            title.clone(),
            Default::default(),
            resolution,
            added,
        )
        .await
        .unwrap();
    assert!(!created.reused_existing);
    assert_eq!(result.updated, 1);
    assert!(added.is_some());
    assert_eq!(
        store.get(&request.id).await.unwrap().unwrap().status,
        MediaRequestStatus::Approved
    );
    let claims: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM lifecycle_claims WHERE title_id = ?")
            .bind(&title.id)
            .fetch_one(services.pool())
            .await
            .unwrap();
    assert_eq!(claims, 1);
}

#[tokio::test]
async fn media_request_policy_columns_round_trip() {
    let (services, db) = temp_services("scryer_media_request_policy").await;
    seed_library(&services, "library-1").await;
    seed_user(&services, "requester-1").await;
    seed_user(&services, "resolver-1").await;
    super::media_request_requesters::seed_title(&services, "title-1").await;
    let store = request_store(&services);

    let mut request = new_request("request-1", "library-1", "requester-1");
    request.requested_lease_days = Some(14);
    request.metadata_snapshot_json =
        r#"{"schema_version":1,"genres":["Drama"],"partial":false}"#.to_string();
    store
        .submit(request, &user("requester-1"), request_event("request-1"))
        .await
        .expect("submit request");

    let stored = store
        .get("request-1")
        .await
        .expect("read request")
        .expect("request exists");
    assert_eq!(stored.requested_lease_days, Some(14));
    assert!(stored.approved_lease_days.is_none());
    assert!(stored.decision_id.is_none());
    assert!(stored.decided_by_rule_set_ids.is_empty());
    assert!(stored.policy_tags.is_empty());
    assert!(
        stored.metadata_snapshot_json.contains("\"Drama\""),
        "the snapshot is stored verbatim: {}",
        stored.metadata_snapshot_json
    );

    // A submit that captured nothing lands as the column's empty-object
    // default rather than as an empty string.
    let mut bare = new_request("request-bare", "library-1", "requester-1");
    bare.metadata_snapshot_json = String::new();
    store
        .submit(bare, &user("requester-1"), request_event("request-bare"))
        .await
        .expect("submit request");
    assert_eq!(
        store
            .get("request-bare")
            .await
            .unwrap()
            .expect("request exists")
            .metadata_snapshot_json,
        "{}"
    );

    store
        .resolve_pending(
            "request-1",
            MediaRequestResolution {
                status: MediaRequestStatus::Approved,
                resolved_by_user_id: Some("resolver-1".to_string()),
                resolved_at: Utc::now(),
                created_title_id: Some("title-1".to_string()),
                approved_quality_profile_id: None,
                approved_quality_profile_name: None,
                approved_lease_days: Some(30),
                decision_id: Some("decision-1".to_string()),
                decided_by_rule_set_ids: vec!["rule-a".to_string(), "rule-b".to_string()],
                policy_tags: vec!["family".to_string()],
                event: request_event("request-1"),
            },
        )
        .await
        .expect("resolve request");

    let resolved = store
        .get("request-1")
        .await
        .unwrap()
        .expect("request exists");
    assert_eq!(resolved.status, MediaRequestStatus::Approved);
    assert_eq!(
        resolved.requested_lease_days,
        Some(14),
        "the requested lease is history and the approval must not overwrite it"
    );
    assert_eq!(resolved.approved_lease_days, Some(30));
    assert_eq!(resolved.decision_id.as_deref(), Some("decision-1"));
    assert_eq!(
        resolved.decided_by_rule_set_ids,
        vec!["rule-a".to_string(), "rule-b".to_string()]
    );
    assert_eq!(resolved.policy_tags, vec!["family".to_string()]);

    // `list` reads the same column set as `get`; a path that missed the new
    // columns would show them empty here.
    let listed = store
        .list(scryer_application::MediaRequestQuery::default())
        .await
        .expect("list requests");
    let listed = listed
        .iter()
        .find(|request| request.id == "request-1")
        .expect("request in listing");
    assert_eq!(listed.approved_lease_days, Some(30));
    assert_eq!(listed.policy_tags, vec!["family".to_string()]);

    let _ = std::fs::remove_file(db);
}

/// The `library_title_count` fact's read (spec 0003 §3.2). The port defaults it
/// over a full listing; the store answers it in SQL, so the two must agree.
#[tokio::test]
async fn counting_titles_in_a_library_answers_the_library_fact() {
    use super::media_request_requesters::seed_title;
    use scryer_application::TitleRepository;

    let (services, db) = temp_services("scryer_library_title_count").await;
    seed_library(&services, "library-1").await;
    let titles = crate::TitleStore::new(services.datastore());

    assert_eq!(
        TitleRepository::count_titles_in_library(&titles, "library-1")
            .await
            .expect("count titles"),
        0,
        "an empty library is a real answer, not an unknown"
    );

    seed_title(&services, "title-1").await;
    seed_title(&services, "title-2").await;
    let library_id = TitleRepository::get_by_id(&titles, "title-1")
        .await
        .expect("read title")
        .expect("title exists")
        .library_id;
    assert_eq!(
        TitleRepository::count_titles_in_library(&titles, &library_id)
            .await
            .expect("count titles"),
        2
    );
    assert_eq!(
        TitleRepository::count_titles_in_library(&titles, "library-nowhere")
            .await
            .expect("count titles"),
        0
    );

    let _ = std::fs::remove_file(db);
}

/// The two writes the evaluation and edit paths need on a request that is still
/// pending: the verdict stamp, and the lease the requester changed their mind
/// about (spec 0003 FR-016, FR-040).
#[tokio::test]
async fn pending_request_writes_carry_the_verdict_and_the_edited_lease() {
    let (services, db) = temp_services("scryer_media_request_pending_writes").await;
    seed_library(&services, "library-1").await;
    seed_user(&services, "requester-1").await;
    let store = request_store(&services);

    let mut request = new_request("request-1", "library-1", "requester-1");
    request.requested_lease_days = Some(14);
    store
        .submit(request, &user("requester-1"), request_event("request-1"))
        .await
        .expect("submit request");

    store
        .record_decision_on_request(
            "request-1",
            Some("decision-7"),
            &["rule-a".to_string()],
            &["kids".to_string(), "family".to_string()],
        )
        .await
        .expect("stamp the verdict");

    let stamped = store
        .get("request-1")
        .await
        .unwrap()
        .expect("request exists");
    assert_eq!(stamped.status, MediaRequestStatus::Pending);
    assert_eq!(stamped.decision_id.as_deref(), Some("decision-7"));
    assert_eq!(stamped.decided_by_rule_set_ids, vec!["rule-a".to_string()]);
    assert_eq!(
        stamped.policy_tags,
        vec!["kids".to_string(), "family".to_string()]
    );

    // An edit rewrites the requested lease, including back to "forever".
    store
        .update_pending_request_preferences(
            "request-1",
            "1080p".to_string(),
            "1080P".to_string(),
            None,
            None,
            Some(7),
            request_event("request-1"),
        )
        .await
        .expect("edit preferences");
    assert_eq!(
        store
            .get("request-1")
            .await
            .unwrap()
            .expect("request exists")
            .requested_lease_days,
        Some(7)
    );
    store
        .update_pending_request_preferences(
            "request-1",
            "1080p".to_string(),
            "1080P".to_string(),
            None,
            None,
            None,
            request_event("request-1"),
        )
        .await
        .expect("edit preferences");
    assert_eq!(
        store
            .get("request-1")
            .await
            .unwrap()
            .expect("request exists")
            .requested_lease_days,
        None,
        "an edit back to forever clears the lease rather than leaving the old one"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn media_request_history_reads_answer_the_fact_builder() {
    let (services, db) = temp_services("scryer_media_request_history").await;
    seed_library(&services, "library-1").await;
    seed_user(&services, "requester-1").await;
    seed_user(&services, "requester-2").await;
    seed_user(&services, "resolver-1").await;
    let store = request_store(&services);

    // Two requests from one user for the same identity, one from another user.
    let mut first = new_request("request-1", "library-1", "requester-1");
    first.identity_fingerprint = "fingerprint-shared".to_string();
    let mut second = new_request("request-2", "library-1", "requester-1");
    second.identity_fingerprint = "fingerprint-shared".to_string();
    let mut third = new_request("request-3", "library-1", "requester-2");
    third.identity_fingerprint = "fingerprint-shared".to_string();
    let fourth = new_request("request-4", "library-1", "requester-1");

    for (request, submitter) in [
        (first, "requester-1"),
        (second, "requester-1"),
        (third, "requester-2"),
        (fourth, "requester-1"),
    ] {
        let request_id = request.id.clone();
        store
            .submit(request, &user(submitter), request_event(&request_id))
            .await
            .expect("submit request");
    }

    store
        .resolve_pending(
            "request-1",
            MediaRequestResolution {
                status: MediaRequestStatus::Rejected,
                resolved_by_user_id: Some("resolver-1".to_string()),
                resolved_at: Utc::now(),
                created_title_id: None,
                approved_quality_profile_id: None,
                approved_quality_profile_name: None,
                approved_lease_days: None,
                decision_id: None,
                decided_by_rule_set_ids: Vec::new(),
                policy_tags: Vec::new(),
                event: request_event("request-1"),
            },
        )
        .await
        .expect("reject request");

    // Counts are per submitter, and a status narrows them.
    assert_eq!(
        store
            .count_for_requester("requester-1", None, None, None)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        store
            .count_for_requester("requester-1", Some(MediaRequestStatus::Pending), None, None)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .count_for_requester(
                "requester-1",
                Some(MediaRequestStatus::Rejected),
                None,
                None
            )
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .count_for_requester("requester-2", None, None, None)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .count_for_requester(
                "requester-1",
                None,
                Some(Utc::now() + chrono::Duration::days(1)),
                None,
            )
            .await
            .unwrap(),
        0,
        "the `since` window excludes everything older"
    );
    assert_eq!(
        store
            .count_for_requester(
                "requester-1",
                None,
                Some(Utc::now() - chrono::Duration::days(1)),
                None,
            )
            .await
            .unwrap(),
        3
    );

    // History is every requester and every status: a previous denial is
    // exactly the row a status filter would hide.
    let history = store
        .history_for_fingerprint("fingerprint-shared")
        .await
        .expect("read history");
    assert_eq!(history.len(), 3);
    assert!(
        history
            .iter()
            .any(|request| request.status == MediaRequestStatus::Rejected),
        "the denial must be visible to the history facts"
    );
    assert!(
        history
            .iter()
            .any(|request| request.created_by_user_id == "requester-2"),
        "history spans every requester, not just the one asking"
    );
    assert!(
        store
            .history_for_fingerprint("fingerprint-unknown")
            .await
            .unwrap()
            .is_empty()
    );

    assert_eq!(
        store
            .count_for_requester(
                "requester-1",
                Some(MediaRequestStatus::Pending),
                None,
                Some("request-2")
            )
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .count_for_requester(
                "requester-1",
                Some(MediaRequestStatus::Rejected),
                None,
                Some("request-1")
            )
            .await
            .unwrap(),
        0
    );
    assert!(
        store
            .latest_request_at_for_user("requester-2", Some("request-3"))
            .await
            .unwrap()
            .is_none()
    );

    // Never having asked is a real answer, not an unknown.
    assert!(
        store
            .latest_request_at_for_user("requester-1", None)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .latest_request_at_for_user("nobody", None)
            .await
            .unwrap()
            .is_none()
    );

    let _ = std::fs::remove_file(db);
}
