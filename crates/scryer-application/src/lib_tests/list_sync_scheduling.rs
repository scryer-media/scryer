//! When public and personal lists are synced: following starts a sync at
//! once, requests made during a run share one follow-up run, and exclusion
//! changes clear only the fingerprints they can apply to.

use super::*;

fn list_manager() -> User {
    let mut manager = User::new_admin("list-manager");
    manager.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::MANAGE_LISTS,
        loaded: true,
        ..Default::default()
    };
    manager
}

async fn list_sync_starts(harness: &MediaRequestTestHarness) -> usize {
    harness
        .domain_events
        .events
        .lock()
        .await
        .iter()
        .filter(|event| {
            matches!(
                &event.payload,
                DomainEventPayload::JobRunStarted(data) if data.job_key == "list_sync"
            )
        })
        .count()
}

#[tokio::test]
async fn sync_now_clears_the_stored_fingerprint_so_the_list_is_read_in_full() {
    let harness = bootstrap_media_request_app();
    let mut followed = crate::lists::test_support::subscription("public-list-one");
    followed.sync.fetch_fingerprint = Some("fingerprint-one".to_string());
    *harness.lists.subscriptions.lock().unwrap() = vec![followed];

    let queued = harness
        .app
        .sync_public_list_now(&list_manager(), "public-list-one")
        .await
        .expect("sync queued");

    assert_eq!(queued, vec!["public-list-one".to_string()]);
    let stored = harness.lists.subscription("public-list-one");
    assert_eq!(stored.sync.fetch_fingerprint, None);
    assert!(stored.sync.next_at.is_some());
    assert_eq!(list_sync_starts(&harness).await, 1);
}

/// A harness with the fixture list provider installed, so a list can be
/// followed.
fn bootstrap_with_fixture_lists() -> MediaRequestTestHarness {
    bootstrap_media_request_app_with_list_plugins(Arc::new(
        crate::lists::test_support::ScriptedProvider(
            crate::lists::test_support::ScriptedLists::new(),
        ),
    ))
}

async fn follow_fixture_list(harness: &MediaRequestTestHarness) -> scryer_domain::ListSubscription {
    harness
        .app
        .subscribe_public_list(
            &list_manager(),
            crate::lists::public::PublicListInput {
                provider: Some(crate::lists::test_support::PROVIDER.to_string()),
                source_type: Some("user_list".to_string()),
                params: std::collections::BTreeMap::from([(
                    "list_id".to_string(),
                    "followed-fixture-source".to_string(),
                )]),
                mode: scryer_domain::ListMode::Add,
                ..Default::default()
            },
        )
        .await
        .expect("list followed")
}

/// Waits for a list sync run other than `other_than` to end.
async fn next_list_sync_run_end(
    runs: &mut tokio::sync::broadcast::Receiver<JobRun>,
    other_than: Option<&str>,
) -> JobRun {
    within_deadline("a list sync run to end", async {
        loop {
            let run = runs.recv().await.expect("job run events stay open");
            if run.job_key == JobKey::ListSync
                && run.status.is_terminal()
                && Some(run.id.as_str()) != other_than
            {
                return run;
            }
        }
    })
    .await
}

fn sync_runs_of(harness: &MediaRequestTestHarness, subscription_id: &str) -> usize {
    harness
        .lists
        .runs
        .lock()
        .unwrap()
        .iter()
        .filter(|run| run.subscription_id == subscription_id)
        .count()
}

#[tokio::test]
async fn following_a_list_starts_its_first_sync_at_once() {
    let harness = bootstrap_with_fixture_lists();
    let mut runs = harness.app.runtime.jobs.job_run_tracker.subscribe();

    let followed = follow_fixture_list(&harness).await;

    let stored = harness.lists.subscription(&followed.id);
    assert!(stored.sync.next_at.is_some(), "the new list is due now");
    assert_eq!(
        list_sync_starts(&harness).await,
        1,
        "following starts the first sync instead of waiting for the schedule"
    );
    next_list_sync_run_end(&mut runs, None).await;
    assert_eq!(sync_runs_of(&harness, &followed.id), 1, "synced once");
}

