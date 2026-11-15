//! User-owned maintenance rules are removed with their history; the
//! system-owned rule and everything outside maintenance history stay.
//! Exercised through the real upgrade runner, with and without foreign-key
//! enforcement, and run twice to prove the second run changes nothing.

use sqlx::SqlitePool;

const SQLITE: &str =
    include_str!("../../../scryer/src/db/migrations/0346_remove_user_maintenance_rules.sql");
const POSTGRES: &str = include_str!(
    "../../../scryer/src/db/postgres/migrations/0346_remove_user_maintenance_rules.sql"
);

/// Version the catalog is replayed to before 0346 is applied.
const PRE_UPGRADE_VERSION: i64 = 345;

const USER_RULE: &str = "user-rule";
const SECOND_USER_RULE: &str = "second-user-rule";
const SYSTEM_RULE: &str = "system-rule";
const TITLE: &str = "title-kept";

/// Tables holding one rule's history, and how a row names its rule.
const HISTORY_COUNTS: &[(&str, &str)] = &[
    (
        "maintenance_rule_revisions",
        "SELECT COUNT(*) FROM maintenance_rule_revisions WHERE rule_set_id = ?1",
    ),
    (
        "maintenance_rule_set_libraries",
        "SELECT COUNT(*) FROM maintenance_rule_set_libraries WHERE rule_set_id = ?1",
    ),
    (
        "maintenance_rule_exclusions",
        "SELECT COUNT(*) FROM maintenance_rule_exclusions WHERE rule_set_id = ?1",
    ),
    (
        "maintenance_evaluation_runs",
        "SELECT COUNT(*) FROM maintenance_evaluation_runs WHERE rule_set_id = ?1",
    ),
    (
        "lifecycle_candidates",
        "SELECT COUNT(*) FROM lifecycle_candidates WHERE rule_set_id = ?1",
    ),
    (
        "lifecycle_action_runs",
        "SELECT COUNT(*) FROM lifecycle_action_runs WHERE candidate_id LIKE ?1 || '-%'",
    ),
    (
        "maintenance_action_steps",
        "SELECT COUNT(*) FROM maintenance_action_steps WHERE candidate_id LIKE ?1 || '-%'",
    ),
    (
        "maintenance_action_step_attempts",
        "SELECT COUNT(*) FROM maintenance_action_step_attempts WHERE candidate_id LIKE ?1 || '-%'",
    ),
    (
        "maintenance_action_job_receipts",
        "SELECT COUNT(*) FROM maintenance_action_job_receipts WHERE candidate_id LIKE ?1 || '-%'",
    ),
    (
        "maintenance_sequence_terminal_memberships",
        "SELECT COUNT(*) FROM maintenance_sequence_terminal_memberships
         WHERE candidate_id LIKE ?1 || '-%'",
    ),
];

async fn pre_upgrade_pool(foreign_keys: bool) -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory SQLite should open");
    crate::migrations::replay_source_catalog_for_fresh_install(
        &pool,
        Some(PRE_UPGRADE_VERSION),
        true,
    )
    .await
    .expect("pre-0346 migration fixture should apply");
    let pragma = if foreign_keys {
        "PRAGMA foreign_keys = ON"
    } else {
        "PRAGMA foreign_keys = OFF"
    };
    sqlx::query(pragma)
        .execute(&pool)
        .await
        .expect("foreign-key mode should set");
    pool
}

async fn exec(pool: &SqlitePool, sql: &str, binds: &[&str]) {
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql.to_string()));
    for bind in binds {
        query = query.bind(*bind);
    }
    query
        .execute(pool)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}

async fn count(pool: &SqlitePool, sql: &str, bind: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .bind(bind)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|error| panic!("count failed: {error}\n{sql}"))
}

async fn seed_unrelated(pool: &SqlitePool) {
    exec(
        pool,
        "INSERT INTO titles (id, name, facet, monitored, created_at, library_id, root_folder_id)
         SELECT ?1, 'Synthetic Kept Film', 'movie', 1, '2026-09-01', l.id, r.id
         FROM libraries l JOIN library_roots r ON r.library_id = l.id
         WHERE l.facet = 'movie' AND l.is_default = 1 AND r.is_default = 1",
        &[TITLE],
    )
    .await;
    exec(
        pool,
        "INSERT INTO lifecycle_claims (id, title_id, producer, kind, state)
         VALUES ('claim-kept', ?1, 'request', 'retain_until', 'active')",
        &[TITLE],
    )
    .await;
    exec(
        pool,
        "INSERT INTO request_rule_sets (id, name) VALUES ('request-rule-kept', 'Kept request rule')",
        &[],
    )
    .await;
    exec(
        pool,
        "INSERT INTO maintenance_rule_exclusions (id, rule_set_id, title_id, subject_id)
         VALUES ('exclusion-global', NULL, ?1, ?1)",
        &[TITLE],
    )
    .await;
}

