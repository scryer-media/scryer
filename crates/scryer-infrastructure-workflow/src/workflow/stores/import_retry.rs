use crate::queries::sql_runtime::SqlTx;
use scryer_application::{IMPORT_RETRY_TRACKED_STATE_REASON, ImportRetryClaim};

/// A write lock works on both SQLite and PostgreSQL and leaves identity unchanged.
pub(super) async fn lock_retry_download(tx: &mut SqlTx<'_>, id: &str) -> AppResult<()> {
    let rows = SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE downloads SET id = id WHERE id = {}",
        &[SqlArg::Text(id.into())],
    )
    .await?;
    if rows != 1 {
        return Err(AppError::NotFound(
            "canonical download for import retry".into(),
        ));
    }
    Ok(())
}

/// Ordinary tracker writes cannot erase a retry's recovery ownership.
pub(super) async fn guard_import_retry_tx(tx: &mut SqlTx<'_>, id: &str) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE downloads SET id = id WHERE id = {}",
        &[SqlArg::Text(id.into())],
    )
    .await?;
    if SqlRuntime::fetch_optional(SqlExec::Tx(tx),
        "SELECT id FROM download_identity_states WHERE canonical_download_id = {} AND reason IN ({}, {}) LIMIT 1",
        &[SqlArg::Text(id.into()), SqlArg::Text(IMPORT_RETRY_TRACKED_STATE_REASON.into()),
          SqlArg::Text(scryer_application::DOWNLOAD_PASSWORD_RETRY_REASON.into())]).await?.is_some() {
        return Err(AppError::Validation("download is awaiting retry reconciliation".into()));
    }
    Ok(())
}

