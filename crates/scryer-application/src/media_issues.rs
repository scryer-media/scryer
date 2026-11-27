//! Bad-file reports: a reporter says a library file is bad, request rules or
//! an operator decide, and a confirmed report marks the file bad on the
//! title's blocklist. Reports are kept as audit records.

use scryer_domain::{
    BlocklistContentIdentity, Library, LibraryPermission, MediaFacet, MediaIssue,
    MediaIssueEventData, MediaIssueExecutionStatus, MediaIssueReason, MediaIssueRemediation,
    MediaIssueStatus, MediaIssueUnmarkRoute, MediaRequest, MediaRequestStatus, Title, User,
};
use scryer_rules::request::Observation;

use crate::{AppError, AppResult, AppUseCase, NewMediaIssue, TitleMediaFile, UnmarkedMediaIssue};

#[derive(Clone, Debug)]
pub struct SubmitMediaIssueInput {
    pub media_file_id: String,
    pub reason: MediaIssueReason,
    pub details: Option<String>,
    pub remediation: MediaIssueRemediation,
}

pub const MAX_MEDIA_ISSUE_DETAILS_LENGTH: usize = 4096;

/// The refusal fact naming the title blocklist entry a restored file matches.
pub const BAD_FILE_ENTRY_ID_FACT: &str = "blocklistEntryId";

/// The recycle-bin reason of a download copy recycled with a reported file.
pub(crate) const MEDIA_ISSUE_DOWNLOAD_COPY_RECYCLE_REASON: &str = "media_issue_download_copy";

/// Joins the reported file's preview fingerprint and the version of the
/// download copy the same confirmation also recycles.
const DOWNLOAD_COPY_FINGERPRINT_SEPARATOR: &str = "|download:";

/// Joins the reported file's preview fingerprint and the version of the
/// download copy that is the reported file's other hard-linked name.
const LINKED_COPY_FINGERPRINT_SEPARATOR: &str = "|linked:";

/// A copy of a reported file's content left in the download folder it was
/// imported from.
#[derive(Clone, Debug)]
pub(crate) struct MediaIssueDownloadCopy {
    pub(crate) path: std::path::PathBuf,
    /// The download folder the import recorded; the copy lies inside it.
    pub(crate) download_dir: std::path::PathBuf,
    pub(crate) version: String,
    /// Whether the copy is the other name of the file it was checked
    /// against, rather than a separate file.
    pub(crate) linked: bool,
}

/// The download copy a confirmed recycle fingerprint also names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmedDownloadCopy<'a> {
    pub(crate) version: &'a str,
    /// The copy is the reported file's other hard-linked name.
    pub(crate) linked: bool,
}

impl ConfirmedDownloadCopy<'_> {
    /// Whether `copy`, found now, is still the copy this confirmation named.
    /// A linked copy's change time moves when its other name is recycled,
    /// so it is compared by device, file number, length and modification
    /// time.
    pub(crate) fn still_names(&self, copy: &MediaIssueDownloadCopy) -> bool {
        if self.linked {
            copy.linked
                && crate::disk::StoredVersion::new(&copy.version)
                    .same_content(crate::disk::StoredVersion::new(self.version))
                    == crate::disk::Sameness::Same
        } else {
            !copy.linked && copy.version == self.version
        }
    }
}

/// The reported file's part of a confirmed recycle fingerprint, and the
/// download copy it also confirmed, if any.
pub(crate) fn split_media_issue_recycle_fingerprint(
    fingerprint: &str,
) -> (&str, Option<ConfirmedDownloadCopy<'_>>) {
    if let Some((file, version)) = fingerprint.split_once(LINKED_COPY_FINGERPRINT_SEPARATOR) {
        return (
            file,
            Some(ConfirmedDownloadCopy {
                version,
                linked: true,
            }),
        );
    }
    match fingerprint.split_once(DOWNLOAD_COPY_FINGERPRINT_SEPARATOR) {
        Some((file, version)) => (
            file,
            Some(ConfirmedDownloadCopy {
                version,
                linked: false,
            }),
        ),
        None => (fingerprint, None),
    }
}

/// Refuse a confirmation whose choice about a hard-linked download copy does
/// not match its preview: a linked pair is recycled only when the operator
/// chose to recycle both names, and that choice without a linked pair in the
/// preview is stale.
pub(crate) fn require_linked_copy_choice(
    confirmed: Option<ConfirmedDownloadCopy<'_>>,
    recycle_both_names: bool,
) -> AppResult<()> {
    match (
        confirmed.is_some_and(|copy| copy.linked),
        recycle_both_names,
    ) {
        (true, false) => Err(AppError::validation_refused(
            crate::error_reasons::media::RECYCLE_HARDLINKED_DOWNLOAD_COPY,
            "the reported file is hard-linked to its download copy; recycle both names or cancel",
        )),
        (false, true) => Err(AppError::validation_refused(
            crate::delete_refusal::PREVIEW_STALE,
            "the recycle preview no longer shows a hard-linked download copy",
        )),
        _ => Ok(()),
    }
}

/// What an operator confirms before a reported file is recycled.
#[derive(Clone, Debug)]
pub struct MediaIssueRecyclePreview {
    pub preview: crate::DeletePreview,
    /// [`crate::error_reasons::media::RECYCLE_HARDLINKED_DOWNLOAD_COPY`] when
    /// the reported file's only other name is its download copy, so the
    /// recycle moves both names or nothing.
    pub hard_link_reason: Option<&'static str>,
}

