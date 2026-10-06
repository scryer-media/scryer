//! The list engine's effects, bound to the application: the on-leave tag, the
//! departure record, and the events public lists put on the feed.

use super::*;
use crate::lists::AppListActions;
use crate::lists::act::ListActions;
use crate::lists::fetch::{ListFailure, ListFailureClass};
use crate::lists::leave::LEFT_LIST_TAG;
use crate::lists::test_support::{resolved_item, subscription};
use scryer_domain::{
    DomainEventPayload, DomainEventStream, LibraryPermission, ListMembershipState, ListMode,
    ListOnLeave, ListScope, MediaRequestStatus,
};

async fn list_events(harness: &MediaRequestTestHarness) -> Vec<DomainEvent> {
    harness
        .domain_events
        .events
        .lock()
        .await
        .iter()
        .filter(|event| event.payload.event_type().as_str().starts_with("list_"))
        .cloned()
        .collect()
}

#[tokio::test]
async fn the_tag_action_registers_the_left_list_tag_once_and_applies_it() {
    let harness = bootstrap_media_request_app();
    harness
        .titles
        .store
        .lock()
        .await
        .push(make_due_hydration_title(
            "title-alpha",
            MediaFacet::Movie,
            9051,
        ));
    harness
        .titles
        .store
        .lock()
        .await
        .push(make_due_hydration_title(
            "title-beta",
            MediaFacet::Movie,
            9052,
        ));
    let actions = AppListActions::new(&harness.app);

    actions
        .tag_title("title-alpha", LEFT_LIST_TAG)
        .await
        .expect("first tag registers the label");
    actions
        .tag_title("title-beta", LEFT_LIST_TAG)
        .await
        .expect("second tag reuses the label");

    let definitions = harness.titles.title_tag_definitions.lock().await.clone();
    assert_eq!(
        definitions
            .iter()
            .filter(|definition| definition.label == LEFT_LIST_TAG)
            .count(),
        1,
        "the label is registered once"
    );
    let titles = harness.titles.store.lock().await;
    for id in ["title-alpha", "title-beta"] {
        let title = titles.iter().find(|title| title.id == id).expect("title");
        assert!(
            title.tags.iter().any(|tag| tag == LEFT_LIST_TAG),
            "{id} carries the tag"
        );
    }
}

#[tokio::test]
async fn a_public_departure_is_recorded_on_the_title_with_its_action() {
    let harness = bootstrap_media_request_app();
    harness
        .titles
        .store
        .lock()
        .await
        .push(make_due_hydration_title(
            "title-alpha",
            MediaFacet::Movie,
            9053,
        ));
    let list = subscription("public-list-one");

    AppListActions::new(&harness.app)
        .record_departure(&list, "title-alpha", ListOnLeave::Unmonitor)
        .await
        .expect("departure recorded");

    let events = list_events(&harness).await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].stream,
        DomainEventStream::Title {
            title_id: "title-alpha".to_string()
        }
    );
    let DomainEventPayload::ListTitleLeft(data) = &events[0].payload else {
        panic!("expected a list departure, got {:?}", events[0].payload);
    };
    assert_eq!(data.list.list_name, list.name);
    assert_eq!(data.action, ListOnLeave::Unmonitor);
    assert_eq!(data.title.title_name, "Title title-alpha");
}

