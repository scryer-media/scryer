//! A lifecycle rule's enabled flag, evaluation mode, effect arming and re-arm
//! flag collapse into one stored state, an edited-since-enabled flag and the
//! revision deletion was confirmed for. Every stored combination is seeded
//! on a system-owned rule, the one kind of rule the upgrade keeps, before the
//! upgrade and read back after it through the real upgrade runner.

use sqlx::SqlitePool;

const SQLITE: &str = include_str!("../../../scryer/src/db/migrations/0348_lifecycle_rules.sql");
const POSTGRES: &str =
    include_str!("../../../scryer/src/db/postgres/migrations/0348_lifecycle_rules.sql");

/// Version the catalog is replayed to before 0348 is applied.
const PRE_UPGRADE_VERSION: i64 = 347;

/// One stored combination and what it must read as after the upgrade.
struct Case {
    id: &'static str,
    enabled: bool,
    mode: &'static str,
    arming: &'static str,
    rearm_required: bool,
    revision: i64,
    state: &'static str,
    edited_since_enabled: bool,
    confirmed_revision: Option<i64>,
}

const CASES: &[Case] = &[
    // Disabled reads as disabled whatever else the row holds.
    Case {
        id: "rule-disabled",
        enabled: false,
        mode: "disabled",
        arming: "none",
        rearm_required: false,
        revision: 1,
        state: "disabled",
        edited_since_enabled: false,
        confirmed_revision: None,
    },
    Case {
        id: "rule-disabled-flagged",
        enabled: false,
        mode: "disabled",
        arming: "none",
        rearm_required: true,
        revision: 2,
        state: "disabled",
        edited_since_enabled: true,
        confirmed_revision: None,
    },
    Case {
        id: "rule-not-enabled-but-armed",
        enabled: false,
        mode: "observe",
        arming: "destructive",
        rearm_required: false,
        revision: 2,
        state: "disabled",
        edited_since_enabled: false,
        confirmed_revision: None,
    },
    Case {
        id: "rule-disabled-mode-but-armed",
        enabled: true,
        mode: "disabled",
        arming: "destructive",
        rearm_required: false,
        revision: 2,
        state: "disabled",
        edited_since_enabled: false,
        confirmed_revision: None,
    },
    // A mode this build never wrote evaluated nothing, so it stays off.
    Case {
        id: "rule-unknown-mode",
        enabled: true,
        mode: "shadow",
        arming: "reversible",
        rearm_required: false,
        revision: 1,
        state: "disabled",
        edited_since_enabled: false,
        confirmed_revision: None,
    },
    // Observing without arming is a dry run, flagged or not.
    Case {
        id: "rule-dry-run",
        enabled: true,
        mode: "observe",
        arming: "none",
        rearm_required: false,
        revision: 1,
        state: "dry_run",
        edited_since_enabled: false,
        confirmed_revision: None,
    },
    Case {
        id: "rule-dry-run-flagged",
        enabled: true,
        mode: "observe",
        arming: "none",
        rearm_required: true,
        revision: 3,
        state: "dry_run",
        edited_since_enabled: true,
        confirmed_revision: None,
    },
    // An arming this build cannot read armed nothing.
    Case {
        id: "rule-unknown-arming",
        enabled: true,
        mode: "observe",
        arming: "sweeping",
        rearm_required: false,
        revision: 1,
        state: "dry_run",
        edited_since_enabled: false,
        confirmed_revision: None,
    },
    // Armed rules are enabled; only deletion carries a confirmation.
    Case {
        id: "rule-reversible",
        enabled: true,
        mode: "observe",
        arming: "reversible",
        rearm_required: false,
        revision: 3,
        state: "enabled",
        edited_since_enabled: false,
        confirmed_revision: None,
    },
    Case {
        id: "rule-destructive",
        enabled: true,
        mode: "observe",
        arming: "destructive",
        rearm_required: false,
        revision: 4,
        state: "enabled",
        edited_since_enabled: false,
        confirmed_revision: Some(4),
    },
    // Armed but flagged for review read as a dry run before, so it stays one
    // and carries no confirmation.
    Case {
        id: "rule-destructive-flagged",
        enabled: true,
        mode: "observe",
        arming: "destructive",
        rearm_required: true,
        revision: 2,
        state: "dry_run",
        edited_since_enabled: true,
        confirmed_revision: None,
    },
    Case {
        id: "rule-reversible-flagged",
        enabled: true,
        mode: "observe",
        arming: "reversible",
        rearm_required: true,
        revision: 2,
        state: "dry_run",
        edited_since_enabled: true,
        confirmed_revision: None,
    },
];

