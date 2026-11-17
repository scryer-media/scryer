//! Lifecycle rules name their level in their action heads and read every
//! storage root, so neither a rule-set subject kind nor a revision storage
//! root is stored. Every user rule, whatever its scope or root, is removed
//! with its history; a system-owned rule keeps everything whatever its stored
//! scope or root, and only the two columns go. Exercised through the real
//! upgrade runner, with and without foreign-key enforcement, and run twice to
//! prove the second run changes nothing.

use sqlx::SqlitePool;

const SQLITE: &str = include_str!("../../../scryer/src/db/migrations/0348_lifecycle_rules.sql");
const POSTGRES: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0348_lifecycle_rules.sql");

/// Version the catalog is replayed to before 0348 is applied.
const PRE_UPGRADE_VERSION: i64 = 347;

const TITLE_RULE: &str = "rule-title-kept";
const SEASON_RULE: &str = "rule-season-scoped";
const EPISODE_RULE: &str = "rule-episode-scoped";
const ROOT_RULE: &str = "rule-root-confined";
const SYSTEM_RULE: &str = "rule-system";
const SYSTEM_SEASON_RULE: &str = "rule-season-system";
const TITLE: &str = "title-synthetic-quillmoor";

/// Tables holding one rule's history, under their names after the upgrade,
/// and how a row names its rule.
const HISTORY_COUNTS: &[(&str, &str)] = &[
    (
        "lifecycle_rule_sets",
        "SELECT COUNT(*) FROM lifecycle_rule_sets WHERE id = ?1",
    ),
    (
        "lifecycle_rule_revisions",
        "SELECT COUNT(*) FROM lifecycle_rule_revisions WHERE rule_set_id = ?1",
    ),
    (
        "lifecycle_rule_set_libraries",
        "SELECT COUNT(*) FROM lifecycle_rule_set_libraries WHERE rule_set_id = ?1",
    ),
    (
        "lifecycle_rule_exclusions",
        "SELECT COUNT(*) FROM lifecycle_rule_exclusions WHERE rule_set_id = ?1",
    ),
    (
        "lifecycle_evaluation_runs",
        "SELECT COUNT(*) FROM lifecycle_evaluation_runs WHERE rule_set_id = ?1",
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
        "lifecycle_action_steps",
        "SELECT COUNT(*) FROM lifecycle_action_steps WHERE candidate_id LIKE ?1 || '-%'",
    ),
    (
        "lifecycle_action_step_attempts",
        "SELECT COUNT(*) FROM lifecycle_action_step_attempts WHERE candidate_id LIKE ?1 || '-%'",
    ),
    (
        "lifecycle_action_job_receipts",
        "SELECT COUNT(*) FROM lifecycle_action_job_receipts WHERE candidate_id LIKE ?1 || '-%'",
    ),
    (
        "lifecycle_sequence_terminal_memberships",
        "SELECT COUNT(*) FROM lifecycle_sequence_terminal_memberships
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
    .expect("pre-0348 migration fixture should apply");
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

/// A statement under the table names before the upgrade: the upgrade
/// renames every `maintenance_` rule table to the same name with
/// `lifecycle_`, and the candidate and action run tables keep theirs.
fn before_upgrade(sql: &str) -> String {
    [
        "rule_sets",
        "rule_revisions",
        "rule_set_libraries",
        "rule_exclusions",
        "evaluation_runs",
        "action_steps",
        "action_step_attempts",
        "action_job_receipts",
        "sequence_terminal_memberships",
    ]
    .iter()
    .fold(sql.to_string(), |sql, table| {
        sql.replace(
            &format!("lifecycle_{table} "),
            &format!("maintenance_{table} "),
        )
        .replace(
            &format!("lifecycle_{table}\n"),
            &format!("maintenance_{table}\n"),
        )
    })
}

/// Runs a seed statement written with the names after the upgrade against
/// the database before it.
async fn exec(pool: &SqlitePool, sql: &str, binds: &[&str]) {
    let sql = before_upgrade(sql);
    let sql = sql.as_str();
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
         SELECT ?1, 'Synthetic Quillmoor', 'series', 1, '2026-09-01', l.id, r.id
         FROM libraries l JOIN library_roots r ON r.library_id = l.id
         WHERE l.facet = 'series' AND l.is_default = 1 AND r.is_default = 1",
        &[TITLE],
    )
    .await;
    exec(
        pool,
        "INSERT INTO lifecycle_rule_exclusions (id, rule_set_id, title_id, subject_kind, subject_id)
         VALUES ('exclusion-global', NULL, ?1, 'season', 'season-synthetic')",
        &[TITLE],
    )
    .await;
}

/// One rule with a row in every table its history reaches. `roots` holds the
/// storage root of each revision in order; the last one is current.
async fn seed_rule(
    pool: &SqlitePool,
    rule: &str,
    subject_kind: &str,
    roots: &[Option<&str>],
    system_owned: bool,
) {
    let current = roots.len().to_string();
    exec(
        pool,
        "INSERT INTO lifecycle_rule_sets (id, name, subject_kind, system_owned, current_revision_number)
         VALUES (?1, ?1, ?2, ?3, ?4)",
        &[
            rule,
            subject_kind,
            if system_owned { "1" } else { "0" },
            &current,
        ],
    )
    .await;
    for (index, root) in roots.iter().enumerate() {
        let number = (index + 1).to_string();
        let mut query = sqlx::query(
            "INSERT INTO maintenance_rule_revisions
                (id, rule_set_id, revision_number, rego_source, action_spec,
                 matcher_content_hash, storage_root_id)
             VALUES (?1 || '-revision-' || ?2, ?1, ?2, 'package rules', '{}', 'hash', ?3)",
        )
        .bind(rule)
        .bind(&number);
        query = query.bind(*root);
        query.execute(pool).await.expect("revision seed");
    }
    exec(
        pool,
        "INSERT INTO lifecycle_rule_set_libraries (rule_set_id, library_id, position)
         SELECT ?1, id, 0 FROM libraries WHERE facet = 'series' AND is_default = 1",
        &[rule],
    )
    .await;
    exec(
        pool,
        "INSERT INTO lifecycle_rule_exclusions (id, rule_set_id, title_id, subject_kind, subject_id)
         VALUES (?1 || '-exclusion', ?1, ?2, ?3, ?1 || '-subject')",
        &[rule, TITLE, subject_kind],
    )
    .await;
    exec(
        pool,
        "INSERT INTO lifecycle_evaluation_runs (id, rule_set_id, revision_number)
         VALUES (?1 || '-run', ?1, 1)",
        &[rule],
    )
    .await;
    // A terminal candidate and one executing a checkpointed step: the
    // checkpoint goes with the rule, so it can never resume under the new
    // reading.
    for (suffix, state) in [("executing", "executing"), ("done", "succeeded")] {
        let candidate = format!("{rule}-{suffix}");
        let subject = format!("{candidate}-subject");
        exec(
            pool,
            "INSERT INTO lifecycle_candidates
                (id, rule_set_id, revision_number, title_id, subject_kind, subject_id, state)
             VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6)",
            &[&candidate, rule, TITLE, subject_kind, &subject, state],
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
            "INSERT INTO lifecycle_action_steps
                (candidate_id, match_generation, revision_number, step_id, rule_set_id,
                 title_id, subject_kind, subject_id, sequence_content_hash, step_kind, state,
                 target_identity_json, created_at, updated_at)
             VALUES (?1, 1, 1, 'delete_files', ?2, ?3, ?4, ?5, 'hash', 'delete_files',
                 'running', '{\"storage_root_id\":\"root-synthetic\"}',
                 '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')",
            &[&candidate, rule, TITLE, subject_kind, &subject],
        )
        .await;
        exec(
            pool,
            "INSERT INTO lifecycle_action_step_attempts
                (id, candidate_id, match_generation, revision_number, step_id, attempt, state,
                 started_at, created_at, updated_at)
             VALUES (?1 || '-attempt', ?1, 1, 1, 'delete_files', 1, 'running',
                 '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')",
            &[&candidate],
        )
        .await;
        exec(
            pool,
            "INSERT INTO lifecycle_action_job_receipts
                (candidate_id, match_generation, revision_number, step_id, dispatch_attempt,
                 schema_version, logical_request_key, request_hash, state, created_at,
                 updated_at)
             VALUES (?1, 1, 1, 'delete_files', 1, 1, ?1, 'request-hash', 'accepted',
                 '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')",
            &[&candidate],
        )
        .await;
        exec(
            pool,
            "INSERT INTO lifecycle_sequence_terminal_memberships
                (id, candidate_id, rule_set_id, revision_number, matcher_content_hash,
                 title_id, subject_kind, subject_id, match_generation, sequence_content_hash,
                 outcome, created_at)
             VALUES (?1 || '-membership', ?1, ?2, 1, 'hash', ?3, ?4, ?5, 1, 'hash',
                 'succeeded', '2026-09-01T00:00:00Z')",
            &[&candidate, rule, TITLE, subject_kind, &subject],
        )
        .await;
    }
}