/// The recorded source path inside its download folder. A relative path that
/// is absolute or climbs out of the folder names nothing.
fn download_copy_path(
    download_dir: &std::path::Path,
    relative_path: &str,
) -> Option<std::path::PathBuf> {
    let relative = crate::stored_paths::stored_path_to_path_buf(relative_path);
    let normal = relative
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)));
    (normal && relative.components().next().is_some() && download_dir.is_absolute())
        .then(|| download_dir.join(relative))
}

/// The most of the reporter's requests for one title the relationship fact
/// reads. A title is created by a handful of requests at most.
const REPORTER_REQUEST_LIMIT: usize = 100;

/// Refuse a report whose details are too long, or an `Other` report without
/// details.
pub fn validate_media_issue_details(
    reason: MediaIssueReason,
    details: Option<&str>,
) -> AppResult<()> {
    let details = details.map(str::trim).filter(|details| !details.is_empty());
    if details.is_some_and(|details| details.chars().count() > MAX_MEDIA_ISSUE_DETAILS_LENGTH) {
        return Err(AppError::validation_refused(
            crate::error_reasons::media::DETAILS_TOO_LONG,
            "media issue details exceed the 4096-character limit",
        ));
    }
    if reason == MediaIssueReason::Other && details.is_none() {
        return Err(AppError::validation_refused(
            crate::error_reasons::media::REASON_REQUIRES_DETAILS,
            "describe the problem when the reason is other",
        ));
    }
    Ok(())
}

/// Parse a report reason sent as text, such as a rule test sample.
pub fn parse_media_issue_reason(value: &str) -> AppResult<MediaIssueReason> {
    MediaIssueReason::parse(value.trim()).ok_or_else(|| {
        AppError::validation_refused(
            crate::error_reasons::media::REASON_UNKNOWN,
            format!("unknown media issue reason {value:?}"),
        )
    })
}

pub(crate) fn media_issue_event_data(issue: &MediaIssue) -> MediaIssueEventData {
    MediaIssueEventData {
        issue_id: issue.id.clone(),
        library_id: issue.library_id.clone(),
        title_id: issue.title_id.clone(),
        media_file_id: issue.media_file_id.clone(),
        remediation: issue.remediation.as_str().to_string(),
        status: issue.status.as_str().to_string(),
        execution_status: issue.execution_status.as_str().to_string(),
    }
}

/// Where the reported file came from, as the report rule section shows it.
pub(crate) fn media_issue_provenance(file: &TitleMediaFile) -> &'static str {
    if release_block_for_file(file, "").is_some() {
        "grabbed"
    } else {
        // Old imports, adopted downloads and scanned rows do not all record
        // their origin reliably.
        "unknown"
    }
}

/// The grab the file came from, as a blocklist entry, when it is known.
fn release_block_for_file(file: &TitleMediaFile, reason: &str) -> Option<crate::NewBlocklistEntry> {
    file.release_listing_json
        .as_deref()
        .and_then(crate::release_listing_view)
        .and(file.grabbed_release_title.as_ref())
        .filter(|name| !name.trim().is_empty())
        .map(|name| crate::NewBlocklistEntry {
            title_id: file.title_id.clone(),
            release_name: name.clone(),
            indexer_id: file.indexer_source.clone().unwrap_or_default(),
            info_hash: file.release_hash.clone(),
            reason: Some(reason.to_owned()),
        })
}

pub(crate) fn content_identity(
    proof: &scryer_domain::ImportContentProof,
) -> BlocklistContentIdentity {
    BlocklistContentIdentity {
        scheme: crate::fs_integrity::IMPORT_CONTENT_PROOF_SCHEME.to_string(),
        size_bytes: proof.size_bytes,
        sample_bytes: proof.sample_bytes,
        sample_blake3: proof.sample_blake3.clone(),
    }
}

pub(crate) fn content_matches_proof(
    content: &BlocklistContentIdentity,
    proof: &scryer_domain::ImportContentProof,
) -> bool {
    content.scheme == crate::fs_integrity::IMPORT_CONTENT_PROOF_SCHEME
        && content.size_bytes == proof.size_bytes
        && content.sample_bytes == proof.sample_bytes
        && content.sample_blake3 == proof.sample_blake3
}

