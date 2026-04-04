use async_trait::async_trait;
use scryer_application::{
    AcquisitionStateRepository, AppError, AppResult, BlocklistRepository, DomainEventRepository,
    DownloadClientConfigRepository, DownloadSubmission, DownloadSubmissionRepository,
    HousekeepingRepository, ImportArtifact, ImportArtifactRepository, ImportRepository,
    IndexerConfigRepository, InsertMediaFileInput, JobKey, JobRunRecord, JobRunRepository,
    JobRunStatus, JobTriggerSource, LibraryProbeRepository, LibraryProbeSignature,
    MediaFileRepository, NewBlocklistEntry, NewTitleHistoryEvent, NotificationChannelRepository,
    NotificationSubscriptionRepository, PendingRelease, PendingReleaseRepository,
    PendingReleaseStatus, PluginInstallationRepository, PostProcessingScriptRepository,
    PrimaryCollectionSummary, QualityProfile as ApplicationQualityProfile,
    QualityProfileRepository, ReleaseAttemptRepository, ReleaseDecision,
    ReleaseDownloadAttemptOutcome, ReleaseDownloadFailureSignature, RuleSetRepository,
    SettingsRepository, ShowRepository, SuccessfulGrabCommit, SystemInfoProvider,
    TitleHistoryFilter, TitleHistoryPage, TitleHistoryRepository, TitleMediaFile,
    TitleMediaSizeSummary, TitleMetadataUpdate, TitleReleaseBlocklistEntry, TitleRepository,
    UserRepository, WantedItem, WantedItemRepository,
};
use scryer_domain::{
    BlocklistEntry, CalendarEpisode, Collection, CollectionType, DomainEvent, DomainEventFilter,
    DownloadClientConfig, Entitlement, Episode, ImportRecord, ImportStatus, IndexerConfig,
    MediaFacet, NewDomainEvent, NotificationChannelConfig, NotificationSubscription,
    PluginInstallation, PostProcessingScript, PostProcessingScriptRun, RuleSet, Title,
    TitleHistoryEventType, TitleHistoryRecord, User,
};
use std::collections::HashMap;

use crate::sqlite_services::SqliteServices;

fn parse_rfc3339_or_now(value: Option<String>) -> chrono::DateTime<chrono::Utc> {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now)
}

fn job_run_record_from_workflow(record: crate::WorkflowOperationRecord) -> AppResult<JobRunRecord> {
    let job_key = record
        .job_key
        .as_deref()
        .and_then(JobKey::parse)
        .ok_or_else(|| AppError::Repository("workflow operation missing valid job_key".into()))?;
    let trigger_source = record
        .trigger_source
        .as_deref()
        .and_then(JobTriggerSource::parse)
        .ok_or_else(|| {
            AppError::Repository("workflow operation missing valid trigger_source".into())
        })?;
    let status = JobRunStatus::parse(&record.status)
        .ok_or_else(|| AppError::Repository("workflow operation missing valid status".into()))?;

    Ok(JobRunRecord {
        id: record.id,
        job_key,
        operation_type: record.operation_type,
        status,
        trigger_source,
        actor_user_id: record.actor_user_id,
        progress_json: record.progress_json,
        summary_json: record.summary_json,
        summary_text: record.summary_text,
        error_text: record.error_text,
        started_at: parse_rfc3339_or_now(record.started_at),
        completed_at: record
            .completed_at
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
            .map(|value| value.with_timezone(&chrono::Utc)),
        created_at: parse_rfc3339_or_now(Some(record.created_at)),
        updated_at: parse_rfc3339_or_now(Some(record.updated_at)),
    })
}

