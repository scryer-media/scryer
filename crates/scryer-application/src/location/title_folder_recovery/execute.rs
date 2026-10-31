use super::*;
use crate::domain_events::{DomainEventActor, new_title_domain_event, title_context_snapshot};
use crate::location::execution::{ImportFilePermissionsApplier, PlacedContentPermissions};
use crate::location::executor::*;
use crate::location::ownership_guard::LiveRunnerGuard;
use crate::location::verify::{
    CopyProgress, VerifiedCopier, VerifiedFile, hash_existing_file_with_progress,
};
use async_trait::async_trait;
use chrono::Utc;
use scryer_domain::{DomainEventPayload, TitleUpdatedEventData};

#[cfg(all(test, unix))]
#[path = "execute_tests.rs"]
mod tests;

impl RecoveryPlan {
    pub fn operation_id(&self) -> String {
        format!("{RECOVERY_VERSION}:{}", self.title.id)
    }
    pub fn work(&self) -> OperationWorkPlan {
        OperationWorkPlan::new(vec![PlannedTitle {
            title_id: self.title.id.clone(),
            sequence: 0,
            classification: None,
            placement: TitleCheckpointPlacement {
                source_library_id: Some(self.title.library_id.clone()),
                source_root_id: Some(self.title.root_folder_id.clone()),
                source_folder_path: self.title.folder_path.clone(),
                destination_library_id: Some(self.title.library_id.clone()),
                destination_root_id: self.destination_root.as_ref().map(|r| r.id.clone()),
                destination_folder_path: self.destination.clone(),
                ..Default::default()
            },
            files: self
                .files
                .iter()
                .map(|f| PlannedFile {
                    media_file_id: Some(f.media.id.clone()),
                    source_path: stored_path_to_path_buf(&f.media.file_path),
                    destination_path: stored_path_to_path_buf(&f.destination),
                    size_bytes: f.media.size_bytes.max(0) as u64,
                    source_content: None,
                })
                .collect(),
            outcomes: Default::default(),
        }])
    }
}

impl AppUseCase {
    /// Enroll once. The operation ID and unique recovery key are durable across restarts.
    pub async fn enroll_title_folder_recovery(&self) -> AppResult<()> {
        let store = &self.services.library.location_operations;
        let catalog = store.recovery_catalog().await?;
        let mut plans = Vec::new();
        for owner in &catalog.owners {
            if !eligible(owner.folder_path.as_deref(), &catalog.roots) {
                continue;
            }
            let id = format!("{RECOVERY_VERSION}:{}", owner.id);
            if store.get_location_operation(&id).await?.is_some() {
                continue;
            }
            if let Some(title) = self.services.catalog.titles.get_by_id(&owner.id).await? {
                plans.push(self.plan_title_folder_recovery(&title).await?);
            }
        }
        // No iteration-order winner when two damaged titles propose overlapping folders.
        let collisions: BTreeSet<_> = plans
            .iter()
            .filter_map(|p| {
                p.destination
                    .as_deref()
                    .filter(|d| {
                        plans.iter().any(|q| {
                            q.title.id != p.title.id
                                && q.destination
                                    .as_deref()
                                    .is_some_and(|other| overlaps(d, other))
                        })
                    })
                    .map(|_| p.title.id.clone())
            })
            .collect();
        for mut plan in plans {
            if collisions.contains(&plan.title.id) {
                plan.unresolved =
                    Some("Multiple recovery candidates propose overlapping folders".into());
                plan.files.clear();
            }
            let work = plan.work();
            let now = Utc::now();
            let json = serde_json::to_string(&plan).map_err(|e| relay_failure(e.to_string()))?;
            let operation = LocationOperation {
                id: plan.operation_id(),
                operation_type: LocationOperationType::TitleFolderRecovery,
                mode: LocationExecutionMode::CopyPreserveSources,
                state: LocationOperationState::Queued,
                initiated_by_user_id: None,
                source_library_id: Some(plan.title.library_id.clone()),
                destination_library_id: Some(plan.title.library_id.clone()),
                source_root_id: Some(plan.title.root_folder_id.clone()),
                destination_root_id: plan.destination_root.as_ref().map(|r| r.id.clone()),
                plan_fingerprint: blake3::hash(json.as_bytes()).to_hex().to_string(),
                verification_depth: VerificationDepth::Full,
                verification_fallback_count: 0,
                counters: LocationOperationCounters {
                    titles_total: 1,
                    files_total: work.files_total(),
                    bytes_total: work.bytes_total(),
                    ..Default::default()
                },
                detail: plan.unresolved.clone(),
                reason_code: None,
                job_run_id: None,
                workflow_operation_id: None,
                cancel_requested: false,
                cancel_requested_at: None,
                confirmed_at: None,
                started_at: None,
                created_at: now,
                updated_at: now,
                completed_at: None,
            };
            // Reserve liveness before the queued row becomes visible to the
            // FIFO. Slow enrollment must never look like a lost runner.
            let Some(guard) = self.runtime.library.location_runners.begin(&operation.id) else {
                continue;
            };
            if let Err(error) = store
                .create_location_operation(&operation, Some(&json))
                .await
            {
                let Some(existing) = store.get_location_operation(&operation.id).await? else {
                    return Err(error);
                };
                if existing.state.is_terminal() {
                    continue;
                }
                let existing_plan = self.read_title_folder_recovery_plan(&existing).await?;
                self.spawn_registered_title_folder_recovery(existing.id, existing_plan, guard);
            } else {
                self.spawn_registered_title_folder_recovery(operation.id, plan, guard);
            }
        }
        for operation in store.list_active_location_operations().await? {
            if operation.operation_type == LocationOperationType::TitleFolderRecovery
                && !self.runtime.library.location_runners.is_live(&operation.id)
            {
                let plan = self.resume_title_folder_recovery(&operation).await?;
                self.spawn_title_folder_recovery(operation.id, plan);
            }
        }
        Ok(())
    }

