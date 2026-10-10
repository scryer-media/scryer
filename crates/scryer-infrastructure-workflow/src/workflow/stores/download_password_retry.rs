use scryer_application::{
    DOWNLOAD_PASSWORD_AMBIGUOUS_REASON, DOWNLOAD_PASSWORD_REQUIRED_REASON,
    DOWNLOAD_PASSWORD_RETRY_REASON, DownloadClientRetryOutcome, DownloadPasswordRetryClaim,
    DownloadPasswordRetryClaimOutcome, DownloadPasswordRetryObservation,
};

/// A retry whose dispatch outcome was never classified stops blocking a new
/// attempt after this long. Matches the stale-import recovery window.
const PASSWORD_RETRY_UNCERTAIN_TIMEOUT: chrono::Duration = chrono::Duration::minutes(45);

/// Marks a retry whose request to the client ended without a known outcome.
const PASSWORD_RETRY_DISPATCH_FINISHED_KEY: &str = "dispatch_finished_at";

fn parse_retry_timestamp(value: &serde_json::Value) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

/// The reason the download held before an unconfirmed retry, when that retry's
/// dispatch outcome is unknown and old enough to replace. A claim whose
/// dispatch finished uncertain is replaceable `timeout` after it finished. A
/// claim with no such marker may still be dispatching, so it is replaceable
/// only once its dispatch can no longer be running: `timeout` beyond the
/// dispatch bound after the claim (a process that stopped mid-dispatch never
/// writes the marker). Acknowledged or confirmed retries never time out here.
/// Replacement happens in the claiming transaction, so the download never
/// leaves its retry fence and nothing is deleted or released.
async fn expired_uncertain_password_retry(
    tx: &mut SqlTx<'_>,
    id: &str,
    now: chrono::DateTime<Utc>,
    timeout: chrono::Duration,
) -> AppResult<Option<String>> {
    let Some(row) = SqlRuntime::fetch_optional(SqlExec::Tx(tx),
        "SELECT s.password_retry_state, st.updated_at FROM download_submissions s
         JOIN download_identity_states st ON st.identity_key = {} AND st.canonical_download_id = s.id
         WHERE s.id = {} AND s.password_retry_state IS NOT NULL AND st.tracked_state = 'failed' AND st.reason = {}",
        &[SqlArg::Text(format!("download:{id}")), SqlArg::Text(id.to_owned()),
          SqlArg::Text(DOWNLOAD_PASSWORD_RETRY_REASON.into())]).await? else {
        return Ok(None);
    };
    let Ok(detail) = serde_json::from_str::<serde_json::Value>(&row.text("password_retry_state")?)
    else {
        return Ok(None);
    };
    if detail["confirmed"].as_bool() == Some(true) || detail.get("accepted_item_id").is_some() {
        return Ok(None);
    }
    let replaceable_at = if let Some(finished) = detail.get(PASSWORD_RETRY_DISPATCH_FINISHED_KEY) {
        let Some(finished_at) = parse_retry_timestamp(finished) else {
            return Ok(None);
        };
        finished_at + timeout
    } else {
        let claimed_at = match detail.get("claimed_at") {
            Some(value) => match parse_retry_timestamp(value) {
                Some(value) => value,
                None => return Ok(None),
            },
            None => row.timestamp("updated_at")?,
        };
        let Ok(dispatch_bound) = chrono::Duration::from_std(
            scryer_application::DOWNLOAD_PASSWORD_RETRY_DISPATCH_TIMEOUT,
        ) else {
            return Ok(None);
        };
        claimed_at + dispatch_bound + timeout
    };
    if now < replaceable_at {
        return Ok(None);
    }
    Ok(Some(
        detail["previous_reason"]
            .as_str()
            .filter(|reason| *reason != DOWNLOAD_PASSWORD_RETRY_REASON)
            .unwrap_or(DOWNLOAD_PASSWORD_REQUIRED_REASON)
            .to_owned(),
    ))
}