/// One rule with a row in every table its history reaches.
async fn seed_rule(pool: &SqlitePool, rule: &str, system_owned: bool) {
    exec(
        pool,
        "INSERT INTO maintenance_rule_sets (id, name, system_owned) VALUES (?1, ?1, ?2)",
        &[rule, if system_owned { "1" } else { "0" }],
    )
    .await;
    exec(
        pool,
        "INSERT INTO maintenance_rule_revisions
            (id, rule_set_id, revision_number, rego_source, action_spec, matcher_content_hash)
         VALUES (?1 || '-revision', ?1, 1, 'package rules', '{}', 'hash')",
        &[rule],
    )
    .await;
    exec(
        pool,
        "INSERT INTO maintenance_rule_set_libraries (rule_set_id, library_id, position)
         SELECT ?1, id, 0 FROM libraries WHERE facet = 'movie' AND is_default = 1",
        &[rule],
    )
    .await;
    exec(
        pool,
        "INSERT INTO maintenance_rule_exclusions (id, rule_set_id, title_id, subject_id)
         VALUES (?1 || '-exclusion', ?1, ?2, ?2)",
        &[rule, TITLE],
    )
    .await;
    exec(
        pool,
        "INSERT INTO maintenance_evaluation_runs (id, rule_set_id, revision_number)
         VALUES (?1 || '-run', ?1, 1)",
        &[rule],
    )
    .await;
    // One terminal and one open candidate: terminal history goes too.
    for (suffix, state) in [("open", "pending_action"), ("done", "succeeded")] {
        let candidate = format!("{rule}-{suffix}");
        exec(
            pool,
            "INSERT INTO lifecycle_candidates
                (id, rule_set_id, revision_number, title_id, subject_id, state)
             VALUES (?1, ?2, 1, ?3, ?3, ?4)",
            &[&candidate, rule, TITLE, state],
        )
        .await;
        exec(
            pool,
            "INSERT INTO lifecycle_action_runs
                (id, candidate_id, rule_set_id, revision_number, title_id, action_kind,
                 match_generation, idempotency_key, attempt, status, started_at)
             VALUES (?1 || '-action', ?1, ?2, 1, ?3, 'action_sequence', 1, ?1, 1, 'held',
                 '2026-09-01T00:00:00Z')",
            &[&candidate, rule, TITLE],
        )
        .await;
        exec(
            pool,
            "INSERT INTO maintenance_action_steps
                (candidate_id, match_generation, revision_number, step_id, rule_set_id,
                 title_id, subject_kind, subject_id, sequence_content_hash, step_kind, state,
                 created_at, updated_at)
             VALUES (?1, 1, 1, 'search', ?2, ?3, 'title', ?3, 'hash', 'search', 'held',
                 '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')",
            &[&candidate, rule, TITLE],
        )
        .await;
        exec(
            pool,
            "INSERT INTO maintenance_action_step_attempts
                (id, candidate_id, match_generation, revision_number, step_id, attempt, state,
                 started_at, created_at, updated_at)
             VALUES (?1 || '-attempt', ?1, 1, 1, 'search', 1, 'held',
                 '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')",
            &[&candidate],
        )
        .await;
        exec(
            pool,
            "INSERT INTO maintenance_action_job_receipts
                (candidate_id, match_generation, revision_number, step_id, dispatch_attempt,
                 schema_version, logical_request_key, request_hash, state, created_at,
                 updated_at)
             VALUES (?1, 1, 1, 'search', 1, 1, ?1, 'request-hash', 'accepted',
                 '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')",
            &[&candidate],
        )
        .await;
        exec(
            pool,
            "INSERT INTO maintenance_sequence_terminal_memberships
                (id, candidate_id, rule_set_id, revision_number, matcher_content_hash,
                 title_id, subject_kind, subject_id, match_generation, sequence_content_hash,
                 outcome, created_at)
             VALUES (?1 || '-membership', ?1, ?2, 1, 'hash', ?3, 'title', ?1, 1, 'hash',
                 'succeeded', '2026-09-01T00:00:00Z')",
            &[&candidate, rule, TITLE],
        )
        .await;
    }
}

