/// Scheduler value hint for a hot acquisition target (recent air/release/add):
/// high value so the scope is processed promptly and keeps admitting
/// even while the account's API quota is under pressure. Equals the neutral
/// baseline, so hot work is never shed by the low-value pressure gate.
const BACKGROUND_HOT_TARGET_VALUE: f64 = 1.0;

/// Scheduler value hint for a cold acquisition target (long-tail / upgrades):
/// low value so the quota-pressure gate drains it first,
/// yielding shared account quota to RSS polls and hot acquisition. Above the
/// absolute `LOW_VALUE_BACKGROUND_THRESHOLD` floor, so a cold scope still
/// admits when quota is healthy — it only defers once quota tightens.
const BACKGROUND_COLD_TARGET_VALUE: f64 = 0.25;

/// Maximum number of titles whose missing-media acquisition pipelines may be
/// evaluated concurrently. Indexer strategy admission is bounded separately.
const BACKGROUND_ACQUISITION_TITLE_LIMIT: usize = 4;

/// How far apart two instances' maintenance evaluation passes can drift. Much
/// smaller than the eight-hour cadence on purpose: the jitter is there to keep
/// a fleet from sweeping in lockstep, not to postpone the first pass.
const MAINTENANCE_EVALUATION_JITTER_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(30 * 60);

/// Same rationale, distinct salt, for the action handler's twelve-hour cadence.
const LIFECYCLE_ACTION_HANDLING_JITTER_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(30 * 60);

/// Same rationale again for the media-server signal sweep (RFC 137 §7.3). The
/// jitter matters more here than for the local passes: every instance in a
/// fleet would otherwise hit the same Jellyfin server at the same moment.
const MEDIA_SERVER_SIGNAL_SYNC_JITTER_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy)]
struct BackgroundAcquisitionSettings {
    max_scopes_per_cycle: usize,
}

impl AppUseCase {
    async fn background_acquisition_settings(&self) -> AppResult<BackgroundAcquisitionSettings> {
        let max_scopes_per_cycle = self
            .read_setting_i64_value(
                crate::acquisition::convergence::ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY,
                None,
            )
            .await?
            .filter(|value| *value > 0)
            .unwrap_or(
                crate::acquisition::convergence::DEFAULT_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE,
            ) as usize;
        Ok(BackgroundAcquisitionSettings {
            max_scopes_per_cycle,
        })
    }
}

async fn blocked_acquisition_facets_after_quiet_wait(app: &AppUseCase) -> Vec<MediaFacet> {
    let blocked_facets = app
        .runtime
        .library
        .library_scan_tracker
        .active_facets()
        .await;
    if blocked_facets.is_empty() {
        return Vec::new();
    }

    metrics::counter!("scryer_background_acquisition_scan_owned_yields_total").increment(1);
    debug!(
        blocked_facets = ?active_scan_facet_labels(&blocked_facets),
        wait_secs = ACQUISITION_SCAN_QUIET_WAIT.as_secs(),
        "background acquisition: yielding while library scan owns active facet"
    );

    let _ = tokio::time::timeout(
        ACQUISITION_SCAN_QUIET_WAIT,
        app.runtime
            .library
            .library_scan_tracker
            .wait_for_active_facets_change(&blocked_facets),
    )
    .await;

    let blocked_facets = app
        .runtime
        .library
        .library_scan_tracker
        .active_facets()
        .await;

    if !blocked_facets.is_empty() {
        debug!(
            blocked_facets = ?active_scan_facet_labels(&blocked_facets),
            "background acquisition: deferring due wanted items for actively scanning facets"
        );
    }

    blocked_facets
}
/// Run one background acquisition cycle: recover failed downloads, derive the
/// missing-media target set, rotate the cursor, and process at most four titles
/// concurrently. Fingerprinted convergence coverage only decides which indexer
/// corpus searches may be skipped; it is not the activity being scheduled.
/// The floor under a deferred-work re-arm. A cooldown that has just expired
/// must not turn the poller into a busy loop.
const DEFERRED_RETRY_MIN_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// What one convergence cycle did, for the poller and the summary line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BackgroundAcquisitionCycleOutcome {
    pub titles_walked: usize,
    pub targets_derived: usize,
    /// Scopes the scheduler pre-skip declined because every uncovered indexer
    /// was cooling down or quota-spent. Only this half can re-arm the poller,
    /// and only when the snapshot named an instant the cooldown lifts at.
    pub deferred_scopes: usize,
    /// Titles this cycle stepped over because an interactive walk held them.
    ///
    /// Telemetry only: it deliberately does *not* re-arm the retry timer. An
    /// interactive walk over a large title runs for minutes, so a re-arm here
    /// would run a full convergence cycle every re-arm interval for the whole
    /// job — each one advancing the cold cursor and spending indexer evaluation
    /// budget. The job wakes the poller itself when it releases the lock (see
    /// [`run_interactive_title_acquisition_walk`]), so exactly one cycle
    /// follows it.
    pub skipped_locked_titles: usize,
    /// The soonest a cooling host is expected back, when the scheduler snapshot
    /// named one. An exhausted quota carries no instant at all, and an untimed
    /// deferral waits for the poller's own tick rather than guessing.
    pub retry_after: Option<std::time::Duration>,
}

impl BackgroundAcquisitionCycleOutcome {
    /// The delay before the poller runs another cycle for this one's deferred
    /// scopes, or `None` to wait for the next tick.
    ///
    /// A re-arm is only worth anything if it *beats* the tick, so it is offered
    /// only when the cycle deferred scopes, the snapshot named a recovery
    /// instant, and that instant falls inside `poll_interval`. The interval is
    /// therefore the natural bound — there is no separate cap — and
    /// [`DEFERRED_RETRY_MIN_DELAY`] is the floor that stops a just-lifted
    /// cooldown spinning the loop. An untimed deferral (an exhausted quota)
    /// waits for the tick.
    pub(crate) fn deferred_retry_delay(
        self,
        poll_interval: std::time::Duration,
    ) -> Option<std::time::Duration> {
        if self.deferred_scopes == 0 {
            return None;
        }
        self.retry_after
            .filter(|retry_after| *retry_after < poll_interval)
            .map(|retry_after| retry_after.max(DEFERRED_RETRY_MIN_DELAY))
    }
}

async fn run_background_acquisition_cycle(app: &AppUseCase) -> BackgroundAcquisitionCycleOutcome {
    let blocked_facets = blocked_acquisition_facets_after_quiet_wait(app).await;
    run_background_acquisition_cycle_with_blocked_facets(app, &blocked_facets).await
}

pub(crate) async fn run_background_acquisition_cycle_with_blocked_facets(
    app: &AppUseCase,
    blocked_facets: &[MediaFacet],
) -> BackgroundAcquisitionCycleOutcome {
    let cycle_started = std::time::Instant::now();
    prune_standby_candidates(app).await;

    // Failed downloads first: each failure blocklists its release and re-opens
    // its scope under the existing coverage, so this cycle's derivation already
    // sees it — and the scope's saved search results are tried below before
    // any indexer is queried.
    let dl_snapshot = DownloadClientSnapshot::fetch(app).await;
    check_grabbed_for_failures(app, &dl_snapshot).await;

    let now = Utc::now();
    let settings = match app.background_acquisition_settings().await {
        Ok(settings) => settings,
        Err(err) => {
            warn!(error = %err, "failed to load background acquisition settings, skipping cycle");
            return BackgroundAcquisitionCycleOutcome::default();
        }
    };

    let mut targets = match app.derive_acquisition_targets(&now).await {
        Ok(targets) => targets,
        Err(err) => {
            warn!(error = %err, "failed to derive acquisition targets");
            return BackgroundAcquisitionCycleOutcome::default();
        }
    };
    if !blocked_facets.is_empty() {
        targets.retain(|target| !blocked_facets.contains(&target.facet));
    }
    if targets.is_empty() {
        return BackgroundAcquisitionCycleOutcome::default();
    }

    if !has_enabled_download_clients(app).await {
        warn!(
            target_count = targets.len(),
            "background acquisition: no enabled download clients configured, skipping cycle"
        );
        return BackgroundAcquisitionCycleOutcome {
            targets_derived: targets.len(),
            ..BackgroundAcquisitionCycleOutcome::default()
        };
    }

    let hot_resume = app.background_acquisition_hot_resume_position().await;
    let resume = app.background_acquisition_resume_position().await;
    let max_scopes = settings.max_scopes_per_cycle.max(1);
    let selection = crate::acquisition::targets::select_background_acquisition_batch(
        &targets,
        hot_resume.as_deref(),
        resume.as_deref(),
        max_scopes,
    );
    app.store_background_acquisition_hot_resume_position(selection.hot_resume_after.as_deref())
        .await;
    app.store_background_acquisition_resume_position(selection.resume_after.as_deref())
        .await;
    if selection.indices.is_empty() {
        return BackgroundAcquisitionCycleOutcome {
            targets_derived: targets.len(),
            ..BackgroundAcquisitionCycleOutcome::default()
        };
    }

    debug!(
        target_count = targets.len(),
        selected_count = selection.indices.len(),
        "background acquisition cycle: evaluating missing scopes"
    );

    // Scheduler availability, resolved once per cycle for the pre-skip.
    let availability = app.scheduler_availability().await;
    let indexer_hosts = app.indexer_scheduler_host_keys().await;

    let cycle = Arc::new(BackgroundAcquisitionCycleCoordinator::default());

    // Count selected episode scopes per (title_id, season_num). Season pack
    // search is only worthwhile when >= 2 episodes from the same season are in
    // this cycle — mirroring Sonarr's "count > 1 missing" rule before issuing a
    // SeasonSearchCriteria.
    let mut season_due_counts: std::collections::HashMap<(String, u32), usize> =
        std::collections::HashMap::new();
    for index in &selection.indices {
        let target = &targets[*index];
        if target.media_type == "episode"
            && let Some(sn) = target.season_number.as_deref()
            && let Ok(n) = sn.parse::<u32>()
            && n > 0
        {
            *season_due_counts
                .entry((target.title_id.clone(), n))
                .or_insert(0) += 1;
        }
    }

    let mut ready_titles =
        build_background_acquisition_title_work(&targets, &selection.indices, None);
    let title_ids = ready_titles
        .iter()
        .map(|work| work.title_id.clone())
        .collect::<Vec<_>>();
    let titles_by_id = match app.services.catalog.titles.get_by_ids(&title_ids).await {
        Ok(titles) => titles
            .into_iter()
            .map(|title| (title.id.clone(), title))
            .collect::<HashMap<_, _>>(),
        Err(error) => {
            warn!(error = %error, "background acquisition: failed to load selected titles");
            return BackgroundAcquisitionCycleOutcome {
                targets_derived: targets.len(),
                ..BackgroundAcquisitionCycleOutcome::default()
            };
        }
    };
    let mut in_flight = FuturesUnordered::new();
    let mut titles_walked = 0usize;
    let mut skipped_locked_titles = 0usize;
    let availability = &availability;
    let indexer_hosts = &indexer_hosts;
    let season_due_counts = &season_due_counts;
    let dl_snapshot = &dl_snapshot;
    let now = &now;
    let targets = &targets;

    debug!(
        selected_count = selection.indices.len(),
        title_count = ready_titles.len(),
        title_limit = BACKGROUND_ACQUISITION_TITLE_LIMIT,
        "background acquisition cycle: dispatching title work"
    );

    loop {
        while in_flight.len() < BACKGROUND_ACQUISITION_TITLE_LIMIT {
            let Some(title_work) = ready_titles.pop_front() else {
                break;
            };
            let Some(title) = titles_by_id.get(&title_work.title_id).cloned() else {
                warn!(
                    title_id = title_work.title_id.as_str(),
                    "background acquisition target references missing title"
                );
                continue;
            };
            // A title an operator's interactive walk holds is skipped, not
            // waited on: the cycle has other titles to get through and the
            // cursor comes back to this one. The skip is counted for telemetry
            // but does not re-arm the retry timer — the job notifies the wake
            // when it releases the lock, so one cycle follows it instead of one
            // cycle per retry interval for the length of the walk.
            let Some(walk_guard) = app
                .runtime
                .acquisition
                .title_walk_locks
                .try_acquire(&title_work.title_id)
                .await
            else {
                skipped_locked_titles += 1;
                debug!(
                    title_id = title_work.title_id.as_str(),
                    "background acquisition: an interactive walk holds this title, skipping"
                );
                continue;
            };
            let cycle = Arc::clone(&cycle);
            debug!(
                title_id = title_work.title_id.as_str(),
                queued_titles = ready_titles.len(),
                active_titles = in_flight.len() + 1,
                "background acquisition title work started"
            );
            in_flight.push(async move {
                let _walk_guard = walk_guard;
                let title_id = title_work.title_id.clone();
                let result = process_background_acquisition_title(
                    app,
                    title,
                    title_work,
                    targets,
                    now,
                    availability,
                    indexer_hosts,
                    &cycle,
                    season_due_counts,
                    dl_snapshot,
                    TitleWalkOptions::background(),
                    |_, _| {},
                )
                .await;
                (title_id, result)
            });
        }

        let Some((title_id, result)) = in_flight.next().await else {
            break;
        };
        titles_walked += 1;
        if let Err(err) = result {
            warn!(
                title_id = title_id.as_str(),
                error = %err,
                "failed to process background acquisition title"
            );
            metrics::counter!("scryer_background_acquisition_title_work_total", "outcome" => "failed")
                .increment(1);
        } else {
            metrics::counter!("scryer_background_acquisition_title_work_total", "outcome" => "completed")
                .increment(1);
        }
    }

    let deferred_scopes = cycle.deferred_scopes();
    let outcome = BackgroundAcquisitionCycleOutcome {
        titles_walked,
        targets_derived: targets.len(),
        deferred_scopes,
        skipped_locked_titles,
        retry_after: (deferred_scopes > 0)
            .then(|| availability.earliest_recovery_in(now))
            .flatten(),
    };
    // One line per cycle, above the per-title summaries.
    info!(
        titles_walked = outcome.titles_walked,
        targets_derived = outcome.targets_derived,
        selected_scopes = selection.indices.len(),
        deferred_scopes = outcome.deferred_scopes,
        skipped_locked_titles = outcome.skipped_locked_titles,
        elapsed_ms = cycle_started.elapsed().as_millis() as u64,
        "background acquisition cycle complete"
    );
    outcome
}
/// Whether an in-flight submission should stop this scope being searched again.
///
/// `scope_is_occupied` says whether a primary file already sits in the scope. It
/// separates the two cases the last clause cares about: a completed download for
/// an *empty* scope is still on its way to becoming a file, so searching again
/// would duplicate it; a completed download for an *occupied* scope has already
/// resolved one way or the other, and an upgrade search may proceed.
///
/// This used to read `wanted_items.current_score.is_none()` — a score standing in
/// for "has anything landed here", which is the only honest thing that column
/// ever said, and only in one of its five states.
fn submission_blocks_search_for_wanted_item(
    submission: &DownloadSubmission,
    item: &AcquisitionScopeState,
    episode_collection_id: Option<&str>,
    dl_snapshot: &DownloadClientSnapshot,
    tracked_state: Option<scryer_domain::TrackedDownloadState>,
    scope_is_occupied: bool,
) -> bool {
    if !submission_blocks_wanted_item(submission, item, episode_collection_id) {
        return false;
    }

    if tracked_state == Some(scryer_domain::TrackedDownloadState::Failed) {
        return false;
    }

    // **A failure the handler has not processed yet.** The scope is about to be
    // reopened or blocklisted by `handle_failed_downloads`; searching it now
    // races that, and the release it would find is very likely the one that just
    // failed. Sonarr excludes `FailedPending` from `QueueSpecification` for the
    // same reason — it wants the failure resolved first, not the scope frozen.
    if tracked_state
        .is_some_and(|state| matches!(state, scryer_domain::TrackedDownloadState::FailedPending))
    {
        return true;
    }

    // An unobservable queue reads as "possibly active" everywhere else too
    // (`DownloadClientSnapshot::is_active`); with no way to build honest queued
    // pseudo-incumbents, the old whole-scope skip is the safe answer.
    if dl_snapshot.queue_listing_failed() {
        return true;
    }

    // An initial acquisition already claiming an empty scope must finish (or
    // fail authoritatively) before another corpus search is admitted. This is
    // especially important for a season/title pack: its submission covers the
    // child episodes across later scheduler cycles, after the cycle-local pack
    // proposal is gone. Occupied scopes retain queued-pseudo-incumbent behavior
    // so genuine upgrade searches can still compare against an in-flight grab.
    //
    // One escape: a claim still `Downloading` past the staleness bound is a
    // stalled swarm, not a claim — a dead torrent never fails on its own, and
    // without this the scope would freeze until an operator noticed. The scope
    // re-enters the D18 comparison (Sonarr's `QueueSpecification` shape), so
    // only a strictly better release is grabbed beside the stall.
    let tracked_submission_is_live = tracked_state.is_some_and(|state| {
        matches!(
            state,
            scryer_domain::TrackedDownloadState::Downloading
                | scryer_domain::TrackedDownloadState::ImportPending
                | scryer_domain::TrackedDownloadState::Importing
                | scryer_domain::TrackedDownloadState::ImportBlocked
        )
    });
    if !scope_is_occupied
        && !dl_snapshot.active_downloading_is_stale(
            submission.download_client_id.as_deref(),
            &submission.download_client_item_id,
        )
        && (tracked_submission_is_live
            || submission_is_active(submission, dl_snapshot))
    {
        return true;
    }

    // A completed download for a scope with nothing in it is on its way to
    // becoming that file. Searching again would fetch the same episode twice,
    // and there is nothing queued left to compare against — it has left the
    // queue. For an *occupied* scope the download has already resolved one way
    // or the other and an upgrade search may proceed.
    submission_is_completed(submission, dl_snapshot) && !scope_is_occupied
}

impl AppUseCase {
    /// One convergence cycle, returning what it walked so a test can assert on
    /// the deferral counters the poller re-arms from.
    #[cfg(test)]
    pub(crate) async fn run_background_acquisition_cycle_once(
        &self,
    ) -> BackgroundAcquisitionCycleOutcome {
        run_background_acquisition_cycle(self).await
    }
}

#[derive(Default)]
struct BackgroundAcquisitionCycleCoordinator {
    state: Mutex<BackgroundAcquisitionCycleState>,
}

#[derive(Default)]
struct BackgroundAcquisitionCycleState {
    attempted_titles: HashSet<String>,
    claimed_episode_ids: HashSet<String>,
    season_pack_attempted: HashSet<(String, u32)>,
    season_pack_grabbed: HashSet<(String, u32)>,
    season_pack_viable: HashSet<(String, u32)>,
    /// Episode-shaped releases a season query surfaced. Merged into the episode
    /// scope's own results — never a substitute for its query.
    season_candidates: HashMap<(String, u32), Vec<IndexerSearchResult>>,
    grabbed_urls: HashSet<String>,
    attempted_urls_by_route: Vec<(DownloadRouteKey, String)>,
    failed_routes: Vec<DownloadRouteKey>,
    /// Scopes this cycle declined to search because every uncovered indexer was
    /// cooling down or quota-spent. Work the cycle *wanted* to do and will do
    /// as soon as the condition lifts, so a cooldown that expires inside the
    /// poll interval re-arms the poller instead of leaving the scope idle for
    /// the rest of the tick.
    deferred_scopes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubmissionClaim {
    Granted,
    AlreadySubmitted,
    AlreadyAttempted,
    RouteUnavailable,
}

impl BackgroundAcquisitionCycleCoordinator {
    fn lock(&self) -> std::sync::MutexGuard<'_, BackgroundAcquisitionCycleState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn claimed_episode_ids(&self) -> HashSet<String> {
        self.lock().claimed_episode_ids.clone()
    }

    fn record_deferred_scope(&self) {
        self.lock().deferred_scopes += 1;
    }

    fn deferred_scopes(&self) -> usize {
        self.lock().deferred_scopes
    }

    fn is_episode_claimed(&self, episode_id: &str) -> bool {
        self.lock().claimed_episode_ids.contains(episode_id)
    }

    fn claim_episode_ids<I>(&self, episode_ids: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.lock().claimed_episode_ids.extend(episode_ids);
    }

    fn begin_title_pack(&self, title_id: &str) -> bool {
        self.lock().attempted_titles.insert(title_id.to_string())
    }

    fn complete_title_pack_stage(&self, title_id: &str) {
        self.lock().attempted_titles.insert(title_id.to_string());
    }

    fn begin_season_pack(&self, key: &(String, u32)) -> bool {
        self.lock().season_pack_attempted.insert(key.clone())
    }

    fn complete_season_pack_stage(&self, key: &(String, u32)) {
        self.lock().season_pack_attempted.insert(key.clone());
    }

    fn cache_season_candidates(
        &self,
        key: &(String, u32),
        candidates: impl IntoIterator<Item = IndexerSearchResult>,
    ) {
        let mut state = self.lock();
        let cached = state.season_candidates.entry(key.clone()).or_default();
        for candidate in candidates {
            let duplicate = cached
                .iter()
                .any(|existing| same_indexer_release(existing, &candidate));
            if !duplicate {
                cached.push(candidate);
            }
        }
    }