macro_rules! db_call {
    ($self:ident, $cmd:ident { $($field:ident),* $(,)? }) => {{
        let (reply_tx, reply_rx) = ::tokio::sync::oneshot::channel();
        $self.sender
            .send(crate::commands::DbCommand::$cmd { $($field,)* reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }};
}

#[async_trait]
impl AcquisitionStateRepository for SqliteServices {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
        self.commit_successful_grab(commit.clone()).await
    }
}

#[async_trait]
impl TitleRepository for SqliteServices {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        db_call!(self, ListTitles { facet, query })
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        let id = id.to_string();
        db_call!(self, GetTitleById { id })
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        crate::queries::title::get_title_by_external_id_query(&self.pool, source, value).await
    }

    async fn create(&self, title: Title) -> AppResult<Title> {
        db_call!(self, CreateTitle { title })
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        let id = id.to_string();
        db_call!(self, UpdateTitleMonitored { id, monitored })
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
        let id = id.to_string();
        db_call!(
            self,
            UpdateTitleMetadata {
                id,
                name,
                facet,
                tags_json
            }
        )
    }

    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        let id = id.to_string();
        db_call!(self, UpdateTitleHydratedMetadata { id, metadata })
    }

    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<scryer_domain::ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title> {
        crate::queries::title::replace_title_match_state_query(&self.pool, id, external_ids, tags)
            .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        db_call!(self, DeleteTitle { id })
    }

    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()> {
        crate::queries::title::set_title_folder_path_query(&self.pool, id, folder_path).await
    }

    async fn list_unhydrated(&self, limit: usize, language: &str) -> AppResult<Vec<Title>> {
        let language = language.to_string();
        db_call!(self, ListUnhydratedTitles { limit, language })
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        db_call!(self, ClearMetadataLanguageForAll {})
    }
}

#[async_trait]
impl ShowRepository for SqliteServices {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        let title_id = title_id.to_string();
        db_call!(self, ListCollectionsForTitle { title_id })
    }

    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        let title_ids = title_ids.to_vec();
        let collections = db_call!(self, ListCollectionsForTitles { title_ids })?;
        let mut grouped = HashMap::<String, Vec<Collection>>::new();
        for collection in collections {
            grouped
                .entry(collection.title_id.clone())
                .or_default()
                .push(collection);
        }
        Ok(grouped)
    }

    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        let title_ids = title_ids.to_vec();
        db_call!(self, ListPrimaryCollectionSummaries { title_ids })
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        let collection_id = collection_id.to_string();
        db_call!(self, GetCollectionById { collection_id })
    }

    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        let ordered_path = ordered_path.to_string();
        db_call!(self, GetCollectionByOrderedPath { ordered_path })
    }

    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        db_call!(self, CreateCollection { collection })
    }

    async fn update_collection(
        &self,
        collection_id: &str,
        collection_type: Option<CollectionType>,
        collection_index: Option<String>,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
        monitored: Option<bool>,
    ) -> AppResult<Collection> {
        let collection_id = collection_id.to_string();
        db_call!(
            self,
            UpdateCollection {
                collection_id,
                collection_type,
                collection_index,
                label,
                ordered_path,
                first_episode_number,
                last_episode_number,
                monitored,
            }
        )
    }

    async fn update_collection_interstitial_movie(
        &self,
        collection_id: &str,
        interstitial_movie: scryer_domain::InterstitialMovieMetadata,
    ) -> AppResult<Collection> {
        let collection_id = collection_id.to_string();
        db_call!(
            self,
            UpdateCollectionInterstitialMovie {
                collection_id,
                interstitial_movie
            }
        )
    }

    async fn update_collection_specials_movies(
        &self,
        collection_id: &str,
        specials_movies: Vec<scryer_domain::InterstitialMovieMetadata>,
    ) -> AppResult<Collection> {
        let collection_id = collection_id.to_string();
        db_call!(
            self,
            UpdateCollectionSpecialsMovies {
                collection_id,
                specials_movies
            }
        )
    }

    async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<String>,
    ) -> AppResult<()> {
        let collection_id = collection_id.to_string();
        db_call!(
            self,
            UpdateInterstitialSeasonEpisode {
                collection_id,
                season_episode
            }
        )
    }

    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        let collection_id = collection_id.to_string();
        db_call!(
            self,
            SetCollectionEpisodesMonitored {
                collection_id,
                monitored
            }
        )
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        let collection_id = collection_id.to_string();
        db_call!(self, DeleteCollection { collection_id })
    }

    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        crate::queries::title::delete_collections_for_title_query(&self.pool, title_id).await
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        let collection_id = collection_id.to_string();
        db_call!(self, ListEpisodesForCollection { collection_id })
    }

    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        let title_id = title_id.to_string();
        db_call!(self, ListEpisodesForTitle { title_id })
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        let episode_id = episode_id.to_string();
        db_call!(self, GetEpisodeById { episode_id })
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        db_call!(self, CreateEpisode { episode })
    }

    async fn update_episode(
        &self,
        episode_id: &str,
        episode_type: Option<scryer_domain::EpisodeType>,
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
        overview: Option<String>,
        tvdb_id: Option<String>,
    ) -> AppResult<Episode> {
        let episode_id = episode_id.to_string();
        db_call!(
            self,
            UpdateEpisode {
                episode_id,
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
                overview,
                tvdb_id,
            }
        )
    }

    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        let episode_id = episode_id.to_string();
        db_call!(self, DeleteEpisode { episode_id })
    }

    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        crate::queries::title::delete_episodes_for_title_query(&self.pool, title_id).await
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
        let start_date = start_date.to_string();
        let end_date = end_date.to_string();
        db_call!(
            self,
            ListEpisodesInDateRange {
                start_date,
                end_date
            }
        )
    }
}

