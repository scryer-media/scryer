use scryer_infrastructure_sql::runtime::{SqlArg, SqlRuntime, StoreDatastore};

/// Per-target progress survives partial upgrades and records the original
/// metadata. Completion of one provider never hides another provider's failure.
#[cfg(test)]
#[path = "compatibility_journal_tests.rs"]
mod tests;

pub(crate) async fn ensure(datastore: &StoreDatastore) -> Result<(), String> {
    // Schema migration 0225 owns the table on both engines, including restore.
    SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT migration_id FROM application_compatibility_journal LIMIT 0",
        &[],
    )
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

#[cfg(test)]
pub(crate) async fn completed(
    datastore: &StoreDatastore,
    migration: &str,
    subject: &str,
    digest: &str,
) -> Result<bool, String> {
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT status FROM application_compatibility_journal
         WHERE migration_id = {} AND subject_id = {} AND source_digest = {}",
        &[
            SqlArg::Text(migration.into()),
            SqlArg::Text(subject.into()),
            SqlArg::Text(digest.into()),
        ],
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(rows
        .first()
        .is_some_and(|row| row.text("status").ok().as_deref() == Some("validated")))
}

pub(crate) async fn record(
    datastore: &StoreDatastore,
    migration: &str,
    subject: &str,
    digest: &str,
    original: &str,
    status: &str,
    detail: Option<String>,
) -> Result<(), String> {
    SqlRuntime::execute_write(
        datastore,
        "record_compatibility_progress",
        "INSERT INTO application_compatibility_journal
         (migration_id, subject_id, source_digest, original_metadata, status, detail)
         VALUES ({}, {}, {}, {}, {}, {})
         ON CONFLICT (migration_id, subject_id, source_digest)
         DO UPDATE SET status = excluded.status, detail = excluded.detail",
        vec![
            SqlArg::Text(migration.into()),
            SqlArg::Text(subject.into()),
            SqlArg::Text(digest.into()),
            SqlArg::Text(original.into()),
            SqlArg::Text(status.into()),
            SqlArg::OptText(detail),
        ],
    )
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}
