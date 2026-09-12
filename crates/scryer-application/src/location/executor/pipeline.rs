use super::*;
use crate::location::live::{TransferHub, TransferTitle};
use crate::location::model::LocationExecutionMode;
use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use std::sync::atomic::AtomicBool;

/// What one file's transfer future hands back to the driver.
enum PipelineFileOutcome {
    Verified(VerifiedFile),
    /// The user canceled while the file waited for storage. Nothing moved, so
    /// the file is neither done nor failed; the driver stops at the boundary.
    Canceled,
}

pub(super) struct PipelineOutcome {
    pub failures: Vec<TitleFailure>,
    pub stop: Option<(StopReason, String)>,
}

type FileResult<'a> = (
    &'a PlannedTitle,
    &'a PlannedFile,
    AppResult<PipelineFileOutcome>,
);
type FileTask<'a> = BoxFuture<'a, FileResult<'a>>;

/// Keep admitted file futures polled even while the scheduler awaits a catalog
/// write. A file may hold the same writer gate needed by progress persistence.
async fn drive_files<'a>(
    mut admissions: tokio::sync::mpsc::UnboundedReceiver<FileTask<'a>>,
    completed: tokio::sync::mpsc::UnboundedSender<FileResult<'a>>,
) {
    let mut files = FuturesUnordered::new();
    let mut closed = false;
    loop {
        if closed && files.is_empty() {
            break;
        }
        tokio::select! {
            next = admissions.recv(), if !closed => match next {
                Some(file) => files.push(file),
                None => closed = true,
            },
            Some(result) = files.next(), if !files.is_empty() => {
                // The scheduler can exit on a store error. Still drain every
                // admitted file before the operation guard may be released.
                let _ = completed.send(result);
            },
        }
    }
}

