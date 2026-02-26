use scryer_application::{AppResult, PrimaryCollectionSummary, ReleaseDecision, ReleaseDownloadAttemptOutcome, TitleMetadataUpdate, WantedItem};
use scryer_domain::{Collection, DownloadClientConfig, Episode, HistoryEvent, ImportRecord, IndexerConfig, MediaFacet, Title, User};
use scryer_application::QualityProfile;
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use crate::types::{
    MigrationStatus, ReleaseDownloadFailureSignatureRecord, SettingDefinitionSeed,
    SettingsDefinitionRecord, SettingsValueRecord, TitleReleaseBlocklistRecord,
    WorkflowOperationRecord,
};
use crate::{
    migrations,
    queries::{
        download_client::*, event::*, indexer::*, quality::*, settings::*,
        title::*, user::*, workflow::*,
    },
};

use tokio::sync::oneshot::Sender;

use crate::encryption::EncryptionKey;

pub(crate) enum DbCommand {
    SetEncryptionKey {
        key: EncryptionKey,
        reply: Sender<AppResult<()>>,
    },
    ListTitles {
        facet: Option<MediaFacet>,
        query: Option<String>,
        reply: Sender<AppResult<Vec<Title>>>,
    },
    GetTitleById {
        id: String,
        reply: Sender<AppResult<Option<Title>>>,
    },
    CreateTitle {
        title: Title,
        reply: Sender<AppResult<Title>>,
    },
    UpdateTitleMonitored {
        id: String,
        monitored: bool,
        reply: Sender<AppResult<Title>>,
    },
    UpdateTitleMetadata {
        id: String,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags_json: Option<String>,
        reply: Sender<AppResult<Title>>,
    },
    UpdateTitleHydratedMetadata {
        id: String,
        metadata: TitleMetadataUpdate,
        reply: Sender<AppResult<Title>>,
    },
    ListCollectionsForTitle {
        title_id: String,
        reply: Sender<AppResult<Vec<Collection>>>,
    },
    ListPrimaryCollectionSummaries {
        title_ids: Vec<String>,
        reply: Sender<AppResult<Vec<PrimaryCollectionSummary>>>,
    },
    GetCollectionById {
        collection_id: String,
        reply: Sender<AppResult<Option<Collection>>>,
    },
    CreateCollection {
        collection: Collection,
        reply: Sender<AppResult<Collection>>,
    },
    UpdateCollection {
        collection_id: String,
        collection_type: Option<String>,
        collection_index: Option<String>,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
        monitored: Option<bool>,
        reply: Sender<AppResult<Collection>>,
    },
    SetCollectionEpisodesMonitored {
        collection_id: String,
        monitored: bool,
        reply: Sender<AppResult<()>>,
    },
    ListEpisodesForCollection {
        collection_id: String,
        reply: Sender<AppResult<Vec<Episode>>>,
    },
    GetEpisodeById {
        episode_id: String,
        reply: Sender<AppResult<Option<Episode>>>,
    },
    CreateEpisode {
        episode: Episode,
        reply: Sender<AppResult<Episode>>,
    },
    UpdateEpisode {
        episode_id: String,
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
        reply: Sender<AppResult<Episode>>,
    },
    DeleteCollection {
        collection_id: String,
        reply: Sender<AppResult<()>>,
    },
    DeleteEpisode {
        episode_id: String,
        reply: Sender<AppResult<()>>,
    },
    DeleteTitle {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    ListEvents {
        title_id: Option<String>,
        limit: i64,
        offset: i64,
        reply: Sender<AppResult<Vec<HistoryEvent>>>,
    },
    AppendEvent {
        event: HistoryEvent,
        reply: Sender<AppResult<()>>,
    },
    ListUsers {
        reply: Sender<AppResult<Vec<User>>>,
    },
    GetUserById {
        id: String,
        reply: Sender<AppResult<Option<User>>>,
    },
    GetUserByUsername {
        username: String,
        reply: Sender<AppResult<Option<User>>>,
    },
    CreateUser {
        user: User,
        reply: Sender<AppResult<User>>,
    },
    UpdateUserEntitlements {
        id: String,
        entitlements_json: String,
        reply: Sender<AppResult<User>>,
    },
    UpdateUserPassword {
        id: String,
        password_hash: String,
        reply: Sender<AppResult<User>>,
    },
    DeleteUser {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    ListIndexerConfigs {
        provider_type: Option<String>,
        reply: Sender<AppResult<Vec<IndexerConfig>>>,
    },
    GetIndexerConfig {
        id: String,
        reply: Sender<AppResult<Option<IndexerConfig>>>,
    },
    CreateIndexerConfig {
        config: IndexerConfig,
        reply: Sender<AppResult<IndexerConfig>>,
    },
    UpdateIndexerConfig {
        id: String,
        name: Option<String>,
        provider_type: Option<String>,
        base_url: Option<String>,
        api_key_encrypted: Option<String>,
        rate_limit_seconds: Option<i64>,
        rate_limit_burst: Option<i64>,
        is_enabled: Option<bool>,
        enable_interactive_search: Option<bool>,
        enable_auto_search: Option<bool>,
        reply: Sender<AppResult<IndexerConfig>>,
    },
    TouchIndexerLastError {
        provider_type: String,
        reply: Sender<AppResult<()>>,
    },
    DeleteIndexerConfig {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    ListDownloadClientConfigs {
        client_type: Option<String>,
        reply: Sender<AppResult<Vec<DownloadClientConfig>>>,
    },
    GetDownloadClientConfig {
        id: String,
        reply: Sender<AppResult<Option<DownloadClientConfig>>>,
    },
    CreateDownloadClientConfig {
        config: DownloadClientConfig,
        reply: Sender<AppResult<DownloadClientConfig>>,
    },
    UpdateDownloadClientConfig {
        id: String,
        name: Option<String>,
        client_type: Option<String>,
        base_url: Option<String>,
        config_json: Option<String>,
        is_enabled: Option<bool>,
        reply: Sender<AppResult<DownloadClientConfig>>,
    },
    DeleteDownloadClientConfig {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    EnsureSettingDefinition {
        category: String,
        scope: String,
        key_name: String,
        data_type: String,
        default_value_json: String,
        is_sensitive: bool,
        validation_json: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    BatchEnsureSettingDefinitions {
        definitions: Vec<SettingDefinitionSeed>,
        reply: Sender<AppResult<()>>,
    },
    BatchGetSettingsWithDefaults {
        keys: Vec<(String, String, Option<String>)>,
        reply: Sender<AppResult<Vec<Option<SettingsValueRecord>>>>,
    },
    BatchUpsertSettingsIfNotOverridden {
        /// Vec of (scope, key_name, value_json, source)
        entries: Vec<(String, String, String, String)>,
        reply: Sender<AppResult<()>>,
    },
    ListSettingDefinitions {
        scope: Option<String>,
        reply: Sender<AppResult<Vec<SettingsDefinitionRecord>>>,
    },
    ListSettingsWithValues {
        scope: String,
        scope_id: Option<String>,
        reply: Sender<AppResult<Vec<SettingsValueRecord>>>,
    },
    GetSettingWithDefaults {
        scope: String,
        key_name: String,
        scope_id: Option<String>,
        reply: Sender<AppResult<Option<SettingsValueRecord>>>,
    },
    UpsertSettingValue {
        scope: String,
        key_name: String,
        scope_id: Option<String>,
        value_json: String,
        source: String,
        updated_by_user_id: Option<String>,
        reply: Sender<AppResult<SettingsValueRecord>>,
    },
    ListQualityProfiles {
        scope: String,
        scope_id: Option<String>,
        reply: Sender<AppResult<Vec<QualityProfile>>>,
    },
    ReplaceQualityProfiles {
        scope: String,
        scope_id: Option<String>,
        profiles_json: Vec<QualityProfile>,
        reply: Sender<AppResult<()>>,
    },
    UpsertQualityProfiles {
        scope: String,
        scope_id: Option<String>,
        profiles_json: Vec<QualityProfile>,
        reply: Sender<AppResult<()>>,
    },
    ListAppliedMigrations {
        reply: Sender<AppResult<Vec<MigrationStatus>>>,
    },
    CreateWorkflowOperation {
        operation_type: String,
        status: String,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
        reply: Sender<AppResult<WorkflowOperationRecord>>,
    },
    CreateImportRequest {
        source_system: String,
        source_ref: String,
        import_type: String,
        payload_json: String,
        reply: Sender<AppResult<String>>,
    },
    CreateReleaseDownloadAttempt {
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    ListFailedReleaseDownloadAttempts {
        limit: i64,
        reply: Sender<AppResult<Vec<ReleaseDownloadFailureSignatureRecord>>>,
    },
    ListFailedReleaseDownloadAttemptsForTitle {
        title_id: String,
        limit: i64,
        reply: Sender<AppResult<Vec<TitleReleaseBlocklistRecord>>>,
    },
    GetLatestSourcePassword {
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        reply: Sender<AppResult<Option<String>>>,
    },
    GetImportBySourceRef {
        source_system: String,
        source_ref: String,
        reply: Sender<AppResult<Option<ImportRecord>>>,
    },
    UpdateImportStatus {
        import_id: String,
        status: String,
        result_json: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    RecoverStaleProcessingImports {
        stale_seconds: i64,
        reply: Sender<AppResult<u64>>,
    },
    ListPendingImports {
        reply: Sender<AppResult<Vec<ImportRecord>>>,
    },
    ListImports {
        limit: i64,
        reply: Sender<AppResult<Vec<ImportRecord>>>,
    },
    InsertMediaFile {
        title_id: String,
        file_path: String,
        size_bytes: i64,
        quality_label: Option<String>,
        reply: Sender<AppResult<String>>,
    },
    LinkFileToEpisode {
        file_id: String,
        episode_id: String,
        reply: Sender<AppResult<()>>,
    },
    ListMediaFilesForTitle {
        title_id: String,
        reply: Sender<AppResult<Vec<scryer_application::TitleMediaFile>>>,
    },
    FindEpisodeByTitleAndNumbers {
        title_id: String,
        season_number: String,
        episode_number: String,
        reply: Sender<AppResult<Option<scryer_domain::Episode>>>,
    },
    FindEpisodeByTitleAndAbsoluteNumber {
        title_id: String,
        absolute_number: String,
        reply: Sender<AppResult<Option<scryer_domain::Episode>>>,
    },
    UpsertWantedItem {
        item: WantedItem,
        reply: Sender<AppResult<String>>,
    },
    ListDueWantedItems {
        now: String,
        batch_limit: i64,
        reply: Sender<AppResult<Vec<WantedItem>>>,
    },
    UpdateWantedItemStatus {
        id: String,
        status: String,
        next_search_at: Option<String>,
        last_search_at: Option<String>,
        search_count: i64,
        current_score: Option<i32>,
        grabbed_release: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    GetWantedItemForTitle {
        title_id: String,
        episode_id: Option<String>,
        reply: Sender<AppResult<Option<WantedItem>>>,
    },
    DeleteWantedItemsForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    InsertReleaseDecision {
        decision: ReleaseDecision,
        reply: Sender<AppResult<String>>,
    },
    GetWantedItemById {
        id: String,
        reply: Sender<AppResult<Option<WantedItem>>>,
    },
    ListWantedItems {
        status: Option<String>,
        media_type: Option<String>,
        title_id: Option<String>,
        limit: i64,
        offset: i64,
        reply: Sender<AppResult<Vec<WantedItem>>>,
    },
    CountWantedItems {
        status: Option<String>,
        media_type: Option<String>,
        title_id: Option<String>,
        reply: Sender<AppResult<i64>>,
    },
    ListReleaseDecisionsForTitle {
        title_id: String,
        limit: i64,
        reply: Sender<AppResult<Vec<ReleaseDecision>>>,
    },
    ListReleaseDecisionsForWantedItem {
        wanted_item_id: String,
        limit: i64,
        reply: Sender<AppResult<Vec<ReleaseDecision>>>,
    },
}

pub(crate) fn spawn_db_command_worker(pool: SqlitePool) -> mpsc::Sender<DbCommand> {
    let (sender, mut receiver) = mpsc::channel(64);
    tokio::spawn(async move {
        let mut encryption_key: Option<EncryptionKey> = None;
        while let Some(command) = receiver.recv().await {
            match command {
                DbCommand::SetEncryptionKey { key, reply } => {
                    encryption_key = Some(key);
                    let _ = reply.send(Ok(()));
                }
                DbCommand::ListTitles {
                    facet,
                    query,
                    reply,
                } => {
                    let _ = reply.send(list_titles_query(&pool, facet, query).await);
                }
                DbCommand::GetTitleById { id, reply } => {
                    let _ = reply.send(get_title_by_id_query(&pool, &id).await);
                }
                DbCommand::CreateTitle { title, reply } => {
                    let _ = reply.send(create_title_query(&pool, &title).await);
                }
                DbCommand::UpdateTitleMonitored {
                    id,
                    monitored,
                    reply,
                } => {
                    let _ =
                        reply.send(update_title_monitored_query(&pool, &id, monitored).await);
                }
                DbCommand::UpdateTitleMetadata {
                    id,
                    name,
                    facet,
                    tags_json,
                    reply,
                } => {
                    let _ = reply.send(
                        update_title_metadata_query(&pool, &id, name, facet, tags_json).await,
                    );
                }
                DbCommand::UpdateTitleHydratedMetadata {
                    id,
                    metadata,
                    reply,
                } => {
                    let _ = reply.send(
                        update_title_hydrated_metadata_query(&pool, &id, metadata).await,
                    );
                }
                DbCommand::ListCollectionsForTitle { title_id, reply } => {
                    let _ =
                        reply.send(list_collections_for_title_query(&pool, &title_id).await);
                }
                DbCommand::ListPrimaryCollectionSummaries { title_ids, reply } => {
                    let _ =
                        reply.send(list_primary_collection_summaries_query(&pool, &title_ids).await);
                }
                DbCommand::GetCollectionById {
                    collection_id,
                    reply,
                } => {
                    let _ =
                        reply.send(get_collection_by_id_query(&pool, &collection_id).await);
                }
                DbCommand::CreateCollection { collection, reply } => {
                    let _ = reply.send(create_collection_query(&pool, &collection).await);
                }
                DbCommand::UpdateCollection {
                    collection_id,
                    collection_type,
                    collection_index,
                    label,
                    ordered_path,
                    first_episode_number,
                    last_episode_number,
                    monitored,
                    reply,
                } => {
                    let _ = reply.send(
                        update_collection_query(
                            &pool,
                            &collection_id,
                            collection_type,
                            collection_index,
                            label,
                            ordered_path,
                            first_episode_number,
                            last_episode_number,
                            monitored,
                        )
                        .await,
                    );
                }
                DbCommand::SetCollectionEpisodesMonitored {
                    collection_id,
                    monitored,
                    reply,
                } => {
                    let _ = reply.send(
                        set_collection_episodes_monitored_query(&pool, &collection_id, monitored)
                            .await,
                    );
                }
                DbCommand::ListEpisodesForCollection {
                    collection_id,
                    reply,
                } => {
                    let _ = reply
                        .send(list_episodes_for_collection_query(&pool, &collection_id).await);
                }
                DbCommand::GetEpisodeById { episode_id, reply } => {
                    let _ = reply.send(get_episode_by_id_query(&pool, &episode_id).await);
                }
                DbCommand::CreateEpisode { episode, reply } => {
                    let _ = reply.send(create_episode_query(&pool, &episode).await);
                }
                DbCommand::UpdateEpisode {
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
                    reply,
                } => {
                    let _ = reply.send(
                        update_episode_query(
                            &pool,
                            &episode_id,
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
                        )
                        .await,
                    );
                }
                DbCommand::DeleteCollection {
                    collection_id,
                    reply,
                } => {
                    let _ = reply.send(delete_collection_query(&pool, &collection_id).await);
                }
                DbCommand::DeleteEpisode { episode_id, reply } => {
                    let _ = reply.send(delete_episode_query(&pool, &episode_id).await);
                }
                DbCommand::DeleteTitle { id, reply } => {
                    let _ = reply.send(delete_title_query(&pool, &id).await);
                }
                DbCommand::ListEvents {
                    title_id,
                    limit,
                    offset,
                    reply,
                } => {
                    let _ = reply.send(list_events_query(&pool, title_id, limit, offset).await);
                }
                DbCommand::AppendEvent { event, reply } => {
                    let _ = reply.send(append_event_query(&pool, &event).await);
                }
                DbCommand::ListUsers { reply } => {
                    let _ = reply.send(list_users_query(&pool).await);
                }
                DbCommand::GetUserById { id, reply } => {
                    let _ = reply.send(get_user_by_id_query(&pool, &id).await);
                }
                DbCommand::GetUserByUsername { username, reply } => {
                    let _ = reply.send(get_user_by_username_query(&pool, &username).await);
                }
                DbCommand::CreateUser { user, reply } => {
                    let _ = reply.send(create_user_query(&pool, &user).await);
                }
                DbCommand::UpdateUserEntitlements {
                    id,
                    entitlements_json,
                    reply,
                } => {
                    let _ = reply
                        .send(update_user_entitlements_query(
                            &pool,
                            &id,
                            &entitlements_json,
                        )
                        .await);
                }
                DbCommand::UpdateUserPassword {
                    id,
                    password_hash,
                    reply,
                } => {
                    let _ = reply.send(
                        update_user_password_query(&pool, &id, &password_hash).await,
                    );
                }
                DbCommand::DeleteUser { id, reply } => {
                    let _ = reply.send(delete_user_query(&pool, &id).await);
                }
                DbCommand::ListIndexerConfigs {
                    provider_type,
                    reply,
                } => {
                    let _ =
                        reply.send(list_indexer_configs_query(&pool, provider_type, encryption_key.as_ref()).await);
                }
                DbCommand::GetIndexerConfig { id, reply } => {
                    let _ = reply.send(get_indexer_config_query(&pool, &id, encryption_key.as_ref()).await);
                }
                DbCommand::CreateIndexerConfig { config, reply } => {
                    let _ = reply.send(create_indexer_config_query(&pool, &config, encryption_key.as_ref()).await);
                }
                DbCommand::UpdateIndexerConfig {
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
                    reply,
                } => {
                    let _ = reply.send(
                        update_indexer_config_query(
                            &pool,
                            &id,
                            name,
                            provider_type,
                            base_url,
                            api_key_encrypted,
                            rate_limit_seconds,
                            rate_limit_burst,
                            is_enabled,
                            enable_interactive_search,
                            enable_auto_search,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
                }
                DbCommand::TouchIndexerLastError {
                    provider_type,
                    reply,
                } => {
                    let _ = reply.send(
                        touch_indexer_last_error_query(&pool, &provider_type).await,
                    );
                }
                DbCommand::DeleteIndexerConfig { id, reply } => {
                    let _ = reply.send(delete_indexer_config_query(&pool, &id).await);
                }
                DbCommand::ListDownloadClientConfigs { client_type, reply } => {
                    let _ = reply
                        .send(list_download_client_configs_query(&pool, client_type, encryption_key.as_ref()).await);
                }
                DbCommand::GetDownloadClientConfig { id, reply } => {
                    let _ = reply.send(get_download_client_config_query(&pool, &id, encryption_key.as_ref()).await);
                }
                DbCommand::CreateDownloadClientConfig { config, reply } => {
                    let _ = reply.send(create_download_client_config_query(&pool, &config, encryption_key.as_ref()).await);
                }
                DbCommand::UpdateDownloadClientConfig {
                    id,
                    name,
                    client_type,
                    base_url,
                    config_json,
                    is_enabled,
                    reply,
                } => {
                    let _ = reply
                        .send(
                            update_download_client_config_query(
                                &pool,
                                &id,
                                name,
                                client_type,
                                base_url,
                                config_json,
                                is_enabled,
                                encryption_key.as_ref(),
                            )
                            .await,
                        );
                }
                DbCommand::DeleteDownloadClientConfig { id, reply } => {
                    let _ = reply.send(delete_download_client_config_query(&pool, &id).await);
                }
                DbCommand::EnsureSettingDefinition {
                    category,
                    scope,
                    key_name,
                    data_type,
                    default_value_json,
                    is_sensitive,
                    validation_json,
                    reply,
                } => {
                    let _ = reply
                        .send(
                            ensure_setting_definition_query(
                                &pool,
                                &category,
                                &scope,
                                &key_name,
                                &data_type,
                                &default_value_json,
                                is_sensitive,
                                validation_json,
                            )
                            .await,
                        );
                }
                DbCommand::BatchEnsureSettingDefinitions { definitions, reply } => {
                    let _ = reply.send(
                        batch_ensure_setting_definitions_query(&pool, &definitions).await,
                    );
                }
                DbCommand::BatchGetSettingsWithDefaults { keys, reply } => {
                    let _ = reply.send(
                        batch_get_settings_with_defaults_query(&pool, &keys, encryption_key.as_ref()).await,
                    );
                }
                DbCommand::BatchUpsertSettingsIfNotOverridden { entries, reply } => {
                    let _ = reply.send(
                        batch_upsert_settings_if_not_overridden_query(&pool, &entries, encryption_key.as_ref()).await,
                    );
                }
                DbCommand::ListSettingDefinitions { scope, reply } => {
                    let _ = reply
                        .send(list_setting_definitions_query(&pool, scope).await);
                }
                DbCommand::ListSettingsWithValues {
                    scope,
                    scope_id,
                    reply,
                } => {
                    let _ = reply
                        .send(list_settings_with_defaults_query(&pool, &scope, scope_id, encryption_key.as_ref()).await);
                }
                DbCommand::GetSettingWithDefaults {
                    scope,
                    key_name,
                    scope_id,
                    reply,
                } => {
                    let _ = reply.send(
                        get_setting_with_defaults_query(&pool, &scope, &key_name, scope_id, encryption_key.as_ref()).await,
                    );
                }
                DbCommand::UpsertSettingValue {
                    scope,
                    key_name,
                    scope_id,
                    value_json,
                    source,
                    updated_by_user_id,
                    reply,
                } => {
                    let _ = reply.send(
                        upsert_setting_value_query(
                            &pool,
                            &scope,
                            &key_name,
                            scope_id,
                            &value_json,
                            &source,
                            updated_by_user_id,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListQualityProfiles {
                    scope,
                    scope_id,
                    reply,
                } => {
                    let _ = reply.send(list_quality_profiles_query(&pool, &scope, scope_id).await);
                }
                DbCommand::ReplaceQualityProfiles {
                    scope,
                    scope_id,
                    profiles_json,
                    reply,
                } => {
                    let _ = reply.send(
                        replace_quality_profiles_query(&pool, &scope, scope_id, profiles_json).await,
                    );
                }
                DbCommand::UpsertQualityProfiles {
                    scope,
                    scope_id,
                    profiles_json,
                    reply,
                } => {
                    let _ = reply.send(
                        upsert_quality_profiles_query(&pool, &scope, scope_id, profiles_json).await,
                    );
                }
                DbCommand::ListAppliedMigrations { reply } => {
                    let _ = reply.send(migrations::list_applied_migrations(&pool).await);
                }
                DbCommand::CreateWorkflowOperation {
                    operation_type,
                    status,
                    actor_user_id,
                    progress_json,
                    started_at,
                    completed_at,
                    reply,
                } => {
                    let _ = reply.send(
                        create_workflow_operation_query(
                            &pool,
                            operation_type,
                            status,
                            actor_user_id,
                            progress_json,
                            started_at,
                            completed_at,
                        )
                        .await,
                    );
                }
                DbCommand::CreateReleaseDownloadAttempt {
                    title_id,
                    source_hint,
                    source_title,
                    outcome,
                    error_message,
                    source_password,
                    reply,
                } => {
                    let _ = reply.send(
                        create_release_download_attempt_query(
                            &pool,
                            title_id,
                            source_hint,
                            source_title,
                            outcome,
                            error_message,
                            source_password,
                        )
                        .await,
                    );
                }
                DbCommand::GetLatestSourcePassword {
                    title_id,
                    source_hint,
                    source_title,
                    reply,
                } => {
                    let _ = reply.send(
                        get_latest_source_password_query(
                            &pool,
                            title_id.as_deref(),
                            source_hint.as_deref(),
                            source_title.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::CreateImportRequest {
                    source_system,
                    source_ref,
                    import_type,
                    payload_json,
                    reply,
                } => {
                    let _ = reply.send(
                        create_import_request_query(
                            &pool,
                            source_system,
                            source_ref,
                            import_type,
                            payload_json,
                        )
                        .await,
                    );
                }
                DbCommand::ListFailedReleaseDownloadAttempts { limit, reply } => {
                    let _ = reply.send(
                        list_failed_release_download_attempts_query(&pool, limit).await,
                    );
                }
                DbCommand::ListFailedReleaseDownloadAttemptsForTitle {
                    title_id,
                    limit,
                    reply,
                } => {
                    let _ = reply.send(
                        list_failed_release_download_attempts_for_title_query(
                            &pool,
                            &title_id,
                            limit,
                        )
                        .await,
                    );
                }
                DbCommand::GetImportBySourceRef {
                    source_system,
                    source_ref,
                    reply,
                } => {
                    let _ = reply.send(
                        get_import_by_source_ref_query(&pool, &source_system, &source_ref).await,
                    );
                }
                DbCommand::UpdateImportStatus {
                    import_id,
                    status,
                    result_json,
                    reply,
                } => {
                    let _ = reply.send(
                        update_import_status_query(&pool, &import_id, &status, result_json).await,
                    );
                }
                DbCommand::RecoverStaleProcessingImports {
                    stale_seconds,
                    reply,
                } => {
                    let _ = reply.send(
                        recover_stale_processing_imports_query(&pool, stale_seconds).await,
                    );
                }
                DbCommand::ListPendingImports { reply } => {
                    let _ = reply.send(list_pending_imports_query(&pool).await);
                }
                DbCommand::ListImports { limit, reply } => {
                    let _ = reply.send(list_imports_query(&pool, limit).await);
                }
                DbCommand::InsertMediaFile {
                    title_id,
                    file_path,
                    size_bytes,
                    quality_label,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::media_file::insert_media_file_query(
                            &pool, &title_id, &file_path, size_bytes, quality_label,
                        )
                        .await,
                    );
                }
                DbCommand::LinkFileToEpisode {
                    file_id,
                    episode_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::media_file::link_file_to_episode_query(
                            &pool, &file_id, &episode_id,
                        )
                        .await,
                    );
                }
                DbCommand::ListMediaFilesForTitle { title_id, reply } => {
                    let _ = reply.send(
                        crate::queries::media_file::list_media_files_for_title_query(
                            &pool, &title_id,
                        )
                        .await,
                    );
                }
                DbCommand::FindEpisodeByTitleAndNumbers {
                    title_id,
                    season_number,
                    episode_number,
                    reply,
                } => {
                    let _ = reply.send(
                        find_episode_by_title_and_numbers_query(
                            &pool, &title_id, &season_number, &episode_number,
                        )
                        .await,
                    );
                }
                DbCommand::FindEpisodeByTitleAndAbsoluteNumber {
                    title_id,
                    absolute_number,
                    reply,
                } => {
                    let _ = reply.send(
                        find_episode_by_title_and_absolute_number_query(
                            &pool, &title_id, &absolute_number,
                        )
                        .await,
                    );
                }
                DbCommand::UpsertWantedItem { item, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::upsert_wanted_item_query(&pool, &item).await,
                    );
                }
                DbCommand::ListDueWantedItems { now, batch_limit, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_due_wanted_items_query(&pool, &now, batch_limit).await,
                    );
                }
                DbCommand::UpdateWantedItemStatus {
                    id, status, next_search_at, last_search_at,
                    search_count, current_score, grabbed_release, reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::update_wanted_item_status_query(
                            &pool, &id, &status,
                            next_search_at.as_deref(), last_search_at.as_deref(),
                            search_count, current_score, grabbed_release.as_deref(),
                        ).await,
                    );
                }
                DbCommand::GetWantedItemForTitle { title_id, episode_id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::get_wanted_item_for_title_query(
                            &pool, &title_id, episode_id.as_deref(),
                        ).await,
                    );
                }
                DbCommand::DeleteWantedItemsForTitle { title_id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::delete_wanted_items_for_title_query(&pool, &title_id).await,
                    );
                }
                DbCommand::InsertReleaseDecision { decision, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::insert_release_decision_query(&pool, &decision).await,
                    );
                }
                DbCommand::GetWantedItemById { id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::get_wanted_item_by_id_query(&pool, &id).await,
                    );
                }
                DbCommand::ListWantedItems { status, media_type, title_id, limit, offset, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_wanted_items_query(
                            &pool, status.as_deref(), media_type.as_deref(),
                            title_id.as_deref(), limit, offset,
                        ).await,
                    );
                }
                DbCommand::CountWantedItems { status, media_type, title_id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::count_wanted_items_query(
                            &pool, status.as_deref(), media_type.as_deref(),
                            title_id.as_deref(),
                        ).await,
                    );
                }
                DbCommand::ListReleaseDecisionsForTitle { title_id, limit, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_release_decisions_for_title_query(
                            &pool, &title_id, limit,
                        ).await,
                    );
                }
                DbCommand::ListReleaseDecisionsForWantedItem { wanted_item_id, limit, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_release_decisions_for_wanted_item_query(
                            &pool, &wanted_item_id, limit,
                        ).await,
                    );
                }
            }
        }
    });

    sender
}