    pub async fn resume_title_folder_recovery(
        &self,
        operation: &LocationOperation,
    ) -> AppResult<RecoveryPlan> {
        if self.runtime.library.location_runners.is_live(&operation.id) {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_ALREADY_RUNNING,
                "Recovery is already running",
            ));
        }
        let mut plan = self.read_title_folder_recovery_plan(operation).await?;
        if matches!(
            operation.state,
            LocationOperationState::Failed | LocationOperationState::Canceled
        ) {
            if !self
                .services
                .library
                .location_operations
                .recovery_committed(&operation.id)
                .await?
            {
                let title = self
                    .services
                    .catalog
                    .titles
                    .get_by_id(&plan.title.id)
                    .await?
                    .ok_or_else(|| {
                        fail(
                            crate::error_reasons::location::RECOVERY_TITLE_NOT_FOUND,
                            "Title no longer exists",
                        )
                    })?;
                plan = self.plan_title_folder_recovery(&title).await?;
                let json =
                    serde_json::to_string(&plan).map_err(|e| relay_failure(e.to_string()))?;
                self.services
                    .library
                    .location_operations
                    .replace_recovery_plan(&operation.id, &json)
                    .await?;
            }
            self.services
                .library
                .location_operations
                .reopen_location_operation(&operation.id)
                .await?;
        }
        Ok(plan)
    }

    async fn read_title_folder_recovery_plan(
        &self,
        operation: &LocationOperation,
    ) -> AppResult<RecoveryPlan> {
        let json = self
            .services
            .library
            .location_operations
            .get_location_operation_plan_json(&operation.id)
            .await?
            .ok_or_else(|| {
                fail(
                    crate::error_reasons::location::RECOVERY_PLAN_NOT_FOUND,
                    "Recovery plan is missing",
                )
            })?;
        let plan: RecoveryPlan =
            serde_json::from_str(&json).map_err(|e| relay_failure(e.to_string()))?;
        if plan.version != RECOVERY_VERSION
            || plan.operation_id() != operation.id
            || operation.mode != LocationExecutionMode::CopyPreserveSources
        {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_PLAN_UNSUPPORTED,
                "Unsupported recovery plan",
            ));
        }
        Ok(plan)
    }

    pub fn spawn_title_folder_recovery(&self, operation_id: String, plan: RecoveryPlan) {
        let Some(guard) = self.runtime.library.location_runners.begin(&operation_id) else {
            return;
        };
        self.spawn_registered_title_folder_recovery(operation_id, plan, guard);
    }

    fn spawn_registered_title_folder_recovery(
        &self,
        operation_id: String,
        plan: RecoveryPlan,
        guard: LiveRunnerGuard,
    ) {
        let app = self.clone();
        self.restore_coordinator().clone().spawn_worker(async move {
            let _guard = guard;
            if let Err(error) = app.run_title_folder_recovery(&operation_id, &plan).await {
                tracing::warn!(operation_id, %error, "Title folder recovery stopped; original files preserved");
            }
        });
    }

    pub(crate) async fn run_title_folder_recovery(
        &self,
        operation_id: &str,
        plan: &RecoveryPlan,
    ) -> AppResult<OperationRunOutcome> {
        let _slot = self
            .runtime
            .library
            .location_runners
            .wait_for_turn(
                self.services.library.location_operations.as_ref(),
                operation_id,
            )
            .await?;
        let job = self
            .open_location_operation_job_run(
                operation_id,
                1,
                super::super::operations::LocationJobRunActor::Resumed(None),
            )
            .await?;
        self.services
            .library
            .location_operations
            .set_location_operation_job_run(operation_id, &job.id)
            .await?;
        let observer = super::super::operations::LocationJobRunProgress {
            app: self.clone(),
            job_run_id: job.id.clone(),
        };
        let permissions = self
            .resolve_import_file_permissions(Some(&plan.title.library_id), &plan.title.facet)
            .await?;
        let seams = RecoveryExecution {
            app: self,
            plan,
            permissions: ImportFilePermissionsApplier::new(
                self.services.workflow.file_importer.clone(),
                permissions,
            ),
        };
        let work = plan.work();
        self.services
            .library
            .location_operations
            .seed_transfer_titles(
                operation_id,
                &[super::super::live::TransferTitle {
                    title_id: plan.title.id.clone(),
                    name: plan.title.name.clone(),
                    sequence: 0,
                    state: TitleCheckpointState::Pending,
                    files_total: work.files_total(),
                    files_done: 0,
                    bytes_total: work.bytes_total().max(0) as u64,
                    copy_bytes: 0,
                    verification_bytes: 0,
                    current_file: None,
                    copying: 0,
                    verifying: 0,
                    detail: plan.unresolved.clone(),
                }],
            )
            .await?;
        // The catalog transaction requires every durable receipt. On restart
        // the runner counts those receipts without repeating filesystem work.
        let result = LocationOperationRunner::new(
            self.services.library.location_operations.as_ref(),
            &seams,
            &seams,
            &seams,
        )
        .with_ownership_registry(&self.runtime.library.location_ownership)
        .with_progress_observer(&observer)
        .with_transfers(&self.runtime.library.location_runners.transfers)
        .with_file_move_retry(FileMoveRetry {
            attempts: 1,
            base_delay: std::time::Duration::ZERO,
        })
        .run(operation_id, &work)
        .await;
        self.close_location_operation_job_run_for(&job.id, &result)
            .await;
        result
    }
}

