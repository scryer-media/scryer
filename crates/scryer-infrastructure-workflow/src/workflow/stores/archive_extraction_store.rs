use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashSet;

use scryer_application::{
    AppError, AppResult, ArchiveExtractionRecord, ArchiveExtractionRepository,
    ArchiveExtractionState, ArchiveScratchSummary, UnsettledArchiveExtraction,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const COLUMNS: &str = "client_id, client_item_id, state, workspace_path, archive_digest, \
                       attempts, last_reason_code, output_count, output_bytes, \
                       next_attempt_at, settled_at, created_at, updated_at";

/// `archive_extractions` (migration 0357): one row per tracked download whose
/// archive workspace is on disk. Reads and writes are by primary key, except
/// the settled and unsettled scans, which `idx_archive_extractions_settled_at`
/// serves, the workspace-name lookup, which `idx_archive_extractions_workspace_name`
/// serves, and the scratch summary, which reads the whole table: it holds only
/// downloads with scratch on disk.
#[derive(Clone)]
pub struct ArchiveExtractionStore {
    datastore: StoreDatastore,
}

impl ArchiveExtractionStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    /// Set `settled_at` for the keyed rows the `condition` prefix admits, in
    /// one transaction of bind-safe chunks. No statement runs for no keys.
    async fn set_settled_at(
        &self,
        keys: &[(String, String)],
        at: DateTime<Utc>,
        condition: &'static str,
        op_name: &'static str,
    ) -> AppResult<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let keys = keys.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, op_name, move |tx| {
            let keys = keys.clone();
            Box::pin(async move {
                for chunk in keys.chunks(KEY_CHUNK) {
                    let mut args = vec![SqlArg::Timestamp(at), SqlArg::Timestamp(at)];
                    let predicate = keys_predicate(chunk, &mut args);
                    SqlRuntime::execute(
                        SqlExec::Tx(&mut *tx),
                        &format!(
                            "UPDATE archive_extractions SET settled_at = {{}}, updated_at = {{}} \
                             WHERE {condition}({predicate})"
                        ),
                        &args,
                    )
                    .await?;
                }
                Ok(())
            })
        })
        .await
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

/// Keys per statement: two binds each, well inside either datastore's limit.
const KEY_CHUNK: usize = 400;

/// Workspace names per statement.
const NAME_CHUNK: usize = 500;

/// A workspace path's last 16 characters, its name; the expression
/// `idx_archive_extractions_workspace_name` indexes, spelled the same.
const WORKSPACE_NAME_EXPR: &str = "substr(workspace_path, length(workspace_path) - 15)";

fn limit_arg(limit: usize) -> SqlArg {
    SqlArg::I64(i64::try_from(limit).unwrap_or(i64::MAX))
}

