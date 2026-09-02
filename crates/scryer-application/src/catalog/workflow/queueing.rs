const RECENT_QUEUE_PRIORITY_WINDOW_DAYS: i64 = 14;
fn wanted_item_candidates_for_submission_scope(
    title_id: &str,
    scope: &SubmissionScope,
    episodes: &[Episode],
) -> Vec<(AcquisitionScopeState, Option<String>)> {
    match scope {
        SubmissionScope::Orphan => Vec::new(),
        SubmissionScope::Title => vec![(
            AcquisitionScopeState {
                id: String::new(),
                title_id: title_id.to_string(),
                title_name: None,
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: None,
                collection_id: None,
                series_movie_link_id: None,
                season_number: None,
                episode_number: None,
                media_type: "movie".to_string(),
                last_search_at: None,
                status: AcquisitionScopeStatus::Wanted,
                grabbed_release: None,
                landed_bar: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: String::new(),
                updated_at: String::new(),
            },
            None,
        )],
        SubmissionScope::Episode { episode_id } => {
            let candidate = episodes
                .iter()
                .find(|episode| episode.id == *episode_id)
                .map(|episode| {
                    (
                        wanted_item_candidate_for_episode(title_id, episode),
                        episode.collection_id.clone(),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        wanted_item_candidate_for_episode_id(title_id, episode_id, None, None),
                        None,
                    )
                });
            vec![candidate]
        }
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids
            .iter()
            .map(|episode_id| {
                episodes
                    .iter()
                    .find(|episode| episode.id == *episode_id)
                    .map(|episode| {
                        (
                            wanted_item_candidate_for_episode(title_id, episode),
                            episode.collection_id.clone(),
                        )
                    })
                    .unwrap_or_else(|| {
                        (
                            wanted_item_candidate_for_episode_id(title_id, episode_id, None, None),
                            None,
                        )
                    })
            })
            .collect(),
        SubmissionScope::Collection { collection_id } => episodes
            .iter()
            .filter(|episode| episode.collection_id.as_deref() == Some(collection_id.as_str()))
            .map(|episode| {
                (
                    wanted_item_candidate_for_episode(title_id, episode),
                    episode.collection_id.clone(),
                )
            })
            .collect(),
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => vec![(
            AcquisitionScopeState {
                id: String::new(),
                title_id: title_id.to_string(),
                title_name: None,
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: None,
                collection_id: None,
                series_movie_link_id: Some(series_movie_link_id.clone()),
                season_number: None,
                episode_number: None,
                media_type: "series_movie".to_string(),
                last_search_at: None,
                status: AcquisitionScopeStatus::Wanted,
                grabbed_release: None,
                landed_bar: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: String::new(),
                updated_at: String::new(),
            },
            None,
        )],
    }
}
fn wanted_item_candidate_for_episode(title_id: &str, episode: &Episode) -> AcquisitionScopeState {
    wanted_item_candidate_for_episode_id(
        title_id,
        &episode.id,
        episode.collection_id.clone(),
        episode.season_number.clone(),
    )
}
fn wanted_item_candidate_for_episode_id(
    title_id: &str,
    episode_id: &str,
    collection_id: Option<String>,
    season_number: Option<String>,
) -> AcquisitionScopeState {
    AcquisitionScopeState {
        landed_bar: None,
        id: String::new(),
        title_id: title_id.to_string(),
        title_name: None,
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: Some(episode_id.to_string()),
        collection_id,
        series_movie_link_id: None,
        season_number,
        episode_number: None,
        media_type: "episode".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: String::new(),
        updated_at: String::new(),
    }
}
fn submission_for_scope(title_id: &str, scope: &SubmissionScope) -> DownloadSubmission {
    DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        title_id: title_id.to_string(),
        // Scope matching only; this submission is never persisted.
        release_size_bytes: None,
        facet: String::new(),
        download_client_id: None,
        download_client_type: String::new(),
        download_client_item_id: String::new(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: None,
        info_hash: None,
        request_signature: None,
        purpose: DownloadSubmissionPurpose::Standard,
        scope: scope.clone(),
    }
}
async fn episode_ids_for_queue_scope(app: &AppUseCase, scope: &SubmissionScope) -> Vec<String> {
    match scope {
        SubmissionScope::Episode { episode_id } => vec![episode_id.clone()],
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids.clone(),
        SubmissionScope::Collection { collection_id } => app
            .services
            .catalog
            .shows
            .list_episodes_for_collection(collection_id)
            .await
            .map(|episodes| episodes.into_iter().map(|episode| episode.id).collect())
            .unwrap_or_default(),
        SubmissionScope::Title | SubmissionScope::SeriesMovie { .. } | SubmissionScope::Orphan => {
            Vec::new()
        }
    }
}
fn validate_manual_queue_purpose(
    purpose: DownloadSubmissionPurpose,
    title: &Title,
    scope: &SubmissionScope,
) -> AppResult<()> {
    if !purpose.is_additional_file() {
        return Ok(());
    }

    match scope {
        SubmissionScope::Title if title.facet == MediaFacet::Movie => Ok(()),
        SubmissionScope::Title => Err(AppError::Validation(
            "additional-file title queueing supports only movie titles".into(),
        )),
        SubmissionScope::Episode { .. } => Ok(()),
        SubmissionScope::EpisodeSet { .. } => Err(AppError::Validation(
            "additional-file queueing supports only title and single-episode scopes".into(),
        )),
        SubmissionScope::Collection { .. } => Err(AppError::Validation(
            "additional-file queueing does not support collection scopes yet".into(),
        )),
        SubmissionScope::SeriesMovie { .. } => Ok(()),
        SubmissionScope::Orphan => Err(AppError::Validation(
            "additional-file queueing requires a title or episode scope".into(),
        )),
    }
}
/// `Warning` blocks like any other live state: the download is still in the
/// client and — unlike a failure — is never cleaned up on its own, so treating
/// it as absent would leave the operator with two grabs for one release.
/// Sonarr's `QueueSpecification` skips only `FailedPending` for the same reason.
fn queue_state_blocks_submission(state: DownloadQueueState) -> bool {
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
/// Blocking a warned grab without letting it be replaced would be a dead end:
/// nothing else removes it, so the operator needs the swap.
fn queue_state_is_replaceable(state: DownloadQueueState) -> bool {
    matches!(
        state,
        DownloadQueueState::Queued
            | DownloadQueueState::Downloading
            | DownloadQueueState::Paused
            | DownloadQueueState::Warning
    )
}
pub(crate) fn queue_item_matches_submission(
    item: &DownloadQueueItem,
    submission: &DownloadSubmission,
) -> bool {
    item.download_client_item_id == submission.download_client_item_id
        && submission
            .download_client_id
            .as_deref()
            .map(|client_id| client_id == item.client_id)
            .unwrap_or(true)
}
impl AppUseCase {
    pub(crate) fn is_recent_for_queue_priority(&self, baseline_date: Option<&str>) -> Option<bool> {
        baseline_date.map(|_| {
            release_is_recent_for_queue_priority(baseline_date, RECENT_QUEUE_PRIORITY_WINDOW_DAYS)
        })
    }
}
impl AppUseCase {
    pub async fn list_title_release_blocklist(
        &self,
        actor: &User,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        let entries = self
            .services
            .workflow
            .blocklist_repo
            .list_for_title(title_id, limit.clamp(1, 1_000))
            .await?;
        Ok(entries
            .into_iter()
            .map(|entry| TitleReleaseBlocklistEntry {
                id: entry.id,
                release_name: entry.release_name,
                error_message: entry.reason,
                attempted_at: entry.created_at,
            })
            .collect())
    }
}
impl AppUseCase {
    /// Clears one blocked release, re-allowing it for the title immediately.
    ///
    /// One delete is the whole operation: the schema keys a block on its
    /// release, so there is no second row naming the same one to sweep up.
    pub async fn clear_title_release_blocklist_entry(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<()> {
        let entry = self
            .services
            .workflow
            .blocklist_repo
            .get(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("blocklist entry {id}")))?;
        self.require_title_permission(
            actor,
            &entry.title_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.services.workflow.blocklist_repo.remove(id).await
    }
}
impl AppUseCase {
    async fn queue_manual_release_for_title(
        &self,
        actor: &User,
        title: &Title,
        queued_release: QueuedReleaseSelection,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
        purpose: DownloadSubmissionPurpose,
    ) -> AppResult<QueueDownloadOutcome> {
        validate_manual_queue_purpose(purpose, title, &scope)?;
        let QueuedReleaseSelection {
            indexer_id,
            source_hint,
            source_kind,
            source_title,
            source_password,
            info_hash_hint,
            size_bytes,
            seeders,
        } = queued_release;
        let source_provider_name = if let Some(indexer_id) = indexer_id.as_deref() {
            self.services
                .integrations
                .indexer_configs
                .get_by_id(indexer_id)
                .await?
                .map(|config| config.name)
        } else {
            None
        };
        let source_hint_for_attempt = normalize_release_attempt_value(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_value(source_title.as_deref());
        let request_signature = normalize_release_selection_signature(
            source_hint_for_attempt.as_deref(),
            source_title_for_attempt.as_deref(),
            source_kind,
        );
        let source_password = normalize_release_password(source_password.as_deref());
        let _ = self
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

        let category = self.derive_download_category(&title.facet).await;
        let is_recent = self.is_recent_for_queue_priority(
            title
                .first_aired
                .as_deref()
                .or(title.digital_release_date.as_deref()),
        );
        let download_id = scryer_domain::download_identity::DownloadId::new();
        let job_result = self
            .submit_canonical_download(CanonicalDownloadSubmissionIntent {
                request: DownloadClientAddRequest {
                    title: title.clone(),
                    search_facet: matches!(scope, SubmissionScope::SeriesMovie { .. })
                        .then_some(scryer_domain::MediaFacet::Movie),
                    purpose,
                    download_id: Some(download_id),
                    source_hint,
                    staged_nzb: None,
                    resolved_download_artifact: None,
                    source_kind,
                    source_title: source_title_for_attempt.clone(),
                    source_password: source_password.clone(),
                    category: Some(category),
                    queue_priority: None,
                    download_directory: None,
                    release_title: None,
                    indexer_name: None,
                    indexer_id: indexer_id.clone(),
                    info_hash_hint: info_hash_hint.clone(),
                    seed_goal_ratio: None,
                    seed_goal_seconds: None,
                    // Manual/interactive queueing carries a resolved source URL,
                    // not the indexer release object, so there is no `extra` map to
                    // read tracker minimums from on this path.
                    tracker_min_seed_ratio: None,
                    tracker_min_seed_time_minutes: None,
                    season_pack_seed_ratio: None,
                    season_pack_seed_time_minutes: None,
                    is_recent,
                    season_pack: matches!(
                        scope,
                        SubmissionScope::EpisodeSet { .. } | SubmissionScope::Collection { .. }
                    )
                    .then_some(true),
                    pinned_download_client_id: None,
                },
                scope: scope.clone(),
                conflict_policy,
                request_signature: request_signature.clone(),
                source_provider_name: source_provider_name.clone(),
                release_size_bytes: size_bytes,
            })
            .await;

        let grab = match job_result {
            Ok(CanonicalDownloadSubmissionOutcome::Accepted(submission))
                if !submission.newly_submitted =>
            {
                return Ok(QueueDownloadOutcome::Queued(QueuedDownloadResult {
                    job_id: submission.grab.job_id,
                    queued_release: QueuedReleaseSelection {
                        indexer_id,
                        source_hint: source_hint_for_attempt,
                        source_kind,
                        source_title: source_title_for_attempt,
                        source_password,
                        info_hash_hint,
                        size_bytes,
                        seeders,
                    },
                    reused_existing: true,
                }));
            }
            Ok(CanonicalDownloadSubmissionOutcome::Conflict(conflict)) => {
                return Ok(QueueDownloadOutcome::Conflict(conflict));
            }
            Ok(CanonicalDownloadSubmissionOutcome::Accepted(submitted)) => {
                let grab = submitted.grab.clone();
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => "manual", "facet" => facet_label).increment(1);
                }
                self.record_indexer_grab(indexer_id.as_deref(), source_provider_name.as_deref());
                if source_title_for_attempt.is_none() {
                    // The persisted indexer release title is THE name the
                    // import parses at completion. Only the API/SDK
                    // `addTitle{sourceHint}` grab can omit it (candidate-token,
                    // best-release, RSS, pending, and auto-search grabs always
                    // carry one); the import then degrades to the client-
                    // reported release name, so leave a grab-time breadcrumb.
                    tracing::info!(
                        title_id = %title.id,
                        client_id = ?grab.client_id,
                        client_type = %grab.client_type,
                        download_client_item_id = %grab.job_id,
                        source_hint = ?source_hint_for_attempt,
                        "queued download submission without a release title; import will parse the client-reported release name"
                    );
                }
                let submission_identity = ClientJobLocator::new(
                    grab.client_id.as_deref(),
                    &grab.client_type,
                    &grab.job_id,
                );
                let submission_actor = crate::domain_events::DomainEventActor::from(actor)
                    .into_download_submission_actor_snapshot();
                if let Err(error) = self
                    .services
                    .workflow
                    .download_submissions
                    .record_submission_actor_snapshot(&submission_identity, submission_actor)
                    .await
                {
                    tracing::warn!(
                        error = %error,
                        client_id = ?grab.client_id,
                        client_type = %grab.client_type,
                        download_client_item_id = %grab.job_id,
                        "download_submission_actor_snapshot_persistence_failed"
                    );
                }
                let _ = self
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
                grab
            }
            Err(error) => {
                let source_gone = error.is_download_source_gone();
                let submit_unavailable = is_download_submit_unavailable_error(&error)
                    || error.is_download_submit_ambiguous()
                    || source_gone;
                let error_message = error.to_string();
                if source_gone {
                    tracing::info!(
                        release = ?source_title_for_attempt,
                        "operator download source gone; leaving it unblocked"
                    );
                }
                let _ = self
                    .services
                    .workflow
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt.clone(),
                        source_title_for_attempt.clone(),
                        if submit_unavailable {
                            ReleaseDownloadAttemptOutcome::Pending
                        } else {
                            ReleaseDownloadAttemptOutcome::Failed
                        },
                        Some(error_message.clone()),
                        source_password,
                    )
                    .await;
                if !submit_unavailable && let Some(release_name) = source_title_for_attempt.clone()
                {
                    // The per-title blocklist entry is what search-time
                    // exclusion consults (and what the operator can remove);
                    // the Failed attempt above is the audit record.
                    let _ = self
                        .services
                        .workflow
                        .blocklist_repo
                        .block(&NewBlocklistEntry {
                            title_id: title.id.clone(),
                            release_name,
                            indexer_id: indexer_id.clone().unwrap_or_default(),
                            info_hash: info_hash_hint.clone(),
                            reason: Some(error_message.clone()),
                        })
                        .await;
                }
                return Err(error);
            }
        };

        let grabbed_episode_ids = episode_ids_for_queue_scope(self, &scope).await;

        self.append_domain_event(new_title_domain_event(
            actor,
            title,
            DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                title: title_context_snapshot(title),
                source_title: source_title_for_attempt.clone(),
                source_hint: source_hint_for_attempt.clone(),
                source_provider: source_provider_name.clone(),
                download_id: Some(grab.job_id.clone()),
                episode_ids: grabbed_episode_ids,
            }),
        ))
        .await?;

        Ok(QueueDownloadOutcome::Queued(QueuedDownloadResult {
            job_id: grab.job_id,
            queued_release: QueuedReleaseSelection {
                indexer_id,
                source_hint: source_hint_for_attempt,
                source_kind,
                source_title: source_title_for_attempt,
                source_password,
                info_hash_hint,
                size_bytes,
                seeders,
            },
            reused_existing: false,
        }))
    }
}
impl AppUseCase {
    pub async fn add_title_and_queue_download_with_outcome(
        &self,
        actor: &User,
        request: NewTitle,
        queued_release: QueuedReleaseSelection,
    ) -> AppResult<AddTitleAndQueueDownloadOutcome> {
        let library_id = scryer_domain::default_library_id_for_facet(&request.facet);
        self.add_title_and_queue_download_with_outcome_in_library(
            actor,
            request,
            library_id,
            queued_release,
        )
        .await
    }
}
impl AppUseCase {
    pub async fn add_title_and_queue_download_with_outcome_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        queued_release: QueuedReleaseSelection,
    ) -> AppResult<AddTitleAndQueueDownloadOutcome> {
        self.add_title_and_queue_download_with_options_patch_outcome_in_library(
            actor,
            request,
            library_id,
            TitleOptionsPatch::default(),
            queued_release,
        )
        .await
    }

    pub async fn add_title_and_queue_download_with_options_patch_outcome_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        options_patch: TitleOptionsPatch,
        queued_release: QueuedReleaseSelection,
    ) -> AppResult<AddTitleAndQueueDownloadOutcome> {
        let add_outcome = self
            .add_title_with_options_patch_outcome_in_library(
                actor,
                request,
                library_id,
                options_patch,
            )
            .await?;
        let title = add_outcome.title.clone();
        let queued = self
            .queue_manual_release_for_title(
                actor,
                &title,
                queued_release,
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
                DownloadSubmissionPurpose::OperatorQueued,
            )
            .await?;
        let QueueDownloadOutcome::Queued(queued) = queued else {
            return Err(AppError::Validation(
                "a download is already queued for this title".into(),
            ));
        };

        Ok(AddTitleAndQueueDownloadOutcome {
            title,
            metadata_hydration_state: add_outcome.metadata_hydration_state,
            reused_existing_title: add_outcome.reused_existing_title,
            download_job_id: queued.job_id,
            reused_queued_download: queued.reused_existing,
        })
    }
}
impl AppUseCase {
    pub async fn add_title_and_queue_download(
        &self,
        actor: &User,
        request: NewTitle,
        queued_release: QueuedReleaseSelection,
    ) -> AppResult<(Title, String)> {
        let outcome = self
            .add_title_and_queue_download_with_outcome(actor, request, queued_release)
            .await?;
        Ok((outcome.title, outcome.download_job_id))
    }
}
impl AppUseCase {
    pub async fn queue_existing_title_download(
        &self,
        actor: &User,
        title_id: &str,
        queued_release: QueuedReleaseSelection,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        self.queue_existing_title_download_with_purpose(
            actor,
            title_id,
            queued_release,
            scope,
            conflict_policy,
            DownloadSubmissionPurpose::Standard,
        )
        .await
    }