#[async_trait]
impl UserRepository for SqliteServices {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
        let username = username.to_string();
        db_call!(self, GetUserByUsername { username })
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
        let id = id.to_string();
        db_call!(self, GetUserById { id })
    }

    async fn create(&self, user: User) -> AppResult<User> {
        db_call!(self, CreateUser { user })
    }

    async fn list_all(&self) -> AppResult<Vec<User>> {
        db_call!(self, ListUsers {})
    }

    async fn update_entitlements(
        &self,
        id: &str,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User> {
        let entitlements_json = serde_json::to_string(&entitlements)
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let id = id.to_string();
        db_call!(
            self,
            UpdateUserEntitlements {
                id,
                entitlements_json
            }
        )
    }

    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User> {
        let id = id.to_string();
        db_call!(self, UpdateUserPassword { id, password_hash })
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        db_call!(self, DeleteUser { id })
    }
}

#[async_trait]
impl DomainEventRepository for SqliteServices {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        crate::queries::domain_event::append_domain_event_query(&self.pool, &event).await
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        crate::queries::domain_event::append_domain_events_query(&self.pool, &events).await
    }

    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        crate::queries::domain_event::list_domain_events_query(&self.pool, filter).await
    }

    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        crate::queries::domain_event::list_domain_events_after_sequence_query(
            &self.pool,
            after_sequence,
            limit,
        )
        .await
    }

    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
        crate::queries::domain_event::get_event_subscriber_offset_query(&self.pool, subscriber)
            .await
    }

    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
        crate::queries::domain_event::set_event_subscriber_offset_query(
            &self.pool, subscriber, sequence,
        )
        .await
    }
}

#[async_trait]
impl IndexerConfigRepository for SqliteServices {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        db_call!(self, ListIndexerConfigs { provider_type })
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
        let id = id.to_string();
        db_call!(self, GetIndexerConfig { id })
    }

    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        db_call!(self, CreateIndexerConfig { config })
    }

    async fn touch_last_error(&self, provider_type: &str) -> AppResult<()> {
        let provider_type = provider_type.to_string();
        db_call!(self, TouchIndexerLastError { provider_type })
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
        let id = id.to_string();
        db_call!(
            self,
            UpdateIndexerConfig {
                id,
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
            }
        )
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        db_call!(self, DeleteIndexerConfig { id })
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

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        self.upsert_setting_value(
            scope.to_string(),
            key_name.to_string(),
            scope_id,
            value_json,
            source.to_string(),
            updated_by_user_id,
        )
        .await?;
        Ok(())
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

    async fn vacuum_into(&self, dest_path: &str) -> AppResult<()> {
        self.vacuum_into_db(dest_path).await
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
        self.update_download_client_config(id, name, client_type, base_url, config_json, is_enabled)
            .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.delete_download_client_config(id).await
    }

    async fn reorder(&self, ordered_ids: Vec<String>) -> AppResult<()> {
        self.reorder_download_client_configs(ordered_ids).await
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
        self.list_failed_release_download_attempt_signatures(limit)
            .await
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
        self.get_latest_source_password(title_id, source_hint, source_title)
            .await
    }
}

