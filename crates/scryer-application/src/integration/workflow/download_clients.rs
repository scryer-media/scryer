fn normalized_download_client_id(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_string()
}
impl AppUseCase {
    fn normalize_download_client_type(&self, client_type: impl AsRef<str>) -> AppResult<String> {
        let normalized = client_type.as_ref().trim().to_lowercase();
        if normalized.is_empty() {
            return Err(AppError::Validation("client type is required".into()));
        }

        if NATIVE_DOWNLOAD_CLIENT_TYPES
            .iter()
            .any(|value| value.eq(&normalized.as_str()))
        {
            return Ok(normalized);
        }

        if self
            .services
            .integrations
            .download_client_plugin_provider
            .available()
            .is_some_and(|provider| {
                provider
                    .available_provider_types()
                    .into_iter()
                    .any(|value| value == normalized)
            })
        {
            return Ok(normalized);
        }

        Err(AppError::Validation(format!(
            "unsupported download client type '{}'",
            client_type.as_ref()
        )))
    }
}
impl AppUseCase {
    fn normalize_download_client_config_json(&self, raw: impl AsRef<str>) -> AppResult<String> {
        let raw = raw.as_ref().trim();
        if raw.is_empty() {
            return Ok("{}".to_string());
        }

        let parsed: serde_json::Value =
            serde_json::from_str(raw).map_err(|error| AppError::Validation(error.to_string()))?;
        serde_json::to_string(&parsed).map_err(|error| AppError::Repository(error.to_string()))
    }
}
impl AppUseCase {
    pub async fn list_download_client_configs(
        &self,
        actor: &User,
        client_type: Option<String>,
    ) -> AppResult<Vec<DownloadClientConfig>> {
        let settings_permissions = scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageSystemSettings,
            scryer_domain::AppPermission::ManageCatalogSettings,
        ]);
        if !self
            .has_any_app_permission(actor, settings_permissions)
            .await?
        {
            return Err(AppError::Unauthorized(
                "You do not have permission to perform this action".to_string(),
            ));
        }

        let client_type = client_type
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(value) = client_type.as_deref() {
            self.normalize_download_client_type(value)?;
        }

        self.services
            .integrations
            .download_client_configs
            .list(client_type)
            .await
    }
}
impl AppUseCase {
    async fn enabled_download_clients_by_priority(&self) -> AppResult<Vec<DownloadClientConfig>> {
        let mut enabled_clients = self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|item| item.is_enabled)
            .collect::<Vec<_>>();