    fn season_candidates(&self, key: &(String, u32)) -> Vec<IndexerSearchResult> {
        self.lock()
            .season_candidates
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    fn season_pack_grabbed(&self, key: &(String, u32)) -> bool {
        self.lock().season_pack_grabbed.contains(key)
    }

    fn season_pack_viable(&self, key: &(String, u32)) -> bool {
        self.lock().season_pack_viable.contains(key)
    }

    fn mark_season_pack_grabbed(&self, key: &(String, u32)) {
        let mut state = self.lock();
        state.season_pack_grabbed.insert(key.clone());
        state.season_pack_viable.insert(key.clone());
    }

    fn mark_season_pack_viable(&self, key: &(String, u32)) {
        self.lock().season_pack_viable.insert(key.clone());
    }

    fn clear_season_pack_viable(&self, key: &(String, u32)) {
        self.lock().season_pack_viable.remove(key);
    }

    fn failed_routes(&self) -> Vec<DownloadRouteKey> {
        self.lock().failed_routes.clone()
    }

    fn mark_failed_route(&self, route: DownloadRouteKey) {
        let mut state = self.lock();
        if !state.failed_routes.contains(&route) {
            state.failed_routes.push(route);
        }
    }

    fn claim_submission(&self, route: DownloadRouteKey, url: &str) -> SubmissionClaim {
        let mut state = self.lock();
        if state.failed_routes.contains(&route) {
            return SubmissionClaim::RouteUnavailable;
        }
        if state.grabbed_urls.contains(url) {
            return SubmissionClaim::AlreadySubmitted;
        }
        let attempted = (route, url.to_string());
        if state.attempted_urls_by_route.contains(&attempted) {
            return SubmissionClaim::AlreadyAttempted;
        }
        state.attempted_urls_by_route.push(attempted);
        SubmissionClaim::Granted
    }

    fn mark_submitted(&self, url: &str) {
        self.lock().grabbed_urls.insert(url.to_string());
    }
}

/// A grab one stage of a title's walk *would* make, held back so the title is
/// arbitrated once instead of the first stage to reach a submission winning by
/// arrival order.
///
/// Proposals are title-local — the walk that builds them is strictly sequential
/// and lives in a single future — so they are owned by
/// `process_background_acquisition_title` and never enter the cycle
/// coordinator, which is cross-title state.
struct GrabProposal {
    stage: BackgroundAcquisitionWorkKind,
    /// The episodes this proposal may claim, sorted and deduped.
    ///
    /// It must never under-state what the stage can actually take — a set that
    /// does lets two lanes grab the same episode. Over-stating it only defers a
    /// scope, because the greedy pass re-tests against the episodes a winner
    /// *actually* claimed: a proposal set aside here still gets its turn when
    /// the winner took less than it could have, or took nothing at all.
    ///
    /// Each stage supplies the narrowest set that satisfies that: the season
    /// pack its whole season, an episode-evidence proposal the union over its
    /// eligible candidates, and the series pack only its *best* candidate's
    /// coverage — its runner-ups are saved against other anchors and are
    /// deliberately left for those scopes to pick up.
    episode_ids: Vec<String>,
    /// The `(title, season)` a pack proposal speaks for.
    ///
    /// A season pack covers its season whether or not the catalog gave the
    /// episodes a collection to resolve through — and when it did not,
    /// `episode_ids` is empty. This is the same suppression
    /// `mark_season_pack_grabbed` used to provide the moment the pack was
    /// submitted; the submission now happens too late to protect the walk.
    season_key: Option<(String, u32)>,
    /// Best-first: the walk the inline site used to run, relocated.
    ///
    /// The *whole* ranked list, not just the submittable rows — the standby
    /// persistence the commit performs indexes into it.
    ranked_candidates: Vec<IndexerSearchResult>,
    /// Indices into `ranked_candidates`, best-first, of the rows the stage-time
    /// evaluation found submittable.
    eligible: Vec<usize>,
    commit: GrabProposalCommit,
}

enum GrabProposalCommit {
    /// Reachable only if the series-pack lane is ever deferred — it commits
    /// inline today, for the reasons on [`try_series_pack_for_title`]. The arm
    /// is kept so that flipping the lane is a matter of pushing the proposal
    /// instead of committing it.
    SeriesPack(Box<SeriesPackCommit>),
    SeasonPack(Box<SeasonPackCommit>),
    /// A covered episode scope's in-hand evidence (Phase B): candidates a
    /// season query already surfaced, ranked but never searched for.
    EpisodeEvidence(Box<EpisodeEvidenceCommit>),
}

struct SeriesPackCommit {
    anchors: HashMap<String, AcquisitionScopeState>,
    episodes: Vec<Episode>,
    blocklist: crate::app_usecase_discovery::TitleReleaseBlocklistSignatures,
}

struct SeasonPackCommit {
    season: u32,
    season_key: (String, u32),
    item: AcquisitionScopeState,
    episode: Option<Episode>,
    /// The proposal's declared season-wide episode set, unioned into the
    /// commit's actual claims so a grab claims exactly what was declared even
    /// when the submission scope resolves narrower (no collection row).
    season_episode_ids: Vec<String>,
}

struct EpisodeEvidenceCommit {
    context: ScopeGrabContext,
    blocklist: crate::app_usecase_discovery::TitleReleaseBlocklistSignatures,
}

/// Everything the episode-scope submission walk needs that is not the candidate
/// list itself. Owned, because the walk now runs after the stage that built it
/// has returned.
struct ScopeGrabContext {
    item: AcquisitionScopeState,
    episode: Option<Episode>,
    media_type: String,
    download_category: String,
    /// `None` for a scope that recorded no coverage; the ambiguous-submit
    /// re-open has nothing to prune in that case.
    convergence_scope_key: Option<String>,
}

/// What the episode-scope submission walk did, from the caller's point of view.
enum ScopeGrabOutcome {
    /// A release was submitted (or the submit was ambiguous, or a conflicting
    /// submission already existed). The scope's saved list was handled by the
    /// walk itself, so retention must not run over it.
    Settled { claimed_episode_ids: Vec<String> },
    /// Every candidate was tried and none was grabbed. The scope keeps its
    /// ranked remainder through standby retention.
    Exhausted,
}

impl GrabProposal {
    fn covers_episode(&self, episode_id: &str) -> bool {
        self.episode_ids.iter().any(|id| id == episode_id)
    }

    fn owns_season(&self, season_key: &(String, u32)) -> bool {
        self.season_key.as_ref() == Some(season_key)
    }

    /// The candidate whose release worth stands for the whole proposal.
    fn best_candidate(&self) -> Option<&IndexerSearchResult> {
        self.eligible
            .first()
            .and_then(|index| self.ranked_candidates.get(*index))
    }
}

/// Arbitrate this title's held-back grabs and commit the winners.
///
/// Best-first by release worth, greedy on episode conflicts: a proposal whose
/// episodes are already claimed — by an earlier cycle, another title, or a
/// winner committed moments ago — is set aside. Conflicts are tested against
/// what has actually been claimed rather than what the winners declared, so a
/// winner that grabbed less than its candidate list could have (or grabbed
/// nothing at all, every submit having failed) hands the next-best proposal for
/// those episodes its turn.
///
/// Returns `(committed, set_aside)` for the walk's summary line; a proposal
/// whose submissions all failed is folded into `stats` as a failed work item,
/// which is what an interactive job's terminal status reads.
async fn arbitrate_and_commit_title_grabs(
    app: &AppUseCase,
    title: &Title,
    mut proposals: Vec<GrabProposal>,
    cycle: &BackgroundAcquisitionCycleCoordinator,
    dl_snapshot: &DownloadClientSnapshot,
    now: &DateTime<Utc>,
    stats: &mut TitleWalkStats,
) -> (usize, usize) {
    if proposals.is_empty() {
        return (0, 0);
    }

    // `RankHead::compare` is the half of the search comparator that reads off a
    // scored result alone — allowed, tier, revision, score — so it is the only
    // part that means anything *between* two searches. The listing tie-breakers
    // below it (indexer priority, seeders, age) order rows within one query and
    // would be comparing unrelated things here.
    proposals.sort_by(
        |left, right| match (left.best_candidate(), right.best_candidate()) {
            (Some(left), Some(right)) => crate::acquisition::scoring::RankHead::compare(left, right),
            // A proposal with no candidate cannot win; it sorts last rather than
            // being dropped, so nothing it still owes gets skipped.
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        },
    );

    let mut claimed = cycle.claimed_episode_ids();
    let mut committed = 0usize;
    let mut set_aside = 0usize;
    for proposal in proposals {
        if proposal
            .episode_ids
            .iter()
            .any(|episode_id| claimed.contains(episode_id))
        {
            set_aside += 1;
            debug!(
                title_id = title.id.as_str(),
                stage = ?proposal.stage,
                episode_count = proposal.episode_ids.len(),
                "grab arbitration: proposal set aside, its episodes are already claimed"
            );
            continue;
        }

        let mut failures = GrabFailureTally::default();
        let grabbed = commit_grab_proposal(
            app,
            title,
            proposal,
            cycle,
            dl_snapshot,
            now,
            &mut failures,
        )
        .await;
        stats.record_grab_failures(failures);
        if !grabbed.is_empty() {
            committed += 1;
            cycle.claim_episode_ids(grabbed.iter().cloned());
            claimed.extend(grabbed);
        }
    }
    (committed, set_aside)
}

/// Run one proposal's submission walk. Returns the episodes it actually claimed,
/// and records into `failures` whether its submissions ended in a failure-class
/// error — the proposal is one work item however many candidates it burned.
async fn commit_grab_proposal(
    app: &AppUseCase,
    title: &Title,
    proposal: GrabProposal,
    cycle: &BackgroundAcquisitionCycleCoordinator,
    dl_snapshot: &DownloadClientSnapshot,
    now: &DateTime<Utc>,
    failures: &mut GrabFailureTally,
) -> Vec<String> {
    let GrabProposal {
        stage,
        ranked_candidates,
        eligible,
        commit,
        ..
    } = proposal;
    match commit {
        GrabProposalCommit::SeriesPack(commit) => {
            match commit_series_pack_proposal(
                app,
                title,
                &ranked_candidates,
                &commit,
                cycle,
                dl_snapshot,
                now,
            )
            .await
            {
                Ok(episode_ids) => episode_ids,
                Err(error) => {
                    failures.record(&error);
                    warn!(
                        title_id = title.id.as_str(),
                        error = %error,
                        "series-pack grab commit failed"
                    );
                    Vec::new()
                }
            }
        }
        GrabProposalCommit::SeasonPack(commit) => {
            match commit_season_pack_proposal(
                app,
                title,
                &ranked_candidates,
                &eligible,
                &commit,
                cycle,
                now,
                failures,
            )
            .await
            {
                Ok(episode_ids) => episode_ids,
                Err(error) => {
                    failures.record(&error);
                    warn!(
                        title_id = title.id.as_str(),
                        error = %error,
                        "season-pack grab commit failed"
                    );
                    Vec::new()
                }
            }
        }
        GrabProposalCommit::EpisodeEvidence(commit) => {
            let EpisodeEvidenceCommit {
                mut context,
                blocklist,
            } = *commit;
            let mut failed_routes = cycle.failed_routes();
            match commit_scope_grab(
                app,
                title,
                &mut context,
                &ranked_candidates,
                &eligible,
                &mut failed_routes,
                &blocklist,
                cycle,
                now,
                failures,
            )
            .await
            {
                Ok(ScopeGrabOutcome::Settled {
                    claimed_episode_ids,
                }) => claimed_episode_ids,
                // In-hand evidence that could not be grabbed leaves no trace:
                // the scope ran no query, recorded no coverage and saved no
                // standby list, so it is still a target next cycle.
                Ok(ScopeGrabOutcome::Exhausted) => Vec::new(),
                Err(error) => {
                    failures.record(&error);
                    warn!(
                        title_id = title.id.as_str(),
                        stage = ?stage,
                        error = %error,
                        "episode-evidence grab commit failed"
                    );
                    Vec::new()
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
enum BackgroundAcquisitionWorkKind {
    TitlePack,
    SeasonPack { season: u32 },
    Scope,
}

/// Who asked for a title's staged walk.
///
/// The stages, the proposal set and the arbitration are identical either way —
/// that is the point of running one walk for both callers. The intent only
/// decides which of the *economy* gates apply: the ones that exist so a machine
/// sweep does not spend indexer budget re-asking questions it already has
/// answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AcquisitionWalkIntent {
    /// The convergence cycle. A converged scope rides RSS instead of searching,
    /// and a scope whose every uncovered indexer is cooling down or quota-spent
    /// is pre-skipped rather than handed to the scheduler.
    Background,
    /// An operator asked for this title now. Coverage is still *recorded* — the
    /// walk really did run those queries — but it is not *read* as a reason to
    /// skip, and the scheduler is left to admit or defer each query itself
    /// rather than the walk pre-skipping on its behalf. Searches run in the
    /// interactive lane (no background candidate value), which is what bypasses
    /// destination cooldown and corpus reuse.
    Interactive,
}

impl AcquisitionWalkIntent {
    fn is_interactive(self) -> bool {
        matches!(self, Self::Interactive)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Interactive => "interactive",
        }
    }

    /// The candidate-value hint a search issued under this intent carries.
    ///
    /// `None` is the operator lane: the search client admits it through the
    /// interactive semaphore, skips the background corpus reuse, and the
    /// scheduler bypasses destination cooldown for it.
    fn background_value(self, target: &crate::acquisition::targets::AcquisitionTarget) -> Option<f64> {
        match self {
            Self::Interactive => None,
            Self::Background => Some(if target.is_hot {
                BACKGROUND_HOT_TARGET_VALUE
            } else {
                BACKGROUND_COLD_TARGET_VALUE
            }),
        }
    }
}

/// The walk-wide inputs one title's staged walk runs under.
#[derive(Clone)]
pub(crate) struct TitleWalkOptions {
    intent: AcquisitionWalkIntent,
    /// Restrict the walk to one season. `None` walks the whole title.
    season_filter: Option<u32>,
    /// Cancelled by the operator's job; the background cycle passes a token it
    /// never cancels, so the two paths use one code path.
    cancellation: tokio_util::sync::CancellationToken,
}

impl TitleWalkOptions {
    fn background() -> Self {
        Self {
            intent: AcquisitionWalkIntent::Background,
            season_filter: None,
            cancellation: tokio_util::sync::CancellationToken::new(),
        }
    }

    /// The search token for a query this walk issues.
    fn search_cancellation(&self) -> tokio_util::sync::CancellationToken {
        self.cancellation.clone()
    }
}

/// What one title's walk did, for the summary line and the job's progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TitleWalkStats {
    /// Work items processed: pack stages plus episode scopes.
    pub stages: usize,
    /// Indexer queries this walk issued. A stage that rode cached coverage,
    /// replayed a saved candidate list or was pre-skipped issues none.
    pub queries: usize,
    /// Grabs committed during the walk itself — the saved-candidate replays,
    /// the series-pack lane and the uncovered episode scopes.
    pub inline_grabs: usize,
    /// Proposals held back for the end-of-walk arbitration.
    pub proposals: usize,
    /// Proposals the arbitration committed.
    pub committed: usize,
    /// Proposals the arbitration set aside because their episodes were already
    /// claimed.
    pub set_aside: usize,
    /// Work items whose submission attempts all ended in a failure-class error
    /// — one per episode scope or committed proposal, however many candidates
    /// it burned through.
    ///
    /// This is what a job's terminal status is computed from, so the
    /// classification is the same one the flat per-scope path uses
    /// (`scope_search_error_is_failure`): a stage that found nothing, or found
    /// nothing auto-eligible, ran a complete search and is not a failure.
    pub failed: usize,
    /// Whether any of those failures was a retryable download-submission
    /// failure — a mapped client that is disabled or unreachable. It fails a
    /// job outright rather than warning, because nothing the operator asked for
    /// could even be attempted.
    pub submit_unavailable: bool,
}

/// What one work item's submission attempts amounted to.
///
/// A work item may try several candidates; only the item as a whole counts, so
/// this is folded once at the end rather than incremented per candidate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct GrabFailureTally {
    /// A submit or dispatch error of failure class was observed.
    failed: bool,
    /// At least one of them was a retryable download-submission failure.
    submit_unavailable: bool,
}

impl GrabFailureTally {
    fn record(&mut self, error: &AppError) {
        if !crate::acquisition::wanted_views::scope_search_error_is_failure(error) {
            return;
        }
        self.failed = true;
        self.submit_unavailable |= error.is_retryable_download_submit_failure();
    }
}

impl TitleWalkStats {
    /// Fold one work item's submission attempts into the walk's counters.
    fn record_grab_failures(&mut self, tally: GrabFailureTally) {
        if tally.failed {
            self.failed += 1;
        }
        self.submit_unavailable |= tally.submit_unavailable;
    }
}

/// The mutable state one title's walk carries between its stages: the proposals
/// held for arbitration and the counters behind the summary line.
///
/// Deliberately not in the cycle coordinator: the walk is strictly sequential
/// inside one future, so this needs no locking, and none of it means anything
/// to another title.
struct TitleWalkSession {
    options: TitleWalkOptions,
    proposals: Vec<GrabProposal>,
    stats: TitleWalkStats,
}

impl TitleWalkSession {
    fn new(options: TitleWalkOptions) -> Self {
        Self {
            options,
            proposals: Vec::new(),
            stats: TitleWalkStats::default(),
        }
    }

    fn intent(&self) -> AcquisitionWalkIntent {
        self.options.intent
    }
}

#[derive(Clone, Debug)]
struct BackgroundAcquisitionWork {
    target_index: usize,
    kind: BackgroundAcquisitionWorkKind,
}

#[derive(Debug)]
struct BackgroundAcquisitionTitleWork {
    title_id: String,
    ready: VecDeque<BackgroundAcquisitionWork>,
}

/// Order a title's selected targets for the walk: `(season, episode)`
/// ascending, unnumbered last, stable among equals.
///
/// Derived order is the datastore's, and `list_missing_scope_candidates` orders
/// episodes by their random-UUID id — so a 226-episode series walked in derived
/// order reaches the episode an operator is waiting for at a random position.
/// Sorting costs nothing here and makes the walk (and its time-to-first-grab)
/// predictable. It changes only the order; the selected set is untouched.
fn sort_title_work_indices(
    targets: &[crate::acquisition::targets::AcquisitionTarget],
    indices: &mut [usize],
) {
    indices.sort_by_key(|index| crate::acquisition::targets::scope_walk_order_key(&targets[*index]));
}

/// A target's season number, when it names a real season (0 is the specials /
/// series-movie bucket and has no season pack of its own).
fn packable_season_for_target(
    target: &crate::acquisition::targets::AcquisitionTarget,
) -> Option<u32> {
    target
        .season_number
        .as_deref()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|season| *season > 0)
}

/// Build the per-title ready queues: pack stages first, then episode scopes.
///
/// `season_filter` restricts a walk to one season — the shape an operator's
/// season-scoped request asks for. The title-pack stage is dropped in that
/// case: a whole-series release answers for seasons the request never asked
/// about, and committing one would grab far more than was requested.
fn build_background_acquisition_title_work(
    targets: &[crate::acquisition::targets::AcquisitionTarget],
    selected_indices: &[usize],
    season_filter: Option<u32>,
) -> VecDeque<BackgroundAcquisitionTitleWork> {
    let mut title_order = Vec::new();
    let mut indices_by_title = HashMap::<String, Vec<usize>>::new();
    for &target_index in selected_indices {
        if let Some(season) = season_filter
            && packable_season_for_target(&targets[target_index]) != Some(season)
        {
            continue;
        }
        let title_id = targets[target_index].title_id.clone();
        if !indices_by_title.contains_key(&title_id) {
            title_order.push(title_id.clone());
        }
        indices_by_title
            .entry(title_id)
            .or_default()
            .push(target_index);
    }

    title_order
        .into_iter()
        .filter_map(|title_id| {
            let mut indices = indices_by_title.remove(&title_id)?;
            sort_title_work_indices(targets, &mut indices);
            let mut ready = VecDeque::new();
            let episode_indices = indices
                .iter()
                .copied()
                .filter(|index| targets[*index].media_type == "episode")
                .collect::<Vec<_>>();
            // The title-pack stage doubles as the pack stage for its anchor's
            // season, so the remaining season packs are the seasons it did not
            // already speak for — emitted in ascending season order because
            // `episode_indices` is sorted.
            let title_pack_index = season_filter
                .is_none()
                .then(|| episode_indices.first().copied())
                .flatten();
            if let Some(title_pack_index) = title_pack_index {
                ready.push_back(BackgroundAcquisitionWork {
                    target_index: title_pack_index,
                    kind: BackgroundAcquisitionWorkKind::TitlePack,
                });
            }
            let mut seen_seasons = HashSet::new();
            for target_index in episode_indices {
                let Some(season) = packable_season_for_target(&targets[target_index]) else {
                    continue;
                };
                if !seen_seasons.insert(season) || Some(target_index) == title_pack_index {
                    continue;
                }
                ready.push_back(BackgroundAcquisitionWork {
                    target_index,
                    kind: BackgroundAcquisitionWorkKind::SeasonPack { season },
                });
            }
            ready.extend(
                indices
                    .into_iter()
                    .map(|target_index| BackgroundAcquisitionWork {
                        target_index,
                        kind: BackgroundAcquisitionWorkKind::Scope,
                    }),
            );
            Some(BackgroundAcquisitionTitleWork { title_id, ready })
        })
        .collect()
}

fn episode_ids_for_scope(scope: &SubmissionScope) -> Option<&[String]> {
    match scope {
        SubmissionScope::EpisodeSet { episode_ids } => Some(episode_ids),
        _ => None,
    }
}

async fn recovered_scope_episode_ids(app: &AppUseCase, scope: &SubmissionScope) -> Vec<String> {
    match scope {
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids.clone(),
        SubmissionScope::Episode { episode_id } => vec![episode_id.clone()],
        SubmissionScope::Collection { collection_id } => match app
            .services
            .catalog
            .shows
            .list_episodes_for_collection(collection_id)
            .await
        {
            Ok(episodes) => episodes.into_iter().map(|episode| episode.id).collect(),
            Err(error) => {
                warn!(
                    collection_id,
                    error = %error,
                    "series-pack search: failed to expand recovered collection coverage"
                );
                Vec::new()
            }
        },
        SubmissionScope::Title | SubmissionScope::SeriesMovie { .. } | SubmissionScope::Orphan => {
            Vec::new()
        }
    }
}

/// Put back a standby list that a replacement write cleared but did not
/// refill. `persist_standby_candidates` deletes before it inserts, so every
/// caller that can fail partway needs this.
async fn restore_standby_releases(
    app: &AppUseCase,
    anchor: &AcquisitionScopeState,
    standby_releases: &[PendingRelease],
) {
    let _ = app
        .services
        .workflow
        .pending_releases
        .delete_standby_pending_releases_for_wanted_item(&anchor.id)
        .await;
    for standby in standby_releases {
        if let Err(error) = app
            .services
            .workflow
            .pending_releases
            .insert_pending_release(standby)
            .await
        {
            warn!(
                wanted_item_id = anchor.id.as_str(),
                release = standby.release_title.as_str(),
                error = %error,
                "failed to restore standby candidate"
            );
        }
    }
}

/// Two results that are the same posting from the same indexer.
fn same_indexer_release(left: &IndexerSearchResult, right: &IndexerSearchResult) -> bool {
    left.indexer_id == right.indexer_id && left.guid == right.guid && left.title == right.title
}

fn is_series_pack_candidate(candidate: &IndexerSearchResult) -> bool {
    candidate
        .parsed_release_metadata
        .as_ref()
        .and_then(|parsed| parsed.episode.as_ref())
        .is_some_and(|episode| episode.is_series_pack)
}

/// One title lookup for a whole-series or multi-season release, planned and
/// committed back to back.
///
/// **This lane still commits inline, and deliberately so.** Its runner-ups are
/// persisted against their own anchors the moment a pack is grabbed, and the
/// sibling episode scopes later in the same walk pick a *disjoint* pack up
/// through their saved-candidate walks — which only works because the winner
/// has already claimed its episodes by then. Holding the grab back until the
/// end of the walk would leave those scopes with nothing saved to find, and
/// nothing claimed to exclude the overlapping runner-up by. The lane also runs
/// before any season query, so at the point it decides there is no in-hand
/// episode evidence for it to be ranked against anyway.
///
/// The split into [`plan_series_pack_for_title`] and
/// [`commit_series_pack_proposal`] is kept: deferring this lane later is a
/// matter of moving the second call, not of taking the function apart again.
#[expect(
    clippy::too_many_arguments,
    reason = "the one-shot title lookup needs the cycle search state"
)]
async fn try_series_pack_for_title(
    app: &AppUseCase,
    title: &Title,
    search_title: &Title,
    target: &crate::acquisition::targets::AcquisitionTarget,
    now: &DateTime<Utc>,
    availability: &crate::acquisition::convergence::SchedulerAvailability,
    indexer_hosts: &HashMap<String, String>,
    dl_snapshot: &DownloadClientSnapshot,
    submissions: &[DownloadSubmission],
    tracked_states: &HashMap<
        crate::contracts::ClientJobLocator,
        scryer_domain::TrackedDownloadState,
    >,
    claimed_episode_ids: &HashSet<String>,
    cycle: &BackgroundAcquisitionCycleCoordinator,
    session: &mut TitleWalkSession,
) -> AppResult<Option<Vec<String>>> {
    let Some(proposal) = plan_series_pack_for_title(
        app,
        title,
        search_title,
        target,
        availability,
        indexer_hosts,
        dl_snapshot,
        submissions,
        tracked_states,
        claimed_episode_ids,
        session,
    )
    .await?
    else {
        return Ok(None);
    };
    let GrabProposalCommit::SeriesPack(commit) = &proposal.commit else {
        return Ok(None);
    };
    // The series-pack lane's submissions run inside the standby-recovery walk,
    // which reports an exhausted list rather than the submit error behind it —
    // so this lane contributes no failure count. Every other stage does, and an
    // interactive job that could not submit anywhere sees it from those.
    let episode_ids = commit_series_pack_proposal(
        app,
        title,
        &proposal.ranked_candidates,
        commit,
        cycle,
        dl_snapshot,
        now,
    )
    .await?;
    Ok((!episode_ids.is_empty()).then_some(episode_ids))
}

