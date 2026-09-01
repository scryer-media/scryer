/// If subtitles.auto_download_on_import is enabled, spawn a background subtitle search.
fn maybe_trigger_subtitle_search(app: &AppUseCase, title_id: &str, media_file_id: &str) {
    let app = app.clone();
    let title_id = title_id.to_string();
    let media_file_id = media_file_id.to_string();
    tokio::spawn(async move {
        let auto = app
            .subtitle_settings()
            .await
            .ok()
            .map(|settings| settings.auto_download_on_import)
            .unwrap_or(false);
        if auto {
            crate::spawn_subtitle_search_for_file(app, title_id, media_file_id);
        }
    });
}

async fn analyze_and_persist_imported_media_file(
    app: &AppUseCase,
    title_id: &str,
    media_file_id: &str,
    file_path: &std::path::Path,
) {
    let acceptance = match app
        .services
        .library
        .media_analyzer
        .analyze_file(file_path.to_path_buf())
        .await
    {
        Ok(crate::MediaAnalysisOutcome::Valid(analysis)) => {
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: Some(*analysis),
                scan_error: None,
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
        Ok(crate::MediaAnalysisOutcome::Invalid(error)) => {
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: None,
                scan_error: Some(error),
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id,
                file_id = %media_file_id,
                file_path = %file_path.display(),
                "failed to analyze imported media file"
            );
            crate::post_download_gate::ImportedFileAcceptance {
                analysis: None,
                scan_error: Some(error.to_string()),
                rule_file_doc: None,
                audio_language_warning: None,
            }
        }
    };

    crate::post_download_gate::persist_media_analysis_result(
        &app.services.library.media_files,
        media_file_id,
        &acceptance,
    )
    .await;
}

