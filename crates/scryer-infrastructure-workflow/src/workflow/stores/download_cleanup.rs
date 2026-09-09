// Durable automatic removal is independent of import and client visibility.
use scryer_application::{DownloadCleanupClaim, DownloadCleanupRecord};

const CLEANUP_COLUMNS: &str = "c.download_id, c.client_id, c.client_type, c.item_id,
    c.tracked_state, c.attempts, c.history_offset, c.payload_checkpoint,
    c.title_id, c.facet, c.source_title";

fn cleanup_record(row: &SqlRow) -> AppResult<DownloadCleanupRecord> {
    let id = row.text("download_id")?;
    Ok(DownloadCleanupRecord {
        download_id: DownloadId::parse(&id)
            .ok_or_else(|| AppError::Repository("invalid cleanup download id".into()))?,
        client_id: row.text("client_id")?,
        client_type: row.text("client_type")?,
        item_id: row.text("item_id")?,
        tracked_state: row.text("tracked_state")?,
        attempts: row.opt_i64("attempts")?.unwrap_or(0).max(0) as u32,
        history_offset: row.opt_i64("history_offset")?.unwrap_or(0).max(0) as usize,
        payload_checkpoint: row.opt_text("payload_checkpoint")?,
        title_id: blank_to_none(row.opt_text("title_id")?),
        facet: blank_to_none(row.opt_text("facet")?),
        source_title: blank_to_none(row.opt_text("source_title")?),
    })
}

async fn enqueue_cleanup_tx(tx: &mut SqlTx<'_>, id: &str, state: &str) -> AppResult<()> {
    if !matches!(
        state,
        "imported" | "imported_seeding" | "failed" | "ignored"
    ) {
        return Ok(());
    }
    let now = Utc::now();
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO download_cleanup
         (download_id, client_id, client_type, item_id, title_id, facet, source_title, tracked_state,
          next_attempt_at, created_at, updated_at)
         SELECT d.id, COALESCE(b.client_config_id, s.download_client_id, ''),
                COALESCE(b.client_type_snapshot, s.download_client_type, ''),
                COALESCE(b.native_item_id, s.download_client_item_id, ''),
                s.title_id, s.facet, s.source_title, {}, {}, {}, {}
           FROM downloads d
           JOIN download_submissions s ON s.id = d.id
           LEFT JOIN download_client_bindings b ON b.download_id = d.id
          WHERE d.id = {}
            AND TRIM(COALESCE(b.native_item_id, s.download_client_item_id, '')) <> ''
         ON CONFLICT(download_id) DO UPDATE SET
             tracked_state = excluded.tracked_state,
             status = CASE WHEN download_cleanup.tracked_state <> excluded.tracked_state
                           AND NOT (download_cleanup.tracked_state = 'imported_seeding' AND excluded.tracked_state = 'imported')
                           THEN 'pending' ELSE download_cleanup.status END,
             next_attempt_at = CASE WHEN download_cleanup.tracked_state <> excluded.tracked_state
                           THEN excluded.next_attempt_at ELSE download_cleanup.next_attempt_at END,
             updated_at = excluded.updated_at",
        &[
            SqlArg::Text(state.to_string()), SqlArg::Timestamp(now),
            SqlArg::Timestamp(now), SqlArg::Timestamp(now), SqlArg::Text(id.to_string()),
        ],
    ).await?;
    Ok(())
}

