use std::future::Future;
use std::path::{Path, PathBuf};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path as AxumPath, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use scryer_application::disk::{
    Access, Disk, IfPresent, Open, OwnerOnly, Parents, PathReading, Remove, Replace,
};
use scryer_application::{AppError, AppUseCase, BackupInfo, BackupStatus};
use scryer_infrastructure_runtime::{
    DatastoreConfig, DatastoreEngine, datastore_file_path,
    restore_prepared_backup_directory_to_datastore,
};
use serde::Serialize;
use tokio_util::io::ReaderStream;

use crate::middleware::map_app_error;

const BACKUP_METADATA_EXTENSION: &str = ".metadata.json";
const BACKUP_PLAINTEXT_EXTENSION: &str = ".tar.zst";
const BACKUP_ENCRYPTED_EXTENSION: &str = ".enc";
const LEGACY_BACKUP_PLAINTEXT_EXTENSION: &str = ".scryer-backup.tar.zst";
const LEGACY_BACKUP_ENCRYPTED_EXTENSION: &str = ".scryer-backup.enc";
const INSTANCE_SECRETS_ENV_FILENAME: &str = "instance-secrets.env";
const PENDING_RESTORE_DB_FILENAME: &str = "restored-scryer.db";
const PENDING_RESTORE_PREPARED_BUNDLE_DIRNAME: &str = "prepared-bundle";
const PENDING_RESTORE_DIRNAME: &str = "restore-pending";
const PENDING_RESTORE_READY_FILENAME: &str = "restore-ready";

#[derive(Clone)]
pub(crate) struct BackupRouteState {
    pub(crate) app: AppUseCase,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

fn pending_restore_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(PENDING_RESTORE_DIRNAME)
}

fn pending_restore_db_path(data_dir: &Path) -> PathBuf {
    pending_restore_dir(data_dir).join(PENDING_RESTORE_DB_FILENAME)
}

fn pending_restore_prepared_bundle_dir(data_dir: &Path) -> PathBuf {
    pending_restore_dir(data_dir).join(PENDING_RESTORE_PREPARED_BUNDLE_DIRNAME)
}

fn pending_restore_instance_secrets_path(data_dir: &Path) -> PathBuf {
    pending_restore_dir(data_dir).join(INSTANCE_SECRETS_ENV_FILENAME)
}

fn pending_restore_ready_path(data_dir: &Path) -> PathBuf {
    pending_restore_dir(data_dir).join(PENDING_RESTORE_READY_FILENAME)
}

fn managed_instance_secrets_path(data_dir: &Path) -> PathBuf {
    data_dir.join(INSTANCE_SECRETS_ENV_FILENAME)
}

fn metadata_path_for_backup(backup_dir: &Path, filename: &str) -> PathBuf {
    backup_dir.join(format!("{filename}{BACKUP_METADATA_EXTENSION}"))
}

fn is_supported_backup_filename(filename: &str) -> bool {
    !filename.contains('/')
        && !filename.contains('\\')
        && (filename.ends_with(BACKUP_PLAINTEXT_EXTENSION)
            || filename.ends_with(BACKUP_ENCRYPTED_EXTENSION)
            || filename.ends_with(LEGACY_BACKUP_PLAINTEXT_EXTENSION)
            || filename.ends_with(LEGACY_BACKUP_ENCRYPTED_EXTENSION))
}

fn conflict_response(message: impl Into<String>) -> Response {
    (
        StatusCode::CONFLICT,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
        .into_response()
}

fn parse_backup_download_authorization(
    headers: &HeaderMap,
    raw_query: Option<&str>,
) -> Result<String, AppError> {
    if raw_query
        .as_ref()
        .is_some_and(|query| query.split('&').any(|pair| pair.starts_with("ticket=")))
    {
        return Err(AppError::unauthorized_reason(
            crate::error_reasons::HTTP_BACKUP_TICKET_QUERY_FORBIDDEN,
            "backup download ticket query is not accepted",
        ));
    }

    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Err(AppError::unauthorized_reason(
            crate::error_reasons::HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_REQUIRED,
            "backup download authorization is required",
        ));
    };
    let value = value.to_str().map_err(|_| {
        AppError::unauthorized_reason(
            crate::error_reasons::HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_INVALID,
            "backup download authorization is invalid",
        )
    })?;
    let (scheme, token) = value.split_once(' ').ok_or_else(|| {
        AppError::unauthorized_reason(
            crate::error_reasons::HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_INVALID,
            "backup download authorization is invalid",
        )
    })?;
    if !scheme.eq_ignore_ascii_case("bearer") || token.trim().is_empty() || token.contains(' ') {
        return Err(AppError::unauthorized_reason(
            crate::error_reasons::HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_INVALID,
            "backup download authorization is invalid",
        ));
    }

    Ok(token.trim().to_string())
}

