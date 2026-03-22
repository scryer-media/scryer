use crate::types::{
    MigrationStatus, ReleaseDownloadFailureSignatureRecord, SettingDefinitionSeed,
    SettingsDefinitionRecord, SettingsValueRecord, TitleReleaseBlocklistRecord,
    WorkflowOperationRecord,
};
use crate::{
    migrations,
    queries::{
        blocklist as blocklist_queries, download_client::*, event::*, housekeeping, indexer::*,
        notification_channel, notification_subscription, plugin_installation::*,
        post_processing_script as pp_queries, quality::*, rule_set::*, settings::*, title::*,
        title_history as th_queries, user::*, workflow::*,
    },
};
use scryer_application::QualityProfile;
use scryer_application::{
    AppError, AppResult, PendingRelease, PrimaryCollectionSummary, ReleaseDecision,
    ReleaseDownloadAttemptOutcome, TitleMediaSizeSummary, TitleMetadataUpdate, WantedItem,
};
use scryer_domain::{
    BlocklistEntry, CalendarEpisode, Collection, CollectionType, DownloadClientConfig, Episode,
    HistoryEvent, ImportRecord, IndexerConfig, MediaFacet, PluginInstallation, RuleSet, Title,
    TitleHistoryRecord, User,
};
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use tokio::sync::oneshot::Sender;

