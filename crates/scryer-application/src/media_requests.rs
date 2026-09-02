use super::*;
use crate::domain_events::new_global_domain_event;
use crate::ports::MediaRequestResolution;
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventPayload, DomainEventType, LibraryPermission,
    MediaRequestResolvedEventData, MediaRequestStatus, MediaRequestSubmittedEventData,
};
use std::collections::BTreeSet;

const TITLE_QUALITY_PROFILE_TAG_PREFIX: &str = "scryer:quality-profile:";
const TITLE_MONITOR_TYPE_TAG_PREFIX: &str = "scryer:monitor-type:";

#[derive(Clone, Debug)]
pub struct SubmitMediaRequestInput {
    pub library_id: String,
    pub facet: MediaFacet,
    pub title: String,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub language: Option<String>,
    pub content_status: Option<String>,
    pub requested_quality_profile_id: Option<String>,
    pub requested_monitor_type: Option<String>,
    pub external_ids: Vec<ExternalId>,
}

#[derive(Clone, Debug)]
pub struct SubmitMediaRequestOutcome {
    pub request_id: String,
}

#[derive(Clone, Debug)]
pub struct ApproveMediaRequestOutcome {
    pub title_id: String,
    pub wanted_search: Option<WantedSearchOutcome>,
    pub search_error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ListMediaRequestsInput {
    pub facet: Option<MediaFacet>,
    pub library_ids: Option<Vec<String>>,
    pub status: Option<MediaRequestStatus>,
}

#[derive(Clone, Debug)]
pub struct UpdateMediaRequestInput {
    pub request_id: String,
    pub requested_quality_profile_id: String,
    pub requested_monitor_type: Option<String>,
}

impl AppUseCase {
    pub async fn submit_media_request(
        &self,
        actor: &User,
        input: SubmitMediaRequestInput,
    ) -> AppResult<SubmitMediaRequestOutcome> {
        let title = input.title.trim().to_string();
        if title.is_empty() {
            return Err(AppError::Validation("request title is required".into()));
        }

        let mut external_ids = normalize_media_request_external_ids(input.external_ids)?;
        if external_ids.is_empty() {
            return Err(AppError::Validation(
                "media requests must include SMG external identifiers".into(),
            ));
        }
        if !external_ids
            .iter()
            .any(is_smg_request_correlation_external_id)
        {
            return Err(AppError::Validation(
                "media requests must include a searchable SMG identifier".into(),
            ));
        }

        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(input.library_id.trim())
            .await?
            .ok_or_else(|| AppError::NotFound("library not found".into()))?;

        if library.facet != input.facet {
            return Err(AppError::Validation(
                "library facet does not match requested media facet".into(),
            ));
        }

        self.require_library_permission(actor, &library.id, LibraryPermission::Request)
            .await?;
        let metadata_enrichment = self
            .enrich_media_request_metadata(&input.facet, external_ids)
            .await;
        external_ids = metadata_enrichment.external_ids;
        self.ensure_request_subject_is_not_in_library(
            &library.id,
            library.facet.clone(),
            &external_ids,
        )
        .await?;
        let profile_reference_guard = self
            .runtime
            .catalog
            .quality_profile_reference_lock
            .lock()
            .await;
        let (requested_quality_profile_id, requested_quality_profile_name) = self
            .request_quality_profile_snapshot_for_submission(
                &library,
                input.requested_quality_profile_id,
            )
            .await?;
        let requested_monitor_type =
            normalize_requested_monitor_type(&input.facet, input.requested_monitor_type)?;
        let poster_url = metadata_enrichment.poster_url;
        let overview = metadata_enrichment
            .overview
            .or_else(|| normalized_optional_string(input.overview));
        let rating_summary = metadata_enrichment.rating_summary;

        let request = NewMediaRequest {
            id: Id::new().0,
            library_id: library.id.clone(),
            facet: input.facet,
            identity_fingerprint: media_request_identity_fingerprint(&external_ids),
            title,
            sort_title: normalized_optional_string(input.sort_title),
            slug: normalized_optional_string(input.slug),
            poster_url,
            year: input.year,
            overview,
            runtime_minutes: input.runtime_minutes,
            language: normalized_optional_string(input.language),
            content_status: normalized_optional_string(input.content_status),
            rating_summary,
            requested_quality_profile_id: Some(requested_quality_profile_id),
            requested_quality_profile_name: Some(requested_quality_profile_name),
            requested_monitor_type,
            external_ids,
            created_by_user_id: actor.id.clone(),
        };
        let submitted_event = new_global_domain_event(
            actor,
            DomainEventPayload::MediaRequestSubmitted(MediaRequestSubmittedEventData {
                request_id: request.id.clone(),
                library_id: request.library_id.clone(),
                facet: request.facet.clone(),
                title_name: request.title.clone(),
                external_ids: request.external_ids.clone(),
                poster_url: request.poster_url.clone(),
                year: request.year,
                requested_quality_profile_id: request.requested_quality_profile_id.clone(),
                requested_quality_profile_name: request.requested_quality_profile_name.clone(),
                requested_monitor_type: request.requested_monitor_type.clone(),
            }),
        );

        let submission = self
            .services
            .catalog
            .media_requests
            .submit(request, actor, submitted_event)
            .await?;
        drop(profile_reference_guard);
        self.publish_stored_domain_event(&submission.event).await;
        let submitted_request = submission.request;
        let request_id = submitted_request.id.clone();

        if self
            .has_granted_library_permission(
                actor,
                &submitted_request.library_id,
                LibraryPermission::AutoApproveRequests,
            )
            .await?
        {
            self.auto_approve_submitted_media_request(actor, submitted_request)
                .await?;
        }

        Ok(SubmitMediaRequestOutcome { request_id })
    }

