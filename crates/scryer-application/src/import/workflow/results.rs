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
        .map_err(|e| {
        AppError::Repository(format!("failed to deserialize import payload: {e}"))
    })?;
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
    if let Some(title_id) = authorization_title_id {
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
                ..base_completed_import_result(import_id, &completed, &release_evidence, started_at)
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
/// discharged. Automatic failed-torrent cleanup also honors frozen profile
/// obligations and requires the client's removal verdict. Operator Ignore is
/// an explicit entry-only override; blocklisting remains independent.
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
    record: Option<&crate::DownloadCleanupRecord>,
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
    // gate reads. Client-reported failures require this verdict in addition
    // to satisfying the persisted profile.
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
    if state == TrackedDownloadState::Failed
        && is_torrent
        && failure_origin == TerminalFailureOrigin::ClientFailure
        && observed_can_remove != Some(true)
    {
        return TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::RetryableFailure);
    }
    let mut seeding_report = None;
    if state.counts_as_imported() || (state == TrackedDownloadState::Failed && is_torrent) {
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
                        if stopped.is_some() {
                            TerminalDownloadCleanupOutcome::SeedingEntryKept
                        } else {
                            TerminalDownloadCleanupOutcome::RetryableFailure
                        },
                        report(decision.reason, stopped),
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
    // - A client-reported `Failed` requires both the client's `can_remove`
    //   verdict and the seeding gate. Failure cannot erase a frozen profile
    //   obligation, private rail, or `never_remove` policy.
    // - A burned import-gate `Failed` is a release failure, so torrents use
    //   the same gate and data behavior as imported torrents while Usenet
    //   history deletion includes its client-side data.
    // - `Ignored` keeps today's behavior on purpose: the operator told Scryer
    //   to stop tracking the download, not to delete what it downloaded.
    //
    // `torrent-blackhole` is excluded outright. Its "remove" is a
    // `remove_dir_all` on a watch folder some *other* client is seeding from;
    // the gate keeps it for imported and failed states.
    let torrent_data_removal_allowed = is_torrent
        && !client_type
            .trim()
            .eq_ignore_ascii_case(crate::seeding_gate::TORRENT_BLACKHOLE_CLIENT_TYPE);
    let remove_data = match state {
        TrackedDownloadState::Imported | TrackedDownloadState::ImportedSeeding => {
            !is_torrent || torrent_data_removal_allowed
        }
        TrackedDownloadState::Failed
            if failure_origin == TerminalFailureOrigin::ImportGate && is_torrent =>
        {
            torrent_data_removal_allowed
        }
        TrackedDownloadState::Failed if failure_origin == TerminalFailureOrigin::ImportGate => true,
        TrackedDownloadState::Failed => {
            observed_can_remove == Some(true) && torrent_data_removal_allowed
        }
        _ => false,
    };

    let mut native_payload_refused = record
        .and_then(|record| record.payload_checkpoint.as_deref())
        .and_then(|checkpoint| serde_json::from_str::<serde_json::Value>(checkpoint).ok())
        .is_some_and(|checkpoint| client_refused_payload_deletion(Some(&checkpoint)));
    let mut refused_record: Option<crate::DownloadCleanupRecord> = None;
    let (outcome, payload_disposition, payload_cleanup_checkpoint) = loop {
        // Weaver deletes its own files unless its API key was refused the
        // scope to do so: from then on (persisted on the checkpoint) the host
        // path deletes the payload and Weaver only removes the entry.
        //
        // TODO(0.21.0): remove this compatibility shim (`native_payload_refused`,
        // the `Unauthorized` arm below, `CLIENT_REFUSED_PAYLOAD_DELETION_KEY`
        // and its carry-through in `persist_plan` and `completed.rs`) once
        // Weaver integration-scoped keys can delete completed files again.
        let host_managed_payload = remove_data
            && (client_type.trim() != "weaver" || native_payload_refused)
            && !plugin_has_native_data_removal(app, client_id).await;
        let record = refused_record.as_ref().or(record);
        let mut payload_disposition = None;
        let payload_cleanup_checkpoint = if host_managed_payload {
            match remove_host_payload_before_entry_cleanup(
                app,
                canonical_download_id,
                client_id,
                client_type,
                download_client_item_id,
                record,
            )
            .await
            {
                Ok(report) => {
                    payload_disposition = Some(report.disposition);
                    report.filesystem_checkpoint
                }
                Err(error) => {
                    // Counted on the checkpoint rather than the row's claim count
                    // so seeding holds and client outages do not spend the host
                    // deletion retry budget.
                    let mut checkpoint = record
                        .and_then(|record| record.payload_checkpoint.as_deref())
                        .and_then(|checkpoint| serde_json::from_str::<serde_json::Value>(checkpoint).ok())
                        .filter(serde_json::Value::is_object)
                        .unwrap_or_else(|| serde_json::json!({}));
                    let failures = checkpoint
                        .get("host_payload_failures")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|failures| u32::try_from(failures).ok())
                        .unwrap_or(0)
                        .saturating_add(1);
                    if let Some(record) = record {
                        checkpoint["host_payload_failures"] = serde_json::json!(failures);
                        if let Err(error) = app
                            .services
                            .workflow
                            .download_submissions
                            .checkpoint_download_cleanup_payload(&record.download_id, &checkpoint.to_string())
                            .await
                        {
                            tracing::warn!(client_id, download_client_item_id, error = %error,
                                "failed to checkpoint host payload failure count");
                        }
                    }
                    let attempts = failures;
                    if attempts < HOST_PAYLOAD_CLEANUP_ATTEMPTS_BEFORE_ENTRY_REMOVAL {
                        tracing::warn!(
                            client_id,
                            client_type,
                            download_client_item_id,
                            state = state.as_str(),
                            attempts,
                            error = %error,
                            "failed to remove download client payload before removing the client entry; will retry"
                        );
                        let seeding = seeding_report.map(|report| SeedingGateReport {
                            action: Some(SeedingReleaseAction::Kept),
                            ..report
                        });
                        return TerminalDownloadCleanup {
                            outcome: TerminalDownloadCleanupOutcome::RetryableFailure,
                            seeding,
                            payload: None,
                        };
                    }
                    // Sonarr parity: host-side data deletion is best-effort. After
                    // repeated failures the client entry is removed anyway so the
                    // scope is released; the payload stays for the operator and
                    // the settled outcome says so.
                    tracing::warn!(
                        client_id,
                        client_type,
                        download_client_item_id,
                        state = state.as_str(),
                        attempts,
                        error = %error,
                        "giving up on host payload deletion; removing the client entry with the payload retained on disk"
                    );
                    payload_disposition = Some(HostPayloadDisposition::Unverified);
                    None
                }
            }
        } else {
            None
        };

        let payload_already_removed = record
            .and_then(|record| record.payload_checkpoint.as_deref())
            .and_then(|checkpoint| serde_json::from_str::<serde_json::Value>(checkpoint).ok())
            .is_some_and(|checkpoint| {
                checkpoint
                    .get("payload_removed")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
            });
        let client_remove_data = remove_data
            && !payload_already_removed
            && (!host_managed_payload || client_type == "sabnzbd");
        if (client_remove_data || !remove_data)
            && !host_managed_payload
            && let Some(record) = record
        {
            // Persist the operation and identity only after policy and seeding
            // eligibility pass. Authoritative absence can then recover a lost
            // response for either native data deletion or entry-only removal.
            let checkpoint = serde_json::json!({
                "native_remove_requested": client_remove_data,
                "entry_only_remove_requested": !remove_data,
                "download_id": record.download_id.to_string(),
                "client_id": record.client_id,
                "client_type": record.client_type,
                "item_id": record.item_id,
            })
            .to_string();
            if let Err(error) = app
                .services
                .workflow
                .download_submissions
                .checkpoint_download_cleanup_payload(&record.download_id, &checkpoint)
                .await
            {
                tracing::warn!(client_id, download_client_item_id, error = %error,
                    "failed to checkpoint native removal intent");
                return TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::RetryableFailure);
            }
        }
        let delete_result = if client_id.is_empty() {
            app.services
                .integrations
                .download_client
                .delete_queue_item_for_client(
                    client_type,
                    download_client_item_id,
                    is_history,
                    client_remove_data,
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
                    client_remove_data,
                )
                .await
        };

        let outcome = match delete_result {
            Ok(()) => {
                tracing::info!(
                    client_id,
                    client_type,
                    download_client_item_id,
                    remove_data,
                    "terminal download entry removed"
                );
                TerminalDownloadCleanupOutcome::Removed
            }
            Err(AppError::Unauthorized(error))
                if client_remove_data && client_type.trim() == "weaver" && !native_payload_refused =>
            {
                // Weaver kept the entry and refused only the files. Delete the
                // payload from the host and come back for the entry; remember
                // the refusal so later attempts do not ask Weaver again.
                tracing::warn!(
                    client_id,
                    client_type,
                    download_client_item_id,
                    state = state.as_str(),
                    error = %error,
                    "weaver refused payload deletion (API key lacks admin scope); falling back to host-side payload deletion"
                );
                native_payload_refused = true;
                if let Some(record) = record {
                    let mut checkpoint = record
                        .payload_checkpoint
                        .as_deref()
                        .and_then(|checkpoint| serde_json::from_str::<serde_json::Value>(checkpoint).ok())
                        .filter(serde_json::Value::is_object)
                        .unwrap_or_else(|| serde_json::json!({}));
                    checkpoint[CLIENT_REFUSED_PAYLOAD_DELETION_KEY] = serde_json::json!(true);
                    let checkpoint = checkpoint.to_string();
                    if let Err(error) = app
                        .services
                        .workflow
                        .download_submissions
                        .checkpoint_download_cleanup_payload(&record.download_id, &checkpoint)
                        .await
                    {
                        tracing::warn!(client_id, download_client_item_id, error = %error,
                            "failed to checkpoint the client's payload deletion refusal");
                    }
                    refused_record = Some(crate::DownloadCleanupRecord {
                        payload_checkpoint: Some(checkpoint),
                        ..record.clone()
                    });
                }
                continue;
            }
            Err(error) => {
                if !remove_data
                    && !terminal_download_item_is_still_visible(
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
        break (outcome, payload_disposition, payload_cleanup_checkpoint);
    };

    if matches!(
        outcome,
        TerminalDownloadCleanupOutcome::Removed | TerminalDownloadCleanupOutcome::AlreadyGone
    ) && let Some(checkpoint) = payload_cleanup_checkpoint.as_deref()
        && let Err(error) = clear_host_payload_cleanup_checkpoint(checkpoint).await
    {
        tracing::warn!(
            client_id,
            client_type,
            download_client_item_id,
            checkpoint = %checkpoint.display(),
            error = %error,
            "failed to clear completed download client payload cleanup checkpoint"
        );
    }

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
    TerminalDownloadCleanup {
        outcome,
        seeding,
        payload: payload_disposition,
    }
}

async fn plugin_has_native_data_removal(app: &AppUseCase, client_id: &str) -> bool {
    let Some(provider) = app
        .services
        .integrations
        .download_client_plugin_provider
        .available()
    else {
        return false;
    };
    let Ok(Some(config)) = app
        .services
        .integrations
        .download_client_configs
        .get_by_id(client_id)
        .await
    else {
        return false;
    };
    provider
        .client_for_config(&config)
        .is_some_and(|client| client.supports_native_data_removal())
}

/// How much of each inventoried file is sampled for its content proof: the
/// first and last 256 KiB. Enough to notice a replaced or rewritten file on a
/// retry without re-reading every archive volume in full.
const PAYLOAD_SAMPLE_BYTES: u64 = 256 * 1024;
const PAYLOAD_INVENTORY_MAX_ENTRIES: usize = 10_000;
const PAYLOAD_INVENTORY_MAX_DEPTH: usize = 8;
/// A host payload cleanup that keeps failing is retried this many times before
/// the client entry is removed anyway. Sonarr's `DeleteItemData` is
/// best-effort for the same reason: the client entry is what blocks the scope,
/// and an operator can sweep a leftover folder, whereas an entry that never
/// leaves the client keeps every future grab for that scope on hold.
pub(crate) const HOST_PAYLOAD_CLEANUP_ATTEMPTS_BEFORE_ENTRY_REMOVAL: u32 = 3;

/// What host-side payload deletion actually achieved for one terminal cleanup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HostPayloadDisposition {
    /// Every inventoried file was re-verified and removed, and the job
    /// directory (when there was one) is gone.
    Removed,
    /// Only verified files were removed; something unrecognised, changed, or
    /// not provably this job's stayed behind, and so did its directory.
    PartiallyRetained,
    /// Host deletion kept failing; the entry was removed with the payload
    /// left in place for the operator.
    Unverified,
}