async fn history(pool: &SqlitePool, rule: &str, upgraded: bool) -> Vec<(&'static str, i64)> {
    let mut rows = Vec::with_capacity(HISTORY_COUNTS.len());
    for (table, sql) in HISTORY_COUNTS {
        let sql = if upgraded {
            sql.to_string()
        } else {
            before_upgrade(sql)
        };
        rows.push((*table, count(pool, &sql, rule).await));
    }
    rows
}

fn seeded_history(revisions: i64) -> Vec<(&'static str, i64)> {
    HISTORY_COUNTS
        .iter()
        .map(|(table, _)| {
            let rows = match *table {
                "lifecycle_rule_revisions" => revisions,
                "lifecycle_rule_sets"
                | "lifecycle_rule_set_libraries"
                | "lifecycle_rule_exclusions"
                | "lifecycle_evaluation_runs" => 1,
                _ => 2,
            };
            (*table, rows)
        })
        .collect()
}

fn removed() -> Vec<(&'static str, i64)> {
    HISTORY_COUNTS
        .iter()
        .map(|(table, _)| (*table, 0))
        .collect()
}

/// Every seeded rule, its revisions' roots, and whether the upgrade keeps it.
const RULES: &[(&str, &str, &[Option<&str>], bool, bool)] = &[
    (TITLE_RULE, "title", &[None, None], false, false),
    (SEASON_RULE, "season", &[None], false, false),
    (EPISODE_RULE, "episode", &[None], false, false),
    // Confined to a root in an earlier revision only: a candidate of that
    // revision may still hold a checkpoint naming the root.
    (
        ROOT_RULE,
        "title",
        &[Some("root-synthetic"), None],
        false,
        false,
    ),
    (
        SYSTEM_RULE,
        "title",
        &[Some("root-synthetic"), None],
        true,
        true,
    ),
    (SYSTEM_SEASON_RULE, "season", &[None], true, true),
];

