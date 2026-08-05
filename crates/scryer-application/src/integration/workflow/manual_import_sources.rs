fn apply_manual_import_record_to_queue_item(item: &mut DownloadQueueItem, record: &ImportRecord) {
    apply_import_record_overlay_to_queue_item(item, record);

    if let Some(result_json) = record.result_json.as_deref()
        && let Ok(result) = serde_json::from_str::<crate::ManualImportExecutionResult>(result_json)
    {
        item.import_error_code = result.error_code;
        item.import_error_message = result.error_message.clone();
        if let Some(message) = result.error_message {
            item.attention_reason = Some(message);
        }
    }
}
async fn enrich_queue_item_import_states(app: &AppUseCase, items: &mut [DownloadQueueItem]) {
    let import_sources = items
        .iter()
        .filter(|item| queue_item_import_state_eligible(item))
        .map(|item| {
            DownloadSourceIdentity::new(
                Some(item.client_id.as_str()).filter(|value| !value.trim().is_empty()),
                &item.client_type,
                &item.download_client_item_id,
            )
        })
        .collect::<Vec<_>>();

    let delete_sources = items
        .iter()
        .map(|item| {
            (
                Some(item.client_id.clone()).filter(|value| !value.trim().is_empty()),
                item.client_type.clone(),
                item.download_client_item_id.clone(),
                is_history_download_state(&item.state),
            )
        })
        .collect::<Vec<_>>();

    let records = if import_sources.is_empty() {
        Vec::new()
    } else {
        match app
            .services
            .workflow
            .imports
            .list_imports_for_identities(&import_sources)
            .await
        {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!(error = %error, "failed to batch-load import state for queue items");
                Vec::new()
            }
        }
    };
    let delete_commands = match app
        .services
        .workflow
        .download_queue_commands
        .list_latest_delete_commands_for_sources(&delete_sources)
        .await
    {
        Ok(commands) => commands,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to batch-load delete command state for queue items"
            );
            Vec::new()
        }
    };

    let mut manual_records = HashMap::new();
    let mut fallback_records = HashMap::new();
    let mut delete_records = HashMap::new();
    for record in records {
        let key = DownloadSourceIdentity::new(
            record.source_client_id.as_deref(),
            &record.source_system,
            &record.source_ref,
        );
        if record.import_type == ImportType::ManualImport {
            manual_records.entry(key).or_insert(record);
        } else {
            fallback_records.entry(key).or_insert(record);
        }
    }
    for command in delete_commands {
        let key = (
            command.client_id.clone().unwrap_or_default(),
            command.client_type.clone(),
            command.download_client_item_id.clone(),
            command.is_history,
        );
        delete_records.entry(key).or_insert(command);
    }

    for item in items.iter_mut() {
        let import_key = DownloadSourceIdentity::new(
            Some(item.client_id.as_str()).filter(|value| !value.trim().is_empty()),
            &item.client_type,
            &item.download_client_item_id,
        );
        let delete_key = (
            item.client_id.clone(),
            item.client_type.clone(),
            item.download_client_item_id.clone(),
            is_history_download_state(&item.state),
        );
        let legacy_delete_key = (
            String::new(),
            item.client_type.clone(),
            item.download_client_item_id.clone(),
            is_history_download_state(&item.state),
        );
        if queue_item_import_state_eligible(item) {
            if let Some(record) = manual_records.get(&import_key) {
                apply_manual_import_record_to_queue_item(item, record);
            } else if let Some(record) = fallback_records.get(&import_key) {
                apply_import_record_to_queue_item(item, record);
            }
        }
        if let Some(command) = delete_records
            .get(&delete_key)
            .or_else(|| delete_records.get(&legacy_delete_key))
        {
            apply_delete_command_to_queue_item(item, command);
        }
    }
}
impl AppUseCase {
    pub async fn queue_manual_import_selection(
        &self,
        actor: &User,
        selection_id: String,
        mappings: Vec<crate::ManualImportCandidateMapping>,
    ) -> AppResult<String> {
        let selection = self
            .services
            .workflow
            .imports
            .get_manual_import_selection(&selection_id, &actor.id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(
                    "manual import selection is unavailable; reopen the import dialog".to_string(),
                )
            })?;
        // Validate AFTER loading the selection: whether a mapping needs an
        // explicit target depends on the title's facet (a movie has no
        // sub-target to name), and the facet is only knowable once the
        // selection identifies the title.
        let selection_title = self
            .services
            .catalog
            .titles
            .get_by_id(&selection.title_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation("manual import selection title is unavailable".to_string())
            })?;
        crate::import_workflow::validate_manual_import_candidate_mapping_targets(
            &mappings,
            &selection_title.facet,
        )?;
        let source_identity = selection.source_identity.clone();
        let client_id = source_identity
            .client_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::Validation("download is no longer available for manual import".to_string())
            })?;
        let completed = crate::import_workflow::resolve_current_manual_import_source(
            self,
            actor,
            client_id,
            &source_identity.client_type,
            &source_identity.item_id,
            &selection.title_id,
        )
        .await
        .map_err(|_| {
            AppError::Validation("download is no longer available for manual import".to_string())
        })?;
        let trusted_root = std::fs::canonicalize(&completed.dest_dir).map_err(|_| {
            AppError::Validation("download is no longer available for manual import".to_string())
        })?;
        for candidate in selection.candidates.iter().filter(|candidate| {
            mappings
                .iter()
                .any(|mapping| mapping.candidate_id == candidate.id)
        }) {
            crate::import_workflow::validate_manual_import_source_under_trusted_root(
                &crate::stored_paths::stored_path_to_path_buf(&candidate.canonical_path),
                &trusted_root,
            )
            .map_err(|_| {
                AppError::Validation(
                    "download is no longer available for manual import".to_string(),
                )
            })?;
        }

        if let Some(existing) = crate::import_workflow::find_active_manual_import_for_source(
            self,
            Some(client_id),
            &source_identity.client_type,
            &source_identity.item_id,
        )
        .await?
        {
            return Ok(existing.id);
        }

        let candidate_ids = mappings
            .iter()
            .map(|mapping| mapping.candidate_id.clone())
            .collect::<Vec<_>>();
        let selection = self
            .services
            .workflow
            .imports
            .consume_manual_import_selection(&selection_id, &actor.id, &candidate_ids)
            .await?
            .ok_or_else(|| {
                AppError::Validation(
                    "manual import selection changed; reopen the import dialog".to_string(),
                )
            })?;
        let candidates = selection
            .candidates
            .iter()
            .map(|candidate| (candidate.id.as_str(), candidate))
            .collect::<HashMap<_, _>>();
        let files = mappings
            .into_iter()
            .map(|mapping| {
                let candidate = candidates.get(mapping.candidate_id.as_str()).ok_or_else(|| {
                    AppError::Validation("manual import candidate is unavailable".to_string())
                })?;
                Ok(crate::ManualImportFileMapping {
                    file_path: candidate.canonical_path.clone(),
                    episode_id: mapping.episode_id,
                    series_movie_link_id: mapping.series_movie_link_id,
                    quality: candidate.quality.clone(),
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        let source_identity = selection.source_identity.clone();
        let payload_json = serde_json::to_string(&crate::ManualImportRequestPayload {
            requested_by_user_id: Some(actor.id.clone()),
            title_id: Some(selection.title_id.clone()),
            download_client_item_id: source_identity.item_id.clone(),
            client_id: source_identity.client_id.clone(),
            client_type: source_identity.client_type.clone(),
            files,
            requested_at: Utc::now().to_rfc3339(),
            selection_id: Some(selection.id),
        })
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let import_id = self
            .services
            .workflow
            .imports
            .queue_import_request(
                source_identity.clone(),
                ImportType::ManualImport.as_str().to_string(),
                payload_json,
            )
            .await?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&selection.title_id)
            .await?;
        self.emit_import_requested_event(
            actor,
            title.as_ref(),
            source_identity.client_type,
            source_identity.item_id,
            scryer_domain::ImportRequestKind::Manual,
        )
        .await;
        Ok(import_id)
    }

}
impl AppUseCase {
    pub async fn trigger_manual_import(
        &self,
        actor: &User,
        completed: &CompletedDownload,
        override_title_id: Option<&str>,
    ) -> AppResult<scryer_domain::ImportResult> {
        // If a title_id override is provided, inject it into the parameters
        let mut completed = completed.clone();
        if let Some(title_id) = override_title_id
            && !completed
                .parameters
                .iter()
                .any(|(k, _)| k == "*scryer_title_id")
        {
            completed
                .parameters
                .push(("*scryer_title_id".to_string(), title_id.to_string()));
        }
        self.require_completed_download_permission(
            actor,
            &completed,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;

        crate::import_workflow::import_completed_download_for_manual_review(self, actor, &completed)
            .await
    }
}