impl LocationOperationRunner<'_> {
    /// Bounded copy admission feeds verification futures. A file's transition to
    /// read-back admits the next copy; verification has no semaphore. All
    /// futures are drained before a stop or repository error leaves this scope.
    pub(super) async fn run_pipeline(
        &self,
        operation: &LocationOperation,
        plan: &OperationWorkPlan,
        progress: &mut RunProgress,
        hub: &TransferHub,
    ) -> AppResult<PipelineOutcome> {
        let (admissions, admitted) = tokio::sync::mpsc::unbounded_channel();
        let (completed, completions) = tokio::sync::mpsc::unbounded_channel();
        let (result, ()) = tokio::join!(
            self.schedule_pipeline(operation, plan, progress, hub, admissions, completions),
            drive_files(admitted, completed),
        );
        result
    }

    async fn schedule_pipeline<'run>(
        &'run self,
        operation: &'run LocationOperation,
        plan: &'run OperationWorkPlan,
        progress: &mut RunProgress,
        hub: &'run TransferHub,
        admissions: tokio::sync::mpsc::UnboundedSender<FileTask<'run>>,
        mut completions: tokio::sync::mpsc::UnboundedReceiver<FileResult<'run>>,
    ) -> AppResult<PipelineOutcome> {
        let mut pending = 0usize;
        let notify = Arc::new(tokio::sync::Notify::new());
        let mut copying = Vec::<Arc<AtomicBool>>::new();
        let mut title_index = 0;
        let mut file_index = 0;
        let mut admitted = BTreeSet::new();
        let mut verified_paths: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut remaining: BTreeMap<String, usize> = BTreeMap::new();
        let mut failed = BTreeMap::<String, (LocationReasonCode, String)>::new();
        let mut rows = BTreeMap::<String, TransferTitle>::new();
        let mut stop = None;
        let mut fatal = None;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut checkpoint_at = std::time::Instant::now();
        hub.begin(
            &operation.id,
            progress.counters(plan).bytes_processed.max(0) as u64,
        );
        self.write_progress(
            operation,
            LocationOperationState::Moving,
            progress,
            plan,
            None,
            false,
            None,
        )
        .await?;

        loop {
            copying.retain(|ready| !ready.load(Ordering::Acquire));
            if stop.is_none() && fatal.is_none() {
                // No new file starts until cancellation has been checked. A
                // title already verifying is allowed to finish its proof.
                match self
                    .store
                    .location_operation_cancel_requested(&operation.id)
                    .await
                {
                    Ok(true) => {
                        stop = Some((
                            StopReason::UserCanceled,
                            "canceled after draining in-flight transfers".into(),
                        ))
                    }
                    Err(error) => fatal = Some(error),
                    _ => {}
                }
                while stop.is_none()
                    && fatal.is_none()
                    && !hub.has_storage_waiters(&operation.id)
                    && title_index < plan.titles.len()
                    && copying.len()
                        < crate::location::transfer::CopyCoordinator::shared().capacity()
                {
                    let title = &plan.titles[title_index];
                    if progress.is_settled(&title.title_id) || failed.contains_key(&title.title_id)
                    {
                        title_index += 1;
                        file_index = 0;
                        continue;
                    }
                    // Catalog-only and recovered titles can all finish inside
                    // this admission loop without polling any file futures.
                    match self
                        .store
                        .location_operation_cancel_requested(&operation.id)
                        .await
                    {
                        Ok(true) => {
                            stop = Some((
                                StopReason::UserCanceled,
                                "canceled after draining in-flight transfers".into(),
                            ));
                            break;
                        }
                        Err(error) => {
                            fatal = Some(error);
                            break;
                        }
                        Ok(false) => {}
                    }
                    if !admitted.contains(&title.title_id) {
                        let admission = async {
                            let mut paths = self
                                .store
                                .verified_destination_paths(&operation.id, &title.title_id)
                                .await?;
                            if operation.mode == LocationExecutionMode::MoveWithScryer {
                                let resolutions = self
                                    .store
                                    .file_resolutions(&operation.id, &title.title_id)
                                    .await?;
                                // A prior copy record alone is not an existing-file identity proof.
                                let mut current = BTreeSet::new();
                                for resolution in resolutions {
                                    if paths.contains(&resolution.destination_path)
                                        && resolution.proof_is_current().await
                                    {
                                        current.insert(resolution.original_destination);
                                    }
                                }
                                paths = current;
                            }
                            let admission = self
                                .admission
                                .admit_title(TitleAdmissionContext {
                                    operation,
                                    title,
                                    verified_destinations: &paths,
                                })
                                .await?;
                            verified_paths.insert(title.title_id.clone(), paths);
                            Ok::<_, AppError>(admission)
                        }
                        .await;
                        match admission {
                            Ok(TitleAdmission::Proceed) => {}
                            Ok(TitleAdmission::Stale(reason)) => {
                                stop = Some((StopReason::StalePlan, reason));
                                break;
                            }
                            Ok(TitleAdmission::Blocked(reason)) => {
                                if let Err(error) = self
                                    .settle_title(
                                        operation,
                                        title,
                                        TitleCheckpointState::Blocked,
                                        Some(reason.clone()),
                                        progress,
                                        plan,
                                        None,
                                    )
                                    .await
                                {
                                    fatal = Some(error);
                                }
                                progress.warnings.push(reason);
                                hub.file_failed(
                                    &operation.id,
                                    &title.title_id,
                                    "",
                                    title.files.iter().map(|file| file.size_bytes).sum(),
                                );
                                title_index += 1;
                                file_index = 0;
                                continue;
                            }
                            Ok(TitleAdmission::Skip(reason)) => {
                                if let Err(error) = self
                                    .settle_title(
                                        operation,
                                        title,
                                        TitleCheckpointState::Skipped,
                                        Some(reason),
                                        progress,
                                        plan,
                                        None,
                                    )
                                    .await
                                {
                                    fatal = Some(error);
                                }
                                title_index += 1;
                                file_index = 0;
                                continue;
                            }
                            Err(error) => {
                                fatal = Some(error);
                                break;
                            }
                        }
                        admitted.insert(title.title_id.clone());
                        let missing = title
                            .files
                            .iter()
                            .filter(|file| {
                                !verified_paths[&title.title_id]
                                    .contains(&file.stored_destination())
                            })
                            .count();
                        remaining.insert(title.title_id.clone(), missing);
                        match self
                            .store
                            .transfer_title(&operation.id, &title.title_id)
                            .await
                        {
                            Ok(Some(row)) => {
                                rows.insert(title.title_id.clone(), row);
                            }
                            Ok(None) => {}
                            Err(error) => {
                                fatal = Some(error);
                                break;
                            }
                        }
                        if missing == 0 {
                            for file in &title.files {
                                hub.file_done(
                                    &operation.id,
                                    &title.title_id,
                                    &file.stored_destination(),
                                    file.size_bytes,
                                );
                            }
                            if let Err(error) = self
                                .finish_pipeline_title(
                                    operation,
                                    title,
                                    progress,
                                    plan,
                                    &mut failed,
                                )
                                .await
                            {
                                fatal = Some(error);
                            }
                            title_index += 1;
                            file_index = 0;
                            continue;
                        }
                    }
                    if file_index >= title.files.len() {
                        title_index += 1;
                        file_index = 0;
                        continue;
                    }
                    let file = &title.files[file_index];
                    file_index += 1;
                    if verified_paths[&title.title_id].contains(&file.stored_destination()) {
                        hub.file_done(
                            &operation.id,
                            &title.title_id,
                            &file.stored_destination(),
                            file.size_bytes,
                        );
                        progress.note_file_done(&title.title_id, file);
                        continue;
                    }
                    let ready = Arc::new(AtomicBool::new(false));
                    copying.push(ready.clone());
                    let signal = ready.clone();
                    let wake = notify.clone();
                    admissions
                        .send(
                            async move {
                                let result = std::panic::AssertUnwindSafe(self.pipeline_file(
                                    operation,
                                    title,
                                    file,
                                    hub,
                                    signal.clone(),
                                    wake.clone(),
                                ))
                                .catch_unwind()
                                .await
                                .unwrap_or_else(|_| {
                                    Err(AppError::Repository("file transfer task panicked".into()))
                                });
                                signal.store(true, Ordering::Release);
                                wake.notify_one();
                                (title, file, result)
                            }
                            .boxed(),
                        )
                        .map_err(|_| AppError::Repository("file transfer driver stopped".into()))?;
                    pending += 1;
                }
            }
            if pending == 0 {
                if title_index >= plan.titles.len() || stop.is_some() || fatal.is_some() {
                    break;
                }
                continue;
            }
            tokio::select! {
                Some((title, file, result)) = completions.recv() => {
                    pending -= 1;
                    let result = match result {
                        Ok(PipelineFileOutcome::Canceled) => {
                            // The file handed itself back untouched. Stop at
                            // this boundary like any other observed cancel.
                            if stop.is_none() {
                                stop = Some((
                                    StopReason::UserCanceled,
                                    "canceled after draining in-flight transfers".into(),
                                ));
                            }
                            None
                        }
                        Ok(PipelineFileOutcome::Verified(verified)) => Some({
                            if verified.depth.fell_back {
                                progress.verification_fallbacks += 1;
                                if let Some(detail) = verified.detail.clone() {
                                    progress.warnings.push(detail.clone());
                                    if let Some(row) = rows.get_mut(&title.title_id) {
                                        row.detail = Some(detail);
                                        let mut persisted = row.clone();
                                        persisted.copy_bytes = 0; persisted.verification_bytes = 0;
                                        if let Err(error) = self.store.upsert_transfer_title(&operation.id, &persisted).await { fatal = Some(error); }
                                    }
                                }
                            }
                            let permitted = verified.permits_source_removal();
                            let detail = verified.detail.clone();
                            let record = verified.into_record(FileVerificationIdentity {
                                operation_id: &operation.id, title_id: &title.title_id,
                                media_file_id: file.media_file_id.as_deref(),
                            }, Utc::now());
                            match self.store.record_location_file_verification(&record).await {
                                Ok(()) if permitted => {
                                    hub.file_done(&operation.id, &title.title_id, &file.stored_destination(), file.size_bytes);
                                    Ok(())
                                },
                                Ok(()) => Err((
                                    LocationReasonCode::VerificationMismatch,
                                    AppError::Validation(detail.unwrap_or_else(|| "destination verification failed".into())),
                                )),
                                Err(error) => { fatal = Some(error); Ok(()) },
                            }
                        }),
                        // A transfer that could not complete is a storage
                        // problem; the source is intact.
                        Err(error) => Some(Err((LocationReasonCode::StorageError, error))),
                    };
                    match result {
                        Some(Err((reason_code, error))) => {
                            hub.file_failed(&operation.id, &title.title_id, &file.stored_destination(), file.size_bytes);
                            failed.insert(title.title_id.clone(), (reason_code, error.to_string()));
                        }
                        Some(Ok(())) if fatal.is_none() => progress.note_file_done(&title.title_id, file),
                        _ => {}
                    }
                    let count = remaining.get_mut(&title.title_id).expect("admitted title");
                    *count = count.saturating_sub(1);
                    if *count == 0
                        && stop.is_none()
                        && fatal.is_none()
                        && !failed.contains_key(&title.title_id)
                        && let Err(error) = self
                            .finish_pipeline_title(operation, title, progress, plan, &mut failed)
                            .await
                    {
                        fatal = Some(error);
                    }
                },
                () = notify.notified() => {},
                _ = tick.tick() => {},
            }
            // Persist only state transitions and completed facts. Byte counters
            // below stay in memory; sorted pages read the durable phase rank.
            rows.retain(|id, _| !progress.is_settled(id) && !progress.failed.contains(id));
            for (id, row) in &mut rows {
                if progress.is_settled(id) || progress.failed.contains(id) {
                    continue;
                }
                let previous = (
                    row.state,
                    row.copying,
                    row.verifying,
                    row.current_file.clone(),
                    row.files_done,
                );
                hub.overlay(&operation.id, row);
                row.files_done = *progress.files_done.get(id).unwrap_or(&0);
                if row.verifying > 0 {
                    row.state = TitleCheckpointState::Verifying;
                } else if row.copying > 0 {
                    row.state = TitleCheckpointState::Moving;
                } else if let Some((_, detail)) = failed.get(id) {
                    row.state = TitleCheckpointState::Failed;
                    row.detail = Some(detail.clone());
                } else if stop.is_some()
                    && matches!(
                        row.state,
                        TitleCheckpointState::Moving | TitleCheckpointState::Verifying
                    )
                {
                    // Nothing of this title is in flight any more and the run
                    // is stopping short. Its verified files are durable; the
                    // title itself is back to waiting for a resume, not moving.
                    row.state = TitleCheckpointState::Pending;
                }
                if previous
                    != (
                        row.state,
                        row.copying,
                        row.verifying,
                        row.current_file.clone(),
                        row.files_done,
                    )
                {
                    let mut persisted = row.clone();
                    persisted.copy_bytes = 0;
                    persisted.verification_bytes = 0;
                    if let Err(error) = self
                        .store
                        .upsert_transfer_title(&operation.id, &persisted)
                        .await
                    {
                        fatal = Some(error);
                    }
                    hub.changed();
                }
            }
            if checkpoint_at.elapsed() >= Duration::from_secs(1) {
                let detail = hub
                    .has_storage_waiters(&operation.id)
                    .then(|| storage_wait_detail(hub.storage_wait_interval(&operation.id)));
                if let Err(error) = self
                    .write_progress(
                        operation,
                        LocationOperationState::Moving,
                        progress,
                        plan,
                        detail,
                        false,
                        None,
                    )
                    .await
                {
                    fatal = Some(error);
                }
                checkpoint_at = std::time::Instant::now();
            }
        }
        // There are no live filesystem futures beyond this point.
        if let Some(error) = fatal {
            return Err(error);
        }
        let mut failures = Vec::new();
        for title in &plan.titles {
            if let Some((reason_code, detail)) = failed.remove(&title.title_id) {
                self.settle_title(
                    operation,
                    title,
                    TitleCheckpointState::Failed,
                    Some(detail.clone()),
                    progress,
                    plan,
                    Some(reason_code),
                )
                .await?;
                failures.push(TitleFailure {
                    title_id: title.title_id.clone(),
                    reason_code,
                    detail,
                });
            }
        }
        Ok(PipelineOutcome { failures, stop })
    }

    async fn pipeline_file(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
        file: &PlannedFile,
        hub: &TransferHub,
        ready: Arc<AtomicBool>,
        notify: Arc<tokio::sync::Notify>,
    ) -> AppResult<PipelineFileOutcome> {
        let storage =
            super::recovery::StorageWatch::capture(&file.source_path, &file.destination_path).await;
        let mut attempt = 1;
        loop {
            let operation_id = operation.id.clone();
            let title_id = title.title_id.clone();
            let path = file.stored_destination();
            let size = file.size_bytes;
            let copied = Arc::new(AtomicU64::new(0));
            let sink_hub = hub.clone();
            let copy_operation = operation_id.clone();
            let copy_title = title_id.clone();
            let copy_path = path.clone();
            let phase_hub = hub.clone();
            let phase_ready = ready.clone();
            let phase_notify = notify.clone();
            let previous_phase = AtomicU64::new(0);
            let compare_hub = hub.clone();
            let compare_operation = operation_id.clone();
            let compare_title = title_id.clone();
            let compare_path = path.clone();
            let compare_ready = ready.clone();
            let compare_notify = notify.clone();
            let sink = CopyProgress::from_fn(move |bytes| {
                let bytes = copied
                    .fetch_add(bytes, Ordering::Relaxed)
                    .saturating_add(bytes);
                sink_hub.file_update(
                    &copy_operation,
                    &copy_title,
                    &copy_path,
                    size,
                    scryer_domain::ImportTransferPhase::Copying,
                    bytes,
                );
            })
            .with_transfer_sink(move |phase, bytes| {
                phase_hub.file_update(&operation_id, &title_id, &path, size, phase, bytes);
                if phase == scryer_domain::ImportTransferPhase::Verifying {
                    phase_ready.store(true, Ordering::Release);
                }
                if previous_phase.swap(phase as u64 + 1, Ordering::Relaxed) != phase as u64 + 1 {
                    phase_notify.notify_one();
                }
            });
            let sink = sink.with_comparison_sink(move |bytes, total| {
                compare_hub.file_comparing(
                    &compare_operation,
                    &compare_title,
                    &compare_path,
                    size,
                    bytes,
                    total,
                );
                compare_ready.store(true, Ordering::Release);
                if bytes == 0 || bytes == total {
                    compare_notify.notify_one();
                }
            });
            let result = self
                .mover
                .move_file(FileMoveRequest {
                    operation_id: &operation.id,
                    title,
                    file,
                    depth: operation.verification_depth,
                    progress: &sink,
                })
                .await;
            let result = match result {
                Ok(verified)
                    if verified.outcome
                        == crate::location::model::FileVerificationOutcome::Unavailable
                        && match &storage {
                            Some(storage) => !storage.available().await,
                            None => false,
                        } =>
                {
                    Err(AppError::Repository(verified.detail.unwrap_or_else(|| {
                        "storage became unavailable during verification".into()
                    })))
                }
                result => result,
            };
            match result {
                Err(error) if move_error_is_transient(&error) => {
                    if let Some(storage) = &storage
                        && !storage.available().await
                    {
                        tracing::warn!(operation_id = %operation.id, source = %file.source_path.display(),
                            error = %error, "transfer waiting for storage; probing every few seconds, backing off to once a minute");
                        let waiting_path = file.stored_destination();
                        hub.file_waiting_for_storage(
                            &operation.id,
                            &title.title_id,
                            &waiting_path,
                            file.size_bytes,
                        );
                        let reconnected = super::recovery::wait_for_reconnection(
                            || storage.available(),
                            || {
                                self.store
                                    .location_operation_cancel_requested(&operation.id)
                            },
                            |interval| {
                                hub.file_storage_probe_scheduled(
                                    &operation.id,
                                    &title.title_id,
                                    &waiting_path,
                                    interval,
                                );
                            },
                        )
                        .await?;
                        if !reconnected {
                            // A cancel is not a failure: the file never moved,
                            // and the title must not settle Failed on a
                            // Canceled operation.
                            hub.file_released(&operation.id, &title.title_id, &waiting_path);
                            return Ok(PipelineFileOutcome::Canceled);
                        }
                        hub.file_update(
                            &operation.id,
                            &title.title_id,
                            &file.stored_destination(),
                            file.size_bytes,
                            scryer_domain::ImportTransferPhase::Waiting,
                            0,
                        );
                        attempt = 1;
                    } else if attempt < self.retry.attempts {
                        tokio::time::sleep(self.retry.delay_before(attempt + 1)).await;
                        attempt += 1;
                    } else {
                        return Err(error);
                    }
                }
                result => return result.map(PipelineFileOutcome::Verified),
            }
        }
    }

    async fn finish_pipeline_title(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
        progress: &mut RunProgress,
        plan: &OperationWorkPlan,
        failed: &mut BTreeMap<String, (LocationReasonCode, String)>,
    ) -> AppResult<()> {
        if self
            .store
            .location_operation_cancel_requested(&operation.id)
            .await?
        {
            return Ok(());
        }
        let started = std::time::Instant::now();
        let transfer_detail = self
            .store
            .transfer_title(&operation.id, &title.title_id)
            .await?
            .and_then(|row| row.detail);
        let mut paths = self
            .store
            .verified_destination_paths(&operation.id, &title.title_id)
            .await?;
        let resolutions = self
            .store
            .file_resolutions(&operation.id, &title.title_id)
            .await?;
        progress.record_resolved_outcomes(&title.title_id, &resolutions);
        for resolution in resolutions {
            if resolution.completed && paths.contains(&resolution.destination_path) {
                paths.insert(resolution.original_destination);
            }
        }
        // The completeness gate reconstructs these counts from durable proofs.
        progress.files_done.remove(&title.title_id);
        progress.bytes_done.remove(&title.title_id);
        let mut phase = TitleRunPhase::Moving;
        match self
            .run_title(operation, title, &paths, progress, plan, &mut phase)
            .await
        {
            Ok(TitleRunOutcome::Finished {
                notes,
                mut warnings,
            }) => {
                warnings.extend(transfer_detail);
                let state = if warnings.is_empty() {
                    TitleCheckpointState::Completed
                } else {
                    TitleCheckpointState::CompletedWithWarnings
                };
                let detail = finished_title_detail(&notes, &warnings);
                self.settle_title(operation, title, state, detail, progress, plan, None)
                    .await?;
                if let Some(hub) = self.transfers {
                    hub.title_finalized(&operation.id, started.elapsed());
                }
                progress.warnings.extend(warnings);
            }
            Ok(TitleRunOutcome::Canceled) => {}
            Err(error) => {
                failed.insert(
                    title.title_id.clone(),
                    (classify_title_failure(phase, &error), error.to_string()),
                );
            }
        }
        Ok(())
    }
}

/// The operation-level detail shown while every transfer waits for storage.
fn storage_wait_detail(interval: Option<Duration>) -> String {
    let cadence = match interval.map(|interval| interval.as_secs()) {
        None => "Checking again shortly.".to_owned(),
        Some(60..) => "Checking once a minute.".to_owned(),
        Some(1) => "Checking every second.".to_owned(),
        Some(seconds) => format!("Checking every {seconds} seconds."),
    };
    format!("Waiting for storage to reconnect. {cadence}")
}