/// A `WHERE` fragment for a chunk of keys: one equality per client with its
/// item ids in an `IN` list, which the primary key answers.
fn keys_predicate(keys: &[(String, String)], args: &mut Vec<SqlArg>) -> String {
    let mut groups: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for (client_id, client_item_id) in keys {
        groups
            .entry(client_id.as_str())
            .or_default()
            .push(client_item_id.as_str());
    }
    groups
        .into_iter()
        .map(|(client_id, items)| {
            args.push(SqlArg::Text(client_id.to_string()));
            let item_placeholders = vec!["{}"; items.len()].join(", ");
            args.extend(items.into_iter().map(|item| SqlArg::Text(item.to_string())));
            format!("(client_id = {{}} AND client_item_id IN ({item_placeholders}))")
        })
        .collect::<Vec<_>>()
        .join(" OR ")
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

    async fn get_many(&self, keys: &[(String, String)]) -> AppResult<Vec<ArchiveExtractionRecord>> {
        let mut records = Vec::new();
        for chunk in keys.chunks(KEY_CHUNK) {
            let mut args = Vec::with_capacity(chunk.len() * 2);
            let predicate = keys_predicate(chunk, &mut args);
            for row in SqlRuntime::fetch_all(
                self.datastore.read_exec(),
                &format!("SELECT {COLUMNS} FROM archive_extractions WHERE {predicate}"),
                &args,
            )
            .await?
            {
                records.push(record_from_row(&row)?);
            }
        }
        Ok(records)
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

    async fn settle(&self, keys: &[(String, String)], at: DateTime<Utc>) -> AppResult<()> {
        self.set_settled_at(
            keys,
            at,
            "settled_at IS NULL AND ",
            "settle_archive_extractions",
        )
        .await
    }

    async fn defer_settled(&self, keys: &[(String, String)], at: DateTime<Utc>) -> AppResult<()> {
        self.set_settled_at(keys, at, "", "defer_settled_archive_extractions")
            .await
    }

    async fn list_settled_before(
        &self,
        before: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<ArchiveExtractionRecord>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {COLUMNS} FROM archive_extractions \
                 WHERE settled_at <= {{}} ORDER BY settled_at LIMIT {{}}"
            ),
            &[SqlArg::Timestamp(before), limit_arg(limit)],
        )
        .await?
        .iter()
        .map(record_from_row)
        .collect()
    }

    async fn delete_many(&self, keys: &[(String, String)]) -> AppResult<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let keys = keys.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_archive_extractions", move |tx| {
            let keys = keys.clone();
            Box::pin(async move {
                for chunk in keys.chunks(KEY_CHUNK) {
                    let mut args = Vec::with_capacity(chunk.len() * 2);
                    let predicate = keys_predicate(chunk, &mut args);
                    SqlRuntime::execute(
                        SqlExec::Tx(&mut *tx),
                        &format!("DELETE FROM archive_extractions WHERE {predicate}"),
                        &args,
                    )
                    .await?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn list_unsettled_after(
        &self,
        after: Option<(String, String)>,
        changed_before: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<UnsettledArchiveExtraction>> {
        let mut args = vec![SqlArg::Timestamp(changed_before)];
        let after_predicate = match after {
            Some((client_id, client_item_id)) => {
                args.extend(key_args(&client_id, &client_item_id));
                "AND (client_id, client_item_id) > ({}, {}) "
            }
            None => "",
        };
        args.push(limit_arg(limit));
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT client_id, client_item_id, updated_at FROM archive_extractions \
                 WHERE settled_at IS NULL AND updated_at <= {{}} {after_predicate}\
                 ORDER BY client_id, client_item_id LIMIT {{}}"
            ),
            &args,
        )
        .await?
        .iter()
        .map(|row| {
            Ok(UnsettledArchiveExtraction {
                client_id: row.text("client_id")?,
                client_item_id: row.text("client_item_id")?,
                updated_at: row.timestamp("updated_at")?,
            })
        })
        .collect()
    }

    async fn recorded_workspace_names(&self, names: &[String]) -> AppResult<HashSet<String>> {
        let mut recorded = HashSet::new();
        for chunk in names.chunks(NAME_CHUNK) {
            let placeholders = vec!["{}"; chunk.len()].join(", ");
            let args: Vec<_> = chunk.iter().cloned().map(SqlArg::Text).collect();
            for row in SqlRuntime::fetch_all(
                self.datastore.read_exec(),
                &format!(
                    "SELECT DISTINCT {WORKSPACE_NAME_EXPR} AS workspace_name \
                     FROM archive_extractions \
                     WHERE {WORKSPACE_NAME_EXPR} IN ({placeholders})"
                ),
                &args,
            )
            .await?
            {
                recorded.insert(row.text("workspace_name")?);
            }
        }
        Ok(recorded)
    }

    async fn scratch_summary(&self) -> AppResult<ArchiveScratchSummary> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            // Only extracted and extracting downloads have output on disk; a
            // failed, blocked, password-waiting or pending one's was removed.
            "SELECT CAST(COALESCE(SUM(CASE WHEN state IN ('extracted', 'extracting') \
                         THEN 1 ELSE 0 END), 0) AS BIGINT) AS workspaces, \
                    CAST(COALESCE(SUM(CASE WHEN state IN ('extracted', 'extracting') \
                         THEN output_bytes ELSE 0 END), 0) AS BIGINT) AS bytes, \
                    CAST(COALESCE(SUM(CASE WHEN state = 'blocked' THEN 1 ELSE 0 END), 0) \
                         AS BIGINT) AS blocked \
             FROM archive_extractions",
            &[],
        )
        .await?;
        let Some(row) = row else {
            return Ok(ArchiveScratchSummary::default());
        };
        Ok(ArchiveScratchSummary {
            workspaces: row.i64("workspaces")?.max(0) as u64,
            bytes: row.i64("bytes")?.max(0) as u64,
            blocked: row.i64("blocked")?.max(0) as u64,
        })
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
            "../../../../scryer/src/db/migrations/0358_archive_extractions.sql"
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

    fn key(client: &str, item: &str) -> (String, String) {
        (client.to_string(), item.to_string())
    }

    async fn settled_at(store: &ArchiveExtractionStore, item: &str) -> Option<DateTime<Utc>> {
        store
            .get("client-alpha", item)
            .await
            .expect("get")
            .expect("row")
            .settled_at
    }

    #[tokio::test]
    async fn settle_marks_only_the_named_unsettled_downloads() {
        let store = store().await;
        for item in ["item-1", "item-2", "item-3"] {
            store.save(&record(item)).await.expect("save");
        }
        store
            .settle(&[key("client-alpha", "item-1")], at(10))
            .await
            .expect("first settle");

        store
            .settle(
                &[
                    key("client-alpha", "item-1"),
                    key("client-alpha", "item-2"),
                    key("client-beta", "item-3"),
                    key("client-alpha", "item-missing"),
                ],
                at(20),
            )
            .await
            .expect("second settle");

        assert_eq!(
            settled_at(&store, "item-1").await,
            Some(at(10)),
            "keeps its time"
        );
        assert_eq!(settled_at(&store, "item-2").await, Some(at(20)));
        assert_eq!(
            settled_at(&store, "item-3").await,
            None,
            "another client's item"
        );
        store.settle(&[], at(30)).await.expect("no keys");
    }

    #[tokio::test]
    async fn get_many_reads_only_the_named_downloads_across_chunks() {
        let store = store().await;
        let items: Vec<String> = (0..KEY_CHUNK * 2 + 3)
            .map(|n| format!("item-{n}"))
            .collect();
        for item in &items {
            store.save(&record(item)).await.expect("save");
        }
        let mut other = record("item-0");
        other.client_id = "client-beta".to_string();
        store.save(&other).await.expect("save other client");

        let mut keys: Vec<_> = items
            .iter()
            .step_by(2)
            .map(|item| key("client-alpha", item))
            .collect();
        keys.push(key("client-alpha", "item-missing"));
        let mut read: Vec<_> = store
            .get_many(&keys)
            .await
            .expect("get_many")
            .into_iter()
            .map(|row| (row.client_id, row.client_item_id))
            .collect();
        read.sort();
        let mut expected: Vec<_> = items
            .iter()
            .step_by(2)
            .map(|item| key("client-alpha", item))
            .collect();
        expected.sort();
        assert_eq!(read, expected);
        assert!(store.get_many(&[]).await.expect("no keys").is_empty());
        assert_eq!(
            store
                .get_many(&[key("client-beta", "item-0")])
                .await
                .unwrap(),
            vec![other]
        );
    }

    #[tokio::test]
    async fn defer_moves_the_settled_time_of_the_named_downloads() {
        let store = store().await;
        for item in ["item-1", "item-2"] {
            store.save(&record(item)).await.expect("save");
        }
        let both = [key("client-alpha", "item-1"), key("client-alpha", "item-2")];
        store.settle(&both, at(10)).await.expect("settle");

        store
            .defer_settled(&both[..1], at(50))
            .await
            .expect("defer");

        assert_eq!(settled_at(&store, "item-1").await, Some(at(50)));
        assert_eq!(settled_at(&store, "item-2").await, Some(at(10)));
        let row = store.get("client-alpha", "item-1").await.unwrap().unwrap();
        assert_eq!(row.updated_at, at(50));
    }

    #[tokio::test]
    async fn the_settled_scan_returns_due_rows_oldest_first_up_to_its_limit() {
        let store = store().await;
        for (item, settled) in [
            ("item-1", Some(30)),
            ("item-2", Some(10)),
            ("item-3", Some(20)),
            ("item-4", Some(90)),
            ("item-5", None),
        ] {
            let mut row = record(item);
            row.settled_at = settled.map(at);
            store.save(&row).await.expect("save");
        }

        let due = store.list_settled_before(at(30), 10).await.expect("list");
        let items: Vec<_> = due.iter().map(|row| row.client_item_id.as_str()).collect();
        assert_eq!(items, ["item-2", "item-3", "item-1"]);

        let first = store.list_settled_before(at(30), 2).await.expect("list");
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].client_item_id, "item-2");
        assert!(
            store
                .list_settled_before(at(5), 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn the_settled_scan_is_served_by_its_index() {
        let store = store().await;
        let StoreDatastore::Sqlite { pool, .. } = &store.datastore else {
            unreachable!("the fixture is SQLite");
        };
        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "EXPLAIN QUERY PLAN SELECT client_id FROM archive_extractions \
             WHERE settled_at <= ?1 ORDER BY settled_at LIMIT ?2",
        )
        .bind("2026-01-01T00:00:00Z")
        .bind(100_i64)
        .fetch_all(pool)
        .await
        .expect("plan");
        let detail: Vec<_> = plan.iter().map(|row| row.3.as_str()).collect();
        assert!(
            detail
                .iter()
                .any(|line| line.contains("idx_archive_extractions_settled_at")),
            "{detail:?}"
        );
        assert!(
            !detail.iter().any(|line| line.contains("TEMP B-TREE")),
            "{detail:?}"
        );
    }

    async fn plan_details(
        store: &ArchiveExtractionStore,
        sql: &str,
        binds: &[&str],
    ) -> Vec<String> {
        let StoreDatastore::Sqlite { pool, .. } = &store.datastore else {
            unreachable!("the fixture is SQLite");
        };
        let mut query =
            sqlx::query_as::<_, (i64, i64, i64, String)>(sqlx::AssertSqlSafe(sql.to_string()));
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .fetch_all(pool)
            .await
            .expect("plan")
            .into_iter()
            .map(|row| row.3)
            .collect()
    }

    #[tokio::test]
    async fn the_unsettled_scan_pages_in_key_order_past_settled_rows() {
        let store = store().await;
        for (client, item, settled) in [
            ("client-beta", "item-1", None),
            ("client-alpha", "item-3", None),
            ("client-alpha", "item-1", None),
            ("client-alpha", "item-2", Some(10)),
            ("client-alpha", "item-4", None),
        ] {
            let mut row = record(item);
            row.client_id = client.to_string();
            row.settled_at = settled.map(at);
            row.updated_at = at(7);
            store.save(&row).await.expect("save");
        }

        let first = store
            .list_unsettled_after(None, at(7), 2)
            .await
            .expect("first");
        let keys: Vec<_> = first
            .iter()
            .map(|row| (row.client_id.as_str(), row.client_item_id.as_str()))
            .collect();
        assert_eq!(
            keys,
            [("client-alpha", "item-1"), ("client-alpha", "item-3")]
        );
        assert_eq!(first[0].updated_at, at(7));

        let last = first.last().unwrap();
        let rest = store
            .list_unsettled_after(
                Some((last.client_id.clone(), last.client_item_id.clone())),
                at(7),
                10,
            )
            .await
            .expect("rest");
        let keys: Vec<_> = rest
            .iter()
            .map(|row| (row.client_id.as_str(), row.client_item_id.as_str()))
            .collect();
        assert_eq!(
            keys,
            [("client-alpha", "item-4"), ("client-beta", "item-1")]
        );
    }

    #[tokio::test]
    async fn the_unsettled_scan_leaves_rows_changed_after_the_cutoff() {
        let store = store().await;
        for (item, updated) in [("item-1", 5), ("item-2", 9), ("item-3", 7)] {
            let mut row = record(item);
            row.settled_at = None;
            row.updated_at = at(updated);
            store.save(&row).await.expect("save");
        }

        let page = store
            .list_unsettled_after(None, at(7), 10)
            .await
            .expect("page");
        let items: Vec<_> = page.iter().map(|row| row.client_item_id.as_str()).collect();
        assert_eq!(
            items,
            ["item-1", "item-3"],
            "item-2 changed after the cutoff"
        );
        // A full page of old rows is not cut short by recent ones between them.
        let first = store
            .list_unsettled_after(None, at(7), 2)
            .await
            .expect("first");
        let items: Vec<_> = first
            .iter()
            .map(|row| row.client_item_id.as_str())
            .collect();
        assert_eq!(items, ["item-1", "item-3"]);
    }

    #[tokio::test]
    async fn the_unsettled_scan_and_workspace_lookup_are_served_by_their_indexes() {
        let store = store().await;
        let unsettled = plan_details(
            &store,
            "EXPLAIN QUERY PLAN SELECT client_id, client_item_id, updated_at \
             FROM archive_extractions WHERE settled_at IS NULL AND updated_at <= ?1 \
             AND (client_id, client_item_id) > (?2, ?3) \
             ORDER BY client_id, client_item_id LIMIT 500",
            &["2026-01-01T00:00:00Z", "client-alpha", "item-1"],
        )
        .await;
        // The index holds every column the scan reads, the change time
        // included, so the cutoff is checked without reading the rows.
        assert!(
            unsettled
                .iter()
                .any(|line| { line.contains("COVERING INDEX idx_archive_extractions_settled_at") }),
            "{unsettled:?}"
        );
        assert!(
            !unsettled.iter().any(|line| line.contains("TEMP B-TREE")),
            "{unsettled:?}"
        );

        let names = plan_details(
            &store,
            &format!(
                "EXPLAIN QUERY PLAN SELECT DISTINCT {WORKSPACE_NAME_EXPR} AS workspace_name \
                 FROM archive_extractions WHERE {WORKSPACE_NAME_EXPR} IN (?1, ?2)"
            ),
            &["0123456789abcdef", "fedcba9876543210"],
        )
        .await;
        assert!(
            names
                .iter()
                .any(|line| line.contains("idx_archive_extractions_workspace_name")),
            "{names:?}"
        );
    }

    #[tokio::test]
    async fn the_workspace_lookup_names_only_recorded_workspaces() {
        let store = store().await;
        for (item, path) in [
            ("item-1", "/library/.scry-ex/00000000000000a1"),
            // Another spelling of the library root names the same workspace.
            ("item-2", "/library//.scry-ex/00000000000000b2"),
        ] {
            let mut row = record(item);
            row.workspace_path = path.to_string();
            store.save(&row).await.expect("save");
        }
        let mut asked: Vec<String> = (0x1000..(0x1000 + NAME_CHUNK + 20))
            .map(|index| format!("{index:016x}"))
            .collect();
        asked.push("00000000000000b2".to_string());

        let recorded = store
            .recorded_workspace_names(&asked)
            .await
            .expect("lookup");
        assert_eq!(recorded, HashSet::from(["00000000000000b2".to_string()]));
        assert!(
            store
                .recorded_workspace_names(&[])
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn the_scratch_summary_counts_only_output_on_disk_and_blocked_apart() {
        let store = store().await;
        assert_eq!(
            store.scratch_summary().await.expect("empty"),
            ArchiveScratchSummary::default()
        );
        for (item, state, bytes) in [
            (
                "item-1",
                ArchiveExtractionState::Extracted,
                5_000_000_000_u64,
            ),
            ("item-2", ArchiveExtractionState::Blocked, 0),
            ("item-3", ArchiveExtractionState::Extracted, 7),
            ("item-4", ArchiveExtractionState::Extracting, 0),
            // A failed or waiting attempt's output was removed; a stale
            // byte count on its row is not scratch on disk.
            ("item-5", ArchiveExtractionState::Failed, 11),
            ("item-6", ArchiveExtractionState::NeedsPassword, 13),
            ("item-7", ArchiveExtractionState::Pending, 17),
        ] {
            let mut row = record(item);
            row.state = state;
            row.output_bytes = bytes;
            store.save(&row).await.expect("save");
        }
        assert_eq!(
            store.scratch_summary().await.expect("summary"),
            ArchiveScratchSummary {
                workspaces: 3,
                bytes: 5_000_000_007,
                blocked: 1,
            },
            "extracted and extracting count; blocked is counted apart"
        );
    }

    #[tokio::test]
    async fn batches_past_one_statement_settle_and_delete_every_named_row() {
        let store = store().await;
        let mut keys = Vec::new();
        for index in 0..(KEY_CHUNK + 50) {
            let client = if index % 2 == 0 {
                "client-alpha"
            } else {
                "client-beta"
            };
            let item = format!("item-{index}");
            let mut row = record(&item);
            row.client_id = client.to_string();
            store.save(&row).await.expect("save");
            keys.push(key(client, &item));
        }
        let kept = record("item-kept");
        store.save(&kept).await.expect("save kept");

        store.settle(&keys, at(10)).await.expect("settle");
        assert_eq!(
            store
                .list_settled_before(at(10), 10_000)
                .await
                .unwrap()
                .len(),
            keys.len()
        );
        store.delete_many(&keys).await.expect("delete");

        assert!(
            store
                .list_settled_before(at(10), 10_000)
                .await
                .unwrap()
                .is_empty()
        );
        for (client, item) in keys.iter().step_by(97) {
            assert_eq!(store.get(client, item).await.unwrap(), None);
        }
        assert_eq!(
            store.get("client-alpha", "item-kept").await.unwrap(),
            Some(kept)
        );
        store.delete_many(&[]).await.expect("no keys");
    }
}
