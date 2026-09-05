//! Round-trips for the lifecycle claim store (spec 0003 FR-041..FR-044).
//!
//! The interesting behaviour is not the insert: it is that every transition is
//! conditional. A replayed import must not restart a spent window, a release
//! must not resurrect a lapsed hold, and the partial unique index must refuse a
//! second live claim for the same request.

use super::*;
use scryer_application::LifecycleClaimRepository;
use scryer_domain::{
    LifecycleClaim, LifecycleClaimKind, LifecycleClaimProducer, LifecycleClaimState,
};

fn claim(id: &str, title_id: &str, producer_ref: Option<&str>) -> LifecycleClaim {
    let now = Utc::now();
    LifecycleClaim {
        id: id.to_string(),
        title_id: title_id.to_string(),
        library_id: "library-1".to_string(),
        producer: LifecycleClaimProducer::RequestLease,
        producer_ref: producer_ref.map(str::to_string),
        kind: LifecycleClaimKind::RetainUntil,
        state: LifecycleClaimState::Dormant,
        duration_days: Some(14),
        starts_at: None,
        expires_at: None,
        created_by: Some("user-1".to_string()),
        created_at: now,
        updated_at: now,
        released_reason: None,
    }
}

fn keep(id: &str, title_id: &str, producer_ref: Option<&str>) -> LifecycleClaim {
    let now = Utc::now();
    LifecycleClaim {
        id: id.to_string(),
        title_id: title_id.to_string(),
        library_id: "library-1".to_string(),
        producer: LifecycleClaimProducer::RequestPermanent,
        producer_ref: producer_ref.map(str::to_string),
        kind: LifecycleClaimKind::Keep,
        state: LifecycleClaimState::Active,
        duration_days: None,
        starts_at: Some(now),
        expires_at: None,
        created_by: Some("user-1".to_string()),
        created_at: now,
        updated_at: now,
        released_reason: None,
    }
}

async fn assert_claim_round_trip(store: &dyn LifecycleClaimRepository) -> AppResult<()> {
    store
        .create(&claim("claim-1", "title-1", Some("request-1")))
        .await?;

    let loaded = store.get("claim-1").await?.expect("claim should exist");
    assert_eq!(loaded.title_id, "title-1");
    assert_eq!(loaded.library_id, "library-1");
    assert_eq!(loaded.producer, LifecycleClaimProducer::RequestLease);
    assert_eq!(loaded.producer_ref.as_deref(), Some("request-1"));
    assert_eq!(loaded.kind, LifecycleClaimKind::RetainUntil);
    assert_eq!(loaded.state, LifecycleClaimState::Dormant);
    assert_eq!(loaded.duration_days, Some(14));
    assert!(loaded.starts_at.is_none());
    assert!(loaded.expires_at.is_none());
    assert!(
        loaded.is_live(),
        "a dormant claim still holds: the title simply has not imported yet"
    );

    assert_eq!(
        store.list_dormant(10).await?.len(),
        1,
        "the dormant claim is what the activation pass looks for"
    );

    // The partial unique index refuses a second live claim for the same
    // producing request.
    store
        .create(&claim("claim-1-dup", "title-1", Some("request-1")))
        .await
        .expect_err("a second live claim for the same request must be refused");

    // Two operator pins carry no producer_ref, so the index does not apply.
    let mut pin_a = keep("pin-a", "title-1", None);
    pin_a.producer = LifecycleClaimProducer::OperatorKeep;
    let mut pin_b = keep("pin-b", "title-1", None);
    pin_b.producer = LifecycleClaimProducer::OperatorKeep;
    store.create(&pin_a).await?;
    store.create(&pin_b).await?;

    let starts_at = Utc::now() - chrono::Duration::days(20);
    let expires_at = starts_at + chrono::Duration::days(14);
    let activated_at = Utc::now();
    store
        .activate("claim-1", starts_at, Some(expires_at), activated_at)
        .await?;
    let activated = store.get("claim-1").await?.expect("claim should exist");
    assert_eq!(activated.state, LifecycleClaimState::Active);
    assert!(activated.starts_at.is_some());
    assert!(
        activated.updated_at.timestamp() >= activated_at.timestamp() - 1,
        "updated_at is the write time, not the backdated start the sweep supplied"
    );

    // A replayed import must not restart a window the requester already spent.
    let replay_starts_at = Utc::now();
    store
        .activate(
            "claim-1",
            replay_starts_at,
            Some(replay_starts_at),
            replay_starts_at,
        )
        .await?;
    let unchanged = store.get("claim-1").await?.expect("claim should exist");
    assert_eq!(
        unchanged.expires_at.map(|value| value.timestamp()),
        activated.expires_at.map(|value| value.timestamp()),
        "activation is conditional on dormancy"
    );

    let live = store
        .list_live_for_titles(&["title-1".to_string(), "title-missing".to_string()])
        .await?;
    assert_eq!(
        live.get("title-1").map(Vec::len),
        Some(3),
        "the lease and both pins are live"
    );
    assert!(
        !live.contains_key("title-missing"),
        "a title with no live claim is absent, not empty"
    );

    assert_eq!(store.list_for_title("title-1").await?.len(), 3);

    // Extending a live claim moves the date; the pins are unaffected.
    let extended_to = Utc::now() + chrono::Duration::days(7);
    store.extend("claim-1", extended_to, Utc::now()).await?;
    assert_eq!(
        store
            .get("claim-1")
            .await?
            .expect("claim should exist")
            .expires_at
            .map(|value| value.timestamp()),
        Some(extended_to.timestamp())
    );

    // Expiry only touches active retention claims whose window has elapsed.
    let past = Utc::now() - chrono::Duration::days(1);
    store.extend("claim-1", past, Utc::now()).await?;
    assert_eq!(store.expire_due(Utc::now()).await?, 1);
    assert_eq!(
        store.get("claim-1").await?.expect("claim exists").state,
        LifecycleClaimState::Expired
    );
    assert_eq!(
        store.get("pin-a").await?.expect("pin exists").state,
        LifecycleClaimState::Active,
        "a keep has no expiry and must never expire"
    );

    // An expired lease cannot be extended back into force: that is a new claim.
    store
        .extend(
            "claim-1",
            Utc::now() + chrono::Duration::days(30),
            Utc::now(),
        )
        .await
        .expect_err("an expired claim must not be extendable");

    // The fact builder's read: retention claims only, live and expired, so
    // "this lease ran out" is distinguishable from "there never was one".
    let history = store
        .list_retention_history_for_titles(&["title-1".to_string(), "title-missing".to_string()])
        .await?;
    let title_history = history.get("title-1").expect("the lapsed lease is history");
    assert_eq!(
        title_history
            .iter()
            .map(|claim| claim.id.as_str())
            .collect::<Vec<_>>(),
        vec!["claim-1"],
        "the two keeps are live but are not leases, so they are not lease history"
    );
    assert_eq!(title_history[0].state, LifecycleClaimState::Expired);
    assert!(
        !history.contains_key("title-missing"),
        "a title with no retention history is absent, not empty"
    );

    Ok(())
}

