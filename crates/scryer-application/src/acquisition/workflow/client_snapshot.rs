/// How long an item may sit in `Downloading` before its claim on an empty
/// scope stops suppressing background searches and falls back to the D18
/// pseudo-incumbent comparison. A healthy download finishes well inside this;
/// what exceeds it is the stalled-swarm shape that would otherwise freeze its
/// scope indefinitely, since a dead torrent never fails on its own.
pub(crate) const STALE_INITIAL_CLAIM_SECONDS: i64 = 24 * 60 * 60;

/// Snapshot of the download client's current queue and recent history,
/// fetched once per polling cycle to avoid repeated API calls.
pub(crate) struct DownloadClientSnapshot {
    /// Lowercase title names of items currently queued or downloading.
    active_titles: std::collections::HashSet<String>,
    /// Download client item IDs of items currently queued/downloading.
    /// Used for episode-level dedup (check by submission ID, not title name).
    active_client_ids: std::collections::HashSet<String>,
    /// Raw native item ID counts for legacy rows that predate configured
    /// client IDs. Used only when the raw ID is unique in the snapshot.
    active_raw_item_id_counts: std::collections::HashMap<String, usize>,
    /// Active items that have been *downloading* longer than
    /// [`STALE_INITIAL_CLAIM_SECONDS`], by exact identity and by raw item id.
    ///
    /// A live download suppresses background searches for the empty scope it
    /// claims, but a claim this old is a stalled-swarm shape, not a claim — the
    /// scope falls back to the D18 pseudo-incumbent comparison so a strictly
    /// better release can still be acquired beside it. Only `Downloading`
    /// qualifies: `Queued`/`Paused`/`Warning` are operator-visible states with
    /// their own exits. Computed at snapshot build, where the full queue item
    /// (state and `queued_at`) is in hand; an item without a parseable
    /// `queued_at` never reads as stale, so clients that report no age keep
    /// the suppression.
    stale_downloading_client_ids: std::collections::HashSet<String>,
    stale_downloading_raw_item_ids: std::collections::HashSet<String>,
    /// Download client item IDs of items that completed successfully.
    completed_client_ids: std::collections::HashSet<String>,
    completed_raw_item_id_counts: std::collections::HashMap<String, usize>,
    /// Failed history items keyed by download client job ID (NZBGet NZBID,
    /// SABnzbd nzo_id, Weaver job UUID). Matched against `download_submissions`
    /// table to find which scryer title a failed download belongs to.
    failed_by_download_id: std::collections::HashMap<String, FailedDownloadSnapshot>,
    /// True when `list_queue()` errored while building this snapshot. An
    /// unobservable queue must be treated as "possibly active" for automatic
    /// grabs so a transient client outage cannot cause a blind double-submit
    /// (the Scryer-shaped analogue of Sonarr's download-client backoff).
    queue_listing_failed: bool,
    /// True when `list_history()` errored while building this snapshot. Failure
    /// detection reads only history, so an unobservable history simply yields
    /// no failures rather than acting on an empty map.
    history_listing_failed: bool,
    /// Download client IDs whose queue or history could not be read this
    /// cycle: the read errored, timed out, or was skipped during feedback
    /// backoff. A submission on such a client cannot be proven gone, so it
    /// keeps its claim on the scope until the client answers again or the
    /// tracked ledger settles it.
    unreadable_client_ids: std::collections::HashSet<String>,
}

/// Folds one leg's read report into the snapshot: every client that did not
/// answer is remembered, and a leg nobody answered is marked failed exactly
/// like a hard listing error. Returns the items that were read.
fn note_unreadable_clients(
    listing: crate::DownloadClientListing,
    leg: &'static str,
    unreadable_client_ids: &mut std::collections::HashSet<String>,
    listing_failed: &mut bool,
) -> Vec<scryer_domain::DownloadQueueItem> {
    if !listing.unreadable_client_ids.is_empty() {
        if listing.all_unreadable() {
            *listing_failed = true;
            warn!(
                leg,
                clients = ?listing.unreadable_client_ids,
                "download client snapshot: no download client answered; treating the listing as unobservable"
            );
        } else {
            warn!(
                leg,
                clients = ?listing.unreadable_client_ids,
                "download client snapshot: some download clients did not answer; their downloads keep their claims"
            );
        }
        unreadable_client_ids.extend(listing.unreadable_client_ids);
    }
    listing.items
}

fn download_client_item_identity(client_id: Option<&str>, item_id: &str) -> String {
    let client_id = client_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    if client_id.is_empty() {
        return item_id.to_string();
    }

    format!("{client_id}:{item_id}")
}
#[derive(Clone, Debug)]
pub(crate) struct FailedDownloadSnapshot {
    reason: String,
    download_client_item_id: String,
    client_id: String,
    client_name: Option<String>,
}
#[derive(Clone, Debug)]
pub(crate) struct DownloadFailureContext {
    pub wanted_item: Option<AcquisitionScopeState>,
    pub title_id: Option<String>,
    pub client_id: String,
    pub client_type: String,
    pub client_name: Option<String>,
    pub client_item_id: String,
    pub release_title: String,
    pub reason: String,
    pub remove_from_client_if_configured: bool,
    pub skip_reacquire: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureHandlingOutcome {
    /// The failed release is blocklisted and its scope is `wanted` again under
    /// its existing coverage. The cursor walks the scope's saved search results
    /// (`try_saved_candidates`) before it would spend an indexer query; a scope
    /// whose saved results are exhausted simply stays converged.
    Reopened,
    /// The failed release was recorded and blocklisted, but no acquisition
    /// scope changed. Operator-queued and manual-replacement grabs are outside
    /// the automatic loop: they save no standby list, never change convergence
    /// coverage, and never re-open or walk a scope on client failure.
    RecordedOnly,
    RecordedNoReacquire,
    AlreadyHandled,
}
/// Result of walking a scope's saved search results, best first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StandbyRecoveryOutcome {
    /// The next eligible saved result was grabbed; the scope is `grabbed`
    /// again. Returning the submitted scope lets the cursor suppress exactly
    /// the recovered pack's coverage without reparsing the saved release.
    Recovered { scope: SubmissionScope },
    /// The saved release is already active in a download client.
    Active { scope: SubmissionScope },
    /// The download client could not be consulted; the list is left intact for
    /// the next cycle.
    Deferred { scope: Option<SubmissionScope> },
    /// A better saved result is still held by a delay profile. The promotion
    /// lane owns it, so this walk must not take a worse release.
    Parked { scope: Option<SubmissionScope> },
    /// Every saved result has been tried (or is no longer eligible). Sources
    /// whose artifact vanished are returned so the cursor can refresh only
    /// their coverage once.
    Exhausted { stale_indexer_ids: Vec<String> },
}

fn order_standby_releases(
    standby_releases: &mut [PendingRelease],
    season_pack_ids: &HashSet<String>,
    series_pack_ids: &HashSet<String>,
) {
    standby_releases.sort_by_key(|release| {
        // Episode rows always recover before packs. Among packs, whole-series
        // rows come before plain season packs so their persisted `added_at`
        // sequence remains the search rank they were saved with.
        let (group, score) = if series_pack_ids.contains(&release.id) {
            (1, 0)
        } else if season_pack_ids.contains(&release.id) {
            (2, release.release_score)
        } else {
            (0, release.release_score)
        };
        (
            group,
            std::cmp::Reverse(score),
            release.added_at.clone(),
            release.id.clone(),
        )
    });
}
// Canonical owner for all title-affecting failed release / blocklist side effects.
#[expect(
    clippy::too_many_arguments,
    reason = "failure recording persists the full release-attribution envelope for auditability"
)]
async fn record_failed_release_outcome(
    app: &AppUseCase,
    title_id: Option<&str>,
    attribution: &FailedReleaseAttribution,
    source_title: Option<String>,
    source_hint: Option<String>,
    // Both halves of the blocklist key: the indexer the release came from
    // (empty when unattributed) and the torrent's infohash when it had one.
    blocklist_indexer_id: String,
    blocklist_info_hash: Option<String>,
    download_id: Option<String>,
    client_id: Option<String>,
    client_name: Option<String>,
    client_type: Option<String>,
    quality: Option<String>,
    failure_reason: Option<String>,
    blocklist_reason: Option<String>,
    source_password: Option<String>,
) {
    let normalized_source_title = normalize_release_name(source_title.as_deref());
    let normalized_source_hint = normalize_release_attempt_hint(source_hint.as_deref());
    let normalized_client_id = normalized_non_empty_owned(client_id);
    let normalized_client_name = normalized_non_empty_owned(client_name);
    let normalized_client_type = normalized_non_empty_owned(client_type);

    let mut blocklist_persisted = false;
    if let Some(title_id) = title_id {
        let _ = app
            .services
            .workflow
            .release_attempts
            .record_release_attempt(
                Some(title_id.to_string()),
                normalized_source_hint.clone(),
                normalized_source_title.clone(),
                ReleaseDownloadAttemptOutcome::Failed,
                failure_reason.clone(),
                source_password,
            )
            .await;

        if let Some(reason) = blocklist_reason.clone()
            && let Some(release_name) = normalized_source_title.clone()
        {
            // `block` is idempotent against the schema, so a failure already
            // recorded by another path returns Ok(false) rather than
            // duplicating the row -- and that answer is what decides whether
            // the ReleaseBlocklisted event fires.
            match app
                .services
                .workflow
                .blocklist_repo
                .block(&NewBlocklistEntry {
                    title_id: title_id.to_string(),
                    release_name,
                    indexer_id: blocklist_indexer_id,
                    info_hash: blocklist_info_hash,
                    reason: Some(reason),
                })
                .await
            {
                Ok(written) => {
                    blocklist_persisted = written;
                }
                Err(error) => {
                    warn!(
                        title_id,
                        source_title = normalized_source_title.as_deref().unwrap_or(""),
                        error = %error,
                        "failed to persist blocklist entry for failed download"
                    );
                }
            }
        }
    }

    let title = attribution.title.as_ref();
    let title_snapshot = title.map(title_context_snapshot);
    let payload = DomainEventPayload::DownloadFailed(DownloadFailedEventData {
        title: title_snapshot.clone(),
        source_title: normalized_source_title.clone(),
        source_hint: normalized_source_hint.clone(),
        download_id: download_id.clone(),
        client_id: normalized_client_id.clone(),
        client_name: normalized_client_name.clone(),
        client_type: normalized_client_type.clone(),
        quality: quality.clone(),
        reason: failure_reason,
        episode_ids: attribution.episode_ids.clone(),
        collection_id: attribution.collection_id.clone(),
    });
    let _ = app
        .append_domain_event(title_scoped_domain_event(title_id, title, payload))
        .await;

    if blocklist_persisted && let Some(reason) = blocklist_reason {
        let payload = DomainEventPayload::ReleaseBlocklisted(ReleaseBlocklistedEventData {
            title: title_snapshot,
            source_title: normalized_source_title,
            source_hint: normalized_source_hint,
            download_id,
            client_id: normalized_client_id,
            client_name: normalized_client_name,
            client_type: normalized_client_type,
            quality,
            reason: Some(reason),
            episode_ids: attribution.episode_ids.clone(),
            collection_id: attribution.collection_id.clone(),
        });
        let _ = app
            .append_domain_event(title_scoped_domain_event(title_id, title, payload))
            .await;
    }
}
/// Record one completed client item in the snapshot's completed sets. Both the
/// queue listing (a finished item the client still shows) and the history
/// listing report completed items — often the same item — so the insert-guard
/// keeps the raw-id count a per-item count rather than a per-listing count.
fn note_completed_item(
    completed_client_ids: &mut std::collections::HashSet<String>,
    completed_raw_item_id_counts: &mut std::collections::HashMap<String, usize>,
    client_id: &str,
    download_client_item_id: &str,
) {
    if completed_client_ids.insert(download_client_item_identity(
        Some(client_id),
        download_client_item_id,
    )) {
        *completed_raw_item_id_counts
            .entry(download_client_item_id.to_string())
            .or_insert(0) += 1;
    }
}

