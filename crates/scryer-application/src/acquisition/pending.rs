use super::*;
use crate::acquisition_decision_helpers::is_download_submit_unavailable_error;
use crate::domain_events::{new_title_domain_event, title_context_snapshot};
use chrono::{Duration, Utc};
use scryer_domain::{DomainEventPayload, ReleaseGrabbedEventData};
use tracing::{info, warn};

use crate::acquisition::seed_goals::ReleaseSeedMinimums;
use crate::acquisition::submission::{
    CanonicalDownloadSubmissionIntent, CanonicalDownloadSubmissionOutcome, GrabTrigger,
    record_grab_submission_outcome,
};
use crate::delay_profile::DelayProfile;
use crate::types::{
    PendingRelease, PendingReleaseObservation, PendingReleaseRole, PendingReleaseStatus,
};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PendingGrabOutcome {
    Grabbed {
        scope: SubmissionScope,
    },
    /// The release remains the best choice, but its delay profile still holds
    /// it. The caller must not try a lower-ranked release.
    Parked,
    /// The indexer no longer serves this artifact. A standby walk expires only
    /// that row and tries the next candidate; other callers defer without a
    /// blocklist entry.
    SourceGone,
    Rejected,
    Deferred,
}

/// Which path is promoting a pending release.
///
/// The two are not interchangeable: the automatic path re-judges the release
/// against current policy before grabbing it, while an operator's explicit
/// grab-now has already overruled that verdict. Sonarr draws the same line — its
/// RSS sync re-runs every specification over the pending list, and its manual
/// grab runs none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingGrabTrigger {
    Automatic,
    Operator,
}

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
    #[expect(
        clippy::too_many_arguments,
        reason = "pending-release orchestration persists the full delayed grab context explicitly"
    )]
    #[cfg(test)]
    pub(crate) async fn insert_pending_release(
        &self,
        wanted: &AcquisitionScopeState,
        title: &scryer_domain::Title,
        release_title: &str,
        release_url: Option<&str>,
        source_kind: Option<DownloadSourceKind>,
        release_size_bytes: Option<i64>,
        release_score: i32,
        scoring_log_json: Option<String>,
        indexer_source: Option<&str>,
        indexer_id: Option<&str>,
        release_guid: Option<&str>,
        delay_minutes: i64,
        source_password: Option<&str>,
        published_at: Option<&str>,
        info_hash: Option<&str>,
        seed_minimums: ReleaseSeedMinimums,
        seeders: Option<i64>,
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
            indexer_id: indexer_id.map(str::to_string),
            release_guid: release_guid.map(str::to_string),
            added_at: now.to_rfc3339(),
            last_observed_at: now.to_rfc3339(),
            delay_until: delay_until.to_rfc3339(),
            status: PendingReleaseStatus::Waiting,
            grabbed_at: None,
            source_password: crate::normalize_release_password(source_password),
            published_at: published_at.map(str::to_string),
            info_hash: info_hash.map(str::to_string),
            seed_minimums,
            seeders,
            release_identity: String::new(),
            coverage_identity: String::new(),
            role: PendingReleaseRole::Primary,
            last_decision_code: None,
            release_age_unknown: false,
        };

        match self
            .services
            .workflow
            .pending_releases
            .insert_pending_release(&pending)
            .await
        {
            Ok(_) => {
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

    /// Persist an explicitly derived indexer observation. This path preserves
    /// the caller's eligibility instant and identity facts verbatim.
    pub(crate) async fn insert_pending_release_observation(
        &self,
        pending: &PendingRelease,
        observation: &PendingReleaseObservation,
    ) -> AppResult<String> {
        self.services
            .workflow
            .pending_releases
            .insert_pending_release_observation(pending, observation)
            .await
    }

    /// Order one scope's expired pending releases best-first, on facts derived
    /// now rather than remembered from when they were parked (BL3).
    ///
    /// The key is `RankHead`'s: refused releases last, then tier, then revision,
    /// then score — the same order `evaluate_admission` compares in, so the
    /// release this picks is the one the gate would prefer. A scope whose title
    /// or profile cannot be resolved keeps the stored order: an unorderable
    /// group is still worth trying, and the gate refuses whatever it should.
    async fn order_expired_releases_by_rank(
        &self,
        wanted: &AcquisitionScopeState,
        releases: &mut [PendingRelease],
    ) {
        if releases.len() < 2 {
            return;
        }
        let Ok(Some(title)) = self
            .services
            .catalog
            .titles
            .get_by_id(&wanted.title_id)
            .await
        else {
            releases.sort_by_key(|release| std::cmp::Reverse(release.release_score));
            return;
        };
        let Ok(profile) = self.resolve_quality_profile_for_title(&title).await else {
            releases.sort_by_key(|release| std::cmp::Reverse(release.release_score));
            return;
        };
        let context = self
            .resolve_canonical_scoring_context(&title, &profile)
            .await;
        let catalog_episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await
            .unwrap_or_default();
        let catalog_collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();

        let mut keys: std::collections::HashMap<String, (bool, usize, i32, i32)> =
            std::collections::HashMap::with_capacity(releases.len());
        for release in releases.iter() {
            let facts = crate::quality::canonical_context::score_parked_release_title(
                &title,
                &release.release_title,
                release.release_size_bytes,
                &catalog_episodes,
                &catalog_collections,
                &context,
            );
            keys.insert(
                release.id.clone(),
                (
                    !facts.allowed,
                    crate::admission::tier_sort_key(facts.tier_index),
                    facts.revision.saturating_neg(),
                    facts.score.saturating_neg(),
                ),
            );
        }
        releases.sort_by(|left, right| {
            keys.get(&left.id)
                .cmp(&keys.get(&right.id))
                .then_with(|| left.id.cmp(&right.id))
        });
    }

    /// Park the best candidate of a scope for human review (Pillar A3): the
    /// auto path rejected it as `ambiguous_identity`, so it is neither grabbed
    /// nor silently dropped. The row carries no delay semantics — the expiry
    /// processor only reads `waiting` rows — and is resolved by the existing
    /// grab-now / dismiss actions on the pending-releases view.
    ///
    /// Idempotent per scope: an already-parked row for the same wanted item
    /// short-circuits, so repeated cycles do not pile up review rows.
    pub(crate) async fn park_pending_release_for_review(
        &self,
        wanted: &AcquisitionScopeState,
        title: &scryer_domain::Title,
        candidate: &IndexerSearchResult,
        release_score: i32,
        scoring_log_json: Option<String>,
    ) {
        let existing = self
            .services
            .workflow
            .pending_releases
            .list_pending_releases_for_title(&title.id)
            .await
            .unwrap_or_default();
        // One review row per TITLE, not per wanted item: identity ambiguity is
        // a title-level condition, and a 24-episode ambiguous season must not
        // flood the review queue with 24 identical rows.
        if existing
            .iter()
            .any(|release| release.status == PendingReleaseStatus::NeedsReview)
        {
            return;
        }

        let now = Utc::now();
        let canonical_source = candidate.canonical_download_source();
        let pending = PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: candidate.title.clone(),
            release_url: canonical_source.as_ref().map(|(source, _)| source.clone()),
            source_kind: canonical_source
                .as_ref()
                .map(|(_, kind)| *kind)
                .or(candidate.source_kind),
            release_size_bytes: candidate.size_bytes,
            release_score,
            scoring_log_json,
            indexer_source: Some(candidate.source.clone()),
            indexer_id: candidate.indexer_id.clone(),
            release_guid: candidate.guid.clone(),
            added_at: now.to_rfc3339(),
            last_observed_at: now.to_rfc3339(),
            // No timer applies; the column is NOT NULL, so the parked row simply
            // records when review was requested.
            delay_until: now.to_rfc3339(),
            status: PendingReleaseStatus::NeedsReview,
            grabbed_at: None,
            source_password: crate::normalize_release_password(candidate.password_hint.as_deref()),
            published_at: candidate.published_at.clone(),
            info_hash: candidate
                .extra
                .get("info_hash")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            seed_minimums: ReleaseSeedMinimums::from_release_extra(&candidate.extra),
            // Same map the admission gate read when the candidate was offered,
            // so promotion can re-judge the swarm against the current threshold.
            seeders: crate::acquisition::seed_goals::seeders_from_extra(&candidate.extra),
            release_identity: String::new(),
            coverage_identity: String::new(),
            role: PendingReleaseRole::Primary,
            last_decision_code: None,
            release_age_unknown: false,
        };

        match self
            .services
            .workflow
            .pending_releases
            .insert_pending_release(&pending)
            .await
        {
            Ok(_) => info!(
                title = title.name.as_str(),
                release = candidate.title.as_str(),
                score = release_score,
                "pending release: parked for review — canonical title is ambiguous"
            ),
            Err(error) => warn!(
                error = %error,
                title = title.name.as_str(),
                release = candidate.title.as_str(),
                "pending release: failed to park ambiguous-identity candidate"
            ),
        }
    }

    /// Process pending releases whose delay has expired.
    /// Called periodically from the acquisition poller.
    pub async fn process_expired_pending_releases(&self) -> AppResult<u32> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        let expired = self
            .services
            .workflow
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
            let Some(wanted) = self
                .services
                .workflow
                .acquisition_scope_states
                .get_acquisition_scope_state_by_id(&wanted_item_id)
                .await?
            else {
                // Wanted item gone — mark all as expired
                for pr in &releases {
                    let _ = self
                        .services
                        .workflow
                        .pending_releases
                        .expire_pending_release(&pr.id, "wanted_item_missing")
                        .await;
                }
                continue;
            };

            // Skip if already grabbed or completed
            if wanted.status == AcquisitionScopeStatus::Grabbed
                || wanted.status == AcquisitionScopeStatus::Completed
            {
                // A successful grab retires only freshly judged lower-or-equal
                // overlaps. Keep unresolved candidates here; a higher-quality
                // fallback remains eligible for a later upgrade.
                continue;
            }

            // **Which release gets grabbed is a decision, so it is made on
            // freshly-derived facts** (D13/D20, BL3). The stored
            // `release_score` is what the release scored when it was parked —
            // under whatever profile, persona and rule packs were live then,
            // and on the pre-Chunk-1 scale for anything parked before the
            // upgrade — and it does not carry the tier at all, so ordering by
            // it grabbed a 720p release at a stale 900 ahead of a 2160p one at
            // 400 and marked the 2160p `Superseded` without ever scoring it.
            //
            // Ordered by the search rank's own key, which is the same ladder
            // admission compares on: allowed, tier, revision, score.
            self.order_expired_releases_by_rank(&wanted, &mut releases)
                .await;

            // Try to grab the best release
            let mut grabbed = false;
            for pr in &releases {
                match self
                    .try_grab_pending_release(&wanted, pr, &now, PendingGrabTrigger::Automatic)
                    .await
                {
                    Ok(PendingGrabOutcome::Grabbed { .. }) => {
                        // Mark this one as grabbed
                        let _ = self
                            .services
                            .workflow
                            .pending_releases
                            .update_pending_release_status(
                                &pr.id,
                                PendingReleaseStatus::Grabbed,
                                Some(&now.to_rfc3339()),
                            )
                            .await;
                        grabbed = true;
                        grabbed_count += 1;
                        break;
                    }
                    Ok(PendingGrabOutcome::Rejected) => {
                        // This release couldn't be grabbed (blocklisted, etc) — try next
                        let _ = self
                            .services
                            .workflow
                            .pending_releases
                            .expire_pending_release(&pr.id, "pending_release_rejected")
                            .await;
                    }
                    Ok(PendingGrabOutcome::Deferred) => {
                        info!(
                            release = pr.release_title.as_str(),
                            "pending release: download client unavailable, keeping release pending"
                        );
                        break;
                    }
                    Ok(PendingGrabOutcome::SourceGone) => {
                        info!(
                            release = pr.release_title.as_str(),
                            "pending release: source gone outside standby walk; keeping it pending"
                        );
                        break;
                    }
                    Ok(PendingGrabOutcome::Parked) => {
                        info!(
                            release = pr.release_title.as_str(),
                            "pending release: delay profile still holds the best release"
                        );
                        break;
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            release = pr.release_title.as_str(),
                            "pending release: grab attempt failed"
                        );
                        let _ = self
                            .services
                            .workflow
                            .pending_releases
                            .expire_pending_release(&pr.id, "pending_release_processing_error")
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
    pub async fn list_pending_releases(&self, actor: &User) -> AppResult<Vec<PendingRelease>> {
        let authorized_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if authorized_library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let releases = self
            .services
            .workflow
            .pending_releases
            .list_waiting_pending_releases()
            .await?;
        let mut allowed = Vec::with_capacity(releases.len());
        for release in releases {
            let Some(title) = self
                .services
                .catalog
                .titles
                .get_by_id(&release.title_id)
                .await?
            else {
                continue;
            };
            if authorized_library_ids.contains(&title.library_id) {
                allowed.push(release);
            }
        }
        Ok(allowed)
    }

    /// Paged, storage-side counterpart to [`Self::list_pending_releases`]. The
    /// `waiting` base set, the optional `title_id` / `wanted_item_id` / `statuses`
    /// filters, library authorization, ordering, limit/offset, and the total
    /// count are all resolved in SQL. Returns `(page, total_matching)`.
    pub async fn list_pending_releases_page(
        &self,
        actor: &User,
        title_id: Option<String>,
        wanted_item_id: Option<String>,
        statuses: Vec<PendingReleaseStatus>,
        limit: i64,
        offset: i64,
    ) -> AppResult<(Vec<PendingRelease>, i64)> {
        let authorized_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?;
        if authorized_library_ids.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let sort = if wanted_item_id.is_some()
            && !statuses.is_empty()
            && statuses
                .iter()
                .all(|status| *status == PendingReleaseStatus::Standby)
        {
            PendingReleasePageSort::ReleaseScoreDesc
        } else {
            PendingReleasePageSort::DelayUntilAsc
        };
        let query = PendingReleasesPageQuery {
            library_ids: authorized_library_ids,
            title_id,
            wanted_item_id,
            statuses: statuses
                .into_iter()
                .map(|status| status.as_str().to_string())
                .collect(),
            limit,
            offset,
            sort,
        };
        self.services
            .workflow
            .pending_releases
            .list_pending_releases_page(query)
            .await
    }

    pub async fn get_pending_release(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<PendingRelease>> {
        let release = self
            .services
            .workflow
            .pending_releases
            .get_pending_release(id)
            .await?;
        if let Some(release) = release.as_ref() {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(&release.title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {}", release.title_id)))?;
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(release)
    }

    /// Require `View` on the library owning `wanted_item_id`, resolving the
    /// library from the wanted item (falling back to its title). Shared by the
    /// per-wanted-item pending-release reads.
    async fn require_wanted_item_view(&self, actor: &User, wanted_item_id: &str) -> AppResult<()> {
        let wanted = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("wanted item {wanted_item_id}")))?;
        let library_id = if let Some(library_id) = wanted.library_id.as_deref() {
            library_id.to_string()
        } else {
            self.services
                .catalog
                .titles
                .get_by_id(&wanted.title_id)
                .await?
                .map(|title| title.library_id)
                .ok_or_else(|| AppError::NotFound(format!("title {}", wanted.title_id)))?
        };
        self.require_library_permission(actor, &library_id, scryer_domain::LibraryPermission::View)
            .await
    }

    pub async fn list_pending_releases_for_wanted_item(
        &self,
        actor: &User,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        self.require_wanted_item_view(actor, wanted_item_id).await?;
        self.services
            .workflow
            .pending_releases
            .list_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    /// Paged, storage-side counterpart to
    /// [`Self::list_pending_releases_for_wanted_item`]. Authorization is scoped to
    /// the single wanted item, so no library filter is pushed to the query.
    pub async fn list_pending_releases_for_wanted_item_page(
        &self,
        actor: &User,
        wanted_item_id: &str,
        limit: i64,
        offset: i64,
    ) -> AppResult<(Vec<PendingRelease>, i64)> {
        self.require_wanted_item_view(actor, wanted_item_id).await?;
        let query = PendingReleasesPageQuery {
            library_ids: Vec::new(),
            title_id: None,
            wanted_item_id: Some(wanted_item_id.to_string()),
            statuses: Vec::new(),
            limit,
            offset,
            sort: PendingReleasePageSort::ReleaseScoreDesc,
        };
        self.services
            .workflow
            .pending_releases
            .list_pending_releases_page(query)
            .await
    }

    /// Force-grab a pending release immediately, ignoring the delay.
    pub async fn force_grab_pending_release(&self, actor: &User, id: &str) -> AppResult<bool> {
        let pr = self
            .services
            .workflow
            .pending_releases
            .get_pending_release(id)
            .await?;
        let Some(pr) = pr else {
            return Err(AppError::Repository(format!(
                "pending release {id} not found"
            )));
        };
        if !pr.status.is_open_for_review() {
            return Err(AppError::Repository(format!(
                "pending release {id} is not in waiting status"
            )));
        }
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&pr.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", pr.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let now = Utc::now();
        let wanted = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(&pr.wanted_item_id)
            .await?
            .ok_or_else(|| {
                AppError::Repository(format!("wanted item {} not found", pr.wanted_item_id))
            })?;
        Ok(matches!(
            self.try_grab_pending_release(&wanted, &pr, &now, PendingGrabTrigger::Operator)
                .await?,
            PendingGrabOutcome::Grabbed { .. }
        ))
    }

    /// Dismiss a pending release (set status to dismissed).
    pub async fn dismiss_pending_release(&self, actor: &User, id: &str) -> AppResult<bool> {
        let pr = self
            .services
            .workflow
            .pending_releases
            .get_pending_release(id)
            .await?;
        let Some(pr) = pr else {
            return Err(AppError::Repository(format!(
                "pending release {id} not found"
            )));
        };
        if !pr.status.is_open_for_review() {
            return Err(AppError::Repository(format!(
                "pending release {id} is not in waiting status"
            )));
        }
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&pr.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", pr.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        // Dismissing a review row is a verdict, not a deferral: burn the release
        // so the same ambiguous release cannot be re-offered and the scope
        // converges on a different candidate (Pillar A3). The per-title
        // blocklist entry is what search-time exclusion consults (and what the
        // operator can remove); the Failed attempt is the audit record.
        if pr.status == PendingReleaseStatus::NeedsReview {
            let reason = "dismissed from review: ambiguous identity".to_string();
            let _ = self
                .services
                .workflow
                .release_attempts
                .record_release_attempt(
                    Some(title.id.clone()),
                    pr.release_url.clone(),
                    Some(pr.release_title.clone()),
                    ReleaseDownloadAttemptOutcome::Failed,
                    Some(reason.clone()),
                    crate::normalize_release_password(pr.source_password.as_deref()),
                )
                .await;
            if let Err(error) = self
                .services
                .workflow
                .blocklist_repo
                .block(&NewBlocklistEntry {
                    title_id: title.id.clone(),
                    release_name: pr.release_title.clone(),
                    indexer_id: pr.indexer_id.clone().unwrap_or_default(),
                    info_hash: pr.info_hash.clone(),
                    reason: Some(reason),
                })
                .await
            {
                warn!(
                    error = %error,
                    title_id = title.id.as_str(),
                    release = pr.release_title.as_str(),
                    "failed to persist blocklist entry for dismissed review release"
                );
            }
        }
        self.services
            .workflow
            .pending_releases
            .update_pending_release_status(id, PendingReleaseStatus::Dismissed, None)
            .await?;
        Ok(true)
    }

    /// Attempt to grab a single pending release.
    pub(crate) async fn try_grab_pending_release(
        &self,
        wanted: &AcquisitionScopeState,
        pr: &PendingRelease,
        now: &chrono::DateTime<Utc>,
        trigger: PendingGrabTrigger,
    ) -> AppResult<PendingGrabOutcome> {
        // Load title
        let Some(title) = self.services.catalog.titles.get_by_id(&pr.title_id).await? else {
            return Ok(PendingGrabOutcome::Rejected);
        };

        // Check the per-title blocklist (the single, removable exclusion source).
        let db_blocklist = self
            .load_title_release_blocklist_signatures(&title.id)
            .await;

        if crate::app_usecase_discovery::is_release_blocklisted(
            pr.indexer_id.as_deref(),
            &pr.release_title,
            pr.info_hash.as_deref(),
            &db_blocklist,
        ) {
            return Ok(PendingGrabOutcome::Rejected);
        }

        // Check if this release is already active in the download client.
        // Without this check, the pending processor could retry a release
        // that's currently downloading (e.g. grabbed via background search
        // while this pending release was waiting).
        let dl_snapshot = super::acquisition_workflow::DownloadClientSnapshot::fetch(self).await;
        if dl_snapshot.is_active(&pr.release_title) {
            info!(
                release = pr.release_title.as_str(),
                "pending release: skipping, already active in download client"
            );
            return Ok(PendingGrabOutcome::Rejected);
        }

        // Swarm health, re-judged against the threshold in force *now*. The row
        // carries the count the indexer reported when it was parked (migration
        // 0169) and the threshold may have moved since — a raised floor, a new
        // profile, a fresh Prowlarr import. A delayed grab must not land in a
        // swarm too small to finish it just because it was healthy an hour ago.
        // Same shape as Sonarr's RSS sync, which re-runs every specification
        // over the pending list against the release's originally stored seeders;
        // an unknown count stays eligible there and here.
        if trigger == PendingGrabTrigger::Automatic {
            let minimum_seeders = self
                .minimum_seeders_for_indexer(pr.indexer_id.as_deref())
                .await;
            if !crate::acquisition::seed_goals::meets_minimum_seeders(
                pr.source_kind,
                pr.indexer_id.as_deref(),
                pr.seeders,
                minimum_seeders,
            ) {
                info!(
                    release = pr.release_title.as_str(),
                    indexer_id = pr.indexer_id.as_deref().unwrap_or("unknown"),
                    seeders = ?pr.seeders,
                    minimum_seeders,
                    reason =
                        crate::acquisition_release_search::ReleaseAutoDecisionCode::MinimumSeeders
                            .as_str(),
                    "pending release: rejecting, too few seeders for this indexer's seeding profile"
                );
                return Ok(PendingGrabOutcome::Rejected);
            }
        }

        // What the parked release actually covers, re-derived from its own title
        // against the catalog — the same derivation the search lane does.
        //
        // The anchor state row is not the answer: the RSS pack lane parks a
        // season pack against its *first monitored member's* row, so
        // `wanted.submission_scope()` reports `Episode { anchor }` and the whole
        // pack would be judged against one episode when the delay elapsed — no
        // per-member gate, no `unaired_members`, no `SeasonIncomplete`. D8's
        // "one pack gate" had a hole exactly the size of the delay lane.
        let catalog_episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await
            .unwrap_or_default();
        let catalog_collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();
        // Resolved **once**, and the answer is the one that gets submitted (MA3).
        // The submit block used to parse and resolve coverage a second time with
        // a different parse context (an episode anchor rather than the title's
        // catalog), so a season pack whose collection did not resolve was gated
        // as a single episode and then submitted as a season pack: no per-member
        // subject, no `unaired_members`, no `SeasonIncomplete`, and a download
        // record describing something the gate never judged.
        //
        // Scoring waits for the profile below; only the coverage is needed here.
        let pending_scope_fallback = wanted.submission_scope();
        let pending_parse_context = crate::release_parser::build_release_parse_context_for_title(
            &title,
            &catalog_episodes,
            Some(title.facet.as_str()),
        );
        let pending_parsed = crate::release_parser::parse_release_metadata_for_target(
            &pr.release_title,
            &pending_parse_context,
        );
        let pending_coverage = crate::acquisition_coverage::resolve_release_coverage(
            &pending_parsed,
            &catalog_episodes,
            &catalog_collections,
            None,
        );
        let pending_scope = pending_coverage.submission_scope_or(&pending_scope_fallback);

        // A parked release adopted onto an episode scope must not contradict
        // that episode's numbering. The Unknown-coverage fallback above
        // otherwise stamps a release numbered for a *different* episode (an
        // absolute-numbered anime release, most commonly) as covering the
        // wanted one, and the standby walk then burns the whole parked
        // sequence — grab, import mismatch, next — one release per pass.
        if let SubmissionScope::Episode { episode_id } = &pending_scope
            && let Some(requested) = catalog_episodes
                .iter()
                .find(|episode| &episode.id == episode_id)
            && crate::acquisition_coverage::parsed_release_contradicts_requested_episode(
                &pending_parsed,
                requested,
            )
        {
            info!(
                release = pr.release_title.as_str(),
                episode_id = episode_id.as_str(),
                reason =
                    crate::acquisition_release_search::ReleaseAutoDecisionCode::EpisodeMismatch
                        .as_str(),
                "pending release: rejecting, parsed numbering contradicts the wanted episode"
            );
            return Ok(PendingGrabOutcome::Rejected);
        }

        let is_series_pack = pending_parsed
            .episode
            .as_ref()
            .is_some_and(|episode| episode.is_series_pack);
        let existing_files = match self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
        {
            Ok(files) => files
                .into_iter()
                .filter(|file| file.role.is_primary())
                .collect::<Vec<_>>(),
            Err(error) if is_series_pack => {
                warn!(
                    title_id = title.id.as_str(),
                    error = %error,
                    "pending series pack: media ownership is unavailable; deferring retry"
                );
                return Ok(PendingGrabOutcome::Deferred);
            }
            Err(_) => Vec::new(),
        };
        if is_series_pack {
            let mut owned_episode_ids = existing_files
                .iter()
                .filter_map(|file| file.episode_id.clone())
                .collect::<std::collections::HashSet<_>>();
            let submissions = match self
                .services
                .workflow
                .download_submissions
                .list_for_title(&title.id)
                .await
            {
                Ok(submissions) => submissions,
                Err(error) => {
                    warn!(
                        title_id = title.id.as_str(),
                        error = %error,
                        "pending series pack: submission ownership is unavailable; deferring retry"
                    );
                    return Ok(PendingGrabOutcome::Deferred);
                }
            };
            let identities = submissions
                .iter()
                .map(crate::contracts::ClientJobLocator::from_submission)
                .collect::<Vec<_>>();
            let tracked_states = match self
                .services
                .workflow
                .download_submissions
                .list_identity_tracked_states_for_client_items(&identities)
                .await
            {
                Ok(states) => states
                    .into_iter()
                    .filter_map(|(identity, state)| {
                        scryer_domain::TrackedDownloadState::from_str_opt(&state)
                            .map(|state| (identity, state))
                    })
                    .collect(),
                Err(error) => {
                    warn!(
                        title_id = title.id.as_str(),
                        error = %error,
                        "pending series pack: tracked submission ownership is unavailable; deferring retry"
                    );
                    return Ok(PendingGrabOutcome::Deferred);
                }
            };
            owned_episode_ids.extend(
                crate::acquisition_coverage::in_flight_series_pack_episode_ids(
                    &catalog_episodes,
                    &submissions,
                    &tracked_states,
                    &dl_snapshot,
                ),
            );
            if !crate::acquisition_coverage::series_pack_missing_ratio_qualifies(
                &pending_parsed,
                &catalog_episodes,
                &owned_episode_ids,
            ) {
                return Ok(PendingGrabOutcome::Rejected);
            }
        }
        let cutoff_scope = self.cutoff_scope_for(&pending_scope).await;
        let analyzed_cutoff_quality =
            crate::acquisition::decision_helpers::analyzed_cutoff_quality_for_scope(
                &existing_files,
                &cutoff_scope,
            );
        // A resolution failure defers rather than errors: the callers expire
        // parked releases on Err, and a possibly transient settings problem
        // must not permanently burn delayed or standby candidates.
        let upgrade_context = match self
            .resolve_upgrade_context_for_title_with_category_and_quality(
                &title,
                None,
                analyzed_cutoff_quality,
            )
            .await
        {
            Ok(context) => context,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = title.id.as_str(),
                    "pending release: failed to resolve quality profile; keeping release pending"
                );
                return Ok(PendingGrabOutcome::Deferred);
            }
        };

        // A delayed release faces the same gate as an immediate one, against what
        // is on disk now rather than what the ledger remembered when it was
        // queued — the wait is exactly when the library can have moved on.
        let scoring_context = self
            .resolve_canonical_scoring_context(&title, &upgrade_context.profile)
            .await;

        // **Re-scored, not remembered** (D13/D20). The stored `release_score` was
        // written when the release was parked, under whatever profile, persona,
        // rule packs and scoring algorithm were live then — a delay profile can
        // hold a release for hours and an operator can edit a profile in the
        // meantime. Sonarr re-runs its whole decision engine over pending
        // releases on every sync; so do we, from the two facts the row keeps.
        //
        // The column stays for display and history. It is not an input.
        let facts = crate::quality::canonical_context::score_parked_release_title(
            &title,
            &pr.release_title,
            pr.release_size_bytes,
            &catalog_episodes,
            &catalog_collections,
            &scoring_context,
        );
        if !facts.allowed {
            // A profile edit while the release waited now vetoes it. Expired,
            // not grabbed: there is nothing to wait for.
            info!(
                release = pr.release_title.as_str(),
                codes = ?facts.block_codes,
                "pending release: rejected by the current profile on re-scoring"
            );
            return Ok(PendingGrabOutcome::Rejected);
        }
        let candidate_runtime_minutes = facts.size_basis.total_runtime_minutes;
        let candidate_score = facts.score;

        let mut admission = self
            .admission_subject_for_scope(
                &title,
                &pending_scope,
                &scoring_context,
                candidate_runtime_minutes,
                crate::quality::canonical_context::SubjectIntent::Grab,
            )
            .await;
        // D18: whatever is already downloading for this scope is a
        // pseudo-incumbent. A parked release has no submission of its own yet,
        // so nothing here can self-block.
        let mut queued = Vec::new();
        if !dl_snapshot.queue_listing_failed() {
            let submissions = self
                .services
                .workflow
                .download_submissions
                .list_for_title(&title.id)
                .await
                .unwrap_or_default();
            if !submissions.is_empty() {
                let identities = submissions
                    .iter()
                    .map(crate::contracts::ClientJobLocator::from_submission)
                    .collect::<Vec<_>>();
                let tracked_states = self
                    .services
                    .workflow
                    .download_submissions
                    .list_identity_tracked_states_for_client_items(&identities)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|(identity, state)| {
                        scryer_domain::TrackedDownloadState::from_str_opt(&state)
                            .map(|state| (identity, state))
                    })
                    .collect();
                let membership = self.scope_membership_for(&title, &pending_scope).await;
                queued = self
                    .queued_releases_for_scope(
                        &title,
                        &membership.view(),
                        &scoring_context,
                        &submissions,
                        &tracked_states,
                        &dl_snapshot,
                        &catalog_episodes,
                        &catalog_collections,
                    )
                    .await;
            }
        }
        // The ledger's recorded grab claims the scope even when the client
        // shows nothing for it this pass.
        let membership = self.scope_membership_for(&title, &pending_scope).await;
        let queued = self
            .queued_releases_with_grabbed_claims(
                queued,
                &title,
                &membership.view(),
                &scoring_context,
                &catalog_episodes,
                &catalog_collections,
            )
            .await;
        admission = admission.with_queued(queued);
        let policy = crate::admission::AdmissionPolicy {
            allow_upgrades: upgrade_context.profile.criteria.allow_upgrades,
            min_delta: upgrade_context.thresholds.same_tier_min_delta,
            // Sonarr reads `UpgradeAllowed ? CutoffFormatScore : MinFormatScore`
            // here; the `else` arm is unreachable in this ladder, because a
            // no-upgrade profile returns `UpgradesDisabled` before either gate
            // consults the cutoff. So this is just the cutoff (D19).
            cutoff_score: upgrade_context.profile.criteria.cutoff_score,
            manual_override: false,
            // D18: the grab lanes, and only the grab lanes, treat in-flight
            // submissions as pseudo-incumbents.
            applies_to_queue: true,
        };
        // Tier, revision and score all out of the one derivation, so the parked
        // release cannot be compared by facts that disagree with each other.
        let candidate_facts = crate::admission::CandidateFacts::new(
            facts.tier_index,
            facts.revision,
            candidate_score,
        )
        .with_release_title(&pr.release_title);
        // The same candidate-aware cutoff gate the search and RSS lanes run
        // (D15). It used to be a scope-level `if cutoff_reached { return }`
        // above the score, which meant a PROPER parked behind a delay profile
        // was thrown away when the delay elapsed. A parked release is
        // reconsidered outside an active search — Sonarr's
        // `SearchCriteria == null` — so the old-file guard binds here too.
        if let Some(code) = crate::acquisition_release_search::cutoff_refusal(
            candidate_facts,
            &admission,
            crate::acquisition_release_search::incumbent_at_cutoff(
                upgrade_context.cutoff_reached,
                &admission,
                upgrade_context.profile.criteria.cutoff_score,
            ),
            true,
            now,
        ) {
            info!(
                release = pr.release_title.as_str(),
                decision = code.as_str(),
                "pending release: refused by the cutoff gate"
            );
            return Ok(PendingGrabOutcome::Rejected);
        }

        if !crate::admission::evaluate_admission(&admission, candidate_facts, &policy).is_admitted()
        {
            return Ok(PendingGrabOutcome::Rejected);
        }
        if let Some(incumbent) = admission.best_incumbent()
            && crate::acquisition_policy::upgrade_cooldown_is_active(
                crate::acquisition_policy::CooldownCandidate {
                    tier_index: candidate_facts.tier_index,
                    score: candidate_score,
                },
                incumbent,
                wanted.last_search_at.as_deref(),
                now,
                &upgrade_context.thresholds,
            )
        {
            return Ok(PendingGrabOutcome::Rejected);
        }

        let source_hint = pr.release_url.clone();
        let source_kind = pr
            .source_kind
            .or_else(|| DownloadSourceKind::infer_from_hint(source_hint.as_deref()));

        // A pending row is not grandfathered into the old delay decision. The
        // row may have waited because of a different profile, and an operator
        // may lengthen that profile before the promotion or standby walk gets
        // here. Evaluate only after admission/cooldown so a held row is one
        // that would otherwise be grabbed.
        if trigger == PendingGrabTrigger::Automatic {
            let delay_profiles = self.load_delay_profiles().await;
            if let Some(delay_decision) = crate::delay_profile::grab_time_delay_decision(
                &delay_profiles,
                &title.tags,
                &title.facet,
                source_kind,
                pr.published_at
                    .as_deref()
                    .and_then(crate::quality_profile::parse_published_at),
                candidate_score,
                crate::quality_profile::parse_published_at(&pr.added_at),
                now,
            ) && delay_decision.should_hold()
            {
                let delay_until = *now + Duration::minutes(delay_decision.effective_delay_minutes);
                if let Err(error) = self
                    .services
                    .workflow
                    .pending_releases
                    .update_pending_release_delay_until(&pr.id, &delay_until.to_rfc3339())
                    .await
                {
                    warn!(
                        error = %error,
                        release = pr.release_title.as_str(),
                        "pending release: failed to extend delay; keeping current row intact"
                    );
                    return Ok(PendingGrabOutcome::Deferred);
                }
                if let Err(error) = self
                    .services
                    .workflow
                    .pending_releases
                    .update_pending_release_status(&pr.id, PendingReleaseStatus::Waiting, None)
                    .await
                {
                    warn!(
                        error = %error,
                        release = pr.release_title.as_str(),
                        "pending release: failed to park delayed release"
                    );
                    return Ok(PendingGrabOutcome::Deferred);
                }
                crate::acquisition_workflow::record_pending_release_decision(
                    self,
                    wanted,
                    &title,
                    pr,
                    candidate_score,
                    crate::acquisition_release_search::ReleaseAutoDecisionCode::PendingDelay,
                    admission.best_score(),
                    now,
                )
                .await;
                return Ok(PendingGrabOutcome::Parked);
            }
        }

        // Submit to download client
        let source_title = Some(pr.release_title.clone());
        let request_signature = normalize_release_selection_signature(
            source_hint.as_deref(),
            source_title.as_deref(),
            source_kind,
        );
        let source_password = crate::normalize_release_password(pr.source_password.as_deref());

        let _ = self
            .services
            .workflow
            .release_attempts
            .record_release_attempt(
                Some(title.id.clone()),
                source_hint.clone(),
                source_title.clone(),
                ReleaseDownloadAttemptOutcome::Pending,
                None,
                source_password.clone(),
            )
            .await;

        let download_cat = self.derive_download_category(&title.facet).await;
        let is_recent = self.is_recent_for_queue_priority(
            pr.published_at
                .as_deref()
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

        let download_id = scryer_domain::download_identity::DownloadId::new();

        // Season-pack detection for the seeding-goal resolver. The submission
        // scope this function derives later needs catalog lookups that only run
        // once the grab succeeds, so the pack signal comes straight off the
        // release title parse (Sonarr's `ParsedEpisodeInfo.FullSeason`).
        let is_season_pack = parse_release_metadata_for_target(
            &pr.release_title,
            &build_release_parse_context(&title, None, None, Some(title.facet.as_str())),
        )
        .episode
        .is_some_and(|episode| episode.full_season);

        let canonical_result = self
            .submit_canonical_download(CanonicalDownloadSubmissionIntent {
                request: DownloadClientAddRequest {
                    title: title.clone(),
                    search_facet: (wanted.media_type == "series_movie")
                        .then_some(scryer_domain::MediaFacet::Movie),
                    purpose: crate::DownloadSubmissionPurpose::Standard,
                    download_id: Some(download_id),
                    source_hint: source_hint.clone(),
                    staged_nzb: None,
                    resolved_download_artifact: None,
                    source_kind,
                    source_title: source_title.clone(),
                    source_password: source_password.clone(),
                    category: Some(download_cat),
                    queue_priority: None,
                    download_directory: None,
                    release_title: Some(pr.release_title.clone()),
                    indexer_name: pr.indexer_source.clone(),
                    indexer_id: pr.indexer_id.clone(),
                    info_hash_hint: pr.info_hash.clone(),
                    seed_goal_ratio: None,
                    seed_goal_seconds: None,
                    // Captured off the release `extra` map when the row was parked
                    // (migration 0165), so a delayed grab gets the same tracker
                    // clamp as an immediate one. Rows parked before that migration
                    // carry `None` and simply fall back to the profile's own goals.
                    tracker_min_seed_ratio: pr.seed_minimums.min_seed_ratio,
                    tracker_min_seed_time_minutes: pr.seed_minimums.min_seed_time_minutes,
                    season_pack_seed_ratio: pr.seed_minimums.season_pack_seed_ratio,
                    season_pack_seed_time_minutes: pr.seed_minimums.season_pack_seed_time_minutes,
                    is_recent,
                    season_pack: is_season_pack.then_some(true),
                    pinned_download_client_id: None,
                },
                scope: pending_scope.clone(),
                conflict_policy: SubmissionConflictPolicy::Skip,
                request_signature: request_signature.clone(),
                source_provider_name: pr.indexer_source.clone(),
                release_size_bytes: pr.release_size_bytes,
            })
            .await;

        record_grab_submission_outcome(
            GrabTrigger::Pending,
            &title.facet,
            pr.indexer_source.as_deref(),
            &canonical_result,
        );

        let canonical_submission = match canonical_result {
            Ok(CanonicalDownloadSubmissionOutcome::Accepted(submission)) => Ok(submission),
            Ok(CanonicalDownloadSubmissionOutcome::Conflict(_)) => {
                return Ok(PendingGrabOutcome::Deferred);
            }
            Err(error) => Err(error),
        };

        match canonical_submission {
            Ok(canonical_submission) => {
                let grab = canonical_submission.grab;
                self.record_indexer_grab(pr.indexer_id.as_deref(), pr.indexer_source.as_deref());

                let _ = self
                    .services
                    .workflow
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint,
                        source_title.clone(),
                        ReleaseDownloadAttemptOutcome::Success,
                        None,
                        source_password.clone(),
                    )
                    .await;

                // **The scope that was gated is the scope that is submitted**
                // (MA3). This block used to resolve coverage a second time,
                // against a different parse context, and could land on a
                // different answer than the gate above.
                let submission_scope = pending_scope.clone();
                let grabbed_json = serde_json::json!({
                    "title": pr.release_title,
                    "score": pr.release_score,
                    "grabbed_at": now.to_rfc3339(),
                    "source": "pending_release",
                    "source_provider": pr.indexer_source.clone(),
                })
                .to_string();
                let download_job_id = grab.job_id.clone();
                let covered_wanted_item_ids = self
                    .covered_wanted_item_ids_for_submission_scope(
                        &title.id,
                        &submission_scope,
                        &wanted.id,
                    )
                    .await?;

                self.services
                    .workflow
                    .acquisition_state
                    .commit_successful_grab(&SuccessfulGrabCommit {
                        wanted_item_id: wanted.id.clone(),
                        covered_wanted_item_ids,
                        grabbed_release: grabbed_json,
                        last_search_at: Some(now.to_rfc3339()),
                        grabbed_pending_release_id: Some(pr.id.clone()),
                        grabbed_at: Some(now.to_rfc3339()),
                    })
                    .await?;
                let _ = self
                    .append_domain_event(new_title_domain_event(
                        None,
                        &title,
                        DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                            title: title_context_snapshot(&title),
                            source_title: Some(pr.release_title.clone()),
                            source_hint: None,
                            source_provider: None,
                            download_id: Some(download_job_id),
                            episode_ids: wanted.episode_id.iter().cloned().collect(),
                        }),
                    ))
                    .await;

                Ok(PendingGrabOutcome::Grabbed {
                    scope: pending_scope,
                })
            }
            Err(err) => {
                warn!(
                    title = title.name.as_str(),
                    release = pr.release_title.as_str(),
                    error = %err,
                    "pending release: download submission failed"
                );

                // An ambiguous submit (the request may have been accepted but
                // the response was lost) is deferred exactly like an
                // unavailable client. A gone source is similarly never a
                // blocklist reason, although a standby walk may skip that row.
                let defer = is_download_submit_unavailable_error(&err)
                    || err.is_download_submit_ambiguous();
                let source_gone = err.is_download_source_gone();

                let _ = self
                    .services
                    .workflow
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint.clone(),
                        source_title.clone(),
                        if defer || source_gone {
                            ReleaseDownloadAttemptOutcome::Pending
                        } else {
                            ReleaseDownloadAttemptOutcome::Failed
                        },
                        Some(err.to_string()),
                        source_password.clone(),
                    )
                    .await;

                if source_gone {
                    info!(
                        release = pr.release_title.as_str(),
                        "pending release: download source is gone"
                    );
                    return Ok(PendingGrabOutcome::SourceGone);
                }

                if defer {
                    return Ok(PendingGrabOutcome::Deferred);
                }

                // A definitive submit failure burns the release for this title:
                // the per-title blocklist entry is what search-time exclusion
                // consults (and what the operator can remove); the Failed
                // attempt above is the audit record.
                if let Some(release_name) = source_title.clone()
                    && let Err(error) = self
                        .services
                        .workflow
                        .blocklist_repo
                        .block(&NewBlocklistEntry {
                            title_id: title.id.clone(),
                            release_name,
                            indexer_id: pr.indexer_id.clone().unwrap_or_default(),
                            info_hash: pr.info_hash.clone(),
                            reason: Some(format!("grab failed: {err}")),
                        })
                        .await
                {
                    warn!(
                        error = %error,
                        title_id = title.id.as_str(),
                        release = pr.release_title.as_str(),
                        "failed to persist blocklist entry for failed pending release grab"
                    );
                }
                Ok(PendingGrabOutcome::Rejected)
            }
        }
    }
}