    pub async fn queue_existing_title_download_with_purpose(
        &self,
        actor: &User,
        title_id: &str,
        queued_release: QueuedReleaseSelection,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
        purpose: DownloadSubmissionPurpose,
    ) -> AppResult<QueueDownloadOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.queue_manual_release_for_title(
            actor,
            &title,
            queued_release,
            scope,
            conflict_policy,
            purpose,
        )
        .await
    }
}
impl AppUseCase {
    /// Queue an operator-chosen replacement for a title/episode's existing primary
    /// file. On import the replacement always lands (it bypasses the required-audio
    /// gate and forces the upgrade, recycling the old primary, with a score boost),
    /// and the release that produced the current primary is blocklisted so it is not
    /// auto-re-downloaded over the manual pick.
    pub async fn queue_replacement_release(
        &self,
        actor: &User,
        title_id: &str,
        queued_release: QueuedReleaseSelection,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.blocklist_replaced_primary_release(&title, &scope)
            .await;
        self.queue_manual_release_for_title(
            actor,
            &title,
            queued_release,
            scope,
            conflict_policy,
            DownloadSubmissionPurpose::ManualReplacement,
        )
        .await
    }

    /// Queue a replacement from a signed interactive-search candidate token (the
    /// release the operator chose from search results). Scope comes from the token.
    pub async fn queue_replacement_release_from_candidate_token(
        &self,
        actor: &User,
        title_id: &str,
        candidate_token: &str,
        conflict_policy: SubmissionConflictPolicy,
        announced_size_bytes: Option<i64>,
    ) -> AppResult<QueueDownloadOutcome> {
        let (queued_release, signed_scope) = self
            .verify_release_candidate_token_for_signed_scope(actor, title_id, candidate_token)
            .await?;
        if let Some(announced_size_bytes) = announced_size_bytes
            && queued_release.size_bytes != Some(announced_size_bytes)
        {
            return Err(AppError::Validation(
                "release size does not match the signed candidate".into(),
            ));
        }
        let outcome = self
            .queue_replacement_release(
                actor,
                title_id,
                queued_release.clone(),
                signed_scope,
                conflict_policy,
            )
            .await?;
        Ok(match outcome {
            QueueDownloadOutcome::Queued(mut queued) => {
                queued.queued_release = queued_release;
                QueueDownloadOutcome::Queued(queued)
            }
            QueueDownloadOutcome::Conflict(conflict) => QueueDownloadOutcome::Conflict(conflict),
        })
    }

