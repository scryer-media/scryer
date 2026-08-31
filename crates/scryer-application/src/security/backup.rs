use super::backup_bundle::{
    BACKUP_ENCRYPTED_EXTENSION, BACKUP_FORMAT_VERSION, BACKUP_PLAINTEXT_EXTENSION,
    BackupBundleExportRequest, BackupExportSecrets, LEGACY_BACKUP_ENCRYPTED_EXTENSION,
    LEGACY_BACKUP_PLAINTEXT_EXTENSION,
};
use super::*;
use crate::domain_events::DomainEventActor;
use crate::types::{BackupStatus, BackupTrigger};
use chrono::TimeZone;
use scryer_domain::{ConfigurationChangeAction, Id};
use semver::Version;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tracing::{error, info, warn};

const BACKUP_METADATA_EXTENSION: &str = ".metadata.json";
const AUTO_BACKUP_CURRENT_VERSION_RETENTION_COUNT: usize = 3;
const AUTO_BACKUP_PREVIOUS_VERSION_RETENTION_COUNT: usize = 1;
const BACKUP_EXECUTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const BACKUP_STALE_TIMEOUT_MINUTES: i64 = 30;
const BACKUP_TIMEOUT_ERROR_MESSAGE: &str = "backup bundle creation timed out after 30 minutes";
const AUTO_BACKUP_INVALID_VERSION_ERROR_MESSAGE: &str =
    "automatic backup was created by an older Scryer version and is no longer valid";

static CURRENT_SCRYER_VERSION: LazyLock<Version> = LazyLock::new(|| {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION must be a valid semver")
});

fn metadata_filename(filename: &str) -> String {
    format!("{filename}{BACKUP_METADATA_EXTENSION}")
}

fn metadata_path(backup_dir: &Path, filename: &str) -> PathBuf {
    backup_dir.join(metadata_filename(filename))
}

fn bundle_path(backup_dir: &Path, filename: &str) -> PathBuf {
    backup_dir.join(filename)
}

fn is_supported_backup_filename(filename: &str) -> bool {
    !filename.contains('/')
        && !filename.contains('\\')
        && (filename.ends_with(BACKUP_PLAINTEXT_EXTENSION)
            || filename.ends_with(BACKUP_ENCRYPTED_EXTENSION)
            || filename.ends_with(LEGACY_BACKUP_PLAINTEXT_EXTENSION)
            || filename.ends_with(LEGACY_BACKUP_ENCRYPTED_EXTENSION))
}

fn build_backup_filename(encrypted: bool) -> String {
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
    let unique = Id::new()
        .0
        .chars()
        .filter(|ch| *ch != '-')
        .take(8)
        .collect::<String>();
    let extension = if encrypted {
        BACKUP_ENCRYPTED_EXTENSION
    } else {
        BACKUP_PLAINTEXT_EXTENSION
    };
    format!("{timestamp}_{unique}{extension}")
}

fn creating_backup_info(
    filename: String,
    created_at: String,
    source_engine: String,
    source_migration_key: Option<String>,
    encrypted: bool,
    trigger: BackupTrigger,
) -> BackupInfo {
    BackupInfo {
        filename,
        size_bytes: 0,
        created_at,
        format_version: BACKUP_FORMAT_VERSION.to_string(),
        source_scryer_version: env!("CARGO_PKG_VERSION").to_string(),
        source_engine,
        source_migration_key,
        encrypted,
        row_counts: BTreeMap::new(),
        trigger,
        status: BackupStatus::Creating,
        error_message: None,
    }
}

fn failed_backup_info(base: BackupInfo, error_message: String) -> BackupInfo {
    BackupInfo {
        status: BackupStatus::Failed,
        error_message: Some(error_message),
        ..base
    }
}

fn backup_timeout_error_message() -> String {
    BACKUP_TIMEOUT_ERROR_MESSAGE.to_string()
}

fn parse_backup_created_at(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn parse_backup_source_version(value: &str) -> Option<Version> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    Version::parse(trimmed.trim_start_matches('v')).ok()
}

fn is_stale_creating_backup(info: &BackupInfo, now_utc: chrono::DateTime<chrono::Utc>) -> bool {
    info.status == BackupStatus::Creating
        && parse_backup_created_at(&info.created_at).is_some_and(|created_at| {
            now_utc.signed_duration_since(created_at)
                >= chrono::Duration::minutes(BACKUP_STALE_TIMEOUT_MINUTES)
        })
}

fn auto_backup_is_invalid_for_current_version(info: &BackupInfo) -> bool {
    if info.trigger != BackupTrigger::Auto {
        return false;
    }

    match parse_backup_source_version(&info.source_scryer_version) {
        Some(version) => version < *CURRENT_SCRYER_VERSION,
        None => true,
    }
}

