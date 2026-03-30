use super::*;
use crate::acquisition_policy::{AcquisitionThresholds, compute_search_schedule, evaluate_upgrade};
use crate::types::PendingReleaseStatus;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use scryer_domain::NotificationEventType;
use std::collections::HashMap;
use tracing::{info, trace, warn};

const FAILED_GRAB_OLD_TITLE_DAYS: i64 = 14;
const FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES: i64 = 20;
const MAX_STANDBY_CANDIDATES_PER_WANTED_ITEM: usize = 5;
const STANDBY_RETENTION_HOURS: i64 = 24;

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
            collection_id: None,
            season_number: None,
            media_type: "movie".to_string(),
            search_phase: schedule.search_phase.to_string(),
            next_search_at: Some(next_search_at),
            last_search_at: None,
            search_count: 0,
            baseline_date,
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
        };

        match self
            .services
            .wanted_items
            .ensure_wanted_item_seeded(&item)
            .await
        {
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
                    collection_id: None,
                    season_number: episode.season_number.clone(),
                    media_type: "episode".to_string(),
                    search_phase: schedule.search_phase.to_string(),
                    next_search_at: Some(next_search_at),
                    last_search_at: None,
                    search_count: 0,
                    baseline_date,
                    status: WantedStatus::Wanted,
                    grabbed_release: None,
                    current_score: None,
                    created_at: now.to_rfc3339(),
                    updated_at: now.to_rfc3339(),
                };

                if let Err(err) = self
                    .services
                    .wanted_items
                    .ensure_wanted_item_seeded(&item)
                    .await
                {
                    warn!(
                        title_id = title.id.as_str(),
                        episode_id = episode.id.as_str(),
                        error = %err,
                        "failed to upsert wanted item for episode"
                    );
                }
            }
        }

        // Generate wanted items for interstitial anime movies (franchise movies stored in Season 00)
        if title.facet == scryer_domain::MediaFacet::Anime {
            for collection in &collections {
                if collection.collection_type != CollectionType::Interstitial
                    || !collection.monitored
                {
                    continue;
                }
                // Skip if already has a file on disk
                if collection.ordered_path.is_some() {
                    continue;
                }
                let Some(ref movie) = collection.interstitial_movie else {
                    continue;
                };
                // Skip filler movies unless the user opted in
                if movie.continuity_status.as_deref() == Some("filler") {
                    let monitor_filler = self
                        .read_setting_string_value("anime.monitor_filler_movies", None)
                        .await
                        .ok()
                        .flatten()
                        .as_deref()
                        == Some("true");
                    if !monitor_filler {
                        continue;
                    }
                }

                // Skip if the movie already exists as a separate Movie facet title
                // (prevents downloading the same movie twice)
                if (!movie.imdb_id.is_empty() || movie.movie_tmdb_id.is_some())
                    && let Ok(all_titles) = self.services.titles.list(None, None).await
                {
                    let already_exists = all_titles.iter().any(|t| {
                        t.facet == scryer_domain::MediaFacet::Movie
                            && ((!movie.imdb_id.is_empty()
                                && t.imdb_id.as_deref() == Some(&movie.imdb_id))
                                || movie.movie_tmdb_id.as_deref().is_some_and(|tmdb| {
                                    t.external_ids
                                        .iter()
                                        .any(|eid| eid.source == "tmdb" && eid.value == tmdb)
                                }))
                    });
                    if already_exists {
                        trace!(
                            movie_name = movie.name.as_str(),
                            "skipping interstitial wanted item: movie exists as separate title"
                        );
                        continue;
                    }
                }

                let baseline_date = movie.digital_release_date.clone();
                let schedule =
                    compute_search_schedule("movie", baseline_date.as_deref(), "primary", now);

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
                    collection_id: Some(collection.id.clone()),
                    season_number: Some("0".to_string()),
                    media_type: "interstitial_movie".to_string(),
                    search_phase: schedule.search_phase.to_string(),
                    next_search_at: Some(next_search_at),
                    last_search_at: None,
                    search_count: 0,
                    baseline_date,
                    status: WantedStatus::Wanted,
                    grabbed_release: None,
                    current_score: None,
                    created_at: now.to_rfc3339(),
                    updated_at: now.to_rfc3339(),
                };

                if let Err(err) = self
                    .services
                    .wanted_items
                    .ensure_wanted_item_seeded(&item)
                    .await
                {
                    warn!(
                        title_id = title.id.as_str(),
                        collection_id = collection.id.as_str(),
                        movie_name = movie.name.as_str(),
                        error = %err,
                        "failed to upsert wanted item for interstitial movie"
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
    /// Download client item IDs of items currently queued/downloading.
    /// Used for episode-level dedup (check by submission ID, not title name).
    active_client_ids: std::collections::HashSet<String>,
    /// Download client item IDs of items that completed successfully.
    completed_client_ids: std::collections::HashSet<String>,
    /// Failed history items keyed by download client job ID (NZBGet NZBID,
    /// SABnzbd nzo_id, Weaver job UUID). Matched against `download_submissions`
    /// table to find which scryer title a failed download belongs to.
    failed_by_download_id: std::collections::HashMap<String, FailedDownloadSnapshot>,
}

#[derive(Clone, Debug)]
pub(crate) struct FailedDownloadSnapshot {
    reason: String,
    download_client_item_id: String,
    client_id: String,
}

#[derive(Clone, Debug)]
pub(crate) struct DownloadFailureContext {
    pub wanted_item: Option<WantedItem>,
    pub title_id: Option<String>,
    pub client_id: String,
    pub client_item_id: String,
    pub release_title: String,
    pub reason: String,
    pub remove_from_client_if_configured: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureHandlingOutcome {
    RecoveredFromStandby,
    RequeuedFreshSearch,
    RequeuedDeferred,
    RecordedOnly,
}

impl DownloadClientSnapshot {
    pub(crate) async fn fetch(app: &AppUseCase) -> Self {
        let mut active_titles = std::collections::HashSet::new();
        let mut active_client_ids = std::collections::HashSet::new();
        let mut completed_client_ids = std::collections::HashSet::new();
        let mut failed_by_download_id = std::collections::HashMap::new();

        // Fetch current queue
        if let Ok(queue) = app.services.download_client.list_queue().await {
            for item in &queue {
                match item.state {
                    DownloadQueueState::Queued
                    | DownloadQueueState::Downloading
                    | DownloadQueueState::Paused => {
                        active_titles.insert(item.title_name.to_ascii_lowercase());
                        active_client_ids.insert(item.download_client_item_id.clone());
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

        // Fetch recent history — key by download client job ID (works across all
        // clients: NZBGet, SABnzbd, Weaver).
        if let Ok(history) = app.services.download_client.list_history().await {
            for item in &history {
                if item.state == DownloadQueueState::Completed {
                    completed_client_ids.insert(item.download_client_item_id.clone());
                } else if item.state == DownloadQueueState::Failed {
                    let reason = item
                        .attention_reason
                        .as_deref()
                        .unwrap_or("unknown")
                        .to_ascii_uppercase();
                    failed_by_download_id.insert(
                        item.download_client_item_id.clone(),
                        FailedDownloadSnapshot {
                            reason,
                            download_client_item_id: item.download_client_item_id.clone(),
                            client_id: item.client_id.clone(),
                        },
                    );
                }
            }
            if !failed_by_download_id.is_empty() {
                info!(
                    failed_count = failed_by_download_id.len(),
                    "download client snapshot: failed history items"
                );
            }
        }

        Self {
            active_titles,
            active_client_ids,
            completed_client_ids,
            failed_by_download_id,
        }
    }

    /// Returns true if a release with this title is currently queued/downloading.
    pub(crate) fn is_active(&self, release_title: &str) -> bool {
        self.active_titles
            .contains(&release_title.to_ascii_lowercase())
    }

    /// If a download with this job ID failed in history with a blocklist-worthy
    /// reason, returns the failure snapshot.
    pub(crate) fn failed_item(
        &self,
        download_client_item_id: &str,
    ) -> Option<&FailedDownloadSnapshot> {
        self.failed_by_download_id.get(download_client_item_id)
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
        info!("check_grabbed_for_failures: no grabbed wanted items");
        return;
    }

    info!(
        count = grabbed_items.len(),
        "check_grabbed_for_failures: checking grabbed wanted items against download client"
    );

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
        let submissions = app
            .services
            .download_submissions
            .list_for_title(&item.title_id)
            .await
            .unwrap_or_default();

        info!(
            title_id = item.title_id.as_str(),
            release = release_title.as_str(),
            submission_count = submissions.len(),
            submission_ids = ?submissions.iter().map(|s| s.download_client_item_id.as_str()).collect::<Vec<_>>(),
            "check_grabbed_for_failures: looking up submissions for grabbed item"
        );

        let failed = submissions.iter().find_map(|sub| {
            dl_snapshot
                .failed_item(&sub.download_client_item_id)
                .map(|f| (f, sub.source_title.clone()))
        });

        if let Some((failed_item, _source_title)) = failed {
            warn!(
                title_id = item.title_id.as_str(),
                release = release_title.as_str(),
                reason = failed_item.reason.as_str(),
                "grabbed release failed in download client"
            );

            let _ = process_download_failure(
                app,
                DownloadFailureContext {
                    wanted_item: Some(item.clone()),
                    title_id: Some(item.title_id.clone()),
                    client_id: failed_item.client_id.clone(),
                    client_item_id: failed_item.download_client_item_id.clone(),
                    release_title: release_title.clone(),
                    reason: failed_item.reason.clone(),
                    remove_from_client_if_configured: true,
                },
                Some(dl_snapshot),
            )
            .await;
        }
    }
}

pub(crate) async fn process_download_failure(
    app: &AppUseCase,
    context: DownloadFailureContext,
    snapshot: Option<&DownloadClientSnapshot>,
) -> FailureHandlingOutcome {
    let resolved_title_id = context
        .wanted_item
        .as_ref()
        .map(|item| item.title_id.clone())
        .or(context.title_id.clone());

    if let Some(title_id) = resolved_title_id.clone() {
        let hint = normalize_release_attempt_hint(None);
        let rel_title = normalize_release_attempt_title(Some(&context.release_title));
        let failure_message = format!("download client failure: {}", context.reason);

        let _ = app
            .services
            .release_attempts
            .record_release_attempt(
                Some(title_id.clone()),
                hint,
                rel_title,
                ReleaseDownloadAttemptOutcome::Failed,
                Some(failure_message.clone()),
                None,
            )
            .await;

        let _ = app
            .services
            .blocklist_repo
            .add(&NewBlocklistEntry {
                title_id,
                source_title: Some(context.release_title.clone()),
                source_hint: None,
                quality: None,
                download_id: Some(context.client_item_id.clone()),
                reason: Some(failure_message),
                data: Default::default(),
            })
            .await;
    }

    let wanted_item = match context.wanted_item.clone() {
        Some(item) => Some(item),
        None => {
            resolve_failure_wanted_item(app, resolved_title_id.as_deref(), &context.release_title)
                .await
        }
    };

    let outcome = if let Some(item) = wanted_item.as_ref() {
        let now = Utc::now();
        let owned_snapshot = if snapshot.is_none() {
            Some(DownloadClientSnapshot::fetch(app).await)
        } else {
            None
        };
        let active_snapshot = snapshot.or(owned_snapshot.as_ref());

        if let Some(active_snapshot) = active_snapshot {
            if recover_from_standby_candidates(
                app,
                item,
                &context.release_title,
                active_snapshot,
                &now,
            )
            .await
            {
                FailureHandlingOutcome::RecoveredFromStandby
            } else {
                let immediate_research = should_research_failed_grab(item, &now);
                let next_search_at = if immediate_research {
                    now.to_rfc3339()
                } else {
                    (now + Duration::minutes(FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES)).to_rfc3339()
                };

                let _ = app
                    .services
                    .wanted_items
                    .schedule_wanted_item_search(&WantedSearchTransition {
                        id: item.id.clone(),
                        next_search_at: Some(next_search_at),
                        last_search_at: item.last_search_at.clone(),
                        search_count: item.search_count,
                        current_score: item.current_score,
                        grabbed_release: None,
                    })
                    .await;

                let message = if immediate_research {
                    format!(
                        "download failed for '{}'; standby exhausted, re-queuing for fresh search",
                        context.release_title
                    )
                } else {
                    format!(
                        "download failed for '{}'; standby exhausted, deferring reacquisition",
                        context.release_title
                    )
                };

                let _ = app
                    .services
                    .record_activity_event(
                        None,
                        Some(item.title_id.clone()),
                        None,
                        ActivityKind::AcquisitionDownloadFailed,
                        message,
                        ActivitySeverity::Warning,
                        vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                    )
                    .await;

                if immediate_research {
                    FailureHandlingOutcome::RequeuedFreshSearch
                } else {
                    FailureHandlingOutcome::RequeuedDeferred
                }
            }
        } else {
            FailureHandlingOutcome::RecordedOnly
        }
    } else {
        let _ = app
            .services
            .record_activity_event(
                None,
                resolved_title_id.clone(),
                None,
                ActivityKind::AcquisitionDownloadFailed,
                format!(
                    "Download failed: {} — {}",
                    context.release_title, context.reason
                ),
                ActivitySeverity::Error,
                vec![ActivityChannel::WebUi, ActivityChannel::Toast],
            )
            .await;
        FailureHandlingOutcome::RecordedOnly
    };

    if context.remove_from_client_if_configured
        && let Some(title_id) = resolved_title_id.as_deref()
        && let Ok(Some(title)) = app.services.titles.get_by_id(title_id).await
        && app
            .should_remove_failed_download(&title.facet, &context.client_id)
            .await
        && let Err(error) = app
            .services
            .download_client
            .delete_queue_item(&context.client_item_id, true)
            .await
    {
        warn!(
            title_id,
            client_id = context.client_id.as_str(),
            download_client_item_id = context.client_item_id.as_str(),
            error = %error,
            "failed to delete failed download from client history"
        );
    }

    let _ = app
        .services
        .download_submissions
        .delete_by_client_item_id(&context.client_item_id)
        .await;

    outcome
}

async fn resolve_failure_wanted_item(
    app: &AppUseCase,
    title_id: Option<&str>,
    release_title: &str,
) -> Option<WantedItem> {
    let title_id = title_id?.trim();
    if title_id.is_empty() {
        return None;
    }

    let grabbed_items = app
        .services
        .wanted_items
        .list_wanted_items(Some("grabbed"), None, Some(title_id), 25, 0)
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

fn extract_grabbed_release_title(raw: Option<&str>) -> Option<String> {
    raw.and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|value| {
            value
                .get("title")
                .and_then(|title| title.as_str())
                .map(str::to_string)
        })
}

/// Process due wanted items: search indexers and auto-grab best releases.
async fn process_due_wanted_items(app: &AppUseCase) {
    prune_standby_candidates(app).await;

    // Check for download failures first — re-queues failed items with
    // next_search_at=NOW so they appear in the due list below.
    let dl_snapshot = DownloadClientSnapshot::fetch(app).await;
    check_grabbed_for_failures(app, &dl_snapshot).await;

    // Capture `now` AFTER failure check so that items just re-queued
    // are guaranteed to satisfy `next_search_at <= now`.
    let now = Utc::now();
    let now_str = now.to_rfc3339();

    let batch_size = match app.acquisition_settings().await {
        Ok(settings) => settings.batch_size.clamp(1, 500) as i64,
        Err(err) => {
            warn!(error = %err, "failed to load acquisition settings, using default batch size");
            50
        }
    };

    let due_items = match app
        .services
        .wanted_items
        .list_due_wanted_items(&now_str, batch_size)
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
        if item.media_type == "episode"
            && let Some(sn) = item.season_number.as_deref()
            && let Ok(n) = sn.parse::<u32>()
            && n > 0
        {
            *season_due_counts
                .entry((item.title_id.clone(), n))
                .or_insert(0) += 1;
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

        // Re-read the wanted item status after processing.  If the item was
        // successfully grabbed inside process_single_wanted_item (status changed
        // to "grabbed"), we must NOT overwrite it with a search schedule — doing
        // so would reset it to "wanted" and prevent check_grabbed_for_failures
        // from ever detecting download failures.
        let current = app
            .services
            .wanted_items
            .get_wanted_item_by_id(&item.id)
            .await
            .ok()
            .flatten();

        if let Some(ref wi) = current
            && wi.status == WantedStatus::Grabbed
        {
            // Item was grabbed — don't touch it.  The download failure
            // detector will handle re-queuing if the download fails.
            continue;
        }

        // Item is still "wanted" (no grab succeeded, or all candidates were
        // exhausted).  Update the search schedule with backoff.
        let schedule = compute_search_schedule(
            &item.media_type,
            item.baseline_date.as_deref(),
            &item.search_phase,
            &now,
        );

        let _ = app
            .services
            .wanted_items
            .schedule_wanted_item_search(&WantedSearchTransition {
                id: item.id.clone(),
                next_search_at: Some(schedule.next_search_at),
                last_search_at: Some(now.to_rfc3339()),
                search_count: item.search_count + 1,
                current_score: item.current_score,
                grabbed_release: item.grabbed_release.clone(),
            })
            .await;
    }
}

async fn prune_standby_candidates(app: &AppUseCase) {
    let all_standby = app
        .services
        .pending_releases
        .list_all_standby_pending_releases()
        .await
        .unwrap_or_default();

    if all_standby.is_empty() {
        return;
    }

    let now = Utc::now();
    let cutoff = now - Duration::hours(STANDBY_RETENTION_HOURS);
    let mut grouped: std::collections::HashMap<String, Vec<PendingRelease>> =
        std::collections::HashMap::new();
    for release in all_standby {
        grouped
            .entry(release.wanted_item_id.clone())
            .or_default()
            .push(release);
    }

    for (wanted_item_id, mut releases) in grouped {
        let wanted = app
            .services
            .wanted_items
            .get_wanted_item_by_id(&wanted_item_id)
            .await
            .ok()
            .flatten();

        let Some(wanted) = wanted else {
            let _ = app
                .services
                .pending_releases
                .delete_standby_pending_releases_for_wanted_item(&wanted_item_id)
                .await;
            continue;
        };

        if wanted.status != WantedStatus::Grabbed {
            let _ = app
                .services
                .pending_releases
                .delete_standby_pending_releases_for_wanted_item(&wanted_item_id)
                .await;
            continue;
        }

        releases.sort_by(|left, right| right.added_at.cmp(&left.added_at));
        for (index, release) in releases.iter().enumerate() {
            let added_at = crate::quality_profile::parse_published_at(&release.added_at);
            let is_stale = added_at.is_none_or(|added_at| added_at < cutoff);
            let is_overflow = index >= MAX_STANDBY_CANDIDATES_PER_WANTED_ITEM;
            if is_stale || is_overflow {
                let _ = app
                    .services
                    .pending_releases
                    .update_pending_release_status(&release.id, PendingReleaseStatus::Expired, None)
                    .await;
            }
        }
    }
}

impl AppUseCase {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn run_acquisition_cycle_once(&self) {
        process_due_wanted_items(self).await;
    }
}

/// Returns true if the error indicates ALL download clients for a given
/// source kind have been exhausted (network down, auth failed, etc.).
///
/// The download client router (`router.rs`) tries each enabled client in
/// priority order.  Per-client `Repository` errors trigger failover to the
/// next client.  When every client has been tried, the router returns this
/// aggregate error.  Other `AppError` variants (`Validation`, etc.) are
/// release-specific and don't imply infrastructure failure.
fn is_all_clients_failed_error(err: &AppError) -> bool {
    matches!(err, AppError::Repository(msg) if msg.contains("all prioritized download clients failed"))
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

    // Episode-level gate: skip if a download for this title is already active
    // or completed in the download client.  Prevents grab spirals where multiple
    // releases for the same episode are grabbed simultaneously.
    let submissions = app
        .services
        .download_submissions
        .list_for_title(&item.title_id)
        .await
        .unwrap_or_default();

    let has_active_or_completed = submissions.iter().any(|sub| {
        dl_snapshot
            .active_client_ids
            .contains(&sub.download_client_item_id)
            || dl_snapshot
                .completed_client_ids
                .contains(&sub.download_client_item_id)
    });

    if has_active_or_completed {
        info!(
            title = title.name.as_str(),
            "skipping search — download for this title is already active or completed"
        );
        return Ok(());
    }

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

    // For interstitial movies, build a synthetic title from the collection's movie metadata
    // so the search uses the movie's name/year/IMDB ID instead of the parent series'
    let search_title = if item.media_type == "interstitial_movie" {
        if let Some(ref coll_id) = item.collection_id
            && let Ok(Some(collection)) = app.services.shows.get_collection_by_id(coll_id).await
            && let Some(ref movie) = collection.interstitial_movie
        {
            let mut t = title.clone();
            t.name = movie.name.clone();
            t.year = movie.year;
            t.imdb_id = if movie.imdb_id.is_empty() {
                None
            } else {
                Some(movie.imdb_id.clone())
            };
            // Add TMDB ID as external ID for indexer search
            if let Some(ref tmdb_id) = movie.movie_tmdb_id {
                t.external_ids.retain(|e| e.source != "tmdb");
                t.external_ids.push(scryer_domain::ExternalId {
                    source: "tmdb".into(),
                    value: tmdb_id.clone(),
                });
            }
            // Add AniDB ID as external ID for AnimeTosho searches
            if let Some(ref anidb_id) = movie.movie_anidb_id {
                t.external_ids.retain(|e| e.source != "anidb");
                t.external_ids.push(scryer_domain::ExternalId {
                    source: "anidb".into(),
                    value: anidb_id.clone(),
                });
            }
            t.aliases = vec![];
            t
        } else {
            title.clone()
        }
    } else {
        title.clone()
    };

    // Resolve episode-specific anidb_id from anibridge (e.g. Bleach S17E08 → 15449)
    let search_title = if item.media_type == "episode" {
        if let Some(ref ep) = episode {
            if let (Ok(s), Ok(e)) = (
                ep.season_number.as_deref().unwrap_or("").parse::<i32>(),
                ep.episode_number.as_deref().unwrap_or("").parse::<i32>(),
            ) {
                if let Some(tvdb_str) = tvdb_id_from_external_ids(&search_title.external_ids) {
                    if let Ok(tvdb_num) = tvdb_str.parse::<i64>() {
                        if let Ok(mappings) = app
                            .services
                            .metadata_gateway
                            .anibridge_mappings_for_episode(tvdb_num, s, e)
                            .await
                        {
                            if let Some(m) = mappings
                                .iter()
                                .find(|m| m.source_type == "anidb" && m.source_scope == "R")
                            {
                                let mut t = search_title;
                                t.external_ids.retain(|e| e.source != "anidb");
                                t.external_ids.push(scryer_domain::ExternalId {
                                    source: "anidb".into(),
                                    value: m.source_id.to_string(),
                                });
                                t
                            } else {
                                search_title
                            }
                        } else {
                            search_title
                        }
                    } else {
                        search_title
                    }
                } else {
                    search_title
                }
            } else {
                search_title
            }
        } else {
            search_title
        }
    } else {
        search_title
    };

    // Build search queries based on media type
    let sq = build_search_queries(&search_title, item, episode.as_ref(), &app.facet_registry);
    let (queries, imdb_id, tvdb_id, anidb_id, category) =
        (sq.queries, sq.imdb_id, sq.tvdb_id, sq.anidb_id, sq.category);
    let (search_season, search_episode) = (sq.season, sq.episode);

    // Derive the download client category separately — search_category ("series")
    // is for Newznab query type, download_category ("tv") is for NZBGet routing.
    //
    // ── Season pack priority ──────────────────────────────────────────────────
    // For episode wanted items, try a season pack search first. Season packs are
    // a first-class release type on Usenet and are more efficient than individual
    // episodes. Individual episode searches only run if no season pack was found
    // this cycle for this (title, season).
    if item.media_type == "episode"
        && let Some(season_num) = search_season
    {
        let season_key = (title.id.clone(), season_num);

        // Only attempt a season pack search when >= 2 episodes from this season
        // are due this cycle (mirrors Sonarr: count > 1 missing → SeasonSearchCriteria).
        let due_count = season_due_counts.get(&season_key).copied().unwrap_or(0);

        if due_count >= 2 && !season_pack_attempted.contains(&season_key) {
            season_pack_attempted.insert(season_key.clone());

            let pack_queries = vec![format!("{} S{:0>2}", title.name, season_num)];

            // Load season episodes for runtime scoring and upgrade checking.
            let season_episodes = if let Some(ref coll_id) = item.collection_id {
                app.services
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

            let pack_results = app
                .search_and_score_releases(
                    pack_queries,
                    imdb_id.clone(),
                    tvdb_id.clone(),
                    anidb_id.clone(),
                    Some(category.clone()),
                    Some(title.facet.as_str().to_string()),
                    &title.tags,
                    "background_acquisition_season_pack",
                    SearchMode::Auto,
                    pack_runtime,
                    Some(season_num),
                    None, // episode=None signals a season pack search
                    None, // no absolute episode for season packs
                    &title.tagged_aliases,
                )
                .await
                .unwrap_or_default();

            let title_normalized = crate::app_usecase_rss::normalize_for_matching(&title.name);
            if let Some(best_pack) = pack_results.iter().find(|r| {
                let parsed = crate::parse_release_metadata(&r.title);
                let is_pack = parsed.episode.as_ref().is_some_and(|episode| {
                    episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
                        && episode.season == Some(season_num)
                        && !episode.is_season_extra
                });

                r.quality_profile_decision
                    .as_ref()
                    .map(|d| d.allowed)
                    .unwrap_or(false)
                    && is_pack
                    && crate::app_usecase_rss::normalize_for_matching(&r.title)
                        .contains(&title_normalized)
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
                        .media_files
                        .list_media_files_for_title(&title.id)
                        .await
                        .unwrap_or_default();

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

                        let grab_result = app
                            .services
                            .download_client
                            .submit_download(&DownloadClientAddRequest {
                                title: title.clone(),
                                source_hint: pack_url.clone(),
                                staged_nzb: None,
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
                                        collection_id: None,
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
                                        None,
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
                } // close else (pack not dominated)
            }
        }

        // If a season pack was grabbed this cycle (by this item or an earlier
        // item for the same season), skip the individual episode search.
        if season_pack_grabbed.contains(&season_key) {
            return Ok(());
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
            anidb_id,
            Some(category.clone()),
            Some(title.facet.as_str().to_string()),
            &title.tags,
            "background_acquisition",
            SearchMode::Auto,
            runtime_minutes,
            search_season,
            search_episode,
            absolute_episode,
            &title.tagged_aliases,
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
            None,
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

    // Resolve quality profile once (used by upgrade evaluation for each candidate).
    let profile = app
        .resolve_quality_profile(
            &title.tags,
            title.imdb_id.as_deref(),
            tvdb_id_from_external_ids(&title.external_ids).as_deref(),
            Some(&category),
        )
        .await
        .unwrap_or_else(|_| crate::quality_profile::default_quality_profile_for_search());

    // Cutoff tier check — skip upgrades if the existing file meets the cutoff quality.
    // This is independent of any candidate and can short-circuit before the loop.
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

    let thresholds = app
        .acquisition_thresholds(&profile.criteria.scoring_persona)
        .await;

    // Load existing media files for repack group validation.
    let existing_files = app
        .services
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap_or_default();

    let delay_profiles = app.load_delay_profiles().await;

    // ── Candidate fallthrough loop ──────────────────────────────────────────
    // Iterate ranked candidates (sorted by preference_score DESC).  If a grab
    // fails, try the next candidate instead of re-searching from scratch next
    // cycle.  Mirrors Sonarr's ProcessDownloadDecisions loop.
    let mut had_allowed_candidate = false;
    let mut skipped_for_failed = false;
    let mut grab_attempts: usize = 0;
    // Track source kinds where ALL download clients failed.  Avoids hammering
    // dead clients with more candidates of the same protocol.
    let mut failed_source_kinds: Vec<DownloadSourceKind> = Vec::new();

    let title_norm = crate::app_usecase_rss::normalize_for_matching(&title.name);

    for (candidate_index, candidate) in results.iter().enumerate() {
        let is_allowed = candidate
            .quality_profile_decision
            .as_ref()
            .map(|d| d.allowed)
            .unwrap_or(false);
        if !is_allowed {
            continue;
        }

        // Reject releases whose title doesn't contain the target title name.
        // Prevents false matches from RSS feeds returning unrelated releases.
        if !crate::app_usecase_rss::normalize_for_matching(&candidate.title).contains(&title_norm) {
            continue;
        }

        // Negative score lower bound — candidates are sorted by score descending,
        // so once we see a negative score ALL remaining candidates are also negative.
        // A negative total means penalties outweigh bonuses; not worth grabbing.
        let candidate_score = candidate
            .quality_profile_decision
            .as_ref()
            .map(|d| d.preference_score)
            .unwrap_or(0);
        if candidate_score < 0 {
            info!(
                title_id = title.id.as_str(),
                score = candidate_score,
                "remaining candidates have negative scores, stopping candidate evaluation"
            );
            break;
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

        if db_blocklist.contains(&candidate.title.to_ascii_lowercase()) {
            warn!(
                title = title.name.as_str(),
                release = candidate.title.as_str(),
                "skipping DB-blocklisted release"
            );
            skipped_for_failed = true;
            continue;
        }

        // Skip candidates whose source kind already failed this cycle (all
        // download clients for that protocol are unavailable).
        if let Some(sk) = candidate.source_kind
            && failed_source_kinds.contains(&sk)
        {
            info!(
                title = title.name.as_str(),
                release = candidate.title.as_str(),
                source_kind = ?sk,
                "skipping candidate — source kind failed earlier this cycle"
            );
            continue;
        }

        // ── Upgrade evaluation ──────────────────────────────────────────────
        let decision = evaluate_upgrade(
            candidate_score,
            item.current_score,
            profile.criteria.allow_upgrades,
            item.last_search_at.as_deref(),
            now,
            &thresholds,
            profile.criteria.min_score_to_grab,
        );

        // Record the decision for every candidate we evaluate.
        let decision_record = ReleaseDecision {
            id: Id::new().0,
            wanted_item_id: item.id.clone(),
            title_id: title.id.clone(),
            release_title: candidate.title.clone(),
            release_url: candidate
                .download_url
                .clone()
                .or_else(|| candidate.link.clone()),
            release_size_bytes: candidate.size_bytes,
            decision_code: decision.code().to_string(),
            candidate_score,
            current_score: item.current_score,
            score_delta: item.current_score.map(|c| candidate_score - c),
            explanation_json: candidate.quality_profile_decision.as_ref().map(|d| {
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
                    None,
                    ActivityKind::AcquisitionCandidateRejected,
                    format!(
                        "{}: '{}' ({})",
                        decision.code(),
                        candidate.title,
                        title.name
                    ),
                    ActivitySeverity::Info,
                    vec![ActivityChannel::WebUi],
                )
                .await;
            // Upgrade policy rejection is quality-based.  Candidates are sorted
            // by score descending, so no lower-scored candidate can satisfy a
            // stricter delta requirement.  Stop the loop entirely.
            break;
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

        // Deduplicate: skip if this exact URL was already submitted this cycle.
        if let Some(url) = source_hint.as_deref()
            && !grabbed_urls.insert(url.to_string())
        {
            info!(
                title = title.name.as_str(),
                release = candidate.title.as_str(),
                "skipping duplicate release already submitted this cycle"
            );
            // Mark this wanted item as grabbed too since the release covers it
            let grabbed_json = serde_json::json!({
                "title": candidate.title,
                "score": candidate_score,
                "grabbed_at": now.to_rfc3339(),
                "deduplicated": true,
            })
            .to_string();
            let _ = app
                .services
                .wanted_items
                .transition_wanted_to_grabbed(&WantedGrabTransition {
                    id: item.id.clone(),
                    last_search_at: Some(now.to_rfc3339()),
                    search_count: item.search_count + 1,
                    current_score: item.current_score,
                    grabbed_release: grabbed_json,
                })
                .await;
            return Ok(());
        }

        let source_title = Some(candidate.title.clone());
        let source_hint_for_attempt = normalize_release_attempt_hint(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_title(source_title.as_deref());
        let source_password = normalize_release_password(candidate.password_hint.as_deref());

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

        // Skip repacks from a different release group than the existing file.
        if crate::acquisition_policy::should_skip_repack_group_mismatch(
            candidate,
            &existing_files,
            item.episode_id.as_deref(),
        ) {
            info!(
                title = title.name.as_str(),
                release = candidate.title.as_str(),
                "skipping repack — release group doesn't match existing file"
            );
            continue;
        }

        if let Some(delay_decision) = crate::delay_profile::resolve_delay_decision(
            &delay_profiles,
            &title.tags,
            &title.facet,
            candidate.source_kind,
            candidate
                .published_at
                .as_deref()
                .and_then(crate::quality_profile::parse_published_at),
            candidate_score,
            now,
        ) && delay_decision.should_hold()
        {
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
                delay_decision.effective_delay_minutes,
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

        let is_recent = app.is_recent_for_queue_priority(
            candidate
                .published_at
                .as_deref()
                .or(episode.as_ref().and_then(|item| item.air_date.as_deref()))
                .or(item.baseline_date.as_deref())
                .or(title.first_aired.as_deref())
                .or(title.digital_release_date.as_deref()),
        );

        info!(
            title = title.name.as_str(),
            release = candidate.title.as_str(),
            score = candidate_score,
            decision = decision.code(),
            attempt = grab_attempts,
            "auto-grabbing release"
        );

        let grab_result = app
            .services
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: title.clone(),
                source_hint: source_hint.clone(),
                staged_nzb: None,
                source_kind: candidate.source_kind,
                source_title: source_title.clone(),
                source_password: source_password.clone(),
                category: Some(download_cat.clone()),
                queue_priority: None,
                download_directory: None,
                release_title: Some(candidate.title.clone()),
                indexer_name: Some(candidate.source.clone()),
                info_hash_hint: candidate
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
                // ── Success ─────────────────────────────────────────────────
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => candidate.source.clone(), "facet" => facet_label).increment(1);
                }

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
                    data.insert("indexer".into(), serde_json::json!(&candidate.source));
                    data.insert(
                        "download_client".into(),
                        serde_json::json!(&grab.client_type),
                    );
                    if let Some(rg) = candidate
                        .parsed_release_metadata
                        .as_ref()
                        .and_then(|m| m.release_group.as_ref())
                    {
                        data.insert("release_group".into(), serde_json::json!(rg));
                    }
                    if let Some(sz) = candidate.size_bytes {
                        data.insert("size_bytes".into(), serde_json::json!(sz));
                    }
                    if let Some(proto) = &candidate.source_kind {
                        data.insert("protocol".into(), serde_json::json!(format!("{:?}", proto)));
                    }
                    if let Some(pub_at) = &candidate.published_at {
                        data.insert("published_date".into(), serde_json::json!(pub_at));
                    }
                    if let Some(url) = &candidate.info_url {
                        data.insert("info_url".into(), serde_json::json!(url));
                    }
                    data.insert("score".into(), serde_json::json!(candidate_score));
                    if grab_attempts > 1 {
                        data.insert(
                            "fallthrough_attempt".into(),
                            serde_json::json!(grab_attempts),
                        );
                    }
                    let _ = app
                        .services
                        .record_title_history(NewTitleHistoryEvent {
                            title_id: title.id.clone(),
                            episode_id: episode.as_ref().map(|e| e.id.clone()),
                            collection_id: None,
                            event_type: TitleHistoryEventType::Grabbed,
                            source_title: source_title.clone(),
                            quality: candidate
                                .parsed_release_metadata
                                .as_ref()
                                .and_then(|m| m.quality.as_ref())
                                .map(|q| q.to_string()),
                            download_id: Some(grab.job_id.clone()),
                            data,
                        })
                        .await;
                }

                // Record download submission for auto-import matching
                let facet_str =
                    serde_json::to_string(&title.facet).unwrap_or_else(|_| "\"other\"".to_string());
                let grabbed_json = serde_json::json!({
                    "title": candidate.title,
                    "score": candidate_score,
                    "grabbed_at": now.to_rfc3339(),
                })
                .to_string();

                app.services
                    .acquisition_state
                    .commit_successful_grab(&SuccessfulGrabCommit {
                        wanted_item_id: item.id.clone(),
                        search_count: item.search_count + 1,
                        current_score: item.current_score,
                        grabbed_release: grabbed_json,
                        last_search_at: Some(now.to_rfc3339()),
                        download_submission: DownloadSubmission {
                            title_id: title.id.clone(),
                            facet: facet_str.trim_matches('"').to_string(),
                            download_client_type: grab.client_type,
                            download_client_item_id: grab.job_id,
                            source_title: source_title.clone(),
                            collection_id: item.collection_id.clone(),
                        },
                        grabbed_pending_release_id: None,
                        grabbed_at: Some(now.to_rfc3339()),
                    })
                    .await?;

                persist_standby_candidates(
                    app,
                    item,
                    &title,
                    &profile,
                    &results,
                    candidate_index + 1,
                    now,
                    dl_snapshot,
                    &db_blocklist,
                    &thresholds,
                    &existing_files,
                    &delay_profiles,
                    &title_norm,
                )
                .await;

                {
                    let mut grab_meta = HashMap::new();
                    grab_meta.insert("title_name".to_string(), serde_json::json!(title.name));
                    grab_meta.insert(
                        "release_title".to_string(),
                        serde_json::json!(candidate.title),
                    );
                    grab_meta.insert("indexer".to_string(), serde_json::json!(candidate.source));
                    grab_meta.insert("score".to_string(), serde_json::json!(candidate_score));
                    let grab_envelope = crate::activity::NotificationEnvelope {
                        event_type: NotificationEventType::Grab,
                        title: format!("Grabbed: {}", title.name),
                        body: format!(
                            "'{}' auto-grabbed for {} (score: {})",
                            candidate.title, title.name, candidate_score
                        ),
                        facet: Some(format!("{:?}", title.facet).to_lowercase()),
                        metadata: grab_meta,
                    };
                    let _ = app
                        .services
                        .record_activity_event_with_notification(
                            None,
                            Some(title.id.clone()),
                            None,
                            ActivityKind::MovieDownloaded,
                            format!(
                                "auto-grabbed: {} (score: {})",
                                candidate.title, candidate_score
                            ),
                            ActivitySeverity::Success,
                            vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                            grab_envelope,
                        )
                        .await;
                }

                return Ok(());
            }
            Err(err) => {
                // ── Grab failed — try next candidate ────────────────────────
                warn!(
                    title = title.name.as_str(),
                    release = candidate.title.as_str(),
                    attempt = grab_attempts,
                    error = %err,
                    "grab failed, trying next candidate"
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
                        None,
                        ActivityKind::AcquisitionDownloadFailed,
                        format!(
                            "grab failed for '{}' (attempt {}/10, trying next): {}",
                            candidate.title, grab_attempts, err
                        ),
                        ActivitySeverity::Warning,
                        vec![ActivityChannel::WebUi],
                    )
                    .await;

                // If ALL download clients for this source kind are down, mark it
                // so we skip remaining candidates with the same protocol.
                if is_all_clients_failed_error(&err)
                    && let Some(sk) = candidate.source_kind
                {
                    if !failed_source_kinds.contains(&sk) {
                        failed_source_kinds.push(sk);
                    }
                    info!(
                        source_kind = ?sk,
                        "all download clients failed for source kind, skipping remaining candidates with same protocol"
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
        info!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            "background acquisition: all allowed candidates were already active or had negative scores"
        );
    } else {
        info!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            result_count = results.len(),
            "background acquisition: no allowed candidates found (all blocked by quality profile)"
        );
    }

    // Re-queue for next cycle
    let _ = app
        .services
        .wanted_items
        .schedule_wanted_item_search(&WantedSearchTransition {
            id: item.id.clone(),
            next_search_at: Some(now.to_rfc3339()),
            last_search_at: Some(now.to_rfc3339()),
            search_count: item.search_count + 1,
            current_score: item.current_score,
            grabbed_release: item.grabbed_release.clone(),
        })
        .await;

    Ok(())
}

async fn recover_from_standby_candidates(
    app: &AppUseCase,
    item: &WantedItem,
    failed_release_title: &str,
    dl_snapshot: &DownloadClientSnapshot,
    now: &DateTime<Utc>,
) -> bool {
    let standby_releases = app
        .services
        .pending_releases
        .list_standby_pending_releases_for_wanted_item(&item.id)
        .await
        .unwrap_or_default();

    for standby in standby_releases {
        let mut effective_wanted = item.clone();
        effective_wanted.grabbed_release = None;
        effective_wanted.last_search_at = None;

        let claimed = app
            .services
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

        if dl_snapshot.is_active(&standby.release_title) {
            let _ = app
                .services
                .pending_releases
                .update_pending_release_status(&standby.id, PendingReleaseStatus::Expired, None)
                .await;
            continue;
        }

        info!(
            title_id = item.title_id.as_str(),
            failed_release = failed_release_title,
            standby_release = standby.release_title.as_str(),
            "attempting standby reacquisition"
        );

        match app
            .try_grab_pending_release(&effective_wanted, &standby, now)
            .await
        {
            Ok(true) => {
                let grabbed_at = now.to_rfc3339();
                let _ = app
                    .services
                    .pending_releases
                    .update_pending_release_status(
                        &standby.id,
                        PendingReleaseStatus::Grabbed,
                        Some(&grabbed_at),
                    )
                    .await;

                let siblings = app
                    .services
                    .pending_releases
                    .list_standby_pending_releases_for_wanted_item(&item.id)
                    .await
                    .unwrap_or_default();
                for sibling in siblings {
                    if sibling.id == standby.id {
                        continue;
                    }
                    let _ = app
                        .services
                        .pending_releases
                        .update_pending_release_status(
                            &sibling.id,
                            PendingReleaseStatus::Superseded,
                            None,
                        )
                        .await;
                }

                let _ = app
                    .services
                    .record_activity_event(
                        None,
                        Some(item.title_id.clone()),
                        None,
                        ActivityKind::AcquisitionCandidateAccepted,
                        format!(
                            "standby reacquisition grabbed '{}' after '{}' failed",
                            standby.release_title, failed_release_title
                        ),
                        ActivitySeverity::Success,
                        vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                    )
                    .await;

                return true;
            }
            Ok(false) | Err(_) => {
                let _ = app
                    .services
                    .pending_releases
                    .update_pending_release_status(&standby.id, PendingReleaseStatus::Expired, None)
                    .await;
            }
        }
    }

    false
}

async fn persist_standby_candidates(
    app: &AppUseCase,
    item: &WantedItem,
    title: &Title,
    profile: &QualityProfile,
    results: &[IndexerSearchResult],
    start_index: usize,
    now: &DateTime<Utc>,
    dl_snapshot: &DownloadClientSnapshot,
    db_blocklist: &std::collections::HashSet<String>,
    thresholds: &AcquisitionThresholds,
    existing_files: &[TitleMediaFile],
    delay_profiles: &[crate::DelayProfile],
    title_norm: &str,
) {
    let _ = app
        .services
        .pending_releases
        .delete_standby_pending_releases_for_wanted_item(&item.id)
        .await;

    let mut persisted = 0usize;
    let mut seen_source_hints = std::collections::HashSet::new();

    for candidate in results.iter().skip(start_index) {
        if persisted >= MAX_STANDBY_CANDIDATES_PER_WANTED_ITEM {
            break;
        }

        let is_allowed = candidate
            .quality_profile_decision
            .as_ref()
            .map(|decision| decision.allowed)
            .unwrap_or(false);
        if !is_allowed {
            continue;
        }

        if !crate::app_usecase_rss::normalize_for_matching(&candidate.title).contains(title_norm) {
            continue;
        }

        let candidate_score = candidate
            .quality_profile_decision
            .as_ref()
            .map(|decision| decision.preference_score)
            .unwrap_or(0);
        if candidate_score < 0 {
            break;
        }

        let decision = evaluate_upgrade(
            candidate_score,
            item.current_score,
            true,
            item.last_search_at.as_deref(),
            now,
            thresholds,
            profile.criteria.min_score_to_grab,
        );
        if !decision.is_accept() {
            continue;
        }

        if dl_snapshot.is_active(&candidate.title) {
            continue;
        }

        if db_blocklist.contains(&candidate.title.to_ascii_lowercase()) {
            continue;
        }

        if crate::acquisition_policy::should_skip_repack_group_mismatch(
            candidate,
            existing_files,
            item.episode_id.as_deref(),
        ) {
            continue;
        }

        if let Some(delay_decision) = crate::delay_profile::resolve_delay_decision(
            delay_profiles,
            &title.tags,
            &title.facet,
            candidate.source_kind,
            candidate
                .published_at
                .as_deref()
                .and_then(crate::quality_profile::parse_published_at),
            candidate_score,
            now,
        ) && delay_decision.should_hold()
        {
            continue;
        }

        let source_hint = candidate
            .download_url
            .clone()
            .or_else(|| candidate.link.clone());
        let Some(source_hint_value) = source_hint else {
            continue;
        };
        if !seen_source_hints.insert(source_hint_value.clone()) {
            continue;
        }

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
            release_guid: candidate.guid.clone(),
            added_at: now.to_rfc3339(),
            delay_until: now.to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: candidate.password_hint.clone(),
            published_at: candidate.published_at.clone(),
            info_hash: candidate
                .extra
                .get("info_hash")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        };

        if app
            .services
            .pending_releases
            .insert_pending_release(&standby)
            .await
            .is_ok()
        {
            persisted += 1;
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
}

fn should_research_failed_grab(item: &WantedItem, now: &DateTime<Utc>) -> bool {
    !is_old_failed_grab_title(item, now)
        && is_last_search_stale(item.last_search_at.as_deref(), now)
}

fn is_old_failed_grab_title(item: &WantedItem, now: &DateTime<Utc>) -> bool {
    let Some(baseline_date) = item.baseline_date.as_deref() else {
        return false;
    };
    let Some(parsed_date) = parse_failed_grab_baseline_date(baseline_date) else {
        return false;
    };
    now.date_naive()
        .signed_duration_since(parsed_date)
        .num_days()
        > FAILED_GRAB_OLD_TITLE_DAYS
}

fn is_last_search_stale(last_search_at: Option<&str>, now: &DateTime<Utc>) -> bool {
    let Some(last_search_at) = last_search_at else {
        return true;
    };
    let Some(last_search_at) = crate::quality_profile::parse_published_at(last_search_at) else {
        return true;
    };
    (*now - last_search_at).num_minutes() > FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES
}

fn parse_failed_grab_baseline_date(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .ok()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|value| value.date_naive())
        })
        .or_else(|| {
            chrono::DateTime::parse_from_rfc2822(raw)
                .ok()
                .map(|value| value.date_naive())
        })
}

struct SearchQueryResult {
    queries: Vec<String>,
    imdb_id: Option<String>,
    tvdb_id: Option<String>,
    anidb_id: Option<String>,
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
    let anidb_id = anidb_id_from_external_ids(&title.external_ids);

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
            let mut seen = std::collections::HashSet::new();
            queries.retain(|q| seen.insert(q.to_ascii_lowercase()));
            if queries.is_empty() && imdb_id.is_some() {
                queries.push(String::new());
            }
            SearchQueryResult {
                queries,
                imdb_id,
                tvdb_id,
                anidb_id,
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
                    // Include title name so freetext-only indexers get usable queries
                    queries.push(format!(
                        "{} S{:0>2}E{:0>2}",
                        title.name, season_num, episode_num
                    ));
                }

                // For anime season 0 (specials/OVAs): use title-based search
                // S00E## format produces poor results on indexers
                if season_num == 0 && title.facet == scryer_domain::MediaFacet::Anime {
                    if let Some(label) = ep.episode_label.as_deref().filter(|l| !l.is_empty()) {
                        queries.push(format!("{} {}", title.name, label));
                    }
                    if episode_num > 0 {
                        if ep.episode_type == scryer_domain::EpisodeType::Ova {
                            queries.push(format!("{} OVA {:0>2}", title.name, episode_num));
                        } else {
                            queries.push(format!("{} Special {:0>2}", title.name, episode_num));
                        }
                    }
                }

                // For anime: add absolute number query as a fallback.
                // Long-running series (One Piece, Naruto) use absolute numbering
                // on indexers, not TVDB season numbering. For modern seasonal anime
                // (Demon Slayer, Jujutsu Kaisen) S##E## is the primary format.
                // Insert absolute first only if it diverges from the episode number
                // (indicating a long-running show where S##E## won't match).
                // For anime: add absolute number query only when it differs from
                // the episode number (long-running series like One Piece where
                // TVDB S22E01 doesn't match release titles using absolute 1146).
                // When abs == ep (seasonal anime like Demon Slayer), the S##E##
                // query already covers it and the TVDB structured search handles
                // Newznab/Torznab.
                if title.facet == scryer_domain::MediaFacet::Anime
                    && let Some(abs) = ep
                        .absolute_number
                        .as_deref()
                        .and_then(|a| a.parse::<usize>().ok())
                        .filter(|&a| a > 0 && a != episode_num)
                {
                    // Long-running: absolute is primary (auto mode uses first query)
                    queries.insert(0, format!("{} {:0>3}", title.name, abs));
                }

                if !queries.is_empty() {
                    // Dedup (e.g. S10E10 == S10E10 when both formats produce the same string)
                    let mut seen = std::collections::HashSet::new();
                    queries.retain(|q| seen.insert(q.to_ascii_lowercase()));
                }
            }

            // Fallback: if we couldn't build S##E## queries, use title name.
            // Always include the title name so indexers that only support freetext
            // search (e.g. AnimeTosho) have something to query with.
            if queries.is_empty() {
                queries.push(title.name.clone());
            }

            SearchQueryResult {
                queries,
                imdb_id,
                tvdb_id,
                anidb_id,
                category,
                season: season_param,
                episode: episode_param,
            }
        }
        "interstitial_movie" => {
            // Search for franchise movies using the movie's own metadata, not the series'
            // Search in the "movies" category since anime movies are released as movies
            let mut queries = Vec::new();
            // The movie name and year come from the interstitial collection metadata,
            // passed via the title's name/year fields for interstitial wanted items
            if !title.name.is_empty() {
                let query = if let Some(year) = title.year {
                    format!("{} {}", title.name, year)
                } else {
                    title.name.clone()
                };
                queries.push(query);
            }
            let mut seen = std::collections::HashSet::new();
            queries.retain(|q| seen.insert(q.to_ascii_lowercase()));
            if queries.is_empty() && imdb_id.is_some() {
                queries.push(String::new());
            }
            SearchQueryResult {
                queries,
                imdb_id,
                tvdb_id,
                anidb_id,
                category: "movies".to_string(),
                season: None,
                episode: None,
            }
        }
        _ => SearchQueryResult {
            queries: vec![],
            imdb_id: None,
            tvdb_id: None,
            anidb_id: None,
            category,
            season: None,
            episode: None,
        },
    }
}

pub(crate) fn tvdb_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source == "tvdb")
        .map(|id| id.value.clone())
}

pub(crate) fn anidb_id_from_external_ids(external_ids: &[ExternalId]) -> Option<String> {
    external_ids
        .iter()
        .find(|id| id.source == "anidb")
        .map(|id| id.value.clone())
}

// --- Public use-case methods for the wanted items API ---

impl AppUseCase {
    pub async fn get_wanted_item(&self, actor: &User, id: &str) -> AppResult<Option<WantedItem>> {
        require(actor, &Entitlement::ViewCatalog)?;
        self.services.wanted_items.get_wanted_item_by_id(id).await
    }

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

    pub async fn trigger_season_wanted_search(
        &self,
        title_id: &str,
        season_number: u32,
    ) -> AppResult<usize> {
        let season_str = season_number.to_string();
        let items = self
            .services
            .wanted_items
            .list_wanted_items(Some("wanted"), Some("episode"), Some(title_id), 500, 0)
            .await?;

        let now = Utc::now();
        let mut queued = 0usize;
        for item in &items {
            if item.season_number.as_deref() == Some(season_str.as_str()) {
                self.services
                    .wanted_items
                    .schedule_wanted_item_search(&WantedSearchTransition {
                        id: item.id.clone(),
                        next_search_at: Some(now.to_rfc3339()),
                        last_search_at: item.last_search_at.clone(),
                        search_count: item.search_count,
                        current_score: item.current_score,
                        grabbed_release: item.grabbed_release.clone(),
                    })
                    .await?;
                queued += 1;
            }
        }

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
            .schedule_wanted_item_search(&WantedSearchTransition {
                id: item.id.clone(),
                next_search_at: Some(now.to_rfc3339()),
                last_search_at: item.last_search_at.clone(),
                search_count: item.search_count,
                current_score: item.current_score,
                grabbed_release: item.grabbed_release.clone(),
            })
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
            .transition_wanted_to_paused(&WantedPauseTransition {
                id: item.id.clone(),
                last_search_at: item.last_search_at.clone(),
                search_count: item.search_count,
                current_score: item.current_score,
                grabbed_release: item.grabbed_release.clone(),
            })
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
            .schedule_wanted_item_search(&WantedSearchTransition {
                id: item.id.clone(),
                next_search_at: Some(schedule.next_search_at),
                last_search_at: item.last_search_at.clone(),
                search_count: item.search_count,
                current_score: item.current_score,
                grabbed_release: item.grabbed_release.clone(),
            })
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
            .schedule_wanted_item_search(&WantedSearchTransition {
                id: item.id.clone(),
                next_search_at: Some(schedule.next_search_at),
                last_search_at: None,
                search_count: 0,
                current_score: None,
                grabbed_release: None,
            })
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
            if item.status == WantedStatus::Grabbed {
                return Ok(0);
            }

            self.services
                .wanted_items
                .schedule_wanted_item_search(&WantedSearchTransition {
                    id: item.id.clone(),
                    next_search_at: Some(next_search_at.clone()),
                    last_search_at: item.last_search_at.clone(),
                    search_count: item.search_count,
                    current_score: item.current_score,
                    grabbed_release: item.grabbed_release.clone(),
                })
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
            collection_id: None,
            season_number: None,
            media_type: "movie".to_string(),
            search_phase: schedule.search_phase.to_string(),
            next_search_at: Some(next_search_at),
            last_search_at: None,
            search_count: 0,
            baseline_date,
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
        };

        self.services
            .wanted_items
            .ensure_wanted_item_seeded(&item)
            .await?;
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
                    if item.status == WantedStatus::Grabbed {
                        continue;
                    }

                    self.services
                        .wanted_items
                        .schedule_wanted_item_search(&WantedSearchTransition {
                            id: item.id.clone(),
                            next_search_at: Some(next_search_at.clone()),
                            last_search_at: item.last_search_at.clone(),
                            search_count: item.search_count,
                            current_score: item.current_score,
                            grabbed_release: item.grabbed_release.clone(),
                        })
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
                    collection_id: None,
                    season_number: episode.season_number.clone(),
                    media_type: "episode".to_string(),
                    search_phase: schedule.search_phase.to_string(),
                    next_search_at: Some(next_search_at.clone()),
                    last_search_at: None,
                    search_count: 0,
                    baseline_date,
                    status: WantedStatus::Wanted,
                    grabbed_release: None,
                    current_score: None,
                    created_at: now.to_rfc3339(),
                    updated_at: now.to_rfc3339(),
                };

                self.services
                    .wanted_items
                    .ensure_wanted_item_seeded(&item)
                    .await?;
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
                sync_interval_seconds: 3600,
                batch_size: 50,
            }
        }
    };

    if !settings.enabled {
        info!("background acquisition poller is disabled (acquisition.enabled != true)");
        return;
    }

    info!("background acquisition poller started");

    // Initial wanted state sync
    if let Err(err) = app.sync_wanted_state().await {
        warn!(error = %err, "initial wanted state sync failed");
    }

    // Reset items that were searched but never found anything. This recovers
    // from scenarios where a bug (e.g. broken capability filter) caused searches
    // to return 0 results and items got rescheduled far into the future.
    let now_str = Utc::now().to_rfc3339();
    match app
        .services
        .wanted_items
        .reset_fruitless_wanted_items(&now_str)
        .await
    {
        Ok(count) if count > 0 => {
            info!(count, "reset fruitless wanted items to search immediately");
        }
        Err(err) => {
            warn!(error = %err, "failed to reset fruitless wanted items");
        }
        _ => {}
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

    let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(
        settings.poll_interval_seconds.max(1) as u64,
    ));
    let mut sync_interval = tokio::time::interval(std::time::Duration::from_secs(
        settings.sync_interval_seconds.max(1) as u64,
    ));
    let mut metadata_refresh_interval = tokio::time::interval(std::time::Duration::from_hours(12));
    let mut registry_refresh_interval = tokio::time::interval(std::time::Duration::from_hours(24));
    let mut health_check_interval = tokio::time::interval(std::time::Duration::from_hours(6));
    let mut staged_nzb_prune_interval = tokio::time::interval(std::time::Duration::from_hours(1));
    let mut housekeeping_interval = tokio::time::interval(std::time::Duration::from_hours(24));
    let mut rss_sync_interval = tokio::time::interval(std::time::Duration::from_mins(15));
    let mut pending_release_interval = tokio::time::interval(std::time::Duration::from_mins(1));

    // Consume the first tick immediately
    poll_interval.tick().await;
    sync_interval.tick().await;
    metadata_refresh_interval.tick().await;
    registry_refresh_interval.tick().await;
    health_check_interval.tick().await;
    staged_nzb_prune_interval.tick().await;
    housekeeping_interval.tick().await;
    rss_sync_interval.tick().await;
    pending_release_interval.tick().await;

    let wake = app.services.acquisition_wake.clone();

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
                run_task("wanted_items", async move {
                    process_due_wanted_items(&app).await;
                }).await;
            }
            _ = poll_interval.tick() => {
                let app = app.clone();
                run_task("wanted_items", async move {
                    process_due_wanted_items(&app).await;
                }).await;
            }
            _ = sync_interval.tick() => {
                let app = app.clone();
                run_task("sync_state", async move {
                    if let Err(err) = app.sync_wanted_state().await {
                        warn!(error = %err, "periodic wanted state sync failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "sync_state").increment(1);
                    }
                }).await;
            }
            _ = metadata_refresh_interval.tick() => {
                let app = app.clone();
                run_task("metadata_refresh", async move {
                    info!("starting periodic metadata refresh for monitored series");
                    app.refresh_monitored_series_metadata().await;
                }).await;
            }
            _ = registry_refresh_interval.tick() => {
                let app = app.clone();
                run_task("registry_refresh", async move {
                    info!("refreshing plugin registry");
                    if let Err(e) = app.refresh_plugin_registry_internal().await {
                        warn!(error = %e, "periodic plugin registry refresh failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "registry_refresh").increment(1);
                    }
                }).await;
            }
            _ = health_check_interval.tick() => {
                let app = app.clone();
                run_task("health_check", async move {
                    let results = app.run_health_checks().await;
                    *app.services.health_check_results.write().await = results;
                    info!("periodic health checks completed");
                }).await;
            }
            _ = staged_nzb_prune_interval.tick() => {
                let app = app.clone();
                run_task("staged_nzb_prune", async move {
                    match app.services.staged_nzb_store.prune_staged_nzbs_older_than(chrono::Utc::now() - chrono::Duration::hours(1)).await {
                        Ok(pruned) => {
                            if pruned > 0 {
                                info!(staged_nzb_artifacts_pruned = pruned, "periodic staged nzb prune completed");
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "periodic staged nzb prune failed");
                            metrics::counter!("scryer_task_errors_total", "task" => "staged_nzb_prune").increment(1);
                        }
                    }
                }).await;
            }
            _ = housekeeping_interval.tick() => {
                let app = app.clone();
                run_task("housekeeping", async move {
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
                }).await;
            }
            _ = pending_release_interval.tick() => {
                let app = app.clone();
                run_task("pending_releases", async move {
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
                }).await;
            }
            _ = rss_sync_interval.tick() => {
                let app = app.clone();
                run_task("rss_sync", async move {
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
                }).await;
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