    /// Blocklist the release(s) that produced the primary file(s) being replaced,
    /// so the auto-poller does not re-download them over a manual replacement.
    async fn blocklist_replaced_primary_release(&self, title: &Title, scope: &SubmissionScope) {
        let episode_ids = episode_ids_for_queue_scope(self, scope).await;
        let primary_files: Vec<crate::TitleMediaFile> = if episode_ids.is_empty() {
            self.services
                .library
                .media_files
                .list_media_files_for_title(&title.id)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|file| file.role.is_primary())
                .collect()
        } else {
            self.services
                .library
                .media_files
                .list_live_media_files_for_episode_ids(&title.id, &episode_ids)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|scoped| scoped.media_file)
                .filter(|file| file.role.is_primary())
                .collect()
        };

        let mut seen = std::collections::HashSet::new();
        for file in primary_files {
            let Some(release_title) = file
                .grabbed_release_title
                .as_deref()
                .and_then(|value| normalize_release_attempt_value(Some(value)))
            else {
                continue;
            };
            if !seen.insert(release_title.to_ascii_lowercase()) {
                continue;
            }
            // The Failed release attempt is the audit record; the per-title
            // blocklist entry below is what search-time exclusion
            // (`is_release_blocklisted`) consults, so the old release is not
            // auto-re-grabbed over the manual replacement.
            let _ = self
                .services
                .workflow
                .release_attempts
                .record_release_attempt(
                    Some(title.id.clone()),
                    None,
                    Some(release_title.clone()),
                    ReleaseDownloadAttemptOutcome::Failed,
                    Some("manual_replacement".to_string()),
                    None,
                )
                .await;
            let _ = self
                .services
                .workflow
                .blocklist_repo
                // No indexer: the release name comes off a file on disk, whose
                // only recorded provenance is the indexer's display name. The
                // empty indexer blocks it everywhere, which is what a manual
                // replacement wants.
                .block(&crate::NewBlocklistEntry {
                    title_id: title.id.clone(),
                    release_name: release_title,
                    indexer_id: String::new(),
                    info_hash: None,
                    reason: Some("manual_replacement".to_string()),
                })
                .await;
        }
    }
}
impl AppUseCase {
    pub async fn queue_existing_title_download_from_candidate_token(
        &self,
        actor: &User,
        title_id: &str,
        candidate_token: &str,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        self.queue_existing_title_download_from_candidate_token_with_purpose(
            actor,
            title_id,
            candidate_token,
            scope,
            conflict_policy,
            DownloadSubmissionPurpose::Standard,
            None,
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the candidate token, caller scope, queue policy, purpose, and announced size are independently validated inputs"
    )]
    pub async fn queue_existing_title_download_from_candidate_token_with_purpose(
        &self,
        actor: &User,
        title_id: &str,
        candidate_token: &str,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
        purpose: DownloadSubmissionPurpose,
        announced_size_bytes: Option<i64>,
    ) -> AppResult<QueueDownloadOutcome> {
        let (queued_release, signed_scope) = self
            .verify_release_candidate_token_for_signed_scope(actor, title_id, candidate_token)
            .await?;
        if let Some(announced_size_bytes) = announced_size_bytes
            && queued_release.size_bytes != Some(announced_size_bytes)
        {
            return Err(AppError::Validation(
                "release size does not match the signed candidate".into(),
            ));
        }
        let outcome = self
            .queue_existing_title_download_with_purpose(
                actor,
                title_id,
                queued_release.clone(),
                signed_scope,
                conflict_policy,
                purpose,
            )
            .await?;
        let _ = scope;
        Ok(match outcome {
            QueueDownloadOutcome::Queued(mut queued) => {
                queued.queued_release = queued_release;
                QueueDownloadOutcome::Queued(queued)
            }
            QueueDownloadOutcome::Conflict(conflict) => QueueDownloadOutcome::Conflict(conflict),
        })
    }
}
impl AppUseCase {
    /// Ask the scorer to choose a release for this scope.
    ///
    /// Although an operator starts this action, the scorer—not the operator—
    /// selects the release, so its submission purpose remains `Standard` and
    /// import guard failures use the automatic convergence policy.
    pub async fn queue_best_release(
        &self,
        actor: &User,
        title_id: &str,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let (search_title, subject) = match &scope {
            SubmissionScope::Title | SubmissionScope::Orphan => (
                title.clone(),
                self.resolve_release_search_subject_for_title(&title)
                    .await?,
            ),
            SubmissionScope::Episode { episode_id } => {
                let episode = self
                    .services
                    .catalog
                    .shows
                    .get_episode_by_id(episode_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
                let season = episode.season_number.clone().ok_or_else(|| {
                    AppError::Validation("episode is missing season number".into())
                })?;
                let episode_number = episode.episode_number.clone().ok_or_else(|| {
                    AppError::Validation("episode is missing episode number".into())
                })?;
                (
                    title.clone(),
                    self.resolve_release_search_subject_for_episode(
                        &title,
                        &season,
                        &episode_number,
                    )
                    .await?,
                )
            }
            SubmissionScope::EpisodeSet { .. } => {
                return Err(AppError::Validation(
                    "best-release search is not supported for multi-episode scopes".into(),
                ));
            }
            SubmissionScope::Collection { .. } => {
                return Err(AppError::Validation(
                    "best-release search is not supported for collection scopes".into(),
                ));
            }
            SubmissionScope::SeriesMovie {
                series_movie_link_id,
            } => {
                let link = self
                    .services
                    .catalog
                    .shows
                    .get_series_movie_link_by_id(series_movie_link_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound(format!("series movie link {}", series_movie_link_id))
                    })?;
                self.resolve_release_search_subject_for_series_movie(&title, &link)
                    .await?
            }
        };

        let wanted = match &scope {
            SubmissionScope::Title => {
                self.resolve_or_create_wanted_state_row(&format!("title:{}", title.id))
                    .await?
            }
            SubmissionScope::Episode { episode_id } => {
                self.resolve_or_create_wanted_state_row(&format!("episode:{episode_id}"))
                    .await?
            }
            SubmissionScope::SeriesMovie {
                series_movie_link_id,
            } => {
                self.resolve_or_create_wanted_state_row(&format!(
                    "series_movie:{series_movie_link_id}"
                ))
                .await?
            }
            SubmissionScope::Orphan
            | SubmissionScope::EpisodeSet { .. }
            | SubmissionScope::Collection { .. } => None,
        };

        let results = self
            .search_and_evaluate_subject(
                &search_title,
                &subject,
                &actor.id,
                SearchMode::Auto,
                tokio_util::sync::CancellationToken::new(),
            )
            .await?;
        if let Some(candidate) = results
            .iter()
            .find(|candidate| candidate.auto_decision_code.as_deref() == Some("ambiguous_identity"))
            && let Some(wanted) = wanted.as_ref()
        {
            let candidate_score = candidate
                .quality_profile_decision
                .as_ref()
                .map(|decision| decision.preference_score)
                .unwrap_or_default();
            self.park_pending_release_for_review(wanted, &title, candidate, candidate_score, None)
                .await;
        }
        let Some(best_index) = results
            .iter()
            .position(|candidate| candidate.auto_eligible == Some(true))
        else {
            return Err(AppError::NoAutoEligibleRelease {
                candidate_count: results.len(),
                reasons: summarize_auto_eligibility_reasons(&results),
            });
        };
        let best = results
            .into_iter()
            .nth(best_index)
            .expect("eligible release index came from results");
        let queue_scope = if matches!(
            &scope,
            SubmissionScope::Collection { .. } | SubmissionScope::SeriesMovie { .. }
        ) {
            scope
        } else if let Some(parsed) = best.parsed_release_metadata.as_ref() {
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
            let requested_episode = match &scope {
                SubmissionScope::Episode { episode_id } => catalog_episodes
                    .iter()
                    .find(|episode| episode.id == *episode_id),
                _ => None,
            };
            crate::acquisition_coverage::resolve_release_coverage(
                parsed,
                &catalog_episodes,
                &catalog_collections,
                requested_episode,
            )
            .submission_scope_or(&scope)
        } else {
            scope
        };

        let canonical_source = best.canonical_download_source();
        self.queue_existing_title_download(
            actor,
            title_id,
            QueuedReleaseSelection {
                indexer_id: best.indexer_id.clone(),
                source_hint: canonical_source.as_ref().map(|(source, _)| source.clone()),
                source_kind: canonical_source
                    .as_ref()
                    .map(|(_, kind)| *kind)
                    .or(best.source_kind),
                source_title: Some(best.title.clone()),
                source_password: best.password_hint.clone(),
                info_hash_hint: best
                    .extra
                    .get("info_hash")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                size_bytes: best.size_bytes,
                seeders: crate::acquisition::seed_goals::seeders_from_extra(&best.extra),
            },
            queue_scope,
            conflict_policy,
        )
        .await
    }
}
fn summarize_auto_eligibility_reasons(
    results: &[IndexerSearchResult],
) -> Vec<crate::AutoEligibilityReason> {
    let mut reasons = std::collections::BTreeMap::new();
    for candidate in results {
        let code = candidate
            .auto_decision_code
            .as_deref()
            .unwrap_or("unknown")
            .to_string();
        let summary = candidate
            .auto_decision_summary
            .as_deref()
            .unwrap_or("automatic eligibility was not evaluated")
            .to_string();
        reasons
            .entry(code.clone())
            .and_modify(|reason: &mut crate::AutoEligibilityReason| reason.count += 1)
            .or_insert(crate::AutoEligibilityReason {
                code,
                summary,
                count: 1,
            });
    }

    let mut reasons: Vec<_> = reasons.into_values().collect();
    reasons.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.code.cmp(&right.code))
    });
    reasons
}

