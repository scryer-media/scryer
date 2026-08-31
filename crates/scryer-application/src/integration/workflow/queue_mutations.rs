impl AppUseCase {
    pub async fn pause_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            None,
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        if let Some(client_id) = client_id.filter(|value| !value.trim().is_empty()) {
            self.services
                .integrations
                .download_client
                .pause_queue_item_for_client(client_id, download_client_item_id)
                .await?;
        } else {
            self.services
                .integrations
                .download_client
                .pause_queue_item(download_client_item_id)
                .await?;
        }
        self.emit_download_queue_item_command_issued_event(
            actor,
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Pause,
        )
        .await;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn resume_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            None,
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        if let Some(client_id) = client_id.filter(|value| !value.trim().is_empty()) {
            self.services
                .integrations
                .download_client
                .resume_queue_item_for_client(client_id, download_client_item_id)
                .await?;
        } else {
            self.services
                .integrations
                .download_client
                .resume_queue_item(download_client_item_id)
                .await?;
        }
        self.emit_download_queue_item_command_issued_event(
            actor,
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Resume,
        )
        .await;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn delete_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
    ) -> AppResult<crate::DownloadQueueCommandRecord> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let client_type = self.normalize_download_client_type(client_type)?;
        let locator = crate::ClientJobLocator::new(client_id, &client_type, download_client_item_id);
        let canonical_download_id = match self
            .services
            .workflow
            .download_registry
            .find_active_binding_by_locator(&locator)
            .await
        {
            Ok(Some(binding)) => Some(binding.download_id),
            Ok(None) => {
                tracing::warn!(
                    target: "download_identity_resolver",
                    client_config_id = ?locator.client_id,
                    client_type = %locator.client_type,
                    native_item_id = %locator.item_id,
                    "queue delete has no active download binding; using legacy command identity"
                );
                None
            }
            Err(error) => {
                tracing::warn!(
                    target: "download_identity_resolver",
                    client_config_id = ?locator.client_id,
                    client_type = %locator.client_type,
                    native_item_id = %locator.item_id,
                    error = %error,
                    "failed to resolve queue delete download binding; using legacy command identity"
                );
                None
            }
        };
        let command_repository = &self.services.workflow.download_queue_commands;
        let command = if let Some(canonical_download_id) = canonical_download_id.as_ref() {
            command_repository
                .queue_delete_command_for_download(
                    Some(canonical_download_id),
                    client_id,
                    &client_type,
                    download_client_item_id,
                    is_history,
                    Some(actor.id.as_str()),
                )
                .await?
        } else {
            command_repository
                .queue_delete_command(
                    client_id,
                    &client_type,
                    download_client_item_id,
                    is_history,
                    Some(actor.id.as_str()),
                )
                .await?
        };
        self.emit_download_queue_item_command_issued_event(
            actor,
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Delete,
        )
        .await;
        Ok(command)
    }
}