        enabled_clients.sort_by_key(|config| config.client_priority);
        Ok(enabled_clients)
    }
}
impl AppUseCase {
    pub(crate) async fn resolve_manual_import_source(
        &self,
        client_id: Option<&str>,
        client_type: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<ManualImportSourceResolution> {
        let source_ref = download_client_item_id.trim();
        if source_ref.is_empty() {
            return Ok(ManualImportSourceResolution::NotEligible {
                message: "download client item id is required".to_string(),
            });
        }

        let completed = self
            .find_completed_manual_import_source(client_id, client_type, source_ref)
            .await?
            .map(Box::new);
        if completed.is_some() {
            Ok(ManualImportSourceResolution::Eligible { completed })
        } else {
            Ok(ManualImportSourceResolution::NotEligible {
                message: format!("download source {source_ref} is no longer available for import"),
            })
        }
    }
}
impl AppUseCase {
    async fn find_completed_manual_import_source(
        &self,
        client_id: Option<&str>,
        client_type: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<Option<CompletedDownload>> {
        let Some(client_id) = client_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let Some(client_type) = client_type.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let download_client_item_id = download_client_item_id.trim();
        if download_client_item_id.is_empty() {
            return Ok(None);
        }

        let identity = ClientJobLocator::new(Some(client_id), client_type, download_client_item_id);
        let has_scryer_submission = self
            .services
            .workflow
            .download_submissions
            .find_by_client_item_id(&identity)
            .await?
            .as_ref()
            .is_some_and(crate::import_parameters::submission_has_scryer_origin);

        let Some(completed) = self
            .services
            .integrations
            .download_client
            .get_completed_download_for_source(client_id, client_type, download_client_item_id)
            .await?
        else {
            return Ok(None);
        };
        Ok((self
            .completed_download_admission(has_scryer_submission, &completed, None)
            .await
            == crate::services::CompletedDownloadAdmission::Admitted)
            .then_some(completed))
    }
}
impl AppUseCase {
    pub async fn get_download_client_config(
        &self,
        actor: &User,
        client_id: &str,
    ) -> AppResult<Option<DownloadClientConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        self.services
            .integrations
            .download_client_configs
            .get_by_id(client_id)
            .await
    }
}
impl AppUseCase {
    /// Mirror of the indexer rule (`validate_enabled_proxy_config_id`): a
    /// download client may only reference a proxy that exists and is enabled.
    /// Any kind is allowed, including a challenge solver — the operator's
    /// choice, with the semantics documented on the GraphQL field.
    async fn validate_enabled_download_client_proxy(&self, raw_id: &str) -> AppResult<String> {
        let id = raw_id.trim();
        if id.is_empty() {
            return Err(AppError::Validation(
                "proxy config id cannot be empty".into(),
            ));
        }
        let config = self
            .services
            .integrations
            .proxy_configs
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::Validation("Proxy configuration was not found.".into()))?;
        if !config.is_enabled {
            return Err(AppError::Validation(
                "Proxy is disabled for this download client.".into(),
            ));
        }
        Ok(config.id)
    }
}
impl AppUseCase {
    pub async fn create_download_client_config(
        &self,
        actor: &User,
        input: NewDownloadClientConfig,
    ) -> AppResult<DownloadClientConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Validation(
                "download client name is required".into(),
            ));
        }

        let client_type = self.normalize_download_client_type(input.client_type)?;
        let config_json = self.normalize_download_client_config_json(input.config_json)?;
        crate::parse_download_client_remote_path_mappings(&config_json)?;

        let proxy_config_id = match input.proxy_config_id {
            Some(id) => Some(self.validate_enabled_download_client_proxy(&id).await?),
            None => None,
        };

        let existing = self
            .services
            .integrations
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
            config_json,
            client_priority,
            is_enabled: input.is_enabled,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            proxy_config_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let created = self
            .services
            .integrations
            .download_client_configs
            .create(config)
            .await?;
        self.refresh_download_client_category_admission_best_effort()
            .await;
        self.emit_configuration_changed_event(
            actor,
            "download_client",
            Some(created.id.clone()),
            scryer_domain::ConfigurationChangeAction::Saved,
        )
        .await;

        Ok(created)
    }
}
impl AppUseCase {
    pub async fn update_download_client_config(
        &self,
        actor: &User,
        update: DownloadClientConfigUpdate,
    ) -> AppResult<DownloadClientConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let client_id = update.id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one download client field must be provided".into(),
            ));
        }

        let normalized_name = update.name.map(|value| value.trim().to_string());
        if normalized_name
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            return Err(AppError::Validation("client name cannot be empty".into()));
        }

        let normalized_client_type = match update.client_type {
            Some(value) => Some(self.normalize_download_client_type(value)?),
            None => None,
        };
        let normalized_config_json = match update.config_json {
            Some(value) => {
                let normalized = self.normalize_download_client_config_json(value)?;
                crate::parse_download_client_remote_path_mappings(&normalized)?;
                Some(normalized)
            }
            None => None,
        };

        // Tri-state, exactly like `IndexerConfigUpdate::proxy_config_id`:
        // omitted keeps the stored assignment, an explicit null clears it, and
        // a value has to name a proxy that exists and is enabled.
        let proxy_config_id_patch = match update.proxy_config_id {
            Some(Some(id)) => Some(Some(
                self.validate_enabled_download_client_proxy(&id).await?,
            )),
            Some(None) => Some(None),
            None => None,
        };

        if let Some(client_type) = normalized_client_type.as_deref() {
            let existing_client = self
                .services
                .integrations
                .download_client_configs
                .get_by_id(client_id)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("download client config '{client_id}' not found"))
                })?;
            let mut candidate_client = existing_client;
            candidate_client.client_type = client_type.to_string();
            let mapped_indexers = self
                .services
                .integrations
                .indexer_configs
                .list(None)
                .await?;
            for indexer in mapped_indexers
                .iter()
                .filter(|indexer| indexer.download_client_id.as_deref() == Some(client_id))
            {
                self.validate_indexer_download_client_mapping(indexer, &candidate_client)?;
            }
        }

        let updated = self
            .services
            .integrations
            .download_client_configs
            .update(DownloadClientConfigUpdate {
                id: client_id.to_string(),
                name: normalized_name,
                client_type: normalized_client_type,
                config_json: normalized_config_json,
                is_enabled: update.is_enabled,
                proxy_config_id: proxy_config_id_patch,
            })
            .await?;
        self.refresh_download_client_category_admission_best_effort()
            .await;
        self.emit_configuration_changed_event(
            actor,
            "download_client",
            Some(updated.id.clone()),
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;

        Ok(updated)
    }
}
impl AppUseCase {
    pub async fn delete_download_client_config(
        &self,
        actor: &User,
        client_id: &str,
    ) -> AppResult<u64> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        let mapped_indexer_ids = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.download_client_id.as_deref() == Some(client_id))
            .map(|config| config.id)
            .collect::<Vec<_>>();

        let cleared_count = self
            .services
            .integrations
            .download_client_configs
            .delete_with_cleared_indexer_mapping_count(client_id)
            .await?;
        self.refresh_download_client_category_admission_best_effort()
            .await;
        for indexer_id in mapped_indexer_ids {
            self.emit_configuration_changed_event(
                actor,
                "indexer",
                Some(indexer_id),
                scryer_domain::ConfigurationChangeAction::Updated,
            )
            .await;
        }
        self.emit_configuration_changed_event(
            actor,
            "download_client",
            Some(client_id.to_string()),
            scryer_domain::ConfigurationChangeAction::Deleted,
        )
        .await;

        Ok(cleared_count)
    }
}
impl AppUseCase {
    pub async fn reorder_download_clients(
        &self,
        actor: &User,
        ordered_ids: Vec<String>,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .download_client_configs
            .reorder(ordered_ids)
            .await
    }
}