fn normalize_release_attempt_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Grab-time persistence of the indexer release title for the manual queue
/// path (`queue_manual_release_for_title`), which every interactive/API grab
/// funnels through. The best-release, RSS, pending-release, and auto-search
/// paths already assert their persisted `source_title` in `lib_tests`.
#[cfg(test)]
mod grab_time_release_title_tests {
    use crate::{
        AppResult, ClientJobLocator, DownloadSubmission, DownloadSubmissionRepository,
        QueuedReleaseSelection, SubmissionConflictPolicy, SubmissionScope,
    };
    use async_trait::async_trait;
    use scryer_domain::{MediaFacet, NewTitle};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct RecordingDownloadSubmissionRepo {
        rows: Arc<Mutex<Vec<DownloadSubmission>>>,
    }

    #[async_trait]
    impl DownloadSubmissionRepository for RecordingDownloadSubmissionRepo {
        async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
            self.rows.lock().await.push(submission);
            Ok(())
        }

        async fn record_ambiguous_submission(
            &self,
            submission: DownloadSubmission,
        ) -> AppResult<()> {
            self.record_submission(submission).await
        }

        async fn record_submission_with_identity(
            &self,
            submission: DownloadSubmission,
            _: crate::DownloadSubmissionIdentity,
            _: Option<crate::PersistedSeedGoals>,
        ) -> AppResult<crate::CanonicalDownloadIdentityDisposition> {
            self.record_submission(submission).await?;
            Ok(crate::CanonicalDownloadIdentityDisposition::Requested)
        }

