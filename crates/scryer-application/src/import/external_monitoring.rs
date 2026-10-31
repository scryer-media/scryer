use super::*;

const MAX_IMPORT_SETUP_SECRET_INSTANCES: usize = 25;
const MAX_IMPORT_SETUP_SECRET_OVERRIDES: usize = 500;
const MAX_IMPORT_SETUP_SECRET_ID_LEN: usize = 128;
const MAX_IMPORT_SETUP_SECRET_DEDUP_KEY_LEN: usize = 1024;
const MAX_IMPORT_SETUP_SECRET_VALUE_LEN: usize = 8192;

impl AppUseCase {
    pub async fn get_external_import_setup_secret_draft(
        &self,
        actor: &User,
    ) -> AppResult<Option<ExternalImportSetupSecretDraft>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.services
            .workflow
            .external_import_setup_secret_drafts
            .get_for_owner(&actor.id)
            .await
    }

    pub async fn external_import_setup_secret_draft_status(
        &self,
        actor: &User,
    ) -> AppResult<ExternalImportSetupSecretDraftStatus> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.services
            .workflow
            .external_import_setup_secret_drafts
            .status_for_actor(&actor.id)
            .await
    }

    pub async fn save_external_import_setup_secret_draft(
        &self,
        actor: &User,
        draft: ExternalImportSetupSecretDraftInput,
    ) -> AppResult<ExternalImportSetupSecretDraftSaveResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let draft = validate_external_import_setup_secret_draft(draft)?;
        self.services
            .workflow
            .external_import_setup_secret_drafts
            .save_for_owner(&actor.id, draft)
            .await
    }

    pub async fn clear_external_import_setup_secret_draft(&self, actor: &User) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.services
            .workflow
            .external_import_setup_secret_drafts
            .clear_for_owner(&actor.id)
            .await
    }

    pub async fn append_external_import_monitor_snapshot_chunk(
        &self,
        actor: &User,
        chunk: ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .workflow
            .external_import_monitor_snapshots
            .append_external_import_monitor_snapshot_chunk(&chunk)
            .await
    }

    pub async fn clear_external_import_monitor_snapshot_chunks(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .workflow
            .external_import_monitor_snapshots
            .delete_external_import_monitor_snapshot_chunks_for_session_prefix(
                crate::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID,
                facet,
            )
            .await
    }

    pub async fn clear_external_import_monitor_snapshot_chunks_for_session(
        &self,
        actor: &User,
        session_id: &str,
        facet: MediaFacet,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .workflow
            .external_import_monitor_snapshots
            .delete_external_import_monitor_snapshot_chunks(session_id, facet)
            .await
    }

    pub async fn list_external_import_monitor_snapshot_chunks_for_session(
        &self,
        actor: &User,
        session_id: &str,
        facet: MediaFacet,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .workflow
            .external_import_monitor_snapshots
            .list_external_import_monitor_snapshot_chunk_batch(
                session_id,
                facet,
                entry_kind,
                after_chunk_index,
                limit,
            )
            .await
    }

    pub async fn clear_external_import_arr_source_session_chunks(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<()> {
        if crate::is_external_import_monitor_apply_session_id(session_id) {
            return Ok(());
        }
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            self.clear_external_import_monitor_snapshot_chunks_for_session(
                actor, session_id, facet,
            )
            .await?;
        }
        Ok(())
    }

    pub async fn clear_stale_external_import_arr_source_chunks_once(
        &self,
        actor: &User,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let mut cleanup_done = self
            .runtime
            .imports
            .external_import_source_chunk_cleanup_done
            .lock()
            .await;
        if *cleanup_done {
            return Ok(());
        }

        self.services
            .workflow
            .external_import_monitor_snapshots
            .delete_external_import_monitor_snapshot_chunks_except_session_prefix(
                crate::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID,
            )
            .await?;
        *cleanup_done = true;
        Ok(())
    }

    pub async fn begin_external_import_monitor_warmup(
        &self,
        actor: &User,
        connection_fingerprint: &str,
    ) -> AppResult<ExternalImportMonitorWarmupBeginResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let initial_snapshot =
            ExternalImportMonitorWarmupProgressSnapshot::new(scryer_domain::Id::new().0);

        Ok(self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .begin(&actor.id, connection_fingerprint, initial_snapshot)
            .await)
    }

    pub async fn prune_terminal_external_import_warmup_sessions(
        &self,
        actor: &User,
        max_age: chrono::Duration,
    ) -> AppResult<Vec<String>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        Ok(self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .prune_terminal_older_than(max_age)
            .await)
    }

    pub async fn maintain_external_import_arr_source_sessions(
        &self,
        actor: &User,
    ) -> AppResult<()> {
        self.clear_stale_external_import_arr_source_chunks_once(actor)
            .await?;

        let terminal_ttl = chrono::Duration::hours(2);
        let removed_session_ids = self
            .prune_terminal_external_import_warmup_sessions(actor, terminal_ttl)
            .await?;
        for session_id in removed_session_ids {
            self.clear_external_import_arr_source_session_chunks(actor, &session_id)
                .await?;
        }

        Ok(())
    }

    pub async fn get_external_import_monitor_warmup_status(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<ExternalImportMonitorWarmupProgressSnapshot> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .snapshot(&actor.id, session_id)
            .await
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::external_import::SESSION_GONE,
                    format!("no warmup session '{session_id}'"),
                )
            })
    }

    pub async fn subscribe_external_import_monitor_warmup_progress(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<tokio::sync::watch::Receiver<ExternalImportMonitorWarmupProgressSnapshot>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .subscribe(&actor.id, session_id)
            .await
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::external_import::SESSION_GONE,
                    format!("no warmup session '{session_id}'"),
                )
            })
    }

    pub async fn cancel_external_import_monitor_warmup(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        Ok(self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .cancel(&actor.id, session_id)
            .await)
    }

    pub async fn remove_external_import_monitor_warmup_session(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        Ok(self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .remove(&actor.id, session_id)
            .await)
    }

    pub async fn claim_external_import_monitor_warmup(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<ExternalImportMonitorWarmupProgressSnapshot> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .claim(&actor.id, session_id)
            .await
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::external_import::SESSION_GONE,
                    format!("no warmup session '{session_id}'"),
                )
            })
    }

    pub async fn external_import_monitor_warmup_connection_fingerprint(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<String> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .connection_fingerprint(&actor.id, session_id)
            .await
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::external_import::SESSION_GONE,
                    format!("no warmup session '{session_id}'"),
                )
            })
    }

    pub async fn update_external_import_monitor_warmup_progress(
        &self,
        session_id: &str,
        mut snapshot: ExternalImportMonitorWarmupProgressSnapshot,
    ) {
        snapshot.touch();
        let _ = self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .update(session_id, snapshot)
            .await;
    }

    pub async fn set_external_import_monitor_warmup_scan_hints(
        &self,
        actor: &User,
        session_id: &str,
        scan_hints: LibraryScanHintSet,
    ) {
        let _ = self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .set_scan_hints(&actor.id, session_id, scan_hints)
            .await;
    }

    pub async fn set_external_import_arr_source_warmup_result(
        &self,
        session_id: &str,
        result: ExternalImportArrSourceWarmupResult,
    ) {
        let _ = self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .set_arr_source_result(session_id, result)
            .await;
    }

    pub async fn set_external_import_prowlarr_warmup_result(
        &self,
        session_id: &str,
        result: ExternalImportProwlarrWarmupResult,
    ) {
        let _ = self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .set_prowlarr_result(session_id, result)
            .await;
    }

    pub async fn external_import_arr_source_warmup_result(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<ExternalImportArrSourceWarmupResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .arr_source_result(&actor.id, session_id)
            .await
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::import::ARR_SOURCE_WARMUP_RESULT_NOT_FOUND,
                    format!("no arr source warmup result '{session_id}'"),
                )
            })
    }

    pub async fn external_import_prowlarr_warmup_result(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<ExternalImportProwlarrWarmupResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .prowlarr_result(&actor.id, session_id)
            .await
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::import::PROWLARR_WARMUP_RESULT_NOT_FOUND,
                    format!("no prowlarr warmup result '{session_id}'"),
                )
            })
    }

    pub async fn external_import_monitor_warmup_scan_hints(
        &self,
        actor: &User,
        session_id: &str,
    ) -> Option<LibraryScanHintSet> {
        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .scan_hints(&actor.id, session_id)
            .await
    }

    pub async fn acquire_external_import_apply_guard(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.runtime
            .imports
            .external_import_apply_lock
            .clone()
            .lock_owned()
            .await
    }
}

