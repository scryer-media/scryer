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
    // - A client-reported `Failed` never enters the gate — no private rail, no
    //   `never_remove`, no HandOff — so the client's own `can_remove` is the
    //   only rail there is. A torrent it will not release (or cannot answer
    //   for) retains both its entry and data for a later attempt.
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

    if state == TrackedDownloadState::Failed
        && is_torrent
        && failure_origin == TerminalFailureOrigin::ClientFailure
        && observed_can_remove != Some(true)
    {
        return TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::RetryableFailure);
    }
    let host_managed_payload = remove_data
        && client_type.trim() != "weaver"
        && !plugin_has_native_data_removal(app, client_id).await;
    let payload_cleanup_checkpoint = if host_managed_payload {
        match remove_host_payload_before_entry_cleanup(
            app,
            client_id,
            client_type,
            download_client_item_id,
            record,
        )
        .await
        {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                tracing::warn!(
                    client_id,
                    client_type,
                    download_client_item_id,
                    state = state.as_str(),
                    error = %error,
                    "failed to remove download client payload before removing the client entry"
                );
                let seeding = seeding_report.map(|report| SeedingGateReport {
                    action: Some(SeedingReleaseAction::Kept),
                    ..report
                });
                return TerminalDownloadCleanup {
                    outcome: TerminalDownloadCleanupOutcome::RetryableFailure,
                    seeding,
                };
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
    TerminalDownloadCleanup { outcome, seeding }
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

async fn remove_host_payload_before_entry_cleanup(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    record: Option<&crate::DownloadCleanupRecord>,
) -> AppResult<Option<std::path::PathBuf>> {
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
        return Ok(saved
            .get("filesystem_checkpoint")
            .and_then(serde_json::Value::as_str)
            .map(std::path::PathBuf::from));
    }

    let mut completed = if let Some(completed) = record
        .and_then(|record| record.payload_checkpoint.as_deref())
        .and_then(|checkpoint| serde_json::from_str::<serde_json::Value>(checkpoint).ok())
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

    let status = app
        .services
        .integrations
        .download_client
        .get_client_status_for_client_id(client_id)
        .await?;
    let roots = status
        .remote_output_roots
        .iter()
        .map(|root| {
            let root = std::path::PathBuf::from(root.trim());
            if !root.is_absolute()
                || root == std::path::Path::new("/")
                || root
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(AppError::Validation(format!(
                    "download client reported an unusable output root: {}",
                    root.display()
                )));
            }
            Ok(root)
        })
        .collect::<AppResult<Vec<_>>>()?;
    if roots.is_empty() {
        return Err(AppError::Validation(
            "download client did not report an absolute local output root for payload cleanup"
                .to_string(),
        ));
    }

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
    {
        return Err(AppError::Validation(format!(
            "download client payload path must not contain traversal: {}",
            target.display()
        )));
    }
    let containing_root = crate::fs_safety::most_specific_containing_root(&target, &roots)
        .ok_or_else(|| {
            AppError::Validation(format!(
                "download client payload path {} is outside configured output roots",
                target.display()
            ))
        })?;
    if target == containing_root {
        return Err(AppError::Validation(format!(
            "refusing to delete download client output root {}",
            target.display()
        )));
    }
    crate::fs_safety::resolve_available_root_for_path(&target, &roots)?;
    ensure_payload_target_parents_are_not_symlinks(&containing_root, &target).await?;

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
    if recycle_roots.iter().any(|root| {
        crate::catalog_workflow::library_root_paths_overlap(
            &completed.dest_dir,
            &root.to_string_lossy(),
        )
    }) {
        return Err(AppError::Validation(
            "payload overlaps a recycle root".into(),
        ));
    }
    if crate::catalog_workflow::library_root_folders_from_libraries(&libraries, None)
        .iter()
        .any(|root| {
            crate::catalog_workflow::library_root_paths_overlap(&completed.dest_dir, &root.path)
        })
    {
        return Err(AppError::Validation(format!(
            "refusing to delete download client payload {} because it overlaps a configured library root",
            target.display()
        )));
    }

    let checkpoint = host_payload_cleanup_checkpoint(
        &containing_root,
        client_id,
        client_type,
        download_client_item_id,
        &target,
    );
    ensure_host_payload_cleanup_checkpoint(&checkpoint).await?;

    if let Some(record) = record {
        let plan = serde_json::json!({
            "completed": authoritative_completed, "payload_removed": false
        });
        app.services
            .workflow
            .download_submissions
            .checkpoint_download_cleanup_payload(&record.download_id, &plan.to_string())
            .await?;
    }
    match tokio::fs::symlink_metadata(&target).await {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
            crate::fs_safety::remove_file_safely_if_exists(&target).await
        }
        Ok(metadata) if metadata.is_dir() => {
            let owned_name = target
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name == completed.name || completed.release_name.as_deref() == Some(name)
                });
            if !owned_name {
                return Err(AppError::Validation(
                    "payload directory ownership is unverified; refusing shared-directory deletion"
                        .into(),
                ));
            }
            crate::fs_safety::remove_dir_all_safely_if_exists(&target).await
        }
        Ok(_) => Err(AppError::Validation(format!(
            "download client payload path {} is not a file, directory, or symlink",
            target.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to inspect download client payload {}: {error}",
            target.display()
        ))),
    }?;
    if let Some(record) = record {
        let plan = serde_json::json!({
            "completed": authoritative_completed, "payload_removed": true,
            "filesystem_checkpoint": checkpoint
        });
        app.services
            .workflow
            .download_submissions
            .checkpoint_download_cleanup_payload(&record.download_id, &plan.to_string())
            .await?;
    }
    tracing::info!(client_id, client_type, download_client_item_id, path = %target.display(),
        "terminal download payload removed");
    Ok(Some(checkpoint))
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

    let guard = file_result.source_cleanup.clone().ok_or_else(|| {
        AppError::Repository(format!(
            "move import did not return a source cleanup guard for {}",
            file_result.source_path.display()
        ))
    })?;

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
