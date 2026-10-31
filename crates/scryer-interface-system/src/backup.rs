use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use async_graphql::{Context, ID, Object, Result as GqlResult, SimpleObject, Upload};
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppUseCase, BackupBundleInspectSummary, inspect_backup_bundle,
    prepare_backup_restore_payload, validate_inspected_bundle_is_restorable,
};
use scryer_domain::AppPermission;
use scryer_interface_core::{
    RestoreDatastoreConfig, RestoreDatastoreEngine, RestoreDatastoreHandle,
    RestoreSqliteDatastoreRequest,
};
use scryer_interface_core::{
    app_from_ctx, require_config_app_permission, restore_context_from_ctx, to_gql_error,
};
use scryer_interface_media::mappers::from_backup_info;
use scryer_interface_media::types::{
    BackupInfoPayload, BackupRowCountPayload, CreateBackupInput, DeleteBackupInput,
    DeleteBackupPayload, Long, PrepareBackupDownloadInput,
};

const INSTANCE_SECRETS_ENV_FILENAME: &str = "instance-secrets.env";
const PENDING_RESTORE_DB_FILENAME: &str = "restored-scryer.db";
const PENDING_RESTORE_PREPARED_BUNDLE_DIRNAME: &str = "prepared-bundle";
const PENDING_RESTORE_DIRNAME: &str = "restore-pending";
const PENDING_RESTORE_READY_FILENAME: &str = "restore-ready";
const RESTORE_STAGING_DIRNAME: &str = "restore-staging";
const RESTORE_UPLOAD_TTL_SECONDS: u64 = 24 * 60 * 60;
const STAGED_BUNDLE_FILENAME: &str = "bundle.upload";

#[derive(Clone, SimpleObject)]
/// Summarizes the contents and source compatibility metadata of a backup bundle.
struct RestoreSummaryPayload {
    /// Backup bundle format version.
    format_version: String,
    /// UTC timestamp when the backup bundle was created.
    created_at: DateTime<Utc>,
    /// Scryer version that created the backup.
    source_scryer_version: String,
    /// Datastore engine used by the source instance.
    source_engine: String,
    /// Source database migration key; null when the bundle does not provide one.
    source_migration_key: Option<String>,
    /// Whether the backup contents are encrypted.
    encrypted: bool,
    /// Row counts grouped by backed-up entity.
    row_counts: Vec<BackupRowCountPayload>,
    /// Total rows contained in the backup.
    total_rows: Long,
}

#[derive(Clone, SimpleObject)]
/// Identifies a staged restore upload and its inspected contents.
struct RestoreInspectPayload {
    /// Temporary restore upload ID used by the apply operation.
    upload_id: ID,
    /// Inspected backup contents and source metadata.
    summary: RestoreSummaryPayload,
}

#[derive(Clone, SimpleObject)]
/// Reports the backup contents staged for restoration on restart.
struct RestoreApplyPayload {
    /// Backup contents and source metadata accepted for restoration.
    summary: RestoreSummaryPayload,
}

#[derive(Clone, SimpleObject)]
/// Provides a temporary authorization token and route for downloading one backup.
struct BackupDownloadUrlPayload {
    /// HTTP path from which the selected backup can be downloaded.
    download_url: String,
    /// Short-lived token authorizing the backup download.
    download_authorization_token: String,
    /// UTC timestamp after which the download token is invalid.
    expires_at: DateTime<Utc>,
}

#[derive(Clone, SimpleObject)]
/// An independently owned restore attempt and its current processing phase.
pub struct RestoreJobPayload {
    /// Stable identifier for this accepted attempt.
    id: ID,
    /// Staged upload associated with this attempt.
    upload_id: ID,
    /// Whether this attempt inspects or applies the backup.
    kind: String,
    /// Current processing or recovery phase.
    phase: String,
    /// UTC time when the processing budget began.
    started_at: DateTime<Utc>,
    /// Original UTC deadline, preserved through restart.
    expires_at: DateTime<Utc>,
    /// Stable failure reason code, if processing stopped.
    reason: Option<String>,
    /// Server sentence for the failure, when this process observed it; clients
    /// translate `reason` and show this only for a code they do not know.
    message: Option<String>,
    /// Approximate database import percentage; 100 still requires finalization.
    import_percent: Option<i32>,
    /// Inspected contents when available; never contains backup secrets.
    summary: Option<RestoreSummaryPayload>,
}