fn normalize_backup_info(mut info: BackupInfo, backup_dir: &Path) -> BackupInfo {
    let path = bundle_path(backup_dir, &info.filename);
    let now_utc = chrono::Utc::now();
    match info.status {
        BackupStatus::Ready => match std::fs::metadata(&path) {
            Ok(metadata) => {
                info.size_bytes = metadata.len();
                if auto_backup_is_invalid_for_current_version(&info) {
                    info.status = BackupStatus::Invalid;
                    if info.error_message.is_none() {
                        info.error_message =
                            Some(AUTO_BACKUP_INVALID_VERSION_ERROR_MESSAGE.to_string());
                    }
                }
            }
            Err(_) => {
                info.size_bytes = 0;
                info.status = BackupStatus::Failed;
                if info.error_message.is_none() {
                    info.error_message = Some("backup bundle file is missing".to_string());
                }
            }
        },
        BackupStatus::Creating => {
            info.size_bytes = std::fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            if is_stale_creating_backup(&info, now_utc) {
                info.status = BackupStatus::Failed;
                if info.error_message.is_none() {
                    info.error_message = Some(backup_timeout_error_message());
                }
            }
        }
        BackupStatus::Invalid => {
            info.size_bytes = std::fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
        }
        BackupStatus::Failed => {
            info.size_bytes = std::fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
        }
    }
    info
}

fn list_backup_files(backup_dir: &Path) -> Vec<BackupInfo> {
    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(backup_dir) else {
        return entries;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(bundle_filename) = filename.strip_suffix(BACKUP_METADATA_EXTENSION) else {
            continue;
        };
        if !is_supported_backup_filename(bundle_filename) {
            continue;
        }

        match std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<BackupInfo>(&bytes).ok())
        {
            Some(info) => entries.push(normalize_backup_info(info, backup_dir)),
            None => warn!(path = %path.display(), "failed to load backup metadata entry"),
        }
    }

    entries.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.filename.cmp(&a.filename))
    });
    entries
}

fn write_backup_metadata(backup_dir: &Path, info: &BackupInfo) -> AppResult<()> {
    let path = metadata_path(backup_dir, &info.filename);
    let temp_path = path.with_extension("metadata.json.tmp");
    let payload = serde_json::to_vec_pretty(info).map_err(|error| {
        AppError::Repository(format!("failed to encode backup metadata: {error}"))
    })?;

    std::fs::write(&temp_path, payload).map_err(|error| {
        AppError::Repository(format!("failed to write backup metadata: {error}"))
    })?;
    ensure_owner_only_permissions(&temp_path)?;
    std::fs::rename(&temp_path, &path).map_err(|error| {
        AppError::Repository(format!("failed to finalize backup metadata: {error}"))
    })?;
    ensure_owner_only_permissions(&path)?;
    Ok(())
}

fn auto_backup_filenames_to_prune(
    entries: &[BackupInfo],
    current_version_retention_count: usize,
    previous_version_retention_count: usize,
) -> Vec<String> {
    let current_version = &*CURRENT_SCRYER_VERSION;
    let mut retained_filenames = BTreeSet::new();

    let mut current_version_entries = entries
        .iter()
        .filter(|entry| {
            entry.trigger == BackupTrigger::Auto
                && entry.status == BackupStatus::Ready
                && parse_backup_source_version(&entry.source_scryer_version)
                    .is_some_and(|version| version == *current_version)
        })
        .collect::<Vec<_>>();
    current_version_entries.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.filename.cmp(&a.filename))
    });
    retained_filenames.extend(
        current_version_entries
            .into_iter()
            .take(current_version_retention_count)
            .map(|entry| entry.filename.clone()),
    );

    let previous_version = entries
        .iter()
        .filter(|entry| {
            entry.trigger == BackupTrigger::Auto
                && matches!(entry.status, BackupStatus::Ready | BackupStatus::Invalid)
        })
        .filter_map(|entry| parse_backup_source_version(&entry.source_scryer_version))
        .filter(|version| version < current_version)
        .max();
    if let Some(previous_version) = previous_version {
        let mut previous_version_entries = entries
            .iter()
            .filter(|entry| {
                entry.trigger == BackupTrigger::Auto
                    && matches!(entry.status, BackupStatus::Ready | BackupStatus::Invalid)
                    && parse_backup_source_version(&entry.source_scryer_version)
                        .is_some_and(|version| version == previous_version)
            })
            .collect::<Vec<_>>();
        previous_version_entries.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.filename.cmp(&a.filename))
        });
        retained_filenames.extend(
            previous_version_entries
                .into_iter()
                .take(previous_version_retention_count)
                .map(|entry| entry.filename.clone()),
        );
    }

    entries
        .iter()
        .filter(|entry| {
            if entry.trigger != BackupTrigger::Auto
                || !matches!(entry.status, BackupStatus::Ready | BackupStatus::Invalid)
            {
                return false;
            }

            let Some(source_version) = parse_backup_source_version(&entry.source_scryer_version)
            else {
                return false;
            };

            source_version < *current_version
                || (source_version == *current_version && entry.status == BackupStatus::Ready)
        })
        .filter(|entry| !retained_filenames.contains(&entry.filename))
        .map(|entry| entry.filename.clone())
        .collect()
}

fn has_ready_current_auto_backup(entries: &[BackupInfo], filename: &str) -> bool {
    entries.iter().any(|entry| {
        entry.filename == filename
            && entry.trigger == BackupTrigger::Auto
            && entry.status == BackupStatus::Ready
            && parse_backup_source_version(&entry.source_scryer_version)
                .is_some_and(|version| version == *CURRENT_SCRYER_VERSION)
    })
}

fn has_creating_backup_for_trigger(entries: &[BackupInfo], trigger: BackupTrigger) -> bool {
    entries
        .iter()
        .any(|entry| entry.status == BackupStatus::Creating && entry.trigger == trigger)
}