struct RecoveryExecution<'a> {
    app: &'a AppUseCase,
    plan: &'a RecoveryPlan,
    permissions: ImportFilePermissionsApplier,
}

impl RecoveryExecution<'_> {
    async fn revalidate_folder_identity(&self) -> AppResult<()> {
        let root = self.plan.destination_root.as_ref().ok_or_else(|| {
            fail(
                crate::error_reasons::location::RECOVERY_ROOT_NOT_FOUND,
                "Recovery root missing",
            )
        })?;
        inspect_path(&stored_path_to_path_buf(&root.path), false).await?;
        let destination =
            stored_path_to_path_buf(self.plan.destination.as_deref().ok_or_else(|| {
                fail(
                    crate::error_reasons::location::RECOVERY_DESTINATION_NOT_FOUND,
                    "Recovery destination missing",
                )
            })?);
        inspect_path(&destination, !self.plan.files.is_empty()).await?;
        self.app
            .validate_generated_recovery_destination(self.plan)
            .await?;
        let matched = folder_identity_matches(&self.plan.title, &destination, true).await?;
        if self.plan.files.is_empty() && !matched {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_FOLDER_IDENTITY_CHANGED,
                "Exact folder identity changed; ownership remains unresolved",
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl TitleAdmissionCheck for RecoveryExecution<'_> {
    async fn admit_title(&self, context: TitleAdmissionContext<'_>) -> AppResult<TitleAdmission> {
        if self
            .app
            .services
            .library
            .location_operations
            .recovery_committed(&context.operation.id)
            .await?
        {
            return Ok(TitleAdmission::Proceed);
        }
        if let Some(reason) = &self.plan.unresolved {
            return Ok(TitleAdmission::Stale(reason.clone()));
        }
        let catalog = self
            .app
            .services
            .library
            .location_operations
            .recovery_catalog()
            .await?;
        if let Err(error) = validate_catalog(self.plan, &catalog) {
            return Ok(TitleAdmission::Stale(error.to_string()));
        }
        self.revalidate_folder_identity().await?;
        if let Some(reason) = self
            .app
            .active_work_blocking_a_move(&self.plan.title)
            .await?
        {
            return Ok(TitleAdmission::Stale(reason));
        }
        for file in &self.plan.files {
            if let Some(expected) = &file.source_version {
                let path = stored_path_to_path_buf(&file.media.file_path);
                inspect_path(&path, false).await?;
                if recovery_file_version(&path).await? != *expected {
                    return Ok(TitleAdmission::Stale(
                        "Source changed; originals preserved".into(),
                    ));
                }
            }
        }
        Ok(TitleAdmission::Proceed)
    }
}

#[async_trait]
impl TitleFileMover for RecoveryExecution<'_> {
    async fn move_file(&self, request: FileMoveRequest<'_>) -> AppResult<VerifiedFile> {
        if self
            .app
            .services
            .library
            .location_operations
            .recovery_committed(request.operation_id)
            .await?
        {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_COMMIT_RECEIPT_REQUIRED,
                "Recovery already committed; a missing receipt cannot authorize another copy",
            ));
        }
        let file = self
            .plan
            .files
            .iter()
            .find(|f| Some(&f.media.id) == request.file.media_file_id.as_ref())
            .ok_or_else(|| {
                fail(
                    crate::error_reasons::location::RECOVERY_FILE_NOT_IN_PLAN,
                    "Recovery file is not in the persisted plan",
                )
            })?;
        let source = &request.file.source_path;
        let destination = &request.file.destination_path;
        inspect_path(destination, true).await?;
        if file.source_version.is_some() {
            inspect_path(source, false).await?;
            if Some(recovery_file_version(source).await?) != file.source_version {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_SOURCE_CHANGED,
                    "Source changed before copying",
                ));
            }
        }
        if tokio::fs::symlink_metadata(destination)
            .await
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            if file.source_version.is_none() {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_SOURCE_AND_DESTINATION_NOT_FOUND,
                    "Both source and destination are missing",
                ));
            }
            let parent = destination.parent().ok_or_else(|| {
                fail(
                    crate::error_reasons::location::RECOVERY_DESTINATION_PARENT_REQUIRED,
                    "Destination has no parent",
                )
            })?;
            create_recovery_destination_directories(parent, &self.permissions).await?;
            // Each attempt gets its own durably recorded staging name. An
            // interrupted attempt is never mistaken for permission to delete it.
            let staging = parent.join(format!(".scryer-recovery-{}", scryer_domain::Id::new().0));
            self.app
                .services
                .library
                .location_operations
                .reserve_recovery_staging(
                    request.operation_id,
                    file,
                    &path_to_stored_string(&staging),
                )
                .await?;
            VerifiedCopier::new()
                .copy_to_journaled_staging(source, &staging, request.progress)
                .await?;
            let staged_file = RecoveryFile {
                destination: path_to_stored_string(&staging),
                ..file.clone()
            };
            prove_file(&staged_file, request.progress.clone()).await?;
            inspect_path(parent, false).await?;
            crate::fs_safety::promote_recovery_staged_file(&staging, destination)
                .await
                .map_err(|e| {
                    fail(crate::error_reasons::location::RECOVERY_DESTINATION_CLAIM_FAILED, format!(
                        "Destination could not be claimed; original and staged files preserved: {e}"
                    ))
                })?;
            crate::location::verify::sync_parent_directory_after_promotion(destination).await;
        }
        let hashes =
            prove_and_apply_recovery_permissions(file, request.progress.clone(), &self.permissions)
                .await?;
        Ok(VerifiedFile {
            source_path: source.clone(),
            destination_path: destination.clone(),
            hashes: Some(hashes),
            depth: AppliedVerificationDepth::exact(VerificationDepth::Full),
            outcome: FileVerificationOutcome::Verified,
            detail: Some("Full content verified; original source preserved".into()),
        })
    }
}