fn job_payload(
    app: &AppUseCase,
    status: scryer_application::RestoreJobStatus,
) -> GqlResult<RestoreJobPayload> {
    use scryer_application::{RestoreJobKind as K, RestoreJobPhase as P};
    let message = matches!(status.phase, P::Failed | P::Interrupted)
        .then(|| recorded_failure(&status.id))
        .flatten()
        .map(|error| to_gql_error(error).message);
    Ok(RestoreJobPayload {
        id: status.id.into(),
        upload_id: status.upload_id.into(),
        kind: match status.kind {
            K::Inspect => "inspect",
            K::Apply => "apply",
        }
        .into(),
        phase: match status.phase {
            P::Inspecting => "inspecting",
            P::Draining => "draining",
            P::Restoring => "restoring",
            P::Cancelling => "cancelling",
            P::Restarting => "restarting",
            P::Complete => "complete",
            P::Failed => "failed",
            P::Interrupted => "interrupted",
        }
        .into(),
        started_at: status.started_at,
        expires_at: status.expires_at,
        reason: status.reason,
        message,
        import_percent: status.import_percent.map(i32::from),
        summary: app
            .restore_coordinator()
            .summary()
            .as_ref()
            .map(restore_summary_payload)
            .transpose()
            .map_err(to_gql_error)?,
    })
}

// A first-run code authorizes only restore controls, never an administrator
// session or unrelated GraphQL operations. It remains retryable until restart.
async fn require_restore_permission(ctx: &Context<'_>, setup_code: Option<&str>) -> GqlResult<()> {
    use scryer_application::initial_auth::{SetupState, refusal};
    let app = app_from_ctx(ctx)?;
    if matches!(
        app.initial_authentication_state()
            .await
            .map_err(to_gql_error)?,
        Some(SetupState::Choice)
    ) {
        let Some(request) = ctx.data_opt::<scryer_interface_core::InitialAuthenticationRequest>()
        else {
            return Err(to_gql_error(AppError::validation_refused(
                refusal::REQUIRED,
                "initial restore requires an admitted HTTP setup request",
            )));
        };
        return scryer_interface_core::auth_runtime_from_ctx(ctx)
            .initial_authentication
            .verify(request.client_ip, setup_code)
            .map_err(to_gql_error);
    }
    require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
    Ok(())
}

#[derive(Default)]
pub struct BackupQueries;
#[Object]
impl BackupQueries {
    /// Observe the current restore without extending its deadline or starting work.
    async fn restore_status(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            secret,
            desc = "One-time setup code authorizing restore while initial authentication is pending."
        )]
        setup_code: Option<String>,
    ) -> GqlResult<Option<RestoreJobPayload>> {
        require_restore_permission(ctx, setup_code.as_deref()).await?;
        let app = app_from_ctx(ctx)?;
        app.restore_coordinator()
            .snapshot()
            .map(|s| job_payload(&app, s))
            .transpose()
    }
}

/// The error a worker in this process last failed with, keyed by attempt id.
/// The coordinator persists only the reason code; this keeps the original
/// error so callers still waiting on the attempt receive it unchanged.
static LAST_RESTORE_FAILURE: std::sync::Mutex<Option<(String, AppError)>> =
    std::sync::Mutex::new(None);

fn record_failure(id: &str, error: &AppError) {
    *LAST_RESTORE_FAILURE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((id.to_owned(), error.clone()));
}

fn recorded_failure(id: &str) -> Option<AppError> {
    LAST_RESTORE_FAILURE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|(failed, _)| failed == id)
        .map(|(_, error)| error.clone())
}

/// The error a failed or interrupted attempt reports: the worker's own error
/// when the attempt kept its reason, otherwise the attempt's recorded reason.
fn restore_failure_error(status: &scryer_application::RestoreJobStatus) -> AppError {
    let recorded = recorded_failure(&status.id);
    if let Some(error) = recorded
        .as_ref()
        .filter(|error| error.validation_reason().is_some())
        .filter(|error| error.validation_reason() == status.reason.as_deref())
    {
        return error.clone();
    }
    let reason = scryer_application::restore_refusal::ALL
        .iter()
        .copied()
        .find(|reason| Some(*reason) == status.reason.as_deref())
        .unwrap_or(scryer_application::restore_refusal::RESTORE_INTERRUPTED);
    match recorded {
        Some(AppError::ValidationRefused { message, .. })
        | Some(AppError::ValidationRefusedWithCount { message, .. }) => {
            AppError::validation_refused(reason, message)
        }
        Some(error) => AppError::validation_refused(reason, error.to_string()),
        None => AppError::validation_refused(
            reason,
            "Restore did not complete; see restore status and server logs",
        ),
    }
}

async fn wait_for_restore(app: &AppUseCase, id: &str) -> GqlResult<RestoreJobPayload> {
    loop {
        let notified = app.restore_coordinator().changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if let Some(status) = app.restore_coordinator().snapshot().filter(|s| s.id == id) {
            use scryer_application::RestoreJobPhase as P;
            if matches!(status.phase, P::Complete | P::Restarting) {
                return job_payload(app, status);
            }
            if matches!(status.phase, P::Failed | P::Interrupted) {
                return Err(to_gql_error(restore_failure_error(&status)));
            }
        } else {
            return Err(to_gql_error(
                scryer_application::RestoreCoordinator::maintenance_error(),
            ));
        }
        notified.await;
    }
}

#[derive(Default)]
pub struct BackupMutations;

