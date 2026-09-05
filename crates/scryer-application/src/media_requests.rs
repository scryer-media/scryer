pub mod snapshot;

use super::*;
use crate::domain_events::new_global_domain_event;
use crate::ports::MediaRequestResolution;
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventPayload, DomainEventType, LibraryPermission,
    LifecycleClaim, LifecycleClaimKind, LifecycleClaimProducer, LifecycleClaimState,
    MONITOR_TYPE_ADVANCED, MediaRequestResolvedEventData, MediaRequestStatus,
    MediaRequestSubmittedEventData, MonitorSelection, RequestDecisionOutcome,
};
use snapshot::{MediaRequestMetadataSnapshot, MediaRequestMetadataSnapshotExt};
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
    pub rating_summary: TitleRatingSummary,
    pub requested_quality_profile_id: Option<String>,
    pub requested_monitor_type: Option<String>,
    /// Season/series-movie picks; only meaningful with the `advanced` monitor type.
    pub requested_monitor_selection: Option<MonitorSelection>,
    /// How long the requester wants the media kept, in days. `None` is forever
    /// (spec 0003 FR-040).
    pub requested_lease_days: Option<i64>,
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
    /// Set when the title was created and the request resolved, but the
    /// retention claim could not be written (spec 0003 §4.5).
    ///
    /// The approval is **not** rolled back: the requester has their title, and
    /// undoing an approval because a bookkeeping row failed would be a far
    /// worse outcome than a title nothing holds. The caller surfaces this so an
    /// operator can re-pin it by hand.
    pub claim_error: Option<String>,
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
    pub requested_monitor_selection: Option<MonitorSelection>,
    /// The lease the requester now wants; `None` is forever. Always written —
    /// the edit form carries the current value, so there is no "leave it alone".
    pub requested_lease_days: Option<i64>,
}

/// Where a resolution's policy provenance comes from, carried from the
/// evaluation to the row, the event, and the claim in one piece so the four can
/// never disagree (spec 0003 FR-016).
#[derive(Clone, Debug, Default)]
pub(crate) struct RequestDecisionProvenance {
    pub(crate) decision_id: Option<String>,
    pub(crate) decided_by_rule_set_ids: Vec<String>,
    /// Tags the policy emitted. Applied to the created title only on an
    /// approval; a denial keeps them in the trace and the event alone (FR-050).
    pub(crate) policy_tags: Vec<String>,
    pub(crate) reason_codes: Vec<String>,
}

impl RequestDecisionProvenance {
    fn from_evaluation(evaluation: &crate::request_rules::RequestEvaluation) -> Self {
        Self {
            decision_id: evaluation.decision_id.clone(),
            decided_by_rule_set_ids: evaluation.deciding_rule_set_ids.clone(),
            policy_tags: evaluation.tags.clone(),
            reason_codes: evaluation
                .reasons
                .iter()
                .map(|reason| reason.code.clone())
                .collect(),
        }
    }
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
        let metadata_enrichment = self.enrich_request_draft(&input.facet, external_ids).await;
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
        let requested_monitor_selection = normalize_requested_monitor_selection(
            requested_monitor_type.as_deref(),
            input.requested_monitor_selection,
        )?;
        let requested_lease_days =
            crate::request_rules::validate_lease_days(input.requested_lease_days)?;
        let poster_url = metadata_enrichment.poster_url;
        let background_url = metadata_enrichment.background_url;
        // Kept, not consumed: the same snapshot that is persisted on the row is
        // what the rules are evaluated against, so the trace and the stored
        // facts can never describe different metadata.
        let metadata_snapshot = metadata_enrichment.snapshot;
        let metadata_snapshot_json = metadata_snapshot.to_json();
        let overview = metadata_enrichment
            .overview
            .or_else(|| normalized_optional_string(input.overview));
        let rating_summary = metadata_enrichment.rating_summary;
        let rating_summary =
            if rating_summary.rating.is_some() || !rating_summary.external_ratings.is_empty() {
                rating_summary
            } else {
                input.rating_summary
            };

        let request = NewMediaRequest {
            id: Id::new().0,
            library_id: library.id.clone(),
            facet: input.facet,
            identity_fingerprint: media_request_identity_fingerprint(&external_ids),
            title,
            sort_title: normalized_optional_string(input.sort_title),
            slug: normalized_optional_string(input.slug),
            poster_url,
            background_url,
            year: input.year,
            overview,
            runtime_minutes: input.runtime_minutes,
            language: normalized_optional_string(input.language),
            content_status: normalized_optional_string(input.content_status),
            rating_summary,
            requested_quality_profile_id: Some(requested_quality_profile_id),
            requested_quality_profile_name: Some(requested_quality_profile_name),
            requested_monitor_type,
            requested_monitor_selection,
            requested_lease_days,
            metadata_snapshot_json,
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
                requested_lease_days: request.requested_lease_days,
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

        // The bare Auto-Approve permission check that used to live here is now
        // *inside* the evaluation: with no rules, an unreadable gate, or any
        // failure, `effective_outcome` is exactly what that check produced
        // (spec 0003 FR-011, FR-012).
        let evaluation = self
            .evaluate_request_draft(
                actor,
                &library,
                &request_draft_from_media_request(&submitted_request),
                &metadata_snapshot,
                crate::request_rules::RequestEvaluationPurpose::Submit {
                    request_id: request_id.clone(),
                },
            )
            .await?;
        self.stamp_request_decision(&request_id, &evaluation).await;
        self.act_on_request_decision(actor, submitted_request, &evaluation)
            .await?;

        Ok(SubmitMediaRequestOutcome { request_id })
    }