#[tokio::test]
async fn personal_lists_publish_only_to_the_owner_feed() {
    let harness = bootstrap_media_request_app();
    harness
        .titles
        .store
        .lock()
        .await
        .push(make_due_hydration_title(
            "title-alpha",
            MediaFacet::Movie,
            9054,
        ));
    let mut list = subscription("personal-list-one");
    list.scope = ListScope::Personal;
    list.owner_user_id = harness.user.id.clone();
    let actions = AppListActions::new(&harness.app);

    actions
        .record_departure(&list, "title-alpha", ListOnLeave::Log)
        .await
        .expect("departure handled");
    actions
        .record_sync_failure(
            &list,
            &ListFailure::new(ListFailureClass::NotFound, "Fixture Provider"),
        )
        .await
        .expect("failure handled");

    let events = list_events(&harness).await;
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event.stream
        == DomainEventStream::User {
            user_id: harness.user.id.clone(),
        }));
    let filter = scryer_domain::DomainEventFilter {
        limit: 10,
        ..Default::default()
    };
    let mut owner = harness.user.clone();
    owner.authorization.default_library = scryer_domain::LibraryPermissionMask::from_permissions([
        LibraryPermission::View,
        LibraryPermission::Request,
    ]);
    assert_eq!(
        harness
            .app
            .list_domain_events(&owner, &filter)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(
        harness
            .app
            .list_domain_events(&harness.manager, &filter)
            .await
            .unwrap()
            .is_empty()
    );
    let mut another = owner.clone();
    another.id = "another-member".into();
    assert!(
        harness
            .app
            .list_domain_events(&another, &filter)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        harness
            .app
            .recent_activity_page(10, 0)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_public_sync_failure_carries_the_plain_words_reason() {
    let harness = bootstrap_media_request_app();
    let list = subscription("public-list-one");
    let failure = ListFailure::new(ListFailureClass::NotFound, "Fixture Provider");

    AppListActions::new(&harness.app)
        .record_sync_failure(&list, &failure)
        .await
        .expect("failure recorded");

    let events = list_events(&harness).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].stream, DomainEventStream::Global);
    let DomainEventPayload::ListSyncFailed(data) = &events[0].payload else {
        panic!("expected a list sync failure, got {:?}", events[0].payload);
    };
    assert_eq!(data.reason, failure.message);
    assert_eq!(data.failure_class, "not_found");
    assert_eq!(data.list.subscription_id, "public-list-one");
}

#[tokio::test]
async fn a_held_public_list_request_is_announced_as_held() {
    let harness = bootstrap_media_request_app();
    harness.users.store.lock().await.push(harness.user.clone());
    let mut list = subscription("public-list-one");
    list.owner_user_id = harness.user.id.clone();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let route = crate::lists::test_support::route(MediaFacet::Movie, &library_id);
    let mut item = resolved_item("alpha");
    item.external_ids = vec![ExternalId::new("tvdb", "9055")];

    let request_id = AppListActions::new(&harness.app)
        .submit_request(&list, &route, &item, true)
        .await
        .expect("held request admitted");

    let events = list_events(&harness).await;
    assert_eq!(events.len(), 1);
    let DomainEventPayload::ListRequestSubmitted(data) = &events[0].payload else {
        panic!("expected a list request, got {:?}", events[0].payload);
    };
    assert!(data.held);
    assert_eq!(data.request_id, request_id);
    assert_eq!(
        crate::notifications::dispatcher::notification_event_type(&events[0].payload),
        Some(scryer_domain::NotificationEventType::ListItemHeld)
    );
}

#[tokio::test]
async fn unfollowing_a_public_list_is_announced() {
    let harness = bootstrap_media_request_app();
    *harness.lists.subscriptions.lock().unwrap() = vec![subscription("public-list-one")];
    let mut manager = harness.manager.clone();
    manager.authorization.app = AppPermissionMask::MANAGE_LISTS;

    harness
        .app
        .unsubscribe_public_list(&manager, "public-list-one")
        .await
        .expect("unfollowed");

    let events = list_events(&harness).await;
    assert_eq!(events.len(), 1);
    let DomainEventPayload::ListUnfollowed(data) = &events[0].payload else {
        panic!("expected an unfollow, got {:?}", events[0].payload);
    };
    assert_eq!(data.list.list_name, "Fixture list public-list-one");
}

#[tokio::test]
async fn maintenance_reads_list_facts_for_a_batch_of_titles() {
    use crate::lists::test_support::membership;

    let harness = bootstrap_media_request_app();
    *harness.lists.subscriptions.lock().unwrap() = vec![subscription("public-list-one")];
    let mut row = membership(
        "public-list-one",
        "item-one",
        scryer_domain::ListMembershipState::Added,
    );
    row.title_id = Some("title-alpha".to_string());
    row.added_by_list = true;
    harness.lists.insert_rows(vec![row]);
    let titles = vec![
        make_due_hydration_title("title-alpha", MediaFacet::Movie, 9056),
        make_due_hydration_title("title-beta", MediaFacet::Movie, 9057),
    ];

    let facts = harness
        .app
        .maintenance_list_facts_for_titles(&titles)
        .await
        .expect("list facts load");

    let alpha = facts.get("title-alpha").expect("listed title has facts");
    assert!(alpha.added_by_list);
    assert!(alpha.on_enabled_list);
    assert_eq!(
        alpha.names,
        vec!["Fixture list public-list-one".to_string()]
    );
    assert!(
        !facts.contains_key("title-beta"),
        "a title no list holds has no entry, which reads as known-empty"
    );
}