struct HostPayloadCleanupReport {
    disposition: HostPayloadDisposition,
    filesystem_checkpoint: Option<std::path::PathBuf>,
}

/// One file Scryer expects to delete, captured before the first deletion and
/// re-verified file by file on every attempt. A file that no longer matches
/// its identity and content sample is not the file that was inventoried and
/// is left alone.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PayloadInventoryEntry {
    path: String,
    len: u64,
    modified_unix_nanos: Option<i64>,
    dev: Option<u64>,
    ino: Option<u64>,
    sample_bytes: u64,
    sample_blake3: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PayloadInventory {
    pub(crate) target: String,
    pub(crate) entries: Vec<PayloadInventoryEntry>,
    /// Relative paths inside the job directory that are not payload-class
    /// files (or are symlinks) and are never deleted by Scryer.
    pub(crate) retained: Vec<String>,
    pub(crate) directories: Vec<String>,
}

struct PayloadRemovalResult {
    removed: usize,
    retained: Vec<String>,
    directory_removed: bool,
}

/// Payload classes a download client leaves behind for a job: the media
/// itself, its subtitles and artwork, archive volumes, parity, and the scene
/// sidecars. Anything else is unknown to Scryer and stays on disk.
fn is_payload_class_file(path: &std::path::Path) -> bool {
    if scryer_domain::is_video_file(path) || scryer_domain::is_subtitle_file(path) {
        return true;
    }
    let Some(extension) = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
    else {
        return false;
    };
    const SIDECAR_EXTENSIONS: &[&str] = &[
        "rar", "zip", "7z", "par2", "nfo", "sfv", "srr", "srs", "txt", "log", "md5", "sha1",
        "sha256", "url", "diz", "m3u", "torrent", "nzb", "jpg", "jpeg", "png", "webp", "gif",
    ];
    if SIDECAR_EXTENSIONS.contains(&extension.as_str()) {
        return true;
    }
    // Multi-volume archives: `.r00`..`.r99` (and `.s00`, `.z00` continuations)
    // plus raw split volumes `.001`...
    let bytes = extension.as_bytes();
    bytes.len() == 3
        && bytes[1..].iter().all(u8::is_ascii_digit)
        && (matches!(bytes[0], b'r' | b's' | b'z') || bytes[0].is_ascii_digit())
}

