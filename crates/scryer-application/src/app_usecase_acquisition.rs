use super::*;
use crate::acquisition_policy::{compute_search_schedule, evaluate_upgrade, AcquisitionThresholds};
use chrono::{DateTime, Utc};
use tracing::{info, warn};

impl AppUseCase {
    /// Sync the wanted_items table with current monitored state.
    /// Creates entries for monitored media without files, removes stale entries.
    pub(crate) async fn sync_wanted_state(&self) -> AppResult<()> {
        let titles = self.services.titles.list(None, None).await?;
        let now = Utc::now();

        for title in &titles {
            if !title.monitored {
                // Clean up wanted items for unmonitored titles
                if let Err(err) = self.services.wanted_items.delete_wanted_items_for_title(&title.id).await {
                    warn!(title_id = title.id.as_str(), error = %err, "failed to clean wanted items for unmonitored title");
                }
                continue;
            }

            if let Some(handler) = self.facet_registry.get(&title.facet) {
                if handler.has_episodes() {
                    self.sync_wanted_series(title, &now).await;
                } else {
                    self.sync_wanted_movie(title, &now).await;
                }
            }
        }

        Ok(())
    }

    async fn sync_wanted_movie(&self, title: &Title, now: &DateTime<Utc>) {
        self.sync_wanted_movie_inner(title, now, false).await;
    }

    pub(crate) async fn sync_wanted_movie_inner(&self, title: &Title, now: &DateTime<Utc>, immediate: bool) {
        // Check if movie already has a media file
        let has_file = match self.services.media_files.list_media_files_for_title(&title.id).await {
            Ok(files) => !files.is_empty(),
            Err(_) => false,
        };

        if has_file {
            return;
        }

        // Minimum availability gate: skip search if the movie hasn't reached the
        // configured availability threshold yet.
        let availability = title.min_availability.as_deref().unwrap_or("announced");
        match availability {
            "in_cinemas" => {
                if let Some(ref first_aired) = title.first_aired {
                    if let Ok(date) = chrono::NaiveDate::parse_from_str(first_aired, "%Y-%m-%d") {
                        if date > now.date_naive() {
                            info!(
                                title_id = title.id.as_str(),
                                min_availability = availability,
                                first_aired = first_aired.as_str(),
                                "skipping movie: not yet in cinemas"
                            );
                            return;
                        }
                    }
                }
            }
            "released" => {
                let is_released = if let Some(ref digital) = title.digital_release_date {
                    chrono::NaiveDate::parse_from_str(digital, "%Y-%m-%d")
                        .map(|d| d <= now.date_naive())
                        .unwrap_or(false)
                } else if let Some(ref first_aired) = title.first_aired {
                    // Fallback: first_aired + 90 days
                    chrono::NaiveDate::parse_from_str(first_aired, "%Y-%m-%d")
                        .map(|d| d + chrono::Duration::days(90) <= now.date_naive())
                        .unwrap_or(false)
                } else {
                    false
                };
                if !is_released {
                    info!(
                        title_id = title.id.as_str(),
                        min_availability = availability,
                        digital_release_date = title.digital_release_date.as_deref().unwrap_or("none"),
                        first_aired = title.first_aired.as_deref().unwrap_or("none"),
                        "skipping movie: not yet released"
                    );
                    return;
                }
            }
            // "announced" or anything else: always search
            _ => {}
        }

        // Determine baseline date for search scheduling
        let baseline_date = title.first_aired.clone();

        let schedule = compute_search_schedule(
            "movie",
            baseline_date.as_deref(),
            "primary",
            now,
        );

        // When immediate=true (called from add_title), set next_search_at to now
        // so the background poller picks it up on the next 60-second tick.
        let next_search_at = if immediate {
            now.to_rfc3339()
        } else {
            schedule.next_search_at
        };

        let item = WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: None,
            episode_id: None,
            media_type: "movie".to_string(),
            search_phase: schedule.search_phase,
            next_search_at: Some(next_search_at),
            last_search_at: None,
            search_count: 0,
            baseline_date,
            status: "wanted".to_string(),
            grabbed_release: None,
            current_score: None,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
        };