async fn pre_upgrade_pool() -> SqlitePool {
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
    pool
}

async fn seed(pool: &SqlitePool, case: &Case) {
    sqlx::query(
        "INSERT INTO maintenance_rule_sets
            (id, name, enabled, evaluation_mode, effect_arming, destructive_rearm_required,
             current_revision_number, system_owned)
         VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, 1)",
    )
    .bind(case.id)
    .bind(case.enabled)
    .bind(case.mode)
    .bind(case.arming)
    .bind(case.rearm_required)
    .bind(case.revision)
    .execute(pool)
    .await
    .unwrap_or_else(|error| panic!("seeding {} failed: {error}", case.id));
}

async fn read(pool: &SqlitePool, id: &str) -> (String, bool, Option<i64>) {
    sqlx::query_as(
        "SELECT state, edited_since_enabled, destructive_confirmed_revision
           FROM lifecycle_rule_sets WHERE id = ?1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|error| panic!("reading {id} failed: {error}"))
}

#[tokio::test]
async fn every_stored_combination_maps_to_one_state() {
    let pool = pre_upgrade_pool().await;
    for case in CASES {
        seed(&pool, case).await;
    }

    crate::migrations::run_upgrade_migrations_through(&pool, PRE_UPGRADE_VERSION + 1)
        .await
        .expect("0348 upgrade should apply");

    for case in CASES {
        assert_eq!(
            read(&pool, case.id).await,
            (
                case.state.to_string(),
                case.edited_since_enabled,
                case.confirmed_revision
            ),
            "{}",
            case.id
        );
    }

    let retired: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('lifecycle_rule_sets')
          WHERE name IN ('enabled', 'evaluation_mode', 'effect_arming',
                         'destructive_rearm_required')",
    )
    .fetch_one(&pool)
    .await
    .expect("table info should read");
    assert_eq!(retired, 0, "the four retired columns are gone");
}

#[tokio::test]
async fn a_new_rule_row_defaults_to_disabled() {
    let pool = pre_upgrade_pool().await;
    crate::migrations::run_upgrade_migrations_through(&pool, PRE_UPGRADE_VERSION + 1)
        .await
        .expect("0348 upgrade should apply");
    sqlx::query("INSERT INTO lifecycle_rule_sets (id, name) VALUES ('rule-fresh', 'Fresh')")
        .execute(&pool)
        .await
        .expect("insert");
    assert_eq!(
        read(&pool, "rule-fresh").await,
        ("disabled".to_string(), false, None)
    );
}

/// The Postgres statements for the state are the SQLite ones with
/// Postgres's boolean and 64-bit revision column.
#[test]
fn postgres_statements_match_sqlite() {
    let state = |sql: &'static str| {
        let start = sql
            .find("-- A rule stores one state")
            .expect("state section start");
        let end = start
            + sql[start..]
                .find("-- A stored source declares")
                .expect("state section end");
        &sql[start..end]
    };
    assert_eq!(
        state(POSTGRES),
        state(SQLITE)
            .replace(
                "edited_since_enabled INTEGER NOT NULL DEFAULT 0",
                "edited_since_enabled BOOLEAN NOT NULL DEFAULT false"
            )
            .replace(
                "destructive_confirmed_revision INTEGER;",
                "destructive_confirmed_revision BIGINT;"
            )
            .replace("enabled = 0 OR", "enabled = false OR")
            .replace("WHEN enabled = 1", "WHEN enabled = true")
            .replace(
                "destructive_rearm_required = 0",
                "destructive_rearm_required = false"
            )
    );
}
