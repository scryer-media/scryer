/// Retry a previously failed import, optionally with an archive password.
pub async fn retry_failed_import(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    password: Option<&str>,
) -> AppResult<ImportResult> {
    let record = app
        .services
        .workflow
        .imports
        .get_import_by_id(import_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("import {import_id}")))?;

    if record.status != ImportStatus::Failed {
        return Err(AppError::Validation(format!(
            "import {} has status '{}', only failed imports can be retried",
            import_id,
            record.status.as_str()
        )));
    }

    let payload: StoredCompletedImportRequestPayload = serde_json::from_str(&record.payload_json)
        .map_err(|e| AppError::Repository(format!("failed to deserialize import payload: {e}")))?;
    let (mut completed, persisted) = match payload {
        StoredCompletedImportRequestPayload::Current(payload) => {
            (payload.completed.clone(), Some(payload))
        }
        StoredCompletedImportRequestPayload::Legacy(completed) => (completed, None),
    };
    remap_completed_download_for_client(app, &mut completed).await;

    // A live submission row is authoritative over what the failed attempt
    // persisted (an operator may have reassigned the download since); the
    // persisted evidence is the fallback for a lost row or a transient lookup
    // failure only.
    let ImportProvenance {
        completed,
        release_evidence,
        target_title_id,
        ..
    } = resolve_import_provenance(
        app,
        completed,
        ImportProvenanceRequest {
            identity_policy: CompletedImportIdentityPolicy::RequireSubmission,
            queue_item: None,
            requested_target_title_id: None,
            release_evidence_override: None,
            persisted: persisted.as_ref(),
            tolerate_lookup_failure: true,
        },
    )
    .await?;

    let authorization_title_id = release_evidence
        .title_id()
        .map(str::to_string)
        .or_else(|| target_title_id.clone())
        .or_else(|| {
            extract_parameter(&completed.parameters, "*scryer_title_id")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
    if let Some(title_id) = authorization_title_id
    {
        let title = app
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        app.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
    } else if app
        .authorized_library_ids(
            actor,
            None,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?
        .is_empty()
    {
        return Err(AppError::Unauthorized(
            "You do not have access to this library".to_string(),
        ));
    }

    app.update_import_status_and_notify(import_id, ImportStatus::Processing, None)
        .await?;

    let started_at = Utc::now();
    match run_import(
        app,
        actor,
        import_id,
        &completed,
        &release_evidence,
        target_title_id.as_deref(),
        started_at,
        password,
        None,
    )
    .await
    {
        Ok(result) => Ok(result),
        Err(error) => {
            let skip_reason = if crate::archive_extractor::is_password_required_error(&error) {
                Some(ImportSkipReason::PasswordRequired)
            } else if crate::archive_extractor::is_timeout_error(&error) {
                Some(ImportSkipReason::ArchiveExtractionTimedOut)
            } else {
                None
            };
            let result = ImportResult {
                decision: ImportDecision::Failed,
                skip_reason,
                error_message: Some(error.to_string()),
                release_burned: false,
                ..base_completed_import_result(
                    import_id,
                    &completed,
                    &release_evidence,
                    started_at,
                )
            };
            let result_json = serde_json::to_string(&result).ok();
            app.update_import_status_and_notify(import_id, ImportStatus::Failed, result_json)
                .await?;
            Ok(result)
        }
    }
}
/// Identifies why a failed download reached terminal cleanup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalFailureOrigin {
    ClientFailure,
    ImportGate,
}

pub(crate) async fn should_remove_terminal_download(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    library_id: Option<&str>,
    facet: Option<&MediaFacet>,
    state: TrackedDownloadState,
    cache: Option<&TerminalCleanupTickCache>,
) -> bool {
    let client_id = client_id.trim();
    let routing_key = if client_id.is_empty() {
        client_type
    } else {
        client_id
    };

    match state {
        TrackedDownloadState::Imported | TrackedDownloadState::ImportedSeeding => match facet {
            Some(facet) => {
                should_remove_completed_download_cached(app, library_id, facet, routing_key, cache)
                    .await
            }
            None => false,
        },
        TrackedDownloadState::Failed => match facet {
            Some(facet) => {
                app.should_remove_failed_download(library_id, facet, routing_key)
                    .await
            }
            None => false,
        },
        TrackedDownloadState::Ignored => true,
        _ => false,
    }
}

/// Remove a terminal download's entry from its client, subject to the
/// seeding-aware gate.
///
/// Removing a torrent's entry stops it seeding even with `remove_data: false`,
/// so for torrent-protocol items an `Imported` state is no longer sufficient
/// on its own — the gate has to agree that the seeding obligation is
/// discharged. Failed and Ignored downloads are deliberately *not* gated:
/// blocklist and retry must never wait on seeding (Sonarr's rule).
///
/// Once a removal is agreed, a torrent's payload goes with the entry
/// (`remove_data`, Sonarr's `deleteData: true`); see the call site for which
/// states qualify and which keep today's behavior.
#[expect(
    clippy::too_many_arguments,
    reason = "terminal cleanup carries client identity, routing scope, state, and the seeding gate's view of the client entry"
)]
async fn reconcile_terminal_download_cleanup(
    app: &AppUseCase,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    library_id: Option<&str>,
    facet: Option<&MediaFacet>,
    state: TrackedDownloadState,
    failure_origin: TerminalFailureOrigin,
    precomputed_should_remove: Option<bool>,
    present_in_client: bool,
    // The freshest seeding observation the caller holds, or `None` to have the
    // gate look one up from the published tracked-download snapshot.
    observation: Option<crate::seeding_gate::TorrentSeedingObservation>,
    // The reconcile tick's shared reads, or `None` for callers outside a tick
    // (manual import), which take the per-row path.
    cache: Option<&TerminalCleanupTickCache>,
) -> TerminalDownloadCleanup {
    let client_id = client_id.trim();
    let should_remove = match precomputed_should_remove {
        Some(should_remove) => should_remove,
        None => {
            should_remove_terminal_download(
                app,
                client_id,
                client_type,
                library_id,
                facet,
                state,
                cache,
            )
            .await
        }
    };

    if !should_remove {
        return TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::NotConfigured);
    }

    // Carried past the gate for the seeding history events: the gate consumes
    // the observation, and the release event has to report the ratio and seed
    // time the decision was actually taken on.
    let observed_ratio = observation
        .as_ref()
        .and_then(|observation| observation.seed_ratio);
    let observed_seed_time_seconds = observation
        .as_ref()
        .and_then(|observation| observation.seed_time_seconds);
    // The client's own removal verdict, post-trust-floor — the same value the
    // gate would read. A client-reported failure does not enter the gate and
    // retains this behavior unchanged.
    let observed_can_remove = observation
        .as_ref()
        .and_then(|observation| observation.can_remove);
    let report = |reason: &'static str, action: Option<SeedingReleaseAction>| SeedingGateReport {
        reason,
        action,
        seed_ratio: observed_ratio,
        seed_time_seconds: observed_seed_time_seconds,
    };

    let is_torrent = crate::seeding_gate::client_type_is_torrent(app, client_type);
    let mut seeding_report = None;
    if state.counts_as_imported()
        || (state == TrackedDownloadState::Failed
            && failure_origin == TerminalFailureOrigin::ImportGate
            && is_torrent)
    {
        let key = crate::seeding_gate::SeedGoalLookupKey {
            canonical_download_id: canonical_download_id.cloned(),
            client_id: client_id.to_string(),
            client_type: client_type.trim().to_string(),
            client_item_id: download_client_item_id.trim().to_string(),
            info_hash: crate::normalize_torrent_info_hash(Some(download_client_item_id)),
        };
        let decision = crate::seeding_gate::evaluate_seeding_gate_with(
            app,
            &key,
            present_in_client,
            observation,
            cache.map(TerminalCleanupTickCache::goal_batch),
        )
        .await;
        match decision.outcome {
            crate::seeding_gate::SeedingGateOutcome::NotApplicable => {}
            crate::seeding_gate::SeedingGateOutcome::Vanished => {
                return TerminalDownloadCleanup::gated(
                    TerminalDownloadCleanupOutcome::AlreadyGone,
                    report(decision.reason, Some(SeedingReleaseAction::Vanished)),
                );
            }
            crate::seeding_gate::SeedingGateOutcome::HandedOff => {
                tracing::info!(
                    client_id,
                    client_type,
                    download_client_item_id,
                    state = state.as_str(),
                    reason = decision.reason,
                    "post-import handoff: leaving the client entry untouched and no longer managing this torrent"
                );
                return TerminalDownloadCleanup::gated(
                    TerminalDownloadCleanupOutcome::HandedOff,
                    report(decision.reason, Some(SeedingReleaseAction::HandedOff)),
                );
            }
            crate::seeding_gate::SeedingGateOutcome::Hold => {
                tracing::debug!(
                    client_id,
                    client_type,
                    download_client_item_id,
                    state = state.as_str(),
                    reason = decision.reason,
                    "seeding gate is holding a torrent entry after import"
                );
                return TerminalDownloadCleanup::gated(
                    TerminalDownloadCleanupOutcome::HeldForSeeding,
                    report(decision.reason, None),
                );
            }
            crate::seeding_gate::SeedingGateOutcome::Released { action } => match action {
                scryer_domain::SeedGoalMetAction::RemoveEntry => {
                    seeding_report =
                        Some(report(decision.reason, Some(SeedingReleaseAction::Removed)));
                }
                scryer_domain::SeedGoalMetAction::StopSeeding => {
                    let stopped = stop_seeding_for_terminal_download(
                        app,
                        client_id,
                        client_type,
                        download_client_item_id,
                        decision.reason,
                    )
                    .await;
                    return TerminalDownloadCleanup::gated(
                        TerminalDownloadCleanupOutcome::SeedingEntryKept,
                        report(decision.reason, Some(stopped)),
                    );
                }
                scryer_domain::SeedGoalMetAction::Keep => {
                    tracing::info!(
                        client_id,
                        client_type,
                        download_client_item_id,
                        reason = decision.reason,
                        "seeding goal met; keeping the client entry per profile policy"
                    );
                    return TerminalDownloadCleanup::gated(
                        TerminalDownloadCleanupOutcome::SeedingEntryKept,
                        report(decision.reason, Some(SeedingReleaseAction::Kept)),
                    );
                }
            },
        }
    }

    let is_history = matches!(
        state,
        TrackedDownloadState::Imported
            | TrackedDownloadState::ImportedSeeding
            | TrackedDownloadState::Failed
            | TrackedDownloadState::Ignored
    );

    // Sonarr removes an imported download's data with its entry
    // (`RemoveItem(item, deleteData: true)`, DownloadEventHub
    // .RemoveFromDownloadClient), and does the same on failure — but only after
    // `Handle(DownloadFailedEvent)` returns early unless
    // `trackedDownload.DownloadItem.CanBeRemoved`. For torrents that verdict is
    // the client's seed-limit answer, and only a *manual* failure forces it
    // (`TrackedDownload.Fail()`); an automatic one leaves it alone. So:
    //
    // - `Imported`/`ImportedSeeding` reaching this line means the gate released
    //   the entry with `RemoveEntry`: the obligation is discharged, the import
    //   already produced the library file, and a copy import's client-side copy
    //   would otherwise be orphaned.
    // - A client-reported `Failed` never enters the gate — no private rail, no
    //   `never_remove`, no HandOff — so the client's own `can_remove` is the
    //   only rail there is. A torrent it will not release (or cannot answer
    //   for) keeps today's entry-only removal.
    // - A burned import-gate `Failed` is a release failure, so torrents use
    //   the same gate and data behavior as imported torrents while Usenet
    //   history deletion includes its client-side data.
    // - `Ignored` keeps today's behavior on purpose: the operator told Scryer
    //   to stop tracking the download, not to delete what it downloaded.
    //
    // `torrent-blackhole` is excluded outright. Its "remove" is a
    // `remove_dir_all` on a watch folder some *other* client is seeding from;
    // the gate refuses it for imported states, and `Failed` skips the gate, so
    // the rule has to be restated here.
    let torrent_data_removal_allowed = is_torrent
        && !client_type
            .trim()
            .eq_ignore_ascii_case(crate::seeding_gate::TORRENT_BLACKHOLE_CLIENT_TYPE);
    let remove_data = match state {
        TrackedDownloadState::Imported | TrackedDownloadState::ImportedSeeding => {
            torrent_data_removal_allowed
        }
        TrackedDownloadState::Failed
            if failure_origin == TerminalFailureOrigin::ImportGate && is_torrent =>
        {
            torrent_data_removal_allowed
        }
        TrackedDownloadState::Failed
            if failure_origin == TerminalFailureOrigin::ImportGate =>
        {
            true
        }
        TrackedDownloadState::Failed => {
            observed_can_remove == Some(true) && torrent_data_removal_allowed
        }
        _ => false,
    };

    let delete_result = if client_id.is_empty() {
        app.services
            .integrations
            .download_client
            .delete_queue_item_for_client(
                client_type,
                download_client_item_id,
                is_history,
                remove_data,
            )
            .await
    } else {
        app.services
            .integrations
            .download_client
            .delete_queue_item_for_client_id(
                client_id,
                download_client_item_id,
                is_history,
                remove_data,
            )
            .await
    };

    let outcome = match delete_result {
        Ok(()) => TerminalDownloadCleanupOutcome::Removed,
        Err(error) => {
            if !terminal_download_item_is_still_visible(
                app,
                client_id,
                client_type,
                download_client_item_id,
                is_history,
            )
            .await
            {
                tracing::debug!(
                    client_id,
                    client_type,
                    download_client_item_id,
                    state = state.as_str(),
                    error = %error,
                    "download item was already absent after delete error"
                );
                TerminalDownloadCleanupOutcome::AlreadyGone
            } else {
                tracing::warn!(
                    client_id,
                    client_type,
                    download_client_item_id,
                    state = state.as_str(),
                    error = %error,
                    "failed to remove terminal download from client"
                );
                TerminalDownloadCleanupOutcome::RetryableFailure
            }
        }
    };

    // The removal may have failed after the gate released the entry; report
    // what actually happened rather than the intent.
    let seeding = seeding_report.map(|report| SeedingGateReport {
        action: match outcome {
            TerminalDownloadCleanupOutcome::Removed => Some(SeedingReleaseAction::Removed),
            TerminalDownloadCleanupOutcome::AlreadyGone => Some(SeedingReleaseAction::Vanished),
            _ => Some(SeedingReleaseAction::Kept),
        },
        ..report
    });
    TerminalDownloadCleanup { outcome, seeding }
}
/// `SeedGoalMetAction::StopSeeding`: leave the entry in the client but stop it
/// uploading.
///
/// Pause is the only stop control the download-client port exposes
/// (`DownloadControlAction::Pause` in the plugin SDK), and for a torrent that
/// has finished downloading, paused *is* stopped seeding. A client that does
/// not support pause degrades to `Keep`: the entry stays and nothing is
/// removed, which is the safe direction.
async fn stop_seeding_for_terminal_download(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    reason: &'static str,
) -> SeedingReleaseAction {
    let paused = if client_id.is_empty() {
        app.services
            .integrations
            .download_client
            .pause_queue_item(download_client_item_id)
            .await
    } else {
        app.services
            .integrations
            .download_client
            .pause_queue_item_for_client(client_id, download_client_item_id)
            .await
    };

    match paused {
        Ok(()) => {
            tracing::info!(
                client_id,
                client_type,
                download_client_item_id,
                reason,
                "seeding goal met; paused the torrent per profile policy"
            );
            SeedingReleaseAction::Paused
        }
        Err(error) => {
            tracing::warn!(
                client_id,
                client_type,
                download_client_item_id,
                reason,
                error = %error,
                "seeding goal met but this client cannot stop the torrent; keeping the entry untouched"
            );
            SeedingReleaseAction::Kept
        }
    }
}