        match self.services.wanted_items.upsert_wanted_item(&item).await {
            Ok(_) => {
                info!(
                    title_id = title.id.as_str(),
                    title_name = title.name.as_str(),
                    next_search_at = item.next_search_at.as_deref().unwrap_or("none"),
                    search_phase = item.search_phase.as_str(),
                    immediate = immediate,
                    "created wanted item for movie"
                );
            }
            Err(err) => {
                warn!(title_id = title.id.as_str(), error = %err, "failed to upsert wanted item for movie");
            }
        }
    }

    async fn sync_wanted_series(&self, title: &Title, now: &DateTime<Utc>) {
        self.sync_wanted_series_inner(title, now, false).await;
    }

    /// Sync wanted items for a series. When `immediate` is true, sets `next_search_at = now`
    /// so the background poller picks up new items on the next 60-second tick.
    pub(crate) async fn sync_wanted_series_inner(&self, title: &Title, now: &DateTime<Utc>, immediate: bool) {
        let collections = match self.services.shows.list_collections_for_title(&title.id).await {
            Ok(c) => c,
            Err(err) => {
                warn!(title_id = title.id.as_str(), error = %err, "failed to list collections for wanted sync");
                return;
            }
        };

        // Get existing files for the title to know which episodes already have files
        let existing_files = self
            .services
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default();
        let episodes_with_files: std::collections::HashSet<String> = existing_files
            .iter()
            .filter_map(|f| f.episode_id.clone())
            .collect();

        for collection in &collections {
            if !collection.monitored {
                continue;
            }

            let episodes = match self.services.shows.list_episodes_for_collection(&collection.id).await {
                Ok(eps) => eps,
                Err(_) => continue,
            };

            for episode in &episodes {
                if !episode.monitored {
                    continue;
                }

                if episodes_with_files.contains(&episode.id) {
                    continue;
                }

                let baseline_date = episode.air_date.clone();

                let schedule = compute_search_schedule(
                    "episode",
                    baseline_date.as_deref(),
                    "primary",
                    now,
                );

                let next_search_at = if immediate {
                    now.to_rfc3339()
                } else {
                    schedule.next_search_at
                };

                let item = WantedItem {
                    id: Id::new().0,
                    title_id: title.id.clone(),
                    title_name: None,
                    episode_id: Some(episode.id.clone()),
                    media_type: "episode".to_string(),
                    search_phase: schedule.search_phase,
                    next_search_at: Some(next_search_at),
                    last_search_at: None,
                    search_count: 0,
                    baseline_date,
                    status: "wanted".to_string(),
                    grabbed_release: None,
                    current_score: None,
                    created_at: now.to_rfc3339(),
                    updated_at: now.to_rfc3339(),
                };

                if let Err(err) = self.services.wanted_items.upsert_wanted_item(&item).await {
                    warn!(
                        title_id = title.id.as_str(),
                        episode_id = episode.id.as_str(),
                        error = %err,
                        "failed to upsert wanted item for episode"
                    );
                }
            }
        }
    }
}

/// Snapshot of the download client's current queue and recent history,
/// fetched once per polling cycle to avoid repeated API calls.
struct DownloadClientSnapshot {
    /// Lowercase title names of items currently queued or downloading.
    active_titles: std::collections::HashSet<String>,
    /// Failed history items keyed by lowercase title name, value is the
    /// attention_reason (e.g. "HEALTH", "PAR", "UNPACK").
    failed_titles: std::collections::HashMap<String, String>,
}