async fn assert_release_and_convert(store: &dyn LifecycleClaimRepository) -> AppResult<()> {
    store
        .create(&claim("claim-r", "title-2", Some("request-2")))
        .await?;
    store
        .create(&claim("claim-s", "title-3", Some("request-3")))
        .await?;

    let now = Utc::now();
    assert_eq!(
        store
            .release_for_producer_ref(
                LifecycleClaimProducer::RequestLease,
                "request-2",
                "request_canceled",
                now,
            )
            .await?,
        1
    );
    let released = store.get("claim-r").await?.expect("claim exists");
    assert_eq!(released.state, LifecycleClaimState::Released);
    assert_eq!(
        released.released_reason.as_deref(),
        Some("request_canceled")
    );
    assert!(
        !released.is_live(),
        "a released claim holds nothing but stays as history"
    );
    // Releasing again is a no-op: the claim is no longer live.
    assert_eq!(
        store
            .release_for_producer_ref(
                LifecycleClaimProducer::RequestLease,
                "request-2",
                "request_canceled",
                now,
            )
            .await?,
        0
    );
    // Releasing frees the (producer, producer_ref) slot for a fresh claim.
    store
        .create(&claim("claim-r2", "title-2", Some("request-2")))
        .await?;

    assert_eq!(
        store
            .release_for_title("title-3", "title_deleted", now)
            .await?,
        1
    );
    assert_eq!(
        store.get("claim-s").await?.expect("claim exists").state,
        LifecycleClaimState::Released
    );

    // Conversion marks the lease converted and inserts the keep in one write.
    store
        .create(&claim("claim-c", "title-4", Some("request-4")))
        .await?;
    let mut replacement = keep("claim-c-keep", "title-4", Some("request-4"));
    replacement.producer = LifecycleClaimProducer::RequestPermanent;
    store
        .convert_to_permanent("claim-c", &replacement, Utc::now())
        .await?;
    let converted = store.get("claim-c").await?.expect("claim exists");
    assert_eq!(converted.state, LifecycleClaimState::Converted);
    assert_eq!(
        converted.released_reason.as_deref(),
        Some("converted_to_permanent")
    );
    let keep_claim = store.get("claim-c-keep").await?.expect("keep exists");
    assert_eq!(keep_claim.kind, LifecycleClaimKind::Keep);
    assert!(keep_claim.is_live());

    // A released or converted lease is withdrawn, not spent: it must never
    // reach the fact builder, or a title whose request was canceled would read
    // as one whose lease expired.
    let history = store
        .list_retention_history_for_titles(&["title-3".to_string(), "title-4".to_string()])
        .await?;
    assert!(
        !history.contains_key("title-3"),
        "a released lease is not lease history the facts may read"
    );
    assert!(
        !history.contains_key("title-4"),
        "a converted lease is not lease history the facts may read"
    );

    // A claim that is no longer live cannot be converted twice.
    store
        .convert_to_permanent(
            "claim-c",
            &keep("claim-c-keep-2", "title-4", None),
            Utc::now(),
        )
        .await
        .expect_err("a converted claim must not convert again");
    assert!(
        store.get("claim-c-keep-2").await?.is_none(),
        "the failed conversion must not have inserted its replacement"
    );

    // Releasing one claim by id — the administrator's explicit withdrawal —
    // touches exactly that row and, like every other transition here, is
    // conditional on the claim still being live.
    store
        .create(&claim("claim-x", "title-5", Some("request-5")))
        .await?;
    store
        .create(&claim("claim-y", "title-5", Some("request-6")))
        .await?;
    assert_eq!(store.release_claim("claim-x", "operator", now).await?, 1);
    let released = store.get("claim-x").await?.expect("claim exists");
    assert_eq!(released.state, LifecycleClaimState::Released);
    assert_eq!(released.released_reason.as_deref(), Some("operator"));
    assert!(
        store.get("claim-y").await?.expect("claim exists").is_live(),
        "releasing one claim must not touch its neighbour on the same title"
    );
    assert_eq!(
        store.release_claim("claim-x", "operator", now).await?,
        0,
        "a claim that is already terminal cannot be released again"
    );
    Ok(())
}