fn skip_reason_for_import_check_code(
    code: crate::import_checks::ImportCheckCode,
) -> ImportSkipReason {
    match code {
        crate::import_checks::ImportCheckCode::DuplicateFile => ImportSkipReason::AlreadyImported,
        crate::import_checks::ImportCheckCode::InsufficientDiskSpace => ImportSkipReason::DiskFull,
        crate::import_checks::ImportCheckCode::StillUnpacking => {
            ImportSkipReason::DownloadInProgress
        }
        crate::import_checks::ImportCheckCode::InvalidExtension
        | crate::import_checks::ImportCheckCode::SampleFile
        | crate::import_checks::ImportCheckCode::SampleDirectory => {
            ImportSkipReason::PolicyMismatch
        }
    }
}

async fn skip_reason_for_import_check_rejection(
    app: &AppUseCase,
    code: crate::import_checks::ImportCheckCode,
    dest_path: &Path,
) -> AppResult<ImportSkipReason> {
    if code.is_duplicate_file() {
        let stored_dest_path = path_to_stored_string(dest_path);
        let cataloged = app
            .services
            .library
            .media_files
            .get_media_file_by_path(&stored_dest_path)
            .await?
            .is_some();
        if !cataloged {
            return Ok(ImportSkipReason::DuplicateFile);
        }
    }
    Ok(skip_reason_for_import_check_code(code))
}