/// A job directory is Scryer's to sweep when its name is the release Scryer
/// grabbed. Clients sanitise names differently (SABnzbd and NZBGet both strip
/// or replace characters), so the comparison ignores everything but
/// alphanumerics.
fn payload_directory_name_matches_release(name: &str, source_title: &str) -> bool {
    if name == source_title {
        return true;
    }
    let normalize = |value: &str| {
        value
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|ch| ch.to_ascii_lowercase())
            .collect::<String>()
    };
    let title = normalize(source_title);
    !title.is_empty() && normalize(name) == title
}

fn unix_nanos(time: Option<std::time::SystemTime>) -> Option<i64> {
    let time = time?;
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_nanos()).ok(),
        Err(error) => i64::try_from(error.duration().as_nanos())
            .ok()
            .map(|nanos| -nanos),
    }
}

#[cfg(unix)]
fn file_device_and_inode(metadata: &std::fs::Metadata) -> (Option<u64>, Option<u64>) {
    use std::os::unix::fs::MetadataExt;
    (Some(metadata.dev()), Some(metadata.ino()))
}

#[cfg(not(unix))]
fn file_device_and_inode(_metadata: &std::fs::Metadata) -> (Option<u64>, Option<u64>) {
    (None, None)
}

fn sample_file_proof(path: &std::path::Path, len: u64) -> std::io::Result<(u64, String)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let head = len.min(PAYLOAD_SAMPLE_BYTES);
    let mut buffer = vec![0u8; head as usize];
    file.read_exact(&mut buffer)?;
    hasher.update(&buffer);
    let mut sampled = head;
    if len > 2 * PAYLOAD_SAMPLE_BYTES {
        file.seek(SeekFrom::End(-(PAYLOAD_SAMPLE_BYTES as i64)))?;
        let mut tail = vec![0u8; PAYLOAD_SAMPLE_BYTES as usize];
        file.read_exact(&mut tail)?;
        hasher.update(&tail);
        sampled += PAYLOAD_SAMPLE_BYTES;
    } else if len > head {
        let mut rest = Vec::new();
        file.read_to_end(&mut rest)?;
        hasher.update(&rest);
        sampled = len;
    }
    Ok((sampled, hasher.finalize().to_hex().to_string()))
}

