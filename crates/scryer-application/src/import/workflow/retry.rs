async fn validate_import_retry_source(
    app: &AppUseCase,
    record: &ImportRecord,
    completed: &CompletedDownload,
) -> AppResult<scryer_domain::download_identity::DownloadId> {
    let locator = completed_download_identity(completed);
    if locator
        != ClientJobLocator::new(
            record.source_client_id.as_deref(),
            &record.source_system,
            &record.source_ref,
        )
    {
        return Err(AppError::validation_refused(crate::error_reasons::import::RECORD_CONFLICTS_COMPLETED_SOURCE_IDENTITY, "the import record conflicts with its completed source identity; use manual import"
                ));
    }
    let registry = &app.services.workflow.download_registry;
    let binding = registry.find_active_binding_by_locator(&locator).await?;
    let canonical = app
        .services
        .workflow
        .imports
        .canonical_download_id_for_import(&record.id)
        .await?
        .ok_or_else(|| {
            AppError::validation_refused(crate::error_reasons::import::COMPLETED_SOURCE_NO_TRUSTWORTHY_DOWNLOAD_IDENTITY, "the completed source has no trustworthy download identity; use manual import"
                    )
        })?;
    if binding
        .as_ref()
        .is_some_and(|binding| binding.download_id != canonical)
    {
        return Err(AppError::validation_refused(crate::error_reasons::import::CLIENT_JOB_ID_NOW_BELONGS_DIFFERENT_DOWNLOAD, "the client job ID now belongs to a different download; use manual import for the retained files"));
    }
    let submissions = &app.services.workflow.download_submissions;
    let identity = DownloadSubmissionIdentity::default();
    let state = submissions
        .get_identity_tracked_state_for_download(Some(&canonical), &identity, Some(&locator))
        .await?;
    let reason = submissions
        .get_identity_tracked_state_reason_for_download(Some(&canonical), &identity, Some(&locator))
        .await?;
    if reason.as_deref() == Some(crate::IMPORT_RETRY_TRACKED_STATE_REASON) {
        return Err(AppError::validation_refused(crate::error_reasons::import::RETRY_ALREADY_RUNNING_AWAITING_RECONCILIATION, "this import retry is already running or awaiting reconciliation"));
    }
    if matches!(state.as_deref(), Some("failed" | "failed_pending"))
        && reason.as_deref()
            != Some(crate::tracked_downloads::IMPORT_GATE_REJECTED_TRACKED_STATE_REASON)
    {
        return Err(AppError::validation_refused(crate::error_reasons::import::DOWNLOAD_FAILED, "the download failed; retry the download before importing"));
    }
    if matches!(
        state.as_deref(),
        Some("imported" | "imported_seeding" | "ignored")
    ) {
        return Err(AppError::validation_refused(crate::error_reasons::import::DOWNLOAD_ALREADY_SETTLED, "the download is already settled; refresh its history"));
    }
    if !completed.client_id.trim().is_empty()
        && let Some(config) = app
            .services
            .integrations
            .download_client_configs
            .get_by_id(&completed.client_id)
            .await?
    {
        if !config
            .client_type
            .eq_ignore_ascii_case(&completed.client_type)
        {
            return Err(AppError::validation_refused(crate::error_reasons::import::DOWNLOAD_CLIENT_CONFIGURATION_NO_LONGER_MATCHES_SOURCE, "the download client configuration no longer matches this source; use manual import"));
        }
        let mut offset = 0;
        let mut established = false;
        for _ in 0..10 {
            match app
                .services
                .integrations
                .download_client
                .observe_download(&locator, offset)
                .await?
            {
                crate::DownloadClientObservation::Present(item) => {
                    if ClientJobLocator::new(
                        Some(&item.client_id),
                        &item.client_type,
                        &item.download_client_item_id,
                    ) != locator
                    {
                        return Err(AppError::validation_refused(crate::error_reasons::import::CLIENT_RETURNED_DIFFERENT_DOWNLOAD_IDENTITY, "the client returned a different download identity; refresh and retry"
                                ));
                    }
                    if !matches!(
                        item.state,
                        scryer_domain::DownloadQueueState::Completed
                            | scryer_domain::DownloadQueueState::ImportPending
                    ) {
                        return Err(AppError::validation_refused(crate::error_reasons::import::CLIENT_DOES_NOT_REPORT_COMPLETED_DOWNLOAD, "the client does not report a completed download; import retry cannot restart a failed or active download"));
                    }
                    established = true;
                    break;
                }
                crate::DownloadClientObservation::Absent => {
                    established = true;
                    break;
                }
                crate::DownloadClientObservation::Unknown {
                    next_history_offset,
                    ..
                } => {
                    if next_history_offset <= offset {
                        break;
                    }
                    offset = next_history_offset;
                }
            }
        }
        if !established {
            return Err(AppError::validation_refused(crate::error_reasons::import::DOWNLOAD_STATE_UNAVAILABLE, "the client's current download state could not be established; try again when its history is available"));
        }
    }
    if Disk::process().follow(&completed.dest_dir).await.exists() != Some(true) {
        return Err(AppError::validation_refused(crate::error_reasons::import::SOURCE_UNAVAILABLE, "the import source is no longer available; restore the download or correct its path before retrying"));
    }
    Ok(canonical)
}