fn ensure_owner_only_permissions(path: &Path) -> std::io::Result<()> {
    Disk::process()
        .restrict_to_owner_blocking(path, OwnerOnly::File)
        .if_present()?;
    Ok(())
}

/// Whether `reading` shows something at its path. A path that could not be
/// read is an error, so nothing is renamed or removed on a guess.
fn present(reading: PathReading, path: &Path) -> std::io::Result<bool> {
    reading.exists().ok_or_else(|| {
        std::io::Error::other(format!(
            "restore path could not be read: {}",
            path.display()
        ))
    })
}

/// Run blocking restore file work off the async runtime, under the caller's
/// restore deadline.
async fn off_runtime<T: Send + 'static>(
    job: impl FnOnce() -> std::io::Result<T> + Send + 'static,
) -> std::io::Result<T> {
    let deadline = scryer_application::current_restore_deadline();
    tokio::task::spawn_blocking(move || match deadline {
        Some(deadline) => deadline.scope_blocking(job),
        None => job(),
    })
    .await
    .map_err(std::io::Error::other)?
}

fn sqlite_related_paths(db_file: &Path) -> Vec<PathBuf> {
    ["", "-wal", "-shm", "-journal"]
        .into_iter()
        .map(|suffix| {
            if suffix.is_empty() {
                db_file.to_path_buf()
            } else {
                PathBuf::from(format!("{}{}", db_file.display(), suffix))
            }
        })
        .collect()
}

fn promotion_temp_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.restore-promote", path.display()))
}

fn promotion_backup_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.restore-backup", path.display()))
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    Disk::process().remove_blocking(path, Remove::File)?;
    Ok(())
}

/// Rename `source` over `target`, a path this restore owns.
fn rename_over(source: &Path, target: &Path) -> std::io::Result<()> {
    Disk::process().move_replacing_blocking(source, target)?;
    Ok(())
}

fn restore_backup_file_if_present(target: &Path) -> std::io::Result<()> {
    remove_file_if_exists(&promotion_temp_path(target))?;

    let backup = promotion_backup_path(target);
    if !present(Disk::process().follow_blocking(&backup), &backup)? {
        return Ok(());
    }

    remove_file_if_exists(target)?;
    rename_over(&backup, target)?;
    ensure_owner_only_permissions(target)
}

fn cleanup_promotion_files(target: &Path) -> std::io::Result<()> {
    remove_file_if_exists(&promotion_temp_path(target))?;
    remove_file_if_exists(&promotion_backup_path(target))
}

fn stage_file_for_promotion(source: &Path, target: &Path) -> std::io::Result<()> {
    let disk = Disk::process();
    if let Some(parent) = target.parent() {
        disk.create_dir_blocking(parent, Parents::Create)?;
    }
    let temp = promotion_temp_path(target);
    remove_file_if_exists(&temp)?;
    let mut input =
        scryer_application::RestoreReader::new(disk.open_blocking(source, Open::read())?);
    let mut output = disk.open_blocking(&temp, Open::create().with_access(Access::OwnerOnly))?;
    ensure_owner_only_permissions(&temp)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()
}

fn promote_staged_file(target: &Path) -> std::io::Result<()> {
    let disk = Disk::process();
    let temp = promotion_temp_path(target);
    if !present(disk.follow_blocking(&temp), &temp)? {
        return Ok(());
    }
    retire_live_file_without_replacement(target)?;
    rename_over(&temp, target)?;
    ensure_owner_only_permissions(target)
}