#[tokio::test]
async fn a_title_page_names_only_the_public_lists_that_hold_it() {
    use crate::lists::test_support::membership;

    let harness = bootstrap_media_request_app();
    let mut personal = subscription("personal-list-one");
    personal.scope = scryer_domain::ListScope::Personal;
    *harness.lists.subscriptions.lock().unwrap() = vec![subscription("public-list-one"), personal];
    harness
        .titles
        .store
        .lock()
        .await
        .push(make_due_hydration_title(
            "title-alpha",
            MediaFacet::Movie,
            9058,
        ));
    let row = |subscription_id: &str, title_id: &str| {
        let mut row = membership(
            subscription_id,
            "item-one",
            scryer_domain::ListMembershipState::Added,
        );
        row.title_id = Some(title_id.to_string());
        row.added_by_list = true;
        row
    };
    harness.lists.insert_rows(vec![
        row("public-list-one", "title-alpha"),
        row("personal-list-one", "title-alpha"),
        // A title the store does not hold is never visible to anyone.
        row("public-list-one", "title-unknown"),
    ]);

    let by_title = harness
        .app
        .public_list_memberships_for_titles(
            &harness.manager,
            &["title-alpha".to_string(), "title-unknown".to_string()],
        )
        .await
        .expect("memberships load");

    assert_eq!(by_title.len(), 1);
    let alpha = &by_title["title-alpha"];
    assert_eq!(alpha.len(), 1, "the personal list is never named");
    assert_eq!(alpha[0].subscription_id, "public-list-one");
    assert!(alpha[0].added_by_list);
    assert_eq!(alpha[0].left_at, None);
}

#[tokio::test]
async fn a_list_page_shows_only_titles_still_on_the_list() {
    use crate::lists::test_support::membership;

    let harness = bootstrap_media_request_app();
    let mut list = subscription("public-list-one");
    // Without a route or a title, a row is visible to every reader.
    list.routes.clear();
    *harness.lists.subscriptions.lock().unwrap() = vec![list];
    let current = membership(
        "public-list-one",
        "item-current",
        scryer_domain::ListMembershipState::Added,
    );
    let mut departed = membership(
        "public-list-one",
        "item-departed",
        scryer_domain::ListMembershipState::Added,
    );
    departed.left_at = Some(Utc::now());
    harness.lists.insert_rows(vec![current, departed]);

    let page = harness
        .app
        .public_list_memberships(&harness.manager, "public-list-one", 100, 0)
        .await
        .expect("memberships load");

    assert_eq!(page.total_count, 1);
    let keys = page
        .items
        .iter()
        .map(|row| row.item_key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["item-current"], "a departed title just leaves");
}

/// Act on one candidate from a public list in `mode` whose owner holds
/// `permissions` on the routed movie library.
async fn act_for_owner(
    mode: scryer_domain::ListMode,
    permissions: &[scryer_domain::LibraryPermission],
    tvdb_id: u32,
) -> (MediaRequestTestHarness, crate::lists::act::ActOutcome) {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let owner = library_permission_user("list-owner", &library_id, permissions);
    harness.users.store.lock().await.push(owner.clone());
    let mut list = subscription("public-list-one");
    list.owner_user_id = owner.id.clone();
    list.mode = mode;
    list.routes = vec![crate::lists::test_support::route(
        MediaFacet::Movie,
        &library_id,
    )];
    let mut item = resolved_item("alpha");
    item.external_ids = vec![ExternalId::new("tvdb", tvdb_id.to_string())];

    let outcome = crate::lists::act::act_on_candidate(
        &AppListActions::new(&harness.app),
        &list,
        &item,
        false,
    )
    .await;
    (harness, outcome)
}