/// Search for a whole-series or multi-season release and, if one qualifies,
/// describe it as a proposal.
///
/// Everything that is a fact about what the indexers hold happens here — the
/// query, the evaluation, the anchor resolution and the coverage record. Only
/// the submission walk is left to the commit half.
#[expect(
    clippy::too_many_arguments,
    reason = "the one-shot title lookup needs the cycle search state"
)]
async fn plan_series_pack_for_title(
    app: &AppUseCase,
    title: &Title,
    search_title: &Title,
    target: &crate::acquisition::targets::AcquisitionTarget,
    availability: &crate::acquisition::convergence::SchedulerAvailability,
    indexer_hosts: &HashMap<String, String>,
    dl_snapshot: &DownloadClientSnapshot,
    submissions: &[DownloadSubmission],
    tracked_states: &HashMap<
        crate::contracts::ClientJobLocator,
        scryer_domain::TrackedDownloadState,
    >,
    claimed_episode_ids: &HashSet<String>,
    session: &mut TitleWalkSession,
) -> AppResult<Option<GrabProposal>> {
    let mut title_subject = app
        .resolve_release_search_subject_for_title(search_title)
        .await?;
    title_subject.submission_scope = SubmissionScope::Title;
    let episodes = match app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await
    {
        Ok(episodes) => episodes,
        Err(error) => {
            warn!(
                title_id = title.id.as_str(),
                error = %error,
                "series-pack search: failed to load episodes"
            );
            return Ok(None);
        }
    };
    let mut owned_episode_ids = match app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
    {
        Ok(files) => files
            .into_iter()
            .filter(|file| file.role.is_primary())
            .filter_map(|file| file.episode_id)
            .collect::<HashSet<_>>(),
        Err(error) => {
            warn!(
                title_id = title.id.as_str(),
                error = %error,
                "series-pack search: failed to load media ownership"
            );
            return Ok(None);
        }
    };
    owned_episode_ids.extend(
        crate::acquisition_coverage::in_flight_series_pack_episode_ids(
            &episodes,
            submissions,
            tracked_states,
            dl_snapshot,
        ),
    );
    let eligible_collection_ids =
        crate::acquisition_coverage::eligible_series_pack_collection_ids(&episodes);
    if eligible_collection_ids.is_empty()
        || crate::acquisition_coverage::eligible_missing_series_pack_episode_count(
            &episodes,
            &owned_episode_ids,
        ) < 2
        || !crate::acquisition_coverage::title_series_pack_missing_ratio_qualifies(
            &episodes,
            &owned_episode_ids,
        )
    {
        return Ok(None);
    }

    let Some(convergence) = app
        .resolve_series_pack_convergence(search_title, &title_subject, &eligible_collection_ids)
        .await
    else {
        return Ok(None);
    };
    let uncovered = match app
        .uncovered_indexers_for_scope(
            &convergence.scope_key,
            &convergence.facet,
            &convergence.fingerprint,
            &convergence.routed_indexer_ids,
        )
        .await
    {
        Ok(uncovered) => uncovered,
        Err(error) => {
            warn!(
                title_id = title.id.as_str(),
                scope_key = convergence.scope_key.as_str(),
                error = %error,
                "series-pack search: failed to read set coverage; searching all routed indexers"
            );
            convergence.routed_indexer_ids.clone()
        }
    };
    // An interactive walk reads neither gate: the operator asked for this title
    // now, so a converged set is re-asked and admission is left to the
    // scheduler, which bypasses destination cooldown for the interactive lane.
    let intent = session.intent();
    let searchable = if intent.is_interactive() {
        convergence.routed_indexer_ids.clone()
    } else {
        if uncovered.is_empty()
            || !uncovered.iter().any(|indexer_id| {
                availability.indexer_available(
                    indexer_hosts.get(indexer_id).map(String::as_str),
                    indexer_id,
                )
            })
        {
            return Ok(None);
        }
        uncovered
    };
    if searchable.is_empty() {
        return Ok(None);
    }
    session.stats.queries += 1;
    let search_outcome = app
        .search_and_score_subject_restricted_with_fired_indexers(
            search_title,
            &title_subject,
            "background_acquisition_series_pack",
            SearchMode::Auto,
            session.options.search_cancellation(),
            Some(searchable.into_iter().collect()),
            intent.background_value(target),
        )
        .await?;

    let (evaluated_candidates, qualifying_collection_ids) = evaluate_series_pack_candidates(
        app,
        title,
        &title_subject,
        search_outcome.results,
        &episodes,
        &owned_episode_ids,
        claimed_episode_ids,
    )
    .await;
    let session_finalized = app
        .finalize_evaluated_search_session_or_warn(
            &search_outcome.search_session_id,
            &evaluated_candidates,
            &title.id,
        )
        .await;

    if evaluated_candidates.is_empty() {
        if session_finalized {
            record_series_pack_search_coverage(
                app,
                &convergence,
                &search_outcome.complete_indexer_ids,
                &qualifying_collection_ids,
            )
            .await;
        }
        return Ok(None);
    }

    let anchors =
        series_pack_candidate_anchors(app, title, &evaluated_candidates, &episodes).await?;
    if session_finalized {
        record_series_pack_search_coverage(
            app,
            &convergence,
            &search_outcome.complete_indexer_ids,
            &qualifying_collection_ids,
        )
        .await;
    }
    let blocklist = app.load_title_release_blocklist_signatures(&title.id).await;

    // Only the *best* candidate's coverage here, not the union across the list.
    //
    // The runner-ups are not this proposal's fallbacks in the usual sense:
    // `persist_series_pack_runner_ups` saves each against its own anchor, and a
    // later episode scope picks a disjoint one up through its saved-candidate
    // walk — that is how two non-overlapping packs are grabbed in one cycle.
    // Declaring the union would suppress exactly those scopes.
    let episode_ids = match evaluated_candidates
        .first()
        .and_then(|candidate| candidate.parsed_release_metadata.as_ref())
    {
        Some(parsed) => {
            let scope = crate::acquisition_coverage::resolve_release_coverage(
                parsed, &episodes, &[], None,
            )
            .submission_scope();
            let mut episode_ids = recovered_scope_episode_ids(app, &scope).await;
            episode_ids.sort();
            episode_ids.dedup();
            episode_ids
        }
        None => Vec::new(),
    };

    // `evaluate_series_pack_candidates` has already dropped everything the
    // walk would refuse, so the whole ranked list is submittable.
    let eligible = (0..evaluated_candidates.len()).collect();

    Ok(Some(GrabProposal {
        stage: BackgroundAcquisitionWorkKind::TitlePack,
        episode_ids,
        season_key: None,
        ranked_candidates: evaluated_candidates,
        eligible,
        commit: GrabProposalCommit::SeriesPack(Box::new(SeriesPackCommit {
            anchors,
            episodes,
            blocklist,
        })),
    }))
}

/// Submit the winning series-pack proposal, walking its ranked list exactly as
/// the inline lookup used to. Returns the episodes the grab covers.
async fn commit_series_pack_proposal(
    app: &AppUseCase,
    title: &Title,
    evaluated_candidates: &[IndexerSearchResult],
    commit: &SeriesPackCommit,
    cycle: &BackgroundAcquisitionCycleCoordinator,
    dl_snapshot: &DownloadClientSnapshot,
    now: &DateTime<Utc>,
) -> AppResult<Vec<String>> {
    let SeriesPackCommit {
        anchors,
        episodes,
        blocklist,
    } = commit;
    let failed_routes = cycle.failed_routes();
    let claimed_episode_ids = &cycle.claimed_episode_ids();
    let failed_routes = failed_routes.as_slice();

    for (candidate_index, candidate) in evaluated_candidates.iter().enumerate() {
        let key = crate::app_usecase_discovery::release_search_key(candidate);
        let Some(anchor) = anchors.get(&key) else {
            continue;
        };
        let preserved_standby = match app
            .services
            .workflow
            .pending_releases
            .list_standby_pending_releases_for_wanted_item(&anchor.id)
            .await
        {
            Ok(standby) => standby,
            Err(error) => {
                warn!(
                    wanted_item_id = anchor.id.as_str(),
                    error = %error,
                    "series-pack search: failed to snapshot anchor standby candidates"
                );
                continue;
            }
        };
        if !persist_standby_candidates(
            app,
            anchor,
            title,
            evaluated_candidates,
            candidate_index,
            now,
            failed_routes,
            blocklist,
            |saved| crate::app_usecase_discovery::release_search_key(saved) == key,
        )
        .await
        {
            restore_standby_releases(app, anchor, &preserved_standby).await;
            continue;
        }

        let Some(candidate_scope) = candidate.parsed_release_metadata.as_ref().map(|parsed| {
            crate::acquisition_coverage::resolve_release_coverage(parsed, episodes, &[], None)
                .submission_scope()
        }) else {
            warn!(
                release = candidate.title.as_str(),
                "series-pack search: evaluated candidate lost parsed metadata"
            );
            restore_standby_releases(app, anchor, &preserved_standby).await;
            continue;
        };
        let outcome = try_saved_candidates(
            app,
            anchor,
            None,
            Some(claimed_episode_ids),
            dl_snapshot,
            now,
        )
        .await;
        let (scope, standby_start, recovered) = match outcome {
            StandbyRecoveryOutcome::Recovered { scope } => (Some(scope), candidate_index + 1, true),
            StandbyRecoveryOutcome::Active { scope } => (Some(scope), candidate_index + 1, false),
            StandbyRecoveryOutcome::Deferred { scope } => (scope, candidate_index, false),
            StandbyRecoveryOutcome::Parked { scope } => {
                let candidate_is_parked = scope.as_ref() == Some(&candidate_scope);
                (
                    scope,
                    if candidate_is_parked {
                        candidate_index + 1
                    } else {
                        candidate_index
                    },
                    false,
                )
            }
            StandbyRecoveryOutcome::Exhausted { .. } => {
                restore_standby_releases(app, anchor, &preserved_standby).await;
                continue;
            }
        };
        if recovered {
            persist_series_pack_runner_ups(
                app,
                title,
                evaluated_candidates,
                standby_start,
                anchors,
                now,
                failed_routes,
                blocklist,
            )
            .await;
        } else {
            restore_standby_releases(app, anchor, &preserved_standby).await;
        }
        return Ok(match scope {
            Some(scope) => recovered_scope_episode_ids(app, &scope).await,
            None => Vec::new(),
        });
    }

    Ok(Vec::new())
}

/// Submit the winning season-pack proposal, walking its ranked list exactly as
/// the inline season stage used to. Returns the episodes the grab covers.
#[expect(
    clippy::too_many_arguments,
    reason = "the relocated pack submission carries the cycle inputs plus its failure tally"
)]
async fn commit_season_pack_proposal(
    app: &AppUseCase,
    title: &Title,
    pack_results: &[IndexerSearchResult],
    eligible: &[usize],
    commit: &SeasonPackCommit,
    cycle: &BackgroundAcquisitionCycleCoordinator,
    now: &DateTime<Utc>,
    failures: &mut GrabFailureTally,
) -> AppResult<Vec<String>> {
    let season_num = commit.season;
    let season_key = commit.season_key.clone();
    let item = &commit.item;
    let episode = &commit.episode;
    let mut failed_routes = cycle.failed_routes();

    'season_pack_candidates: for best_pack_index in eligible.iter().copied() {
        let best_pack = &pack_results[best_pack_index];
        let pack_route = DownloadRouteKey::for_candidate(best_pack)
            .expect("candidate route key always exists, including unknown source kind");
        if failed_routes.contains(&pack_route) {
            continue;
        }
        let pack_url = best_pack
            .canonical_download_source()
            .map(|(source, _)| source);
        let url_str = pack_url.as_deref().unwrap_or("").to_string();
        if !url_str.is_empty()
            && matches!(
                cycle.claim_submission(pack_route.clone(), &url_str),
                SubmissionClaim::Granted
            )
        {
            let download_cat = app.derive_download_category(&title.facet).await;
            let is_recent = app.is_recent_for_queue_priority(
                best_pack
                    .published_at
                    .as_deref()
                    .or(episode.as_ref().and_then(|item| item.air_date.as_deref()))
                    .or(title.first_aired.as_deref())
                    .or(title.digital_release_date.as_deref()),
            );
            let pack_title = Some(best_pack.title.clone());
            let pack_hint = normalize_release_attempt_hint(pack_url.as_deref());
            let pack_title_norm = normalize_release_name(pack_title.as_deref());
            let pack_password =
                normalize_release_password(best_pack.password_hint.as_deref());
            let request_signature = normalize_release_selection_signature(
                pack_url.as_deref(),
                pack_title.as_deref(),
                best_pack.source_kind,
            );
            let info_hash_hint = best_pack.info_hash().map(str::to_string);
            let seed_minimums =
                crate::ReleaseSeedMinimums::from_release_extra(&best_pack.extra);
            let download_id = scryer_domain::download_identity::DownloadId::new();
            let submission_scope = collection_download_submission_scope_for_wanted_item(
                item,
                episode.as_ref(),
            );

            let canonical_result = app
                .submit_canonical_download(CanonicalDownloadSubmissionIntent {
                    request: DownloadClientAddRequest {
                        title: title.clone(),
                        search_facet: None,
                        purpose: crate::DownloadSubmissionPurpose::Standard,
                        download_id: Some(download_id),
                        source_hint: pack_url.clone(),
                        staged_nzb: None,
                        resolved_download_artifact: None,
                        source_kind: best_pack.source_kind,
                        source_title: pack_title.clone(),
                        source_password: pack_password.clone(),
                        category: Some(download_cat),
                        queue_priority: None,
                        download_directory: None,
                        release_title: Some(best_pack.title.clone()),
                        indexer_name: Some(best_pack.source.clone()),
                        indexer_id: best_pack.indexer_id.clone(),
                        info_hash_hint: info_hash_hint.clone(),
                        seed_goal_ratio: None,
                        seed_goal_seconds: None,
                        tracker_min_seed_ratio: seed_minimums.min_seed_ratio,
                        tracker_min_seed_time_minutes: seed_minimums
                            .min_seed_time_minutes,
                        season_pack_seed_ratio: seed_minimums.season_pack_seed_ratio,
                        season_pack_seed_time_minutes: seed_minimums
                            .season_pack_seed_time_minutes,
                        is_recent,
                        season_pack: Some(true),
                        pinned_download_client_id: None,
                    },
                    scope: submission_scope.clone(),
                    conflict_policy: SubmissionConflictPolicy::Skip,
                    request_signature: request_signature.clone(),
                    source_provider_name: Some(best_pack.source.clone()),
                    release_size_bytes: best_pack.size_bytes,
                })
                .await;

            let grab_indexer = app
                .grab_indexer_name(
                    best_pack.indexer_id.as_deref(),
                    Some(best_pack.source.as_str()),
                )
                .await;
            record_grab_submission_outcome(
                GrabTrigger::SeasonPack,
                &title.facet,
                grab_indexer.as_deref(),
                &canonical_result,
            );

            let canonical_submission = match canonical_result {
                Ok(CanonicalDownloadSubmissionOutcome::Accepted(submission)) => {
                    Ok(submission)
                }
                Ok(CanonicalDownloadSubmissionOutcome::Conflict(_)) => {
                    break 'season_pack_candidates;
                }
                Err(error) => Err(error),
            };

            match canonical_submission {
                Ok(canonical_submission) => {
                    let newly_submitted = canonical_submission.newly_submitted;
                    let grab = canonical_submission.grab;
                    let download_job_id = grab.job_id.clone();
                    if newly_submitted {
                        app.record_indexer_grab(
                            best_pack.indexer_id.as_deref(),
                            grab_indexer.as_deref(),
                        );
                    }
                    cycle.mark_submitted(&url_str);
                    cycle.mark_season_pack_grabbed(&season_key);
                    let _ = app
                        .services
                        .workflow
                        .release_attempts
                        .record_release_attempt(
                            Some(title.id.clone()),
                            pack_hint,
                            pack_title_norm,
                            ReleaseDownloadAttemptOutcome::Success,
                            None,
                            pack_password,
                        )
                        .await;
                    let mut grabbed_episode_ids = match &submission_scope {
                        SubmissionScope::Episode { episode_id } => {
                            vec![episode_id.clone()]
                        }
                        SubmissionScope::EpisodeSet { episode_ids } => {
                            episode_ids.clone()
                        }
                        SubmissionScope::Collection { collection_id } => app
                            .services
                            .catalog
                            .shows
                            .list_episodes_for_collection(collection_id)
                            .await
                            .map(|episodes| {
                                episodes.into_iter().map(|episode| episode.id).collect()
                            })
                            .unwrap_or_default(),
                        SubmissionScope::Title
                        | SubmissionScope::SeriesMovie { .. }
                        | SubmissionScope::Orphan => Vec::new(),
                    };
                    // Claim what was declared: the arbitration's conflict test
                    // reads these actual claims, and a claim narrower than the
                    // season-wide declaration would let a covered episode
                    // proposal commit on top of this pack.
                    grabbed_episode_ids.extend(commit.season_episode_ids.iter().cloned());
                    grabbed_episode_ids.sort();
                    grabbed_episode_ids.dedup();
                    let covered_wanted_item_ids = app
                        .covered_wanted_item_ids_for_submission_scope(
                            &title.id,
                            &submission_scope,
                            &item.id,
                        )
                        .await?;
                    let grabbed_json = serde_json::json!({
                        "title": best_pack.title,
                        "score": best_pack
                            .quality_profile_decision
                            .as_ref()
                            .map(|decision| decision.preference_score)
                            .unwrap_or(0),
                        "grabbed_at": now.to_rfc3339(),
                        "season_pack": true,
                        "source_provider": best_pack.source.clone(),
                    })
                    .to_string();
                    app.services
                        .workflow
                        .acquisition_state
                        .commit_successful_grab(&SuccessfulGrabCommit {
                            wanted_item_id: item.id.clone(),
                            covered_wanted_item_ids,
                            grabbed_release: grabbed_json,
                            last_search_at: Some(now.to_rfc3339()),
                            grabbed_pending_release_id: None,
                            grabbed_at: Some(now.to_rfc3339()),
                        })
                        .await?;
                    let pack_blocklist =
                        app.load_title_release_blocklist_signatures(&title.id).await;
                    persist_standby_candidates(
                        app,
                        item,
                        title,
                        pack_results,
                        best_pack_index + 1,
                        now,
                        &failed_routes,
                        &pack_blocklist,
                        |candidate| {
                            candidate_is_season_pack_for_season(candidate, season_num)
                        },
                    )
                    .await;
                    let pack_score = best_pack
                        .quality_profile_decision
                        .as_ref()
                        .map(|d| d.preference_score)
                        .unwrap_or(0);
                    let mut grab_meta = HashMap::new();
                    grab_meta.insert(
                        "title_name".to_string(),
                        serde_json::json!(title.name),
                    );
                    grab_meta.insert(
                        "release_title".to_string(),
                        serde_json::json!(best_pack.title),
                    );
                    grab_meta.insert(
                        "indexer".to_string(),
                        serde_json::json!(best_pack.source),
                    );
                    grab_meta
                        .insert("score".to_string(), serde_json::json!(pack_score));
                    let _ = app
                        .append_domain_event(new_title_domain_event(
                            None,
                            title,
                            DomainEventPayload::ReleaseGrabbed(
                                ReleaseGrabbedEventData {
                                    title: title_context_snapshot(title),
                                    source_title: Some(best_pack.title.clone()),
                                    source_hint: Some(best_pack.source.clone()),
                                    source_provider: Some(best_pack.source.clone()),
                                    download_id: Some(download_job_id),
                                    episode_ids: grabbed_episode_ids.clone(),
                                },
                            ),
                        ))
                        .await;
                    info!(
                        title = title.name.as_str(),
                        season = season_num,
                        release = best_pack.title.as_str(),
                        "season pack grabbed; skipping individual episode searches for this season"
                    );
                    return Ok(grabbed_episode_ids);
                }
                Err(err) => {
                    let submit_unavailable = is_download_submit_unavailable_error(&err);
                    let ambiguous = err.is_download_submit_ambiguous();
                    // An ambiguous submit may well have been accepted, so it is
                    // not counted against the job; every other shape is.
                    if !ambiguous {
                        failures.record(&err);
                    }
                    if submit_unavailable && !failed_routes.contains(&pack_route) {
                        failed_routes.push(pack_route.clone());
                        cycle.mark_failed_route(pack_route.clone());
                    }
                    if ambiguous {
                        cycle.mark_submitted(&url_str);
                        cycle.mark_season_pack_viable(&season_key);
                    } else if !submit_unavailable {
                        cycle.clear_season_pack_viable(&season_key);
                    }
                    warn!(
                        title = title.name.as_str(),
                        season = season_num,
                        error = %err,
                        retry_alternate_route = submit_unavailable,
                        "season pack grab failed"
                    );
                    // Transient (client unavailable) and ambiguous
                    // (request may have been accepted) submits are
                    // deferred: Pending attempt, never blocklisted.
                    // Only a definitive failure burns the pack.
                    let source_gone = err.is_download_source_gone();
                    let defer = submit_unavailable || ambiguous || source_gone;
                    let _ = app
                        .services
                        .workflow
                        .release_attempts
                        .record_release_attempt(
                            Some(title.id.clone()),
                            pack_hint.clone(),
                            pack_title_norm.clone(),
                            if defer {
                                ReleaseDownloadAttemptOutcome::Pending
                            } else {
                                ReleaseDownloadAttemptOutcome::Failed
                            },
                            Some(err.to_string()),
                            pack_password,
                        )
                        .await;
                    if !defer && let Some(release_name) = pack_title_norm
                        && let Err(error) = app
                            .services
                            .workflow
                            .blocklist_repo
                            .block(&NewBlocklistEntry {
                                title_id: title.id.clone(),
                                release_name,
                                indexer_id: best_pack
                                    .indexer_id
                                    .clone()
                                    .unwrap_or_default(),
                                info_hash: best_pack.info_hash().map(str::to_string),
                                reason: Some(format!("season pack grab failed: {err}")),
                            })
                            .await
                        {
                            warn!(
                                error = %error,
                                title_id = title.id.as_str(),
                                release = best_pack.title.as_str(),
                                "failed to persist blocklist entry for failed season pack grab"
                            );
                        }
                    if !submit_unavailable {
                        break 'season_pack_candidates;
                    }
                }
            }
        }
    }

    Ok(Vec::new())
}