async fn create_recovery_destination_directories(
    parent: &Path,
    permissions: &dyn PlacedContentPermissions,
) -> AppResult<()> {
    inspect_path(parent, true).await?;
    let mut missing = Vec::new();
    let mut current = parent;
    loop {
        match tokio::fs::symlink_metadata(current).await {
            Ok(metadata) if metadata.is_dir() => break,
            Ok(_) => {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_DESTINATION_PARENT_NOT_DIRECTORY,
                    "Recovery destination parent is not a directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(current.to_path_buf());
                current = current.parent().ok_or_else(|| {
                    fail(
                        crate::error_reasons::location::RECOVERY_DESTINATION_ANCESTOR_NOT_FOUND,
                        "Recovery destination has no existing ancestor",
                    )
                })?;
            }
            Err(error) => return Err(relay_failure(error.to_string())),
        }
    }
    for directory in missing.into_iter().rev() {
        match tokio::fs::create_dir(&directory).await {
            Ok(()) => {
                inspect_path(&directory, false).await?;
                permissions.apply_to_directory(&directory).await?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                inspect_path(&directory, false).await?;
                if !tokio::fs::symlink_metadata(&directory)
                    .await
                    .map_err(|e| relay_failure(e.to_string()))?
                    .is_dir()
                {
                    return Err(fail(
                        crate::error_reasons::location::RECOVERY_DESTINATION_PARENT_NOT_DIRECTORY,
                        "Recovery destination parent is not a directory",
                    ));
                }
            }
            Err(error) => return Err(relay_failure(error.to_string())),
        }
    }
    inspect_path(parent, false).await
}