fn validate_external_import_setup_secret_draft(
    draft: ExternalImportSetupSecretDraftInput,
) -> AppResult<ExternalImportSetupSecretDraftInput> {
    if draft.is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::import::EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_CANNOT_EMPTY,
            "external import setup secret draft cannot be empty; clear the draft instead"
                .to_string(),
        ));
    }
    if draft.instance_api_keys.len() > MAX_IMPORT_SETUP_SECRET_INSTANCES {
        return Err(AppError::validation_refused(crate::error_reasons::import::EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST_INSTANCE_API_KEYS, format!(
            "external import setup secret draft supports at most {MAX_IMPORT_SETUP_SECRET_INSTANCES} instance API keys"
        )));
    }
    for (label, len) in [
        (
            "download client API key overrides",
            draft.download_client_api_key_overrides.len(),
        ),
        (
            "download client password overrides",
            draft.download_client_password_overrides.len(),
        ),
        (
            "indexer API key overrides",
            draft.indexer_api_key_overrides.len(),
        ),
    ] {
        if len > MAX_IMPORT_SETUP_SECRET_OVERRIDES {
            return Err(AppError::validation_refused(
                crate::error_reasons::import::EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST,
                format!(
                    "external import setup secret draft supports at most {MAX_IMPORT_SETUP_SECRET_OVERRIDES} {label}"
                ),
            ));
        }
    }

    let mut instance_ids = HashSet::new();
    let mut instance_api_keys = Vec::with_capacity(draft.instance_api_keys.len());
    for entry in draft.instance_api_keys {
        let instance_id = required_trimmed(
            entry.instance_id,
            "instanceId",
            MAX_IMPORT_SETUP_SECRET_ID_LEN,
        )?;
        let api_key = required_trimmed(entry.api_key, "apiKey", MAX_IMPORT_SETUP_SECRET_VALUE_LEN)?;
        if !instance_ids.insert(instance_id.clone()) {
            return Err(AppError::validation_refused(
                crate::error_reasons::import::EXTERNAL_IMPORT_SETUP_INSTANCEID_DUPLICATED,
                format!("duplicate external import setup instanceId: {instance_id}"),
            ));
        }
        instance_api_keys.push(ExternalImportSetupInstanceApiKeyDraft {
            instance_id,
            kind: entry.kind,
            api_key,
        });
    }

    Ok(ExternalImportSetupSecretDraftInput {
        instance_api_keys,
        download_client_api_key_overrides: validate_secret_overrides(
            "downloadClientApiKeyOverrides",
            draft.download_client_api_key_overrides,
        )?,
        download_client_password_overrides: validate_secret_overrides(
            "downloadClientPasswordOverrides",
            draft.download_client_password_overrides,
        )?,
        indexer_api_key_overrides: validate_secret_overrides(
            "indexerApiKeyOverrides",
            draft.indexer_api_key_overrides,
        )?,
    })
}

