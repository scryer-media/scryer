use sqlx::Row;
use sqlx::sqlite::SqlitePoolOptions;

/// The migration that removes every request rule set.
pub(super) const REQUEST_RULE_ACTIONS_VERSION: i64 = 355;

const SQLITE_MIGRATION: &str =
    include_str!("../../../scryer/src/db/migrations/0355_request_rule_actions.sql");
const POSTGRES_MIGRATION: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0355_request_rule_actions.sql");

async fn pool_before_upgrade() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::replay_source_catalog_for_fresh_install(
        &pool,
        Some(REQUEST_RULE_ACTIONS_VERSION - 1),
        true,
    )
    .await
    .expect("pre-upgrade catalog should apply");
    pool
}

async fn execute(pool: &sqlx::SqlitePool, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(pool)
        .await
        .unwrap();
}

async fn count(pool: &sqlx::SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Request rule sources in the shape the upgrade does not carry over.
const REVISION_ONE: &str = "package scryer.request.user.x\nimport rego.v1\n\
    approve if { input.request.origin == \"manual\" }\n\
    tags contains \"kids\" if { input.request.origin == \"manual\" }\n";
const REVISION_TWO: &str = "package scryer.request.user.x\nimport rego.v1\n\
    manual if { input.request.origin == \"manual\" }\n";

/// A synthetic instance with two request rule sets, one scoped to a library,
/// a decision and the approved request it decided, and one scoring and one
/// lifecycle rule set.
async fn seed(pool: &sqlx::SqlitePool) {
    execute(
        pool,
        "INSERT INTO libraries (id, name, facet, slug, created_at, updated_at)
         VALUES ('lib-1', 'Fixture Library', 'movie', 'fixture-library',
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .await;
    for (id, enabled, mode) in [("rule-a", 1, "enforce"), ("rule-b", 0, "disabled")] {
        execute(
            pool,
            &format!(
                "INSERT INTO request_rule_sets
                    (id, name, description, enabled, evaluation_mode, current_revision_number,
                     created_at, updated_at)
                 VALUES ('{id}', 'Fixture {id}', 'Synthetic', {enabled}, '{mode}', 2,
                     '2026-01-02T03:04:05Z', '2026-01-03T04:05:06Z')"
            ),
        )
        .await;
        for (number, source) in [(1, REVISION_ONE), (2, REVISION_TWO)] {
            execute(
                pool,
                &format!(
                    "INSERT INTO request_rule_revisions
                        (id, rule_set_id, revision_number, rego_source, matcher_content_hash,
                         created_by, created_at)
                     VALUES ('{id}-r{number}', '{id}', {number}, '{source}', 'hash-{number}',
                         'user-1', '2026-01-02T03:04:05Z')"
                ),
            )
            .await;
        }
    }
    execute(
        pool,
        "INSERT INTO request_rule_set_libraries (rule_set_id, library_id, position)
         VALUES ('rule-a', 'lib-1', 0)",
    )
    .await;
    execute(
        pool,
        "INSERT INTO request_rule_decisions
            (id, request_id, mode, effective_outcome, policy_outcome, votes_json, tags_json)
         VALUES ('decision-1', 'request-1', 'enforce', 'auto_approve', 'auto_approve',
             '[{\"rule_set_id\":\"rule-a\",\"rule_set_name\":\"Fixture rule-a\"}]',
             '[\"kids\"]')",
    )
    .await;
    execute(
        pool,
        "INSERT INTO users (id, username) VALUES ('user-1', 'fixture-requester')",
    )
    .await;
    execute(
        pool,
        "INSERT INTO media_requests
            (id, library_id, facet, status, identity_fingerprint, title, created_by_user_id,
             created_at, updated_at, decision_id, decided_by_rule_set_ids)
         VALUES ('request-1', 'lib-1', 'movie', 'approved', 'fingerprint-1', 'Fixture Title',
             'user-1', '2026-01-02T03:04:05Z', '2026-01-02T03:04:05Z', 'decision-1',
             '[\"rule-a\"]')",
    )
    .await;
    execute(
        pool,
        "INSERT INTO rule_sets (id, name, rego_source) VALUES ('scoring-1', 'Scoring', 'package s')",
    )
    .await;
    execute(
        pool,
        "INSERT INTO maintenance_rule_sets (id, name) VALUES ('lifecycle-1', 'Lifecycle')",
    )
    .await;
}

/// Every row the upgrade must keep, as text, so a test can compare before and
/// after: request decisions, media requests, libraries, scoring and lifecycle
/// rule sets.
async fn kept_rows(pool: &sqlx::SqlitePool) -> Vec<String> {
    let mut rows = Vec::new();
    for (table, columns) in [
        (
            "request_rule_decisions",
            "id, request_id, votes_json, tags_json",
        ),
        ("media_requests", "id, decision_id, decided_by_rule_set_ids"),
        ("libraries", "id, name"),
        ("rule_sets", "id, name, rego_source"),
        ("maintenance_rule_sets", "id, name"),
    ] {
        let projection = columns
            .split(", ")
            .map(|column| format!("quote({column})"))
            .collect::<Vec<_>>()
            .join(" || '|' || ");
        let table_rows: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT '{table}|' || {projection} FROM {table} ORDER BY id"
        )))
        .fetch_all(pool)
        .await
        .unwrap();
        rows.extend(table_rows);
    }
    rows
}