pub(crate) fn inventory_payload_directory_blocking(target: &std::path::Path) -> AppResult<PayloadInventory> {
    let mut inventory = PayloadInventory {
        target: target.to_string_lossy().into_owned(),
        ..PayloadInventory::default()
    };
    let mut stack = vec![(target.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    let io_error = |what: &str, path: &std::path::Path, error: std::io::Error| {
        AppError::Repository(format!("failed to {what} {}: {error}", path.display()))
    };
    while let Some((directory, depth)) = stack.pop() {
        let entries = std::fs::read_dir(&directory)
            .map_err(|error| io_error("inventory download payload directory", &directory, error))?;
        for entry in entries {
            let entry = entry
                .map_err(|error| io_error("read download payload directory", &directory, error))?;
            seen += 1;
            if seen > PAYLOAD_INVENTORY_MAX_ENTRIES {
                return Err(AppError::Validation(format!(
                    "download payload directory {} has too many entries to inventory safely",
                    target.display()
                )));
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(target)
                .map_err(|_| {
                    AppError::Repository(format!(
                        "download payload entry {} escaped {}",
                        path.display(),
                        target.display()
                    ))
                })?
                .to_string_lossy()
                .into_owned();
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| io_error("inspect download payload entry", &path, error))?;
            if metadata.file_type().is_symlink() {
                inventory.retained.push(relative);
                continue;
            }
            if metadata.is_dir() {
                if depth + 1 > PAYLOAD_INVENTORY_MAX_DEPTH {
                    inventory.retained.push(relative);
                    continue;
                }
                inventory.directories.push(relative);
                stack.push((path, depth + 1));
                continue;
            }
            if !metadata.is_file() || !is_payload_class_file(&path) {
                inventory.retained.push(relative);
                continue;
            }
            let (sample_bytes, sample_blake3) = sample_file_proof(&path, metadata.len())
                .map_err(|error| io_error("sample download payload file", &path, error))?;
            let (dev, ino) = file_device_and_inode(&metadata);
            inventory.entries.push(PayloadInventoryEntry {
                path: relative,
                len: metadata.len(),
                modified_unix_nanos: unix_nanos(metadata.modified().ok()),
                dev,
                ino,
                sample_bytes,
                sample_blake3,
            });
        }
    }
    Ok(inventory)
}

fn relative_payload_path_is_safe(relative: &str) -> bool {
    let path = std::path::Path::new(relative);
    !relative.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

/// Re-checks, immediately before one file is deleted, that the path still
/// lands where the inventory saw it: every directory between the job
/// directory and the file is a real directory (not a symlink planted since),
/// and the file's real parent is still inside the job directory and outside
/// every library and recycle root. An inode-and-hash match alone would follow
/// a moved directory into a library.
fn payload_file_location_is_verified(
    target: &std::path::Path,
    canonical_target: &std::path::Path,
    path: &std::path::Path,
    protected_roots: &[String],
) -> bool {
    let Ok(relative) = path.strip_prefix(target) else {
        return false;
    };
    let components: Vec<_> = relative.components().collect();
    let Some((_, ancestors)) = components.split_last() else {
        return false;
    };
    let mut cursor = target.to_path_buf();
    for component in ancestors {
        cursor.push(component);
        match std::fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            _ => return false,
        }
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    let Ok(canonical_parent) = std::fs::canonicalize(parent) else {
        return false;
    };
    if !canonical_parent.starts_with(canonical_target) {
        return false;
    }
    let canonical_parent = canonical_parent.to_string_lossy();
    !protected_roots.iter().any(|root| {
        crate::catalog_workflow::library_root_paths_overlap(&canonical_parent, root)
    })
}

fn canonical_payload_directory(target: &std::path::Path) -> AppResult<std::path::PathBuf> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(AppError::Validation(format!(
                "download payload directory {} is no longer a real directory",
                target.display()
            )));
        }
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to inspect download payload directory {}: {error}",
                target.display()
            )));
        }
    }
    std::fs::canonicalize(target).map_err(|error| {
        AppError::Repository(format!(
            "failed to resolve download payload directory {}: {error}",
            target.display()
        ))
    })
}