#[async_trait]
impl DownloadSubmissionRepository for SqliteServices {
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        self.record_download_submission(
            submission.title_id,
            submission.facet,
            submission.download_client_type,
            submission.download_client_item_id,
            submission.source_title,
            submission.collection_id,
        )
        .await
    }

    async fn find_by_client_item_id(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        self.find_download_submission(download_client_type, download_client_item_id)
            .await
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        self.list_download_submissions_for_title(title_id).await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.delete_download_submissions_for_title(title_id).await
    }

    async fn delete_by_client_item_id(&self, download_client_item_id: &str) -> AppResult<()> {
        self.delete_download_submission_by_client_item_id(download_client_item_id)
            .await
    }

    async fn update_tracked_state(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
        tracked_state: &str,
    ) -> AppResult<()> {
        self.update_tracked_state(download_client_type, download_client_item_id, tracked_state)
            .await
    }

    async fn get_tracked_state(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<String>> {
        self.get_tracked_state(download_client_type, download_client_item_id)
            .await
    }
}

#[async_trait]
impl ImportArtifactRepository for SqliteServices {
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()> {
        self.insert_import_artifact(artifact).await
    }

    async fn list_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Vec<ImportArtifact>> {
        self.list_import_artifacts_by_source_ref(source_system, source_ref)
            .await
    }

    async fn count_by_result(
        &self,
        source_system: &str,
        source_ref: &str,
        result: &str,
    ) -> AppResult<u64> {
        self.count_import_artifacts_by_result(source_system, source_ref, result)
            .await
    }
}

#[async_trait]
impl JobRunRepository for SqliteServices {
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let record = crate::queries::workflow::create_job_workflow_operation_query(
            &self.pool,
            run.operation_type.clone(),
            run.status.as_str().to_string(),
            run.job_key.as_str().to_string(),
            run.trigger_source.as_str().to_string(),
            run.actor_user_id.clone(),
            run.progress_json.clone(),
            run.summary_json.clone(),
            run.summary_text.clone(),
            run.error_text.clone(),
            Some(run.started_at.to_rfc3339()),
            run.completed_at.map(|value| value.to_rfc3339()),
        )
        .await?;

        job_run_record_from_workflow(record)
    }

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let record = crate::queries::workflow::update_job_workflow_operation_query(
            &self.pool,
            &run.id,
            run.status.as_str(),
            run.progress_json.clone(),
            run.summary_json.clone(),
            run.summary_text.clone(),
            run.error_text.clone(),
            run.completed_at.map(|value| value.to_rfc3339()),
        )
        .await?;

        job_run_record_from_workflow(record)
    }

    async fn get_job_run(&self, run_id: &str) -> AppResult<Option<JobRunRecord>> {
        crate::queries::workflow::get_workflow_operation_by_id_query(&self.pool, run_id)
            .await?
            .map(job_run_record_from_workflow)
            .transpose()
    }

    async fn list_job_runs(
        &self,
        job_key: Option<JobKey>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        crate::queries::workflow::list_job_workflow_operations_query(
            &self.pool,
            job_key.map(JobKey::as_str),
            limit as i64,
        )
        .await?
        .into_iter()
        .map(job_run_record_from_workflow)
        .collect()
    }

    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>> {
        crate::queries::workflow::list_active_job_workflow_operations_query(&self.pool)
            .await?
            .into_iter()
            .map(job_run_record_from_workflow)
            .collect()
    }
}

#[async_trait]
impl LibraryProbeRepository for SqliteServices {
    async fn get_probe_signature(
        &self,
        title_id: &str,
    ) -> AppResult<Option<LibraryProbeSignature>> {
        Ok(
            crate::queries::workflow::get_library_probe_signature_query(&self.pool, title_id)
                .await?
                .map(|record| LibraryProbeSignature {
                    title_id: record.title_id,
                    path: record.path,
                    probe_signature_scheme: record.probe_signature_scheme,
                    probe_signature_value: record.probe_signature_value,
                    last_probed_at: record
                        .last_probed_at
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                        .map(|value| value.with_timezone(&chrono::Utc)),
                    last_changed_at: record
                        .last_changed_at
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                        .map(|value| value.with_timezone(&chrono::Utc)),
                }),
        )
    }

