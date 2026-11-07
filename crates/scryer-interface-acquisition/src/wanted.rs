use async_graphql::{Context, Enum, ID, InputObject, Object, Result as GqlResult, SimpleObject};
use scryer_application::WantedSearchOutcome;

use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::types::*;

#[derive(Default)]
pub struct WantedMutations;

/// What to do about downloads of the same title that are already running.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum StandbyQueueChoiceValue {
    /// Queue only when nothing else is downloading; otherwise list the conflicts.
    Check,
    /// Queue alongside the confirmed conflicts, leaving them running.
    Queue,
    /// Ask the clients to cancel the confirmed conflicts, then queue without
    /// waiting for them to act. Cancellation is best effort: a conflict that is
    /// already post-processing may still finish.
    CancelAndQueue,
}

/// One conflicting download the operator confirmed, as the client names it.
#[derive(InputObject)]
pub struct StandbyConflictIdentityInput {
    /// The configured download client that holds the job, when known.
    pub client_id: Option<ID>,
    /// The download client's type, such as `sabnzbd` or `weaver`.
    pub client_type: String,
    /// The client's own id for the job.
    pub item_id: String,
}

/// The result of asking to queue a saved standby candidate.
#[derive(SimpleObject)]
pub struct QueueStandbyReleasePayload {
    /// Whether the release is now queued (or already was).
    pub grabbed: bool,
    /// Running downloads of this title that still need the operator's choice.
    pub conflicts: Vec<QueueDownloadConflictPayload>,
    /// Conflicts whose client accepted the cancel request. Scryer stops
    /// tracking them; a client may still finish a job it was post-processing.
    pub cancelled: Vec<QueueDownloadConflictPayload>,
    /// Conflicts whose cancel request failed. They keep running and stay
    /// tracked; the release is queued anyway.
    pub cancel_failed: Vec<QueueDownloadConflictPayload>,
}

/// Shared builder for the media-request approval search outcome. The convergence
/// cutover removed the per-item `trigger*WantedSearch` mutations (the convergence
/// cursor and `triggerAcquisitionSearch` own search now), but the media-request
/// approval flow still reports its post-approval search via this payload.
pub(crate) fn wanted_search_payload(outcome: WantedSearchOutcome) -> WantedSearchPayload {
    WantedSearchPayload {
        queued_count: outcome.queued_count as i32,
        skipped_in_progress_count: outcome.skipped_in_progress_count as i32,
    }
}

