use scryer_domain::{
    Library, LibraryPermission, MediaFacet, MediaIssue, MediaIssueRemediation, MediaIssueStatus,
    MediaRequest, MediaRequestStatus, Title, User,
};
use scryer_rules::request::Observation;

use crate::request_rules::RequestDraft;
use crate::{
    AppError, AppResult, AppUseCase, NewLibraryContentBlock, NewMediaIssue, TitleMediaFile,
};

#[derive(Clone, Debug)]
pub struct SubmitMediaIssueInput {
    pub media_file_id: String,
    pub reason_code: String,
    pub details: Option<String>,
    pub remediation: MediaIssueRemediation,
}

pub const MAX_MEDIA_ISSUE_DETAILS_LENGTH: usize = 4096;

async fn reporter_requested_affected_media(
    app: &AppUseCase,
    actor: &User,
    title: &Title,
    affected_episode_ids: &[String],
    series_movie_link_ids: &[String],
) -> Observation<bool> {
    const REQUEST_LIMIT: usize = 100;
    let requests = app
        .services
        .catalog
        .media_requests
        .list(crate::MediaRequestQuery {
            library_ids: Some(vec![title.library_id.clone()]),
            status: Some(MediaRequestStatus::Approved),
            requester_user_id: Some(actor.id.clone()),
            limit: Some(REQUEST_LIMIT + 1),
            ..Default::default()
        })
        .await;
    let requests = match requests {
        Ok(requests) => requests,
        Err(_) => {
            return Observation::unknown("media_request_scope_lookup_failed");
        }
    };

    let truncated = requests.len() > REQUEST_LIMIT;
    let eligible = requests
        .iter()
        .filter(|request| approved_reporter_request(request, actor, title))
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return if truncated {
            Observation::unknown("media_request_scope_lookup_incomplete")
        } else {
            Observation::known(false)
        };
    }
    if title.facet == MediaFacet::Movie {
        return Observation::known(true);
    }
    if affected_episode_ids.is_empty() && series_movie_link_ids.is_empty() {
        return Observation::unknown("affected_media_scope_missing");
    }
    if affected_episode_ids.len() > 500 || series_movie_link_ids.len() > 1 {
        return Observation::unknown("affected_media_scope_too_large");
    }
    let affected_seasons = if affected_episode_ids.is_empty() {
        Vec::new()
    } else {
        let episodes = match app
            .services
            .catalog
            .shows
            .get_episodes_by_ids(affected_episode_ids)
            .await
        {
            Ok(episodes) => episodes,
            Err(_) => return Observation::unknown("affected_episode_lookup_failed"),
        };
        if episodes.len() != affected_episode_ids.len()
            || episodes.iter().any(|episode| episode.title_id != title.id)
        {
            return Observation::unknown("affected_episode_lookup_incomplete");
        }
        let Some(seasons) = episodes
            .iter()
            .map(|episode| episode.season_number.as_deref()?.parse().ok())
            .collect::<Option<Vec<i32>>>()
        else {
            return Observation::unknown("affected_episode_season_unknown");
        };
        seasons
    };
    let series_movie_ids = match series_movie_link_ids.first() {
        None => None,
        Some(id) => {
            let Ok(Some(link)) = app
                .services
                .catalog
                .shows
                .get_series_movie_link_by_id(id)
                .await
            else {
                return Observation::unknown("affected_series_movie_scope_unknown");
            };
            if link.series_title_id != title.id {
                return Observation::unknown("affected_series_movie_scope_unknown");
            }
            let movie = link.movie;
            let ids = [
                ("tvdb", movie.tvdb_id),
                ("tmdb", movie.tmdb_id),
                ("imdb", movie.imdb_id),
                ("anidb", movie.anidb_id),
                ("mal", movie.mal_id),
            ]
            .into_iter()
            .filter_map(|(source, value)| {
                value.map(|value| scryer_domain::ExternalId {
                    source: source.to_string(),
                    kind: None,
                    value,
                })
            })
            .collect::<Vec<_>>();
            if ids.is_empty() {
                return Observation::unknown("affected_series_movie_scope_unknown");
            }
            Some(ids)
        }
    };

    let mut uncertain = truncated;
    for request in eligible {
        match request_covers_series_scopes(request, &affected_seasons, series_movie_ids.as_deref())
        {
            Some(true) => return Observation::known(true),
            Some(false) => {}
            None => uncertain = true,
        };
    }
    if uncertain {
        Observation::unknown("persisted_request_scope_incomplete")
    } else {
        Observation::known(false)
    }
}

fn approved_reporter_request(request: &MediaRequest, actor: &User, title: &Title) -> bool {
    // Fulfillment does not change the request's persisted Approved status.
    request.status == MediaRequestStatus::Approved
        && request.library_id == title.library_id
        && request.facet == title.facet
        && request.created_title_id.as_deref() == Some(title.id.as_str())
        && (request.created_by_user_id == actor.id
            || request
                .requesters
                .iter()
                .any(|requester| requester.user_id == actor.id))
}

