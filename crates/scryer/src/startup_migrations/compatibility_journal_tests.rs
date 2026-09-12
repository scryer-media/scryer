use super::*;
use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};

#[tokio::test]
async fn compatibility_journal_retries_preserve_original_and_survive_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .join("journal.db")
        .to_string_lossy()
        .into_owned();
    let services = SqliteServices::new_with_mode(path.clone(), MigrationMode::Apply)
        .await
        .unwrap();
    let datastore = services.datastore();
    ensure(&datastore).await.unwrap();
    record(
        &datastore, "repair", "provider", "digest", "original", "pending", None,
    )
    .await
    .unwrap();
    assert!(
        !completed(&datastore, "repair", "provider", "digest")
            .await
            .unwrap()
    );
    record(
        &datastore,
        "repair",
        "provider",
        "digest",
        "changed",
        "blocked",
        Some("unavailable".into()),
    )
    .await
    .unwrap();
    drop(datastore);
    drop(services);
    let services = SqliteServices::new_with_mode(path, MigrationMode::Apply)
        .await
        .unwrap();
    let datastore = services.datastore();
    assert!(
        !completed(&datastore, "repair", "provider", "digest")
            .await
            .unwrap()
    );
    record(
        &datastore,
        "repair",
        "provider",
        "digest",
        "changed again",
        "validated",
        None,
    )
    .await
    .unwrap();
    assert!(
        completed(&datastore, "repair", "provider", "digest")
            .await
            .unwrap()
    );
    assert!(
        !completed(&datastore, "repair", "other", "digest")
            .await
            .unwrap()
    );
    assert!(
        !completed(&datastore, "repair", "provider", "new bytes")
            .await
            .unwrap()
    );
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT original_metadata, detail FROM application_compatibility_journal",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].text("original_metadata").unwrap(), "original");
    assert_eq!(rows[0].opt_text("detail").unwrap(), None);
}

#[tokio::test]
async fn compatibility_journal_failed_completion_remains_retryable() {
    let temp = tempfile::tempdir().unwrap();
    let services = SqliteServices::new_with_mode(
        temp.path()
            .join("journal.db")
            .to_string_lossy()
            .into_owned(),
        MigrationMode::Apply,
    )
    .await
    .unwrap();
    let datastore = services.datastore();
    record(
        &datastore, "repair", "provider", "digest", "original", "pending", None,
    )
    .await
    .unwrap();
    SqlRuntime::execute_write(
        &datastore,
        "inject_journal_failure",
        "CREATE TRIGGER reject_journal_update BEFORE UPDATE ON application_compatibility_journal
         BEGIN SELECT RAISE(ABORT, 'synthetic journal failure'); END",
        vec![],
    )
    .await
    .unwrap();
    assert!(
        record(
            &datastore,
            "repair",
            "provider",
            "digest",
            "new",
            "validated",
            None
        )
        .await
        .is_err()
    );
    assert!(
        !completed(&datastore, "repair", "provider", "digest")
            .await
            .unwrap()
    );
    SqlRuntime::execute_write(
        &datastore,
        "remove_journal_failure",
        "DROP TRIGGER reject_journal_update",
        vec![],
    )
    .await
    .unwrap();
    record(
        &datastore,
        "repair",
        "provider",
        "digest",
        "new",
        "validated",
        None,
    )
    .await
    .unwrap();
    assert!(
        completed(&datastore, "repair", "provider", "digest")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn blocked_subjects_reports_only_still_blocked_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .join("blocked.db")
        .to_string_lossy()
        .into_owned();
    let services = SqliteServices::new_with_mode(path, MigrationMode::Apply)
        .await
        .unwrap();
    let datastore = services.datastore();
    ensure(&datastore).await.unwrap();

    record(
        &datastore,
        "repair",
        "stuck",
        "digest-a",
        "original",
        "blocked",
        Some("no compatible build".into()),
    )
    .await
    .unwrap();
    record(
        &datastore,
        "repair",
        "fixed",
        "digest-b",
        "original",
        "validated",
        None,
    )
    .await
    .unwrap();
    record(
        &datastore, "repair", "trying", "digest-c", "original", "pending", None,
    )
    .await
    .unwrap();
    record(
        &datastore, "other", "stuck", "digest-d", "original", "blocked", None,
    )
    .await
    .unwrap();

    let blocked = blocked_subjects(&datastore, "repair").await.unwrap();
    assert_eq!(
        blocked,
        std::collections::HashSet::from([("stuck".to_string(), "digest-a".to_string())]),
        "only this migration's still-blocked evidence counts"
    );

    // A later boot that clears the blocker must drop it from the set, so the
    // recovery catalog refresh is armed again for any future evidence.
    record(
        &datastore,
        "repair",
        "stuck",
        "digest-a",
        "original",
        "validated",
        None,
    )
    .await
    .unwrap();
    assert!(
        blocked_subjects(&datastore, "repair")
            .await
            .unwrap()
            .is_empty()
    );
}