    async fn upsert_probe_signature(&self, probe: &LibraryProbeSignature) -> AppResult<()> {
        crate::queries::workflow::upsert_library_probe_signature_query(
            &self.pool,
            &probe.title_id,
            &probe.path,
            probe.probe_signature_scheme.clone(),
            probe.probe_signature_value.clone(),
            probe.last_probed_at.map(|value| value.to_rfc3339()),
            probe.last_changed_at.map(|value| value.to_rfc3339()),
        )
        .await
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

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>> {
        self.get_import_by_id(id).await
    }

    async fn get_import_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Option<ImportRecord>> {
        self.get_import_by_source_ref(source_system, source_ref)
            .await
    }

    async fn update_import_status(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        self.update_import_status(import_id, status, result_json)
            .await
    }

    async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64> {
        self.recover_stale_processing_imports(stale_seconds).await
    }

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        self.list_pending_imports().await
    }

    async fn is_already_imported(&self, source_system: &str, source_ref: &str) -> AppResult<bool> {
        match self
            .get_import_by_source_ref(source_system, source_ref)
            .await?
        {
            Some(record) => Ok(matches!(
                record.status,
                ImportStatus::Completed | ImportStatus::Skipped
            )),
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
        let scope = scope.to_string();
        db_call!(self, ListQualityProfiles { scope, scope_id })
    }

    async fn replace_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
        profiles: Vec<ApplicationQualityProfile>,
    ) -> AppResult<()> {
        self.replace_quality_profiles(scope.to_string(), scope_id, profiles)
            .await
    }
}

#[async_trait]
impl MediaFileRepository for SqliteServices {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        self.insert_media_file(input).await
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        self.link_file_to_episode(file_id, episode_id).await
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        self.list_media_files_for_title(title_id).await
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        self.list_title_media_size_summaries(title_ids).await
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<scryer_application::TitleEpisodeProgressSummary>> {
        self.list_title_episode_progress_summaries(title_ids).await
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: scryer_application::MediaFileAnalysis,
    ) -> AppResult<()> {
        self.update_media_file_analysis(file_id, analysis).await
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        self.update_media_file_source_signature(
            file_id,
            size_bytes,
            source_signature_scheme,
            source_signature_value,
        )
        .await
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        self.update_media_file_path(file_id, file_path).await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        self.mark_scan_failed(file_id, error).await
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        self.get_media_file_by_id(file_id).await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        self.get_media_file_by_path(file_path).await
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        self.delete_media_file(file_id).await
    }
}

#[async_trait]
impl WantedItemRepository for SqliteServices {
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String> {
        self.upsert_wanted_item(item).await
    }

    async fn ensure_wanted_item_seeded(&self, item: &WantedItem) -> AppResult<String> {
        self.ensure_wanted_item_seeded_atomic(item.clone()).await
    }

    async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
    ) -> AppResult<Vec<WantedItem>> {
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
            id,
            status,
            next_search_at,
            last_search_at,
            search_count,
            current_score,
            grabbed_release,
        )
        .await
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

    async fn delete_wanted_items_for_collection(&self, collection_id: &str) -> AppResult<()> {
        self.delete_wanted_items_for_collection(collection_id).await
    }

    async fn delete_wanted_items_for_episode(&self, episode_id: &str) -> AppResult<()> {
        self.delete_wanted_items_for_episode(episode_id).await
    }

    async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64> {
        self.reset_fruitless_wanted_items(now).await
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
        self.list_wanted_items(status, media_type, title_id, limit, offset)
            .await
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
        self.list_release_decisions_for_wanted_item(wanted_item_id, limit)
            .await
    }
}

