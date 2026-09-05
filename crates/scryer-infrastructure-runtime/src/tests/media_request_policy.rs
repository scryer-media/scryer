//! Round-trips for the policy columns 0220 added to `media_requests` and for
//! the three history reads the request-fact builder needs (spec 0003 FR-030,
//! FR-040, FR-050).

use super::media_request_requesters::{
    new_request, request_event, request_store, seed_library, seed_user, user,
};
use super::*;
use scryer_application::{MediaRequestRepository, MediaRequestResolution};
use scryer_domain::MediaRequestStatus;

#[tokio::test]
async fn media_request_policy_columns_round_trip() {
    let (services, db) = temp_services("scryer_media_request_policy").await;
    seed_library(&services, "library-1").await;
    seed_user(&services, "requester-1").await;
    seed_user(&services, "resolver-1").await;
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
                created_title_id: None,
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
            .count_for_requester("requester-1", None, None)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        store
            .count_for_requester("requester-1", Some(MediaRequestStatus::Pending), None)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .count_for_requester("requester-1", Some(MediaRequestStatus::Rejected), None)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .count_for_requester("requester-2", None, None)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .count_for_requester(
                "requester-1",
                None,
                Some(Utc::now() + chrono::Duration::days(1))
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
                Some(Utc::now() - chrono::Duration::days(1))
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

    // Never having asked is a real answer, not an unknown.
    assert!(
        store
            .latest_request_at_for_user("requester-1")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .latest_request_at_for_user("nobody")
            .await
            .unwrap()
            .is_none()
    );

    let _ = std::fs::remove_file(db);
}
