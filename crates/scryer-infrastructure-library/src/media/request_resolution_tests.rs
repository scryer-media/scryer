use super::tests::{seed_owners, seed_request, test_store};
use super::*;
use scryer_application::LifecycleClaimRepository;
use scryer_domain::{
    DomainEventActorKind, DomainEventPayload, DomainEventStream, MediaRequestResolvedEventData,
};

fn approval(request: &MediaRequest) -> MediaRequestResolution {
    let now = Utc::now();
    MediaRequestResolution {
        status: MediaRequestStatus::Approved,
        resolved_by_user_id: None,
        resolved_at: now,
        created_title_id: Some("approved-title".into()),
        approved_quality_profile_id: None,
        approved_quality_profile_name: None,
        approved_lease_days: Some(45),
        decision_id: Some("primary-trace".into()),
        decided_by_rule_set_ids: vec!["primary-policy".into()],
        policy_tags: vec!["primary-tag".into()],
        event: NewDomainEvent {
            event_id: scryer_domain::Id::new().0,
            occurred_at: now,
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".into(),
            title_id: None,
            facet: Some(MediaFacet::Movie),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Global,
            payload: DomainEventPayload::MediaRequestApproved(MediaRequestResolvedEventData {
                request_id: request.id.clone(),
                library_id: request.library_id.clone(),
                facet: request.facet.clone(),
                title_name: request.title.clone(),
                external_ids: request.external_ids.clone(),
                created_title_id: Some("approved-title".into()),
                requested_quality_profile_id: None,
                requested_quality_profile_name: None,
                requested_monitor_type: None,
                approved_quality_profile_id: None,
                approved_quality_profile_name: None,
                decided_by_rule_set_ids: vec!["primary-policy".into()],
                decision_reason_codes: vec!["primary-reason".into()],
                approved_lease_days: Some(45),
                policy_tags: vec!["primary-tag".into()],
            }),
        },
    }
}

async fn seed_resolution_requests(store: &MediaRequestStore) -> MediaRequest {
    seed_owners(store).await;
    SqlRuntime::execute_write(
        &store.datastore,
        "seed_resolution_root",
        "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {})",
        vec![
            SqlArg::Text("request-resolution-root".into()),
            SqlArg::Text("library-under-test".into()),
            SqlArg::Text("/media/request-resolution".into()),
            SqlArg::Text("/media/request-resolution".into()),
            SqlArg::Bool(true),
            SqlArg::Timestamp(Utc::now()),
            SqlArg::Timestamp(Utc::now()),
        ],
    ).await.unwrap();
    SqlRuntime::execute_write(
        &store.datastore,
        "seed_resolution_title",
        "INSERT INTO titles (id, library_id, root_folder_id, name, facet, created_at) VALUES ({}, {}, {}, {}, {}, {})",
        vec![
            SqlArg::Text("approved-title".into()),
            SqlArg::Text("library-under-test".into()),
            SqlArg::Text("request-resolution-root".into()),
            SqlArg::Text("Approved title".into()),
            SqlArg::Text("movie".into()),
            SqlArg::Timestamp(Utc::now()),
        ],
    )
    .await
    .unwrap();
    for (id, days) in [
        ("primary", Some(30)),
        ("sibling", Some(5)),
        ("forever", None),
    ] {
        seed_request(store, id, "pending", &["original-tag"]).await;
        SqlRuntime::execute_write(&store.datastore, "seed_resolution_policy",
            "UPDATE media_requests SET requested_lease_days = {}, decision_id = {}, decided_by_rule_set_ids = {} WHERE id = {}",
            vec![SqlArg::OptI64(days), SqlArg::Text(format!("{id}-trace")), SqlArg::Text(format!("[\"{id}-policy\"]")), SqlArg::Text(id.into())]).await.unwrap();
        SqlRuntime::execute_write(&store.datastore, "seed_resolution_identity",
            "INSERT INTO media_request_external_ids (request_id, library_id, source, external_id, created_at) VALUES ({}, {}, {}, {}, {})",
            vec![SqlArg::Text(id.into()), SqlArg::Text("library-under-test".into()), SqlArg::Text("tvdb".into()), SqlArg::Text("9000".into()), SqlArg::Timestamp(Utc::now())]).await.unwrap();
    }
    store.get("primary").await.unwrap().unwrap()
}

