use super::*;
use chrono::{Duration, Utc};
use scryer_domain::NotificationEventType;
use std::collections::HashMap;
use tracing::{info, warn};

use crate::delay_profile::DelayProfile;
use crate::types::{PendingRelease, PendingReleaseStatus};

impl AppUseCase {
    /// Load delay profiles from settings.
    pub(crate) async fn load_delay_profiles(&self) -> Vec<DelayProfile> {
        match self.delay_profiles().await {
            Ok(profiles) => profiles,
            Err(error) => {
                warn!(error = %error, "failed to load delay profile catalog");
                vec![]
            }
        }
    }

    /// Insert a pending release when a delay profile holds back a grab.
    pub(crate) async fn insert_pending_release(
        &self,
        wanted: &WantedItem,
        title: &scryer_domain::Title,
        release_title: &str,
        release_url: Option<&str>,
        source_kind: Option<DownloadSourceKind>,
        release_size_bytes: Option<i64>,
        release_score: i32,
        scoring_log_json: Option<String>,
        indexer_source: Option<&str>,
        release_guid: Option<&str>,
        delay_minutes: i64,
        source_password: Option<&str>,
        published_at: Option<&str>,
        info_hash: Option<&str>,
    ) {
        let now = Utc::now();
        let delay_until = now + Duration::minutes(delay_minutes);

        let pending = PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: release_title.to_string(),
            release_url: release_url.map(str::to_string),
            source_kind,
            release_size_bytes,
            release_score,
            scoring_log_json,
            indexer_source: indexer_source.map(str::to_string),
            release_guid: release_guid.map(str::to_string),
            added_at: now.to_rfc3339(),
            delay_until: delay_until.to_rfc3339(),
            status: PendingReleaseStatus::Waiting,
            grabbed_at: None,
            source_password: source_password.map(str::to_string),
            published_at: published_at.map(str::to_string),
            info_hash: info_hash.map(str::to_string),
        };

        // Supersede any existing waiting releases for this wanted item with a lower score
        let existing = self
            .services
            .pending_releases
            .list_pending_releases_for_wanted_item(&wanted.id)
            .await
            .unwrap_or_default();