#[tokio::test]
async fn a_request_list_owner_who_manages_titles_adds_the_title() {
    let (harness, outcome) =
        act_for_owner(ListMode::Request, &[LibraryPermission::ManageTitles], 9060).await;

    assert_eq!(
        outcome.state,
        ListMembershipState::Added,
        "{:?}",
        outcome.reason
    );
    assert!(outcome.added_by_list);
    assert_eq!(outcome.request_id, None);
    let title_id = outcome.title_id.expect("the add names its title");
    assert!(
        harness
            .titles
            .store
            .lock()
            .await
            .iter()
            .any(|title| title.id == title_id),
        "the title is in the library"
    );
    assert!(
        harness.media_requests.requests.lock().await.is_empty(),
        "no request is filed"
    );
}

#[tokio::test]
async fn a_request_list_owner_who_may_only_request_files_a_request() {
    let (harness, outcome) =
        act_for_owner(ListMode::Request, &[LibraryPermission::Request], 9061).await;

    assert_eq!(
        outcome.state,
        ListMembershipState::Requested,
        "{:?}",
        outcome.reason
    );
    assert!(outcome.request_id.is_some());
    assert_eq!(outcome.title_id, None);
    assert_eq!(harness.media_requests.requests.lock().await.len(), 1);
}

#[tokio::test]
async fn a_list_request_keeps_its_id_when_approving_it_fails() {
    let harness = bootstrap_media_request_app();
    harness
        .media_requests
        .fail_approvals
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let owner = library_permission_user(
        "list-owner",
        &library_id,
        &[
            LibraryPermission::Request,
            LibraryPermission::AutoApproveRequests,
        ],
    );
    harness.users.store.lock().await.push(owner.clone());
    let mut list = subscription("public-list-one");
    list.owner_user_id = owner.id.clone();
    list.mode = ListMode::Request;
    list.routes = vec![crate::lists::test_support::route(
        MediaFacet::Movie,
        &library_id,
    )];
    let mut item = resolved_item("alpha");
    item.external_ids = vec![ExternalId::new("tvdb", "9070".to_string())];

    let outcome = crate::lists::act::act_on_candidate(
        &AppListActions::new(&harness.app),
        &list,
        &item,
        false,
    )
    .await;

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1, "the request was filed once");
    assert_eq!(requests[0].status, MediaRequestStatus::Pending);
    assert_eq!(
        outcome.request_id.as_ref(),
        Some(&requests[0].id),
        "the membership keeps the filed request"
    );
    assert_eq!(outcome.state, ListMembershipState::Requested);
}

#[tokio::test]
async fn a_hold_list_parks_its_items_for_review_whoever_owns_it() {
    let owners: [(&[LibraryPermission], u32); 3] = [
        (&[LibraryPermission::ManageTitles], 9062),
        (&[LibraryPermission::Request], 9063),
        // An owner whose grant would approve the request still waits.
        (&[LibraryPermission::AutoApproveRequests], 9069),
    ];
    for (permissions, tvdb_id) in owners {
        let (harness, outcome) = act_for_owner(ListMode::Hold, permissions, tvdb_id).await;

        assert_eq!(
            outcome.state,
            ListMembershipState::Held,
            "{permissions:?}: {:?}",
            outcome.reason
        );
        assert_eq!(outcome.title_id, None, "{permissions:?}");
        assert!(!outcome.added_by_list, "{permissions:?}");
        let requests = harness.media_requests.requests.lock().await;
        assert_eq!(requests.len(), 1, "{permissions:?}");
        assert_eq!(Some(&requests[0].id), outcome.request_id.as_ref());
        assert_eq!(
            requests[0].status,
            MediaRequestStatus::Pending,
            "{permissions:?}: a held item waits for a reviewer"
        );
        assert!(
            harness.titles.store.lock().await.is_empty(),
            "{permissions:?}: nothing is added before review"
        );
    }
}

#[tokio::test]
async fn an_owner_who_may_neither_add_nor_request_is_blocked() {
    for (mode, tvdb_id) in [(ListMode::Request, 9064), (ListMode::Hold, 9065)] {
        let (harness, outcome) = act_for_owner(mode, &[LibraryPermission::View], tvdb_id).await;

        assert_eq!(
            outcome.state,
            ListMembershipState::BlockedPermission,
            "{mode:?}"
        );
        assert_eq!(outcome.reason.as_deref(), Some("not_permitted"), "{mode:?}");
        assert_eq!(outcome.title_id, None, "{mode:?}");
        assert_eq!(outcome.request_id, None, "{mode:?}");
        assert!(harness.titles.store.lock().await.is_empty(), "{mode:?}");
        assert!(
            harness.media_requests.requests.lock().await.is_empty(),
            "{mode:?}"
        );
    }
}