async fn finalize_import_source_cleanup(
    app: &AppUseCase,
    import_mode: scryer_domain::ImportMode,
    file_result: &scryer_domain::ImportFileResult,
    final_dest_path: &Path,
    completed: Option<&scryer_domain::CompletedDownload>,
) -> AppResult<scryer_domain::ImportStrategy> {
    if import_mode != scryer_domain::ImportMode::Move {
        return Ok(file_result.strategy);
    }

    // FR-044, at the application-level gate as well as inside the copy. The
    // importer already refuses to build a cleanup guard for a copy it could not
    // prove, so this is belt-and-braces — but source removal is irreversible,
    // and a guard that arrives next to a non-passing verification is a bug this
    // must not act on.
    if let Some(verification) = file_result.verification.as_ref()
        && !verification.permits_source_removal()
    {
        return Err(AppError::Repository(format!(
            "move import source cleanup blocked because the destination copy was not verified: {} ({})",
            file_result.dest_path.display(),
            verification.stamp(),
        )));
    }

    let guard = file_result.source_cleanup.clone().ok_or_else(|| {
        AppError::Repository(format!(
            "move import did not return a source cleanup guard for {}",
            file_result.source_path.display()
        ))
    })?;

    if let Some(verification) = file_result.verification.as_ref() {
        // FR-043: the applied depth is recorded wherever the import surface can
        // carry it today. Activity stamping is a later package; the fact is
        // already persisted on the media file (migration 0205).
        tracing::info!(
            dest_path = %file_result.dest_path.display(),
            verification = %verification.stamp(),
            bytes = verification.hashes.size_bytes,
            "verified import copy before removing the source"
        );
    }

    let execution_context = crate::ImportFileExecutionContext::new(
        completed.map_or("", |item| item.client_id.as_str()),
        completed.map_or("", |item| item.client_type.as_str()),
    );
    app.services
        .workflow
        .file_importer
        .remove_import_source_after_verified_import_with_context(
            guard,
            final_dest_path,
            &execution_context,
        )
        .await?;

    Ok(scryer_domain::ImportStrategy::Move)
}