pub(crate) async fn retry_tracked_import(
    app: &AppUseCase,
    actor: &User,
    tracked: &crate::tracked_downloads::TrackedDownload,
) -> AppResult<ImportResult> {
    let source = ClientJobLocator::new(
        Some(&tracked.client_id),
        &tracked.client_type,
        &tracked.client_item.download_client_item_id,
    );
    let mut records = app
        .services
        .workflow
        .imports
        .list_imports_for_identities(&[source])
        .await?;
    records.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    for record in records {
        if app
            .services
            .workflow
            .imports
            .canonical_download_id_for_import(&record.id)
            .await?
            == Some(tracked.download_id)
        {
            return retry_failed_import(app, actor, &record.id, None).await;
        }
    }
    Err(AppError::validation_refused(crate::error_reasons::import::NO_COMPLETED_IMPORT_ATTEMPT_UNAVAILABLE, "no completed import attempt is available to retry; use manual import"))
}

async fn reconcile_claimed_history_retry(
    app: &AppUseCase,
    td: &mut crate::tracked_downloads::TrackedDownload,
    completed: &CompletedDownload,
    evidence: &ReleaseEvidence,
    result: &ImportResult,
    claim: &crate::ImportRetryClaim,
) -> AppResult<()> {
    compute_history_retry_state(app, td, completed, evidence, result).await?;
    let reason = if td.state == TrackedDownloadState::ImportBlocked {
        Some(crate::tracked_downloads::ImportBlockedReason::AfterImport.as_str())
    } else if td.burned_by_import_gate {
        Some(crate::tracked_downloads::IMPORT_GATE_REJECTED_TRACKED_STATE_REASON)
    } else {
        None
    };
    if !app
        .services
        .workflow
        .imports
        .finish_import_retry(
            claim,
            td.state,
            reason,
            td.status_messages.first().map(|message| message.text.as_str()),
        )
        .await?
        .is_finalized()
    {
        return Err(AppError::validation_refused(crate::error_reasons::import::NEWER_IMPORT_ATTEMPT_OWNS_DOWNLOAD, "a newer import attempt owns this download; refresh its history"));
    }
    if td.state.counts_as_imported() {
        crate::completed_download_handler::schedule_non_destructive_import_mark(
            app,
            td,
            result,
            Some(completed),
        );
    }
    app.refresh_import_record_queue_snapshot(&claim.import_id)
        .await;
    Ok(())
}