#[async_trait]
impl RuleSetRepository for SqliteServices {
    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        db_call!(self, ListRuleSets {})
    }

    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        db_call!(self, ListEnabledRuleSets {})
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>> {
        let id = id.to_string();
        db_call!(self, GetRuleSet { id })
    }

    async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let rule_set = rule_set.clone();
        db_call!(self, CreateRuleSet { rule_set })
    }

    async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let rule_set = rule_set.clone();
        db_call!(self, UpdateRuleSet { rule_set })
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        db_call!(self, DeleteRuleSet { id })
    }

    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()> {
        let id = scryer_domain::Id::new().0;
        let rule_set_id = rule_set_id.to_string();
        let action = action.to_string();
        let rego_source = rego_source.map(|s| s.to_string());
        let actor_id = actor_id.map(|s| s.to_string());
        db_call!(
            self,
            RecordRuleSetHistory {
                id,
                rule_set_id,
                action,
                rego_source,
                actor_id
            }
        )
    }

    async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>> {
        let key = key.to_string();
        db_call!(self, GetRuleSetByManagedKey { key })
    }

    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()> {
        let key = key.to_string();
        db_call!(self, DeleteRuleSetByManagedKey { key })
    }

    async fn list_rule_sets_by_managed_key_prefix(&self, prefix: &str) -> AppResult<Vec<RuleSet>> {
        let prefix = prefix.to_string();
        db_call!(self, ListRuleSetsByManagedKeyPrefix { prefix })
    }
}

#[async_trait]
impl PluginInstallationRepository for SqliteServices {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>> {
        db_call!(self, ListPluginInstallations {})
    }

    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>> {
        let plugin_id = plugin_id.to_string();
        db_call!(self, GetPluginInstallation { plugin_id })
    }

    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let installation = installation.clone();
        let wasm_bytes = wasm_bytes.map(|b| b.to_vec());
        db_call!(
            self,
            CreatePluginInstallation {
                installation,
                wasm_bytes
            }
        )
    }

    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let installation = installation.clone();
        let wasm_bytes = wasm_bytes.map(|b| b.to_vec());
        db_call!(
            self,
            UpdatePluginInstallation {
                installation,
                wasm_bytes
            }
        )
    }

    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()> {
        let plugin_id = plugin_id.to_string();
        db_call!(self, DeletePluginInstallation { plugin_id })
    }

    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<Vec<u8>>)>> {
        db_call!(self, GetEnabledPluginWasmBytes {})
    }

    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        let plugin_id = plugin_id.to_string();
        let name = name.to_string();
        let description = description.to_string();
        let version = version.to_string();
        let provider_type = provider_type.to_string();
        db_call!(
            self,
            SeedBuiltinPlugin {
                plugin_id,
                name,
                description,
                version,
                provider_type
            }
        )
    }

    async fn store_registry_cache(&self, json: &str) -> AppResult<()> {
        let json = json.to_string();
        db_call!(self, StoreRegistryCache { json })
    }

    async fn get_registry_cache(&self) -> AppResult<Option<String>> {
        db_call!(self, GetRegistryCache {})
    }
}

// ── Notification Channels ──────────────────────────────────────────────

#[async_trait]
impl NotificationChannelRepository for SqliteServices {
    async fn list_channels(&self) -> AppResult<Vec<NotificationChannelConfig>> {
        db_call!(self, ListNotificationChannels {})
    }

    async fn get_channel(&self, id: &str) -> AppResult<Option<NotificationChannelConfig>> {
        let id = id.to_string();
        db_call!(self, GetNotificationChannel { id })
    }

    async fn create_channel(
        &self,
        config: NotificationChannelConfig,
    ) -> AppResult<NotificationChannelConfig> {
        db_call!(self, CreateNotificationChannel { config })
    }

    async fn update_channel(
        &self,
        config: NotificationChannelConfig,
    ) -> AppResult<NotificationChannelConfig> {
        db_call!(self, UpdateNotificationChannel { config })
    }

    async fn delete_channel(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        db_call!(self, DeleteNotificationChannel { id })
    }
}

// ── Notification Subscriptions ─────────────────────────────────────────

#[async_trait]
impl NotificationSubscriptionRepository for SqliteServices {
    async fn list_subscriptions(&self) -> AppResult<Vec<NotificationSubscription>> {
        db_call!(self, ListNotificationSubscriptions {})
    }

    async fn list_subscriptions_for_channel(
        &self,
        channel_id: &str,
    ) -> AppResult<Vec<NotificationSubscription>> {
        let channel_id = channel_id.to_string();
        db_call!(self, ListNotificationSubscriptionsForChannel { channel_id })
    }