async fn evaluate_series_pack_candidates(
    app: &AppUseCase,
    title: &Title,
    title_subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
    candidates: Vec<IndexerSearchResult>,
    episodes: &[Episode],
    owned_episode_ids: &HashSet<String>,
    claimed_episode_ids: &HashSet<String>,
) -> (Vec<IndexerSearchResult>, HashSet<String>) {
    let mut groups = HashMap::<Vec<String>, Vec<(usize, IndexerSearchResult)>>::new();
    let mut collection_ids = HashSet::new();

    for (rank, candidate) in candidates.into_iter().enumerate() {
        if !is_series_pack_candidate(&candidate) {
            continue;
        }
        let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
            continue;
        };
        if !crate::acquisition_coverage::series_pack_missing_ratio_qualifies(
            parsed,
            episodes,
            owned_episode_ids,
        ) {
            continue;
        }

        collection_ids.extend(crate::acquisition_coverage::series_pack_collection_ids(
            parsed, episodes,
        ));
        let scope =
            crate::acquisition_coverage::resolve_release_coverage(parsed, episodes, &[], None)
                .submission_scope();
        let Some(mut episode_ids) = episode_ids_for_scope(&scope).map(<[String]>::to_vec) else {
            continue;
        };
        episode_ids.sort();
        episode_ids.dedup();
        if episode_ids
            .iter()
            .any(|episode_id| claimed_episode_ids.contains(episode_id))
        {
            continue;
        }
        groups
            .entry(episode_ids)
            .or_default()
            .push((rank, candidate));
    }

    let mut evaluated = Vec::new();
    for (episode_ids, ranked_candidates) in groups {
        let mut ranks_by_key = HashMap::new();
        let mut candidates = Vec::with_capacity(ranked_candidates.len());
        for (rank, candidate) in ranked_candidates {
            ranks_by_key.insert(
                crate::app_usecase_discovery::release_search_key(&candidate),
                rank,
            );
            candidates.push(candidate);
        }

        let mut scoped_subject = title_subject.clone();
        scoped_subject.submission_scope = SubmissionScope::EpisodeSet { episode_ids };
        for candidate in app
            .evaluate_search_results_for_subject(title, &scoped_subject, candidates, false)
            .await
        {
            let key = crate::app_usecase_discovery::release_search_key(&candidate);
            if let Some(rank) = ranks_by_key.remove(&key) {
                evaluated.push((rank, candidate));
            }
        }
    }

    evaluated.sort_by_key(|(rank, _)| *rank);
    (
        evaluated
            .into_iter()
            .filter(|(_, candidate)| {
                matches!(
                    annotated_auto_decision_code(candidate),
                    ReleaseAutoDecisionCode::Eligible
                        | ReleaseAutoDecisionCode::PendingDelay
                        | ReleaseAutoDecisionCode::AlreadyActive
                )
            })
            .map(|(_, candidate)| candidate)
            .collect(),
        collection_ids,
    )
}

async fn series_pack_candidate_anchors(
    app: &AppUseCase,
    title: &Title,
    candidates: &[IndexerSearchResult],
    episodes: &[Episode],
) -> AppResult<HashMap<String, AcquisitionScopeState>> {
    let states = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states_for_title_ids(std::slice::from_ref(&title.id))
        .await?;
    let mut states_by_episode = states
        .into_iter()
        .filter_map(|state| {
            state
                .episode_id
                .clone()
                .map(|episode_id| (episode_id, state))
        })
        .collect::<HashMap<_, _>>();
    let mut anchors = HashMap::new();

    for candidate in candidates {
        let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
            continue;
        };
        let scope =
            crate::acquisition_coverage::resolve_release_coverage(parsed, episodes, &[], None)
                .submission_scope();
        let Some(anchor_episode_id) =
            episode_ids_for_scope(&scope).and_then(|episode_ids| episode_ids.first())
        else {
            continue;
        };
        let anchor = if let Some(anchor) = states_by_episode.get(anchor_episode_id).cloned() {
            anchor
        } else {
            let Some(anchor_episode) = episodes
                .iter()
                .find(|episode| episode.id == *anchor_episode_id)
            else {
                continue;
            };
            let mut anchor = app.new_wanted_state_view(
                title,
                "episode",
                Some(anchor_episode.id.clone()),
                anchor_episode.collection_id.clone(),
                None,
                anchor_episode.season_number.clone(),
            );
            anchor.id = app
                .services
                .workflow
                .acquisition_scope_states
                .ensure_acquisition_scope_state(&anchor)
                .await?;
            states_by_episode.insert(anchor_episode.id.clone(), anchor.clone());
            anchor
        };
        anchors.insert(
            crate::app_usecase_discovery::release_search_key(candidate),
            anchor,
        );
    }

    Ok(anchors)
}

#[expect(
    clippy::too_many_arguments,
    reason = "series-pack runner-ups retain their exact covered anchor and global rank"
)]
async fn persist_series_pack_runner_ups(
    app: &AppUseCase,
    title: &Title,
    candidates: &[IndexerSearchResult],
    start_index: usize,
    anchors: &HashMap<String, AcquisitionScopeState>,
    now: &DateTime<Utc>,
    failed_routes: &[DownloadRouteKey],
    blocklist: &crate::app_usecase_discovery::TitleReleaseBlocklistSignatures,
) {
    let mut anchor_ids = candidates
        .iter()
        .skip(start_index)
        .filter_map(|candidate| {
            anchors.get(&crate::app_usecase_discovery::release_search_key(candidate))
        })
        .map(|anchor| anchor.id.clone())
        .collect::<Vec<_>>();
    anchor_ids.sort();
    anchor_ids.dedup();

    for anchor_id in anchor_ids {
        let Some(anchor) = anchors.values().find(|anchor| anchor.id == anchor_id) else {
            continue;
        };
        persist_standby_candidates(
            app,
            anchor,
            title,
            candidates,
            start_index,
            now,
            failed_routes,
            blocklist,
            |candidate| {
                anchors
                    .get(&crate::app_usecase_discovery::release_search_key(candidate))
                    .is_some_and(|candidate_anchor| candidate_anchor.id == anchor_id)
            },
        )
        .await;
    }
}

async fn record_series_pack_search_coverage(
    app: &AppUseCase,
    convergence: &crate::acquisition::convergence::ScopeConvergence,
    fired_indexer_ids: &[String],
    collection_ids: &HashSet<String>,
) {
    app.record_convergence_coverage(convergence, fired_indexer_ids)
        .await;
    for collection_id in collection_ids {
        let Some(scope_key) =
            crate::acquisition::convergence::series_pack_collection_scope_key(collection_id)
        else {
            continue;
        };
        let mut collection_convergence = convergence.clone();
        collection_convergence.scope_key = scope_key;
        app.record_convergence_coverage(&collection_convergence, fired_indexer_ids)
            .await;
    }
}

struct BackgroundAcquisitionTitleContext {
    title: scryer_domain::Title,
    episodes_by_id: HashMap<String, scryer_domain::Episode>,
    submissions: Vec<DownloadSubmission>,
    tracked_states:
        HashMap<crate::contracts::ClientJobLocator, scryer_domain::TrackedDownloadState>,
}

impl BackgroundAcquisitionTitleContext {
    async fn load(app: &AppUseCase, title: scryer_domain::Title) -> AppResult<Self> {
        let episodes = app
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await?;
        let submission_guard = app
            .runtime
            .acquisition
            .download_submission_guards
            .acquire_title(&title.id)
            .await;
        let submissions = app
            .services
            .workflow
            .download_submissions
            .list_for_title(&title.id)
            .await?;
        if app
            .services
            .workflow
            .download_submissions
            .list_active_unbound_for_title(&title.id)
            .await?
            .is_empty()
        {
            app.runtime
                .acquisition
                .download_submission_guards
                .prime_title_state(&title.id, submissions.clone(), episodes.clone());
        } else {
            app.runtime
                .acquisition
                .download_submission_guards
                .clear_title_state(&title.id);
        }
        drop(submission_guard);
        let submission_identities = submissions
            .iter()
            .map(crate::contracts::ClientJobLocator::from_submission)
            .collect::<Vec<_>>();
        let tracked_states = app
            .services
            .workflow
            .download_submissions
            .list_identity_tracked_states_for_client_items(&submission_identities)
            .await?
            .into_iter()
            .filter_map(|(identity, state)| {
                scryer_domain::TrackedDownloadState::from_str_opt(&state)
                    .map(|state| (identity, state))
            })
            .collect();

        Ok(Self {
            title,
            episodes_by_id: episodes
                .into_iter()
                .map(|episode| (episode.id.clone(), episode))
                .collect(),
            submissions,
            tracked_states,
        })
    }
}

/// Walk one title through the staged acquisition pipeline: the title-pack
/// stage, then each season's pack stage, then the episode scopes — in that
/// order and in `(season, episode)` order within the scopes.
///
/// `options.intent` decides which economy gates apply (see
/// [`AcquisitionWalkIntent`]); everything else — the stages, the proposals, the
/// end-of-walk arbitration — is identical for both callers. `on_stage` is
/// invoked after each work item so an interactive job can report progress at
/// stage-and-scope granularity.
#[expect(
    clippy::too_many_arguments,
    reason = "one title coordinator owns the cycle-wide acquisition inputs"
)]
async fn process_background_acquisition_title<F>(
    app: &AppUseCase,
    title: scryer_domain::Title,
    mut title_work: BackgroundAcquisitionTitleWork,
    targets: &[crate::acquisition::targets::AcquisitionTarget],
    now: &DateTime<Utc>,
    availability: &crate::acquisition::convergence::SchedulerAvailability,
    indexer_hosts: &HashMap<String, String>,
    cycle: &BackgroundAcquisitionCycleCoordinator,
    season_due_counts: &HashMap<(String, u32), usize>,
    dl_snapshot: &DownloadClientSnapshot,
    options: TitleWalkOptions,
    mut on_stage: F,
) -> AppResult<TitleWalkStats>
where
    F: FnMut(&BackgroundAcquisitionWorkKind, &crate::acquisition::targets::AcquisitionTarget),
{
    let started = std::time::Instant::now();
    let intent = options.intent;
    let context = BackgroundAcquisitionTitleContext::load(app, title).await?;
    let mut session = TitleWalkSession::new(options);

    while let Some(work) = title_work.ready.pop_front() {
        if session.options.cancellation.is_cancelled() {
            debug!(
                title_id = title_work.title_id.as_str(),
                "acquisition title walk cancelled"
            );
            break;
        }
        let target = &targets[work.target_index];
        let pack_stage_only = !matches!(work.kind, BackgroundAcquisitionWorkKind::Scope);
        debug!(
            title_id = title_work.title_id.as_str(),
            scope_key = target.scope_key.as_str(),
            work = ?work.kind,
            "background acquisition target work started"
        );
        if let Err(error) = process_single_target(
            app,
            target,
            now,
            availability,
            indexer_hosts,
            cycle,
            season_due_counts,
            dl_snapshot,
            &context,
            pack_stage_only,
            &mut session,
        )
        .await
        {
            warn!(
                scope_key = target.scope_key.as_str(),
                title_id = target.title_id.as_str(),
                error = %error,
                "failed to process background acquisition target"
            );
            metrics::counter!("scryer_background_acquisition_target_work_total", "outcome" => "failed")
                .increment(1);
        } else {
            metrics::counter!("scryer_background_acquisition_target_work_total", "outcome" => "completed")
                .increment(1);
        }

        match work.kind {
            BackgroundAcquisitionWorkKind::TitlePack => {
                cycle.complete_title_pack_stage(&title_work.title_id);
                if let Some(season) = target
                    .season_number
                    .as_deref()
                    .and_then(|value| value.parse::<u32>().ok())
                {
                    cycle.complete_season_pack_stage(&(title_work.title_id.clone(), season));
                }
            }
            BackgroundAcquisitionWorkKind::SeasonPack { season } => {
                cycle.complete_season_pack_stage(&(title_work.title_id.clone(), season));
            }
            BackgroundAcquisitionWorkKind::Scope => {}
        }

        session.stats.stages += 1;
        on_stage(&work.kind, target);
        if session.stats.stages.is_multiple_of(ACQUISITION_SLICE_YIELD_INTERVAL) {
            tokio::task::yield_now().await;
        }
    }

    // Every stage has had its say; the arbitration point is the end of the
    // sequential walk, in the same future that built the proposals.
    let proposals = std::mem::take(&mut session.proposals);
    session.stats.proposals = proposals.len();
    let mut stats = std::mem::take(&mut session.stats);
    let (committed, set_aside) = arbitrate_and_commit_title_grabs(
        app,
        &context.title,
        proposals,
        cycle,
        dl_snapshot,
        now,
        &mut stats,
    )
    .await;
    session.stats = stats;
    session.stats.committed = committed;
    session.stats.set_aside = set_aside;

    // One line per title walk. The debug! lines above narrate the steps; this
    // is the shape of the whole pass, which is what an operator reading a slow
    // cycle actually needs.
    info!(
        title_id = context.title.id.as_str(),
        title_name = context.title.name.as_str(),
        intent = intent.as_str(),
        season_filter = session.options.season_filter,
        stages = session.stats.stages,
        queries = session.stats.queries,
        inline_grabs = session.stats.inline_grabs,
        proposals = session.stats.proposals,
        committed = session.stats.committed,
        set_aside = session.stats.set_aside,
        failed = session.stats.failed,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "acquisition title walk complete"
    );

    Ok(session.stats)
}

/// One step of an interactive title walk, for the job's progress rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AcquisitionWalkProgress {
    /// Work items in this walk: pack stages plus episode scopes. Known before
    /// the first stage runs, so the job's total is right from the start.
    pub total: usize,
    /// Work items finished.
    pub processed: usize,
    /// What the walk is on — `"Series pack"`, `"Season 2 pack"`, `"S02E05"`.
    pub stage_label: String,
}

/// A human label for one work item, for the job's `current_title` field.
fn acquisition_walk_stage_label(
    title_name: &str,
    kind: &BackgroundAcquisitionWorkKind,
    target: &crate::acquisition::targets::AcquisitionTarget,
) -> String {
    match kind {
        BackgroundAcquisitionWorkKind::TitlePack => format!("{title_name} — series pack"),
        BackgroundAcquisitionWorkKind::SeasonPack { season } => {
            format!("{title_name} — season {season} pack")
        }
        BackgroundAcquisitionWorkKind::Scope => match (
            target.season_number.as_deref(),
            target.episode_number.as_deref(),
        ) {
            (Some(season), Some(episode)) => format!("{title_name} — S{season}E{episode}"),
            (Some(season), None) => format!("{title_name} — season {season}"),
            _ => title_name.to_string(),
        },
    }
}

/// Wakes the convergence poller when an interactive walk gives its title lock
/// back, so exactly one cycle follows the job.
///
/// The cycle skips a locked title rather than waiting, and deliberately does not
/// re-arm its retry timer for the skip — a walk over a large title runs for
/// minutes, and a timer would spend a whole cycle every interval for the length
/// of it. This is the other half of that decision: one wake when the title is
/// free again, on every exit path including cancellation and error.
struct InteractiveWalkLease {
    guard: Option<tokio::sync::OwnedMutexGuard<()>>,
    wake: Arc<tokio::sync::Notify>,
}

impl Drop for InteractiveWalkLease {
    fn drop(&mut self) {
        // Release before waking: the cycle this wakes takes the title lock with
        // `try_acquire` and would skip the title all over again if it ran first.
        drop(self.guard.take());
        self.wake.notify_one();
    }
}

