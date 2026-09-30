use super::*;

use async_trait::async_trait;
use scryer_application::{
    AppResult, ExternalImportMonitorSnapshotChunk, ExternalImportMonitorSnapshotEntryKind,
    ExternalImportMonitorSnapshotRepository,
};
use scryer_domain::MediaFacet;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

#[derive(Clone)]
pub struct ExternalImportMonitorStore {
    datastore: StoreDatastore,
}

impl ExternalImportMonitorStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl ExternalImportMonitorSnapshotRepository for ExternalImportMonitorStore {
    async fn claim_external_import_monitor_snapshot(
        &self,
        session_id: &str,
        consumed_session_id: &str,
        facet: MediaFacet,
    ) -> AppResult<u64> {
        execute_write(
            &self.datastore,
            "claim_external_import_monitor_snapshot",
            "UPDATE external_import_monitor_snapshot_chunks SET session_id = {}
             WHERE session_id = {} AND facet = {}"
                .to_string(),
            vec![
                SqlArg::Text(consumed_session_id.to_string()),
                SqlArg::Text(session_id.to_string()),
                SqlArg::Text(facet.as_str().to_string()),
            ],
        )
        .await
        .map_err(map_snapshot_chunk_error)
    }

    async fn append_external_import_monitor_snapshot_chunk(
        &self,
        chunk: &ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()> {
        let chunk = chunk.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "append_external_import_monitor_snapshot_chunk",
            move |tx| {
                let chunk = chunk.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO external_import_monitor_snapshot_chunks
                         (session_id, facet, entry_kind, chunk_index, payload_ndjson, created_at)
                         VALUES ({}, {}, {}, {}, {}, {})
                         ON CONFLICT(session_id, facet, entry_kind, chunk_index) DO UPDATE SET
                             payload_ndjson = excluded.payload_ndjson,
                             created_at = excluded.created_at",
                        &[
                            SqlArg::Text(chunk.session_id),
                            SqlArg::Text(chunk.facet.as_str().to_string()),
                            SqlArg::Text(chunk.entry_kind.as_str().to_string()),
                            SqlArg::I32(chunk.chunk_index),
                            SqlArg::Text(chunk.payload_ndjson),
                            SqlArg::Timestamp(parse_datetime_or_now(Some(&chunk.created_at))),
                        ],
                    )
                    .await
                    .map_err(map_snapshot_chunk_error)?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn list_external_import_monitor_snapshot_chunk_batch(
        &self,
        session_id: &str,
        facet: MediaFacet,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
        fetch_snapshot_chunks(
            self.datastore.read_exec(),
            "SELECT session_id, facet, entry_kind, chunk_index, payload_ndjson, created_at
             FROM external_import_monitor_snapshot_chunks
             WHERE session_id = {} AND facet = {} AND entry_kind = {} AND ({} IS NULL OR chunk_index > {})
             ORDER BY chunk_index ASC
             LIMIT {}",
            &[
                SqlArg::Text(session_id.to_string()),
                SqlArg::Text(facet.as_str().to_string()),
                SqlArg::Text(entry_kind.as_str().to_string()),
                SqlArg::OptI32(after_chunk_index),
                SqlArg::OptI32(after_chunk_index),
                SqlArg::I32(limit),
            ],
        )
        .await
    }

    async fn delete_external_import_monitor_snapshot_chunks(
        &self,
        session_id: &str,
        facet: MediaFacet,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_external_import_monitor_snapshot_chunks",
            "DELETE FROM external_import_monitor_snapshot_chunks WHERE session_id = {} AND facet = {}"
                .to_string(),
            vec![
                SqlArg::Text(session_id.to_string()),
                SqlArg::Text(facet.as_str().to_string()),
            ],
        )
        .await
        .map_err(map_snapshot_chunk_error)?;
        Ok(())
    }

    async fn delete_external_import_monitor_snapshot_chunks_for_session_prefix(
        &self,
        session_prefix: &str,
        facet: MediaFacet,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_external_import_monitor_snapshot_chunks_for_session_prefix",
            "DELETE FROM external_import_monitor_snapshot_chunks
              WHERE session_id LIKE {} AND facet = {}"
                .to_string(),
            vec![
                SqlArg::Text(format!("{session_prefix}%")),
                SqlArg::Text(facet.as_str().to_string()),
            ],
        )
        .await
        .map_err(map_snapshot_chunk_error)?;
        Ok(())
    }

