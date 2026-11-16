//! Maintenance rules become lifecycle rules: their tables and indexes take the
//! new name with every row kept, the gates keep their stored values, job
//! history follows the new job key, builder documents name the new family,
//! and stored reason codes take the new prefix. Everything outside that is
//! left as it was. Exercised through the real upgrade runner, with and
//! without foreign-key enforcement.

use sqlx::SqlitePool;

const SQLITE: &str =
    include_str!("../../../scryer/src/db/migrations/0357_rename_maintenance_to_lifecycle.sql");
const POSTGRES: &str = include_str!(
    "../../../scryer/src/db/postgres/migrations/0357_rename_maintenance_to_lifecycle.sql"
);

/// Version the catalog is replayed to before the rename is applied.
const PRE_UPGRADE_VERSION: i64 = 356;

const RULE: &str = "rule-emberfall";
const CANDIDATE: &str = "rule-emberfall-candidate";
const TITLE: &str = "title-emberfall";

/// Each renamed table, its old name, and how many rows the seed gives it.
const TABLES: &[(&str, &str, i64)] = &[
    ("maintenance_rule_sets", "lifecycle_rule_sets", 1),
    ("maintenance_rule_revisions", "lifecycle_rule_revisions", 1),
    (
        "maintenance_rule_set_libraries",
        "lifecycle_rule_set_libraries",
        1,
    ),
    (
        "maintenance_rule_exclusions",
        "lifecycle_rule_exclusions",
        2,
    ),
    (
        "maintenance_evaluation_runs",
        "lifecycle_evaluation_runs",
        1,
    ),
    ("maintenance_action_steps", "lifecycle_action_steps", 1),
    (
        "maintenance_action_step_attempts",
        "lifecycle_action_step_attempts",
        1,
    ),
    (
        "maintenance_action_job_receipts",
        "lifecycle_action_job_receipts",
        1,
    ),
    (
        "maintenance_sequence_terminal_memberships",
        "lifecycle_sequence_terminal_memberships",
        1,
    ),
];

const INDEXES: &[&str] = &[
    "idx_lifecycle_rule_set_libraries_library",
    "idx_lifecycle_rule_revisions_rule_set",
    "idx_lifecycle_rule_exclusions_title",
    "idx_lifecycle_rule_exclusions_rule_subject",
    "idx_lifecycle_rule_exclusions_global_subject",
    "idx_lifecycle_evaluation_runs_rule_set",
    "idx_lifecycle_evaluation_runs_started",
    "idx_lifecycle_action_step_attempts_step",
    "idx_lifecycle_action_step_attempts_started",
    "idx_lifecycle_action_job_receipts_step",
    "idx_lifecycle_action_job_receipts_accepted_request",
    "idx_lifecycle_sequence_terminal_memberships_active",
    "idx_lifecycle_sequence_terminal_memberships_active_page",
];

const GATES: &[&str] = &[
    "evaluation",
    "result_display",
    "reversible_effects",
    "destructive_effects",
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
    .expect("pre-rename migration fixture should apply");
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

async fn exec(pool: &SqlitePool, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(pool)
        .await
        .unwrap_or_else(|error| panic!("fixture statement failed: {error}\n{sql}"));
}

async fn scalar_i64(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(pool)
        .await
        .unwrap_or_else(|error| panic!("query failed: {error}\n{sql}"))
}

async fn text(pool: &SqlitePool, sql: &str) -> Option<String> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(pool)
        .await
        .unwrap_or_else(|error| panic!("query failed: {error}\n{sql}"))
}

async fn object_count(pool: &SqlitePool, kind: &str, name: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2")
        .bind(kind)
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("schema lookup")
}