#[Object]
impl BackupMutations {
    /// Create a password-protected backup and return its stored backup metadata.
    async fn create_backup(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Password used to encrypt the new backup bundle.")]
        input: CreateBackupInput,
    ) -> GqlResult<BackupInfoPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let info = app
            .create_backup(&actor, &input.password)
            .await
            .map_err(to_gql_error)?;
        from_backup_info(info).map_err(|error| to_gql_error(AppError::Repository(error)))
    }

    /// Create a short-lived authorization token for downloading an existing backup.
    async fn prepare_backup_download(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Stored backup filename to authorize for download.")]
        input: PrepareBackupDownloadInput,
    ) -> GqlResult<BackupDownloadUrlPayload> {
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        let ticket = app
            .prepare_backup_download(&actor, &input.filename)
            .await
            .map_err(to_gql_error)?;
        let encoded_filename = encode_path_segment(&input.filename);

        Ok(BackupDownloadUrlPayload {
            download_url: format!("/backups/{encoded_filename}/download"),
            download_authorization_token: ticket.token,
            expires_at: parse_datetime(&ticket.expires_at, "backup download expires_at")
                .map_err(to_gql_error)?,
        })
    }

    /// Delete a stored backup by filename and report whether it existed.
    async fn delete_backup(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Stored backup filename to delete.")] input: DeleteBackupInput,
    ) -> GqlResult<DeleteBackupPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let filename = input.filename;
        let deleted = app
            .delete_backup(&actor, &filename)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteBackupPayload { filename, deleted })
    }

    /// Inspect and stage an uploaded restore bundle while setup mode is active.
    /// The returned upload ID remains temporary and must be supplied to the apply operation.
    async fn start_inspect_restore_bundle(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Backup bundle supplied through GraphQL multipart upload.")]
        bundle_upload: Upload,
        #[graphql(desc = "Password required to decrypt the backup bundle.")] password: Option<
            String,
        >,
        #[graphql(
            secret,
            desc = "One-time setup code authorizing restore while initial authentication is pending."
        )]
        setup_code: Option<String>,
    ) -> GqlResult<RestoreJobPayload> {
        require_restore_permission(ctx, setup_code.as_deref()).await?;
        let app = app_from_ctx(ctx)?;
        ensure_setup_mode(&app).await.map_err(to_gql_error)?;

        let restore = restore_context_from_ctx(ctx)?;
        ensure_restore_supported(&restore.datastore_config).map_err(to_gql_error)?;

        let upload = bundle_upload.value(ctx).map_err(|error| {
            to_gql_error(AppError::validation_refused(
                scryer_interface_core::refusal::INTERFACE_RESTORE_UPLOAD_INVALID,
                format!("invalid upload: {error}"),
            ))
        })?;
        let password = normalize_password(password);
        if !upload.filename.to_ascii_lowercase().ends_with(".enc") {
            return Err(to_gql_error(AppError::validation_refused(
                scryer_application::error_reasons::security::BACKUP_BUNDLE_INVALID,
                "restore requires an encrypted .enc backup bundle",
            )));
        }
        let guard = app.try_acquire_system_maintenance().map_err(to_gql_error)?;

        if !app
            .restore_coordinator()
            .snapshot()
            .is_some_and(|s| s.phase.active())
        {
            prune_stale_restore_uploads(&restore.data_dir);
        }
        let upload_id = next_restore_upload_id();
        let upload_dir = restore_upload_dir(&restore.data_dir, &upload_id);
        tokio::fs::create_dir_all(&upload_dir)
            .await
            .map_err(|error| {
                to_gql_error(AppError::Repository(format!(
                    "failed to create restore staging directory: {error}"
                )))
            })?;
        ensure_owner_only_dir_permissions(&upload_dir).map_err(|error| {
            to_gql_error(AppError::Repository(format!(
                "failed to protect restore staging directory: {error}"
            )))
        })?;

        let bundle_path = upload_dir.join(STAGED_BUNDLE_FILENAME);
        let guard = match write_uploaded_bundle(upload, bundle_path, guard).await {
            Ok(guard) => guard,
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&upload_dir).await;
                return Err(to_gql_error(error));
            }
        };

        let (status, _) = app
            .restore_coordinator()
            .start(
                &restore.data_dir,
                &upload_id,
                scryer_application::RestoreJobKind::Inspect,
            )
            .map_err(to_gql_error)?;
        let worker_app = app.clone();
        let id = status.id.clone();
        let deadline = scryer_application::RestoreDeadline::resume(status.expires_at);
        tokio::spawn(async move {
            let _guard = guard;
            let result = inspect_staged_upload(
                worker_app.restore_coordinator(),
                &id,
                &deadline,
                upload_dir,
                password,
            )
            .await;
            match result {
                Ok(summary) => {
                    worker_app.restore_coordinator().set_summary(summary);
                    if let Err(error) = worker_app.restore_coordinator().transition(
                        &id,
                        scryer_application::RestoreJobPhase::Complete,
                        None,
                    ) {
                        tracing_restore_error(&worker_app, &id, &deadline, error);
                    }
                }
                Err(error) => {
                    tracing_restore_error(&worker_app, &id, &deadline, error);
                }
            }
        });
        job_payload(&app, status)
    }

    /// Compatibility operation; accepted work survives a disconnected caller.
    async fn inspect_restore_bundle(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Backup bundle supplied through GraphQL multipart upload.")]
        bundle_upload: Upload,
        #[graphql(desc = "Password required to decrypt the backup bundle.")] password: Option<
            String,
        >,
        #[graphql(
            secret,
            desc = "One-time setup code authorizing restore while initial authentication is pending."
        )]
        setup_code: Option<String>,
    ) -> GqlResult<RestoreInspectPayload> {
        let job = self
            .start_inspect_restore_bundle(ctx, bundle_upload, password, setup_code)
            .await??;
        let app = app_from_ctx(ctx)?;
        let job = wait_for_restore(&app, job.id.as_str()).await?;
        Ok(RestoreInspectPayload {
            upload_id: job.upload_id,
            summary: job.summary.ok_or_else(|| {
                to_gql_error(scryer_application::RestoreCoordinator::maintenance_error())
            })?,
        })
    }

    /// Stage an inspected restore bundle for application on restart and schedule that restart.
    async fn start_apply_restore_bundle(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Temporary upload ID returned by inspectRestoreBundle.")] upload_id: ID,
        #[graphql(desc = "Password required to decrypt the backup bundle.")] password: Option<
            String,
        >,
        #[graphql(
            secret,
            desc = "One-time setup code authorizing restore while initial authentication is pending."
        )]
        setup_code: Option<String>,
    ) -> GqlResult<RestoreJobPayload> {
        require_restore_permission(ctx, setup_code.as_deref()).await?;
        let app = app_from_ctx(ctx)?;
        ensure_setup_mode(&app).await.map_err(to_gql_error)?;
        if let Some(status) = app.restore_coordinator().snapshot().filter(|s| {
            s.phase.active()
                && s.kind == scryer_application::RestoreJobKind::Apply
                && s.upload_id == upload_id.as_str()
        }) {
            return job_payload(&app, status);
        }
        let maintenance_guard = app.try_acquire_system_maintenance().map_err(to_gql_error)?;

        let restore = restore_context_from_ctx(ctx)?;
        ensure_restore_supported(&restore.datastore_config).map_err(to_gql_error)?;

        let upload_id = upload_id.as_ref().trim();
        if !is_valid_restore_upload_id(upload_id) {
            return Err(to_gql_error(AppError::validation_refused(
                scryer_interface_core::refusal::INTERFACE_RESTORE_UPLOAD_ID_INVALID,
                "invalid restore upload id",
            )));
        }

        if let Some(status) = resume_failed_restart(
            app.restore_coordinator(),
            &restore.data_dir,
            upload_id,
            &restore.restart,
        )
        .map_err(to_gql_error)?
        {
            drop(maintenance_guard);
            return job_payload(&app, status);
        }

        let bundle_path =
            staged_upload_bundle_path(&restore.data_dir, upload_id).map_err(to_gql_error)?;

        let upload_dir = restore_upload_dir(&restore.data_dir, upload_id);
        let data_dir = restore.data_dir.clone();
        let datastore_config = restore.datastore_config;
        let datastore = restore.datastore.clone();
        let password = normalize_password(password);
        let (status, _) = app
            .restore_coordinator()
            .start(
                &data_dir,
                upload_id,
                scryer_application::RestoreJobKind::Apply,
            )
            .map_err(to_gql_error)?;
        let deadline = scryer_application::RestoreDeadline::resume(status.expires_at);
        let id = status.id.clone();
        let worker_app = app.clone();
        tokio::spawn(async move {
            let _guard = maintenance_guard;
            let work = async {
                worker_app.restore_coordinator().shutdown_workers.cancel();
                worker_app.shutdown_scripts().await;
                worker_app.restore_coordinator().drain(&deadline).await?;
                worker_app.restore_coordinator().transition(
                    &id,
                    scryer_application::RestoreJobPhase::Restoring,
                    None,
                )?;
                let worker_deadline = deadline.clone();
                tokio::task::spawn_blocking(move || {
                    worker_deadline.scope_blocking(|| {
                        stage_restore_bundle(
                            data_dir,
                            bundle_path,
                            datastore_config,
                            datastore,
                            password,
                        )
                    })
                })
                .await
                .map_err(|e| AppError::Repository(format!("restore worker failed: {e}")))?
            };
            let result = worker_app
                .restore_coordinator()
                .supervise(&id, &deadline, work)
                .await;
            match result {
                Ok(summary) => {
                    // The staged copy is no longer needed once the restore is
                    // prepared, and it holds the bundle's instance secrets.
                    let _ = tokio::fs::remove_dir_all(&upload_dir).await;
                    worker_app.restore_coordinator().set_summary(summary);
                    match worker_app.restore_coordinator().transition(
                        &id,
                        scryer_application::RestoreJobPhase::Restarting,
                        None,
                    ) {
                        Ok(()) => restore.restart.schedule_restart(),
                        Err(error) => tracing_restore_error(&worker_app, &id, &deadline, error),
                    }
                }
                Err(error) => tracing_restore_error(&worker_app, &id, &deadline, error),
            }
        });
        job_payload(&app, status)
    }

    /// Compatibility operation; restoration is owned by the coordinator.
    async fn apply_restore_bundle(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Temporary upload ID returned by inspectRestoreBundle.")] upload_id: ID,
        #[graphql(desc = "Password required to decrypt the backup bundle.")] password: Option<
            String,
        >,
        #[graphql(
            secret,
            desc = "One-time setup code authorizing restore while initial authentication is pending."
        )]
        setup_code: Option<String>,
    ) -> GqlResult<RestoreApplyPayload> {
        let job = self
            .start_apply_restore_bundle(ctx, upload_id, password, setup_code)
            .await??;
        let app = app_from_ctx(ctx)?;
        let _ = wait_for_restore(&app, job.id.as_str()).await?;
        let summary = app.restore_coordinator().summary().ok_or_else(|| {
            to_gql_error(scryer_application::RestoreCoordinator::maintenance_error())
        })?;
        Ok(RestoreApplyPayload {
            summary: restore_summary_payload(&summary).map_err(to_gql_error)?,
        })
    }
}

