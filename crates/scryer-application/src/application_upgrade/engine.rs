use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::application_upgrade::InstallationKind;
use crate::application_upgrade::helper_plan::reboot_required_completion_allowed;
use crate::application_upgrade::helper_plan::{
    APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA, ApplicationUpgradeHelperMode,
    ApplicationUpgradeHelperOwner, ApplicationUpgradeHelperPlan, ApplicationUpgradeHelperRelaunch,
    ApplicationUpgradeHelperReplacement,
};
use crate::application_upgrade::manifest::{
    UPGRADE_MANIFEST_MAX_BYTES, UpgradeArchitecture, UpgradeArchive, UpgradeArtifact,
    UpgradeChannel, UpgradeManifest, UpgradePlatform, parse_and_validate_upgrade_manifest,
    scryer_release_required_signer,
};
use crate::domain_events::DomainEventActor;
use crate::plugins::catalog::verify_signed_blob;
use crate::{
    AppError, AppResult, AppUseCase, JobKey, JobRun, JobRunRecord, JobRunStatus, JobTriggerSource,
    SCRYER_VERSION, filesystem_space_raw,
};
use scryer_domain::{
    DomainEventPayload, Id, JobRunCompletedEventData, JobRunFailedEventData,
    JobRunStartedEventData, User,
};

/// Stable progress phase names consumed by the application-upgrade UI.
pub mod phases {
    pub const CHECKING: &str = "checking";
    pub const DOWNLOADING: &str = "downloading";
    pub const VERIFYING: &str = "verifying";
    pub const STAGING: &str = "staging";
    pub const APPLYING: &str = "applying";
    pub const AWAITING_ELEVATION: &str = "awaiting_elevation";
    pub const RESTARTING: &str = "restarting";
    pub const REBOOT_REQUIRED: &str = "reboot_required";
}

const UPGRADE_BUNDLE_MAX_BYTES: u64 = 256 * 1024;
const UPGRADE_STAGING_RESERVE_BYTES: u64 = 64 * 1024 * 1024;
const DOWNLOAD_PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
const JOURNAL_SCHEMA: &str = "scryer.upgrade.journal.v1";

/// Progress persisted in `workflow_operations.progress_json` for an application upgrade.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUpgradeProgress {
    pub status: String,
    pub phase: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub target_version: String,
    pub target_tag: String,
    pub error: Option<String>,
}

/// Internal start request assembled by the GraphQL mutation after installation assessment.
#[derive(Clone, Debug)]
pub struct ApplicationUpgradeJobRequest {
    pub expected_tag: String,
    pub expected_version: String,
    pub installation_kind: InstallationKind,
    /// Tests and nonstandard executable hosts may provide the startup evidence path directly.
    pub executable_path: Option<PathBuf>,
    /// Whether the desktop tray owns and supervises this backend process.
    pub tray_supervised: bool,
}

/// Accepted durable job run returned once the asynchronous engine is registered.
#[derive(Clone, Debug)]
pub struct ApplicationUpgradeJobAccepted {
    pub job_run: JobRun,
}

/// Crash-safe handoff between applying an upgrade and validating the next boot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationUpgradeJournal {
    pub schema: String,
    pub run_id: String,
    pub expected_version: String,
    pub expected_tag: String,
    pub executable_path: PathBuf,
    pub backup_path: PathBuf,
    #[serde(default)]
    pub backup_paths: Vec<PathBuf>,
    pub phase: String,
    pub helper_error: Option<String>,
    #[serde(default)]
    pub written_at: Option<DateTime<Utc>>,
}

/// Executable and backup locations for a portable promotion.
///
/// These are resolved before anything is moved so the durable journal can be
/// written ahead of the promotion it describes.
#[derive(Clone, Debug)]
#[cfg_attr(windows, allow(dead_code))]
struct PortableUpgradePaths {
    executable_path: PathBuf,
    backup_path: PathBuf,
}

/// Failure state of a portable promotion after its journal was written.
///
/// Once the current executable has moved aside, a failed restoration must keep
/// the journal and backup paths available for recovery on the next boot.
#[cfg_attr(windows, allow(dead_code))]
enum PortablePromotionFailure {
    Restored(AppError),
    RecoveryRequired(AppError),
}

#[cfg_attr(windows, allow(dead_code))]
impl From<AppError> for PortablePromotionFailure {
    fn from(error: AppError) -> Self {
        Self::Restored(error)
    }
}