/// Deletes exactly the inventoried files that still match their recorded
/// identity and content sample, then removes directories only once they are
/// empty. Never a recursive delete: an unknown, changed, or symlinked entry
/// keeps itself and every directory above it.
fn remove_inventoried_payload_blocking(
    target: &std::path::Path,
    inventory: &PayloadInventory,
    protected_roots: &[String],
) -> AppResult<PayloadRemovalResult> {
    let canonical_target = canonical_payload_directory(target)?;
    let mut removed = 0usize;
    let mut retained = Vec::new();
    for entry in &inventory.entries {
        if !relative_payload_path_is_safe(&entry.path) {
            return Err(AppError::Validation(format!(
                "download payload inventory entry {:?} is not a safe relative path",
                entry.path
            )));
        }
        let path = target.join(&entry.path);
        if !payload_file_location_is_verified(target, &canonical_target, &path, protected_roots) {
            retained.push(entry.path.clone());
            continue;
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "failed to inspect download payload file {}: {error}",
                    path.display()
                )));
            }
        };
        let (dev, ino) = file_device_and_inode(&metadata);
        let identity_matches = !metadata.file_type().is_symlink()
            && metadata.is_file()
            && metadata.len() == entry.len
            && unix_nanos(metadata.modified().ok()) == entry.modified_unix_nanos
            && dev == entry.dev
            && ino == entry.ino;
        if !identity_matches {
            retained.push(entry.path.clone());
            continue;
        }
        let proof = sample_file_proof(&path, metadata.len()).map_err(|error| {
            AppError::Repository(format!(
                "failed to re-sample download payload file {}: {error}",
                path.display()
            ))
        })?;
        if proof != (entry.sample_bytes, entry.sample_blake3.clone()) {
            retained.push(entry.path.clone());
            continue;
        }
        #[cfg(windows)]
        {
            let mut permissions = metadata.permissions();
            if permissions.readonly() {
                permissions.set_readonly(false);
                let _ = std::fs::set_permissions(&path, permissions);
            }
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "failed to remove download payload file {}: {error}",
                    path.display()
                )));
            }
        }
    }

    // Deepest directories first; a directory that still holds anything simply
    // stays, and so do its ancestors.
    let mut directories: Vec<&String> = inventory
        .directories
        .iter()
        .filter(|directory| relative_payload_path_is_safe(directory))
        .collect();
    directories.sort_by_key(|directory| std::cmp::Reverse(directory.matches(std::path::MAIN_SEPARATOR).count()));
    for directory in directories {
        let _ = std::fs::remove_dir(target.join(directory));
    }
    let directory_removed = match std::fs::remove_dir(target) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    };
    if !directory_removed {
        // Report what is still there, bounded, so the outcome explains itself.
        let mut leftovers = Vec::new();
        let mut stack = vec![target.to_path_buf()];
        while let Some(directory) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if leftovers.len() >= 50 {
                    break;
                }
                let relative = path
                    .strip_prefix(target)
                    .map(|relative| relative.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.to_string_lossy().into_owned());
                match std::fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                        stack.push(path);
                    }
                    _ => leftovers.push(relative),
                }
            }
        }
        for leftover in leftovers {
            if !retained.contains(&leftover) {
                retained.push(leftover);
            }
        }
    }
    Ok(PayloadRemovalResult {
        removed,
        retained,
        directory_removed,
    })
}

/// Checkpoint flag set once a client with native data deletion (Weaver)
/// refused to delete the payload: from then on the host deletes it and the
/// client only removes the entry, on every later attempt of the same row.
const CLIENT_REFUSED_PAYLOAD_DELETION_KEY: &str = "client_refused_payload_deletion";

fn client_refused_payload_deletion(checkpoint: Option<&serde_json::Value>) -> bool {
    checkpoint.is_some_and(|checkpoint| {
        checkpoint
            .get(CLIENT_REFUSED_PAYLOAD_DELETION_KEY)
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    })
}