/// Run one title — optionally one season of it — through the same staged walk
/// the convergence cycle runs, under interactive intent.
///
/// This is the whole point of the intent split: an operator's title-scoped
/// search used to expand to one `queue_best_release` per missing episode, which
/// resolves an episode subject only. It grabbed S01E01 in milliseconds without
/// ever asking whether a season or series pack existed — the exact invariant the
/// background walk's stage order protects — and then spent minutes on the rest
/// of the season one episode at a time. Running the staged walk instead keeps
/// one rule for both callers: every pack proposal exists before any episode
/// scope runs, and the arbitration at the end of the walk decides between them.
///
/// `scope_keys` is the set the *request* resolved to. It has to be applied here
/// rather than re-derived, because a request narrows by wanted kind, facet,
/// library and season, and the derivation does not: a "search cutoff-unmet for
/// this title" request that walked every derived target would search the
/// title's missing scopes too. The pack stages are then built from the
/// restricted set, so they speak only for scopes the request asked about.
/// `None` walks everything derived for the title.
///
/// The per-title walk lock is taken by *waiting*, unlike the cycle's
/// `try_acquire`: the operator asked for this title, and a pass over it is
/// bounded — but only in the sense that it finishes. A cycle pass over a
/// several-hundred-episode title with inline episode queries can take minutes,
/// so the wait is reported as its own progress step rather than pretending to be
/// instant. Holding the lock is what lets this walk use its own coordinator —
/// two walks over one title would each hold their own claim set and could both
/// commit a pack.
pub(crate) async fn run_interactive_title_acquisition_walk<F>(
    app: &AppUseCase,
    title_id: &str,
    season_filter: Option<u32>,
    scope_keys: Option<&HashSet<String>>,
    cancellation: tokio_util::sync::CancellationToken,
    mut on_progress: F,
) -> AppResult<TitleWalkStats>
where
    F: FnMut(AcquisitionWalkProgress),
{
    let title = app
        .services
        .catalog
        .titles
        .get_by_id(title_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;

    let now = Utc::now();
    // Cost: this derives every acquisition target in the library and keeps one
    // title's. There is no per-title derivation to call, and adding one means
    // threading a title filter through both halves of `derive_acquisition_targets`.
    let targets = app
        .derive_acquisition_targets(&now)
        .await?
        .into_iter()
        .filter(|target| target.title_id == title_id)
        .filter(|target| {
            scope_keys.is_none_or(|keys| keys.contains(target.scope_key.as_str()))
        })
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Ok(TitleWalkStats::default());
    }
    let indices = (0..targets.len()).collect::<Vec<_>>();
    let mut ready_titles =
        build_background_acquisition_title_work(&targets, &indices, season_filter);
    let Some(title_work) = ready_titles.pop_front() else {
        return Ok(TitleWalkStats::default());
    };

    // Same rule as the cycle's: a season pack is only worth a query when the
    // season has more than one episode in the walk (Sonarr's "count > 1
    // missing" before a SeasonSearchCriteria). Counted over this walk's own
    // scopes rather than a cycle batch.
    let mut season_due_counts: HashMap<(String, u32), usize> = HashMap::new();
    for work in &title_work.ready {
        let target = &targets[work.target_index];
        if matches!(work.kind, BackgroundAcquisitionWorkKind::Scope)
            && target.media_type == "episode"
            && let Some(season) = packable_season_for_target(target)
        {
            *season_due_counts
                .entry((target.title_id.clone(), season))
                .or_insert(0) += 1;
        }
    }

    let total = title_work.ready.len();
    on_progress(AcquisitionWalkProgress {
        total,
        processed: 0,
        stage_label: title.name.clone(),
    });

    // Wait for any cycle pass over this title to finish before starting. That
    // pass is bounded but not necessarily quick, so a job that is going to wait
    // says so instead of showing an idle bar.
    let walk_guard = match app
        .runtime
        .acquisition
        .title_walk_locks
        .try_acquire(title_id)
        .await
    {
        Some(guard) => guard,
        None => {
            on_progress(AcquisitionWalkProgress {
                total,
                processed: 0,
                stage_label: format!(
                    "{} — waiting for the background acquisition walk of this title to finish",
                    title.name
                ),
            });
            app.runtime
                .acquisition
                .title_walk_locks
                .acquire(title_id)
                .await
        }
    };
    let _walk_lease = InteractiveWalkLease {
        guard: Some(walk_guard),
        wake: Arc::clone(&app.runtime.acquisition.acquisition_wake),
    };
    if cancellation.is_cancelled() {
        return Ok(TitleWalkStats::default());
    }

    // Resolved but not read by an interactive walk (see `AcquisitionWalkIntent`);
    // the walk shares one code path with the cycle, so the input still has to
    // exist.
    let availability = app.scheduler_availability().await;
    let indexer_hosts = app.indexer_scheduler_host_keys().await;
    let dl_snapshot = DownloadClientSnapshot::fetch(app).await;
    let cycle = BackgroundAcquisitionCycleCoordinator::default();

    let title_name = title.name.clone();
    let mut processed = 0usize;
    process_background_acquisition_title(
        app,
        title,
        title_work,
        &targets,
        &now,
        &availability,
        &indexer_hosts,
        &cycle,
        &season_due_counts,
        &dl_snapshot,
        TitleWalkOptions {
            intent: AcquisitionWalkIntent::Interactive,
            season_filter,
            cancellation,
        },
        |kind, target| {
            processed += 1;
            on_progress(AcquisitionWalkProgress {
                total,
                processed,
                stage_label: acquisition_walk_stage_label(&title_name, kind, target),
            });
        },
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "target processing coordinates shared acquisition state across a title pass"
)]
async fn process_single_target(
    app: &AppUseCase,
    target: &crate::acquisition::targets::AcquisitionTarget,
    now: &DateTime<Utc>,
    availability: &crate::acquisition::convergence::SchedulerAvailability,
    indexer_hosts: &std::collections::HashMap<String, String>,
    cycle: &BackgroundAcquisitionCycleCoordinator,
    season_due_counts: &std::collections::HashMap<(String, u32), usize>,
    dl_snapshot: &DownloadClientSnapshot,
    context: &BackgroundAcquisitionTitleContext,
    pack_stage_only: bool,
    session: &mut TitleWalkSession,
) -> AppResult<()> {
    let title = &context.title;
    let intent = session.intent();

    // Load episode data for episode-scoped targets
    let episode = if target.media_type == "episode" {
        target
            .episode_id
            .as_deref()
            .and_then(|episode_id| context.episodes_by_id.get(episode_id))
            .cloned()
    } else {
        None
    };
    let effective_collection_id = target
        .collection_id
        .clone()
        .or_else(|| episode.as_ref().and_then(|ep| ep.collection_id.clone()));
    if episode
        .as_ref()
        .is_some_and(|episode| cycle.is_episode_claimed(&episode.id))
    {
        return Ok(());
    }
    // A pack this title's walk already proposed covers these episodes. Nothing
    // here may spend an indexer query on them: until the pack has been
    // arbitrated it is still the presumptive grab, and the old code reached this
    // point only after the pack had already been submitted and claimed them.
    let proposed_season_key = episode.as_ref().and_then(|episode| {
        episode
            .season_number
            .as_deref()
            .and_then(|season| season.parse::<u32>().ok())
            .map(|season| (title.id.clone(), season))
    });
    let covered_by_proposed_pack = episode.as_ref().is_some_and(|episode| {
        session.proposals.iter().any(|held| {
            held.covers_episode(&episode.id)
                || proposed_season_key
                    .as_ref()
                    .is_some_and(|season_key| held.owns_season(season_key))
        })
    });

    // The scope's acquisition-state row, or an unpersisted view when nothing
    // has happened to the scope yet — persisted the moment it is actually
    // searched, so decisions and grabs have their anchor.
    let mut item = match app
        .find_wanted_state_for_scope(
            &target.title_id,
            target.episode_id.as_deref(),
            target.collection_id.as_deref(),
            target.series_movie_link_id.as_deref(),
        )
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => app.new_wanted_state_view(
            title,
            &target.media_type,
            target.episode_id.clone(),
            effective_collection_id.clone(),
            target.series_movie_link_id.clone(),
            target.season_number.clone(),
        ),
        Err(err) => {
            warn!(
                scope_key = target.scope_key.as_str(),
                error = %err,
                "failed to load acquisition state for target"
            );
            return Ok(());
        }
    };
    let item = &mut item;

    // Item-aware gate: skip only when an active/recent submission blocks this
    // wanted item, not every sibling episode on the same title.
    let submissions = &context.submissions;
    let tracked_states = &context.tracked_states;
    let episode_collection_id = episode_collection_id_for_wanted_item(item, episode.as_ref());

    let has_blocking_download_submission = submissions.iter().any(|submission| {
        let identity = crate::contracts::ClientJobLocator::from_submission(submission);
        submission_blocks_search_for_wanted_item(
            submission,
            item,
            episode_collection_id.as_deref(),
            dl_snapshot,
            tracked_states.get(&identity).copied(),
            target.occupied,
        )
    });

    if has_blocking_download_submission {
        info!(
            title = title.name.as_str(),
            media_type = item.media_type.as_str(),
            episode_id = item.episode_id.as_deref(),
            collection_id = episode_collection_id
                .as_deref()
                .or(item.collection_id.as_deref()),
            "skipping search — download for this wanted item is already active or completed"
        );
        return Ok(());
    }

    if covered_by_proposed_pack {
        // A pack stage is the *only* thing that can already cover these
        // episodes here, and its search ran before this one would have. Two
        // rules follow.
        //
        // **Economy.** No query is spent to find out whether the pack should
        // win: a comparison never justifies a dispatch, and today's code spends
        // nothing at all once a pack has taken the episodes. That also rules
        // out the saved-standby walk, which grabs as it goes — the waiting lane
        // and the arbitration must never both hold a claim on one release.
        //
        // **Default.** With no evidence already in hand, the pack wins
        // unopposed, which is exactly what happens today.
        if pack_stage_only {
            return Ok(());
        }
        return propose_covered_episode_evidence(
            app,
            target,
            context,
            item,
            episode.as_ref(),
            cycle,
            &mut session.proposals,
        )
        .await;
    }

    // Saved search results first: a failure never costs an indexer query. A
    // `wanted` scope that still holds ranked results from its last search —
    // the remainder after a grab that later failed — walks them in order,
    // re-judged against the blocklist, the swarm and admission. Only an
    // exhausted list (or a scope that never saved one) reaches the convergence
    // gate below.
    let claimed_episode_ids = cycle.claimed_episode_ids();
    let stale_standby_indexer_ids =
        if item.status == AcquisitionScopeStatus::Wanted && !item.id.is_empty() {
            match try_saved_candidates(
                app,
                item,
                None,
                Some(&claimed_episode_ids),
                dl_snapshot,
                now,
            )
            .await
            {
                StandbyRecoveryOutcome::Recovered { scope }
                | StandbyRecoveryOutcome::Active { scope } => {
                    if let Some(episode_ids) = episode_ids_for_scope(&scope) {
                        cycle.claim_episode_ids(episode_ids.iter().cloned());
                    }
                    if let SubmissionScope::Collection { collection_id } = &scope {
                        if let Ok(episodes) = app
                            .services
                            .catalog
                            .shows
                            .list_episodes_for_collection(collection_id)
                            .await
                        {
                            cycle.claim_episode_ids(episodes.into_iter().map(|episode| episode.id));
                        }
                        if let Some(season) = target
                            .season_number
                            .as_deref()
                            .or(episode
                                .as_ref()
                                .and_then(|episode| episode.season_number.as_deref()))
                            .and_then(|season| season.parse::<u32>().ok())
                        {
                            cycle.mark_season_pack_grabbed(&(title.id.clone(), season));
                        }
                    }
                    session.stats.inline_grabs += 1;
                    info!(
                        title = title.name.as_str(),
                        scope_key = target.scope_key.as_str(),
                        "grabbed the next saved search result; no indexer query spent"
                    );
                    return Ok(());
                }
                StandbyRecoveryOutcome::Deferred { .. } => {
                    info!(
                        title = title.name.as_str(),
                        scope_key = target.scope_key.as_str(),
                        "saved search result kept pending until the download client recovers"
                    );
                    return Ok(());
                }
                StandbyRecoveryOutcome::Parked { .. } => {
                    info!(
                        title = title.name.as_str(),
                        scope_key = target.scope_key.as_str(),
                        "best saved search result is held by its delay profile"
                    );
                    return Ok(());
                }
                StandbyRecoveryOutcome::Exhausted { stale_indexer_ids } => stale_indexer_ids,
            }
        } else {
            Vec::new()
        };

    let search_title = app
        .release_search_title_for_wanted_item(title, item, episode.as_ref())
        .await;

    let subject = app
        .resolve_release_search_subject_for_wanted_item(
            title,
            &search_title,
            item,
            episode.as_ref(),
        )
        .await;
    let search_season = subject.season;

    // Exhausting saved results is a recovery action, not a new search. Preserve
    // that contract before either the title or episode lane spends an indexer
    // query.
    if !stale_standby_indexer_ids.is_empty() {
        if let Some(convergence) = app.resolve_scope_convergence(&search_title, &subject).await {
            info!(
                title_id = title.id.as_str(),
                scope_key = convergence.scope_key.as_str(),
                stale_indexer_ids = ?stale_standby_indexer_ids,
                "background acquisition: pruned stale standby coverage; the next cycle will refresh these indexers"
            );
            for indexer_id in stale_standby_indexer_ids {
                app.prune_scope_key_coverage(&convergence.scope_key, Some(&indexer_id))
                    .await;
            }
        }
        return Ok(());
    }

    // One title lookup per cycle discovers a qualifying whole-series or
    // multi-season release before the established season and episode paths.
    // A season-scoped walk skips it: a whole-series release answers for seasons
    // the operator never asked about.
    if target.media_type == "episode"
        && title.facet != MediaFacet::Movie
        && session.options.season_filter.is_none()
        && let Some(target_episode) = episode.as_ref()
        && cycle.begin_title_pack(&title.id)
    {
        let claimed_episode_ids = cycle.claimed_episode_ids();
        match try_series_pack_for_title(
            app,
            title,
            &search_title,
            target,
            now,
            availability,
            indexer_hosts,
            dl_snapshot,
            submissions,
            tracked_states,
            &claimed_episode_ids,
            cycle,
            session,
        )
        .await
        {
            Ok(Some(episode_ids)) => {
                session.stats.inline_grabs += 1;
                let claims_target = episode_ids.contains(&target_episode.id);
                cycle.claim_episode_ids(episode_ids);
                if claims_target {
                    return Ok(());
                }
            }
            Ok(None) => {}
            Err(error) => {
                warn!(
                    title_id = title.id.as_str(),
                    error = %error,
                    "series-pack title search failed"
                );
            }
        }
    }

    // Pack stages own distinct fingerprints; only the later scope stage may be
    // short-circuited by episode/title coverage.
    let (uncovered, convergence_scope_key) = if pack_stage_only {
        (HashSet::new(), None)
    } else {
        let Some(convergence) = app.resolve_scope_convergence(&search_title, &subject).await else {
            debug!(
                title_id = title.id.as_str(),
                scope_key = target.scope_key.as_str(),
                "background acquisition: scope has no routed indexers, skipping"
            );
            return Ok(());
        };
        let uncovered = match app
            .uncovered_indexers_for_scope(
                &convergence.scope_key,
                &convergence.facet,
                &convergence.fingerprint,
                &convergence.routed_indexer_ids,
            )
            .await
        {
            Ok(uncovered) => uncovered,
            Err(err) => {
                warn!(
                    scope_key = convergence.scope_key.as_str(),
                    error = %err,
                    "failed to read scope coverage; searching all routed indexers"
                );
                convergence.routed_indexer_ids.clone()
            }
        };
        // Both gates below are economy gates for the machine sweep, and an
        // interactive walk reads neither: the operator asked for this scope
        // now, so a converged scope is re-searched against every routed indexer
        // and admission is the scheduler's call, not a pre-skip here. Coverage
        // is still *recorded* below — the queries really did run.
        if intent.is_interactive() {
            let routed = convergence
                .routed_indexer_ids
                .iter()
                .cloned()
                .collect::<HashSet<_>>();
            if routed.is_empty() {
                return Ok(());
            }
            (routed, Some(convergence.scope_key))
        } else {
            if uncovered.is_empty() {
                debug!(
                    title_id = title.id.as_str(),
                    title_name = title.name.as_str(),
                    media_type = target.media_type.as_str(),
                    "background acquisition: scope converged across routed indexers, riding RSS"
                );
                return Ok(());
            }
            // Scheduler pre-skip: every uncovered indexer is cooling down or quota
            // exhausted — spend nothing; the scope stays a target and the cursor
            // returns to it once the scheduler frees capacity. The cycle counts it
            // so the poller can re-arm sooner than the next full tick.
            if !uncovered.iter().any(|indexer_id| {
                availability.indexer_available(
                    indexer_hosts.get(indexer_id).map(String::as_str),
                    indexer_id,
                )
            }) {
                cycle.record_deferred_scope();
                debug!(
                    title_id = title.id.as_str(),
                    scope_key = target.scope_key.as_str(),
                    uncovered_count = uncovered.len(),
                    "background acquisition: uncovered indexers unavailable this cycle, deferring scope"
                );
                return Ok(());
            }
            (uncovered.into_iter().collect(), Some(convergence.scope_key))
        }
    };

    // The scope is about to be searched — its state row exists from here on,
    // so release decisions and grabs have their anchor.
    item.id = app
        .services
        .workflow
        .acquisition_scope_states
        .ensure_acquisition_scope_state(item)
        .await?;
    let mut failed_routes = cycle.failed_routes();

    // Derive the download client category separately — search_category ("series")
    // is for Newznab query type, download_category ("series") is for NZBGet routing.
    //
    // ── Season pack priority ──────────────────────────────────────────────────
    // For episode wanted items, try a season pack search first. Season packs are
    // a first-class release type on Usenet and are more efficient than individual
    // episodes. Individual episode searches only run if no season pack was found
    // this cycle for this (title, season).
    if target.media_type == "episode"
        && let Some(season_num) = search_season
    {
        let season_key = (title.id.clone(), season_num);

        // Only attempt a season pack search when >= 2 episodes from this season
        // are due this cycle (mirrors Sonarr: count > 1 missing → SeasonSearchCriteria).
        let due_count = season_due_counts.get(&season_key).copied().unwrap_or(0);

        if due_count >= 2 && cycle.begin_season_pack(&season_key) {
            let recent_failed_seasons =
                load_recent_failed_season_pack_seasons_for_title(app, &title.id, now).await;

            if recent_failed_seasons.contains(&season_num) {
                info!(
                    title = title.name.as_str(),
                    season = season_num,
                    cooldown_minutes = FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES,
                    "skipping season-pack search after recent failed season-pack attempt"
                );
            } else {
                // Load season episodes for runtime scoring and upgrade checking.
                let season_episodes = if let Some(ref coll_id) = effective_collection_id {
                    app.services
                        .catalog
                        .shows
                        .list_episodes_for_collection(coll_id)
                        .await
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };

                // Calculate total season runtime for accurate size scoring.
                // A 10-episode × 24-min season should expect ~10× a single episode's size.
                let pack_runtime = if !season_episodes.is_empty() {
                    let ep_count = season_episodes.len().max(1) as i32;
                    let per_ep = title.runtime_minutes.unwrap_or(24);
                    Some(per_ep * ep_count)
                } else {
                    title.runtime_minutes
                };

                let pack_subject = app
                    .resolve_release_search_subject_for_season_pack(
                        &search_title,
                        item,
                        episode.as_ref(),
                        season_num,
                        pack_runtime,
                    )
                    .await?;

                // The pack is its own convergence unit: a
                // converged pack scope rides RSS, an unconverged one is searched
                // against its uncovered subset.
                let pack_uncovered = match app
                    .resolve_scope_convergence(&search_title, &pack_subject)
                    .await
                {
                    Some(pack_convergence) => app
                        .uncovered_indexers_for_scope(
                            &pack_convergence.scope_key,
                            &pack_convergence.facet,
                            &pack_convergence.fingerprint,
                            &pack_convergence.routed_indexer_ids,
                        )
                        .await
                        .ok(),
                    None => None,
                };
                // An interactive walk does not read the pack scope's coverage:
                // the operator asked for this season now, so the pack question
                // is re-asked across every routed indexer.
                let pack_converged = !intent.is_interactive()
                    && pack_uncovered
                        .as_ref()
                        .is_some_and(|uncovered| uncovered.is_empty());
                let pack_restriction = (!intent.is_interactive())
                    .then_some(pack_uncovered)
                    .flatten();
                let pack_results = if pack_converged {
                    debug!(
                        title_id = title.id.as_str(),
                        season = season_num,
                        "season pack scope converged, riding RSS"
                    );
                    Vec::new()
                } else {
                    session.stats.queries += 1;
                    match app
                        .search_and_evaluate_subject_restricted(
                            &search_title,
                            &pack_subject,
                            "background_acquisition_season_pack",
                            SearchMode::Auto,
                            session.options.search_cancellation(),
                            pack_restriction.map(|uncovered| uncovered.into_iter().collect()),
                            // The pack shares the target's recency lane (§D3);
                            // an interactive walk takes the operator lane.
                            intent.background_value(target),
                        )
                        .await
                    {
                        Ok(results) => results,
                        Err(err) => {
                            warn!(
                                title_id = title.id.as_str(),
                                season = season_num,
                                error = %err,
                                "season pack search failed"
                            );
                            Vec::new()
                        }
                    }
                };

                // The season query can surface episode-shaped releases the
                // narrower episode query never returns. Keep them for the
                // episode scopes; only the *substitution* was the defect.
                cycle.cache_season_candidates(
                    &season_key,
                    pack_results
                        .iter()
                        .filter(|candidate| {
                            !candidate_is_season_pack_for_season(candidate, season_num)
                        })
                        .cloned(),
                );

                for candidate in pack_results
                    .iter()
                    .filter(|candidate| candidate_is_season_pack_for_season(candidate, season_num))
                {
                    let decision_code = annotated_auto_decision_code(candidate);
                    // Recorded before any gate ran for this pack, so there is no
                    // bar to name.
                    record_release_decision(app, item, title, candidate, decision_code, None, now)
                        .await;
                    // `AlreadyActive` only. That pack is already downloading,
                    // so searching its episodes would duplicate it. `PendingDelay`
                    // means the delay profile chose to *wait* — not that the pack
                    // won — and suppressing the episode lane for the whole delay
                    // window is a decision the profile never made. The delayed
                    // pack still parks in `pending_releases` and is re-judged
                    // against whatever lands meanwhile when the window expires.
                    if matches!(decision_code, ReleaseAutoDecisionCode::AlreadyActive) {
                        cycle.mark_season_pack_viable(&season_key);
                    }
                }

                // The submission walk is held back so the pack is ranked
                // against whatever else this title's walk turns up. Everything
                // above stays: the query, the decision rows, and the
                // `AlreadyActive` suppression are facts about the world, not
                // preferences waiting on an arbitration.
                let eligible = pack_results
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| {
                        candidate_is_season_pack_for_season(candidate, season_num)
                            && candidate.auto_eligible == Some(true)
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                if !eligible.is_empty() {
                    let mut episode_ids = recovered_scope_episode_ids(
                        app,
                        &collection_download_submission_scope_for_wanted_item(
                            item,
                            episode.as_ref(),
                        ),
                    )
                    .await;
                    // A season pack speaks for its whole season, collection row
                    // or not. The submission scope alone under-states that when
                    // the catalog gave the season no collection to resolve
                    // through — and an under-stated set is the one thing the
                    // proposal invariant forbids: the arbitration's conflict
                    // test could then commit a covered episode grab alongside
                    // the pack.
                    episode_ids.extend(
                        context
                            .episodes_by_id
                            .values()
                            .filter(|episode| {
                                episode
                                    .season_number
                                    .as_deref()
                                    .and_then(|season| season.parse::<u32>().ok())
                                    == Some(season_num)
                            })
                            .map(|episode| episode.id.clone()),
                    );
                    episode_ids.sort();
                    episode_ids.dedup();
                    session.proposals.push(GrabProposal {
                        stage: BackgroundAcquisitionWorkKind::SeasonPack { season: season_num },
                        episode_ids: episode_ids.clone(),
                        season_key: Some(season_key.clone()),
                        ranked_candidates: pack_results,
                        eligible,
                        commit: GrabProposalCommit::SeasonPack(Box::new(SeasonPackCommit {
                            season: season_num,
                            season_key: season_key.clone(),
                            item: item.clone(),
                            episode: episode.clone(),
                            season_episode_ids: episode_ids,
                        })),
                    });
                }
            }
        }

        // If a season pack was grabbed or remains viable this cycle (by this
        // item or an earlier item for the same season), skip the individual
        // episode search unless the pack submission definitively failed.
        if cycle.season_pack_grabbed(&season_key) {
            return Ok(());
        }
        if cycle.season_pack_viable(&season_key) {
            info!(
                title = title.name.as_str(),
                season = season_num,
                "season pack candidate found; skipping individual episode search for this cycle"
            );
            return Ok(());
        }
    }
    // ── End season pack priority ──────────────────────────────────────────────
    if pack_stage_only {
        return Ok(());
    }
    // Uses the per-facet default download category; the selected client's
    // explicit routing category overrides this inside the router.
    let download_cat = app.derive_download_category(&title.facet).await;

    if subject.queries.is_empty() {
        info!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            media_type = item.media_type.as_str(),
            "background acquisition: no search queries built, skipping"
        );
        return Ok(());
    }

    debug!(
        title_id = title.id.as_str(),
        title_name = title.name.as_str(),
        queries = ?subject.queries,
        imdb_id = subject.imdb_id.as_deref().unwrap_or(""),
        tvdb_id = subject.tvdb_id.as_deref().unwrap_or(""),
        category = subject.category.as_str(),
        "background acquisition: searching indexers"
    );

    // Anything this scope's season query already surfaced, restricted to the
    // indexers this scope still needs. It is *added* to the episode query's own
    // results, not used in place of them: substituting converged the scope on a
    // query it never ran.
    let season_extras = search_season
        .map(|season| cycle.season_candidates(&(title.id.clone(), season)))
        .unwrap_or_default()
        .into_iter()
        .filter(|candidate| {
            candidate
                .indexer_id
                .as_ref()
                .is_some_and(|indexer_id| uncovered.contains(indexer_id))
        })
        .collect::<Vec<_>>();

    session.stats.queries += 1;
    let search_outcome = match app
        .search_and_score_subject_restricted(
            &search_title,
            &subject,
            "background_acquisition",
            SearchMode::Auto,
            session.options.search_cancellation(),
            Some(uncovered),
            intent.background_value(target),
        )
        .await
    {
        Ok(r) => r,
        Err(err) => {
            warn!(
                title_id = title.id.as_str(),
                error = %err,
                "background search failed"
            );
            return Ok(());
        }
    };
    let mut scored = search_outcome.results;
    for candidate in season_extras {
        if !scored
            .iter()
            .any(|existing| same_indexer_release(existing, &candidate))
        {
            scored.push(candidate);
        }
    }
    // One evaluation over the union, so the merged rows are ranked by the same
    // comparator as the rest rather than appended past it.
    let results = app
        .evaluate_search_results_for_subject(&search_title, &subject, scored, false)
        .await;
    // A finalize failure withholds coverage (the scope re-searches next cycle)
    // but never the grab walk below: these are live results in hand, and
    // retention bookkeeping does not outrank acquiring with them.
    if app
        .finalize_evaluated_search_session_or_warn(
            &search_outcome.search_session_id,
            &results,
            &title.id,
        )
        .await
    {
        app.record_search_coverage(
            &search_title,
            &subject,
            &search_outcome.complete_indexer_ids,
        )
        .await;
    }

    // Cooldown state, not cadence: the upgrade policy and failed-grab handling
    // read when this scope last actually searched.
    let _ = app
        .services
        .workflow
        .acquisition_scope_states
        .record_acquisition_scope_search_attempt(&item.id, &now.to_rfc3339())
        .await;

    if results.is_empty() {
        debug!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            "background acquisition: search returned 0 results"
        );
        return Ok(());
    }

    debug!(
        title_id = title.id.as_str(),
        title_name = title.name.as_str(),
        result_count = results.len(),
        "background acquisition: evaluating candidates"
    );

    // Load the per-title blocklist (covers post-import failures like fake/non-video
    // files, in addition to the download-client snapshot checked below). It is the
    // single, removable exclusion source; the failed-attempt log never gates.
    let db_blocklist = app.load_title_release_blocklist_signatures(&title.id).await;
    let existing_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|file| file.role.is_primary())
        .collect::<Vec<_>>();
    let cutoff_scope = app.cutoff_scope_for(&subject.submission_scope).await;
    let analyzed_cutoff_quality =
        crate::acquisition::decision_helpers::analyzed_cutoff_quality_for_scope(
            &existing_files,
            &cutoff_scope,
        );

    let upgrade_context = match app
        .resolve_upgrade_context_for_title_with_category_and_quality(
            &search_title,
            Some(subject.category.as_str()),
            analyzed_cutoff_quality,
        )
        .await
    {
        Ok(context) => context,
        Err(error) => {
            warn!(
                title_id = title.id.as_str(),
                error = %error,
                "background acquisition: failed to resolve quality profile; skipping target"
            );
            return Ok(());
        }
    };
    let profile = &upgrade_context.profile;

    // The bar the gate compares against, resolved once for the whole loop so the
    // decision log records the file that actually decided each candidate rather
    // than a number remembered on the scope row — and so the cutoff check below
    // can ask about the *score* half of the cutoff, not just the quality half.
    let admission = {
        let scoring_context = app.resolve_canonical_scoring_context(title, profile).await;
        app.admission_subject_for_scope(
            title,
            &item.submission_scope(),
            &scoring_context,
            title.runtime_minutes,
            crate::quality::canonical_context::SubjectIntent::Grab,
        )
        .await
    };
    let incumbent_bar = admission.best_score();

    // The one place a scope-level cutoff short-circuit survives (D15).
    //
    // Its siblings in the RSS and pending lanes are gone: the cutoff is now a
    // candidate-aware gate (`cutoff_refusal`) so a PROPER can still reach a
    // scope that has otherwise finished. Active search deliberately does not
    // get that escape — Sonarr's `ProperSpecification` accepts only on the feed
    // lane, so an at-cutoff scope reached by active search stops here.
    //
    // **Both halves of the cutoff**, as Sonarr reads it: the quality has
    // arrived *and* the bar has reached `cutoff_score`. Gating on the quality
    // alone abandoned every target `derive_format_cutoff_targets` produces —
    // quality at cutoff, score below it — which is exactly the population D19
    // exists to re-search.
    //
    // It is *not* a pre-search return, despite where it sits: the indexer query
    // already ran above. What it saves is the candidate loop — one decision row
    // per result plus the grab attempts.
    if crate::acquisition_release_search::incumbent_at_cutoff(
        upgrade_context.cutoff_reached,
        &admission,
        profile.criteria.cutoff_score,
    ) {
        tracing::debug!(
            title_id = title.id.as_str(),
            cutoff = profile.criteria.cutoff_tier.as_deref().unwrap_or(""),
            cutoff_score = profile.criteria.cutoff_score,
            "cutoff reached, skipping upgrade"
        );
        return Ok(());
    }
    let delay_profiles = app.load_delay_profiles().await;

    // ── Candidate fallthrough loop ──────────────────────────────────────────
    // Iterate ranked candidates (sorted by preference_score DESC).  If a grab
    // fails, try the next candidate instead of re-searching from scratch next
    // cycle.  Mirrors Sonarr's ProcessDownloadDecisions loop.
    let mut had_allowed_candidate = false;
    let mut had_quality_allowed_candidate = false;
    let mut skipped_for_failed = false;
    let mut skipped_for_title_mismatch = false;
    // Park the best ambiguous candidate before a higher-ranked eligible release
    // can return from the loop. Otherwise the pending-review side effect depends
    // on incidental candidate ordering.
    let mut parked_ambiguous_identity = false;
    if let Some(candidate) = results.iter().find(|candidate| {
        candidate
            .quality_profile_decision
            .as_ref()
            .is_some_and(|decision| decision.allowed)
            && matches!(
                effective_auto_decision_code_for_route(candidate, &failed_routes, &db_blocklist),
                ReleaseAutoDecisionCode::AmbiguousIdentity
            )
    }) {
        parked_ambiguous_identity = true;
        let candidate_score = candidate
            .quality_profile_decision
            .as_ref()
            .map(|decision| decision.preference_score)
            .unwrap_or_default();
        app.park_pending_release_for_review(
            item,
            title,
            candidate,
            candidate_score,
            serialize_decision_explanation(candidate),
        )
        .await;
    }
    let mut eligible: Vec<usize> = Vec::new();
    let mut next_pending_role = PendingReleaseRole::Primary;

    for (candidate_index, candidate) in results.iter().enumerate() {
        let is_allowed = candidate
            .quality_profile_decision
            .as_ref()
            .map(|d| d.allowed)
            .unwrap_or(false);
        let decision_code = if is_allowed {
            effective_auto_decision_code_for_route(candidate, &failed_routes, &db_blocklist)
        } else {
            ReleaseAutoDecisionCode::QualityBlocked
        };
        if !is_allowed {
            // Blocked on quality alone: admission never looked at an incumbent.
            record_release_decision(app, item, title, candidate, decision_code, None, now).await;
            app.emit_acquisition_candidate_rejected_event(
                None,
                title,
                candidate.title.clone(),
                decision_code.as_str().to_string(),
            )
            .await;
            continue;
        }

        had_quality_allowed_candidate = true;

        let candidate_score = candidate
            .quality_profile_decision
            .as_ref()
            .map(|d| d.preference_score)
            .unwrap_or(0);

        if !matches!(
            decision_code,
            ReleaseAutoDecisionCode::TitleMismatch
                | ReleaseAutoDecisionCode::EpisodeMismatch
                | ReleaseAutoDecisionCode::CategoryMismatch
                | ReleaseAutoDecisionCode::AmbiguousIdentity
        ) {
            had_allowed_candidate = true;
        }
        if matches!(
            decision_code,
            ReleaseAutoDecisionCode::TitleMismatch
                | ReleaseAutoDecisionCode::EpisodeMismatch
                | ReleaseAutoDecisionCode::CategoryMismatch
                | ReleaseAutoDecisionCode::AmbiguousIdentity
        ) {
            skipped_for_title_mismatch = true;
        }
        if matches!(decision_code, ReleaseAutoDecisionCode::DbBlocklisted) {
            skipped_for_failed = true;
        }

        record_release_decision(
            app,
            item,
            title,
            candidate,
            decision_code,
            incumbent_bar,
            now,
        )
        .await;

        if !decision_code.is_eligible() {
            app.emit_acquisition_candidate_rejected_event(
                None,
                title,
                candidate.title.clone(),
                decision_code.as_str().to_string(),
            )
            .await;
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
            if matches!(decision_code, ReleaseAutoDecisionCode::AmbiguousIdentity)
                && !parked_ambiguous_identity
            {
                parked_ambiguous_identity = true;
                app.park_pending_release_for_review(
                    item,
                    title,
                    candidate,
                    candidate_score,
                    serialize_decision_explanation(candidate),
                )
                .await;
                // Keep walking the ranked list: a lower-scored candidate that
                // does present a disambiguator is still grabbable this cycle.
                continue;
            }
            if matches!(
                decision_code,
                ReleaseAutoDecisionCode::PendingDelay
                    | ReleaseAutoDecisionCode::MinimumAge
                    | ReleaseAutoDecisionCode::ReleaseAgeUnknown
            ) {
                let scoring_json = candidate.quality_profile_decision.as_ref().map(|decision| {
                    serde_json::to_string(
                        &decision
                            .scoring_log
                            .iter()
                            .map(|entry| serde_json::json!({"code": entry.code, "delta": entry.delta}))
                            .collect::<Vec<_>>(),
                    )
                    .unwrap_or_default()
                });

                let canonical_source = candidate.canonical_download_source();
                let parsed_published_at = candidate
                    .published_at
                    .as_deref()
                    .and_then(crate::quality_profile::parse_published_at);
                let normalized_published_at =
                    parsed_published_at.map(|published_at| published_at.to_rfc3339());
                let delay = automatic_candidate_delay_decision(
                    candidate,
                    &search_title,
                    &admission,
                    profile,
                    &delay_profiles,
                    false,
                    None,
                    now,
                );
                let eligible_at =
                    if matches!(decision_code, ReleaseAutoDecisionCode::ReleaseAgeUnknown) {
                        crate::delay_profile::resolve_delay_profile(
                            &delay_profiles,
                            &search_title.tags,
                            &search_title.facet,
                        )
                        .map(|profile| {
                            profile.release_age_unknown_escalation_deadline(
                                candidate.source_kind,
                                *now,
                            )
                        })
                        .unwrap_or(*now)
                    } else {
                        delay
                            .and_then(|decision| decision.eligible_at)
                            .unwrap_or(*now)
                    };
                let pending = PendingRelease {
                    id: Id::new().0,
                    wanted_item_id: item.id.clone(),
                    title_id: title.id.clone(),
                    release_title: candidate.title.clone(),
                    release_url: canonical_source.as_ref().map(|(source, _)| source.clone()),
                    source_kind: canonical_source
                        .as_ref()
                        .map(|(_, kind)| *kind)
                        .or(candidate.source_kind),
                    release_size_bytes: candidate.size_bytes,
                    release_score: candidate_score,
                    scoring_log_json: scoring_json,
                    indexer_source: Some(candidate.source.clone()),
                    indexer_id: candidate.indexer_id.clone(),
                    release_guid: candidate.guid.clone(),
                    added_at: now.to_rfc3339(),
                    last_observed_at: now.to_rfc3339(),
                    delay_until: eligible_at.to_rfc3339(),
                    status: PendingReleaseStatus::Waiting,
                    grabbed_at: None,
                    source_password: crate::normalize_release_password(
                        candidate.password_hint.as_deref(),
                    ),
                    published_at: normalized_published_at,
                    info_hash: candidate
                        .extra
                        .get("info_hash")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    seed_minimums: crate::ReleaseSeedMinimums::from_release_extra(&candidate.extra),
                    seeders: crate::acquisition::seed_goals::seeders_from_extra(&candidate.extra),
                    release_identity: String::new(),
                    coverage_identity: String::new(),
                    role: next_pending_role,
                    last_decision_code: Some(decision_code.as_str().to_string()),
                    release_age_unknown: matches!(
                        decision_code,
                        ReleaseAutoDecisionCode::ReleaseAgeUnknown
                    ),
                };
                let observation = PendingReleaseObservation::derived(&pending, next_pending_role);
                match app
                    .insert_pending_release_observation(&pending, &observation)
                    .await
                {
                    Ok(_) => next_pending_role = PendingReleaseRole::Fallback,
                    Err(error) => {
                        warn!(
                            error = %error,
                            title = title.name.as_str(),
                            release = candidate.title.as_str(),
                            decision = decision_code.as_str(),
                            "pending release: failed to persist automatic search hold"
                        );
                    }
                }
            }
            continue;
        }

        // The submission itself is deferred: this walk decides *whether* the
        // candidate may be grabbed, and arbitration decides which of the
        // title's stages actually spends the grab.
        eligible.push(candidate_index);
    }
    // ── End candidate fallthrough loop ───────────────────────────────────────

    // No pack proposal covers this scope — every pack stage for the title was
    // planned before any scope stage ran, so nothing can start covering it
    // now — which leaves this grab with nobody to be arbitrated against. It
    // commits in place, exactly where it always did.
    let mut grab_context = ScopeGrabContext {
        item: item.clone(),
        episode: episode.clone(),
        media_type: target.media_type.clone(),
        download_category: download_cat.clone(),
        convergence_scope_key: convergence_scope_key.clone(),
    };
    let mut grab_failures = GrabFailureTally::default();
    let scope_grab = commit_scope_grab(
        app,
        title,
        &mut grab_context,
        &results,
        &eligible,
        &mut failed_routes,
        &db_blocklist,
        cycle,
        now,
        &mut grab_failures,
    )
    .await;
    // Counted before the `?`: a scope whose submissions failed is a failed work
    // item whether or not the commit itself then errored out.
    session.stats.record_grab_failures(grab_failures);
    if let ScopeGrabOutcome::Settled {
        claimed_episode_ids,
    } = scope_grab?
    {
        // Claimed even though no sibling scope can want these episodes: the
        // pack proposals held back for the end of the walk are committed *after*
        // this, and their overlap guard reads the claim set.
        session.stats.inline_grabs += 1;
        cycle.claim_episode_ids(claimed_episode_ids);
        return Ok(());
    }

    // All candidates exhausted without a successful grab.
    if !eligible.is_empty() {
        warn!(
            title = title.name.as_str(),
            attempts = eligible.len(),
            "all grab attempts failed, re-queuing for next cycle"
        );
    } else if had_allowed_candidate && skipped_for_failed {
        warn!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            "background acquisition: no suitable candidates found after skipping blocklisted or active releases"
        );
    } else if had_allowed_candidate {
        debug!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            "background acquisition: all allowed candidates were already active or had negative scores"
        );
    } else if had_quality_allowed_candidate && skipped_for_title_mismatch {
        debug!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            result_count = results.len(),
            "background acquisition: quality-allowed candidates were rejected by title matching"
        );
    } else {
        debug!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            result_count = results.len(),
            "background acquisition: no allowed candidates found (all blocked by quality profile)"
        );
    }

    // No grab this cycle. Coverage was recorded when the search returned, so
    // the cursor will not re-search these indexers — retain the ranked
    // remainder as standby and the next cycle replays it without an indexer
    // query. Without this the scope is converged with no corpus behind it, and
    // acquisition stops for it until the long-tail backstop, an RSS hit, or an
    // operator trigger.
    //
    // A partial write is the same failure one step later: `persist_standby_candidates`
    // clears the existing list before inserting, so a `false` return can leave
    // fewer rows than it removed. Drop the coverage in that case and let the
    // cursor come back.
    let preserved_standby = match app
        .services
        .workflow
        .pending_releases
        .list_standby_pending_releases_for_wanted_item(&item.id)
        .await
    {
        Ok(preserved) => preserved,
        Err(error) => {
            // `persist_standby_candidates` clears before it writes, so a list we
            // could not snapshot is a list we must not touch.
            warn!(
                wanted_item_id = item.id.as_str(),
                error = %error,
                "failed to snapshot standby candidates; leaving the saved list untouched"
            );
            return Ok(());
        }
    };
    // The walk above parks delayed and age-held candidates as `Waiting`. A
    // second `Standby` row for one of those would give two lanes a claim on the
    // same release: the standby walk re-parks its copy as `Waiting` too, and the
    // delay promoter then has two rows to grab. Ambiguous-identity parks are
    // `NeedsReview` and are never grabbed automatically, so they need no
    // exclusion — which is what makes this waiting-only listing the right one.
    let waiting_urls = match app
        .services
        .workflow
        .pending_releases
        .list_pending_releases_for_wanted_item(&item.id)
        .await
    {
        Ok(waiting) => waiting
            .into_iter()
            .filter_map(|release| release.release_url)
            .collect::<HashSet<String>>(),
        Err(error) => {
            warn!(
                wanted_item_id = item.id.as_str(),
                error = %error,
                "failed to read waiting pending releases; leaving the saved standby list untouched"
            );
            return Ok(());
        }
    };
    // Re-read the blocklist: a definitive grab failure in the walk above wrote
    // entries the loop's snapshot predates, and retaining a release this cycle
    // just burned would hand the next walk a row it must immediately skip.
    let retention_blocklist = app.load_title_release_blocklist_signatures(&title.id).await;
    let retention_complete = persist_standby_candidates(
        app,
        item,
        title,
        &results,
        0,
        now,
        &failed_routes,
        &retention_blocklist,
        |candidate| {
            candidate
                .canonical_download_source()
                .is_some_and(|(source, _)| !waiting_urls.contains(&source))
        },
    )
    .await;
    // `persist_standby_candidates` clears before it inserts, so both an empty
    // list and an incomplete write mean the scope may now hold less than it did.
    // Neither may leave it converged with nothing to walk.
    let retained_standby = app
        .services
        .workflow
        .pending_releases
        .list_standby_pending_releases_for_wanted_item(&item.id)
        .await
        .unwrap_or_default();
    if !retention_complete || retained_standby.is_empty() {
        if !preserved_standby.is_empty() {
            // An upgrade search that rejects everything must not cost the scope
            // the corpus its last search earned.
            restore_standby_releases(app, item, &preserved_standby).await;
        } else if let Some(scope_key) = convergence_scope_key.as_deref()
            && !retention_complete
        {
            warn!(
                title_id = title.id.as_str(),
                scope_key,
                "standby retention failed with nothing to fall back on; re-opening scope coverage"
            );
            app.prune_scope_key_coverage(scope_key, None).await;
        }
        // A complete write with nothing worth keeping is a scope with no
        // acceptable release. It stays converged, or it re-searches forever.
    }

    Ok(())
}