    /// Write the verdict's provenance onto a request row that is still pending.
    ///
    /// Best effort: the request exists and the requester has been told so, and a
    /// store that refuses the stamp must not turn a successful submission into
    /// an error. The trace itself is already written.
    async fn stamp_request_decision(
        &self,
        request_id: &str,
        evaluation: &crate::request_rules::RequestEvaluation,
    ) {
        if let Err(error) = self
            .services
            .catalog
            .media_requests
            .record_decision_on_request(
                request_id,
                evaluation.decision_id.as_deref(),
                &evaluation.deciding_rule_set_ids,
                &evaluation.tags,
            )
            .await
        {
            tracing::warn!(
                request_id,
                error = %error,
                "could not stamp the request rule verdict onto the request"
            );
        }
    }

    /// Do what the verdict says. `ManualReview` deliberately does nothing: the
    /// request stays pending and waits for a person.
    async fn act_on_request_decision(
        &self,
        actor: &User,
        request: MediaRequest,
        evaluation: &crate::request_rules::RequestEvaluation,
    ) -> AppResult<()> {
        let provenance = RequestDecisionProvenance::from_evaluation(evaluation);
        match evaluation.effective_outcome {
            RequestDecisionOutcome::AutoApprove => {
                self.auto_approve_submitted_media_request(actor, request, provenance)
                    .await
            }
            RequestDecisionOutcome::Deny => {
                self.deny_submitted_media_request(&request, provenance)
                    .await
            }
            RequestDecisionOutcome::ManualReview => Ok(()),
        }
    }

