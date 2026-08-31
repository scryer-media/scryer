fn parsed_release_season_pack_season(parsed: &crate::ParsedReleaseMetadata) -> Option<u32> {
    parsed.episode.as_ref().and_then(|episode| {
        (episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
            && !episode.is_season_extra)
            .then_some(episode.season)
            .flatten()
    })
}
fn candidate_is_season_pack_for_season(candidate: &IndexerSearchResult, season_num: u32) -> bool {
    let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
        return false;
    };

    parsed_release_season_pack_season(parsed) == Some(season_num)
}
#[derive(Clone, Debug, Default)]
struct FailedReleaseAttribution {
    title: Option<Title>,
    episode_ids: Vec<String>,
    collection_id: Option<String>,
}
fn release_quality_hint(source_title: Option<&str>) -> Option<String> {
    source_title.and_then(|title| crate::parse_release_metadata(title).quality)
}
async fn resolve_failed_release_attribution(
    app: &AppUseCase,
    title_id: Option<&str>,
    failed_submission: Option<&DownloadSubmission>,
    wanted_item: Option<&AcquisitionScopeState>,
    failed_collection_items: Option<&[AcquisitionScopeState]>,
) -> FailedReleaseAttribution {
    let title = match title_id {
        Some(title_id) => app
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await
            .ok()
            .flatten(),
        None => None,
    };

    let mut attribution = FailedReleaseAttribution {
        title,
        ..Default::default()
    };

    if let Some(submission) = failed_submission {
        if let Some(episode_ids) = submission.scope.episode_ids() {
            for episode_id in episode_ids {
                push_unique_episode_id(&mut attribution.episode_ids, Some(episode_id));
            }
        }
        attribution.collection_id = submission.scope.collection_id().map(str::to_string);
    }

    if let Some(item) = wanted_item {
        push_unique_episode_id(&mut attribution.episode_ids, item.episode_id.as_deref());
        if attribution.collection_id.is_none() {
            attribution.collection_id = item.collection_id.clone();
        }
    }

    if let Some(items) = failed_collection_items {
        for item in items {
            push_unique_episode_id(&mut attribution.episode_ids, item.episode_id.as_deref());
            if attribution.collection_id.is_none() {
                attribution.collection_id = item.collection_id.clone();
            }
        }
    }

    attribution
}
pub(crate) fn download_submission_scope_for_release_title(
    item: &AcquisitionScopeState,
    episode: Option<&Episode>,
    release_title: &str,
) -> SubmissionScope {
    if item.media_type == "episode" {
        let parsed = crate::parse_release_metadata(release_title);
        if parsed.episode.as_ref().is_some_and(|episode| {
            episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
        }) {
            return collection_download_submission_scope_for_wanted_item(item, episode);
        }
    }

    direct_download_submission_scope_for_wanted_item(item, episode)
}
/// What a scope stands for once its catalog lookups are done.
///
/// Scope intersection is asked in two places — "does this submission block that
/// wanted row" and "is this submission in flight for the scope I am about to
/// grab into" (D18) — and they must not answer differently, which they would if
/// each pattern-matched the enum in its own direction. Resolving the target side
/// to members first makes one predicate serve both.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ScopeMembership<'a> {
    /// Episodes the scope covers. Empty for title and series-movie-link scopes.
    pub episode_ids: &'a [String],
    /// Collections those episodes belong to; usually one, empty when unresolved.
    pub collection_ids: &'a [String],
    pub series_movie_link_id: Option<&'a str>,
}

/// [`ScopeMembership`] with the lookups owned, for callers that resolve it once
/// and then borrow it per submission.
#[derive(Debug, Clone, Default)]
pub(crate) struct OwnedScopeMembership {
    pub episode_ids: Vec<String>,
    pub collection_ids: Vec<String>,
    pub series_movie_link_id: Option<String>,
}

impl OwnedScopeMembership {
    pub(crate) fn view(&self) -> ScopeMembership<'_> {
        ScopeMembership {
            episode_ids: &self.episode_ids,
            collection_ids: &self.collection_ids,
            series_movie_link_id: self.series_movie_link_id.as_deref(),
        }
    }
}

/// Does a submission's scope overlap the scope described by `target`?
pub(crate) fn submission_scope_intersects(
    submission: &SubmissionScope,
    target: &ScopeMembership<'_>,
) -> bool {
    let covers = |episode_id: &String| target.episode_ids.contains(episode_id);
    match submission {
        // An orphan grab occupies nothing.
        SubmissionScope::Orphan => false,
        // A title-scoped download is the whole title, so it covers everything
        // under it.
        SubmissionScope::Title => true,
        SubmissionScope::Episode { episode_id } => covers(episode_id),
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids.iter().any(covers),
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => target.series_movie_link_id == Some(series_movie_link_id.as_str()),
        SubmissionScope::Collection { collection_id } => {
            target.collection_ids.iter().any(|id| id == collection_id)
        }
    }
}