/// An interrupted retry that ends as already imported placed nothing in this
/// run, so whether each download file it imported is the library file itself
/// is settled now, before the download's client may remove it with its data.
/// A file imported from an owned archive workspace is Scryer's own copy and
/// not the client's. One read of the library rows' paths for all of them.
/// The mark that keeps the client from removing the download's data is the
/// download's, so it is written once, for the first file found to be the
/// library file, and the files after it are not checked. Files the reads
/// could not tell apart without setting the download's file aside are
/// recorded once, together, for the download's cleanup to settle. A check
/// that fails fails the recovery, which keeps the claim and tries again on
/// its next pass.
async fn settle_interrupted_retry_library_files(
    app: &AppUseCase,
    import_id: &str,
    completed: &CompletedDownload,
    artifacts: &[ImportArtifact],
) -> AppResult<()> {
    let placed: Vec<(&str, &str)> = artifacts
        .iter()
        .filter(|artifact| matches!(artifact.result.as_str(), "imported" | "already_present"))
        .filter(|artifact| artifact.workspace_relative_path.is_none())
        .filter_map(|artifact| {
            Some((
                artifact.imported_media_file_id.as_deref()?,
                artifact.relative_path.as_deref()?,
            ))
        })
        .collect();
    if placed.is_empty() {
        return Ok(());
    }
    let file_ids: Vec<String> = placed.iter().map(|(id, _)| (*id).to_string()).collect();
    let library_paths: HashMap<String, String> = app
        .services
        .library
        .media_files
        .list_media_file_paths(&file_ids)
        .await?
        .into_iter()
        .collect();
    let download_folder = stored_path_to_path_buf(&completed.dest_dir);
    let mut unsettled = Vec::new();
    for (file_id, relative_path) in placed {
        let Some(library_path) = library_paths.get(file_id) else {
            continue;
        };
        let source = download_folder.join(relative_path);
        let library_path = stored_path_to_path_buf(library_path);
        match settle_already_imported_source(app, import_id, &source, &library_path).await? {
            // The download's data is kept from its client: nothing is left
            // for a cleanup to settle.
            crate::LibraryFileSameness::Same => return Ok(()),
            crate::LibraryFileSameness::Apart => {}
            crate::LibraryFileSameness::Unsettled => {
                unsettled.push(crate::UnsettledLibraryFile::new(&source, &library_path));
            }
        }
    }
    record_library_files_unsettled(app, import_id, &unsettled).await
}

pub(crate) fn schedule_import_retry_recovery(app: &AppUseCase) {
    let Ok(cursor) = app
        .runtime
        .imports
        .execution_coordinator
        .retry_recovery_cursor
        .clone()
        .try_lock_owned()
    else {
        return;
    };
    let app = app.clone();
    tokio::spawn(async move {
        let mut cursor = cursor;
        match app
            .services
            .workflow
            .imports
            .list_import_retry_recovery(cursor.as_ref(), 25)
            .await
        {
            Ok(claims) => {
                *cursor = if claims.len() == 25 {
                    claims.last().map(|claim| claim.download_id)
                } else {
                    None
                };
                for claim in claims {
                    if let Err(error) = recover_import_retry(&app, &claim).await {
                        tracing::warn!(import_id = %claim.import_id, error = %error, "import retry remains pending reconciliation");
                    }
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "could not read import retry recovery work")
            }
        }
    });
}