#[cfg_attr(windows, allow(dead_code))]
impl PortablePromotionFailure {
    fn into_parts(self) -> (AppError, bool) {
        match self {
            Self::Restored(error) => (error, true),
            Self::RecoveryRequired(error) => (error, false),
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
struct WindowsUpgradeHandoffInput<'a> {
    run_id: &'a str,
    expected_version: &'a str,
    expected_tag: &'a str,
    installation_kind: InstallationKind,
    tray_supervised: bool,
    executable_path: &'a Path,
    install_dir: &'a Path,
    /// Process id of the backend the helper must outlive before it replaces files.
    backend_process_id: u32,
    artifact: Option<&'a UpgradeArtifact>,
    extracted_dir: Option<&'a Path>,
    msi_path: Option<&'a Path>,
    journal_path: PathBuf,
    direct_relaunch_args: &'a [String],
    direct_relaunch_cwd: &'a Path,
    current_version: &'a str,
    written_at: DateTime<Utc>,
}

#[cfg_attr(not(windows), allow(dead_code))]
struct WindowsUpgradeHandoff {
    journal: ApplicationUpgradeJournal,
    plan: ApplicationUpgradeHelperPlan,
    progress_phase: &'static str,
}

type UpgradeSpaceCheck = fn(&Path, u64) -> AppResult<()>;
type UpgradeRename = fn(&Path, &Path) -> std::io::Result<()>;

fn rename_path(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

struct UpgradePipelineDependencies<'a> {
    client: &'a reqwest::Client,
    artifact_url_override: Option<&'a str>,
    ensure_available_space: UpgradeSpaceCheck,
    #[cfg_attr(windows, allow(dead_code))]
    rename: UpgradeRename,
}

impl ApplicationUpgradeProgress {
    fn checking(request: &ApplicationUpgradeJobRequest) -> Self {
        Self {
            status: JobRunStatus::Running.as_str().to_string(),
            phase: phases::CHECKING.to_string(),
            downloaded_bytes: 0,
            total_bytes: 0,
            target_version: request.expected_version.clone(),
            target_tag: request.expected_tag.clone(),
            error: None,
        }
    }
}

impl AppUseCase {
    /// Validate the signed-update notice and begin the single-flight application-upgrade job.
    pub async fn start_application_upgrade_job(
        &self,
        actor: &User,
        request: ApplicationUpgradeJobRequest,
    ) -> AppResult<ApplicationUpgradeJobAccepted> {
        if request.expected_tag.trim().is_empty() {
            return Err(AppError::Validation(
                "expectedTag must not be empty".to_string(),
            ));
        }
        if request.expected_version.trim().is_empty() {
            return Err(AppError::Validation(
                "expectedVersion must not be empty".to_string(),
            ));
        }

        let notice = self.smg_scryer_update_notice().await?.ok_or_else(|| {
            AppError::Validation("no application update notice is available".to_string())
        })?;
        if !notice.available {
            return Err(AppError::Validation(
                "the application update notice is not available".to_string(),
            ));
        }
        if notice.latest_tag != request.expected_tag {
            return Err(AppError::Validation(
                "expectedTag does not match the current application update notice".to_string(),
            ));
        }
        if notice.latest_version != request.expected_version {
            return Err(AppError::Validation(
                "expectedVersion does not match the current application update notice".to_string(),
            ));
        }

        let expected_version = Version::parse(&request.expected_version).map_err(|error| {
            AppError::Validation(format!("expectedVersion must be valid semver: {error}"))
        })?;
        let running_version = Version::parse(SCRYER_VERSION).map_err(|error| {
            AppError::Repository(format!("running application version is invalid: {error}"))
        })?;
        if expected_version <= running_version {
            return Err(AppError::Validation(
                "expectedVersion must be strictly newer than the running version".to_string(),
            ));
        }

        if !matches!(
            request.installation_kind,
            InstallationKind::Portable | InstallationKind::DirectMsi
        ) {
            return Err(AppError::Validation(
                "application upgrade installation is not eligible".to_string(),
            ));
        }

        let maintenance_guard = self.try_acquire_system_maintenance()?;
        if self
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(JobKey::ApplicationUpgrade)
            .await
        {
            return Err(AppError::Validation(
                "an application upgrade job is already running".to_string(),
            ));
        }

        let now = chrono::Utc::now();
        let mut run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::ApplicationUpgrade,
            operation_type: format!(
                "application_upgrade:{SCRYER_VERSION}->{}",
                request.expected_version
            ),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: serde_json::to_string(&ApplicationUpgradeProgress::checking(&request))
                .ok(),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;
        let job_run = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(job_run.clone())
            .await;

        let actor_event = DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor_event.clone(),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;

        let app = self.clone();
        tokio::spawn(async move {
            app.run_application_upgrade_job(run, actor_event, request, maintenance_guard)
                .await;
        });

        Ok(ApplicationUpgradeJobAccepted { job_run })
    }

    /// Return the current tracked run and the newest persisted run for the upgrade status query.
    pub async fn application_upgrade_job_runs(
        &self,
    ) -> AppResult<(Option<JobRun>, Option<JobRun>)> {
        let active = self
            .runtime
            .jobs
            .job_run_tracker
            .active_run_for_job(JobKey::ApplicationUpgrade)
            .await;
        let latest = self
            .services
            .events
            .job_runs
            .list_job_runs(Some(JobKey::ApplicationUpgrade), 1)
            .await?
            .into_iter()
            .next()
            .map(|record| JobRun::from_record(&record, None));
        Ok((active, latest))
    }

    async fn run_application_upgrade_job(
        &self,
        mut run: JobRunRecord,
        actor: DomainEventActor,
        request: ApplicationUpgradeJobRequest,
        _maintenance_guard: tokio::sync::OwnedMutexGuard<()>,
    ) {
        let result = self.execute_application_upgrade(&mut run, &request).await;
        if let Err(error) = result {
            self.cleanup_application_upgrade_staging();
            if let Err(finish_error) = self
                .finish_application_upgrade_failure(&mut run, actor, error.to_string())
                .await
            {
                tracing::error!(error = %finish_error, run_id = %run.id, "failed to finish application upgrade job");
            }
        }
    }

    async fn execute_application_upgrade(
        &self,
        run: &mut JobRunRecord,
        request: &ApplicationUpgradeJobRequest,
    ) -> AppResult<()> {
        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress::checking(request),
        )
        .await?;
        let client = application_upgrade_http_client()?;
        let manifest_url =
            release_asset_url(&request.expected_tag, "scryer-upgrade-manifest.json")?;
        let bundle_url = release_asset_url(
            &request.expected_tag,
            "scryer-upgrade-manifest.json.sigstore.json",
        )?;
        let manifest_raw = fetch_capped_bytes(
            &client,
            manifest_url.as_str(),
            UPGRADE_MANIFEST_MAX_BYTES,
            "upgrade manifest",
        )
        .await?;
        let bundle_raw = fetch_capped_bytes(
            &client,
            bundle_url.as_str(),
            UPGRADE_BUNDLE_MAX_BYTES,
            "upgrade manifest signature bundle",
        )
        .await?;
        verify_upgrade_manifest_signature(manifest_raw.clone(), bundle_raw).await?;
        let manifest = parse_and_validate_upgrade_manifest(&manifest_raw)?;
        self.run_upgrade_pipeline(run, request, &manifest, &client, None)
            .await
    }

    async fn run_upgrade_pipeline(
        &self,
        run: &mut JobRunRecord,
        request: &ApplicationUpgradeJobRequest,
        manifest: &UpgradeManifest,
        client: &reqwest::Client,
        artifact_url_override: Option<&str>,
    ) -> AppResult<()> {
        self.run_upgrade_pipeline_with_dependencies(
            run,
            request,
            manifest,
            UpgradePipelineDependencies {
                client,
                artifact_url_override,
                ensure_available_space,
                rename: rename_path,
            },
        )
        .await
    }

    async fn run_upgrade_pipeline_with_dependencies(
        &self,
        run: &mut JobRunRecord,
        request: &ApplicationUpgradeJobRequest,
        manifest: &UpgradeManifest,
        dependencies: UpgradePipelineDependencies<'_>,
    ) -> AppResult<()> {
        if manifest.tag != request.expected_tag {
            return Err(AppError::Validation(
                "upgrade manifest tag does not match expectedTag".to_string(),
            ));
        }
        if manifest.version != request.expected_version {
            return Err(AppError::Validation(
                "upgrade manifest version does not match expectedVersion".to_string(),
            ));
        }
        let artifact = select_artifact(manifest, request.installation_kind)?.clone();

        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress {
                phase: phases::DOWNLOADING.to_string(),
                total_bytes: artifact.size,
                ..ApplicationUpgradeProgress::checking(request)
            },
        )
        .await?;
        let staging_dir = self.application_upgrade_staging_dir();
        recreate_staging_dir(&staging_dir)?;
        (dependencies.ensure_available_space)(&staging_dir, staging_space_requirement(&artifact))?;
        let download_path = staging_dir.join("artifact");
        download_artifact(
            self,
            run,
            request,
            dependencies.client,
            &artifact,
            dependencies.artifact_url_override,
            &download_path,
        )
        .await?;

        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress {
                phase: phases::VERIFYING.to_string(),
                downloaded_bytes: artifact.size,
                total_bytes: artifact.size,
                ..ApplicationUpgradeProgress::checking(request)
            },
        )
        .await?;
        verify_artifact_hash(&download_path, &artifact)?;
        validate_archive_members(&download_path, &artifact)?;

        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress {
                phase: phases::STAGING.to_string(),
                downloaded_bytes: artifact.size,
                total_bytes: artifact.size,
                ..ApplicationUpgradeProgress::checking(request)
            },
        )
        .await?;
        let extracted_dir = staging_dir.join("extracted");
        extract_archive(&download_path, &artifact, &extracted_dir)?;

        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress {
                phase: phases::APPLYING.to_string(),
                downloaded_bytes: artifact.size,
                total_bytes: artifact.size,
                ..ApplicationUpgradeProgress::checking(request)
            },
        )
        .await?;
        #[cfg(windows)]
        {
            return self
                .handoff_windows_upgrade(run, request, &artifact, &extracted_dir, &download_path)
                .await;
        }

        #[cfg(not(windows))]
        {
            let paths = portable_upgrade_paths(request, SCRYER_VERSION)?;
            let journal_path = self.application_upgrade_journal_path();
            let journal = ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: request.expected_version.clone(),
                expected_tag: request.expected_tag.clone(),
                executable_path: paths.executable_path.clone(),
                backup_path: paths.backup_path.clone(),
                backup_paths: vec![paths.backup_path.clone()],
                phase: phases::RESTARTING.to_string(),
                helper_error: None,
                written_at: Some(Utc::now()),
            };
            // The journal has to be durable before the binary moves: a crash in
            // between must never leave a promoted executable that the next boot
            // has no record of.
            write_journal(&journal_path, &journal)?;
            if let Err(failure) = apply_portable_upgrade(
                &extracted_dir,
                &artifact,
                &paths,
                &request.expected_version,
                dependencies.ensure_available_space,
                dependencies.rename,
            ) {
                let (error, restored) = failure.into_parts();
                if restored {
                    if let Err(cleanup_error) = remove_file_if_exists(&journal_path) {
                        tracing::warn!(
                            error = %cleanup_error,
                            "failed to remove the application upgrade journal after a restored promotion failure"
                        );
                    }
                } else {
                    tracing::error!(
                        journal_path = %journal_path.display(),
                        backup_path = %paths.backup_path.display(),
                        "portable application upgrade could not restore the previous executable; preserving recovery journal"
                    );
                }
                return Err(error);
            }

            if let Err(error) = self
                .update_application_upgrade_progress(
                    run,
                    ApplicationUpgradeProgress {
                        phase: phases::RESTARTING.to_string(),
                        downloaded_bytes: artifact.size,
                        total_bytes: artifact.size,
                        ..ApplicationUpgradeProgress::checking(request)
                    },
                )
                .await
            {
                return Err(roll_back_portable_promotion(
                    &paths,
                    &journal_path,
                    dependencies.rename,
                    error,
                ));
            }
            let restart = match self.application_upgrade_restart_handle() {
                Ok(restart) => restart,
                Err(error) => {
                    return Err(roll_back_portable_promotion(
                        &paths,
                        &journal_path,
                        dependencies.rename,
                        error,
                    ));
                }
            };
            restart.schedule_restart();
            Ok(())
        }
    }

    fn application_upgrade_restart_handle(
        &self,
    ) -> AppResult<crate::application_upgrade::ApplicationUpgradeRestartHandle> {
        self.runtime
            .jobs
            .application_upgrade_restart
            .read()
            .ok()
            .and_then(|handle| handle.clone())
            .ok_or_else(|| {
                AppError::Repository(
                    "application upgrade restart controller is not configured".to_string(),
                )
            })
    }

    #[cfg(windows)]
    async fn handoff_windows_upgrade(
        &self,
        run: &mut JobRunRecord,
        request: &ApplicationUpgradeJobRequest,
        artifact: &UpgradeArtifact,
        extracted_dir: &Path,
        msi_path: &Path,
    ) -> AppResult<()> {
        let executable_path = request
            .executable_path
            .clone()
            .or_else(|| std::env::current_exe().ok())
            .ok_or_else(|| {
                AppError::Repository("failed to resolve the running executable path".to_string())
            })?;
        let install_dir = executable_path.parent().map(PathBuf::from).ok_or_else(|| {
            AppError::Validation("running executable has no parent directory".to_string())
        })?;
        let direct_relaunch_args = std::env::args_os()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let direct_relaunch_cwd = std::env::current_dir().unwrap_or_else(|_| install_dir.clone());
        let handoff = build_windows_upgrade_handoff(WindowsUpgradeHandoffInput {
            run_id: &run.id,
            expected_version: &request.expected_version,
            expected_tag: &request.expected_tag,
            installation_kind: request.installation_kind,
            tray_supervised: request.tray_supervised,
            executable_path: &executable_path,
            install_dir: &install_dir,
            backend_process_id: std::process::id(),
            artifact: Some(artifact),
            extracted_dir: Some(extracted_dir),
            msi_path: Some(msi_path),
            journal_path: self.application_upgrade_journal_path(),
            direct_relaunch_args: &direct_relaunch_args,
            direct_relaunch_cwd: &direct_relaunch_cwd,
            current_version: SCRYER_VERSION,
            written_at: Utc::now(),
        })?;
        if let Some(existing_backup) = handoff
            .journal
            .backup_paths
            .iter()
            .find(|path| path.exists())
        {
            return Err(AppError::Validation(format!(
                "refusing to overwrite existing application backup '{}'",
                existing_backup.display()
            )));
        }
        write_journal(&handoff.plan.journal_path, &handoff.journal)?;
        self.update_application_upgrade_progress(
            run,
            ApplicationUpgradeProgress {
                phase: handoff.progress_phase.to_string(),
                downloaded_bytes: artifact.size,
                total_bytes: artifact.size,
                ..ApplicationUpgradeProgress::checking(request)
            },
        )
        .await?;
        let helper_dir = self.application_upgrade_helper_dir();
        let plan_path = helper_dir.join("plan.json");
        let helper_path = helper_dir.join("scryer-upgrade-helper.exe");
        write_helper_plan(&plan_path, &handoff.plan)?;
        copy_and_spawn_windows_upgrade_helper(&helper_path, &plan_path)?;
        self.application_upgrade_restart_handle()?.schedule_exit();
        Ok(())
    }

    async fn update_application_upgrade_progress(
        &self,
        run: &mut JobRunRecord,
        progress: ApplicationUpgradeProgress,
    ) -> AppResult<()> {
        run.progress_json = serde_json::to_string(&progress).ok();
        run.updated_at = chrono::Utc::now();
        let updated = self.services.events.job_runs.update_job_run(run).await?;
        *run = updated.clone();
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        Ok(())
    }

    async fn finish_application_upgrade_failure(
        &self,
        run: &mut JobRunRecord,
        actor: DomainEventActor,
        error_text: String,
    ) -> AppResult<()> {
        let mut progress = run
            .progress_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<ApplicationUpgradeProgress>(raw).ok())
            .unwrap_or(ApplicationUpgradeProgress {
                status: JobRunStatus::Running.as_str().to_string(),
                phase: phases::CHECKING.to_string(),
                downloaded_bytes: 0,
                total_bytes: 0,
                target_version: String::new(),
                target_tag: String::new(),
                error: None,
            });
        progress.status = JobRunStatus::Failed.as_str().to_string();
        progress.error = Some(error_text.clone());
        let now = chrono::Utc::now();
        run.status = JobRunStatus::Failed;
        run.progress_json = serde_json::to_string(&progress).ok();
        run.summary_text = Some("Application upgrade failed".to_string());
        run.error_text = Some(error_text.clone());
        run.completed_at = Some(now);
        run.updated_at = now;
        let updated = self.services.events.job_runs.update_job_run(run).await?;
        *run = updated.clone();
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor,
                updated.id.clone(),
                DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                    run_id: updated.id.clone(),
                    job_key: updated.job_key.as_str().to_string(),
                    error_text: Some(error_text),
                }),
            ))
            .await;
        Ok(())
    }

    /// Finalize the journal created before a restart and return runs that must
    /// remain running because an operating-system reboot is still required.
    pub async fn finalize_application_upgrade_journal(&self) -> AppResult<Vec<String>> {
        self.finalize_application_upgrade_journal_with_boot_time(None)
            .await
    }

    /// Finalize an upgrade journal with an injectable operating-system boot time.
    /// Windows hosts supply this from `GetTickCount64`; tests inject a fixed value.
    pub async fn finalize_application_upgrade_journal_with_boot_time(
        &self,
        boot_time: Option<SystemTime>,
    ) -> AppResult<Vec<String>> {
        let journal_path = self.application_upgrade_journal_path();
        let Some(journal) = load_journal(&journal_path)? else {
            return Ok(Vec::new());
        };
        if journal.schema != JOURNAL_SCHEMA {
            return Err(AppError::Validation(format!(
                "unsupported application upgrade journal schema '{}'",
                journal.schema
            )));
        }
        let current_executable = std::env::current_exe().map_err(|error| {
            AppError::Repository(format!("failed to resolve running executable: {error}"))
        })?;
        let expected_version_booted = SCRYER_VERSION == journal.expected_version;
        // Startup evidence records the canonical executable path, so the journal
        // comparison has to canonicalize too; otherwise a Homebrew or symlinked
        // layout looks like a boot of the wrong binary.
        let expected_executable_booted =
            canonical_path(&current_executable) == canonical_path(&journal.executable_path);
        if journal.phase == phases::REBOOT_REQUIRED {
            if reboot_required_completion_allowed(
                journal.written_at,
                boot_time.map(DateTime::<Utc>::from),
                expected_version_booted,
                expected_executable_booted,
            ) {
                self.complete_journal_application_upgrade(&journal, &journal_path)
                    .await?;
                return Ok(Vec::new());
            }
            // The run stays Running until the operator reboots. Rehydrate the
            // in-memory tracker so single-flight admission still sees it and a
            // second upgrade cannot start behind the pending one. A tracker
            // failure must not swallow the exclusion, or startup reconciliation
            // would fail the very run it is meant to preserve.
            if let Err(error) = self
                .rehydrate_application_upgrade_active_run(&journal.run_id)
                .await
            {
                tracing::warn!(
                    error = %error,
                    run_id = %journal.run_id,
                    "failed to re-register the application upgrade run awaiting a reboot"
                );
            }
            return Ok(vec![journal.run_id]);
        }
        if let Some(error) = journal.helper_error.clone() {
            self.finish_journal_application_upgrade(
                &journal,
                JobRunStatus::Failed,
                None,
                Some(error),
            )
            .await?;
            remove_file_if_exists(&journal_path)?;
            remove_dir_if_exists(&self.application_upgrade_staging_dir())?;
            remove_dir_if_exists(&self.application_upgrade_helper_dir())?;
            return Ok(Vec::new());
        }
        if journal.phase != phases::RESTARTING {
            return Err(AppError::Validation(format!(
                "unsupported application upgrade journal phase '{}'",
                journal.phase
            )));
        }

        if expected_version_booted && expected_executable_booted {
            self.complete_journal_application_upgrade(&journal, &journal_path)
                .await?;
            return Ok(Vec::new());
        }

        self.finish_journal_application_upgrade(
            &journal,
            JobRunStatus::Failed,
            None,
            Some("upgrade did not boot the expected version; backups preserved".to_string()),
        )
        .await?;
        Ok(Vec::new())
    }

    /// Put a still-running upgrade run back into the in-memory job tracker.
    ///
    /// The tracker is rebuilt from scratch on every start, so a run that
    /// survives a restart (an upgrade waiting for an operating-system reboot)
    /// is invisible to `has_active_job` until it is re-registered here.
    async fn rehydrate_application_upgrade_active_run(&self, run_id: &str) -> AppResult<()> {
        let Some(record) = self.services.events.job_runs.get_job_run(run_id).await? else {
            return Ok(());
        };
        if record.status.is_terminal() {
            return Ok(());
        }
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&record, None))
            .await;
        Ok(())
    }

    async fn complete_journal_application_upgrade(
        &self,
        journal: &ApplicationUpgradeJournal,
        journal_path: &Path,
    ) -> AppResult<()> {
        let old_version = self
            .services
            .events
            .job_runs
            .get_job_run(&journal.run_id)
            .await?
            .and_then(|run| {
                run.operation_type
                    .split_once(':')
                    .and_then(|(_, versions)| versions.split_once("->"))
                    .map(|(old, _)| old.to_string())
            })
            .unwrap_or_else(|| "previous version".to_string());
        self.finish_journal_application_upgrade(
            journal,
            JobRunStatus::Completed,
            Some(format!(
                "Upgraded application from {old_version} to {}",
                journal.expected_version
            )),
            None,
        )
        .await?;
        remove_file_if_exists(&journal.backup_path)?;
        for backup_path in &journal.backup_paths {
            remove_file_if_exists(backup_path)?;
        }
        remove_file_if_exists(journal_path)?;
        remove_dir_if_exists(&self.application_upgrade_staging_dir())?;
        remove_dir_if_exists(&self.application_upgrade_helper_dir())?;
        Ok(())
    }

    async fn finish_journal_application_upgrade(
        &self,
        journal: &ApplicationUpgradeJournal,
        status: JobRunStatus,
        summary_text: Option<String>,
        error_text: Option<String>,
    ) -> AppResult<()> {
        let mut run = self
            .services
            .events
            .job_runs
            .get_job_run(&journal.run_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("application upgrade run {}", journal.run_id))
            })?;
        // A run that already reached a terminal status was finalized by the
        // pipeline itself; re-finalizing would rewrite its outcome. Recovery
        // files are still cleaned up by the caller.
        if run.status.is_terminal() {
            tracing::info!(
                run_id = %run.id,
                status = %run.status.as_str(),
                "skipping application upgrade journal finalization for an already finished run"
            );
            return Ok(());
        }
        let mut progress = run
            .progress_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<ApplicationUpgradeProgress>(raw).ok())
            .unwrap_or(ApplicationUpgradeProgress {
                status: JobRunStatus::Running.as_str().to_string(),
                phase: phases::RESTARTING.to_string(),
                downloaded_bytes: 0,
                total_bytes: 0,
                target_version: journal.expected_version.clone(),
                target_tag: journal.expected_tag.clone(),
                error: None,
            });
        progress.status = status.as_str().to_string();
        progress.error = error_text.clone();
        let now = chrono::Utc::now();
        run.status = status;
        run.progress_json = serde_json::to_string(&progress).ok();
        run.summary_text = summary_text.clone();
        run.error_text = error_text.clone();
        run.completed_at = Some(now);
        run.updated_at = now;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        let payload = match status {
            JobRunStatus::Completed => {
                DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                    run_id: updated.id.clone(),
                    job_key: updated.job_key.as_str().to_string(),
                    summary_text,
                })
            }
            JobRunStatus::Failed => DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                error_text,
            }),
            _ => unreachable!("journal finalization only writes terminal statuses"),
        };
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                DomainEventActor::system(),
                updated.id.clone(),
                payload,
            ))
            .await;
        Ok(())
    }

    fn application_upgrade_root_dir(&self) -> PathBuf {
        self.runtime
            .environment
            .config_dir
            .as_ref()
            .join("application-upgrade")
    }

    fn application_upgrade_staging_dir(&self) -> PathBuf {
        self.application_upgrade_root_dir().join("staging")
    }

    fn application_upgrade_helper_dir(&self) -> PathBuf {
        self.application_upgrade_root_dir().join("helper")
    }

    fn application_upgrade_journal_path(&self) -> PathBuf {
        self.application_upgrade_root_dir().join("journal.json")
    }

    fn cleanup_application_upgrade_staging(&self) {
        if let Err(error) = remove_dir_if_exists(&self.application_upgrade_staging_dir()) {
            tracing::warn!(error = %error, "failed to clean application upgrade staging directory");
        }
    }
}

