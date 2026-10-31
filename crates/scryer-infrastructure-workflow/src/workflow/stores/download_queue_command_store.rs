use std::collections::HashMap;

use super::*;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, DownloadQueueCommandRecord, DownloadQueueCommandRepository,
};
use scryer_domain::{DownloadQueueCommandAction, DownloadQueueDeleteStatus, Id};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

/// Sources per delete-command lookup query: far below SQLite's 1000-deep
/// expression limit, and at four parameters per source below even the legacy
/// 999 bound-parameter limit.
const DELETE_COMMAND_SOURCE_CHUNK: usize = 200;

#[derive(Clone)]
pub struct DownloadQueueCommandStore {
    datastore: StoreDatastore,
}

impl DownloadQueueCommandStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl DownloadQueueCommandRepository for DownloadQueueCommandStore {
    async fn queue_delete_command(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        let client_id = client_id.map(str::to_string);
        let client_type = client_type.to_string();
        let download_client_item_id = download_client_item_id.to_string();
        let requested_by_user_id = requested_by_user_id.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "queue_delete_download_command",
            move |tx| {
                let client_id = client_id.clone();
                let client_type = client_type.clone();
                let download_client_item_id = download_client_item_id.clone();
                let requested_by_user_id = requested_by_user_id.clone();
                Box::pin(async move {
                    let id = Id::new().0;
                    let now = Utc::now();
                    let normalized_client_id = normalize_download_client_id(client_id.as_deref());
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO download_queue_commands
                         (id, action, client_id, client_type, download_client_item_id, is_history, status, error_text, requested_by_user_id, started_at, finished_at, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, NULL, {}, NULL, NULL, {}, {})
                         ON CONFLICT DO NOTHING",
                        &[
                            SqlArg::Text(id),
                            SqlArg::Text(DownloadQueueCommandAction::Delete.as_str().to_string()),
                            SqlArg::Text(normalized_client_id.clone()),
                            SqlArg::Text(client_type.clone()),
                            SqlArg::Text(download_client_item_id.clone()),
                            SqlArg::Bool(is_history),
                            SqlArg::Text(DownloadQueueDeleteStatus::Queued.as_str().to_string()),
                            SqlArg::OptText(requested_by_user_id),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                        ],
                    )
                    .await?;
                    fetch_optional_delete_command(
                        SqlExec::Tx(tx),
                        "WHERE action = {}
                           AND COALESCE(client_id, '') = {}
                           AND client_type = {}
                           AND download_client_item_id = {}
                           AND is_history = {}
                           AND status IN ('queued', 'running')
                         ORDER BY created_at DESC, id DESC
                         LIMIT 1",
                        &[
                            SqlArg::Text(DownloadQueueCommandAction::Delete.as_str().to_string()),
                            SqlArg::Text(normalized_client_id),
                            SqlArg::Text(client_type),
                            SqlArg::Text(download_client_item_id),
                            SqlArg::Bool(is_history),
                        ],
                    )
                    .await?
                    .ok_or_else(|| AppError::Repository("failed to load queued delete command".into()))
                })
            },
        )
        .await
    }

    async fn queue_delete_command_for_download(
        &self,
        canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        let canonical_download_id = canonical_download_id.map(ToString::to_string);
        let client_id = client_id.map(str::to_string);
        let client_type = client_type.to_string();
        let download_client_item_id = download_client_item_id.to_string();
        let requested_by_user_id = requested_by_user_id.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "queue_delete_download_command_for_download",
            move |tx| {
                let canonical_download_id = canonical_download_id.clone();
                let client_id = client_id.clone();
                let client_type = client_type.clone();
                let download_client_item_id = download_client_item_id.clone();
                let requested_by_user_id = requested_by_user_id.clone();
                Box::pin(async move {
                    let id = Id::new().0;
                    let now = Utc::now();
                    let normalized_client_id = normalize_download_client_id(client_id.as_deref());
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO download_queue_commands
                         (id, action, canonical_download_id, client_id, client_type, download_client_item_id, is_history, status, error_text, requested_by_user_id, started_at, finished_at, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, NULL, {}, NULL, NULL, {}, {})
                         ON CONFLICT DO NOTHING",
                        &[
                            SqlArg::Text(id),
                            SqlArg::Text(DownloadQueueCommandAction::Delete.as_str().to_string()),
                            SqlArg::OptText(canonical_download_id.clone()),
                            SqlArg::Text(normalized_client_id.clone()),
                            SqlArg::Text(client_type.clone()),
                            SqlArg::Text(download_client_item_id.clone()),
                            SqlArg::Bool(is_history),
                            SqlArg::Text(DownloadQueueDeleteStatus::Queued.as_str().to_string()),
                            SqlArg::OptText(requested_by_user_id),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                        ],
                    )
                    .await?;
                    if let Some(canonical_download_id) = canonical_download_id {
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "UPDATE download_queue_commands
                             SET canonical_download_id = {}
                             WHERE action = 'delete'
                               AND COALESCE(client_id, '') = {}
                               AND client_type = {}
                               AND download_client_item_id = {}
                               AND is_history = {}
                               AND status IN ('queued', 'running')
                               AND canonical_download_id IS NULL",
                            &[
                                SqlArg::Text(canonical_download_id),
                                SqlArg::Text(normalized_client_id.clone()),
                                SqlArg::Text(client_type.clone()),
                                SqlArg::Text(download_client_item_id.clone()),
                                SqlArg::Bool(is_history),
                            ],
                        )
                        .await?;
                    }
                    fetch_optional_delete_command(
                        SqlExec::Tx(tx),
                        "WHERE action = {}
                           AND COALESCE(client_id, '') = {}
                           AND client_type = {}
                           AND download_client_item_id = {}
                           AND is_history = {}
                           AND status IN ('queued', 'running')
                         ORDER BY created_at DESC, id DESC
                         LIMIT 1",
                        &[
                            SqlArg::Text(DownloadQueueCommandAction::Delete.as_str().to_string()),
                            SqlArg::Text(normalized_client_id),
                            SqlArg::Text(client_type),
                            SqlArg::Text(download_client_item_id),
                            SqlArg::Bool(is_history),
                        ],
                    )
                    .await?
                    .ok_or_else(|| AppError::Repository("failed to load queued delete command".into()))
                })
            },
        )
        .await
    }

    async fn recover_stale_running_delete_commands(&self, stale_seconds: i64) -> AppResult<u64> {
        let now = Utc::now();
        let cutoff = now - chrono::Duration::seconds(stale_seconds);
        let rows = execute_write(
            &self.datastore,
            "recover_stale_running_delete_download_commands",
            "UPDATE download_queue_commands
             SET status = 'queued',
                 error_text = NULL,
                 started_at = NULL,
                 finished_at = NULL,
                 updated_at = {}
             WHERE action = 'delete'
               AND status = 'running'
               AND updated_at <= {}"
                .to_string(),
            vec![SqlArg::Timestamp(now), SqlArg::Timestamp(cutoff)],
        )
        .await?;
        Ok(rows)
    }

    async fn list_pending_delete_commands(&self) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        fetch_delete_commands(
            self.datastore.read_exec(),
            "WHERE action = 'delete' AND status = 'queued' ORDER BY created_at ASC, id ASC",
            &[],
        )
        .await
    }

    async fn mark_delete_command_running(&self, id: &str) -> AppResult<()> {
        update_delete_command_status(
            &self.datastore,
            id,
            DownloadQueueDeleteStatus::Running,
            None,
        )
        .await
    }

    async fn mark_delete_command_completed(&self, id: &str) -> AppResult<()> {
        update_delete_command_status(
            &self.datastore,
            id,
            DownloadQueueDeleteStatus::Completed,
            None,
        )
        .await
    }

    async fn mark_delete_command_failed(
        &self,
        id: &str,
        error_text: Option<&str>,
    ) -> AppResult<()> {
        update_delete_command_status(
            &self.datastore,
            id,
            DownloadQueueDeleteStatus::Failed,
            error_text,
        )
        .await
    }

    async fn list_latest_delete_commands_for_sources(
        &self,
        sources: &[(Option<String>, String, String, bool)],
        completed_only: bool,
    ) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        let completed_clause = if completed_only {
            " AND status = 'completed'"
        } else {
            ""
        };
        // SQLite rejects an OR chain deeper than 1000 terms and caps bound
        // parameters, so the sources are queried in chunks. Every row of a
        // given (client_id, client_type, item_id, is_history) key matches every
        // source that selects that key, so any chunk that returns a key returns
        // all of its rows, newest first. Keeping the first row seen per key
        // across chunks therefore still selects the newest command.
        let mut latest = HashMap::new();
        for chunk in sources.chunks(DELETE_COMMAND_SOURCE_CHUNK) {
            let mut args = Vec::with_capacity(chunk.len() * 4);
            let mut clauses = Vec::with_capacity(chunk.len());
            for (client_id, client_type, download_client_item_id, is_history) in chunk {
                let normalized_client_id = normalize_download_client_id(client_id.as_deref());
                let client_clause = if normalized_client_id.is_empty() {
                    "COALESCE(client_id, '') = ''".to_string()
                } else {
                    args.push(SqlArg::Text(normalized_client_id));
                    "(COALESCE(client_id, '') = {} OR COALESCE(client_id, '') = '')".to_string()
                };
                args.push(SqlArg::Text(client_type.clone()));
                args.push(SqlArg::Text(download_client_item_id.clone()));
                args.push(SqlArg::Bool(*is_history));
                clauses.push(format!(
                    "({client_clause} AND client_type = {{}} AND download_client_item_id = {{}} AND is_history = {{}})"
                ));
            }
            let rows = fetch_delete_commands(
                self.datastore.read_exec(),
                &format!(
                    "WHERE action = 'delete'{completed_clause} AND ({}) ORDER BY created_at DESC, id DESC",
                    clauses.join(" OR ")
                ),
                &args,
            )
            .await?;
            for record in rows {
                let key = (
                    record.client_id.clone().unwrap_or_default(),
                    record.client_type.clone(),
                    record.download_client_item_id.clone(),
                    record.is_history,
                );
                latest.entry(key).or_insert(record);
            }
        }
        Ok(latest.into_values().collect())
    }

    async fn prune_terminal_delete_commands_older_than(&self, days: i64) -> AppResult<u32> {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let rows = execute_write(
            &self.datastore,
            "prune_terminal_delete_download_commands_older_than",
            "DELETE FROM download_queue_commands
             WHERE action = 'delete'
               AND status IN ('completed', 'failed')
               AND updated_at < {}"
                .to_string(),
            vec![SqlArg::Timestamp(cutoff)],
        )
        .await?;
        Ok(rows as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    async fn store() -> DownloadQueueCommandStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        sqlx::query(
            "CREATE TABLE download_queue_commands (
                 id TEXT PRIMARY KEY,
                 action TEXT NOT NULL,
                 canonical_download_id TEXT,
                 client_id TEXT,
                 client_type TEXT NOT NULL,
                 download_client_item_id TEXT NOT NULL,
                 is_history INTEGER NOT NULL,
                 status TEXT NOT NULL,
                 error_text TEXT,
                 requested_by_user_id TEXT,
                 started_at TEXT,
                 finished_at TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE UNIQUE INDEX idx_download_queue_commands_active_unique
                 ON download_queue_commands(
                     action,
                     COALESCE(client_id, ''),
                     client_type,
                     download_client_item_id,
                     is_history
                 )
                 WHERE status IN ('queued', 'running')",
        )
        .execute(&pool)
        .await
        .expect("queue command fixture table should be created");
        DownloadQueueCommandStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    #[tokio::test]
    async fn queue_delete_command_for_download_persists_canonical_id() {
        let store = store().await;
        let canonical_download_id = scryer_domain::download_identity::DownloadId::new();

        let command = store
            .queue_delete_command_for_download(
                Some(&canonical_download_id),
                Some("client-1"),
                "nzbget",
                "job-1",
                false,
                None,
            )
            .await
            .expect("queue delete command");

        assert_eq!(command.canonical_download_id, Some(canonical_download_id));
    }

    #[tokio::test]
    async fn queue_delete_dedup_enriches_but_never_replaces_canonical_id() {
        let store = store().await;
        let canonical_download_id = scryer_domain::download_identity::DownloadId::new();
        let other_download_id = scryer_domain::download_identity::DownloadId::new();

        let legacy = store
            .queue_delete_command(None, "nzbget", "legacy-first", false, None)
            .await
            .expect("queue legacy delete command");
        let deduped_canonical = store
            .queue_delete_command_for_download(
                Some(&canonical_download_id),
                None,
                "nzbget",
                "legacy-first",
                false,
                None,
            )
            .await
            .expect("dedupe canonical delete command");
        assert_eq!(deduped_canonical.id, legacy.id);
        assert_eq!(
            deduped_canonical.canonical_download_id,
            Some(canonical_download_id)
        );
        assert_eq!(deduped_canonical.updated_at, legacy.updated_at);

        let canonical = store
            .queue_delete_command_for_download(
                Some(&canonical_download_id),
                Some("client-2"),
                "nzbget",
                "canonical-first",
                true,
                None,
            )
            .await
            .expect("queue canonical delete command");
        let deduped_legacy = store
            .queue_delete_command(Some("client-2"), "nzbget", "canonical-first", true, None)
            .await
            .expect("dedupe legacy delete command");
        assert_eq!(deduped_legacy.id, canonical.id);
        assert_eq!(
            deduped_legacy.canonical_download_id,
            Some(canonical_download_id)
        );

        let deduped_other = store
            .queue_delete_command_for_download(
                Some(&other_download_id),
                Some("client-2"),
                "nzbget",
                "canonical-first",
                true,
                None,
            )
            .await
            .expect("dedupe conflicting canonical delete command");
        assert_eq!(deduped_other.id, canonical.id);
        assert_eq!(
            deduped_other.canonical_download_id,
            Some(canonical_download_id)
        );
    }

    #[tokio::test]
    async fn completed_delete_evidence_ignores_newer_noncompleted_commands() {
        let store = store().await;
        let completed = store
            .queue_delete_command(Some("client-1"), "weaver", "job-1", false, None)
            .await
            .expect("queue completed command");
        store
            .mark_delete_command_completed(&completed.id)
            .await
            .expect("complete first command");
        let failed = store
            .queue_delete_command(Some("client-1"), "weaver", "job-1", false, None)
            .await
            .expect("queue failed command");
        store
            .mark_delete_command_failed(&failed.id, Some("fixture failure"))
            .await
            .expect("fail second command");
        store
            .queue_delete_command(Some("client-1"), "weaver", "job-1", false, None)
            .await
            .expect("queue pending command");

        let evidence = store
            .list_latest_delete_commands_for_sources(
                &[(
                    Some("client-1".to_string()),
                    "weaver".to_string(),
                    "job-1".to_string(),
                    false,
                )],
                true,
            )
            .await
            .expect("load completed evidence");

        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].id, completed.id);
    }

    #[tokio::test]
    async fn latest_delete_commands_load_for_more_sources_than_one_query_allows() {
        let store = store().await;
        let StoreDatastore::Sqlite { pool, .. } = &store.datastore else {
            panic!("fixture store should be sqlite");
        };
        const SOURCE_COUNT: usize = 1_200;
        let source = |index: usize| {
            (
                Some("client-1".to_string()),
                "weaver".to_string(),
                format!("fixture-job-{index:04}"),
                index % 2 == 1,
            )
        };
        let mut tx = pool.begin().await.expect("begin fixture insert");
        for index in 0..SOURCE_COUNT {
            let (client_id, client_type, item_id, is_history) = source(index);
            sqlx::query(
                "INSERT INTO download_queue_commands
                 (id, action, client_id, client_type, download_client_item_id, is_history, status, created_at, updated_at)
                 VALUES (?, 'delete', ?, ?, ?, ?, 'completed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            )
            .bind(format!("cmd-{index:04}"))
            .bind(client_id)
            .bind(client_type)
            .bind(item_id)
            .bind(is_history)
            .execute(&mut *tx)
            .await
            .expect("insert fixture delete command");
        }
        sqlx::query(
            "INSERT INTO download_queue_commands
             (id, action, client_id, client_type, download_client_item_id, is_history, status, created_at, updated_at)
             VALUES ('cmd-0000-newer', 'delete', 'client-1', 'weaver', 'fixture-job-0000', 0, 'queued', '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z')",
        )
        .execute(&mut *tx)
        .await
        .expect("insert newer fixture delete command");
        tx.commit().await.expect("commit fixture insert");

        // The doubled source is requested again at the end so it is looked up
        // in both the first and the last chunk.
        let mut sources = (0..SOURCE_COUNT).map(source).collect::<Vec<_>>();
        sources.push(source(0));

        let commands = store
            .list_latest_delete_commands_for_sources(&sources, false)
            .await
            .expect("load delete commands for every source");

        assert_eq!(commands.len(), SOURCE_COUNT);
        let by_item = commands
            .iter()
            .map(|command| (command.download_client_item_id.as_str(), command))
            .collect::<HashMap<_, _>>();
        for index in 0..SOURCE_COUNT {
            let (_, _, item_id, is_history) = source(index);
            let command = by_item
                .get(item_id.as_str())
                .unwrap_or_else(|| panic!("missing delete command for {item_id}"));
            assert_eq!(command.is_history, is_history);
            let expected_id = if index == 0 {
                "cmd-0000-newer".to_string()
            } else {
                format!("cmd-{index:04}")
            };
            assert_eq!(command.id, expected_id);
        }
    }
}