    async fn list_subscriptions_for_event(
        &self,
        event_type: scryer_domain::NotificationEventType,
    ) -> AppResult<Vec<NotificationSubscription>> {
        crate::queries::notification_subscription::list_notification_subscriptions_for_event_query(
            &self.pool, event_type,
        )
        .await
    }

    async fn create_subscription(
        &self,
        sub: NotificationSubscription,
    ) -> AppResult<NotificationSubscription> {
        db_call!(self, CreateNotificationSubscription { sub })
    }

    async fn update_subscription(
        &self,
        sub: NotificationSubscription,
    ) -> AppResult<NotificationSubscription> {
        db_call!(self, UpdateNotificationSubscription { sub })
    }

    async fn delete_subscription(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        db_call!(self, DeleteNotificationSubscription { id })
    }
}

#[async_trait]
impl HousekeepingRepository for SqliteServices {
    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        db_call!(self, DeleteReleaseDecisionsOlderThan { days })
    }

    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        db_call!(self, DeleteReleaseAttemptsOlderThan { days })
    }

    async fn delete_dispatched_event_outboxes_older_than(&self, days: i64) -> AppResult<u32> {
        db_call!(self, DeleteDispatchedEventOutboxesOlderThan { days })
    }

    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        db_call!(self, DeleteHistoryEventsOlderThan { days })
    }

    async fn delete_domain_events_older_than(&self, days: i64) -> AppResult<u32> {
        db_call!(self, DeleteDomainEventsOlderThan { days })
    }

    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        db_call!(self, ListAllMediaFilePaths {})
    }

    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32> {
        if ids.is_empty() {
            return Ok(0);
        }
        let ids = ids.to_vec();
        db_call!(self, DeleteMediaFilesByIds { ids })
    }
}

#[async_trait]
impl PendingReleaseRepository for SqliteServices {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
        self.insert_pending_release(release).await
    }

    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>> {
        self.list_expired_pending_releases(now).await
    }

    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        self.list_waiting_pending_releases().await
    }

    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
        self.get_pending_release(id).await
    }

    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        self.list_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn update_pending_release_status(
        &self,
        id: &str,
        status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        self.update_pending_release_status(id, status, grabbed_at)
            .await
    }

    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        self.list_standby_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        self.delete_standby_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        self.list_all_standby_pending_releases().await
    }

    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        self.compare_and_set_pending_release_status(id, current_status, next_status, grabbed_at)
            .await
    }

    async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        self.supersede_pending_releases_for_wanted_item(wanted_item_id, except_id)
            .await
    }

    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        self.delete_pending_releases_for_title(title_id).await
    }
}

#[async_trait]
impl TitleHistoryRepository for SqliteServices {
    async fn record_event(&self, event: &NewTitleHistoryEvent) -> AppResult<String> {
        let data_json = if event.data.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&event.data)
                    .map_err(|e| AppError::Repository(e.to_string()))?,
            )
        };
        self.insert_title_history_event(
            event.title_id.clone(),
            event.episode_id.clone(),
            event.collection_id.clone(),
            event.event_type.as_str().to_string(),
            event.source_title.clone(),
            event.quality.clone(),
            event.download_id.clone(),
            data_json,
        )
        .await
    }

    async fn list_history(&self, filter: &TitleHistoryFilter) -> AppResult<TitleHistoryPage> {
        let event_types = filter.event_types.as_ref().map(|types| {
            types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect::<Vec<_>>()
        });
        let (records, total_count) = self
            .list_title_history(
                event_types,
                filter.title_ids.clone(),
                filter.download_id.clone(),
                filter.limit,
                filter.offset,
            )
            .await?;
        Ok(TitleHistoryPage {
            records,
            total_count,
        })
    }

    async fn list_for_title(
        &self,
        title_id: &str,
        event_types: Option<&[TitleHistoryEventType]>,
        limit: usize,
        offset: usize,
    ) -> AppResult<TitleHistoryPage> {
        let type_strings = event_types.map(|types| {
            types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect::<Vec<_>>()
        });
        let (records, total_count) = self
            .list_title_history_for_title(title_id, type_strings, limit, offset)
            .await?;
        Ok(TitleHistoryPage {
            records,
            total_count,
        })
    }

    async fn list_for_episode(
        &self,
        episode_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleHistoryRecord>> {
        self.list_title_history_for_episode(episode_id, limit).await
    }

    async fn find_by_download_id(&self, download_id: &str) -> AppResult<Vec<TitleHistoryRecord>> {
        self.find_title_history_by_download_id(download_id).await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.delete_title_history_for_title(title_id).await
    }
}