async fn seed(pool: &SqlitePool) {
    exec(
        pool,
        &format!(
            "INSERT INTO titles (id, name, facet, monitored, created_at, library_id, root_folder_id)
             SELECT '{TITLE}', 'Synthetic Emberfall', 'movie', 1, '2026-09-01', l.id, r.id
             FROM libraries l JOIN library_roots r ON r.library_id = l.id
             WHERE l.facet = 'movie' AND l.is_default = 1 AND r.is_default = 1"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO maintenance_rule_sets (id, name) VALUES ('{RULE}', 'Emberfall rule')"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO maintenance_rule_revisions
                (id, rule_set_id, revision_number, rego_source, action_spec,
                 matcher_content_hash, builder_model_json)
             VALUES ('{RULE}-revision', '{RULE}', 1, 'package scryer.maintenance.user', '{{}}',
                 'hash', '{{\"family\":\"maintenance\",\"join\":\"ALL\",\"version\":2}}')"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO maintenance_rule_set_libraries (rule_set_id, library_id, position)
             SELECT '{RULE}', id, 0 FROM libraries WHERE facet = 'movie' AND is_default = 1"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO maintenance_rule_exclusions (id, rule_set_id, title_id, subject_id)
             VALUES ('{RULE}-exclusion', '{RULE}', '{TITLE}', '{TITLE}'),
                    ('exclusion-global', NULL, '{TITLE}', '{TITLE}')"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO maintenance_evaluation_runs (id, rule_set_id, revision_number)
             VALUES ('{RULE}-run', '{RULE}', 1)"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO lifecycle_candidates
                (id, rule_set_id, revision_number, title_id, subject_id, state, state_reason)
             VALUES ('{CANDIDATE}', '{RULE}', 1, '{TITLE}', '{TITLE}', 'blocked',
                 'MAINTENANCE_RULE_CHANGED_WHILE_CONFIRMING'),
                    ('{CANDIDATE}-kept', '{RULE}', 1, '{TITLE}', 'other-subject', 'blocked',
                 'playback_hold'),
                    ('{CANDIDATE}-repeated', '{RULE}', 1, '{TITLE}', 'third-subject', 'blocked',
                 'MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE')"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO lifecycle_action_runs
                (id, candidate_id, rule_set_id, revision_number, title_id, action_kind,
                 match_generation, idempotency_key, attempt, status, hold_reason, detail,
                 started_at)
             VALUES ('{CANDIDATE}-action', '{CANDIDATE}', '{RULE}', 1, '{TITLE}',
                 'action_sequence', 1, '{CANDIDATE}', 1, 'held',
                 'LIBRARY_MAINTENANCE_DELETION_FILE_CHANGED',
                 '{{\"files\":[{{\"error_reason\":\"LIBRARY_MAINTENANCE_DELETION_FILE_CHANGED\"}},{{\"error_reason\":\"SERVICES_MAINTENANCE_OPERATION_PROGRESS\"}}],\"lock\":\"maintenance-sequence:{CANDIDATE}\"}}',
                 '2026-09-01T00:00:00Z'),
                    ('{CANDIDATE}-action-kept', '{CANDIDATE}', '{RULE}', 1, '{TITLE}',
                 'action_sequence', 1, '{CANDIDATE}', 2, 'held',
                 'SERVICES_MAINTENANCE_OPERATION_PROGRESS', '{{}}',
                 '2026-09-01T00:00:00Z')"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO maintenance_action_steps
                (candidate_id, match_generation, revision_number, step_id, rule_set_id,
                 title_id, subject_kind, subject_id, sequence_content_hash, step_kind, state,
                 hold_reason, provenance_json, created_at, updated_at)
             VALUES ('{CANDIDATE}', 1, 1, 'search', '{RULE}', '{TITLE}', 'title', '{TITLE}',
                 'hash', 'search', 'held', 'CONFIGURATION_MAINTENANCE_RULE_HAS_HISTORY',
                 '{{\"reason\":\"ACQUISITION_MAINTENANCE_SEARCH_DISPATCH_REQUIRES_ACCEPTED_RECEIPT_INTENT\"}}',
                 '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO maintenance_action_step_attempts
                (id, candidate_id, match_generation, revision_number, step_id, attempt, state,
                 hold_reason, provenance_json, started_at, created_at, updated_at)
             VALUES ('{CANDIDATE}-attempt', '{CANDIDATE}', 1, 1, 'search', 1, 'held',
                 'INTERFACE_MAINTENANCE_RULE_NOT_FOUND',
                 '{{\"reason\":\"MAINTENANCE_RULE_SET_NOT_FOUND\"}}',
                 '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO maintenance_action_job_receipts
                (candidate_id, match_generation, revision_number, step_id, dispatch_attempt,
                 schema_version, logical_request_key, request_hash, state,
                 reconciliation_evidence_json, created_at, updated_at)
             VALUES ('{CANDIDATE}', 1, 1, 'search', 1, 1, '{CANDIDATE}', 'request-hash',
                 'accepted',
                 '{{\"reason\":\"MAINTENANCE_ACTION_INVALID\",\"coverage\":\"MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE\"}}',
                 '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')"
        ),
    )
    .await;
    exec(
        pool,
        &format!(
            "INSERT INTO maintenance_sequence_terminal_memberships
                (id, candidate_id, rule_set_id, revision_number, matcher_content_hash,
                 title_id, subject_kind, subject_id, match_generation, sequence_content_hash,
                 outcome, created_at)
             VALUES ('{CANDIDATE}-membership', '{CANDIDATE}', '{RULE}', 1, 'hash', '{TITLE}',
                 'title', '{TITLE}', 1, 'hash', 'succeeded', '2026-09-01T00:00:00Z')"
        ),
    )
    .await;
    for (index, gate) in GATES.iter().enumerate() {
        exec(
            pool,
            &format!(
                "INSERT INTO settings_definitions
                    (id, category, scope, key_name, data_type, default_value_json,
                     is_sensitive, created_at, updated_at)
                 VALUES ('gate-{gate}', 'general', 'system', 'maintenance.gate.{gate}',
                     'boolean', 'false', 0, '2026-09-01', '2026-09-01')"
            ),
        )
        .await;
        let value = if index % 2 == 0 { "true" } else { "false" };
        exec(
            pool,
            &format!(
                "INSERT INTO settings_values
                    (id, setting_definition_id, scope, value_json, source, created_at,
                     updated_at)
                 VALUES ('gate-value-{gate}', 'gate-{gate}', 'system', '{value}', 'user',
                     '2026-09-01', '2026-09-01')"
            ),
        )
        .await;
    }
    exec(
        pool,
        "INSERT INTO settings_definitions
            (id, category, scope, key_name, data_type, default_value_json, is_sensitive,
             created_at, updated_at)
         VALUES ('unrelated-setting', 'general', 'system', 'maintenance.gate.synthetic_other',
             'boolean', 'false', 0, '2026-09-01', '2026-09-01')",
    )
    .await;
    exec(
        pool,
        "INSERT INTO workflow_operations
            (id, operation_type, status, job_key, created_at, updated_at)
         VALUES ('job-run-lifecycle', 'maintenance_rule_evaluation', 'completed',
                 'maintenance_rule_evaluation', '2026-09-01', '2026-09-01'),
                ('job-run-other', 'housekeeping', 'completed', 'housekeeping',
                 '2026-09-01', '2026-09-01')",
    )
    .await;
    exec(
        pool,
        "INSERT INTO rule_sets (id, name, rego_source, builder_model_json)
         VALUES ('scoring-kept', 'Scoring', 'package s',
                 '{\"family\":\"maintenance\",\"version\":2}')",
    )
    .await;
}

async fn assert_renamed(pool: &SqlitePool, context: &str) {
    for (old, new, rows) in TABLES {
        assert_eq!(
            object_count(pool, "table", old).await,
            0,
            "{context}: {old}"
        );
        assert_eq!(
            scalar_i64(pool, &format!("SELECT COUNT(*) FROM {new}")).await,
            *rows,
            "{context}: {new} keeps its rows"
        );
    }
    assert_eq!(
        scalar_i64(
            pool,
            "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'idx\\_maintenance\\_%' ESCAPE '\\'"
        )
        .await,
        0,
        "{context}: no index keeps the old name"
    );
    for index in INDEXES {
        assert_eq!(
            object_count(pool, "index", index).await,
            1,
            "{context}: {index}"
        );
    }
    assert_eq!(
        scalar_i64(pool, "SELECT COUNT(*) FROM pragma_foreign_key_check").await,
        0,
        "{context}: every reference resolves"
    );

    for gate in GATES {
        assert_eq!(
            text(
                pool,
                &format!("SELECT key_name FROM settings_definitions WHERE id = 'gate-{gate}'")
            )
            .await
            .as_deref(),
            Some(format!("lifecycle.gate.{gate}").as_str()),
            "{context}: gate {gate} renamed"
        );
    }
    assert_eq!(
        scalar_i64(
            pool,
            "SELECT COUNT(*) FROM settings_values v JOIN settings_definitions d
               ON d.id = v.setting_definition_id
             WHERE d.key_name LIKE 'lifecycle.gate.%' AND v.value_json = 'true'"
        )
        .await,
        2,
        "{context}: gate values follow their definitions"
    );
    assert_eq!(
        text(
            pool,
            "SELECT key_name FROM settings_definitions WHERE id = 'unrelated-setting'"
        )
        .await
        .as_deref(),
        Some("maintenance.gate.synthetic_other"),
        "{context}: only the four gates are renamed"
    );

    assert_eq!(
        text(
            pool,
            "SELECT job_key || '|' || operation_type FROM workflow_operations
             WHERE id = 'job-run-lifecycle'"
        )
        .await
        .as_deref(),
        Some("lifecycle_rule_evaluation|lifecycle_rule_evaluation"),
        "{context}: job history follows the job key"
    );
    assert_eq!(
        text(
            pool,
            "SELECT job_key || '|' || operation_type FROM workflow_operations
             WHERE id = 'job-run-other'"
        )
        .await
        .as_deref(),
        Some("housekeeping|housekeeping"),
        "{context}"
    );

    assert_eq!(
        text(
            pool,
            &format!(
                "SELECT builder_model_json FROM lifecycle_rule_revisions
                 WHERE id = '{RULE}-revision'"
            )
        )
        .await
        .as_deref(),
        Some("{\"family\":\"lifecycle\",\"join\":\"ALL\",\"version\":2}"),
        "{context}: builder family"
    );
    assert_eq!(
        text(
            pool,
            &format!(
                "SELECT rego_source FROM lifecycle_rule_revisions WHERE id = '{RULE}-revision'"
            )
        )
        .await
        .as_deref(),
        Some("package scryer.maintenance.user"),
        "{context}: rule sources are left as written"
    );
    assert_eq!(
        text(
            pool,
            "SELECT builder_model_json FROM rule_sets WHERE id = 'scoring-kept'"
        )
        .await
        .as_deref(),
        Some("{\"family\":\"maintenance\",\"version\":2}"),
        "{context}: scoring rules are not touched"
    );

    for (sql, expected) in [
        (
            format!("SELECT state_reason FROM lifecycle_candidates WHERE id = '{CANDIDATE}'"),
            "LIFECYCLE_RULE_CHANGED_WHILE_CONFIRMING".to_string(),
        ),
        (
            format!("SELECT state_reason FROM lifecycle_candidates WHERE id = '{CANDIDATE}-kept'"),
            "playback_hold".to_string(),
        ),
        (
            format!(
                "SELECT state_reason FROM lifecycle_candidates WHERE id = '{CANDIDATE}-repeated'"
            ),
            "LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE".to_string(),
        ),
        (
            format!("SELECT hold_reason FROM lifecycle_action_runs WHERE id = '{CANDIDATE}-action'"),
            "LIBRARY_LIFECYCLE_DELETION_FILE_CHANGED".to_string(),
        ),
        (
            format!(
                "SELECT hold_reason FROM lifecycle_action_runs WHERE id = '{CANDIDATE}-action-kept'"
            ),
            "SERVICES_MAINTENANCE_OPERATION_PROGRESS".to_string(),
        ),
        (
            format!("SELECT detail FROM lifecycle_action_runs WHERE id = '{CANDIDATE}-action'"),
            format!(
                "{{\"files\":[{{\"error_reason\":\"LIBRARY_LIFECYCLE_DELETION_FILE_CHANGED\"}},{{\"error_reason\":\"SERVICES_MAINTENANCE_OPERATION_PROGRESS\"}}],\"lock\":\"maintenance-sequence:{CANDIDATE}\"}}"
            ),
        ),
        (
            "SELECT hold_reason || '|' || provenance_json FROM lifecycle_action_steps".to_string(),
            "CONFIGURATION_LIFECYCLE_RULE_HAS_HISTORY|{\"reason\":\"ACQUISITION_LIFECYCLE_SEARCH_DISPATCH_REQUIRES_ACCEPTED_RECEIPT_INTENT\"}".to_string(),
        ),
        (
            "SELECT hold_reason || '|' || provenance_json FROM lifecycle_action_step_attempts"
                .to_string(),
            "INTERFACE_LIFECYCLE_RULE_NOT_FOUND|{\"reason\":\"LIFECYCLE_RULE_SET_NOT_FOUND\"}"
                .to_string(),
        ),
        (
            "SELECT reconciliation_evidence_json FROM lifecycle_action_job_receipts".to_string(),
            "{\"reason\":\"LIFECYCLE_ACTION_INVALID\",\"coverage\":\"LIFECYCLE_AMBIGUOUS_LIFECYCLE_FILE_COVERAGE\"}"
                .to_string(),
        ),
    ] {
        assert_eq!(
            text(pool, &sql).await.as_deref(),
            Some(expected.as_str()),
            "{context}: {sql}"
        );
    }
}

async fn run_upgrade(foreign_keys: bool) {
    let context = format!("foreign keys {foreign_keys}");
    let pool = pre_upgrade_pool(foreign_keys).await;
    seed(&pool).await;
    for (old, _, rows) in TABLES {
        assert_eq!(
            scalar_i64(&pool, &format!("SELECT COUNT(*) FROM {old}")).await,
            *rows,
            "{context}: seeded {old}"
        );
    }

    crate::migrations::run_upgrade_migrations_through(&pool, PRE_UPGRADE_VERSION + 1)
        .await
        .expect("the rename should apply");
    assert_renamed(&pool, &context).await;

    // A later startup applies nothing more.
    crate::migrations::run_upgrade_migrations_through(&pool, PRE_UPGRADE_VERSION + 1)
        .await
        .expect("a second startup applies");
    assert_renamed(&pool, &format!("{context}, second startup")).await;

    // Foreign keys still point at the renamed tables: removing the rule takes
    // its history with it, and leaves the global exclusion.
    if foreign_keys {
        exec(
            &pool,
            &format!("DELETE FROM lifecycle_rule_sets WHERE id = '{RULE}'"),
        )
        .await;
        for (_, new, _) in TABLES {
            let expected = i64::from(*new == "lifecycle_rule_exclusions");
            assert_eq!(
                scalar_i64(&pool, &format!("SELECT COUNT(*) FROM {new}")).await,
                expected,
                "{context}: {new} cascades"
            );
        }
    }
}

#[tokio::test]
async fn the_rename_keeps_every_row_with_foreign_keys_enforced() {
    run_upgrade(true).await;
}

#[tokio::test]
async fn the_rename_keeps_every_row_without_foreign_keys() {
    run_upgrade(false).await;
}

/// The Postgres statements are the SQLite ones, except that Postgres renames
/// indexes and constraints in place, and spells case-sensitive matching with
/// `LIKE` and `strpos`.
#[test]
fn postgres_statements_match_sqlite() {
    let section = |sql: &'static str, from: &str, to: &str| -> &'static str {
        let start = sql.find(from).expect("section start");
        let end = if to.is_empty() {
            sql.len()
        } else {
            start + sql[start..].find(to).expect("section end")
        };
        &sql[start..end]
    };
    let tables = |sql: &'static str| section(sql, "ALTER TABLE maintenance_rule_sets", "\n\n--");
    assert_eq!(tables(POSTGRES), tables(SQLITE));

    let sqlite_updates = section(SQLITE, "-- The instance gates keep", "");
    let mut converted = String::with_capacity(sqlite_updates.len());
    let mut rest = sqlite_updates;
    while let Some(start) = rest.find("GLOB '") {
        converted.push_str(&rest[..start]);
        let pattern_start = start + "GLOB '".len();
        let pattern_end = pattern_start + rest[pattern_start..].find('\'').expect("pattern end");
        converted.push_str("LIKE '");
        converted.push_str(
            &rest[pattern_start..pattern_end]
                .replace('_', "\\_")
                .replace('*', "%"),
        );
        converted.push('\'');
        rest = &rest[pattern_end + 1..];
    }
    converted.push_str(rest);
    let converted = converted
        .replace("instr(", "strpos(")
        .replace("GLOB is case-sensitive", "LIKE is case-sensitive");
    assert_eq!(
        section(POSTGRES, "-- The instance gates keep", ""),
        converted
    );

    for index in INDEXES {
        assert!(SQLITE.contains(&format!(" {index}\n")), "{index}");
    }
    assert!(POSTGRES.contains("ALTER INDEX %I RENAME TO %I"));
    assert!(POSTGRES.contains("RENAME CONSTRAINT %I TO %I"));
}
