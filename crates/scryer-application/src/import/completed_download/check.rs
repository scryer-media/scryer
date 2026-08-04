use super::lookup::{
    apply_download_id_state, completed_download_source_identity, download_id_tracked_state,
    find_completed_download, maybe_resolve_title_from_completed_download,
    observed_completed_download_identity, observed_queue_item_identity, queue_item_source_identity,
};
use super::path_state::{CompletedDownloadPathState, evaluate_completed_download_path};
use super::*;
use crate::tracked_downloads::ForeignDownloadClassification;

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
    // Skipped/Failed — stay blocked until the user explicitly retries.
    if td.state == TrackedDownloadState::ImportBlocked && td.import_attempted {
        return;
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

    if matches!(
        td.foreign_import_classification,
        Some(ForeignDownloadClassification::DroneParameter)
    ) {
        return;
    }

    let queue_identity = observed_queue_item_identity(&td.client_item);
    let queue_source_identity = queue_item_source_identity(&td.client_item);
    if let Some(state) =
        download_id_tracked_state(app, &queue_identity, Some(&queue_source_identity)).await
        && state.is_terminal()
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
                "missing_completed_history_identity",
                "completed queue item carried DownloadId but completed history did not contain a matching DownloadId",
            )
            .await;
        }
        return;
    };
    td.waiting_for_completed_history = false;

    let completed_identity = observed_completed_download_identity(&completed);
    let completed_source_identity = completed_download_source_identity(&completed);
    if let Some(state) =
        download_id_tracked_state(app, &completed_identity, Some(&completed_source_identity)).await
        && state.is_terminal()
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

    if let Some(classification) = classify_foreign_completed_download(app, td, &completed).await {
        mark_foreign_download(td, classification);
        return;
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
                "assigned_title_missing",
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
                "completed_title_identity_mismatch",
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
            // Match Sonarr/Radarr's conservative handling for risky ID-only
            // matches: interactive/Scryer-origin grabs may continue, but
            // foreign downloads that only resolved through embedded IDs still
            // need manual confirmation before import.
            if !td.client_item.is_scryer_origin || has_id_only_conflict(td) {
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
        td.warn(FOREIGN_CATEGORY_BLOCKED_MESSAGE);
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

async fn classify_foreign_completed_download(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    completed: &CompletedDownload,
) -> Option<ForeignDownloadClassification> {
    // Category ownership is configuration-derived, so a previous category
    // classification must be reconsidered whenever this completed item is
    // checked again.
    if matches!(
        td.foreign_import_classification,
        Some(
            ForeignDownloadClassification::ForeignCategory
                | ForeignDownloadClassification::NoImportableVideo
        )
    ) {
        td.foreign_import_classification = None;
    }

    // A download Scryer submitted is definitionally not another app's, so none
    // of the foreign signals below may be applied to it.
    //
    // Without this guard the category branch runs for Scryer's own grabs: if the
    // observed category does not survive the owned-categories round-trip the item
    // is classified ForeignCategory, short-circuited before title resolution, and
    // silently never imported — no import row, no identity row, no status
    // message. That is how a whole class of auto-grabbed series imports
    // disappeared while their releases logged `decision="eligible"`.
    //
    // The trusted set mirrors completed_download_allows_automatic_import so the
    // two gates cannot disagree about what counts as Scryer's own work.
    if td.client_item.is_scryer_origin
        || matches!(
            td.match_type,
            TitleMatchType::Submission | TitleMatchType::ClientParameter
        )
    {
        return None;
    }

    if completed
        .parameters
        .iter()
        .any(|(key, _)| key.trim().eq_ignore_ascii_case("drone"))
    {
        return Some(ForeignDownloadClassification::DroneParameter);
    }

    if let Some(observed_category) = normalized_download_category(
        completed
            .category
            .as_deref()
            .or(td.client_item.category.as_deref()),
    ) && !td.client_id.trim().is_empty()
    {
        if app
            .owned_download_client_categories_snapshot()
            .await
            .is_some_and(|snapshot| !snapshot.owns_category(&td.client_id, observed_category))
        {
            return Some(ForeignDownloadClassification::ForeignCategory);
        }
    }

    if !td.client_item.is_scryer_origin {
        let path = std::path::Path::new(&completed.dest_dir);
        match crate::import_workflow::find_video_files(path, false) {
            Ok(video_files) if video_files.is_empty() => {
                // Do not extract here. The import workflow needs the resolved
                // title to stage extraction safely; this shared planner only
                // decides whether the source remains an archive candidate.
                let archive_candidate =
                    match crate::archive_extractor::archive_extraction_would_be_needed(path) {
                        Ok(needed) => needed,
                        Err(_) => return None,
                    };
                if !archive_candidate && matches!(contains_archive_file(path), Ok(false)) {
                    return Some(ForeignDownloadClassification::NoImportableVideo);
                }
            }
            Ok(_) | Err(_) => {}
        }
    }

    None
}

fn contains_archive_file(path: &std::path::Path) -> std::io::Result<bool> {
    if path.is_file() {
        return Ok(scryer_domain::is_archive_file(path));
    }

    let mut directories = vec![path.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let entry_path = entry.path();
            if file_type.is_dir() {
                directories.push(entry_path);
            } else if file_type.is_file() && scryer_domain::is_archive_file(&entry_path) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn mark_foreign_download(td: &mut TrackedDownload, classification: ForeignDownloadClassification) {
    // Foreign classification is runtime-only: nothing is persisted, no status
    // message is set, and the row vanishes from every user-facing surface. That
    // makes a misclassification observable ONLY as an absence, which is
    // effectively undiagnosable in the field and cost a full triage cycle here.
    // This line is the single breadcrumb that distinguishes "hidden on purpose"
    // from "silently lost".
    tracing::info!(
        id = %td.id,
        classification = ?classification,
        client_id = %td.client_id,
        client_type = %td.client_type,
        category = ?td.client_item.category,
        is_scryer_origin = td.client_item.is_scryer_origin,
        match_type = ?td.match_type,
        "download classified as foreign; hidden from user-facing download activity"
    );
    td.foreign_import_classification = Some(classification);
    td.state = TrackedDownloadState::Downloading;
    td.waiting_for_completed_history = false;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();
    td.path_missing_since = None;
    td.no_video_import_retry = None;
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

async fn completed_download_allows_automatic_import(
    app: &AppUseCase,
    td: &TrackedDownload,
    completed: &CompletedDownload,
) -> bool {
    if matches!(
        td.match_type,
        TitleMatchType::Submission | TitleMatchType::ClientParameter
    ) || td.client_item.is_scryer_origin
    {
        return true;
    }

    let Some(observed_category) = normalized_download_category(
        completed
            .category
            .as_deref()
            .or(td.client_item.category.as_deref()),
    ) else {
        return true;
    };

    let Some(title_id) = td
        .title_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    let title = match app.services.catalog.titles.get_by_id(title_id).await {
        Ok(Some(title)) => title,
        Ok(None) => return false,
        Err(error) => {
            tracing::warn!(
                title_id,
                error = %error,
                "completed download category gate could not load title"
            );
            return false;
        }
    };

    match app
        .effective_download_client_category_for_title(&title, &td.client_id)
        .await
    {
        Ok(Some(expected_category)) => observed_category == expected_category.trim(),
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(
                title_id,
                client_id = td.client_id.as_str(),
                error = %error,
                "completed download category gate could not resolve effective category"
            );
            false
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssignedTitleProof {
    /// A raw name proved the assigned title; import may proceed.
    Proven,
    /// Every raw name failed the proof — block for manual review.
    Disproven,
    /// The assigned title no longer exists — block under its own reason.
    MissingTitle,
    /// Infrastructure error while proving — retry next tick, never block.
    Unknown,
}

async fn completed_download_proves_assigned_title(
    app: &AppUseCase,
    td: &TrackedDownload,
    completed: &CompletedDownload,
) -> AssignedTitleProof {
    if !matches!(
        td.match_type,
        TitleMatchType::Submission | TitleMatchType::ClientParameter
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

    let folder_name = std::path::Path::new(&completed.dest_dir)
        .file_name()
        .and_then(|value| value.to_str());
    let mut completion_sources = Vec::<&str>::new();
    for raw_title in [Some(completed.name.as_str()), folder_name]
        .into_iter()
        .flatten()
    {
        let raw_title = raw_title.trim();
        if !raw_title.is_empty() && !completion_sources.contains(&raw_title) {
            completion_sources.push(raw_title);
        }
    }

    // What actually finished on disk outranks every other signal: a completion
    // name that positively asserts a *different* library title's identity —
    // and not the assigned one — is a contradiction, not obfuscation. Neither
    // the durable submission linkage nor the historical source_title may
    // override it.
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

    // A Submission or ClientParameter match is Scryer's own durable grab-time
    // identity: the submission row / embedded client parameters were written
    // when the grab passed the full disambiguator discipline, with strictly
    // more evidence (indexer ids, year, uniqueness) than a filename can carry.
    // Re-deriving identity from the release name can only lose information, so
    // with no contradiction on disk the linkage stands as proof.
    if matches!(
        td.match_type,
        TitleMatchType::Submission | TitleMatchType::ClientParameter
    ) {
        return AssignedTitleProof::Proven;
    }

    for raw_title in &completion_sources {
        if proves_assigned_title(raw_title) {
            return AssignedTitleProof::Proven;
        }
    }

    // The grabbed release name is fallback proof for clients that obfuscate the
    // completed name and folder mid-flight; a Scryer-origin grab must not lose
    // its identity to that. Junk cannot ride in on it — the matcher rejects a
    // non-proving source_title exactly like any other raw name.
    if let Some(source_title) = td
        .source_title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && !completion_sources.contains(&source_title)
        && proves_assigned_title(source_title)
    {
        return AssignedTitleProof::Proven;
    }

    AssignedTitleProof::Disproven
}

fn normalized_download_category(category: Option<&str>) -> Option<&str> {
    category.map(str::trim).filter(|value| !value.is_empty())
}

pub(super) fn has_id_only_conflict(td: &TrackedDownload) -> bool {
    td.status_messages
        .iter()
        .any(|message| message == ID_ONLY_CONFLICT_MESSAGE)
}

async fn set_state_to_import_blocked(app: &AppUseCase, td: &mut TrackedDownload) {
    let was_blocked = td.state == TrackedDownloadState::ImportBlocked;
    td.state = TrackedDownloadState::ImportBlocked;
    td.waiting_for_completed_history = false;
    td.status = TrackedDownloadStatus::Warning;

    if !was_blocked {
        crate::tracked_downloads::persist_tracked_download_state_marker(
            app,
            td,
            TrackedDownloadState::ImportBlocked,
            Some("import_blocked_pre_import"),
            td.status_messages.first().map(String::as_str),
        )
        .await;
    }

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
    reason: &str,
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
    // set_state_to_import_blocked writes the generic blocked marker; record
    // the specific identity reason afterwards so it wins the upsert.
    set_state_to_import_blocked(app, td).await;
    let source_identity = DownloadSourceIdentity::new(
        Some(td.client_id.as_str()),
        &td.client_type,
        &td.client_item.download_client_item_id,
    );
    if let Err(error) = app
        .services
        .workflow
        .download_submissions
        .record_identity_tracked_state(
            &observed_identity,
            Some(&source_identity),
            TrackedDownloadState::ImportBlocked.as_str(),
            Some(reason),
            Some(detail),
        )
        .await
    {
        tracing::warn!(
            error = %error,
            client_id = td.client_id.as_str(),
            client_type = td.client_type.as_str(),
            download_client_item_id = td.client_item.download_client_item_id.as_str(),
            reason,
            "failed to persist durable tracked-download manual-review state"
        );
    }
}