async fn prove_and_apply_recovery_permissions(
    file: &RecoveryFile,
    progress: CopyProgress,
    permissions: &dyn PlacedContentPermissions,
) -> AppResult<StreamedContentHashes> {
    let hashes = prove_file(file, progress).await?;
    // In-place files are originals. Only a verified independent copy may have
    // destination permissions applied; source modes remain untouched.
    if !folder_paths_match(&file.media.file_path, &file.destination) {
        permissions
            .apply_to_file(&stored_path_to_path_buf(&file.destination))
            .await?;
    }
    Ok(hashes)
}

pub(super) async fn prove_file(
    file: &RecoveryFile,
    progress: CopyProgress,
) -> AppResult<StreamedContentHashes> {
    let source = stored_path_to_path_buf(&file.media.file_path);
    let destination = stored_path_to_path_buf(&file.destination);
    inspect_path(&destination, false).await?;
    let dest_before = recovery_file_version(&destination).await?;
    let destination_hash = hash_existing_file_with_progress(&destination, progress.clone()).await?;
    let expected = if let Some(version) = &file.source_version {
        inspect_path(&source, false).await?;
        if !folder_paths_match(&file.media.file_path, &file.destination) && *version == dest_before
        {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_INDEPENDENT_COPY_REQUIRED,
                "Recovery requires an independent destination copy, not a hardlink",
            ));
        }
        if recovery_file_version(&source).await? != *version {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_SOURCE_CHANGED,
                "Source changed during recovery",
            ));
        }
        let hashes = hash_existing_file_with_progress(&source, progress).await?;
        if recovery_file_version(&source).await? != *version {
            return Err(fail(
                crate::error_reasons::location::RECOVERY_SOURCE_CHANGED,
                "Source changed while verifying",
            ));
        }
        hashes.full_blake3
    } else {
        file.expected_hash.clone().ok_or_else(|| {
            fail(
                crate::error_reasons::location::RECOVERY_SOURCE_CONTENT_PROOF_REQUIRED,
                "Missing source has no full-content proof",
            )
        })?
    };
    if destination_hash.full_blake3 != expected
        || destination_hash.size_bytes != file.media.size_bytes.max(0) as u64
        || recovery_file_version(&destination).await? != dest_before
    {
        return Err(fail(
            crate::error_reasons::location::RECOVERY_DESTINATION_CONTENT_MISMATCH,
            "Destination content does not match; both files preserved",
        ));
    }
    Ok(destination_hash)
}