fn remove_backup_artifacts(backup_dir: &Path, filename: &str) -> AppResult<bool> {
    let bundle = bundle_path(backup_dir, filename);
    let metadata = metadata_path(backup_dir, filename);
    let bundle_exists = bundle.exists();
    let metadata_exists = metadata.exists();
    if !bundle_exists && !metadata_exists {
        return Ok(false);
    }

    if bundle_exists {
        std::fs::remove_file(&bundle).map_err(|error| {
            AppError::Repository(format!("failed to delete backup bundle: {error}"))
        })?;
    }
    if metadata_exists {
        std::fs::remove_file(&metadata).map_err(|error| {
            AppError::Repository(format!("failed to delete backup metadata: {error}"))
        })?;
    }

    Ok(true)
}

fn cleanup_partial_backup_bundle(backup_dir: &Path, filename: &str) {
    let _ = std::fs::remove_file(bundle_path(backup_dir, filename));
}

#[cfg(unix)]
fn ensure_owner_only_permissions(path: &Path) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;

    if !path.exists() {
        return Ok(());
    }

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|error| {
        AppError::Repository(format!("failed to set backup permissions: {error}"))
    })?;
    Ok(())
}

#[cfg(not(unix))]
fn ensure_owner_only_permissions(_path: &Path) -> AppResult<()> {
    Ok(())
}

fn validate_backup_passphrase(passphrase: &str) -> AppResult<()> {
    if passphrase.trim().is_empty() {
        return Err(AppError::Validation(
            "backup password is required for full backups".to_string(),
        ));
    }

    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Backup export wiring carries all export inputs explicitly"
)]
async fn export_backup_file(
    exporter: Arc<dyn LogicalBackupExporter>,
    backup_dir: &Path,
    filename: &str,
    passphrase: &str,
    source_engine: String,
    source_migration_key: Option<String>,
    secrets: BackupExportSecrets,
    trigger: BackupTrigger,
) -> AppResult<BackupInfo> {
    let output_path = bundle_path(backup_dir, filename);
    let outcome = exporter
        .export_backup_bundle(BackupBundleExportRequest {
            output_path: output_path.clone(),
            passphrase: passphrase.to_string(),
            source_migration_key,
            source_scryer_version: env!("CARGO_PKG_VERSION").to_string(),
            source_engine,
            secrets,
        })
        .await?;

    let size_bytes = std::fs::metadata(&output_path)
        .map_err(|error| AppError::Repository(format!("failed to stat backup bundle: {error}")))?
        .len();
    let summary = outcome.summary;

    Ok(BackupInfo {
        filename: filename.to_string(),
        size_bytes,
        created_at: summary.created_at,
        format_version: summary.format_version,
        source_scryer_version: summary.source_scryer_version,
        source_engine: summary.source_engine,
        source_migration_key: summary.source_migration_key,
        encrypted: summary.encrypted,
        row_counts: summary.row_counts,
        trigger,
        status: BackupStatus::Ready,
        error_message: None,
    })
}

async fn run_backup_operation_with_timeout<F, T>(
    timeout: std::time::Duration,
    future: F,
) -> AppResult<T>
where
    F: Future<Output = AppResult<T>>,
{
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| AppError::Repository(backup_timeout_error_message()))?
}

impl AppUseCase {
    async fn collect_backup_export_secrets(&self) -> AppResult<BackupExportSecrets> {
        let encryption_master_key = self
            .services
            .config
            .system_info
            .current_encryption_key_base64()
            .await?
            .ok_or_else(|| {
                AppError::Validation(
                    "backup export requires a configured encryption master key".into(),
                )
            })?;

        Ok(BackupExportSecrets {
            encryption_master_key,
            jwt_signing_secret: self.auth.jwt_signing_salt.clone(),
            smg_registration_secret: self.services.config.smg_registration_secret.clone(),
            smg_gateway_url: self.services.config.smg_gateway_url.clone(),
        })
    }

    async fn prepare_backup_request(
        &self,
        trigger: BackupTrigger,
        encrypted: bool,
    ) -> AppResult<PreparedBackupRequest> {
        let dir = self.effective_backup_dir().await?;
        std::fs::create_dir_all(&dir).map_err(|error| {
            AppError::Repository(format!("failed to create backup directory: {error}"))
        })?;

        let datastore_info = self.services.config.system_info.datastore_info().await?;
        let queued = creating_backup_info(
            build_backup_filename(encrypted),
            chrono::Utc::now().to_rfc3339(),
            datastore_info.engine.clone(),
            datastore_info.current_migration_key.clone(),
            encrypted,
            trigger,
        );
        write_backup_metadata(&dir, &queued)?;
        self.publish_settings_changed(vec!["backup".to_string()]);

        Ok(PreparedBackupRequest {
            dir,
            queued,
            source_engine: datastore_info.engine,
            source_migration_key: datastore_info.current_migration_key,
        })
    }

