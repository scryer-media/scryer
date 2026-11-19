use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, ArchiveExtractionRecord, ArchiveExtractionRepository,
    ArchiveExtractionState,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const COLUMNS: &str = "client_id, client_item_id, state, workspace_path, archive_digest, \
                       attempts, last_reason_code, output_count, output_bytes, \
                       next_attempt_at, settled_at, created_at, updated_at";

/// `archive_extractions` (migration 0357): one row per tracked download whose
/// archive workspace is on disk. Every read and write is by primary key.
#[derive(Clone)]
pub struct ArchiveExtractionStore {
    datastore: StoreDatastore,
}

impl ArchiveExtractionStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

fn record_from_row(row: &SqlRow) -> AppResult<ArchiveExtractionRecord> {
    let state = row.text("state")?;
    Ok(ArchiveExtractionRecord {
        client_id: row.text("client_id")?,
        client_item_id: row.text("client_item_id")?,
        state: ArchiveExtractionState::parse(&state).ok_or_else(|| {
            AppError::Repository(format!("unknown archive extraction state {state:?}"))
        })?,
        workspace_path: row.text("workspace_path")?,
        archive_digest: row.opt_text("archive_digest")?,
        attempts: row.i64("attempts")?.clamp(0, i64::from(u32::MAX)) as u32,
        last_reason_code: row.opt_text("last_reason_code")?,
        output_count: row.i64("output_count")?.max(0) as u64,
        output_bytes: row.i64("output_bytes")?.max(0) as u64,
        next_attempt_at: row.opt_timestamp("next_attempt_at")?,
        settled_at: row.opt_timestamp("settled_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn key_args(client_id: &str, client_item_id: &str) -> [SqlArg; 2] {
    [
        SqlArg::Text(client_id.to_string()),
        SqlArg::Text(client_item_id.to_string()),
    ]
}

#[async_trait]
impl ArchiveExtractionRepository for ArchiveExtractionStore {
    async fn get(
        &self,
        client_id: &str,
        client_item_id: &str,
    ) -> AppResult<Option<ArchiveExtractionRecord>> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {COLUMNS} FROM archive_extractions \
                 WHERE client_id = {{}} AND client_item_id = {{}}"
            ),
            &key_args(client_id, client_item_id),
        )
        .await?
        .as_ref()
        .map(record_from_row)
        .transpose()
    }

    async fn save(&self, record: &ArchiveExtractionRecord) -> AppResult<()> {
        let record = record.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "save_archive_extraction", move |tx| {
            let record = record.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    &format!(
                        "INSERT INTO archive_extractions ({COLUMNS}) \
                         VALUES ({{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}, {{}}) \
                         ON CONFLICT(client_id, client_item_id) DO UPDATE SET \
                            state = excluded.state, \
                            workspace_path = excluded.workspace_path, \
                            archive_digest = excluded.archive_digest, \
                            attempts = excluded.attempts, \
                            last_reason_code = excluded.last_reason_code, \
                            output_count = excluded.output_count, \
                            output_bytes = excluded.output_bytes, \
                            next_attempt_at = excluded.next_attempt_at, \
                            settled_at = excluded.settled_at, \
                            created_at = excluded.created_at, \
                            updated_at = excluded.updated_at"
                    ),
                    &[
                        SqlArg::Text(record.client_id),
                        SqlArg::Text(record.client_item_id),
                        SqlArg::Text(record.state.as_str().to_string()),
                        SqlArg::Text(record.workspace_path),
                        SqlArg::OptText(record.archive_digest),
                        SqlArg::I64(i64::from(record.attempts)),
                        SqlArg::OptText(record.last_reason_code),
                        SqlArg::I64(i64::try_from(record.output_count).unwrap_or(i64::MAX)),
                        SqlArg::I64(i64::try_from(record.output_bytes).unwrap_or(i64::MAX)),
                        SqlArg::OptTimestamp(record.next_attempt_at),
                        SqlArg::OptTimestamp(record.settled_at),
                        SqlArg::Timestamp(record.created_at),
                        SqlArg::Timestamp(record.updated_at),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn delete(&self, client_id: &str, client_item_id: &str) -> AppResult<()> {
        let args = key_args(client_id, client_item_id);
        SqlRuntime::run_in_transaction(&self.datastore, "delete_archive_extraction", move |tx| {
            let args = args.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM archive_extractions WHERE client_id = {} AND client_item_id = {}",
                    &args,
                )
                .await?;
                Ok(())
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    /// The store over the migration's own SQLite schema.
    async fn store() -> ArchiveExtractionStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        sqlx::raw_sql(include_str!(
            "../../../../scryer/src/db/migrations/0357_archive_extractions.sql"
        ))
        .execute(&pool)
        .await
        .expect("archive extraction migration applies");
        ArchiveExtractionStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + minutes * 60, 0).expect("fixture instant is valid")
    }

    fn record(item: &str) -> ArchiveExtractionRecord {
        ArchiveExtractionRecord {
            client_id: "client-alpha".to_string(),
            client_item_id: item.to_string(),
            state: ArchiveExtractionState::Extracting,
            workspace_path: format!("/library/.scry-ex/{item}"),
            archive_digest: Some("digest".to_string()),
            attempts: 1,
            last_reason_code: None,
            output_count: 0,
            output_bytes: 0,
            next_attempt_at: None,
            settled_at: None,
            created_at: at(0),
            updated_at: at(0),
        }
    }

    #[tokio::test]
    async fn a_saved_record_round_trips_and_a_second_save_replaces_it() {
        let store = store().await;
        let first = record("item-1");
        store.save(&first).await.expect("save");
        assert_eq!(
            store.get("client-alpha", "item-1").await.expect("get"),
            Some(first.clone())
        );

        let extracted = ArchiveExtractionRecord {
            state: ArchiveExtractionState::Extracted,
            last_reason_code: Some("ARCHIVE_EXTRACTION_INTERRUPTED".to_string()),
            output_count: 3,
            output_bytes: 5_000_000_000,
            next_attempt_at: Some(at(5)),
            settled_at: Some(at(6)),
            updated_at: at(7),
            ..first
        };
        store.save(&extracted).await.expect("replace");
        assert_eq!(
            store.get("client-alpha", "item-1").await.expect("get"),
            Some(extracted)
        );
    }

    #[tokio::test]
    async fn delete_removes_only_the_named_download() {
        let store = store().await;
        store.save(&record("item-1")).await.expect("save one");
        store.save(&record("item-2")).await.expect("save two");

        store
            .delete("client-alpha", "item-1")
            .await
            .expect("delete");
        store
            .delete("client-alpha", "item-missing")
            .await
            .expect("delete unknown");

        assert_eq!(
            store.get("client-alpha", "item-1").await.expect("get"),
            None
        );
        assert!(
            store
                .get("client-alpha", "item-2")
                .await
                .expect("get")
                .is_some()
        );
        assert_eq!(store.get("client-beta", "item-2").await.expect("get"), None);
    }
}
