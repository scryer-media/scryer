use scryer_domain::{
    ListExclusionScope, ListMembershipState, ListScope, MediaFacet, MediaRequest,
    MediaRequestOrigin, MediaRequestStatus, TitleRatingSummary,
};

use super::remember_rejected_request;
use crate::lists::test_support::{MemoryListStore, at, membership, subscription, tmdb};

pub(crate) fn rejected_request(id: &str, origin: MediaRequestOrigin) -> MediaRequest {
    MediaRequest {
        id: id.to_string(),
        library_id: "library-movies".to_string(),
        facet: MediaFacet::Movie,
        status: MediaRequestStatus::Rejected,
        identity_fingerprint: format!("fingerprint-{id}"),
        title: "Fixture Title alpha".to_string(),
        sort_title: None,
        slug: None,
        poster_url: None,
        background_url: None,
        year: Some(2031),
        overview: None,
        runtime_minutes: None,
        language: None,
        content_status: None,
        rating_summary: TitleRatingSummary::default(),
        requested_quality_profile_id: None,
        requested_quality_profile_name: None,
        requested_monitor_type: None,
        requested_monitor_selection: None,
        resolved_by_user_id: Some("reviewer-one".to_string()),
        resolved_at: Some(at(5)),
        created_title_id: None,
        approved_quality_profile_id: None,
        approved_quality_profile_name: None,
        requested_lease_days: None,
        approved_lease_days: None,
        decision_id: None,
        decided_by_rule_set_ids: Vec::new(),
        policy_tags: Vec::new(),
        metadata_snapshot_json: "{}".to_string(),
        external_ids: vec![tmdb("alpha-id")],
        requesters: Vec::new(),
        origin,
        created_by_user_id: "owner-one".to_string(),
        created_at: at(0),
        updated_at: at(5),
    }
}

fn held_row(subscription_id: &str, request_id: &str) -> scryer_domain::ListMembership {
    let mut row = membership(subscription_id, "alpha", ListMembershipState::Held);
    row.request_id = Some(request_id.to_string());
    row
}

#[tokio::test]
async fn rejecting_a_public_list_request_excludes_it_from_that_list() {
    let store = MemoryListStore::with_subscriptions(vec![subscription("public-a")]);
    store.insert_rows(vec![held_row("public-a", "request-1")]);
    let request = rejected_request(
        "request-1",
        MediaRequestOrigin::PublicList {
            subscription_id: "public-a".to_string(),
        },
    );

    let record = remember_rejected_request(&store, &store, &store, &request, "reviewer-one", at(5))
        .await
        .unwrap();

    assert_eq!(record.exclusions_created, 1);
    let exclusions = store.exclusions.lock().unwrap().clone();
    assert_eq!(exclusions.len(), 1);
    assert_eq!(
        exclusions[0].scope,
        ListExclusionScope::List {
            subscription_id: "public-a".to_string()
        }
    );
    assert_eq!(exclusions[0].external_ids, vec![tmdb("alpha-id")]);
    assert_eq!(
        exclusions[0].created_by_user_id.as_deref(),
        Some("reviewer-one")
    );
    assert_eq!(
        store.row("public-a", "alpha").state,
        ListMembershipState::Excluded
    );

    // A second rejection of the same title adds nothing.
    let again = remember_rejected_request(&store, &store, &store, &request, "reviewer-one", at(6))
        .await
        .unwrap();
    assert_eq!(again.exclusions_created, 0);
    assert_eq!(store.exclusions.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn rejecting_a_personal_list_request_marks_the_row_rejected_without_an_exclusion() {
    let mut personal = subscription("personal-a");
    personal.scope = ListScope::Personal;
    let store = MemoryListStore::with_subscriptions(vec![personal]);
    store.insert_rows(vec![held_row("personal-a", "request-2")]);
    let request = rejected_request(
        "request-2",
        MediaRequestOrigin::PersonalList {
            subscription_id: "personal-a".to_string(),
        },
    );

    let record = remember_rejected_request(&store, &store, &store, &request, "reviewer-one", at(5))
        .await
        .unwrap();

    assert_eq!(record.exclusions_created, 0);
    assert_eq!(record.memberships_rejected, 1);
    assert!(store.exclusions.lock().unwrap().is_empty());
    assert_eq!(
        store.row("personal-a", "alpha").state,
        ListMembershipState::Rejected
    );
}

#[tokio::test]
async fn rejecting_a_manual_request_touches_no_list() {
    let store = MemoryListStore::with_subscriptions(vec![subscription("public-a")]);
    let request = rejected_request("request-3", MediaRequestOrigin::Manual);

    let record = remember_rejected_request(&store, &store, &store, &request, "reviewer-one", at(5))
        .await
        .unwrap();

    assert_eq!(record, Default::default());
    assert!(store.exclusions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_departed_row_keeps_its_history() {
    let store = MemoryListStore::with_subscriptions(vec![subscription("public-a")]);
    let mut departed = held_row("public-a", "request-4");
    departed.left_at = Some(at(3));
    store.insert_rows(vec![departed]);
    let request = rejected_request(
        "request-4",
        MediaRequestOrigin::PublicList {
            subscription_id: "public-a".to_string(),
        },
    );

    remember_rejected_request(&store, &store, &store, &request, "reviewer-one", at(5))
        .await
        .unwrap();

    let row = store.row("public-a", "alpha");
    assert_eq!(row.state, ListMembershipState::Held);
    assert_eq!(row.left_at, Some(at(3)));
}