#[tokio::test]
async fn a_list_followed_while_a_sync_is_running_is_synced_when_that_sync_ends() {
    let harness = bootstrap_with_fixture_lists();
    let app = &harness.app;
    let tracker = &app.runtime.jobs.job_run_tracker;
    // A sync already mid-pass: it read its due set before the list below
    // was followed, so it will not reach that list.
    let now = chrono::Utc::now();
    let running = JobRunRecord {
        id: "list-sync-in-progress".to_string(),
        job_key: JobKey::ListSync,
        operation_type: JobKey::ListSync.as_str().to_string(),
        status: JobRunStatus::Running,
        trigger_source: JobTriggerSource::ScheduledInterval,
        actor_user_id: None,
        progress_json: None,
        summary_json: None,
        summary_text: None,
        error_text: None,
        started_at: now,
        completed_at: None,
        created_at: now,
        updated_at: now,
    };
    tracker
        .upsert_active_run(JobRun::from_record(&running, None))
        .await;

    let followed = follow_fixture_list(&harness).await;
    // "Sync now" during the same run goes the same way.
    app.sync_public_list_now(&list_manager(), &followed.id)
        .await
        .expect("sync now accepted");
    assert_eq!(
        list_sync_starts(&harness).await,
        0,
        "no second sync runs beside the one in progress"
    );
    assert_eq!(sync_runs_of(&harness, &followed.id), 0);

    let mut runs = tracker.subscribe();
    app.finish_job_run(
        running,
        crate::domain_events::DomainEventActor::system(),
        None,
        None,
        None,
        None,
    )
    .await
    .expect("the running sync ends");

    next_list_sync_run_end(&mut runs, Some("list-sync-in-progress")).await;
    assert_eq!(
        list_sync_starts(&harness).await,
        1,
        "requests made during one run share a single follow-up run"
    );
    assert_eq!(
        sync_runs_of(&harness, &followed.id),
        1,
        "the followed list is synced without waiting for the schedule"
    );
}

#[tokio::test]
async fn titles_and_requests_name_the_public_lists_that_hold_them() {
    let harness = bootstrap_media_request_app();
    *harness.lists.subscriptions.lock().unwrap() =
        vec![crate::lists::test_support::subscription("public-list-one")];
    harness
        .titles
        .store
        .lock()
        .await
        .push(make_due_hydration_title(
            "title-alpha",
            MediaFacet::Movie,
            9061,
        ));
    let mut row = crate::lists::test_support::membership(
        "public-list-one",
        "item-one",
        scryer_domain::ListMembershipState::Added,
    );
    row.title_id = Some("title-alpha".to_string());
    harness.lists.insert_rows(vec![row]);
    let request = crate::lists::rejection::tests::rejected_request(
        "request-one",
        scryer_domain::MediaRequestOrigin::PublicList {
            subscription_id: "public-list-one".to_string(),
        },
    );
    let titles = ["title-alpha".to_string()];

    let memberships = harness
        .app
        .public_list_memberships_for_titles(&harness.manager, &titles)
        .await
        .unwrap();
    assert_eq!(memberships["title-alpha"].len(), 1);
    let facts = harness
        .app
        .media_request_policy_facts(&harness.manager, std::slice::from_ref(&request))
        .await
        .unwrap();
    assert_eq!(
        facts["request-one"].public_list_name.as_deref(),
        Some("Fixture list public-list-one")
    );
}

fn movie_library_viewer() -> User {
    let mut viewer = User::new_admin("list-viewer");
    viewer.authorization = scryer_domain::UserAuthorization {
        libraries: HashMap::from([(
            scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            scryer_domain::LibraryPermissionMask::from_permissions([
                scryer_domain::LibraryPermission::View,
            ]),
        )]),
        loaded: true,
        ..Default::default()
    };
    viewer
}