pub(crate) fn submission_blocks_wanted_item(
    submission: &DownloadSubmission,
    item: &AcquisitionScopeState,
    episode_collection_id: Option<&str>,
) -> bool {
    let episode_ids: Vec<String> = if item.media_type == "episode" {
        item.episode_id.clone().into_iter().collect()
    } else {
        Vec::new()
    };
    let collection_ids: Vec<String> = if item.media_type == "episode" {
        episode_collection_id
            .map(str::to_string)
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    submission_scope_intersects(
        &submission.scope,
        &ScopeMembership {
            episode_ids: &episode_ids,
            collection_ids: &collection_ids,
            series_movie_link_id: (item.media_type == "series_movie")
                .then_some(item.series_movie_link_id.as_deref())
                .flatten(),
        },
    )
}
fn resolved_failed_release_hint(failed_submission: Option<&DownloadSubmission>) -> Option<String> {
    failed_submission
        .and_then(|submission| normalize_release_attempt_hint(submission.source_hint.as_deref()))
}
async fn mark_wanted_item_failed_without_reacquire(
    app: &AppUseCase,
    item: &AcquisitionScopeState,
) -> AppResult<()> {
    app.services
        .workflow
        .acquisition_scope_states
        .update_acquisition_scope_status(
            &item.id,
            AcquisitionScopeStatus::Wanted.as_str(),
            item.last_search_at.as_deref(),
            None,
        )
        .await
        .map_err(|err| {
            warn!(
                wanted_item_id = item.id.as_str(),
                title_id = item.title_id.as_str(),
                error = %err,
                "failed to mark wanted item failed without scheduling reacquisition"
            );
            err
        })
}
async fn load_recent_failed_season_pack_seasons_for_title(
    app: &AppUseCase,
    title_id: &str,
    now: &DateTime<Utc>,
) -> HashSet<u32> {
    let cutoff = *now - Duration::minutes(FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES);

    // Every hard failure writes a per-title blocklist entry, so the cooldown
    // reads recent entries (newest first) rather than the failed-attempt log,
    // which is history/audit only and never gates.
    match app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(title_id, 200)
        .await
    {
        Ok(entries) => entries
            .into_iter()
            .filter_map(|entry| {
                let release_name = entry.release_name;
                let created_at = crate::quality_profile::parse_published_at(&entry.created_at)?;
                (created_at >= cutoff)
                    .then(|| crate::parse_release_metadata(&release_name))
                    .and_then(|parsed| parsed_release_season_pack_season(&parsed))
            })
            .collect(),
        Err(err) => {
            warn!(
                title_id,
                error = %err,
                "failed to load recent failed season pack blocklist entries"
            );
            HashSet::new()
        }
    }
}
impl AppUseCase {
    async fn wanted_item_is_mismatch_recovery_candidate(
        &self,
        item: &AcquisitionScopeState,
    ) -> AppResult<bool> {
        let decisions = self
            .services
            .workflow
            .acquisition_scope_states
            .list_release_decisions_for_acquisition_scope_state(&item.id, 10, 0)
            .await?;
        Ok(!decisions.is_empty()
            && decisions
                .iter()
                .all(|decision| decision.decision_code == "title_mismatch"))
    }
}
impl AppUseCase {
    pub async fn wanted_item_mismatch_recovery_eligible(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<bool> {
        let Some(item) = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(wanted_item_id)
            .await?
        else {
            return Ok(false);
        };

        self.wanted_item_is_mismatch_recovery_candidate(&item).await
    }
}
impl AppUseCase {
    pub async fn trigger_title_mismatch_recovery_search(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<usize> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let items = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                statuses: vec!["wanted".into()],
                title_id: Some(title_id.to_string()),
                limit: 500,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?;

        let mut queued = 0usize;
        for item in &items {
            if !self
                .wanted_item_is_mismatch_recovery_candidate(item)
                .await?
            {
                continue;
            }

            // This operator-authorized recovery can change a title or alias
            // without changing the search fingerprint, so it must override
            // convergence and search every routed indexer again.
            self.reopen_wanted_scope_for_acquisition(item, CoverageReopen::All)
                .await;
            queued += 1;
        }

        Ok(queued)
    }
}
impl AppUseCase {
    async fn queue_monitored_series_items_for_search(
        &self,
        title: &Title,
        _now: &DateTime<Utc>,
    ) -> AppResult<WantedSearchOutcome> {
        self.reopen_series_scopes_for_search(title, None).await
    }
}
impl AppUseCase {
    /// Re-open every fileless monitored episode scope of `title` (optionally
    /// restricted to one season) for acquisition: the derived target set already
    /// contains them; the re-open prunes all coverage so even converged scopes
    /// are searched again on the next cycle (§D5 — a trigger overrides
    /// convergence). Scopes with an in-flight grab are skipped.
    async fn reopen_series_scopes_for_search(
        &self,
        title: &Title,
        season_number: Option<&str>,
    ) -> AppResult<WantedSearchOutcome> {
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await?;

        let existing_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|file| file.role.is_primary())
            .collect::<Vec<_>>();
        let episodes_with_files: std::collections::HashSet<String> = existing_files
            .iter()
            .filter_map(|file| file.episode_id.clone())
            .collect();
        let mut outcome = WantedSearchOutcome::default();

        for collection in &collections {
            if !collection.monitored {
                continue;
            }

            let episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_collection(&collection.id)
                .await?;

            for episode in &episodes {
                if !episode.monitored || episodes_with_files.contains(&episode.id) {
                    continue;
                }
                if let Some(season) = season_number
                    && episode.season_number.as_deref() != Some(season)
                {
                    continue;
                }

                let item = match self
                    .services
                    .workflow
                    .acquisition_scope_states
                    .get_acquisition_scope_state_for_title(&title.id, Some(&episode.id))
                    .await?
                {
                    Some(item) => {
                        if item.status == AcquisitionScopeStatus::Grabbed {
                            continue;
                        }
                        item
                    }
                    None => self.new_wanted_state_view(
                        title,
                        "episode",
                        Some(episode.id.clone()),
                        None,
                        None,
                        episode.season_number.clone(),
                    ),
                };

                let scheduled = self
                    .reopen_wanted_scope_with_policy(title, &item, SubmissionConflictPolicy::Skip)
                    .await?;
                outcome.queued_count += scheduled.queued_count;
                outcome.skipped_in_progress_count += scheduled.skipped_in_progress_count;
                if outcome.conflict.is_none() {
                    outcome.conflict = scheduled.conflict;
                }
            }
        }

        Ok(outcome)
    }
}