    async fn complete_backup_request(
        &self,
        actor: impl Into<DomainEventActor>,
        prepared: PreparedBackupRequest,
        passphrase: String,
    ) -> AppResult<BackupInfo> {
        let filename = prepared.queued.filename.clone();
        let trigger = prepared.queued.trigger;
        let result = run_backup_operation_with_timeout(BACKUP_EXECUTION_TIMEOUT, async {
            validate_backup_passphrase(&passphrase)?;
            let secrets = self.collect_backup_export_secrets().await?;
            export_backup_file(
                self.services.config.logical_backup_exporter.clone(),
                &prepared.dir,
                &filename,
                &passphrase,
                prepared.source_engine.clone(),
                prepared.source_migration_key.clone(),
                secrets,
                trigger,
            )
            .await
        })
        .await;

        let next_info = match &result {
            Ok(info) => {
                info!(
                    filename = %info.filename,
                    size_bytes = info.size_bytes,
                    encrypted = info.encrypted,
                    trigger = info.trigger.as_str(),
                    "backup bundle created"
                );
                info.clone()
            }
            Err(error) => {
                let message = error.to_string();
                cleanup_partial_backup_bundle(&prepared.dir, &filename);
                error!(
                    filename = %filename,
                    error = %message,
                    trigger = trigger.as_str(),
                    "backup bundle creation failed"
                );
                failed_backup_info(prepared.queued.clone(), message)
            }
        };

        if let Err(error) = write_backup_metadata(&prepared.dir, &next_info) {
            error!(
                filename = %filename,
                error = %error,
                "failed to persist backup bundle metadata"
            );
        }

        self.emit_configuration_changed_event(
            actor,
            "backup",
            Some(filename),
            ConfigurationChangeAction::Saved,
        )
        .await;
        self.publish_settings_changed(vec!["backup".to_string()]);

        match result {
            Ok(_) => Ok(next_info),
            Err(error) => Err(error),
        }
    }

    async fn create_backup_inline(
        &self,
        actor: impl Into<DomainEventActor>,
        trigger: BackupTrigger,
        passphrase: &str,
    ) -> AppResult<BackupInfo> {
        validate_backup_passphrase(passphrase)?;
        let prepared = self.prepare_backup_request(trigger, true).await?;
        info!(
            filename = %prepared.queued.filename,
            encrypted = prepared.queued.encrypted,
            trigger = prepared.queued.trigger.as_str(),
            "backup bundle starting"
        );
        self.complete_backup_request(actor, prepared, passphrase.to_string())
            .await
    }

    pub async fn create_backup(&self, actor: &User, passphrase: &str) -> AppResult<BackupInfo> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        validate_backup_passphrase(passphrase)?;

        let Some(execution_guard) = self
            .runtime
            .jobs
            .backup_execution_guards
            .try_acquire(BackupTrigger::Manual.as_str())
            .await
        else {
            return Err(AppError::Validation(
                "a manual backup is already running".to_string(),
            ));
        };

        let backup_dir = self.effective_backup_dir().await?;
        if has_creating_backup_for_trigger(&list_backup_files(&backup_dir), BackupTrigger::Manual) {
            return Err(AppError::Validation(
                "a manual backup is already running".to_string(),
            ));
        }

        let prepared = self
            .prepare_backup_request(BackupTrigger::Manual, true)
            .await?;
        let queued = prepared.queued.clone();
        info!(
            filename = %queued.filename,
            encrypted = queued.encrypted,
            trigger = queued.trigger.as_str(),
            "backup bundle scheduled"
        );

        let app = self.clone();
        let actor_event = DomainEventActor::from(actor);
        let passphrase_for_task = passphrase.to_string();
        tokio::spawn(async move {
            let _execution_guard = execution_guard;
            if let Err(error) = app
                .complete_backup_request(actor_event, prepared, passphrase_for_task)
                .await
            {
                warn!(error = %error, "manual backup bundle task failed");
            }
        });