/// Host-side payload deletion for clients that cannot delete their own data
/// (NZBGet, entry-only torrent plugins) or whose deletion Scryer mirrors
/// (SABnzbd). Deletion is inventory-and-verify, never a recursive wipe:
///
/// 1. The job directory must be provably this job's (named for the release
///    Scryer grabbed), or Scryer's own completed import record must place an
///    imported source file inside it (in which case only those imported files
///    are removed, nothing else).
/// 2. Its contents are inventoried (identity + content sample) and persisted on
///    the cleanup row before anything is deleted, restricted to payload classes.
/// 3. Each inventoried file is re-verified against that record immediately
///    before its own `remove_file`; anything else stays, and directories are
///    only removed once empty.
///
/// Operator path mappings are still honoured for remote-to-local translation
/// and, when a mapped root contains the payload, for the root-availability and
/// symlink-ancestor checks; they are no longer required to exist. Library and
/// recycle roots are refused outright.
async fn remove_host_payload_before_entry_cleanup(
    app: &AppUseCase,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    record: Option<&crate::DownloadCleanupRecord>,
) -> AppResult<HostPayloadCleanupReport> {
    let client_id = client_id.trim();
    if client_id.is_empty() {
        return Err(AppError::Validation(
            "download client payload cleanup requires a configured client id".to_string(),
        ));
    }

    let saved = record
        .and_then(|record| record.payload_checkpoint.as_deref())
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()
        .map_err(|error| AppError::Repository(format!("invalid payload checkpoint: {error}")))?;
    if let Some(saved) = saved.as_ref()
        && saved
            .get("payload_removed")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    {
        tracing::info!(
            client_id,
            client_type,
            download_client_item_id,
            "payload deletion already checkpointed; retrying entry operation only"
        );
        return Ok(HostPayloadCleanupReport {
            disposition: saved
                .get("disposition")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or(HostPayloadDisposition::Removed),
            filesystem_checkpoint: saved
                .get("filesystem_checkpoint")
                .and_then(serde_json::Value::as_str)
                .map(std::path::PathBuf::from),
        });
    }

    let mut completed = if let Some(completed) = saved
        .as_ref()
        .and_then(|checkpoint| checkpoint.get("completed").cloned())
        .and_then(|completed| {
            serde_json::from_value::<scryer_domain::CompletedDownload>(completed).ok()
        }) {
        completed
    } else {
        app
        .services
        .integrations
        .download_client
        .get_completed_download_for_source(client_id, client_type, download_client_item_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "download client completed download {download_client_item_id} is unavailable for payload cleanup"
            ))
        })?
    };
    let authoritative_completed = completed.clone();
    let config = app
        .services
        .integrations
        .download_client_configs
        .get_by_id(client_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("download client {client_id}")))?;
    let mappings = parse_download_client_remote_path_mappings(&config.config_json)?;
    apply_remote_path_mappings_to_completed_download(&mut completed, &mappings);

    // Mapped roots translate paths and, when one contains the payload, add the
    // root-availability and symlink-ancestor checks. Deletion authority itself
    // comes from the per-file verification below, not from the roots.
    let roots = mappings
        .iter()
        .map(|mapping| {
            let root = mapping.local_root();
            let root = std::path::PathBuf::from(root.trim());
            if !root.is_absolute()
                || root == std::path::Path::new("/")
                || root
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(AppError::Validation(format!(
                    "download client path mapping has an unusable local root: {}",
                    root.display()
                )));
            }
            Ok(root)
        })
        .collect::<AppResult<Vec<_>>>()?;

    let target = std::path::PathBuf::from(completed.dest_dir.trim());
    if !target.is_absolute() {
        return Err(AppError::Validation(format!(
            "download client payload path must be absolute: {}",
            target.display()
        )));
    }
    if target
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
        || target.parent().is_none()
    {
        return Err(AppError::Validation(format!(
            "download client payload path must not be a filesystem root or contain traversal: {}",
            target.display()
        )));
    }
    if roots.contains(&target) {
        return Err(AppError::Validation(format!(
            "refusing to delete download client output root {}",
            target.display()
        )));
    }
    let containing_root = crate::fs_safety::most_specific_containing_root(&target, &roots);
    if let Some(root) = containing_root.as_deref() {
        crate::fs_safety::resolve_available_root_for_path(&target, &roots)?;
        ensure_payload_target_parents_are_not_symlinks(root, &target).await?;
    }

    let libraries = app.services.catalog.libraries.list(None).await?;
    let library_roots =
        crate::catalog_workflow::library_root_folders_from_libraries(&libraries, None);
    // Protection must fail closed when the configured recycle path cannot be
    // read. Display-oriented recycle configuration intentionally uses defaults
    // on read errors and is not suitable as deletion authority.
    let custom_recycle = app
        .read_setting_string_value_for_scope(
            crate::settings::keys::SETTINGS_SCOPE_MEDIA,
            crate::settings::keys::RECYCLE_BIN_PATH_KEY,
            None,
        )
        .await?;
    let mut recycle_roots: Vec<std::path::PathBuf> = library_roots
        .iter()
        .map(|root| std::path::Path::new(&root.path).join(".scryer-recycle"))
        .collect();
    if let Some(path) = custom_recycle
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        recycle_roots.push(path.into());
    }
    if library_roots.is_empty() {
        recycle_roots.push("/tmp/.scryer-recycle".into());
    }
    let protected_roots: Vec<String> = library_roots
        .iter()
        .map(|root| root.path.clone())
        .chain(
            recycle_roots
                .iter()
                .map(|root| root.to_string_lossy().into_owned()),
        )
        .collect();
    // The path as reported and, when it resolves differently, the path as it
    // actually lands on disk: a symlinked job folder must not reach a library.
    let mut protected_candidates = vec![target.to_string_lossy().into_owned()];
    if let Ok(canonical) = tokio::fs::canonicalize(&target).await
        && canonical != target
    {
        protected_candidates.push(canonical.to_string_lossy().into_owned());
    }
    for candidate in &protected_candidates {
        if recycle_roots.iter().any(|root| {
            crate::catalog_workflow::library_root_paths_overlap(candidate, &root.to_string_lossy())
        }) {
            return Err(AppError::Validation("payload overlaps a recycle root".into()));
        }
        if library_roots
            .iter()
            .any(|root| crate::catalog_workflow::library_root_paths_overlap(candidate, &root.path))
        {
            return Err(AppError::Validation(format!(
                "refusing to delete download client payload {} because it overlaps a configured library root",
                target.display()
            )));
        }
    }

    let checkpoint = containing_root.as_deref().map(|root| {
        host_payload_cleanup_checkpoint(
            root,
            client_id,
            client_type,
            download_client_item_id,
            &target,
        )
    });
    if let Some(checkpoint) = checkpoint.as_deref() {
        ensure_host_payload_cleanup_checkpoint(checkpoint).await?;
    }

    // Scryer's own completed import records for this canonical job: the files
    // it actually imported from this payload, with the sizes it verified.
    let mut imported_sources: Vec<(std::path::PathBuf, Option<u64>)> = Vec::new();
    if let Some(download_id) = canonical_download_id {
        let locator =
            crate::ClientJobLocator::new(Some(client_id), client_type, download_client_item_id);
        let imports = app
            .services
            .workflow
            .imports
            .list_imports_for_identities(&[locator])
            .await?;
        for import in imports {
            if import.status != ImportStatus::Completed
                || app
                    .services
                    .workflow
                    .imports
                    .canonical_download_id_for_import(&import.id)
                    .await?
                    != Some(*download_id)
            {
                continue;
            }
            let Some(result) = import
                .result_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            else {
                continue;
            };
            if result.get("decision").and_then(|v| v.as_str()) != Some("imported") {
                continue;
            }
            if let Some(source) = result.get("source_path").and_then(|v| v.as_str()) {
                imported_sources.push((
                    std::path::PathBuf::from(source),
                    result
                        .get("file_size_bytes")
                        .and_then(serde_json::Value::as_i64)
                        .and_then(|size| u64::try_from(size).ok()),
                ));
            }
        }
    }

    // Host deletion failures are counted on the checkpoint, separately from
    // the row's claim count: seeding holds and client outages must not eat
    // the retry budget promised to a temporarily unavailable mount.
    let prior_failures = saved
        .as_ref()
        .and_then(|checkpoint| checkpoint.get("host_payload_failures").cloned());
    let client_refused = client_refused_payload_deletion(saved.as_ref());
    let persist_plan = |mut plan: serde_json::Value| {
        let prior_failures = prior_failures.clone();
        async move {
        if let Some(failures) = prior_failures {
            plan["host_payload_failures"] = failures;
        }
        if client_refused {
            plan[CLIENT_REFUSED_PAYLOAD_DELETION_KEY] = serde_json::json!(true);
        }
        if let Some(record) = record {
            app.services
                .workflow
                .download_submissions
                .checkpoint_download_cleanup_payload(&record.download_id, &plan.to_string())
                .await?;
        }
        AppResult::Ok(())
        }
    };

    let disposition = match tokio::fs::symlink_metadata(&target).await {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
            // A single-file job: the client's path and name are not proof.
            // Require Scryer's persisted import result for this canonical job.
            if canonical_download_id.is_none() {
                return Err(AppError::Validation(
                    "file payload cleanup requires durable job ownership".into(),
                ));
            }
            if !imported_sources
                .iter()
                .any(|(source, _)| source.as_path() == target)
            {
                return Err(AppError::Validation(
                    "file payload ownership is unverified; retaining source and client entry"
                        .into(),
                ));
            }
            persist_plan(serde_json::json!({
                "completed": authoritative_completed, "payload_removed": false,
                "target": target,
            }))
            .await?;
            crate::fs_safety::remove_file_safely_if_exists(&target).await?;
            HostPayloadDisposition::Removed
        }
        Ok(metadata) if metadata.is_dir() => {
            // The release Scryer grabbed, from the durable cleanup row or the
            // submission it was copied from. Legacy jobs whose client-side id
            // is not a canonical Scryer id still have a submission for this
            // exact client/item locator.
            let mut source_title = match record {
                Some(record) => record.source_title.clone(),
                None => match canonical_download_id {
                    Some(id) => app
                        .services
                        .workflow
                        .download_submissions
                        .find_by_canonical_download_id(id)
                        .await?
                        .and_then(|submission| submission.source_title),
                    None => None,
                },
            };
            if source_title.is_none() {
                let locator = crate::ClientJobLocator::new(
                    Some(client_id),
                    client_type,
                    download_client_item_id,
                );
                source_title = app
                    .services
                    .workflow
                    .download_submissions
                    .list_for_client_items(&[locator])
                    .await?
                    .into_iter()
                    .filter_map(|submission| submission.source_title)
                    .next_back();
            }
            let name_matches_release = target
                .file_name()
                .and_then(|name| name.to_str())
                .zip(source_title.as_deref())
                .is_some_and(|(name, title)| payload_directory_name_matches_release(name, title));
            let imported_here: Vec<&(std::path::PathBuf, Option<u64>)> = imported_sources
                .iter()
                .filter(|(source, _)| source.starts_with(&target) && source.as_path() != target)
                .collect();
            if name_matches_release {
                // The release's own job directory: inventory it once, then
                // delete only what re-verifies.
                let inventory = match saved
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.get("inventory").cloned())
                    .and_then(|value| serde_json::from_value::<PayloadInventory>(value).ok())
                    .filter(|inventory| {
                        std::path::Path::new(&inventory.target) == target.as_path()
                    }) {
                    Some(inventory) => inventory,
                    None => {
                        let walk_target = target.clone();
                        let inventory = tokio::task::spawn_blocking(move || {
                            inventory_payload_directory_blocking(&walk_target)
                        })
                        .await
                        .map_err(|error| {
                            AppError::Repository(format!("payload inventory task panicked: {error}"))
                        })??;
                        persist_plan(serde_json::json!({
                            "completed": authoritative_completed, "payload_removed": false,
                            "target": target, "inventory": inventory,
                        }))
                        .await?;
                        inventory
                    }
                };
                let remove_target = target.clone();
                let remove_protected = protected_roots.clone();
                let result = tokio::task::spawn_blocking(move || {
                    remove_inventoried_payload_blocking(&remove_target, &inventory, &remove_protected)
                })
                .await
                .map_err(|error| {
                    AppError::Repository(format!("payload removal task panicked: {error}"))
                })??;
                if result.directory_removed && result.retained.is_empty() {
                    tracing::info!(client_id, client_type, download_client_item_id,
                        path = %target.display(), removed = result.removed,
                        "terminal download payload removed");
                    HostPayloadDisposition::Removed
                } else {
                    tracing::warn!(client_id, client_type, download_client_item_id,
                        path = %target.display(), removed = result.removed,
                        retained = ?result.retained,
                        "terminal download payload partially retained: unrecognised or changed files were left in place");
                    HostPayloadDisposition::PartiallyRetained
                }
            } else if !imported_here.is_empty() {
                // Not provably the job's own directory (a shared completed
                // folder, or a client-renamed duplicate). Remove only the files
                // Scryer itself imported from it, verified by size, and leave
                // the directory and everything else alone.
                persist_plan(serde_json::json!({
                    "completed": authoritative_completed, "payload_removed": false,
                    "target": target,
                    "imported_sources": imported_here.iter().map(|(source, _)| source).collect::<Vec<_>>(),
                }))
                .await?;
                let canonical_target = canonical_payload_directory(&target)?;
                let mut removed = 0usize;
                for (source, size) in imported_here {
                    if !payload_file_location_is_verified(
                        &target,
                        &canonical_target,
                        source,
                        &protected_roots,
                    ) {
                        tracing::warn!(client_id, client_type, download_client_item_id,
                            path = %source.display(),
                            "imported source no longer resolves inside the job directory; retained");
                        continue;
                    }
                    match tokio::fs::symlink_metadata(source).await {
                        Ok(metadata)
                            if metadata.is_file()
                                && !metadata.file_type().is_symlink()
                                && size.is_none_or(|size| size == metadata.len()) =>
                        {
                            crate::fs_safety::remove_file_safely_if_exists(source).await?;
                            removed += 1;
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(AppError::Repository(format!(
                                "failed to inspect imported source {}: {error}",
                                source.display()
                            )));
                        }
                    }
                }
                // Only ever an empty-directory removal; anything left keeps it.
                let directory_removed = tokio::fs::remove_dir(&target).await.is_ok();
                tracing::warn!(client_id, client_type, download_client_item_id,
                    path = %target.display(), removed, directory_removed,
                    "payload directory is not provably this job's; removed only Scryer's own imported files");
                if directory_removed {
                    HostPayloadDisposition::Removed
                } else {
                    HostPayloadDisposition::PartiallyRetained
                }
            } else {
                return Err(AppError::Validation(
                    "payload directory ownership is unverified; refusing shared-directory deletion"
                        .into(),
                ));
            }
        }
        Ok(_) => {
            return Err(AppError::Validation(format!(
                "download client payload path {} is not a file, directory, or symlink",
                target.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HostPayloadDisposition::Removed,
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to inspect download client payload {}: {error}",
                target.display()
            )));
        }
    };
    persist_plan(serde_json::json!({
        "completed": authoritative_completed, "payload_removed": true,
        "target": target, "disposition": disposition,
        "filesystem_checkpoint": checkpoint,
    }))
    .await?;
    Ok(HostPayloadCleanupReport {
        disposition,
        filesystem_checkpoint: checkpoint,
    })
}