        async fn find_by_client_item_id(
            &self,
            identity: &ClientJobLocator,
        ) -> AppResult<Option<DownloadSubmission>> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .find(|row| ClientJobLocator::from_submission(row) == *identity)
                .cloned())
        }

        async fn list_for_client_items(
            &self,
            client_items: &[ClientJobLocator],
        ) -> AppResult<Vec<DownloadSubmission>> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .filter(|row| client_items.contains(&ClientJobLocator::from_submission(row)))
                .cloned()
                .collect())
        }

        async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .filter(|row| row.title_id == title_id)
                .cloned()
                .collect())
        }

        async fn find_by_title_and_request_signature(
            &self,
            title_id: &str,
            request_signature: &str,
            purpose: crate::DownloadSubmissionPurpose,
            scope: &SubmissionScope,
        ) -> AppResult<Option<DownloadSubmission>> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .find(|row| {
                    row.title_id == title_id
                        && row.request_signature.as_deref() == Some(request_signature)
                        && row.purpose == purpose
                        && &row.scope == scope
                })
                .cloned())
        }

        async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
            self.rows
                .lock()
                .await
                .retain(|row| row.title_id != title_id);
            Ok(())
        }

        async fn delete_by_client_item_id(&self, identity: &ClientJobLocator) -> AppResult<()> {
            self.rows
                .lock()
                .await
                .retain(|row| ClientJobLocator::from_submission(row) != *identity);
            Ok(())
        }

        async fn update_tracked_state(&self, _: &ClientJobLocator, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn get_tracked_state(&self, _: &ClientJobLocator) -> AppResult<Option<String>> {
            Ok(None)
        }
    }

    fn movie_request(name: &str) -> NewTitle {
        NewTitle {
            name: name.to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn manual_queue_persists_the_indexer_release_title_on_the_submission_row() {
        let (base_app, user) = crate::lib_tests::bootstrap();
        let submissions = Arc::new(RecordingDownloadSubmissionRepo::default());
        let app = base_app.with_test_overrides(|services| {
            services.with_download_submissions(submissions.clone())
        });
        let title = app
            .add_title(&user, movie_request("Paper Lantern"))
            .await
            .expect("create title");

        // The shape a candidate token / best-release selection arrives in.
        app.queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some("https://indexer.invalid/get/paper-lantern.nzb".to_string()),
                source_kind: None,
                source_title: Some("  Paper.Lantern.2012.1080p.WEB-DL-GRP  ".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: Some(1_234_567),
                seeders: None,
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("queue release");

        let rows = submissions.rows.lock().await.clone();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].title_id, title.id);
        assert_eq!(rows[0].scope, SubmissionScope::Title);
        assert_eq!(rows[0].release_size_bytes, Some(1_234_567));
        assert_eq!(
            rows[0].source_title.as_deref(),
            Some("Paper.Lantern.2012.1080p.WEB-DL-GRP"),
            "the indexer release title must be persisted (trimmed) at grab time"
        );
    }

    #[tokio::test]
    async fn add_title_source_hint_only_grab_records_no_release_title() {
        // The API/SDK `addTitleAndQueueDownload{sourceHint}` path may omit the
        // release title; the row is still a non-orphan Scryer submission (the
        // import degrades only the parsed name, see ReleaseEvidence).
        let (base_app, user) = crate::lib_tests::bootstrap();
        let submissions = Arc::new(RecordingDownloadSubmissionRepo::default());
        let app = base_app.with_test_overrides(|services| {
            services.with_download_submissions(submissions.clone())
        });

        let (title, _job_id) = app
            .add_title_and_queue_download(
                &user,
                movie_request("Harbor Lights"),
                QueuedReleaseSelection {
                    indexer_id: None,
                    source_hint: Some("https://indexer.invalid/get/harbor-lights.nzb".to_string()),
                    source_kind: None,
                    source_title: None,
                    source_password: None,
                    info_hash_hint: None,
                    size_bytes: None,
                    seeders: None,
                },
            )
            .await
            .expect("add title and queue");

        let rows = submissions.rows.lock().await.clone();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].title_id, title.id);
        assert_eq!(rows[0].scope, SubmissionScope::Title);
        assert_eq!(rows[0].source_title, None);
        assert!(crate::import_parameters::submission_has_scryer_origin(
            &rows[0]
        ));
    }
}