fn application_upgrade_http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .connect_timeout(Duration::from_secs(30))
        // Release artifacts can legitimately take longer than an ordinary API
        // request on slow links; retain a bounded long-running HTTP budget.
        .timeout(scryer_outbound_http::LONG_RUNNING_HTTP_OPERATION_TIMEOUT)
        .build()
        .map_err(|error| {
            AppError::Repository(format!("failed to build upgrade HTTP client: {error}"))
        })
}

async fn verify_upgrade_manifest_signature(
    manifest_raw: Vec<u8>,
    bundle_raw: Vec<u8>,
) -> AppResult<()> {
    verify_signed_blob(manifest_raw, bundle_raw, scryer_release_required_signer())
        .await
        .map_err(|error| {
            AppError::Validation(format!(
                "upgrade manifest signature verification failed: {error}"
            ))
        })
}

fn release_asset_url(tag: &str, filename: &str) -> AppResult<url::Url> {
    let mut url = url::Url::parse("https://github.com/scryer-media/scryer/releases/download/")
        .map_err(|error| AppError::Repository(format!("invalid release URL base: {error}")))?;
    url.path_segments_mut()
        .map_err(|_| {
            AppError::Repository("release URL base cannot accept path segments".to_string())
        })?
        .push(tag)
        .push(filename);
    Ok(url)
}

async fn fetch_capped_bytes(
    client: &reqwest::Client,
    url: &str,
    cap: u64,
    label: &str,
) -> AppResult<Vec<u8>> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| AppError::Repository(format!("failed to fetch {label}: {error}")))?
        .error_for_status()
        .map_err(|error| AppError::Repository(format!("failed to fetch {label}: {error}")))?;
    if response
        .content_length()
        .is_some_and(|content_length| content_length > cap)
    {
        return Err(AppError::Validation(format!(
            "{label} exceeds the maximum size of {cap} bytes"
        )));
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|error| AppError::Repository(format!("failed to read {label}: {error}")))?;
        let next_len = u64::try_from(bytes.len())
            .unwrap_or(u64::MAX)
            .saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        if next_len > cap {
            return Err(AppError::Validation(format!(
                "{label} exceeds the maximum size of {cap} bytes"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn select_artifact(
    manifest: &UpgradeManifest,
    installation_kind: InstallationKind,
) -> AppResult<&UpgradeArtifact> {
    let platform = match std::env::consts::OS {
        "macos" => UpgradePlatform::Darwin,
        "linux" => UpgradePlatform::Linux,
        "windows" => UpgradePlatform::Windows,
        os => {
            return Err(AppError::Validation(format!(
                "no application upgrade artifact is available for operating system {os}"
            )));
        }
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => UpgradeArchitecture::Arm64,
        "x86_64" => UpgradeArchitecture::X86_64,
        arch => {
            return Err(AppError::Validation(format!(
                "no application upgrade artifact is available for architecture {arch}"
            )));
        }
    };
    let channel = match installation_kind {
        InstallationKind::Portable => UpgradeChannel::Portable,
        InstallationKind::DirectMsi => UpgradeChannel::Msi,
        _ => {
            return Err(AppError::Validation(
                "application upgrade installation is not eligible".to_string(),
            ));
        }
    };
    manifest
        .artifacts
        .iter()
        .find(|artifact| {
            artifact.platform == platform && artifact.arch == arch && artifact.channel == channel
        })
        .ok_or_else(|| {
            AppError::Validation("no upgrade artifact is available for this platform".to_string())
        })
}

async fn download_artifact(
    app: &AppUseCase,
    run: &mut JobRunRecord,
    request: &ApplicationUpgradeJobRequest,
    client: &reqwest::Client,
    artifact: &UpgradeArtifact,
    artifact_url_override: Option<&str>,
    destination: &Path,
) -> AppResult<()> {
    let response = client
        .get(artifact_url_override.unwrap_or(&artifact.url))
        .send()
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to download upgrade artifact: {error}"))
        })?
        .error_for_status()
        .map_err(|error| {
            AppError::Repository(format!("failed to download upgrade artifact: {error}"))
        })?;
    if response
        .content_length()
        .is_some_and(|content_length| content_length > artifact.size)
    {
        return Err(AppError::Validation(
            "upgrade artifact exceeds the manifest size".to_string(),
        ));
    }

    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to create upgrade staging file: {error}"))
        })?;
    let mut downloaded = 0_u64;
    let mut hasher = blake3::Hasher::new();
    let mut last_progress = Instant::now() - DOWNLOAD_PROGRESS_INTERVAL;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            AppError::Repository(format!("failed to read upgrade artifact response: {error}"))
        })?;
        let next_downloaded =
            downloaded.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        if next_downloaded > artifact.size {
            return Err(AppError::Validation(
                "upgrade artifact exceeds the manifest size".to_string(),
            ));
        }
        file.write_all(&chunk).await.map_err(|error| {
            AppError::Repository(format!("failed to write upgrade staging file: {error}"))
        })?;
        hasher.update(&chunk);
        downloaded = next_downloaded;
        if last_progress.elapsed() >= DOWNLOAD_PROGRESS_INTERVAL {
            app.update_application_upgrade_progress(
                run,
                ApplicationUpgradeProgress {
                    phase: phases::DOWNLOADING.to_string(),
                    downloaded_bytes: downloaded,
                    total_bytes: artifact.size,
                    ..ApplicationUpgradeProgress::checking(request)
                },
            )
            .await?;
            last_progress = Instant::now();
        }
    }
    file.flush().await.map_err(|error| {
        AppError::Repository(format!("failed to flush upgrade staging file: {error}"))
    })?;
    if downloaded != artifact.size {
        return Err(AppError::Validation(format!(
            "upgrade artifact size mismatch: expected {} bytes, received {downloaded}",
            artifact.size
        )));
    }
    let expected_hash = blake3::Hash::from_hex(&artifact.blake3)
        .map_err(|error| AppError::Validation(format!("invalid manifest BLAKE3 hash: {error}")))?;
    if hasher.finalize() != expected_hash {
        return Err(AppError::Validation(
            "upgrade artifact BLAKE3 hash does not match the manifest".to_string(),
        ));
    }
    app.update_application_upgrade_progress(
        run,
        ApplicationUpgradeProgress {
            phase: phases::DOWNLOADING.to_string(),
            downloaded_bytes: downloaded,
            total_bytes: artifact.size,
            ..ApplicationUpgradeProgress::checking(request)
        },
    )
    .await
}

fn verify_artifact_hash(path: &Path, artifact: &UpgradeArtifact) -> AppResult<()> {
    let mut file = fs::File::open(path).map_err(|error| {
        AppError::Repository(format!("failed to open upgrade artifact: {error}"))
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            AppError::Repository(format!("failed to read upgrade artifact: {error}"))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if hasher.finalize().to_hex().as_str() != artifact.blake3 {
        return Err(AppError::Validation(
            "upgrade artifact BLAKE3 hash does not match the manifest".to_string(),
        ));
    }
    Ok(())
}

fn validate_archive_members(path: &Path, artifact: &UpgradeArtifact) -> AppResult<()> {
    match artifact.archive {
        UpgradeArchive::TarGz => validate_tar_members(path, artifact),
        UpgradeArchive::Msi => Ok(()),
    }
}

fn validate_tar_members(path: &Path, artifact: &UpgradeArtifact) -> AppResult<()> {
    let file = fs::File::open(path).map_err(|error| {
        AppError::Repository(format!("failed to open upgrade archive: {error}"))
    })?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let mut actual = BTreeMap::new();
    for entry in archive.entries().map_err(archive_error)? {
        let entry = entry.map_err(archive_error)?;
        let member_path = archive_member_path(entry.path().map_err(archive_error)?.as_ref())?;
        if !entry.header().entry_type().is_file() {
            return Err(AppError::Validation(format!(
                "upgrade archive member '{member_path}' is not a regular file"
            )));
        }
        let size = entry.size();
        if actual.insert(member_path.clone(), size).is_some() {
            return Err(AppError::Validation(format!(
                "upgrade archive has duplicate member '{member_path}'"
            )));
        }
    }
    ensure_member_set_matches(&actual, artifact)
}

fn ensure_member_set_matches(
    actual: &BTreeMap<String, u64>,
    artifact: &UpgradeArtifact,
) -> AppResult<()> {
    let expected = artifact
        .members
        .iter()
        .map(|member| (member.path.clone(), member.size))
        .collect::<BTreeMap<_, _>>();
    if actual != &expected {
        return Err(AppError::Validation(
            "upgrade archive members do not exactly match the signed manifest".to_string(),
        ));
    }
    Ok(())
}

fn extract_archive(path: &Path, artifact: &UpgradeArtifact, destination: &Path) -> AppResult<()> {
    fs::create_dir_all(destination).map_err(|error| {
        AppError::Repository(format!(
            "failed to create extracted upgrade directory: {error}"
        ))
    })?;
    match artifact.archive {
        UpgradeArchive::TarGz => extract_tar(path, artifact, destination),
        UpgradeArchive::Msi => Ok(()),
    }
}

fn extract_tar(path: &Path, artifact: &UpgradeArtifact, destination: &Path) -> AppResult<()> {
    let file = fs::File::open(path).map_err(|error| {
        AppError::Repository(format!("failed to open upgrade archive: {error}"))
    })?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let expected = artifact_member_paths(artifact);
    for entry in archive.entries().map_err(archive_error)? {
        let mut entry = entry.map_err(archive_error)?;
        let member_path = archive_member_path(entry.path().map_err(archive_error)?.as_ref())?;
        let member = expected.get(&member_path).ok_or_else(|| {
            AppError::Validation(format!("unexpected upgrade archive member '{member_path}'"))
        })?;
        if !entry.header().entry_type().is_file() || entry.size() != member.size {
            return Err(AppError::Validation(format!(
                "invalid upgrade archive member '{member_path}'"
            )));
        }
        let output = destination.join(&member_path);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(archive_error)?;
        }
        let mut output_file = fs::File::create(&output).map_err(archive_error)?;
        std::io::copy(&mut entry, &mut output_file).map_err(archive_error)?;
        set_extracted_permissions(
            &output,
            entry.header().mode().unwrap_or(0o644),
            member.executable,
        )?;
    }
    Ok(())
}

fn artifact_member_paths(
    artifact: &UpgradeArtifact,
) -> BTreeMap<String, crate::application_upgrade::manifest::UpgradeArtifactMember> {
    artifact
        .members
        .iter()
        .cloned()
        .map(|member| (member.path.clone(), member))
        .collect()
}

fn archive_member_path(path: &Path) -> AppResult<String> {
    let raw = path.to_string_lossy();
    let windows_drive_prefix = raw.as_bytes().get(1) == Some(&b':')
        && raw
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic());
    if path.is_absolute() || raw.starts_with('\\') || raw.contains('\\') || windows_drive_prefix {
        return Err(AppError::Validation(
            "upgrade archive contains an absolute member path".to_string(),
        ));
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => {
                components.push(component.to_string_lossy().to_string())
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::Validation(
                    "upgrade archive contains an unsafe member path".to_string(),
                ));
            }
        }
    }
    if components.is_empty() {
        return Err(AppError::Validation(
            "upgrade archive contains an empty member path".to_string(),
        ));
    }
    Ok(components.join("/"))
}