#[Object]
impl WantedMutations {
    /// Queue a saved candidate as an operator override, confirming title-wide conflicts first.
    async fn queue_standby_release(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The saved standby candidate to queue.")] id: ID,
        #[graphql(
            default_with = "StandbyQueueChoiceValue::Check",
            desc = "What to do about downloads of this title that are already running."
        )]
        choice: StandbyQueueChoiceValue,
        #[graphql(
            default,
            desc = "The conflicts the operator saw and confirmed; any other running download is listed again instead."
        )]
        confirmed: Vec<StandbyConflictIdentityInput>,
    ) -> GqlResult<QueueStandbyReleasePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let choice = match choice {
            StandbyQueueChoiceValue::Check => scryer_application::StandbyQueueChoice::Check,
            StandbyQueueChoiceValue::Queue => scryer_application::StandbyQueueChoice::Queue,
            StandbyQueueChoiceValue::CancelAndQueue => {
                scryer_application::StandbyQueueChoice::CancelAndQueue
            }
        };
        let confirmed = confirmed
            .into_iter()
            .map(|item| {
                scryer_application::ClientJobLocator::new(
                    item.client_id.as_ref().map(|id| id.as_str()),
                    item.client_type,
                    item.item_id,
                )
            })
            .collect::<Vec<_>>();
        let outcome = app
            .queue_standby_release(&actor, id.as_str(), choice, &confirmed)
            .await
            .map_err(to_gql_error)?;
        Ok(QueueStandbyReleasePayload {
            grabbed: outcome.grabbed,
            conflicts: outcome
                .conflicts
                .into_iter()
                .map(crate::downloads::queue_download_conflict_payload)
                .collect(),
            cancelled: outcome
                .cancelled
                .into_iter()
                .map(crate::downloads::queue_download_conflict_payload)
                .collect(),
            cancel_failed: outcome
                .cancel_failed
                .into_iter()
                .map(crate::downloads::queue_download_conflict_payload)
                .collect(),
        })
    }

    /// Reopen title-mismatch scopes after a rematch so acquisition coverage can converge again.
    async fn trigger_title_mismatch_recovery_search(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title identity whose changed match scopes should be searched again.")]
        title_id: ID,
    ) -> GqlResult<TriggerTitleMismatchRecoverySearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id_string = title_id.to_string();
        let queued = app
            .trigger_title_mismatch_recovery_search(&actor, &title_id_string)
            .await
            .map_err(to_gql_error)?;
        Ok(TriggerTitleMismatchRecoverySearchPayload {
            title_id,
            queued_count: queued as i32,
        })
    }

    /// Pause acquisition for a scope. `id` is a state-row id or a convergence scope
    /// key; a scope key with no row yet materializes one.
    async fn pause_wanted_item(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Wanted-state row id or convergence scope key to pause.")] id: ID,
    ) -> GqlResult<PauseWantedItemPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        app.pause_wanted_item(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(PauseWantedItemPayload { id: ID::from(id) })
    }

    /// Resume acquisition for a paused scope. `id` is a state-row id or a
    /// convergence scope key.
    async fn resume_wanted_item(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Wanted-state row id or convergence scope key to resume.")] id: ID,
    ) -> GqlResult<ResumeWantedItemPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        app.resume_wanted_item(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(ResumeWantedItemPayload { id: ID::from(id) })
    }

    /// Force a pending release to be grabbed and report whether the grab was accepted.
    async fn force_grab_pending_release(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Pending-release identity to grab.")] id: ID,
    ) -> GqlResult<ForceGrabPendingReleasePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        let grabbed = app
            .force_grab_pending_release(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(ForceGrabPendingReleasePayload {
            id: ID::from(id),
            grabbed,
        })
    }

    /// Dismiss a pending release so it is not considered for the current pending state.
    async fn dismiss_pending_release(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Pending-release identity to dismiss.")] id: ID,
    ) -> GqlResult<DismissPendingReleasePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = id.to_string();
        app.dismiss_pending_release(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DismissPendingReleasePayload { id: ID::from(id) })
    }

    /// Start a background acquisition search for the selected wanted or upgrade scopes.
    async fn trigger_acquisition_search(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Optional wanted-kind, facet, library, title, season, and item filters; an empty library list includes all permitted libraries."
        )]
        input: TriggerAcquisitionSearchInput,
    ) -> GqlResult<AcquisitionSearchJobPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let automatic = input.intent == Some(AcquisitionSearchIntentValue::Automatic);
        if automatic
            && (input.wanted_kind.is_some()
                || input.wanted_item_id.is_some()
                || input.facet.is_some()
                || input.library_ids.is_some())
        {
            return Err(to_gql_error(
                scryer_application::AppError::validation_refused(
                    scryer_interface_core::refusal::INTERFACE_ACQUISITION_SCOPE_INVALID,
                    "Automatic search selects a title and optional season, without Wanted filters",
                ),
            ));
        }
        let request = scryer_application::AcquisitionSearchRequest {
            automatic,
            wanted_kind: input
                .wanted_kind
                .map(|kind| match kind {
                    WantedKindValue::Missing => scryer_application::WantedKind::Missing,
                    WantedKindValue::CutoffUpgrade => scryer_application::WantedKind::CutoffUpgrade,
                })
                .unwrap_or(scryer_application::WantedKind::Missing),
            facet: input.facet.map(MediaFacetValue::into_domain),
            library_ids: input
                .library_ids
                .map(|ids| ids.into_iter().map(String::from).collect())
                .unwrap_or_default(),
            title_id: input.title_id.map(String::from),
            season_number: input.season_number,
            wanted_item_id: input.wanted_item_id.map(String::from),
        };
        let run = app
            .start_acquisition_search_job(&actor, request)
            .await
            .map_err(to_gql_error)?;
        let job_run = scryer_interface_media::mappers::from_job_run(run);
        let progress = job_run.progress_json.as_ref().map(|value| &value.0);
        let count = |key: &str| {
            progress
                .and_then(|value| value.get(key))
                .and_then(|value| value.as_u64())
                .unwrap_or(0)
                .min(i32::MAX as u64) as i32
        };
        let state = match progress
            .and_then(|value| value.get("state"))
            .and_then(|value| value.as_str())
        {
            Some("completed") => AcquisitionSearchJobStateValue::Completed,
            Some("failed") => AcquisitionSearchJobStateValue::Failed,
            Some("cancelled") => AcquisitionSearchJobStateValue::Cancelled,
            _ => AcquisitionSearchJobStateValue::Running,
        };
        Ok(AcquisitionSearchJobPayload {
            id: job_run.id.clone(),
            state,
            total: count("total"),
            processed: count("processed"),
            grabbed_count: count("grabbedCount"),
            failed_count: count("failedCount"),
            current_title: progress
                .and_then(|value| value.get("currentTitle"))
                .and_then(|value| value.as_str())
                .map(str::to_string),
            started_at: job_run.started_at,
            finished_at: job_run.completed_at,
            job_run: Some(job_run),
        })
    }

    /// Request cancellation of an acquisition-search job and report whether its state changed.
    async fn cancel_acquisition_search(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Acquisition-search job identity to cancel.")] id: ID,
    ) -> GqlResult<CancelAcquisitionSearchPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let accepted = app
            .cancel_acquisition_search(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(CancelAcquisitionSearchPayload { id, accepted })
    }
}