fn completed_download_identity(completed: &CompletedDownload) -> ClientJobLocator {
    ClientJobLocator::new(
        Some(completed.client_id.as_str()),
        &completed.client_type,
        &completed.download_client_item_id,
    )
}
fn additional_import_dest_path(
    canonical_dest_path: &Path,
    parsed: &ParsedReleaseMetadata,
) -> PathBuf {
    let parent = canonical_dest_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let stem = canonical_dest_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("additional");
    let extension = canonical_dest_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mkv");
    let raw_label = parsed
        .edition
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(parsed.raw_title.as_str());
    let sanitized_label = sanitize_filesystem_component(raw_label)
        .trim()
        .chars()
        .take(48)
        .collect::<String>();
    let label = if sanitized_label.is_empty() {
        "additional".to_string()
    } else {
        sanitized_label
    };
    let hash = blake3::hash(parsed.raw_title.as_bytes()).to_hex();
    let hash = &hash.as_str()[..8];
    let base_name = sanitize_filesystem_component(&format!("{stem} - {label} {hash}.{extension}"));
    let mut candidate = parent.join(&base_name);
    if !candidate.exists() {
        return candidate;
    }

    for suffix in 2..=999 {
        let name = sanitize_filesystem_component(&format!(
            "{stem} - {label} {hash} ({suffix}).{extension}"
        ));
        candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    parent.join(sanitize_filesystem_component(&format!(
        "{stem} - {label} {hash} {}.{extension}",
        Id::new().0
    )))
}
const SCRYER_TITLE_ID_PARAM: &str = "*scryer_title_id";
const SCRYER_FACET_PARAM: &str = "*scryer_facet";
const SCRYER_COLLECTION_ID_PARAM: &str = "*scryer_collection_id";
const SCRYER_SERIES_MOVIE_LINK_ID_PARAM: &str = "*scryer_series_movie_link_id";

/// The pure "stamp" step of provenance resolution: a completed download whose
/// live submission is a Scryer grab carries that grab's identity parameters
/// (authoritative over whatever the client echoed) and its persisted indexer
/// release title as `release_name`. A submission recorded without a release
/// title must not blank a real client-reported name; the completed download
/// keeps it.
fn stamp_scryer_submission_origin(
    completed: &CompletedDownload,
    submission: &DownloadSubmission,
) -> CompletedDownload {
    let mut resolved = completed.clone();
    resolved.parameters = authoritative_scryer_origin_parameters(&completed.parameters, submission);
    resolved.release_name =
        submission_source_title(submission).or_else(|| completed_observed_release_name(completed));
    resolved
}

fn authoritative_scryer_origin_parameters(
    parameters: &[(String, String)],
    submission: &DownloadSubmission,
) -> Vec<(String, String)> {
    let mut resolved = parameters
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                SCRYER_TITLE_ID_PARAM
                    | SCRYER_FACET_PARAM
                    | SCRYER_COLLECTION_ID_PARAM
                    | SCRYER_SERIES_MOVIE_LINK_ID_PARAM
            )
        })
        .cloned()
        .collect::<Vec<_>>();

    if !submission.title_id.trim().is_empty() {
        resolved.push((
            SCRYER_TITLE_ID_PARAM.to_string(),
            submission.title_id.clone(),
        ));
    }
    if !submission.facet.trim().is_empty() {
        resolved.push((SCRYER_FACET_PARAM.to_string(), submission.facet.clone()));
    }
    match &submission.scope {
        SubmissionScope::Collection { collection_id } => {
            resolved.push((
                SCRYER_COLLECTION_ID_PARAM.to_string(),
                collection_id.clone(),
            ));
        }
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => {
            resolved.push((
                SCRYER_SERIES_MOVIE_LINK_ID_PARAM.to_string(),
                series_movie_link_id.clone(),
            ));
        }
        SubmissionScope::Episode { .. }
        | SubmissionScope::EpisodeSet { .. }
        | SubmissionScope::Title
        | SubmissionScope::Orphan => {}
    }
    resolved
}
async fn terminal_download_item_is_still_visible(
    app: &AppUseCase,
    client_id: &str,
    client_type: &str,
    download_client_item_id: &str,
    is_history: bool,
) -> bool {
    let lookup = if is_history {
        app.services
            .integrations
            .download_client
            .list_history()
            .await
    } else {
        app.services.integrations.download_client.list_queue().await
    };

    match lookup {
        Ok(items) => items.iter().any(|item| {
            item.download_client_item_id == download_client_item_id
                && item.client_type.eq_ignore_ascii_case(client_type)
                && (client_id.is_empty() || item.client_id.trim() == client_id)
        }),
        Err(error) => {
            tracing::warn!(
                error = %error,
                client_id,
                client_type,
                download_client_item_id,
                is_history,
                "failed to confirm download item visibility after delete error"
            );
            true
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalDownloadCleanupOutcome {
    NotConfigured,
    Removed,
    AlreadyGone,
    RetryableFailure,
    /// The torrent is imported but has not discharged its seeding obligation.
    /// The tracked download stays visible in `ImportedSeeding` and re-enters
    /// the gate on the next poll.
    HeldForSeeding,
    /// The seeding obligation is discharged but the profile (or the client's
    /// own nature, as with `torrent-blackhole`) says the entry stays. Nothing
    /// further to reconcile.
    SeedingEntryKept,
    /// The profile's post-import tracking is `HandOff`: the download settles
    /// with the client entry untouched and Scryer stops managing the torrent.
    /// Kept distinct from `SeedingEntryKept` so logs, tests and the history
    /// event can tell "the goal was met and the profile keeps the entry" from
    /// "the operator opted out of management".
    HandedOff,
}
/// What the gate actually did with a client entry it released.
///
/// Distinct from `SeedGoalMetAction`, which is the profile's *intent*: a
/// `StopSeeding` profile on a client that cannot pause degrades to `Kept`, and
/// the history has to say what happened, not what was wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeedingReleaseAction {
    Removed,
    Paused,
    Kept,
    /// The entry was already gone from the client when the gate looked.
    Vanished,
    /// The entry was left exactly as it is and Scryer stopped managing the
    /// torrent, per the profile's post-import tracking.
    HandedOff,
}

impl SeedingReleaseAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::Paused => "paused",
            Self::Kept => "kept",
            Self::Vanished => "vanished",
            Self::HandedOff => "handed_off",
        }
    }
}

/// The seeding gate's verdict for one terminal cleanup, carried back for the
/// seeding history events. Absent when the gate never ran (usenet, or removal
/// disabled), in which case no seeding history is recorded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SeedingGateReport {
    /// The gate's reason constant, verbatim — the same string the queue
    /// projection derives its badge from.
    pub reason: &'static str,
    /// What happened to the entry; `None` while it is still held.
    pub action: Option<SeedingReleaseAction>,
    /// Observed at the moment of the decision, when the client reports it.
    pub seed_ratio: Option<f64>,
    pub seed_time_seconds: Option<i64>,
}

