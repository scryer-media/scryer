use super::*;

impl AppUseCase {
    fn normalize_download_client_type(&self, client_type: impl AsRef<str>) -> AppResult<String> {
        let normalized = client_type.as_ref().trim().to_lowercase();
        if normalized.is_empty() {
            return Err(AppError::Validation("client type is required".into()));
        }

        if ALLOWED_DOWNLOAD_CLIENT_TYPES
            .iter()
            .any(|value| value.eq(&normalized.as_str()))
        {
            return Ok(normalized);
        }

        Err(AppError::Validation(format!(
            "unsupported download client type '{}'",
            client_type.as_ref()
        )))
    }

    fn normalize_download_client_config_json(&self, raw: impl AsRef<str>) -> AppResult<String> {
        let raw = raw.as_ref().trim();
        if raw.is_empty() {
            return Ok("{}".to_string());
        }

        let parsed: serde_json::Value =
            serde_json::from_str(raw).map_err(|error| AppError::Validation(error.to_string()))?;
        serde_json::to_string(&parsed).map_err(|error| AppError::Repository(error.to_string()))
    }

    pub async fn list_indexer_configs(
        &self,
        actor: &User,
        provider_filter: Option<String>,
    ) -> AppResult<Vec<IndexerConfig>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services
            .indexer_configs
            .list(provider_filter.map(|provider| provider.trim().to_lowercase()))
            .await
    }

    pub async fn get_indexer_config(
        &self,
        actor: &User,
        config_id: &str,
    ) -> AppResult<Option<IndexerConfig>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services.indexer_configs.get_by_id(config_id).await
    }

    pub async fn create_indexer_config(
        &self,
        actor: &User,
        input: NewIndexerConfig,
    ) -> AppResult<IndexerConfig> {
        require(actor, &Entitlement::ManageConfig)?;

        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Validation("indexer name is required".into()));
        }

        let provider_type = input.provider_type.trim().to_lowercase();
        if provider_type.is_empty() {
            return Err(AppError::Validation("provider type is required".into()));
        }

        let base_url = input.base_url.trim().to_string();
        if base_url.is_empty() {
            return Err(AppError::Validation("base URL is required".into()));
        }

        let api_key_encrypted = input
            .api_key_encrypted
            .map(|value| value.trim().to_string())
            .and_then(|value| if value.is_empty() { None } else { Some(value) });

        if let Some(value) = api_key_encrypted.as_deref() {
            if value.len() < 8 {
                return Err(AppError::Validation(
                    "api key appears too short; provide a valid key".into(),
                ));
            }
        }

        let config = IndexerConfig {
            id: Id::new().0,
            name,
            provider_type,
            base_url,
            api_key_encrypted,
            rate_limit_seconds: input.rate_limit_seconds,
            rate_limit_burst: input.rate_limit_burst,
            disabled_until: None,
            is_enabled: input.is_enabled,
            enable_interactive_search: input.enable_interactive_search,
            enable_auto_search: input.enable_auto_search,
            last_health_status: None,
            last_error_at: None,
            config_json: input.config_json.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.services.indexer_configs.create(config).await
    }

    pub async fn update_indexer_config(
        &self,
        actor: &User,
        config_id: &str,
        name: Option<String>,
        provider_type: Option<String>,
        base_url: Option<String>,
        api_key_encrypted: Option<String>,
        rate_limit_seconds: Option<i64>,
        rate_limit_burst: Option<i64>,
        is_enabled: Option<bool>,
        enable_interactive_search: Option<bool>,
        enable_auto_search: Option<bool>,
        config_json: Option<String>,
    ) -> AppResult<IndexerConfig> {
        require(actor, &Entitlement::ManageConfig)?;

        let has_any_updates = name.is_some()
            || provider_type.is_some()
            || base_url.is_some()
            || api_key_encrypted.is_some()
            || rate_limit_seconds.is_some()
            || rate_limit_burst.is_some()
            || is_enabled.is_some()
            || enable_interactive_search.is_some()
            || enable_auto_search.is_some()
            || config_json.is_some();
        if !has_any_updates {
            return Err(AppError::Validation(
                "at least one indexer field must be provided".into(),
            ));
        }

        let normalized_name = name.map(|value| value.trim().to_string());
        if normalized_name.as_ref().is_some_and(String::is_empty) {
            return Err(AppError::Validation("indexer name cannot be empty".into()));
        }

        let normalized_provider = provider_type.map(|value| value.trim().to_lowercase());
        if normalized_provider.as_ref().is_some_and(String::is_empty) {
            return Err(AppError::Validation("provider type cannot be empty".into()));
        }

        let normalized_base_url = base_url.map(|value| value.trim().to_string());
        if normalized_base_url.as_ref().is_some_and(String::is_empty) {
            return Err(AppError::Validation("base URL cannot be empty".into()));
        }

        let normalized_api_key = api_key_encrypted
            .map(|value| value.trim().to_string())
            .and_then(|value| if value.is_empty() { None } else { Some(value) });

        if let Some(value) = normalized_api_key.as_ref() {
            if value.len() < 8 {
                return Err(AppError::Validation(
                    "api key appears too short; provide a valid key".into(),
                ));
            }
        }

        let updated = self
            .services
            .indexer_configs
            .update(
                config_id,
                normalized_name,
                normalized_provider,
                normalized_base_url,
                normalized_api_key,
                rate_limit_seconds,
                rate_limit_burst,
                is_enabled,
                enable_interactive_search,
                enable_auto_search,
                config_json,
            )
            .await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                None,
                EventType::ActionTriggered,
                format!("indexer config updated: {}", updated.id),
            )
            .await?;

        Ok(updated)
    }

    pub async fn delete_indexer_config(&self, actor: &User, config_id: &str) -> AppResult<()> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services.indexer_configs.delete(config_id).await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                None,
                EventType::ActionTriggered,
                format!("indexer config deleted: {config_id}"),
            )
            .await?;
        Ok(())
    }

    pub async fn list_download_client_configs(
        &self,
        actor: &User,
        client_type: Option<String>,
    ) -> AppResult<Vec<DownloadClientConfig>> {
        require(actor, &Entitlement::ManageConfig)?;

        let client_type = client_type
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(value) = client_type.as_deref() {
            self.normalize_download_client_type(value)?;
        }

        self.services
            .download_client_configs
            .list(client_type)
            .await
    }

    pub async fn list_download_queue(
        &self,
        actor: &User,
        include_all_activity: bool,
        include_history_only: bool,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        require(actor, &Entitlement::ManageConfig)?;

        let mut enabled_clients = self
            .services
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|item| item.is_enabled)
            .collect::<Vec<_>>();

        if enabled_clients.is_empty() {
            return Ok(vec![]);
        }

        enabled_clients.sort_by_key(|config| config.client_priority);
        let primary_client = enabled_clients
            .into_iter()
            .next()
            .ok_or_else(|| AppError::NotFound("no enabled download clients".to_string()))?;

        let queue_items = if include_history_only {
            Vec::new()
        } else {
            self.services.download_client.list_queue().await?
        };
        let history_items = if include_history_only {
            self.services.download_client.list_history().await?
        } else {
            Vec::new()
        };

        let mut items: Vec<DownloadQueueItem> = if include_history_only {
            history_items
        } else {
            queue_items
        };

        // Enrich items with download_submissions data (for SABnzbd which
        // cannot embed metadata in the download itself). This populates
        // title_id, facet, and is_scryer_origin from the submissions table.
        for item in &mut items {
            if item.is_scryer_origin {
                continue;
            }
            if let Ok(Some(submission)) = self
                .services
                .download_submissions
                .find_by_client_item_id(&item.client_type, &item.download_client_item_id)
                .await
            {
                item.is_scryer_origin = true;
                item.title_id = Some(submission.title_id);
                item.facet = Some(submission.facet);
            }
        }

        let merged = items
            .into_iter()
            .filter(|item| include_history_only || include_all_activity || item.is_scryer_origin)
            .filter(|item| {
                if include_history_only {
                    item.state == DownloadQueueState::Completed
                        || item.state == DownloadQueueState::ImportPending
                        || item.state == DownloadQueueState::Failed
                } else {
                    item.state == DownloadQueueState::ImportPending
                        || item.state == DownloadQueueState::Failed
                        || item.state == DownloadQueueState::Downloading
                        || item.state == DownloadQueueState::Queued
                        || item.state == DownloadQueueState::Paused
                }
            })
            .map(|item| {
                let mut mapped = item;
                mapped.client_id = primary_client.id.clone();
                mapped.client_name = primary_client.name.clone();
                mapped.client_type = primary_client.client_type.clone();
                mapped.attention_required = matches!(
                    mapped.state,
                    DownloadQueueState::Failed | DownloadQueueState::ImportPending
                );
                if mapped.attention_reason.is_none() {
                    mapped.attention_reason = if mapped.attention_required {
                        Some("requires attention".to_string())
                    } else {
                        None
                    };
                }
                mapped
            })
            .collect::<Vec<_>>();

        let mut merged = merged;

        if include_history_only {
            merged.sort_by(|left, right| {
                parse_sort_value(right.last_updated_at.as_deref(), left.last_updated_at.as_deref())
            });
            merged.truncate(50);
        } else {
            // Enrich completed/failed items with import status from the imports table
            merged.sort_by(|left, right| {
                let left_rank = queue_state_sort_rank(&left.state);
                let right_rank = queue_state_sort_rank(&right.state);
                if left_rank != right_rank {
                    return left_rank.cmp(&right_rank);
                }

                match left.state {
                    DownloadQueueState::Downloading => {
                        right.progress_percent
                            .cmp(&left.progress_percent)
                            .then_with(|| left.id.cmp(&right.id))
                    }
                    DownloadQueueState::Queued | DownloadQueueState::Paused => {
                        parse_sort_value(left.queued_at.as_deref(), right.queued_at.as_deref())
                    }
                    _ => parse_sort_value(left.last_updated_at.as_deref(), right.last_updated_at.as_deref())
                        .reverse(),
                }
            });
        }

        // Enrich completed/failed items with import status from the imports table
        for item in &mut merged {
            if !matches!(
                item.state,
                DownloadQueueState::Completed | DownloadQueueState::Failed | DownloadQueueState::ImportPending
            ) {
                continue;
            }
            if let Ok(Some(record)) = self
                .services
                .imports
                .get_import_by_source_ref(&item.client_type, &item.download_client_item_id)
                .await
            {
                item.import_status = Some(record.status);
                // Extract error_message from result_json for visibility
                if let Some(ref result_json) = record.result_json {
                    if let Ok(result) = serde_json::from_str::<scryer_domain::ImportResult>(result_json) {
                        if let Some(ref error_msg) = result.error_message {
                            item.import_error_message = Some(error_msg.clone());
                            item.attention_reason = Some(error_msg.clone());
                        }
                    }
                }
            }
        }

        Ok(merged)
    }

    pub fn subscribe_download_queue(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<DownloadQueueItem>>> {
        require(actor, &Entitlement::ManageConfig)?;
        Ok(self.services.download_queue_broadcast.subscribe())
    }

    pub async fn queue_manual_import(
        &self,
        actor: &User,
        title_id: Option<String>,
        client_type: Option<String>,
        download_client_item_id: String,
    ) -> AppResult<String> {
        require(actor, &Entitlement::TriggerActions)?;

        let source_ref = download_client_item_id.trim().to_string();
        if source_ref.is_empty() {
            return Err(AppError::Validation(
                "download client item id is required".to_string(),
            ));
        }

        let normalized_client_type = client_type
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "download_client".to_string());

        let payload_json = serde_json::json!({
            "requested_by_user_id": actor.id.clone(),
            "title_id": title_id.clone(),
            "download_client_item_id": source_ref.clone(),
            "client_type": normalized_client_type.clone(),
            "requested_at": Utc::now().to_rfc3339(),
        })
        .to_string();

        let import_id = self
            .services
            .imports
            .queue_import_request(
                normalized_client_type.clone(),
                source_ref.clone(),
                "manual".to_string(),
                payload_json,
            )
            .await?;

        self.services
            .record_event(
                Some(actor.id.clone()),
                title_id.clone(),
                EventType::ActionTriggered,
                format!("manual import queued for {} ({source_ref})", normalized_client_type),
            )
            .await?;
        self.record_activity_event(
            actor,
            title_id,
            ActivityKind::SystemNotice,
            format!("manual import queued for download item {source_ref}"),
            ActivitySeverity::Info,
            vec![ActivityChannel::WebUi, ActivityChannel::Toast],
        )
        .await?;

        Ok(import_id)
    }

    pub async fn trigger_manual_import(
        &self,
        actor: &User,
        completed: &CompletedDownload,
        override_title_id: Option<&str>,
    ) -> AppResult<scryer_domain::ImportResult> {
        require(actor, &Entitlement::TriggerActions)?;

        // If a title_id override is provided, inject it into the parameters
        let mut completed = completed.clone();
        if let Some(title_id) = override_title_id {
            if !completed.parameters.iter().any(|(k, _)| k == "*scryer_title_id") {
                completed
                    .parameters
                    .push(("*scryer_title_id".to_string(), title_id.to_string()));
            }
        }

        crate::app_usecase_import::import_completed_download(self, actor, &completed).await
    }

    pub async fn pause_download_queue_item(
        &self,
        actor: &User,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        require(actor, &Entitlement::TriggerActions)?;
        self.services
            .download_client
            .pause_queue_item(download_client_item_id)
            .await?;
        self.record_activity_event(
            actor,
            None,
            ActivityKind::SystemNotice,
            format!("download paused: {download_client_item_id}"),
            ActivitySeverity::Info,
            vec![ActivityChannel::WebUi],
        )
        .await?;
        Ok(())
    }

    pub async fn resume_download_queue_item(
        &self,
        actor: &User,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        require(actor, &Entitlement::TriggerActions)?;
        self.services
            .download_client
            .resume_queue_item(download_client_item_id)
            .await?;
        self.record_activity_event(
            actor,
            None,
            ActivityKind::SystemNotice,
            format!("download resumed: {download_client_item_id}"),
            ActivitySeverity::Info,
            vec![ActivityChannel::WebUi],
        )
        .await?;
        Ok(())
    }

    pub async fn delete_download_queue_item(
        &self,
        actor: &User,
        download_client_item_id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        require(actor, &Entitlement::TriggerActions)?;
        self.services
            .download_client
            .delete_queue_item(download_client_item_id, is_history)
            .await?;
        self.record_activity_event(
            actor,
            None,
            ActivityKind::SystemNotice,
            format!("download deleted: {download_client_item_id}"),
            ActivitySeverity::Info,
            vec![ActivityChannel::WebUi],
        )
        .await?;
        Ok(())
    }

    pub async fn get_download_client_config(
        &self,
        actor: &User,
        client_id: &str,
    ) -> AppResult<Option<DownloadClientConfig>> {
        require(actor, &Entitlement::ManageConfig)?;
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        self.services
            .download_client_configs
            .get_by_id(client_id)
            .await
    }

    pub async fn create_download_client_config(
        &self,
        actor: &User,
        input: NewDownloadClientConfig,
    ) -> AppResult<DownloadClientConfig> {
        require(actor, &Entitlement::ManageConfig)?;

        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Validation(
                "download client name is required".into(),
            ));
        }

        let client_type = self.normalize_download_client_type(input.client_type)?;
        let base_url = input
            .base_url
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let config_json = self.normalize_download_client_config_json(input.config_json)?;

        let existing = self
            .services
            .download_client_configs
            .list(None)
            .await?;
        let client_priority = existing
            .into_iter()
            .map(|entry| entry.client_priority)
            .max()
            .unwrap_or(0)
            + 1;

        let config = DownloadClientConfig {
            id: Id::new().0,
            name,
            client_type,
            base_url,
            config_json,
            client_priority,
            is_enabled: input.is_enabled,
            status: "idle".to_string(),
            last_error: None,
            last_seen_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let created = self.services.download_client_configs.create(config).await?;
        self.record_activity_event(
            actor,
            None,
            ActivityKind::SettingSaved,
            format!("download client created: {}", created.id),
            ActivitySeverity::Success,
            vec![ActivityChannel::WebUi, ActivityChannel::Toast],
        )
        .await?;

        Ok(created)
    }

    pub async fn update_download_client_config(
        &self,
        actor: &User,
        client_id: &str,
        name: Option<String>,
        client_type: Option<String>,
        base_url: Option<String>,
        config_json: Option<String>,
        is_enabled: Option<bool>,
    ) -> AppResult<DownloadClientConfig> {
        require(actor, &Entitlement::ManageConfig)?;
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        let has_any_updates = name.is_some()
            || client_type.is_some()
            || base_url.is_some()
            || config_json.is_some()
            || is_enabled.is_some();
        if !has_any_updates {
            return Err(AppError::Validation(
                "at least one download client field must be provided".into(),
            ));
        }

        let normalized_name = name.map(|value| value.trim().to_string());
        if normalized_name
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            return Err(AppError::Validation("client name cannot be empty".into()));
        }

        let normalized_client_type = match client_type {
            Some(value) => Some(self.normalize_download_client_type(value)?),
            None => None,
        };
        let normalized_base_url = base_url
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let normalized_config_json = match config_json {
            Some(value) => Some(self.normalize_download_client_config_json(value)?),
            None => None,
        };

        let updated = self
            .services
            .download_client_configs
            .update(
                client_id,
                normalized_name,
                normalized_client_type,
                normalized_base_url,
                normalized_config_json,
                is_enabled,
            )
            .await?;
        self.record_activity_event(
            actor,
            None,
            ActivityKind::SettingSaved,
            format!("download client updated: {}", updated.id),
            ActivitySeverity::Success,
            vec![ActivityChannel::WebUi, ActivityChannel::Toast],
        )
        .await?;

        Ok(updated)
    }

    pub async fn delete_download_client_config(
        &self,
        actor: &User,
        client_id: &str,
    ) -> AppResult<()> {
        require(actor, &Entitlement::ManageConfig)?;
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        self.services
            .download_client_configs
            .delete(client_id)
            .await?;
        self.record_activity_event(
            actor,
            None,
            ActivityKind::SettingSaved,
            format!("download client deleted: {client_id}"),
            ActivitySeverity::Success,
            vec![ActivityChannel::WebUi, ActivityChannel::Toast],
        )
        .await?;

        Ok(())
    }
}

