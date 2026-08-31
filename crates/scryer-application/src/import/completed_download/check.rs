use super::lookup::{
    apply_download_id_state, completed_download_source_identity, download_id_tracked_state,
    find_completed_download, maybe_resolve_title_from_completed_download,
    observed_completed_download_identity, observed_queue_item_identity, queue_item_source_identity,
};
use super::path_state::{CompletedDownloadPathState, evaluate_completed_download_path};
use super::*;

pub async fn check(app: &AppUseCase, td: &mut TrackedDownload) {
    check_with_lookup(app, td, None).await;
}

pub(crate) async fn check_with_lookup(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    completed_lookup: Option<&CompletedDownloadLookup>,
) {
    // Only process if client reports completed.
    if td.client_item.state != DownloadQueueState::Completed {
        return;
    }

    let waiting_for_completed_history_retry = td.state == TrackedDownloadState::ImportPending
        && td.waiting_for_completed_history
        && completed_lookup.is_some();

    // Only process if still in a check-eligible state.
    if td.state != TrackedDownloadState::Downloading
        && td.state != TrackedDownloadState::ImportBlocked
        && !waiting_for_completed_history_retry
    {
        return;
    }
    if waiting_for_completed_history_retry {
        td.state = TrackedDownloadState::Downloading;
    }

    // Don't re-evaluate a post-import block. Import already ran and returned
    // Skipped/Failed — stay blocked until the user explicitly retries. The
    // migrated pre-import deduplication state is the exception: it never ran
    // an import, so reopen it and let verification decide.
    if td.state == TrackedDownloadState::ImportBlocked {
        let persisted_reason =
            crate::tracked_downloads::import_blocked_reason_for_tracked(app, td).await;
        if td.import_attempted {
            if persisted_reason != Some(crate::tracked_downloads::ImportBlockedReason::AfterImport)
            {
                crate::tracked_downloads::persist_import_blocked_state_marker(
                    app,
                    td,
                    crate::tracked_downloads::ImportBlockedReason::AfterImport,
                    td.status_messages.first().map(String::as_str),
                )
                .await;
            }
            return;
        }
        let reopens_for_verification = persisted_reason
            .is_some_and(crate::tracked_downloads::ImportBlockedReason::reopens_for_verification);
        if !reopens_for_verification {
            return;
        }
        tracing::info!(
            id = %td.id,
            "check: reopening migrated unverified already-imported block"
        );
        td.reset_for_import_retry();
        td.state = TrackedDownloadState::Downloading;
    }

    // A blocked download that was explicitly assigned by the user should remain
    // in the manual-import flow until the user queues that import. Assigning a
    // title should not silently convert it back into auto-import.
    if td.state == TrackedDownloadState::ImportBlocked
        && td.match_type == TitleMatchType::Submission
    {
        return;
    }

    if td.state == TrackedDownloadState::ImportBlocked && td.path_missing_since.is_some() {
        return;
    }

    let queue_identity = observed_queue_item_identity(&td.client_item);
    let queue_source_identity = queue_item_source_identity(&td.client_item);
    if let Some(state) =
        download_id_tracked_state(
            app,
            td.canonical_download_id(),
            &queue_identity,
            Some(&queue_source_identity),
        )
        .await
        // `is_import_settled` rather than `is_terminal`: a torrent parked in
        // `ImportedSeeding` is already in the library and must not be offered
        // for import again while it works off its seeding goal.
        && state.is_import_settled()
    {
        apply_download_id_state(td, state);
        return;
    }

    let Some(completed) = find_completed_download(app, td, completed_lookup).await else {
        if completed_lookup.is_some_and(|lookup| !lookup.is_exhaustive()) {
            mark_waiting_for_completed_history(td, completed_lookup);
            return;
        }
        if !crate::download_submission_identity_is_empty(&queue_identity) {
            if missing_completed_history_is_retryable(td, &queue_identity) {
                mark_waiting_for_completed_history(td, completed_lookup);
                return;
            }
            block_tracked_download_identity_for_manual_review(
                app,
                td,
                crate::tracked_downloads::ImportBlockedReason::MissingCompletedHistoryIdentity,
                "completed queue item carried DownloadId but completed history did not contain a matching DownloadId",
            )
            .await;
        }
        return;
    };
    td.waiting_for_completed_history = false;
    // The first tick that sees this download's completion is the transition
    // into whatever the admission gate decides below; later ticks re-evaluate
    // the same completion and must not repeat the breadcrumb.
    let first_completed_sighting = td.completed_source.is_none();
    td.completed_source = Some(completed.clone());

    match app
        .completed_download_admission(
            tracked_download_is_scryer_origin(td),
            &completed,
            td.client_item.category.as_deref(),
        )
        .await
    {
        crate::services::CompletedDownloadAdmission::Admitted => {
            if matches!(
                td.import_hold,
                Some(crate::tracked_downloads::ImportHold::ExternalManager)
            ) {
                td.import_hold = None;
            }
        }
        crate::services::CompletedDownloadAdmission::ExternalManager => {
            td.import_hold = Some(crate::tracked_downloads::ImportHold::ExternalManager);
            return;
        }
        crate::services::CompletedDownloadAdmission::NotAdmitted {
            category,
            admission_snapshot_missing,
        } => {
            // Nothing is persisted, no status message is set, and the queue
            // filter hides the row from every user-facing surface, so a
            // misclassification is observable ONLY as an absence. This single
            // breadcrumb distinguishes "hidden on purpose" from "silently
            // lost" (its absence once cost a full triage cycle).
            if first_completed_sighting {
                tracing::info!(
                    id = %td.id,
                    client_id = %td.client_id,
                    client_type = %td.client_type,
                    category = ?category,
                    is_scryer_origin = td.client_item.is_scryer_origin,
                    match_type = ?td.match_type,
                    admission_snapshot_missing,
                    "check: completed observation is not admitted by download client category; held from import and hidden from download activity"
                );
            }
            return;
        }
    }

    let completed_identity = observed_completed_download_identity(&completed);
    let completed_source_identity = completed_download_source_identity(&completed);
    if let Some(state) = download_id_tracked_state(
        app,
        td.canonical_download_id(),
        &completed_identity,
        Some(&completed_source_identity),
    )
    .await
        && state.is_import_settled()
    {
        apply_download_id_state(td, state);
        return;
    }

    match evaluate_completed_download_path(td, &completed, Utc::now()) {
        CompletedDownloadPathState::Ready => {}
        CompletedDownloadPathState::Retry => {
            return;
        }
        CompletedDownloadPathState::Blocked => {
            tracing::warn!(
                id = %td.id,
                dest_dir = %completed.dest_dir,
                empty_dest_dir = completed.dest_dir.trim().is_empty(),
                grace_minutes = COMPLETED_PATH_GRACE_PERIOD_MINUTES,
                "completed download path remained unavailable after grace window; blocking import until manual retry"
            );
            set_state_to_import_blocked(app, td).await;
            return;
        }
    }

    maybe_resolve_title_from_completed_download(app, td, &completed).await;

    match completed_download_proves_assigned_title(app, td, &completed).await {
        AssignedTitleProof::Proven => {}
        AssignedTitleProof::Unknown => {
            // Infrastructure failed, not the proof. A durable block here would
            // turn a transient title-repo or matcher error into a permanent
            // identity verdict, so leave the download untouched and let the
            // next check tick retry.
            tracing::warn!(
                id = %td.id,
                "completed download identity gate hit an infrastructure error; retrying next tick"
            );
            return;
        }
        AssignedTitleProof::MissingTitle => {
            let detail = "The title assigned to this download at grab time no longer exists in the library. Automatic import is blocked until the download is reassigned or removed.";
            block_tracked_download_identity_for_manual_review(
                app,
                td,
                crate::tracked_downloads::ImportBlockedReason::AssignedTitleMissing,
                detail,
            )
            .await;
            if td.state != TrackedDownloadState::ImportBlocked {
                td.warn(detail);
                set_state_to_import_blocked(app, td).await;
            }
            return;
        }
        AssignedTitleProof::Disproven => {
            let detail = "The completed release name no longer proves the title assigned at grab time. Automatic import is blocked to prevent replacing media for a different series.";
            block_tracked_download_identity_for_manual_review(
                app,
                td,
                crate::tracked_downloads::ImportBlockedReason::CompletedTitleIdentityMismatch,
                detail,
            )
            .await;
            if td.state != TrackedDownloadState::ImportBlocked {
                td.warn(detail);
                set_state_to_import_blocked(app, td).await;
            }
            return;
        }
    }

    // Auto-import safety gating.
    match td.match_type {
        TitleMatchType::Unmatched => {
            if !td
                .status_messages
                .iter()
                .any(|m| m.contains("couldn't be matched"))
            {
                td.status_messages.clear();
                td.warn("Download couldn't be matched to a library title. Assign a title manually or check the download name.");
            }
            set_state_to_import_blocked(app, td).await;
            return;
        }
        TitleMatchType::IdOnly => {
            if has_id_only_conflict(td) {
                if !td.status_messages.iter().any(|m| {
                    m.contains("matched by ID only") || m.contains(ID_ONLY_CONFLICT_MESSAGE)
                }) {
                    td.status_messages.clear();
                    td.warn(
                        "Download was matched to a title by ID only. Manual confirmation required to import.",
                    );
                }
                set_state_to_import_blocked(app, td).await;
                return;
            }
        }
        TitleMatchType::Submission
        | TitleMatchType::ClientParameter
        | TitleMatchType::TitleParse => {
            // High-confidence matches — proceed.
        }
    }

    // Check that the resolved title still exists.
    // (This is a sync check against cached data; the actual title lookup
    //  was done during resolve_title. If the title was deleted since then,
    //  title_id will still be set but import will fail gracefully.)

    if td.title_id.is_none() || td.title_id.as_deref() == Some("") {
        td.warn("No title linked to this download.");
        set_state_to_import_blocked(app, td).await;
        return;
    }

    if !completed_download_allows_automatic_import(app, td, &completed).await {
        td.status_messages.clear();
        td.warn(
            "Download category is known to Scryer but does not match this title's active route. Confirm the mapping with Manual Import.",
        );
        set_state_to_import_blocked(app, td).await;
        return;
    }

    // All checks passed — queue for import.
    tracing::info!(
        id = %td.id,
        title_id = ?td.title_id,
        match_type = ?td.match_type,
        "check: transitioning to ImportPending"
    );
    td.state = TrackedDownloadState::ImportPending;
    td.waiting_for_completed_history = false;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();
}

