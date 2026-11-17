use sqlx::Row;
use sqlx::sqlite::SqlitePoolOptions;

/// The migration that removes every request rule set.
pub(super) const REQUEST_RULE_ACTIONS_VERSION: i64 = 349;

const SQLITE_MIGRATION: &str =
    include_str!("../../../scryer/src/db/migrations/0349_request_rules.sql");
const POSTGRES_MIGRATION: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0349_request_rules.sql");

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

/// Stored decisions whose votes the upgrade reads to link each decision to
/// the rule sets it names: well formed, repeated, empty, not JSON, not an
/// array, and an array mixing usable votes with ones that name nothing.
const DECISION_VOTES: [(&str, &str); 6] = [
    (
        "decision-1",
        "[{\"rule_set_id\":\"rule-a\",\"rule_set_name\":\"Fixture rule-a\"}]",
    ),
    (
        "decision-2",
        "[{\"rule_set_id\":\"rule-a\"},{\"rule_set_id\":\"rule-b\"},{\"rule_set_id\":\"rule-a\"}]",
    ),
    ("decision-empty", "[]"),
    ("decision-invalid", "not json"),
    ("decision-object", "{\"rule_set_id\":\"rule-a\"}"),
    (
        "decision-mixed",
        "[1,\"rule-a\",{\"rule_set_name\":\"no id\"},{\"rule_set_id\":\"\"},\
         {\"rule_set_id\":7},{\"rule_set_id\":\"rule-gone\"}]",
    ),
];

/// The links the upgrade writes for [`DECISION_VOTES`].
fn expected_links() -> Vec<(String, String)> {
    [
        ("decision-1", "rule-a"),
        ("decision-2", "rule-a"),
        ("decision-2", "rule-b"),
        ("decision-mixed", "rule-gone"),
    ]
    .into_iter()
    .map(|(decision, rule_set)| (decision.to_string(), rule_set.to_string()))
    .collect()
}

/// A synthetic instance with two request rule sets, one scoped to a library,
/// decisions and the approved request one of them decided, and one scoring
/// and one lifecycle rule set.
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
    for (id, votes) in DECISION_VOTES {
        sqlx::query(
            "INSERT INTO request_rule_decisions
                (id, request_id, mode, effective_outcome, policy_outcome, votes_json, tags_json)
             VALUES (?, ?, 'enforce', 'auto_approve', 'auto_approve', ?, '[\"kids\"]')",
        )
        .bind(id)
        .bind(format!("request-for-{id}"))
        .bind(votes)
        .execute(pool)
        .await
        .unwrap();
    }
    execute(
        pool,
        "UPDATE request_rule_decisions SET request_id = 'request-1' WHERE id = 'decision-1'",
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
        "INSERT INTO lifecycle_rule_sets (id, name) VALUES ('lifecycle-1', 'Lifecycle')",
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
        ("lifecycle_rule_sets", "id, name"),
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

async fn decision_links(pool: &sqlx::SqlitePool) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT decision_id, rule_set_id FROM request_rule_decision_rule_sets
         ORDER BY decision_id, rule_set_id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn column_present(pool: &sqlx::SqlitePool, table: &str, column: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pragma_table_info(?) WHERE name = ?")
        .bind(table)
        .bind(column)
        .fetch_one(pool)
        .await
        .unwrap()
        == 1
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
    for (table, column, present) in [
        ("request_rule_sets", "builder_model_json", true),
        ("rule_sets", "builder_model_json", true),
        ("request_rule_revisions", "source_content_hash", true),
        ("request_rule_revisions", "matcher_content_hash", false),
        ("request_rule_sets", "evaluation_mode", false),
        ("request_rule_decisions", "mode", false),
    ] {
        assert_eq!(
            column_present(&pool, table, column).await,
            present,
            "{table}.{column}"
        );
    }
}

/// Every stored decision is linked to the rule sets its votes name, once
/// each, whether or not the rule set still exists; votes that are not an
/// array, and votes without a rule set id, link nothing.
#[tokio::test]
async fn the_upgrade_links_each_decision_to_the_rule_sets_its_votes_name() {
    let pool = pool_before_upgrade().await;
    seed(&pool).await;

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade should apply");

    assert_eq!(decision_links(&pool).await, expected_links());
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'idx_request_rule_decision_rule_sets_rule_set'"
        )
        .await,
        1
    );

    // A link goes with its decision.
    assert_eq!(count(&pool, "PRAGMA foreign_keys").await, 1);
    execute(
        &pool,
        "DELETE FROM request_rule_decisions WHERE id = 'decision-2'",
    )
    .await;
    assert_eq!(
        decision_links(&pool).await,
        expected_links()
            .into_iter()
            .filter(|(decision, _)| decision != "decision-2")
            .collect::<Vec<_>>()
    );
}