/// A terminal cleanup's outcome plus, when the seeding gate ran, its verdict.
///
/// Compares equal to a bare `TerminalDownloadCleanupOutcome` so every existing
/// call site and assertion reads as before while the seeding detail rides
/// along for the history events.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerminalDownloadCleanup {
    pub outcome: TerminalDownloadCleanupOutcome,
    pub seeding: Option<SeedingGateReport>,
}

impl TerminalDownloadCleanup {
    fn bare(outcome: TerminalDownloadCleanupOutcome) -> Self {
        Self {
            outcome,
            seeding: None,
        }
    }

    fn gated(outcome: TerminalDownloadCleanupOutcome, seeding: SeedingGateReport) -> Self {
        Self {
            outcome,
            seeding: Some(seeding),
        }
    }
}

impl PartialEq<TerminalDownloadCleanupOutcome> for TerminalDownloadCleanup {
    fn eq(&self, other: &TerminalDownloadCleanupOutcome) -> bool {
        self.outcome == *other
    }
}

pub(crate) fn terminal_download_cleanup_is_complete(
    outcome: TerminalDownloadCleanupOutcome,
) -> bool {
    matches!(
        outcome,
        TerminalDownloadCleanupOutcome::NotConfigured
            | TerminalDownloadCleanupOutcome::Removed
            | TerminalDownloadCleanupOutcome::AlreadyGone
            | TerminalDownloadCleanupOutcome::SeedingEntryKept
            | TerminalDownloadCleanupOutcome::HandedOff
    )
}
pub(crate) async fn cleanup_routing_scope_for_title_id(
    app: &AppUseCase,
    title_id: Option<&str>,
) -> (Option<String>, Option<MediaFacet>) {
    let Some(title_id) = title_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return (None, None);
    };

    match app.services.catalog.titles.get_by_id(title_id).await {
        Ok(Some(title)) => (Some(title.library_id), Some(title.facet)),
        Ok(None) | Err(_) => (None, None),
    }
}

/// Library and facet a terminal cleanup routes by, resolved from its title.
type CleanupRoutingScope = (Option<String>, Option<MediaFacet>);

/// What `should_remove_completed_download` actually depends on: routing scope
/// plus the client key the entry would be removed from.
type RemovalPolicyKey = (Option<String>, MediaFacet, String);

/// The reads every settled tracked row in one reconcile tick would otherwise
/// repeat for itself.
///
/// `reconcile_terminal_tracked_downloads` re-offers *every* settled row to the
/// removal gate on every poll — that re-offering is what eventually releases a
/// held torrent — so the per-row cost is paid once per row per tick, forever,
/// for as long as a torrent is held. Without this each row runs its own
/// seed-goal query, its own title lookup and its own routing-entry read, and
/// rows of the same title (a season pack) or the same client repeat those
/// answers verbatim.
///
/// Deliberately scoped to one tick and never reused across ticks: routing
/// configuration and persisted goals can change between polls, and acting on a
/// stale `remove_completed` or a stale goal is exactly the class of mistake
/// that removes a torrent still under obligation.
pub(crate) struct TerminalCleanupTickCache {
    goals: crate::seeding_gate::SeedGoalBatch,
    routing_scopes: std::sync::Mutex<HashMap<String, CleanupRoutingScope>>,
    remove_completed: std::sync::Mutex<HashMap<RemovalPolicyKey, bool>>,
    /// Reads that actually reached a repository, so a test can pin the hoist
    /// itself rather than only the shape of the key.
    routing_scope_reads: std::sync::atomic::AtomicUsize,
    remove_completed_reads: std::sync::atomic::AtomicUsize,
}