pub(crate) async fn recover_import_retry(
    app: &AppUseCase,
    claim: &crate::ImportRetryClaim,
) -> AppResult<()> {
    let record = app
        .services
        .workflow
        .imports
        .get_import_by_id(&claim.import_id)
        .await?
        .ok_or_else(|| AppError::not_found_reason(crate::error_reasons::import::RETRY_IMPORT_RECORD_NOT_FOUND, "retry import record"))?;
    let payload: StoredCompletedImportRequestPayload =
        serde_json::from_str(&record.payload_json)
            .map_err(|e| AppError::Repository(format!("invalid completed import payload: {e}")))?;
    let (completed, evidence, target_title_id) = match payload {
        StoredCompletedImportRequestPayload::Current(payload) => (
            payload.completed,
            payload.release_evidence,
            payload.target_title_id,
        ),
        StoredCompletedImportRequestPayload::Legacy(completed) => {
            let provenance = resolve_import_provenance(
                app,
                completed,
                ImportProvenanceRequest {
                    identity_policy: CompletedImportIdentityPolicy::RequireSubmission,
                    queue_item: None,
                    requested_target_title_id: None,
                    release_evidence_override: None,
                    persisted: None,
                    tolerate_lookup_failure: false,
                },
            )
            .await?;
            (
                provenance.completed,
                provenance.release_evidence,
                provenance.target_title_id,
            )
        }
    };
    let Some(_permit) = app
        .runtime
        .imports
        .execution_coordinator
        .try_acquire_source(&completed)
        .await
    else {
        return Ok(());
    };
    // The recovery page can race a live attempt finishing while we acquire its permit.
    let current_claim = app
        .services
        .workflow
        .imports
        .get_import_retry_claim(&claim.download_id)
        .await?;
    if !current_claim.is_some_and(|current| {
        current.attempt_id == claim.attempt_id && current.import_id == claim.import_id
    }) {
        return Ok(());
    }
    let record = app
        .services
        .workflow
        .imports
        .get_import_by_id(&claim.import_id)
        .await?
        .ok_or_else(|| AppError::not_found_reason(crate::error_reasons::import::RETRY_IMPORT_RECORD_NOT_FOUND, "retry import record"))?;
    let id = crate::tracked_downloads::tracked_download_id(
        Some(&completed.client_id),
        &completed.client_type,
        &completed.download_client_item_id,
    );
    let handle = app.runtime.acquisition.tracked_download_handle.clone();
    let mut tracked = if let Some(handle) = handle.as_ref() {
        handle
            .begin_history_retry(id.clone(), claim.download_id)
            .await?
    } else {
        None
    };
    let mut finished = None;
    let outcome = async {
        if tracked.is_none() { tracked = retry_tracked_snapshot(app, &claim.import_id, &completed).await?; }
        let td = tracked.as_mut().ok_or_else(|| AppError::Repository("retry snapshot unavailable".into()))?;
        // Claiming cleared the old result, so any recorded result belongs to this execution.
        let result = record.result_json.as_deref().and_then(|json| serde_json::from_str::<ImportResult>(json).ok());
        let result = match result {
            Some(result) => result,
            None => {
                let artifacts = app.services.workflow.import_artifacts.list_by_source_identity_for_download(Some(&claim.download_id), &claim.source).await?;
                let verified_candidate = artifacts.iter().any(|artifact| matches!(artifact.result.as_str(), "imported" | "already_present"));
                if verified_candidate {
                    settle_interrupted_retry_library_files(app, &claim.import_id, &completed, &artifacts).await?;
                }
                let result = ImportResult {
                    decision: if verified_candidate { ImportDecision::Skipped } else { ImportDecision::Failed },
                    skip_reason: Some(if verified_candidate { scryer_domain::ImportSkipReason::AlreadyImported } else { scryer_domain::ImportSkipReason::PolicyMismatch }),
                    error_message: Some("Import retry was interrupted; review retained files and retry import if needed".into()),
                    title_id: evidence.title_id().map(str::to_string).or_else(|| target_title_id.clone()),
                    ..base_completed_import_result(&claim.import_id, &completed, &evidence, claim.started_at)
                };
                app.update_import_status_and_notify(&claim.import_id, ImportStatus::Failed, serde_json::to_string(&result).ok()).await?;
                result
            }
        };
        reconcile_claimed_history_retry(app, td, &completed, &evidence, &result, claim).await?;
        finished = tracked.take().map(Box::new);
        Ok(())
    }.await;
    if let Some(handle) = handle {
        handle
            .finish_history_retry(id, claim.download_id, finished)
            .await?;
    }
    outcome
}

#[cfg(all(test, unix))]
mod already_imported_entry_tests {
    use super::library_file_is_import_source_blocking as already_imported_is_source_entry;
    use std::path::Path;

    fn download(root: &Path) -> std::path::PathBuf {
        let folder = root.join("downloads");
        std::fs::create_dir_all(&folder).expect("downloads");
        let source = folder.join("sample.mkv");
        std::fs::write(&source, b"only video bytes").expect("source");
        source
    }