#[tokio::test]
async fn a_held_request_from_a_title_manager_goes_through_ordinary_review() {
    for (approve, tvdb_id) in [(true, 9066), (false, 9067)] {
        let (harness, outcome) =
            act_for_owner(ListMode::Hold, &[LibraryPermission::ManageTitles], tvdb_id).await;
        let request_id = outcome.request_id.expect("the hold files a request");

        if approve {
            let approved = harness
                .app
                .approve_media_request(
                    &harness.manager,
                    &request_id,
                    "1080p",
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .expect("a reviewer approves the held request");
            assert!(
                harness
                    .titles
                    .store
                    .lock()
                    .await
                    .iter()
                    .any(|title| title.id == approved.title_id),
                "approval adds the title"
            );
        } else {
            let dismissed = harness
                .app
                .dismiss_media_request(&harness.manager, &request_id)
                .await
                .expect("a reviewer dismisses the held request");
            assert_eq!(dismissed, 1);
            assert!(harness.titles.store.lock().await.is_empty());
        }
        let requests = harness.media_requests.requests.lock().await;
        let request = requests
            .iter()
            .find(|request| request.id == request_id)
            .expect("request");
        let expected = if approve {
            MediaRequestStatus::Approved
        } else {
            MediaRequestStatus::Rejected
        };
        assert_eq!(request.status, expected);
        assert_eq!(
            request.resolved_by_user_id.as_deref(),
            Some(harness.manager.id.as_str()),
            "the reviewer, not the list owner, resolved it"
        );
    }
}

#[tokio::test]
async fn a_title_manager_still_cannot_file_an_ordinary_request() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let manager = library_permission_user(
        "title-manager",
        &library_id,
        &[LibraryPermission::ManageTitles],
    );

    let error = harness
        .app
        .submit_media_request(&manager, media_request_input(library_id, 9068))
        .await
        .expect_err("Manage Titles alone does not make a title requestable");

    assert!(matches!(error, AppError::Unauthorized(_)), "{error:?}");
    assert!(harness.media_requests.requests.lock().await.is_empty());
}

const LIST_LABEL: &str = "fixture-label";

/// A public series list whose route asks for every per-title option, owned by
/// a user holding `permissions` on the routed library and kept in the list
/// store so its requests can be traced back to it.
async fn series_list_with_options(
    harness: &MediaRequestTestHarness,
    mode: ListMode,
    permissions: &[LibraryPermission],
) -> (scryer_domain::ListSubscription, String) {
    harness
        .app
        .ensure_title_tag_registered(LIST_LABEL, "Fixture label")
        .await
        .expect("label registered");
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let root_id = harness
        .libraries
        .get_by_id(&library_id)
        .await
        .expect("library read")
        .and_then(|library| library.roots.first().map(|root| root.id.clone()))
        .expect("the series library has a root");
    let owner = library_permission_user("list-owner", &library_id, permissions);
    harness.users.store.lock().await.push(owner.clone());
    let mut route = crate::lists::test_support::route(MediaFacet::Series, &library_id);
    route.quality_profile_id = Some("1080p".to_string());
    route.monitor_type = "FUTURE_EPISODES".to_string();
    route.root_folder_id = Some(root_id);
    route.use_season_folders = Some(false);
    route.release_numbering = Some("ALTERNATE".to_string());
    route.tags = vec![LIST_LABEL.to_string()];
    let mut list = subscription("public-list-one");
    list.owner_user_id = owner.id.clone();
    list.mode = mode;
    list.kinds = vec![MediaFacet::Series];
    list.routes = vec![route];
    *harness.lists.subscriptions.lock().unwrap() = vec![list.clone()];
    (list, library_id)
}

fn series_item(tvdb_id: u32) -> crate::lists::resolve::ResolvedItem {
    let mut item = resolved_item("alpha");
    item.kind = Some(MediaFacet::Series);
    item.external_ids = vec![ExternalId::new("tvdb", tvdb_id.to_string())];
    item
}

