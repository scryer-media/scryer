use super::*;
use crate::location::live::{TransferHub, TransferTitle};
use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream::FuturesUnordered};
use std::sync::atomic::AtomicBool;

pub(super) struct PipelineOutcome {
    pub failures: Vec<String>,
    pub stop: Option<(StopReason, String)>,
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
        let mut pending: FuturesUnordered<
            BoxFuture<'_, (&PlannedTitle, &PlannedFile, AppResult<VerifiedFile>)>,
        > = FuturesUnordered::new();
        let notify = Arc::new(tokio::sync::Notify::new());
        let mut copying = Vec::<Arc<AtomicBool>>::new();
        let mut title_index = 0;
        let mut file_index = 0;
        let mut admitted = BTreeSet::new();
        let mut verified_paths: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut remaining: BTreeMap<String, usize> = BTreeMap::new();
        let mut failed = BTreeMap::<String, String>::new();
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
                            let paths = self
                                .store
                                .verified_destination_paths(&operation.id, &title.title_id)
                                .await?;
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
                                    )
                                    .await
                                {
                                    fatal = Some(error);
                                }
                                progress.warnings.push(reason);
                                hub.file_failed(&operation.id, &title.title_id, "");
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
                    pending.push(
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
                    );
                }
            }
            if pending.is_empty() {
                if title_index >= plan.titles.len() || stop.is_some() || fatal.is_some() {
                    break;
                }
                continue;
            }
            tokio::select! {
                Some((title, file, result)) = pending.next() => {
                    let result = match result {
                        Ok(verified) => {
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
                                Ok(()) => Err(AppError::Validation(detail.unwrap_or_else(|| "destination verification failed".into()))),
                                Err(error) => { fatal = Some(error); Ok(()) },
                            }
                        },
                        Err(error) => Err(error),
                    };
                    if let Err(error) = result {
                        hub.file_failed(&operation.id, &title.title_id, &file.stored_destination());
                        failed.insert(title.title_id.clone(), error.to_string());
                    } else if fatal.is_none() {
                        progress.note_file_done(&title.title_id, file);
                    }
                    let count = remaining.get_mut(&title.title_id).expect("admitted title");
                    *count = count.saturating_sub(1);
                    if *count == 0 && stop.is_none() && fatal.is_none() && !failed.contains_key(&title.title_id) {
                        if let Err(error) = self.finish_pipeline_title(operation, title, progress, plan, &mut failed).await { fatal = Some(error); }
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
                } else if let Some(detail) = failed.get(id) {
                    row.state = TitleCheckpointState::Failed;
                    row.detail = Some(detail.clone());
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
                if let Err(error) = self
                    .write_progress(
                        operation,
                        LocationOperationState::Moving,
                        progress,
                        plan,
                        None,
                        false,
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
            if let Some(detail) = failed.remove(&title.title_id) {
                self.settle_title(
                    operation,
                    title,
                    TitleCheckpointState::Failed,
                    Some(detail.clone()),
                    progress,
                    plan,
                )
                .await?;
                failures.push(format!("{}: {detail}", title.title_id));
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
    ) -> AppResult<VerifiedFile> {
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
            match result {
                Err(error) if attempt < self.retry.attempts && move_error_is_transient(&error) => {
                    tokio::time::sleep(self.retry.delay_before(attempt + 1)).await;
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    async fn finish_pipeline_title(
        &self,
        operation: &LocationOperation,
        title: &PlannedTitle,
        progress: &mut RunProgress,
        plan: &OperationWorkPlan,
        failed: &mut BTreeMap<String, String>,
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
        let paths = self
            .store
            .verified_destination_paths(&operation.id, &title.title_id)
            .await?;
        // The completeness gate reconstructs these counts from durable proofs.
        progress.files_done.remove(&title.title_id);
        progress.bytes_done.remove(&title.title_id);
        match self
            .run_title(operation, title, &paths, progress, plan)
            .await
        {
            Ok(TitleRunOutcome::Finished(mut warnings)) => {
                warnings.extend(transfer_detail);
                let state = if warnings.is_empty() {
                    TitleCheckpointState::Completed
                } else {
                    TitleCheckpointState::CompletedWithWarnings
                };
                let detail = (!warnings.is_empty()).then(|| warnings.join("; "));
                self.settle_title(operation, title, state, detail, progress, plan)
                    .await?;
                if let Some(hub) = self.transfers {
                    hub.title_finalized(&operation.id, started.elapsed());
                }
                progress.warnings.extend(warnings);
            }
            Ok(TitleRunOutcome::Canceled) => {}
            Err(error) => {
                failed.insert(title.title_id.clone(), error.to_string());
            }
        }
        Ok(())
    }
}
