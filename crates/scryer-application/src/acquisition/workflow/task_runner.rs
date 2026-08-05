/// Scheduler value hint for a hot convergence target (recent air/release/add):
/// high value so the scope converges promptly and keeps admitting
/// even while the account's API quota is under pressure. Equals the neutral
/// baseline, so hot work is never shed by the low-value pressure gate.
const BACKGROUND_HOT_TARGET_VALUE: f64 = 1.0;

/// Scheduler value hint for a cold convergence target (long-tail / upgrades):
/// low value so the quota-pressure gate drains it first,
/// yielding shared account quota to RSS polls and hot acquisition. Above the
/// absolute `LOW_VALUE_BACKGROUND_THRESHOLD` floor, so a cold scope still
/// admits when quota is healthy — it only defers once quota tightens.
const BACKGROUND_COLD_TARGET_VALUE: f64 = 0.25;

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
/// Run one convergence cycle: recover failed downloads, derive
/// the target set, rotate the cursor over it, and search each selected scope's
/// uncovered indexers. Plan-112 admission inside the search is the only rate
/// authority; the cycle merely bounds evaluation cost and pre-skips scopes the
/// scheduler could not serve right now.
async fn run_convergence_cycle(app: &AppUseCase) {
    let blocked_facets = blocked_acquisition_facets_after_quiet_wait(app).await;
    run_convergence_cycle_with_blocked_facets(app, &blocked_facets).await;
}

pub(crate) async fn run_convergence_cycle_with_blocked_facets(
    app: &AppUseCase,
    blocked_facets: &[MediaFacet],
) {
    prune_standby_candidates(app).await;

    // Failed downloads first: each failure re-opens its scope (coverage pruned,
    // state reset), so this cycle's derivation already sees it as searchable.
    let dl_snapshot = DownloadClientSnapshot::fetch(app).await;
    check_grabbed_for_failures(app, &dl_snapshot).await;

    let now = Utc::now();
    let settings = match app.convergence_settings().await {
        Ok(settings) => settings,
        Err(err) => {
            warn!(error = %err, "failed to load convergence settings, skipping cycle");
            return;
        }
    };

    let mut targets = match app.derive_acquisition_targets(&now).await {
        Ok(targets) => targets,
        Err(err) => {
            warn!(error = %err, "failed to derive acquisition targets");
            return;
        }
    };
    if !blocked_facets.is_empty() {
        targets.retain(|target| !blocked_facets.contains(&target.facet));
    }
    if targets.is_empty() {
        return;
    }

    if !has_enabled_download_clients(app).await {
        warn!(
            target_count = targets.len(),
            "background acquisition: no enabled download clients configured, skipping cycle"
        );
        return;
    }

    let resume = app.convergence_cursor_resume_position().await;
    let max_scopes = settings.long_tail_backfill_max_scopes_per_cycle.max(1) as usize;
    let selection = crate::acquisition::targets::select_convergence_batch(
        &targets,
        resume.as_deref(),
        max_scopes,
    );
    app.store_convergence_cursor_resume_position(selection.resume_after.as_deref())
        .await;
    if selection.indices.is_empty() {
        return;
    }

    debug!(
        target_count = targets.len(),
        selected_count = selection.indices.len(),
        "convergence cycle: evaluating scopes"
    );

    // Scheduler availability, resolved once per cycle for the pre-skip.
    let availability = app.scheduler_availability().await;
    let indexer_hosts = app.indexer_scheduler_host_keys().await;

    // Track URLs already submitted this cycle to avoid sending the same NZB
    // multiple times (e.g. a season pack matching several episode scopes).
    let mut grabbed_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Track (title_id, season_num) for which a season pack search was attempted this cycle.
    let mut season_pack_attempted: std::collections::HashSet<(String, u32)> =
        std::collections::HashSet::new();
    // Track (title_id, season_num) for which a season pack was successfully grabbed this cycle.
    let mut season_pack_grabbed: std::collections::HashSet<(String, u32)> =
        std::collections::HashSet::new();
    // Track seasons where a viable season-pack candidate was found but not
    // definitively failed. This avoids spending per-episode searches behind a
    // pack that is pending delay or waiting on transient download-client state.
    let mut season_pack_viable: std::collections::HashSet<(String, u32)> =
        std::collections::HashSet::new();
    let mut recent_failed_season_packs_by_title: std::collections::HashMap<String, HashSet<u32>> =
        std::collections::HashMap::new();

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

    let mut processed_in_slice = 0usize;
    for index in selection.indices {
        let target = &targets[index];
        if let Err(err) = process_single_target(
            app,
            target,
            &now,
            &availability,
            &indexer_hosts,
            &mut grabbed_urls,
            &mut season_pack_attempted,
            &mut season_pack_grabbed,
            &mut season_pack_viable,
            &mut recent_failed_season_packs_by_title,
            &season_due_counts,
            &dl_snapshot,
        )
        .await
        {
            warn!(
                scope_key = target.scope_key.as_str(),
                title_id = target.title_id.as_str(),
                error = %err,
                "failed to process acquisition target"
            );
        }
        processed_in_slice += 1;
        if processed_in_slice.is_multiple_of(ACQUISITION_SLICE_YIELD_INTERVAL) {
            tokio::task::yield_now().await;
        }
    }
}
fn submission_blocks_search_for_wanted_item(
    submission: &DownloadSubmission,
    item: &AcquisitionScopeState,
    episode_collection_id: Option<&str>,
    dl_snapshot: &DownloadClientSnapshot,
) -> bool {
    if !submission_blocks_wanted_item(submission, item, episode_collection_id) {
        return false;
    }

    if submission_is_active(submission, dl_snapshot) {
        return true;
    }

    submission_is_completed(submission, dl_snapshot) && item.current_score.is_none()
}

