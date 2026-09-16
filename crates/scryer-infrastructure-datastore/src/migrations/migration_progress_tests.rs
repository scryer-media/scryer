//! The migration runner counts what it applies, so the upgrade screen can show
//! how far an upgrade has got.

use sqlx::SqlitePool;

use super::{MigrationHookContext, MigrationProgress, embedded_catalog};
use crate::MigrationMode;

async fn pool_at_version(version: i64) -> SqlitePool {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix extension should register before the migration fixture");
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory SQLite should open");
    super::replay_source_catalog_for_fresh_install(&pool, Some(version), true)
        .await
        .expect("pre-upgrade migration fixture should apply");
    pool
}

async fn run_with_progress(pool: &SqlitePool, progress: &MigrationProgress) {
    super::run_migrations_with_hook_context(
        pool,
        MigrationMode::Apply,
        MigrationHookContext {
            progress: progress.clone(),
            ..MigrationHookContext::default()
        },
    )
    .await
    .expect("migrations should apply");
}

#[tokio::test]
async fn upgrade_counts_every_pending_migration() {
    let catalog = embedded_catalog().expect("embedded catalog");
    let from_version = catalog.max_version() - 3;
    let expected = catalog
        .migrations
        .iter()
        .filter(|migration| migration.version > from_version)
        .count();
    assert!(expected > 0, "the fixture must leave migrations pending");

    let pool = pool_at_version(from_version).await;
    let progress = MigrationProgress::default();
    assert_eq!(
        progress.snapshot(),
        None,
        "nothing is counted before the run"
    );

    run_with_progress(&pool, &progress).await;
    assert_eq!(progress.snapshot(), Some((expected, expected)));
}

#[tokio::test]
async fn up_to_date_database_reports_no_progress() {
    let catalog = embedded_catalog().expect("embedded catalog");
    let pool = pool_at_version(catalog.max_version()).await;
    let progress = MigrationProgress::default();

    run_with_progress(&pool, &progress).await;
    assert_eq!(progress.snapshot(), None);
}

#[test]
fn clones_share_counts_and_never_report_past_the_total() {
    let reader = MigrationProgress::default();
    let writer = reader.clone();
    writer.begin(2);
    assert_eq!(reader.snapshot(), Some((0, 2)));
    writer.complete_one();
    assert_eq!(reader.snapshot(), Some((1, 2)));
    writer.complete_one();
    writer.complete_one();
    assert_eq!(reader.snapshot(), Some((2, 2)));
}