/// Whether the tracked download is a Scryer grab (queue item enriched from a
/// Scryer-origin submission row or Scryer's own client parameters).
pub(super) fn tracked_download_is_scryer_origin(td: &TrackedDownload) -> bool {
    td.client_item.is_scryer_origin
}

fn observed_download_category<'a>(
    td: &'a TrackedDownload,
    completed: &'a CompletedDownload,
) -> Option<&'a str> {
    completed
        .category
        .as_deref()
        .or(td.client_item.category.as_deref())
        .map(str::trim)
        .filter(|category| !category.is_empty())
}

async fn completed_download_allows_automatic_import(
    app: &AppUseCase,
    td: &TrackedDownload,
    completed: &CompletedDownload,
) -> bool {
    if tracked_download_is_scryer_origin(td) {
        return true;
    }
    let Some(category) = observed_download_category(td, completed) else {
        return false;
    };
    let Some(title_id) = td.title_id.as_deref() else {
        return false;
    };
    let title = match app.services.catalog.titles.get_by_id(title_id).await {
        Ok(Some(title)) => title,
        Ok(None) => return false,
        Err(error) => {
            tracing::warn!(title_id, error = %error, "could not load title for category admission");
            return false;
        }
    };
    match app
        .effective_download_client_category_for_title(&title, &td.client_id)
        .await
    {
        Ok(Some(expected)) => {
            crate::services::normalize_download_client_category(category)
                == crate::services::normalize_download_client_category(&expected)
        }
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(
                title_id,
                client_id = td.client_id,
                error = %error,
                "could not resolve effective category for automatic import"
            );
            false
        }
    }
}