/// Build an episode-scope proposal out of evidence already in hand, for a scope
/// a proposed pack covers.
///
/// The free evidence is what a season query for this title already surfaced
/// this cycle: episode-shaped rows the narrower episode query never returns,
/// cached on the coordinator. Re-ranking them costs nothing, and it is the only
/// way an episode can outbid the pack without a query being spent to find out.
///
/// The scope neither records nor consumes convergence coverage here — it ran no
/// query — so it remains a target for a later cycle if the pack wins, and its
/// saved standby list is left exactly as the pack-covered scope found it.
async fn propose_covered_episode_evidence(
    app: &AppUseCase,
    target: &crate::acquisition::targets::AcquisitionTarget,
    context: &BackgroundAcquisitionTitleContext,
    item: &AcquisitionScopeState,
    episode: Option<&Episode>,
    cycle: &BackgroundAcquisitionCycleCoordinator,
    proposals: &mut Vec<GrabProposal>,
) -> AppResult<()> {
    let title = &context.title;
    let Some(season) = episode
        .and_then(|episode| episode.season_number.as_deref())
        .and_then(|season| season.parse::<u32>().ok())
    else {
        return Ok(());
    };
    let extras = cycle.season_candidates(&(title.id.clone(), season));
    if extras.is_empty() {
        return Ok(());
    }

    let search_title = app
        .release_search_title_for_wanted_item(title, item, episode)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(title, &search_title, item, episode)
        .await;
    // One evaluation, the same one the live path runs, so a pack and an episode
    // are compared on scores produced by the same comparator.
    let results = app
        .evaluate_search_results_for_subject(&search_title, &subject, extras, false)
        .await;

    let failed_routes = cycle.failed_routes();
    let db_blocklist = app.load_title_release_blocklist_signatures(&title.id).await;
    let eligible = results
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate
                .quality_profile_decision
                .as_ref()
                .is_some_and(|decision| decision.allowed)
                && effective_auto_decision_code_for_route(candidate, &failed_routes, &db_blocklist)
                    .is_eligible()
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return Ok(());
    }

    let catalog_episodes = context
        .episodes_by_id
        .values()
        .cloned()
        .collect::<Vec<Episode>>();
    let catalog_collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .unwrap_or_default();
    let mut episode_ids = Vec::new();
    for index in &eligible {
        let candidate = &results[*index];
        let scope = match candidate.parsed_release_metadata.as_ref() {
            Some(parsed) => crate::acquisition_coverage::resolve_release_coverage(
                parsed,
                &catalog_episodes,
                &catalog_collections,
                episode,
            )
            .submission_scope_or(&direct_download_submission_scope_for_wanted_item(
                item, episode,
            )),
            None => direct_download_submission_scope_for_wanted_item(item, episode),
        };
        episode_ids.extend(recovered_scope_episode_ids(app, &scope).await);
    }
    episode_ids.sort();
    episode_ids.dedup();

    proposals.push(GrabProposal {
        stage: BackgroundAcquisitionWorkKind::Scope,
        episode_ids,
        season_key: None,
        ranked_candidates: results,
        eligible,
        commit: GrabProposalCommit::EpisodeEvidence(Box::new(EpisodeEvidenceCommit {
            context: ScopeGrabContext {
                item: item.clone(),
                episode: episode.cloned(),
                media_type: target.media_type.clone(),
                download_category: app.derive_download_category(&title.facet).await,
                convergence_scope_key: None,
            },
            blocklist: db_blocklist,
        })),
    });
    Ok(())
}