        let dominated = existing.iter().all(|e| e.release_score <= release_score);

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
    pub async fn process_expired_pending_releases(&self) -> AppResult<u32> {
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
            by_wanted
                .entry(pr.wanted_item_id.clone())
                .or_default()
                .push(pr);
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
                        .update_pending_release_status(&pr.id, PendingReleaseStatus::Expired, None)
                        .await;
                }
                continue;
            };

            // Skip if already grabbed or completed
            if wanted.status == WantedStatus::Grabbed || wanted.status == WantedStatus::Completed {
                for pr in &releases {
                    let _ = self
                        .services
                        .pending_releases
                        .update_pending_release_status(
                            &pr.id,
                            PendingReleaseStatus::Superseded,
                            None,
                        )
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
                                PendingReleaseStatus::Grabbed,
                                Some(&now.to_rfc3339()),
                            )
                            .await;
                        // Mark siblings as superseded
                        let _ = self
                            .services
                            .pending_releases
                            .supersede_pending_releases_for_wanted_item(&wanted_item_id, &pr.id)
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
                            .update_pending_release_status(
                                &pr.id,
                                PendingReleaseStatus::Expired,
                                None,
                            )
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
                            .update_pending_release_status(
                                &pr.id,
                                PendingReleaseStatus::Expired,
                                None,
                            )
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
        self.services
            .pending_releases
            .list_waiting_pending_releases()
            .await
    }

    pub async fn get_pending_release(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<PendingRelease>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services.pending_releases.get_pending_release(id).await
    }

    pub async fn list_pending_releases_for_wanted_item(
        &self,
        actor: &User,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services
            .pending_releases
            .list_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    /// Force-grab a pending release immediately, ignoring the delay.
    pub async fn force_grab_pending_release(&self, id: &str) -> AppResult<bool> {
        let pr = self
            .services
            .pending_releases
            .get_pending_release(id)
            .await?;
        let Some(pr) = pr else {
            return Err(AppError::Repository(format!(
                "pending release {id} not found"
            )));
        };
        if pr.status != PendingReleaseStatus::Waiting {
            return Err(AppError::Repository(format!(
                "pending release {id} is not in waiting status"
            )));
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
        let pr = self
            .services
            .pending_releases
            .get_pending_release(id)
            .await?;
        let Some(pr) = pr else {
            return Err(AppError::Repository(format!(
                "pending release {id} not found"
            )));
        };
        if pr.status != PendingReleaseStatus::Waiting {
            return Err(AppError::Repository(format!(
                "pending release {id} is not in waiting status"
            )));
        }
        self.services
            .pending_releases
            .update_pending_release_status(id, PendingReleaseStatus::Dismissed, None)
            .await?;
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

        // Check if this release is already active in the download client.
        // Without this check, the pending processor could retry a release
        // that's currently downloading (e.g. grabbed via background search
        // while this pending release was waiting).
        let dl_snapshot = super::app_usecase_acquisition::DownloadClientSnapshot::fetch(self).await;
        if dl_snapshot.is_active(&pr.release_title) {
            info!(
                release = pr.release_title.as_str(),
                "pending release: skipping, already active in download client"
            );
            return Ok(false);
        }

        // Check upgrade decision
        let category = title.facet.as_str();
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

        // Cutoff tier check
        if crate::quality_profile::has_reached_cutoff(
            wanted.grabbed_release.as_deref(),
            profile.criteria.cutoff_tier.as_deref(),
            &profile.criteria.quality_tiers,
        ) {
            return Ok(false);
        }

        let thresholds = self
            .acquisition_thresholds(&profile.criteria.scoring_persona)
            .await;
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
        let source_kind = pr
            .source_kind
            .or_else(|| DownloadSourceKind::infer_from_hint(source_hint.as_deref()));
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
                pr.source_password.clone(),
            )
            .await;

        let download_cat = self.derive_download_category(&title.facet).await;
        let is_recent = self.is_recent_for_queue_priority(
            pr.published_at
                .as_deref()
                .or(wanted.baseline_date.as_deref())
                .or(title.first_aired.as_deref())
                .or(title.digital_release_date.as_deref()),
        );

        info!(
            title = title.name.as_str(),
            release = pr.release_title.as_str(),
            score = pr.release_score,
            status = pr.status.as_str(),
            "persisted candidate: grabbing"
        );

        let grab_result = self
            .services
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: title.clone(),
                source_hint: source_hint.clone(),
                source_kind,
                source_title: source_title.clone(),
                source_password: pr.source_password.clone(),
                category: Some(download_cat),
                queue_priority: None,
                download_directory: None,
                release_title: Some(pr.release_title.clone()),
                indexer_name: pr.indexer_source.clone(),
                info_hash_hint: pr.info_hash.clone(),
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent,
                season_pack: None,
            })
            .await;

        match grab_result {
            Ok(grab) => {
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    let indexer_label = pr
                        .indexer_source
                        .as_deref()
                        .unwrap_or("unknown")
                        .to_string();
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
                        pr.source_password.clone(),
                    )
                    .await;

                let facet_str =
                    serde_json::to_string(&title.facet).unwrap_or_else(|_| "\"other\"".to_string());
                let grabbed_json = serde_json::json!({
                    "title": pr.release_title,
                    "score": pr.release_score,
                    "grabbed_at": now.to_rfc3339(),
                    "source": "pending_release",
                })
                .to_string();

                self.services
                    .acquisition_state
                    .commit_successful_grab(&SuccessfulGrabCommit {
                        wanted_item_id: wanted.id.clone(),
                        search_count: wanted.search_count,
                        current_score: wanted.current_score,
                        grabbed_release: grabbed_json,
                        last_search_at: Some(now.to_rfc3339()),
                        download_submission: DownloadSubmission {
                            title_id: title.id.clone(),
                            facet: facet_str.trim_matches('"').to_string(),
                            download_client_type: grab.client_type,
                            download_client_item_id: grab.job_id,
                            source_title: source_title.clone(),
                            collection_id: None,
                        },
                        grabbed_pending_release_id: Some(pr.id.clone()),
                        grabbed_at: Some(now.to_rfc3339()),
                    })
                    .await?;

                {
                    let mut grab_meta = HashMap::new();
                    grab_meta.insert("title_name".to_string(), serde_json::json!(title.name));
                    grab_meta.insert(
                        "release_title".to_string(),
                        serde_json::json!(pr.release_title),
                    );
                    grab_meta.insert("score".to_string(), serde_json::json!(pr.release_score));
                    let grab_envelope = crate::activity::NotificationEnvelope {
                        event_type: NotificationEventType::Grab,
                        title: format!("Grabbed: {}", title.name),
                        body: format!(
                            "Pending release '{}' grabbed for {} (score: {})",
                            pr.release_title, title.name, pr.release_score
                        ),
                        facet: Some(format!("{:?}", title.facet).to_lowercase()),
                        metadata: grab_meta,
                    };
                    let _ = self
                        .services
                        .record_activity_event_with_notification(
                            None,
                            Some(title.id.clone()),
                            None,
                            ActivityKind::MovieDownloaded,
                            format!(
                                "Pending release grabbed: {} (score: {})",
                                pr.release_title, pr.release_score
                            ),
                            ActivitySeverity::Success,
                            vec![ActivityChannel::WebUi, ActivityChannel::Toast],
                            grab_envelope,
                        )
                        .await;
                }

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
                        pr.source_password.clone(),
                    )
                    .await;

                Ok(false)
            }
        }
    }
}