#[cfg(unix)]
fn set_extracted_permissions(path: &Path, mode: u32, executable: bool) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = if executable {
        mode | 0o111
    } else {
        mode & !0o111
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777)).map_err(|error| {
        AppError::Repository(format!(
            "failed to set extracted upgrade permissions: {error}"
        ))
    })
}

#[cfg(not(unix))]
fn set_extracted_permissions(_path: &Path, _mode: u32, _executable: bool) -> AppResult<()> {
    Ok(())
}

/// Resolve the executable and backup locations a portable promotion will use.
///
/// This performs every check that must precede the durable journal: the
/// installation must be portable, the executable must be resolvable and live in
/// a directory, and no earlier backup may be overwritten.
#[cfg_attr(windows, allow(dead_code))]
fn portable_upgrade_paths(
    request: &ApplicationUpgradeJobRequest,
    current_version: &str,
) -> AppResult<PortableUpgradePaths> {
    #[cfg(unix)]
    {
        if request.installation_kind != InstallationKind::Portable {
            return Err(AppError::Validation(
                "portable replacement is only available for portable installations".to_string(),
            ));
        }
        let executable_path = request
            .executable_path
            .clone()
            .or_else(|| std::env::current_exe().ok())
            .ok_or_else(|| {
                AppError::Repository("failed to resolve the running executable path".to_string())
            })?;
        if executable_path.parent().is_none() {
            return Err(AppError::Validation(
                "running executable has no parent directory".to_string(),
            ));
        }
        let backup_path = PathBuf::from(format!(
            "{}.pre-upgrade-{current_version}",
            executable_path.display()
        ));
        if backup_path.exists() {
            return Err(AppError::Validation(format!(
                "refusing to overwrite existing application backup '{}'",
                backup_path.display()
            )));
        }
        Ok(PortableUpgradePaths {
            executable_path,
            backup_path,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (request, current_version);
        Err(AppError::Validation(
            "portable replacement is not available on this platform".to_string(),
        ))
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn apply_portable_upgrade(
    extracted_dir: &Path,
    artifact: &UpgradeArtifact,
    paths: &PortableUpgradePaths,
    expected_version: &str,
    ensure_available_space: UpgradeSpaceCheck,
    rename: UpgradeRename,
) -> Result<(), PortablePromotionFailure> {
    #[cfg(unix)]
    {
        let executable_dir = paths.executable_path.parent().ok_or_else(|| {
            AppError::Validation("running executable has no parent directory".to_string())
        })?;
        let new_binary = find_upgraded_executable(extracted_dir, artifact, &paths.executable_path)?;
        let new_binary_size = fs::metadata(&new_binary)
            .map_err(|error| {
                AppError::Repository(format!("failed to stat upgraded executable: {error}"))
            })?
            .len();
        ensure_available_space(
            executable_dir,
            new_binary_size.saturating_add(UPGRADE_STAGING_RESERVE_BYTES),
        )?;
        let new_path = executable_dir.join(format!(".scryer-upgrade-new-{expected_version}"));
        fs::copy(&new_binary, &new_path).map_err(|error| {
            AppError::Repository(format!("failed to stage replacement executable: {error}"))
        })?;
        if let Err(error) = rename(&paths.executable_path, &paths.backup_path) {
            let _ = fs::remove_file(&new_path);
            return Err(AppError::Repository(format!(
                "failed to retain current executable backup: {error}"
            ))
            .into());
        }
        if let Err(error) = rename(&new_path, &paths.executable_path) {
            return match rename(&paths.backup_path, &paths.executable_path) {
                Ok(()) => {
                    let _ = fs::remove_file(&new_path);
                    Err(AppError::Repository(format!(
                        "failed to replace application executable: {error}; the previous executable was restored"
                    ))
                    .into())
                }
                Err(rollback_error) => Err(PortablePromotionFailure::RecoveryRequired(
                    AppError::Repository(format!(
                        "failed to replace application executable: {error}; failed to restore the previous executable from '{}': {rollback_error}",
                        paths.backup_path.display()
                    )),
                )),
            };
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (
            extracted_dir,
            artifact,
            paths,
            expected_version,
            ensure_available_space,
            rename,
        );
        Err(AppError::Validation(
            "portable replacement is not available on this platform".to_string(),
        )
        .into())
    }
}

/// Undo a completed promotion after a later step failed.
///
/// The backup is moved back over the newly installed executable. The journal is
/// removed only after that restoration succeeds so an interrupted rollback
/// retains the paths needed for recovery.
#[cfg(not(windows))]
fn roll_back_portable_promotion(
    paths: &PortableUpgradePaths,
    journal_path: &Path,
    rename: UpgradeRename,
    error: AppError,
) -> AppError {
    let outcome = match rename(&paths.backup_path, &paths.executable_path) {
        Ok(()) => {
            let mut outcome = "the previous executable was restored".to_string();
            if let Err(cleanup_error) = remove_file_if_exists(journal_path) {
                outcome.push_str(&format!(
                    "; the application upgrade journal could not be removed: {cleanup_error}"
                ));
            }
            outcome
        }
        Err(rollback_error) => format!(
            "the previous executable could not be restored from '{}': {rollback_error}; the recovery journal was retained",
            paths.backup_path.display()
        ),
    };
    AppError::Repository(format!(
        "application upgrade failed after the executable was replaced: {error}; {outcome}"
    ))
}

#[cfg(unix)]
fn find_upgraded_executable(
    extracted_dir: &Path,
    artifact: &UpgradeArtifact,
    executable_path: &Path,
) -> AppResult<PathBuf> {
    let current_name = executable_path.file_name();
    let exact = artifact
        .members
        .iter()
        .find(|member| member.executable && Path::new(&member.path).file_name() == current_name);
    let candidates = artifact
        .members
        .iter()
        .filter(|member| member.executable)
        .collect::<Vec<_>>();
    let selected = exact
        .or_else(|| (candidates.len() == 1).then_some(candidates[0]))
        .ok_or_else(|| {
            AppError::Validation(
                "upgrade archive does not identify a unique replacement executable".to_string(),
            )
        })?;
    Ok(extracted_dir.join(&selected.path))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn build_windows_upgrade_handoff(
    input: WindowsUpgradeHandoffInput<'_>,
) -> AppResult<WindowsUpgradeHandoff> {
    let owner = if input.tray_supervised {
        ApplicationUpgradeHelperOwner::Tray
    } else {
        ApplicationUpgradeHelperOwner::Direct
    };
    let tray_path = input.install_dir.join("scryer-tray.exe");
    let relaunch = if owner == ApplicationUpgradeHelperOwner::Tray {
        ApplicationUpgradeHelperRelaunch {
            program: tray_path.clone(),
            args: vec!["--login-start".to_string()],
            cwd: input.install_dir.to_path_buf(),
        }
    } else {
        ApplicationUpgradeHelperRelaunch {
            program: input.executable_path.to_path_buf(),
            args: input.direct_relaunch_args.to_vec(),
            cwd: input.direct_relaunch_cwd.to_path_buf(),
        }
    };
    let backup_suffix = format!(".pre-upgrade-{}", input.current_version);
    let (mode, replace, backup_paths, staged_dir, msi_path, progress_phase) = match input
        .installation_kind
    {
        InstallationKind::Portable => {
            let artifact = input.artifact.ok_or_else(|| {
                AppError::Validation(
                    "portable Windows upgrade handoff requires an artifact".to_string(),
                )
            })?;
            let extracted_dir = input.extracted_dir.ok_or_else(|| {
                AppError::Validation(
                    "portable Windows upgrade handoff requires an extracted directory".to_string(),
                )
            })?;
            let replacements =
                windows_portable_replacements(extracted_dir, artifact, input.install_dir)?;
            let backup_paths = replacements
                .iter()
                .map(|replacement| {
                    PathBuf::from(format!(
                        "{}{}",
                        replacement.to_install.display(),
                        backup_suffix
                    ))
                })
                .collect();
            (
                ApplicationUpgradeHelperMode::Portable,
                replacements,
                backup_paths,
                Some(extracted_dir.to_path_buf()),
                None,
                phases::RESTARTING,
            )
        }
        InstallationKind::DirectMsi => {
            let msi_path = input.msi_path.ok_or_else(|| {
                AppError::Validation(
                    "MSI Windows upgrade handoff requires an installer path".to_string(),
                )
            })?;
            (
                ApplicationUpgradeHelperMode::Msi,
                Vec::new(),
                Vec::new(),
                None,
                Some(msi_path.to_path_buf()),
                phases::AWAITING_ELEVATION,
            )
        }
        _ => {
            return Err(AppError::Validation(
                "application upgrade installation is not eligible".to_string(),
            ));
        }
    };
    let journal = ApplicationUpgradeJournal {
        schema: JOURNAL_SCHEMA.to_string(),
        run_id: input.run_id.to_string(),
        expected_version: input.expected_version.to_string(),
        expected_tag: input.expected_tag.to_string(),
        executable_path: input.executable_path.to_path_buf(),
        backup_path: PathBuf::from(format!(
            "{}{}",
            input.executable_path.display(),
            backup_suffix
        )),
        backup_paths,
        phase: phases::RESTARTING.to_string(),
        helper_error: None,
        written_at: Some(input.written_at),
    };
    let plan = ApplicationUpgradeHelperPlan {
        schema: APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA.to_string(),
        mode,
        owner,
        journal_path: input.journal_path,
        staged_dir,
        msi_path,
        install_dir: input.install_dir.to_path_buf(),
        wait_process_ids: vec![input.backend_process_id],
        replace,
        backup_suffix,
        relaunch,
        tray_shutdown_program: (owner == ApplicationUpgradeHelperOwner::Tray).then_some(tray_path),
        expected_version: input.expected_version.to_string(),
        expected_tag: input.expected_tag.to_string(),
    };
    plan.validate().map_err(AppError::Validation)?;
    Ok(WindowsUpgradeHandoff {
        journal,
        plan,
        progress_phase,
    })
}

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_portable_replacements(
    extracted_dir: &Path,
    artifact: &UpgradeArtifact,
    install_dir: &Path,
) -> AppResult<Vec<ApplicationUpgradeHelperReplacement>> {
    ["scryer.exe", "scryer-tray.exe"]
        .into_iter()
        .map(|filename| {
            let member = artifact
                .members
                .iter()
                .find(|member| {
                    Path::new(&member.path)
                        .file_name()
                        .is_some_and(|name| name == filename)
                })
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "upgrade archive does not contain required Windows executable '{filename}'"
                    ))
                })?;
            Ok(ApplicationUpgradeHelperReplacement {
                from_staged: extracted_dir.join(&member.path),
                to_install: install_dir.join(filename),
            })
        })
        .collect()
}

#[cfg(windows)]
fn write_helper_plan(path: &Path, plan: &ApplicationUpgradeHelperPlan) -> AppResult<()> {
    let parent = path.parent().ok_or_else(|| {
        AppError::Repository(
            "application upgrade helper plan path has no parent directory".to_string(),
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        AppError::Repository(format!(
            "failed to create application upgrade helper directory: {error}"
        ))
    })?;
    let bytes = serde_json::to_vec(plan).map_err(|error| {
        AppError::Repository(format!(
            "failed to encode application upgrade helper plan: {error}"
        ))
    })?;
    let temporary = parent.join(".plan.tmp");
    fs::write(&temporary, bytes).map_err(|error| {
        AppError::Repository(format!(
            "failed to write application upgrade helper plan: {error}"
        ))
    })?;
    fs::rename(&temporary, path).map_err(|error| {
        AppError::Repository(format!(
            "failed to activate application upgrade helper plan: {error}"
        ))
    })
}

#[cfg(windows)]
fn copy_and_spawn_windows_upgrade_helper(helper_path: &Path, plan_path: &Path) -> AppResult<()> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    let source = std::env::current_exe().map_err(|error| {
        AppError::Repository(format!(
            "failed to resolve upgrade helper source executable: {error}"
        ))
    })?;
    fs::copy(&source, helper_path).map_err(|error| {
        AppError::Repository(format!("failed to copy temporary upgrade helper: {error}"))
    })?;
    std::process::Command::new(helper_path)
        .arg("--upgrade-helper")
        .arg(plan_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| {
            AppError::Repository(format!("failed to spawn temporary upgrade helper: {error}"))
        })?;
    Ok(())
}

fn recreate_staging_dir(path: &Path) -> AppResult<()> {
    remove_dir_if_exists(path)?;
    fs::create_dir_all(path).map_err(|error| {
        AppError::Repository(format!(
            "failed to create upgrade staging directory: {error}"
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            AppError::Repository(format!(
                "failed to protect upgrade staging directory: {error}"
            ))
        })?;
    }
    Ok(())
}

/// Bytes the staging filesystem must hold: the downloaded artifact, everything
/// it decompresses into, and the fixed working reserve.
///
/// MSI artifacts declare no members, so their admission is the artifact plus the
/// reserve exactly as before.
fn staging_space_requirement(artifact: &UpgradeArtifact) -> u64 {
    artifact
        .members
        .iter()
        .fold(artifact.size, |total, member| {
            total.saturating_add(member.size)
        })
        .saturating_add(UPGRADE_STAGING_RESERVE_BYTES)
}

fn ensure_available_space(path: &Path, required_bytes: u64) -> AppResult<()> {
    let space = filesystem_space_raw(path).map_err(|error| {
        AppError::Repository(format!(
            "failed to inspect upgrade filesystem space: {error}"
        ))
    })?;
    if space.available_bytes < required_bytes {
        return Err(AppError::Validation(format!(
            "insufficient free space for application upgrade: need {required_bytes} bytes, have {} bytes",
            space.available_bytes
        )));
    }
    Ok(())
}

fn write_journal(path: &Path, journal: &ApplicationUpgradeJournal) -> AppResult<()> {
    let parent = path.parent().ok_or_else(|| {
        AppError::Repository("application upgrade journal path has no parent directory".to_string())
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        AppError::Repository(format!(
            "failed to create application upgrade journal directory: {error}"
        ))
    })?;
    let bytes = serde_json::to_vec(journal).map_err(|error| {
        AppError::Repository(format!(
            "failed to encode application upgrade journal: {error}"
        ))
    })?;
    let temporary = parent.join(format!(".journal-{}.tmp", journal.run_id));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|error| {
        AppError::Repository(format!(
            "failed to create application upgrade journal: {error}"
        ))
    })?;
    file.write_all(&bytes).map_err(|error| {
        AppError::Repository(format!(
            "failed to write application upgrade journal: {error}"
        ))
    })?;
    file.sync_all().map_err(|error| {
        AppError::Repository(format!(
            "failed to flush application upgrade journal: {error}"
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).map_err(|error| {
            AppError::Repository(format!(
                "failed to protect application upgrade journal: {error}"
            ))
        })?;
    }
    activate_journal(&temporary, path).map_err(|error| {
        AppError::Repository(format!(
            "failed to activate application upgrade journal: {error}"
        ))
    })
}

#[cfg(not(windows))]
fn activate_journal(temporary: &Path, path: &Path) -> std::io::Result<()> {
    fs::rename(temporary, path)
}

/// Atomically replace an existing journal on Windows.
///
/// `std::fs::rename` cannot replace an existing destination there. `MoveFileExW`
/// does, and `MOVEFILE_WRITE_THROUGH` keeps the helper's terminal state durable
/// before it relaunches the application.
#[cfg(windows)]
fn activate_journal(temporary: &Path, path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let existing = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replacement = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both paths are NUL-terminated UTF-16 buffers that outlive the call.
    if unsafe {
        MoveFileExW(
            existing.as_ptr(),
            replacement.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Resolve a path through symlinks, falling back to the path as given.
///
/// Startup evidence canonicalizes the running executable, so every comparison
/// against it must resolve the same way or a symlinked install (Homebrew's
/// `/usr/local/opt`, `/home/linuxbrew`) never matches itself.
fn canonical_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn load_journal(path: &Path) -> AppResult<Option<ApplicationUpgradeJournal>> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to read application upgrade journal: {error}"
            )));
        }
    };
    serde_json::from_slice(&raw).map(Some).map_err(|error| {
        AppError::Validation(format!("invalid application upgrade journal: {error}"))
    })
}

/// Persist a terminal status observed by the temporary upgrade helper.
///
/// The helper is intentionally hosted by the executable crate, so journal mutation
/// remains here with the schema owner rather than duplicating its atomic-write logic.
pub fn application_upgrade_helper_update_journal(
    path: &Path,
    phase: &str,
    helper_error: Option<String>,
) -> AppResult<()> {
    let mut journal = load_journal(path)?.ok_or_else(|| {
        AppError::NotFound(format!(
            "application upgrade journal '{}' was not found",
            path.display()
        ))
    })?;
    journal.phase = phase.to_string();
    journal.helper_error = helper_error;
    write_journal(path, &journal)
}

fn remove_file_if_exists(path: &Path) -> AppResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to remove application upgrade file '{}': {error}",
            path.display()
        ))),
    }
}

