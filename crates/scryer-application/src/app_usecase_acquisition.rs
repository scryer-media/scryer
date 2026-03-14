use super::*;
use crate::acquisition_policy::{compute_search_schedule, evaluate_upgrade, AcquisitionThresholds};
use chrono::{DateTime, Utc};
use scryer_domain::NotificationEventType;
use std::collections::HashMap;
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
                if let Err(err) = self
                    .services
                    .wanted_items
                    .delete_wanted_items_for_title(&title.id)
                    .await
                {
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

    pub(crate) async fn sync_wanted_movie_inner(
        &self,
        title: &Title,
        now: &DateTime<Utc>,
        immediate: bool,
    ) {
        // Check if movie already has a media file
        let has_file = match self
            .services
            .media_files
            .list_media_files_for_title(&title.id)
            .await
        {
            Ok(files) => !files.is_empty(),
            Err(_) => false,
        };

        if has_file {
            return;
        }

        // Minimum availability gate: skip search if the movie hasn't reached the
        // configured availability threshold yet.
        let availability = title.min_availability.as_deref().unwrap_or("announced");
        if !is_movie_available_for_acquisition(title, availability, now) {
            info!(
                title_id = title.id.as_str(),
                min_availability = availability,
                "skipping movie: availability threshold not reached"
            );
            return;
        }

        // Determine baseline date for search scheduling
        let baseline_date = title.first_aired.clone();

        let schedule = compute_search_schedule("movie", baseline_date.as_deref(), "primary", now);

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
            season_number: None,
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
    pub(crate) async fn sync_wanted_series_inner(
        &self,
        title: &Title,
        now: &DateTime<Utc>,
        immediate: bool,
    ) {
        let collections = match self
            .services
            .shows
            .list_collections_for_title(&title.id)
            .await
        {
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

            let episodes = match self
                .services
                .shows
                .list_episodes_for_collection(&collection.id)
                .await
            {
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

                let schedule =
                    compute_search_schedule("episode", baseline_date.as_deref(), "primary", now);

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
                    season_number: episode.season_number.clone(),
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
pub(crate) struct DownloadClientSnapshot {
    /// Lowercase title names of items currently queued or downloading.
    active_titles: std::collections::HashSet<String>,
    /// Failed history items keyed by lowercase title name.
    failed_titles: std::collections::HashMap<String, FailedDownloadSnapshot>,
}

#[derive(Clone, Debug)]
pub(crate) struct FailedDownloadSnapshot {
    reason: String,
    download_client_item_id: String,
    client_id: String,
}

impl DownloadClientSnapshot {
    pub(crate) async fn fetch(app: &AppUseCase) -> Self {
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
                                FailedDownloadSnapshot {
                                    reason: reason_upper,
                                    download_client_item_id: item.download_client_item_id.clone(),
                                    client_id: item.client_id.clone(),
                                },
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
    pub(crate) fn is_active(&self, release_title: &str) -> bool {
        self.active_titles
            .contains(&release_title.to_ascii_lowercase())
    }

    /// If a release with this title failed in history with a blocklist-worthy
    /// reason, returns the failure reason (e.g. "HEALTH").
    pub(crate) fn failed_item(&self, release_title: &str) -> Option<&FailedDownloadSnapshot> {
        self.failed_titles.get(&release_title.to_ascii_lowercase())
    }
}

/// Check grabbed wanted items against the download client. If a grabbed
/// release has failed in the download client, blocklist it and re-queue the
/// wanted item for immediate re-search.
async fn check_grabbed_for_failures(app: &AppUseCase, dl_snapshot: &DownloadClientSnapshot) {
    let grabbed_items = match app
        .services
        .wanted_items
        .list_wanted_items(Some("grabbed"), None, None, 200, 0)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            warn!(error = %err, "failed to list grabbed wanted items for failure check");
            return;
        }
    };

    if grabbed_items.is_empty() {
        return;
    }

    let now = Utc::now();
    let now_str = now.to_rfc3339();

    for item in &grabbed_items {
        // Extract the grabbed release title from the stored JSON
        let release_title = item
            .grabbed_release
            .as_deref()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .and_then(|v| v.get("title").and_then(|t| t.as_str().map(String::from)));

        let Some(release_title) = release_title else {
            continue;
        };

        if let Some(failed_item) = dl_snapshot.failed_item(&release_title) {
            warn!(
                title_id = item.title_id.as_str(),
                release = release_title.as_str(),
                reason = failed_item.reason.as_str(),
                "grabbed release failed in download client, re-queuing for search"
            );

            // Blocklist the failed release
            let hint = normalize_release_attempt_hint(None);
            let rel_title = normalize_release_attempt_title(Some(&release_title));

            let _ = app
                .services
                .release_attempts
                .record_release_attempt(
                    Some(item.title_id.clone()),
                    hint,
                    rel_title,
                    ReleaseDownloadAttemptOutcome::Failed,
                    Some(format!("download client failure: {}", failed_item.reason)),
                    None,
                )
                .await;

            // Re-queue for immediate re-search
            let _ = app
                .services
                .wanted_items
                .update_wanted_item_status(
                    &item.id,
                    "wanted",
                    Some(&now_str),
                    None,
                    item.search_count,
                    item.current_score,
                    None,
                )
                .await;

            let _ = app
                .services
                .record_activity_event(
                    None,
                    Some(item.title_id.clone()),
                    ActivityKind::AcquisitionDownloadFailed,
                    format!(
                        "download failed for '{}', re-queuing for search",
                        release_title
                    ),
                    ActivitySeverity::Warning,
                    vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                )
                .await;

            if let Ok(Some(title)) = app.services.titles.get_by_id(&item.title_id).await {
                if app
                    .should_remove_failed_download(&title.facet, &failed_item.client_id)
                    .await
                {
                    if let Err(error) = app
                        .services
                        .download_client
                        .delete_queue_item(&failed_item.download_client_item_id, true)
                        .await
                    {
                        warn!(
                            title_id = item.title_id.as_str(),
                            client_id = failed_item.client_id.as_str(),
                            download_client_item_id = failed_item.download_client_item_id.as_str(),
                            error = %error,
                            "failed to delete failed download from client history"
                        );
                    }
                }
            }
        }
    }
}

/// Process due wanted items: search indexers and auto-grab best releases.
async fn process_due_wanted_items(app: &AppUseCase) {
    let now = Utc::now();
    let now_str = now.to_rfc3339();

    let due_items = match app
        .services
        .wanted_items
        .list_due_wanted_items(&now_str, 50)
        .await
    {
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

    // Check grabbed items for download failures and re-queue them
    check_grabbed_for_failures(app, &dl_snapshot).await;

    // Track URLs already submitted this cycle to avoid sending the same NZB
    // multiple times (e.g. a season pack matching several episode wanted items).
    let mut grabbed_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Track (title_id, season_num) for which a season pack search was attempted this cycle.
    let mut season_pack_attempted: std::collections::HashSet<(String, u32)> =
        std::collections::HashSet::new();
    // Track (title_id, season_num) for which a season pack was successfully grabbed this cycle.
    let mut season_pack_grabbed: std::collections::HashSet<(String, u32)> =
        std::collections::HashSet::new();

    // Count due episode items per (title_id, season_num). Season pack search is only
    // worthwhile when >= 2 episodes from the same season are due this cycle — mirroring
    // Sonarr's rule of "count > 1 missing" before issuing a SeasonSearchCriteria.
    let mut season_due_counts: std::collections::HashMap<(String, u32), usize> =
        std::collections::HashMap::new();
    for item in &due_items {
        if item.media_type == "episode" {
            if let Some(sn) = item.season_number.as_deref() {
                if let Ok(n) = sn.parse::<u32>() {
                    if n > 0 {
                        *season_due_counts
                            .entry((item.title_id.clone(), n))
                            .or_insert(0) += 1;
                    }
                }
            }
        }
    }

    for item in &due_items {
        if let Err(err) = process_single_wanted_item(
            app,
            item,
            &now,
            &mut grabbed_urls,
            &mut season_pack_attempted,
            &mut season_pack_grabbed,
            &season_due_counts,
            &dl_snapshot,
        )
        .await
        {
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

        let new_status = if schedule.search_phase == "paused" {
            "paused"
        } else {
            "wanted"
        };

        let _ = app
            .services
            .wanted_items
            .update_wanted_item_status(
                &item.id,
                new_status,
                Some(&schedule.next_search_at),
                Some(&now.to_rfc3339()),
                item.search_count + 1,
                item.current_score,
                item.grabbed_release.as_deref(),
            )
            .await;
    }
}

async fn process_single_wanted_item(
    app: &AppUseCase,
    item: &WantedItem,
    now: &DateTime<Utc>,
    grabbed_urls: &mut std::collections::HashSet<String>,
    season_pack_attempted: &mut std::collections::HashSet<(String, u32)>,
    season_pack_grabbed: &mut std::collections::HashSet<(String, u32)>,
    season_due_counts: &std::collections::HashMap<(String, u32), usize>,
    dl_snapshot: &DownloadClientSnapshot,
) -> AppResult<()> {
    // Load the title to get search context
    let title = match app.services.titles.get_by_id(&item.title_id).await? {
        Some(t) => t,
        None => {
            warn!(
                title_id = item.title_id.as_str(),
                "wanted item references missing title"
            );
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
    let sq = build_search_queries(&title, item, episode.as_ref(), &app.facet_registry);
    let (queries, imdb_id, tvdb_id, category) = (sq.queries, sq.imdb_id, sq.tvdb_id, sq.category);
    let (search_season, search_episode) = (sq.season, sq.episode);

    // Derive the download client category separately — search_category ("series")
    // is for Newznab query type, download_category ("tv") is for NZBGet routing.
    //
    // ── Season pack priority ──────────────────────────────────────────────────
    // For episode wanted items, try a season pack search first. Season packs are
    // a first-class release type on Usenet and are more efficient than individual
    // episodes. Individual episode searches only run if no season pack was found
    // this cycle for this (title, season).
    if item.media_type == "episode" {
        if let Some(season_num) = search_season {
            let season_key = (title.id.clone(), season_num);

            // Only attempt a season pack search when >= 2 episodes from this season
            // are due this cycle (mirrors Sonarr: count > 1 missing → SeasonSearchCriteria).
            let due_count = season_due_counts.get(&season_key).copied().unwrap_or(0);

            if due_count >= 2 && !season_pack_attempted.contains(&season_key) {
                season_pack_attempted.insert(season_key.clone());

                let mut pack_queries =
                    vec![format!("S{:0>2}", season_num), format!("S{}", season_num)];
                let mut seen = std::collections::HashSet::new();
                pack_queries.retain(|q| seen.insert(q.to_ascii_lowercase()));

                let pack_results = app
                    .search_and_score_releases(
                        pack_queries,
                        imdb_id.clone(),
                        tvdb_id.clone(),
                        Some(category.clone()),
                        &title.tags,
                        50,
                        "background_acquisition_season_pack",
                        SearchMode::Auto,
                        title.runtime_minutes,
                        Some(season_num),
                        None, // episode=None signals a season pack search
                        None, // no absolute episode for season packs
                    )
                    .await
                    .unwrap_or_default();

                if let Some(best_pack) = pack_results.iter().find(|r| {
                    r.quality_profile_decision
                        .as_ref()
                        .map(|d| d.allowed)
                        .unwrap_or(false)
                }) {
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
                        let pack_password = normalize_release_password(
                            best_pack.nzbgeek_password_protected.as_deref(),
                        );

                        let grab_result = app
                            .services
                            .download_client
                            .submit_download(&DownloadClientAddRequest {
                                title: title.clone(),
                                source_hint: pack_url.clone(),
                                source_kind: best_pack.source_kind,
                                source_title: pack_title.clone(),
                                source_password: pack_password.clone(),
                                category: Some(download_cat),
                                queue_priority: None,
                                download_directory: None,
                                release_title: Some(best_pack.title.clone()),
                                indexer_name: Some(best_pack.source.clone()),
                                info_hash_hint: best_pack
                                    .extra
                                    .get("info_hash")
                                    .and_then(|value| value.as_str())
                                    .map(str::to_string),
                                seed_goal_ratio: None,
                                seed_goal_seconds: None,
                                is_recent,
                                season_pack: Some(true),
                            })
                            .await;

                        match grab_result {
                            Ok(grab) => {
                                let facet_label = serde_json::to_string(&title.facet)
                                    .unwrap_or_else(|_| "\"other\"".to_string())
                                    .trim_matches('"')
                                    .to_string();
                                metrics::counter!("scryer_grabs_total", "indexer" => best_pack.source.clone(), "facet" => facet_label).increment(1);
                                season_pack_grabbed.insert(season_key.clone());
                                let _ = app
                                    .services
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
                                let _ = app
                                    .services
                                    .download_submissions
                                    .record_submission(DownloadSubmission {
                                        title_id: title.id.clone(),
                                        facet: facet_str.trim_matches('"').to_string(),
                                        download_client_type: grab.client_type,
                                        download_client_item_id: grab.job_id,
                                        source_title: Some(best_pack.title.clone()),
                                    })
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
                                let grab_envelope = crate::activity::NotificationEnvelope {
                                    event_type: NotificationEventType::Grab,
                                    title: format!("Grabbed: {} S{:0>2}", title.name, season_num),
                                    body: format!(
                                        "Season pack '{}' grabbed for {}",
                                        best_pack.title, title.name
                                    ),
                                    facet: Some(format!("{:?}", title.facet).to_lowercase()),
                                    metadata: grab_meta,
                                };
                                let _ = app
                                    .services
                                    .record_activity_event_with_notification(
                                        None,
                                        Some(title.id.clone()),
                                        ActivityKind::AcquisitionCandidateAccepted,
                                        format!(
                                            "season pack grabbed: {} S{:0>2} '{}' (score: {})",
                                            title.name, season_num, best_pack.title, pack_score,
                                        ),
                                        ActivitySeverity::Success,
                                        vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                                        grab_envelope,
                                    )
                                    .await;
                                info!(
                                    title = title.name.as_str(),
                                    season = season_num,
                                    release = best_pack.title.as_str(),
                                    "season pack grabbed; skipping individual episode searches for this season"
                                );
                            }
                            Err(err) => {
                                warn!(
                                    title = title.name.as_str(),
                                    season = season_num,
                                    error = %err,
                                    "season pack grab failed, will fall back to individual episode search"
                                );
                                let _ = app
                                    .services
                                    .release_attempts
                                    .record_release_attempt(
                                        Some(title.id.clone()),
                                        pack_hint,
                                        pack_title_norm,
                                        ReleaseDownloadAttemptOutcome::Failed,
                                        Some(err.to_string()),
                                        pack_password,
                                    )
                                    .await;
                            }
                        }
                    }
                }
            }

            // If a season pack was grabbed this cycle (by this item or an earlier
            // item for the same season), skip the individual episode search.
            if season_pack_grabbed.contains(&season_key) {
                return Ok(());
            }
        }
    }
    // ── End season pack priority ──────────────────────────────────────────────
    // Uses the per-facet default download category; the selected client's
    // explicit routing category overrides this inside the router.
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
    let absolute_episode = episode
        .as_ref()
        .and_then(|ep| ep.absolute_number.as_deref())
        .and_then(|v| v.parse::<u32>().ok());
    let results = match app
        .search_and_score_releases(
            queries,
            imdb_id,
            tvdb_id,
            Some(category.clone()),
            &title.tags,
            200,
            "background_acquisition",
            SearchMode::Auto,
            runtime_minutes,
            search_season,
            search_episode,
            absolute_episode,
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

    // Emit search completed activity
    let _ = app
        .services
        .record_activity_event(
            None,
            Some(title.id.clone()),
            ActivityKind::AcquisitionSearchCompleted,
            format!("{} results for '{}'", results.len(), title.name),
            ActivitySeverity::Info,
            vec![ActivityChannel::WebUi],
        )
        .await;

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

    // Load DB-level blocklist (covers post-import failures like fake/non-video files,
    // in addition to the download-client snapshot checked below).
    let db_blocklist: std::collections::HashSet<String> = app
        .services
        .release_attempts
        .list_failed_release_signatures_for_title(&title.id, 200)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| e.source_title)
        .map(|t| t.to_ascii_lowercase())
        .collect();

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

        if let Some(failed_item) = dl_snapshot.failed_item(&candidate.title) {
            warn!(
                title = title.name.as_str(),
                release = candidate.title.as_str(),
                reason = failed_item.reason.as_str(),
                "release failed in download client history, adding to blocklist"
            );

            let hint = normalize_release_attempt_hint(
                candidate
                    .download_url
                    .as_deref()
                    .or(candidate.link.as_deref()),
            );
            let rel_title = normalize_release_attempt_title(Some(&candidate.title));
            let password =
                normalize_release_password(candidate.nzbgeek_password_protected.as_deref());

            let _ = app
                .services
                .release_attempts
                .record_release_attempt(
                    Some(title.id.clone()),
                    hint,
                    rel_title,
                    ReleaseDownloadAttemptOutcome::Failed,
                    Some(format!("download client failure: {}", failed_item.reason)),
                    password,
                )
                .await;

            skipped_for_failed = true;
            continue;
        }

        if db_blocklist.contains(&candidate.title.to_ascii_lowercase()) {
            warn!(
                title = title.name.as_str(),
                release = candidate.title.as_str(),
                "skipping DB-blocklisted release"
            );
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
    let profile = app
        .resolve_quality_profile(
            &title.tags,
            title.imdb_id.as_deref(),
            tvdb_id_from_external_ids(&title.external_ids).as_deref(),
            Some(&category),
        )
        .await
        .unwrap_or_else(|_| crate::quality_profile::default_quality_profile_for_search());

    // Cutoff tier check — skip upgrades if the existing file meets the cutoff quality
    if crate::quality_profile::has_reached_cutoff(
        item.grabbed_release.as_deref(),
        profile.criteria.cutoff_tier.as_deref(),
        &profile.criteria.quality_tiers,
    ) {
        tracing::debug!(
            title_id = title.id.as_str(),
            cutoff = profile.criteria.cutoff_tier.as_deref().unwrap_or(""),
            "cutoff quality reached, skipping upgrade"
        );
        return Ok(());
    }

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
            serde_json::to_string(
                &d.scoring_log
                    .iter()
                    .map(|e| serde_json::json!({"code": e.code, "delta": e.delta}))
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default()
        }),
        created_at: now.to_rfc3339(),
    };

    let _ = app
        .services
        .wanted_items
        .insert_release_decision(&decision_record)
        .await;

    if !decision.is_accept() {
        let _ = app
            .services
            .record_activity_event(
                None,
                Some(title.id.clone()),
                ActivityKind::AcquisitionCandidateRejected,
                format!("{}: '{}' ({})", decision.code(), best.title, title.name),
                ActivitySeverity::Info,
                vec![ActivityChannel::WebUi],
            )
            .await;
        return Ok(());
    }

    {
        let mut grab_meta = HashMap::new();
        grab_meta.insert("title_name".to_string(), serde_json::json!(title.name));
        grab_meta.insert("release_title".to_string(), serde_json::json!(best.title));
        grab_meta.insert("indexer".to_string(), serde_json::json!(best.source));
        grab_meta.insert("score".to_string(), serde_json::json!(candidate_score));
        let grab_envelope = crate::activity::NotificationEnvelope {
            event_type: NotificationEventType::Grab,
            title: format!("Grabbed: {}", title.name),
            body: format!(
                "'{}' grabbed for {} (score: {})",
                best.title, title.name, candidate_score
            ),
            facet: Some(format!("{:?}", title.facet).to_lowercase()),
            metadata: grab_meta,
        };
        let _ = app
            .services
            .record_activity_event_with_notification(
                None,
                Some(title.id.clone()),
                ActivityKind::AcquisitionCandidateAccepted,
                format!(
                    "'{}' score={} delta={} ({})",
                    best.title,
                    candidate_score,
                    decision_record.score_delta.unwrap_or(candidate_score),
                    title.name
                ),
                ActivitySeverity::Success,
                vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                grab_envelope,
            )
            .await;
    }

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
            })
            .to_string();
            let _ = app
                .services
                .wanted_items
                .update_wanted_item_status(
                    &item.id,
                    "grabbed",
                    None,
                    Some(&now.to_rfc3339()),
                    item.search_count + 1,
                    item.current_score,
                    Some(&grabbed_json),
                )
                .await;
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

    let is_recent = app.is_recent_for_queue_priority(
        best.published_at
            .as_deref()
            .or(episode.as_ref().and_then(|item| item.air_date.as_deref()))
            .or(item.baseline_date.as_deref())
            .or(title.first_aired.as_deref())
            .or(title.digital_release_date.as_deref()),
    );

    info!(
        title = title.name.as_str(),
        release = best.title.as_str(),
        score = candidate_score,
        decision = decision.code(),
        "auto-grabbing release"
    );

    let grab_result = app
        .services
        .download_client
        .submit_download(&DownloadClientAddRequest {
            title: title.clone(),
            source_hint: source_hint.clone(),
            source_kind: best.source_kind,
            source_title: source_title.clone(),
            source_password: source_password.clone(),
            category: Some(download_cat.clone()),
            queue_priority: None,
            download_directory: None,
            release_title: Some(best.title.clone()),
            indexer_name: Some(best.source.clone()),
            info_hash_hint: best
                .extra
                .get("info_hash")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            seed_goal_ratio: None,
            seed_goal_seconds: None,
            is_recent,
            season_pack: Some(false),
        })
        .await;

    match grab_result {
        Ok(grab) => {
            {
                let facet_label = serde_json::to_string(&title.facet)
                    .unwrap_or_else(|_| "\"other\"".to_string())
                    .trim_matches('"')
                    .to_string();
                metrics::counter!("scryer_grabs_total", "indexer" => best.source.clone(), "facet" => facet_label).increment(1);
            }

            // Record as release attempt for blocklist tracking
            let _ = app
                .services
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
            {
                let mut data = HashMap::new();
                data.insert("indexer".into(), serde_json::json!(&best.source));
                data.insert("download_client".into(), serde_json::json!(&grab.client_type));
                if let Some(rg) = best.parsed_release_metadata.as_ref().and_then(|m| m.release_group.as_ref()) {
                    data.insert("release_group".into(), serde_json::json!(rg));
                }
                if let Some(sz) = best.size_bytes {
                    data.insert("size_bytes".into(), serde_json::json!(sz));
                }
                if let Some(proto) = &best.source_kind {
                    data.insert("protocol".into(), serde_json::json!(format!("{:?}", proto)));
                }
                if let Some(pub_at) = &best.published_at {
                    data.insert("published_date".into(), serde_json::json!(pub_at));
                }
                if let Some(url) = &best.info_url {
                    data.insert("info_url".into(), serde_json::json!(url));
                }
                data.insert("score".into(), serde_json::json!(candidate_score));
                let _ = app
                    .services
                    .record_title_history(NewTitleHistoryEvent {
                        title_id: title.id.clone(),
                        episode_id: episode.as_ref().map(|e| e.id.clone()),
                        collection_id: None,
                        event_type: TitleHistoryEventType::Grabbed,
                        source_title: source_title.clone(),
                        quality: best.parsed_release_metadata.as_ref().and_then(|m| m.quality.as_ref()).map(|q| q.to_string()),
                        download_id: Some(grab.job_id.clone()),
                        data,
                    })
                    .await;
            }

            // Record download submission for auto-import matching
            let facet_str =
                serde_json::to_string(&title.facet).unwrap_or_else(|_| "\"other\"".to_string());
            let _ = app
                .services
                .download_submissions
                .record_submission(DownloadSubmission {
                    title_id: title.id.clone(),
                    facet: facet_str.trim_matches('"').to_string(),
                    download_client_type: grab.client_type,
                    download_client_item_id: grab.job_id,
                    source_title: source_title.clone(),
                })
                .await;

            // Update wanted item to grabbed
            let grabbed_json = serde_json::json!({
                "title": best.title,
                "score": candidate_score,
                "grabbed_at": now.to_rfc3339(),
            })
            .to_string();

            let _ = app
                .services
                .wanted_items
                .update_wanted_item_status(
                    &item.id,
                    "grabbed",
                    None,
                    Some(&now.to_rfc3339()),
                    item.search_count + 1,
                    item.current_score,
                    Some(&grabbed_json),
                )
                .await;

            let mut grab_meta = HashMap::new();
            grab_meta.insert("title_name".to_string(), serde_json::json!(title.name));
            grab_meta.insert("release_title".to_string(), serde_json::json!(best.title));
            grab_meta.insert("indexer".to_string(), serde_json::json!(best.source));
            grab_meta.insert("score".to_string(), serde_json::json!(candidate_score));
            let grab_envelope = crate::activity::NotificationEnvelope {
                event_type: NotificationEventType::Grab,
                title: format!("Grabbed: {}", title.name),
                body: format!(
                    "'{}' auto-grabbed for {} (score: {})",
                    best.title, title.name, candidate_score
                ),
                facet: Some(format!("{:?}", title.facet).to_lowercase()),
                metadata: grab_meta,
            };
            let _ = app
                .services
                .record_activity_event_with_notification(
                    None,
                    Some(title.id.clone()),
                    ActivityKind::MovieDownloaded,
                    format!("auto-grabbed: {} (score: {})", best.title, candidate_score),
                    ActivitySeverity::Success,
                    vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                    grab_envelope,
                )
                .await;
        }
        Err(err) => {
            warn!(
                title = title.name.as_str(),
                error = %err,
                "auto-grab download submission failed"
            );

            let _ = app
                .services
                .release_attempts
                .record_release_attempt(
                    Some(title.id.clone()),
                    source_hint_for_attempt,
                    source_title_for_attempt,
                    ReleaseDownloadAttemptOutcome::Failed,
                    Some(err.to_string()),
                    source_password,
                )
                .await;

            let _ = app
                .services
                .record_activity_event(
                    None,
                    Some(title.id.clone()),
                    ActivityKind::AcquisitionDownloadFailed,
                    format!("download failed for '{}': {}", title.name, err),
                    ActivitySeverity::Error,
                    vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                )
                .await;

            // Re-queue for immediate re-search so the next cycle tries a different release
            let _ = app
                .services
                .wanted_items
                .update_wanted_item_status(
                    &item.id,
                    "wanted",
                    Some(&now.to_rfc3339()),
                    Some(&now.to_rfc3339()),
                    item.search_count + 1,
                    item.current_score,
                    item.grabbed_release.as_deref(),
                )
                .await;

            info!(
                title = title.name.as_str(),
                wanted_item_id = item.id.as_str(),
                "re-queued wanted item for immediate re-search after download failure"
            );
        }
    }

    Ok(())
}

struct SearchQueryResult {
    queries: Vec<String>,
    imdb_id: Option<String>,
    tvdb_id: Option<String>,
    category: String,
    season: Option<u32>,
    episode: Option<u32>,
}

fn build_search_queries(
    title: &Title,
    item: &WantedItem,
    episode: Option<&Episode>,
    facet_registry: &crate::FacetRegistry,
) -> SearchQueryResult {
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
            // Add alias-based queries for broader search coverage
            for alias in &title.aliases {
                if !alias.is_empty() {
                    let query = if let Some(year) = title.year {
                        format!("{} {}", alias, year)
                    } else {
                        alias.clone()
                    };
                    queries.push(query);
                }
            }
            // Dedup queries (alias may duplicate the primary name)
            let mut seen = std::collections::HashSet::new();
            queries.retain(|q| seen.insert(q.to_ascii_lowercase()));
            if queries.is_empty() && imdb_id.is_some() {
                queries.push(String::new());
            }
            SearchQueryResult {
                queries,
                imdb_id,
                tvdb_id,
                category,
                season: None,
                episode: None,
            }
        }
        "episode" => {
            let mut queries = Vec::new();
            let mut season_param: Option<u32> = None;
            let mut episode_param: Option<u32> = None;

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

                if season_num > 0 {
                    season_param = Some(season_num as u32);
                }
                if episode_num > 0 {
                    episode_param = Some(episode_num as u32);
                }

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

            SearchQueryResult {
                queries,
                imdb_id,
                tvdb_id,
                category,
                season: season_param,
                episode: episode_param,
            }
        }
        _ => SearchQueryResult {
            queries: vec![],
            imdb_id: None,
            tvdb_id: None,
            category,
            season: None,
            episode: None,
        },
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

    pub async fn trigger_title_wanted_search(&self, title_id: &str) -> AppResult<usize> {
        let title = self
            .services
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title not found".to_string()))?;

        let now = Utc::now();
        let queued = if let Some(handler) = self.facet_registry.get(&title.facet) {
            if handler.has_episodes() {
                self.queue_monitored_series_items_for_search(&title, &now)
                    .await?
            } else if title.monitored {
                self.queue_monitored_movie_for_search(&title, &now).await?
            } else {
                0
            }
        } else {
            0
        };

        if queued > 0 {
            self.services.acquisition_wake.notify_one();
        }

        Ok(queued)
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

impl AppUseCase {
    async fn queue_monitored_movie_for_search(
        &self,
        title: &Title,
        now: &DateTime<Utc>,
    ) -> AppResult<usize> {
        let has_file = self
            .services
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .map(|files| !files.is_empty())
            .unwrap_or(false);

        if has_file {
            return Ok(0);
        }

        let next_search_at = now.to_rfc3339();
        if let Some(item) = self
            .services
            .wanted_items
            .get_wanted_item_for_title(&title.id, None)
            .await?
        {
            if item.status == "grabbed" {
                return Ok(0);
            }

            self.services
                .wanted_items
                .update_wanted_item_status(
                    &item.id,
                    "wanted",
                    Some(&next_search_at),
                    item.last_search_at.as_deref(),
                    item.search_count,
                    item.current_score,
                    item.grabbed_release.as_deref(),
                )
                .await?;
            return Ok(1);
        }

        let baseline_date = title.first_aired.clone();
        let schedule = compute_search_schedule("movie", baseline_date.as_deref(), "primary", now);
        let item = WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: None,
            episode_id: None,
            season_number: None,
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

        self.services.wanted_items.upsert_wanted_item(&item).await?;
        Ok(1)
    }

    async fn queue_monitored_series_items_for_search(
        &self,
        title: &Title,
        now: &DateTime<Utc>,
    ) -> AppResult<usize> {
        let collections = self
            .services
            .shows
            .list_collections_for_title(&title.id)
            .await?;

        let existing_files = self
            .services
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default();
        let episodes_with_files: std::collections::HashSet<String> = existing_files
            .iter()
            .filter_map(|file| file.episode_id.clone())
            .collect();
        let next_search_at = now.to_rfc3339();
        let mut queued = 0usize;

        for collection in &collections {
            if !collection.monitored {
                continue;
            }

            let episodes = self
                .services
                .shows
                .list_episodes_for_collection(&collection.id)
                .await?;

            for episode in &episodes {
                if !episode.monitored || episodes_with_files.contains(&episode.id) {
                    continue;
                }

                if let Some(item) = self
                    .services
                    .wanted_items
                    .get_wanted_item_for_title(&title.id, Some(&episode.id))
                    .await?
                {
                    if item.status == "grabbed" {
                        continue;
                    }

                    self.services
                        .wanted_items
                        .update_wanted_item_status(
                            &item.id,
                            "wanted",
                            Some(&next_search_at),
                            item.last_search_at.as_deref(),
                            item.search_count,
                            item.current_score,
                            item.grabbed_release.as_deref(),
                        )
                        .await?;
                    queued += 1;
                    continue;
                }

                let baseline_date = episode.air_date.clone();
                let schedule =
                    compute_search_schedule("episode", baseline_date.as_deref(), "primary", now);
                let item = WantedItem {
                    id: Id::new().0,
                    title_id: title.id.clone(),
                    title_name: None,
                    episode_id: Some(episode.id.clone()),
                    season_number: episode.season_number.clone(),
                    media_type: "episode".to_string(),
                    search_phase: schedule.search_phase,
                    next_search_at: Some(next_search_at.clone()),
                    last_search_at: None,
                    search_count: 0,
                    baseline_date,
                    status: "wanted".to_string(),
                    grabbed_release: None,
                    current_score: None,
                    created_at: now.to_rfc3339(),
                    updated_at: now.to_rfc3339(),
                };

                self.services.wanted_items.upsert_wanted_item(&item).await?;
                queued += 1;
            }
        }

        Ok(queued)
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

    // Run initial health checks after a short delay to let services initialize
    {
        let app = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let results = app.run_health_checks().await;
            *app.services.health_check_results.write().await = results;
            info!("initial health checks completed");
        });
    }

    let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(60));
    let mut sync_interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    let mut metadata_refresh_interval =
        tokio::time::interval(std::time::Duration::from_secs(43200)); // 12h
    let mut registry_refresh_interval =
        tokio::time::interval(std::time::Duration::from_secs(86400)); // 24h
    let mut health_check_interval = tokio::time::interval(std::time::Duration::from_secs(21600)); // 6h
    let mut housekeeping_interval = tokio::time::interval(std::time::Duration::from_secs(86400)); // 24h
    let mut rss_sync_interval = tokio::time::interval(std::time::Duration::from_secs(900)); // 15min
    let mut pending_release_interval = tokio::time::interval(std::time::Duration::from_secs(60)); // 1min

    // Consume the first tick immediately
    poll_interval.tick().await;
    sync_interval.tick().await;
    metadata_refresh_interval.tick().await;
    registry_refresh_interval.tick().await;
    health_check_interval.tick().await;
    housekeeping_interval.tick().await;
    rss_sync_interval.tick().await;
    pending_release_interval.tick().await;

    let wake = app.services.acquisition_wake.clone();

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!("background acquisition poller shutting down");
                break;
            }
            _ = wake.notified() => {
                let t = std::time::Instant::now();
                process_due_wanted_items(&app).await;
                metrics::counter!("scryer_task_runs_total", "task" => "wanted_items").increment(1);
                metrics::histogram!("scryer_task_duration_seconds", "task" => "wanted_items").record(t.elapsed().as_secs_f64());
            }
            _ = poll_interval.tick() => {
                let t = std::time::Instant::now();
                process_due_wanted_items(&app).await;
                metrics::counter!("scryer_task_runs_total", "task" => "wanted_items").increment(1);
                metrics::histogram!("scryer_task_duration_seconds", "task" => "wanted_items").record(t.elapsed().as_secs_f64());
            }
            _ = sync_interval.tick() => {
                let t = std::time::Instant::now();
                if let Err(err) = app.sync_wanted_state().await {
                    warn!(error = %err, "periodic wanted state sync failed");
                    metrics::counter!("scryer_task_errors_total", "task" => "sync_state").increment(1);
                }
                metrics::counter!("scryer_task_runs_total", "task" => "sync_state").increment(1);
                metrics::histogram!("scryer_task_duration_seconds", "task" => "sync_state").record(t.elapsed().as_secs_f64());
            }
            _ = metadata_refresh_interval.tick() => {
                let t = std::time::Instant::now();
                info!("starting periodic metadata refresh for monitored series");
                app.refresh_monitored_series_metadata().await;
                metrics::counter!("scryer_task_runs_total", "task" => "metadata_refresh").increment(1);
                metrics::histogram!("scryer_task_duration_seconds", "task" => "metadata_refresh").record(t.elapsed().as_secs_f64());
            }
            _ = registry_refresh_interval.tick() => {
                let t = std::time::Instant::now();
                info!("refreshing plugin registry");
                if let Err(e) = app.refresh_plugin_registry_internal().await {
                    warn!(error = %e, "periodic plugin registry refresh failed");
                    metrics::counter!("scryer_task_errors_total", "task" => "registry_refresh").increment(1);
                }
                metrics::counter!("scryer_task_runs_total", "task" => "registry_refresh").increment(1);
                metrics::histogram!("scryer_task_duration_seconds", "task" => "registry_refresh").record(t.elapsed().as_secs_f64());
            }
            _ = health_check_interval.tick() => {
                let t = std::time::Instant::now();
                let results = app.run_health_checks().await;
                *app.services.health_check_results.write().await = results;
                info!("periodic health checks completed");
                metrics::counter!("scryer_task_runs_total", "task" => "health_check").increment(1);
                metrics::histogram!("scryer_task_duration_seconds", "task" => "health_check").record(t.elapsed().as_secs_f64());
            }
            _ = housekeeping_interval.tick() => {
                let t = std::time::Instant::now();
                match app.run_housekeeping().await {
                    Ok(report) => info!(
                        orphaned_media_files = report.orphaned_media_files,
                        stale_release_decisions = report.stale_release_decisions,
                        stale_release_attempts = report.stale_release_attempts,
                        expired_event_outboxes = report.expired_event_outboxes,
                        stale_history_events = report.stale_history_events,
                        "periodic housekeeping completed"
                    ),
                    Err(e) => {
                        warn!(error = %e, "periodic housekeeping failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "housekeeping").increment(1);
                    }
                }
                if let Err(e) = app.auto_backup_if_due().await {
                    warn!(error = %e, "auto-backup failed");
                }
                metrics::counter!("scryer_task_runs_total", "task" => "housekeeping").increment(1);
                metrics::histogram!("scryer_task_duration_seconds", "task" => "housekeeping").record(t.elapsed().as_secs_f64());
            }
            _ = pending_release_interval.tick() => {
                let t = std::time::Instant::now();
                match app.process_expired_pending_releases().await {
                    Ok(grabbed) => {
                        if grabbed > 0 {
                            info!(grabbed, "pending release processor: grabbed expired releases");
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "pending release processor failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "pending_releases").increment(1);
                    }
                }
                metrics::counter!("scryer_task_runs_total", "task" => "pending_releases").increment(1);
                metrics::histogram!("scryer_task_duration_seconds", "task" => "pending_releases").record(t.elapsed().as_secs_f64());
            }
            _ = rss_sync_interval.tick() => {
                let t = std::time::Instant::now();
                match app.run_rss_sync().await {
                    Ok(report) => {
                        if report.releases_fetched > 0 || report.releases_grabbed > 0 || report.releases_held > 0 {
                            info!(
                                fetched = report.releases_fetched,
                                matched = report.releases_matched,
                                grabbed = report.releases_grabbed,
                                held = report.releases_held,
                                "periodic RSS sync completed"
                            );
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "periodic RSS sync failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "rss_sync").increment(1);
                    }
                }
                metrics::counter!("scryer_task_runs_total", "task" => "rss_sync").increment(1);
                metrics::histogram!("scryer_task_duration_seconds", "task" => "rss_sync").record(t.elapsed().as_secs_f64());
            }
        }
    }
}

/// Determine whether a movie has reached its configured availability threshold.
///
/// Returns `true` if the movie should be included in acquisition searches,
/// `false` if it should be skipped because its release dates haven't passed yet.
pub(crate) fn is_movie_available_for_acquisition(
    title: &Title,
    availability: &str,
    now: &DateTime<Utc>,
) -> bool {
    match availability {
        "in_cinemas" => title
            .first_aired
            .as_deref()
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .map(|date| date <= now.date_naive())
            .unwrap_or(false),
        "released" => {
            if let Some(ref digital) = title.digital_release_date {
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
            }
        }
        // "announced" or anything else: always search
        _ => true,
    }
}

#[cfg(test)]
#[path = "app_usecase_acquisition_tests.rs"]
mod app_usecase_acquisition_tests;