#[tokio::test]
async fn lifecycle_claims_round_trip() {
    let (services, db) = temp_services("scryer_lifecycle_claims").await;
    let store = crate::LifecycleClaimStore::new(services.datastore());
    assert_claim_round_trip(&store)
        .await
        .expect("claims should round-trip");
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn lifecycle_claims_release_and_convert() {
    let (services, db) = temp_services("scryer_lifecycle_claims_release").await;
    let store = crate::LifecycleClaimStore::new(services.datastore());
    assert_release_and_convert(&store)
        .await
        .expect("claims should release and convert");
    let _ = std::fs::remove_file(db);
}

/// `count_live_for_user` joins through the producing request, so it needs real
/// `media_requests` rows rather than the bare claim table.
#[tokio::test]
async fn lifecycle_claims_count_live_for_user() {
    use crate::tests::media_request_requesters::{
        new_request, request_store, seed_library, seed_user, user,
    };
    use scryer_application::MediaRequestRepository;

    let (services, db) = temp_services("scryer_lifecycle_claims_user").await;
    seed_library(&services, "library-1").await;
    seed_user(&services, "requester-1").await;
    seed_user(&services, "requester-2").await;

    let requests = request_store(&services);
    for (request_id, submitter) in [("request-1", "requester-1"), ("request-2", "requester-2")] {
        requests
            .submit(
                new_request(request_id, "library-1", submitter),
                &user(submitter),
                crate::tests::media_request_requesters::request_event(request_id),
            )
            .await
            .expect("submit request");
    }

    let store = crate::LifecycleClaimStore::new(services.datastore());
    store
        .create(&claim("claim-u1", "title-1", Some("request-1")))
        .await
        .expect("create claim");
    store
        .create(&claim("claim-u2", "title-2", Some("request-2")))
        .await
        .expect("create claim");

    assert_eq!(
        store.count_live_for_user("requester-1").await.unwrap(),
        1,
        "only the claims produced by this user's own requests count"
    );

    store
        .release_for_producer_ref(
            LifecycleClaimProducer::RequestLease,
            "request-1",
            "request_canceled",
            Utc::now(),
        )
        .await
        .expect("release claim");
    assert_eq!(
        store.count_live_for_user("requester-1").await.unwrap(),
        0,
        "a released claim is no longer a live lease"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn lifecycle_claims_round_trip_postgres() -> AppResult<()> {
    let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        eprintln!("skipping PostgreSQL lifecycle claim test; SCRYER_TEST_POSTGRES_URL is not set");
        return Ok(());
    };

    let admin_pool = sqlx::PgPool::connect(&raw_url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!(
        "scryer_test_{}_{}",
        std::process::id(),
        Id::new().0.replace('-', "_")
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| AppError::Repository(format!("failed to create schema: {error}")))?;

    let result = async {
        let mut url = url::Url::parse(&raw_url)
            .map_err(|error| AppError::Validation(format!("invalid postgres test URL: {error}")))?;
        url.query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));
        let services =
            crate::PostgresServices::new_with_mode(url.to_string(), crate::MigrationMode::Apply)
                .await?;
        let store = crate::LifecycleClaimStore::new(services.datastore());
        let result = async {
            assert_claim_round_trip(&store).await?;
            assert_release_and_convert(&store).await
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