fn remove_dir_if_exists(path: &Path) -> AppResult<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to remove application upgrade staging directory '{}': {error}",
            path.display()
        ))),
    }
}

fn archive_error(error: impl std::fmt::Display) -> AppError {
    AppError::Validation(format!("invalid upgrade archive: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JobRunRepository;
    use crate::application_upgrade::manifest::{
        UPGRADE_MANIFEST_SCHEMA_VERSION, UpgradeArtifactMember,
    };
    use crate::application_upgrade::{ApplicationUpgradeRestartHandle, InstallationKind};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn archive_member_paths_reject_parent_components() {
        let error = archive_member_path(Path::new("bin/../scryer")).expect_err("unsafe path");
        assert!(error.to_string().contains("unsafe member path"));
    }

    #[test]
    fn archive_member_paths_reject_windows_paths_on_all_platforms() {
        for path in ["C:\\scryer", "bin\\scryer"] {
            let error = archive_member_path(Path::new(path)).expect_err("unsafe path");
            assert!(error.to_string().contains("absolute member path"));
        }
    }

    #[test]
    fn journal_round_trip_is_schema_stable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("application-upgrade/journal.json");
        let journal = ApplicationUpgradeJournal {
            schema: JOURNAL_SCHEMA.to_string(),
            run_id: "run-1".to_string(),
            expected_version: "0.18.22".to_string(),
            expected_tag: "v0.18.22".to_string(),
            executable_path: PathBuf::from("/opt/scryer/scryer"),
            backup_path: PathBuf::from("/opt/scryer/scryer.pre-upgrade-0.18.21"),
            backup_paths: vec![PathBuf::from("/opt/scryer/scryer.pre-upgrade-0.18.21")],
            phase: phases::RESTARTING.to_string(),
            helper_error: None,
            written_at: Some(Utc::now()),
        };
        write_journal(&path, &journal).expect("write journal");
        assert_eq!(load_journal(&path).expect("load journal"), Some(journal));
    }

    #[test]
    fn legacy_journal_without_additive_fields_still_parses() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("application-upgrade/journal.json");
        fs::create_dir_all(path.parent().expect("journal parent")).expect("create parent");
        fs::write(
            &path,
            r#"{
                "schema":"scryer.upgrade.journal.v1",
                "run_id":"run-1",
                "expected_version":"0.18.22",
                "expected_tag":"v0.18.22",
                "executable_path":"/opt/scryer/scryer",
                "backup_path":"/opt/scryer/scryer.pre-upgrade-0.18.21",
                "phase":"reboot_required",
                "helper_error":null
            }"#,
        )
        .expect("write legacy journal");
        let journal = load_journal(&path)
            .expect("load legacy journal")
            .expect("journal exists");
        assert!(journal.backup_paths.is_empty());
        assert_eq!(journal.written_at, None);
    }

    fn portable_tar_artifact(size: u64) -> UpgradeArtifact {
        UpgradeArtifact {
            platform: UpgradePlatform::Linux,
            arch: UpgradeArchitecture::X86_64,
            channel: UpgradeChannel::Portable,
            asset_name: "scryer.tar.gz".to_string(),
            url: "https://github.com/scryer-media/scryer/releases/download/v0.18.22/scryer.tar.gz"
                .to_string(),
            size: 0,
            blake3: "0".repeat(64),
            archive: UpgradeArchive::TarGz,
            members: vec![
                crate::application_upgrade::manifest::UpgradeArtifactMember {
                    path: "scryer".to_string(),
                    size,
                    executable: true,
                },
            ],
        }
    }

    fn windows_portable_artifact(members: Vec<UpgradeArtifactMember>) -> UpgradeArtifact {
        UpgradeArtifact {
            platform: UpgradePlatform::Windows,
            arch: UpgradeArchitecture::X86_64,
            channel: UpgradeChannel::Portable,
            asset_name: "scryer-windows-x86_64-portable.tar.gz".to_string(),
            url: "https://example.invalid/scryer-windows-x86_64-portable.tar.gz".to_string(),
            size: 0,
            blake3: "0".repeat(64),
            archive: UpgradeArchive::TarGz,
            members,
        }
    }

    fn windows_member(path: &str, size: u64) -> UpgradeArtifactMember {
        UpgradeArtifactMember {
            path: path.to_string(),
            size,
            executable: true,
        }
    }

    #[test]
    fn windows_handoff_builder_covers_portable_and_msi_direct_and_tray_owners() {
        let executable_path = PathBuf::from("C:/Scryer/scryer.exe");
        let install_dir = PathBuf::from("C:/Scryer");
        let extracted_dir = PathBuf::from("C:/data/application-upgrade/staging/extracted");
        let msi_path = PathBuf::from("C:/data/application-upgrade/staging/artifact");
        let journal_path = PathBuf::from("C:/data/application-upgrade/journal.json");
        let direct_args = vec!["--data-dir".to_string(), "C:/data".to_string()];
        let direct_cwd = PathBuf::from("C:/working");
        let artifact = windows_portable_artifact(vec![
            windows_member("bin/scryer.exe", 1),
            windows_member("bin/scryer-tray.exe", 1),
        ]);
        let written_at = Utc::now();

        for (installation_kind, tray_supervised) in [
            (InstallationKind::Portable, false),
            (InstallationKind::Portable, true),
            (InstallationKind::DirectMsi, false),
            (InstallationKind::DirectMsi, true),
        ] {
            let handoff = build_windows_upgrade_handoff(WindowsUpgradeHandoffInput {
                run_id: "run-1",
                expected_version: "99.0.0",
                expected_tag: "v99.0.0",
                installation_kind,
                tray_supervised,
                executable_path: &executable_path,
                install_dir: &install_dir,
                backend_process_id: 4242,
                artifact: Some(&artifact),
                extracted_dir: Some(&extracted_dir),
                msi_path: Some(&msi_path),
                journal_path: journal_path.clone(),
                direct_relaunch_args: &direct_args,
                direct_relaunch_cwd: &direct_cwd,
                current_version: "98.0.0",
                written_at,
            })
            .expect("build Windows upgrade handoff");

            handoff.plan.validate().expect("validate helper plan");
            assert_eq!(handoff.journal.phase, phases::RESTARTING);
            assert_eq!(handoff.journal.written_at, Some(written_at));
            assert_eq!(handoff.plan.backup_suffix, ".pre-upgrade-98.0.0");
            assert_eq!(handoff.plan.wait_process_ids, vec![4242]);
            assert_eq!(
                handoff.journal.backup_path,
                PathBuf::from("C:/Scryer/scryer.exe.pre-upgrade-98.0.0")
            );
            assert_eq!(handoff.plan.journal_path, journal_path);

            if tray_supervised {
                assert_eq!(handoff.plan.owner, ApplicationUpgradeHelperOwner::Tray);
                assert_eq!(
                    handoff.plan.relaunch.program,
                    install_dir.join("scryer-tray.exe")
                );
                assert_eq!(handoff.plan.relaunch.args, vec!["--login-start"]);
                assert_eq!(handoff.plan.relaunch.cwd, install_dir);
                assert_eq!(
                    handoff.plan.tray_shutdown_program,
                    Some(install_dir.join("scryer-tray.exe"))
                );
            } else {
                assert_eq!(handoff.plan.owner, ApplicationUpgradeHelperOwner::Direct);
                assert_eq!(handoff.plan.relaunch.program, executable_path);
                assert_eq!(handoff.plan.relaunch.args, direct_args);
                assert_eq!(handoff.plan.relaunch.cwd, direct_cwd);
                assert_eq!(handoff.plan.tray_shutdown_program, None);
            }

            match installation_kind {
                InstallationKind::Portable => {
                    assert_eq!(handoff.plan.mode, ApplicationUpgradeHelperMode::Portable);
                    assert_eq!(handoff.progress_phase, phases::RESTARTING);
                    assert_eq!(handoff.plan.staged_dir, Some(extracted_dir.clone()));
                    assert_eq!(handoff.plan.msi_path, None);
                    assert_eq!(
                        handoff.plan.replace,
                        vec![
                            ApplicationUpgradeHelperReplacement {
                                from_staged: extracted_dir.join("bin/scryer.exe"),
                                to_install: install_dir.join("scryer.exe"),
                            },
                            ApplicationUpgradeHelperReplacement {
                                from_staged: extracted_dir.join("bin/scryer-tray.exe"),
                                to_install: install_dir.join("scryer-tray.exe"),
                            },
                        ]
                    );
                    assert_eq!(
                        handoff.journal.backup_paths,
                        vec![
                            PathBuf::from("C:/Scryer/scryer.exe.pre-upgrade-98.0.0"),
                            PathBuf::from("C:/Scryer/scryer-tray.exe.pre-upgrade-98.0.0"),
                        ]
                    );
                }
                InstallationKind::DirectMsi => {
                    assert_eq!(handoff.plan.mode, ApplicationUpgradeHelperMode::Msi);
                    assert_eq!(handoff.progress_phase, phases::AWAITING_ELEVATION);
                    assert_eq!(handoff.plan.staged_dir, None);
                    assert_eq!(handoff.plan.msi_path, Some(msi_path.clone()));
                    assert!(handoff.plan.replace.is_empty());
                    assert!(handoff.journal.backup_paths.is_empty());
                }
                _ => unreachable!("test only covers eligible Windows installation kinds"),
            }
        }
    }

    #[test]
    fn tar_archive_members_must_match_the_signed_manifest_exactly() {
        let temp = tempfile::tempdir().expect("tempdir");
        let archive_path = temp.path().join("upgrade.tar.gz");
        let output = fs::File::create(&archive_path).expect("create archive");
        let encoder = flate2::write::GzEncoder::new(output, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let bytes = b"new executable";
        let mut header = tar::Header::new_gnu();
        header.set_path("scryer").expect("set path");
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append(&header, &bytes[..]).expect("append member");
        let encoder = archive.into_inner().expect("finish tar");
        encoder.finish().expect("finish gzip");

        let artifact = portable_tar_artifact(bytes.len() as u64);
        validate_archive_members(&archive_path, &artifact).expect("manifest member matches");

        let mismatch = portable_tar_artifact(bytes.len() as u64 + 1);
        let error = validate_archive_members(&archive_path, &mismatch)
            .expect_err("signed member size must match");
        assert!(error.to_string().contains("do not exactly match"));
    }

    /// The Windows portable artifact travels the same `.tar.gz` container as
    /// every other platform, so it gets the same member validation.
    const WINDOWS_ARCHIVE_MEMBERS: [(&str, &[u8], u32); 4] = [
        ("scryer.exe", b"windows backend".as_slice(), 0o755),
        ("scryer-tray.exe", b"windows tray".as_slice(), 0o755),
        ("LICENSE", b"license text".as_slice(), 0o644),
        ("README.txt", b"readme text".as_slice(), 0o644),
    ];

    fn write_windows_archive(directory: &Path, members: &[(&str, &[u8], u32)]) -> PathBuf {
        fs::create_dir_all(directory).expect("create archive directory");
        let archive_path = directory.join("scryer-windows-x86_64-portable.tar.gz");
        fs::write(&archive_path, tar_gz(members)).expect("write windows upgrade archive");
        archive_path
    }

    fn windows_manifest_members(members: &[(&str, &[u8], u32)]) -> Vec<UpgradeArtifactMember> {
        let mut members = members
            .iter()
            .map(|(path, bytes, mode)| UpgradeArtifactMember {
                path: (*path).to_string(),
                size: bytes.len() as u64,
                executable: mode & 0o111 != 0,
            })
            .collect::<Vec<_>>();
        members.sort_by(|left, right| left.path.cmp(&right.path));
        members
    }

    #[test]
    fn windows_tar_archive_members_must_match_the_signed_manifest_exactly() {
        let temp = tempfile::tempdir().expect("tempdir");
        let archive_path = write_windows_archive(temp.path(), &WINDOWS_ARCHIVE_MEMBERS);
        let artifact =
            windows_portable_artifact(windows_manifest_members(&WINDOWS_ARCHIVE_MEMBERS));
        validate_archive_members(&archive_path, &artifact).expect("archive matches the manifest");

        // A member the manifest never signed.
        let mut missing = artifact.clone();
        missing.members.retain(|member| member.path != "LICENSE");
        assert!(
            validate_archive_members(&archive_path, &missing)
                .expect_err("unsigned member is rejected")
                .to_string()
                .contains("do not exactly match")
        );

        // A signed member the archive does not carry.
        let mut extra = artifact.clone();
        extra.members.push(windows_member("scryer-extra.exe", 1));
        extra
            .members
            .sort_by(|left, right| left.path.cmp(&right.path));
        assert!(
            validate_archive_members(&archive_path, &extra)
                .expect_err("absent member is rejected")
                .to_string()
                .contains("do not exactly match")
        );

        // A member whose length differs from the signed length.
        let mut resized = artifact.clone();
        resized
            .members
            .iter_mut()
            .find(|member| member.path == "scryer.exe")
            .expect("backend member")
            .size += 1;
        assert!(
            validate_archive_members(&archive_path, &resized)
                .expect_err("resized member is rejected")
                .to_string()
                .contains("do not exactly match")
        );
    }

    #[test]
    fn windows_tar_archive_rejects_duplicate_and_non_regular_members() {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact =
            windows_portable_artifact(windows_manifest_members(&WINDOWS_ARCHIVE_MEMBERS));

        let mut duplicated = WINDOWS_ARCHIVE_MEMBERS.to_vec();
        duplicated.push(("scryer.exe", b"windows backend".as_slice(), 0o755));
        let duplicate_path = write_windows_archive(&temp.path().join("duplicate"), &duplicated);
        assert!(
            validate_archive_members(&duplicate_path, &artifact)
                .expect_err("duplicate member is rejected")
                .to_string()
                .contains("duplicate member")
        );

        let directory_path = temp.path().join("directory");
        fs::create_dir_all(&directory_path).expect("create archive directory");
        let archive_path = directory_path.join("scryer-windows-x86_64-portable.tar.gz");
        let encoder = flate2::write::GzEncoder::new(
            fs::File::create(&archive_path).expect("create archive"),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_path("bin").expect("set directory path");
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append(&header, &[][..]).expect("append directory");
        builder
            .into_inner()
            .expect("finish tar")
            .finish()
            .expect("finish gzip");
        assert!(
            validate_archive_members(&archive_path, &artifact)
                .expect_err("directory entry is rejected")
                .to_string()
                .contains("is not a regular file")
        );
    }

    #[test]
    fn windows_tar_artifact_hash_must_match_the_signed_manifest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let archive_path = write_windows_archive(temp.path(), &WINDOWS_ARCHIVE_MEMBERS);
        let bytes = fs::read(&archive_path).expect("read archive");

        let mut artifact =
            windows_portable_artifact(windows_manifest_members(&WINDOWS_ARCHIVE_MEMBERS));
        artifact.size = bytes.len() as u64;
        artifact.blake3 = blake3::hash(&bytes).to_hex().to_string();
        verify_artifact_hash(&archive_path, &artifact).expect("hash matches the manifest");

        artifact.blake3 = blake3::hash(b"other bytes").to_hex().to_string();
        assert!(
            verify_artifact_hash(&archive_path, &artifact)
                .expect_err("hash mismatch is rejected")
                .to_string()
                .contains("BLAKE3 hash does not match")
        );
    }

    #[test]
    fn windows_tar_extraction_produces_the_layout_the_helper_swap_expects() {
        let temp = tempfile::tempdir().expect("tempdir");
        let archive_path = write_windows_archive(temp.path(), &WINDOWS_ARCHIVE_MEMBERS);
        let artifact =
            windows_portable_artifact(windows_manifest_members(&WINDOWS_ARCHIVE_MEMBERS));
        let extracted_dir = temp.path().join("extracted");
        extract_archive(&archive_path, &artifact, &extracted_dir).expect("extract archive");

        for (path, bytes, _) in WINDOWS_ARCHIVE_MEMBERS {
            let output = extracted_dir.join(path);
            assert!(
                output.is_file(),
                "{path} is a regular file after extraction"
            );
            assert_eq!(fs::read(&output).expect("read extracted member"), bytes);
        }

        // The helper swaps the two executables by their manifest member paths,
        // so extraction must place them exactly where the plan will look.
        let install_dir = Path::new("C:/Program Files/Scryer");
        let replacements = windows_portable_replacements(&extracted_dir, &artifact, install_dir)
            .expect("build helper replacements");
        assert_eq!(
            replacements,
            vec![
                ApplicationUpgradeHelperReplacement {
                    from_staged: extracted_dir.join("scryer.exe"),
                    to_install: install_dir.join("scryer.exe"),
                },
                ApplicationUpgradeHelperReplacement {
                    from_staged: extracted_dir.join("scryer-tray.exe"),
                    to_install: install_dir.join("scryer-tray.exe"),
                },
            ]
        );
        for replacement in &replacements {
            assert!(
                replacement.from_staged.is_file(),
                "staged {} exists",
                replacement.from_staged.display()
            );
        }
    }

    #[test]
    fn windows_tar_extraction_rejects_members_the_manifest_never_signed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let archive_path = write_windows_archive(temp.path(), &WINDOWS_ARCHIVE_MEMBERS);
        let mut artifact =
            windows_portable_artifact(windows_manifest_members(&WINDOWS_ARCHIVE_MEMBERS));
        artifact
            .members
            .retain(|member| member.path != "README.txt");
        let error = extract_archive(&archive_path, &artifact, &temp.path().join("extracted"))
            .expect_err("unsigned member is rejected");
        assert!(
            error
                .to_string()
                .contains("unexpected upgrade archive member")
        );

        let mut resized =
            windows_portable_artifact(windows_manifest_members(&WINDOWS_ARCHIVE_MEMBERS));
        resized
            .members
            .iter_mut()
            .find(|member| member.path == "scryer-tray.exe")
            .expect("tray member")
            .size += 1;
        let error = extract_archive(&archive_path, &resized, &temp.path().join("resized"))
            .expect_err("resized member is rejected");
        assert!(error.to_string().contains("invalid upgrade archive member"));
    }

    #[cfg(unix)]
    fn test_request(executable_path: PathBuf) -> ApplicationUpgradeJobRequest {
        ApplicationUpgradeJobRequest {
            expected_tag: "v99.0.0".to_string(),
            expected_version: "99.0.0".to_string(),
            installation_kind: InstallationKind::Portable,
            executable_path: Some(executable_path),
            tray_supervised: false,
        }
    }

    #[cfg(unix)]
    fn test_run(request: &ApplicationUpgradeJobRequest) -> JobRunRecord {
        let now = chrono::Utc::now();
        JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::ApplicationUpgrade,
            operation_type: format!(
                "application_upgrade:{SCRYER_VERSION}->{}",
                request.expected_version
            ),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: None,
            progress_json: serde_json::to_string(&ApplicationUpgradeProgress::checking(request))
                .ok(),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[cfg(unix)]
    fn runtime_platform() -> UpgradePlatform {
        match std::env::consts::OS {
            "linux" => UpgradePlatform::Linux,
            "macos" => UpgradePlatform::Darwin,
            os => panic!("unsupported unix upgrade test platform {os}"),
        }
    }

    #[cfg(unix)]
    fn runtime_architecture() -> UpgradeArchitecture {
        match std::env::consts::ARCH {
            "x86_64" => UpgradeArchitecture::X86_64,
            "aarch64" => UpgradeArchitecture::Arm64,
            arch => panic!("unsupported upgrade test architecture {arch}"),
        }
    }

    fn tar_gz(members: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for (path, bytes, mode) in members {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).expect("set archive path");
            header.set_size(bytes.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            archive
                .append(&header, *bytes)
                .expect("append archive member");
        }
        archive
            .into_inner()
            .expect("finish archive")
            .finish()
            .expect("finish gzip")
    }

    #[cfg(unix)]
    fn portable_manifest(bytes: &[u8], members: Vec<UpgradeArtifactMember>) -> UpgradeManifest {
        UpgradeManifest {
            schema: UPGRADE_MANIFEST_SCHEMA_VERSION.to_string(),
            tag: "v99.0.0".to_string(),
            version: "99.0.0".to_string(),
            artifacts: vec![UpgradeArtifact {
                platform: runtime_platform(),
                arch: runtime_architecture(),
                channel: UpgradeChannel::Portable,
                asset_name: "scryer.tar.gz".to_string(),
                url:
                    "https://github.com/scryer-media/scryer/releases/download/v99.0.0/scryer.tar.gz"
                        .to_string(),
                size: bytes.len() as u64,
                blake3: blake3::hash(bytes).to_hex().to_string(),
                archive: UpgradeArchive::TarGz,
                members,
            }],
        }
    }

    #[cfg(unix)]
    fn executable_member(bytes: &[u8]) -> UpgradeArtifactMember {
        UpgradeArtifactMember {
            path: "scryer".to_string(),
            size: bytes.len() as u64,
            executable: true,
        }
    }

    #[cfg(unix)]
    async fn artifact_server(body: Vec<u8>) -> (MockServer, String) {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/artifact"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;
        let url = format!("{}/artifact", server.uri());
        (server, url)
    }

    #[cfg(unix)]
    fn test_http_client() -> reqwest::Client {
        scryer_outbound_http::install_default_rustls_provider();
        reqwest::Client::new()
    }

    #[cfg(unix)]
    async fn run_pipeline_and_finish_failure(
        app: &AppUseCase,
        run: &mut JobRunRecord,
        request: &ApplicationUpgradeJobRequest,
        manifest: &UpgradeManifest,
        dependencies: UpgradePipelineDependencies<'_>,
    ) -> AppError {
        let error = app
            .run_upgrade_pipeline_with_dependencies(run, request, manifest, dependencies)
            .await
            .expect_err("pipeline should fail");
        app.cleanup_application_upgrade_staging();
        app.finish_application_upgrade_failure(run, DomainEventActor::system(), error.to_string())
            .await
            .expect("persist failed application upgrade run");
        error
    }

    #[cfg(unix)]
    static REQUESTED_STAGING_BYTES: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);

    #[cfg(unix)]
    fn injected_insufficient_space(_path: &Path, required_bytes: u64) -> AppResult<()> {
        REQUESTED_STAGING_BYTES.store(required_bytes, Ordering::SeqCst);
        Err(AppError::Validation(
            "insufficient free space for application upgrade: injected test limit".to_string(),
        ))
    }

    #[cfg(unix)]
    fn fail_replacement_rename(from: &Path, to: &Path) -> std::io::Result<()> {
        if from
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(".scryer-upgrade-new-"))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected replacement rename failure",
            ));
        }
        fs::rename(from, to)
    }

    #[cfg(unix)]
    fn fail_replacement_and_rollback_rename(from: &Path, to: &Path) -> std::io::Result<()> {
        let name = from.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with(".scryer-upgrade-new-") || name.contains(".pre-upgrade-") {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected replacement and rollback rename failure",
            ));
        }
        fs::rename(from, to)
    }

    #[cfg(unix)]
    fn fail_post_promotion_rollback_rename(from: &Path, to: &Path) -> std::io::Result<()> {
        if from
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains(".pre-upgrade-"))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected post-promotion rollback rename failure",
            ));
        }
        fs::rename(from, to)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_happy_path_replaces_portable_executable_and_writes_restart_journal() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let restarted = Arc::new(AtomicBool::new(false));
        let restart_observed = Arc::clone(&restarted);
        app.set_application_upgrade_restart_handle(ApplicationUpgradeRestartHandle::new(
            move || {
                restart_observed.store(true, Ordering::SeqCst);
            },
        ));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        app.run_upgrade_pipeline_with_dependencies(
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: rename_path,
            },
        )
        .await
        .expect("portable upgrade pipeline succeeds");

        assert_eq!(
            fs::read(&executable_path).expect("replacement executable"),
            new_binary
        );
        let backup_path = PathBuf::from(format!(
            "{}.pre-upgrade-{SCRYER_VERSION}",
            executable_path.display()
        ));
        assert_eq!(
            fs::read(&backup_path).expect("backup executable"),
            b"old executable"
        );
        let journal = load_journal(&app.application_upgrade_journal_path())
            .expect("load journal")
            .expect("journal exists");
        assert_eq!(journal.phase, phases::RESTARTING);
        assert_eq!(journal.executable_path, executable_path);
        assert_eq!(journal.backup_path, backup_path);
        let progress: ApplicationUpgradeProgress =
            serde_json::from_str(run.progress_json.as_deref().expect("running progress"))
                .expect("decode progress");
        assert_eq!(run.status, JobRunStatus::Running);
        assert_eq!(progress.status, JobRunStatus::Running.as_str());
        assert_eq!(progress.phase, phases::RESTARTING);
        assert!(restarted.load(Ordering::SeqCst));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_blake3_mismatch_fails_and_cleans_staging_without_touching_executable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let mut manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        manifest.artifacts[0].blake3 = "0".repeat(64);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: rename_path,
            },
        )
        .await;

        assert!(error.to_string().contains("BLAKE3 hash does not match"));
        assert_eq!(run.status, JobRunStatus::Failed);
        assert!(!app.application_upgrade_staging_dir().exists());
        assert_eq!(
            fs::read(&executable_path).expect("original executable"),
            b"old executable"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_oversize_response_fails_with_manifest_size_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let mut manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        manifest.artifacts[0].size = manifest.artifacts[0].size.saturating_sub(1);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: rename_path,
            },
        )
        .await;

        assert!(error.to_string().contains("exceeds the manifest size"));
        assert_eq!(run.status, JobRunStatus::Failed);
        assert!(!app.application_upgrade_staging_dir().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_archive_member_mismatch_fails_before_apply() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[
            ("scryer", new_binary, 0o755),
            ("unexpected.txt", b"extra member", 0o644),
        ]);
        let manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: rename_path,
            },
        )
        .await;

        assert!(error.to_string().contains("members do not exactly match"));
        assert_eq!(run.status, JobRunStatus::Failed);
        assert_eq!(
            fs::read(&executable_path).expect("original executable"),
            b"old executable"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_insufficient_space_uses_injected_space_check() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path);
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: None,
                ensure_available_space: injected_insufficient_space,
                rename: rename_path,
            },
        )
        .await;

        assert!(
            error
                .to_string()
                .contains("insufficient free space for application upgrade")
        );
        assert_eq!(run.status, JobRunStatus::Failed);
        assert!(!app.application_upgrade_staging_dir().exists());
        // Staging admission must budget for the decompressed members as well as
        // the compressed artifact.
        let artifact = &manifest.artifacts[0];
        assert_eq!(
            REQUESTED_STAGING_BYTES.load(Ordering::SeqCst),
            artifact.size + new_binary.len() as u64 + UPGRADE_STAGING_RESERVE_BYTES
        );
    }

    #[test]
    fn staging_admission_includes_every_decompressed_member() {
        let mut artifact = portable_tar_artifact(10);
        artifact.size = 7;
        assert_eq!(
            staging_space_requirement(&artifact),
            7 + 10 + UPGRADE_STAGING_RESERVE_BYTES
        );

        artifact.members.push(UpgradeArtifactMember {
            path: "scryer-tray".to_string(),
            size: 5,
            executable: true,
        });
        assert_eq!(
            staging_space_requirement(&artifact),
            7 + 10 + 5 + UPGRADE_STAGING_RESERVE_BYTES
        );

        // MSI artifacts declare no members, so their admission is unchanged.
        artifact.members.clear();
        assert_eq!(
            staging_space_requirement(&artifact),
            7 + UPGRADE_STAGING_RESERVE_BYTES
        );

        let mut saturating = portable_tar_artifact(u64::MAX);
        saturating.size = u64::MAX;
        assert_eq!(staging_space_requirement(&saturating), u64::MAX);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_apply_rename_failure_restores_original_executable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: fail_replacement_rename,
            },
        )
        .await;

        let backup_path = PathBuf::from(format!(
            "{}.pre-upgrade-{SCRYER_VERSION}",
            executable_path.display()
        ));
        assert!(
            error
                .to_string()
                .contains("failed to replace application executable")
        );
        assert_eq!(run.status, JobRunStatus::Failed);
        assert_eq!(
            fs::read(&executable_path).expect("rolled-back executable"),
            b"old executable"
        );
        assert!(!backup_path.exists(), "backup should have been rolled back");
        assert!(
            !app.application_upgrade_journal_path().exists(),
            "a failed promotion must not leave its journal behind"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_preserves_the_journal_when_apply_rollback_fails() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: fail_replacement_and_rollback_rename,
            },
        )
        .await;

        let backup_path = PathBuf::from(format!(
            "{}.pre-upgrade-{SCRYER_VERSION}",
            executable_path.display()
        ));
        assert!(
            error
                .to_string()
                .contains("failed to restore the previous executable")
        );
        assert_eq!(run.status, JobRunStatus::Failed);
        assert!(
            !executable_path.exists(),
            "failed restoration leaves no live executable"
        );
        assert_eq!(
            fs::read(&backup_path).expect("preserved backup"),
            b"old executable"
        );
        assert!(
            app.application_upgrade_journal_path().exists(),
            "a failed restoration must retain the recovery journal"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_rolls_the_promotion_back_when_a_post_promotion_step_fails() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        // No restart handle is configured, so the step after promotion fails.
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: rename_path,
            },
        )
        .await;

        let message = error.to_string();
        assert!(
            message.contains("restart controller is not configured"),
            "error should name the original failure: {message}"
        );
        assert!(
            message.contains("the previous executable was restored"),
            "error should name the rollback outcome: {message}"
        );
        assert_eq!(run.status, JobRunStatus::Failed);
        assert_eq!(
            fs::read(&executable_path).expect("rolled-back executable"),
            b"old executable"
        );
        let backup_path = PathBuf::from(format!(
            "{}.pre-upgrade-{SCRYER_VERSION}",
            executable_path.display()
        ));
        assert!(!backup_path.exists(), "backup should have been rolled back");
        assert!(
            !app.application_upgrade_journal_path().exists(),
            "a rolled-back promotion must leave no journal"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pipeline_retains_recovery_state_when_post_promotion_rollback_fails() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable_path = temp.path().join("bin/scryer");
        fs::create_dir_all(executable_path.parent().expect("executable parent"))
            .expect("create executable directory");
        fs::write(&executable_path, b"old executable").expect("write old executable");
        let new_binary = b"new executable";
        let archive = tar_gz(&[("scryer", new_binary, 0o755)]);
        let manifest = portable_manifest(&archive, vec![executable_member(new_binary)]);
        let (_server, artifact_url) = artifact_server(archive).await;
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(executable_path.clone());
        let mut run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let client = test_http_client();

        let error = run_pipeline_and_finish_failure(
            &app,
            &mut run,
            &request,
            &manifest,
            UpgradePipelineDependencies {
                client: &client,
                artifact_url_override: Some(&artifact_url),
                ensure_available_space,
                rename: fail_post_promotion_rollback_rename,
            },
        )
        .await;

        let backup_path = PathBuf::from(format!(
            "{}.pre-upgrade-{SCRYER_VERSION}",
            executable_path.display()
        ));
        assert!(
            error
                .to_string()
                .contains("the recovery journal was retained")
        );
        assert_eq!(run.status, JobRunStatus::Failed);
        assert_eq!(
            fs::read(&executable_path).expect("promoted executable"),
            new_binary
        );
        assert_eq!(
            fs::read(&backup_path).expect("preserved backup"),
            b"old executable"
        );
        assert!(
            app.application_upgrade_journal_path().exists(),
            "a failed rollback must retain the recovery journal"
        );
    }

    #[test]
    fn helper_journal_updates_replace_an_existing_journal_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("application-upgrade/journal.json");
        let journal = ApplicationUpgradeJournal {
            schema: JOURNAL_SCHEMA.to_string(),
            run_id: "run-1".to_string(),
            expected_version: "0.18.22".to_string(),
            expected_tag: "v0.18.22".to_string(),
            executable_path: PathBuf::from("C:/Scryer/scryer.exe"),
            backup_path: PathBuf::from("C:/Scryer/scryer.exe.pre-upgrade-0.18.21"),
            backup_paths: vec![PathBuf::from("C:/Scryer/scryer.exe.pre-upgrade-0.18.21")],
            phase: phases::RESTARTING.to_string(),
            helper_error: None,
            written_at: Some(Utc::now()),
        };
        write_journal(&path, &journal).expect("write journal");

        application_upgrade_helper_update_journal(
            &path,
            phases::REBOOT_REQUIRED,
            Some("elevation was declined".to_string()),
        )
        .expect("update an existing journal in place");

        let updated = load_journal(&path)
            .expect("load updated journal")
            .expect("journal exists");
        assert_eq!(updated.phase, phases::REBOOT_REQUIRED);
        assert_eq!(
            updated.helper_error.as_deref(),
            Some("elevation was declined")
        );
        assert_eq!(updated.run_id, journal.run_id);
        assert_eq!(updated.written_at, journal.written_at);
    }

    #[tokio::test]
    async fn tampered_signature_is_rejected_by_the_real_sigstore_verifier() {
        let error = verify_upgrade_manifest_signature(
            b"{\"schema\":\"scryer.upgrade.manifest.v1\"}".to_vec(),
            b"not a sigstore bundle".to_vec(),
        )
        .await
        .expect_err("garbage signature bundle must be rejected");
        assert!(
            error
                .to_string()
                .contains("upgrade manifest signature verification failed")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_finalization_completes_matching_boot_and_cleans_recovery_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let executable_path = std::env::current_exe().expect("current executable");
        let request = test_request(executable_path.clone());
        let run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let backup_path = temp.path().join("scryer.pre-upgrade");
        fs::write(&backup_path, b"backup").expect("write backup");
        let staging_file = app.application_upgrade_staging_dir().join("artifact");
        fs::create_dir_all(staging_file.parent().expect("staging parent")).expect("create staging");
        fs::write(&staging_file, b"staged artifact").expect("write staging");
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: SCRYER_VERSION.to_string(),
                expected_tag: request.expected_tag.clone(),
                executable_path,
                backup_path: backup_path.clone(),
                backup_paths: vec![backup_path.clone()],
                phase: phases::RESTARTING.to_string(),
                helper_error: None,
                written_at: Some(Utc::now()),
            },
        )
        .expect("write journal");

        assert!(
            app.finalize_application_upgrade_journal()
                .await
                .expect("finalize journal")
                .is_empty()
        );

        let finalized = job_runs
            .get_job_run(&run.id)
            .await
            .expect("load finalized run")
            .expect("run exists");
        assert_eq!(finalized.status, JobRunStatus::Completed);
        assert!(!backup_path.exists());
        assert!(!app.application_upgrade_journal_path().exists());
        assert!(!app.application_upgrade_staging_dir().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_finalization_records_helper_error_and_preserves_backup() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(temp.path().join("bin/scryer"));
        let run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let backup_path = temp.path().join("scryer.pre-upgrade");
        fs::write(&backup_path, b"backup").expect("write backup");
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: request.expected_version.clone(),
                expected_tag: request.expected_tag.clone(),
                executable_path: request.executable_path.clone().expect("executable path"),
                backup_path: backup_path.clone(),
                backup_paths: vec![backup_path.clone()],
                phase: phases::RESTARTING.to_string(),
                helper_error: Some("elevation helper failed".to_string()),
                written_at: Some(Utc::now()),
            },
        )
        .expect("write journal");

        app.finalize_application_upgrade_journal()
            .await
            .expect("finalize helper failure");

        let finalized = job_runs
            .get_job_run(&run.id)
            .await
            .expect("load finalized run")
            .expect("run exists");
        assert_eq!(finalized.status, JobRunStatus::Failed);
        assert_eq!(
            finalized.error_text.as_deref(),
            Some("elevation helper failed")
        );
        assert!(backup_path.exists());
        assert!(!app.application_upgrade_journal_path().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_finalization_preserves_files_when_boot_version_mismatches() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(temp.path().join("bin/scryer"));
        let run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let backup_path = temp.path().join("scryer.pre-upgrade");
        fs::write(&backup_path, b"backup").expect("write backup");
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: "0.0.0".to_string(),
                expected_tag: request.expected_tag.clone(),
                executable_path: request.executable_path.clone().expect("executable path"),
                backup_path: backup_path.clone(),
                backup_paths: vec![backup_path.clone()],
                phase: phases::RESTARTING.to_string(),
                helper_error: None,
                written_at: Some(Utc::now()),
            },
        )
        .expect("write journal");

        app.finalize_application_upgrade_journal()
            .await
            .expect("finalize mismatch");

        let finalized = job_runs
            .get_job_run(&run.id)
            .await
            .expect("load finalized run")
            .expect("run exists");
        assert_eq!(finalized.status, JobRunStatus::Failed);
        assert!(
            finalized
                .error_text
                .as_deref()
                .is_some_and(|error| error.contains("backups preserved"))
        );
        assert!(backup_path.exists());
        assert!(app.application_upgrade_journal_path().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_finalization_leaves_reboot_required_run_untouched() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(temp.path().join("bin/scryer"));
        let run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let backup_path = temp.path().join("scryer.pre-upgrade");
        fs::write(&backup_path, b"backup").expect("write backup");
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: request.expected_version.clone(),
                expected_tag: request.expected_tag.clone(),
                executable_path: request.executable_path.clone().expect("executable path"),
                backup_path: backup_path.clone(),
                backup_paths: vec![backup_path.clone()],
                phase: phases::REBOOT_REQUIRED.to_string(),
                helper_error: None,
                written_at: None,
            },
        )
        .expect("write journal");

        assert_eq!(
            app.finalize_application_upgrade_journal()
                .await
                .expect("finalize reboot journal"),
            vec![run.id.clone()]
        );

        let unchanged = job_runs
            .get_job_run(&run.id)
            .await
            .expect("load run")
            .expect("run exists");
        assert_eq!(unchanged.status, JobRunStatus::Running);
        assert!(backup_path.exists());
        assert!(app.application_upgrade_journal_path().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_finalization_completes_reboot_required_after_a_new_boot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let executable_path = std::env::current_exe().expect("current executable");
        let request = test_request(executable_path.clone());
        let run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let backup_path = temp.path().join("scryer.pre-upgrade");
        let tray_backup_path = temp.path().join("scryer-tray.pre-upgrade");
        fs::write(&backup_path, b"backup").expect("write backup");
        fs::write(&tray_backup_path, b"tray backup").expect("write tray backup");
        let helper_file = app.application_upgrade_helper_dir().join("plan.json");
        fs::create_dir_all(helper_file.parent().expect("helper parent")).expect("create helper");
        fs::write(&helper_file, b"helper plan").expect("write helper plan");
        let staging_file = app.application_upgrade_staging_dir().join("artifact");
        fs::create_dir_all(staging_file.parent().expect("staging parent")).expect("create staging");
        fs::write(&staging_file, b"staged artifact").expect("write staging");
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: SCRYER_VERSION.to_string(),
                expected_tag: request.expected_tag.clone(),
                executable_path,
                backup_path: backup_path.clone(),
                backup_paths: vec![backup_path.clone(), tray_backup_path.clone()],
                phase: phases::REBOOT_REQUIRED.to_string(),
                helper_error: None,
                written_at: Some(Utc::now() - chrono::Duration::seconds(5)),
            },
        )
        .expect("write journal");

        assert!(
            app.finalize_application_upgrade_journal_with_boot_time(Some(SystemTime::now()))
                .await
                .expect("finalize reboot journal")
                .is_empty()
        );
        assert_eq!(
            job_runs
                .get_job_run(&run.id)
                .await
                .expect("load run")
                .expect("run exists")
                .status,
            JobRunStatus::Completed
        );
        assert!(!backup_path.exists());
        assert!(!tray_backup_path.exists());
        assert!(!app.application_upgrade_journal_path().exists());
        assert!(!app.application_upgrade_staging_dir().exists());
        assert!(!app.application_upgrade_helper_dir().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_finalization_excludes_reboot_required_before_a_new_boot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let executable_path = std::env::current_exe().expect("current executable");
        let request = test_request(executable_path.clone());
        let run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let backup_path = temp.path().join("scryer.pre-upgrade");
        fs::write(&backup_path, b"backup").expect("write backup");
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: SCRYER_VERSION.to_string(),
                expected_tag: request.expected_tag.clone(),
                executable_path,
                backup_path: backup_path.clone(),
                backup_paths: vec![backup_path.clone()],
                phase: phases::REBOOT_REQUIRED.to_string(),
                helper_error: None,
                written_at: Some(Utc::now()),
            },
        )
        .expect("write journal");

        assert_eq!(
            app.finalize_application_upgrade_journal_with_boot_time(Some(
                SystemTime::now() - Duration::from_secs(60)
            ))
            .await
            .expect("finalize reboot journal"),
            vec![run.id.clone()]
        );
        assert_eq!(
            job_runs
                .get_job_run(&run.id)
                .await
                .expect("load run")
                .expect("run exists")
                .status,
            JobRunStatus::Running
        );
        assert!(backup_path.exists());
        assert!(app.application_upgrade_journal_path().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reboot_required_run_stays_single_flight_after_a_restart() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let executable_path = std::env::current_exe().expect("current executable");
        let request = test_request(executable_path.clone());
        let run = test_run(&request);
        job_runs.seed(run.clone()).await;
        let backup_path = temp.path().join("scryer.pre-upgrade");
        fs::write(&backup_path, b"backup").expect("write backup");
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: request.expected_version.clone(),
                expected_tag: request.expected_tag.clone(),
                executable_path,
                backup_path: backup_path.clone(),
                backup_paths: vec![backup_path],
                phase: phases::REBOOT_REQUIRED.to_string(),
                helper_error: None,
                written_at: Some(Utc::now()),
            },
        )
        .expect("write journal");

        assert!(
            !app.runtime
                .jobs
                .job_run_tracker
                .has_active_job(JobKey::ApplicationUpgrade)
                .await,
            "a fresh process starts with an empty job tracker"
        );
        assert_eq!(
            app.finalize_application_upgrade_journal()
                .await
                .expect("finalize reboot journal"),
            vec![run.id.clone()]
        );
        assert!(
            app.runtime
                .jobs
                .job_run_tracker
                .has_active_job(JobKey::ApplicationUpgrade)
                .await,
            "the pending reboot run must be tracked as active again"
        );
        assert_eq!(
            app.runtime
                .jobs
                .job_run_tracker
                .active_run_for_job(JobKey::ApplicationUpgrade)
                .await
                .map(|tracked| tracked.id),
            Some(run.id.clone())
        );

        app.upsert_system_setting_json(
            "smg.scryer_update_notice",
            &crate::SmgScryerUpdateNotice {
                available: true,
                current_version: SCRYER_VERSION.to_string(),
                latest_version: request.expected_version.clone(),
                latest_tag: request.expected_tag.clone(),
                release_url: None,
                published_at: None,
                checked_at: Utc::now().to_rfc3339(),
            },
            None,
        )
        .await
        .expect("seed update notice");

        let error = app
            .start_application_upgrade_job(&actor, test_request(temp.path().join("bin/scryer")))
            .await
            .expect_err("a second upgrade must be refused while one awaits reboot");
        assert!(
            error.to_string().contains("already running"),
            "unexpected error: {error}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn journal_finalization_does_not_rewrite_an_already_finished_run() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (app, _actor, job_runs) =
            crate::lib_tests::bootstrap_application_upgrade(temp.path().join("data"));
        let request = test_request(temp.path().join("bin/scryer"));
        let mut run = test_run(&request);
        run.status = JobRunStatus::Completed;
        run.summary_text = Some("Application upgrade completed".to_string());
        run.completed_at = Some(Utc::now());
        job_runs.seed(run.clone()).await;
        write_journal(
            &app.application_upgrade_journal_path(),
            &ApplicationUpgradeJournal {
                schema: JOURNAL_SCHEMA.to_string(),
                run_id: run.id.clone(),
                expected_version: request.expected_version.clone(),
                expected_tag: request.expected_tag.clone(),
                executable_path: request.executable_path.clone().expect("executable path"),
                backup_path: temp.path().join("scryer.pre-upgrade"),
                backup_paths: Vec::new(),
                phase: phases::RESTARTING.to_string(),
                helper_error: Some("elevation helper failed".to_string()),
                written_at: Some(Utc::now()),
            },
        )
        .expect("write journal");

        app.finalize_application_upgrade_journal()
            .await
            .expect("finalize journal for a finished run");

        let unchanged = job_runs
            .get_job_run(&run.id)
            .await
            .expect("load run")
            .expect("run exists");
        assert_eq!(unchanged.status, JobRunStatus::Completed);
        assert_eq!(unchanged.error_text, None);
        assert_eq!(
            unchanged.summary_text.as_deref(),
            Some("Application upgrade completed")
        );
        assert!(
            !app.application_upgrade_journal_path().exists(),
            "recovery files are still cleaned up"
        );
    }
}