impl TerminalCleanupTickCache {
    /// Prefetch the seed goals for every settled row in this tick in one query.
    /// The memoized caches start empty and fill as rows are reconciled.
    pub(crate) async fn prefetch(app: &AppUseCase, identities: &[ClientJobLocator]) -> Self {
        Self {
            goals: crate::seeding_gate::SeedGoalBatch::prefetch(app, identities).await,
            routing_scopes: std::sync::Mutex::new(HashMap::new()),
            remove_completed: std::sync::Mutex::new(HashMap::new()),
            routing_scope_reads: std::sync::atomic::AtomicUsize::new(0),
            remove_completed_reads: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub(crate) fn goal_batch(&self) -> &crate::seeding_gate::SeedGoalBatch {
        &self.goals
    }

    /// `(routing-scope reads, remove-completed reads)` that missed the memo.
    #[cfg(test)]
    pub(crate) fn memo_reads(&self) -> (usize, usize) {
        use std::sync::atomic::Ordering;
        (
            self.routing_scope_reads.load(Ordering::Relaxed),
            self.remove_completed_reads.load(Ordering::Relaxed),
        )
    }
}

/// `cleanup_routing_scope_for_title_id`, answering from the tick cache when the
/// same title has already been resolved in this tick.
async fn cleanup_routing_scope_for_title_id_cached(
    app: &AppUseCase,
    title_id: Option<&str>,
    cache: Option<&TerminalCleanupTickCache>,
) -> CleanupRoutingScope {
    let Some(cache) = cache else {
        return cleanup_routing_scope_for_title_id(app, title_id).await;
    };
    let Some(key) = title_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return (None, None);
    };

    if let Ok(scopes) = cache.routing_scopes.lock()
        && let Some(hit) = scopes.get(&key)
    {
        return hit.clone();
    }
    cache
        .routing_scope_reads
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let resolved = cleanup_routing_scope_for_title_id(app, Some(key.as_str())).await;
    if let Ok(mut scopes) = cache.routing_scopes.lock() {
        scopes.insert(key, resolved.clone());
    }
    resolved
}

/// `AppUseCase::should_remove_completed_download`, memoized per tick on the
/// `(library_id, facet, routing_key)` tuple it actually depends on.
async fn should_remove_completed_download_cached(
    app: &AppUseCase,
    library_id: Option<&str>,
    facet: &MediaFacet,
    routing_key: &str,
    cache: Option<&TerminalCleanupTickCache>,
) -> bool {
    let Some(cache) = cache else {
        return app
            .should_remove_completed_download(library_id, facet, routing_key)
            .await;
    };
    let key = (
        library_id.map(str::to_string),
        facet.clone(),
        routing_key.to_string(),
    );
    if let Ok(policies) = cache.remove_completed.lock()
        && let Some(hit) = policies.get(&key).copied()
    {
        return hit;
    }
    cache
        .remove_completed_reads
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let resolved = app
        .should_remove_completed_download(library_id, facet, routing_key)
        .await;
    if let Ok(mut policies) = cache.remove_completed.lock() {
        policies.insert(key, resolved);
    }
    resolved
}

async fn terminal_failure_origin_for_tracked(
    app: &AppUseCase,
    tracked: &crate::tracked_downloads::TrackedDownload,
    state: TrackedDownloadState,
) -> TerminalFailureOrigin {
    if tracked.burned_by_import_gate {
        return TerminalFailureOrigin::ImportGate;
    }
    if state != TrackedDownloadState::Failed {
        return TerminalFailureOrigin::ClientFailure;
    }
    if tracked.burned_by_import_gate {
        return TerminalFailureOrigin::ImportGate;
    }

    let identity = crate::tracked_downloads::observed_queue_item_identity(&tracked.client_item);
    if crate::download_submission_identity_is_empty(&identity) {
        return TerminalFailureOrigin::ClientFailure;
    }
    let source_identity = ClientJobLocator::new(
        Some(tracked.client_id.as_str()),
        &tracked.client_type,
        &tracked.client_item.download_client_item_id,
    );
    let seeding_gate_failure = app
        .services
        .workflow
        .download_submissions
        .get_identity_tracked_state_reason(&identity, Some(&source_identity))
        .await
        .ok()
        .flatten()
        .is_some_and(|reason| {
            matches!(
                reason.as_str(),
                crate::tracked_downloads::IMPORT_GATE_REJECTED_TRACKED_STATE_REASON
                    | crate::tracked_downloads::WARNING_TIMEOUT_TRACKED_STATE_REASON
            )
        });
    if seeding_gate_failure {
        TerminalFailureOrigin::ImportGate
    } else {
        TerminalFailureOrigin::ClientFailure
    }
}

pub(crate) async fn reconcile_terminal_download_cleanup_for_tracked(
    app: &AppUseCase,
    tracked: &crate::tracked_downloads::TrackedDownload,
    state: TrackedDownloadState,
    cache: Option<&TerminalCleanupTickCache>,
) -> TerminalDownloadCleanup {
    let (library_id, resolved_facet) =
        cleanup_routing_scope_for_title_id_cached(app, tracked.title_id.as_deref(), cache).await;
    let facet = resolved_facet.or_else(|| facet_from_tracked_label(tracked.facet.as_deref()));
    let precomputed_should_remove = if state == TrackedDownloadState::Failed
        && !tracked.burned_by_import_gate
    {
        let should_remove = crate::import_workflow::should_remove_terminal_download(
            app,
            &tracked.client_id,
            &tracked.client_type,
            library_id.as_deref(),
            facet.as_ref(),
            state,
            cache,
        )
        .await;
        if !should_remove {
            return TerminalDownloadCleanup::bare(TerminalDownloadCleanupOutcome::NotConfigured);
        }
        Some(should_remove)
    } else {
        None
    };
    let failure_origin = terminal_failure_origin_for_tracked(app, tracked, state).await;
    reconcile_terminal_download_cleanup(
        app,
        tracked.canonical_download_id(),
        &tracked.client_id,
        &tracked.client_type,
        &tracked.client_item.download_client_item_id,
        library_id.as_deref(),
        facet.as_ref(),
        state,
        failure_origin,
        precomputed_should_remove,
        // The tracker already answers "is this still in the client?": a row
        // absent from the client's snapshot past the grace window is marked
        // untrackable. Reusing that avoids a per-item listing call every tick.
        tracked.is_trackable,
        // The live tracked row was refreshed from the client earlier in this
        // same tick, so its observation is fresher than the published snapshot
        // (which is only republished *after* reconcile runs). Passing it in is
        // what makes each cycle re-evaluate against current ratio/seed time
        // rather than the answer that first parked the row.
        crate::seeding_gate::observation_from_queue_item(&tracked.client_item),
        cache,
    )
    .await
}
/// Scryer's own transient markers on a *non-`Failed`* import result.
///
/// Execution-phase failures never come through here: they arrive as
/// `ImportDecision::Failed` and are retried by the phase rule in
/// `completed_import_result_is_retryable` regardless of the message (Sonarr's
/// model — no error-string catalogue). This list only recognises the transient
/// conditions Scryer itself reports as a `Skipped`/`Rejected` result after an
/// execution race. Import-check outcomes such as a source still unpacking are
/// represented by `ImportSkipReason`, never their message text.
fn completed_import_error_message_is_retryable(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    const SCRYER_TRANSIENT_PHRASES: &[&str] = &[
        "source changed",
        "temporarily",
        "not found or inaccessible",
    ];
    SCRYER_TRANSIENT_PHRASES
        .iter()
        .any(|needle| normalized.contains(needle))
        || contains_word(&normalized, "locked")
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack.match_indices(word).any(|(start, _)| {
        let end = start + word.len();
        let before_is_word = haystack[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        let after_is_word = haystack[end..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric);
        !before_is_word && !after_is_word
    })
}

async fn resolve_import_quality_profile(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> crate::AppResult<crate::QualityProfile> {
    let tvdb_id = title
        .external_ids
        .iter()
        .find(|external_id| external_id.source == "tvdb")
        .map(|external_id| external_id.value.as_str());
    let category_hint = crate::post_download_gate::facet_to_category_hint(&title.facet);
    // Resolution failures propagate: gating an import against a substitute
    // profile silently applies the wrong quality rules, which is the failure
    // mode the strict resolver exists to prevent. A validation failure (e.g. a
    // dangling profile reference) needs operator action and surfaces as a
    // blocked import; any other failure is treated as transient and worded so
    // `completed_import_error_message_is_retryable` re-attempts it.
    app.resolve_quality_profile(crate::app_usecase_discovery::QualityProfileLookup {
        title_tags: &title.tags,
        library_id: Some(title.library_id.as_str()),
        imdb_id: title.imdb_id.as_deref(),
        tvdb_id,
        category_hint: Some(category_hint),
    })
    .await
    .map_err(|error| match error {
        crate::AppError::Validation(_) => error,
        other => crate::AppError::Repository(format!(
            "quality profile resolution temporarily unavailable: {other}"
        )),
    })
}
/// "Not media at all" — a sample, a promo, a zero-length placeholder. Owned by
/// the import pipeline alone: scoring no longer has a floor to keep in step with
/// it, because the smallness it used to veto below is now a penalty read off the
/// size curve like any other band.
const SAMPLE_SIZE_THRESHOLD: u64 = 50 * 1024 * 1024;

fn non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}