fn retire_live_file_without_replacement(target: &Path) -> std::io::Result<()> {
    let backup = promotion_backup_path(target);
    remove_file_if_exists(&backup)?;
    if present(Disk::process().follow_blocking(target), target)? {
        rename_over(target, &backup)?;
    }
    Ok(())
}

/// Make renames inside `directory` durable, or fail. Windows offers no
/// directory handle to flush; NTFS journals the rename itself.
fn sync_directory(directory: &Path) -> std::io::Result<()> {
    let outcome = Disk::process().sync_dir_blocking(directory)?;
    if cfg!(unix) {
        outcome.required()?;
    }
    Ok(())
}

fn sync_parent_directories(targets: &[PathBuf]) -> std::io::Result<()> {
    let mut synced: Vec<&Path> = Vec::new();
    for parent in targets.iter().filter_map(|target| target.parent()) {
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        if !synced.contains(&parent) {
            sync_directory(parent)?;
            synced.push(parent);
        }
    }
    Ok(())
}

fn recover_interrupted_restore_promotion(targets: &[PathBuf]) -> std::io::Result<()> {
    for target in targets {
        restore_backup_file_if_present(target)?;
    }
    Ok(())
}

fn cleanup_restore_promotion_artifacts(targets: &[PathBuf]) -> std::io::Result<()> {
    for target in targets {
        cleanup_promotion_files(target)?;
    }
    Ok(())
}

async fn load_backup_metadata(backup_dir: &Path, filename: &str) -> Result<BackupInfo, AppError> {
    let path = metadata_path_for_backup(backup_dir, filename);
    let bytes = Disk::process().read(&path).await.map_err(|error| {
        AppError::not_found_reason(
            crate::error_reasons::HTTP_BACKUP_METADATA_NOT_FOUND,
            format!("backup metadata not found: {}", std::io::Error::from(error)),
        )
    })?;
    serde_json::from_slice::<BackupInfo>(&bytes)
        .map_err(|error| AppError::Repository(format!("backup metadata is invalid: {error}")))
}