    pub async fn list_media_requests(
        &self,
        actor: &User,
        input: ListMediaRequestsInput,
    ) -> AppResult<Vec<MediaRequest>> {
        let allowed_ids = self
            .library_ids_with_library_permission(
                actor,
                input.facet.clone(),
                LibraryPermission::ManageTitles,
            )
            .await?;
        let allowed_ids = allowed_ids.into_iter().collect::<HashSet<_>>();

        let library_ids = match input.library_ids {
            Some(requested_ids) => requested_ids
                .into_iter()
                .filter(|id| allowed_ids.contains(id))
                .collect::<Vec<_>>(),
            None => allowed_ids.into_iter().collect::<Vec<_>>(),
        };

        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        self.services
            .catalog
            .media_requests
            .list(MediaRequestQuery {
                facet: input.facet,
                library_ids: Some(library_ids),
                status: input.status,
                requester_user_id: None,
            })
            .await
    }

    pub async fn list_my_media_requests(
        &self,
        actor: &User,
        input: ListMediaRequestsInput,
    ) -> AppResult<Vec<MediaRequest>> {
        let allowed_ids = self
            .library_ids_with_library_permission(
                actor,
                input.facet.clone(),
                LibraryPermission::Request,
            )
            .await?;
        let allowed_ids = allowed_ids.into_iter().collect::<HashSet<_>>();

        let library_ids = match input.library_ids {
            Some(requested_ids) => requested_ids
                .into_iter()
                .filter(|id| allowed_ids.contains(id))
                .collect::<Vec<_>>(),
            None => allowed_ids.into_iter().collect::<Vec<_>>(),
        };

        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        self.services
            .catalog
            .media_requests
            .list(MediaRequestQuery {
                facet: input.facet,
                library_ids: Some(library_ids),
                status: input.status,
                requester_user_id: Some(actor.id.clone()),
            })
            .await
    }