fn assert_carries_route_layout(tags: &[String]) {
    for expected in [
        "scryer:season-folder:disabled",
        "scryer:release-numbering:alternate",
    ] {
        assert!(
            tags.iter().any(|tag| tag == expected),
            "{expected} missing from {tags:?}"
        );
    }
}

#[tokio::test]
async fn a_list_add_creates_the_title_with_every_route_option() {
    let harness = bootstrap_media_request_app();
    let (list, _) =
        series_list_with_options(&harness, ListMode::Add, &[LibraryPermission::View]).await;

    let outcome = crate::lists::act::act_on_candidate(
        &AppListActions::new(&harness.app),
        &list,
        &series_item(9080),
        false,
    )
    .await;

    assert_eq!(
        outcome.state,
        ListMembershipState::Added,
        "{:?}",
        outcome.reason
    );
    let title_id = outcome.title_id.expect("the add names its title");
    let titles = harness.titles.store.lock().await;
    let title = titles
        .iter()
        .find(|title| title.id == title_id)
        .expect("title");
    for expected in [
        "scryer:quality-profile:1080p",
        "scryer:monitor-type:futureepisodes",
        LIST_LABEL,
    ] {
        assert!(
            title.tags.iter().any(|tag| tag == expected),
            "{expected} missing from {:?}",
            title.tags
        );
    }
    assert_carries_route_layout(&title.tags);
}

#[tokio::test]
async fn a_held_list_request_offers_the_list_labels_and_lands_the_route_options() {
    // An approver who keeps the offered labels, and one who clears them.
    for (approver_tags, tvdb_id) in [(None, 9081), (Some(Vec::new()), 9082)] {
        let harness = bootstrap_media_request_app();
        let (list, library_id) =
            series_list_with_options(&harness, ListMode::Hold, &[LibraryPermission::Request]).await;
        let outcome = crate::lists::act::act_on_candidate(
            &AppListActions::new(&harness.app),
            &list,
            &series_item(tvdb_id),
            false,
        )
        .await;
        let request_id = outcome.request_id.expect("the hold files a request");
        let pending = harness
            .media_requests
            .requests
            .lock()
            .await
            .iter()
            .find(|request| request.id == request_id)
            .cloned()
            .expect("request");
        assert_eq!(
            pending.policy_tags,
            vec![LIST_LABEL.to_string()],
            "the reviewer is offered the list's labels"
        );

        let clears = approver_tags.is_some();
        let approved = harness
            .app
            .approve_media_request(
                &harness.manager,
                &request_id,
                "1080p",
                None,
                None,
                None,
                approver_tags,
            )
            .await
            .expect("approved");

        let titles = harness.titles.store.lock().await;
        let title = titles
            .iter()
            .find(|title| title.id == approved.title_id)
            .expect("title");
        assert_eq!(
            title.tags.iter().any(|tag| tag == LIST_LABEL),
            !clears,
            "approver tags {clears}: {:?}",
            title.tags
        );
        assert_carries_route_layout(&title.tags);
        let route = &list.routes[0];
        assert_eq!(Some(&title.root_folder_id), route.root_folder_id.as_ref());
        assert_eq!(title.library_id, library_id);
    }
}

#[tokio::test]
async fn a_list_request_whose_route_root_is_gone_still_approves() {
    let harness = bootstrap_media_request_app();
    let (mut list, _) =
        series_list_with_options(&harness, ListMode::Hold, &[LibraryPermission::Request]).await;
    list.routes[0].root_folder_id = Some("root-removed".to_string());
    *harness.lists.subscriptions.lock().unwrap() = vec![list.clone()];
    let outcome = crate::lists::act::act_on_candidate(
        &AppListActions::new(&harness.app),
        &list,
        &series_item(9083),
        false,
    )
    .await;
    let request_id = outcome.request_id.expect("the hold files a request");

    let approved = harness
        .app
        .approve_media_request(
            &harness.manager,
            &request_id,
            "1080p",
            None,
            None,
            None,
            None,
        )
        .await
        .expect("a stale root falls back to the library default");

    let titles = harness.titles.store.lock().await;
    let title = titles
        .iter()
        .find(|title| title.id == approved.title_id)
        .expect("title");
    assert_ne!(title.root_folder_id, "root-removed");
    assert_carries_route_layout(&title.tags);
}