    /// An unsettled library file's paths round-trip through their stored
    /// form, a download file whose name is not UTF-8 included, so a cleanup
    /// tests the very files the import found.
    #[test]
    fn unsettled_library_file_paths_round_trip_when_not_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let source = Path::new("/downloads").join(std::ffi::OsStr::from_bytes(b"sample-\xff.mkv"));
        let library = Path::new("/library/Sample.Fixture.mkv");
        let file = crate::UnsettledLibraryFile::new(&source, library);
        assert_eq!(crate::stored_paths::stored_path_to_path_buf(&file.source_path), source);
        assert_eq!(file.source(), source);
        assert_eq!(file.library(), library);
    }

    /// The download's own file under another name is the library file: its
    /// folder reached through a link, or a link to it. A separate file of the
    /// same content, a second hard link, and a download file that is gone are
    /// not. The download's file is back at its name every time.
    #[test]
    fn the_library_file_is_the_download_file_only_when_one_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = download(dir.path());
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(source.parent().unwrap(), &alias).expect("folder link");
        let library = dir.path().join("library");
        std::fs::create_dir_all(&library).expect("library");
        let pointer = library.join("Pointer.mkv");
        std::os::unix::fs::symlink(&source, &pointer).expect("file link");
        let separate = library.join("Separate.mkv");
        std::fs::write(&separate, b"only video bytes").expect("separate");
        // Last, since a second name of the download's file reads as another
        // entry wherever that file is read.
        let linked = library.join("Linked.mkv");
        for (library_path, same) in [
            (alias.join("sample.mkv"), true),
            (pointer, true),
            (separate, false),
            (linked.clone(), false),
        ] {
            if library_path == linked {
                std::fs::hard_link(&source, &linked).expect("hard link");
            }
            assert_eq!(
                already_imported_is_source_entry(&source, &library_path).expect("told"),
                same,
                "{}",
                library_path.display()
            );
            assert_eq!(std::fs::read(&source).unwrap(), b"only video bytes");
            assert!(!crate::disk::aside_path_for(&source).exists());
        }

        let gone = source.with_file_name("gone.mkv");
        assert!(!already_imported_is_source_entry(&gone, &library.join("Separate.mkv")).unwrap());
    }

    /// A download's file an earlier check left at its set-aside name goes back
    /// to its name first, and the check then settles as it would have: a
    /// separate library file is not it, its own folder reached through a link
    /// is.
    #[test]
    fn a_source_left_set_aside_is_put_back_before_the_check_settles() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = download(dir.path());
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(source.parent().unwrap(), &alias).expect("folder link");
        let library = dir.path().join("library");
        std::fs::create_dir_all(&library).expect("library");
        let separate = library.join("Separate.mkv");
        std::fs::write(&separate, b"only video bytes").expect("separate");
        let aside = crate::disk::aside_path_for(&source);
        for (library_path, same) in [(separate.clone(), false), (alias.join("sample.mkv"), true)] {
            std::fs::rename(&source, &aside).expect("left set aside");
            assert_eq!(
                already_imported_is_source_entry(&source, &library_path).expect("told"),
                same,
                "{}",
                library_path.display()
            );
            assert_eq!(std::fs::read(&source).unwrap(), b"only video bytes");
            assert!(!aside.exists(), "nothing is left at the set-aside name");
        }
        assert_eq!(std::fs::read(&separate).unwrap(), b"only video bytes");
    }

    /// A set-aside file that cannot go back is kept where it is and named in
    /// the refusal; nothing is removed.
    #[test]
    fn a_source_that_cannot_be_put_back_is_kept_and_named() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let source = download(dir.path());
        let folder = source.parent().unwrap().to_path_buf();
        let library = dir.path().join("library");
        std::fs::create_dir_all(&library).expect("library");
        let separate = library.join("Separate.mkv");
        std::fs::write(&separate, b"only video bytes").expect("separate");
        let aside = crate::disk::aside_path_for(&source);
        std::fs::rename(&source, &aside).expect("left set aside");
        // The folder can be read but not renamed in, so the put-back fails.
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = already_imported_is_source_entry(&source, &separate);
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        if std::fs::metadata(&source).is_ok() {
            // A process that may rename in any folder put it back anyway.
            eprintln!("skipped: this process may rename in a read-only folder");
            return;
        }
        let Err(crate::AppError::ManualReconciliationRefused { reason, facts, .. }) = result else {
            panic!("refused with the kept path: {result:?}");
        };
        assert_eq!(reason, crate::error_reasons::import::DESTINATION_ALIAS_UNKNOWN);
        assert!(facts.iter().any(|fact| {
            fact.key == crate::error_reasons::import::KEPT_PATH_FACT
                && fact.value == crate::RefusalFactValue::Text(aside.display().to_string())
        }));
        assert_eq!(std::fs::read(&aside).unwrap(), b"only video bytes");
        assert_eq!(std::fs::read(&separate).unwrap(), b"only video bytes");
    }
}