/// A history-listed job that is still being worked on. Mirrors the queue leg's
/// live-claim set minus `ImportPending` (a history item awaiting import lists
/// as `Completed`), so a job the client reports from either listing claims its
/// scope the same way.
fn history_item_is_live_claim(state: DownloadQueueState) -> bool {
    matches!(
        state,
        DownloadQueueState::Queued
            | DownloadQueueState::Downloading
            | DownloadQueueState::Paused
            | DownloadQueueState::Verifying
            | DownloadQueueState::Repairing
            | DownloadQueueState::Extracting
            | DownloadQueueState::ImportPending
            | DownloadQueueState::Warning
    )
}

impl DownloadClientSnapshot {
    pub(crate) async fn fetch(app: &AppUseCase) -> Self {
        let mut active_titles = std::collections::HashSet::new();
        let mut active_client_ids = std::collections::HashSet::new();
        let mut active_raw_item_id_counts = std::collections::HashMap::new();
        let mut stale_downloading_client_ids = std::collections::HashSet::new();
        let mut stale_downloading_raw_item_ids = std::collections::HashSet::new();
        let mut completed_client_ids = std::collections::HashSet::new();
        let mut completed_raw_item_id_counts = std::collections::HashMap::new();
        let mut failed_by_download_id = std::collections::HashMap::new();
        let mut queue_listing_failed = false;
        let mut history_listing_failed = false;
        let mut unreadable_client_ids = std::collections::HashSet::new();
        let now = chrono::Utc::now();

        // Fetch current queue
        match app
            .services
            .integrations
            .download_client
            .list_queue_with_read_report()
            .await
        {
            Ok(listing) => {
                let queue = note_unreadable_clients(
                    listing,
                    "queue",
                    &mut unreadable_client_ids,
                    &mut queue_listing_failed,
                );
                for item in &queue {
                    match item.state {
                        DownloadQueueState::Queued
                        | DownloadQueueState::Downloading
                        | DownloadQueueState::Paused
                        // Post-download client work (verify/repair/extract) and
                        // a completed download awaiting import are both live
                        // claims on their scope: the bytes exist and are on
                        // their way to becoming the file.
                        | DownloadQueueState::Verifying
                        | DownloadQueueState::Repairing
                        | DownloadQueueState::Extracting
                        | DownloadQueueState::ImportPending
                        // A warned download is live work the client is still
                        // holding, so the double-submit guard has to see it;
                        // otherwise an automatic search grabs a second copy
                        // behind a torrent that nothing will clean up.
                        | DownloadQueueState::Warning => {
                            active_titles.insert(item.title_name.to_ascii_lowercase());
                            active_client_ids.insert(download_client_item_identity(
                                Some(item.client_id.as_str()),
                                &item.download_client_item_id,
                            ));
                            *active_raw_item_id_counts
                                .entry(item.download_client_item_id.clone())
                                .or_insert(0) += 1;
                            if item.state == DownloadQueueState::Downloading
                                && item
                                    .queued_at
                                    .as_deref()
                                    .and_then(crate::quality_profile::parse_published_at)
                                    .is_some_and(|queued_at| {
                                        now.signed_duration_since(queued_at).num_seconds()
                                            >= STALE_INITIAL_CLAIM_SECONDS
                                    })
                            {
                                stale_downloading_client_ids.insert(
                                    download_client_item_identity(
                                        Some(item.client_id.as_str()),
                                        &item.download_client_item_id,
                                    ),
                                );
                                stale_downloading_raw_item_ids
                                    .insert(item.download_client_item_id.clone());
                            }
                        }
                        // A finished download still sitting in the client's
                        // queue view (Weaver holds items there until import
                        // removes them) is bytes on disk awaiting import — the
                        // double-submit guard must see it as a claim, not as
                        // "gone". History listing below also reports completed
                        // items; the insert-guard keeps the raw-id count a
                        // per-item count rather than a per-listing count.
                        DownloadQueueState::Completed => {
                            note_completed_item(
                                &mut completed_client_ids,
                                &mut completed_raw_item_id_counts,
                                &item.client_id,
                                &item.download_client_item_id,
                            );
                        }
                        _ => {}
                    }
                }
                if !active_titles.is_empty() {
                    info!(
                        active_count = active_titles.len(),
                        "download client snapshot: active queue items"
                    );
                }
            }
            Err(error) => {
                queue_listing_failed = true;
                warn!(
                    error = %error,
                    "download client snapshot: queue listing failed; treating queue as possibly-active to avoid blind double-submits"
                );
            }
        }

        // Fetch recent history — key by download client job ID (works across all
        // clients: NZBGet, SABnzbd, Weaver).
        match app
            .services
            .integrations
            .download_client
            .list_history_with_read_report()
            .await
        {
            Ok(listing) => {
                let history = note_unreadable_clients(
                    listing,
                    "history",
                    &mut unreadable_client_ids,
                    &mut history_listing_failed,
                );
                for item in &history {
                    if item.state == DownloadQueueState::Completed {
                        note_completed_item(
                            &mut completed_client_ids,
                            &mut completed_raw_item_id_counts,
                            &item.client_id,
                            &item.download_client_item_id,
                        );
                    } else if history_item_is_live_claim(item.state) {
                        // SABnzbd moves a job into *history* the moment the
                        // download finishes and post-processes it there
                        // (verify → repair → unpack → move). For those minutes
                        // the job is in neither the queue leg above nor the
                        // completed set, so this leg used to report the scope
                        // as free: every RSS pass in that window grabbed the
                        // same release again. Post-processing is a live claim.
                        active_titles.insert(item.title_name.to_ascii_lowercase());
                        active_client_ids.insert(download_client_item_identity(
                            Some(item.client_id.as_str()),
                            &item.download_client_item_id,
                        ));
                        *active_raw_item_id_counts
                            .entry(item.download_client_item_id.clone())
                            .or_insert(0) += 1;
                    } else if item.state == DownloadQueueState::Failed {
                        // `Warning` is not a failure: it stays out of this map
                        // so failure recovery never fires on a download the
                        // client can still finish.
                        let reason = item
                            .attention_reason
                            .as_deref()
                            .unwrap_or("unknown")
                            .to_ascii_uppercase();
                        failed_by_download_id.insert(
                            download_client_item_identity(
                                Some(item.client_id.as_str()),
                                &item.download_client_item_id,
                            ),
                            FailedDownloadSnapshot {
                                reason,
                                download_client_item_id: item.download_client_item_id.clone(),
                                client_id: item.client_id.clone(),
                                client_name: normalized_non_empty_owned(Some(
                                    item.client_name.clone(),
                                )),
                            },
                        );
                    }
                }
                if !failed_by_download_id.is_empty() {
                    debug!(
                        failed_count = failed_by_download_id.len(),
                        "download client snapshot: failed history items"
                    );
                }
            }
            Err(error) => {
                history_listing_failed = true;
                warn!(
                    error = %error,
                    "download client snapshot: history listing failed; failure detection is skipped this cycle"
                );
            }
        }

        Self {
            active_titles,
            active_client_ids,
            active_raw_item_id_counts,
            stale_downloading_client_ids,
            stale_downloading_raw_item_ids,
            completed_client_ids,
            completed_raw_item_id_counts,
            failed_by_download_id,
            queue_listing_failed,
            history_listing_failed,
            unreadable_client_ids,
        }
    }