#[tokio::test]
async fn a_reader_sees_list_titles_only_in_libraries_they_may_view() {
    use crate::lists::test_support::{membership, route, subscription};
    use scryer_domain::ListMembershipState;

    let harness = bootstrap_media_request_app();
    let movies = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let series = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let mut followed = subscription("public-list-one");
    followed.kinds = vec![MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime];
    let mut series_route = route(MediaFacet::Series, &series);
    series_route.quality_profile_id = Some("profile-hidden".to_string());
    series_route.root_folder_id = Some("root-hidden".to_string());
    series_route.tags = vec!["tag-hidden".to_string()];
    let mut movie_route = route(MediaFacet::Movie, &movies);
    movie_route.quality_profile_id = Some("profile-shown".to_string());
    followed.routes = vec![movie_route, series_route];
    *harness.lists.subscriptions.lock().unwrap() = vec![followed];

    let mut in_series_library = make_due_hydration_title("title-hidden", MediaFacet::Series, 9062);
    in_series_library.library_id = series.clone();
    let in_movie_library = make_due_hydration_title("title-shown", MediaFacet::Movie, 9063);
    {
        let mut store = harness.titles.store.lock().await;
        store.push(in_series_library);
        store.push(in_movie_library);
    }
    let row = |key: &str, kind: MediaFacet, title_id: Option<&str>| {
        let mut row = membership("public-list-one", key, ListMembershipState::Added);
        row.kind = kind;
        row.title_id = title_id.map(str::to_string);
        row.request_id = title_id.map(|id| format!("request-{id}"));
        row
    };
    harness.lists.insert_rows(vec![
        row("movie-unadded", MediaFacet::Movie, None),
        row("series-unadded", MediaFacet::Series, None),
        // The title's own library decides, not the route.
        row(
            "movie-in-series-library",
            MediaFacet::Movie,
            Some("title-hidden"),
        ),
        row(
            "series-in-movie-library",
            MediaFacet::Series,
            Some("title-shown"),
        ),
        // No route and no title: only what the provider's list shows.
        row("anime-unrouted", MediaFacet::Anime, None),
    ]);
    let viewer = movie_library_viewer();

    let page = harness
        .app
        .public_list_memberships(&viewer, "public-list-one", 100, 0)
        .await
        .expect("memberships load");
    let mut keys = page
        .items
        .iter()
        .map(|row| row.item_key.as_str())
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["anime-unrouted", "movie-unadded", "series-in-movie-library"]
    );
    assert_eq!(page.total_count, 3);

    let seen = harness
        .app
        .public_list_subscription(&viewer, "public-list-one")
        .await
        .unwrap()
        .expect("the list stays visible");
    let shown = &seen.routes[0];
    assert_eq!(shown.library_id, movies);
    assert_eq!(shown.quality_profile_id.as_deref(), Some("profile-shown"));
    let hidden = &seen.routes[1];
    assert_eq!(hidden.library_id, series, "the library id stays");
    assert_eq!(hidden.quality_profile_id, None);
    assert_eq!(hidden.root_folder_id, None);
    assert!(hidden.tags.is_empty());

    let listed = harness
        .app
        .public_list_subscriptions(&viewer)
        .await
        .unwrap();
    assert_eq!(listed[0].routes[1].quality_profile_id, None);

    let everything = harness
        .app
        .public_list_memberships(&harness.manager, "public-list-one", 100, 0)
        .await
        .unwrap();
    assert_eq!(
        everything.total_count, 5,
        "a reader of every library sees all"
    );
}

fn fixture_exclusion(
    id: &str,
    kind: MediaFacet,
    scope: scryer_domain::ListExclusionScope,
) -> scryer_domain::ListExclusion {
    scryer_domain::ListExclusion {
        id: id.to_string(),
        kind,
        external_ids: vec![crate::lists::test_support::tmdb("excluded-id")],
        display_title: "Fixture Excluded Title".to_string(),
        year: None,
        scope,
        created_by_user_id: None,
        created_at: crate::lists::test_support::at(0),
    }
}