fn mark_waiting_for_completed_history(
    td: &mut TrackedDownload,
    completed_lookup: Option<&CompletedDownloadLookup>,
) {
    tracing::warn!(
        id = %td.id,
        item_id = %td.client_item.download_client_item_id,
        download_id = ?td.client_item.download_id,
        match_type = ?td.match_type,
        is_scryer_origin = td.client_item.is_scryer_origin,
        lookup_exhaustive = completed_lookup.is_some_and(CompletedDownloadLookup::is_exhaustive),
        "check: completed download not found in client history, will retry"
    );
    td.state = TrackedDownloadState::ImportPending;
    td.waiting_for_completed_history = true;
    td.status = TrackedDownloadStatus::Warning;
    td.status_messages = vec![
        "Completed download is waiting for client history to expose a matching item; retrying."
            .to_string(),
    ];
}

fn missing_completed_history_is_retryable(
    td: &TrackedDownload,
    identity: &crate::DownloadSubmissionIdentity,
) -> bool {
    durable_global_download_id(identity)
        && (td.client_item.is_scryer_origin
            || matches!(
                td.match_type,
                TitleMatchType::Submission | TitleMatchType::ClientParameter
            ))
}

fn durable_global_download_id(identity: &crate::DownloadSubmissionIdentity) -> bool {
    let Some(download_id) = identity
        .download_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    download_id.starts_with("scryer-download:")
        || (matches!(download_id.len(), 40 | 64)
            && download_id.chars().all(|ch| ch.is_ascii_hexdigit()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AssignedTitleProof {
    /// The assignment stands — a durable Scryer identity or explicit
    /// assignment whose title still exists, or a completion-time raw name that
    /// proved a parse match; import may proceed.
    Proven,
    /// No title is assigned, or a parse-matched observation's completion
    /// contradicts it / no raw name proves it — block for manual review.
    Disproven,
    /// The assigned title no longer exists — block under its own reason.
    MissingTitle,
    /// Infrastructure error while proving — retry next tick, never block.
    Unknown,
}

pub(super) async fn completed_download_proves_assigned_title(
    app: &AppUseCase,
    td: &TrackedDownload,
    completed: &CompletedDownload,
) -> AssignedTitleProof {
    // TitleParse is an observation and must be re-proven from completion-time
    // canonical evidence. Unmatched and IdOnly downloads have their own
    // existing manual-review gates below this identity check.
    if !matches!(
        td.match_type,
        TitleMatchType::TitleParse | TitleMatchType::Submission | TitleMatchType::ClientParameter
    ) && !td.client_item.is_scryer_origin
    {
        return AssignedTitleProof::Proven;
    }

    let Some(title_id) = td
        .title_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return AssignedTitleProof::Disproven;
    };
    let title = match app.services.catalog.titles.get_by_id(title_id).await {
        Ok(Some(title)) => title,
        Ok(None) => return AssignedTitleProof::MissingTitle,
        Err(error) => {
            tracing::warn!(
                title_id,
                error = %error,
                "completed download identity gate could not load assigned title"
            );
            return AssignedTitleProof::Unknown;
        }
    };
    // A Scryer submission is a durable grab-time identity, and a Submission or
    // ClientParameter match is an explicit trusted assignment (an operator
    // assignment deliberately shares the Submission label). Once the assigned
    // title is known to still exist, that assignment stands as proof: the
    // downloader's current display label and raw artifact name cannot
    // disprove it, so no completion-name check runs for it. Only a TitleParse
    // observation continues below and must be re-proven from completion-time
    // evidence.
    if matches!(
        td.match_type,
        TitleMatchType::Submission | TitleMatchType::ClientParameter
    ) || td.client_item.is_scryer_origin
    {
        return AssignedTitleProof::Proven;
    }
    let matcher = match app.monitored_title_matcher().await {
        Ok(matcher) => matcher,
        Err(error) => {
            tracing::warn!(
                title_id,
                error = %error,
                "completed download identity gate could not load title matcher"
            );
            return AssignedTitleProof::Unknown;
        }
    };
    let mut evidence = crate::acquisition_release_search::canonical_title_evidence(&title);
    evidence.ambiguity =
        crate::acquisition_release_search::TitleIdentityAmbiguity::from_shared_keys(
            matcher.shared_lookup_keys(title_id, &evidence.lookup_keys),
        );

    // A series movie is searched and grabbed under the *movie's* identity —
    // `series_movie_search_title` swaps in the movie's name, facet, year and
    // ids — so the gate must accept proof against that same identity.
    // Validating a linked movie's release against the parent series alone
    // would let the series' year veto every legitimately grabbed series movie.
    let mut proof_subjects = vec![(title.clone(), evidence)];
    match app
        .services
        .catalog
        .shows
        .list_series_movie_links_for_title(title_id)
        .await
    {
        Ok(links) => {
            for link in links {
                let link_title =
                    crate::acquisition_release_search::series_movie_search_title(&title, &link);
                let mut link_evidence =
                    crate::acquisition_release_search::canonical_title_evidence(&link_title);
                link_evidence.ambiguity =
                    crate::acquisition_release_search::TitleIdentityAmbiguity::from_shared_keys(
                        matcher.shared_lookup_keys(title_id, &link_evidence.lookup_keys),
                    );
                proof_subjects.push((link_title, link_evidence));
            }
        }
        Err(error) => {
            tracing::warn!(
                title_id,
                error = %error,
                "completed download identity gate could not load series movie links"
            );
            return AssignedTitleProof::Unknown;
        }
    }

    let proves_assigned_title = |raw_title: &str| -> bool {
        proof_subjects.iter().any(|(subject_title, evidence)| {
            let parsed =
                crate::parse_release_metadata_for_target(raw_title, &evidence.parse_context);
            let Some(evidence_match) =
                crate::acquisition_release_search::match_parsed_release_to_title_evidence(
                    &parsed, evidence,
                )
            else {
                return false;
            };
            let external_id_matches = parsed
                .imdb_id
                .as_deref()
                .zip(
                    crate::acquisition_search_queries::imdb_id_from_title(subject_title).as_deref(),
                )
                .is_some_and(|(observed, expected)| observed.eq_ignore_ascii_case(expected))
                || parsed
                    .tmdb_id
                    .as_deref()
                    .zip(
                        crate::acquisition_search_queries::tmdb_id_from_external_ids(
                            &subject_title.external_ids,
                        )
                        .as_deref(),
                    )
                    .is_some_and(|(observed, expected)| observed == expected)
                || parsed
                    .tvdb_id
                    .as_deref()
                    .zip(
                        crate::acquisition_search_queries::tvdb_id_from_external_ids(
                            &subject_title.external_ids,
                        )
                        .as_deref(),
                    )
                    .is_some_and(|(observed, expected)| observed == expected);

            if evidence_match.requires_external_id && !external_id_matches {
                return false;
            }
            if evidence.ambiguity.requires_disambiguator()
                && !evidence_match.year_corroborated
                && !external_id_matches
                && !evidence
                    .ambiguity
                    .key_is_unique_to_title(&evidence_match.matched_key)
            {
                return false;
            }
            true
        })
    };

    // The client-reported release name, else the media file names (non-sample,
    // largest first) — the same claims the completion-time re-resolution used.
    let completion_sources = crate::import_workflow::completed_download_release_claims(completed);

    // For a parse-matched observation, what actually finished on disk outranks
    // the provisional match: a completion name that positively asserts a
    // *different* library title's identity — and not the assigned one — is a
    // contradiction, not obfuscation, and disproves the assignment outright.
    let completion_contradicts_assignment = completion_sources.iter().any(|raw_title| {
        let anchor_keys =
            crate::acquisition_release_search::context_free_identity_anchor_keys(raw_title);
        matcher.keys_name_another_title(title_id, &anchor_keys)
            && !proof_subjects.iter().any(|(_, evidence)| {
                anchor_keys.iter().any(|anchor_key| {
                    crate::acquisition_release_search::evidence_key_for_normalized(
                        evidence, anchor_key,
                    )
                    .is_some()
                })
            })
    });
    if completion_contradicts_assignment {
        return AssignedTitleProof::Disproven;
    }

    // Otherwise a completion-time name (the client-reported release name, or
    // the media file names when the client exposes none) must positively
    // prove the parse-matched title; every raw name failing is a disproof.
    for raw_title in &completion_sources {
        if proves_assigned_title(raw_title) {
            return AssignedTitleProof::Proven;
        }
    }

    AssignedTitleProof::Disproven
}

pub(super) fn has_id_only_conflict(td: &TrackedDownload) -> bool {
    td.status_messages
        .iter()
        .any(|message| message == ID_ONLY_CONFLICT_MESSAGE)
}

async fn set_state_to_import_blocked(app: &AppUseCase, td: &mut TrackedDownload) {
    set_state_to_import_blocked_with_reason(
        app,
        td,
        crate::tracked_downloads::ImportBlockedReason::PreImport,
    )
    .await;
}

async fn set_state_to_import_blocked_with_reason(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    reason: crate::tracked_downloads::ImportBlockedReason,
) {
    td.state = TrackedDownloadState::ImportBlocked;
    td.waiting_for_completed_history = false;
    td.status = TrackedDownloadStatus::Warning;

    crate::tracked_downloads::persist_import_blocked_state_marker(
        app,
        td,
        reason,
        td.status_messages.first().map(String::as_str),
    )
    .await;

    if td.notified_manual_interaction {
        return;
    }

    td.notified_manual_interaction = true;
    let message = td
        .status_messages
        .first()
        .cloned()
        .unwrap_or_else(|| "Manual interaction required for this download.".to_string());

    let event = match td.title_id.as_ref() {
        Some(title_id) => match app.services.catalog.titles.get_by_id(title_id).await {
            Ok(Some(title)) => new_title_domain_event(
                None,
                &title,
                scryer_domain::DomainEventPayload::ImportRejected(ImportRejectedEventData {
                    title: Some(title_context_snapshot(&title)),
                    status: ImportStatus::Skipped,
                    import_id: None,
                    source_system: Some(td.client_type.clone()),
                    source_ref: Some(td.client_item.download_client_item_id.clone()),
                    source_title: td
                        .source_title
                        .clone()
                        .or_else(|| Some(td.client_item.title_name.clone())),
                    source_path: None,
                    dest_path: None,
                    quality: None,
                    reason: Some(message.clone()),
                    skip_reason: None,
                    episode_ids: Vec::new(),
                }),
            ),
            _ => new_global_domain_event(
                None,
                scryer_domain::DomainEventPayload::ImportRejected(ImportRejectedEventData {
                    title: None,
                    status: ImportStatus::Skipped,
                    import_id: None,
                    source_system: Some(td.client_type.clone()),
                    source_ref: Some(td.client_item.download_client_item_id.clone()),
                    source_title: td
                        .source_title
                        .clone()
                        .or_else(|| Some(td.client_item.title_name.clone())),
                    source_path: None,
                    dest_path: None,
                    quality: None,
                    reason: Some(message.clone()),
                    skip_reason: None,
                    episode_ids: Vec::new(),
                }),
            ),
        },
        None => new_global_domain_event(
            None,
            scryer_domain::DomainEventPayload::ImportRejected(ImportRejectedEventData {
                title: None,
                status: ImportStatus::Skipped,
                import_id: None,
                source_system: Some(td.client_type.clone()),
                source_ref: Some(td.client_item.download_client_item_id.clone()),
                source_title: td
                    .source_title
                    .clone()
                    .or_else(|| Some(td.client_item.title_name.clone())),
                source_path: None,
                dest_path: None,
                quality: None,
                reason: Some(message),
                skip_reason: None,
                episode_ids: Vec::new(),
            }),
        ),
    };

    let _ = app.append_domain_event(event).await;
}

async fn block_tracked_download_identity_for_manual_review(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    reason: crate::tracked_downloads::ImportBlockedReason,
    detail: &str,
) {
    let observed_identity = observed_queue_item_identity(&td.client_item);
    if crate::download_submission_identity_is_empty(&observed_identity) {
        return;
    }
    if !td.status_messages.iter().any(|message| message == detail) {
        td.status_messages.clear();
        td.status_messages.push(detail.to_string());
    }
    set_state_to_import_blocked_with_reason(app, td, reason).await;
}