fn validate_secret_overrides(
    label: &str,
    entries: Vec<ExternalImportSetupSecretOverrideDraft>,
) -> AppResult<Vec<ExternalImportSetupSecretOverrideDraft>> {
    let mut keys = HashSet::new();
    let mut normalized = Vec::with_capacity(entries.len());
    for entry in entries {
        let dedup_key = required_trimmed(
            entry.dedup_key,
            "dedupKey",
            MAX_IMPORT_SETUP_SECRET_DEDUP_KEY_LEN,
        )?;
        let secret = required_trimmed(entry.secret, label, MAX_IMPORT_SETUP_SECRET_VALUE_LEN)?;
        if !keys.insert(dedup_key.clone()) {
            return Err(AppError::validation_refused(
                crate::error_reasons::import::EXTERNAL_IMPORT_SETUP_DEDUPKEY_DUPLICATED,
                format!("duplicate external import setup dedupKey in {label}: {dedup_key}"),
            ));
        }
        normalized.push(ExternalImportSetupSecretOverrideDraft { dedup_key, secret });
    }
    Ok(normalized)
}

fn required_trimmed(value: String, label: &str, max_len: usize) -> AppResult<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::import::EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_REQUIRED,
            format!("external import setup secret draft {label} cannot be empty"),
        ));
    }
    if value.len() > max_len {
        return Err(AppError::validation_refused(
            crate::error_reasons::import::EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_OUT_OF_RANGE,
            format!("external import setup secret draft {label} must be at most {max_len} bytes"),
        ));
    }
    Ok(value)
}