async fn write_retry_state(
    tx: &mut SqlTx<'_>,
    claim: &ImportRetryClaim,
    state: &str,
    reason: Option<String>,
    detail: Option<String>,
) -> AppResult<()> {
    let now = Utc::now();
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO download_identity_states
         (id, identity_key, canonical_download_id, client_id, client_type, download_client_item_id,
          tracked_state, reason, detail, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(identity_key) DO UPDATE SET tracked_state = excluded.tracked_state,
         reason = excluded.reason, detail = excluded.detail, updated_at = excluded.updated_at",
        &[
            SqlArg::Text(scryer_domain::Id::new().0),
            SqlArg::Text(format!("download:{}", claim.download_id)),
            SqlArg::Text(claim.download_id.to_string()),
            SqlArg::OptText(claim.source.client_id.clone()),
            SqlArg::Text(claim.source.client_type.clone()),
            SqlArg::Text(claim.source.item_id.clone()),
            SqlArg::Text(state.into()),
            SqlArg::OptText(reason),
            SqlArg::OptText(detail),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE download_submissions SET tracked_state = {} WHERE id = {}",
        &[
            SqlArg::Text(state.into()),
            SqlArg::Text(claim.download_id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

impl ImportStore {
    async fn claim_retry(
        &self,
        claim: &ImportRetryClaim,
        expected_updated_at: chrono::DateTime<Utc>,
        payload_json: &str,
    ) -> AppResult<scryer_application::ImportRetryClaimOutcome> {
        let claim = claim.clone();
        let payload_json = payload_json.to_owned();
        SqlRuntime::run_in_transaction(&self.datastore, "claim_import_retry", move |tx| {
            let claim = claim.clone();
            let payload_json = payload_json.clone();
            Box::pin(async move {
                let id = claim.download_id.to_string();
                lock_retry_download(tx, &id).await?;
                if SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT download_id FROM download_cleanup WHERE download_id = {} AND lease_until IS NOT NULL LIMIT 1",
                    &[SqlArg::Text(id.clone())]).await?.is_some() {
                    return Ok(scryer_application::ImportRetryClaimOutcome::Busy);
                }
                if SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT id FROM download_identity_states WHERE canonical_download_id = {} AND reason IN ({}, {}) LIMIT 1",
                    &[SqlArg::Text(id.clone()), SqlArg::Text(IMPORT_RETRY_TRACKED_STATE_REASON.into()),
                      SqlArg::Text(scryer_application::DOWNLOAD_PASSWORD_RETRY_REASON.into())]).await?.is_some() {
                    return Ok(scryer_application::ImportRetryClaimOutcome::Busy);
                }
                if SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT id FROM imports WHERE canonical_download_id = {} AND id <> {} AND status IN ('pending', 'processing', 'running', 'queued') LIMIT 1",
                    &[SqlArg::Text(id.clone()), SqlArg::Text(claim.import_id.clone())]).await?.is_some() {
                    return Ok(scryer_application::ImportRetryClaimOutcome::Busy);
                }
                if SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT download_id FROM download_client_bindings WHERE client_config_id = {} AND native_item_id = {} AND download_id <> {} AND ended_at IS NULL LIMIT 1",
                    &[SqlArg::OptText(claim.source.client_id.clone()), SqlArg::Text(claim.source.item_id.clone()), SqlArg::Text(id.clone())]).await?.is_some() {
                    return Ok(scryer_application::ImportRetryClaimOutcome::Busy);
                }
                let payload_arg = json_arg_for_tx(tx, Some(&payload_json))?;
                let changed = SqlRuntime::execute(SqlExec::Tx(tx),
                    "UPDATE imports SET status = 'processing', result_json = NULL, payload_json = {}, started_at = {}, finished_at = NULL, updated_at = {}
                     WHERE id = {} AND canonical_download_id = {} AND status IN ('failed', 'skipped') AND updated_at = {}",
                    &[payload_arg, SqlArg::Timestamp(claim.started_at), SqlArg::Timestamp(claim.started_at),
                      SqlArg::Text(claim.import_id.clone()), SqlArg::Text(id), SqlArg::Timestamp(expected_updated_at)]).await?;
                if changed != 1 { return Ok(scryer_application::ImportRetryClaimOutcome::Busy); }
                let detail = serde_json::to_string(&claim).map_err(|e| AppError::Repository(e.to_string()))?;
                write_retry_state(tx, &claim, "importing", Some(IMPORT_RETRY_TRACKED_STATE_REASON.into()), Some(detail)).await?;
                Ok(scryer_application::ImportRetryClaimOutcome::Claimed)
            })
        }).await
    }

    async fn finish_retry(
        &self,
        claim: &ImportRetryClaim,
        state: scryer_domain::TrackedDownloadState,
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> AppResult<scryer_application::ImportRetryFinishOutcome> {
        let claim = claim.clone();
        let reason = reason.map(str::to_owned);
        let detail = detail.map(str::to_owned);
        SqlRuntime::run_in_transaction(&self.datastore, "finish_import_retry", move |tx| {
            let claim = claim.clone();
            let reason = reason.clone();
            let detail = detail.clone();
            Box::pin(async move {
                let id = claim.download_id.to_string();
                lock_retry_download(tx, &id).await?;
                let row = SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT detail FROM download_identity_states WHERE identity_key = {} AND reason = {}",
                    &[SqlArg::Text(format!("download:{id}")), SqlArg::Text(IMPORT_RETRY_TRACKED_STATE_REASON.into())]).await?;
                let stored = row.and_then(|row| row.opt_text("detail").ok().flatten())
                    .and_then(|detail| serde_json::from_str::<ImportRetryClaim>(&detail).ok());
                if !stored.is_some_and(|stored| stored.attempt_id == claim.attempt_id && stored.import_id == claim.import_id) {
                    return Ok(scryer_application::ImportRetryFinishOutcome::Superseded);
                }
                let import = SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT result_json FROM imports WHERE id = {}",
                    &[SqlArg::Text(claim.import_id.clone())]).await?
                    .ok_or_else(|| AppError::Repository("retry import disappeared".into()))?;
                let mut result: serde_json::Value = json_text_from_row(&import, "result_json")?
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .ok_or_else(|| AppError::Repository("retry result is not recorded yet".into()))?;
                if let Some(previous) = claim.previous_result_json.as_deref() {
                    result["previous_attempt"] = serde_json::from_str(previous).unwrap_or_else(|_| serde_json::Value::String(previous.into()));
                }
                let status = if state.counts_as_imported() { "completed" }
                    else if state == scryer_domain::TrackedDownloadState::ImportBlocked { "skipped" }
                    else { "failed" };
                let result_arg = json_arg_for_tx(tx, Some(&result.to_string()))?;
                SqlRuntime::execute(SqlExec::Tx(tx),
                    "UPDATE imports SET status = {}, result_json = {}, finished_at = {}, updated_at = {} WHERE id = {}",
                    &[SqlArg::Text(status.into()), result_arg, SqlArg::Timestamp(Utc::now()),
                      SqlArg::Timestamp(Utc::now()), SqlArg::Text(claim.import_id.clone())]).await?;
                write_retry_state(tx, &claim, state.as_str(), reason, detail).await?;
                // Keep the cleanup record and checkpoint, but fence its previous outcome.
                SqlRuntime::execute(SqlExec::Tx(tx),
                    "UPDATE download_cleanup SET tracked_state = {}, updated_at = {} WHERE download_id = {} AND {} = 'import_blocked'",
                    &[SqlArg::Text(state.as_str().into()), SqlArg::Timestamp(Utc::now()), SqlArg::Text(id.clone()), SqlArg::Text(state.as_str().into())]).await?;
                super::download_submission_store::enqueue_cleanup_tx(tx, &id, state.as_str()).await?;
                Ok(scryer_application::ImportRetryFinishOutcome::Finalized)
            })
        }).await
    }

    async fn retry_recovery(
        &self,
        after: Option<&scryer_domain::download_identity::DownloadId>,
        limit: usize,
    ) -> AppResult<Vec<ImportRetryClaim>> {
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(),
            "SELECT detail FROM download_identity_states WHERE reason = {} AND canonical_download_id > {} ORDER BY canonical_download_id LIMIT {}",
            &[SqlArg::Text(IMPORT_RETRY_TRACKED_STATE_REASON.into()), SqlArg::Text(after.map(ToString::to_string).unwrap_or_default()),
              SqlArg::I64(limit.min(100) as i64)]).await?;
        rows.iter()
            .map(|row| {
                let detail = row.text("detail")?;
                serde_json::from_str(&detail).map_err(|e| {
                    AppError::Repository(format!("invalid import retry recovery marker: {e}"))
                })
            })
            .collect()
    }
}