/// Stored votes holding a `\u0000` escape inside a string are still JSON to
/// SQLite: the decision is linked exactly as the same votes without it.
#[tokio::test]
async fn votes_with_an_escaped_nul_link_as_the_same_votes_without_it() {
    let pool = pool_before_upgrade().await;
    for (id, name) in [
        ("decision-nul", "Fixture\\u0000rule-a"),
        ("decision-plain", "Fixture rule-a"),
    ] {
        sqlx::query(
            "INSERT INTO request_rule_decisions
                (id, request_id, mode, effective_outcome, policy_outcome, votes_json, tags_json)
             VALUES (?, ?, 'enforce', 'auto_approve', 'auto_approve', ?, '[]')",
        )
        .bind(id)
        .bind(format!("request-for-{id}"))
        .bind(format!(
            "[{{\"rule_set_id\":\"rule-a\",\"rule_set_name\":\"{name}\"}},\
             {{\"rule_set_id\":\"rule-c\"}}]"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    let stored: String = sqlx::query_scalar(
        "SELECT votes_json FROM request_rule_decisions WHERE id = 'decision-nul'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(stored.contains("\\u0000"), "the fixture stores the escape");

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade should apply");

    let links = decision_links(&pool).await;
    let rule_sets_of = |decision: &str| {
        links
            .iter()
            .filter(|(linked, _)| linked == decision)
            .map(|(_, rule_set)| rule_set.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(rule_sets_of("decision-plain"), ["rule-a", "rule-c"]);
    assert_eq!(rule_sets_of("decision-nul"), rule_sets_of("decision-plain"));
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
        "INSERT INTO request_rule_sets (id, name, enabled)
         VALUES ('rule-new', 'Written after', 1)",
    )
    .await;
    execute(
        &pool,
        "INSERT INTO request_rule_revisions
            (id, rule_set_id, revision_number, rego_source, source_content_hash)
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
    let links = decision_links(&pool).await;

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("a second run applies nothing");

    assert_eq!(request_rule_row_counts(&pool).await, [1, 1, 1]);
    assert_eq!(kept_rows(&pool).await, kept);
    assert_eq!(decision_links(&pool).await, links);
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

/// Both engines delete only the request rule tables, children first, and
/// share every statement before the backfill, which each engine writes in
/// its own JSON dialect.
#[test]
fn both_engines_delete_only_request_rule_tables_children_first() {
    for sql in [SQLITE_MIGRATION, POSTGRES_MIGRATION] {
        assert_eq!(
            statements(sql)
                .into_iter()
                .filter(|statement| statement.starts_with("DELETE"))
                .collect::<Vec<_>>(),
            vec![
                "DELETE FROM request_rule_set_libraries",
                "DELETE FROM request_rule_revisions",
                "DELETE FROM request_rule_sets",
            ]
        );
    }
    let before_backfill = |sql: &'static str| {
        sql.split_once("-- Filled from every stored decision's votes.")
            .expect("the backfill is marked")
            .0
    };
    assert_eq!(
        before_backfill(SQLITE_MIGRATION),
        before_backfill(POSTGRES_MIGRATION),
        "the two engines share every statement before the backfill"
    );
}