#[async_trait]
impl TitleReconciler for RecoveryExecution<'_> {
    async fn reconcile_title(
        &self,
        operation: &LocationOperation,
        _title: &PlannedTitle,
    ) -> AppResult<TitleStepOutcome> {
        if !self
            .app
            .services
            .library
            .location_operations
            .recovery_committed(&operation.id)
            .await?
        {
            let current = self
                .app
                .services
                .catalog
                .titles
                .get_by_id(&self.plan.title.id)
                .await?
                .ok_or_else(|| {
                    fail(
                        crate::error_reasons::location::RECOVERY_TITLE_NOT_FOUND,
                        "Title no longer exists",
                    )
                })?;
            if current != self.plan.title {
                return Err(fail(
                    crate::error_reasons::location::RECOVERY_TITLE_CHANGED,
                    "Title changed during recovery; original mappings preserved",
                ));
            }
            self.revalidate_folder_identity().await?;
            // Receipts do not prove that a destination remained unchanged after a restart.
            for file in &self.plan.files {
                prove_file(file, CopyProgress::none()).await?;
            }
            let mut title = self.plan.title.clone();
            title.folder_path = self.plan.destination.clone();
            title.root_folder_id = self
                .plan
                .destination_root
                .as_ref()
                .ok_or_else(|| {
                    fail(
                        crate::error_reasons::location::RECOVERY_ROOT_NOT_FOUND,
                        "Recovery root missing",
                    )
                })?
                .id
                .clone();
            let mut event = new_title_domain_event(
                DomainEventActor::system(),
                &title,
                DomainEventPayload::TitleUpdated(TitleUpdatedEventData {
                    title: title_context_snapshot(&title),
                }),
            );
            event.event_id = format!("{}:committed", operation.id);
            event.correlation_id = Some(operation.id.clone());
            if let Some(stored) = self
                .app
                .services
                .library
                .location_operations
                .commit_title_folder_recovery(&operation.id, self.plan, &event)
                .await?
            {
                self.app.publish_stored_domain_event(&stored).await;
            }
        }
        let preserved: Vec<_> = self
            .plan
            .files
            .iter()
            .filter(|f| {
                f.source_version.is_some()
                    && !folder_paths_match(&f.media.file_path, &f.destination)
            })
            .map(|f| f.media.file_path.as_str())
            .collect();
        Ok(TitleStepOutcome::noted(format!(
            "Recovered title ownership. Original files retained: {}",
            preserved.join("; ")
        )))
    }
    async fn clean_up_title(
        &self,
        _operation: &LocationOperation,
        _title: &PlannedTitle,
    ) -> AppResult<TitleStepOutcome> {
        // No source cleanup. A fileless catalog adopted by exact ID still
        // needs a scoped scan; keep this step resumable if discovery fails.
        if self.plan.expected_media.is_empty() {
            self.app
                .reconcile_title_folder(
                    &scryer_domain::User::system_execution_actor(),
                    &self.plan.title.id,
                )
                .await?;
        }
        Ok(TitleStepOutcome::clean())
    }
}