    pub async fn approve_media_request(
        &self,
        actor: &User,
        request_id: &str,
        quality_profile_id: &str,
        monitor_type: Option<String>,
    ) -> AppResult<ApproveMediaRequestOutcome> {
        let request = self
            .load_manageable_pending_media_request(actor, request_id)
            .await?;
        let profile_reference_guard = self
            .runtime
            .catalog
            .quality_profile_reference_lock
            .lock()
            .await;
        self.require_library_permission(
            actor,
            &request.library_id,
            LibraryPermission::ManageTitles,
        )
        .await?;
        let (approved_quality_profile_id, approved_quality_profile_name) =
            self.quality_profile_snapshot(quality_profile_id).await?;
        let approved_monitor_type = match monitor_type {
            Some(value) => normalize_requested_monitor_type(&request.facet, Some(value))?,
            None => normalize_requested_monitor_type(
                &request.facet,
                request.requested_monitor_type.clone(),
            )?,
        };
        let outcome = self
            .add_title_with_outcome_after_library_authorization_profile_lock_held(
                actor,
                media_request_to_new_title(
                    &request,
                    Some(&approved_quality_profile_id),
                    approved_monitor_type.as_deref(),
                ),
                request.library_id.clone(),
            )
            .await?;
        let resolved_event = new_global_domain_event(
            actor,
            DomainEventPayload::MediaRequestApproved(media_request_resolved_event_data(
                &request,
                Some(outcome.title.id.clone()),
                Some(approved_quality_profile_id.clone()),
                Some(approved_quality_profile_name.clone()),
            )),
        );
        let resolution = self
            .services
            .catalog
            .media_requests
            .resolve_pending_overlapping(
                &request,
                MediaRequestResolution {
                    status: MediaRequestStatus::Approved,
                    resolved_by_user_id: actor.id.clone(),
                    resolved_at: chrono::Utc::now(),
                    created_title_id: Some(outcome.title.id.clone()),
                    approved_quality_profile_id: Some(approved_quality_profile_id),
                    approved_quality_profile_name: Some(approved_quality_profile_name),
                    event: resolved_event,
                },
            )
            .await?;
        drop(profile_reference_guard);
        if let Some(event) = &resolution.event {
            self.publish_stored_domain_event(event).await;
        }
        let title_id = outcome.title.id.clone();
        let wanted_search = match self
            .trigger_title_wanted_search(
                actor,
                &title_id,
                SubmissionConflictPolicy::from_replace_flag(false),
            )
            .await
        {
            Ok(wanted_search) => Some(wanted_search),
            Err(error) => {
                return Ok(ApproveMediaRequestOutcome {
                    title_id,
                    wanted_search: None,
                    search_error: Some(error.to_string()),
                });
            }
        };

        Ok(ApproveMediaRequestOutcome {
            title_id,
            wanted_search,
            search_error: None,
        })
    }

    pub async fn dismiss_media_request(&self, actor: &User, request_id: &str) -> AppResult<u64> {
        let request = self
            .load_manageable_pending_media_request(actor, request_id)
            .await?;
        let resolved_event = new_global_domain_event(
            actor,
            DomainEventPayload::MediaRequestRejected(media_request_resolved_event_data(
                &request, None, None, None,
            )),
        );
        let resolution = self
            .services
            .catalog
            .media_requests
            .resolve_pending_overlapping(
                &request,
                MediaRequestResolution {
                    status: MediaRequestStatus::Rejected,
                    resolved_by_user_id: actor.id.clone(),
                    resolved_at: chrono::Utc::now(),
                    created_title_id: None,
                    approved_quality_profile_id: None,
                    approved_quality_profile_name: None,
                    event: resolved_event,
                },
            )
            .await?;
        if let Some(event) = &resolution.event {
            self.publish_stored_domain_event(event).await;
        }
        Ok(resolution.updated)
    }

    pub async fn update_my_media_request(
        &self,
        actor: &User,
        input: UpdateMediaRequestInput,
    ) -> AppResult<MediaRequest> {
        let request = self
            .load_requester_pending_media_request(actor, &input.request_id)
            .await?;
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(&request.library_id)
            .await?
            .ok_or_else(|| AppError::NotFound("library not found".into()))?;
        let profile_reference_guard = self
            .runtime
            .catalog
            .quality_profile_reference_lock
            .lock()
            .await;
        let (requested_quality_profile_id, requested_quality_profile_name) = self
            .request_quality_profile_snapshot_for_submission(
                &library,
                Some(input.requested_quality_profile_id),
            )
            .await?;
        let requested_monitor_type =
            normalize_requested_monitor_type(&request.facet, input.requested_monitor_type)?;
        let updated_event = new_global_domain_event(
            actor,
            DomainEventPayload::MediaRequestUpdated(media_request_submitted_event_data(
                &request,
                Some(requested_quality_profile_id.clone()),
                Some(requested_quality_profile_name.clone()),
                requested_monitor_type.clone(),
            )),
        );

        let update = self
            .services
            .catalog
            .media_requests
            .update_pending_request_preferences(
                &request.id,
                requested_quality_profile_id,
                requested_quality_profile_name,
                requested_monitor_type,
                updated_event,
            )
            .await?;
        drop(profile_reference_guard);
        self.publish_stored_domain_event(&update.event).await;
        Ok(update.request)
    }