pub(crate) async fn download_backup_handler(
    State(state): State<BackupRouteState>,
    AxumPath(filename): AxumPath<String>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let ticket = match parse_backup_download_authorization(&headers, raw_query.as_deref()) {
        Ok(ticket) => ticket,
        Err(error) => return map_app_error(error),
    };

    if let Err(error) = state
        .app
        .authorize_backup_download_ticket(&filename, &ticket)
        .await
    {
        return map_app_error(error);
    }

    let backup_dir = match state.app.effective_backup_dir().await {
        Ok(path) => path,
        Err(error) => return map_app_error(error),
    };
    if !is_supported_backup_filename(&filename) {
        return map_app_error(AppError::validation_refused(
            crate::error_reasons::HTTP_BACKUP_FILENAME_INVALID,
            "invalid backup filename",
        ));
    }
    let info = match load_backup_metadata(&backup_dir, &filename).await {
        Ok(info) => info,
        Err(error) => return map_app_error(error),
    };

    match info.status {
        BackupStatus::Creating => {
            return conflict_response("backup bundle is still being created");
        }
        BackupStatus::Failed => {
            return conflict_response(
                info.error_message
                    .unwrap_or_else(|| "backup bundle creation failed".to_string()),
            );
        }
        BackupStatus::Invalid => {
            return conflict_response(
                info.error_message
                    .unwrap_or_else(|| "backup bundle is invalid".to_string()),
            );
        }
        BackupStatus::Ready => {}
    }

    let bundle = backup_dir.join(&filename);
    let file = match Disk::process().open(&bundle, Open::read()).await {
        Ok(file) => file,
        Err(error) => {
            return map_app_error(AppError::not_found_reason(
                crate::error_reasons::HTTP_BACKUP_BUNDLE_NOT_FOUND,
                format!("backup bundle not found: {}", std::io::Error::from(error)),
            ));
        }
    };
    let content_length = match file.metadata().await {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            return map_app_error(AppError::Repository(format!(
                "failed to stat backup bundle: {error}"
            )));
        }
    };

    let stream = ReaderStream::new(file);
    let mut response = Body::from_stream(stream).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(value) = HeaderValue::from_str(&content_length.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

pub(crate) async fn finalize_pending_restore_if_present(
    data_dir: &Path,
    datastore_config: &DatastoreConfig,
) -> std::io::Result<bool> {
    finalize_pending_restore_if_present_with_postgres_restore(
        data_dir,
        datastore_config,
        |prepared_bundle_dir, datastore_config| async move {
            restore_prepared_backup_directory_to_datastore(datastore_config, &prepared_bundle_dir)
                .await?;
            Ok(())
        },
    )
    .await
}

async fn finalize_pending_restore_if_present_with_postgres_restore<F, Fut>(
    data_dir: &Path,
    datastore_config: &DatastoreConfig,
    restore_postgres_bundle: F,
) -> std::io::Result<bool>
where
    F: FnOnce(PathBuf, DatastoreConfig) -> Fut,
    Fut: Future<Output = Result<(), AppError>>,
{
    let disk = Disk::process();
    let pending_dir = pending_restore_dir(data_dir);
    let pending_ready = pending_restore_ready_path(data_dir);
    if disk.follow(&pending_dir).await.exists() != Some(true) {
        return Ok(false);
    }
    let target_secrets = managed_instance_secrets_path(data_dir);
    let mut promotion_targets = vec![target_secrets.clone()];
    if datastore_config.engine == DatastoreEngine::Sqlite {
        promotion_targets.extend(sqlite_related_paths(&datastore_file_path(
            &datastore_config.database_url,
        )));
    }
    let promoted_marker = pending_dir.join("restore-promoted");
    if disk
        .read(&promoted_marker)
        .await
        .is_ok_and(|bytes| bytes == b"complete")
    {
        // All replacements committed before cleanup began. A restart during
        // cleanup must never roll back only the backups that remain.
        let targets = promotion_targets.clone();
        off_runtime(move || cleanup_restore_promotion_artifacts(&targets))
            .await
            .map_err(restore_recovery_error)?;
        let _ = disk.remove(&pending_dir, Remove::Tree).await;
        return Ok(true);
    }
    match disk.follow(&pending_ready).await.exists() {
        Some(true) => {}
        Some(false) => {
            let _ = disk.remove(&pending_dir, Remove::Tree).await;
            return Ok(false);
        }
        // An unreadable marker leaves the pending restore in place.
        None => {
            return Err(std::io::Error::other(
                "pending restore marker could not be read",
            ));
        }
    }

    let pending_secrets = pending_restore_instance_secrets_path(data_dir);
    if disk.follow(&pending_secrets).await.exists() != Some(true) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "pending restore secrets are missing",
        ));
    }
    let targets = promotion_targets.clone();
    off_runtime(move || recover_interrupted_restore_promotion(&targets))
        .await
        .map_err(restore_recovery_error)?;
    scryer_application::check_restore_deadline().map_err(std::io::Error::other)?;

    let mut postgres_committed = false;
    let promotion_result = async {
        let (source, target) = (pending_secrets.clone(), target_secrets.clone());
        let stage_secrets = move || stage_file_for_promotion(&source, &target);
        let target = target_secrets.clone();
        let promote_secrets = move || promote_staged_file(&target);
        match datastore_config.engine {
            DatastoreEngine::Postgres => {
                let prepared_bundle_dir = pending_restore_prepared_bundle_dir(data_dir);
                if disk.follow(&prepared_bundle_dir).await.exists() != Some(true) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "pending restore bundle is missing",
                    ));
                }
                off_runtime(stage_secrets).await?;
                restore_postgres_bundle(prepared_bundle_dir, datastore_config.clone())
                    .await
                    .map_err(std::io::Error::other)?;
                postgres_committed = true;
                off_runtime(promote_secrets).await
            }
            DatastoreEngine::Sqlite => {
                let pending_db = pending_restore_db_path(data_dir);
                if disk.follow(&pending_db).await.exists() != Some(true) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "pending restore database is missing",
                    ));
                }

                let target_db = datastore_file_path(&datastore_config.database_url);
                let pairs: Vec<(PathBuf, PathBuf)> = sqlite_related_paths(&pending_db)
                    .into_iter()
                    .zip(sqlite_related_paths(&target_db))
                    .collect();

                // Which pending files exist is read once, while staging, and
                // decides the promotion below.
                let (pairs, staged) = off_runtime(move || {
                    let disk = Disk::process();
                    let mut staged = Vec::with_capacity(pairs.len());
                    for (pending_path, target_path) in &pairs {
                        let exists = present(disk.follow_blocking(pending_path), pending_path)?;
                        if exists {
                            stage_file_for_promotion(pending_path, target_path)?;
                        }
                        staged.push(exists);
                    }
                    stage_secrets()?;
                    Ok((pairs, staged))
                })
                .await?;

                // Promotion is recovery-protected once it starts. Expiry before
                // that boundary leaves the live files intact; expiry afterward
                // must not interrupt the multi-file promotion.
                scryer_application::check_restore_deadline().map_err(std::io::Error::other)?;
                off_runtime(move || {
                    for ((_, target_path), staged) in pairs.iter().zip(staged) {
                        if staged {
                            promote_staged_file(target_path)?;
                        } else {
                            retire_live_file_without_replacement(target_path)?;
                        }
                    }
                    promote_secrets()
                })
                .await
            }
        }
    }
    .await;

    if let Err(error) = promotion_result {
        if postgres_committed {
            // The transaction is committed. Restoring old secrets here could
            // pair them with the new database; retain all recovery artifacts.
            return Err(restore_recovery_error(error));
        }
        let targets = promotion_targets.clone();
        off_runtime(move || recover_interrupted_restore_promotion(&targets))
            .await
            .map_err(restore_recovery_error)?;
        return Err(error);
    }

    // The marker lives in another directory from the files it vouches for,
    // so their renames must be durable before it is. Then the marker's own
    // entry must be durable before cleanup removes any rollback copy.
    let marker_dir = pending_dir.clone();
    off_runtime(move || {
        let disk = Disk::process();
        sync_parent_directories(&promotion_targets)?;
        disk.write_blocking(
            &promoted_marker,
            b"complete",
            Replace::Allowed,
            Access::Default,
        )?;
        disk.sync_blocking(&promoted_marker)?.required()?;
        sync_directory(&marker_dir)?;
        cleanup_restore_promotion_artifacts(&promotion_targets)
    })
    .await
    .map_err(restore_recovery_error)?;
    let _ = disk.remove(&pending_dir, Remove::Tree).await;
    Ok(true)
}