fn tracing_restore_error(
    app: &AppUseCase,
    id: &str,
    deadline: &scryer_application::RestoreDeadline,
    error: AppError,
) {
    record_failure(id, &error);
    app.restore_coordinator().fail(id, deadline, error);
}

/// Inspect one staged upload under the attempt's deadline. A bundle that fails
/// inspection is never applied, so its upload directory is removed.
async fn inspect_staged_upload(
    coordinator: &scryer_application::RestoreCoordinator,
    id: &str,
    deadline: &scryer_application::RestoreDeadline,
    upload_dir: PathBuf,
    password: Option<String>,
) -> Result<BackupBundleInspectSummary, AppError> {
    let worker_deadline = deadline.clone();
    let bundle_path = upload_dir.join(STAGED_BUNDLE_FILENAME);
    let work = async {
        tokio::task::spawn_blocking(move || {
            worker_deadline.scope_blocking(|| {
                let summary = inspect_backup_bundle(&bundle_path, password.as_deref())?;
                worker_deadline.check()?;
                validate_inspected_bundle_is_restorable(&summary)?;
                Ok::<_, AppError>(summary)
            })
        })
        .await
        .unwrap_or_else(|error| {
            Err(AppError::Repository(format!(
                "restore inspection worker failed: {error}"
            )))
        })
    };
    let result = coordinator.supervise(id, deadline, work).await;
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&upload_dir).await;
    }
    result
}