async fn assert_outcome(pool: &SqlitePool, context: &str) {
    for (rule, _, roots, _, kept) in RULES {
        let expected = if *kept {
            seeded_history(roots.len() as i64)
        } else {
            removed()
        };
        assert_eq!(
            history(pool, rule, true).await,
            expected,
            "{context}: {rule}"
        );
    }
    assert_eq!(
        count(
            pool,
            "SELECT COUNT(*) FROM lifecycle_rule_exclusions WHERE id = ?1",
            "exclusion-global"
        )
        .await,
        1,
        "{context}: a global season exclusion stays"
    );
    assert_eq!(
        count(pool, "SELECT COUNT(*) FROM titles WHERE id = ?1", TITLE).await,
        1,
        "{context}: titles are untouched"
    );
    for (table, column) in [
        ("lifecycle_rule_sets", "subject_kind"),
        ("lifecycle_rule_revisions", "storage_root_id"),
    ] {
        assert_eq!(
            count(
                pool,
                &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                column
            )
            .await,
            0,
            "{context}: {table}.{column} is not stored"
        );
    }
    assert_eq!(
        count(
            pool,
            "SELECT COUNT(*) FROM pragma_foreign_key_check WHERE ?1 = ?1",
            "x"
        )
        .await,
        0,
        "{context}: every reference resolves"
    );
}

async fn upgrade(pool: &SqlitePool) {
    crate::migrations::run_upgrade_migrations_through(pool, PRE_UPGRADE_VERSION + 1)
        .await
        .expect("0348 upgrade should apply");
}

async fn run_upgrade(foreign_keys: bool) {
    let context = format!("foreign keys {foreign_keys}");
    let pool = pre_upgrade_pool(foreign_keys).await;
    seed_unrelated(&pool).await;
    for (rule, kind, roots, system_owned, _) in RULES {
        seed_rule(&pool, rule, kind, roots, *system_owned).await;
        assert_eq!(
            history(&pool, rule, false).await,
            seeded_history(roots.len() as i64),
            "{context}: seeded {rule}"
        );
    }

    upgrade(&pool).await;
    assert_outcome(&pool, &context).await;

    // A second run of the upgrade applies nothing and changes nothing.
    upgrade(&pool).await;
    assert_outcome(&pool, &format!("{context}, second run")).await;
}

#[tokio::test]
async fn the_upgrade_removes_scoped_and_root_confined_rules_with_foreign_keys_enforced() {
    run_upgrade(true).await;
}

#[tokio::test]
async fn the_upgrade_removes_scoped_and_root_confined_rules_without_foreign_keys() {
    run_upgrade(false).await;
}

/// Both engines drop the two columns with the same statements.
#[test]
fn both_engines_drop_the_scope_and_root_columns() {
    for statement in [
        "ALTER TABLE lifecycle_rule_revisions DROP COLUMN storage_root_id;",
        "ALTER TABLE lifecycle_rule_sets DROP COLUMN subject_kind;",
    ] {
        assert!(SQLITE.contains(statement), "{statement}");
        assert!(POSTGRES.contains(statement), "{statement}");
    }
}