        Ok(queued)
    }

    pub async fn list_backups(&self, actor: &User) -> AppResult<Vec<BackupInfo>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let backup_dir = self.effective_backup_dir().await?;
        Ok(list_backup_files(&backup_dir))
    }

    pub async fn prepare_backup_download(
        &self,
        actor: &User,
        filename: &str,
    ) -> AppResult<crate::BackupDownloadTicket> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        if !is_supported_backup_filename(filename) {
            return Err(AppError::Validation("invalid backup filename".into()));
        }

        let backup_dir = self.effective_backup_dir().await?;
        let info = list_backup_files(&backup_dir)
            .into_iter()
            .find(|entry| entry.filename == filename)
            .ok_or_else(|| AppError::NotFound("backup metadata could not be found".into()))?;

        match info.status {
            BackupStatus::Creating => {
                return Err(AppError::Validation(
                    "backup bundle is still being created".into(),
                ));
            }
            BackupStatus::Failed => {
                return Err(AppError::Validation(
                    info.error_message
                        .unwrap_or_else(|| "backup bundle creation failed".to_string()),
                ));
            }
            BackupStatus::Invalid => {
                return Err(AppError::Validation(
                    info.error_message
                        .unwrap_or_else(|| "backup bundle is invalid".to_string()),
                ));
            }
            BackupStatus::Ready => {}
        }

        let bundle = bundle_path(&backup_dir, filename);
        if !bundle.is_file() {
            return Err(AppError::NotFound(
                "backup bundle could not be found".into(),
            ));
        }

        self.issue_backup_download_token(actor, filename).await
    }

    pub async fn delete_backup(&self, actor: &User, filename: &str) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        if !is_supported_backup_filename(filename) {
            return Err(AppError::Validation("invalid backup filename".into()));
        }

        let backup_dir = self.effective_backup_dir().await?;
        let deleted = remove_backup_artifacts(&backup_dir, filename)?;
        if !deleted {
            return Ok(false);
        }

        info!(filename, "backup deleted");
        self.emit_configuration_changed_event(
            actor,
            "backup",
            Some(filename.to_string()),
            ConfigurationChangeAction::Deleted,
        )
        .await;
        self.publish_settings_changed(vec!["backup".to_string()]);
        Ok(true)
    }

    async fn enforce_auto_backup_retention(&self, created_filename: &str) -> AppResult<u32> {
        let dir = self.effective_backup_dir().await?;
        let entries = list_backup_files(&dir);
        if !has_ready_current_auto_backup(&entries, created_filename) {
            warn!(
                filename = %created_filename,
                "skipping automatic backup retention because the new backup is not ready"
            );
            return Ok(0);
        }
        let mut deleted = 0u32;

        for filename in auto_backup_filenames_to_prune(
            &entries,
            AUTO_BACKUP_CURRENT_VERSION_RETENTION_COUNT,
            AUTO_BACKUP_PREVIOUS_VERSION_RETENTION_COUNT,
        ) {
            match remove_backup_artifacts(&dir, &filename) {
                Ok(true) => deleted += 1,
                Ok(false) => {}
                Err(error) => warn!(
                    filename = %filename,
                    error = %error,
                    "failed to remove old backup"
                ),
            }
        }

        if deleted > 0 {
            info!(deleted, "old backups pruned by retention policy");
        }
        Ok(deleted)
    }

    pub async fn auto_backup_settings(&self) -> AppResult<crate::AutoBackupSettings> {
        self.load_auto_backup_settings().await
    }

    pub(crate) async fn run_auto_backup_job(&self) -> AppResult<AutoBackupRunOutcome> {
        let settings = self.auto_backup_settings().await?;
        if !settings.enabled {
            return Ok(AutoBackupRunOutcome::Skipped {
                reason: "Automatic backups are disabled".to_string(),
            });
        }

        let Some(_execution_guard) = self
            .runtime
            .jobs
            .backup_execution_guards
            .try_acquire(BackupTrigger::Auto.as_str())
            .await
        else {
            return Ok(AutoBackupRunOutcome::Skipped {
                reason: "Skipped because another automatic backup is already running".to_string(),
            });
        };

        let dir = self.effective_backup_dir().await?;
        let entries = list_backup_files(&dir);
        if has_creating_backup_for_trigger(&entries, BackupTrigger::Auto) {
            return Ok(AutoBackupRunOutcome::Skipped {
                reason: "Skipped because another automatic backup is already running".to_string(),
            });
        }

        let passphrase = self
            .read_setting_string_value(AUTO_BACKUP_KEY_KEY, None)
            .await?
            .filter(|value| !value.trim().is_empty());
        let Some(passphrase) = passphrase else {
            return Ok(AutoBackupRunOutcome::Skipped {
                reason: "Skipped because automatic backup key is not configured".to_string(),
            });
        };
        let info = self
            .create_backup_inline(None, BackupTrigger::Auto, &passphrase)
            .await?;
        let pruned_count = self.enforce_auto_backup_retention(&info.filename).await?;

        Ok(AutoBackupRunOutcome::Created { info, pruned_count })
    }
}

#[derive(Clone, Debug)]
struct PreparedBackupRequest {
    dir: PathBuf,
    queued: BackupInfo,
    source_engine: String,
    source_migration_key: Option<String>,
}

#[derive(Clone, Debug)]
pub enum AutoBackupRunOutcome {
    Created { info: BackupInfo, pruned_count: u32 },
    Skipped { reason: String },
}

fn parse_daily_time_local(value: &str) -> AppResult<(u32, u32)> {
    let (hour, minute) = value
        .trim()
        .split_once(':')
        .ok_or_else(|| AppError::Validation("daily time must use HH:MM format".to_string()))?;
    let hour = hour
        .parse::<u32>()
        .map_err(|_| AppError::Validation("daily time hour must be numeric".to_string()))?;
    let minute = minute
        .parse::<u32>()
        .map_err(|_| AppError::Validation("daily time minute must be numeric".to_string()))?;
    if hour > 23 || minute > 59 {
        return Err(AppError::Validation(
            "daily time must be between 00:00 and 23:59".to_string(),
        ));
    }
    Ok((hour, minute))
}

fn resolve_local_scheduled_time(
    date: chrono::NaiveDate,
    hour: u32,
    minute: u32,
) -> Option<chrono::DateTime<chrono::Local>> {
    let naive = date.and_hms_opt(hour, minute, 0)?;
    for minute_offset in 0..=180 {
        let candidate = naive + chrono::Duration::minutes(minute_offset);
        match chrono::Local.from_local_datetime(&candidate) {
            chrono::LocalResult::Single(value) => return Some(value),
            chrono::LocalResult::Ambiguous(first, second) => {
                return Some(if first <= second { first } else { second });
            }
            chrono::LocalResult::None => continue,
        }
    }
    None
}

