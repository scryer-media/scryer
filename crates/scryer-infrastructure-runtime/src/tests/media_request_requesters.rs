//! Round-trips for the batched requester lookup the maintenance `requested`
//! facts read (RFC 137 section 8).
//!
//! The invariant under test is that a title is a key in the result exactly when
//! a media request created it: the facts builder reads key presence, not list
//! length, so a title that slipped out of the map would silently become "not
//! requested" rather than unknown.

use super::*;
use scryer_application::{
    MediaRequestRepository, MediaRequestResolution, NewMediaRequest, UserRepository,
};
use scryer_domain::{DomainEventActorKind, MediaRequestStatus, User};

pub(super) fn request_store(services: &SqliteServices) -> crate::MediaRequestStore {
    crate::MediaRequestStore::new(services.datastore())
}

pub(super) async fn seed_user(services: &SqliteServices, id: &str) {
    UserRepository::create(
        &user_store(services),
        User {
            id: id.to_string(),
            username: id.to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: Default::default(),
        },
    )
    .await
    .expect("seed user");
}

pub(super) async fn seed_library(services: &SqliteServices, id: &str) {
    sqlx::query(
        "INSERT INTO libraries
            (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES (?, ?, ?, ?, 0, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
    )
    .bind(id)
    .bind(MediaFacet::Movie.as_str())
    .bind(id)
    .bind(id)
    .execute(services.pool())
    .await
    .expect("seed library");
}

pub(super) async fn seed_title(services: &SqliteServices, id: &str) {
    TitleRepository::create(&title_store(services), make_test_title(id, None))
        .await
        .expect("seed title");
}

pub(super) fn request_event(request_id: &str) -> NewDomainEvent {
    NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_kind: DomainEventActorKind::System,
        actor_user_id: None,
        actor_display_name: "System".to_string(),
        title_id: None,
        facet: Some(MediaFacet::Movie),
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::Global,
        payload: DomainEventPayload::MediaRequestSubmitted(
            scryer_domain::MediaRequestSubmittedEventData {
                requested_lease_days: None,
                request_id: request_id.to_string(),
                library_id: "library-1".to_string(),
                facet: MediaFacet::Movie,
                title_name: format!("Request {request_id}"),
                external_ids: Vec::new(),
                poster_url: None,
                year: None,
                requested_quality_profile_id: None,
                requested_quality_profile_name: None,
                requested_monitor_type: None,
            },
        ),
    }
}

pub(super) fn new_request(id: &str, library_id: &str, submitter: &str) -> NewMediaRequest {
    NewMediaRequest {
        rating_summary: scryer_domain::TitleRatingSummary::default(),
        background_url: None,
        requested_monitor_selection: None,
        id: id.to_string(),
        library_id: library_id.to_string(),
        facet: MediaFacet::Movie,
        identity_fingerprint: format!("fingerprint-{id}"),
        title: format!("Request {id}"),
        sort_title: None,
        slug: None,
        poster_url: None,
        year: None,
        overview: None,
        runtime_minutes: None,
        language: None,
        content_status: None,
        requested_quality_profile_id: None,
        requested_quality_profile_name: None,
        requested_monitor_type: None,
        requested_lease_days: None,
        metadata_snapshot_json: "{}".to_string(),
        external_ids: Vec::new(),
        created_by_user_id: submitter.to_string(),
    }
}

pub(super) fn user(id: &str) -> User {
    User {
        id: id.to_string(),
        username: id.to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    }
}

/// A person who seconds an existing request. The store's own `submit` only ever
/// records the submitter, so the extra row is written directly.
async fn second_request(services: &SqliteServices, request_id: &str, user_id: &str, offset: i64) {
    sqlx::query(
        "INSERT INTO media_request_requesters (request_id, user_id, requested_at)
         VALUES (?, ?, ?)",
    )
    .bind(request_id)
    .bind(user_id)
    .bind((Utc::now() + chrono::Duration::seconds(offset)).to_rfc3339())
    .execute(services.pool())
    .await
    .expect("second the request");
}

async fn approve_onto_title(services: &SqliteServices, request_id: &str, title_id: &str) {
    request_store(services)
        .resolve_pending(
            request_id,
            MediaRequestResolution {
                status: MediaRequestStatus::Approved,
                resolved_by_user_id: Some("resolver".to_string()),
                resolved_at: Utc::now(),
                created_title_id: Some(title_id.to_string()),
                approved_quality_profile_id: None,
                approved_quality_profile_name: None,
                approved_lease_days: None,
                decision_id: None,
                decided_by_rule_set_ids: Vec::new(),
                policy_tags: Vec::new(),
                event: request_event(request_id),
            },
        )
        .await
        .expect("approve request");
}

#[tokio::test]
async fn requester_ids_union_every_request_that_created_the_title() {
    let (services, db) = temp_services("scryer_media_request_requesters").await;
    seed_library(&services, "library-1").await;
    for id in ["user-a", "user-b", "user-c", "resolver"] {
        seed_user(&services, id).await;
    }
    for id in ["title-requested", "title-pending", "title-untouched"] {
        seed_title(&services, id).await;
    }
    let store = request_store(&services);

    // Two separate requests end up creating the same title, and the people on
    // them overlap: the union has to dedupe across requests, not just within.
    store
        .submit(
            new_request("request-1", "library-1", "user-a"),
            &user("user-a"),
            request_event("request-1"),
        )
        .await
        .expect("submit the first request");
    second_request(&services, "request-1", "user-b", 30).await;
    approve_onto_title(&services, "request-1", "title-requested").await;

    store
        .submit(
            new_request("request-2", "library-1", "user-c"),
            &user("user-c"),
            request_event("request-2"),
        )
        .await
        .expect("submit the second request");
    second_request(&services, "request-2", "user-a", 60).await;
    approve_onto_title(&services, "request-2", "title-requested").await;

    // A request nobody approved created no title, so it must not make its
    // subject look requested.
    store
        .submit(
            new_request("request-3", "library-1", "user-b"),
            &user("user-b"),
            request_event("request-3"),
        )
        .await
        .expect("submit the pending request");

    let by_title = store
        .requester_user_ids_by_title_ids(&[
            "title-requested".to_string(),
            "title-pending".to_string(),
            "title-untouched".to_string(),
        ])
        .await
        .expect("batch requester lookup");

    assert_eq!(
        by_title.keys().collect::<Vec<_>>(),
        vec![&"title-requested".to_string()],
        "only a title a request actually created may appear: {by_title:?}"
    );
    let requesters = &by_title["title-requested"];
    assert_eq!(
        requesters.len(),
        3,
        "user-a appears on both requests and must be listed once: {requesters:?}"
    );
    assert_eq!(
        requesters.iter().collect::<std::collections::HashSet<_>>(),
        [
            "user-a".to_string(),
            "user-b".to_string(),
            "user-c".to_string()
        ]
        .iter()
        .collect::<std::collections::HashSet<_>>()
    );
    assert_eq!(
        requesters[0], "user-a",
        "the earliest request's submitter comes first: {requesters:?}"
    );

    // Stable across calls: the facts document is compared between runs, so an
    // order that drifts would look like a changed fact.
    let again = store
        .requester_user_ids_by_title_ids(&["title-requested".to_string()])
        .await
        .expect("second lookup");
    assert_eq!(&again["title-requested"], requesters);

    let empty = store
        .requester_user_ids_by_title_ids(&[])
        .await
        .expect("empty lookup");
    assert!(empty.is_empty());

    let _ = std::fs::remove_file(db);
}