impl AppUseCase {
    #[cfg(test)]
    pub(crate) async fn run_convergence_cycle_once(&self) {
        run_convergence_cycle(self).await;
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "target processing coordinates shared acquisition state across a single cycle"
)]
async fn process_single_target(
    app: &AppUseCase,
    target: &crate::acquisition::targets::AcquisitionTarget,
    now: &DateTime<Utc>,
    availability: &crate::acquisition::convergence::SchedulerAvailability,
    indexer_hosts: &std::collections::HashMap<String, String>,
    grabbed_urls: &mut std::collections::HashSet<String>,
    season_pack_attempted: &mut std::collections::HashSet<(String, u32)>,
    season_pack_grabbed: &mut std::collections::HashSet<(String, u32)>,
    season_pack_viable: &mut std::collections::HashSet<(String, u32)>,
    recent_failed_season_packs_by_title: &mut std::collections::HashMap<String, HashSet<u32>>,
    season_due_counts: &std::collections::HashMap<(String, u32), usize>,
    dl_snapshot: &DownloadClientSnapshot,
) -> AppResult<()> {
    // Load the title to get search context
    let title = match app
        .services
        .catalog
        .titles
        .get_by_id(&target.title_id)
        .await?
    {
        Some(t) => t,
        None => {
            warn!(
                title_id = target.title_id.as_str(),
                "acquisition target references missing title"
            );
            return Ok(());
        }
    };

    // Load episode data for episode-scoped targets
    let episode = if target.media_type == "episode" {
        if let Some(ep_id) = target.episode_id.as_deref() {
            match app.services.catalog.shows.get_episode_by_id(ep_id).await {
                Ok(ep) => ep,
                Err(err) => {
                    warn!(episode_id = ep_id, error = %err, "failed to load episode for acquisition target");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };
    let effective_collection_id = target
        .collection_id
        .clone()
        .or_else(|| episode.as_ref().and_then(|ep| ep.collection_id.clone()));

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
            &title,
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
    let submissions = app
        .services
        .workflow
        .download_submissions
        .list_for_title(&item.title_id)
        .await
        .unwrap_or_default();
    let episode_collection_id = episode_collection_id_for_wanted_item(item, episode.as_ref());

    let has_blocking_download_submission = submissions.iter().any(|submission| {
        submission_blocks_search_for_wanted_item(
            submission,
            item,
            episode_collection_id.as_deref(),
            dl_snapshot,
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

    let search_title = app
        .release_search_title_for_wanted_item(&title, item, episode.as_ref())
        .await;

    let subject = app
        .resolve_release_search_subject_for_wanted_item(
            &title,
            &search_title,
            item,
            episode.as_ref(),
        )
        .await;
    let search_season = subject.season;

    // Convergence gate: a converged scope rides RSS; an
    // unconverged one is searched against exactly its uncovered indexer subset.
    // Resolved once here and reused for the restricted search below.
    let Some(convergence) = app
        .resolve_scope_convergence(&search_title, &subject)
        .await
    else {
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
    // returns to it once the scheduler frees capacity.
    if !uncovered.iter().any(|indexer_id| {
        availability.indexer_available(
            indexer_hosts.get(indexer_id).map(String::as_str),
            indexer_id,
        )
    }) {
        debug!(
            title_id = title.id.as_str(),
            scope_key = target.scope_key.as_str(),
            uncovered_count = uncovered.len(),
            "background acquisition: uncovered indexers unavailable this cycle, deferring scope"
        );
        return Ok(());
    }
    let uncovered: std::collections::HashSet<String> = uncovered.into_iter().collect();

    // The scope is about to be searched — its state row exists from here on,
    // so release decisions and grabs have their anchor.
    item.id = app
        .services
        .workflow
        .acquisition_scope_states
        .ensure_acquisition_scope_state(item)
        .await?;

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

        if due_count >= 2 && !season_pack_attempted.contains(&season_key) {
            season_pack_attempted.insert(season_key.clone());

            let recent_failed_seasons =
                if let Some(cached) = recent_failed_season_packs_by_title.get(&title.id) {
                    cached.clone()
                } else {
                    let loaded =
                        load_recent_failed_season_pack_seasons_for_title(app, &title.id, now).await;
                    recent_failed_season_packs_by_title.insert(title.id.clone(), loaded.clone());
                    loaded
                };

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
                let pack_results = if pack_uncovered
                    .as_ref()
                    .is_some_and(|uncovered| uncovered.is_empty())
                {
                    debug!(
                        title_id = title.id.as_str(),
                        season = season_num,
                        "season pack scope converged, riding RSS"
                    );
                    Vec::new()
                } else {
                    match app
                        .search_and_evaluate_subject_restricted(
                            &search_title,
                            &pack_subject,
                            "background_acquisition_season_pack",
                            SearchMode::Auto,
                            tokio_util::sync::CancellationToken::new(),
                            pack_uncovered
                                .map(|uncovered| uncovered.into_iter().collect()),
                            // The pack shares the target's recency lane (§D3).
                            Some(if target.is_hot {
                                BACKGROUND_HOT_TARGET_VALUE
                            } else {
                                BACKGROUND_COLD_TARGET_VALUE
                            }),
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

                for candidate in pack_results
                    .iter()
                    .filter(|candidate| candidate_is_season_pack_for_season(candidate, season_num))
                {
                    let decision_code = annotated_auto_decision_code(candidate);
                    record_release_decision(app, item, &title, candidate, decision_code, now).await;
                    if matches!(
                        decision_code,
                        ReleaseAutoDecisionCode::PendingDelay
                            | ReleaseAutoDecisionCode::AlreadyActive
                    ) {
                        season_pack_viable.insert(season_key.clone());
                    }
                }

                if let Some(best_pack) = pack_results.iter().find(|candidate| {
                    candidate_is_season_pack_for_season(candidate, season_num)
                        && candidate.auto_eligible == Some(true)
                }) {
                    // ── Season pack upgrade guard ───────────────────────────────
                    // Check whether grabbing this pack benefits at least 1 episode.
                    // If every episode already has a file with an equal or better
                    // score, the pack is pure waste — skip it and fall through to
                    // individual episode searches (which will also be skipped by
                    // the per-episode cutoff/upgrade checks).
                    //
                    // TODO: make this user-configurable via quality profile. Some
                    // users may want a stricter threshold (e.g. "only grab season
                    // packs if ≥50% of episodes benefit") to reduce download
                    // bandwidth, rather than the current "any 1 episode" policy.
                    let pack_dominated = if !season_episodes.is_empty() {
                        let pack_score = best_pack
                            .quality_profile_decision
                            .as_ref()
                            .map(|d| d.preference_score)
                            .unwrap_or(0);

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

                        let episode_file_scores: std::collections::HashMap<String, i32> =
                            existing_files
                                .iter()
                                .filter_map(|f| {
                                    f.episode_id
                                        .as_ref()
                                        .zip(f.acquisition_score)
                                        .map(|(eid, score)| (eid.clone(), score))
                                })
                                .collect();

                        // Pack is dominated (no benefit) when every episode in the
                        // season already has a file with score >= pack_score.
                        !season_episodes.iter().any(|ep| {
                            episode_file_scores
                                .get(&ep.id)
                                .map(|&existing| pack_score > existing)
                                .unwrap_or(true) // no file → episode benefits
                        })
                    } else {
                        false // can't determine episodes → allow grab
                    };

                    if pack_dominated {
                        info!(
                            title = title.name.as_str(),
                            season = season_num,
                            release = best_pack.title.as_str(),
                            "season pack skipped: all episodes already have equal or better files"
                        );
                        // Don't grab — fall through to individual episode search
                    } else {
                        // ── End season pack upgrade guard ────────────────────────────

                        let pack_url = best_pack
                            .download_url
                            .clone()
                            .or_else(|| best_pack.link.clone());
                        let url_str = pack_url.as_deref().unwrap_or("").to_string();

                        if !url_str.is_empty() && grabbed_urls.insert(url_str.clone()) {
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
                            let pack_title_norm =
                                normalize_release_attempt_title(pack_title.as_deref());
                            let pack_password =
                                normalize_release_password(best_pack.password_hint.as_deref());
                            let request_signature = normalize_release_selection_signature(
                                pack_url.as_deref(),
                                pack_title.as_deref(),
                                best_pack.source_kind,
                            );
                            let info_hash_hint = best_pack
                                .extra
                                .get("info_hash")
                                .and_then(|value| value.as_str())
                                .map(str::to_string);
                            let download_id = crate::download_identity::new_download_id();
                            let submission_identity = DownloadSubmissionIdentity {
                                download_id: Some(download_id.clone()),
                            };

                            let grab_result = app
                                .services
                                .integrations
                                .download_client
                                .submit_download(&DownloadClientAddRequest {
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
                                    is_recent,
                                    season_pack: Some(true),
                                })
                                .await;

                            match grab_result {
                                Ok(grab) => {
                                    let download_job_id = grab.job_id.clone();
                                    let facet_label = serde_json::to_string(&title.facet)
                                        .unwrap_or_else(|_| "\"other\"".to_string())
                                        .trim_matches('"')
                                        .to_string();
                                    metrics::counter!("scryer_grabs_total", "indexer" => best_pack.source.clone(), "facet" => facet_label).increment(1);
                                    let accepted_identity =
                                        crate::download_identity::accepted_download_submission_identity(
                                            crate::download_identity::AcceptedDownloadIdentityInput {
                                                initial_download_id: submission_identity
                                                    .download_id
                                                    .as_deref(),
                                                source_kind: best_pack.source_kind,
                                                source_hint: pack_url.as_deref(),
                                                info_hash_hint: info_hash_hint.as_deref(),
                                                client_type: Some(grab.client_type.as_str()),
                                                client_item_id: Some(grab.job_id.as_str()),
                                                accepted_info_hash: grab.info_hash.as_deref(),
                                            },
                                    );
                                    season_pack_grabbed.insert(season_key.clone());
                                    season_pack_viable.insert(season_key.clone());
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
                                    let facet_str = serde_json::to_string(&title.facet)
                                        .unwrap_or_else(|_| "\"other\"".to_string());
                                    let submission_scope =
                                        collection_download_submission_scope_for_wanted_item(
                                            item,
                                            episode.as_ref(),
                                        );
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
                                            current_score: item.current_score,
                                            grabbed_release: grabbed_json,
                                            last_search_at: Some(now.to_rfc3339()),
                                            download_submission: DownloadSubmission {
                                                title_id: title.id.clone(),
                                                purpose: crate::DownloadSubmissionPurpose::Standard,
                                                facet: facet_str.trim_matches('"').to_string(),
                                                download_client_id: grab.client_id.clone(),
                                                download_client_type: grab.client_type.clone(),
                                                download_client_item_id: grab.job_id.clone(),
                                                source_hint: None,
                                                source_provider_id: best_pack.indexer_id.clone(),
                                                source_provider_name: Some(best_pack.source.clone()),
                                                source_kind: None,
                                                source_title: Some(best_pack.title.clone()),
                                                request_signature: request_signature.clone(),
                                                scope: submission_scope,
                                            },
                                            download_submission_identity: Some(accepted_identity),
                                            grabbed_pending_release_id: None,
                                            grabbed_at: Some(now.to_rfc3339()),
                                        })
                                        .await?;
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
                                            &title,
                                            DomainEventPayload::ReleaseGrabbed(
                                                ReleaseGrabbedEventData {
                                                    title: title_context_snapshot(&title),
                                                    source_title: Some(best_pack.title.clone()),
                                                    source_hint: Some(best_pack.source.clone()),
                                                    download_id: Some(download_job_id),
                                                    episode_ids: item
                                                        .episode_id
                                                        .iter()
                                                        .cloned()
                                                        .collect(),
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
                                }
                                Err(err) => {
                                    let submit_unavailable =
                                        is_download_submit_unavailable_error(&err);
                                    if submit_unavailable {
                                        season_pack_viable.insert(season_key.clone());
                                    } else {
                                        season_pack_viable.remove(&season_key);
                                    }
                                    warn!(
                                        title = title.name.as_str(),
                                        season = season_num,
                                        error = %err,
                                        fallback_to_episode_search = !submit_unavailable,
                                        "season pack grab failed"
                                    );
                                    let _ = app
                                        .services
                                        .workflow
                                        .release_attempts
                                        .record_release_attempt(
                                            Some(title.id.clone()),
                                            pack_hint,
                                            pack_title_norm,
                                            if submit_unavailable {
                                                ReleaseDownloadAttemptOutcome::Pending
                                            } else {
                                                ReleaseDownloadAttemptOutcome::Failed
                                            },
                                            Some(err.to_string()),
                                            pack_password,
                                        )
                                        .await;
                                }
                            }
                        }
                    } // close else (pack not dominated)
                }
            }
        }

        // If a season pack was grabbed or remains viable this cycle (by this
        // item or an earlier item for the same season), skip the individual
        // episode search unless the pack submission definitively failed.
        if season_pack_grabbed.contains(&season_key) {
            return Ok(());
        }
        if season_pack_viable.contains(&season_key) {
            info!(
                title = title.name.as_str(),
                season = season_num,
                "season pack candidate found; skipping individual episode search for this cycle"
            );
            return Ok(());
        }
    }
    // ── End season pack priority ──────────────────────────────────────────────
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

    // Search and score releases against the uncovered indexer subset only —
    // covered indexers hold no new information for this scope (§D2).
    let results = match app
        .search_and_evaluate_subject_restricted(
            &search_title,
            &subject,
            "background_acquisition",
            SearchMode::Auto,
            tokio_util::sync::CancellationToken::new(),
            Some(uncovered),
            Some(if target.is_hot {
                BACKGROUND_HOT_TARGET_VALUE
            } else {
                BACKGROUND_COLD_TARGET_VALUE
            }),
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

    // Cooldown state, not cadence: the upgrade policy and failed-grab handling
    // read when this scope last actually searched.
    let _ = app
        .services
        .workflow
        .acquisition_scope_states
        .record_acquisition_scope_search_attempt(&item.id, &now.to_rfc3339())
        .await;

    app.emit_acquisition_search_completed_event(None, &title, results.len() as i64)
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

    // Load DB-level blocklist (covers post-import failures like fake/non-video files,
    // in addition to the download-client snapshot checked below).
    let db_blocklist: std::collections::HashSet<String> = app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(&title.id, 200)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| e.source_title)
        .map(|t| t.to_ascii_lowercase())
        .collect();
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
    let analyzed_cutoff_quality =
        crate::acquisition::decision_helpers::analyzed_cutoff_quality_for_scope(
            &existing_files,
            subject.submission_scope.episode_id(),
            subject.submission_scope.series_movie_link_id(),
        );

    let upgrade_context = app
        .resolve_upgrade_context_for_title_with_category_and_quality(
            &search_title,
            item.grabbed_release.as_deref(),
            Some(subject.category.as_str()),
            analyzed_cutoff_quality,
        )
        .await;
    let profile = &upgrade_context.profile;

    // Cutoff tier check — skip upgrades if the existing file meets the cutoff quality.
    // This is independent of any candidate and can short-circuit before the loop.
    if upgrade_context.cutoff_reached {
        tracing::debug!(
            title_id = title.id.as_str(),
            cutoff = profile.criteria.cutoff_tier.as_deref().unwrap_or(""),
            "cutoff quality reached, skipping upgrade"
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
    // Track source kinds where ALL download clients failed.  Avoids hammering
    // dead clients with more candidates of the same protocol.
    let mut failed_source_kinds: Vec<DownloadSourceKind> = Vec::new();
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
                effective_auto_decision_code(candidate, &failed_source_kinds, &db_blocklist),
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
            &title,
            candidate,
            candidate_score,
            serialize_decision_explanation(candidate),
        )
        .await;
    }
    let mut grab_attempts: usize = 0;

    for (candidate_index, candidate) in results.iter().enumerate() {
        let is_allowed = candidate
            .quality_profile_decision
            .as_ref()
            .map(|d| d.allowed)
            .unwrap_or(false);
        let decision_code = if is_allowed {
            effective_auto_decision_code(candidate, &failed_source_kinds, &db_blocklist)
        } else {
            ReleaseAutoDecisionCode::QualityBlocked
        };
        if !is_allowed {
            record_release_decision(app, item, &title, candidate, decision_code, now).await;
            app.emit_acquisition_candidate_rejected_event(
                None,
                &title,
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

        record_release_decision(app, item, &title, candidate, decision_code, now).await;

        if !decision_code.is_eligible() {
            app.emit_acquisition_candidate_rejected_event(
                None,
                &title,
                candidate.title.clone(),
                decision_code.as_str().to_string(),
            )
            .await;
            if matches!(
                decision_code,
                ReleaseAutoDecisionCode::NegativeScore
                    | ReleaseAutoDecisionCode::UpgradeRejected
                    | ReleaseAutoDecisionCode::CutoffReached
            ) {
                break;
            }
            if matches!(decision_code, ReleaseAutoDecisionCode::AmbiguousIdentity)
                && !parked_ambiguous_identity
            {
                parked_ambiguous_identity = true;
                app.park_pending_release_for_review(
                    item,
                    &title,
                    candidate,
                    candidate_score,
                    serialize_decision_explanation(candidate),
                )
                .await;
                // Keep walking the ranked list: a lower-scored candidate that
                // does present a disambiguator is still grabbable this cycle.
                continue;
            }
            if matches!(decision_code, ReleaseAutoDecisionCode::PendingDelay) {
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

                app.insert_pending_release(
                    item,
                    &title,
                    &candidate.title,
                    candidate
                        .download_url
                        .as_deref()
                        .or(candidate.link.as_deref()),
                    candidate.source_kind,
                    candidate.size_bytes,
                    candidate_score,
                    scoring_json,
                    Some(candidate.source.as_str()),
                    candidate.guid.as_deref(),
                    crate::delay_profile::resolve_delay_decision(
                        &delay_profiles,
                        &search_title.tags,
                        &search_title.facet,
                        candidate.source_kind,
                        candidate
                            .published_at
                            .as_deref()
                            .and_then(crate::quality_profile::parse_published_at),
                        candidate_score,
                        now,
                    )
                    .map(|delay| delay.effective_delay_minutes)
                    .unwrap_or_default(),
                    candidate.password_hint.as_deref(),
                    candidate.published_at.as_deref(),
                    candidate
                        .extra
                        .get("info_hash")
                        .and_then(|value| value.as_str()),
                )
                .await;
                return Ok(());
            }
            continue;
        }

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
        let source_hint = candidate
            .download_url
            .clone()
            .or_else(|| candidate.link.clone());

        // Deduplicate submit attempts without inventing grabbed state. Covered
        // wanted items are marked grabbed only by commit_successful_grab.
        if let Some(url) = source_hint.as_deref() {
            if grabbed_urls.contains(url) {
                info!(
                    title = title.name.as_str(),
                    release = candidate.title.as_str(),
                    "skipping duplicate release already submitted this cycle"
                );
                continue;
            }
            grabbed_urls.insert(url.to_string());
        }

        let source_title = Some(candidate.title.clone());
        let source_hint_for_attempt = normalize_release_attempt_hint(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_title(source_title.as_deref());
        let source_password = normalize_release_password(candidate.password_hint.as_deref());
        let request_signature = normalize_release_selection_signature(
            source_hint.as_deref(),
            source_title.as_deref(),
            candidate.source_kind,
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
            decision = decision_code.as_str(),
            attempt = grab_attempts,
            "auto-grabbing release"
        );

        let info_hash_hint = candidate
            .extra
            .get("info_hash")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let download_id = crate::download_identity::new_download_id();
        let submission_identity = DownloadSubmissionIdentity {
            download_id: Some(download_id.clone()),
        };

        let grab_result = app
            .services
            .integrations
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: title.clone(),
                search_facet: (target.media_type == "series_movie")
                    .then_some(MediaFacet::Movie),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                download_id: Some(download_id),
                source_hint: source_hint.clone(),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: candidate.source_kind,
                source_title: source_title.clone(),
                source_password: source_password.clone(),
                category: Some(download_cat.clone()),
                queue_priority: None,
                download_directory: None,
                release_title: Some(candidate.title.clone()),
                indexer_name: Some(candidate.source.clone()),
                indexer_id: candidate.indexer_id.clone(),
                info_hash_hint: info_hash_hint.clone(),
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent,
                season_pack: Some(false),
            })
            .await;

        match grab_result {
            Ok(grab) => {
                // ── Success ─────────────────────────────────────────────────
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => candidate.source.clone(), "facet" => facet_label).increment(1);
                }
                let accepted_identity =
                    crate::download_identity::accepted_download_submission_identity(
                        crate::download_identity::AcceptedDownloadIdentityInput {
                            initial_download_id: submission_identity.download_id.as_deref(),
                            source_kind: candidate.source_kind,
                            source_hint: source_hint.as_deref(),
                            info_hash_hint: info_hash_hint.as_deref(),
                            client_type: Some(grab.client_type.as_str()),
                            client_item_id: Some(grab.job_id.as_str()),
                            accepted_info_hash: grab.info_hash.as_deref(),
                        },
                    );

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
                let facet_str =
                    serde_json::to_string(&title.facet).unwrap_or_else(|_| "\"other\"".to_string());
                let grabbed_json = serde_json::json!({
                    "title": candidate.title,
                    "score": candidate_score,
                    "grabbed_at": now.to_rfc3339(),
                    "source_provider": candidate.source.clone(),
                })
                .to_string();
                let download_job_id = grab.job_id.clone();
                let submission_scope = if let Some(parsed) =
                    candidate.parsed_release_metadata.as_ref()
                {
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
                    .submission_scope_or(
                        &direct_download_submission_scope_for_wanted_item(item, episode.as_ref()),
                    )
                } else {
                    direct_download_submission_scope_for_wanted_item(item, episode.as_ref())
                };
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
                        current_score: item.current_score,
                        grabbed_release: grabbed_json,
                        last_search_at: Some(now.to_rfc3339()),
                        download_submission: DownloadSubmission {
                            title_id: title.id.clone(),
                            purpose: crate::DownloadSubmissionPurpose::Standard,
                            facet: facet_str.trim_matches('"').to_string(),
                            download_client_id: grab.client_id.clone(),
                            download_client_type: grab.client_type.clone(),
                            download_client_item_id: grab.job_id.clone(),
                            source_hint: None,
                            source_provider_id: candidate.indexer_id.clone(),
                            source_provider_name: Some(candidate.source.clone()),
                            source_kind: None,
                            source_title: source_title.clone(),
                            request_signature: request_signature.clone(),
                            scope: submission_scope,
                        },
                        download_submission_identity: Some(accepted_identity),
                        grabbed_pending_release_id: None,
                        grabbed_at: Some(now.to_rfc3339()),
                    })
                    .await?;

                persist_standby_candidates(
                    app,
                    item,
                    &title,
                    &results,
                    candidate_index + 1,
                    now,
                    &failed_source_kinds,
                    &db_blocklist,
                )
                .await;

                let _ = app
                    .append_domain_event(new_title_domain_event(
                        None,
                        &title,
                        DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                            title: title_context_snapshot(&title),
                            source_title: Some(candidate.title.clone()),
                            source_hint: Some(candidate.source.clone()),
                            download_id: Some(download_job_id),
                            episode_ids: item.episode_id.iter().cloned().collect(),
                        }),
                    ))
                    .await;

                return Ok(());
            }
            Err(err) => {
                if matches!(err, AppError::DownloadSubmitAmbiguous(_)) {
                    warn!(
                        title = title.name.as_str(),
                        release = candidate.title.as_str(),
                        attempt = grab_attempts,
                        error = %err,
                        "download submission result is ambiguous; re-opening scope without blocklisting or failover"
                    );

                    return Ok(());
                }

                // ── Grab failed — try next candidate ────────────────────────
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
                let submit_unavailable = is_download_submit_unavailable_error(&err);

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
                        .download_url
                        .clone()
                        .or_else(|| candidate.link.clone())
                        .unwrap_or_else(|| candidate.source.clone());
                    let quality = candidate
                        .parsed_release_metadata
                        .as_ref()
                        .and_then(|parsed| parsed.quality.clone())
                        .or_else(|| release_quality_hint(Some(candidate.title.as_str())));

                    record_failed_release_outcome(
                        app,
                        Some(title.id.as_str()),
                        &attribution,
                        Some(candidate.title.clone()),
                        Some(candidate_source_hint),
                        None,
                        None,
                        None,
                        None,
                        quality,
                        Some(failure_reason),
                        None,
                        source_password.clone(),
                    )
                    .await;
                }

                // If download-client submit is unavailable for this source kind,
                // skip remaining candidates with the same protocol this run.
                if submit_unavailable && let Some(sk) = candidate.source_kind {
                    if !failed_source_kinds.contains(&sk) {
                        failed_source_kinds.push(sk);
                    }
                    info!(
                        source_kind = ?sk,
                        "download client submit unavailable for source kind, skipping remaining candidates with same protocol"
                    );
                }

                // Add URL to exclusion set so we don't re-select this exact
                // release if the same URL appears from a different indexer.
                if let Some(url) = source_hint.as_deref() {
                    grabbed_urls.insert(url.to_string());
                }

                // CONTINUE — try the next candidate
            }
        }
    }
    // ── End candidate fallthrough loop ───────────────────────────────────────

    // All candidates exhausted without a successful grab.
    if grab_attempts > 0 {
        warn!(
            title = title.name.as_str(),
            attempts = grab_attempts,
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

    // No grab this cycle: the scope's coverage now reflects every indexer that
    // answered, so the cursor will not re-search them — new postings arrive via
    // RSS, and any still-uncovered indexers are retried on a later rotation.
    Ok(())
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
        Utc::now() + chrono::Duration::hours(24),
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

    let mut poll_interval = new_skip_interval(std::time::Duration::from_secs(
        settings.poll_interval_seconds.max(1) as u64,
    ));
    let mut registry_refresh_interval = tokio::time::interval(std::time::Duration::from_hours(24));
    let mut health_check_interval = tokio::time::interval(std::time::Duration::from_hours(6));
    let mut staged_nzb_prune_interval = tokio::time::interval(std::time::Duration::from_hours(1));
    let mut housekeeping_interval = tokio::time::interval(std::time::Duration::from_hours(24));
    let mut prowlarr_sync_interval = tokio::time::interval(std::time::Duration::from_mins(5));
    let mut direct_indexer_caps_interval =
        tokio::time::interval(std::time::Duration::from_hours(24));
    let mut rss_sync_interval = tokio::time::interval(std::time::Duration::from_mins(1));
    let mut pending_release_interval = tokio::time::interval(std::time::Duration::from_mins(1));

    // Consume immediate intervals.
    poll_interval.tick().await;
    registry_refresh_interval.tick().await;
    health_check_interval.tick().await;
    staged_nzb_prune_interval.tick().await;
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
        match tokio::spawn(fut).await {
            Ok(()) => {}
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

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!("background acquisition poller shutting down");
                break;
            }
            _ = wake.notified() => {
                let app = app.clone();
                run_task("convergence_cycle", async move {
                    run_convergence_cycle(&app).await;
                }).await;
            }
            _ = poll_interval.tick() => {
                let app = app.clone();
                run_task("convergence_cycle", async move {
                    run_convergence_cycle(&app).await;
                }).await;
            }
            _ = registry_refresh_interval.tick() => {
                let app = app.clone();
                run_task("registry_refresh", async move {
                    app.set_job_next_run_at(
                        JobKey::PluginRegistryRefresh,
                        Utc::now() + chrono::Duration::hours(24),
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

    #[test]
    fn non_metadata_scheduled_job_intervals_remain_unchanged() {
        assert_eq!(JobKey::RssSync.interval_seconds(), Some(15 * 60));
        assert_eq!(
            JobKey::PluginRegistryRefresh.interval_seconds(),
            Some(24 * 60 * 60)
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

    fn wanted_episode_item(title_id: &str, title_name: &str, episode_number: u32) -> AcquisitionScopeState {
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
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn episode_submission(title_id: &str, episode_id: &str, job_id: &str) -> DownloadSubmission {
        DownloadSubmission {
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
            completed_client_ids: Default::default(),
            completed_raw_item_id_counts: Default::default(),
            failed_by_download_id: Default::default(),
            queue_listing_failed: false,
            history_listing_failed: false,
        };
        if completed {
            snapshot.completed_client_ids.insert(key);
        } else {
            snapshot.active_client_ids.insert(key);
        }
        snapshot
    }

    #[test]
    fn completed_submission_blocks_initial_wanted_search() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-baseline");
        let snapshot = snapshot_with_job("job-baseline", true);

        assert!(submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn completed_submission_does_not_block_upgrade_search() {
        let mut item = wanted_episode_item("title-bluey", "Bluey", 1);
        item.current_score = Some(2_950);
        item.grabbed_release = Some("Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb".to_string());
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-baseline");
        let snapshot = snapshot_with_job("job-baseline", true);

        assert!(!submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
        ));
    }

    #[test]
    fn active_submission_still_blocks_upgrade_search() {
        let mut item = wanted_episode_item("title-bluey", "Bluey", 1);
        item.current_score = Some(2_950);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-upgrade");
        let snapshot = snapshot_with_job("job-upgrade", false);

        assert!(submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
        ));
    }

}
