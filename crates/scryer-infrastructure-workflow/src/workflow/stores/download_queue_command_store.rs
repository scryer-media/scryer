use std::collections::HashMap;

use super::*;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, DownloadQueueCommandRecord, DownloadQueueCommandRepository,
};
use scryer_domain::{DownloadQueueCommandAction, DownloadQueueDeleteStatus, Id};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

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
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        let mut args = Vec::new();
        let mut clauses = Vec::with_capacity(sources.len());
        for (client_id, client_type, download_client_item_id, is_history) in sources {
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
        let completed_clause = if completed_only {
            " AND status = 'completed'"
        } else {
            ""
        };
        let rows = fetch_delete_commands(
            self.datastore.read_exec(),
            &format!(
                "WHERE action = 'delete'{completed_clause} AND ({}) ORDER BY created_at DESC, id DESC",
                clauses.join(" OR ")
            ),
            &args,
        )
        .await?;
        let mut latest = HashMap::new();
        for record in rows {
            let key = (
                record.client_id.clone().unwrap_or_default(),
                record.client_type.clone(),
                record.download_client_item_id.clone(),
                record.is_history,
            );
            latest.entry(key).or_insert(record);
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
}