    pub async fn cancel_my_media_request(&self, actor: &User, request_id: &str) -> AppResult<u64> {
        let request = self
            .load_requester_pending_media_request(actor, request_id)
            .await?;
        let canceled_at = chrono::Utc::now();
        let canceled_event = new_global_domain_event(
            actor,
            DomainEventPayload::MediaRequestCanceled(media_request_resolved_event_data(
                &request, None, None, None,
            )),
        );

        let resolution = self
            .services
            .catalog
            .media_requests
            .resolve_pending(
                &request.id,
                MediaRequestResolution {
                    status: MediaRequestStatus::Canceled,
                    resolved_by_user_id: actor.id.clone(),
                    resolved_at: canceled_at,
                    created_title_id: None,
                    approved_quality_profile_id: None,
                    approved_quality_profile_name: None,
                    event: canceled_event,
                },
            )
            .await?;
        if let Some(event) = &resolution.event {
            self.publish_stored_domain_event(event).await;
        }
        Ok(resolution.updated)
    }

    async fn auto_approve_submitted_media_request(
        &self,
        actor: &User,
        request: MediaRequest,
    ) -> AppResult<()> {
        if request.status != MediaRequestStatus::Pending {
            return Ok(());
        }

        let approved_quality_profile_id = request
            .requested_quality_profile_id
            .clone()
            .ok_or_else(|| AppError::Validation("approved quality profile is required".into()))?;
        let approved_quality_profile_name = request
            .requested_quality_profile_name
            .clone()
            .ok_or_else(|| {
                AppError::Validation("approved quality profile name is required".into())
            })?;
        let approved_monitor_type = normalize_requested_monitor_type(
            &request.facet,
            request.requested_monitor_type.clone(),
        )?;
        let outcome = self
            .add_title_with_outcome_after_library_authorization(
                actor,
                media_request_to_new_title(
                    &request,
                    Some(&approved_quality_profile_id),
                    approved_monitor_type.as_deref(),
                ),
                request.library_id.clone(),
            )
            .await?;
        let resolved_event = new_global_domain_event(
            actor,
            DomainEventPayload::MediaRequestApproved(media_request_resolved_event_data(
                &request,
                Some(outcome.title.id.clone()),
                Some(approved_quality_profile_id.clone()),
                Some(approved_quality_profile_name.clone()),
            )),
        );
        let resolution = self
            .services
            .catalog
            .media_requests
            .resolve_pending_overlapping(
                &request,
                MediaRequestResolution {
                    status: MediaRequestStatus::Approved,
                    resolved_by_user_id: actor.id.clone(),
                    resolved_at: chrono::Utc::now(),
                    created_title_id: Some(outcome.title.id.clone()),
                    approved_quality_profile_id: Some(approved_quality_profile_id),
                    approved_quality_profile_name: Some(approved_quality_profile_name),
                    event: resolved_event,
                },
            )
            .await?;
        if let Some(event) = &resolution.event {
            self.publish_stored_domain_event(event).await;
        }

        if let Err(error) = self
            .trigger_title_wanted_search(
                actor,
                &outcome.title.id,
                SubmissionConflictPolicy::from_replace_flag(false),
            )
            .await
        {
            tracing::warn!(
                request_id = request.id.as_str(),
                title_id = outcome.title.id.as_str(),
                error = %error,
                "auto-approved media request but wanted search failed"
            );
        }

        Ok(())
    }

    pub async fn pending_media_request_counts(
        &self,
        actor: &User,
    ) -> AppResult<MediaRequestCounts> {
        let library_ids = self
            .library_ids_with_library_permission(actor, None, LibraryPermission::ManageTitles)
            .await?;

        if library_ids.is_empty() {
            return Ok(MediaRequestCounts::default());
        }

        self.services
            .catalog
            .media_requests
            .count_pending_by_facet(&library_ids)
            .await
    }

    pub async fn can_manage_media_requests(&self, actor: &User) -> AppResult<bool> {
        Ok(!self
            .library_ids_with_library_permission(actor, None, LibraryPermission::ManageTitles)
            .await?
            .is_empty())
    }

    pub async fn can_access_media_requests(&self, actor: &User) -> AppResult<bool> {
        if self.can_manage_media_requests(actor).await? {
            return Ok(true);
        }

        Ok(!self
            .library_ids_with_library_permission(actor, None, LibraryPermission::Request)
            .await?
            .is_empty())
    }