/// Walk an episode scope's eligible candidates, best-first, and submit the
/// first one the download client accepts.
///
/// Relocated wholesale from the inline scope stage so a proposal that wins
/// arbitration commits through exactly the machinery that used to run in place:
/// the cycle-wide submission claim, the failed-route suppression, the blocklist
/// write on a definitive failure, and the walk to the next candidate.
#[expect(
    clippy::too_many_arguments,
    reason = "the relocated submission walk carries the scope context explicitly"
)]
async fn commit_scope_grab(
    app: &AppUseCase,
    title: &Title,
    context: &mut ScopeGrabContext,
    results: &[IndexerSearchResult],
    eligible: &[usize],
    failed_routes: &mut Vec<DownloadRouteKey>,
    db_blocklist: &crate::app_usecase_discovery::TitleReleaseBlocklistSignatures,
    cycle: &BackgroundAcquisitionCycleCoordinator,
    now: &DateTime<Utc>,
    failures: &mut GrabFailureTally,
) -> AppResult<ScopeGrabOutcome> {
    // A proposal built from in-hand evidence never reached the inline
    // `ensure_acquisition_scope_state`, and a grab needs its anchor row.
    if context.item.id.is_empty() {
        context.item.id = app
            .services
            .workflow
            .acquisition_scope_states
            .ensure_acquisition_scope_state(&context.item)
            .await?;
    }
    let item = &context.item;
    let episode = &context.episode;
    let mut grab_attempts: usize = 0;

    for candidate_index in eligible.iter().copied() {
        let candidate = &results[candidate_index];
        let candidate_score = candidate
            .quality_profile_decision
            .as_ref()
            .map(|decision| decision.preference_score)
            .unwrap_or(0);

        // ── Grab attempt ────────────────────────────────────────────────────
        grab_attempts += 1;
        if grab_attempts > 10 {
            warn!(
                title = title.name.as_str(),
                "reached max grab attempts (10), deferring to next cycle"
            );
            break;
        }

        // Submit to download client
        let canonical_source = candidate.canonical_download_source();
        let source_hint = canonical_source.as_ref().map(|(source, _)| source.clone());

        // Successful or ambiguous submissions stay globally deduplicated, but
        // a failed URL is suppressed only within its source/indexer route.
        if let Some(url) = source_hint.as_deref() {
            let route = DownloadRouteKey::for_candidate(candidate)
                .expect("candidate route key always exists, including unknown source kind");
            match cycle.claim_submission(route, url) {
                SubmissionClaim::Granted => {}
                SubmissionClaim::AlreadySubmitted => {
                    info!(
                        title = title.name.as_str(),
                        release = candidate.title.as_str(),
                        "skipping duplicate release already submitted this cycle"
                    );
                    continue;
                }
                SubmissionClaim::AlreadyAttempted | SubmissionClaim::RouteUnavailable => {
                    info!(
                        title = title.name.as_str(),
                        release = candidate.title.as_str(),
                        indexer_id = ?candidate.indexer_id,
                        source_kind = ?candidate.source_kind,
                        "skipping duplicate release already attempted or unavailable this cycle"
                    );
                    continue;
                }
            }
        }

        let source_title = Some(candidate.title.clone());
        let canonical_source_kind = canonical_source
            .as_ref()
            .map(|(_, kind)| *kind)
            .or(candidate.source_kind);
        let source_hint_for_attempt = normalize_release_attempt_hint(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_name(source_title.as_deref());
        let source_password = normalize_release_password(candidate.password_hint.as_deref());
        let request_signature = normalize_release_selection_signature(
            source_hint.as_deref(),
            source_title.as_deref(),
            canonical_source_kind,
        );

        let _ = app
            .services
            .workflow
            .release_attempts
            .record_release_attempt(
                Some(title.id.clone()),
                source_hint_for_attempt.clone(),
                source_title_for_attempt.clone(),
                ReleaseDownloadAttemptOutcome::Pending,
                None,
                source_password.clone(),
            )
            .await;

        let is_recent = app.is_recent_for_queue_priority(
            candidate
                .published_at
                .as_deref()
                .or(episode.as_ref().and_then(|item| item.air_date.as_deref()))
                .or(title.first_aired.as_deref())
                .or(title.digital_release_date.as_deref()),
        );

        info!(
            title = title.name.as_str(),
            release = candidate.title.as_str(),
            score = candidate_score,
            // Re-read rather than carried from the evaluation: a route another
            // candidate burned moments ago is part of why this one is next.
            decision = effective_auto_decision_code_for_route(candidate, failed_routes, db_blocklist)
                .as_str(),
            attempt = grab_attempts,
            "auto-grabbing release"
        );

        let info_hash_hint = candidate.info_hash().map(str::to_string);
        let seed_minimums = crate::ReleaseSeedMinimums::from_release_extra(&candidate.extra);
        // This path used to hardcode `season_pack: false`; the scored candidate
        // already carries a parse, so the seeding resolver can see a real pack.
        let is_season_pack = candidate
            .parsed_release_metadata
            .as_ref()
            .and_then(|parsed| parsed.episode.as_ref())
            .is_some_and(|episode| episode.full_season);
        let download_id = scryer_domain::download_identity::DownloadId::new();
        let submission_scope = if let Some(parsed) = candidate.parsed_release_metadata.as_ref() {
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
            crate::acquisition_coverage::resolve_release_coverage(
                parsed,
                &catalog_episodes,
                &catalog_collections,
                episode.as_ref(),
            )
            .submission_scope_or(&direct_download_submission_scope_for_wanted_item(
                item,
                episode.as_ref(),
            ))
        } else {
            direct_download_submission_scope_for_wanted_item(item, episode.as_ref())
        };

        let canonical_result = app
            .submit_canonical_download(CanonicalDownloadSubmissionIntent {
                request: DownloadClientAddRequest {
                    title: title.clone(),
                    search_facet: (context.media_type == "series_movie")
                        .then_some(MediaFacet::Movie),
                    purpose: crate::DownloadSubmissionPurpose::Standard,
                    download_id: Some(download_id),
                    source_hint: source_hint.clone(),
                    staged_nzb: None,
                    resolved_download_artifact: None,
                    source_kind: canonical_source_kind,
                    source_title: source_title.clone(),
                    source_password: source_password.clone(),
                    category: Some(context.download_category.clone()),
                    queue_priority: None,
                    download_directory: None,
                    release_title: Some(candidate.title.clone()),
                    indexer_name: Some(candidate.source.clone()),
                    indexer_id: candidate.indexer_id.clone(),
                    info_hash_hint: info_hash_hint.clone(),
                    seed_goal_ratio: None,
                    seed_goal_seconds: None,
                    tracker_min_seed_ratio: seed_minimums.min_seed_ratio,
                    tracker_min_seed_time_minutes: seed_minimums.min_seed_time_minutes,
                    season_pack_seed_ratio: seed_minimums.season_pack_seed_ratio,
                    season_pack_seed_time_minutes: seed_minimums.season_pack_seed_time_minutes,
                    is_recent,
                    season_pack: Some(is_season_pack),
                    pinned_download_client_id: None,
                },
                scope: submission_scope.clone(),
                conflict_policy: SubmissionConflictPolicy::Skip,
                request_signature: request_signature.clone(),
                source_provider_name: Some(candidate.source.clone()),
                release_size_bytes: candidate.size_bytes,
            })
            .await;

        let grab_indexer = app
            .grab_indexer_name(
                candidate.indexer_id.as_deref(),
                Some(candidate.source.as_str()),
            )
            .await;
        record_grab_submission_outcome(
            GrabTrigger::Auto,
            &title.facet,
            grab_indexer.as_deref(),
            &canonical_result,
        );

        let canonical_submission = match canonical_result {
            Ok(CanonicalDownloadSubmissionOutcome::Accepted(submission)) => Ok(submission),
            Ok(CanonicalDownloadSubmissionOutcome::Conflict(_)) => {
                // A submission for this release already exists: the scope is
                // spoken for, and its saved list was handled when that grab ran.
                return Ok(ScopeGrabOutcome::Settled {
                    claimed_episode_ids: recovered_scope_episode_ids(app, &submission_scope).await,
                });
            }
            Err(error) => Err(error),
        };

        match canonical_submission {
            Ok(canonical_submission) => {
                let newly_submitted = canonical_submission.newly_submitted;
                let grab = canonical_submission.grab;
                // ── Success ─────────────────────────────────────────────────
                if let Some(url) = source_hint.as_deref() {
                    cycle.mark_submitted(url);
                }
                if newly_submitted {
                    app.record_indexer_grab(
                        candidate.indexer_id.as_deref(),
                        grab_indexer.as_deref(),
                    );
                }
                let _ = app
                    .services
                    .workflow
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt.clone(),
                        source_title_for_attempt.clone(),
                        ReleaseDownloadAttemptOutcome::Success,
                        None,
                        source_password.clone(),
                    )
                    .await;

                // Record title history: Grabbed
                // Record download submission for auto-import matching
                let grabbed_json = serde_json::json!({
                    "title": candidate.title,
                    "score": candidate_score,
                    "grabbed_at": now.to_rfc3339(),
                    "source_provider": candidate.source.clone(),
                })
                .to_string();
                let download_job_id = grab.job_id.clone();
                let covered_wanted_item_ids = app
                    .covered_wanted_item_ids_for_submission_scope(
                        &title.id,
                        &submission_scope,
                        &item.id,
                    )
                    .await?;

                app.services
                    .workflow
                    .acquisition_state
                    .commit_successful_grab(&SuccessfulGrabCommit {
                        wanted_item_id: item.id.clone(),
                        covered_wanted_item_ids,
                        grabbed_release: grabbed_json,
                        last_search_at: Some(now.to_rfc3339()),
                        grabbed_pending_release_id: None,
                        grabbed_at: Some(now.to_rfc3339()),
                    })
                    .await?;
                persist_standby_candidates(
                    app,
                    item,
                    title,
                    results,
                    candidate_index + 1,
                    now,
                    failed_routes,
                    db_blocklist,
                    |_| true,
                )
                .await;

                let _ = app
                    .append_domain_event(new_title_domain_event(
                        None,
                        title,
                        DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                            title: title_context_snapshot(title),
                            source_title: Some(candidate.title.clone()),
                            source_hint: Some(candidate.source.clone()),
                            source_provider: Some(candidate.source.clone()),
                            download_id: Some(download_job_id),
                            episode_ids: item.episode_id.iter().cloned().collect(),
                        }),
                    ))
                    .await;

                return Ok(ScopeGrabOutcome::Settled {
                    claimed_episode_ids: recovered_scope_episode_ids(app, &submission_scope).await,
                });
            }
            Err(err) => {
                if err.is_download_submit_ambiguous() {
                    if let Some(url) = source_hint.as_deref() {
                        cycle.mark_submitted(url);
                    }
                    warn!(
                        title = title.name.as_str(),
                        release = candidate.title.as_str(),
                        attempt = grab_attempts,
                        error = %err,
                        "download submission result is ambiguous; re-opening scope without blocklisting or failover"
                    );

                    if let Some(scope_key) = context.convergence_scope_key.as_deref() {
                        app.prune_scope_key_coverage(scope_key, candidate.indexer_id.as_deref())
                            .await;
                    }

                    // The request may have been accepted. Treat the episodes as
                    // taken so no other proposal grabs them behind it.
                    return Ok(ScopeGrabOutcome::Settled {
                        claimed_episode_ids: recovered_scope_episode_ids(app, &submission_scope)
                            .await,
                    });
                }

                // ── Grab failed — try next candidate ────────────────────────
                // The scope as a whole counts once, however many candidates it
                // walks; an ambiguous submit returned above and never lands here.
                failures.record(&err);
                warn!(
                    title = title.name.as_str(),
                    release = candidate.title.as_str(),
                    attempt = grab_attempts,
                    error = %err,
                    "grab failed, trying next candidate"
                );

                let failure_reason = format!(
                    "grab failed for '{}' (attempt {}/10, trying next): {}",
                    candidate.title, grab_attempts, err
                );
                let source_gone = err.is_download_source_gone();
                let submit_unavailable = is_download_submit_unavailable_error(&err) || source_gone;

                if source_gone {
                    info!(
                        release = candidate.title.as_str(),
                        "download source gone; leaving it unblocked outside standby recovery"
                    );
                }

                if submit_unavailable {
                    let _ = app
                        .services
                        .workflow
                        .release_attempts
                        .record_release_attempt(
                            Some(title.id.clone()),
                            source_hint_for_attempt.clone(),
                            source_title_for_attempt.clone(),
                            ReleaseDownloadAttemptOutcome::Pending,
                            Some(failure_reason.clone()),
                            source_password.clone(),
                        )
                        .await;
                } else {
                    let attribution = FailedReleaseAttribution {
                        title: Some(title.clone()),
                        episode_ids: item.episode_id.iter().cloned().collect(),
                        collection_id: item.collection_id.clone(),
                    };
                    let candidate_source_hint = candidate
                        .canonical_download_source()
                        .map(|(source, _)| source)
                        .unwrap_or_else(|| candidate.source.clone());
                    let quality = candidate
                        .parsed_release_metadata
                        .as_ref()
                        .and_then(|parsed| parsed.quality.clone())
                        .or_else(|| release_quality_hint(Some(candidate.title.as_str())));

                    // A definitive grab failure burns the release for this title:
                    // the per-title blocklist entry is what search-time exclusion
                    // consults (and what the operator can remove); the Failed
                    // attempt is the audit record. Transient failures never
                    // reach here (Pending above).
                    record_failed_release_outcome(
                        app,
                        Some(title.id.as_str()),
                        &attribution,
                        Some(candidate.title.clone()),
                        Some(candidate_source_hint),
                        candidate.indexer_id.clone().unwrap_or_default(),
                        candidate.info_hash().map(str::to_string),
                        None,
                        None,
                        None,
                        None,
                        quality,
                        Some(failure_reason),
                        Some(format!("grab failed: {err}")),
                        source_password.clone(),
                    )
                    .await;
                }

                // If download-client submit is unavailable, suppress only this
                // source/indexer route for the remainder of this cycle.
                if submit_unavailable
                    && let Some(route) = DownloadRouteKey::for_candidate(candidate)
                {
                    if !failed_routes.contains(&route) {
                        failed_routes.push(route.clone());
                        cycle.mark_failed_route(route.clone());
                    }
                    info!(
                        source_kind = ?route.source_kind,
                        indexer_id = ?route.indexer_id,
                        "download client submit unavailable for route, skipping remaining candidates on this route"
                    );
                }

                // CONTINUE — try the next candidate
            }
        }
    }

    Ok(ScopeGrabOutcome::Exhausted)
}

/// The pending deferred-work deadline after a cycle: the earlier of what is
/// already armed and what this cycle asks for, so repeated cycles coalesce onto
/// one timer instead of stacking.
///
/// `poll_interval` is the bound: a re-arm that would not land before the next
/// tick is not worth running an extra cycle for.
fn arm_deferred_acquisition_retry(
    outcome: BackgroundAcquisitionCycleOutcome,
    armed: Option<tokio::time::Instant>,
    poll_interval: std::time::Duration,
) -> Option<tokio::time::Instant> {
    let Some(delay) = outcome.deferred_retry_delay(poll_interval) else {
        return armed;
    };
    let deadline = tokio::time::Instant::now() + delay;
    debug!(
        deferred_scopes = outcome.deferred_scopes,
        delay_ms = delay.as_millis() as u64,
        "background acquisition: re-arming after deferred work"
    );
    Some(armed.map_or(deadline, |armed| armed.min(deadline)))
}

pub async fn start_background_acquisition_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    // Check feature flag
    let enabled = std::env::var("SCRYER_BACKGROUND_ACQUISITION")
        .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "0" | "no" | "off"))
        .unwrap_or(true);

    if !enabled {
        info!("background acquisition poller is disabled (SCRYER_BACKGROUND_ACQUISITION=false)");
        return;
    }

    let settings = match app.acquisition_settings().await {
        Ok(settings) => settings,
        Err(err) => {
            warn!(error = %err, "failed to load acquisition settings, using defaults");
            crate::AcquisitionSettings {
                enabled: true,
                upgrade_cooldown_hours: 24,
                same_tier_min_delta: 120,
                cross_tier_min_delta: 30,
                forced_upgrade_delta_bypass: 400,
                poll_interval_seconds: 60,
                long_tail_backfill_max_scopes_per_cycle:
                    crate::acquisition::convergence::DEFAULT_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE
                        as i32,
                long_tail_reconverge_days: 0,
            }
        }
    };

    if !settings.enabled {
        info!("background acquisition poller is disabled (acquisition.enabled != true)");
        return;
    }

    info!("background acquisition poller started");

    // Run-once cutover seed: recently-searched legacy scopes
    // start converged so first boot does not re-sweep the back-catalog.
    // Spawned so startup stays non-blocking; the cycle racing the seed is
    // harmless (either path only causes a safe converge).
    {
        let app = app.clone();
        tokio::spawn(async move {
            app.seed_convergence_from_legacy_history().await;
        });
    }

    // Run initial health checks after a short delay to let services initialize
    {
        let app = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            if let Err(error) = app
                .run_scheduled_job_now(JobKey::HealthChecks, JobTriggerSource::ScheduledStartup)
                .await
            {
                warn!(error = %error, "initial health checks failed");
            }
        });
    }

    // Refresh managed Prowlarr children as soon as the app is up so startup
    // picks up upstream indexer/config changes without waiting for the first
    // 5-minute interval.
    {
        let app = app.clone();
        tokio::spawn(async move {
            if let Err(error) = app
                .run_scheduled_job_now(JobKey::ProwlarrSync, JobTriggerSource::ScheduledStartup)
                .await
            {
                warn!(error = %error, "initial Prowlarr sync failed");
            }
        });
    }
    {
        let app = app.clone();
        tokio::spawn(async move {
            let actor = scryer_domain::User::new_admin("system-indexer-caps");
            if let Err(error) = app.refresh_enabled_direct_nab_caps_snapshots(&actor).await {
                warn!(error = %error, "initial direct indexer caps refresh failed");
            }
        });
    }

    app.set_job_next_run_at(
        JobKey::PluginRegistryRefresh,
        Utc::now() + chrono::Duration::hours(1),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::HealthChecks,
        Utc::now() + chrono::Duration::seconds(30),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::StagedNzbPrune,
        Utc::now() + chrono::Duration::hours(1),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::FullHashBackfill,
        Utc::now() + chrono::Duration::minutes(15),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::Housekeeping,
        Utc::now() + chrono::Duration::hours(24),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::ProwlarrSync,
        Utc::now() + chrono::Duration::minutes(5),
    )
    .await;
    app.set_job_next_run_at(JobKey::RssSync, Utc::now() + chrono::Duration::minutes(1))
        .await;
    app.set_job_next_run_at(
        JobKey::PendingReleaseProcessing,
        Utc::now() + chrono::Duration::minutes(1),
    )
    .await;

    // Maintenance evaluation (RFC 137 section 10) runs on an eight-hour
    // cadence, offset by a per-instance jitter so a fleet of instances does not
    // sweep their libraries in lockstep. The jitter window is deliberately much
    // smaller than the cadence: it exists to spread load, not to delay the
    // first pass by most of a working day. The job itself re-reads the instance
    // gate at the start of every run, so an operator arming or disarming
    // evaluation never has to restart Scryer.
    let maintenance_evaluation_cadence = std::time::Duration::from_secs(
        crate::jobs::MAINTENANCE_RULE_EVALUATION_INTERVAL_SECONDS as u64,
    );
    let maintenance_evaluation_offset = match app.discovery_scheduler_seed().await {
        Ok(seed) => crate::scheduler::stable_jitter_offset(
            &seed,
            "maintenance_rule_evaluation",
            "global",
            MAINTENANCE_EVALUATION_JITTER_WINDOW,
        ),
        Err(error) => {
            warn!(error = %error, "could not resolve the scheduler seed; maintenance evaluation runs unjittered");
            std::time::Duration::ZERO
        }
    };
    app.set_job_next_run_at(
        JobKey::MaintenanceRuleEvaluation,
        Utc::now()
            + chrono::Duration::from_std(maintenance_evaluation_offset)
                .unwrap_or_else(|_| chrono::Duration::zero()),
    )
    .await;

    let lifecycle_action_cadence = std::time::Duration::from_secs(
        crate::jobs::LIFECYCLE_ACTION_HANDLING_INTERVAL_SECONDS as u64,
    );
    let lifecycle_action_offset = match app.discovery_scheduler_seed().await {
        Ok(seed) => crate::scheduler::stable_jitter_offset(
            &seed,
            "lifecycle_action_handling",
            "global",
            LIFECYCLE_ACTION_HANDLING_JITTER_WINDOW,
        ),
        Err(error) => {
            warn!(error = %error, "could not resolve the scheduler seed; action handling runs unjittered");
            std::time::Duration::ZERO
        }
    };
    app.set_job_next_run_at(
        JobKey::LifecycleActionHandling,
        Utc::now()
            + chrono::Duration::from_std(lifecycle_action_offset)
                .unwrap_or_else(|_| chrono::Duration::zero()),
    )
    .await;

    let media_server_signal_cadence = std::time::Duration::from_secs(
        crate::jobs::MEDIA_SERVER_SIGNAL_SYNC_INTERVAL_SECONDS as u64,
    );
    let media_server_signal_offset = match app.discovery_scheduler_seed().await {
        Ok(seed) => crate::scheduler::stable_jitter_offset(
            &seed,
            "media_server_signal_sync",
            "global",
            MEDIA_SERVER_SIGNAL_SYNC_JITTER_WINDOW,
        ),
        Err(error) => {
            warn!(error = %error, "could not resolve the scheduler seed; media-server signal sync runs unjittered");
            std::time::Duration::ZERO
        }
    };
    app.set_job_next_run_at(
        JobKey::MediaServerSignalSync,
        Utc::now()
            + chrono::Duration::from_std(media_server_signal_offset)
                .unwrap_or_else(|_| chrono::Duration::zero()),
    )
    .await;

    let acquisition_poll_period =
        std::time::Duration::from_secs(settings.poll_interval_seconds.max(1) as u64);
    let mut poll_interval = new_skip_interval(acquisition_poll_period);
    let mut registry_refresh_interval = tokio::time::interval(std::time::Duration::from_hours(1));
    let mut health_check_interval = tokio::time::interval(std::time::Duration::from_hours(6));
    let mut staged_nzb_prune_interval = tokio::time::interval(std::time::Duration::from_hours(1));
    // FR-047: the backfill's own throttling lives inside the job; the interval
    // only decides how often a bounded sweep is offered a turn.
    let mut full_hash_backfill_interval =
        tokio::time::interval(std::time::Duration::from_mins(30));
    let mut housekeeping_interval = tokio::time::interval(std::time::Duration::from_hours(24));
    let mut prowlarr_sync_interval = tokio::time::interval(std::time::Duration::from_mins(5));
    let mut direct_indexer_caps_interval =
        tokio::time::interval(std::time::Duration::from_hours(24));
    let mut rss_sync_interval = tokio::time::interval(std::time::Duration::from_mins(1));
    let mut pending_release_interval = tokio::time::interval(std::time::Duration::from_mins(1));
    let mut maintenance_evaluation_interval = tokio::time::interval_at(
        tokio::time::Instant::now() + maintenance_evaluation_offset,
        maintenance_evaluation_cadence,
    );
    let mut lifecycle_action_interval = tokio::time::interval_at(
        tokio::time::Instant::now() + lifecycle_action_offset,
        lifecycle_action_cadence,
    );
    let mut media_server_signal_interval = tokio::time::interval_at(
        tokio::time::Instant::now() + media_server_signal_offset,
        media_server_signal_cadence,
    );

    // Consume immediate intervals.
    poll_interval.tick().await;
    registry_refresh_interval.tick().await;
    health_check_interval.tick().await;
    staged_nzb_prune_interval.tick().await;
    full_hash_backfill_interval.tick().await;
    housekeeping_interval.tick().await;
    prowlarr_sync_interval.tick().await;
    direct_indexer_caps_interval.tick().await;
    rss_sync_interval.tick().await;
    pending_release_interval.tick().await;

    {
        let app = app.clone();
        let token = token.child_token();
        tokio::spawn(async move {
            run_discovery_sync_worker(app, token).await;
        });
    }

    let wake = app.runtime.acquisition.acquisition_wake.clone();

    /// Run a scheduled task inside a spawned task to isolate panics.
    /// If the task panics, the error is logged and the scheduler loop continues.
    async fn run_task(
        task_name: &'static str,
        fut: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let t = std::time::Instant::now();
        let outcome = tokio::spawn(fut).await;
        // Freshness: a counter that stops moving is indistinguishable from a
        // scrape that never happened, so record *when* the task last ran at
        // all (panic included) and, separately, when it last got through its
        // body. The task future is `()`-typed, so returning at all is the only
        // success signal available here.
        let now = crate::metrics_support::unix_seconds(Utc::now());
        metrics::gauge!(
            crate::metrics_support::TASK_LAST_RUN_TIMESTAMP_SECONDS,
            "task" => task_name
        )
        .set(now);
        match outcome {
            Ok(()) => {
                metrics::gauge!(
                    crate::metrics_support::TASK_LAST_SUCCESS_TIMESTAMP_SECONDS,
                    "task" => task_name
                )
                .set(now);
            }
            Err(e) => {
                tracing::error!(
                    task = task_name,
                    error = %e,
                    "CRITICAL: scheduled task panicked — scheduler continues but this task failed"
                );
                metrics::counter!("scryer_task_panics_total", "task" => task_name).increment(1);
            }
        }
        metrics::counter!("scryer_task_runs_total", "task" => task_name).increment(1);
        metrics::histogram!("scryer_task_duration_seconds", "task" => task_name)
            .record(t.elapsed().as_secs_f64());
    }

    /// One convergence cycle through the panic-isolating wrapper, with its
    /// outcome carried back so the loop can re-arm on deferred work. A cycle
    /// that panicked reports the default outcome — nothing deferred — so a
    /// panicking cycle cannot drive a retry loop.
    async fn run_acquisition_cycle_task(app: &AppUseCase) -> BackgroundAcquisitionCycleOutcome {
        let sink = Arc::new(Mutex::new(BackgroundAcquisitionCycleOutcome::default()));
        let recorded = Arc::clone(&sink);
        let app = app.clone();
        run_task("background_acquisition_cycle", async move {
            let outcome = run_background_acquisition_cycle(&app).await;
            *recorded
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = outcome;
        })
        .await;
        *sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // Bounded re-arm for a scope whose every uncovered indexer was cooling
    // down: the cooldown lifts on its own and re-arms nothing, so without this
    // the scope waits out the full tick. Only a cooldown that expires *inside*
    // the poll interval arms it — an untimed deferral (an exhausted quota) and
    // a title an interactive walk holds both wait for the tick, the latter
    // because the job wakes the poller itself when it releases the lock. One
    // pending deadline at a time, and floored so a condition that has just
    // lifted cannot spin the loop.
    let mut deferred_retry_at: Option<tokio::time::Instant> = None;

    loop {
        // Recreated each iteration from an absolute deadline, so re-entering
        // the select never pushes the re-arm further out.
        let deferred_retry_deadline = deferred_retry_at
            .unwrap_or_else(|| tokio::time::Instant::now() + std::time::Duration::from_secs(3600));
        tokio::select! {
            _ = token.cancelled() => {
                info!("background acquisition poller shutting down");
                break;
            }
            _ = wake.notified() => {
                let outcome = run_acquisition_cycle_task(&app).await;
                deferred_retry_at = arm_deferred_acquisition_retry(
                    outcome,
                    deferred_retry_at,
                    acquisition_poll_period,
                );
            }
            _ = poll_interval.tick() => {
                let outcome = run_acquisition_cycle_task(&app).await;
                deferred_retry_at = arm_deferred_acquisition_retry(
                    outcome,
                    deferred_retry_at,
                    acquisition_poll_period,
                );
            }
            // The fired deadline is dropped rather than carried forward, so a
            // cycle that clears the deferral disarms the timer entirely.
            _ = tokio::time::sleep_until(deferred_retry_deadline), if deferred_retry_at.is_some() => {
                let outcome = run_acquisition_cycle_task(&app).await;
                deferred_retry_at =
                    arm_deferred_acquisition_retry(outcome, None, acquisition_poll_period);
            }
            _ = registry_refresh_interval.tick() => {
                let app = app.clone();
                run_task("registry_refresh", async move {
                    app.set_job_next_run_at(
                        JobKey::PluginRegistryRefresh,
                        Utc::now() + chrono::Duration::hours(1),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::PluginRegistryRefresh, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic plugin registry refresh failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "registry_refresh").increment(1);
                    }
                }).await;
            }
            _ = health_check_interval.tick() => {
                let app = app.clone();
                run_task("health_check", async move {
                    app.set_job_next_run_at(
                        JobKey::HealthChecks,
                        Utc::now() + chrono::Duration::hours(6),
                    ).await;
                    if let Err(err) = app.run_scheduled_job_now(JobKey::HealthChecks, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %err, "periodic health checks failed");
                    }
                }).await;
            }
            _ = staged_nzb_prune_interval.tick() => {
                let app = app.clone();
                run_task("staged_nzb_prune", async move {
                    app.set_job_next_run_at(
                        JobKey::StagedNzbPrune,
                        Utc::now() + chrono::Duration::hours(1),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::StagedNzbPrune, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic staged nzb prune failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "staged_nzb_prune").increment(1);
                    }
                }).await;
            }
            _ = full_hash_backfill_interval.tick() => {
                let app = app.clone();
                run_task("full_hash_backfill", async move {
                    app.set_job_next_run_at(
                        JobKey::FullHashBackfill,
                        Utc::now() + chrono::Duration::minutes(30),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::FullHashBackfill, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic full hash backfill failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "full_hash_backfill").increment(1);
                    }
                }).await;
            }
            _ = housekeeping_interval.tick() => {
                let app = app.clone();
                run_task("housekeeping", async move {
                    app.set_job_next_run_at(
                        JobKey::Housekeeping,
                        Utc::now() + chrono::Duration::hours(24),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::Housekeeping, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic housekeeping failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "housekeeping").increment(1);
                    }
                }).await;
            }
            _ = pending_release_interval.tick() => {
                let app = app.clone();
                run_task("pending_releases", async move {
                    app.set_job_next_run_at(
                        JobKey::PendingReleaseProcessing,
                        Utc::now() + chrono::Duration::minutes(1),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::PendingReleaseProcessing, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "pending release processor failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "pending_releases").increment(1);
                    }
                }).await;
            }
            _ = prowlarr_sync_interval.tick() => {
                let app = app.clone();
                run_task("prowlarr_sync", async move {
                    app.set_job_next_run_at(
                        JobKey::ProwlarrSync,
                        Utc::now() + chrono::Duration::minutes(5),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::ProwlarrSync, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic Prowlarr sync failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "prowlarr_sync").increment(1);
                    }
                }).await;
            }
            _ = direct_indexer_caps_interval.tick() => {
                let app = app.clone();
                run_task("direct_indexer_caps", async move {
                    let actor = scryer_domain::User::new_admin("system-indexer-caps");
                    if let Err(error) = app.refresh_enabled_direct_nab_caps_snapshots(&actor).await {
                        warn!(error = %error, "periodic direct indexer caps refresh failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "direct_indexer_caps").increment(1);
                    }
                }).await;
            }
            _ = maintenance_evaluation_interval.tick() => {
                let app = app.clone();
                let cadence = maintenance_evaluation_cadence;
                run_task("maintenance_rule_evaluation", async move {
                    app.set_job_next_run_at(
                        JobKey::MaintenanceRuleEvaluation,
                        Utc::now()
                            + chrono::Duration::from_std(cadence)
                                .unwrap_or_else(|_| chrono::Duration::hours(8)),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::MaintenanceRuleEvaluation, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "scheduled maintenance rule evaluation failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "maintenance_rule_evaluation").increment(1);
                    }
                }).await;
            }
            _ = lifecycle_action_interval.tick() => {
                let app = app.clone();
                let cadence = lifecycle_action_cadence;
                run_task("lifecycle_action_handling", async move {
                    app.set_job_next_run_at(
                        JobKey::LifecycleActionHandling,
                        Utc::now()
                            + chrono::Duration::from_std(cadence)
                                .unwrap_or_else(|_| chrono::Duration::hours(12)),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::LifecycleActionHandling, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "scheduled maintenance action handling failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "lifecycle_action_handling").increment(1);
                    }
                }).await;
            }
            _ = media_server_signal_interval.tick() => {
                let app = app.clone();
                let cadence = media_server_signal_cadence;
                run_task("media_server_signal_sync", async move {
                    app.set_job_next_run_at(
                        JobKey::MediaServerSignalSync,
                        Utc::now()
                            + chrono::Duration::from_std(cadence)
                                .unwrap_or_else(|_| chrono::Duration::hours(6)),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::MediaServerSignalSync, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "scheduled media-server signal sync failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "media_server_signal_sync").increment(1);
                    }
                }).await;
            }
            _ = rss_sync_interval.tick() => {
                let app = app.clone();
                run_task("rss_sync", async move {
                    app.set_job_next_run_at(
                        JobKey::RssSync,
                        Utc::now() + chrono::Duration::minutes(1),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::RssSync, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic RSS sync failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "rss_sync").increment(1);
                    }
                }).await;
            }
        }
    }
}