fn host_payload_cleanup_checkpoint(
    root: &std::path::Path,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    target: &std::path::Path,
) -> std::path::PathBuf {
    let mut hasher = blake3::Hasher::new();
    for value in [client_id, client_type, download_client_item_id] {
        hasher.update(value.as_bytes());
        hasher.update(&[0]);
    }
    hasher.update(target.to_string_lossy().as_bytes());
    root.join(format!(
        ".scryer-rtorrent-cleanup-{}",
        hasher.finalize().to_hex()
    ))
}

async fn ensure_host_payload_cleanup_checkpoint(checkpoint: &std::path::Path) -> AppResult<()> {
    match tokio::fs::create_dir(checkpoint).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = tokio::fs::symlink_metadata(checkpoint)
                .await
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to inspect download client payload cleanup checkpoint {}: {error}",
                        checkpoint.display()
                    ))
                })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(AppError::Validation(format!(
                    "download client payload cleanup checkpoint {} is not a directory",
                    checkpoint.display()
                )));
            }
            Ok(())
        }
        Err(error) => Err(AppError::Repository(format!(
            "failed to create download client payload cleanup checkpoint {}: {error}",
            checkpoint.display()
        ))),
    }
}

async fn clear_host_payload_cleanup_checkpoint(checkpoint: &std::path::Path) -> AppResult<()> {
    match tokio::fs::remove_dir(checkpoint).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to remove download client payload cleanup checkpoint {}: {error}",
            checkpoint.display()
        ))),
    }
}

