use super::*;
use chrono::{Duration, Utc};
use tracing::{info, warn};

use crate::delay_profile::DelayProfile;
use crate::types::PendingRelease;

impl AppUseCase {
    /// Load delay profiles from settings.
    pub(crate) async fn load_delay_profiles(&self) -> Vec<DelayProfile> {
        let json = self
            .services
            .settings
            .get_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                crate::delay_profile::DELAY_PROFILE_CATALOG_KEY,
                None,
            )
            .await
            .ok()
            .flatten();

        match json {
            Some(raw) => crate::delay_profile::parse_delay_profile_catalog(&raw)
                .unwrap_or_else(|e| {
                    warn!(error = %e, "failed to parse delay profile catalog");
                    vec![]
                }),
            None => vec![],
        }
    }

    /// Insert a pending release when a delay profile holds back a grab.
    pub(crate) async fn insert_pending_release(
        &self,
        wanted: &WantedItem,
        title: &scryer_domain::Title,
        release_title: &str,
        release_url: Option<&str>,
        release_size_bytes: Option<i64>,
        release_score: i32,
        scoring_log_json: Option<String>,
        indexer_source: Option<&str>,
        release_guid: Option<&str>,
        delay_hours: i64,
    ) {
        let now = Utc::now();
        let delay_until = now + Duration::hours(delay_hours);

        let pending = PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: release_title.to_string(),
            release_url: release_url.map(str::to_string),
            release_size_bytes,
            release_score,
            scoring_log_json,
            indexer_source: indexer_source.map(str::to_string),
            release_guid: release_guid.map(str::to_string),
            added_at: now.to_rfc3339(),
            delay_until: delay_until.to_rfc3339(),
            status: "waiting".to_string(),
            grabbed_at: None,
        };

        // Supersede any existing waiting releases for this wanted item with a lower score
        let existing = self
            .services
            .pending_releases
            .list_pending_releases_for_wanted_item(&wanted.id)
            .await
            .unwrap_or_default();

        let dominated = existing
            .iter()
            .all(|e| e.release_score <= release_score);

        if !dominated {
            info!(
                title = title.name.as_str(),
                release = release_title,
                score = release_score,
                "pending release: skipping, higher-scored release already pending"
            );
            return;
        }

        match self
            .services
            .pending_releases
            .insert_pending_release(&pending)
            .await
        {
            Ok(_) => {
                // Mark older, lower-scored releases as superseded
                let _ = self
                    .services
                    .pending_releases
                    .supersede_pending_releases_for_wanted_item(&wanted.id, &pending.id)
                    .await;

                info!(
                    title = title.name.as_str(),
                    release = release_title,
                    score = release_score,
                    delay_until = %delay_until,
                    "pending release: held for delay profile"
                );
            }
            Err(e) => {
                warn!(
                    error = %e,
                    title = title.name.as_str(),
                    release = release_title,
                    "pending release: failed to insert"
                );
            }
        }
    }

    /// Process pending releases whose delay has expired.
    /// Called periodically from the acquisition poller.
    pub(crate) async fn process_expired_pending_releases(&self) -> AppResult<u32> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        let expired = self
            .services
            .pending_releases
            .list_expired_pending_releases(&now_str)
            .await?;

        if expired.is_empty() {
            return Ok(0);
        }

        // Group by wanted_item_id — pick the highest score per group
        let mut by_wanted: std::collections::HashMap<String, Vec<PendingRelease>> =
            std::collections::HashMap::new();
        for pr in expired {
            by_wanted.entry(pr.wanted_item_id.clone()).or_default().push(pr);
        }

        let mut grabbed_count = 0u32;

        for (wanted_item_id, mut releases) in by_wanted {
            // Sort descending by score
            releases.sort_by(|a, b| b.release_score.cmp(&a.release_score));

            let Some(wanted) = self
                .services
                .wanted_items
                .get_wanted_item_by_id(&wanted_item_id)
                .await?
            else {
                // Wanted item gone — mark all as expired
                for pr in &releases {
                    let _ = self
                        .services
                        .pending_releases
                        .update_pending_release_status(&pr.id, "expired", None)
                        .await;
                }
                continue;
            };

            // Skip if already grabbed or completed
            if wanted.status == "grabbed" || wanted.status == "completed" {
                for pr in &releases {
                    let _ = self
                        .services
                        .pending_releases
                        .update_pending_release_status(&pr.id, "superseded", None)
                        .await;
                }
                continue;
            }

            // Try to grab the best release
            let mut grabbed = false;
            for pr in &releases {
                match self.try_grab_pending_release(&wanted, pr, &now).await {
                    Ok(true) => {
                        // Mark this one as grabbed
                        let _ = self
                            .services
                            .pending_releases
                            .update_pending_release_status(
                                &pr.id,
                                "grabbed",
                                Some(&now.to_rfc3339()),
                            )
                            .await;
                        // Mark siblings as superseded
                        let _ = self
                            .services
                            .pending_releases
                            .supersede_pending_releases_for_wanted_item(
                                &wanted_item_id,
                                &pr.id,
                            )
                            .await;
                        grabbed = true;
                        grabbed_count += 1;
                        break;
                    }
                    Ok(false) => {
                        // This release couldn't be grabbed (blocklisted, etc) — try next
                        let _ = self
                            .services
                            .pending_releases
                            .update_pending_release_status(&pr.id, "expired", None)
                            .await;
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            release = pr.release_title.as_str(),
                            "pending release: grab attempt failed"
                        );
                        let _ = self
                            .services
                            .pending_releases
                            .update_pending_release_status(&pr.id, "expired", None)
                            .await;
                    }
                }
            }

            if !grabbed {
                info!(
                    wanted_item_id = wanted_item_id.as_str(),
                    "pending release: no viable release to grab after delay expired"
                );
            }
        }

        Ok(grabbed_count)
    }

    /// List all pending releases that are waiting to be grabbed.
    pub async fn list_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        self.services.pending_releases.list_waiting_pending_releases().await
    }

    /// Force-grab a pending release immediately, ignoring the delay.
    pub async fn force_grab_pending_release(&self, id: &str) -> AppResult<bool> {
        let pr = self.services.pending_releases.get_pending_release(id).await?;
        let Some(pr) = pr else {
            return Err(AppError::Repository(format!("pending release {id} not found")));
        };
        if pr.status != "waiting" {
            return Err(AppError::Repository(format!("pending release {id} is not in waiting status")));
        }
        let now = Utc::now();
        let wanted = self
            .services
            .wanted_items
            .get_wanted_item_by_id(&pr.wanted_item_id)
            .await?
            .ok_or_else(|| {
                AppError::Repository(format!("wanted item {} not found", pr.wanted_item_id))
            })?;
        self.try_grab_pending_release(&wanted, &pr, &now).await
    }

    /// Dismiss a pending release (set status to dismissed).
    pub async fn dismiss_pending_release(&self, id: &str) -> AppResult<bool> {
        let pr = self.services.pending_releases.get_pending_release(id).await?;
        let Some(pr) = pr else {
            return Err(AppError::Repository(format!("pending release {id} not found")));
        };
        if pr.status != "waiting" {
            return Err(AppError::Repository(format!("pending release {id} is not in waiting status")));
        }
        self.services.pending_releases.update_pending_release_status(id, "dismissed", None).await?;
        Ok(true)
    }

    /// Attempt to grab a single pending release. Returns Ok(true) if grabbed successfully.
    pub(crate) async fn try_grab_pending_release(
        &self,
        wanted: &WantedItem,
        pr: &PendingRelease,
        now: &chrono::DateTime<Utc>,
    ) -> AppResult<bool> {
        // Load title
        let Some(title) = self.services.titles.get_by_id(&pr.title_id).await? else {
            return Ok(false);
        };

        // Check blocklist
        let db_blocklist: std::collections::HashSet<String> = self
            .services
            .release_attempts
            .list_failed_release_signatures_for_title(&title.id, 200)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| e.source_title)
            .map(|t| t.to_ascii_lowercase())
            .collect();

        if db_blocklist.contains(&pr.release_title.to_ascii_lowercase()) {
            return Ok(false);
        }

        // Check upgrade decision
        let category = match title.facet {
            scryer_domain::MediaFacet::Movie => "movie",
            scryer_domain::MediaFacet::Tv => "series",
            scryer_domain::MediaFacet::Anime => "anime",
            _ => "other",
        };
        let tvdb_id = title
            .external_ids
            .iter()
            .find(|id| id.source == "tvdb")
            .map(|id| id.value.clone());

        let profile = self
            .resolve_quality_profile(
                &title.tags,
                title.imdb_id.as_deref(),
                tvdb_id.as_deref(),
                Some(category),
            )
            .await
            .unwrap_or_else(|_| crate::quality_profile::default_quality_profile_for_search());

        let thresholds = AcquisitionThresholds::default();
        let decision = crate::acquisition_policy::evaluate_upgrade(
            pr.release_score,
            wanted.current_score,
            profile.criteria.allow_upgrades,
            wanted.last_search_at.as_deref(),
            now,
            &thresholds,
        );

        if !decision.is_accept() {
            return Ok(false);
        }

        // Submit to download client
        let source_hint = pr.release_url.clone();
        let source_title = Some(pr.release_title.clone());

        let _ = self
            .services
            .release_attempts
            .record_release_attempt(
                Some(title.id.clone()),
                source_hint.clone(),
                source_title.clone(),
                ReleaseDownloadAttemptOutcome::Pending,
                None,
                None,
            )
            .await;

        let download_cat = self.derive_download_category(&title.facet).await;

        info!(
            title = title.name.as_str(),
            release = pr.release_title.as_str(),
            score = pr.release_score,
            "pending release: grabbing after delay expired"
        );

        let grab_result = self
            .services
            .download_client
            .submit_to_download_queue(
                &title,
                source_hint.clone(),
                source_title.clone(),
                None,
                Some(download_cat),
            )
            .await;

        match grab_result {
            Ok(grab) => {
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    let indexer_label = pr.indexer_source.as_deref().unwrap_or("unknown").to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => indexer_label, "facet" => facet_label).increment(1);
                }

                let _ = self
                    .services
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint,
                        source_title.clone(),
                        ReleaseDownloadAttemptOutcome::Success,
                        None,
                        None,
                    )
                    .await;

                let facet_str = serde_json::to_string(&title.facet)
                    .unwrap_or_else(|_| "\"other\"".to_string());
                let _ = self
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

                let grabbed_json = serde_json::json!({
                    "title": pr.release_title,
                    "score": pr.release_score,
                    "grabbed_at": now.to_rfc3339(),
                    "source": "pending_release",
                })
                .to_string();

                let _ = self
                    .services
                    .wanted_items
                    .update_wanted_item_status(
                        &wanted.id,
                        "grabbed",
                        None,
                        Some(&now.to_rfc3339()),
                        wanted.search_count,
                        Some(pr.release_score),
                        Some(&grabbed_json),
                    )
                    .await;

                let _ = self
                    .services
                    .record_activity_event(
                        None,
                        Some(title.id.clone()),
                        ActivityKind::MovieDownloaded,
                        format!(
                            "Pending release grabbed: {} (score: {})",
                            pr.release_title, pr.release_score
                        ),
                        ActivitySeverity::Success,
                        vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                    )
                    .await;

                Ok(true)
            }
            Err(err) => {
                warn!(
                    title = title.name.as_str(),
                    release = pr.release_title.as_str(),
                    error = %err,
                    "pending release: download submission failed"
                );

                let _ = self
                    .services
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint,
                        source_title,
                        ReleaseDownloadAttemptOutcome::Failed,
                        Some(err.to_string()),
                        None,
                    )
                    .await;

                Ok(false)
            }
        }
    }
}