impl DownloadSubmissionStore {
    async fn seed_cleanup(&self, limit: usize) -> AppResult<()> {
        if limit == 0 {
            return Ok(());
        }
        // Keyset-like progress through the missing-intent set: completed and
        // failing cleanup rows cannot monopolize this backfill's first page.
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT d.id, COALESCE(
                (SELECT st.tracked_state FROM download_identity_states st
                 WHERE st.canonical_download_id = d.id ORDER BY st.updated_at DESC, st.id DESC LIMIT 1),
                s.tracked_state) AS tracked_state
             FROM downloads d JOIN download_submissions s ON s.id = d.id
             LEFT JOIN download_client_bindings b ON b.download_id = d.id
             WHERE NOT EXISTS (SELECT 1 FROM download_cleanup c WHERE c.download_id = d.id)
               AND TRIM(COALESCE(b.native_item_id, s.download_client_item_id, '')) <> ''
               AND COALESCE(
                (SELECT st.tracked_state FROM download_identity_states st
                 WHERE st.canonical_download_id = d.id ORDER BY st.updated_at DESC, st.id DESC LIMIT 1),
                s.tracked_state) IN ('imported', 'imported_seeding', 'failed', 'ignored')
             ORDER BY d.created_at, d.id LIMIT {}",
            &[SqlArg::I64(limit.min(100) as i64)],
        ).await?;
        for row in rows {
            let id = row.text("id")?;
            let state = row.text("tracked_state")?;
            SqlRuntime::run_in_transaction(&self.datastore, "seed_download_cleanup", move |tx| {
                let id = id.clone();
                let state = state.clone();
                Box::pin(async move { enqueue_cleanup_tx(tx, &id, &state).await })
            })
            .await?;
        }
        Ok(())
    }

    async fn due_cleanup(&self, limit: usize) -> AppResult<Vec<DownloadCleanupRecord>> {
        let now = Utc::now();
        let sql = format!(
            "SELECT * FROM (SELECT {CLEANUP_COLUMNS}, c.next_attempt_at,
             ROW_NUMBER() OVER (PARTITION BY c.client_id ORDER BY c.next_attempt_at, c.download_id) AS client_rank
             FROM download_cleanup c
             WHERE c.status = 'pending' AND c.next_attempt_at <= {{}}
               AND (c.lease_until IS NULL OR c.lease_until <= {{}})) due
             ORDER BY client_rank, next_attempt_at, download_id LIMIT {{}}"
        );
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
                SqlArg::I64(limit.min(100) as i64),
            ],
        )
        .await?
        .iter()
        .map(cleanup_record)
        .collect()
    }

    async fn claim_cleanup(&self, id: &DownloadId) -> AppResult<DownloadCleanupClaim> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "claim_download_cleanup", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let now = Utc::now();
                let changed = SqlRuntime::execute(SqlExec::Tx(tx),
                    "UPDATE download_cleanup SET attempts = attempts + 1, lease_until = {}, updated_at = {}
                     WHERE download_id = {} AND status = 'pending' AND next_attempt_at <= {}
                       AND (lease_until IS NULL OR lease_until <= {})",
                    &[SqlArg::Timestamp(now + chrono::Duration::minutes(5)), SqlArg::Timestamp(now),
                      SqlArg::Text(id.clone()), SqlArg::Timestamp(now), SqlArg::Timestamp(now)],
                ).await?;
                if changed == 0 {
                    let settled = SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                        "SELECT outcome FROM download_cleanup WHERE download_id = {} AND status = 'completed'",
                        &[SqlArg::Text(id.clone())],
                    ).await?;
                    return match settled {
                        Some(row) => Ok(DownloadCleanupClaim::Settled { outcome: row.text("outcome")? }),
                        None => Ok(DownloadCleanupClaim::Deferred),
                    };
                }
                let row = SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    &format!("SELECT {CLEANUP_COLUMNS} FROM download_cleanup c WHERE c.download_id = {{}}"),
                    &[SqlArg::Text(id)],
                ).await?.ok_or_else(|| AppError::Repository("claimed cleanup disappeared".into()))?;
                Ok(DownloadCleanupClaim::Claimed(Box::new(cleanup_record(&row)?)))
            })
        }).await
    }

    async fn checkpoint_cleanup(&self, id: &DownloadId, checkpoint: &str) -> AppResult<()> {
        SqlRuntime::execute_write(&self.datastore, "checkpoint_download_cleanup",
            "UPDATE download_cleanup SET payload_checkpoint = {}, updated_at = {} WHERE download_id = {}",
            vec![SqlArg::Text(checkpoint.to_string()), SqlArg::Timestamp(Utc::now()), SqlArg::Text(id.to_string())],
        ).await?;
        Ok(())
    }

    async fn finish_cleanup(
        &self,
        id: &DownloadId,
        outcome: &str,
        complete: bool,
        retry_seconds: i64,
        history_offset: usize,
        error: Option<&str>,
    ) -> AppResult<()> {
        let now = Utc::now();
        SqlRuntime::execute_write(
            &self.datastore,
            "finish_download_cleanup",
            "UPDATE download_cleanup SET status = {}, outcome = {}, last_error = {},
             history_offset = {}, next_attempt_at = {}, lease_until = NULL, updated_at = {}
             WHERE download_id = {}",
            vec![
                SqlArg::Text(if complete { "completed" } else { "pending" }.into()),
                SqlArg::Text(outcome.into()),
                SqlArg::OptText(error.map(|e| e.chars().take(2048).collect())),
                SqlArg::I64(history_offset as i64),
                SqlArg::Timestamp(now + chrono::Duration::seconds(retry_seconds.max(0))),
                SqlArg::Timestamp(now),
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }
}