fn request_covers_series_scopes(
    request: &MediaRequest,
    affected_seasons: &[i32],
    series_movie_ids: Option<&[scryer_domain::ExternalId]>,
) -> Option<bool> {
    match request.requested_monitor_type.as_deref() {
        Some("advanced") => {
            let selection = request.requested_monitor_selection.as_ref()?;
            Some(
                media_issue_request_covers_episode_scopes(affected_seasons, Some(selection))
                    && series_movie_ids.is_none_or(|ids| selection.monitors_series_movie(ids)),
            )
        }
        Some("allepisodes" | "monitored")
            if series_movie_ids.is_none() && affected_seasons.iter().all(|season| *season > 0) =>
        {
            Some(true)
        }
        Some("none" | "unmonitored") => Some(false),
        // Defaults and time-dependent monitoring do not retain a frozen set
        // of episodes. Today's monitoring state cannot stand in for it.
        _ => None,
    }
}

impl AppUseCase {
    async fn resolve_media_issue_target(
        &self,
        actor: &User,
        media_file_id: &str,
        expected_library_id: Option<&str>,
    ) -> AppResult<(TitleMediaFile, Title, Library, Vec<String>)> {
        let file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(media_file_id.trim())
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::media::FILE_NOT_FOUND,
                    "media file not found",
                )
            })?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&file.title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    "title not found",
                )
            })?;

        if expected_library_id.is_some_and(|id| id != title.library_id) {
            return Err(AppError::not_found_reason(
                crate::error_reasons::media::FILE_NOT_FOUND,
                "media file not found in this library",
            ));
        }
        self.require_library_permission(actor, &title.library_id, LibraryPermission::Request)
            .await?;
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(&title.library_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::request_rules::SAMPLE_LIBRARY_NOT_FOUND,
                    "media library not found",
                )
            })?;
        let mut episode_ids = self
            .services
            .library
            .media_files
            .score_evaluation_episode_ids(&file)
            .await?;
        episode_ids.sort_unstable();
        episode_ids.dedup();
        Ok((file, title, library, episode_ids))
    }

    pub(crate) async fn preview_media_issue_draft(
        &self,
        actor: &User,
        library_id: &str,
        input: &SubmitMediaIssueInput,
    ) -> AppResult<(Library, RequestDraft)> {
        let (file, title, library, episode_ids) = self
            .resolve_media_issue_target(actor, &input.media_file_id, Some(library_id))
            .await?;
        // Preview performs the same guarded file eligibility check as submit.
        self.inspect_media_issue_file(&file).await?;
        let draft = self
            .media_issue_draft(actor, &title, &file, &episode_ids, input)
            .await;
        Ok((library, draft))
    }

    async fn media_issue_draft(
        &self,
        actor: &User,
        title: &Title,
        file: &TitleMediaFile,
        episode_ids: &[String],
        input: &SubmitMediaIssueInput,
    ) -> RequestDraft {
        let selected_relationship = reporter_requested_affected_media(
            self,
            actor,
            title,
            episode_ids,
            &file.series_movie_link_ids,
        )
        .await;
        let wider_hash_impact = match self
            .services
            .workflow
            .blocklist_repo
            .media_issue_has_other_size_candidates(&title.library_id, &file.id, file.size_bytes)
            .await
        {
            Ok(Some(false)) => scryer_rules::request::WiderHashImpact::None,
            _ => scryer_rules::request::WiderHashImpact::Unknown,
        };
        let relationship = match (&selected_relationship, wider_hash_impact) {
            (
                Observation::Known { value: true, .. },
                scryer_rules::request::WiderHashImpact::Unknown,
            ) => Observation::unknown("wider_hash_impact_unproven"),
            _ => selected_relationship,
        };
        RequestDraft {
            kind: "media_issue".to_string(),
            media_issue: Some(scryer_rules::request::RequestMediaIssueDoc {
                operation: "mark_bad".to_string(),
                reason_code: input.reason_code.clone(),
                remediation: input.remediation.as_str().to_string(),
                title_id: title.id.clone(),
                library_id: title.library_id.clone(),
                media_file_id: file.id.clone(),
                affected_episode_ids: episode_ids.to_vec(),
                affected_series_movie_link_ids: file.series_movie_link_ids.clone(),
                wider_hash_impact,
                provenance: if file
                    .release_listing_json
                    .as_deref()
                    .and_then(crate::release_listing_view)
                    .is_some()
                {
                    "grabbed"
                } else {
                    // Old imports, adopted downloads and scanned rows do not
                    // all distinguish origin reliably.
                    "unknown"
                }
                .to_string(),
            }),
            media_issue_reporter_requested_affected_media: Some(relationship),
            facet: title.facet.clone(),
            title: title.name.clone(),
            year: title.year,
            external_ids: title.external_ids.clone(),
            identity_fingerprint: crate::media_requests::media_request_identity_fingerprint(
                &title.external_ids,
            ),
            quality_profile_id: None,
            quality_profile_name: None,
            monitor_type: None,
            monitor_selection: None,
            requested_lease_days: None,
            requested_required_audio_languages: None,
            requested_preferred_audio_languages: None,
            origin: scryer_domain::MediaRequestOrigin::Manual,
            list_rank: None,
        }
    }

    pub async fn submit_media_issue(
        &self,
        actor: &User,
        input: SubmitMediaIssueInput,
    ) -> AppResult<MediaIssue> {
        if input
            .details
            .as_deref()
            .is_some_and(|details| details.chars().count() > MAX_MEDIA_ISSUE_DETAILS_LENGTH)
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::media::DETAILS_TOO_LONG,
                "media issue details exceed the 4096-character limit",
            ));
        }
        let (file, title, library, episode_ids) = self
            .resolve_media_issue_target(actor, &input.media_file_id, None)
            .await?;
        let (file_version, proof) = self.inspect_media_issue_file(&file).await?;
        let draft = self
            .media_issue_draft(actor, &title, &file, &episode_ids, &input)
            .await;
        let issue = self
            .services
            .workflow
            .blocklist_repo
            .submit_media_issue(&NewMediaIssue {
                library_id: title.library_id,
                title_id: title.id,
                media_file_id: file.id,
                file_path_snapshot: file.file_path,
                file_version,
                proof,
                scheme: crate::fs_integrity::IMPORT_CONTENT_PROOF_SCHEME.to_string(),
                reporter_user_id: actor.id.clone(),
                reason_code: input.reason_code,
                details: input.details,
                remediation: input.remediation,
                affected_episode_ids: episode_ids,
                affected_series_movie_link_ids: file.series_movie_link_ids,
            })
            .await?;
        let submitted_event = crate::domain_events::new_global_domain_event(
            actor,
            scryer_domain::DomainEventPayload::MediaIssueSubmitted(
                scryer_domain::MediaIssueEventData {
                    issue_id: issue.id.clone(),
                    library_id: issue.library_id.clone(),
                    title_id: issue.title_id.clone(),
                    media_file_id: issue.media_file_id.clone(),
                    remediation: issue.remediation.as_str().to_string(),
                    status: issue.status.as_str().to_string(),
                    execution_status: issue.execution_status.as_str().to_string(),
                },
            ),
        );
        let _ = self.append_domain_event(submitted_event).await;
        if issue.status != MediaIssueStatus::Pending {
            return Ok(issue);
        }
        if issue.rule_decision_id.is_some() {
            return self.apply_media_issue_policy(actor, issue).await;
        }
        let snapshot = crate::media_requests::snapshot::MediaRequestMetadataSnapshot::unavailable(
            "media_issue_not_acquisition_metadata",
        );
        self.evaluate_request_draft(
            actor,
            &library,
            &draft,
            &snapshot,
            crate::request_rules::RequestEvaluationPurpose::Submit {
                request_id: issue.id.clone(),
            },
        )
        .await?;
        self.apply_media_issue_policy(actor, self.load_media_issue(&issue.id).await?)
            .await
    }

    async fn apply_media_issue_policy(
        &self,
        actor: &User,
        issue: MediaIssue,
    ) -> AppResult<MediaIssue> {
        // A missing trace is not authority to transition a report.
        let Some(decision_id) = issue
            .rule_decision_id
            .as_deref()
            .filter(|_| issue.status == MediaIssueStatus::Pending)
        else {
            return Ok(issue);
        };
        use scryer_domain::RequestDecisionOutcome;
        match issue
            .rule_effective_outcome
            .as_deref()
            .and_then(RequestDecisionOutcome::parse_storage)
        {
            Some(RequestDecisionOutcome::Deny) => {
                let rejected = self
                    .services
                    .workflow
                    .blocklist_repo
                    .reject_media_issue(
                        &issue.id,
                        None,
                        Some(decision_id),
                    )
                    .await?
                    .ok_or_else(|| {
                        AppError::Repository("rejected media issue could not be reloaded".into())
                    })?;
                if rejected.status == MediaIssueStatus::Rejected {
                    let event = crate::domain_events::new_global_domain_event(
                        crate::domain_events::DomainEventActor::system(),
                        scryer_domain::DomainEventPayload::MediaIssueRejected(
                            scryer_domain::MediaIssueEventData {
                                issue_id: rejected.id.clone(),
                                library_id: rejected.library_id.clone(),
                                title_id: rejected.title_id.clone(),
                                media_file_id: rejected.media_file_id.clone(),
                                remediation: rejected.remediation.as_str().to_string(),
                                status: rejected.status.as_str().to_string(),
                                execution_status: rejected.execution_status.as_str().to_string(),
                            },
                        ),
                    );
                    let _ = self.append_domain_event(event).await;
                }
                return Ok(rejected);
            }
            Some(RequestDecisionOutcome::AutoApprove)
                if issue.remediation == MediaIssueRemediation::MarkBad =>
            {
                let file = self.revalidate_media_issue(&issue).await?;
                // Do not replay a stale automatic decision if the indexed
                // footprint changed after the fact snapshot was written.
                if self
                    .services
                    .workflow
                    .blocklist_repo
                    .media_issue_has_other_size_candidates(
                        &issue.library_id,
                        &file.id,
                        file.size_bytes,
                    )
                    .await?
                    != Some(false)
                {
                    return Ok(issue);
                }
                let confirmed = self
                    .confirm_media_issue(
                        actor,
                        &issue,
                        file,
                        MediaIssueRemediation::MarkBad,
                        Some(decision_id),
                        None,
                    )
                    .await?;
                let approved = self.load_media_issue(&issue.id).await?;
                if confirmed && approved.status == MediaIssueStatus::Approved {
                    let event = crate::domain_events::new_global_domain_event(
                        crate::domain_events::DomainEventActor::system(),
                        scryer_domain::DomainEventPayload::MediaIssueApproved(
                            scryer_domain::MediaIssueEventData {
                                issue_id: approved.id.clone(),
                                library_id: approved.library_id.clone(),
                                title_id: approved.title_id.clone(),
                                media_file_id: approved.media_file_id.clone(),
                                remediation: approved.remediation.as_str().to_string(),
                                status: approved.status.as_str().to_string(),
                                execution_status: approved.execution_status.as_str().to_string(),
                            },
                        ),
                    );
                    let _ = self.append_domain_event(event).await;
                }
                Ok(approved)
            }
            _ => Ok(issue),
        }
    }

    pub async fn list_library_content_blocks(
        &self,
        actor: &scryer_domain::User,
        library_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::LibraryContentBlock>> {
        self.require_privileged_library_access(actor, library_id)
            .await?;
        self.services
            .workflow
            .blocklist_repo
            .list_content_blocks(library_id, limit)
            .await
    }

    pub async fn clear_library_content_block(
        &self,
        actor: &scryer_domain::User,
        block_id: &str,
    ) -> AppResult<()> {
        let block = self
            .services
            .workflow
            .blocklist_repo
            .get_content_block(block_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::media::CONTENT_BLOCK_NOT_FOUND,
                    "library content block not found",
                )
            })?;
        self.require_library_permission(actor, &block.library_id, LibraryPermission::ManageTitles)
            .await?;
        let removed = self
            .services
            .workflow
            .blocklist_repo
            .remove_content_block(block_id)
            .await;
        removed
    }

    pub async fn list_media_issues(
        &self,
        actor: &scryer_domain::User,
        library_id: &str,
        status: Option<MediaIssueStatus>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<MediaIssue>> {
        self.require_privileged_library_access(actor, library_id)
            .await?;
        self.services
            .workflow
            .blocklist_repo
            .list_media_issues(library_id, status, limit, offset)
            .await
    }

    pub async fn approve_media_issue(
        &self,
        actor: &scryer_domain::User,
        issue_id: &str,
    ) -> AppResult<MediaIssue> {
        let issue = self.load_media_issue(issue_id).await?;
        self.require_library_permission(actor, &issue.library_id, LibraryPermission::ManageTitles)
            .await?;
        if issue.status != MediaIssueStatus::Pending {
            return Ok(issue);
        }

        let _location_guard = self
            .acquire_location_title_mutation(
                &crate::location::ownership_guard::MEDIA_FILE_PRIMARY_ENTRY,
                &issue.title_id,
            )
            .await?;
        let proof = self.revalidate_media_issue(&issue).await?;
        let confirmed = self
            .confirm_media_issue(
                actor,
                &issue,
                proof,
                MediaIssueRemediation::MarkBad,
                None,
                None,
            )
            .await?;
        let approved = self.load_media_issue(issue_id).await?;
        if !confirmed || approved.status != MediaIssueStatus::Approved {
            return Ok(approved);
        }
        let event = crate::domain_events::new_global_domain_event(
            actor,
            scryer_domain::DomainEventPayload::MediaIssueApproved(
                scryer_domain::MediaIssueEventData {
                    issue_id: approved.id.clone(),
                    library_id: approved.library_id.clone(),
                    title_id: approved.title_id.clone(),
                    media_file_id: approved.media_file_id.clone(),
                    remediation: approved.remediation.as_str().to_string(),
                    status: approved.status.as_str().to_string(),
                    execution_status: approved.execution_status.as_str().to_string(),
                },
            ),
        );
        let _ = self.append_domain_event(event).await;
        Ok(approved)
    }

    async fn load_media_issue(&self, issue_id: &str) -> AppResult<MediaIssue> {
        self.services
            .workflow
            .blocklist_repo
            .get_media_issue(issue_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::media_requests::RESOURCE_NOT_FOUND,
                    "media issue not found",
                )
            })
    }

    pub(crate) async fn inspect_media_issue_file(
        &self,
        file: &crate::TitleMediaFile,
    ) -> AppResult<(String, scryer_domain::ImportContentProof)> {
        let path = crate::stored_paths::stored_path_to_path_buf(&file.file_path);
        let changed = || {
            AppError::validation_refused(
                crate::error_reasons::media::SOURCE_CHANGED_DURING_INSPECTION,
                "could not verify the selected media file identity",
            )
        };
        let (before, metadata) = crate::location::resolution::file_version_and_metadata(&path)
            .await
            .map_err(|_| changed())?;
        if !metadata.is_file() || i64::try_from(metadata.len()).ok() != Some(file.size_bytes) {
            return Err(changed());
        }
        let proof = crate::fs_integrity::import_content_proof_async(&path)
            .await
            .map_err(|_| changed())?;
        let after = crate::location::resolution::file_version(&path)
            .await
            .map_err(|_| changed())?;
        if before != after || proof.size_bytes != metadata.len() {
            return Err(changed());
        }
        Ok((before, proof))
    }

    async fn revalidate_media_issue(&self, issue: &MediaIssue) -> AppResult<crate::TitleMediaFile> {
        let file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(&issue.media_file_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::media::FILE_NOT_FOUND,
                    "reported media file no longer exists",
                )
            })?;
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(&file.title_id)
            .await?;
        let mut affected = self
            .services
            .library
            .media_files
            .score_evaluation_episode_ids(&file)
            .await?;
        affected.sort_unstable();
        affected.dedup();
        if file.title_id != issue.title_id
            || library_id.as_deref() != Some(issue.library_id.as_str())
            || affected != issue.affected_episode_ids
            || file.series_movie_link_ids != issue.affected_series_movie_link_ids
            || file.file_path != issue.file_path_snapshot
            || i64::try_from(issue.size_bytes).unwrap_or(i64::MAX) != file.size_bytes
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::media::REGISTERED_MEDIA_PATH_CHANGED_BEFORE_INSPECTION,
                "the reported media file changed before approval",
            ));
        }
        let (version, proof) = self.inspect_media_issue_file(&file).await?;
        if version != issue.file_version
            || proof.size_bytes != issue.size_bytes
            || proof.sample_bytes != issue.sample_bytes
            || proof.sample_blake3 != issue.sample_blake3
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::media::SOURCE_CHANGED_DURING_INSPECTION,
                "the reported media file identity changed before approval",
            ));
        }

        Ok(file)
    }

    async fn confirm_media_issue(
        &self,
        actor: &scryer_domain::User,
        issue: &MediaIssue,
        file: crate::TitleMediaFile,
        remediation: MediaIssueRemediation,
        rule_decision_id: Option<&str>,
        recycle_fingerprint: Option<&str>,
    ) -> AppResult<bool> {
        let release = file
            .release_listing_json
            .as_deref()
            .and_then(crate::release_listing_view)
            .and_then(|_| file.grabbed_release_title.as_ref())
            .filter(|name| !name.trim().is_empty())
            .map(|name| crate::NewBlocklistEntry {
                title_id: issue.title_id.clone(),
                release_name: name.clone(),
                indexer_id: file.indexer_source.clone().unwrap_or_default(),
                info_hash: file.release_hash.clone(),
                reason: Some(crate::error_reasons::media::CONTENT_BLOCK_MATCHED.to_owned()),
            });
        let confirmed = self
            .services
            .workflow
            .blocklist_repo
            .confirm_media_issue(
                &issue.id,
                if rule_decision_id.is_some() {
                    None
                } else {
                    Some(actor.id.as_str())
                },
                rule_decision_id,
                remediation,
                &NewLibraryContentBlock {
                    library_id: issue.library_id.clone(),
                    proof: scryer_domain::ImportContentProof {
                        size_bytes: issue.size_bytes,
                        sample_bytes: issue.sample_bytes,
                        sample_blake3: issue.sample_blake3.clone(),
                    },
                    scheme: issue.scheme.clone(),
                    reason: Some(issue.reason_code.clone()),
                    approving_actor_id: rule_decision_id.is_none().then(|| actor.id.clone()),
                    source_report_id: Some(issue.id.clone()),
                    source_media_file_id: Some(issue.media_file_id.clone()),
                    provenance_json: serde_json::json!({
                        "kind": "media_issue",
                        "remediation": remediation.as_str(),
                        "source": if release.is_some() { "grabbed" } else { "unknown" }
                    })
                    .to_string()
                    .into(),
                },
                release.as_ref(),
                recycle_fingerprint,
                &self
                        .matching_media_issue_file_ids(issue)
                        .await?,
            )
            .await?;
        if confirmed {
            self.runtime
                .acquisition
                .target_cache
                .mark_title(&issue.title_id);
        }
        Ok(confirmed)
    }

    async fn matching_media_issue_file_ids(
        &self,
        issue: &MediaIssue,
    ) -> AppResult<Vec<String>> {
        let mut matched = Vec::new();
        let mut after_id: Option<String> = None;
        loop {
            let candidates = self
                .services
                .workflow
                .blocklist_repo
                .list_media_issue_matching_candidates(
                    &issue.library_id,
                    i64::try_from(issue.size_bytes).unwrap_or(i64::MAX),
                    after_id.as_deref(),
                    500,
                )
                .await?;
            if candidates.is_empty() {
                break;
            }
            for candidate in &candidates {
                let path = crate::stored_paths::stored_path_to_path_buf(&candidate.file_path);
                let Ok(proof) = crate::fs_integrity::import_content_proof_async(&path).await else {
                    continue;
                };
                if proof.size_bytes == issue.size_bytes
                    && proof.sample_bytes == issue.sample_bytes
                    && proof.sample_blake3 == issue.sample_blake3
                {
                    matched.push(candidate.id.clone());
                }
            }
            after_id = candidates.last().map(|candidate| candidate.id.clone());
            if candidates.len() < 500 {
                break;
            }
        }
        if !matched.iter().any(|id| id == &issue.media_file_id) {
            return Err(AppError::validation_refused(
                crate::error_reasons::media::SOURCE_CHANGED_DURING_INSPECTION,
                "the reported media file no longer matches the confirmed content proof",
            ));
        }
        Ok(matched)
    }

    pub async fn approve_media_issue_recycle_search(
        &self,
        actor: &scryer_domain::User,
        issue_id: &str,
        preview_fingerprint: &str,
        typed_confirmation: Option<&str>,
    ) -> AppResult<MediaIssue> {
        let issue = self.load_media_issue(issue_id).await?;
        self.require_library_permission(actor, &issue.library_id, LibraryPermission::ManageTitles)
            .await?;
        if issue.remediation != MediaIssueRemediation::RecycleAndSearch {
            return Err(AppError::validation_refused(
                crate::error_reasons::media::REMEDIATION_NOT_AVAILABLE,
                "this media issue did not request recycle-and-search",
            ));
        }
        if issue.status == MediaIssueStatus::Approved
            && issue.execution_status == scryer_domain::MediaIssueExecutionStatus::RecycleFailed
        {
            Self::require_media_issue_recycle_confirmation(typed_confirmation)?;
            let preview = self
                .preview_delete_media_file(actor, &issue.media_file_id)
                .await?;
            if preview_fingerprint != preview.fingerprint {
                return Err(AppError::validation_refused(
                    crate::delete_refusal::PREVIEW_STALE,
                    "the recycle preview changed before retry",
                ));
            }
            self.validate_delete_media_file(
                &issue.media_file_id,
                preview_fingerprint,
                typed_confirmation,
            )
            .await?;
            self.revalidate_media_issue(&issue).await?;
            let changed = self
                .services
                .workflow
                .blocklist_repo
                .set_media_issue_execution_status(
                    &issue.id,
                    scryer_domain::MediaIssueExecutionStatus::Recycling,
                    None,
                )
                .await?;
            if !changed {
                return self.load_media_issue(issue_id).await;
            }
            let confirmation = crate::DeleteExecutionConfirmation {
                preview_fingerprint: preview_fingerprint.to_string(),
                typed_confirmation: typed_confirmation.map(str::to_string),
            };
            if let Err(error) = self
                .start_media_file_removal_job(
                    actor,
                    &issue.media_file_id,
                    true,
                    Some(confirmation),
                    Some(issue.clone()),
                )
                .await
            {
                self.record_media_issue_recycle_failure(issue_id, &error)
                    .await?;
            }
            return self.load_media_issue(issue_id).await;
        }
        if issue.status != MediaIssueStatus::Pending {
            return Ok(issue);
        }
        Self::require_media_issue_recycle_confirmation(typed_confirmation)?;
        let preview = self
            .preview_delete_media_file(actor, &issue.media_file_id)
            .await?;
        if preview_fingerprint != preview.fingerprint {
            return Err(AppError::validation_refused(
                crate::delete_refusal::PREVIEW_STALE,
                "the recycle preview changed before confirmation",
            ));
        }
        let confirmation = crate::DeleteExecutionConfirmation {
            preview_fingerprint: preview_fingerprint.to_string(),
            typed_confirmation: typed_confirmation.map(str::to_string),
        };

        self.validate_delete_media_file(
            &issue.media_file_id,
            preview_fingerprint,
            typed_confirmation,
        )
        .await?;
        let proof = self.revalidate_media_issue(&issue).await?;
        if !self
            .confirm_media_issue(
                actor,
                &issue,
                proof,
                MediaIssueRemediation::RecycleAndSearch,
                None,
                Some(preview_fingerprint),
            )
            .await?
        {
            return self.load_media_issue(issue_id).await;
        }

        if let Err(error) = self
            .start_media_file_removal_job(
                actor,
                &issue.media_file_id,
                true,
                Some(confirmation),
                Some(issue.clone()),
            )
            .await
        {
            self.record_media_issue_recycle_failure(issue_id, &error)
                .await?;
        }

        self.load_media_issue(issue_id).await
    }

    pub async fn resume_interrupted_media_issue_recycles(&self) -> AppResult<()> {
        // This startup path runs before accepting API requests. Resume only
        // durable operator confirmations, never policy marks or failed jobs.
        let mut started = std::collections::HashSet::new();
        for _ in 0..10 {
            let intents = self
                .services
                .workflow
                .blocklist_repo
                .interrupted_media_issue_recycles(100)
                .await?;
            if intents.is_empty() {
                break;
            }
            for intent in intents {
                if !started.insert(intent.issue_id.clone()) {
                    continue;
                }
                let issue = self.load_media_issue(&intent.issue_id).await?;
                let actor = self
                    .services
                    .identity
                    .users
                    .get_by_id(&intent.actor_id)
                    .await?
                    .filter(|actor| actor.login_status().is_enabled());
                let result = if let Some(actor) = actor {
                    let confirmation = crate::DeleteExecutionConfirmation {
                        preview_fingerprint: intent.preview_fingerprint,
                        typed_confirmation: Some(
                            crate::library::user_delete::DELETE_TYPED_CONFIRMATION_VALUE.to_owned(),
                        ),
                    };
                    self.start_media_file_removal_job(
                        &actor,
                        &issue.media_file_id,
                        true,
                        Some(confirmation),
                        Some(issue.clone()),
                    )
                    .await
                    .map(|_| ())
                } else {
                    Err(AppError::validation_refused(
                        crate::error_reasons::media::REMEDIATION_NOT_AVAILABLE,
                        "the original media issue reviewer is no longer authorized",
                    ))
                };
                if let Err(error) = result {
                    self.record_media_issue_recycle_failure(&intent.issue_id, &error)
                        .await?;
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn run_media_issue_recycle(
        &self,
        actor: &scryer_domain::User,
        issue: &MediaIssue,
        confirmation: Option<&crate::DeleteExecutionConfirmation>,
    ) -> AppResult<()> {
        let result = self
            .execute_media_issue_recycle(actor, &issue.id, confirmation)
            .await;
        if let Err(error) = &result {
            self.record_media_issue_recycle_failure(&issue.id, error)
                .await?;
        }
        result
    }

    async fn record_media_issue_recycle_failure(
        &self,
        issue_id: &str,
        error: &AppError,
    ) -> AppResult<()> {
        self.services
            .workflow
            .blocklist_repo
            .set_media_issue_execution_status(
                issue_id,
                scryer_domain::MediaIssueExecutionStatus::RecycleFailed,
                Some(
                    error
                        .validation_reason()
                        .unwrap_or(crate::error_reasons::media::RECYCLE_FAILED),
                ),
            )
            .await?;
        let issue = self.load_media_issue(issue_id).await?;
        let event = crate::domain_events::new_global_domain_event(
            crate::domain_events::DomainEventActor::system(),
            scryer_domain::DomainEventPayload::MediaIssueRecycleExecution(
                scryer_domain::MediaIssueEventData {
                    issue_id: issue.id.clone(),
                    library_id: issue.library_id.clone(),
                    title_id: issue.title_id.clone(),
                    media_file_id: issue.media_file_id.clone(),
                    remediation: issue.remediation.as_str().to_string(),
                    status: issue.status.as_str().to_string(),
                    execution_status: issue.execution_status.as_str().to_string(),
                },
            ),
        );
        let _ = self.append_domain_event(event).await;
        Ok(())
    }

    async fn execute_media_issue_recycle(
        &self,
        actor: &scryer_domain::User,
        issue_id: &str,
        confirmation: Option<&crate::DeleteExecutionConfirmation>,
    ) -> AppResult<()> {
        let issue = self.load_media_issue(issue_id).await?;
        let unavailable = || {
            AppError::validation_refused(
                crate::error_reasons::media::REMEDIATION_NOT_AVAILABLE,
                "recycle authorization is unavailable",
            )
        };
        if issue.status != MediaIssueStatus::Approved
            || issue.authorized_remediation != Some(MediaIssueRemediation::RecycleAndSearch)
            || issue.execution_status != scryer_domain::MediaIssueExecutionStatus::Recycling
        {
            return Err(unavailable());
        }
        let confirmation = confirmation.ok_or_else(unavailable)?;
        Self::require_media_issue_recycle_confirmation(confirmation.typed_confirmation.as_deref())?;
        let actor = self
            .services
            .identity
            .users
            .get_by_id(&actor.id)
            .await?
            .filter(|actor| actor.login_status().is_enabled())
            .ok_or_else(unavailable)?;
        self.require_library_permission(&actor, &issue.library_id, LibraryPermission::ManageTitles)
            .await?;
        let _location_guard = self
            .acquire_location_title_mutation(
                &crate::location::ownership_guard::MEDIA_FILE_DELETE_ENTRY,
                &issue.title_id,
            )
            .await?;
        let already_moved = if let Some(receipt) = self
            .services
            .workflow
            .blocklist_repo
            .get_media_issue_recycle_receipt(issue_id)
            .await?
        {
            self.finish_recovered_media_issue_recycle(&issue, &receipt)
                .await?
        } else {
            false
        };
        if !already_moved {
            let file = self.revalidate_media_issue(&issue).await?;
            self.recycle_reviewed_media_file(&issue, &file, confirmation)
                .await?;
        }
        if let Some(file) = self
            .services
            .library
            .media_files
            .get_media_file_by_id(&issue.media_file_id)
            .await?
        {
            if file.title_id != issue.title_id || file.file_path != issue.file_path_snapshot {
                return Err(AppError::validation_refused(
                    crate::delete_refusal::PREVIEW_STALE,
                    "the catalog now points to a different file; cleanup was not run",
                ));
            }
            let mut promotions = crate::catalog::workflow::PrimaryPromotionBatch::default();
            self.record_primary_promotion_candidate(
                &mut promotions,
                &issue.title_id,
                &issue.media_file_id,
            )
            .await;
            self.delete_media_file_record_with_dependents(&file.id)
                .await?;
            self.finish_primary_promotion_batch(
                &crate::domain_events::DomainEventActor::from(&actor),
                promotions,
            )
            .await;
        }
        let outcome = self.queue_media_issue_replacement(&issue).await?;
        self.services
            .workflow
            .blocklist_repo
            .set_media_issue_execution_status(issue_id, outcome, None)
            .await?;
        Ok(())
    }

    fn require_media_issue_recycle_confirmation(typed: Option<&str>) -> AppResult<()> {
        if typed.map(str::trim)
            != Some(crate::library::user_delete::DELETE_TYPED_CONFIRMATION_VALUE)
        {
            return Err(AppError::validation_refused(
                crate::delete_refusal::TYPED_CONFIRMATION_REQUIRED,
                "type DELETE to confirm recycling the reviewed media file",
            ));
        }
        Ok(())
    }

    async fn queue_media_issue_replacement(
        &self,
        issue: &MediaIssue,
    ) -> AppResult<scryer_domain::MediaIssueExecutionStatus> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&issue.title_id)
            .await?;
        let Some(title) =
            title.filter(|title| title.monitored && title.library_id == issue.library_id)
        else {
            return Ok(scryer_domain::MediaIssueExecutionStatus::NoEligibleRelease);
        };
        let scopes = if title.facet == MediaFacet::Movie {
            let movie_scopes = self
                .services
                .workflow
                .acquisition_scope_states
                .list_acquisition_scope_states(crate::AcquisitionScopeStatesQuery {
                    title_id: Some(issue.title_id.clone()),
                    limit: 500,
                    ..Default::default()
                })
                .await?;
            if let Some(movie_scope) = movie_scopes.iter().find(|scope| {
                scope.title_id == issue.title_id
                    && scope.episode_id.is_none()
                    && scope.series_movie_link_id.is_none()
            }) {
                if movie_scope.status == crate::AcquisitionScopeStatus::Grabbed {
                    return Ok(scryer_domain::MediaIssueExecutionStatus::DownloadQueued);
                }
                if matches!(
                    movie_scope.status,
                    crate::AcquisitionScopeStatus::Wanted
                        | crate::AcquisitionScopeStatus::Completed
                ) {
                    self.reopen_wanted_scope_for_acquisition(
                        movie_scope,
                        crate::acquisition::convergence::CoverageReopen::All,
                    )
                    .await;
                    return Ok(scryer_domain::MediaIssueExecutionStatus::Searching);
                }
            }
            Vec::new()
        } else {
            self.services
                .catalog
                .shows
                .get_episodes_by_ids(&issue.affected_episode_ids)
                .await?
                .into_iter()
                .filter(|episode| episode.monitored && episode.title_id == title.id)
                .map(|episode| Some(episode.id))
                .collect()
        };
        if scopes.is_empty() {
            return Ok(scryer_domain::MediaIssueExecutionStatus::NoEligibleRelease);
        }
        // Existing convergence rows carry the scheduler's download-target
        // configuration. Never create a target or resume a paused scope here.
        let scope_states = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(crate::AcquisitionScopeStatesQuery {
                title_id: Some(issue.title_id.clone()),
                limit: 500,
                ..Default::default()
            })
            .await?;
        let mut episode_scopes = std::collections::HashMap::new();
        let mut series_movie_scopes = std::collections::HashMap::new();
        for scope in scope_states {
            if let Some(episode_id) = scope.episode_id.as_deref() {
                episode_scopes.insert(episode_id.to_string(), scope.clone());
            }
            if let Some(series_movie_link_id) = scope.series_movie_link_id.as_deref() {
                series_movie_scopes.insert(series_movie_link_id.to_string(), scope);
            }
        }
        let outcome = scryer_domain::MediaIssueExecutionStatus::NoEligibleRelease;
        let mut saw_searching = false;
        let mut saw_queued = false;
        for episode_id in scopes {
            if let Some(scope) = episode_id
                .as_deref()
                .and_then(|episode_id| episode_scopes.get(episode_id))
            {
                if scope.status == crate::AcquisitionScopeStatus::Grabbed {
                    saw_queued = true;
                } else if matches!(
                    scope.status,
                    crate::AcquisitionScopeStatus::Wanted
                        | crate::AcquisitionScopeStatus::Completed
                ) {
                    self.reopen_wanted_scope_for_acquisition(
                        scope,
                        crate::acquisition::convergence::CoverageReopen::All,
                    )
                    .await;
                    saw_searching = true;
                }
            }
        }
        for series_movie_link_id in &issue.affected_series_movie_link_ids {
            let Some(scope) = series_movie_scopes.get(series_movie_link_id) else {
                continue;
            };
            if scope.status == crate::AcquisitionScopeStatus::Grabbed {
                saw_queued = true;
            } else if matches!(
                scope.status,
                crate::AcquisitionScopeStatus::Wanted | crate::AcquisitionScopeStatus::Completed
            ) {
                self.reopen_wanted_scope_for_acquisition(
                    scope,
                    crate::acquisition::convergence::CoverageReopen::All,
                )
                .await;
                saw_searching = true;
            }
        }
        if saw_queued {
            Ok(scryer_domain::MediaIssueExecutionStatus::DownloadQueued)
        } else if saw_searching {
            Ok(scryer_domain::MediaIssueExecutionStatus::Searching)
        } else {
            Ok(outcome)
        }
    }

    pub async fn reject_media_issue(
        &self,
        actor: &scryer_domain::User,
        issue_id: &str,
    ) -> AppResult<MediaIssue> {
        let issue = self
            .services
            .workflow
            .blocklist_repo
            .get_media_issue(issue_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::media_requests::RESOURCE_NOT_FOUND,
                    "media issue not found",
                )
            })?;
        self.require_library_permission(actor, &issue.library_id, LibraryPermission::ManageTitles)
            .await?;
        if issue.status != MediaIssueStatus::Pending {
            return Ok(issue);
        }
        let rejected = self
            .services
            .workflow
            .blocklist_repo
            .reject_media_issue(&issue.id, Some(actor.id.as_str()), None)
            .await?
            .ok_or_else(|| {
                AppError::Repository("rejected media issue could not be reloaded".into())
            })?;
        let event = crate::domain_events::new_global_domain_event(
            actor,
            scryer_domain::DomainEventPayload::MediaIssueRejected(
                scryer_domain::MediaIssueEventData {
                    issue_id: rejected.id.clone(),
                    library_id: rejected.library_id.clone(),
                    title_id: rejected.title_id.clone(),
                    media_file_id: rejected.media_file_id.clone(),
                    remediation: rejected.remediation.as_str().to_string(),
                    status: rejected.status.as_str().to_string(),
                    execution_status: rejected.execution_status.as_str().to_string(),
                },
            ),
        );
        let _ = self.append_domain_event(event).await;
        Ok(rejected)
    }
}

fn media_issue_request_covers_episode_scopes(
    affected_seasons: &[i32],
    selection: Option<&scryer_domain::MonitorSelection>,
) -> bool {
    let Some(selection) = selection else {
        return false;
    };
    affected_seasons
        .iter()
        .all(|season| selection.monitors_season(*season))
}

#[cfg(test)]
mod tests {
    use super::media_issue_request_covers_episode_scopes;
    use scryer_domain::MonitorSelection;

    #[test]
    fn approved_request_must_cover_every_affected_episode_season() {
        let selection = MonitorSelection {
            seasons: vec![1, 2],
            series_movies: Vec::new(),
        };
        assert!(media_issue_request_covers_episode_scopes(
            &[1, 2],
            Some(&selection),
        ));
        assert!(!media_issue_request_covers_episode_scopes(
            &[1, 3],
            Some(&selection),
        ));
    }
}