async fn ensure_payload_target_parents_are_not_symlinks(
    root: &std::path::Path,
    target: &std::path::Path,
) -> AppResult<()> {
    let relative = target.strip_prefix(root).map_err(|_| {
        AppError::Validation(format!(
            "download client payload path {} is outside output root {}",
            target.display(),
            root.display()
        ))
    })?;
    let mut parent = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            continue;
        };
        let candidate = parent.join(component);
        if candidate == target {
            break;
        }
        match tokio::fs::symlink_metadata(&candidate).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(AppError::Validation(format!(
                    "download client payload path {} traverses symlink {}",
                    target.display(),
                    candidate.display()
                )));
            }
            Ok(_) => parent = candidate,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "failed to inspect download client payload ancestor {}: {error}",
                    candidate.display()
                )));
            }
        }
    }
    Ok(())
}

/// `SeedGoalMetAction::StopSeeding`: leave the entry in the client but stop it
/// uploading.
///
/// Pause is the only stop control the download-client port exposes
/// (`DownloadControlAction::Pause` in the plugin SDK), and for a torrent that
/// has finished downloading, paused *is* stopped seeding. A failed pause
/// remains unresolved so a transient client outage does not discard the action.
async fn stop_seeding_for_terminal_download(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    reason: &'static str,
) -> Option<SeedingReleaseAction> {
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
            Some(SeedingReleaseAction::Paused)
        }
        Err(error) => {
            tracing::warn!(
                client_id,
                client_type,
                download_client_item_id,
                reason,
                error = %error,
                "seeding goal met but pause failed; retaining cleanup for retry"
            );
            None
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