    async fn delete_external_import_monitor_snapshot_chunks_except_session_prefix(
        &self,
        preserved_session_prefix: &str,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_external_import_monitor_snapshot_chunks_except_session_prefix",
            "DELETE FROM external_import_monitor_snapshot_chunks WHERE session_id NOT LIKE {}"
                .to_string(),
            vec![SqlArg::Text(format!("{preserved_session_prefix}%"))],
        )
        .await
        .map_err(map_snapshot_chunk_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    async fn assert_snapshot_claim_behavior(store: ExternalImportMonitorStore) {
        let movie = MediaFacet::Movie;
        let series = MediaFacet::Series;
        let chunk = |session: &str, facet: MediaFacet, index| ExternalImportMonitorSnapshotChunk {
            session_id: session.into(),
            entry_kind: if facet == MediaFacet::Movie {
                ExternalImportMonitorSnapshotEntryKind::Movie
            } else {
                ExternalImportMonitorSnapshotEntryKind::Series
            },
            facet,
            chunk_index: index,
            payload_ndjson: "{}".into(),
            created_at: "2026-09-25T12:00:00Z".into(),
        };
        for item in [
            chunk("pending-a", movie.clone(), 0),
            chunk("pending-a", movie.clone(), 1),
            chunk("pending-b", movie.clone(), 0),
            chunk("pending-a", series.clone(), 0),
        ] {
            store
                .append_external_import_monitor_snapshot_chunk(&item)
                .await
                .unwrap();
        }
        // A collision on a later chunk must roll back the entire claim.
        store
            .append_external_import_monitor_snapshot_chunk(&chunk("collision", movie.clone(), 1))
            .await
            .unwrap();
        assert!(
            store
                .claim_external_import_monitor_snapshot("pending-a", "collision", movie.clone())
                .await
                .is_err()
        );
        let remaining = store
            .list_external_import_monitor_snapshot_chunk_batch(
                "pending-a",
                movie.clone(),
                ExternalImportMonitorSnapshotEntryKind::Movie,
                None,
                10,
            )
            .await
            .unwrap();
        assert_eq!(remaining.len(), 2);
        let (a, b) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            tokio::join!(
                store.claim_external_import_monitor_snapshot(
                    "pending-a",
                    "consumed-a",
                    movie.clone()
                ),
                store.claim_external_import_monitor_snapshot(
                    "pending-a",
                    "consumed-b",
                    movie.clone()
                ),
            )
        })
        .await
        .expect("concurrent claims must complete");
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(matches!((a, b), (2, 0) | (0, 2)));
        let winner = if a == 2 { "consumed-a" } else { "consumed-b" };
        assert_eq!(
            store
                .claim_external_import_monitor_snapshot(
                    "pending-a",
                    "another-attempt",
                    movie.clone()
                )
                .await
                .unwrap(),
            0
        );
        store
            .delete_external_import_monitor_snapshot_chunks(winner, movie.clone())
            .await
            .unwrap();
        assert_eq!(
            store
                .list_external_import_monitor_snapshot_chunk_batch(
                    "pending-b",
                    movie.clone(),
                    ExternalImportMonitorSnapshotEntryKind::Movie,
                    None,
                    10
                )
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .list_external_import_monitor_snapshot_chunk_batch(
                    "pending-a",
                    series,
                    ExternalImportMonitorSnapshotEntryKind::Series,
                    None,
                    10
                )
                .await
                .unwrap()
                .len(),
            1
        );
        store
            .append_external_import_monitor_snapshot_chunk(&chunk("pending-a", movie.clone(), 0))
            .await
            .unwrap();
        assert_eq!(
            store
                .claim_external_import_monitor_snapshot("pending-a", "fresh-attempt", movie)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn sqlite_external_import_monitor_snapshot_claim_is_atomic_and_scoped() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE external_import_monitor_snapshot_chunks (
            session_id TEXT NOT NULL, facet TEXT NOT NULL, entry_kind TEXT NOT NULL,
            chunk_index INTEGER NOT NULL, payload_ndjson TEXT NOT NULL, created_at TEXT NOT NULL,
            PRIMARY KEY (session_id, facet, entry_kind, chunk_index))",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_snapshot_claim_behavior(ExternalImportMonitorStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        }))
        .await;
    }

    #[tokio::test]
    async fn postgres_external_import_monitor_snapshot_claim_is_atomic_and_scoped() {
        let Ok(url) = std::env::var("SCRYER_TEST_POSTGRES_URL") else {
            eprintln!("skipped: SCRYER_TEST_POSTGRES_URL must name an isolated test database");
            return;
        };
        // One connection owns a temporary table; no persistent schema or user data is touched.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query("CREATE TEMP TABLE external_import_monitor_snapshot_chunks (
            session_id TEXT NOT NULL, facet TEXT NOT NULL, entry_kind TEXT NOT NULL,
            chunk_index INTEGER NOT NULL, payload_ndjson TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL,
            PRIMARY KEY (session_id, facet, entry_kind, chunk_index))")
            .execute(&pool).await.unwrap();
        assert_snapshot_claim_behavior(ExternalImportMonitorStore::new(StoreDatastore::Postgres {
            pool: pool.clone(),
        }))
        .await;
        pool.close().await;
    }
}
