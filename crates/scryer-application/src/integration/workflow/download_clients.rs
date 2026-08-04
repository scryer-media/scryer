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
            .await?;
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

        self
            .services
            .integrations
            .download_client
            .get_completed_download_for_source(client_id, client_type, download_client_item_id)
            .await
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let created = self
            .services
            .integrations
            .download_client_configs
            .create(config)
            .await?;
        self.refresh_owned_download_client_categories_best_effort()
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
            })
            .await?;
        self.refresh_owned_download_client_categories_best_effort()
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
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        self.services
            .integrations
            .download_client_configs
            .delete(client_id)
            .await?;
        self.refresh_owned_download_client_categories_best_effort()
            .await;
        self.emit_configuration_changed_event(
            actor,
            "download_client",
            Some(client_id.to_string()),
            scryer_domain::ConfigurationChangeAction::Deleted,
        )
        .await;

        Ok(())
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