pub async fn start_download_queue_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    let actor = match app.find_or_create_default_user().await {
        Ok(actor) => actor,
        Err(error) => {
            tracing::warn!(error = %error, "download queue poller failed to resolve actor");
            return;
        }
    };

    // TODO: revert this aggressive queue polling cadence after PP diagnostics are complete.
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                tracing::info!("download queue poller shutting down");
                break;
            }
            _ = interval.tick() => {
                match app.list_download_queue(&actor, true, false).await {
                    Ok(items) => {
                        // Trigger imports for any newly-completed downloads
                        crate::app_usecase_import::try_import_completed_downloads(&app, &actor, &items).await;

                        // Emit download queue gauge by state
                        let mut counts = [0u64; 6];
                        for item in &items {
                            match item.state {
                                scryer_domain::DownloadQueueState::Queued => counts[0] += 1,
                                scryer_domain::DownloadQueueState::Downloading => counts[1] += 1,
                                scryer_domain::DownloadQueueState::Paused => counts[2] += 1,
                                scryer_domain::DownloadQueueState::Completed => counts[3] += 1,
                                scryer_domain::DownloadQueueState::ImportPending => counts[4] += 1,
                                scryer_domain::DownloadQueueState::Failed => counts[5] += 1,
                            }
                        }
                        let labels = ["queued", "downloading", "paused", "completed", "import_pending", "failed"];
                        for (label, &count) in labels.iter().zip(&counts) {
                            metrics::gauge!("scryer_download_queue_items", "state" => *label).set(count as f64);
                        }

                        let _ = app
                            .services
                            .download_queue_broadcast
                            .send(items);
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "download queue poll failed");
                    }
                }
            }
        }
    }
}

fn parse_sort_value(left: Option<&str>, right: Option<&str>) -> std::cmp::Ordering {
    fn parse(value: Option<&str>) -> i64 {
        value
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0)
    }

    parse(left).cmp(&parse(right))
}

fn queue_state_sort_rank(state: &DownloadQueueState) -> u8 {
    match state {
        DownloadQueueState::Downloading => 0,
        DownloadQueueState::Queued => 1,
        DownloadQueueState::Paused => 2,
        DownloadQueueState::ImportPending => 3,
        DownloadQueueState::Completed => 3,
        DownloadQueueState::Failed => 4,
    }
}