/// Whether the reporter asked for the media the reported file holds: an
/// approved request of theirs created the title and, for a series, covered
/// the affected episodes' seasons or the affected special movie.
pub(crate) async fn reporter_requested_affected_media(
    app: &AppUseCase,
    actor: &User,
    title: &Title,
    affected_episode_ids: &[String],
    series_movie_link_ids: &[String],
) -> Observation<bool> {
    let requests = app
        .services
        .catalog
        .media_requests
        .list(crate::MediaRequestQuery {
            library_ids: Some(vec![title.library_id.clone()]),
            status: Some(MediaRequestStatus::Approved),
            requester_user_id: Some(actor.id.clone()),
            created_title_id: Some(title.id.clone()),
            limit: Some(REPORTER_REQUEST_LIMIT + 1),
            ..Default::default()
        })
        .await;
    let requests = match requests {
        Ok(requests) => requests,
        Err(_) => return Observation::unknown("media_request_scope_lookup_failed"),
    };

    let truncated = requests.len() > REPORTER_REQUEST_LIMIT;
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

    pub async fn submit_media_issue(
        &self,
        actor: &User,
        input: SubmitMediaIssueInput,
    ) -> AppResult<MediaIssue> {
        validate_media_issue_details(input.reason, input.details.as_deref())?;
        let details = input
            .details
            .map(|details| details.trim().to_string())
            .filter(|details| !details.is_empty());
        let (file, title, library, episode_ids) = self
            .resolve_media_issue_target(actor, &input.media_file_id)
            .await?;
        let (file_version, proof) = self.inspect_media_issue_file(&file).await?;
        let provenance = media_issue_provenance(&file);
        let (issue, inserted) = self
            .services
            .workflow
            .blocklist_repo
            .submit_media_issue(&NewMediaIssue {
                library_id: title.library_id.clone(),
                title_id: title.id.clone(),
                media_file_id: file.id.clone(),
                file_path_snapshot: file.file_path.clone(),
                file_version,
                content: content_identity(&proof),
                reporter_user_id: actor.id.clone(),
                reason: input.reason,
                details,
                remediation: input.remediation,
                affected_episode_ids: episode_ids,
                affected_series_movie_link_ids: file.series_movie_link_ids.clone(),
            })
            .await?;
        if !inserted {
            // A duplicate report returns the one already recorded; its rules
            // ran and its event was recorded when it was first submitted.
            return Ok(issue);
        }
        let event = crate::domain_events::new_global_domain_event(
            actor,
            scryer_domain::DomainEventPayload::MediaIssueSubmitted(media_issue_event_data(&issue)),
        );
        let _ = self.append_domain_event(event).await;

        let evaluation = self
            .evaluate_media_issue_report(
                actor,
                &library,
                &crate::request_rules::MediaIssueReportSubject {
                    issue: &issue,
                    title: &title,
                    provenance,
                },
            )
            .await;
        if let Some(decision_id) = evaluation.decision_id.as_deref() {
            self.services
                .workflow
                .blocklist_repo
                .set_media_issue_rule_decision(&issue.id, decision_id)
                .await?;
        }
        match evaluation.outcome {
            crate::request_rules::ReportRuleOutcome::Review => {
                self.load_media_issue(&issue.id).await
            }
            crate::request_rules::ReportRuleOutcome::Reject => {
                let rejected = self
                    .services
                    .workflow
                    .blocklist_repo
                    .reject_media_issue(&issue.id, None)
                    .await?
                    .ok_or_else(|| {
                        AppError::Repository("rejected media issue could not be reloaded".into())
                    })?;
                if rejected.status == MediaIssueStatus::Rejected {
                    let event = crate::domain_events::new_global_domain_event(
                        crate::domain_events::DomainEventActor::system(),
                        scryer_domain::DomainEventPayload::MediaIssueRejected(
                            media_issue_event_data(&rejected),
                        ),
                    );
                    let _ = self.append_domain_event(event).await;
                }
                Ok(rejected)
            }
            crate::request_rules::ReportRuleOutcome::MarkBad => {
                // A rule only marks the file bad. A requested recycle and
                // search still waits for an operator.
                let _location_guard = self
                    .acquire_location_title_mutation(
                        &crate::location::ownership_guard::MEDIA_FILE_PRIMARY_ENTRY,
                        &issue.title_id,
                    )
                    .await?;
                let file = match self.revalidate_media_issue(&issue).await {
                    Ok(file) => file,
                    // The file changed under the rule; an operator decides.
                    Err(_) => return self.load_media_issue(&issue.id).await,
                };
                let confirmed = self
                    .confirm_media_issue(None, &issue, &file, MediaIssueRemediation::MarkBad, None)
                    .await?;
                let approved = self.load_media_issue(&issue.id).await?;
                if confirmed {
                    let event = crate::domain_events::new_global_domain_event(
                        crate::domain_events::DomainEventActor::system(),
                        scryer_domain::DomainEventPayload::MediaIssueApproved(
                            media_issue_event_data(&approved),
                        ),
                    );
                    let _ = self.append_domain_event(event).await;
                }
                Ok(approved)
            }
        }
    }

    pub async fn list_media_issues(
        &self,
        actor: &User,
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

    /// Mark the reported file bad without moving it.
    pub async fn approve_media_issue(&self, actor: &User, issue_id: &str) -> AppResult<MediaIssue> {
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
        let file = self.revalidate_media_issue(&issue).await?;
        let confirmed = self
            .confirm_media_issue(
                Some(actor),
                &issue,
                &file,
                MediaIssueRemediation::MarkBad,
                None,
            )
            .await?;
        let approved = self.load_media_issue(issue_id).await?;
        if confirmed {
            let event = crate::domain_events::new_global_domain_event(
                actor,
                scryer_domain::DomainEventPayload::MediaIssueApproved(media_issue_event_data(
                    &approved,
                )),
            );
            let _ = self.append_domain_event(event).await;
        }
        Ok(approved)
    }

    pub async fn reject_media_issue(&self, actor: &User, issue_id: &str) -> AppResult<MediaIssue> {
        let issue = self.load_media_issue(issue_id).await?;
        self.require_library_permission(actor, &issue.library_id, LibraryPermission::ManageTitles)
            .await?;
        if issue.status != MediaIssueStatus::Pending {
            return Ok(issue);
        }
        let rejected = self
            .services
            .workflow
            .blocklist_repo
            .reject_media_issue(&issue.id, Some(actor.id.as_str()))
            .await?
            .ok_or_else(|| {
                AppError::Repository("rejected media issue could not be reloaded".into())
            })?;
        let event = crate::domain_events::new_global_domain_event(
            actor,
            scryer_domain::DomainEventPayload::MediaIssueRejected(media_issue_event_data(
                &rejected,
            )),
        );
        let _ = self.append_domain_event(event).await;
        Ok(rejected)
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

    /// Read the reported file's version and sampled content proof, refusing
    /// when the file changes while it is read.
    pub(crate) async fn inspect_media_issue_file(
        &self,
        file: &TitleMediaFile,
    ) -> AppResult<(String, scryer_domain::ImportContentProof)> {
        let path = crate::stored_paths::stored_path_to_path_buf(&file.file_path);
        let changed = || {
            AppError::validation_refused(
                crate::error_reasons::media::SOURCE_CHANGED_DURING_INSPECTION,
                "could not verify the selected media file identity",
            )
        };
        let (before, mark) = crate::location::resolution::regular_file_snapshot(&path)
            .await
            .map_err(|_| changed())?;
        if !u64::try_from(file.size_bytes).is_ok_and(|size| mark.is_len(size)) {
            return Err(changed());
        }
        let proof = crate::fs_integrity::import_content_proof_async(&path)
            .await
            .map_err(|_| changed())?;
        let (after, _) = crate::location::resolution::regular_file_snapshot(&path)
            .await
            .map_err(|_| changed())?;
        if crate::disk::StoredVersion::new(&before) != crate::disk::StoredVersion::new(&after)
            || !mark.is_len(proof.size_bytes)
        {
            return Err(changed());
        }
        Ok((before, proof))
    }

    /// Prove the reported file is still the one reported, in the same place
    /// and for the same media.
    async fn revalidate_media_issue(&self, issue: &MediaIssue) -> AppResult<TitleMediaFile> {
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
            || u64::try_from(file.size_bytes).ok() != Some(issue.content.size_bytes)
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::media::REGISTERED_MEDIA_PATH_CHANGED_BEFORE_INSPECTION,
                "the reported media file changed before approval",
            ));
        }
        let (version, proof) = self.inspect_media_issue_file(&file).await?;
        if version != issue.file_version || !content_matches_proof(&issue.content, &proof) {
            return Err(AppError::validation_refused(
                crate::error_reasons::media::SOURCE_CHANGED_DURING_INSPECTION,
                "the reported media file identity changed before approval",
            ));
        }
        Ok(file)
    }

    /// Write the title's bad-file entry, stamp the title's files with the
    /// same content and approve the report. `reviewer` is `None` when a rule
    /// confirmed it.
    async fn confirm_media_issue(
        &self,
        reviewer: Option<&User>,
        issue: &MediaIssue,
        file: &TitleMediaFile,
        remediation: MediaIssueRemediation,
        recycle_fingerprint: Option<&str>,
    ) -> AppResult<bool> {
        let reason = issue.reason.as_str();
        let release = release_block_for_file(file, reason);
        let matching = self.matching_media_issue_file_ids(issue, file).await?;
        let confirmed = self
            .services
            .workflow
            .blocklist_repo
            .confirm_media_issue(crate::ConfirmMediaIssue {
                issue_id: &issue.id,
                reviewer_user_id: reviewer.map(|actor| actor.id.as_str()),
                remediation,
                title_id: &issue.title_id,
                content: &issue.content,
                release: release.as_ref(),
                reason,
                blocks_imports: issue.reason.applies_title_wide(),
                recycle_fingerprint,
                matching_media_file_ids: &matching,
            })
            .await?;
        if confirmed {
            self.runtime
                .acquisition
                .target_cache
                .mark_title(&issue.title_id);
        }
        Ok(confirmed)
    }

    /// The title's files holding the reported content: the reported file,
    /// already proven, and any other file of the same size whose sampled
    /// proof matches. Only same-size files are read. A wrong-content report
    /// marks the reported file alone: the same content may be right for
    /// another episode.
    async fn matching_media_issue_file_ids(
        &self,
        issue: &MediaIssue,
        reported: &TitleMediaFile,
    ) -> AppResult<Vec<String>> {
        if !issue.reason.applies_title_wide() {
            return Ok(vec![reported.id.clone()]);
        }
        let files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&issue.title_id)
            .await?;
        let mut matched = vec![reported.id.clone()];
        for file in files {
            if file.id == reported.id
                || u64::try_from(file.size_bytes).ok() != Some(issue.content.size_bytes)
            {
                continue;
            }
            let path = crate::stored_paths::stored_path_to_path_buf(&file.file_path);
            let Ok(proof) = crate::fs_integrity::import_content_proof_async(&path).await else {
                continue;
            };
            if content_matches_proof(&issue.content, &proof) {
                matched.push(file.id);
            }
        }
        Ok(matched)
    }

    /// Append an unmark event for each report whose bad-file mark was taken
    /// off. The reports stay approved.
    pub(crate) async fn record_media_issue_unmarks(
        &self,
        actor: &User,
        blocklist_id: &str,
        unmarked: &[UnmarkedMediaIssue],
        route: MediaIssueUnmarkRoute,
    ) {
        for issue in unmarked {
            let event = crate::domain_events::new_global_domain_event(
                actor,
                scryer_domain::DomainEventPayload::MediaIssueUnmarked(
                    scryer_domain::MediaIssueUnmarkedEventData {
                        issue_id: issue.issue_id.clone(),
                        library_id: issue.library_id.clone(),
                        title_id: issue.title_id.clone(),
                        blocklist_id: blocklist_id.to_string(),
                        route,
                    },
                ),
            );
            let _ = self.append_domain_event(event).await;
        }
    }

    /// Delete a blocklist entry. A bad-file entry also clears the stamp from
    /// the files it marked and records the unmark on its reports, in one
    /// transaction.
    pub(crate) async fn remove_blocklist_entry_and_unmark(
        &self,
        actor: &User,
        entry: &scryer_domain::BlocklistEntry,
        route: MediaIssueUnmarkRoute,
    ) -> AppResult<()> {
        let repo = &self.services.workflow.blocklist_repo;
        if entry.content.is_none() {
            return repo.remove(&entry.id).await;
        }
        let unmarked = repo.remove_and_unmark(&entry.id).await?;
        self.runtime
            .acquisition
            .target_cache
            .mark_title(&entry.title_id);
        self.record_media_issue_unmarks(actor, &entry.id, &unmarked, route)
            .await;
        Ok(())
    }

    /// The preview an operator confirms before a reported file is recycled.
    /// It names exactly what the recycle moves: the reported file, and a copy
    /// of the same content still in the download folder it was imported
    /// from. A reported file whose only other name is that copy is reported
    /// as a linked pair, which moves both names or nothing. It refuses any
    /// other file the recycle cannot move safely.
    pub async fn preview_media_issue_recycle(
        &self,
        actor: &User,
        issue_id: &str,
    ) -> AppResult<MediaIssueRecyclePreview> {
        let issue = self.load_media_issue(issue_id).await?;
        self.require_library_permission(actor, &issue.library_id, LibraryPermission::ManageTitles)
            .await?;
        self.require_media_issue_recyclable(&issue)?;
        let path = crate::stored_paths::stored_path_to_path_buf(&issue.file_path_snapshot);
        let (_, library_mark) = crate::location::resolution::regular_file_snapshot(&path).await?;
        let copy = self
            .media_issue_download_copy(&issue, Some(&library_mark))
            .await?;
        let linked = copy.as_ref().is_some_and(|copy| copy.linked);
        if !library_mark.has_one_name() && !linked {
            return Err(Self::hardlinked_file_refusal());
        }
        let preview = self
            .preview_delete_media_file(actor, &issue.media_file_id)
            .await?;
        let mut sample_paths = vec![path.display().to_string()];
        let mut fingerprint = preview.fingerprint;
        if let Some(copy) = &copy {
            sample_paths.push(copy.path.display().to_string());
            let separator = if copy.linked {
                LINKED_COPY_FINGERPRINT_SEPARATOR
            } else {
                DOWNLOAD_COPY_FINGERPRINT_SEPARATOR
            };
            fingerprint = format!("{fingerprint}{separator}{}", copy.version);
        }
        let file_count = if copy.is_some() { 2 } else { 1 };
        Ok(MediaIssueRecyclePreview {
            preview: crate::DeletePreview {
                fingerprint,
                total_file_count: file_count,
                media_count: file_count,
                subtitle_count: 0,
                image_count: 0,
                other_count: 0,
                directory_count: 0,
                requires_typed_confirmation: true,
                typed_confirmation_prompt: Some(
                    "Type DELETE to confirm recycling this file.".to_string(),
                ),
                target_label: preview.target_label,
                sample_paths,
            },
            hard_link_reason: linked
                .then_some(crate::error_reasons::media::RECYCLE_HARDLINKED_DOWNLOAD_COPY),
        })
    }

    /// A copy of the reported file's content still in the download folder it
    /// was imported from, at the path Scryer's own import recorded inside
    /// that folder. A regular file with one name counts when its sampled
    /// content is the reported content. A file with two names counts only as
    /// the other name of `other_name`, the reported file or its recycled
    /// payload, proven by one volume and one file number; its content is
    /// then that file's content. Anything else is left alone. Two indexed
    /// reads, one probe and at most one sampled read, and only when an
    /// operator previews or runs a confirmed recycle.
    pub(crate) async fn media_issue_download_copy(
        &self,
        issue: &MediaIssue,
        other_name: Option<&crate::disk::Mark>,
    ) -> AppResult<Option<MediaIssueDownloadCopy>> {
        let workflow = &self.services.workflow;
        let Some(source) = workflow
            .import_artifacts
            .imported_source_for_media_file(&issue.media_file_id)
            .await?
        else {
            return Ok(None);
        };
        let Some(record) = workflow.imports.get_import_by_id(&source.import_id).await? else {
            return Ok(None);
        };
        let Some(download_dir) =
            crate::import::workflow::completed_import_download_dir(self, &record).await
        else {
            return Ok(None);
        };
        let Some(path) = download_copy_path(&download_dir, &source.relative_path) else {
            return Ok(None);
        };
        let library_path = crate::stored_paths::stored_path_to_path_buf(&issue.file_path_snapshot);
        if path == library_path {
            return Ok(None);
        }
        let mark = match crate::disk::Disk::process().fingerprint(&path).await {
            Ok(mark) => mark,
            Err(reading) if reading.exists() == Some(false) => return Ok(None),
            Err(reading) => return Err(AppError::Repository(reading.absence().to_string())),
        };
        if mark.kind() != Some(crate::disk::EntryKind::File) {
            return Ok(None);
        }
        let linked = if mark.has_one_name() {
            false
        } else if other_name.is_some_and(|other| {
            mark.has_two_names()
                && other.has_two_names()
                && mark.same_file(other) == crate::disk::Sameness::Same
        }) {
            true
        } else {
            return Ok(None);
        };
        #[cfg(unix)]
        let version = mark.legacy_unix_version();
        #[cfg(not(unix))]
        let version: Option<String> = None;
        let Some(version) = version else {
            return Ok(None);
        };
        if !linked {
            let proof = crate::fs_integrity::import_content_proof_async(&path).await?;
            if !content_matches_proof(&issue.content, &proof) {
                return Ok(None);
            }
        }
        Ok(Some(MediaIssueDownloadCopy {
            path,
            download_dir,
            version,
            linked,
        }))
    }

    fn require_media_issue_recyclable(&self, issue: &MediaIssue) -> AppResult<()> {
        if issue.remediation != MediaIssueRemediation::RecycleAndSearch {
            return Err(AppError::validation_refused(
                crate::error_reasons::media::REMEDIATION_NOT_AVAILABLE,
                "this media issue did not request recycle-and-search",
            ));
        }
        if !cfg!(unix) {
            // The stored file version does not identify a file after a move
            // on this platform, so the move could not be verified.
            return Err(AppError::validation_refused(
                crate::error_reasons::media::RECYCLE_UNSUPPORTED_PLATFORM,
                "recycling a reported file is not available on this platform",
            ));
        }
        Ok(())
    }

    /// The refusal of a file with another name that is not its download
    /// copy: recycling one name would leave the content in place under
    /// another.
    pub(crate) fn hardlinked_file_refusal() -> AppError {
        AppError::validation_refused(
            crate::error_reasons::media::RECYCLE_HARDLINKED_FILE,
            "the reported file has other hard links; recycling it would leave the content in place",
        )
    }

    /// Mark the reported file bad, recycle it and search for a replacement,
    /// after the operator confirmed the preview. Also retries a failed
    /// recycle, and recycles a file a rule already marked bad. A file
    /// hard-linked to its download copy is recycled only with
    /// `recycle_both_names`, which then moves both names.
    pub async fn approve_media_issue_recycle_search(
        &self,
        actor: &User,
        issue_id: &str,
        preview_fingerprint: &str,
        typed_confirmation: Option<&str>,
        recycle_both_names: bool,
    ) -> AppResult<MediaIssue> {
        let issue = self.load_media_issue(issue_id).await?;
        self.require_library_permission(actor, &issue.library_id, LibraryPermission::ManageTitles)
            .await?;
        self.require_media_issue_recyclable(&issue)?;
        let retry = issue.status == MediaIssueStatus::Approved
            && issue.authorized_remediation == Some(MediaIssueRemediation::RecycleAndSearch)
            && issue.execution_status == MediaIssueExecutionStatus::RecycleFailed;
        let upgrade = issue.status == MediaIssueStatus::Approved
            && issue.authorized_remediation == Some(MediaIssueRemediation::MarkBad)
            && issue.execution_status == MediaIssueExecutionStatus::Blocked;
        if !retry && !upgrade && issue.status != MediaIssueStatus::Pending {
            return Ok(issue);
        }
        Self::require_media_issue_recycle_confirmation(typed_confirmation)?;
        require_linked_copy_choice(
            split_media_issue_recycle_fingerprint(preview_fingerprint).1,
            recycle_both_names,
        )?;
        // A retry whose move already ran finishes from its receipt; the
        // reported path is empty by then, so there is nothing to preview.
        let moved = retry
            && self
                .services
                .workflow
                .blocklist_repo
                .get_media_issue_recycle_receipt(issue_id)
                .await?
                .is_some();
        if moved {
            // The move ran under the fingerprint confirmed then; a retry
            // finishes only that confirmation.
            let confirmed = self
                .services
                .workflow
                .blocklist_repo
                .get_media_issue_recycle_fingerprint(issue_id)
                .await?;
            if confirmed.as_deref() != Some(preview_fingerprint) {
                return Err(AppError::validation_refused(
                    crate::delete_refusal::PREVIEW_STALE,
                    "the recycle preview changed before confirmation",
                ));
            }
        } else {
            let preview = self.preview_media_issue_recycle(actor, issue_id).await?;
            if preview_fingerprint != preview.preview.fingerprint {
                return Err(AppError::validation_refused(
                    crate::delete_refusal::PREVIEW_STALE,
                    "the recycle preview changed before confirmation",
                ));
            }
            self.validate_delete_media_file(
                &issue.media_file_id,
                split_media_issue_recycle_fingerprint(preview_fingerprint).0,
                typed_confirmation,
            )
            .await?;
        }
        if retry {
            if !self
                .services
                .workflow
                .blocklist_repo
                .set_media_issue_execution_status(
                    &issue.id,
                    MediaIssueExecutionStatus::Recycling,
                    None,
                )
                .await?
            {
                return self.load_media_issue(issue_id).await;
            }
        } else {
            let file = self.revalidate_media_issue(&issue).await?;
            if !self
                .confirm_media_issue(
                    Some(actor),
                    &issue,
                    &file,
                    MediaIssueRemediation::RecycleAndSearch,
                    Some(preview_fingerprint),
                )
                .await?
            {
                return self.load_media_issue(issue_id).await;
            }
            let approved = self.load_media_issue(issue_id).await?;
            let event = crate::domain_events::new_global_domain_event(
                actor,
                scryer_domain::DomainEventPayload::MediaIssueApproved(media_issue_event_data(
                    &approved,
                )),
            );
            let _ = self.append_domain_event(event).await;
        }
        let confirmation = crate::DeleteExecutionConfirmation {
            preview_fingerprint: preview_fingerprint.to_string(),
            typed_confirmation: typed_confirmation.map(str::to_string),
        };
        self.start_media_issue_recycle_job(actor, &issue, confirmation)
            .await?;
        self.load_media_issue(issue_id).await
    }

    /// Start the recycle job. A recycle already running for the file is left
    /// to finish and records nothing; any other failure to start is recorded
    /// on the report.
    async fn start_media_issue_recycle_job(
        &self,
        actor: &User,
        issue: &MediaIssue,
        confirmation: crate::DeleteExecutionConfirmation,
    ) -> AppResult<()> {
        match self
            .start_media_file_removal_job(
                actor,
                &issue.media_file_id,
                true,
                Some(confirmation),
                Some(issue.clone()),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(error)
                if error.validation_reason()
                    == Some(crate::error_reasons::catalog::MEDIA_FILE_DELETION_ALREADY_RUNNING) =>
            {
                Ok(())
            }
            Err(error) => {
                self.record_media_issue_recycle_failure(&issue.id, &error)
                    .await
            }
        }
    }

    /// Resume operator-confirmed recycles a restart interrupted. Rule marks
    /// and failed recycles are never resumed. With nothing to resume this
    /// reads once and writes nothing.
    pub async fn resume_interrupted_media_issue_recycles(&self) -> AppResult<()> {
        const RESUME_LIMIT: usize = 1_000;
        let intents = self
            .services
            .workflow
            .blocklist_repo
            .interrupted_media_issue_recycles(RESUME_LIMIT)
            .await?;
        for intent in intents {
            let issue = self.load_media_issue(&intent.issue_id).await?;
            let actor = self
                .services
                .identity
                .users
                .get_by_id(&intent.actor_id)
                .await?
                .filter(|actor| actor.login_status().is_enabled());
            let Some(actor) = actor else {
                let error = AppError::validation_refused(
                    crate::error_reasons::media::REMEDIATION_NOT_AVAILABLE,
                    "the original media issue reviewer is no longer authorized",
                );
                self.record_media_issue_recycle_failure(&intent.issue_id, &error)
                    .await?;
                continue;
            };
            let confirmation = crate::DeleteExecutionConfirmation {
                preview_fingerprint: intent.preview_fingerprint,
                typed_confirmation: Some(
                    crate::library::user_delete::DELETE_TYPED_CONFIRMATION_VALUE.to_owned(),
                ),
            };
            self.start_media_issue_recycle_job(&actor, &issue, confirmation)
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn run_media_issue_recycle(
        &self,
        actor: &User,
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
        let changed = self
            .services
            .workflow
            .blocklist_repo
            .set_media_issue_execution_status(
                issue_id,
                MediaIssueExecutionStatus::RecycleFailed,
                Some(
                    error
                        .validation_reason()
                        .unwrap_or(crate::error_reasons::media::RECYCLE_FAILED),
                ),
            )
            .await?;
        if !changed {
            return Ok(());
        }
        let issue = self.load_media_issue(issue_id).await?;
        let event = crate::domain_events::new_global_domain_event(
            crate::domain_events::DomainEventActor::system(),
            scryer_domain::DomainEventPayload::MediaIssueRecycleExecution(media_issue_event_data(
                &issue,
            )),
        );
        let _ = self.append_domain_event(event).await;
        Ok(())
    }

    async fn execute_media_issue_recycle(
        &self,
        actor: &User,
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
            || issue.execution_status != MediaIssueExecutionStatus::Recycling
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
        // A receipt means a move may already have run. Recover from it before
        // revalidating: after the move the reported path is empty, and the
        // revalidation would refuse what only needs finishing.
        let receipt = self
            .services
            .workflow
            .blocklist_repo
            .get_media_issue_recycle_receipt(issue_id)
            .await?
            .map(|json| {
                serde_json::from_str::<crate::recycle_bin::MediaIssueRecycleReceipt>(&json).map_err(
                    |error| AppError::Repository(format!("invalid recycle receipt: {error}")),
                )
            })
            .transpose()?;
        let confirmed_copy =
            split_media_issue_recycle_fingerprint(&confirmation.preview_fingerprint).1;
        let recovered = match receipt.as_ref() {
            Some(receipt) => self
                .finish_recovered_media_issue_recycle(&issue, receipt, confirmed_copy)
                .await?
                .then(|| receipt.clone()),
            None => None,
        };
        let recycled = match recovered {
            Some(receipt) => receipt,
            None => {
                self.require_media_issue_recyclable(&issue)?;
                let file = self.revalidate_media_issue(&issue).await?;
                let prepared = self
                    .recycle_reviewed_media_file(
                        &issue,
                        &file,
                        confirmation,
                        receipt.map(|receipt| receipt.file),
                    )
                    .await?;
                crate::recycle_bin::MediaIssueRecycleReceipt {
                    file: prepared,
                    linked_copy: None,
                }
            }
        };
        // Before the row goes: the import record finds the copy through it.
        // A failure here leaves the reported file in the bin under its
        // receipt and the copy in place, and a retry finishes from both.
        if let Some(confirmed_copy) = confirmed_copy {
            self.recycle_media_issue_download_copy(&issue, confirmed_copy, &recycled)
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
    ) -> AppResult<MediaIssueExecutionStatus> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&issue.title_id)
            .await?;
        let Some(title) =
            title.filter(|title| title.monitored && title.library_id == issue.library_id)
        else {
            return Ok(MediaIssueExecutionStatus::NoEligibleRelease);
        };
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
        let wanted_episode_ids = if title.facet == MediaFacet::Movie {
            std::collections::HashSet::new()
        } else {
            self.services
                .catalog
                .shows
                .get_episodes_by_ids(&issue.affected_episode_ids)
                .await?
                .into_iter()
                .filter(|episode| episode.monitored && episode.title_id == title.id)
                .map(|episode| episode.id)
                .collect::<std::collections::HashSet<_>>()
        };
        let mut saw_searching = false;
        let mut saw_queued = false;
        for scope in &scope_states {
            let affected = if title.facet == MediaFacet::Movie {
                scope.title_id == issue.title_id
                    && scope.episode_id.is_none()
                    && scope.series_movie_link_id.is_none()
            } else {
                scope
                    .episode_id
                    .as_deref()
                    .is_some_and(|id| wanted_episode_ids.contains(id))
                    || scope.series_movie_link_id.as_deref().is_some_and(|id| {
                        issue.affected_series_movie_link_ids.iter().any(|l| l == id)
                    })
            };
            if !affected {
                continue;
            }
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
        Ok(if saw_queued {
            MediaIssueExecutionStatus::DownloadQueued
        } else if saw_searching {
            MediaIssueExecutionStatus::Searching
        } else {
            MediaIssueExecutionStatus::NoEligibleRelease
        })
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
    use super::*;
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

    #[test]
    fn other_reason_needs_details_and_details_are_bounded() {
        let refusal = |reason, details| {
            validate_media_issue_details(reason, details)
                .unwrap_err()
                .validation_reason()
                .map(str::to_owned)
        };
        assert_eq!(
            refusal(MediaIssueReason::Other, None).as_deref(),
            Some(crate::error_reasons::media::REASON_REQUIRES_DETAILS)
        );
        assert_eq!(
            refusal(MediaIssueReason::Other, Some("   ")).as_deref(),
            Some(crate::error_reasons::media::REASON_REQUIRES_DETAILS)
        );
        let long = "x".repeat(MAX_MEDIA_ISSUE_DETAILS_LENGTH + 1);
        assert_eq!(
            refusal(MediaIssueReason::Corrupted, Some(&long)).as_deref(),
            Some(crate::error_reasons::media::DETAILS_TOO_LONG)
        );
        assert!(validate_media_issue_details(MediaIssueReason::Corrupted, None).is_ok());
        assert!(validate_media_issue_details(MediaIssueReason::Other, Some("hiss")).is_ok());
    }

    #[test]
    fn an_unknown_reason_is_refused_with_its_code() {
        assert_eq!(
            parse_media_issue_reason("audio_out_of_sync").unwrap(),
            MediaIssueReason::AudioOutOfSync
        );
        assert_eq!(
            parse_media_issue_reason("melted")
                .unwrap_err()
                .validation_reason(),
            Some(crate::error_reasons::media::REASON_UNKNOWN)
        );
    }

    #[test]
    fn a_confirmed_fingerprint_names_its_copy_and_whether_it_is_linked() {
        assert_eq!(
            split_media_issue_recycle_fingerprint("file-fp"),
            ("file-fp", None)
        );
        assert_eq!(
            split_media_issue_recycle_fingerprint("file-fp|download:1:2:3:4:5:6:7"),
            (
                "file-fp",
                Some(ConfirmedDownloadCopy {
                    version: "1:2:3:4:5:6:7",
                    linked: false,
                })
            )
        );
        assert_eq!(
            split_media_issue_recycle_fingerprint("file-fp|linked:1:2:3:4:5:6:7"),
            (
                "file-fp",
                Some(ConfirmedDownloadCopy {
                    version: "1:2:3:4:5:6:7",
                    linked: true,
                })
            )
        );
    }

    /// A linked pair is recycled only when the operator chose both names, and
    /// that choice is refused when the preview shows no linked pair.
    #[test]
    fn recycling_a_linked_pair_needs_the_explicit_choice_and_nothing_else_takes_it() {
        let linked = Some(ConfirmedDownloadCopy {
            version: "1:2:3:4:5:6:7",
            linked: true,
        });
        let separate = Some(ConfirmedDownloadCopy {
            version: "1:2:3:4:5:6:7",
            linked: false,
        });
        assert_eq!(
            require_linked_copy_choice(linked, false)
                .unwrap_err()
                .validation_reason(),
            Some(crate::error_reasons::media::RECYCLE_HARDLINKED_DOWNLOAD_COPY)
        );
        require_linked_copy_choice(linked, true).unwrap();
        for confirmed in [None, separate] {
            require_linked_copy_choice(confirmed, false).unwrap();
            assert_eq!(
                require_linked_copy_choice(confirmed, true)
                    .unwrap_err()
                    .validation_reason(),
                Some(crate::delete_refusal::PREVIEW_STALE)
            );
        }
    }

    /// A linked copy's change time moves when its other name is recycled, so
    /// it is compared without it; a separate copy is compared whole.
    #[test]
    fn a_linked_copy_is_still_named_across_a_change_time_move() {
        let copy = |version: &str, linked: bool| MediaIssueDownloadCopy {
            path: std::path::PathBuf::from("/synthetic/copy.mkv"),
            download_dir: std::path::PathBuf::from("/synthetic"),
            version: version.to_string(),
            linked,
        };
        let confirmed = ConfirmedDownloadCopy {
            version: "1:2:3:4:5:6:7",
            linked: true,
        };
        assert!(confirmed.still_names(&copy("1:2:3:4:5:8:9", true)));
        assert!(!confirmed.still_names(&copy("1:2:3:4:6:6:7", true)));
        assert!(!confirmed.still_names(&copy("1:2:3:4:5:6:7", false)));
        let separate = ConfirmedDownloadCopy {
            version: "1:2:3:4:5:6:7",
            linked: false,
        };
        assert!(separate.still_names(&copy("1:2:3:4:5:6:7", false)));
        assert!(!separate.still_names(&copy("1:2:3:4:5:8:9", false)));
    }
}
