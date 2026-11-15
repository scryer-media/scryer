use sqlx::sqlite::SqlitePoolOptions;

const GATE_DEFINITION_ID: &str = "general:system:request_rules.evaluation_enabled";

async fn pool_before_upgrade() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::replay_source_catalog_for_fresh_install(&pool, Some(298), true)
        .await
        .expect("pre-upgrade catalog should apply");
    pool
}

/// Seeds the instance gate as bootstrap wrote it in 0.21.18, with a stored
/// value when the operator had set one.
async fn seed_gate(pool: &sqlx::SqlitePool, value: Option<&str>) {
    sqlx::query(
        "INSERT INTO settings_definitions
            (id, category, scope, key_name, data_type, default_value_json, is_sensitive,
             created_at, updated_at)
         VALUES (?, 'general', 'system', 'request_rules.evaluation_enabled', 'boolean',
             'false', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(GATE_DEFINITION_ID)
    .execute(pool)
    .await
    .unwrap();
    if let Some(value) = value {
        sqlx::query(
            "INSERT INTO settings_values
                (id, setting_definition_id, scope, scope_id, value_json, source,
                 created_at, updated_at)
             VALUES (?, ?, 'system', NULL, ?, 'user',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(format!("{GATE_DEFINITION_ID}:instance"))
        .bind(GATE_DEFINITION_ID)
        .bind(value)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn seed_rule(pool: &sqlx::SqlitePool, id: &str, enabled: bool, mode: &str) {
    sqlx::query(
        "INSERT INTO request_rule_sets (id, name, enabled, evaluation_mode) VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(enabled)
    .bind(mode)
    .execute(pool)
    .await
    .unwrap();
}

async fn rule_state(pool: &sqlx::SqlitePool, id: &str) -> (bool, String) {
    sqlx::query_as("SELECT enabled, evaluation_mode FROM request_rule_sets WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn gate_rows(pool: &sqlx::SqlitePool) -> i64 {
    sqlx::query_scalar(
        "SELECT
            (SELECT COUNT(*) FROM settings_definitions
             WHERE key_name = 'request_rules.evaluation_enabled')
          + (SELECT COUNT(*) FROM settings_values WHERE setting_definition_id = ?)",
    )
    .bind(GATE_DEFINITION_ID)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Stops short of the later upgrade that removes every request rule set, so
/// this upgrade's effect on the rows stays observable.
async fn upgrade(pool: &sqlx::SqlitePool) {
    super::run_upgrade_through(
        pool,
        super::request_rule_actions_upgrade_tests::REQUEST_RULE_ACTIONS_VERSION - 1,
    )
    .await
    .expect("upgrade should apply");
}

#[tokio::test]
async fn an_enforcing_rule_behind_an_off_gate_comes_back_disabled() {
    for gate in [Some("false"), None] {
        let pool = pool_before_upgrade().await;
        seed_gate(&pool, gate).await;
        seed_rule(&pool, "enforcing", true, "enforce").await;

        upgrade(&pool).await;

        assert_eq!(
            rule_state(&pool, "enforcing").await,
            (false, "disabled".to_string()),
            "gate {gate:?}"
        );
        assert_eq!(gate_rows(&pool).await, 0, "the gate is retired");
    }
}

#[tokio::test]
async fn an_enforcing_rule_behind_an_on_gate_is_disabled_without_experimental_opt_in() {
    let pool = pool_before_upgrade().await;
    seed_gate(&pool, Some("true")).await;
    seed_rule(&pool, "enforcing", true, "enforce").await;

    upgrade(&pool).await;

    assert_eq!(
        rule_state(&pool, "enforcing").await,
        (false, "disabled".to_string())
    );
    assert_eq!(gate_rows(&pool).await, 0, "the gate is retired");
}

#[tokio::test]
async fn a_shadow_rule_comes_back_disabled_whatever_the_gate() {
    for gate in [Some("true"), Some("false"), None] {
        let pool = pool_before_upgrade().await;
        seed_gate(&pool, gate).await;
        seed_rule(&pool, "shadowing", true, "shadow").await;

        upgrade(&pool).await;

        assert_eq!(
            rule_state(&pool, "shadowing").await,
            (false, "disabled".to_string()),
            "gate {gate:?}"
        );
    }
}