pub(crate) fn compute_next_auto_backup_run_at(
    daily_time_local: &str,
    now_utc: chrono::DateTime<chrono::Utc>,
) -> AppResult<chrono::DateTime<chrono::Utc>> {
    let (hour, minute) = parse_daily_time_local(daily_time_local)?;
    let now_local = now_utc.with_timezone(&chrono::Local);
    let today = now_local.date_naive();
    let today_run = resolve_local_scheduled_time(today, hour, minute).ok_or_else(|| {
        AppError::Validation("failed to resolve the configured local backup time".to_string())
    })?;
    if today_run >= now_local {
        return Ok(today_run.with_timezone(&chrono::Utc));
    }

    let tomorrow = today
        .succ_opt()
        .ok_or_else(|| AppError::Validation("failed to compute next backup day".to_string()))?;
    let tomorrow_run = resolve_local_scheduled_time(tomorrow, hour, minute).ok_or_else(|| {
        AppError::Validation("failed to resolve the configured local backup time".to_string())
    })?;
    Ok(tomorrow_run.with_timezone(&chrono::Utc))
}

async fn load_auto_backup_scheduler_settings(
    app: &AppUseCase,
) -> Option<crate::AutoBackupSettings> {
    match app.auto_backup_settings().await {
        Ok(settings) => Some(settings),
        Err(error) => {
            warn!(error = %error, "failed to load automatic backup settings");
            None
        }
    }
}

async fn schedule_auto_backup_job(
    app: &AppUseCase,
    settings: Option<&crate::AutoBackupSettings>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let settings = match settings {
        Some(settings) if settings.enabled => settings,
        _ => {
            app.clear_job_next_run_at(JobKey::AutoBackup).await;
            return None;
        }
    };

    let next_run_at =
        match compute_next_auto_backup_run_at(&settings.daily_time_local, chrono::Utc::now()) {
            Ok(next_run_at) => next_run_at,
            Err(error) => {
                warn!(error = %error, "failed to schedule automatic backup job");
                app.clear_job_next_run_at(JobKey::AutoBackup).await;
                return None;
            }
        };
    app.set_job_next_run_at(JobKey::AutoBackup, next_run_at)
        .await;
    Some(next_run_at)
}

fn should_reload_auto_backup_scheduler(
    changed: Result<Vec<String>, tokio::sync::broadcast::error::RecvError>,
) -> bool {
    match changed {
        Ok(keys) => keys.iter().any(|key| {
            key == AUTO_BACKUP_ENABLED_KEY
                || key == AUTO_BACKUP_DAILY_TIME_LOCAL_KEY
                || key == AUTO_BACKUP_KEY_KEY
        }),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
            info!(
                skipped,
                "automatic backup scheduler lagged settings updates"
            );
            true
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => false,
    }
}