/// Rows per history table for one rule, in [`HISTORY_COUNTS`] order.
async fn history(pool: &SqlitePool, rule: &str) -> Vec<(&'static str, i64)> {
    let mut rows = Vec::with_capacity(HISTORY_COUNTS.len() + 1);
    rows.push((
        "maintenance_rule_sets",
        count(
            pool,
            "SELECT COUNT(*) FROM maintenance_rule_sets WHERE id = ?1",
            rule,
        )
        .await,
    ));
    for (table, sql) in HISTORY_COUNTS {
        rows.push((*table, count(pool, sql, rule).await));
    }
    rows
}

fn seeded_history() -> Vec<(&'static str, i64)> {
    let mut rows = vec![("maintenance_rule_sets", 1)];
    for (table, _) in HISTORY_COUNTS {
        let per_rule =
            if table.starts_with("maintenance_rule") || *table == "maintenance_evaluation_runs" {
                1
            } else {
                2
            };
        rows.push((*table, per_rule));
    }
    rows
}

async fn assert_unrelated_rows_intact(pool: &SqlitePool, context: &str) {
    for (sql, bind) in [
        ("SELECT COUNT(*) FROM titles WHERE id = ?1", TITLE),
        (
            "SELECT COUNT(*) FROM lifecycle_claims WHERE id = ?1",
            "claim-kept",
        ),
        (
            "SELECT COUNT(*) FROM request_rule_sets WHERE id = ?1",
            "request-rule-kept",
        ),
        (
            "SELECT COUNT(*) FROM maintenance_rule_exclusions WHERE id = ?1",
            "exclusion-global",
        ),
    ] {
        assert_eq!(count(pool, sql, bind).await, 1, "{context}: {sql} {bind}");
    }
}

async fn upgrade(pool: &SqlitePool) {
    crate::migrations::run_migrations(pool, crate::MigrationMode::Apply)
        .await
        .expect("0346 upgrade should apply");
}

async fn run_upgrade(foreign_keys: bool) {
    let context = format!("foreign keys {foreign_keys}");
    let pool = pre_upgrade_pool(foreign_keys).await;
    seed_unrelated(&pool).await;
    for (rule, system_owned) in [
        (USER_RULE, false),
        (SECOND_USER_RULE, false),
        (SYSTEM_RULE, true),
    ] {
        seed_rule(&pool, rule, system_owned).await;
        assert_eq!(
            history(&pool, rule).await,
            seeded_history(),
            "{context}: seeded {rule}"
        );
    }

    upgrade(&pool).await;

    let removed: Vec<_> = seeded_history()
        .into_iter()
        .map(|(table, _)| (table, 0))
        .collect();
    for rule in [USER_RULE, SECOND_USER_RULE] {
        assert_eq!(
            history(&pool, rule).await,
            removed,
            "{context}: {rule} removed"
        );
    }
    assert_eq!(
        history(&pool, SYSTEM_RULE).await,
        seeded_history(),
        "{context}: the system-owned rule keeps all its history"
    );
    assert_unrelated_rows_intact(&pool, &context).await;

    // Replaying the statements on the upgraded database changes nothing.
    sqlx::raw_sql(sqlx::AssertSqlSafe(SQLITE.to_string()))
        .execute(&pool)
        .await
        .expect("a second run applies");
    assert_eq!(
        history(&pool, SYSTEM_RULE).await,
        seeded_history(),
        "{context}: a second run keeps the system-owned rule"
    );
    for rule in [USER_RULE, SECOND_USER_RULE] {
        assert_eq!(history(&pool, rule).await, removed, "{context}: second run");
    }
    assert_unrelated_rows_intact(&pool, &context).await;
    upgrade(&pool).await;
    assert_eq!(
        history(&pool, SYSTEM_RULE).await,
        seeded_history(),
        "{context}"
    );
}

#[tokio::test]
async fn the_upgrade_removes_user_rules_and_their_history_with_foreign_keys_enforced() {
    run_upgrade(true).await;
}

#[tokio::test]
async fn the_upgrade_removes_user_rules_and_their_history_without_foreign_keys() {
    run_upgrade(false).await;
}

/// The Postgres statements are the SQLite ones with Postgres's boolean.
#[test]
fn postgres_statements_match_sqlite() {
    assert_eq!(
        POSTGRES,
        SQLITE.replace("system_owned = 0", "system_owned = false")
    );
}