async fn request_rule_row_counts(pool: &sqlx::SqlitePool) -> [i64; 3] {
    [
        count(pool, "SELECT COUNT(*) FROM request_rule_sets").await,
        count(pool, "SELECT COUNT(*) FROM request_rule_revisions").await,
        count(pool, "SELECT COUNT(*) FROM request_rule_set_libraries").await,
    ]
}

#[tokio::test]
async fn the_upgrade_removes_request_rule_sets_and_keeps_everything_else() {
    let pool = pool_before_upgrade().await;
    seed(&pool).await;
    assert_eq!(request_rule_row_counts(&pool).await, [2, 4, 1]);
    let kept = kept_rows(&pool).await;
    assert!(kept.iter().any(|row| row.starts_with("media_requests|")));

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade should apply");

    assert_eq!(request_rule_row_counts(&pool).await, [0, 0, 0]);
    assert_eq!(kept_rows(&pool).await, kept, "nothing else changes");

    let decision = sqlx::query(
        "SELECT votes_json, shaping_json FROM request_rule_decisions WHERE id = 'decision-1'",
    )
    .fetch_one(&pool)
    .await
    .expect("the decision is kept");
    assert!(
        decision
            .get::<String, _>("votes_json")
            .contains("Fixture rule-a"),
        "the decision keeps the deleted rule set's name"
    );
    assert_eq!(decision.get::<Option<String>, _>("shaping_json"), None);
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM pragma_table_info('request_rule_sets')
             WHERE name = 'builder_model_json'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_second_run_is_a_no_op_and_keeps_rules_written_after_the_upgrade() {
    let pool = pool_before_upgrade().await;
    seed(&pool).await;
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade should apply");

    execute(
        &pool,
        "INSERT INTO request_rule_sets (id, name, enabled, evaluation_mode)
         VALUES ('rule-new', 'Written after', 1, 'enforce')",
    )
    .await;
    execute(
        &pool,
        "INSERT INTO request_rule_revisions
            (id, rule_set_id, revision_number, rego_source, matcher_content_hash)
         VALUES ('rule-new-r1', 'rule-new', 1, 'package x', 'hash')",
    )
    .await;
    execute(
        &pool,
        "INSERT INTO request_rule_set_libraries (rule_set_id, library_id, position)
         VALUES ('rule-new', 'lib-1', 0)",
    )
    .await;
    let kept = kept_rows(&pool).await;

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("a second run applies nothing");

    assert_eq!(request_rule_row_counts(&pool).await, [1, 1, 1]);
    assert_eq!(kept_rows(&pool).await, kept);
}

/// The deletes alone, on one connection with foreign keys off: removing the
/// children first leaves no revision or scope behind without its rule set.
#[tokio::test]
async fn the_deletes_leave_no_orphan_with_foreign_keys_off() {
    let pool = pool_before_upgrade().await;
    seed(&pool).await;
    let kept = kept_rows(&pool).await;
    execute(&pool, "PRAGMA foreign_keys = OFF").await;
    assert_eq!(count(&pool, "PRAGMA foreign_keys").await, 0);
    for statement in statements(SQLITE_MIGRATION)
        .into_iter()
        .filter(|statement| statement.starts_with("DELETE"))
    {
        execute(&pool, &statement).await;
    }
    assert_eq!(request_rule_row_counts(&pool).await, [0, 0, 0]);
    assert_eq!(kept_rows(&pool).await, kept);
}

/// The statements of one migration file, comments dropped.
fn statements(sql: &str) -> Vec<String> {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .split(';')
        .map(|statement| statement.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|statement| !statement.is_empty())
        .collect()
}

#[test]
fn both_engines_delete_only_request_rule_tables_children_first() {
    for sql in [SQLITE_MIGRATION, POSTGRES_MIGRATION] {
        assert_eq!(
            statements(sql),
            vec![
                "ALTER TABLE request_rule_decisions ADD COLUMN shaping_json TEXT",
                "ALTER TABLE request_rule_sets ADD COLUMN builder_model_json TEXT",
                "DELETE FROM request_rule_set_libraries",
                "DELETE FROM request_rule_revisions",
                "DELETE FROM request_rule_sets",
            ]
        );
    }
    assert_eq!(
        SQLITE_MIGRATION, POSTGRES_MIGRATION,
        "the two engines share one statement set"
    );
}
