use scryer_application::{
    AppError, AppResult, QualityProfile, ReleaseDownloadAttemptOutcome,
    ReleaseDownloadFailureSignature, TitleReleaseBlocklistEntry,
};
use scryer_domain::{
    BlocklistEntry, DownloadClientConfig, Episode, ImportRecord, ImportStatus, TitleHistoryRecord,
};
use tokio::sync::{mpsc, oneshot};

use crate::commands::{DbCommand, spawn_db_command_worker};
use crate::types::{MigrationMode, MigrationStatus, SettingsDefinitionRecord, SettingsValueRecord};

#[derive(Clone)]
pub struct SqliteServices {
    pub(crate) sender: mpsc::Sender<DbCommand>,
    pub(crate) pool: sqlx::SqlitePool,
    db_path: String,
}

impl SqliteServices {
    /// Public pool accessor for cross-crate query access.
    pub fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }

    pub async fn new(path: impl AsRef<str>) -> Result<Self, AppError> {
        Self::new_with_mode(path, MigrationMode::Apply).await
    }

    pub async fn new_with_mode(
        path: impl AsRef<str>,
        migration_mode: MigrationMode,
    ) -> Result<Self, AppError> {
        let db_url = crate::sqlite_url_with_create(path.as_ref());
        let is_memory = db_url.contains(":memory:");

        // Ensure the parent directory exists for file-based databases.
        if !is_memory
            && let Some(file_path) = path
                .as_ref()
                .strip_prefix("sqlite://")
                .or(Some(path.as_ref()))
        {
            let file_path = file_path.split('?').next().unwrap_or(file_path);
            let db_file = std::path::Path::new(file_path);
            if let Some(parent) = db_file.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    tracing::info!(path = %parent.display(), "creating database directory");
                    std::fs::create_dir_all(parent).map_err(|err| {
                        AppError::Repository(format!(
                            "cannot create database directory {}: {err}",
                            parent.display(),
                        ))
                    })?;
                }
                // Log diagnostic info for troubleshooting permission issues.
                if parent.exists() {
                    let meta = std::fs::metadata(parent);
                    let probe = parent.join(".scryer-probe");
                    let writable = std::fs::File::create(&probe).is_ok();
                    let _ = std::fs::remove_file(&probe);
                    tracing::debug!(
                        path = %parent.display(),
                        writable,
                        permissions = ?meta.as_ref().map(|m| m.permissions()),
                        "database directory check",
                    );
                }
            }
        }

        let pool_opts = if is_memory {
            // For in-memory databases the data only lives as long as at least one
            // connection is open.  Prevent the pool from recycling/dropping the
            // single connection (via idle_timeout or max_lifetime) which would
            // silently destroy every table.
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .min_connections(1)
                .idle_timeout(None)
                .max_lifetime(None)
        } else {
            sqlx::sqlite::SqlitePoolOptions::new().max_connections(4)
        };

        // Build connect options so every connection gets WAL + busy_timeout.
        // sqlx already defaults foreign_keys = ON for SQLite.
        let mut connect_opts: sqlx::sqlite::SqliteConnectOptions = db_url
            .parse()
            .map_err(|err: sqlx::Error| AppError::Repository(err.to_string()))?;
        if !is_memory {
            connect_opts = connect_opts
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .busy_timeout(std::time::Duration::from_millis(5000));
        }

        let pool = pool_opts.connect_with(connect_opts).await.map_err(|err| {
            AppError::Repository(format!("cannot open database at {}: {err}", path.as_ref(),))
        })?;

        crate::migrations::run_migrations(&pool, migration_mode)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        let sender = spawn_db_command_worker(pool.clone());

        Ok(Self {
            sender,
            pool,
            db_path: path.as_ref().to_string(),
        })
    }

    pub fn sqlite_path(&self) -> AppResult<String> {
        Ok(self.db_path.clone())
    }

    pub async fn set_encryption_key(&self, key: crate::encryption::EncryptionKey) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::SetEncryptionKey {
                key,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_applied_migrations(&self) -> AppResult<Vec<MigrationStatus>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListAppliedMigrations { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn vacuum_into_db(&self, dest_path: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::VacuumInto {
                dest_path: dest_path.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_workflow_operation(
        &self,
        operation_type: impl Into<String>,
        status: impl Into<String>,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
    ) -> AppResult<crate::types::WorkflowOperationRecord> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateWorkflowOperation {
                operation_type: operation_type.into(),
                status: status.into(),
                actor_user_id,
                progress_json,
                started_at,
                completed_at,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_release_download_attempt(
        &self,
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateReleaseDownloadAttempt {
                title_id,
                source_hint,
                source_title,
                outcome,
                error_message,
                source_password,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_import_request(
        &self,
        source_system: String,
        source_ref: String,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateImportRequest {
                source_system,
                source_ref,
                import_type,
                payload_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_failed_release_download_attempt_signatures(
        &self,
        limit: usize,
    ) -> AppResult<Vec<ReleaseDownloadFailureSignature>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListFailedReleaseDownloadAttempts {
                limit: limit as i64,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        let records = reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))??;

        Ok(records
            .into_iter()
            .map(|record| ReleaseDownloadFailureSignature {
                source_hint: record.source_hint,
                source_title: record.source_title,
            })
            .collect())
    }

    pub async fn list_failed_release_download_attempts_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListFailedReleaseDownloadAttemptsForTitle {
                title_id: title_id.to_string(),
                limit: limit as i64,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        let records = reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))??;

        Ok(records
            .into_iter()
            .map(|record| TitleReleaseBlocklistEntry {
                source_hint: record.source_hint,
                source_title: record.source_title,
                error_message: record.error_message,
                attempted_at: record.attempted_at,
            })
            .collect())
    }

    pub async fn get_latest_source_password(
        &self,
        title_id: Option<&str>,
        source_hint: Option<&str>,
        source_title: Option<&str>,
    ) -> AppResult<Option<String>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetLatestSourcePassword {
                title_id: title_id.map(|value| value.to_string()),
                source_hint: source_hint.map(|value| value.to_string()),
                source_title: source_title.map(|value| value.to_string()),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn record_download_submission(
        &self,
        title_id: String,
        facet: String,
        download_client_type: String,
        download_client_item_id: String,
        source_title: Option<String>,
        collection_id: Option<String>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::RecordDownloadSubmission {
                title_id,
                facet,
                download_client_type,
                download_client_item_id,
                source_title,
                collection_id,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn commit_successful_grab(
        &self,
        commit: scryer_application::SuccessfulGrabCommit,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CommitSuccessfulGrab {
                commit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn find_download_submission(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<scryer_application::DownloadSubmission>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::FindDownloadSubmission {
                download_client_type: download_client_type.to_string(),
                download_client_item_id: download_client_item_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_download_submissions_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_application::DownloadSubmission>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListDownloadSubmissionsForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_download_submissions_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteDownloadSubmissionsForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_download_submission_by_client_item_id(
        &self,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteDownloadSubmissionByClientItemId {
                download_client_item_id: download_client_item_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_tracked_state(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
        tracked_state: &str,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateTrackedState {
                download_client_type: download_client_type.to_string(),
                download_client_item_id: download_client_item_id.to_string(),
                tracked_state: tracked_state.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_tracked_state(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<String>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetTrackedState {
                download_client_type: download_client_type.to_string(),
                download_client_item_id: download_client_item_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn insert_import_artifact(
        &self,
        artifact: scryer_application::ImportArtifact,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertImportArtifact {
                artifact,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_import_artifacts_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Vec<scryer_application::ImportArtifact>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListImportArtifactsBySourceRef {
                source_system: source_system.to_string(),
                source_ref: source_ref.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn count_import_artifacts_by_result(
        &self,
        source_system: &str,
        source_ref: &str,
        result: &str,
    ) -> AppResult<u64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CountImportArtifactsByResult {
                source_system: source_system.to_string(),
                source_ref: source_ref.to_string(),
                result: result.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    #[expect(clippy::too_many_arguments)]
    pub async fn ensure_setting_definition(
        &self,
        category: impl Into<String>,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        data_type: impl Into<String>,
        default_value_json: impl Into<String>,
        is_sensitive: bool,
        validation_json: Option<String>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::EnsureSettingDefinition {
                category: category.into(),
                scope: scope.into(),
                key_name: key_name.into(),
                data_type: data_type.into(),
                default_value_json: default_value_json.into(),
                is_sensitive,
                validation_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn batch_ensure_setting_definitions(
        &self,
        definitions: Vec<crate::types::SettingDefinitionSeed>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::BatchEnsureSettingDefinitions {
                definitions,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn batch_get_settings_with_defaults(
        &self,
        keys: Vec<(String, String, Option<String>)>,
    ) -> AppResult<Vec<Option<SettingsValueRecord>>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::BatchGetSettingsWithDefaults {
                keys,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn batch_upsert_settings_if_not_overridden(
        &self,
        entries: Vec<(String, String, String, String)>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::BatchUpsertSettingsIfNotOverridden {
                entries,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_setting_definitions(
        &self,
        scope: Option<String>,
    ) -> AppResult<Vec<SettingsDefinitionRecord>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListSettingDefinitions {
                scope,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_settings_with_defaults(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Vec<SettingsValueRecord>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListSettingsWithValues {
                scope: scope.into(),
                scope_id,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_setting_with_defaults(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Option<SettingsValueRecord>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetSettingWithDefaults {
                scope: scope.into(),
                key_name: key_name.into(),
                scope_id,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn upsert_setting_value(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
        value_json: impl Into<String>,
        source: impl Into<String>,
        updated_by_user_id: Option<String>,
    ) -> AppResult<SettingsValueRecord> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpsertSettingValue {
                scope: scope.into(),
                key_name: key_name.into(),
                scope_id,
                value_json: value_json.into(),
                source: source.into(),
                updated_by_user_id,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_quality_profiles(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        #[expect(clippy::type_complexity)]
        let (reply_tx, reply_rx): (
            oneshot::Sender<AppResult<Vec<QualityProfile>>>,
            oneshot::Receiver<AppResult<Vec<QualityProfile>>>,
        ) = oneshot::channel();
        self.sender
            .send(DbCommand::ListQualityProfiles {
                scope: scope.into(),
                scope_id,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn replace_quality_profiles(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ReplaceQualityProfiles {
                scope: scope.into(),
                scope_id,
                profiles_json: profiles,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn upsert_quality_profiles(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpsertQualityProfiles {
                scope: scope.into(),
                scope_id,
                profiles_json: profiles,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_quality_profile(&self, profile_id: impl Into<String>) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteQualityProfile {
                profile_id: profile_id.into(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetImportById {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_import_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Option<ImportRecord>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetImportBySourceRef {
                source_system: source_system.to_string(),
                source_ref: source_ref.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_import_status(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateImportStatus {
                import_id: import_id.to_string(),
                status: status.as_str().to_string(),
                result_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::RecoverStaleProcessingImports {
                stale_seconds,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListPendingImports { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_imports(&self, limit: i64) -> AppResult<Vec<ImportRecord>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListImports {
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn insert_media_file(
        &self,
        input: &scryer_application::InsertMediaFileInput,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertMediaFile {
                input: input.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::LinkFileToEpisode {
                file_id: file_id.to_string(),
                episode_id: episode_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_media_files_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_application::TitleMediaFile>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListMediaFilesForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<scryer_application::TitleMediaSizeSummary>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListTitleMediaSizeSummaries {
                title_ids: title_ids.to_vec(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: scryer_application::MediaFileAnalysis,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateMediaFileAnalysis {
                file_id: file_id.to_string(),
                analysis: Box::new(analysis),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::MarkMediaFileScanFailed {
                file_id: file_id.to_string(),
                error: error.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_media_file_by_id(
        &self,
        file_id: &str,
    ) -> AppResult<Option<scryer_application::TitleMediaFile>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetMediaFileById {
                file_id: file_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteMediaFile {
                file_id: file_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::FindEpisodeByTitleAndNumbers {
                title_id: title_id.to_string(),
                season_number: season_number.to_string(),
                episode_number: episode_number.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::FindEpisodeByTitleAndAbsoluteNumber {
                title_id: title_id.to_string(),
                absolute_number: absolute_number.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn upsert_wanted_item(
        &self,
        item: &scryer_application::WantedItem,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpsertWantedItem {
                item: item.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn ensure_wanted_item_seeded_atomic(
        &self,
        item: scryer_application::WantedItem,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::EnsureWantedItemSeeded {
                item,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
    ) -> AppResult<Vec<scryer_application::WantedItem>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListDueWantedItems {
                now: now.to_string(),
                batch_limit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    #[expect(clippy::too_many_arguments)]
    pub async fn update_wanted_item_status(
        &self,
        id: &str,
        status: &str,
        next_search_at: Option<&str>,
        last_search_at: Option<&str>,
        search_count: i64,
        current_score: Option<i32>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateWantedItemStatus {
                id: id.to_string(),
                status: status.to_string(),
                next_search_at: next_search_at.map(str::to_string),
                last_search_at: last_search_at.map(str::to_string),
                search_count,
                current_score,
                grabbed_release: grabbed_release.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<scryer_application::WantedItem>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetWantedItemForTitle {
                title_id: title_id.to_string(),
                episode_id: episode_id.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ResetFruitlessWantedItems {
                now: now.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteWantedItemsForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn insert_release_decision(
        &self,
        decision: &scryer_application::ReleaseDecision,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertReleaseDecision {
                decision: decision.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_wanted_item_by_id(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_application::WantedItem>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetWantedItemById {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<scryer_application::WantedItem>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListWantedItems {
                status: status.map(str::to_string),
                media_type: media_type.map(str::to_string),
                title_id: title_id.map(str::to_string),
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

    pub async fn count_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
    ) -> AppResult<i64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CountWantedItems {
                status: status.map(str::to_string),
                media_type: media_type.map(str::to_string),
                title_id: title_id.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<scryer_application::ReleaseDecision>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListReleaseDecisionsForTitle {
                title_id: title_id.to_string(),
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<scryer_application::ReleaseDecision>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListReleaseDecisionsForWantedItem {
                wanted_item_id: wanted_item_id.to_string(),
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_download_client_configs(
        &self,
        client_type: Option<String>,
    ) -> AppResult<Vec<DownloadClientConfig>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListDownloadClientConfigs {
                client_type,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_download_client_config(
        &self,
        id: &str,
    ) -> AppResult<Option<DownloadClientConfig>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetDownloadClientConfig {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_download_client_config(
        &self,
        config: DownloadClientConfig,
    ) -> AppResult<DownloadClientConfig> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateDownloadClientConfig {
                config,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_download_client_config(
        &self,
        id: &str,
        name: Option<String>,
        client_type: Option<String>,
        base_url: Option<String>,
        config_json: Option<String>,
        is_enabled: Option<bool>,
    ) -> AppResult<DownloadClientConfig> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateDownloadClientConfig {
                id: id.to_string(),
                name,
                client_type,
                base_url,
                config_json,
                is_enabled,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_download_client_config(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteDownloadClientConfig {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn reorder_download_client_configs(&self, ordered_ids: Vec<String>) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ReorderDownloadClientConfigs {
                ordered_ids,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn insert_pending_release(
        &self,
        release: &scryer_application::PendingRelease,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertPendingRelease {
                release: release.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_expired_pending_releases(
        &self,
        now: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListExpiredPendingReleases {
                now: now.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListPendingReleasesForWantedItem {
                wanted_item_id: wanted_item_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_pending_release_status(
        &self,
        id: &str,
        status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdatePendingReleaseStatus {
                id: id.to_string(),
                status,
                grabbed_at: grabbed_at.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListStandbyPendingReleasesForWantedItem {
                wanted_item_id: wanted_item_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteStandbyPendingReleasesForWantedItem {
                wanted_item_id: wanted_item_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_all_standby_pending_releases(
        &self,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListAllStandbyPendingReleases { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: scryer_application::PendingReleaseStatus,
        next_status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CompareAndSetPendingReleaseStatus {
                id: id.to_string(),
                current_status,
                next_status,
                grabbed_at: grabbed_at.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_waiting_pending_releases(
        &self,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListWaitingPendingReleases { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_pending_release(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetPendingRelease {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::SupersedePendingReleasesForWantedItem {
                wanted_item_id: wanted_item_id.to_string(),
                except_id: except_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeletePendingReleasesForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    // ── Title History ─────────────────────────────────────────────────────────

    #[expect(clippy::too_many_arguments)]
    pub async fn insert_title_history_event(
        &self,
        title_id: String,
        episode_id: Option<String>,
        collection_id: Option<String>,
        event_type: String,
        source_title: Option<String>,
        quality: Option<String>,
        download_id: Option<String>,
        data_json: Option<String>,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertTitleHistoryEvent {
                title_id,
                episode_id,
                collection_id,
                event_type,
                source_title,
                quality,
                download_id,
                data_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_title_history(
        &self,
        event_types: Option<Vec<String>>,
        title_ids: Option<Vec<String>>,
        download_id: Option<String>,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<TitleHistoryRecord>, i64)> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListTitleHistory {
                event_types,
                title_ids,
                download_id,
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

    pub async fn list_title_history_for_title(
        &self,
        title_id: &str,
        event_types: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<TitleHistoryRecord>, i64)> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListTitleHistoryForTitle {
                title_id: title_id.to_string(),
                event_types,
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

    pub async fn list_title_history_for_episode(
        &self,
        episode_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleHistoryRecord>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListTitleHistoryForEpisode {
                episode_id: episode_id.to_string(),
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn find_title_history_by_download_id(
        &self,
        download_id: &str,
    ) -> AppResult<Vec<TitleHistoryRecord>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::FindTitleHistoryByDownloadId {
                download_id: download_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_title_history_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteTitleHistoryForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    // ── Blocklist ─────────────────────────────────────────────────────────────

    #[expect(clippy::too_many_arguments)]
    pub async fn insert_blocklist_entry(
        &self,
        title_id: String,
        source_title: Option<String>,
        source_hint: Option<String>,
        quality: Option<String>,
        download_id: Option<String>,
        reason: Option<String>,
        data_json: Option<String>,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertBlocklistEntry {
                title_id,
                source_title,
                source_hint,
                quality,
                download_id,
                reason,
                data_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_blocklist_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<BlocklistEntry>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListBlocklistForTitle {
                title_id: title_id.to_string(),
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_blocklist_all(
        &self,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListBlocklistAll {
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

    pub async fn delete_blocklist_entry(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteBlocklistEntry {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::IsBlocklisted {
                title_id: title_id.to_string(),
                source_title: source_title.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_blocklist_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteBlocklistForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }
}
