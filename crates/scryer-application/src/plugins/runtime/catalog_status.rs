const CATALOG_STATUS_KEY: &str = "plugin_catalog_redirect";
impl AppUseCase {
    pub async fn plugin_catalog_status(&self, actor: &User) -> AppResult<PluginCatalogStatus> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let stored_status_record = self.load_stored_plugin_catalog_status().await?;
        let (stored_status, stored_checked_at) = stored_status_record
            .map(|(payload, checked_at)| (payload, Some(checked_at)))
            .unwrap_or_default();
        let central_source = self
            .services
            .customization
            .plugin_installations
            .get_plugin_catalog_source(CENTRAL_CATALOG_SOURCE_KEY)
            .await?;
        let central_last_seen = central_source
            .as_ref()
            .and_then(|source| source.last_success_at)
            .or_else(|| central_source.as_ref().map(|source| source.updated_at));
        let github_available = stored_status.github_available
            || central_source
                .as_ref()
                .is_some_and(|source| source.last_success_at.is_some());
        let last_error = stored_status.last_error.clone().or_else(|| {
            central_source
                .as_ref()
                .and_then(|source| source.last_error.clone())
        });
        let degraded = !stored_status.blocked_actions.is_empty()
            || stored_status.message.is_some()
            || stored_status.last_error.is_some();

        Ok(PluginCatalogStatus {
            refresh_state: if degraded {
                "degraded".to_string()
            } else {
                "ready".to_string()
            },
            github_available,
            last_checked_at: stored_checked_at
                .or(central_last_seen)
                .map(|checked_at| checked_at.to_rfc3339()),
            outage_message: stored_status.message,
            blocked_actions: stored_status.blocked_actions,
            restore_warnings: stored_status.restore_warnings,
            last_error,
        })
    }
}

fn plugin_catalog_blocked_actions() -> Vec<String> {
    [
        "catalog_refresh",
        "install",
        "install_manual",
        "upgrade",
        "manual_repo_inspection",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[cfg(test)]
fn combined_plugin_catalog_probe_error(
    primary_error: Option<&str>,
    github_error: Option<&str>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(error) = primary_error.filter(|error| !error.trim().is_empty()) {
        parts.push(format!("primary plugin catalog redirect: {error}"));
    }
    if let Some(error) = github_error.filter(|error| !error.trim().is_empty()) {
        parts.push(format!("GitHub plugin catalog redirect: {error}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

impl AppUseCase {
    async fn load_stored_plugin_catalog_status(
        &self,
    ) -> AppResult<Option<(StoredPluginCatalogStatusPayload, chrono::DateTime<Utc>)>> {
        let Some(record) = self
            .services
            .customization
            .plugin_installations
            .get_plugin_catalog_status(CATALOG_STATUS_KEY)
            .await?
        else {
            return Ok(None);
        };

        let payload = serde_json::from_str(&record.status_json).map_err(|error| {
            AppError::Repository(format!(
                "failed to parse stored plugin catalog status '{}': {error}",
                record.status_key
            ))
        })?;
        Ok(Some((payload, record.checked_at)))
    }

    async fn load_stored_plugin_catalog_status_payload(
        &self,
    ) -> AppResult<StoredPluginCatalogStatusPayload> {
        Ok(self
            .load_stored_plugin_catalog_status()
            .await?
            .map(|(payload, _)| payload)
            .unwrap_or_default())
    }
}
impl AppUseCase {
    async fn persist_plugin_catalog_status_payload(
        &self,
        payload: StoredPluginCatalogStatusPayload,
        checked_at: chrono::DateTime<Utc>,
    ) -> AppResult<()> {
        let status_json = serde_json::to_string(&payload).map_err(|error| {
            AppError::Repository(format!(
                "failed to serialize plugin catalog status payload: {error}"
            ))
        })?;
        self.services
            .customization
            .plugin_installations
            .upsert_plugin_catalog_status(&PluginCatalogStatusRecord {
                status_key: CATALOG_STATUS_KEY.to_string(),
                status_json,
                checked_at,
            })
            .await
    }
}