impl DownloadSubmissionStore {
    async fn read_password_retry_observation(
        &self,
        id: &DownloadId,
    ) -> AppResult<Option<DownloadPasswordRetryObservation>> {
        let row = SqlRuntime::fetch_optional(self.datastore.read_exec(),
            "SELECT s.password_retry_state, b.client_config_id, b.client_type_snapshot, b.native_item_id
             FROM download_submissions s JOIN download_client_bindings b ON b.download_id = s.id
             JOIN download_identity_states st ON st.canonical_download_id = s.id AND st.identity_key = {}
             WHERE s.id = {} AND s.password_retry_state IS NOT NULL AND b.ended_at IS NULL
             AND st.tracked_state NOT IN ('imported', 'imported_seeding', 'ignored')",
            &[SqlArg::Text(format!("download:{id}")), SqlArg::Text(id.to_string())]).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let detail: serde_json::Value = serde_json::from_str(&row.text("password_retry_state")?)
            .map_err(|_| AppError::Repository("invalid download retry state".into()))?;
        let claim: DownloadPasswordRetryClaim = serde_json::from_value(detail["claim"].clone())
            .map_err(|_| AppError::Repository("invalid download retry ownership".into()))?;
        Ok(Some(DownloadPasswordRetryObservation {
            claim,
            source: ClientJobLocator::new(
                row.opt_text("client_config_id")?.as_deref(),
                &row.text("client_type_snapshot")?,
                &row.text("native_item_id")?,
            ),
            confirmed: detail["confirmed"].as_bool().unwrap_or(false),
        }))
    }