type PluginWasmBytesReply = AppResult<Vec<(PluginInstallation, Option<Vec<u8>>)>>;

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
        collection_type: Option<CollectionType>,
        collection_index: Option<String>,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
        monitored: Option<bool>,
        reply: Sender<AppResult<Collection>>,
    },
    UpdateInterstitialSeasonEpisode {
        collection_id: String,
        season_episode: Option<String>,
        reply: Sender<AppResult<()>>,
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
    ListUnhydratedTitles {
        limit: usize,
        language: String,
        reply: Sender<AppResult<Vec<Title>>>,
    },
    ClearMetadataLanguageForAll {
        reply: Sender<AppResult<u64>>,
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
        config_json: Option<String>,
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
    ReorderDownloadClientConfigs {
        ordered_ids: Vec<String>,
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
    DeleteQualityProfile {
        profile_id: String,
        reply: Sender<AppResult<()>>,
    },
    ListAppliedMigrations {
        reply: Sender<AppResult<Vec<MigrationStatus>>>,
    },
    VacuumInto {
        dest_path: String,
        reply: Sender<AppResult<()>>,
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
    RecordDownloadSubmission {
        title_id: String,
        facet: String,
        download_client_type: String,
        download_client_item_id: String,
        source_title: Option<String>,
        collection_id: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    FindDownloadSubmission {
        download_client_type: String,
        download_client_item_id: String,
        reply: Sender<AppResult<Option<scryer_application::DownloadSubmission>>>,
    },
    ListDownloadSubmissionsForTitle {
        title_id: String,
        reply: Sender<AppResult<Vec<scryer_application::DownloadSubmission>>>,
    },
    DeleteDownloadSubmissionsForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    GetImportById {
        id: String,
        reply: Sender<AppResult<Option<ImportRecord>>>,
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
        input: scryer_application::InsertMediaFileInput,
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
    ListTitleMediaSizeSummaries {
        title_ids: Vec<String>,
        reply: Sender<AppResult<Vec<TitleMediaSizeSummary>>>,
    },
    UpdateMediaFileAnalysis {
        file_id: String,
        analysis: Box<scryer_application::MediaFileAnalysis>,
        reply: Sender<AppResult<()>>,
    },
    MarkMediaFileScanFailed {
        file_id: String,
        error: String,
        reply: Sender<AppResult<()>>,
    },
    GetMediaFileById {
        file_id: String,
        reply: Sender<AppResult<Option<scryer_application::TitleMediaFile>>>,
    },
    DeleteMediaFile {
        file_id: String,
        reply: Sender<AppResult<()>>,
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
    ListEpisodesInDateRange {
        start_date: String,
        end_date: String,
        reply: Sender<AppResult<Vec<CalendarEpisode>>>,
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
    ResetFruitlessWantedItems {
        now: String,
        reply: Sender<AppResult<u64>>,
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
    // ── Pending Releases ──────────────────────────────────────────────
    InsertPendingRelease {
        release: PendingRelease,
        reply: Sender<AppResult<String>>,
    },
    ListExpiredPendingReleases {
        now: String,
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    ListPendingReleasesForWantedItem {
        wanted_item_id: String,
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    UpdatePendingReleaseStatus {
        id: String,
        status: String,
        grabbed_at: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    SupersedePendingReleasesForWantedItem {
        wanted_item_id: String,
        except_id: String,
        reply: Sender<AppResult<()>>,
    },
    ListWaitingPendingReleases {
        reply: Sender<AppResult<Vec<PendingRelease>>>,
    },
    GetPendingRelease {
        id: String,
        reply: Sender<AppResult<Option<PendingRelease>>>,
    },
    DeletePendingReleasesForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    // ── Rule Sets ──────────────────────────────────────────────────────
    ListRuleSets {
        reply: Sender<AppResult<Vec<RuleSet>>>,
    },
    ListEnabledRuleSets {
        reply: Sender<AppResult<Vec<RuleSet>>>,
    },
    GetRuleSet {
        id: String,
        reply: Sender<AppResult<Option<RuleSet>>>,
    },
    CreateRuleSet {
        rule_set: RuleSet,
        reply: Sender<AppResult<()>>,
    },
    UpdateRuleSet {
        rule_set: RuleSet,
        reply: Sender<AppResult<()>>,
    },
    DeleteRuleSet {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    GetRuleSetByManagedKey {
        key: String,
        reply: Sender<AppResult<Option<RuleSet>>>,
    },
    DeleteRuleSetByManagedKey {
        key: String,
        reply: Sender<AppResult<()>>,
    },
    ListRuleSetsByManagedKeyPrefix {
        prefix: String,
        reply: Sender<AppResult<Vec<RuleSet>>>,
    },
    RecordRuleSetHistory {
        id: String,
        rule_set_id: String,
        action: String,
        rego_source: Option<String>,
        actor_id: Option<String>,
        reply: Sender<AppResult<()>>,
    },
    // ── Post-Processing Scripts ──────────────────────────────────
    ListPPScripts {
        reply: Sender<AppResult<Vec<scryer_domain::PostProcessingScript>>>,
    },
    GetPPScript {
        id: String,
        reply: Sender<AppResult<Option<scryer_domain::PostProcessingScript>>>,
    },
    CreatePPScript {
        script: scryer_domain::PostProcessingScript,
        reply: Sender<AppResult<scryer_domain::PostProcessingScript>>,
    },
    UpdatePPScript {
        script: scryer_domain::PostProcessingScript,
        reply: Sender<AppResult<scryer_domain::PostProcessingScript>>,
    },
    DeletePPScript {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    ListEnabledPPScriptsForFacet {
        facet: String,
        reply: Sender<AppResult<Vec<scryer_domain::PostProcessingScript>>>,
    },
    RecordPPScriptRun {
        run: scryer_domain::PostProcessingScriptRun,
        reply: Sender<AppResult<()>>,
    },
    ListPPScriptRunsForScript {
        script_id: String,
        limit: usize,
        reply: Sender<AppResult<Vec<scryer_domain::PostProcessingScriptRun>>>,
    },
    ListPPScriptRunsForTitle {
        title_id: String,
        limit: usize,
        reply: Sender<AppResult<Vec<scryer_domain::PostProcessingScriptRun>>>,
    },
    // ── Plugin Installations ─────────────────────────────────────
    ListPluginInstallations {
        reply: Sender<AppResult<Vec<PluginInstallation>>>,
    },
    GetPluginInstallation {
        plugin_id: String,
        reply: Sender<AppResult<Option<PluginInstallation>>>,
    },
    CreatePluginInstallation {
        installation: PluginInstallation,
        wasm_bytes: Option<Vec<u8>>,
        reply: Sender<AppResult<PluginInstallation>>,
    },
    UpdatePluginInstallation {
        installation: PluginInstallation,
        wasm_bytes: Option<Vec<u8>>,
        reply: Sender<AppResult<PluginInstallation>>,
    },
    DeletePluginInstallation {
        plugin_id: String,
        reply: Sender<AppResult<()>>,
    },
    GetEnabledPluginWasmBytes {
        reply: Sender<PluginWasmBytesReply>,
    },
    SeedBuiltinPlugin {
        plugin_id: String,
        name: String,
        description: String,
        version: String,
        provider_type: String,
        reply: Sender<AppResult<()>>,
    },
    StoreRegistryCache {
        json: String,
        reply: Sender<AppResult<()>>,
    },
    GetRegistryCache {
        reply: Sender<AppResult<Option<String>>>,
    },
    // ── Notification channels ───────────────────────────────
    ListNotificationChannels {
        reply: Sender<AppResult<Vec<scryer_domain::NotificationChannelConfig>>>,
    },
    GetNotificationChannel {
        id: String,
        reply: Sender<AppResult<Option<scryer_domain::NotificationChannelConfig>>>,
    },
    CreateNotificationChannel {
        config: scryer_domain::NotificationChannelConfig,
        reply: Sender<AppResult<scryer_domain::NotificationChannelConfig>>,
    },
    UpdateNotificationChannel {
        config: scryer_domain::NotificationChannelConfig,
        reply: Sender<AppResult<scryer_domain::NotificationChannelConfig>>,
    },
    DeleteNotificationChannel {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    // ── Notification subscriptions ──────────────────────────
    ListNotificationSubscriptions {
        reply: Sender<AppResult<Vec<scryer_domain::NotificationSubscription>>>,
    },
    ListNotificationSubscriptionsForChannel {
        channel_id: String,
        reply: Sender<AppResult<Vec<scryer_domain::NotificationSubscription>>>,
    },
    ListNotificationSubscriptionsForEvent {
        event_type: String,
        reply: Sender<AppResult<Vec<scryer_domain::NotificationSubscription>>>,
    },
    CreateNotificationSubscription {
        sub: scryer_domain::NotificationSubscription,
        reply: Sender<AppResult<scryer_domain::NotificationSubscription>>,
    },
    UpdateNotificationSubscription {
        sub: scryer_domain::NotificationSubscription,
        reply: Sender<AppResult<scryer_domain::NotificationSubscription>>,
    },
    DeleteNotificationSubscription {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    // ── Housekeeping ───────────────────────────────────────────────────
    DeleteReleaseDecisionsOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteReleaseAttemptsOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteDispatchedEventOutboxesOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    DeleteHistoryEventsOlderThan {
        days: i64,
        reply: Sender<AppResult<u32>>,
    },
    ListAllMediaFilePaths {
        reply: Sender<AppResult<Vec<(String, String)>>>,
    },
    DeleteMediaFilesByIds {
        ids: Vec<String>,
        reply: Sender<AppResult<u32>>,
    },
    // ── Title History ─────────────────────────────────────────────────
    InsertTitleHistoryEvent {
        title_id: String,
        episode_id: Option<String>,
        collection_id: Option<String>,
        event_type: String,
        source_title: Option<String>,
        quality: Option<String>,
        download_id: Option<String>,
        data_json: Option<String>,
        reply: Sender<AppResult<String>>,
    },
    ListTitleHistory {
        event_types: Option<Vec<String>>,
        title_ids: Option<Vec<String>>,
        download_id: Option<String>,
        limit: usize,
        offset: usize,
        reply: Sender<AppResult<(Vec<TitleHistoryRecord>, i64)>>,
    },
    ListTitleHistoryForTitle {
        title_id: String,
        event_types: Option<Vec<String>>,
        limit: usize,
        offset: usize,
        reply: Sender<AppResult<(Vec<TitleHistoryRecord>, i64)>>,
    },
    ListTitleHistoryForEpisode {
        episode_id: String,
        limit: usize,
        reply: Sender<AppResult<Vec<TitleHistoryRecord>>>,
    },
    FindTitleHistoryByDownloadId {
        download_id: String,
        reply: Sender<AppResult<Vec<TitleHistoryRecord>>>,
    },
    DeleteTitleHistoryForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
    },
    // ── Blocklist ─────────────────────────────────────────────────────
    InsertBlocklistEntry {
        title_id: String,
        source_title: Option<String>,
        source_hint: Option<String>,
        quality: Option<String>,
        download_id: Option<String>,
        reason: Option<String>,
        data_json: Option<String>,
        reply: Sender<AppResult<String>>,
    },
    ListBlocklistForTitle {
        title_id: String,
        limit: usize,
        reply: Sender<AppResult<Vec<BlocklistEntry>>>,
    },
    ListBlocklistAll {
        limit: usize,
        offset: usize,
        reply: Sender<AppResult<(Vec<BlocklistEntry>, i64)>>,
    },
    DeleteBlocklistEntry {
        id: String,
        reply: Sender<AppResult<()>>,
    },
    IsBlocklisted {
        title_id: String,
        source_title: String,
        reply: Sender<AppResult<bool>>,
    },
    DeleteBlocklistForTitle {
        title_id: String,
        reply: Sender<AppResult<()>>,
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
                    let _ = reply.send(update_title_monitored_query(&pool, &id, monitored).await);
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
                    let _ = reply
                        .send(update_title_hydrated_metadata_query(&pool, &id, metadata).await);
                }
                DbCommand::ListCollectionsForTitle { title_id, reply } => {
                    let _ = reply.send(list_collections_for_title_query(&pool, &title_id).await);
                }
                DbCommand::ListPrimaryCollectionSummaries { title_ids, reply } => {
                    let _ = reply
                        .send(list_primary_collection_summaries_query(&pool, &title_ids).await);
                }
                DbCommand::GetCollectionById {
                    collection_id,
                    reply,
                } => {
                    let _ = reply.send(get_collection_by_id_query(&pool, &collection_id).await);
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
                DbCommand::UpdateInterstitialSeasonEpisode {
                    collection_id,
                    season_episode,
                    reply,
                } => {
                    let _ = reply.send(
                        update_interstitial_season_episode_query(
                            &pool,
                            &collection_id,
                            season_episode.as_deref(),
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
                    let _ =
                        reply.send(list_episodes_for_collection_query(&pool, &collection_id).await);
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
                    overview,
                    tvdb_id,
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
                            overview,
                            tvdb_id,
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
                DbCommand::ListUnhydratedTitles {
                    limit,
                    language,
                    reply,
                } => {
                    let _ = reply.send(list_unhydrated_titles_query(&pool, limit, &language).await);
                }
                DbCommand::ClearMetadataLanguageForAll { reply } => {
                    let _ = reply.send(clear_metadata_language_for_all_query(&pool).await);
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
                        .send(update_user_entitlements_query(&pool, &id, &entitlements_json).await);
                }
                DbCommand::UpdateUserPassword {
                    id,
                    password_hash,
                    reply,
                } => {
                    let _ =
                        reply.send(update_user_password_query(&pool, &id, &password_hash).await);
                }
                DbCommand::DeleteUser { id, reply } => {
                    let _ = reply.send(delete_user_query(&pool, &id).await);
                }
                DbCommand::ListIndexerConfigs {
                    provider_type,
                    reply,
                } => {
                    let _ = reply.send(
                        list_indexer_configs_query(&pool, provider_type, encryption_key.as_ref())
                            .await,
                    );
                }
                DbCommand::GetIndexerConfig { id, reply } => {
                    let _ = reply
                        .send(get_indexer_config_query(&pool, &id, encryption_key.as_ref()).await);
                }
                DbCommand::CreateIndexerConfig { config, reply } => {
                    let _ = reply.send(
                        create_indexer_config_query(&pool, &config, encryption_key.as_ref()).await,
                    );
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
                    config_json,
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
                            config_json,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
                }
                DbCommand::TouchIndexerLastError {
                    provider_type,
                    reply,
                } => {
                    let _ = reply.send(touch_indexer_last_error_query(&pool, &provider_type).await);
                }
                DbCommand::DeleteIndexerConfig { id, reply } => {
                    let _ = reply.send(delete_indexer_config_query(&pool, &id).await);
                }
                DbCommand::ListDownloadClientConfigs { client_type, reply } => {
                    let _ = reply.send(
                        list_download_client_configs_query(
                            &pool,
                            client_type,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
                }
                DbCommand::GetDownloadClientConfig { id, reply } => {
                    let _ = reply.send(
                        get_download_client_config_query(&pool, &id, encryption_key.as_ref()).await,
                    );
                }
                DbCommand::CreateDownloadClientConfig { config, reply } => {
                    let _ = reply.send(
                        create_download_client_config_query(
                            &pool,
                            &config,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
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
                    let _ = reply.send(
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
                DbCommand::ReorderDownloadClientConfigs { ordered_ids, reply } => {
                    let _ = reply
                        .send(reorder_download_client_configs_query(&pool, &ordered_ids).await);
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
                    let _ = reply.send(
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
                    let _ = reply
                        .send(batch_ensure_setting_definitions_query(&pool, &definitions).await);
                }
                DbCommand::BatchGetSettingsWithDefaults { keys, reply } => {
                    let _ = reply.send(
                        batch_get_settings_with_defaults_query(
                            &pool,
                            &keys,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
                }
                DbCommand::BatchUpsertSettingsIfNotOverridden { entries, reply } => {
                    let _ = reply.send(
                        batch_upsert_settings_if_not_overridden_query(
                            &pool,
                            &entries,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListSettingDefinitions { scope, reply } => {
                    let _ = reply.send(list_setting_definitions_query(&pool, scope).await);
                }
                DbCommand::ListSettingsWithValues {
                    scope,
                    scope_id,
                    reply,
                } => {
                    let _ = reply.send(
                        list_settings_with_defaults_query(
                            &pool,
                            &scope,
                            scope_id,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
                }
                DbCommand::GetSettingWithDefaults {
                    scope,
                    key_name,
                    scope_id,
                    reply,
                } => {
                    let _ = reply.send(
                        get_setting_with_defaults_query(
                            &pool,
                            &scope,
                            &key_name,
                            scope_id,
                            encryption_key.as_ref(),
                        )
                        .await,
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
                        replace_quality_profiles_query(&pool, &scope, scope_id, profiles_json)
                            .await,
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
                DbCommand::DeleteQualityProfile { profile_id, reply } => {
                    let _ = reply.send(delete_quality_profile_query(&pool, &profile_id).await);
                }
                DbCommand::ListAppliedMigrations { reply } => {
                    let _ = reply.send(migrations::list_applied_migrations(&pool).await);
                }
                DbCommand::VacuumInto { dest_path, reply } => {
                    let result = sqlx::query("VACUUM INTO ?")
                        .bind(&dest_path)
                        .execute(&pool)
                        .await
                        .map(|_| ())
                        .map_err(|e| AppError::Repository(format!("vacuum into failed: {e}")));
                    let _ = reply.send(result);
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
                DbCommand::RecordDownloadSubmission {
                    title_id,
                    facet,
                    download_client_type,
                    download_client_item_id,
                    source_title,
                    collection_id,
                    reply,
                } => {
                    let _ = reply.send(
                        record_download_submission_query(
                            &pool,
                            &title_id,
                            &facet,
                            &download_client_type,
                            &download_client_item_id,
                            source_title.as_deref(),
                            collection_id.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::FindDownloadSubmission {
                    download_client_type,
                    download_client_item_id,
                    reply,
                } => {
                    let _ = reply.send(
                        find_download_submission_query(
                            &pool,
                            &download_client_type,
                            &download_client_item_id,
                        )
                        .await,
                    );
                }
                DbCommand::ListDownloadSubmissionsForTitle { title_id, reply } => {
                    let _ = reply
                        .send(list_download_submissions_for_title_query(&pool, &title_id).await);
                }
                DbCommand::DeleteDownloadSubmissionsForTitle { title_id, reply } => {
                    let _ = reply
                        .send(delete_download_submissions_for_title_query(&pool, &title_id).await);
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
                    let _ =
                        reply.send(list_failed_release_download_attempts_query(&pool, limit).await);
                }
                DbCommand::ListFailedReleaseDownloadAttemptsForTitle {
                    title_id,
                    limit,
                    reply,
                } => {
                    let _ = reply.send(
                        list_failed_release_download_attempts_for_title_query(
                            &pool, &title_id, limit,
                        )
                        .await,
                    );
                }
                DbCommand::GetImportById { id, reply } => {
                    let _ = reply.send(get_import_by_id_query(&pool, &id).await);
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
                    let _ = reply
                        .send(recover_stale_processing_imports_query(&pool, stale_seconds).await);
                }
                DbCommand::ListPendingImports { reply } => {
                    let _ = reply.send(list_pending_imports_query(&pool).await);
                }
                DbCommand::ListImports { limit, reply } => {
                    let _ = reply.send(list_imports_query(&pool, limit).await);
                }
                DbCommand::InsertMediaFile { input, reply } => {
                    let _ = reply.send(
                        crate::queries::media_file::insert_media_file_query(&pool, &input).await,
                    );
                }
                DbCommand::LinkFileToEpisode {
                    file_id,
                    episode_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::media_file::link_file_to_episode_query(
                            &pool,
                            &file_id,
                            &episode_id,
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
                DbCommand::ListTitleMediaSizeSummaries { title_ids, reply } => {
                    let _ = reply.send(
                        crate::queries::media_file::list_title_media_size_summaries_query(
                            &pool, &title_ids,
                        )
                        .await,
                    );
                }
                DbCommand::UpdateMediaFileAnalysis {
                    file_id,
                    analysis,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::media_file::update_media_file_analysis_query(
                            &pool, &file_id, &analysis,
                        )
                        .await,
                    );
                }
                DbCommand::MarkMediaFileScanFailed {
                    file_id,
                    error,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::media_file::mark_scan_failed_query(&pool, &file_id, &error)
                            .await,
                    );
                }
                DbCommand::GetMediaFileById { file_id, reply } => {
                    let _ = reply.send(
                        crate::queries::media_file::get_media_file_by_id_query(&pool, &file_id)
                            .await,
                    );
                }
                DbCommand::DeleteMediaFile { file_id, reply } => {
                    let _ = reply.send(
                        crate::queries::media_file::delete_media_file_query(&pool, &file_id).await,
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
                            &pool,
                            &title_id,
                            &season_number,
                            &episode_number,
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
                            &pool,
                            &title_id,
                            &absolute_number,
                        )
                        .await,
                    );
                }
                DbCommand::ListEpisodesInDateRange {
                    start_date,
                    end_date,
                    reply,
                } => {
                    let _ = reply.send(
                        list_episodes_in_date_range_query(&pool, &start_date, &end_date).await,
                    );
                }
                DbCommand::UpsertWantedItem { item, reply } => {
                    let _ = reply
                        .send(crate::queries::wanted::upsert_wanted_item_query(&pool, &item).await);
                }
                DbCommand::ListDueWantedItems {
                    now,
                    batch_limit,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_due_wanted_items_query(
                            &pool,
                            &now,
                            batch_limit,
                        )
                        .await,
                    );
                }
                DbCommand::UpdateWantedItemStatus {
                    id,
                    status,
                    next_search_at,
                    last_search_at,
                    search_count,
                    current_score,
                    grabbed_release,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::update_wanted_item_status_query(
                            &pool,
                            &id,
                            &status,
                            next_search_at.as_deref(),
                            last_search_at.as_deref(),
                            search_count,
                            current_score,
                            grabbed_release.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::GetWantedItemForTitle {
                    title_id,
                    episode_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::get_wanted_item_for_title_query(
                            &pool,
                            &title_id,
                            episode_id.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::DeleteWantedItemsForTitle { title_id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::delete_wanted_items_for_title_query(
                            &pool, &title_id,
                        )
                        .await,
                    );
                }
                DbCommand::ResetFruitlessWantedItems { now, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::reset_fruitless_wanted_items_query(&pool, &now)
                            .await,
                    );
                }
                DbCommand::InsertReleaseDecision { decision, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::insert_release_decision_query(&pool, &decision)
                            .await,
                    );
                }
                DbCommand::GetWantedItemById { id, reply } => {
                    let _ = reply.send(
                        crate::queries::wanted::get_wanted_item_by_id_query(&pool, &id).await,
                    );
                }
                DbCommand::ListWantedItems {
                    status,
                    media_type,
                    title_id,
                    limit,
                    offset,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_wanted_items_query(
                            &pool,
                            status.as_deref(),
                            media_type.as_deref(),
                            title_id.as_deref(),
                            limit,
                            offset,
                        )
                        .await,
                    );
                }
                DbCommand::CountWantedItems {
                    status,
                    media_type,
                    title_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::count_wanted_items_query(
                            &pool,
                            status.as_deref(),
                            media_type.as_deref(),
                            title_id.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListReleaseDecisionsForTitle {
                    title_id,
                    limit,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_release_decisions_for_title_query(
                            &pool, &title_id, limit,
                        )
                        .await,
                    );
                }
                DbCommand::ListReleaseDecisionsForWantedItem {
                    wanted_item_id,
                    limit,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::wanted::list_release_decisions_for_wanted_item_query(
                            &pool,
                            &wanted_item_id,
                            limit,
                        )
                        .await,
                    );
                }
                // ── Pending Releases ──────────────────────────────────────
                DbCommand::InsertPendingRelease { release, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::insert_pending_release_query(
                            &pool, &release,
                        )
                        .await,
                    );
                }
                DbCommand::ListExpiredPendingReleases { now, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_expired_pending_releases_query(
                            &pool, &now,
                        )
                        .await,
                    );
                }
                DbCommand::ListPendingReleasesForWantedItem {
                    wanted_item_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_pending_releases_for_wanted_item_query(
                            &pool, &wanted_item_id,
                        ).await,
                    );
                }
                DbCommand::UpdatePendingReleaseStatus {
                    id,
                    status,
                    grabbed_at,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::update_pending_release_status_query(
                            &pool,
                            &id,
                            &status,
                            grabbed_at.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::SupersedePendingReleasesForWantedItem {
                    wanted_item_id,
                    except_id,
                    reply,
                } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::supersede_pending_releases_for_wanted_item_query(
                            &pool, &wanted_item_id, &except_id,
                        ).await,
                    );
                }
                DbCommand::ListWaitingPendingReleases { reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::list_waiting_pending_releases_query(
                            &pool,
                        )
                        .await,
                    );
                }
                DbCommand::GetPendingRelease { id, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::get_pending_release_query(&pool, &id)
                            .await,
                    );
                }
                DbCommand::DeletePendingReleasesForTitle { title_id, reply } => {
                    let _ = reply.send(
                        crate::queries::pending_releases::delete_pending_releases_for_title_query(
                            &pool, &title_id,
                        )
                        .await,
                    );
                }
                // ── Rule Sets ──────────────────────────────────────────────
                DbCommand::ListRuleSets { reply } => {
                    let _ = reply.send(list_rule_sets_query(&pool).await);
                }
                DbCommand::ListEnabledRuleSets { reply } => {
                    let _ = reply.send(list_enabled_rule_sets_query(&pool).await);
                }
                DbCommand::GetRuleSet { id, reply } => {
                    let _ = reply.send(get_rule_set_by_id_query(&pool, &id).await);
                }
                DbCommand::CreateRuleSet { rule_set, reply } => {
                    let _ = reply.send(insert_rule_set_query(&pool, &rule_set).await);
                }
                DbCommand::UpdateRuleSet { rule_set, reply } => {
                    let _ = reply.send(update_rule_set_query(&pool, &rule_set).await);
                }
                DbCommand::DeleteRuleSet { id, reply } => {
                    let _ = reply.send(delete_rule_set_query(&pool, &id).await);
                }
                DbCommand::GetRuleSetByManagedKey { key, reply } => {
                    let _ = reply.send(get_rule_set_by_managed_key_query(&pool, &key).await);
                }
                DbCommand::DeleteRuleSetByManagedKey { key, reply } => {
                    let _ = reply.send(delete_rule_set_by_managed_key_query(&pool, &key).await);
                }
                DbCommand::ListRuleSetsByManagedKeyPrefix { prefix, reply } => {
                    let _ = reply
                        .send(list_rule_sets_by_managed_key_prefix_query(&pool, &prefix).await);
                }
                DbCommand::RecordRuleSetHistory {
                    id,
                    rule_set_id,
                    action,
                    rego_source,
                    actor_id,
                    reply,
                } => {
                    let _ = reply.send(
                        insert_rule_set_history_query(
                            &pool,
                            &id,
                            &rule_set_id,
                            &action,
                            rego_source.as_deref(),
                            actor_id.as_deref(),
                        )
                        .await,
                    );
                }
                // ── Post-Processing Scripts ──────────────────────────────
                DbCommand::ListPPScripts { reply } => {
                    let _ = reply.send(pp_queries::list_scripts_query(&pool).await);
                }
                DbCommand::GetPPScript { id, reply } => {
                    let _ = reply.send(pp_queries::get_script_by_id_query(&pool, &id).await);
                }
                DbCommand::CreatePPScript { script, reply } => {
                    let _ = reply.send(pp_queries::insert_script_query(&pool, &script).await);
                }
                DbCommand::UpdatePPScript { script, reply } => {
                    let _ = reply.send(pp_queries::update_script_query(&pool, &script).await);
                }
                DbCommand::DeletePPScript { id, reply } => {
                    let _ = reply.send(pp_queries::delete_script_query(&pool, &id).await);
                }
                DbCommand::ListEnabledPPScriptsForFacet { facet, reply } => {
                    let _ =
                        reply.send(pp_queries::list_enabled_for_facet_query(&pool, &facet).await);
                }
                DbCommand::RecordPPScriptRun { run, reply } => {
                    let _ = reply.send(pp_queries::record_run_query(&pool, &run).await);
                }
                DbCommand::ListPPScriptRunsForScript {
                    script_id,
                    limit,
                    reply,
                } => {
                    let _ = reply.send(
                        pp_queries::list_runs_for_script_query(&pool, &script_id, limit).await,
                    );
                }
                DbCommand::ListPPScriptRunsForTitle {
                    title_id,
                    limit,
                    reply,
                } => {
                    let _ = reply
                        .send(pp_queries::list_runs_for_title_query(&pool, &title_id, limit).await);
                }
                // ── Plugin Installations ─────────────────────────────────
                DbCommand::ListPluginInstallations { reply } => {
                    let _ = reply.send(list_plugin_installations_query(&pool).await);
                }
                DbCommand::GetPluginInstallation { plugin_id, reply } => {
                    let _ = reply.send(get_plugin_installation_query(&pool, &plugin_id).await);
                }
                DbCommand::CreatePluginInstallation {
                    installation,
                    wasm_bytes,
                    reply,
                } => {
                    let _ = reply.send(
                        create_plugin_installation_query(
                            &pool,
                            &installation,
                            wasm_bytes.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::UpdatePluginInstallation {
                    installation,
                    wasm_bytes,
                    reply,
                } => {
                    let _ = reply.send(
                        update_plugin_installation_query(
                            &pool,
                            &installation,
                            wasm_bytes.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::DeletePluginInstallation { plugin_id, reply } => {
                    let _ = reply.send(delete_plugin_installation_query(&pool, &plugin_id).await);
                }
                DbCommand::GetEnabledPluginWasmBytes { reply } => {
                    let _ = reply.send(get_enabled_plugin_wasm_bytes_query(&pool).await);
                }
                DbCommand::SeedBuiltinPlugin {
                    plugin_id,
                    name,
                    description,
                    version,
                    provider_type,
                    reply,
                } => {
                    let _ = reply.send(
                        seed_builtin_query(
                            &pool,
                            &plugin_id,
                            &name,
                            &description,
                            &version,
                            &provider_type,
                        )
                        .await,
                    );
                }
                DbCommand::StoreRegistryCache { json, reply } => {
                    let _ = reply.send(store_registry_cache_query(&pool, &json).await);
                }
                DbCommand::GetRegistryCache { reply } => {
                    let _ = reply.send(get_registry_cache_query(&pool).await);
                }
                // ── Notification Channels ────────────────────────────────
                DbCommand::ListNotificationChannels { reply } => {
                    let _ = reply.send(
                        notification_channel::list_notification_channels_query(
                            &pool,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
                }
                DbCommand::GetNotificationChannel { id, reply } => {
                    let _ = reply.send(
                        notification_channel::get_notification_channel_query(
                            &pool,
                            &id,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
                }
                DbCommand::CreateNotificationChannel { config, reply } => {
                    let _ = reply.send(
                        notification_channel::create_notification_channel_query(
                            &pool,
                            &config,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
                }
                DbCommand::UpdateNotificationChannel { config, reply } => {
                    let _ = reply.send(
                        notification_channel::update_notification_channel_query(
                            &pool,
                            &config,
                            encryption_key.as_ref(),
                        )
                        .await,
                    );
                }
                DbCommand::DeleteNotificationChannel { id, reply } => {
                    let _ = reply.send(
                        notification_channel::delete_notification_channel_query(&pool, &id).await,
                    );
                }
                // ── Notification Subscriptions ───────────────────────────
                DbCommand::ListNotificationSubscriptions { reply } => {
                    let _ = reply.send(
                        notification_subscription::list_notification_subscriptions_query(&pool)
                            .await,
                    );
                }
                DbCommand::ListNotificationSubscriptionsForChannel { channel_id, reply } => {
                    let _ = reply.send(
                        notification_subscription::list_notification_subscriptions_for_channel_query(&pool, &channel_id).await,
                    );
                }
                DbCommand::ListNotificationSubscriptionsForEvent { event_type, reply } => {
                    let _ = reply.send(
                        notification_subscription::list_notification_subscriptions_for_event_query(
                            &pool,
                            &event_type,
                        )
                        .await,
                    );
                }
                DbCommand::CreateNotificationSubscription { sub, reply } => {
                    let _ = reply.send(
                        notification_subscription::create_notification_subscription_query(
                            &pool, &sub,
                        )
                        .await,
                    );
                }
                DbCommand::UpdateNotificationSubscription { sub, reply } => {
                    let _ = reply.send(
                        notification_subscription::update_notification_subscription_query(
                            &pool, &sub,
                        )
                        .await,
                    );
                }
                DbCommand::DeleteNotificationSubscription { id, reply } => {
                    let _ = reply.send(
                        notification_subscription::delete_notification_subscription_query(
                            &pool, &id,
                        )
                        .await,
                    );
                }
                // ── Housekeeping ────────────────────────────────────────
                DbCommand::DeleteReleaseDecisionsOlderThan { days, reply } => {
                    let _ = reply.send(
                        housekeeping::delete_release_decisions_older_than_query(&pool, days).await,
                    );
                }
                DbCommand::DeleteReleaseAttemptsOlderThan { days, reply } => {
                    let _ = reply.send(
                        housekeeping::delete_release_attempts_older_than_query(&pool, days).await,
                    );
                }
                DbCommand::DeleteDispatchedEventOutboxesOlderThan { days, reply } => {
                    let _ = reply.send(
                        housekeeping::delete_dispatched_event_outboxes_older_than_query(
                            &pool, days,
                        )
                        .await,
                    );
                }
                DbCommand::DeleteHistoryEventsOlderThan { days, reply } => {
                    let _ = reply.send(
                        housekeeping::delete_history_events_older_than_query(&pool, days).await,
                    );
                }
                DbCommand::ListAllMediaFilePaths { reply } => {
                    let _ = reply.send(housekeeping::list_all_media_file_paths_query(&pool).await);
                }
                DbCommand::DeleteMediaFilesByIds { ids, reply } => {
                    let _ = reply
                        .send(housekeeping::delete_media_files_by_ids_query(&pool, &ids).await);
                }
                // ── Title History ─────────────────────────────────────────
                DbCommand::InsertTitleHistoryEvent {
                    title_id,
                    episode_id,
                    collection_id,
                    event_type,
                    source_title,
                    quality,
                    download_id,
                    data_json,
                    reply,
                } => {
                    let _ = reply.send(
                        th_queries::insert_title_history_event_query(
                            &pool,
                            &title_id,
                            episode_id.as_deref(),
                            collection_id.as_deref(),
                            &event_type,
                            source_title.as_deref(),
                            quality.as_deref(),
                            download_id.as_deref(),
                            data_json.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListTitleHistory {
                    event_types,
                    title_ids,
                    download_id,
                    limit,
                    offset,
                    reply,
                } => {
                    let type_strs: Option<Vec<&str>> = event_types
                        .as_ref()
                        .map(|v| v.iter().map(|s| s.as_str()).collect());
                    let res = th_queries::list_title_history_query(
                        &pool,
                        type_strs.as_deref(),
                        title_ids.as_deref(),
                        download_id.as_deref(),
                        limit,
                        offset,
                    )
                    .await
                    .map(|(rows, total)| {
                        let records = rows.into_iter().map(th_row_to_record).collect();
                        (records, total)
                    });
                    let _ = reply.send(res);
                }
                DbCommand::ListTitleHistoryForTitle {
                    title_id,
                    event_types,
                    limit,
                    offset,
                    reply,
                } => {
                    let type_strs: Option<Vec<&str>> = event_types
                        .as_ref()
                        .map(|v| v.iter().map(|s| s.as_str()).collect());
                    let res = th_queries::list_title_history_for_title_query(
                        &pool,
                        &title_id,
                        type_strs.as_deref(),
                        limit,
                        offset,
                    )
                    .await
                    .map(|(rows, total)| {
                        let records = rows.into_iter().map(th_row_to_record).collect();
                        (records, total)
                    });
                    let _ = reply.send(res);
                }
                DbCommand::ListTitleHistoryForEpisode {
                    episode_id,
                    limit,
                    reply,
                } => {
                    let res =
                        th_queries::list_title_history_for_episode_query(&pool, &episode_id, limit)
                            .await
                            .map(|rows| rows.into_iter().map(th_row_to_record).collect());
                    let _ = reply.send(res);
                }
                DbCommand::FindTitleHistoryByDownloadId { download_id, reply } => {
                    let res =
                        th_queries::find_title_history_by_download_id_query(&pool, &download_id)
                            .await
                            .map(|rows| rows.into_iter().map(th_row_to_record).collect());
                    let _ = reply.send(res);
                }
                DbCommand::DeleteTitleHistoryForTitle { title_id, reply } => {
                    let _ = reply.send(
                        th_queries::delete_title_history_for_title_query(&pool, &title_id).await,
                    );
                }
                // ── Blocklist ─────────────────────────────────────────────
                DbCommand::InsertBlocklistEntry {
                    title_id,
                    source_title,
                    source_hint,
                    quality,
                    download_id,
                    reason,
                    data_json,
                    reply,
                } => {
                    let _ = reply.send(
                        blocklist_queries::insert_blocklist_entry_query(
                            &pool,
                            &title_id,
                            source_title.as_deref(),
                            source_hint.as_deref(),
                            quality.as_deref(),
                            download_id.as_deref(),
                            reason.as_deref(),
                            data_json.as_deref(),
                        )
                        .await,
                    );
                }
                DbCommand::ListBlocklistForTitle {
                    title_id,
                    limit,
                    reply,
                } => {
                    let res =
                        blocklist_queries::list_blocklist_for_title_query(&pool, &title_id, limit)
                            .await
                            .map(|rows| rows.into_iter().map(bl_row_to_entry).collect());
                    let _ = reply.send(res);
                }
                DbCommand::ListBlocklistAll {
                    limit,
                    offset,
                    reply,
                } => {
                    let res = blocklist_queries::list_blocklist_all_query(&pool, limit, offset)
                        .await
                        .map(|(rows, total)| {
                            let entries = rows.into_iter().map(bl_row_to_entry).collect();
                            (entries, total)
                        });
                    let _ = reply.send(res);
                }
                DbCommand::DeleteBlocklistEntry { id, reply } => {
                    let _ = reply
                        .send(blocklist_queries::delete_blocklist_entry_query(&pool, &id).await);
                }
                DbCommand::IsBlocklisted {
                    title_id,
                    source_title,
                    reply,
                } => {
                    let _ = reply.send(
                        blocklist_queries::is_blocklisted_query(&pool, &title_id, &source_title)
                            .await,
                    );
                }
                DbCommand::DeleteBlocklistForTitle { title_id, reply } => {
                    let _ = reply.send(
                        blocklist_queries::delete_blocklist_for_title_query(&pool, &title_id).await,
                    );
                }
            }
        }
    });

    sender
}

fn th_row_to_record(row: th_queries::TitleHistoryRow) -> TitleHistoryRecord {
    TitleHistoryRecord {
        id: row.id,
        title_id: row.title_id,
        episode_id: row.episode_id,
        collection_id: row.collection_id,
        event_type: scryer_domain::HistoryEventType::parse(&row.event_type)
            .unwrap_or(scryer_domain::HistoryEventType::Grabbed),
        source_title: row.source_title,
        quality: row.quality,
        download_id: row.download_id,
        data_json: row.data_json,
        occurred_at: row.occurred_at,
        created_at: row.created_at,
    }
}

fn bl_row_to_entry(row: blocklist_queries::BlocklistRow) -> BlocklistEntry {
    BlocklistEntry {
        id: row.id,
        title_id: row.title_id,
        source_title: row.source_title,
        source_hint: row.source_hint,
        quality: row.quality,
        download_id: row.download_id,
        reason: row.reason,
        data_json: row.data_json,
        created_at: row.created_at,
    }
}