#[async_trait]
impl BlocklistRepository for SqliteServices {
    async fn add(&self, entry: &NewBlocklistEntry) -> AppResult<String> {
        let data_json = if entry.data.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&entry.data)
                    .map_err(|e| AppError::Repository(e.to_string()))?,
            )
        };
        self.insert_blocklist_entry(
            entry.title_id.clone(),
            entry.source_title.clone(),
            entry.source_hint.clone(),
            entry.quality.clone(),
            entry.download_id.clone(),
            entry.reason.clone(),
            data_json,
        )
        .await
    }

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>> {
        self.list_blocklist_for_title(title_id, limit).await
    }

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        self.list_blocklist_all(limit, offset).await
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        self.delete_blocklist_entry(id).await
    }

    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        self.is_blocklisted(title_id, source_title).await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.delete_blocklist_for_title(title_id).await
    }
}

#[async_trait]
impl PostProcessingScriptRepository for SqliteServices {
    async fn list_scripts(&self) -> AppResult<Vec<PostProcessingScript>> {
        db_call!(self, ListPPScripts {})
    }

    async fn get_script(&self, id: &str) -> AppResult<Option<PostProcessingScript>> {
        let id = id.to_string();
        db_call!(self, GetPPScript { id })
    }

    async fn create_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript> {
        db_call!(self, CreatePPScript { script })
    }

    async fn update_script(&self, script: PostProcessingScript) -> AppResult<PostProcessingScript> {
        db_call!(self, UpdatePPScript { script })
    }

    async fn delete_script(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        db_call!(self, DeletePPScript { id })
    }

    async fn list_enabled_for_facet(&self, facet: &str) -> AppResult<Vec<PostProcessingScript>> {
        let facet = facet.to_string();
        db_call!(self, ListEnabledPPScriptsForFacet { facet })
    }

    async fn record_run(&self, run: PostProcessingScriptRun) -> AppResult<()> {
        db_call!(self, RecordPPScriptRun { run })
    }

    async fn list_runs_for_script(
        &self,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>> {
        let script_id = script_id.to_string();
        db_call!(self, ListPPScriptRunsForScript { script_id, limit })
    }

    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<PostProcessingScriptRun>> {
        let title_id = title_id.to_string();
        db_call!(self, ListPPScriptRunsForTitle { title_id, limit })
    }
}

// ── SubtitleDownloadRepository ──────────────────────────────────────────────

#[async_trait]
impl scryer_application::SubtitleDownloadRepository for SqliteServices {
    async fn list_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        crate::queries::subtitle::list_subtitle_downloads_for_title(&self.pool, title_id).await
    }

    async fn get(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        crate::queries::subtitle::get_subtitle_download(&self.pool, id).await
    }

    async fn list_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        crate::queries::subtitle::list_subtitle_downloads_for_media_file(&self.pool, media_file_id)
            .await
    }

    async fn insert(&self, download: &scryer_domain::SubtitleDownload) -> AppResult<()> {
        crate::queries::subtitle::insert_subtitle_download(&self.pool, download).await
    }

    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        crate::queries::subtitle::update_subtitle_download_synced(&self.pool, id, synced).await
    }

    async fn delete(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        crate::queries::subtitle::delete_subtitle_download(&self.pool, id).await
    }

    async fn is_blacklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool> {
        crate::queries::subtitle::is_blacklisted(
            &self.pool,
            media_file_id,
            provider,
            provider_file_id,
        )
        .await
    }

    async fn blacklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        crate::queries::subtitle::insert_blacklist_entry(
            &self.pool,
            media_file_id,
            provider,
            provider_file_id,
            language,
            reason,
        )
        .await?;
        Ok(())
    }
}