/// A successful stage removed its upload, so a restart whose launcher failed is
/// retried from the prepared stage instead of staging again. The ready marker
/// is checked first: without it the stage is gone, the record is left as it
/// is, and the caller reports the missing upload.
fn resume_failed_restart(
    coordinator: &scryer_application::RestoreCoordinator,
    data_dir: &Path,
    upload_id: &str,
    restart: &scryer_interface_core::RestoreRestartHandle,
) -> Result<Option<scryer_application::RestoreJobStatus>, AppError> {
    if !pending_restore_ready_path(data_dir).is_file() {
        return Ok(None);
    }
    let Some(status) = coordinator.resume_failed_restart(upload_id)? else {
        return Ok(None);
    };
    restart.schedule_restart();
    Ok(Some(status))
}

fn staged_upload_bundle_path(data_dir: &Path, upload_id: &str) -> Result<PathBuf, AppError> {
    let bundle_path = restore_upload_bundle_path(data_dir, upload_id);
    if !bundle_path.exists() {
        return Err(AppError::not_found_reason(
            scryer_interface_core::refusal::INTERFACE_RESTORE_UPLOAD_NOT_FOUND,
            "restore upload could not be found; inspect the bundle again",
        ));
    }
    Ok(bundle_path)
}

fn normalize_password(password: Option<String>) -> Option<String> {
    password
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// Upload ids are generated by `next_restore_upload_id`; accepting only its
/// alphabet keeps a supplied id to one plain directory name under staging
/// (no separators, `.`/`..`, or drive prefixes).
fn is_valid_restore_upload_id(upload_id: &str) -> bool {
    !upload_id.is_empty()
        && upload_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn restore_upload_dir(data_dir: &Path, upload_id: &str) -> PathBuf {
    data_dir.join(RESTORE_STAGING_DIRNAME).join(upload_id)
}

fn restore_upload_bundle_path(data_dir: &Path, upload_id: &str) -> PathBuf {
    restore_upload_dir(data_dir, upload_id).join(STAGED_BUNDLE_FILENAME)
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

async fn ensure_setup_mode(app: &AppUseCase) -> Result<(), AppError> {
    if app.setup_complete().await? {
        return Err(AppError::validation_refused(
            scryer_interface_core::refusal::INTERFACE_RESTORE_SETUP_REQUIRED,
            "restore is only available while setup is still incomplete",
        ));
    }
    Ok(())
}

fn ensure_restore_supported(_datastore_config: &RestoreDatastoreConfig) -> Result<(), AppError> {
    Ok(())
}

fn parse_datetime(value: &str, field: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            AppError::validation_refused(
                scryer_interface_core::refusal::INTERFACE_TIMESTAMP_INVALID,
                format!("invalid {field} timestamp: {error}"),
            )
        })
}

fn restore_summary_payload(
    summary: &BackupBundleInspectSummary,
) -> Result<RestoreSummaryPayload, AppError> {
    Ok(RestoreSummaryPayload {
        format_version: summary.format_version.clone(),
        created_at: parse_datetime(&summary.created_at, "restore summary created_at")?,
        source_scryer_version: summary.source_scryer_version.clone(),
        source_engine: summary.source_engine.clone(),
        source_migration_key: summary.source_migration_key.clone(),
        encrypted: summary.encrypted,
        row_counts: summary
            .row_counts
            .iter()
            .map(|(table, row_count)| BackupRowCountPayload {
                table: table.clone(),
                row_count: Long::from_u64_saturating(*row_count),
            })
            .collect(),
        total_rows: Long::from_u64_saturating(summary.total_rows()),
    })
}

fn prune_stale_restore_uploads(data_dir: &Path) {
    let uploads_dir = data_dir.join(RESTORE_STAGING_DIRNAME);
    let Ok(entries) = std::fs::read_dir(&uploads_dir) else {
        return;
    };
    let cutoff = SystemTime::now().checked_sub(Duration::from_secs(RESTORE_UPLOAD_TTL_SECONDS));

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let expired = metadata
            .modified()
            .ok()
            .zip(cutoff)
            .is_some_and(|(modified, cutoff)| modified < cutoff);
        if expired {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

fn next_restore_upload_id() -> String {
    let timestamp_micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros())
        .unwrap_or(0);
    format!("restore_{}_{}", timestamp_micros, std::process::id())
}

async fn write_uploaded_bundle(
    upload: async_graphql::UploadValue,
    bundle_path: PathBuf,
    guard: tokio::sync::OwnedMutexGuard<()>,
) -> Result<tokio::sync::OwnedMutexGuard<()>, AppError> {
    tokio::task::spawn_blocking(move || {
        let mut output = std::fs::File::create(&bundle_path).map_err(|error| {
            AppError::Repository(format!("failed to create restore staging bundle: {error}"))
        })?;
        let mut reader = upload.into_read();
        std::io::copy(&mut reader, &mut output).map_err(|error| {
            AppError::Repository(format!("failed to write restore staging bundle: {error}"))
        })?;
        output.flush().map_err(|error| {
            AppError::Repository(format!("failed to flush restore staging bundle: {error}"))
        })?;
        ensure_owner_only_permissions(&bundle_path).map_err(|error| {
            AppError::Repository(format!("failed to protect restore staging bundle: {error}"))
        })?;
        Ok(guard)
    })
    .await
    .map_err(|error| AppError::Repository(format!("restore upload worker failed: {error}")))?
}

fn stage_restore_bundle(
    data_dir: PathBuf,
    bundle_path: PathBuf,
    datastore_config: RestoreDatastoreConfig,
    datastore: RestoreDatastoreHandle,
    password: Option<String>,
) -> Result<BackupBundleInspectSummary, AppError> {
    scryer_application::check_restore_deadline()?;
    let pending_dir = pending_restore_dir(&data_dir);
    let pending_db_path = pending_restore_db_path(&data_dir);
    let pending_prepared_bundle_dir = pending_restore_prepared_bundle_dir(&data_dir);
    let pending_secrets_path = pending_restore_instance_secrets_path(&data_dir);
    let pending_ready_path = pending_restore_ready_path(&data_dir);
    let _ = std::fs::remove_dir_all(&pending_dir);
    std::fs::create_dir_all(&pending_dir).map_err(|error| {
        AppError::Repository(format!(
            "failed to create pending restore directory: {error}"
        ))
    })?;
    ensure_owner_only_dir_permissions(&pending_dir).map_err(|error| {
        AppError::Repository(format!(
            "failed to protect pending restore directory: {error}"
        ))
    })?;

    let result = (|| match datastore_config.engine {
        RestoreDatastoreEngine::Postgres => {
            let prepared = prepare_backup_restore_payload(&bundle_path, password.as_deref())?;
            let summary = prepared.summary();
            let instance_secrets_env = prepared.instance_secrets_env()?;
            write_owner_only_file_atomically(&pending_secrets_path, &instance_secrets_env)?;
            prepared.persist_extracted_dir(&pending_prepared_bundle_dir)?;
            Ok::<_, AppError>(summary)
        }
        RestoreDatastoreEngine::Sqlite => {
            let prepared =
                datastore.restore_sqlite_bundle_to_path(RestoreSqliteDatastoreRequest {
                    target_db_path: pending_db_path.clone(),
                    migration_mode: datastore_config.migration_mode,
                    bundle_path: bundle_path.clone(),
                    passphrase: password.clone(),
                })?;

            write_owner_only_file_atomically(
                &pending_secrets_path,
                &prepared.instance_secrets_env(),
            )?;

            let summary = prepared.summary().clone();
            ensure_owner_only_permissions(&pending_db_path).map_err(|error| {
                AppError::Repository(format!(
                    "failed to protect pending restore database: {error}"
                ))
            })?;
            Ok::<_, AppError>(summary)
        }
    })();

    let result = result.and_then(|summary| {
        scryer_application::check_restore_deadline()?;
        Ok(summary)
    });
    match result {
        Ok(summary) => {
            let marker_result = std::fs::write(&pending_ready_path, b"ready")
                .and_then(|_| ensure_owner_only_permissions(&pending_ready_path));
            if let Err(error) = marker_result {
                let _ = std::fs::remove_dir_all(&pending_dir);
                return Err(AppError::Repository(format!(
                    "failed to mark pending restore ready: {error}"
                )));
            }
            Ok(summary)
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&pending_dir);
            Err(error)
        }
    }
}

#[cfg(unix)]
fn ensure_owner_only_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if !path.exists() {
        return Ok(());
    }

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn ensure_owner_only_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn ensure_owner_only_dir_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if !path.exists() {
        return Ok(());
    }

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn ensure_owner_only_dir_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn write_owner_only_file_atomically(path: &Path, contents: &str) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| {
        AppError::Repository(format!(
            "cannot resolve parent directory for {}",
            path.display()
        ))
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        AppError::Repository(format!(
            "failed to create parent directory for {}: {error}",
            path.display()
        ))
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            AppError::Repository(format!("invalid restore secrets path {}", path.display()))
        })?;
    let temp_path = parent.join(format!(
        ".{file_name}.tmp.{}.{}",
        std::process::id(),
        next_restore_upload_id()
    ));

    let write_result = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp_path)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(AppError::Repository(format!(
            "failed to write restore secrets {}: {error}",
            temp_path.display()
        )));
    }

    std::fs::rename(&temp_path, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp_path);
        AppError::Repository(format!(
            "failed to move restore secrets into place {}: {error}",
            path.display()
        ))
    })?;
    ensure_owner_only_permissions(path).map_err(|error| {
        AppError::Repository(format!(
            "failed to protect restore secrets {}: {error}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A uniquely named scratch directory removed when the test ends.
    struct ScratchDir(PathBuf);
    impl ScratchDir {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "scryer-backup-test-{}-{}",
                next_restore_upload_id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn background_restore_expired_staging_preserves_existing_and_unrelated_files() {
        let scratch = ScratchDir::new();
        let directory = scratch.path().to_path_buf();
        let pending = pending_restore_dir(&directory);
        std::fs::create_dir(&pending).unwrap();
        let existing = pending.join("existing-stage");
        let unrelated = directory.join("unrelated-file");
        std::fs::write(&existing, b"preserve").unwrap();
        std::fs::write(&unrelated, b"preserve").unwrap();
        let deadline = scryer_application::RestoreDeadline::new();
        deadline.cancel();
        let result = deadline.scope_blocking(|| {
            stage_restore_bundle(
                directory.clone(),
                directory.join("bundle"),
                RestoreDatastoreConfig {
                    engine: RestoreDatastoreEngine::Sqlite,
                    migration_mode: scryer_interface_core::RestoreMigrationMode::Apply,
                },
                RestoreDatastoreHandle::unavailable(),
                None,
            )
        });
        assert_eq!(
            result.unwrap_err().validation_reason(),
            Some(scryer_application::restore_refusal::RESTORE_DEADLINE_EXCEEDED)
        );
        assert_eq!(std::fs::read(existing).unwrap(), b"preserve");
        assert_eq!(std::fs::read(unrelated).unwrap(), b"preserve");
        assert!(!pending_restore_ready_path(&directory).exists());
    }

    #[tokio::test]
    async fn background_restore_failed_inspection_removes_only_its_own_upload() {
        let scratch = ScratchDir::new();
        let upload_dir = restore_upload_dir(scratch.path(), "restore_1_1");
        let sibling_dir = restore_upload_dir(scratch.path(), "restore_2_2");
        std::fs::create_dir_all(&upload_dir).unwrap();
        std::fs::create_dir_all(&sibling_dir).unwrap();
        std::fs::write(
            upload_dir.join(STAGED_BUNDLE_FILENAME),
            b"not a backup bundle",
        )
        .unwrap();
        std::fs::write(sibling_dir.join(STAGED_BUNDLE_FILENAME), b"preserve").unwrap();
        let coordinator = scryer_application::RestoreCoordinator::default();
        let (job, _) = coordinator
            .start(
                scratch.path(),
                "restore_1_1",
                scryer_application::RestoreJobKind::Inspect,
            )
            .unwrap();
        let deadline = scryer_application::RestoreDeadline::resume(job.expires_at);
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            inspect_staged_upload(&coordinator, &job.id, &deadline, upload_dir.clone(), None),
        )
        .await
        .expect("inspection must finish");
        assert!(result.is_err());
        assert!(!upload_dir.exists());
        assert_eq!(
            std::fs::read(sibling_dir.join(STAGED_BUNDLE_FILENAME)).unwrap(),
            b"preserve"
        );
        assert!(scratch.path().join(RESTORE_STAGING_DIRNAME).is_dir());
    }

    #[test]
    fn background_restore_failure_keeps_the_workers_reason_and_sentence() {
        const SYNTHETIC_REFUSAL: &str = "SECURITY_BACKUP_SYNTHETIC_REFUSAL";
        let scratch = ScratchDir::new();
        let coordinator = scryer_application::RestoreCoordinator::default();
        // A failed attempt carrying `reason`, recorded the way a worker leaves it.
        let status = |reason: &str| {
            let (job, _) = coordinator
                .start(
                    scratch.path(),
                    "restore_1_1",
                    scryer_application::RestoreJobKind::Inspect,
                )
                .unwrap();
            coordinator
                .transition(
                    &job.id,
                    scryer_application::RestoreJobPhase::Failed,
                    Some(reason),
                )
                .unwrap();
            coordinator.snapshot().unwrap()
        };
        let failed = status(SYNTHETIC_REFUSAL);
        record_failure(
            &failed.id,
            &AppError::validation_refused(SYNTHETIC_REFUSAL, "synthetic sentence"),
        );
        let error = restore_failure_error(&failed);
        assert_eq!(error.validation_reason(), Some(SYNTHETIC_REFUSAL));
        assert_eq!(
            to_gql_error(error).message,
            "validation: synthetic sentence"
        );

        let interrupted = scryer_application::restore_refusal::RESTORE_INTERRUPTED;
        let failed = status(interrupted);
        record_failure(
            &failed.id,
            &AppError::Repository("synthetic storage failure".to_owned()),
        );
        let error = restore_failure_error(&failed);
        assert_eq!(error.validation_reason(), Some(interrupted));
        assert!(error.to_string().contains("synthetic storage failure"));

        let deadline = scryer_application::restore_refusal::RESTORE_DEADLINE_EXCEEDED;
        let error = restore_failure_error(&status(deadline));
        assert_eq!(error.validation_reason(), Some(deadline));
    }

    #[test]
    fn background_restore_failed_restart_resumes_from_the_prepared_stage() {
        let scratch = ScratchDir::new();
        let restart_requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requests = restart_requests.clone();
        let restart = scryer_interface_core::RestoreRestartHandle::new(move || {
            requests.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let coordinator = scryer_application::RestoreCoordinator::default();
        let (job, _) = coordinator
            .start(
                scratch.path(),
                "restore_1_1",
                scryer_application::RestoreJobKind::Apply,
            )
            .unwrap();
        coordinator
            .transition(
                &job.id,
                scryer_application::RestoreJobPhase::Failed,
                Some(scryer_application::restore_refusal::RESTORE_RESTART_FAILED),
            )
            .unwrap();

        // Without the ready marker the stage is gone: nothing changes, and the
        // normal path reports the upload the successful stage removed.
        assert!(
            resume_failed_restart(&coordinator, scratch.path(), "restore_1_1", &restart)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            coordinator.snapshot().unwrap().phase,
            scryer_application::RestoreJobPhase::Failed
        );
        assert_eq!(
            restart_requests.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        assert_eq!(
            staged_upload_bundle_path(scratch.path(), "restore_1_1")
                .unwrap_err()
                .reason(),
            Some(scryer_interface_core::refusal::INTERFACE_RESTORE_UPLOAD_NOT_FOUND)
        );

        std::fs::create_dir_all(pending_restore_dir(scratch.path())).unwrap();
        std::fs::write(pending_restore_ready_path(scratch.path()), b"ready").unwrap();
        // Another upload's retry does not resume this attempt.
        assert!(
            resume_failed_restart(&coordinator, scratch.path(), "restore_2_2", &restart)
                .unwrap()
                .is_none()
        );
        let resumed = resume_failed_restart(&coordinator, scratch.path(), "restore_1_1", &restart)
            .unwrap()
            .expect("a failed restart with a ready stage resumes");
        assert_eq!(resumed.id, job.id);
        assert_eq!(
            coordinator.snapshot().unwrap().phase,
            scryer_application::RestoreJobPhase::Restarting
        );
        assert_eq!(
            restart_requests.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert!(!scratch.path().join(RESTORE_STAGING_DIRNAME).exists());
        assert!(pending_restore_ready_path(scratch.path()).is_file());
    }

    #[test]
    fn background_restore_upload_ids_stay_one_plain_directory_name() {
        assert!(is_valid_restore_upload_id(&next_restore_upload_id()));
        for invalid in ["", ".", "..", "../x", "a/b", "a\\b", "C:", "a b", "a\0b"] {
            assert!(!is_valid_restore_upload_id(invalid), "{invalid:?}");
        }
    }
}