#[tokio::test]
async fn removing_an_exclusion_clears_the_fingerprints_it_could_apply_to() {
    use crate::lists::test_support::subscription;
    use scryer_domain::ListExclusionScope;

    let harness = bootstrap_media_request_app();
    let followed = |id: &str, kind: MediaFacet, enabled: bool| {
        let mut row = subscription(id);
        row.kinds = vec![kind];
        row.enabled = enabled;
        row.sync.fetch_fingerprint = Some(format!("fingerprint-{id}"));
        row
    };
    *harness.lists.subscriptions.lock().unwrap() = vec![
        followed("movies-on", MediaFacet::Movie, true),
        followed("movies-other", MediaFacet::Movie, true),
        followed("movies-off", MediaFacet::Movie, false),
        followed("series-on", MediaFacet::Series, true),
    ];
    harness.lists.exclusions.lock().unwrap().extend([
        fixture_exclusion(
            "scoped",
            MediaFacet::Movie,
            ListExclusionScope::List {
                subscription_id: "movies-on".to_string(),
            },
        ),
        fixture_exclusion(
            "everywhere",
            MediaFacet::Movie,
            ListExclusionScope::AllLists,
        ),
    ]);
    let fingerprint = |id: &str| harness.lists.subscription(id).sync.fetch_fingerprint;

    harness
        .app
        .remove_list_exclusion(&list_manager(), "scoped")
        .await
        .expect("scoped exclusion removed");
    assert_eq!(fingerprint("movies-on"), None);
    assert!(
        fingerprint("movies-other").is_some(),
        "another list keeps its fingerprint"
    );

    harness
        .app
        .remove_list_exclusion(&list_manager(), "everywhere")
        .await
        .expect("exclusion removed");
    assert_eq!(fingerprint("movies-other"), None);
    assert!(
        fingerprint("movies-off").is_some(),
        "a list that is off is left alone"
    );
    assert!(
        fingerprint("series-on").is_some(),
        "a list of another kind is left alone"
    );
    assert!(harness.lists.exclusions.lock().unwrap().is_empty());
    assert_eq!(list_sync_starts(&harness).await, 0, "no sync is started");
}

#[tokio::test]
async fn adding_an_exclusion_clears_the_fingerprints_it_could_apply_to() {
    use crate::lists::public::NewListExclusionInput;
    use crate::lists::test_support::{subscription, tmdb};
    use scryer_domain::ListExclusionScope;

    let harness = bootstrap_media_request_app();
    let followed = |id: &str, kind: MediaFacet, enabled: bool| {
        let mut row = subscription(id);
        row.kinds = vec![kind];
        row.enabled = enabled;
        row.sync.fetch_fingerprint = Some(format!("fingerprint-{id}"));
        row
    };
    *harness.lists.subscriptions.lock().unwrap() = vec![
        followed("movies-on", MediaFacet::Movie, true),
        followed("movies-other", MediaFacet::Movie, true),
        followed("movies-off", MediaFacet::Movie, false),
        followed("series-on", MediaFacet::Series, true),
    ];
    let fingerprint = |id: &str| harness.lists.subscription(id).sync.fetch_fingerprint;
    let input = |scope: ListExclusionScope| NewListExclusionInput {
        kind: MediaFacet::Movie,
        external_ids: vec![tmdb("excluded-id")],
        display_title: "Fixture Excluded Title".to_string(),
        year: None,
        scope,
    };

    harness
        .app
        .add_list_exclusion(
            &list_manager(),
            input(ListExclusionScope::List {
                subscription_id: "movies-on".to_string(),
            }),
        )
        .await
        .expect("scoped exclusion added");
    assert_eq!(fingerprint("movies-on"), None);
    assert!(
        fingerprint("movies-other").is_some(),
        "another list keeps its fingerprint"
    );

    harness
        .app
        .add_list_exclusion(&list_manager(), input(ListExclusionScope::AllLists))
        .await
        .expect("exclusion added");
    assert_eq!(fingerprint("movies-other"), None);
    assert!(
        fingerprint("movies-off").is_some(),
        "a list that is off is left alone"
    );
    assert!(
        fingerprint("series-on").is_some(),
        "a list of another kind is left alone"
    );
    assert_eq!(harness.lists.exclusions.lock().unwrap().len(), 2);
    assert_eq!(list_sync_starts(&harness).await, 0, "no sync is started");
}

#[tokio::test]
async fn a_list_manager_can_read_quality_profiles_for_list_routes() {
    let harness = bootstrap_media_request_app();

    harness
        .app
        .get_quality_profile_settings(&list_manager())
        .await
        .expect("a list manager reads quality profiles");

    let mut outsider = User::new_admin("outsider");
    outsider.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::default(),
        loaded: true,
        ..Default::default()
    };
    let error = harness
        .app
        .get_quality_profile_settings(&outsider)
        .await
        .expect_err("a user without any of the permissions is refused");
    assert!(matches!(error, AppError::Unauthorized(_)), "{error:?}");
}
