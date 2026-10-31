use super::*;
use sqlx::sqlite::SqlitePoolOptions;

async fn pool() -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect(":memory:")
        .await
        .unwrap()
}

async fn state(pool: &SqlitePool) -> Option<String> {
    sqlx::query_scalar(
        "SELECT value_json FROM settings_values WHERE id = 'initial-authentication-setup'",
    )
    .fetch_optional(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn initial_auth_seed_survives_interrupted_startup_and_restart() {
    let pool = pool().await;
    let catalog = embedded_catalog().unwrap();
    let payload = embedded_payload_bytes().unwrap();
    let baseline = catalog
        .latest_baseline_at_or_below(catalog.max_version(), EngineScope::Sqlite)
        .unwrap();
    {
        let mut conn = pool.acquire().await.unwrap();
        ensure_migration_ledger_shape(&mut conn).await.unwrap();
        apply_baseline(&mut conn, &catalog, &payload, baseline, true)
            .await
            .unwrap();
    }
    // Simulate stopping after the initial transaction, before later migrations.
    assert_eq!(
        state(&pool).await.as_deref(),
        Some("{\"stage\":\"choice\"}")
    );
    run_migrations(&pool, MigrationMode::Apply).await.unwrap();
    assert_eq!(
        state(&pool).await.as_deref(),
        Some("{\"stage\":\"choice\"}")
    );
    sqlx::query("UPDATE settings_values SET value_json = '{\"stage\":\"complete\"}' WHERE id = 'initial-authentication-setup'")
        .execute(&pool).await.unwrap();
    run_migrations(&pool, MigrationMode::Apply).await.unwrap();
    assert_eq!(
        state(&pool).await.as_deref(),
        Some("{\"stage\":\"complete\"}")
    );
}

#[tokio::test]
async fn initial_auth_schema_and_requirement_roll_back_together() {
    let pool = pool().await;
    let catalog = embedded_catalog().unwrap();
    let payload = embedded_payload_bytes().unwrap();
    let baseline = catalog
        .latest_baseline_at_or_below(catalog.max_version(), EngineScope::Sqlite)
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    ensure_migration_ledger_shape(&mut conn).await.unwrap();
    // A duplicate ledger row fails after schema and setup-state insertion.
    let mut tx = conn.begin().await.unwrap();
    insert_applied_migration(&mut tx, &catalog.migrations[0], 0)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        apply_baseline(&mut conn, &catalog, &payload, baseline, true)
            .await
            .is_err()
    );
    assert_eq!(app_object_count(&mut conn).await.unwrap(), 0);
}

#[tokio::test]
async fn initial_auth_historical_database_upgrade_does_not_require_a_choice() {
    let pool = pool().await;
    let catalog = embedded_catalog().unwrap();
    let payload = embedded_payload_bytes().unwrap();
    replay_catalog_into_fresh_db(
        &pool,
        &catalog,
        &payload,
        Some(catalog.max_version() - 1),
        true,
    )
    .await
    .unwrap();
    assert_eq!(state(&pool).await, None);
    run_migrations(&pool, MigrationMode::Apply).await.unwrap();
    assert_eq!(state(&pool).await, None);
}