fn restore_recovery_error(error: std::io::Error) -> std::io::Error {
    std::io::Error::other(AppError::validation_refused(
        scryer_application::restore_refusal::RESTORE_RECOVERY_REQUIRED,
        format!("Restore promotion needs recovery: {error}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staged_fixture(data: &Path) {
        std::fs::create_dir_all(pending_restore_dir(data)).unwrap();
        std::fs::write(pending_restore_db_path(data), b"restored database").unwrap();
        std::fs::write(
            pending_restore_instance_secrets_path(data),
            b"restored secrets",
        )
        .unwrap();
        std::fs::write(pending_restore_ready_path(data), b"ready").unwrap();
        std::fs::write(data.join("scryer.db"), b"original database").unwrap();
        std::fs::write(managed_instance_secrets_path(data), b"original secrets").unwrap();
        std::fs::write(data.join("unrelated"), b"preserve").unwrap();
    }

    #[tokio::test]
    async fn background_restore_expiry_before_promotion_preserves_live_files() {
        let fixture = tempfile::tempdir().unwrap();
        let data = fixture.path();
        staged_fixture(data);
        let deadline = scryer_application::RestoreDeadline::new();
        deadline.cancel();
        let config = DatastoreConfig::sqlite(
            format!("sqlite://{}", data.join("scryer.db").display()),
            data,
            scryer_infrastructure_datastore::MigrationMode::Apply,
        );
        let error = deadline
            .scope(finalize_pending_restore_if_present(data, &config))
            .await
            .unwrap_err();
        assert_eq!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<AppError>()
                .unwrap()
                .validation_reason(),
            Some(scryer_application::restore_refusal::RESTORE_DEADLINE_EXCEEDED)
        );
        assert_eq!(
            std::fs::read(data.join("scryer.db")).unwrap(),
            b"original database"
        );
        assert_eq!(
            std::fs::read(managed_instance_secrets_path(data)).unwrap(),
            b"original secrets"
        );
        assert_eq!(std::fs::read(data.join("unrelated")).unwrap(), b"preserve");
    }

    #[tokio::test]
    async fn background_restore_committed_promotion_cleanup_never_partially_rolls_back() {
        let fixture = tempfile::tempdir().unwrap();
        let data = fixture.path();
        staged_fixture(data);
        let db = data.join("scryer.db");
        std::fs::write(&db, b"committed database").unwrap();
        std::fs::write(managed_instance_secrets_path(data), b"committed secrets").unwrap();
        // Only one backup remains after a crash during cleanup.
        std::fs::write(promotion_backup_path(&db), b"original database").unwrap();
        std::fs::write(
            pending_restore_dir(data).join("restore-promoted"),
            b"complete",
        )
        .unwrap();
        // Interrupted staging-directory cleanup may already have removed
        // its inputs. The durable commit marker still owns recovery.
        std::fs::remove_file(pending_restore_ready_path(data)).unwrap();
        std::fs::remove_file(pending_restore_instance_secrets_path(data)).unwrap();
        let deadline = scryer_application::RestoreDeadline::new();
        deadline.cancel();
        let config = DatastoreConfig::sqlite(
            format!("sqlite://{}", db.display()),
            data,
            scryer_infrastructure_datastore::MigrationMode::Apply,
        );
        assert!(
            deadline
                .scope(finalize_pending_restore_if_present(data, &config))
                .await
                .unwrap()
        );
        assert_eq!(std::fs::read(db).unwrap(), b"committed database");
        assert_eq!(
            std::fs::read(managed_instance_secrets_path(data)).unwrap(),
            b"committed secrets"
        );
        assert_eq!(std::fs::read(data.join("unrelated")).unwrap(), b"preserve");
    }

    #[tokio::test]
    async fn background_restore_expiry_after_postgres_commit_finishes_secret_promotion() {
        let fixture = tempfile::tempdir().unwrap();
        let data = fixture.path();
        staged_fixture(data);
        std::fs::create_dir_all(pending_restore_prepared_bundle_dir(data)).unwrap();
        let deadline = scryer_application::RestoreDeadline::new();
        let config = DatastoreConfig::postgres(
            "postgres://unused/fixture",
            "postgres://unused/fixture",
            scryer_infrastructure_runtime::DatastoreConfigSource::EnvDbUrl,
            data,
            scryer_infrastructure_datastore::MigrationMode::Apply,
        );
        let result = deadline
            .scope(finalize_pending_restore_if_present_with_postgres_restore(
                data,
                &config,
                |_, _| async {
                    deadline.cancel();
                    Ok(())
                },
            ))
            .await
            .unwrap();
        assert!(result);
        assert_eq!(
            std::fs::read(managed_instance_secrets_path(data)).unwrap(),
            b"restored secrets"
        );
        assert_eq!(std::fs::read(data.join("unrelated")).unwrap(), b"preserve");
    }

    #[test]
    fn parse_backup_download_authorization_requires_bearer_token() {
        let missing = parse_backup_download_authorization(&HeaderMap::new(), None).unwrap_err();
        assert!(matches!(
            missing,
            AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
        ));

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer "));
        let blank = parse_backup_download_authorization(&headers, None).unwrap_err();
        assert!(matches!(
            blank,
            AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
        ));

        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        let wrong_scheme = parse_backup_download_authorization(&headers, None).unwrap_err();
        assert!(matches!(
            wrong_scheme,
            AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
        ));
    }

    #[test]
    fn parse_backup_download_authorization_accepts_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc.def"),
        );

        let ticket =
            parse_backup_download_authorization(&headers, None).expect("ticket should parse");
        assert_eq!(ticket, "abc.def");
    }

    #[test]
    fn parse_backup_download_authorization_rejects_ticket_query() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc.def"),
        );

        let error = parse_backup_download_authorization(&headers, Some("ticket=abc.def"))
            .expect_err("query tickets should be rejected");
        assert!(matches!(
            error,
            AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
        ));
    }

    #[tokio::test]
    async fn finalize_pending_restore_ignores_incomplete_restore_without_marker() {
        let temp = tempfile::tempdir().expect("tempdir");
        let data_dir = temp.path();
        let pending_dir = pending_restore_dir(data_dir);
        std::fs::create_dir_all(&pending_dir).expect("pending dir");
        std::fs::write(pending_restore_db_path(data_dir), b"partial").expect("pending db");

        let target_db = data_dir.join("scryer.db");
        std::fs::write(&target_db, b"original").expect("target db");

        let restored = finalize_pending_restore_if_present(
            data_dir,
            &DatastoreConfig::sqlite(
                format!("sqlite://{}", target_db.display()),
                data_dir,
                scryer_infrastructure_datastore::MigrationMode::Apply,
            ),
        )
        .await
        .expect("finalize");

        assert!(!restored);
        assert_eq!(std::fs::read(&target_db).expect("target db"), b"original");
        assert!(!pending_dir.exists());
    }

    #[tokio::test]
    async fn finalize_pending_restore_requires_marker_and_preserves_restored_secrets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let data_dir = temp.path();
        let pending_dir = pending_restore_dir(data_dir);
        std::fs::create_dir_all(&pending_dir).expect("pending dir");
        std::fs::write(pending_restore_db_path(data_dir), b"restored").expect("pending db");
        std::fs::write(
            pending_restore_instance_secrets_path(data_dir),
            b"SCRYER_ENCRYPTION_KEY=\"restored\"\n",
        )
        .expect("pending secrets");
        std::fs::write(pending_restore_ready_path(data_dir), b"ready").expect("ready marker");

        let target_db = data_dir.join("scryer.db");
        let target_secrets = managed_instance_secrets_path(data_dir);
        std::fs::write(&target_db, b"original").expect("target db");
        std::fs::write(&target_secrets, b"old").expect("target secrets");

        let restored = finalize_pending_restore_if_present(
            data_dir,
            &DatastoreConfig::sqlite(
                format!("sqlite://{}", target_db.display()),
                data_dir,
                scryer_infrastructure_datastore::MigrationMode::Apply,
            ),
        )
        .await
        .expect("finalize");

        assert!(restored);
        assert_eq!(std::fs::read(&target_db).expect("target db"), b"restored");
        assert_eq!(
            std::fs::read_to_string(&target_secrets).expect("target secrets"),
            "SCRYER_ENCRYPTION_KEY=\"restored\"\n",
        );
        assert!(!pending_dir.exists());
    }

    #[tokio::test]
    async fn finalize_pending_restore_promotes_postgres_secrets_without_sqlite_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let data_dir = temp.path();
        let pending_dir = pending_restore_dir(data_dir);
        std::fs::create_dir_all(&pending_dir).expect("pending dir");
        std::fs::create_dir_all(pending_restore_prepared_bundle_dir(data_dir))
            .expect("prepared bundle dir");
        std::fs::write(
            pending_restore_instance_secrets_path(data_dir),
            b"SCRYER_ENCRYPTION_KEY=\"restored\"\n",
        )
        .expect("pending secrets");
        std::fs::write(pending_restore_ready_path(data_dir), b"ready").expect("ready marker");

        let target_secrets = managed_instance_secrets_path(data_dir);
        std::fs::write(&target_secrets, b"old").expect("target secrets");

        let restored = finalize_pending_restore_if_present_with_postgres_restore(
            data_dir,
            &DatastoreConfig::postgres(
                "postgres://localhost/scryer".to_string(),
                "postgres://localhost/scryer".to_string(),
                scryer_infrastructure_runtime::DatastoreConfigSource::EnvDbUrl,
                data_dir,
                scryer_infrastructure_datastore::MigrationMode::Apply,
            ),
            |prepared_bundle_dir, _| async move {
                assert_eq!(
                    prepared_bundle_dir,
                    pending_restore_prepared_bundle_dir(data_dir)
                );
                Ok(())
            },
        )
        .await
        .expect("finalize");

        assert!(restored);
        assert_eq!(
            std::fs::read_to_string(&target_secrets).expect("target secrets"),
            "SCRYER_ENCRYPTION_KEY=\"restored\"\n",
        );
        assert!(!pending_dir.exists());
    }

    #[tokio::test]
    async fn finalize_pending_restore_preserves_postgres_pending_state_on_restore_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let data_dir = temp.path();
        let pending_dir = pending_restore_dir(data_dir);
        std::fs::create_dir_all(&pending_dir).expect("pending dir");
        std::fs::create_dir_all(pending_restore_prepared_bundle_dir(data_dir))
            .expect("prepared bundle dir");
        std::fs::write(
            pending_restore_instance_secrets_path(data_dir),
            b"SCRYER_ENCRYPTION_KEY=\"restored\"\n",
        )
        .expect("pending secrets");
        std::fs::write(pending_restore_ready_path(data_dir), b"ready").expect("ready marker");

        let target_secrets = managed_instance_secrets_path(data_dir);
        std::fs::write(&target_secrets, b"old").expect("target secrets");

        let error = finalize_pending_restore_if_present_with_postgres_restore(
            data_dir,
            &DatastoreConfig::postgres(
                "postgres://localhost/scryer".to_string(),
                "postgres://localhost/scryer".to_string(),
                scryer_infrastructure_runtime::DatastoreConfigSource::EnvDbUrl,
                data_dir,
                scryer_infrastructure_datastore::MigrationMode::Apply,
            ),
            |_, _| async move { Err(AppError::Repository("restore failed".into())) },
        )
        .await
        .expect_err("restore should fail");

        assert!(error.to_string().contains("restore failed"));
        assert_eq!(
            std::fs::read_to_string(&target_secrets).expect("target secrets"),
            "old",
        );
        assert!(pending_dir.exists());
    }

    #[tokio::test]
    async fn finalize_pending_restore_recovers_interrupted_sqlite_promotion() {
        let temp = tempfile::tempdir().expect("tempdir");
        let data_dir = temp.path();
        let pending_dir = pending_restore_dir(data_dir);
        std::fs::create_dir_all(&pending_dir).expect("pending dir");
        std::fs::write(pending_restore_db_path(data_dir), b"restored").expect("pending db");
        std::fs::write(
            pending_restore_instance_secrets_path(data_dir),
            b"SCRYER_ENCRYPTION_KEY=\"restored\"\n",
        )
        .expect("pending secrets");
        std::fs::write(pending_restore_ready_path(data_dir), b"ready").expect("ready marker");

        let target_db = data_dir.join("scryer.db");
        let target_secrets = managed_instance_secrets_path(data_dir);
        std::fs::write(&target_db, b"partial-new").expect("target db");
        std::fs::write(&target_secrets, b"partial-new-secrets").expect("target secrets");
        std::fs::write(promotion_backup_path(&target_db), b"original").expect("db backup");
        std::fs::write(promotion_backup_path(&target_secrets), b"old").expect("secrets backup");

        let restored = finalize_pending_restore_if_present(
            data_dir,
            &DatastoreConfig::sqlite(
                format!("sqlite://{}", target_db.display()),
                data_dir,
                scryer_infrastructure_datastore::MigrationMode::Apply,
            ),
        )
        .await
        .expect("finalize");

        assert!(restored);
        assert_eq!(std::fs::read(&target_db).expect("target db"), b"restored");
        assert_eq!(
            std::fs::read_to_string(&target_secrets).expect("target secrets"),
            "SCRYER_ENCRYPTION_KEY=\"restored\"\n",
        );
        assert!(!promotion_backup_path(&target_db).exists());
        assert!(!promotion_backup_path(&target_secrets).exists());
        assert!(!pending_dir.exists());
    }

    #[test]
    fn supported_backup_filenames_use_new_encrypted_extension() {
        assert!(is_supported_backup_filename(
            "20260514_010203_123_47f908fa.tar.zst"
        ));
        assert!(is_supported_backup_filename(
            "20260514_010203_123_47f908fa.enc"
        ));
        assert!(is_supported_backup_filename(
            "scryer_backup_20260514_010203_123.scryer-backup.enc"
        ));
        assert!(!is_supported_backup_filename("random-backup.bin"));
    }
}