async fn finalize_deferred_import_source_cleanup(
    app: &AppUseCase,
    source_cleanup: Option<scryer_domain::ImportSourceCleanupGuard>,
    final_dest_path: &Path,
    completed: Option<&scryer_domain::CompletedDownload>,
) -> AppResult<()> {
    let Some(guard) = source_cleanup else {
        return Ok(());
    };
    let execution_context = crate::ImportFileExecutionContext::new(
        completed.map_or("", |item| item.client_id.as_str()),
        completed.map_or("", |item| item.client_type.as_str()),
    );
    app.services
        .workflow
        .file_importer
        .remove_import_source_after_verified_import_with_context(
            guard,
            final_dest_path,
            &execution_context,
        )
        .await
}
/// Sonarr's phase rule, not an error-string catalogue: an import that was
/// approved but failed while *executing* (`ImportDecision::Failed` — locked or
/// still-growing files, IO, network shares, DB hiccups) is transient by
/// construction and is re-attempted automatically at a capped cadence.
/// Decision-phase outcomes (rejections, policy skips, unmatched identity) are
/// permanent and stay blocked for review. Two exceptions in each direction:
/// a password-protected archive can never succeed without operator input, and
/// disk-full / permission-denied skips are environmental and clear on their own.
/// The message allowlist remains as belt-and-braces for Scryer's own transient
/// markers that surface on non-`Failed` decisions.
pub(crate) fn completed_import_result_is_retryable(result: &ImportResult) -> bool {
    match result.decision {
        ImportDecision::Failed => !matches!(
            result.skip_reason,
            Some(
                ImportSkipReason::PasswordRequired
                    | ImportSkipReason::ArchiveExtractionPluginRequired
                    | ImportSkipReason::ArchiveExtractionTimedOut
            )
        ),
        _ => {
            matches!(
                result.skip_reason,
                Some(
                    ImportSkipReason::DownloadInProgress
                        | ImportSkipReason::DiskFull
                        | ImportSkipReason::PermissionDenied
                )
            ) || result
                .error_message
                .as_deref()
                .is_some_and(completed_import_error_message_is_retryable)
        }
    }
}

fn completed_import_status_for_result(
    result: &ImportResult,
    fallback_status: ImportStatus,
) -> ImportStatus {
    if result.skip_reason == Some(ImportSkipReason::NoVideoFiles)
        || completed_import_result_is_retryable(result)
    {
        ImportStatus::Pending
    } else {
        fallback_status
    }
}