    pub async fn list_media_request_lifecycle_events_for_manager(
        &self,
        actor: &User,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let allowed_library_ids = self
            .library_ids_with_library_permission(actor, None, LibraryPermission::ManageTitles)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if allowed_library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let target_len = if limit == 0 { 100 } else { limit.min(500) };
        let mut page_filter = DomainEventFilter {
            event_types: Some(media_request_lifecycle_event_types()),
            after_sequence: Some(after_sequence),
            limit: 500,
            ..DomainEventFilter::default()
        };
        let mut visible = Vec::new();

        loop {
            let events = self
                .services
                .events
                .domain_events
                .list(&page_filter)
                .await?;
            if events.is_empty() {
                break;
            }

            let next_sequence = events.last().map(|event| event.sequence);
            let batch_len = events.len();
            for event in events {
                if media_request_lifecycle_event_library_id(&event)
                    .is_some_and(|library_id| allowed_library_ids.contains(library_id))
                {
                    visible.push(event);
                    if visible.len() >= target_len {
                        return Ok(visible);
                    }
                }
            }

            if batch_len < 500 {
                break;
            }

            page_filter.after_sequence = next_sequence;
        }

        Ok(visible)
    }

    pub async fn list_media_request_lifecycle_events_for_actor(
        &self,
        actor: &User,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let manageable_library_ids = self
            .library_ids_with_library_permission(actor, None, LibraryPermission::ManageTitles)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let requestable_library_ids = self
            .library_ids_with_library_permission(actor, None, LibraryPermission::Request)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if manageable_library_ids.is_empty() && requestable_library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let target_len = if limit == 0 { 100 } else { limit.min(500) };
        let mut page_filter = DomainEventFilter {
            event_types: Some(media_request_lifecycle_event_types()),
            after_sequence: Some(after_sequence),
            limit: 500,
            ..DomainEventFilter::default()
        };
        let mut visible = Vec::new();

        loop {
            let events = self
                .services
                .events
                .domain_events
                .list(&page_filter)
                .await?;
            if events.is_empty() {
                break;
            }

            let next_sequence = events.last().map(|event| event.sequence);
            let batch_len = events.len();
            for event in events {
                let Some(library_id) = media_request_lifecycle_event_library_id(&event) else {
                    continue;
                };
                let is_manageable = manageable_library_ids.contains(library_id);
                let is_owned_request = requestable_library_ids.contains(library_id)
                    && self
                        .media_request_event_belongs_to_actor(actor, &event)
                        .await?;
                if is_manageable || is_owned_request {
                    visible.push(event);
                    if visible.len() >= target_len {
                        return Ok(visible);
                    }
                }
            }

            if batch_len < 500 {
                break;
            }

            page_filter.after_sequence = next_sequence;
        }

        Ok(visible)
    }

    async fn load_manageable_pending_media_request(
        &self,
        actor: &User,
        request_id: &str,
    ) -> AppResult<MediaRequest> {
        let request_id = request_id.trim();
        if request_id.is_empty() {
            return Err(AppError::Validation("media request id is required".into()));
        }

        let request = self
            .services
            .catalog
            .media_requests
            .get(request_id)
            .await?
            .ok_or_else(|| AppError::NotFound("media request not found".into()))?;
        if request.status != MediaRequestStatus::Pending {
            return Err(AppError::Validation(
                "media request is no longer pending".into(),
            ));
        }

        self.require_library_permission(
            actor,
            &request.library_id,
            LibraryPermission::ManageTitles,
        )
        .await?;
        Ok(request)
    }

    async fn load_requester_pending_media_request(
        &self,
        actor: &User,
        request_id: &str,
    ) -> AppResult<MediaRequest> {
        let request_id = request_id.trim();
        if request_id.is_empty() {
            return Err(AppError::Validation("media request id is required".into()));
        }

        let request = self
            .services
            .catalog
            .media_requests
            .get(request_id)
            .await?
            .ok_or_else(|| AppError::NotFound("media request not found".into()))?;

        self.require_library_permission(actor, &request.library_id, LibraryPermission::Request)
            .await?;

        if !request
            .requesters
            .iter()
            .any(|requester| requester.user_id == actor.id)
        {
            return Err(AppError::Unauthorized(
                "You do not own this media request".to_string(),
            ));
        }

        if request.status != MediaRequestStatus::Pending {
            return Err(AppError::Validation(
                "media request is no longer pending".into(),
            ));
        }

        Ok(request)
    }

    async fn media_request_event_belongs_to_actor(
        &self,
        actor: &User,
        event: &DomainEvent,
    ) -> AppResult<bool> {
        let Some(request_id) = media_request_lifecycle_event_request_id(event) else {
            return Ok(false);
        };
        let Some(request) = self.services.catalog.media_requests.get(request_id).await? else {
            return Ok(false);
        };
        Ok(request
            .requesters
            .iter()
            .any(|requester| requester.user_id == actor.id))
    }