pub async fn start_background_auto_backup_scheduler(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    info!("automatic backup scheduler started");
    let mut settings_changed = app.runtime.events.settings_changed_broadcast.subscribe();
    let mut settings = load_auto_backup_scheduler_settings(&app).await;
    let mut next_run_at = schedule_auto_backup_job(&app, settings.as_ref()).await;

    loop {
        if let Some(when) = next_run_at {
            let delay = when
                .signed_duration_since(chrono::Utc::now())
                .to_std()
                .unwrap_or_default();
            tokio::select! {
                _ = token.cancelled() => {
                    info!("automatic backup scheduler shutting down");
                    app.clear_job_next_run_at(JobKey::AutoBackup).await;
                    return;
                }
                changed = settings_changed.recv() => {
                    if !should_reload_auto_backup_scheduler(changed) {
                        continue;
                    }
                    settings = load_auto_backup_scheduler_settings(&app).await;
                    next_run_at = schedule_auto_backup_job(&app, settings.as_ref()).await;
                }
                _ = tokio::time::sleep(delay) => {
                    if let Err(error) = app
                        .run_scheduled_job_now(JobKey::AutoBackup, JobTriggerSource::ScheduledDaily)
                        .await
                    {
                        warn!(error = %error, "automatic backup job failed");
                    }
                    settings = load_auto_backup_scheduler_settings(&app).await;
                    next_run_at = schedule_auto_backup_job(&app, settings.as_ref()).await;
                }
            }
        } else {
            tokio::select! {
                _ = token.cancelled() => {
                    info!("automatic backup scheduler shutting down");
                    app.clear_job_next_run_at(JobKey::AutoBackup).await;
                    return;
                }
                changed = settings_changed.recv() => {
                    if !should_reload_auto_backup_scheduler(changed) {
                        continue;
                    }
                    settings = load_auto_backup_scheduler_settings(&app).await;
                    next_run_at = schedule_auto_backup_job(&app, settings.as_ref()).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn backup_info(
        filename: &str,
        created_at: &str,
        trigger: BackupTrigger,
        status: BackupStatus,
    ) -> BackupInfo {
        BackupInfo {
            filename: filename.to_string(),
            size_bytes: 0,
            created_at: created_at.to_string(),
            format_version: BACKUP_FORMAT_VERSION.to_string(),
            source_scryer_version: env!("CARGO_PKG_VERSION").to_string(),
            source_engine: "sqlite".to_string(),
            source_migration_key: None,
            encrypted: false,
            row_counts: BTreeMap::new(),
            trigger,
            status,
            error_message: None,
        }
    }

    fn backup_info_with_version(
        filename: &str,
        created_at: &str,
        trigger: BackupTrigger,
        status: BackupStatus,
        source_scryer_version: &str,
    ) -> BackupInfo {
        let mut info = backup_info(filename, created_at, trigger, status);
        info.source_scryer_version = source_scryer_version.to_string();
        info
    }

    #[test]
    fn compute_next_auto_backup_run_at_uses_today_when_before_scheduled_time() {
        let today = chrono::Local::now().date_naive();
        let scheduled = resolve_local_scheduled_time(today, 6, 30).expect("local schedule");
        let now_utc = (scheduled - chrono::Duration::minutes(10)).with_timezone(&chrono::Utc);

        let next = compute_next_auto_backup_run_at("06:30", now_utc).expect("next run");

        assert_eq!(next, scheduled.with_timezone(&chrono::Utc));
    }

    #[test]
    fn compute_next_auto_backup_run_at_keeps_exact_scheduled_time_on_same_day() {
        let today = chrono::Local::now().date_naive();
        let scheduled = resolve_local_scheduled_time(today, 6, 30).expect("local schedule");
        let now_utc = scheduled.with_timezone(&chrono::Utc);

        let next = compute_next_auto_backup_run_at("06:30", now_utc).expect("next run");

        assert_eq!(next, scheduled.with_timezone(&chrono::Utc));
    }

    #[test]
    fn compute_next_auto_backup_run_at_rolls_forward_after_scheduled_time() {
        let today = chrono::Local::now().date_naive();
        let scheduled = resolve_local_scheduled_time(today, 6, 30).expect("local schedule");
        let tomorrow = resolve_local_scheduled_time(today.succ_opt().expect("tomorrow"), 6, 30)
            .expect("tomorrow schedule");
        let now_utc = (scheduled + chrono::Duration::minutes(10)).with_timezone(&chrono::Utc);

        let next = compute_next_auto_backup_run_at("06:30", now_utc).expect("next run");

        assert_eq!(next, tomorrow.with_timezone(&chrono::Utc));
    }

    #[test]
    fn auto_backup_filenames_to_prune_keeps_current_and_previous_version_backups() {
        let entries = vec![
            backup_info(
                "current-04.sbk",
                "2026-05-14T10:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Ready,
            ),
            backup_info(
                "current-03.sbk",
                "2026-05-14T09:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Ready,
            ),
            backup_info(
                "current-02.sbk",
                "2026-05-14T08:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Ready,
            ),
            backup_info(
                "current-01.sbk",
                "2026-05-14T07:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Ready,
            ),
            backup_info_with_version(
                "previous-new.sbk",
                "2026-05-14T06:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Invalid,
                "0.0.2",
            ),
            backup_info_with_version(
                "previous-old.sbk",
                "2026-05-14T05:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Invalid,
                "0.0.2",
            ),
            backup_info_with_version(
                "older-version.sbk",
                "2026-05-14T04:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Invalid,
                "0.0.1",
            ),
            backup_info_with_version(
                "manual.sbk",
                "2026-05-14T03:30:00Z",
                BackupTrigger::Manual,
                BackupStatus::Invalid,
                "0.0.2",
            ),
            backup_info_with_version(
                "failed.sbk",
                "2026-05-14T03:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Failed,
                "0.0.2",
            ),
            backup_info_with_version(
                "creating.sbk",
                "2026-05-14T02:30:00Z",
                BackupTrigger::Auto,
                BackupStatus::Creating,
                "0.0.2",
            ),
            backup_info_with_version(
                "malformed-version.sbk",
                "2026-05-14T02:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Invalid,
                "not-a-version",
            ),
            backup_info_with_version(
                "newer-version.sbk",
                "2026-05-14T01:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Ready,
                "999.0.0",
            ),
        ];

        let pruned = auto_backup_filenames_to_prune(&entries, 3, 1);

        assert_eq!(
            pruned,
            vec![
                "current-01.sbk".to_string(),
                "previous-old.sbk".to_string(),
                "older-version.sbk".to_string(),
            ],
        );
    }

    #[test]
    fn auto_backup_retention_requires_new_backup_to_be_persisted_as_ready() {
        let ready = backup_info(
            "ready.sbk",
            "2026-05-14T06:00:00Z",
            BackupTrigger::Auto,
            BackupStatus::Ready,
        );
        let creating = backup_info(
            "creating.sbk",
            "2026-05-14T05:00:00Z",
            BackupTrigger::Auto,
            BackupStatus::Creating,
        );
        let manual = backup_info(
            "manual.sbk",
            "2026-05-14T04:00:00Z",
            BackupTrigger::Manual,
            BackupStatus::Ready,
        );
        let entries = vec![ready, creating, manual];

        assert!(has_ready_current_auto_backup(&entries, "ready.sbk"));
        assert!(!has_ready_current_auto_backup(&entries, "creating.sbk"));
        assert!(!has_ready_current_auto_backup(&entries, "manual.sbk"));
        assert!(!has_ready_current_auto_backup(&entries, "missing.sbk"));
    }

    #[test]
    fn retention_prune_reconciles_legacy_backup_artifacts_without_touching_unrelated_files() {
        let dir = tempdir().expect("tempdir");
        let foreign_file = dir.path().join("notes.txt");
        std::fs::write(&foreign_file, b"not a backup").expect("write foreign file");
        let entries = vec![
            backup_info(
                "current.sbk",
                "2026-05-14T06:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Ready,
            ),
            backup_info_with_version(
                "previous.sbk",
                "2026-05-14T05:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Invalid,
                "0.0.2",
            ),
            backup_info_with_version(
                "oldest.sbk",
                "2026-05-14T04:00:00Z",
                BackupTrigger::Auto,
                BackupStatus::Invalid,
                "0.0.1",
            ),
        ];
        for entry in &entries {
            std::fs::write(dir.path().join(&entry.filename), b"backup").expect("write backup");
            write_backup_metadata(dir.path(), entry).expect("write metadata");
        }

        for filename in auto_backup_filenames_to_prune(&entries, 3, 1) {
            remove_backup_artifacts(dir.path(), &filename).expect("remove backup");
        }

        assert!(foreign_file.exists());
        assert!(dir.path().join("current.sbk").exists());
        assert!(dir.path().join(metadata_filename("current.sbk")).exists());
        assert!(dir.path().join("previous.sbk").exists());
        assert!(dir.path().join(metadata_filename("previous.sbk")).exists());
        assert!(!dir.path().join("oldest.sbk").exists());
        assert!(!dir.path().join(metadata_filename("oldest.sbk")).exists());
    }

    #[test]
    fn build_backup_filename_is_unique_for_overlapping_starts() {
        let first = build_backup_filename(false);
        let second = build_backup_filename(false);

        assert_ne!(first, second);
        assert!(!first.starts_with("scryer_backup_"));
        assert!(first.ends_with(BACKUP_PLAINTEXT_EXTENSION));
        assert!(first.len() < "scryer_backup_20260514_231046_832_47f908fa484345b790ffef21b9aaa743.scryer-backup.enc".len());
    }

    #[test]
    fn normalize_backup_info_marks_stale_creating_backups_as_failed() {
        let dir = tempdir().expect("tempdir");
        let created_at = (chrono::Utc::now()
            - chrono::Duration::minutes(BACKUP_STALE_TIMEOUT_MINUTES + 1))
        .to_rfc3339();

        let normalized = normalize_backup_info(
            backup_info(
                "stale.sbk",
                &created_at,
                BackupTrigger::Auto,
                BackupStatus::Creating,
            ),
            dir.path(),
        );

        assert_eq!(normalized.status, BackupStatus::Failed);
        assert_eq!(
            normalized.error_message.as_deref(),
            Some(BACKUP_TIMEOUT_ERROR_MESSAGE),
        );
    }

    #[test]
    fn normalize_backup_info_marks_older_auto_backups_as_invalid() {
        let dir = tempdir().expect("tempdir");
        let bundle_path = dir.path().join("auto-old.sbk");
        std::fs::write(&bundle_path, b"bundle").expect("bundle");
        let mut info = backup_info(
            "auto-old.sbk",
            "2026-05-14T00:00:00Z",
            BackupTrigger::Auto,
            BackupStatus::Ready,
        );
        info.source_scryer_version = "0.0.1".to_string();

        let normalized = normalize_backup_info(info, dir.path());

        assert_eq!(normalized.status, BackupStatus::Invalid);
        assert_eq!(
            normalized.error_message.as_deref(),
            Some(AUTO_BACKUP_INVALID_VERSION_ERROR_MESSAGE),
        );
    }

    #[test]
    fn normalize_backup_info_keeps_current_manual_backups_ready() {
        let dir = tempdir().expect("tempdir");
        let bundle_path = dir.path().join("manual-current.sbk");
        std::fs::write(&bundle_path, b"bundle").expect("bundle");

        let normalized = normalize_backup_info(
            backup_info(
                "manual-current.sbk",
                "2026-05-14T00:00:00Z",
                BackupTrigger::Manual,
                BackupStatus::Ready,
            ),
            dir.path(),
        );

        assert_eq!(normalized.status, BackupStatus::Ready);
        assert_eq!(normalized.error_message, None);
    }

    #[test]
    fn has_creating_backup_for_trigger_ignores_other_backup_triggers() {
        let entries = vec![backup_info(
            "manual.sbk",
            "2026-05-14T05:00:00Z",
            BackupTrigger::Manual,
            BackupStatus::Creating,
        )];

        assert!(!has_creating_backup_for_trigger(
            &entries,
            BackupTrigger::Auto
        ));
        assert!(has_creating_backup_for_trigger(
            &entries,
            BackupTrigger::Manual,
        ));
    }

    #[tokio::test]
    async fn run_backup_operation_with_timeout_returns_timeout_error() {
        let error = run_backup_operation_with_timeout(std::time::Duration::from_millis(5), async {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            Ok::<_, AppError>(())
        })
        .await
        .expect_err("slow backup task should time out");

        assert!(error.to_string().contains(BACKUP_TIMEOUT_ERROR_MESSAGE));
    }

    #[test]
    fn cleanup_partial_backup_bundle_removes_existing_bundle() {
        let dir = tempdir().expect("tempdir");
        let filename = build_backup_filename(false);
        let path = bundle_path(dir.path(), &filename);
        std::fs::write(&path, b"partial bundle").expect("write bundle");

        cleanup_partial_backup_bundle(dir.path(), &filename);

        assert!(!path.exists(), "partial bundle should be removed");
    }
}
