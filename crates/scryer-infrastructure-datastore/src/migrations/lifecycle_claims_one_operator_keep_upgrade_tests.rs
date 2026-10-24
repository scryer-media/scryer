//! A database from before the lifecycle claim migrations (0303, 0307, 0308)
//! upgrades to claims that are only a time to delete. 0303 collapsed
//! duplicate operator keeps, 0307 then removed operator keeps outright, and
//! 0308 removed every other keep and every lease with no time to delete. What
//! an upgraded installation is left with is the dated leases, untouched.

use sqlx::sqlite::SqlitePoolOptions;

async fn pool_before_upgrade() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::replay_source_catalog_for_fresh_install(&pool, Some(302), true)
        .await
        .expect("pre-upgrade catalog should apply");
    pool
}

struct Claim<'a> {
    id: &'a str,
    title_id: &'a str,
    producer: &'a str,
    kind: &'a str,
    state: &'a str,
    duration_days: Option<i64>,
    expires_at: Option<&'a str>,
}

async fn seed_claim(pool: &sqlx::SqlitePool, claim: Claim<'_>) {
    sqlx::query(
        "INSERT INTO lifecycle_claims
            (id, title_id, producer, kind, state, duration_days, expires_at,
             created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(claim.id)
    .bind(claim.title_id)
    .bind(claim.producer)
    .bind(claim.kind)
    .bind(claim.state)
    .bind(claim.duration_days)
    .bind(claim.expires_at)
    .execute(pool)
    .await
    .unwrap();
}

async fn remaining_claim_ids(pool: &sqlx::SqlitePool) -> Vec<String> {
    sqlx::query_scalar("SELECT id FROM lifecycle_claims ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn claim_row(
    pool: &sqlx::SqlitePool,
    id: &str,
) -> (String, String, Option<i64>, Option<String>) {
    sqlx::query_as(
        "SELECT kind, state, duration_days, expires_at FROM lifecycle_claims WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn upgrade_removes_every_keep_and_leaves_dated_leases_untouched() {
    let pool = pool_before_upgrade().await;
    let keep = |id, title_id, producer, state| Claim {
        id,
        title_id,
        producer,
        kind: "keep",
        state,
        duration_days: None,
        expires_at: None,
    };
    // Duplicate operator keeps on one title, in every state 0303 looked at,
    // plus one on another title.
    seed_claim(
        &pool,
        keep("keep-old", "title-1", "operator_keep", "active"),
    )
    .await;
    seed_claim(
        &pool,
        keep("keep-new", "title-1", "operator_keep", "active"),
    )
    .await;
    seed_claim(
        &pool,
        keep("keep-dormant", "title-1", "operator_keep", "dormant"),
    )
    .await;
    seed_claim(
        &pool,
        keep("keep-gone", "title-1", "operator_keep", "released"),
    )
    .await;
    seed_claim(
        &pool,
        keep("keep-other", "title-2", "operator_keep", "active"),
    )
    .await;
    // A request approved forever.
    seed_claim(
        &pool,
        keep("request-forever", "title-1", "request_permanent", "active"),
    )
    .await;
    // A keep from some other producer is still a keep.
    seed_claim(&pool, keep("keep-by-rule", "title-3", "request", "active")).await;
    // A lease with no time to delete: no duration and no explicit date.
    seed_claim(
        &pool,
        Claim {
            id: "lease-zero-day",
            title_id: "title-3",
            producer: "request",
            kind: "retain_until",
            state: "active",
            duration_days: Some(0),
            expires_at: None,
        },
    )
    .await;
    // Leases that carry a time to delete.
    seed_claim(
        &pool,
        Claim {
            id: "lease-duration",
            title_id: "title-1",
            producer: "request",
            kind: "retain_until",
            state: "dormant",
            duration_days: Some(30),
            expires_at: None,
        },
    )
    .await;
    seed_claim(
        &pool,
        Claim {
            id: "lease-extended",
            title_id: "title-2",
            producer: "request",
            kind: "retain_until",
            state: "active",
            duration_days: None,
            expires_at: Some("2027-01-01T00:00:00Z"),
        },
    )
    .await;

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade should apply");

    assert_eq!(
        remaining_claim_ids(&pool).await,
        vec!["lease-duration".to_string(), "lease-extended".to_string()],
        "every keep and every lease without a time to delete is gone"
    );
    assert_eq!(
        claim_row(&pool, "lease-duration").await,
        ("retain_until".into(), "dormant".into(), Some(30), None)
    );
    assert_eq!(
        claim_row(&pool, "lease-extended").await,
        (
            "retain_until".into(),
            "active".into(),
            None,
            Some("2027-01-01T00:00:00Z".into())
        )
    );
}