    async fn ensure_request_subject_is_not_in_library(
        &self,
        library_id: &str,
        facet: MediaFacet,
        external_ids: &[ExternalId],
    ) -> AppResult<()> {
        for (source, values) in group_external_id_values_by_source(external_ids) {
            let existing = self
                .services
                .catalog
                .titles
                .list_existing_external_ids_in_library_and_facet(
                    library_id,
                    facet.clone(),
                    &source,
                    &values,
                )
                .await?;
            if !existing.is_empty() {
                return Err(AppError::Validation(
                    "title already exists in the target library".into(),
                ));
            }
        }
        Ok(())
    }

    async fn request_quality_profile_snapshot_for_submission(
        &self,
        library: &Library,
        requested_quality_profile_id: Option<String>,
    ) -> AppResult<(String, String)> {
        let settings = self
            .effective_request_quality_profile_settings_for_library(library)
            .await?;
        let requested_quality_profile_id = normalized_optional_string(requested_quality_profile_id)
            .unwrap_or_else(|| settings.default_profile_id.clone());
        if !settings
            .profile_ids
            .iter()
            .any(|profile_id| profile_id == &requested_quality_profile_id)
        {
            return Err(AppError::Validation(
                "requested quality profile is not allowed for this library".into(),
            ));
        }

        self.quality_profile_snapshot(&requested_quality_profile_id)
            .await
    }