    pub async fn list_media_requests(
        &self,
        actor: &User,
        input: ListMediaRequestsInput,
    ) -> AppResult<Vec<MediaRequest>> {
        let allowed_ids = self
            .authorized_library_ids(actor, input.facet.clone(), LibraryPermission::ManageTitles)
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
            .authorized_library_ids(actor, input.facet.clone(), LibraryPermission::Request)
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

    /// Approve a pending request by hand.
    ///
    /// `lease_days` is the approver's override — `Some(Some(n))` for a finite
    /// window, `Some(None)` for forever, `None` to grant exactly what the
    /// requester asked for. `tags` likewise replaces the policy's tags outright
    /// rather than adding to them: an approver who edits the tag list means the
    /// list they typed (spec 0003 FR-050).
    // Eight parameters because each one is a separate thing the approver can
    // decide, and two of them are nested options whose meaning is exactly the
    // distinction between "not said" and "said nothing"; a struct would flatten
    // that back into a field nobody can read.
    #[allow(clippy::too_many_arguments)]
    pub async fn approve_media_request(
        &self,
        actor: &User,
        request_id: &str,
        quality_profile_id: &str,
        monitor_type: Option<String>,
        monitor_selection: Option<MonitorSelection>,
        lease_days: Option<Option<i64>>,
        tags: Option<Vec<String>>,
    ) -> AppResult<ApproveMediaRequestOutcome> {
        let request = self
            .load_manageable_pending_media_request(actor, request_id)
            .await?;
        let approved_lease_days = match lease_days {
            Some(override_days) => crate::request_rules::validate_lease_days(override_days)?,
            None => crate::request_rules::validate_lease_days(request.requested_lease_days)?,
        };
        let approved_tags = match tags {
            Some(tags) => crate::request_rules::validate_tag_list(tags)?,
            None => request.policy_tags.clone(),
        };
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
        // An approver override replaces the requester's picks; without one the
        // request's stored selection is what gets applied.
        let approved_monitor_selection = normalize_requested_monitor_selection(
            approved_monitor_type.as_deref(),
            monitor_selection.or_else(|| request.requested_monitor_selection.clone()),
        )?;
        let outcome = self
            .add_title_with_options_patch_outcome_after_library_authorization_profile_lock_held(
                actor,
                media_request_to_new_title(
                    &request,
                    Some(&approved_quality_profile_id),
                    approved_monitor_type.as_deref(),
                    &approved_tags,
                ),
                request.library_id.clone(),
                TitleOptionsPatch {
                    monitor_selection: Some(approved_monitor_selection),
                    ..TitleOptionsPatch::default()
                },
            )
            .await?;
        let provenance = RequestDecisionProvenance {
            decision_id: request.decision_id.clone(),
            decided_by_rule_set_ids: request.decided_by_rule_set_ids.clone(),
            policy_tags: approved_tags.clone(),
            reason_codes: Vec::new(),
        };
        let mut event_data = media_request_resolved_event_data(
            &request,
            Some(outcome.title.id.clone()),
            Some(approved_quality_profile_id.clone()),
            Some(approved_quality_profile_name.clone()),
        );
        apply_decision_provenance(&mut event_data, &provenance, approved_lease_days);
        let resolved_event =
            new_global_domain_event(actor, DomainEventPayload::MediaRequestApproved(event_data));
        let resolution = self
            .services
            .catalog
            .media_requests
            .resolve_pending_overlapping(
                &request,
                MediaRequestResolution {
                    status: MediaRequestStatus::Approved,
                    resolved_by_user_id: Some(actor.id.clone()),
                    resolved_at: chrono::Utc::now(),
                    created_title_id: Some(outcome.title.id.clone()),
                    approved_quality_profile_id: Some(approved_quality_profile_id),
                    approved_quality_profile_name: Some(approved_quality_profile_name),
                    approved_lease_days,
                    decision_id: provenance.decision_id.clone(),
                    decided_by_rule_set_ids: provenance.decided_by_rule_set_ids.clone(),
                    policy_tags: provenance.policy_tags.clone(),
                    event: resolved_event,
                },
            )
            .await?;
        drop(profile_reference_guard);
        if let Some(event) = &resolution.event {
            self.publish_stored_domain_event(event).await;
        }
        let title_id = outcome.title.id.clone();
        let claim_error = self
            .create_request_lifecycle_claims(actor, &request, &title_id, approved_lease_days)
            .await;
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
                    claim_error,
                });
            }
        };

        Ok(ApproveMediaRequestOutcome {
            title_id,
            wanted_search,
            search_error: None,
            claim_error,
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
                    resolved_by_user_id: Some(actor.id.clone()),
                    resolved_at: chrono::Utc::now(),
                    created_title_id: None,
                    approved_quality_profile_id: None,
                    approved_quality_profile_name: None,
                    // A dismissal grants nothing, so there is no lease; the
                    // policy provenance already on the row rides along.
                    approved_lease_days: None,
                    decision_id: request.decision_id.clone(),
                    decided_by_rule_set_ids: request.decided_by_rule_set_ids.clone(),
                    policy_tags: request.policy_tags.clone(),
                    event: resolved_event,
                },
            )
            .await?;
        if let Some(event) = &resolution.event {
            self.publish_stored_domain_event(event).await;
        }
        // A dismissed request holds nothing. Releasing is best effort and
        // idempotent: a request that never reached approval has no claim, and
        // the repository answers zero.
        self.release_request_lifecycle_claims(&request.id, CLAIM_RELEASE_REQUEST_REJECTED)
            .await;
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
        let requested_monitor_selection = normalize_requested_monitor_selection(
            requested_monitor_type.as_deref(),
            input.requested_monitor_selection,
        )?;
        let requested_lease_days =
            crate::request_rules::validate_lease_days(input.requested_lease_days)?;
        let mut submitted_event_data = media_request_submitted_event_data(
            &request,
            Some(requested_quality_profile_id.clone()),
            Some(requested_quality_profile_name.clone()),
            requested_monitor_type.clone(),
        );
        submitted_event_data.requested_lease_days = requested_lease_days;
        let updated_event = new_global_domain_event(
            actor,
            DomainEventPayload::MediaRequestUpdated(submitted_event_data),
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
                requested_monitor_selection,
                requested_lease_days,
                updated_event,
            )
            .await?;
        drop(profile_reference_guard);
        self.publish_stored_domain_event(&update.event).await;

        // Re-judged against the edit (spec 0003 FR-011): a pending request that
        // now matches an approve rule is approved, one that now matches a deny
        // rule is denied, and one that matches neither keeps waiting. The
        // *stored* snapshot is reused rather than re-enriched — an edit changes
        // the profile, the monitor, and the lease, never the subject — so the
        // edit cannot silently be judged against different metadata than the
        // submission was.
        let updated_request = update.request.clone();
        let snapshot = updated_request.metadata_snapshot();
        let evaluation = self
            .evaluate_request_draft(
                actor,
                &library,
                &request_draft_from_media_request(&updated_request),
                &snapshot,
                crate::request_rules::RequestEvaluationPurpose::Resubmit {
                    request_id: updated_request.id.clone(),
                },
            )
            .await?;
        self.stamp_request_decision(&updated_request.id, &evaluation)
            .await;
        self.act_on_request_decision(actor, updated_request, &evaluation)
            .await?;

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
                    resolved_by_user_id: Some(actor.id.clone()),
                    resolved_at: canceled_at,
                    created_title_id: None,
                    approved_quality_profile_id: None,
                    approved_quality_profile_name: None,
                    // A cancellation grants nothing, so there is no lease; the
                    // policy provenance already on the row rides along.
                    approved_lease_days: None,
                    decision_id: request.decision_id.clone(),
                    decided_by_rule_set_ids: request.decided_by_rule_set_ids.clone(),
                    policy_tags: request.policy_tags.clone(),
                    event: canceled_event,
                },
            )
            .await?;
        if let Some(event) = &resolution.event {
            self.publish_stored_domain_event(event).await;
        }
        self.release_request_lifecycle_claims(&request.id, CLAIM_RELEASE_REQUEST_CANCELED)
            .await;
        Ok(resolution.updated)
    }

    async fn auto_approve_submitted_media_request(
        &self,
        actor: &User,
        request: MediaRequest,
        provenance: RequestDecisionProvenance,
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
        let approved_monitor_selection = normalize_requested_monitor_selection(
            approved_monitor_type.as_deref(),
            request.requested_monitor_selection.clone(),
        )?;
        // The requester's own lease is what a policy approval grants: nobody
        // overrode it, so `approved` and `requested` are the same window.
        let approved_lease_days = request.requested_lease_days;
        let outcome = self
            .add_title_with_options_patch_outcome_after_library_authorization(
                actor,
                media_request_to_new_title(
                    &request,
                    Some(&approved_quality_profile_id),
                    approved_monitor_type.as_deref(),
                    &provenance.policy_tags,
                ),
                request.library_id.clone(),
                TitleOptionsPatch {
                    monitor_selection: Some(approved_monitor_selection),
                    ..TitleOptionsPatch::default()
                },
            )
            .await?;
        let mut event_data = media_request_resolved_event_data(
            &request,
            Some(outcome.title.id.clone()),
            Some(approved_quality_profile_id.clone()),
            Some(approved_quality_profile_name.clone()),
        );
        apply_decision_provenance(&mut event_data, &provenance, approved_lease_days);
        let resolved_event =
            new_global_domain_event(actor, DomainEventPayload::MediaRequestApproved(event_data));
        let resolution = self
            .services
            .catalog
            .media_requests
            .resolve_pending_overlapping(
                &request,
                MediaRequestResolution {
                    status: MediaRequestStatus::Approved,
                    resolved_by_user_id: Some(actor.id.clone()),
                    resolved_at: chrono::Utc::now(),
                    created_title_id: Some(outcome.title.id.clone()),
                    approved_quality_profile_id: Some(approved_quality_profile_id),
                    approved_quality_profile_name: Some(approved_quality_profile_name),
                    approved_lease_days,
                    decision_id: provenance.decision_id.clone(),
                    decided_by_rule_set_ids: provenance.decided_by_rule_set_ids.clone(),
                    policy_tags: provenance.policy_tags.clone(),
                    event: resolved_event,
                },
            )
            .await?;
        if let Some(event) = &resolution.event {
            self.publish_stored_domain_event(event).await;
        }
        // A claim-store failure never rolls the approval back; it is logged and
        // reported (spec 0003 §4.5). There is no outcome to carry it on here, so
        // the log is the only signal — the human approval path returns it.
        self.create_request_lifecycle_claims(
            actor,
            &request,
            &outcome.title.id,
            approved_lease_days,
        )
        .await;

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

    /// Refuse a request because a rule said to (spec 0003 §4.4).
    ///
    /// `resolved_by_user_id` is `None`: nobody decided this, and stamping the
    /// requester — the only `User` in scope — would render in the UI as "you
    /// rejected your own request". No title is created, and the policy's tags
    /// stay in the trace and the event without ever reaching a title (FR-050).
    async fn deny_submitted_media_request(
        &self,
        request: &MediaRequest,
        provenance: RequestDecisionProvenance,
    ) -> AppResult<()> {
        let mut event_data = media_request_resolved_event_data(request, None, None, None);
        apply_decision_provenance(&mut event_data, &provenance, None);
        // System-authored: the event carries the deciding rules and their
        // reason codes instead of a person.
        let resolved_event = new_global_domain_event(
            crate::domain_events::DomainEventActor::system(),
            DomainEventPayload::MediaRequestRejected(event_data),
        );
        let resolution = self
            .services
            .catalog
            .media_requests
            .resolve_pending_overlapping(
                request,
                MediaRequestResolution {
                    status: MediaRequestStatus::Rejected,
                    resolved_by_user_id: None,
                    resolved_at: chrono::Utc::now(),
                    created_title_id: None,
                    approved_quality_profile_id: None,
                    approved_quality_profile_name: None,
                    approved_lease_days: None,
                    decision_id: provenance.decision_id.clone(),
                    decided_by_rule_set_ids: provenance.decided_by_rule_set_ids.clone(),
                    policy_tags: provenance.policy_tags.clone(),
                    event: resolved_event,
                },
            )
            .await?;
        if let Some(event) = &resolution.event {
            self.publish_stored_domain_event(event).await;
        }
        Ok(())
    }

    /// Write one retention claim per request the approval resolved.
    ///
    /// Every overlapping pending request the resolution swept up is a person
    /// who asked for this title, and each of them asked for their own window —
    /// so each gets its own claim keyed to its own request id. The approver's
    /// override applies to the request they actually approved; the others keep
    /// what they asked for.
    ///
    /// A finite lease is created **dormant**: the window measures how long the
    /// requester keeps the media, and it starts at the title's first import,
    /// not at the approval (WP6a's activation). A forever request is a keep,
    /// created active, because there is no clock to start.
    ///
    /// Returns a message when something failed. The approval is never rolled
    /// back (spec 0003 §4.5).
    async fn create_request_lifecycle_claims(
        &self,
        actor: &User,
        request: &MediaRequest,
        title_id: &str,
        approved_lease_days: Option<i64>,
    ) -> Option<String> {
        let mut errors: Vec<String> = Vec::new();
        let resolved = self
            .resolved_requests_for_title(request, title_id)
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("could not read the resolved requests: {error}"));
                Vec::new()
            });
        // Even if the read-back failed, the request that was approved is known.
        let mut targets: Vec<(String, Option<i64>)> =
            vec![(request.id.clone(), approved_lease_days)];
        for other in resolved {
            if other.id == request.id || targets.iter().any(|(id, _)| id == &other.id) {
                continue;
            }
            targets.push((other.id.clone(), other.requested_lease_days));
        }

        let now = chrono::Utc::now();
        for (request_id, lease_days) in targets {
            let claim = new_request_lifecycle_claim(
                &request_id,
                title_id,
                &request.library_id,
                lease_days,
                &actor.id,
                now,
            );
            if let Err(error) = self.services.catalog.lifecycle_claims.create(&claim).await {
                tracing::error!(
                    request_id = request_id.as_str(),
                    title_id,
                    error = %error,
                    "approved a media request but could not write its retention claim"
                );
                errors.push(format!("request {request_id}: {error}"));
            }
        }

        (!errors.is_empty()).then(|| errors.join("; "))
    }

    /// The requests this approval resolved onto `title_id`, read back through
    /// the identity fingerprint. `resolve_pending_overlapping` reports only a
    /// row count, and each resolved row stamps `created_title_id`, so the
    /// fingerprint history filtered on that id is exactly the set.
    async fn resolved_requests_for_title(
        &self,
        request: &MediaRequest,
        title_id: &str,
    ) -> AppResult<Vec<MediaRequest>> {
        Ok(self
            .services
            .catalog
            .media_requests
            .history_for_fingerprint(&request.identity_fingerprint)
            .await?
            .into_iter()
            .filter(|candidate| {
                candidate.created_title_id.as_deref() == Some(title_id)
                    && candidate.status == MediaRequestStatus::Approved
            })
            .collect())
    }

    /// Withdraw whatever a request was holding. Best effort and idempotent: a
    /// request that never reached approval has no claim.
    async fn release_request_lifecycle_claims(&self, request_id: &str, reason: &str) {
        let now = chrono::Utc::now();
        for producer in [
            LifecycleClaimProducer::RequestLease,
            LifecycleClaimProducer::RequestPermanent,
        ] {
            if let Err(error) = self
                .services
                .catalog
                .lifecycle_claims
                .release_for_producer_ref(producer, request_id, reason, now)
                .await
            {
                tracing::warn!(
                    request_id,
                    reason,
                    error = %error,
                    "could not release a request's lifecycle claim"
                );
            }
        }
    }

    // ── Administrator claim operations (spec 0003 FR-044) ───────────────────
    //
    // All four are gated on `ManageTitles` in the claim's *own* library rather
    // than on a global authority: a claim is a hold on one title, and the people
    // who may delete that title are exactly the people who may decide it stays.

    /// Every claim on a title, live and historical.
    pub async fn list_title_claims(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<Vec<LifecycleClaim>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id_without_external_ids(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title not found".into()))?;
        self.require_library_permission(actor, &title.library_id, LibraryPermission::ManageTitles)
            .await?;
        self.services
            .catalog
            .lifecycle_claims
            .list_for_title(title_id)
            .await
    }

    /// Push a live claim's window out to `expires_at`.
    ///
    /// The repository refuses this for a claim that is not live — an expired
    /// lease is not extended, it is replaced — so a stale UI cannot resurrect a
    /// window that already closed.
    pub async fn extend_title_claim(
        &self,
        actor: &User,
        claim_id: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<LifecycleClaim> {
        let claim = self.require_manageable_claim(actor, claim_id).await?;
        let now = chrono::Utc::now();
        self.services
            .catalog
            .lifecycle_claims
            .extend(&claim.id, expires_at, now)
            .await?;
        self.reload_claim(&claim.id).await
    }

    /// Replace a live claim with an operator keep.
    ///
    /// The replacement is a *new* claim with a different producer, not a mutated
    /// one: the original stays as history in the `converted` state, so the trail
    /// still says a request produced the original hold and an administrator
    /// chose to make it permanent.
    pub async fn convert_title_claim_to_permanent(
        &self,
        actor: &User,
        claim_id: &str,
    ) -> AppResult<LifecycleClaim> {
        let claim = self.require_manageable_claim(actor, claim_id).await?;
        let now = chrono::Utc::now();
        let replacement = LifecycleClaim {
            id: Id::new().0,
            title_id: claim.title_id.clone(),
            library_id: claim.library_id.clone(),
            producer: LifecycleClaimProducer::OperatorKeep,
            // No `producer_ref`: an operator pin has nothing upstream to release
            // against, and it is what keeps two pins on one title from
            // colliding on the live-claim unique index.
            producer_ref: None,
            kind: LifecycleClaimKind::Keep,
            state: LifecycleClaimState::Active,
            duration_days: None,
            starts_at: Some(now),
            expires_at: None,
            created_by: Some(actor.id.clone()),
            created_at: now,
            updated_at: now,
            released_reason: None,
        };
        self.services
            .catalog
            .lifecycle_claims
            .convert_to_permanent(&claim.id, &replacement, now)
            .await?;
        self.reload_claim(&replacement.id).await
    }

    /// Withdraw a claim by hand.
    pub async fn release_title_claim(
        &self,
        actor: &User,
        claim_id: &str,
        reason: &str,
    ) -> AppResult<LifecycleClaim> {
        let claim = self.require_manageable_claim(actor, claim_id).await?;
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(AppError::Validation(
                "a release reason is required".to_string(),
            ));
        }
        self.services
            .catalog
            .lifecycle_claims
            .release_claim(&claim.id, reason, chrono::Utc::now())
            .await?;
        self.reload_claim(&claim.id).await
    }

    async fn require_manageable_claim(
        &self,
        actor: &User,
        claim_id: &str,
    ) -> AppResult<LifecycleClaim> {
        let claim = self
            .services
            .catalog
            .lifecycle_claims
            .get(claim_id)
            .await?
            .ok_or_else(|| AppError::NotFound("lifecycle claim not found".into()))?;
        self.require_library_permission(actor, &claim.library_id, LibraryPermission::ManageTitles)
            .await?;
        Ok(claim)
    }

    async fn reload_claim(&self, claim_id: &str) -> AppResult<LifecycleClaim> {
        self.services
            .catalog
            .lifecycle_claims
            .get(claim_id)
            .await?
            .ok_or_else(|| AppError::NotFound("lifecycle claim not found".into()))
    }

    pub async fn pending_media_request_counts(
        &self,
        actor: &User,
    ) -> AppResult<MediaRequestCounts> {
        let library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::ManageTitles)
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
            .authorized_library_ids(actor, None, LibraryPermission::ManageTitles)
            .await?
            .is_empty())
    }

    pub async fn can_access_media_requests(&self, actor: &User) -> AppResult<bool> {
        if self.can_manage_media_requests(actor).await? {
            return Ok(true);
        }

        Ok(!self
            .authorized_library_ids(actor, None, LibraryPermission::Request)
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
            .authorized_library_ids(actor, None, LibraryPermission::ManageTitles)
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
            .authorized_library_ids(actor, None, LibraryPermission::ManageTitles)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let requestable_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::Request)
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

    pub(crate) async fn request_quality_profile_snapshot_for_submission(
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

/// Reason recorded on claims released because their request was canceled.
pub const CLAIM_RELEASE_REQUEST_CANCELED: &str = "request_canceled";
/// Reason recorded on claims released because their request was rejected.
pub const CLAIM_RELEASE_REQUEST_REJECTED: &str = "request_rejected";

/// The evaluation view of a stored request, so submit and edit judge exactly
/// what was written rather than what the caller happened to pass in.
pub(crate) fn request_draft_from_media_request(
    request: &MediaRequest,
) -> crate::request_rules::RequestDraft {
    crate::request_rules::RequestDraft {
        facet: request.facet.clone(),
        title: request.title.clone(),
        year: request.year,
        external_ids: request.external_ids.clone(),
        identity_fingerprint: request.identity_fingerprint.clone(),
        quality_profile_id: request.requested_quality_profile_id.clone(),
        quality_profile_name: request.requested_quality_profile_name.clone(),
        monitor_type: request.requested_monitor_type.clone(),
        monitor_selection: request.requested_monitor_selection.clone(),
        requested_lease_days: request.requested_lease_days,
    }
}

/// A retention claim for one request. Finite ⇒ dormant `retain_until`; forever
/// ⇒ active `keep`.
fn new_request_lifecycle_claim(
    request_id: &str,
    title_id: &str,
    library_id: &str,
    lease_days: Option<i64>,
    created_by: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> LifecycleClaim {
    let (producer, kind, state, starts_at) = match lease_days {
        // Dormant: the window starts at the title's first import, not now.
        Some(_) => (
            LifecycleClaimProducer::RequestLease,
            LifecycleClaimKind::RetainUntil,
            LifecycleClaimState::Dormant,
            None,
        ),
        // A keep has no clock to start, so it is live from the moment it exists.
        None => (
            LifecycleClaimProducer::RequestPermanent,
            LifecycleClaimKind::Keep,
            LifecycleClaimState::Active,
            Some(now),
        ),
    };
    LifecycleClaim {
        id: Id::new().0,
        title_id: title_id.to_string(),
        library_id: library_id.to_string(),
        producer,
        producer_ref: Some(request_id.to_string()),
        kind,
        state,
        duration_days: lease_days,
        starts_at,
        expires_at: None,
        created_by: Some(created_by.to_string()),
        created_at: now,
        updated_at: now,
        released_reason: None,
    }
}

/// Overwrite the provenance fields the stored row cannot yet carry — at submit
/// time the verdict is newer than the row it is about.
fn apply_decision_provenance(
    data: &mut MediaRequestResolvedEventData,
    provenance: &RequestDecisionProvenance,
    approved_lease_days: Option<i64>,
) {
    data.decided_by_rule_set_ids = provenance.decided_by_rule_set_ids.clone();
    data.decision_reason_codes = provenance.reason_codes.clone();
    data.policy_tags = provenance.policy_tags.clone();
    data.approved_lease_days = approved_lease_days;
}

pub(crate) fn media_request_identity_fingerprint(external_ids: &[ExternalId]) -> String {
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

#[derive(Clone, Debug)]
pub(crate) struct MediaRequestMetadataEnrichment {
    pub(crate) external_ids: Vec<ExternalId>,
    pub(crate) poster_url: Option<String>,
    pub(crate) background_url: Option<String>,
    pub(crate) overview: Option<String>,
    pub(crate) rating_summary: TitleRatingSummary,
    /// Every fact the request rule surface can read, captured at enrichment time (FR-030).
    pub(crate) snapshot: MediaRequestMetadataSnapshot,
}

impl MediaRequestMetadataEnrichment {
    /// The enrichment we fall back to when SMG could not be reached or the subject could not be
    /// identified. It keeps today's behaviour for poster/overview/ratings (all absent, so the
    /// caller's own input wins) and says so explicitly in the snapshot.
    fn unavailable(external_ids: Vec<ExternalId>, reason: &str) -> Self {
        let mut snapshot = MediaRequestMetadataSnapshot::unavailable(reason);
        snapshot.captured_at = Some(Utc::now());
        Self {
            external_ids,
            poster_url: None,
            background_url: None,
            overview: None,
            rating_summary: TitleRatingSummary::default(),
            snapshot,
        }
    }
}

/// Cache key for one enrichment: the facet plus the request's normalized external identifiers.
/// Sorted and joined so two drafts naming the same subject in a different order share one entry.
pub(crate) fn media_request_enrichment_cache_key(
    facet: &MediaFacet,
    external_ids: &[ExternalId],
) -> String {
    let mut parts = external_ids
        .iter()
        .map(|external_id| {
            format!(
                "{}={}",
                external_id.source.trim().to_ascii_lowercase(),
                external_id.value.trim()
            )
        })
        .collect::<Vec<_>>();
    parts.sort();
    parts.dedup();
    format!("{}|{}", facet.as_str(), parts.join(","))
}

impl AppUseCase {
    /// Enrich a request draft, reading through the in-process enrichment cache.
    ///
    /// Pre-flight evaluation and the eventual submit both go through here, so a requester who
    /// previews a decision and then submits it is judged against **one** SMG read (FR-021) — and
    /// SMG sees one call, not one per keystroke-debounce.
    pub(crate) async fn enrich_request_draft(
        &self,
        facet: &MediaFacet,
        external_ids: Vec<ExternalId>,
    ) -> MediaRequestMetadataEnrichment {
        let key = media_request_enrichment_cache_key(facet, &external_ids);
        if let Some(cached) = self.runtime.catalog.media_request_enrichment.get(&key) {
            return (*cached).clone();
        }
        let enrichment = self
            .enrich_media_request_metadata(facet, external_ids)
            .await;
        // Failures are cached too, and deliberately: a preview that said "metadata unavailable —
        // will need approval" must not become an auto-approval seconds later just because the
        // submit happened to catch SMG on a good beat. FR-021 is about agreement, not optimism.
        self.runtime
            .catalog
            .media_request_enrichment
            .insert(key, std::sync::Arc::new(enrichment.clone()));
        enrichment
    }

    async fn enrich_media_request_metadata(
        &self,
        facet: &MediaFacet,
        external_ids: Vec<ExternalId>,
    ) -> MediaRequestMetadataEnrichment {
        let language = self.metadata_language().await;
        // The `MovieMetadata` the movie paths already fetched, kept so the snapshot is built from
        // the same read the hydration used instead of asking SMG a second time.
        let mut raw_movie: Option<(MovieMetadata, &'static str)> = None;
        let result = match facet {
            MediaFacet::Movie => {
                let Some(movie_ref) = movie_title_ref_from_external_ids(&external_ids) else {
                    return MediaRequestMetadataEnrichment::unavailable(
                        external_ids,
                        "movie_subject_unidentifiable",
                    );
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
                            raw_movie = Some((movie.clone(), "smg_titles"));
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
                            return MediaRequestMetadataEnrichment::unavailable(
                                external_ids,
                                "movie_subject_unidentifiable",
                            );
                        };
                        self.services
                            .library
                            .metadata_gateway
                            .get_movie(tvdb_id, &language)
                            .await
                            .map(|movie| {
                                raw_movie = Some((movie.clone(), "smg_movie"));
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
                    return MediaRequestMetadataEnrichment::unavailable(
                        external_ids,
                        "series_subject_unidentifiable",
                    );
                };
                let Some(handler) = self.facet_registry.get(facet) else {
                    tracing::warn!(
                        tvdb_id,
                        facet = facet.as_str(),
                        "failed to enrich media request external IDs because facet handler is missing"
                    );
                    return MediaRequestMetadataEnrichment::unavailable(
                        external_ids,
                        "facet_handler_missing",
                    );
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
                let captured_at = Utc::now();
                let overview = normalized_optional_string(result.metadata_update.overview.clone());
                let poster_url =
                    normalized_optional_string(result.metadata_update.poster_url.clone());
                let background_url =
                    normalized_optional_string(result.metadata_update.background_url.clone());
                let rating_summary = result.metadata_update.ratings.clone().unwrap_or_default();
                let snapshot = match (&raw_movie, &result.raw_series) {
                    (Some((movie, source)), _) => {
                        let mut snapshot =
                            MediaRequestMetadataSnapshot::from_movie(movie, captured_at);
                        snapshot.source = Some((*source).to_string());
                        snapshot
                    }
                    (None, Some(series)) => {
                        MediaRequestMetadataSnapshot::from_series(series, captured_at)
                    }
                    // Hydration succeeded but carried no raw metadata for this facet. That is a
                    // wiring fault, not an SMG answer, so it is partial rather than empty.
                    (None, None) => {
                        tracing::warn!(
                            facet = facet.as_str(),
                            "media request hydration returned no raw metadata for the snapshot"
                        );
                        let mut snapshot =
                            MediaRequestMetadataSnapshot::unavailable("raw_metadata_absent");
                        snapshot.captured_at = Some(captured_at);
                        snapshot
                    }
                };
                let enriched =
                    crate::catalog::facets::handler::external_ids_from_hydration_metadata(
                        external_ids.clone(),
                        &result.metadata_update,
                    );
                MediaRequestMetadataEnrichment {
                    external_ids: normalize_media_request_external_ids(enriched)
                        .unwrap_or(external_ids),
                    poster_url,
                    background_url,
                    overview,
                    rating_summary,
                    snapshot,
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    facet = facet.as_str(),
                    "failed to enrich media request external IDs from hydrated metadata"
                );
                MediaRequestMetadataEnrichment::unavailable(external_ids, "enrichment_failed")
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
        decided_by_rule_set_ids: request.decided_by_rule_set_ids.clone(),
        decision_reason_codes: Vec::new(),
        approved_lease_days: request.approved_lease_days,
        policy_tags: request.policy_tags.clone(),
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
        requested_lease_days: request.requested_lease_days,
    }
}

fn media_request_to_new_title(
    request: &MediaRequest,
    quality_profile_id: Option<&str>,
    monitor_type: Option<&str>,
    policy_tags: &[String],
) -> NewTitle {
    let monitored = monitor_type.map(monitor_type_to_monitored).unwrap_or(true);
    let mut tags = quality_profile_id
        .map(|profile_id| format!("{TITLE_QUALITY_PROFILE_TAG_PREFIX}{profile_id}"))
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(monitor_type) = monitor_type {
        tags.push(format!("{TITLE_MONITOR_TYPE_TAG_PREFIX}{monitor_type}"));
    }
    // Policy and approver tags are plain labels beside the structured
    // `scryer:`-prefixed ones. The tag validator refuses that prefix, so a rule
    // can never mint a tag the resolver would read as a quality profile or a
    // monitor type.
    //
    // TODO(title-tags registry): the sibling title-tags workstream has not
    // landed in this worktree (`title_tag_definitions` does not exist yet), so
    // there is nothing to validate these against beyond the family's own
    // bounds. When it lands, both the policy tags and the approver override
    // should be checked against the registry here.
    for tag in policy_tags {
        let tag = tag.trim();
        if !tag.is_empty() && !tags.iter().any(|existing| existing == tag) {
            tags.push(tag.to_string());
        }
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
        MONITOR_TYPE_ADVANCED if *facet == MediaFacet::Movie => Err(AppError::Validation(
            "advanced monitoring is only valid for series and anime titles".into(),
        )),
        "monitored"
        | "unmonitored"
        | "futureepisodes"
        | "missingandfutureepisodes"
        | "allepisodes"
        | "none"
        | MONITOR_TYPE_ADVANCED => Ok(Some(normalized)),
        _ => Err(AppError::Validation(format!(
            "unsupported request monitor type {value}"
        ))),
    }
}

/// Advanced monitoring requires an explicit, non-empty selection; every other
/// monitor type drops whatever selection was supplied.
pub(crate) fn normalize_requested_monitor_selection(
    monitor_type: Option<&str>,
    selection: Option<MonitorSelection>,
) -> AppResult<Option<MonitorSelection>> {
    if monitor_type != Some(MONITOR_TYPE_ADVANCED) {
        return Ok(None);
    }
    let selection = selection
        .map(|selection| selection.normalized())
        .filter(|selection| !selection.is_empty())
        .ok_or_else(|| {
            AppError::Validation(
                "advanced monitoring requires at least one season or series movie".into(),
            )
        })?;
    Ok(Some(selection))
}

fn monitor_type_to_monitored(value: &str) -> bool {
    !matches!(value, "none" | "unmonitored")
}

fn normalized_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
