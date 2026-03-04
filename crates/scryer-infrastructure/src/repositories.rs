use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, DownloadClientConfigRepository, EventRepository, IndexerConfigRepository,
    ImportRepository, MediaFileRepository, PluginInstallationRepository, PrimaryCollectionSummary,
    ReleaseDecision, RuleSetRepository, SystemInfoProvider, TitleMediaFile, WantedItem, WantedItemRepository,
    QualityProfile as ApplicationQualityProfile, QualityProfileRepository,
    ReleaseAttemptRepository, ReleaseDownloadAttemptOutcome, ReleaseDownloadFailureSignature,
    TitleReleaseBlocklistEntry,
    SettingsRepository, ShowRepository, TitleMetadataUpdate, TitleRepository, UserRepository,
};
use scryer_domain::{
    CalendarEpisode, Collection, DownloadClientConfig, Entitlement, Episode, HistoryEvent,
    ImportRecord, IndexerConfig, MediaFacet, PluginInstallation, RuleSet, Title, User,
};
use tokio::sync::oneshot;

use crate::sqlite_services::SqliteServices;

#[async_trait]
impl TitleRepository for SqliteServices {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListTitles {
                facet,
                query,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::GetTitleById {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn create(&self, title: Title) -> AppResult<Title> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::CreateTitle {
                title,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::UpdateTitleMonitored {
                id: id.to_string(),
                monitored,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        let tags_json = match tags {
            Some(tags) => Some(
                serde_json::to_string(&tags)
                    .map_err(|err| AppError::Repository(err.to_string()))?,
            ),
            None => None,
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::UpdateTitleMetadata {
                id: id.to_string(),
                name,
                facet,
                tags_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::UpdateTitleHydratedMetadata {
                id: id.to_string(),
                metadata,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::DeleteTitle {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }
}

#[async_trait]
impl ShowRepository for SqliteServices {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListCollectionsForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListPrimaryCollectionSummaries {
                title_ids: title_ids.to_vec(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::GetCollectionById {
                collection_id: collection_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::CreateCollection {
                collection,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn update_collection(
        &self,
        collection_id: &str,
        collection_type: Option<String>,
        collection_index: Option<String>,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
        monitored: Option<bool>,
    ) -> AppResult<Collection> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::UpdateCollection {
                collection_id: collection_id.to_string(),
                collection_type,
                collection_index,
                label,
                ordered_path,
                first_episode_number,
                last_episode_number,
                monitored,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::SetCollectionEpisodesMonitored {
                collection_id: collection_id.to_string(),
                monitored,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::DeleteCollection {
                collection_id: collection_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListEpisodesForCollection {
                collection_id: collection_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::GetEpisodeById {
                episode_id: episode_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::CreateEpisode {
                episode,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn update_episode(
        &self,
        episode_id: &str,
        episode_type: Option<String>,
        episode_number: Option<String>,
        season_number: Option<String>,
        episode_label: Option<String>,
        title: Option<String>,
        air_date: Option<String>,
        duration_seconds: Option<i64>,
        has_multi_audio: Option<bool>,
        has_subtitle: Option<bool>,
        monitored: Option<bool>,
        collection_id: Option<String>,
    ) -> AppResult<Episode> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::UpdateEpisode {
                episode_id: episode_id.to_string(),
                episode_type,
                episode_number,
                season_number,
                episode_label,
                title,
                air_date,
                duration_seconds,
                has_multi_audio,
                has_subtitle,
                monitored,
                collection_id,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::DeleteEpisode {
                episode_id: episode_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        self.find_episode_by_title_and_numbers(title_id, season_number, episode_number)
            .await
    }

    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        self.find_episode_by_title_and_absolute_number(title_id, absolute_number)
            .await
    }

    async fn list_episodes_in_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListEpisodesInDateRange {
                start_date: start_date.to_string(),
                end_date: end_date.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }
}

#[async_trait]
impl UserRepository for SqliteServices {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::GetUserByUsername {
                username: username.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::GetUserById {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn create(&self, user: User) -> AppResult<User> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::CreateUser {
                user,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn list_all(&self) -> AppResult<Vec<User>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListUsers {
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn update_entitlements(
        &self,
        id: &str,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User> {
        let entitlements_json = serde_json::to_string(&entitlements)
            .map_err(|err| AppError::Repository(err.to_string()))?;

        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::UpdateUserEntitlements {
                id: id.to_string(),
                entitlements_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::UpdateUserPassword {
                id: id.to_string(),
                password_hash,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::DeleteUser {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }
}

#[async_trait]
impl EventRepository for SqliteServices {
    async fn list(
        &self,
        title_id: Option<String>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<HistoryEvent>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListEvents {
                title_id,
                limit,
                offset,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn append(&self, event: HistoryEvent) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::AppendEvent {
                event,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }
}

#[async_trait]
impl IndexerConfigRepository for SqliteServices {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListIndexerConfigs {
                provider_type,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::GetIndexerConfig {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::CreateIndexerConfig {
                config,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn touch_last_error(&self, provider_type: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::TouchIndexerLastError {
                provider_type: provider_type.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn update(
        &self,
        id: &str,
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
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::UpdateIndexerConfig {
                id: id.to_string(),
                name,
                provider_type,
                base_url,
                api_key_encrypted,
                rate_limit_seconds,
                rate_limit_burst,
                is_enabled,
                enable_interactive_search,
                enable_auto_search,
                config_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::DeleteIndexerConfig {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }
}

#[async_trait]
impl SettingsRepository for SqliteServices {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        match self
            .get_setting_with_defaults(scope.to_string(), key_name.to_string(), scope_id)
            .await?
        {
            Some(record) => Ok(Some(record.effective_value_json)),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl SystemInfoProvider for SqliteServices {
    async fn current_migration_version(&self) -> AppResult<Option<String>> {
        let applied = self.list_applied_migrations().await?;
        let latest = applied
            .iter()
            .filter(|m| m.success)
            .max_by_key(|m| {
                m.migration_key
                    .split('_')
                    .next()
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(-1)
            })
            .map(|m| m.migration_key.clone());
        Ok(latest)
    }

    async fn pending_migration_count(&self) -> AppResult<usize> {
        let applied = self.list_applied_migrations().await?;
        let applied_keys: std::collections::HashSet<String> = applied
            .iter()
            .filter(|m| m.success)
            .map(|m| m.migration_key.clone())
            .collect();
        let embedded = crate::list_embedded_migrations()?;
        let pending = embedded
            .iter()
            .filter(|m| !applied_keys.contains(&m.key))
            .count();
        Ok(pending)
    }

    async fn smg_cert_expires_at(&self) -> AppResult<Option<String>> {
        match self
            .get_setting_with_defaults("system", "smg.cert_expires_at", None)
            .await?
        {
            Some(record) => {
                let value = record.effective_value_json.trim_matches('"').to_string();
                if value.is_empty() || value == "null" {
                    Ok(None)
                } else {
                    Ok(Some(value))
                }
            }
            None => Ok(None),
        }
    }
}

#[async_trait]
impl DownloadClientConfigRepository for SqliteServices {
    async fn list(&self, client_type: Option<String>) -> AppResult<Vec<DownloadClientConfig>> {
        self.list_download_client_configs(client_type).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
        self.get_download_client_config(id).await
    }

    async fn create(&self, config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
        self.create_download_client_config(config).await
    }

    async fn update(
        &self,
        id: &str,
        name: Option<String>,
        client_type: Option<String>,
        base_url: Option<String>,
        config_json: Option<String>,
        is_enabled: Option<bool>,
    ) -> AppResult<DownloadClientConfig> {
        self.update_download_client_config(
            id,
            name,
            client_type,
            base_url,
            config_json,
            is_enabled,
        )
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.delete_download_client_config(id).await
    }
}

#[async_trait]
impl ReleaseAttemptRepository for SqliteServices {
    async fn record_release_attempt(
        &self,
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
    ) -> AppResult<()> {
        self.create_release_download_attempt(
            title_id,
            source_hint,
            source_title,
            outcome,
            error_message,
            source_password,
        )
        .await
    }

    async fn list_failed_release_signatures(
        &self,
        limit: usize,
    ) -> AppResult<Vec<ReleaseDownloadFailureSignature>> {
        self.list_failed_release_download_attempt_signatures(limit).await
    }

    async fn list_failed_release_signatures_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        self.list_failed_release_download_attempts_for_title(title_id, limit)
            .await
    }

    async fn get_latest_source_password(
        &self,
        title_id: Option<&str>,
        source_hint: Option<&str>,
        source_title: Option<&str>,
    ) -> AppResult<Option<String>> {
        self.get_latest_source_password(title_id, source_hint, source_title).await
    }
}

#[async_trait]
impl ImportRepository for SqliteServices {
    async fn queue_import_request(
        &self,
        source_system: String,
        source_ref: String,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        self.create_import_request(source_system, source_ref, import_type, payload_json)
            .await
    }

    async fn get_import_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Option<ImportRecord>> {
        self.get_import_by_source_ref(source_system, source_ref).await
    }

    async fn update_import_status(
        &self,
        import_id: &str,
        status: &str,
        result_json: Option<String>,
    ) -> AppResult<()> {
        self.update_import_status(import_id, status, result_json).await
    }

    async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64> {
        self.recover_stale_processing_imports(stale_seconds).await
    }

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        self.list_pending_imports().await
    }

    async fn is_already_imported(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<bool> {
        match self.get_import_by_source_ref(source_system, source_ref).await? {
            Some(record) => Ok(record.status == "completed"),
            None => Ok(false),
        }
    }

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>> {
        self.list_imports(limit as i64).await
    }
}

#[async_trait]
impl QualityProfileRepository for SqliteServices {
    async fn list_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<ApplicationQualityProfile>> {
        let (reply_tx, reply_rx): (
            oneshot::Sender<AppResult<Vec<ApplicationQualityProfile>>>,
            oneshot::Receiver<AppResult<Vec<ApplicationQualityProfile>>>,
        ) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListQualityProfiles {
                scope: scope.to_string(),
                scope_id,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }
}

#[async_trait]
impl MediaFileRepository for SqliteServices {
    async fn insert_media_file(
        &self,
        title_id: &str,
        file_path: &str,
        size_bytes: i64,
        quality_label: Option<String>,
    ) -> AppResult<String> {
        self.insert_media_file(title_id, file_path, size_bytes, quality_label)
            .await
    }

    async fn link_file_to_episode(
        &self,
        file_id: &str,
        episode_id: &str,
    ) -> AppResult<()> {
        self.link_file_to_episode(file_id, episode_id).await
    }

    async fn list_media_files_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<TitleMediaFile>> {
        self.list_media_files_for_title(title_id).await
    }
}

#[async_trait]
impl WantedItemRepository for SqliteServices {
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String> {
        self.upsert_wanted_item(item).await
    }

    async fn list_due_wanted_items(&self, now: &str, batch_limit: i64) -> AppResult<Vec<WantedItem>> {
        self.list_due_wanted_items(now, batch_limit).await
    }

    async fn update_wanted_item_status(
        &self,
        id: &str,
        status: &str,
        next_search_at: Option<&str>,
        last_search_at: Option<&str>,
        search_count: i64,
        current_score: Option<i32>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        self.update_wanted_item_status(
            id, status, next_search_at, last_search_at,
            search_count, current_score, grabbed_release,
        ).await
    }

    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>> {
        self.get_wanted_item_for_title(title_id, episode_id).await
    }

    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
        self.delete_wanted_items_for_title(title_id).await
    }

    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
        self.insert_release_decision(decision).await
    }

    async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>> {
        self.get_wanted_item_by_id(id).await
    }

    async fn list_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<WantedItem>> {
        self.list_wanted_items(status, media_type, title_id, limit, offset).await
    }

    async fn count_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
    ) -> AppResult<i64> {
        self.count_wanted_items(status, media_type, title_id).await
    }

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        self.list_release_decisions_for_title(title_id, limit).await
    }

    async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        self.list_release_decisions_for_wanted_item(wanted_item_id, limit).await
    }
}

#[async_trait]
impl RuleSetRepository for SqliteServices {
    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListRuleSets { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListEnabledRuleSets { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::GetRuleSet {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::CreateRuleSet {
                rule_set: rule_set.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::UpdateRuleSet {
                rule_set: rule_set.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::DeleteRuleSet {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::RecordRuleSetHistory {
                id: scryer_domain::Id::new().0,
                rule_set_id: rule_set_id.to_string(),
                action: action.to_string(),
                rego_source: rego_source.map(|s| s.to_string()),
                actor_id: actor_id.map(|s| s.to_string()),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }
}

#[async_trait]
impl PluginInstallationRepository for SqliteServices {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::ListPluginInstallations { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn get_plugin_installation(&self, plugin_id: &str) -> AppResult<Option<PluginInstallation>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::GetPluginInstallation {
                plugin_id: plugin_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::CreatePluginInstallation {
                installation: installation.clone(),
                wasm_bytes: wasm_bytes.map(|b| b.to_vec()),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
    ) -> AppResult<PluginInstallation> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::UpdatePluginInstallation {
                installation: installation.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::DeletePluginInstallation {
                plugin_id: plugin_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn get_enabled_plugin_wasm_bytes(&self) -> AppResult<Vec<(PluginInstallation, Option<Vec<u8>>)>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::GetEnabledPluginWasmBytes { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::SeedBuiltinPlugin {
                plugin_id: plugin_id.to_string(),
                name: name.to_string(),
                description: description.to_string(),
                version: version.to_string(),
                provider_type: provider_type.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn store_registry_cache(&self, json: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::StoreRegistryCache {
                json: json.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }

    async fn get_registry_cache(&self) -> AppResult<Option<String>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(crate::commands::DbCommand::GetRegistryCache { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx.await.map_err(|err| AppError::Repository(err.to_string()))?
    }
}