    async fn quality_profile_snapshot(
        &self,
        quality_profile_id: &str,
    ) -> AppResult<(String, String)> {
        let quality_profile_id =
            normalized_optional_string(Some(quality_profile_id.to_string()))
                .ok_or_else(|| AppError::Validation("quality profile id is required".into()))?;
        let profile_settings = self.load_quality_profile_settings().await?;
        let profile = crate::settings::runtime::quality_profile_by_id(
            &profile_settings.profiles,
            &quality_profile_id,
        )?
        .ok_or_else(|| {
            AppError::Validation(format!("unknown quality profile {quality_profile_id}"))
        })?;
        Ok((profile.id.clone(), profile.name.clone()))
    }
}

pub(crate) fn normalize_media_request_external_ids(
    external_ids: Vec<ExternalId>,
) -> AppResult<Vec<ExternalId>> {
    let mut seen = BTreeSet::new();
    for external_id in external_ids {
        let source = external_id.source.trim().to_ascii_lowercase();
        let value = external_id.value.trim().to_string();
        if source.is_empty() || value.is_empty() {
            continue;
        }
        seen.insert((source, value));
    }

    Ok(seen
        .into_iter()
        .map(|(source, value)| ExternalId { source, value })
        .collect())
}

fn media_request_identity_fingerprint(external_ids: &[ExternalId]) -> String {
    crate::helpers::blake3_identity_hex(
        crate::helpers::HashDomain::MediaRequestIdentity,
        external_ids
            .iter()
            .map(|external_id| format!("{}:{}", external_id.source, external_id.value))
            .collect::<Vec<_>>()
            .join("|"),
    )
}

fn media_request_lifecycle_event_types() -> Vec<DomainEventType> {
    vec![
        DomainEventType::MediaRequestSubmitted,
        DomainEventType::MediaRequestUpdated,
        DomainEventType::MediaRequestApproved,
        DomainEventType::MediaRequestRejected,
        DomainEventType::MediaRequestCanceled,
    ]
}

fn media_request_lifecycle_event_request_id(event: &DomainEvent) -> Option<&str> {
    match &event.payload {
        DomainEventPayload::MediaRequestSubmitted(data)
        | DomainEventPayload::MediaRequestUpdated(data) => Some(data.request_id.as_str()),
        DomainEventPayload::MediaRequestApproved(data)
        | DomainEventPayload::MediaRequestRejected(data)
        | DomainEventPayload::MediaRequestCanceled(data) => Some(data.request_id.as_str()),
        _ => None,
    }
}

fn media_request_lifecycle_event_library_id(event: &DomainEvent) -> Option<&str> {
    match &event.payload {
        DomainEventPayload::MediaRequestSubmitted(data)
        | DomainEventPayload::MediaRequestUpdated(data) => Some(data.library_id.as_str()),
        DomainEventPayload::MediaRequestApproved(data)
        | DomainEventPayload::MediaRequestRejected(data)
        | DomainEventPayload::MediaRequestCanceled(data) => Some(data.library_id.as_str()),
        _ => None,
    }
}

fn group_external_id_values_by_source(external_ids: &[ExternalId]) -> Vec<(String, Vec<String>)> {
    let mut grouped = std::collections::BTreeMap::<String, Vec<String>>::new();
    for external_id in external_ids {
        grouped
            .entry(external_id.source.clone())
            .or_default()
            .push(external_id.value.clone());
    }
    grouped.into_iter().collect()
}

fn is_smg_request_correlation_external_id(external_id: &ExternalId) -> bool {
    matches!(external_id.source.as_str(), "tvdb" | "imdb" | "tmdb")
}

fn movie_title_ref_from_external_ids(external_ids: &[ExternalId]) -> Option<crate::MovieTitleRef> {
    let external_id = |source: &str| {
        external_ids
            .iter()
            .find(|external_id| external_id.source.eq_ignore_ascii_case(source))
            .map(|external_id| external_id.value.trim())
            .filter(|value| !value.is_empty())
    };
    let movie_ref = crate::MovieTitleRef {
        smg_id: external_id("smg").and_then(|value| value.parse().ok()),
        tvdb_id: external_id("tvdb").and_then(|value| value.parse().ok()),
        tmdb_id: external_id("tmdb").and_then(|value| value.parse().ok()),
        imdb_id: external_id("imdb").map(str::to_string),
    };
    (movie_ref.smg_id.is_some()
        || movie_ref.tvdb_id.is_some()
        || movie_ref.tmdb_id.is_some()
        || movie_ref.imdb_id.is_some())
    .then_some(movie_ref)
}

struct MediaRequestMetadataEnrichment {
    external_ids: Vec<ExternalId>,
    poster_url: Option<String>,
    overview: Option<String>,
    rating_summary: TitleRatingSummary,
}

impl AppUseCase {
    async fn enrich_media_request_metadata(
        &self,
        facet: &MediaFacet,
        external_ids: Vec<ExternalId>,
    ) -> MediaRequestMetadataEnrichment {
        let language = self.metadata_language().await;
        let result = match facet {
            MediaFacet::Movie => {
                let Some(movie_ref) = movie_title_ref_from_external_ids(&external_ids) else {
                    return MediaRequestMetadataEnrichment {
                        external_ids,
                        poster_url: None,
                        overview: None,
                        rating_summary: TitleRatingSummary::default(),
                    };
                };
                match self
                    .services
                    .library
                    .metadata_gateway
                    .get_movie_titles(std::slice::from_ref(&movie_ref), &language)
                    .await
                {
                    Ok(result) => result
                        .by_ref_index
                        .get(&0)
                        .cloned()
                        .map(|movie| {
                            crate::catalog::facets::handler::movie_to_hydration_result(
                                movie, &language,
                            )
                        })
                        .ok_or_else(|| {
                            AppError::NotFound("movie metadata response missing title".to_string())
                        }),
                    Err(error)
                        if crate::catalog_workflow::movie_title_queries_not_supported(&error) =>
                    {
                        let Some(tvdb_id) = movie_ref.tvdb_id else {
                            return MediaRequestMetadataEnrichment {
                                external_ids,
                                poster_url: None,
                                overview: None,
                                rating_summary: TitleRatingSummary::default(),
                            };
                        };
                        self.services
                            .library
                            .metadata_gateway
                            .get_movie(tvdb_id, &language)
                            .await
                            .map(|movie| {
                                crate::catalog::facets::handler::movie_to_hydration_result(
                                    movie, &language,
                                )
                            })
                    }
                    Err(error) => Err(error),
                }
            }
            MediaFacet::Series | MediaFacet::Anime => {
                let Some(tvdb_id) = external_ids
                    .iter()
                    .find(|external_id| external_id.source == "tvdb")
                    .and_then(|external_id| external_id.value.trim().parse::<i64>().ok())
                else {
                    return MediaRequestMetadataEnrichment {
                        external_ids,
                        poster_url: None,
                        overview: None,
                        rating_summary: TitleRatingSummary::default(),
                    };
                };
                let Some(handler) = self.facet_registry.get(facet) else {
                    tracing::warn!(
                        tvdb_id,
                        facet = facet.as_str(),
                        "failed to enrich media request external IDs because facet handler is missing"
                    );
                    return MediaRequestMetadataEnrichment {
                        external_ids,
                        poster_url: None,
                        overview: None,
                        rating_summary: TitleRatingSummary::default(),
                    };
                };
                handler
                    .hydrate_metadata(
                        self.services.library.metadata_gateway.as_ref(),
                        tvdb_id,
                        &language,
                    )
                    .await
            }
        };
        match result {
            Ok(result) => {
                let overview = normalized_optional_string(result.metadata_update.overview.clone());
                let poster_url =
                    normalized_optional_string(result.metadata_update.poster_url.clone());
                let rating_summary = result.metadata_update.ratings.clone().unwrap_or_default();
                let enriched =
                    crate::catalog::facets::handler::external_ids_from_hydration_metadata(
                        external_ids.clone(),
                        &result.metadata_update,
                    );
                MediaRequestMetadataEnrichment {
                    external_ids: normalize_media_request_external_ids(enriched)
                        .unwrap_or(external_ids),
                    poster_url,
                    overview,
                    rating_summary,
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    facet = facet.as_str(),
                    "failed to enrich media request external IDs from hydrated metadata"
                );
                MediaRequestMetadataEnrichment {
                    external_ids,
                    poster_url: None,
                    overview: None,
                    rating_summary: TitleRatingSummary::default(),
                }
            }
        }
    }
}

fn media_request_resolved_event_data(
    request: &MediaRequest,
    created_title_id: Option<String>,
    approved_quality_profile_id: Option<String>,
    approved_quality_profile_name: Option<String>,
) -> MediaRequestResolvedEventData {
    MediaRequestResolvedEventData {
        request_id: request.id.clone(),
        library_id: request.library_id.clone(),
        facet: request.facet.clone(),
        title_name: request.title.clone(),
        external_ids: request.external_ids.clone(),
        created_title_id,
        requested_quality_profile_id: request.requested_quality_profile_id.clone(),
        requested_quality_profile_name: request.requested_quality_profile_name.clone(),
        requested_monitor_type: request.requested_monitor_type.clone(),
        approved_quality_profile_id,
        approved_quality_profile_name,
    }
}

fn media_request_submitted_event_data(
    request: &MediaRequest,
    requested_quality_profile_id: Option<String>,
    requested_quality_profile_name: Option<String>,
    requested_monitor_type: Option<String>,
) -> MediaRequestSubmittedEventData {
    MediaRequestSubmittedEventData {
        request_id: request.id.clone(),
        library_id: request.library_id.clone(),
        facet: request.facet.clone(),
        title_name: request.title.clone(),
        external_ids: request.external_ids.clone(),
        poster_url: request.poster_url.clone(),
        year: request.year,
        requested_quality_profile_id,
        requested_quality_profile_name,
        requested_monitor_type,
    }
}

fn media_request_to_new_title(
    request: &MediaRequest,
    quality_profile_id: Option<&str>,
    monitor_type: Option<&str>,
) -> NewTitle {
    let monitored = monitor_type.map(monitor_type_to_monitored).unwrap_or(true);
    let mut tags = quality_profile_id
        .map(|profile_id| format!("{TITLE_QUALITY_PROFILE_TAG_PREFIX}{profile_id}"))
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(monitor_type) = monitor_type {
        tags.push(format!("{TITLE_MONITOR_TYPE_TAG_PREFIX}{monitor_type}"));
    }

    NewTitle {
        name: request.title.clone(),
        facet: request.facet.clone(),
        monitored,
        tags,
        external_ids: request.external_ids.clone(),
        root_folder_id: None,
        min_availability: None,
        poster_url: request.poster_url.clone(),
        year: request.year,
        overview: request.overview.clone(),
        sort_title: request.sort_title.clone(),
        slug: request.slug.clone(),
        runtime_minutes: request.runtime_minutes,
        language: request.language.clone(),
        content_status: request.content_status.clone(),
    }
}

fn normalize_requested_monitor_type(
    facet: &MediaFacet,
    value: Option<String>,
) -> AppResult<Option<String>> {
    let Some(value) = normalized_optional_string(value) else {
        return Ok(match facet {
            MediaFacet::Movie => None,
            MediaFacet::Series | MediaFacet::Anime => Some("futureepisodes".to_string()),
        });
    };
    let normalized = value
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-' && *ch != '_')
        .collect::<String>()
        .to_ascii_lowercase();
    match normalized.as_str() {
        "monitored"
        | "unmonitored"
        | "futureepisodes"
        | "missingandfutureepisodes"
        | "allepisodes"
        | "none" => Ok(Some(normalized)),
        _ => Err(AppError::Validation(format!(
            "unsupported request monitor type {value}"
        ))),
    }
}

fn monitor_type_to_monitored(value: &str) -> bool {
    !matches!(value, "none" | "unmonitored")
}

fn normalized_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