    async fn apply_password_retry_observation(
        &self,
        observation: &DownloadPasswordRetryObservation,
        state: scryer_domain::DownloadQueueState,
    ) -> AppResult<bool> {
        use scryer_domain::DownloadQueueState;
        let observation = observation.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "confirm_download_password_retry", move |tx| {
            let observation = observation.clone();
            let state = state.clone();
            Box::pin(async move {
                let id = observation.claim.download_id.to_string();
                super::import_store::lock_retry_download(tx, &id).await?;
                let row = SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT s.password_retry_state FROM download_submissions s JOIN download_client_bindings b ON b.download_id = s.id
                     JOIN download_identity_states st ON st.canonical_download_id = s.id AND st.identity_key = {}
                     WHERE s.id = {} AND s.password_retry_state IS NOT NULL AND b.client_config_id = {} AND b.client_type_snapshot = {}
                     AND b.native_item_id = {} AND b.ended_at IS NULL AND st.tracked_state NOT IN ('imported', 'imported_seeding', 'ignored')",
                    &[SqlArg::Text(format!("download:{id}")), SqlArg::Text(id.clone()),
                      SqlArg::OptText(observation.source.client_id.clone()), SqlArg::Text(observation.source.client_type.clone()),
                      SqlArg::Text(observation.source.item_id.clone())]).await?;
                let Some(row) = row else { return Ok(false); };
                let mut detail: serde_json::Value = serde_json::from_str(&row.text("password_retry_state")?)
                    .map_err(|_| AppError::Repository("invalid download retry state".into()))?;
                let stored: DownloadPasswordRetryClaim = serde_json::from_value(detail["claim"].clone())
                    .map_err(|_| AppError::Repository("invalid download retry ownership".into()))?;
                if stored.attempt_id != observation.claim.attempt_id || stored.download_id != observation.claim.download_id
                    || stored.source != observation.claim.source || stored.authorized_title_id != observation.claim.authorized_title_id
                    || detail["confirmed"].as_bool().unwrap_or(false) != observation.confirmed {
                    return Ok(false);
                }
                // A new acknowledged native ID proves this is the retry even
                // if it failed before a poll saw progress. Same-ID failures
                // need positive attempt evidence; old history is insufficient.
                let acknowledged_replacement = observation.source.item_id != stored.source.item_id
                    && detail["accepted_item_id"].as_str() == Some(observation.source.item_id.as_str());
                if matches!(state, DownloadQueueState::Failed | DownloadQueueState::Warning)
                    && !observation.confirmed && !acknowledged_replacement {
                    return Ok(false);
                }
                let terminal = state == DownloadQueueState::Failed;
                // The independent marker survives import-blocked and other
                // workflow updates. Never reset an already confirmed workflow.
                if observation.confirmed && !terminal { return Ok(true); }
                detail["confirmed"] = serde_json::Value::Bool(true);
                SqlRuntime::execute(SqlExec::Tx(tx),
                    "UPDATE download_identity_states SET tracked_state = 'downloading', reason = NULL, detail = NULL, updated_at = {} WHERE identity_key = {}",
                    &[SqlArg::Timestamp(Utc::now()),
                      SqlArg::Text(format!("download:{id}"))]).await?;
                SqlRuntime::execute(SqlExec::Tx(tx), "UPDATE download_submissions SET tracked_state = 'downloading', password_retry_state = {} WHERE id = {}", &[SqlArg::OptText((!terminal).then(|| detail.to_string())), SqlArg::Text(id.clone())]).await?;
                SqlRuntime::execute(SqlExec::Tx(tx), "UPDATE download_cleanup SET tracked_state = 'downloading', updated_at = {} WHERE download_id = {}", &[SqlArg::Timestamp(Utc::now()), SqlArg::Text(id.clone())]).await?;
                SqlRuntime::execute(SqlExec::Tx(tx), "UPDATE downloads SET terminal_at = NULL WHERE id = {}", &[SqlArg::Text(id)]).await?;
                Ok(true)
            })
        }).await
    }

    async fn claim_remote_password_retry(
        &self,
        claim: &DownloadPasswordRetryClaim,
        password: &str,
    ) -> AppResult<DownloadPasswordRetryClaimOutcome> {
        if password.is_empty() {
            return Err(AppError::ArchivePasswordRequired {
                message: "Enter a new archive password to retry this job.".into(),
            });
        }
        let claim = claim.clone();
        let password = password.to_owned();
        let key = crate::config_store::current_encryption_key(&self.encryption_key)?;
        let uncertain_timeout = self.password_retry_uncertain_timeout;
        let clock = self.password_retry_clock.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "claim_download_password_retry", move |tx| {
            let claim = claim.clone();
            let password = password.clone();
            let key = key.clone();
            let clock = clock.clone();
            Box::pin(async move {
                let id = claim.download_id.to_string();
                super::import_store::lock_retry_download(tx, &id).await?;
                let now = clock();
                // An unconfirmed retry whose dispatch outcome is unknown may be
                // replaced once it is old enough; it stays fenced until then.
                let replaced_reason = expired_uncertain_password_retry(tx, &id, now, uncertain_timeout).await?;
                // A replaceable retry is excluded from the busy check only for
                // its own identity row; any other retry fence still applies.
                let retry_fence_sql = if replaced_reason.is_some() {
                    "SELECT id FROM download_identity_states WHERE canonical_download_id = {} AND (reason = {} OR (reason = {} AND identity_key <> {})) LIMIT 1"
                } else {
                    "SELECT id FROM download_identity_states WHERE canonical_download_id = {} AND reason IN ({}, {}) LIMIT 1"
                };
                let mut retry_fence_args = vec![
                    SqlArg::Text(id.clone()),
                    SqlArg::Text(scryer_application::IMPORT_RETRY_TRACKED_STATE_REASON.into()),
                    SqlArg::Text(DOWNLOAD_PASSWORD_RETRY_REASON.into()),
                ];
                if replaced_reason.is_some() {
                    retry_fence_args.push(SqlArg::Text(format!("download:{id}")));
                }
                // An expired lease still means deletion may have started. It is
                // not safe to retry a download until its owner has reconciled it.
                if SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT download_id FROM download_cleanup WHERE download_id = {} AND
                     (attempts > 0 OR lease_until IS NOT NULL OR status = 'completed' OR payload_checkpoint IS NOT NULL) LIMIT 1",
                    &[SqlArg::Text(id.clone())]).await?.is_some()
                    || SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT id FROM imports WHERE canonical_download_id = {} AND status IN ('pending', 'processing', 'running', 'queued') LIMIT 1",
                    &[SqlArg::Text(id.clone())]).await?.is_some()
                    || SqlRuntime::fetch_optional(SqlExec::Tx(tx), retry_fence_sql, &retry_fence_args).await?.is_some() {
                    return Ok(DownloadPasswordRetryClaimOutcome::Busy);
                }
                let replaceable_reason = if replaced_reason.is_some() { DOWNLOAD_PASSWORD_RETRY_REASON } else { DOWNLOAD_PASSWORD_REQUIRED_REASON };
                let row = SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT s.password_candidates, st.reason FROM download_submissions s
                     JOIN download_client_bindings b ON b.download_id = s.id
                     JOIN download_identity_states st ON st.identity_key = {} AND st.canonical_download_id = s.id
                     WHERE s.id = {} AND b.client_config_id = {} AND b.client_type_snapshot = {}
                       AND b.native_item_id = {} AND b.ended_at IS NULL AND st.tracked_state = 'failed'
                       AND st.reason IN ({}, {}, {}) AND s.title_id = {}",
                    &[SqlArg::Text(format!("download:{id}")), SqlArg::Text(id.clone()),
                      SqlArg::OptText(claim.source.client_id.clone()), SqlArg::Text(claim.source.client_type.clone()),
                      SqlArg::Text(claim.source.item_id.clone()), SqlArg::Text(DOWNLOAD_PASSWORD_REQUIRED_REASON.into()),
                      SqlArg::Text(DOWNLOAD_PASSWORD_AMBIGUOUS_REASON.into()), SqlArg::Text(replaceable_reason.into()),
                      SqlArg::Text(claim.authorized_title_id.clone())]).await?;
                let Some(row) = row else { return Ok(DownloadPasswordRetryClaimOutcome::Busy); };
                let previous_reason = match replaced_reason {
                    Some(reason) => reason,
                    None => row.text("reason")?,
                };
                let mut candidates = scryer_application::DownloadPasswordCandidates::default();
                candidates.push(&password);
                let old = crate::config_store::decrypt_optional_value(key.as_ref(), row.opt_text("password_candidates")?, "download passwords", true)?;
                if let Some(old) = old {
                    let old: scryer_application::DownloadPasswordCandidates = serde_json::from_str(&old)
                        .map_err(|_| AppError::Repository("could not decode download passwords".into()))?;
                    candidates.extend(old.iter().map(String::as_str));
                }
                let json = serde_json::to_string(&candidates)
                    .map_err(|_| AppError::Repository("could not encode download passwords".into()))?;
                let encrypted = crate::config_store::encrypt_optional_value(key.as_ref(), Some(&json), "download passwords", true)?;
                let detail = serde_json::json!({
                    "claim": claim,
                    "previous_reason": previous_reason,
                    "claimed_at": now.to_rfc3339(),
                })
                .to_string();
                SqlRuntime::execute(SqlExec::Tx(tx),
                    "UPDATE download_submissions SET password_candidates = {}, password_retry_state = {} WHERE id = {}",
                    &[SqlArg::OptText(encrypted), SqlArg::Text(detail.clone()), SqlArg::Text(id.clone())]).await?;
                SqlRuntime::execute(SqlExec::Tx(tx),
                    "UPDATE download_identity_states SET reason = {}, detail = {}, updated_at = {} WHERE identity_key = {}",
                    &[SqlArg::Text(DOWNLOAD_PASSWORD_RETRY_REASON.into()), SqlArg::Text(detail),
                      SqlArg::Timestamp(now), SqlArg::Text(format!("download:{id}"))]).await?;
                Ok(DownloadPasswordRetryClaimOutcome::Claimed)
            })
        }).await
    }

    async fn finish_remote_password_retry(
        &self,
        claim: &DownloadPasswordRetryClaim,
        outcome: &DownloadClientRetryOutcome,
    ) -> AppResult<()> {
        let claim = claim.clone();
        let outcome = outcome.clone();
        let clock = self.password_retry_clock.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "finish_download_password_retry", move |tx| {
            let claim = claim.clone();
            let outcome = outcome.clone();
            let clock = clock.clone();
            Box::pin(async move {
                let id = claim.download_id.to_string();
                super::import_store::lock_retry_download(tx, &id).await?;
                let row = SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                    "SELECT s.password_retry_state, b.native_item_id FROM download_submissions s
                     JOIN download_client_bindings b ON b.download_id = s.id AND b.ended_at IS NULL
                     WHERE s.id = {} AND s.password_retry_state IS NOT NULL AND b.client_config_id = {} AND b.client_type_snapshot = {}",
                    &[SqlArg::Text(id.clone()), SqlArg::OptText(claim.source.client_id.clone()), SqlArg::Text(claim.source.client_type.clone())]).await?
                    .ok_or_else(|| AppError::Validation("download retry no longer owns this job".into()))?;
                let mut detail: serde_json::Value = serde_json::from_str(&row.text("password_retry_state")?)
                    .map_err(|_| AppError::Repository("invalid download retry state".into()))?;
                let stored: DownloadPasswordRetryClaim = serde_json::from_value(detail["claim"].clone())
                    .map_err(|_| AppError::Repository("invalid download retry ownership".into()))?;
                if stored.attempt_id != claim.attempt_id || stored.download_id != claim.download_id
                    || stored.source != claim.source || stored.authorized_title_id != claim.authorized_title_id {
                    return Err(AppError::Validation("download retry no longer owns this job".into()));
                }
                if detail["confirmed"].as_bool() == Some(true) {
                    if let DownloadClientRetryOutcome::Accepted { item_id } = &outcome {
                        if row.text("native_item_id")? != *item_id {
                            return Err(AppError::Validation("retry response conflicts with observed progress; reconciliation required".into()));
                        }
                        detail["accepted_item_id"] = serde_json::Value::String(item_id.clone());
                        SqlRuntime::execute(SqlExec::Tx(tx), "UPDATE download_submissions SET password_retry_state = {} WHERE id = {}",
                            &[SqlArg::Text(detail.to_string()), SqlArg::Text(id)]).await?;
                    }
                    // Fresh progress outranks a late acknowledgement/refusal.
                    return Ok(());
                }
                if matches!(outcome, DownloadClientRetryOutcome::Uncertain) {
                    // The request has ended; only now may its claim time out.
                    if detail.get(PASSWORD_RETRY_DISPATCH_FINISHED_KEY).is_none() {
                        detail[PASSWORD_RETRY_DISPATCH_FINISHED_KEY] =
                            serde_json::Value::String(clock().to_rfc3339());
                        let detail = detail.to_string();
                        SqlRuntime::execute(SqlExec::Tx(tx),
                            "UPDATE download_submissions SET password_retry_state = {} WHERE id = {}",
                            &[SqlArg::Text(detail.clone()), SqlArg::Text(id.clone())]).await?;
                        SqlRuntime::execute(SqlExec::Tx(tx),
                            "UPDATE download_identity_states SET detail = {} WHERE identity_key = {} AND reason = {}",
                            &[SqlArg::Text(detail), SqlArg::Text(format!("download:{id}")),
                              SqlArg::Text(DOWNLOAD_PASSWORD_RETRY_REASON.into())]).await?;
                    }
                    return Ok(());
                }
                let (state, reason, item_id) = match outcome {
                    DownloadClientRetryOutcome::Accepted { item_id } if !item_id.is_empty() => {
                        if SqlRuntime::fetch_optional(SqlExec::Tx(tx),
                            "SELECT download_id FROM download_client_bindings WHERE client_config_id = {} AND native_item_id = {} AND download_id <> {} AND ended_at IS NULL LIMIT 1",
                            &[SqlArg::OptText(claim.source.client_id.clone()), SqlArg::Text(item_id.clone()), SqlArg::Text(id.clone())]).await?.is_some() {
                            return Err(AppError::Validation("retry response belongs to another download; reconciliation required".into()));
                        }
                        let changed = SqlRuntime::execute(SqlExec::Tx(tx),
                            "UPDATE download_client_bindings SET native_item_id = {} WHERE download_id = {} AND client_config_id = {} AND client_type_snapshot = {} AND native_item_id = {} AND ended_at IS NULL",
                            &[SqlArg::Text(item_id.clone()), SqlArg::Text(id.clone()), SqlArg::OptText(claim.source.client_id.clone()),
                              SqlArg::Text(claim.source.client_type.clone()), SqlArg::Text(claim.source.item_id.clone())]).await?;
                        if changed != 1 { return Err(AppError::Validation("download binding changed during retry; reconciliation required".into())); }
                        SqlRuntime::execute(SqlExec::Tx(tx), "UPDATE downloads SET terminal_at = NULL WHERE id = {}",
                            &[SqlArg::Text(id.clone())]).await?;
                        // Acceptance alone is not proof that a cached failed
                        // observation belongs to the new attempt. Keep ownership
                        // until a fresh exact-client read observes progress.
                        detail["accepted_item_id"] = serde_json::Value::String(item_id.clone());
                        ("failed", Some(DOWNLOAD_PASSWORD_RETRY_REASON.to_owned()), item_id)
                    }
                    DownloadClientRetryOutcome::Refused => ("failed", Some(detail["previous_reason"].as_str().unwrap_or(DOWNLOAD_PASSWORD_REQUIRED_REASON).to_owned()), claim.source.item_id.clone()),
                    _ => return Ok(()),
                };
                SqlRuntime::execute(SqlExec::Tx(tx),
                    "UPDATE download_identity_states SET tracked_state = {}, reason = {}, detail = {}, download_client_item_id = {}, updated_at = {} WHERE identity_key = {}",
                    &[SqlArg::Text(state.into()), SqlArg::OptText(reason.clone()),
                      SqlArg::OptText((reason.as_deref() == Some(DOWNLOAD_PASSWORD_RETRY_REASON)).then(|| detail.to_string())), SqlArg::Text(item_id.clone()),
                      SqlArg::Timestamp(Utc::now()), SqlArg::Text(format!("download:{id}"))]).await?;
                SqlRuntime::execute(SqlExec::Tx(tx),
                    "UPDATE download_submissions SET tracked_state = {}, download_client_item_id = {}, password_retry_state = {} WHERE id = {}",
                    &[SqlArg::Text(state.into()), SqlArg::Text(item_id.clone()),
                      SqlArg::OptText((reason.as_deref() == Some(DOWNLOAD_PASSWORD_RETRY_REASON)).then(|| detail.to_string())), SqlArg::Text(id.clone())]).await?;
                SqlRuntime::execute(SqlExec::Tx(tx),
                    "UPDATE download_cleanup SET tracked_state = {}, item_id = {}, updated_at = {} WHERE download_id = {}",
                    &[SqlArg::Text(state.into()), SqlArg::Text(item_id), SqlArg::Timestamp(Utc::now()), SqlArg::Text(id)]).await?;
                Ok(())
            })
        }).await
    }
}