#[tokio::test]
async fn overlapping_request_resolution_preserves_each_lease_and_provenance() {
    let store = test_store().await;
    let request = seed_resolution_requests(&store).await;
    let resolution = approval(&request);
    assert_eq!(
        store
            .resolve_pending_overlapping(&request, resolution.clone())
            .await
            .unwrap()
            .updated,
        3
    );
    let claims = crate::media::lifecycle_claims::LifecycleClaimStore::new(store.datastore.clone())
        .list_for_title("approved-title")
        .await
        .unwrap();
    assert_eq!(claims.len(), 3);
    for (id, days) in [
        ("primary", Some(45)),
        ("sibling", Some(5)),
        ("forever", None),
    ] {
        let row = store.get(id).await.unwrap().unwrap();
        assert_eq!(row.approved_lease_days, days);
        assert_eq!(row.decision_id, Some(format!("{id}-trace")));
        assert_eq!(row.decided_by_rule_set_ids, [format!("{id}-policy")]);
        assert_eq!(
            row.policy_tags,
            [if id == "primary" {
                "primary-tag"
            } else {
                "original-tag"
            }]
        );
        let claim = claims
            .iter()
            .find(|claim| claim.producer_ref.as_deref() == Some(id))
            .unwrap();
        assert_eq!(claim.duration_days, days);
    }
    let events = SqlRuntime::fetch_all(
        store.datastore.read_exec(),
        "SELECT payload_json FROM domain_events WHERE event_type = 'media_request_approved'",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        events.len(),
        3,
        "each request has its own durable resolution audit"
    );
    let payloads: Vec<DomainEventPayload> = events
        .iter()
        .map(|row| {
            let encoded = row.opt_bytes("payload_json").unwrap().unwrap();
            let decoded =
                scryer_infrastructure_sql::domain_event_payload::decode_domain_event_payload(
                    &encoded,
                )
                .unwrap();
            serde_json::from_value(decoded).unwrap()
        })
        .collect();
    let sibling = payloads
        .iter()
        .find_map(|payload| match payload {
            DomainEventPayload::MediaRequestApproved(data) if data.request_id == "sibling" => {
                Some(data)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(sibling.decided_by_rule_set_ids, ["sibling-policy"]);
    assert!(sibling.decision_reason_codes.is_empty());
    assert_eq!(sibling.approved_lease_days, Some(5));
    assert_eq!(sibling.policy_tags, ["original-tag"]);
    assert_eq!(
        store
            .resolve_pending_overlapping(&request, resolution)
            .await
            .unwrap()
            .updated,
        0
    );
}

#[tokio::test]
async fn failed_claim_write_keeps_approval_pending_and_retry_creates_all_holds() {
    let store = test_store().await;
    let request = seed_resolution_requests(&store).await;
    let resolution = approval(&request);
    SqlRuntime::execute_write(&store.datastore, "fail_claim_insert",
        "CREATE TRIGGER fail_claim_insert BEFORE INSERT ON lifecycle_claims WHEN NEW.producer_ref = 'sibling' BEGIN SELECT RAISE(ABORT, 'synthetic claim failure'); END", vec![]).await.unwrap();
    let error = store
        .resolve_pending_overlapping(&request, resolution.clone())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("synthetic claim failure"));
    for id in ["primary", "sibling", "forever"] {
        assert_eq!(
            store.get(id).await.unwrap().unwrap().status,
            MediaRequestStatus::Pending
        );
    }
    let claims = crate::media::lifecycle_claims::LifecycleClaimStore::new(store.datastore.clone());
    assert!(
        claims
            .list_for_title("approved-title")
            .await
            .unwrap()
            .is_empty()
    );
    let events = SqlRuntime::fetch_all(
        store.datastore.read_exec(),
        "SELECT event_id FROM domain_events WHERE event_type = 'media_request_approved'",
        &[],
    )
    .await
    .unwrap();
    assert!(events.is_empty());
    SqlRuntime::execute_write(
        &store.datastore,
        "restore_claim_insert",
        "DROP TRIGGER fail_claim_insert",
        vec![],
    )
    .await
    .unwrap();
    store
        .resolve_pending_overlapping(&request, resolution)
        .await
        .unwrap();
    assert_eq!(
        claims.list_for_title("approved-title").await.unwrap().len(),
        3
    );
}

#[tokio::test]
async fn lifecycle_claim_extensions_never_shorten_and_survive_activation() {
    let store = test_store().await;
    let request = seed_resolution_requests(&store).await;
    store
        .resolve_pending_overlapping(&request, approval(&request))
        .await
        .unwrap();
    let claims = crate::media::lifecycle_claims::LifecycleClaimStore::new(store.datastore.clone());
    let rows = claims.list_for_title("approved-title").await.unwrap();
    let finite = rows
        .iter()
        .find(|claim| claim.producer_ref.as_deref() == Some("sibling"))
        .unwrap();
    let permanent = rows
        .iter()
        .find(|claim| claim.producer_ref.as_deref() == Some("forever"))
        .unwrap();
    let now = Utc::now();
    let extension = now + chrono::Duration::days(30);
    claims.extend(&finite.id, extension, now).await.unwrap();
    for invalid in [
        now - chrono::Duration::seconds(1),
        now,
        extension - chrono::Duration::days(1),
        extension,
    ] {
        assert!(claims.extend(&finite.id, invalid, now).await.is_err());
    }
    assert!(claims.extend(&permanent.id, extension, now).await.is_err());
    claims
        .activate(&finite.id, now, Some(now + chrono::Duration::days(5)), now)
        .await
        .unwrap();
    assert_eq!(
        claims.get(&finite.id).await.unwrap().unwrap().expires_at,
        Some(extension)
    );
    claims
        .release_claim(&finite.id, "operator", now)
        .await
        .unwrap();
    assert!(
        claims
            .extend(&finite.id, extension + chrono::Duration::days(1), now)
            .await
            .is_err()
    );
}