    /// Whether the client holding this submission could not be read this
    /// cycle. A submission that names no client is judged against every
    /// client, so any unreadable client makes its absence unprovable.
    pub(crate) fn client_unreadable(&self, client_id: Option<&str>) -> bool {
        match client_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(client_id) => self.unreadable_client_ids.contains(client_id),
            None => !self.unreadable_client_ids.is_empty(),
        }
    }

    /// Returns true if a release with this title is currently
    /// queued/downloading, or if the queue could not be observed this cycle (an
    /// unknown queue is treated as possibly-active so automatic grabs skip/defer
    /// instead of double-submitting blind).
    pub(crate) fn is_active(&self, release_title: &str) -> bool {
        self.queue_listing_failed
            || self
                .active_titles
                .contains(&release_title.to_ascii_lowercase())
    }

    /// Whether the queue could not be listed while building this snapshot.
    /// Callers that would otherwise expire a release on an "already active"
    /// signal must instead defer, since the signal here is "unknown", not
    /// "confirmed active".
    pub(crate) fn queue_listing_failed(&self) -> bool {
        self.queue_listing_failed
    }

    /// If a download with this job ID failed in history with a blocklist-worthy
    /// reason, returns the failure snapshot.
    pub(crate) fn failed_item(
        &self,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> Option<&FailedDownloadSnapshot> {
        // Failure detection reads only history; if it could not be observed we
        // report no failures rather than acting on an incomplete map.
        if self.history_listing_failed {
            return None;
        }
        self.failed_by_download_id
            .get(&download_client_item_identity(
                client_id,
                download_client_item_id,
            ))
            .or_else(|| self.failed_by_download_id.get(download_client_item_id))
    }

    fn has_active_client_item(
        &self,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> bool {
        if self.queue_listing_failed {
            return true;
        }
        let exact_key = download_client_item_identity(client_id, download_client_item_id);
        self.active_client_ids.contains(&exact_key)
            || self.active_raw_item_id_counts.get(download_client_item_id) == Some(&1)
    }

    /// Whether this item's live claim has gone stale: still `Downloading`, but
    /// for longer than [`STALE_INITIAL_CLAIM_SECONDS`]. Mirrors
    /// `has_active_client_item`'s two-way identity match; a blind cycle is
    /// never stale (possibly-active must keep suppressing).
    pub(crate) fn active_downloading_is_stale(
        &self,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> bool {
        if self.queue_listing_failed {
            return false;
        }
        let exact_key = download_client_item_identity(client_id, download_client_item_id);
        self.stale_downloading_client_ids.contains(&exact_key)
            || (self.active_raw_item_id_counts.get(download_client_item_id) == Some(&1)
                && self
                    .stale_downloading_raw_item_ids
                    .contains(download_client_item_id))
    }

    fn has_completed_client_item(
        &self,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> bool {
        let exact_key = download_client_item_identity(client_id, download_client_item_id);
        self.completed_client_ids.contains(&exact_key)
            || self
                .completed_raw_item_id_counts
                .get(download_client_item_id)
                == Some(&1)
    }
}
pub(crate) fn submission_is_active(
    submission: &DownloadSubmission,
    dl_snapshot: &DownloadClientSnapshot,
) -> bool {
    dl_snapshot.has_active_client_item(
        submission.download_client_id.as_deref(),
        &submission.download_client_item_id,
    )
}
/// Whether a submission is in flight for admission purposes (D18 liveness).
///
/// Sonarr's `QueueSpecification` boundary, stated once so the filter and its
/// tests cannot drift:
///
/// - `Downloading | ImportPending | Importing` — in the client, obviously live.
/// - `ImportBlocked` — **live**. The bytes exist and are a real claim on the
///   scope, so an equal or worse release must not be fetched beside them. A
///   *better* one still may, which is what makes a stuck import stop freezing
///   its scope permanently.
/// - `FailedPending` — **not** live, exactly as Sonarr excludes it, so a
///   replacement can be grabbed. The convergence lane keeps its own hard skip
///   for that state until the failure handler has run.
/// - `Imported | ImportedSeeding | Failed | Ignored` — over.
/// - No tracked row at all: fall back to what the client says.
pub(crate) fn submission_is_queued(
    tracked_state: Option<scryer_domain::TrackedDownloadState>,
    snapshot_active: bool,
) -> bool {
    use scryer_domain::TrackedDownloadState;
    match tracked_state {
        Some(
            TrackedDownloadState::Downloading
            | TrackedDownloadState::ImportPending
            | TrackedDownloadState::Importing
            | TrackedDownloadState::ImportBlocked,
        ) => true,
        Some(_) => false,
        None => snapshot_active,
    }
}

fn submission_is_completed(
    submission: &DownloadSubmission,
    dl_snapshot: &DownloadClientSnapshot,
) -> bool {
    dl_snapshot.has_completed_client_item(
        submission.download_client_id.as_deref(),
        &submission.download_client_item_id,
    )
}

/// The D18 liveness question for a submission with the snapshot in hand: does
/// this submission still hold a claim on its scope?
///
/// The tracked-state ledger is authoritative when it has an entry, but it can
/// lag a full import behind the client — the row often appears only when the
/// import finishes. Until then the client snapshot stands in, and a download
/// the client reports as *completed* is exactly as much a claim as one it is
/// still downloading: the bytes exist and are on their way to becoming the
/// file. Without the completed leg, every scope is unprotected for the whole
/// import window and an automatic search grabs a second copy beside it.
///
/// The same holds when the submission's client could not be read at all: the
/// aggregate router degrades a dead or backed-off client to an empty view, so
/// "not in the queue and not in history" is only evidence of absence when the
/// client actually answered. An unreadable client keeps its claims.
pub(crate) fn submission_is_live_claim(
    submission: &DownloadSubmission,
    tracked_state: Option<scryer_domain::TrackedDownloadState>,
    dl_snapshot: &DownloadClientSnapshot,
) -> bool {
    submission_is_queued(
        tracked_state,
        submission_is_active(submission, dl_snapshot)
            || submission_is_completed(submission, dl_snapshot)
            || dl_snapshot.client_unreadable(submission.download_client_id.as_deref()),
    )
}
/// Check grabbed wanted items against the download client. If a grabbed
/// release has failed in the download client, blocklist it and re-queue the
/// wanted item for immediate re-search.
async fn check_grabbed_for_failures(app: &AppUseCase, dl_snapshot: &DownloadClientSnapshot) {
    let grabbed_items = match app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            statuses: vec!["grabbed".into()],
            limit: 200,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
    {
        Ok(items) => items,
        Err(err) => {
            warn!(error = %err, "failed to list grabbed wanted items for failure check");
            return;
        }
    };

    if grabbed_items.is_empty() {
        debug!("check_grabbed_for_failures: no grabbed wanted items");
        return;
    }

    debug!(
        count = grabbed_items.len(),
        "check_grabbed_for_failures: checking grabbed wanted items against download client"
    );

    let mut submissions_by_title = HashMap::new();
    let mut episodes_by_title = HashMap::new();
    let mut processed_failed_submissions = HashSet::new();

    for item in &grabbed_items {
        // Extract the grabbed release title from the stored JSON (for logging/blocklist)
        let release_title = item
            .grabbed_release
            .as_deref()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .and_then(|v| v.get("title").and_then(|t| t.as_str().map(String::from)))
            .unwrap_or_default();

        // Look up the download submission to find the download client job ID.
        // Match by job ID (works across all clients) instead of title name
        // (which gets sanitized differently by each client).
        let submissions = if let Some(cached) = submissions_by_title.get(&item.title_id) {
            cached
        } else {
            let fetched = match app
                .services
                .workflow
                .download_submissions
                .list_for_title(&item.title_id)
                .await
            {
                Ok(submissions) => submissions,
                Err(err) => {
                    warn!(
                        error = %err,
                        title_id = item.title_id.as_str(),
                        "failed to list submissions for grabbed wanted item title"
                    );
                    Vec::new()
                }
            };

            trace!(
                title_id = item.title_id.as_str(),
                release = release_title.as_str(),
                submission_count = fetched.len(),
                submission_ids = ?fetched.iter().map(|s| s.download_client_item_id.as_str()).collect::<Vec<_>>(),
                "check_grabbed_for_failures: looking up submissions for grabbed title"
            );

            submissions_by_title.insert(item.title_id.clone(), fetched);
            submissions_by_title
                .get(&item.title_id)
                .expect("title submissions cache entry should exist")
        };

        let episode_collection_id = if item.episode_id.is_some()
            && submissions
                .iter()
                .any(|submission| matches!(&submission.scope, SubmissionScope::Collection { .. }))
        {
            let episodes = if let Some(cached) = episodes_by_title.get(&item.title_id) {
                cached
            } else {
                let fetched = match app
                    .services
                    .catalog
                    .shows
                    .list_episodes_for_title(&item.title_id)
                    .await
                {
                    Ok(episodes) => episodes,
                    Err(err) => {
                        warn!(
                            error = %err,
                            title_id = item.title_id.as_str(),
                            "failed to list episodes while matching failed download submissions"
                        );
                        Vec::new()
                    }
                };
                episodes_by_title.insert(item.title_id.clone(), fetched);
                episodes_by_title
                    .get(&item.title_id)
                    .expect("title episodes cache entry should exist")
            };
            item.episode_id.as_ref().and_then(|episode_id| {
                episodes
                    .iter()
                    .find(|episode| &episode.id == episode_id)
                    .and_then(|episode| episode.collection_id.as_deref())
            })
        } else {
            None
        };
        let preferred_release = normalize_release_name(Some(release_title.as_str()));
        let mut failed = submissions
            .iter()
            .filter(|submission| {
                submission_blocks_wanted_item(submission, item, episode_collection_id)
            })
            .filter_map(|submission| {
                dl_snapshot
                    .failed_item(
                        submission.download_client_id.as_deref(),
                        &submission.download_client_item_id,
                    )
                    .map(|failed_item| (failed_item, submission))
            })
            .collect::<Vec<_>>();
        failed.sort_by_key(|(_, submission)| {
            preferred_release.is_none()
                || normalize_release_name(submission.source_title.as_deref()) != preferred_release
        });

        for (failed_item, submission) in failed {
            let failure_key = format!(
                "{}:{}:{}",
                submission.download_client_id.as_deref().unwrap_or(""),
                submission.download_client_type,
                submission.download_client_item_id
            );
            if !processed_failed_submissions.insert(failure_key.clone()) {
                debug!(
                    title_id = item.title_id.as_str(),
                    failure_key = failure_key.as_str(),
                    "skipping duplicate failed submission for covered grabbed set"
                );
                continue;
            }

            let release_title = submission
                .source_title
                .clone()
                .unwrap_or_else(|| release_title.clone());
            warn!(
                title_id = item.title_id.as_str(),
                release = release_title.as_str(),
                reason = failed_item.reason.as_str(),
                "grabbed release failed in download client"
            );

            let outcome = process_download_failure(
                app,
                DownloadFailureContext {
                    wanted_item: Some(item.clone()),
                    title_id: Some(item.title_id.clone()),
                    client_id: failed_item.client_id.clone(),
                    client_type: submission.download_client_type.clone(),
                    client_name: failed_item.client_name.clone(),
                    client_item_id: failed_item.download_client_item_id.clone(),
                    release_title: release_title.clone(),
                    reason: failed_item.reason.clone(),
                    remove_from_client_if_configured: true,
                    skip_reacquire: false,
                },
            )
            .await;
            if outcome != FailureHandlingOutcome::AlreadyHandled {
                break;
            }
        }
    }
}
async fn find_failed_submission_for_download(
    app: &AppUseCase,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    context: &DownloadFailureContext,
) -> Option<DownloadSubmission> {
    app.services
        .workflow
        .download_submissions
        .find_by_client_item_id_for_download(
            canonical_download_id,
            &ClientJobLocator::new(
                Some(context.client_id.as_str()),
                &context.client_type,
                &context.client_item_id,
            ),
        )
        .await
        .ok()
        .flatten()
}
fn preferred_failed_release_title(
    context: &DownloadFailureContext,
    failed_submission: Option<&DownloadSubmission>,
) -> Option<String> {
    failed_submission
        .and_then(|submission| normalized_non_empty_owned(submission.source_title.clone()))
        .or_else(|| {
            // Client-echoed fallback only, never the grab-time name: SABnzbd
            // reports `<release>.nzb`, and a blocklist row keyed on that suffix
            // can never match a search-time candidate name.
            normalized_non_empty_owned(Some(strip_nzb_suffix(&context.release_title).to_string()))
        })
}

/// One trailing `.nzb`, ASCII case-insensitive, off a client-reported name.
fn strip_nzb_suffix(release_title: &str) -> &str {
    let trimmed = release_title.trim_end();
    trimmed
        .get(trimmed.len().saturating_sub(4)..)
        .filter(|suffix| suffix.eq_ignore_ascii_case(".nzb"))
        .map_or(trimmed, |_| &trimmed[..trimmed.len() - 4])
}
async fn resolve_failed_pack_episode_wanted_items(
    app: &AppUseCase,
    submission: &DownloadSubmission,
) -> AppResult<Vec<AcquisitionScopeState>> {
    let episode_ids: HashSet<String> = match &submission.scope {
        SubmissionScope::Collection { collection_id } => app
            .services
            .catalog
            .shows
            .list_episodes_for_collection(collection_id)
            .await?
            .into_iter()
            .map(|episode| episode.id)
            .collect(),
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids.iter().cloned().collect(),
        _ => return Ok(Vec::new()),
    };

    if episode_ids.is_empty() {
        return Ok(Vec::new());
    }

    let wanted_items = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states_for_title_ids(std::slice::from_ref(&submission.title_id))
        .await?;

    Ok(wanted_items
        .into_iter()
        .filter(|item| {
            matches!(
                item.status,
                AcquisitionScopeStatus::Wanted | AcquisitionScopeStatus::Grabbed
            ) && item.media_type == "episode"
                && item
                    .episode_id
                    .as_ref()
                    .is_some_and(|episode_id| episode_ids.contains(episode_id))
        })
        .collect())
}
/// Record a client-side failed download.
///
/// Automatic grabs re-open their existing scope coverage so the cursor can
/// walk saved candidates. Operator grabs are deliberately record-only: they
/// are outside the automatic acquisition loop and must not trigger recovery.
pub(crate) async fn process_download_failure(
    app: &AppUseCase,
    context: DownloadFailureContext,
) -> FailureHandlingOutcome {
    process_download_failure_for_download(app, None, context).await
}

pub(crate) async fn process_download_failure_for_download(
    app: &AppUseCase,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    context: DownloadFailureContext,
) -> FailureHandlingOutcome {
    let failed_submission =
        find_failed_submission_for_download(app, canonical_download_id, &context).await;
    if context.wanted_item.is_none() && failed_submission.is_none() {
        info!(
            client_id = context.client_id.as_str(),
            client_type = context.client_type.as_str(),
            download_client_item_id = context.client_item_id.as_str(),
            release_title = context.release_title.as_str(),
            "skipping automatic failed download handling without scryer grab history"
        );
        return FailureHandlingOutcome::RecordedOnly;
    }

    let resolved_title_id = context
        .wanted_item
        .as_ref()
        .map(|item| item.title_id.clone())
        .or(context.title_id.clone())
        .or_else(|| {
            failed_submission
                .as_ref()
                .map(|submission| submission.title_id.clone())
        });
    let download_id = normalized_non_empty_owned(Some(context.client_item_id.clone()));
    let preferred_source_title =
        preferred_failed_release_title(&context, failed_submission.as_ref());
    let normalized_source_title = normalize_release_name(preferred_source_title.as_deref());
    let normalized_source_hint = resolved_failed_release_hint(failed_submission.as_ref());
    // Both halves of the blocklist key come off the submission: source_provider_id
    // has carried the indexer id since 0150, info_hash since 0192.
    let blocklist_indexer_id = failed_submission
        .as_ref()
        .and_then(|submission| submission.source_provider_id.clone())
        .unwrap_or_default();
    let blocklist_info_hash = failed_submission
        .as_ref()
        .and_then(|submission| submission.info_hash.clone());
    let quality = failed_submission
        .as_ref()
        .and_then(|submission| release_quality_hint(submission.source_title.as_deref()))
        .or_else(|| release_quality_hint(Some(context.release_title.as_str())));
    let release_title_for_matching = preferred_source_title
        .as_deref()
        .unwrap_or(context.release_title.as_str());
    let _failure_guard = app
        .runtime
        .acquisition
        .download_failure_guards
        .acquire_release_or_client_item(
            resolved_title_id.as_deref(),
            normalized_source_title.as_deref(),
            &context.client_id,
            &context.client_type,
            &context.client_item_id,
        )
        .await;

    let failure_already_recorded = if let Some(title_id) = resolved_title_id.as_deref() {
        match app
            .services
            .workflow
            .blocklist_repo
            .is_blocked(
                title_id,
                &blocklist_indexer_id,
                normalized_source_title.as_deref().unwrap_or_default(),
                blocklist_info_hash.as_deref(),
            )
            .await
        {
            Ok(true) => {
                info!(
                    title_id,
                    client_id = context.client_id.as_str(),
                    client_type = context.client_type.as_str(),
                    download_client_item_id = context.client_item_id.as_str(),
                    release_title = release_title_for_matching,
                    "skipping duplicate failed download handling; failure already recorded"
                );
                true
            }
            Ok(false) => false,
            Err(error) => {
                warn!(
                    title_id,
                    client_id = context.client_id.as_str(),
                    client_type = context.client_type.as_str(),
                    download_client_item_id = context.client_item_id.as_str(),
                    error = %error,
                    "failed to check for duplicate failed download blocklist entry"
                );
                false
            }
        }
    } else {
        false
    };

    if failure_already_recorded && !context.skip_reacquire {
        return FailureHandlingOutcome::AlreadyHandled;
    }

    let operator_submission = failed_submission.as_ref().is_some_and(|submission| {
        submission.purpose.is_operator_queued() || submission.purpose.is_manual_replacement()
    });

    let failed_pack_items = if let Some(submission) = failed_submission.as_ref() {
        match resolve_failed_pack_episode_wanted_items(app, submission).await {
            Ok(items) if !items.is_empty() => Some(items),
            Ok(_) => None,
            Err(err) => {
                warn!(
                    title_id = submission.title_id.as_str(),
                    download_client_item_id = context.client_item_id.as_str(),
                    error = %err,
                    "failed to resolve wanted items for pack-scoped download failure"
                );
                None
            }
        }
    } else {
        None
    };

    let wanted_item = match context.wanted_item.clone() {
        Some(item) => Some(item),
        None if failed_pack_items.is_none() && failed_submission.is_some() => {
            resolve_failure_wanted_item(
                app,
                resolved_title_id.as_deref(),
                release_title_for_matching,
            )
            .await
        }
        None => None,
    };
    let attribution = resolve_failed_release_attribution(
        app,
        resolved_title_id.as_deref(),
        failed_submission.as_ref(),
        wanted_item.as_ref(),
        failed_pack_items.as_deref(),
    )
    .await;

    let blocklist_reason = format!("download client failure: {}", context.reason);
    let mut failure_recorded = false;

    let (outcome, failure_reason) = if operator_submission {
        (
            FailureHandlingOutcome::RecordedOnly,
            format!(
                "operator download failed for '{}': {}; recorded without automatic recovery",
                release_title_for_matching, context.reason
            ),
        )
    } else if context.skip_reacquire {
        if let Some(items) = failed_pack_items.as_ref() {
            let mut update_error = None;
            for item in items {
                if let Err(err) = mark_wanted_item_failed_without_reacquire(app, item).await {
                    update_error.get_or_insert_with(|| err.to_string());
                }
            }
            if let Some(err) = update_error {
                (
                    FailureHandlingOutcome::RecordedOnly,
                    format!(
                        "pack download failed for '{}': {}; failed to disable reacquisition: {}",
                        release_title_for_matching, context.reason, err
                    ),
                )
            } else {
                (
                    FailureHandlingOutcome::RecordedNoReacquire,
                    format!(
                        "pack download failed for '{}': {}; recorded failure without reacquisition",
                        release_title_for_matching, context.reason
                    ),
                )
            }
        } else if let Some(item) = wanted_item.as_ref() {
            match mark_wanted_item_failed_without_reacquire(app, item).await {
                Ok(()) => (
                    FailureHandlingOutcome::RecordedNoReacquire,
                    format!(
                        "download failed for '{}': {}; recorded failure without reacquisition",
                        release_title_for_matching, context.reason
                    ),
                ),
                Err(err) => (
                    FailureHandlingOutcome::RecordedOnly,
                    format!(
                        "download failed for '{}': {}; failed to disable reacquisition: {}",
                        release_title_for_matching, context.reason, err
                    ),
                ),
            }
        } else {
            (
                FailureHandlingOutcome::RecordedNoReacquire,
                format!(
                    "download failed: {} — {}; recorded failure without reacquisition",
                    release_title_for_matching, context.reason
                ),
            )
        }
    } else if let Some(items) = failed_pack_items.as_ref() {
        let message = format!(
            "pack download failed for '{}': {}; re-opened covered episodes under existing coverage",
            release_title_for_matching, context.reason
        );
        record_failed_release_outcome(
            app,
            resolved_title_id.as_deref(),
            &attribution,
            normalized_source_title.clone(),
            normalized_source_hint.clone(),
            blocklist_indexer_id.clone(),
            blocklist_info_hash.clone(),
            download_id.clone(),
            Some(context.client_id.clone()),
            context.client_name.clone(),
            Some(context.client_type.clone()),
            quality.clone(),
            Some(message.clone()),
            Some(blocklist_reason.clone()),
            None,
        )
        .await;
        failure_recorded = true;
        // A failed pack re-opens every covered episode scope after its
        // release is blocklisted. Coverage is kept: the cursor walks each
        // scope's saved search results before it would query an indexer.
        for item in items {
            app.reopen_wanted_scope_for_acquisition(item, CoverageReopen::Keep)
                .await;
        }

        info!(
            title_id = resolved_title_id.as_deref().unwrap_or(""),
            affected_wanted_items = items.len(),
            release_title = release_title_for_matching,
            "re-opened covered episode scopes after failed pack download"
        );

        (FailureHandlingOutcome::Reopened, message)
    } else if let Some(item) = wanted_item.as_ref() {
        let failure_reason = format!(
            "download failed for '{}': {}; re-opened scope to try its saved search results",
            release_title_for_matching, context.reason
        );
        record_failed_release_outcome(
            app,
            resolved_title_id.as_deref(),
            &attribution,
            normalized_source_title.clone(),
            normalized_source_hint.clone(),
            blocklist_indexer_id.clone(),
            blocklist_info_hash.clone(),
            download_id.clone(),
            Some(context.client_id.clone()),
            context.client_name.clone(),
            Some(context.client_type.clone()),
            quality.clone(),
            Some(failure_reason.clone()),
            Some(blocklist_reason.clone()),
            None,
        )
        .await;
        failure_recorded = true;
        // Blocklisted first, then re-opened under its existing coverage: the
        // cursor walks the scope's saved search results (`try_saved_candidates`)
        // before it would spend an indexer query, and a scope whose saved results
        // are exhausted simply stays converged. Never a coverage prune here — a
        // failure must not cost a re-search.
        app.reopen_wanted_scope_for_acquisition(item, CoverageReopen::Keep)
            .await;

        (FailureHandlingOutcome::Reopened, failure_reason)
    } else {
        (
            FailureHandlingOutcome::RecordedOnly,
            format!(
                "download failed: {} — {}",
                release_title_for_matching, context.reason
            ),
        )
    };

    if !failure_already_recorded && !failure_recorded {
        record_failed_release_outcome(
            app,
            resolved_title_id.as_deref(),
            &attribution,
            normalized_source_title.clone(),
            normalized_source_hint.clone(),
            blocklist_indexer_id.clone(),
            blocklist_info_hash.clone(),
            download_id.clone(),
            Some(context.client_id.clone()),
            context.client_name.clone(),
            Some(context.client_type.clone()),
            quality,
            Some(failure_reason),
            Some(blocklist_reason),
            None,
        )
        .await;
    }

    if context.remove_from_client_if_configured
        && let Some(title) = attribution.title.as_ref()
        && app
            .should_remove_failed_download(
                Some(title.library_id.as_str()),
                &title.facet,
                &context.client_id,
            )
            .await
        && let Err(error) = app
            .services
            .integrations
            .download_client
            // History cleanup for a download the client itself failed: the
            // entry goes, what the client kept is left to the client.
            .delete_queue_item_for_client_id(
                &context.client_id,
                &context.client_item_id,
                true,
                false,
            )
            .await
    {
        warn!(
            title_id = resolved_title_id.as_deref().unwrap_or(""),
            client_id = context.client_id.as_str(),
            download_client_item_id = context.client_item_id.as_str(),
            error = %error,
            "failed to delete failed download from client history"
        );
    }

    let _ = app
        .services
        .workflow
        .download_submissions
        .update_tracked_state(
            &ClientJobLocator::new(
                Some(context.client_id.as_str()),
                &context.client_type,
                &context.client_item_id,
            ),
            scryer_domain::TrackedDownloadState::Failed.as_str(),
        )
        .await;

    outcome
}
async fn resolve_failure_wanted_item(
    app: &AppUseCase,
    title_id: Option<&str>,
    release_title: &str,
) -> Option<AcquisitionScopeState> {
    let title_id = title_id?.trim();
    if title_id.is_empty() {
        return None;
    }

    let grabbed_items = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            statuses: vec!["grabbed".into()],
            title_id: Some(title_id.to_string()),
            limit: 25,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
        .ok()?;

    if grabbed_items.len() == 1 {
        return grabbed_items.into_iter().next();
    }

    grabbed_items.into_iter().find(|item| {
        extract_grabbed_release_title(item.grabbed_release.as_deref())
            .is_some_and(|title| title.eq_ignore_ascii_case(release_title))
    })
}

/// Drop saved search results whose scope no longer needs them.
///
/// A scope keeps its saved results while it is `wanted` (the cursor tries them
/// before spending an indexer query) or `grabbed` (they are the fallback if that
/// grab fails). Anything else — completed, paused, or removed — has no use for
/// them. There is deliberately no age or count limit: the whole ranked list is
/// kept until one of its releases lands or every one of them has been tried.
async fn prune_standby_candidates(app: &AppUseCase) {
    let all_standby = app
        .services
        .workflow
        .pending_releases
        .list_all_standby_pending_releases()
        .await
        .unwrap_or_default();

    if all_standby.is_empty() {
        return;
    }

    let wanted_item_ids: std::collections::HashSet<String> = all_standby
        .into_iter()
        .map(|release| release.wanted_item_id)
        .collect();

    for wanted_item_id in wanted_item_ids {
        let wanted = app
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(&wanted_item_id)
            .await
            .ok()
            .flatten();
        let still_useful = wanted.as_ref().is_some_and(|wanted| {
            matches!(
                wanted.status,
                AcquisitionScopeStatus::Wanted | AcquisitionScopeStatus::Grabbed
            )
        });
        if !still_useful {
            let _ = app
                .services
                .workflow
                .pending_releases
                .delete_standby_pending_releases_for_wanted_item(&wanted_item_id)
                .await;
        }
    }
}
/// Walk a scope's saved search results, best first, and grab the first one
/// that is still eligible.
///
/// Every row is re-judged now, not when it was saved: against the title's
/// blocklist, the download client's current queue, and — through
/// `try_grab_pending_release` — the swarm and admission policy. Rows that no
/// longer qualify are expired and skipped; the rows after a successful grab stay
/// `Standby`, so if that grab fails too the walk continues down the same list.
pub(crate) async fn try_saved_candidates(
    app: &AppUseCase,
    item: &AcquisitionScopeState,
    failed_release_title: Option<&str>,
    excluded_episode_ids: Option<&HashSet<String>>,
    dl_snapshot: &DownloadClientSnapshot,
    now: &DateTime<Utc>,
) -> StandbyRecoveryOutcome {
    // A waiting row is already the chosen best candidate. Never claim a
    // lower-ranked standby release while the delay-promotion lane owns it.
    if !app
        .services
        .workflow
        .pending_releases
        .list_pending_releases_for_wanted_item(&item.id)
        .await
        .unwrap_or_default()
        .is_empty()
    {
        return StandbyRecoveryOutcome::Parked { scope: None };
    }

    let mut standby_releases = app
        .services
        .workflow
        .pending_releases
        .list_standby_pending_releases_for_wanted_item(&item.id)
        .await
        .unwrap_or_default();

    let mut season_pack_ids = HashSet::new();
    let mut series_pack_ids = HashSet::new();
    let mut standby_scopes = HashMap::new();
    // Title-wide standby rows only matter to an episode: movies and other
    // non-episode scopes cannot be covered by a sibling season pack, so avoid
    // the title-wide read and parse work entirely for them.
    if let Some(episode_id) = item.episode_id.as_deref() {
        let target_episode = app
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await
            .ok()
            .flatten();
        // Season-pack rows belong to the anchor episode that found the pack,
        // but every covered episode must be able to continue that same ranked
        // list. Parse each title-wide standby row once, retaining its coverage
        // and pack classification for both the merge and ordering passes.
        let title = app
            .services
            .catalog
            .titles
            .get_by_id(&item.title_id)
            .await
            .ok()
            .flatten();
        if let (Some(target_episode), Some(title)) = (target_episode, title) {
            let catalog_episodes = app
                .services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await
                .unwrap_or_default();
            let catalog_collections = app
                .services
                .catalog
                .shows
                .list_collections_for_title(&title.id)
                .await
                .unwrap_or_default();
            let parse_context = crate::release_parser::build_release_parse_context_for_title(
                &title,
                &catalog_episodes,
                Some(title.facet.as_str()),
            );
            let parse_coverage = |pending: &PendingRelease| {
                let parsed = crate::release_parser::parse_release_metadata_for_target(
                    &pending.release_title,
                    &parse_context,
                );
                let is_season_pack = parsed
                    .episode
                    .as_ref()
                    .is_some_and(|episode| episode.full_season);
                let is_series_pack = parsed
                    .episode
                    .as_ref()
                    .is_some_and(|episode| episode.is_series_pack);
                let coverage = crate::acquisition_coverage::resolve_release_coverage(
                    &parsed,
                    &catalog_episodes,
                    &catalog_collections,
                    None,
                );
                let covers_item = coverage.covers_episode(&target_episode);
                let scope = coverage.submission_scope_or(&item.submission_scope());
                (covers_item, is_season_pack, is_series_pack, scope)
            };

            let title_pending = app
                .services
                .workflow
                .pending_releases
                .list_pending_releases_for_title(&item.title_id)
                .await
                .unwrap_or_default();
            if let Some(scope) = title_pending.iter().find_map(|pending| {
                let metadata = parse_coverage(pending);
                (pending.status == PendingReleaseStatus::Waiting
                    && pending.wanted_item_id != item.id
                    && metadata.0)
                    .then_some(metadata.3)
            }) {
                return StandbyRecoveryOutcome::Parked { scope: Some(scope) };
            }

            let mut known_ids = standby_releases
                .iter()
                .map(|pending| pending.id.clone())
                .collect::<HashSet<_>>();
            let title_standby = app
                .services
                .workflow
                .pending_releases
                .list_standby_pending_releases_for_title(&item.title_id)
                .await
                .unwrap_or_default();
            let mut title_standby_metadata = std::collections::HashMap::new();
            for pending in title_standby {
                let metadata = parse_coverage(&pending);
                standby_scopes.insert(pending.id.clone(), metadata.3.clone());
                let covers_item = metadata.0;
                title_standby_metadata.insert(pending.id.clone(), metadata);
                if known_ids.insert(pending.id.clone())
                    && pending.wanted_item_id != item.id
                    && covers_item
                {
                    standby_releases.push(pending);
                }
            }

            season_pack_ids.extend(
                standby_releases
                    .iter()
                    .filter(|pending| {
                        title_standby_metadata
                            .get(&pending.id)
                            .is_some_and(|metadata| metadata.1)
                    })
                    .map(|pending| pending.id.clone()),
            );
            series_pack_ids.extend(
                standby_releases
                    .iter()
                    .filter(|pending| {
                        title_standby_metadata
                            .get(&pending.id)
                            .is_some_and(|metadata| metadata.2)
                    })
                    .map(|pending| pending.id.clone()),
            );
        }
    }

    // A season pack is a whole-season acquisition. For an episode scope, all
    // single-episode and episode-set standby releases are exhausted first; only
    // then does the walk consider packs. Each partition keeps the canonical
    // persistence order (score descending, then oldest first).
    order_standby_releases(&mut standby_releases, &season_pack_ids, &series_pack_ids);
    // Standby candidates are re-checked against the per-title blocklist (the
    // single, removable exclusion source), never the failed-attempt history.
    let db_blocklist = app
        .load_title_release_blocklist_signatures(&item.title_id)
        .await;
    let mut stale_indexer_ids = HashSet::new();

    for standby in standby_releases {
        let standby_scope = standby_scopes
            .get(&standby.id)
            .cloned()
            .unwrap_or_else(|| item.submission_scope());
        if series_pack_ids.contains(&standby.id)
            && excluded_episode_ids.is_some_and(|excluded| {
                episode_ids_for_scope(&standby_scope)
                    .is_some_and(|episode_ids| episode_ids.iter().any(|id| excluded.contains(id)))
            })
        {
            // Keep the saved result for failure recovery, but never submit an
            // overlapping series pack while this cycle already owns a member.
            continue;
        }
        let mut effective_wanted = item.clone();
        effective_wanted.grabbed_release = None;
        effective_wanted.last_search_at = None;

        let claimed = app
            .services
            .workflow
            .pending_releases
            .compare_and_set_pending_release_status(
                &standby.id,
                PendingReleaseStatus::Standby,
                PendingReleaseStatus::Processing,
                None,
            )
            .await
            .unwrap_or(false);
        if !claimed {
            continue;
        }

        if crate::app_usecase_discovery::is_release_blocklisted(
            standby.indexer_id.as_deref(),
            &standby.release_title,
            standby.info_hash.as_deref(),
            &db_blocklist,
        ) {
            // A blocklist entry is removable, so it is not evidence the release
            // is bad — only that the operator does not want it now. Keep the row
            // walkable rather than burning the corpus behind it.
            let _ = app
                .services
                .workflow
                .pending_releases
                .update_pending_release_status(&standby.id, PendingReleaseStatus::Standby, None)
                .await;
            continue;
        }

        if dl_snapshot.queue_listing_failed() {
            // Cannot confirm the release isn't already active; keep the standby
            // for a later cycle rather than expiring it on an unknown signal.
            info!(
                title_id = item.title_id.as_str(),
                standby_release = standby.release_title.as_str(),
                "standby reacquisition: queue listing failed, keeping release pending"
            );
            let _ = app
                .services
                .workflow
                .pending_releases
                .update_pending_release_status(&standby.id, PendingReleaseStatus::Standby, None)
                .await;
            return StandbyRecoveryOutcome::Deferred {
                scope: Some(standby_scope),
            };
        }

        if dl_snapshot.is_active(&standby.release_title) {
            // The scope is covered for now, but that download can still fail.
            // Expiring the row here is what leaves the next failure with no
            // corpus to walk.
            let _ = app
                .services
                .workflow
                .pending_releases
                .update_pending_release_status(&standby.id, PendingReleaseStatus::Standby, None)
                .await;
            return StandbyRecoveryOutcome::Active {
                scope: standby_scope,
            };
        }

        info!(
            title_id = item.title_id.as_str(),
            failed_release = ?failed_release_title,
            standby_release = standby.release_title.as_str(),
            "trying the next saved search result"
        );

        // Automatic: no operator asked for this release, so it is judged against
        // current policy the same way the delay-expiry promoter judges its rows.
        // Reacquiring into a swarm too small to finish would just fail again.
        match app
            .try_grab_pending_release(
                &effective_wanted,
                &standby,
                now,
                super::pending::PendingGrabTrigger::Automatic,
            )
            .await
        {
            Ok(super::pending::PendingGrabOutcome::Grabbed { scope }) => {
                let grabbed_at = now.to_rfc3339();
                let _ = app
                    .services
                    .workflow
                    .pending_releases
                    .update_pending_release_status(
                        &standby.id,
                        PendingReleaseStatus::Grabbed,
                        Some(&grabbed_at),
                    )
                    .await;

                // The remaining saved results stay `Standby`: if this grab fails
                // too, the next walk continues down the same list.

                if let Ok(Some(title)) = app.services.catalog.titles.get_by_id(&item.title_id).await
                {
                    let _ = app
                        .append_domain_event(new_title_domain_event(
                            None,
                            &title,
                            DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                                title: title_context_snapshot(&title),
                                source_title: Some(standby.release_title.clone()),
                                source_hint: None,
                                source_provider: None,
                                download_id: None,
                                episode_ids: item.episode_id.iter().cloned().collect(),
                            }),
                        ))
                        .await;
                }

                return StandbyRecoveryOutcome::Recovered { scope };
            }
            Ok(super::pending::PendingGrabOutcome::Deferred) => {
                info!(
                    release = standby.release_title.as_str(),
                    "standby reacquisition: download client unavailable, keeping release pending"
                );
                let _ = app
                    .services
                    .workflow
                    .pending_releases
                    .update_pending_release_status(&standby.id, PendingReleaseStatus::Standby, None)
                    .await;
                return StandbyRecoveryOutcome::Deferred {
                    scope: Some(standby_scope),
                };
            }
            Ok(super::pending::PendingGrabOutcome::Parked) => {
                return StandbyRecoveryOutcome::Parked {
                    scope: Some(standby_scope),
                };
            }
            Ok(super::pending::PendingGrabOutcome::SourceGone) => {
                if let Some(indexer_id) = standby.indexer_id.as_ref() {
                    stale_indexer_ids.insert(indexer_id.clone());
                }
                let _ = app
                    .services
                    .workflow
                    .pending_releases
                    .update_pending_release_status(&standby.id, PendingReleaseStatus::Expired, None)
                    .await;
            }
            Ok(super::pending::PendingGrabOutcome::Rejected) | Err(_) => {
                let _ = app
                    .services
                    .workflow
                    .pending_releases
                    .update_pending_release_status(&standby.id, PendingReleaseStatus::Expired, None)
                    .await;
            }
        }
    }

    StandbyRecoveryOutcome::Exhausted {
        stale_indexer_ids: stale_indexer_ids.into_iter().collect(),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "standby candidate persistence carries the search context explicitly"
)]
async fn persist_standby_candidates<F>(
    app: &AppUseCase,
    item: &AcquisitionScopeState,
    title: &Title,
    results: &[IndexerSearchResult],
    start_index: usize,
    now: &DateTime<Utc>,
    failed_routes: &[DownloadRouteKey],
    db_blocklist: &crate::app_usecase_discovery::TitleReleaseBlocklistSignatures,
    include_candidate: F,
) -> bool
where
    F: Fn(&IndexerSearchResult) -> bool,
{
    let _ = app
        .services
        .workflow
        .pending_releases
        .delete_standby_pending_releases_for_wanted_item(&item.id)
        .await;

    let mut persisted = 0usize;
    let mut complete = true;
    let mut seen_source_hints = std::collections::HashSet::<String>::new();

    for (rank, candidate) in results.iter().enumerate().skip(start_index) {
        if !include_candidate(candidate) {
            continue;
        }
        let decision_code =
            effective_auto_decision_code_for_route(candidate, failed_routes, db_blocklist);
        // Transient holds are kept, not dropped. A delayed release becomes
        // grabbable when its window ends, an active one when that download
        // fails, and a route-unavailable one when the client comes back — none
        // of them is evidence about the release itself. Dropping them is how a
        // scope ends up converged with an empty corpus. A blocklisted release is
        // *not* here on purpose: the walk that just burned it, or the operator
        // who blocked it, means there is no reason to write the row fresh.
        if !decision_code.is_eligible()
            && !matches!(
                decision_code,
                ReleaseAutoDecisionCode::PendingDelay
                    | ReleaseAutoDecisionCode::AlreadyActive
                    | ReleaseAutoDecisionCode::DownloadClientUnavailable
            )
        {
            // A fact about the *scope*, not about this candidate: the ranked
            // order is (tier, revision, score) and admission compares the same
            // three in the same order, so nothing below a rejected candidate
            // can do better either.
            //
            // `CutoffReached` used to be listed here and no longer is. Since
            // D15 it is candidate-aware — a same-tier revision upgrade escapes
            // it — and a better-*tier* candidate refused by the cutoff sorts
            // *above* the same-tier PROPER that would pass, so breaking on it
            // would skip the one release worth having. (`NegativeScore` was
            // also listed once; nothing emits it any more — the hardcoded zero
            // floor is gone — and the variant survives only so historical
            // decision rows still decode.)
            if matches!(decision_code, ReleaseAutoDecisionCode::UpgradeRejected) {
                break;
            }
            continue;
        }

        let source_hint = candidate
            .canonical_download_source()
            .map(|(source, _)| source);
        let Some(source_hint_value) = source_hint else {
            continue;
        };
        if !seen_source_hints.insert(source_hint_value.clone()) {
            continue;
        }

        let candidate_score = candidate
            .quality_profile_decision
            .as_ref()
            .map(|decision| decision.preference_score)
            .unwrap_or(0);
        let scoring_log_json = candidate
            .quality_profile_decision
            .as_ref()
            .and_then(|decision| {
                serde_json::to_string(
                    &decision
                        .scoring_log
                        .iter()
                        .map(|entry| serde_json::json!({"code": entry.code, "delta": entry.delta}))
                        .collect::<Vec<_>>(),
                )
                .ok()
            });

        let standby = PendingRelease {
            id: Id::new().0,
            wanted_item_id: item.id.clone(),
            title_id: title.id.clone(),
            release_title: candidate.title.clone(),
            release_url: Some(source_hint_value),
            source_kind: candidate.source_kind,
            release_size_bytes: candidate.size_bytes,
            release_score: candidate_score,
            scoring_log_json,
            indexer_source: Some(candidate.source.clone()),
            indexer_id: candidate.indexer_id.clone(),
            release_guid: candidate.guid.clone(),
            added_at: (*now + chrono::Duration::microseconds(rank as i64))
                .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            last_observed_at: now.to_rfc3339(),
            delay_until: now.to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: crate::normalize_release_password(candidate.password_hint.as_deref()),
            published_at: candidate.published_at.clone(),
            info_hash: candidate
                .extra
                .get("info_hash")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            seed_minimums: crate::ReleaseSeedMinimums::from_release_extra(&candidate.extra),
            seeders: crate::acquisition::seed_goals::seeders_from_extra(&candidate.extra),
            release_identity: String::new(),
            coverage_identity: String::new(),
            role: crate::types::PendingReleaseRole::Fallback,
            last_decision_code: None,
            release_age_unknown: false,
        };

        if app
            .services
            .workflow
            .pending_releases
            .insert_pending_release(&standby)
            .await
            .is_ok()
        {
            persisted += 1;
        } else {
            complete = false;
        }
    }

    if persisted > 0 {
        info!(
            wanted_item_id = item.id.as_str(),
            title_id = title.id.as_str(),
            standby_candidates = persisted,
            "persisted standby candidates for failed-download recovery"
        );
    }
    complete
}

#[cfg(test)]
mod client_snapshot_tests {
    use super::*;

    fn snapshot(
        queue_listing_failed: bool,
        history_listing_failed: bool,
    ) -> DownloadClientSnapshot {
        DownloadClientSnapshot {
            active_titles: std::collections::HashSet::new(),
            active_client_ids: std::collections::HashSet::new(),
            active_raw_item_id_counts: std::collections::HashMap::new(),
            stale_downloading_client_ids: std::collections::HashSet::new(),
            stale_downloading_raw_item_ids: std::collections::HashSet::new(),
            completed_client_ids: std::collections::HashSet::new(),
            completed_raw_item_id_counts: std::collections::HashMap::new(),
            failed_by_download_id: std::collections::HashMap::new(),
            queue_listing_failed,
            history_listing_failed,
            unreadable_client_ids: std::collections::HashSet::new(),
        }
    }

    /// The grab-side double-submit guard matches the *release title string*,
    /// while import admission matches the *scope*. A second release for a scope
    /// another client already has queued therefore passes the grab guard and is
    /// then refused at import as `QueuedEqualOrBetter`.
    #[test]
    fn the_grab_guard_misses_a_different_release_for_an_already_queued_scope() {
        let mut snap = snapshot(false, false);
        // qBittorrent is holding this exact release for S01E01.
        snap.active_titles
            .insert("show.s01e01.720p.web-dl.av1-ntb".to_string());

        // The identical release is correctly recognised as active.
        assert!(
            snap.is_active("Show.S01E01.720p.WEB-DL.AV1-NTb"),
            "the same release title must be seen as already active"
        );

        // A DIFFERENT release for the SAME episode is not, so the grab proceeds
        // and SAB downloads a second copy of a scope that is already queued.
        assert!(
            !snap.is_active("Show.S01E01.1080p.WEB-DL.x264-GROUP"),
            "grab guard is title-string equality, so it cannot see that this \
             scope is already queued under another release name"
        );
    }

    /// SABnzbd post-processes in *history*: the job leaves the queue listing
    /// while it verifies, repairs, unpacks and moves. Each of those states is
    /// still a live claim on the scope; only a settled outcome is not.
    #[test]
    fn history_post_processing_states_are_live_claims() {
        for state in [
            DownloadQueueState::Queued,
            DownloadQueueState::Downloading,
            DownloadQueueState::Paused,
            DownloadQueueState::Verifying,
            DownloadQueueState::Repairing,
            DownloadQueueState::Extracting,
            DownloadQueueState::ImportPending,
            DownloadQueueState::Warning,
        ] {
            assert!(
                history_item_is_live_claim(state),
                "{state:?} in history is work the client is still doing"
            );
        }
        for state in [DownloadQueueState::Completed, DownloadQueueState::Failed] {
            assert!(
                !history_item_is_live_claim(state),
                "{state:?} is an outcome, recorded by the completed/failed sets instead"
            );
        }
    }

    fn standby(id: &str, score: i32, added_at: &str) -> PendingRelease {
        PendingRelease {
            id: id.to_string(),
            wanted_item_id: "wanted".to_string(),
            title_id: "title".to_string(),
            release_title: id.to_string(),
            release_url: None,
            source_kind: None,
            release_size_bytes: None,
            release_score: score,
            scoring_log_json: None,
            indexer_source: None,
            indexer_id: None,
            release_guid: None,
            added_at: added_at.to_string(),
            last_observed_at: added_at.to_string(),
            delay_until: added_at.to_string(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: None,
            info_hash: None,
            seed_minimums: Default::default(),
            seeders: None,
            release_identity: id.to_string(),
            coverage_identity: "scope:wanted".to_string(),
            role: crate::types::PendingReleaseRole::Fallback,
            last_decision_code: None,
            release_age_unknown: false,
        }
    }

    #[test]
    fn standby_order_tries_all_single_episode_rows_before_a_higher_scored_pack() {
        let mut standby = vec![
            standby("season-pack", 900, "2026-01-01T00:00:00Z"),
            standby("single-episode", 100, "2026-01-02T00:00:00Z"),
        ];
        let season_pack_ids = HashSet::from(["season-pack".to_string()]);

        order_standby_releases(&mut standby, &season_pack_ids, &HashSet::new());

        assert_eq!(
            standby
                .iter()
                .map(|pending| pending.id.as_str())
                .collect::<Vec<_>>(),
            vec!["single-episode", "season-pack"]
        );
    }

    #[test]
    fn series_pack_standby_order_preserves_persisted_search_rank() {
        let mut standby = vec![
            standby("rank-two", 900, "2026-01-01T00:00:00.000001000Z"),
            standby("rank-one", 100, "2026-01-01T00:00:00.000000000Z"),
        ];
        let pack_ids = HashSet::from(["rank-one".to_string(), "rank-two".to_string()]);

        order_standby_releases(&mut standby, &pack_ids, &pack_ids);

        assert_eq!(
            standby
                .iter()
                .map(|pending| pending.id.as_str())
                .collect::<Vec<_>>(),
            vec!["rank-one", "rank-two"]
        );
    }

    #[test]
    fn standby_order_is_total_for_mixed_plain_and_series_packs() {
        let mut standby = vec![
            standby("plain", 500, "2026-01-01T00:00:05Z"),
            standby("series-a", 100, "2026-01-01T00:00:00Z"),
            standby("series-b", 900, "2026-01-01T00:00:01Z"),
        ];
        let season_pack_ids = HashSet::from([
            "plain".to_string(),
            "series-a".to_string(),
            "series-b".to_string(),
        ]);
        let series_pack_ids = HashSet::from(["series-a".to_string(), "series-b".to_string()]);

        order_standby_releases(&mut standby, &season_pack_ids, &series_pack_ids);

        assert_eq!(
            standby
                .iter()
                .map(|pending| pending.id.as_str())
                .collect::<Vec<_>>(),
            vec!["series-a", "series-b", "plain"]
        );
    }

    #[test]
    fn standby_order_keeps_all_groups_ordered_in_a_large_mixed_list() {
        let mut standby = (0..8)
            .map(|index| {
                standby(
                    &format!("episode-{index}"),
                    index,
                    &format!("2026-01-01T00:00:{index:02}Z"),
                )
            })
            .chain((0..8).map(|index| {
                standby(
                    &format!("series-{index}"),
                    1000 - index,
                    &format!("2026-01-01T00:01:{index:02}Z"),
                )
            }))
            .chain((0..8).map(|index| {
                standby(
                    &format!("season-{index}"),
                    index,
                    &format!("2026-01-01T00:02:{index:02}Z"),
                )
            }))
            .collect::<Vec<_>>();
        standby.reverse();
        let season_pack_ids = (0..8)
            .map(|index| format!("series-{index}"))
            .chain((0..8).map(|index| format!("season-{index}")))
            .collect::<HashSet<_>>();
        let series_pack_ids = (0..8)
            .map(|index| format!("series-{index}"))
            .collect::<HashSet<_>>();

        order_standby_releases(&mut standby, &season_pack_ids, &series_pack_ids);

        let ids = standby
            .iter()
            .map(|pending| pending.id.as_str())
            .collect::<Vec<_>>();
        assert!(ids[..8].iter().all(|id| id.starts_with("episode-")));
        assert!(ids[8..16].iter().all(|id| id.starts_with("series-")));
        assert!(ids[16..].iter().all(|id| id.starts_with("season-")));
    }

    #[test]
    fn queue_listing_failure_treats_everything_as_active() {
        let snap = snapshot(true, false);
        assert!(snap.queue_listing_failed());
        // Any title / client item is treated as possibly-active so automatic
        // grabs skip/defer instead of double-submitting blind.
        assert!(snap.is_active("Some.Release.That.Is.Not.In.Any.Queue"));
        assert!(snap.has_active_client_item(Some("client-1"), "nzo_missing"));
        assert!(snap.has_active_client_item(None, "nzo_missing"));
    }

    #[test]
    fn observable_empty_queue_reports_nothing_active() {
        let snap = snapshot(false, false);
        assert!(!snap.queue_listing_failed());
        assert!(!snap.is_active("Some.Release"));
        assert!(!snap.has_active_client_item(Some("client-1"), "nzo_missing"));
    }

    #[test]
    fn history_listing_failure_reports_no_failures() {
        let mut snap = snapshot(false, true);
        snap.failed_by_download_id.insert(
            "client-1:nzo_1".to_string(),
            FailedDownloadSnapshot {
                reason: "MISSING ARTICLES".to_string(),
                download_client_item_id: "nzo_1".to_string(),
                client_id: "client-1".to_string(),
                client_name: None,
            },
        );
        // Even with a populated map, an unobservable history must not surface
        // failures (failure detection is skipped this cycle).
        assert!(snap.failed_item(Some("client-1"), "nzo_1").is_none());

        // With history observable, the same entry is reported.
        snap.history_listing_failed = false;
        assert!(snap.failed_item(Some("client-1"), "nzo_1").is_some());
    }

    /// **D18 liveness.** Which tracked states make a submission a queued
    /// pseudo-incumbent, stated once so the filter and Sonarr's
    /// `QueueSpecification` cannot drift apart.
    #[test]
    fn queued_liveness_counts_held_imports_and_excludes_failures() {
        use scryer_domain::TrackedDownloadState as State;

        for state in [
            State::Downloading,
            State::ImportPending,
            State::Importing,
            // A held import is a real claim on the scope: an equal or worse
            // release must not be fetched beside it.
            State::ImportBlocked,
        ] {
            assert!(
                submission_is_queued(Some(state), false),
                "{state:?} must count as queued"
            );
        }

        // Sonarr skips `FailedPending` precisely so a replacement can be
        // grabbed while the failure handler runs.
        assert!(!submission_is_queued(Some(State::FailedPending), false));
        for state in [State::Imported, State::ImportedSeeding, State::Failed] {
            assert!(
                !submission_is_queued(Some(state), false),
                "{state:?} is over"
            );
        }

        // No tracked row: the client is the only witness.
        assert!(submission_is_queued(None, true));
        assert!(!submission_is_queued(None, false));
    }

    fn live_claim_submission(item_id: &str) -> DownloadSubmission {
        DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: "title".to_string(),
            facet: "series".to_string(),
            download_client_id: Some("client-1".to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Show.S01E01.1080p.WEB-DL-GROUP".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            purpose: Default::default(),
            scope: SubmissionScope::Episode {
                episode_id: "ep-1".to_string(),
            },
        }
    }

    /// A downloaded-but-not-yet-imported item is as much a claim on its scope
    /// as one still downloading. The tracked-state ledger often gains its row
    /// only when the import *finishes*, so for the whole import window the
    /// client snapshot is the only witness — and the client reports the item
    /// as completed, not active. Without the completed leg the scope sits
    /// unprotected for that window and an automatic search grabs a second
    /// copy beside the bytes already on disk.
    #[test]
    fn a_completed_unimported_download_is_a_live_claim() {
        let submission = live_claim_submission("11818");
        let mut snap = snapshot(false, false);

        // Invisible to both sets: no claim (the item left the client).
        assert!(!submission_is_live_claim(&submission, None, &snap));

        // Completed in the client, no tracked row yet: still a claim.
        snap.completed_client_ids
            .insert(download_client_item_identity(Some("client-1"), "11818"));
        assert!(submission_is_live_claim(&submission, None, &snap));

        // A terminal tracked state releases the claim regardless of the
        // client still listing the finished item.
        for state in [
            scryer_domain::TrackedDownloadState::Imported,
            scryer_domain::TrackedDownloadState::Failed,
        ] {
            assert!(!submission_is_live_claim(&submission, Some(state), &snap));
        }
    }

    /// The client-echoed fallback name must shed SABnzbd's `.nzb` suffix, or
    /// the blocklist row it keys can never match a search-time candidate.
    #[test]
    fn client_echoed_fallback_sheds_a_trailing_nzb_suffix() {
        assert_eq!(
            strip_nzb_suffix("Show.S01E01.1080p.WEB-DL.nzb"),
            "Show.S01E01.1080p.WEB-DL"
        );
        assert_eq!(
            strip_nzb_suffix("Show.S01E01.1080p.WEB-DL.NZB"),
            "Show.S01E01.1080p.WEB-DL"
        );
        // Only a suffix is shed, exactly once, and only when present.
        assert_eq!(strip_nzb_suffix("Show.S01E01.1080p.WEB-DL"), "Show.S01E01.1080p.WEB-DL");
        assert_eq!(strip_nzb_suffix("Show.nzb.S01E01"), "Show.nzb.S01E01");
        assert_eq!(strip_nzb_suffix("Show.nzb.nzb"), "Show.nzb");
        assert_eq!(strip_nzb_suffix(".nzb"), "");
        assert_eq!(strip_nzb_suffix("nzb"), "nzb");
    }

    /// **Unreadable client.** The aggregate router degrades a dead client to
    /// an empty listing rather than an error, so "absent from queue and
    /// history" is only proof of absence when that client actually answered.
    /// A submission on a client that did not answer keeps its claim; one on a
    /// client that did answer is judged on what it said.
    #[test]
    fn a_submission_on_an_unreadable_client_keeps_its_claim() {
        use scryer_domain::TrackedDownloadState as State;

        let submission = live_claim_submission("nzo_1");
        let mut snap = snapshot(false, false);
        assert!(!submission_is_live_claim(&submission, None, &snap));

        snap.unreadable_client_ids.insert("client-1".to_string());
        assert!(snap.client_unreadable(Some("client-1")));
        assert!(!snap.client_unreadable(Some("client-2")));
        // A submission that names no client is judged against every client.
        assert!(snap.client_unreadable(None));
        assert!(submission_is_live_claim(&submission, None, &snap));

        let mut other_client = live_claim_submission("nzo_2");
        other_client.download_client_id = Some("client-2".to_string());
        assert!(!submission_is_live_claim(&other_client, None, &snap));

        // The tracked ledger still settles it: a terminal state releases the
        // claim even while the client stays unreadable.
        for state in [State::Imported, State::Failed, State::Ignored] {
            assert!(!submission_is_live_claim(&submission, Some(state), &snap));
        }
    }

    /// A leg nobody answered is as blind as a hard listing error; a leg some
    /// clients answered stays observable but remembers who did not.
    #[test]
    fn read_reports_fold_into_listing_failure_and_unreadable_clients() {
        fn listing(unreadable: &[&str], polled: usize) -> crate::DownloadClientListing {
            crate::DownloadClientListing {
                items: Vec::new(),
                unreadable_client_ids: unreadable.iter().map(|id| id.to_string()).collect(),
                polled_client_count: polled,
            }
        }

        let mut unreadable = std::collections::HashSet::new();
        let mut failed = false;
        let items = note_unreadable_clients(listing(&["sab"], 1), "history", &mut unreadable, &mut failed);
        assert!(items.is_empty());
        assert!(failed, "a leg nobody answered is unobservable");
        assert!(unreadable.contains("sab"));

        let mut unreadable = std::collections::HashSet::new();
        let mut failed = false;
        note_unreadable_clients(listing(&["sab"], 2), "queue", &mut unreadable, &mut failed);
        assert!(!failed, "one answering client keeps the leg observable");
        assert!(unreadable.contains("sab"));

        let mut unreadable = std::collections::HashSet::new();
        let mut failed = false;
        note_unreadable_clients(listing(&[], 0), "queue", &mut unreadable, &mut failed);
        assert!(!failed, "no configured client is nothing to be blind about");
        assert!(unreadable.is_empty());
    }
}