async fn run_discovery_sync_worker(app: AppUseCase, token: tokio_util::sync::CancellationToken) {
    // The acquisition poller spawns this worker, so awaiting the startup pass here
    // keeps service startup nonblocking while preventing overlapping discovery runs.
    let discovery_sync_wake = app.runtime.jobs.discovery_sync_wake.clone();
    let mut delay = tokio::select! {
        _ = token.cancelled() => return,
        delay = run_discovery_sync_once(&app, JobTriggerSource::ScheduledStartup) => delay,
    };

    loop {
        tokio::select! {
            _ = token.cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
            _ = discovery_sync_wake.notified() => {}
        }

        if let Some(next_run_at) = app
            .runtime
            .jobs
            .job_run_tracker
            .next_run_at(JobKey::DiscoverySync)
            .await
            && next_run_at > Utc::now()
        {
            delay = discovery_sync_delay_until(next_run_at);
            continue;
        }

        delay = run_discovery_sync_once(&app, JobTriggerSource::ScheduledInterval).await;
    }
}

async fn run_discovery_sync_once(
    app: &AppUseCase,
    trigger_source: JobTriggerSource,
) -> std::time::Duration {
    let started = std::time::Instant::now();
    if let Err(error) = app
        .run_scheduled_job_now(JobKey::DiscoverySync, trigger_source)
        .await
    {
        warn!(
            error = %error,
            trigger_source = trigger_source.as_str(),
            "discovery sync failed"
        );
        metrics::counter!("scryer_task_errors_total", "task" => "discovery_sync").increment(1);
    }
    metrics::counter!("scryer_task_runs_total", "task" => "discovery_sync").increment(1);
    metrics::histogram!("scryer_task_duration_seconds", "task" => "discovery_sync")
        .record(started.elapsed().as_secs_f64());

    app.runtime
        .jobs
        .job_run_tracker
        .next_run_at(JobKey::DiscoverySync)
        .await
        .map(discovery_sync_delay_until)
        .unwrap_or_else(|| std::time::Duration::from_secs(24 * 60 * 60))
}

fn discovery_sync_delay_until(next_run_at: DateTime<Utc>) -> std::time::Duration {
    (next_run_at - Utc::now())
        .to_std()
        .ok()
        .filter(|delay| *delay >= std::time::Duration::from_secs(60))
        .unwrap_or_else(|| std::time::Duration::from_secs(60))
}

#[cfg(test)]
mod task_runner_tests {
    use super::*;
    use crate::acquisition::targets::AcquisitionTarget;

    #[test]
    fn non_metadata_scheduled_job_intervals_remain_unchanged() {
        assert_eq!(JobKey::RssSync.interval_seconds(), Some(15 * 60));
        assert_eq!(
            JobKey::PluginRegistryRefresh.interval_seconds(),
            Some(60 * 60)
        );
        assert_eq!(JobKey::HealthChecks.interval_seconds(), Some(6 * 60 * 60));
        assert_eq!(JobKey::StagedNzbPrune.interval_seconds(), Some(60 * 60));
    }

    #[test]
    fn discovery_sync_delay_until_clamps_stale_times() {
        let stale = Utc::now() - chrono::Duration::minutes(5);
        assert_eq!(
            discovery_sync_delay_until(stale),
            std::time::Duration::from_secs(60)
        );
    }

    fn wanted_episode_item(
        title_id: &str,
        title_name: &str,
        episode_number: u32,
    ) -> AcquisitionScopeState {
        AcquisitionScopeState {
            id: format!("{title_id}-e{episode_number}"),
            title_id: title_id.to_string(),
            title_name: Some(title_name.to_string()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: Some(format!("{title_id}-episode-{episode_number}")),
            collection_id: None,
            series_movie_link_id: None,
            season_number: Some("1".to_string()),
            episode_number: Some(episode_number.to_string()),
            media_type: "episode".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn episode_submission(title_id: &str, episode_id: &str, job_id: &str) -> DownloadSubmission {
        DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title_id.to_string(),
            purpose: DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: job_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Episode {
                episode_id: episode_id.to_string(),
            },
        }
    }

    fn snapshot_with_job(job_id: &str, completed: bool) -> DownloadClientSnapshot {
        let key = download_client_item_identity(Some("primary"), job_id);
        let mut snapshot = DownloadClientSnapshot {
            active_titles: Default::default(),
            active_client_ids: Default::default(),
            active_raw_item_id_counts: Default::default(),
            stale_downloading_client_ids: Default::default(),
            stale_downloading_raw_item_ids: Default::default(),
            completed_client_ids: Default::default(),
            completed_raw_item_id_counts: Default::default(),
            failed_by_download_id: Default::default(),
            queue_listing_failed: false,
            history_listing_failed: false,
            unreadable_client_ids: Default::default(),
        };
        if completed {
            snapshot.completed_client_ids.insert(key);
        } else {
            snapshot.active_client_ids.insert(key);
        }
        snapshot
    }

    /// The queue could not be listed at all this cycle.
    fn blind_queue_snapshot() -> DownloadClientSnapshot {
        DownloadClientSnapshot {
            active_titles: Default::default(),
            active_client_ids: Default::default(),
            active_raw_item_id_counts: Default::default(),
            stale_downloading_client_ids: Default::default(),
            stale_downloading_raw_item_ids: Default::default(),
            completed_client_ids: Default::default(),
            completed_raw_item_id_counts: Default::default(),
            failed_by_download_id: Default::default(),
            queue_listing_failed: true,
            history_listing_failed: false,
            unreadable_client_ids: Default::default(),
        }
    }

    #[test]
    fn completed_submission_blocks_initial_wanted_search() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-baseline");
        let snapshot = snapshot_with_job("job-baseline", true);

        // Nothing occupies the scope yet, so the finished download is still on
        // its way to becoming a file: searching again would duplicate it.
        assert!(submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            None,
            false,
        ));
    }

    #[test]
    fn failed_submission_does_not_block_completed_initial_wanted_search() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-failed");
        let snapshot = snapshot_with_job("job-failed", true);

        assert!(!submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            Some(scryer_domain::TrackedDownloadState::Failed),
            false,
        ));
    }

    #[test]
    fn completed_submission_does_not_block_upgrade_search() {
        let mut item = wanted_episode_item("title-bluey", "Bluey", 1);
        item.grabbed_release = Some("Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb".to_string());
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-baseline");
        let snapshot = snapshot_with_job("job-baseline", true);

        // A file already occupies the scope, so this download has resolved one
        // way or the other and an upgrade search may proceed.
        assert!(!submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            None,
            true,
        ));
    }

    /// An active initial acquisition owns an empty scope. Episode search must
    /// wait for the existing download or pack to resolve.
    #[test]
    fn an_active_submission_blocks_an_empty_scope() {
        let item = wanted_episode_item("title-synthetic", "Synthetic Show", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-upgrade");
        let snapshot = snapshot_with_job("job-upgrade", false);

        assert!(submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            None,
            false,
        ));
    }

    /// Import-blocked media still owns an empty scope until an operator resolves
    /// or removes that manual import.
    #[test]
    fn an_import_blocked_submission_blocks_an_empty_scope() {
        let item = wanted_episode_item("title-synthetic", "Synthetic Show", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-blocked");
        let snapshot = snapshot_with_job("job-blocked", true);

        assert!(submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            Some(scryer_domain::TrackedDownloadState::ImportBlocked),
            false,
        ));
    }

    /// A claim still `Downloading` past the staleness bound stops suppressing
    /// its empty scope: a dead swarm never fails on its own, and without the
    /// escape the scope froze until an operator noticed. The scope falls back
    /// to the D18 pseudo-incumbent comparison; a fresh download keeps the
    /// suppression, and so does a stale one on a blind cycle.
    #[test]
    fn a_stale_downloading_claim_stops_blocking_its_empty_scope() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-stalled");

        // Fresh active download: suppressed, exactly the new rule.
        let fresh = snapshot_with_job("job-stalled", false);
        assert!(submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &fresh,
            Some(scryer_domain::TrackedDownloadState::Downloading),
            false,
        ));

        // The same download past the staleness bound: back to D18 comparison.
        let mut stale = snapshot_with_job("job-stalled", false);
        stale
            .stale_downloading_client_ids
            .insert(download_client_item_identity(Some("primary"), "job-stalled"));
        stale
            .stale_downloading_raw_item_ids
            .insert("job-stalled".to_string());
        assert!(!submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &stale,
            Some(scryer_domain::TrackedDownloadState::Downloading),
            false,
        ));

        // An occupied scope was never suppressed and stays comparison-based.
        assert!(!submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &stale,
            Some(scryer_domain::TrackedDownloadState::Downloading),
            true,
        ));
    }

    /// The two cases that still hard-skip: a failure the handler has not
    /// processed yet (Sonarr excludes `FailedPending` from its queue spec for
    /// the same reason), and a queue that could not be listed at all — with no
    /// way to build honest pseudo-incumbents, the old whole-scope skip is the
    /// safe answer.
    #[test]
    fn a_failed_pending_submission_and_a_blind_queue_still_hard_skip() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-failed");
        let snapshot = snapshot_with_job("job-failed", true);

        assert!(submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            Some(scryer_domain::TrackedDownloadState::FailedPending),
            true,
        ));

        let blind = blind_queue_snapshot();
        assert!(submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &blind,
            None,
            true,
        ));
    }

    #[test]
    fn terminal_imported_state_preserves_normal_upgrade_search() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-imported");
        let snapshot = snapshot_with_job("job-imported", true);

        assert!(!submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            Some(scryer_domain::TrackedDownloadState::Imported),
            true,
        ));
    }

    fn background_acquisition_episode_target(
        title_id: &str,
        season: u32,
        episode: u32,
    ) -> AcquisitionTarget {
        AcquisitionTarget {
            scope_key: format!("{title_id}-s{season}-e{episode}"),
            title_id: title_id.to_string(),
            library_id: "library".to_string(),
            facet: MediaFacet::Series,
            media_type: "episode".to_string(),
            episode_id: Some(format!("{title_id}-s{season}-e{episode}")),
            collection_id: Some(format!("{title_id}-s{season}")),
            series_movie_link_id: None,
            season_number: Some(season.to_string()),
            episode_number: Some(episode.to_string()),
            is_hot: false,
            hot_by_air_date: false,
            occupied: false,
        }
    }

    #[test]
    fn background_acquisition_title_queue_enforces_pack_first_order() {
        let targets = vec![
            background_acquisition_episode_target("synthetic-title", 1, 1),
            background_acquisition_episode_target("synthetic-title", 1, 2),
            background_acquisition_episode_target("synthetic-title", 2, 1),
            background_acquisition_episode_target("synthetic-title", 2, 2),
        ];
        let ready_titles = build_background_acquisition_title_work(&targets, &[0, 1, 2, 3], None);

        assert_eq!(ready_titles.len(), 1);
        let title_work = ready_titles.front().expect("title work");
        assert_eq!(
            title_work
                .ready
                .iter()
                .map(|work| work.target_index)
                .collect::<Vec<_>>(),
            vec![0, 2, 0, 1, 2, 3]
        );
        assert!(matches!(
            title_work.ready[0].kind,
            BackgroundAcquisitionWorkKind::TitlePack
        ));
        assert!(matches!(
            title_work.ready[1].kind,
            BackgroundAcquisitionWorkKind::SeasonPack { season: 2 }
        ));
        assert!(
            title_work
                .ready
                .iter()
                .skip(2)
                .all(|work| matches!(&work.kind, BackgroundAcquisitionWorkKind::Scope))
        );
    }

    #[test]
    fn background_acquisition_title_queue_walks_seasons_and_episodes_in_ascending_order() {
        // Derived order is the datastore's (episode rows come back ordered by
        // random UUID), so the queue has to impose the order itself.
        let targets = vec![
            background_acquisition_episode_target("synthetic-title", 2, 10),
            background_acquisition_episode_target("synthetic-title", 1, 2),
            background_acquisition_episode_target("synthetic-title", 3, 1),
            background_acquisition_episode_target("synthetic-title", 1, 1),
            background_acquisition_episode_target("synthetic-title", 2, 2),
        ];
        let ready_titles = build_background_acquisition_title_work(&targets, &[0, 1, 2, 3, 4], None);

        let title_work = ready_titles.front().expect("title work");
        let scope_order = title_work
            .ready
            .iter()
            .filter(|work| matches!(work.kind, BackgroundAcquisitionWorkKind::Scope))
            .map(|work| {
                (
                    targets[work.target_index].season_number.clone(),
                    targets[work.target_index].episode_number.clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            scope_order,
            vec![
                (Some("1".to_string()), Some("1".to_string())),
                (Some("1".to_string()), Some("2".to_string())),
                (Some("2".to_string()), Some("2".to_string())),
                (Some("2".to_string()), Some("10".to_string())),
                (Some("3".to_string()), Some("1".to_string())),
            ]
        );

        let pack_seasons = title_work
            .ready
            .iter()
            .filter_map(|work| match work.kind {
                BackgroundAcquisitionWorkKind::SeasonPack { season } => Some(season),
                _ => None,
            })
            .collect::<Vec<_>>();
        // Season 1's pack is the title-pack stage's anchor, so it is not
        // repeated; the rest run in ascending season order.
        assert_eq!(pack_seasons, vec![2, 3]);
    }

    #[test]
    fn background_acquisition_title_queue_keeps_unnumbered_scopes_last() {
        let mut unnumbered = background_acquisition_episode_target("synthetic-title", 1, 1);
        unnumbered.scope_key = "synthetic-title-special".to_string();
        unnumbered.season_number = None;
        unnumbered.episode_number = None;
        let targets = vec![
            unnumbered,
            background_acquisition_episode_target("synthetic-title", 1, 1),
        ];
        let ready_titles = build_background_acquisition_title_work(&targets, &[0, 1], None);

        let title_work = ready_titles.front().expect("title work");
        let scope_indices = title_work
            .ready
            .iter()
            .filter(|work| matches!(work.kind, BackgroundAcquisitionWorkKind::Scope))
            .map(|work| work.target_index)
            .collect::<Vec<_>>();
        assert_eq!(scope_indices, vec![1, 0]);
    }

    #[test]
    fn season_scoped_title_queue_drops_the_title_pack_stage_and_other_seasons() {
        let targets = vec![
            background_acquisition_episode_target("synthetic-title", 1, 1),
            background_acquisition_episode_target("synthetic-title", 2, 2),
            background_acquisition_episode_target("synthetic-title", 2, 1),
        ];
        let ready_titles =
            build_background_acquisition_title_work(&targets, &[0, 1, 2], Some(2));

        let title_work = ready_titles.front().expect("title work");
        let kinds = title_work
            .ready
            .iter()
            .map(|work| (work.kind.clone(), work.target_index))
            .collect::<Vec<_>>();
        assert!(
            !kinds
                .iter()
                .any(|(kind, _)| matches!(kind, BackgroundAcquisitionWorkKind::TitlePack)),
            "a season-scoped walk must not grab a whole-series pack: {kinds:?}"
        );
        assert!(matches!(
            kinds[0],
            (BackgroundAcquisitionWorkKind::SeasonPack { season: 2 }, 2)
        ));
        assert_eq!(
            kinds
                .iter()
                .skip(1)
                .map(|(_, index)| *index)
                .collect::<Vec<_>>(),
            vec![2, 1],
            "only season 2 scopes, in episode order"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn deferred_retry_arms_only_a_timed_cooldown_and_coalesces_onto_one_deadline() {
        const POLL: std::time::Duration = std::time::Duration::from_secs(60);

        assert!(
            arm_deferred_acquisition_retry(
                BackgroundAcquisitionCycleOutcome::default(),
                None,
                POLL
            )
            .is_none(),
            "a cycle with nothing deferred must not arm a timer"
        );

        // An untimed deferral — an exhausted quota carries no recovery instant —
        // waits for the poller's own tick rather than guessing at a delay.
        assert!(
            arm_deferred_acquisition_retry(
                BackgroundAcquisitionCycleOutcome {
                    deferred_scopes: 1,
                    retry_after: None,
                    ..BackgroundAcquisitionCycleOutcome::default()
                },
                None,
                POLL,
            )
            .is_none(),
            "an untimed deferral waits for the tick"
        );

        // A title an interactive walk holds is not deferred work at all: the job
        // wakes the poller when it releases the lock, so a multi-minute walk
        // cannot drive a cycle every retry interval for its whole length.
        assert!(
            arm_deferred_acquisition_retry(
                BackgroundAcquisitionCycleOutcome {
                    skipped_locked_titles: 3,
                    retry_after: Some(std::time::Duration::from_secs(5)),
                    ..BackgroundAcquisitionCycleOutcome::default()
                },
                None,
                POLL,
            )
            .is_none(),
            "a title-lock skip must not arm a timer"
        );

        // A cooldown that outlasts the poll interval is left to the tick: an
        // extra cycle that lands after the tick buys nothing.
        assert!(
            arm_deferred_acquisition_retry(
                BackgroundAcquisitionCycleOutcome {
                    deferred_scopes: 1,
                    retry_after: Some(std::time::Duration::from_secs(600)),
                    ..BackgroundAcquisitionCycleOutcome::default()
                },
                None,
                POLL,
            )
            .is_none(),
            "a cooldown longer than the poll interval is the tick's job"
        );

        let now = tokio::time::Instant::now();
        let armed = arm_deferred_acquisition_retry(
            BackgroundAcquisitionCycleOutcome {
                deferred_scopes: 1,
                retry_after: Some(std::time::Duration::from_secs(20)),
                ..BackgroundAcquisitionCycleOutcome::default()
            },
            None,
            POLL,
        )
        .expect("a cooldown inside the poll interval arms a retry");
        assert_eq!(armed - now, std::time::Duration::from_secs(20));

        // A cooldown that has all but expired is raised to the floor, so the
        // loop can never spin.
        let floored = arm_deferred_acquisition_retry(
            BackgroundAcquisitionCycleOutcome {
                deferred_scopes: 1,
                retry_after: Some(std::time::Duration::from_millis(5)),
                ..BackgroundAcquisitionCycleOutcome::default()
            },
            None,
            POLL,
        )
        .expect("armed");
        assert_eq!(floored - now, DEFERRED_RETRY_MIN_DELAY);

        // Coalescing: an already-armed earlier deadline wins over a later one,
        // so repeated cycles keep exactly one pending timer.
        let coalesced = arm_deferred_acquisition_retry(
            BackgroundAcquisitionCycleOutcome {
                deferred_scopes: 1,
                retry_after: Some(std::time::Duration::from_secs(20)),
                ..BackgroundAcquisitionCycleOutcome::default()
            },
            Some(floored),
            POLL,
        )
        .expect("armed");
        assert_eq!(coalesced, floored);

        // A cycle that defers nothing leaves an armed deadline alone.
        assert_eq!(
            arm_deferred_acquisition_retry(
                BackgroundAcquisitionCycleOutcome::default(),
                Some(floored),
                POLL
            ),
            Some(floored)
        );
    }

    #[test]
    fn background_acquisition_submission_claims_are_atomic_and_route_scoped() {
        let cycle = BackgroundAcquisitionCycleCoordinator::default();
        let route = DownloadRouteKey {
            source_kind: Some(DownloadSourceKind::NzbUrl),
            indexer_id: Some("indexer-a".to_string()),
        };

        assert_eq!(
            cycle.claim_submission(route.clone(), "https://indexer.example/a"),
            SubmissionClaim::Granted
        );
        assert_eq!(
            cycle.claim_submission(route.clone(), "https://indexer.example/a"),
            SubmissionClaim::AlreadyAttempted
        );
        cycle.mark_submitted("https://indexer.example/a");
        assert_eq!(
            cycle.claim_submission(route.clone(), "https://indexer.example/a"),
            SubmissionClaim::AlreadySubmitted
        );
        cycle.mark_failed_route(route.clone());
        assert_eq!(
            cycle.claim_submission(route, "https://indexer.example/b"),
            SubmissionClaim::RouteUnavailable
        );
    }

    #[test]
    fn poisoned_background_acquisition_state_remains_recoverable() {
        let cycle = Arc::new(BackgroundAcquisitionCycleCoordinator::default());
        let poisoned = Arc::clone(&cycle);
        assert!(
            std::thread::spawn(move || {
                let _guard = poisoned.state.lock().expect("test lock");
                panic!("poison the test lock");
            })
            .join()
            .is_err()
        );

        assert_eq!(
            cycle.claim_submission(
                DownloadRouteKey {
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    indexer_id: Some("indexer-a".to_string()),
                },
                "https://indexer.example/a",
            ),
            SubmissionClaim::Granted
        );
    }

    #[test]
    fn background_acquisition_title_limit_is_four() {
        assert_eq!(BACKGROUND_ACQUISITION_TITLE_LIMIT, 4);
    }
}