impl DownloadClientSnapshot {
    async fn fetch(app: &AppUseCase) -> Self {
        let mut active_titles = std::collections::HashSet::new();
        let mut failed_titles = std::collections::HashMap::new();

        // Fetch current queue
        if let Ok(queue) = app.services.download_client.list_queue().await {
            for item in &queue {
                match item.state {
                    DownloadQueueState::Queued
                    | DownloadQueueState::Downloading
                    | DownloadQueueState::Paused => {
                        active_titles.insert(item.title_name.to_ascii_lowercase());
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

        // Fetch recent history
        if let Ok(history) = app.services.download_client.list_history().await {
            for item in &history {
                if item.state == DownloadQueueState::Failed {
                    if let Some(reason) = item.attention_reason.as_deref() {
                        let reason_upper = reason.to_ascii_uppercase();
                        if reason_upper == "HEALTH"
                            || reason_upper == "PAR"
                            || reason_upper == "UNPACK"
                        {
                            failed_titles.insert(
                                item.title_name.to_ascii_lowercase(),
                                reason_upper,
                            );
                        }
                    }
                }
            }
            if !failed_titles.is_empty() {
                info!(
                    failed_count = failed_titles.len(),
                    "download client snapshot: failed history items (health/par/unpack)"
                );
            }
        }

        Self {
            active_titles,
            failed_titles,
        }
    }

    /// Returns true if a release with this title is currently queued/downloading.
    fn is_active(&self, release_title: &str) -> bool {
        self.active_titles.contains(&release_title.to_ascii_lowercase())
    }

    /// If a release with this title failed in history with a blocklist-worthy
    /// reason, returns the failure reason (e.g. "HEALTH").
    fn failed_reason(&self, release_title: &str) -> Option<&str> {
        self.failed_titles
            .get(&release_title.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// Process due wanted items: search indexers and auto-grab best releases.
async fn process_due_wanted_items(app: &AppUseCase) {
    let now = Utc::now();
    let now_str = now.to_rfc3339();

    let due_items = match app.services.wanted_items.list_due_wanted_items(&now_str, 50).await {
        Ok(items) => {
            if !items.is_empty() {
                info!(
                    count = items.len(),
                    now = now_str.as_str(),
                    "background acquisition: found due wanted items"
                );
            }
            items
        }
        Err(err) => {
            warn!(error = %err, "failed to list due wanted items");
            return;
        }
    };

    if due_items.is_empty() {
        return;
    }

    info!(count = due_items.len(), "processing due wanted items");

    // Snapshot the download client state once for the entire cycle.
    let dl_snapshot = DownloadClientSnapshot::fetch(app).await;

    // Track URLs already submitted this cycle to avoid sending the same NZB
    // multiple times (e.g. a season pack matching several episode wanted items).
    let mut grabbed_urls: std::collections::HashSet<String> = std::collections::HashSet::new();

    for item in &due_items {
        if let Err(err) = process_single_wanted_item(app, item, &now, &mut grabbed_urls, &dl_snapshot).await {
            warn!(
                wanted_item_id = item.id.as_str(),
                title_id = item.title_id.as_str(),
                error = %err,
                "failed to process wanted item"
            );
        }

        // Update search schedule regardless of outcome
        let schedule = compute_search_schedule(
            &item.media_type,
            item.baseline_date.as_deref(),
            &item.search_phase,
            &now,
        );

        let new_status = if schedule.search_phase == "paused" { "paused" } else { "wanted" };

        let _ = app.services.wanted_items.update_wanted_item_status(
            &item.id,
            new_status,
            Some(&schedule.next_search_at),
            Some(&now.to_rfc3339()),
            item.search_count + 1,
            item.current_score,
            item.grabbed_release.as_deref(),
        ).await;
    }
}

async fn process_single_wanted_item(
    app: &AppUseCase,
    item: &WantedItem,
    now: &DateTime<Utc>,
    grabbed_urls: &mut std::collections::HashSet<String>,
    dl_snapshot: &DownloadClientSnapshot,
) -> AppResult<()> {
    // Load the title to get search context
    let title = match app.services.titles.get_by_id(&item.title_id).await? {
        Some(t) => t,
        None => {
            warn!(title_id = item.title_id.as_str(), "wanted item references missing title");
            return Ok(());
        }
    };

    // Load episode data for episode-type wanted items
    let episode = if item.media_type == "episode" {
        if let Some(ep_id) = item.episode_id.as_deref() {
            match app.services.shows.get_episode_by_id(ep_id).await {
                Ok(ep) => ep,
                Err(err) => {
                    warn!(episode_id = ep_id, error = %err, "failed to load episode for wanted item");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Build search queries based on media type
    let (queries, imdb_id, tvdb_id, category) = build_search_queries(&title, item, episode.as_ref(), &app.facet_registry);

    // Derive the download client category separately — search_category ("series")
    // is for Newznab query type, download_category ("tv") is for NZBGet routing.
    // Uses the configurable per-facet nzbget.category setting with hardcoded fallback.
    let download_cat = app.derive_download_category(&title.facet).await;

    if queries.is_empty() {
        info!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            media_type = item.media_type.as_str(),
            "background acquisition: no search queries built, skipping"
        );
        return Ok(());
    }

    info!(
        title_id = title.id.as_str(),
        title_name = title.name.as_str(),
        queries = ?queries,
        imdb_id = imdb_id.as_deref().unwrap_or(""),
        tvdb_id = tvdb_id.as_deref().unwrap_or(""),
        category = category.as_str(),
        "background acquisition: searching indexers"
    );

    // Resolve per-item runtime for size scoring
    let runtime_minutes = episode
        .as_ref()
        .and_then(|ep| ep.duration_seconds)
        .map(|s| (s / 60) as i32)
        .or(title.runtime_minutes);

    // Search and score releases
    let results = match app.search_and_score_releases(
        queries,
        imdb_id,
        tvdb_id,
        Some(category.clone()),
        &title.tags,
        200,
        "background_acquisition",
        SearchMode::Auto,
        runtime_minutes,
    ).await {
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

    // Emit search completed activity
    let _ = app.services.record_activity_event(
        None,
        Some(title.id.clone()),
        ActivityKind::AcquisitionSearchCompleted,
        format!("{} results for '{}'", results.len(), title.name),
        ActivitySeverity::Info,
        vec![ActivityChannel::WebUi],
    ).await;

    if results.is_empty() {
        info!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            "background acquisition: search returned 0 results"
        );
        return Ok(());
    }

    info!(
        title_id = title.id.as_str(),
        title_name = title.name.as_str(),
        result_count = results.len(),
        "background acquisition: evaluating candidates"
    );

    let mut selected_candidate: Option<&IndexerSearchResult> = None;
    let mut had_allowed_candidate = false;
    let mut skipped_for_failed = false;

    for candidate in &results {
        let is_allowed = candidate
            .quality_profile_decision
            .as_ref()
            .map(|d| d.allowed)
            .unwrap_or(false);
        if !is_allowed {
            continue;
        }

        had_allowed_candidate = true;

        if dl_snapshot.is_active(&candidate.title) {
            info!(
                title = title.name.as_str(),
                release = candidate.title.as_str(),
                "skipping release already active in download client queue"
            );
            continue;
        }

        if let Some(failure_reason) = dl_snapshot.failed_reason(&candidate.title) {
            warn!(
                title = title.name.as_str(),
                release = candidate.title.as_str(),
                reason = failure_reason,
                "release failed in download client history, adding to blocklist"
            );

            let hint = normalize_release_attempt_hint(
                candidate.download_url.as_deref().or(candidate.link.as_deref()),
            );
            let rel_title = normalize_release_attempt_title(Some(&candidate.title));
            let password = normalize_release_password(candidate.nzbgeek_password_protected.as_deref());

            let _ = app
                .services
                .release_attempts
                .record_release_attempt(
                    Some(title.id.clone()),
                    hint,
                    rel_title,
                    ReleaseDownloadAttemptOutcome::Failed,
                    Some(format!("download client failure: {failure_reason}")),
                    password,
                )
                .await;

            skipped_for_failed = true;
            continue;
        }

        selected_candidate = Some(candidate);
        break;
    }

    let Some(best) = selected_candidate else {
        if had_allowed_candidate && skipped_for_failed {
            warn!(
                title_id = title.id.as_str(),
                title_name = title.name.as_str(),
                "background acquisition: no suitable candidates found after skipping blocklisted or active releases"
            );
        } else if had_allowed_candidate {
            info!(
                title_id = title.id.as_str(),
                title_name = title.name.as_str(),
                "background acquisition: all allowed candidates were already active"
            );
        } else {
            info!(
                title_id = title.id.as_str(),
                title_name = title.name.as_str(),
                result_count = results.len(),
                "background acquisition: no allowed candidates found (all blocked by quality profile)"
            );
        }
        return Ok(());
    };

    let candidate_score = best
        .quality_profile_decision
        .as_ref()
        .map(|d| d.preference_score)
        .unwrap_or(0);

    // Resolve quality profile to check allow_upgrades
    let profile = app.resolve_quality_profile(
        &title.tags,
        title.imdb_id.as_deref(),
        tvdb_id_from_external_ids(&title.external_ids).as_deref(),
        Some(&category),
    )
    .await
    .unwrap_or_else(|_| crate::quality_profile::default_quality_profile_for_search());

    // Evaluate upgrade decision
    let thresholds = AcquisitionThresholds::default();
    let decision = evaluate_upgrade(
        candidate_score,
        item.current_score,
        profile.criteria.allow_upgrades,
        item.last_search_at.as_deref(),
        now,
        &thresholds,
    );

    // Record the decision
    let decision_record = ReleaseDecision {
        id: Id::new().0,
        wanted_item_id: item.id.clone(),
        title_id: title.id.clone(),
        release_title: best.title.clone(),
        release_url: best.download_url.clone().or_else(|| best.link.clone()),
        release_size_bytes: best.size_bytes,
        decision_code: decision.code().to_string(),
        candidate_score,
        current_score: item.current_score,
        score_delta: item.current_score.map(|c| candidate_score - c),
        explanation_json: best.quality_profile_decision.as_ref().map(|d| {
            serde_json::to_string(&d.scoring_log.iter().map(|e| {
                serde_json::json!({"code": e.code, "delta": e.delta})
            }).collect::<Vec<_>>()).unwrap_or_default()
        }),
        created_at: now.to_rfc3339(),
    };

    let _ = app.services.wanted_items.insert_release_decision(&decision_record).await;

    if !decision.is_accept() {
        let _ = app.services.record_activity_event(
            None,
            Some(title.id.clone()),
            ActivityKind::AcquisitionCandidateRejected,
            format!("{}: '{}' ({})", decision.code(), best.title, title.name),
            ActivitySeverity::Info,
            vec![ActivityChannel::WebUi],
        ).await;
        return Ok(());
    }

    let _ = app.services.record_activity_event(
        None,
        Some(title.id.clone()),
        ActivityKind::AcquisitionCandidateAccepted,
        format!("'{}' score={} delta={} ({})", best.title, candidate_score,
            decision_record.score_delta.unwrap_or(candidate_score), title.name),
        ActivitySeverity::Success,
        vec![ActivityChannel::WebUi, ActivityChannel::Toast],
    ).await;

    // Submit to download client
    let source_hint = best.download_url.clone().or_else(|| best.link.clone());

    // Deduplicate: skip if this exact URL was already submitted this cycle.
    // This prevents the same season pack from being queued N times when N
    // episode wanted items all resolve to the same release.
    if let Some(url) = source_hint.as_deref() {
        if !grabbed_urls.insert(url.to_string()) {
            info!(
                title = title.name.as_str(),
                release = best.title.as_str(),
                "skipping duplicate release already submitted this cycle"
            );
            // Mark this wanted item as grabbed too since the release covers it
            let grabbed_json = serde_json::json!({
                "title": best.title,
                "score": candidate_score,
                "grabbed_at": now.to_rfc3339(),
                "deduplicated": true,
            }).to_string();
            let _ = app.services.wanted_items.update_wanted_item_status(
                &item.id,
                "grabbed",
                None,
                Some(&now.to_rfc3339()),
                item.search_count + 1,
                item.current_score,
                Some(&grabbed_json),
            ).await;
            return Ok(());
        }
    }

    let source_title = Some(best.title.clone());
    let source_hint_for_attempt = normalize_release_attempt_hint(source_hint.as_deref());
    let source_title_for_attempt = normalize_release_attempt_title(source_title.as_deref());
    let source_password = normalize_release_password(best.nzbgeek_password_protected.as_deref());

    let _ = app
        .services
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

    info!(
        title = title.name.as_str(),
        release = best.title.as_str(),
        score = candidate_score,
        decision = decision.code(),
        "auto-grabbing release"
    );

    let grab_result = app.services.download_client
        .submit_to_download_queue(
            &title,
            source_hint.clone(),
            source_title.clone(),
            source_password.clone(),
            Some(download_cat.clone()),
        )
        .await;

    match grab_result {
        Ok(_job_id) => {
            // Record as release attempt for blocklist tracking
            let _ = app.services.release_attempts
                .record_release_attempt(
                    Some(title.id.clone()),
                    source_hint_for_attempt.clone(),
                    source_title_for_attempt.clone(),
                    ReleaseDownloadAttemptOutcome::Success,
                    None,
                    source_password.clone(),
                )
                .await;

            // Update wanted item to grabbed
            let grabbed_json = serde_json::json!({
                "title": best.title,
                "score": candidate_score,
                "grabbed_at": now.to_rfc3339(),
            }).to_string();

            let _ = app.services.wanted_items.update_wanted_item_status(
                &item.id,
                "grabbed",
                None,
                Some(&now.to_rfc3339()),
                item.search_count + 1,
                item.current_score,
                Some(&grabbed_json),
            ).await;

            let _ = app.services.record_activity_event(
                None,
                Some(title.id.clone()),
                ActivityKind::MovieDownloaded,
                format!("auto-grabbed: {} (score: {})", best.title, candidate_score),
                ActivitySeverity::Success,
                vec![ActivityChannel::WebUi, ActivityChannel::Toast],
            ).await;
        }
        Err(err) => {
            warn!(
                title = title.name.as_str(),
                error = %err,
                "auto-grab download submission failed"
            );

            let _ = app.services.release_attempts
                .record_release_attempt(
                    Some(title.id.clone()),
                    source_hint_for_attempt,
                    source_title_for_attempt,
                    ReleaseDownloadAttemptOutcome::Failed,
                    Some(err.to_string()),
                    source_password,
                )
                .await;

            let _ = app.services.record_activity_event(
                None,
                Some(title.id.clone()),
                ActivityKind::AcquisitionDownloadFailed,
                format!("download failed for '{}': {}", title.name, err),
                ActivitySeverity::Error,
                vec![ActivityChannel::WebUi, ActivityChannel::Toast],
            ).await;
        }
    }

    Ok(())
}

fn build_search_queries(
    title: &Title,
    item: &WantedItem,
    episode: Option<&Episode>,
    facet_registry: &crate::FacetRegistry,
) -> (Vec<String>, Option<String>, Option<String>, String) {
    let imdb_id = title.imdb_id.clone();
    let tvdb_id = tvdb_id_from_external_ids(&title.external_ids);

    let category = facet_registry
        .get(&title.facet)
        .map(|h| h.search_category().to_string())
        .unwrap_or_else(|| "series".to_string());

    match item.media_type.as_str() {
        "movie" => {
            let mut queries = Vec::new();
            if !title.name.is_empty() {
                let query = if let Some(year) = title.year {
                    format!("{} {}", title.name, year)
                } else {
                    title.name.clone()
                };
                queries.push(query);
            }
            if queries.is_empty() && imdb_id.is_some() {
                queries.push(String::new());
            }
            (queries, imdb_id, tvdb_id, category)
        }
        "episode" => {
            let mut queries = Vec::new();

            if let Some(ep) = episode {
                let season_num: usize = ep
                    .season_number
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let episode_num: usize = ep
                    .episode_number
                    .as_deref()
                    .and_then(|e| e.parse().ok())
                    .unwrap_or(0);

                if season_num > 0 && episode_num > 0 {
                    queries.push(format!("S{:0>2}E{:0>2}", season_num, episode_num));
                    queries.push(format!("S{}E{}", season_num, episode_num));
                    queries.push(format!("{}x{}", season_num, episode_num));
                }

                // For anime season 0 (specials/OVAs): use title-based search
                // S00E## format produces poor results on indexers
                if season_num == 0 && title.facet == scryer_domain::MediaFacet::Anime {
                    if let Some(label) = ep.episode_label.as_deref().filter(|l| !l.is_empty()) {
                        queries.push(format!("{} {}", title.name, label));
                    }
                    if episode_num > 0 {
                        if ep.episode_type == "ova" {
                            queries.push(format!("{} OVA {:0>2}", title.name, episode_num));
                        } else {
                            queries.push(format!("{} Special {:0>2}", title.name, episode_num));
                        }
                    }
                }

                // For anime: add absolute number as fallback query
                // (long-running series like Naruto/One Piece use absolute numbering on indexers)
                if title.facet == scryer_domain::MediaFacet::Anime {
                    if let Some(abs) = ep
                        .absolute_number
                        .as_deref()
                        .and_then(|a| a.parse::<usize>().ok())
                        .filter(|&a| a > 0)
                    {
                        queries.push(format!("{:0>3}", abs));
                    }
                }

                if !queries.is_empty() {
                    // Dedup (e.g. S10E10 == S10E10 when both formats produce the same string)
                    let mut seen = std::collections::HashSet::new();
                    queries.retain(|q| seen.insert(q.to_ascii_lowercase()));
                }
            }

            // Fallback: if we couldn't build S##E## queries, use tvdb_id or title
            if queries.is_empty() {
                if tvdb_id.is_some() || imdb_id.is_some() {
                    queries.push(String::new());
                } else if !title.name.is_empty() {
                    queries.push(title.name.clone());
                }
            }

            (queries, imdb_id, tvdb_id, category)
        }
        _ => (vec![], None, None, category),
    }
}

fn tvdb_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source == "tvdb")
        .map(|id| id.value.clone())
}

// --- Public use-case methods for the wanted items API ---

impl AppUseCase {
    pub async fn list_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<(Vec<WantedItem>, i64)> {
        let items = self
            .services
            .wanted_items
            .list_wanted_items(status, media_type, title_id, limit, offset)
            .await?;
        let total = self
            .services
            .wanted_items
            .count_wanted_items(status, media_type, title_id)
            .await?;
        Ok((items, total))
    }

    pub async fn list_release_decisions(
        &self,
        wanted_item_id: Option<&str>,
        title_id: Option<&str>,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        if let Some(wid) = wanted_item_id {
            return self
                .services
                .wanted_items
                .list_release_decisions_for_wanted_item(wid, limit)
                .await;
        }
        if let Some(tid) = title_id {
            return self
                .services
                .wanted_items
                .list_release_decisions_for_title(tid, limit)
                .await;
        }
        Ok(vec![])
    }

    pub async fn trigger_wanted_item_search(&self, wanted_item_id: &str) -> AppResult<()> {
        let item = self
            .services
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;

        let now = Utc::now();
        self.services
            .wanted_items
            .update_wanted_item_status(
                &item.id,
                "wanted",
                Some(&now.to_rfc3339()),
                item.last_search_at.as_deref(),
                item.search_count,
                item.current_score,
                item.grabbed_release.as_deref(),
            )
            .await?;
        self.services.acquisition_wake.notify_one();
        Ok(())
    }

    pub async fn pause_wanted_item(&self, wanted_item_id: &str) -> AppResult<()> {
        let item = self
            .services
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;

        self.services
            .wanted_items
            .update_wanted_item_status(
                &item.id,
                "paused",
                None,
                item.last_search_at.as_deref(),
                item.search_count,
                item.current_score,
                item.grabbed_release.as_deref(),
            )
            .await
    }

    pub async fn resume_wanted_item(&self, wanted_item_id: &str) -> AppResult<()> {
        let item = self
            .services
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;

        let now = Utc::now();
        let schedule = compute_search_schedule(
            &item.media_type,
            item.baseline_date.as_deref(),
            &item.search_phase,
            &now,
        );

        self.services
            .wanted_items
            .update_wanted_item_status(
                &item.id,
                "wanted",
                Some(&schedule.next_search_at),
                item.last_search_at.as_deref(),
                item.search_count,
                item.current_score,
                item.grabbed_release.as_deref(),
            )
            .await
    }

    pub async fn reset_wanted_item(&self, wanted_item_id: &str) -> AppResult<()> {
        let item = self
            .services
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;

        let now = Utc::now();
        let schedule = compute_search_schedule(
            &item.media_type,
            item.baseline_date.as_deref(),
            "primary",
            &now,
        );

        self.services
            .wanted_items
            .update_wanted_item_status(
                &item.id,
                "wanted",
                Some(&schedule.next_search_at),
                None,
                0,
                None,
                None,
            )
            .await
    }
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

    info!("background acquisition poller started");

    // Initial wanted state sync
    if let Err(err) = app.sync_wanted_state().await {
        warn!(error = %err, "initial wanted state sync failed");
    }

    let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(60));
    let mut sync_interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    let mut metadata_refresh_interval = tokio::time::interval(std::time::Duration::from_secs(43200)); // 12h

    // Consume the first tick immediately
    poll_interval.tick().await;
    sync_interval.tick().await;
    metadata_refresh_interval.tick().await;

    let wake = app.services.acquisition_wake.clone();

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!("background acquisition poller shutting down");
                break;
            }
            _ = wake.notified() => {
                process_due_wanted_items(&app).await;
            }
            _ = poll_interval.tick() => {
                process_due_wanted_items(&app).await;
            }
            _ = sync_interval.tick() => {
                if let Err(err) = app.sync_wanted_state().await {
                    warn!(error = %err, "periodic wanted state sync failed");
                }
            }
            _ = metadata_refresh_interval.tick() => {
                info!("starting periodic metadata refresh for monitored series");
                app.refresh_monitored_series_metadata().await;
            }
        }
    }
}
