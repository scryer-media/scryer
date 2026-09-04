use super::*;

use crate::acquisition_decision_helpers::is_download_submit_unavailable_error;
use crate::catalog::workflow::queue_item_matches_submission;
use crate::download_identity::{
    AcceptedDownloadIdentityInput, accepted_download_submission_identity,
};
use crate::services::{CanonicalSubmissionTitleState, UncertainDownloadSubmissionClaim};

#[derive(Clone)]
pub(crate) struct CanonicalDownloadSubmissionIntent {
    pub request: DownloadClientAddRequest,
    pub scope: SubmissionScope,
    pub conflict_policy: SubmissionConflictPolicy,
    pub request_signature: Option<String>,
    pub source_provider_name: Option<String>,
    pub release_size_bytes: Option<i64>,
}

pub(crate) enum CanonicalDownloadSubmissionOutcome {
    Accepted(CanonicalDownloadSubmission),
    Conflict(SubmissionScopeConflict),
}

pub(crate) struct CanonicalDownloadSubmission {
    pub grab: DownloadGrabResult,
    pub newly_submitted: bool,
}

fn accepted_existing(submission: DownloadSubmission) -> CanonicalDownloadSubmissionOutcome {
    let grab = DownloadGrabResult {
        job_id: submission.download_client_item_id.clone(),
        client_id: submission.download_client_id.clone(),
        client_type: submission.download_client_type.clone(),
        info_hash: None,
        download_id: Some(submission.download_id),
        seed_goals: None,
    };
    CanonicalDownloadSubmissionOutcome::Accepted(CanonicalDownloadSubmission {
        grab,
        newly_submitted: false,
    })
}

fn submission_for_grab(
    intent: &CanonicalDownloadSubmissionIntent,
    request: &DownloadClientAddRequest,
    download_id: scryer_domain::download_identity::DownloadId,
    grab: &DownloadGrabResult,
) -> DownloadSubmission {
    DownloadSubmission {
        download_id,
        title_id: request.title.id.clone(),
        facet: request.title.facet.as_str().to_string(),
        download_client_id: grab.client_id.clone(),
        download_client_type: grab.client_type.clone(),
        download_client_item_id: grab.job_id.clone(),
        source_hint: normalize_release_attempt_hint(request.source_hint.as_deref()),
        source_provider_id: request.indexer_id.clone(),
        source_provider_name: intent.source_provider_name.clone(),
        source_kind: request.source_kind,
        source_title: request.source_title.clone(),
        info_hash: request.info_hash_hint.clone(),
        release_size_bytes: intent.release_size_bytes,
        request_signature: intent.request_signature.clone(),
        purpose: request.purpose,
        scope: intent.scope.clone(),
    }
}

fn accepted_identity_for_grab(
    request: &DownloadClientAddRequest,
    download_id: scryer_domain::download_identity::DownloadId,
    grab: &DownloadGrabResult,
) -> DownloadSubmissionIdentity {
    let download_id_wire = download_id.to_wire();
    accepted_download_submission_identity(AcceptedDownloadIdentityInput {
        initial_download_id: Some(download_id_wire.as_str()),
        source_kind: request.source_kind,
        source_hint: request.source_hint.as_deref(),
        info_hash_hint: request.info_hash_hint.as_deref(),
        client_type: Some(grab.client_type.as_str()),
        client_item_id: Some(grab.job_id.as_str()),
        accepted_info_hash: grab.info_hash.as_deref(),
    })
}

fn submission_client_state_is_authoritative(
    snapshot: &DownloadClientSnapshotOutcome,
    submission: &DownloadSubmission,
) -> bool {
    submission
        .download_client_id
        .as_deref()
        .map(str::trim)
        .filter(|client_id| !client_id.is_empty())
        .is_some_and(|client_id| snapshot.authoritative_client_ids.contains(client_id))
}

fn submission_matches_intent(
    submission: &DownloadSubmission,
    intent: &CanonicalDownloadSubmissionIntent,
) -> bool {
    intent.request_signature.is_some()
        && submission.request_signature == intent.request_signature
        && submission.purpose == intent.request.purpose
        && submission.scope == intent.scope
}

impl AppUseCase {
    async fn adopt_canonical_download(
        &self,
        intent: &CanonicalDownloadSubmissionIntent,
        request: &DownloadClientAddRequest,
        effective_download_id: scryer_domain::download_identity::DownloadId,
        adopted_grab: DownloadGrabResult,
    ) -> AppResult<CanonicalDownloadSubmissionOutcome> {
        let title_id = request.title.id.as_str();
        let Some(existing) = self
            .services
            .workflow
            .download_submissions
            .find_by_canonical_download_id(&effective_download_id)
            .await?
        else {
            return Err(AppError::DownloadSubmitRejected(format!(
                "download client reused canonical identity {effective_download_id}, but its submission could not be loaded"
            )));
        };
        if existing.title_id != title_id {
            return Err(AppError::DownloadSubmitRejected(format!(
                "download client reused canonical identity {effective_download_id} owned by title {}, not {title_id}",
                existing.title_id
            )));
        }

        let accepted_identity =
            accepted_identity_for_grab(request, effective_download_id, &adopted_grab);
        if existing.request_signature.is_none()
            && existing.source_hint.is_none()
            && existing.source_title.is_none()
        {
            let submission =
                submission_for_grab(intent, request, effective_download_id, &adopted_grab);
            let disposition = match self
                .services
                .workflow
                .download_submissions
                .record_submission_with_identity(
                    submission.clone(),
                    accepted_identity.clone(),
                    None,
                )
                .await
            {
                Ok(disposition) => disposition,
                Err(error) => {
                    self.runtime
                        .acquisition
                        .download_submission_guards
                        .mark_uncertain(
                            title_id,
                            UncertainDownloadSubmissionClaim::accepted(
                                submission,
                                accepted_identity,
                                None,
                            ),
                        );
                    return Err(AppError::DownloadSubmitAmbiguous(format!(
                        "adopted download submission {effective_download_id} could not be made durable for title {title_id}: {error}"
                    )));
                }
            };
            if let CanonicalDownloadIdentityDisposition::AdoptedExisting { download_id } =
                disposition
                && download_id != effective_download_id
            {
                let Some(rebound) = self
                    .services
                    .workflow
                    .download_submissions
                    .find_by_canonical_download_id(&download_id)
                    .await?
                else {
                    return Err(AppError::DownloadSubmitRejected(format!(
                        "download client reused canonical identity {download_id}, but its submission could not be loaded"
                    )));
                };
                if rebound.title_id != title_id {
                    return Err(AppError::DownloadSubmitRejected(format!(
                        "download client reused canonical identity {download_id} owned by title {}, not {title_id}",
                        rebound.title_id
                    )));
                }
                return Ok(accepted_existing(rebound));
            }
        }
        Ok(CanonicalDownloadSubmissionOutcome::Accepted(
            CanonicalDownloadSubmission {
                grab: adopted_grab,
                newly_submitted: false,
            },
        ))
    }

    pub(crate) async fn submit_canonical_download(
        &self,
        intent: CanonicalDownloadSubmissionIntent,
    ) -> AppResult<CanonicalDownloadSubmissionOutcome> {
        let title_id = intent.request.title.id.clone();
        let _title_guard = self
            .runtime
            .acquisition
            .download_submission_guards
            .acquire_title(&title_id)
            .await;

        if let Some(claim) = self
            .runtime
            .acquisition
            .download_submission_guards
            .uncertain_claim(&title_id)
        {
            match claim {
                UncertainDownloadSubmissionClaim::Ambiguous {
                    download_id,
                    submission,
                } => {
                    if let Some(submission) = submission.as_ref()
                        && self
                            .services
                            .workflow
                            .download_submissions
                            .record_ambiguous_submission(submission.clone())
                            .await
                            .is_ok()
                    {
                        self.runtime
                            .acquisition
                            .download_submission_guards
                            .clear_uncertain(&title_id);
                    }
                    return Err(AppError::DownloadSubmitAmbiguous(format!(
                        "download submission {download_id} is still uncertain for title {title_id}"
                    )));
                }
                UncertainDownloadSubmissionClaim::Accepted {
                    submission,
                    accepted_identity,
                    seed_goals,
                } => {
                    let disposition = match self
                        .services
                        .workflow
                        .download_submissions
                        .record_submission_with_identity(
                            submission.clone(),
                            accepted_identity.clone(),
                            seed_goals.clone(),
                        )
                        .await
                    {
                        Ok(disposition) => disposition,
                        Err(error) => {
                            tracing::warn!(
                                error = %error,
                                title_id = %title_id,
                                download_id = %submission.download_id,
                                "accepted download submission is still not durable"
                            );
                            return Err(AppError::DownloadSubmitAmbiguous(format!(
                                "download submission {} is still uncertain for title {title_id}",
                                submission.download_id
                            )));
                        }
                    };
                    self.runtime
                        .acquisition
                        .download_submission_guards
                        .clear_uncertain(&title_id);
                    match disposition {
                        CanonicalDownloadIdentityDisposition::Requested => {
                            if submission_matches_intent(&submission, &intent) {
                                return Ok(accepted_existing(submission));
                            }
                        }
                        CanonicalDownloadIdentityDisposition::AdoptedExisting { download_id } => {
                            let adopted_grab = DownloadGrabResult {
                                job_id: submission.download_client_item_id,
                                client_id: submission.download_client_id,
                                client_type: submission.download_client_type,
                                info_hash: seed_goals.and_then(|goals| goals.info_hash),
                                download_id: Some(download_id),
                                seed_goals: None,
                            };
                            return self
                                .adopt_canonical_download(
                                    &intent,
                                    &intent.request,
                                    download_id,
                                    adopted_grab,
                                )
                                .await;
                        }
                    }
                }
            }
        }

        let mut state = if let Some(state) = self
            .runtime
            .acquisition
            .download_submission_guards
            .cached_title_state(&title_id)
        {
            state
        } else {
            if !self
                .services
                .workflow
                .download_submissions
                .list_active_unbound_for_title(&title_id)
                .await?
                .is_empty()
            {
                return Err(AppError::DownloadSubmitAmbiguous(format!(
                    "download submission acceptance is unresolved for title {title_id}"
                )));
            }
            let submissions = self
                .services
                .workflow
                .download_submissions
                .list_for_title(&title_id)
                .await?;
            let episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_title(&title_id)
                .await?;
            let state = CanonicalSubmissionTitleState::new(submissions, episodes);
            self.runtime
                .acquisition
                .download_submission_guards
                .store_title_state(&title_id, state.clone());
            state
        };
        let existing = intent
            .request_signature
            .as_deref()
            .and_then(|signature| {
                state.submissions.iter().find(|submission| {
                    submission.request_signature.as_deref() == Some(signature)
                        && submission.purpose == intent.request.purpose
                        && submission.scope == intent.scope
                })
            })
            .cloned();
        let snapshot = if state.submissions.is_empty() {
            None
        } else {
            let guards = &self.runtime.acquisition.download_submission_guards;
            let snapshot_is_authoritative = |snapshot: &DownloadClientSnapshotOutcome| {
                state.submissions.iter().all(|submission| {
                    state
                        .accepted_download_ids
                        .contains(&submission.download_id)
                        || submission_client_state_is_authoritative(snapshot, submission)
                })
            };
            let cached = guards
                .cached_client_snapshot()
                .filter(snapshot_is_authoritative);
            if let Some(snapshot) = cached {
                Some(snapshot)
            } else {
                let _snapshot_guard = guards.acquire_client_snapshot().await;
                let snapshot = if let Some(snapshot) = guards
                    .cached_client_snapshot()
                    .filter(snapshot_is_authoritative)
                {
                    snapshot
                } else {
                    let snapshot = self
                        .services
                        .integrations
                        .download_client
                        .list_snapshot_outcome_excluding_client_types(100, &[])
                        .await?;
                    guards.store_client_snapshot(snapshot.clone());
                    snapshot
                };
                Some(snapshot)
            }
        };

        if let Some(existing) = existing {
            let snapshot = snapshot
                .as_ref()
                .expect("persisted submission has a snapshot");
            if let Some(item) = snapshot
                .items
                .iter()
                .find(|item| queue_item_matches_submission(item, &existing))
            {
                if item.state != DownloadQueueState::Failed {
                    return Ok(accepted_existing(existing));
                }
                if !submission_client_state_is_authoritative(snapshot, &existing) {
                    return Err(AppError::DownloadSubmitUnavailable(format!(
                        "download client state is unavailable for submission {} on title {title_id}",
                        existing.download_id
                    )));
                }
            } else if !submission_client_state_is_authoritative(snapshot, &existing) {
                return Err(AppError::DownloadSubmitUnavailable(format!(
                    "download client state is unavailable for submission {} on title {title_id}",
                    existing.download_id
                )));
            }
            self.services
                .workflow
                .download_submissions
                .delete_by_client_item_id(&ClientJobLocator::from_submission(&existing))
                .await?;
            state.forget(existing.download_id);
            self.runtime
                .acquisition
                .download_submission_guards
                .store_title_state(&title_id, state.clone());
        }

        let conflicts = if intent.request.purpose.is_additional_file() {
            Vec::new()
        } else if let Some(snapshot) = snapshot.as_ref() {
            Self::find_blocking_download_submissions_in_state(
                &intent.request.title,
                &intent.scope,
                &state.submissions,
                snapshot,
                &state.episodes,
                &state.accepted_download_ids,
            )?
        } else {
            Vec::new()
        };
        if !conflicts.is_empty() {
            match intent.conflict_policy {
                SubmissionConflictPolicy::Abort | SubmissionConflictPolicy::Skip => {
                    return Ok(CanonicalDownloadSubmissionOutcome::Conflict(
                        conflicts[0].clone(),
                    ));
                }
                SubmissionConflictPolicy::ReplaceEarly
                    if conflicts.iter().all(|conflict| conflict.replaceable) =>
                {
                    self.replace_blocking_download_submissions(&conflicts)
                        .await?;
                    state.submissions = self
                        .services
                        .workflow
                        .download_submissions
                        .list_for_title(&title_id)
                        .await?;
                    state.accepted_download_ids.clear();
                }
                SubmissionConflictPolicy::ReplaceEarly => {
                    let conflict = conflicts
                        .into_iter()
                        .find(|conflict| !conflict.replaceable)
                        .expect("non-empty conflicts should contain a non-replaceable item");
                    return Ok(CanonicalDownloadSubmissionOutcome::Conflict(conflict));
                }
            }
        }

        let download_id = intent.request.download_id.ok_or_else(|| {
            AppError::Validation("canonical download submission requires a download id".to_string())
        })?;
        let source_kind = intent.request.source_kind;
        let source_hint = normalize_release_attempt_hint(intent.request.source_hint.as_deref());
        let request = DownloadClientAddRequest {
            download_id: Some(download_id),
            ..intent.request.clone()
        };
        let grab = match self
            .services
            .integrations
            .download_client
            .submit_download(&request)
            .await
        {
            Ok(grab) => grab,
            Err(error) => {
                if error.is_download_submit_ambiguous() {
                    let ambiguous = error.ambiguous_download_submission_client().map(
                        |(client_id, client_type)| DownloadSubmission {
                            download_id,
                            title_id: title_id.clone(),
                            facet: request.title.facet.as_str().to_string(),
                            download_client_id: client_id.map(str::to_string),
                            download_client_type: client_type.to_string(),
                            download_client_item_id: String::new(),
                            source_hint: source_hint.clone(),
                            source_provider_id: request.indexer_id.clone(),
                            source_provider_name: intent.source_provider_name.clone(),
                            source_kind,
                            source_title: request.source_title.clone(),
                            info_hash: request.info_hash_hint.clone(),
                            release_size_bytes: intent.release_size_bytes,
                            request_signature: intent.request_signature.clone(),
                            purpose: request.purpose,
                            scope: intent.scope.clone(),
                        },
                    );
                    let persisted = if let Some(ambiguous) = ambiguous.as_ref() {
                        self.services
                            .workflow
                            .download_submissions
                            .record_ambiguous_submission(ambiguous.clone())
                            .await
                            .is_ok()
                    } else {
                        false
                    };
                    if !persisted {
                        self.runtime
                            .acquisition
                            .download_submission_guards
                            .mark_uncertain(
                                &title_id,
                                UncertainDownloadSubmissionClaim::ambiguous(download_id, ambiguous),
                            );
                    }
                }
                return Err(error);
            }
        };

        if grab
            .download_id
            .is_some_and(|returned_download_id| returned_download_id != download_id)
        {
            let ambiguous = DownloadSubmission {
                download_id,
                title_id: title_id.clone(),
                facet: request.title.facet.as_str().to_string(),
                download_client_id: grab.client_id.clone(),
                download_client_type: grab.client_type.clone(),
                download_client_item_id: String::new(),
                source_hint: source_hint.clone(),
                source_provider_id: request.indexer_id.clone(),
                source_provider_name: intent.source_provider_name.clone(),
                source_kind,
                source_title: request.source_title.clone(),
                info_hash: request.info_hash_hint.clone(),
                release_size_bytes: intent.release_size_bytes,
                request_signature: intent.request_signature.clone(),
                purpose: request.purpose,
                scope: intent.scope.clone(),
            };
            if self
                .services
                .workflow
                .download_submissions
                .record_ambiguous_submission(ambiguous.clone())
                .await
                .is_err()
            {
                self.runtime
                    .acquisition
                    .download_submission_guards
                    .mark_uncertain(
                        &title_id,
                        UncertainDownloadSubmissionClaim::ambiguous(download_id, Some(ambiguous)),
                    );
            }
            return Err(AppError::DownloadSubmitAmbiguous(format!(
                "download client returned a different canonical identity for title {title_id}"
            ))
            .with_ambiguous_download_submission_client(
                grab.client_id.clone(),
                grab.client_type.clone(),
            ));
        }

        let accepted_identity = accepted_identity_for_grab(&request, download_id, &grab);
        let submission = submission_for_grab(&intent, &request, download_id, &grab);
        let seed_goals = grab.seed_goals.clone();
        let identity_disposition = match self
            .services
            .workflow
            .download_submissions
            .record_submission_with_identity(
                submission.clone(),
                accepted_identity.clone(),
                seed_goals.clone(),
            )
            .await
        {
            Ok(disposition) => disposition,
            Err(error) => {
                self.runtime
                    .acquisition
                    .download_submission_guards
                    .mark_uncertain(
                        &title_id,
                        UncertainDownloadSubmissionClaim::accepted(
                            submission,
                            accepted_identity,
                            seed_goals,
                        ),
                    );
                return Err(AppError::DownloadSubmitAmbiguous(format!(
                    "accepted download submission {download_id} could not be made durable for title {title_id}: {error}"
                ))
                .with_ambiguous_download_submission_client(
                    grab.client_id.clone(),
                    grab.client_type.clone(),
                ));
            }
        };
        if let CanonicalDownloadIdentityDisposition::AdoptedExisting {
            download_id: effective_download_id,
        } = identity_disposition
        {
            let adopted_grab = DownloadGrabResult {
                download_id: Some(effective_download_id),
                seed_goals: None,
                ..grab
            };
            let outcome = self
                .adopt_canonical_download(&intent, &request, effective_download_id, adopted_grab)
                .await?;
            if let Some(adopted) = self
                .services
                .workflow
                .download_submissions
                .find_by_canonical_download_id(&effective_download_id)
                .await?
            {
                state.remember(adopted);
                self.runtime
                    .acquisition
                    .download_submission_guards
                    .store_title_state(&title_id, state);
            }
            return Ok(outcome);
        }

        state.remember(submission);
        self.runtime
            .acquisition
            .download_submission_guards
            .store_title_state(&title_id, state);

        Ok(CanonicalDownloadSubmissionOutcome::Accepted(
            CanonicalDownloadSubmission {
                grab,
                newly_submitted: true,
            },
        ))
    }
}

// ── Grab submission metrics ────────────────────────────────────────────────
//
// Every grab in the product funnels through `submit_canonical_download`, so
// its outcome is counted once, here, rather than at each caller. Counting only
// successes (what the five call sites used to do inline) made a download
// client that rejects everything look like "no grabs happened" instead of
// "every grab failed", and let each site invent its own `indexer` label.
//
// Label discipline: every value is a `&'static str` from a bounded set, except
// `indexer`, which is a configured indexer name (bounded by the configured
// indexers) with an `unknown` fallback. Titles, URLs, ids and error text never
// reach a label.

const GRAB_SUBMISSIONS_TOTAL: &str = "scryer_grab_submissions_total";
const GRABS_TOTAL: &str = "scryer_grabs_total";
const RSS_SYNC_TOTAL: &str = "scryer_rss_sync_total";
const UNKNOWN_INDEXER: &str = "unknown";

const RESULT_GRABBED: &str = "grabbed";
const RESULT_REUSED: &str = "reused";
const RESULT_CONFLICT: &str = "conflict";
const RESULT_DEFERRED: &str = "deferred";
const RESULT_FAILED: &str = "failed";

/// Which product loop asked for a grab. This is the *trigger*, deliberately
/// separate from the indexer that supplied the release — conflating the two is
/// exactly what the old `indexer="manual"` label did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GrabTrigger {
    /// RSS sync grabbed a matched release.
    Rss,
    /// Background acquisition grabbed a searched candidate.
    Auto,
    /// Background acquisition grabbed a season pack.
    SeasonPack,
    /// A parked pending release came due and was grabbed.
    Pending,
    /// An operator or API client queued a specific release.
    Manual,
}

impl GrabTrigger {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Rss => "rss",
            Self::Auto => "auto",
            Self::SeasonPack => "season_pack",
            Self::Pending => "pending",
            Self::Manual => "manual",
        }
    }
}

/// Classifies one submission outcome into the fixed `result` label set.
///
/// `deferred` uses the same predicate pair the RSS path uses to choose
/// `ReleaseDownloadAttemptOutcome::Pending` over `Failed`, so the metric and
/// the release-attempt store can never disagree about whether a release was
/// burned.
fn grab_submission_result_label(
    result: &AppResult<CanonicalDownloadSubmissionOutcome>,
) -> &'static str {
    match result {
        Ok(CanonicalDownloadSubmissionOutcome::Accepted(submission)) => {
            if submission.newly_submitted {
                RESULT_GRABBED
            } else {
                RESULT_REUSED
            }
        }
        Ok(CanonicalDownloadSubmissionOutcome::Conflict(_)) => RESULT_CONFLICT,
        Err(err) => {
            if is_download_submit_unavailable_error(err) || err.is_download_submit_ambiguous() {
                RESULT_DEFERRED
            } else {
                RESULT_FAILED
            }
        }
    }
}

/// Records the outcome of one `submit_canonical_download` call.
///
/// Call this at every grab site immediately after the submission returns and
/// before the outcome is matched, passing the same indexer name the site hands
/// to `record_indexer_grab`. `scryer_grabs_total` is incremented only for a
/// genuinely new submission, and only from here.
pub(crate) fn record_grab_submission_outcome(
    trigger: GrabTrigger,
    facet: &MediaFacet,
    indexer: Option<&str>,
    result: &AppResult<CanonicalDownloadSubmissionOutcome>,
) {
    let result_label = grab_submission_result_label(result);
    metrics::counter!(
        GRAB_SUBMISSIONS_TOTAL,
        "trigger" => trigger.as_str(),
        "facet" => facet.as_str(),
        "result" => result_label,
    )
    .increment(1);

    if result_label == RESULT_GRABBED {
        let indexer = indexer
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(UNKNOWN_INDEXER)
            .to_string();
        metrics::counter!(
            GRABS_TOTAL,
            "indexer" => indexer,
            "facet" => facet.as_str(),
            "trigger" => trigger.as_str(),
        )
        .increment(1);
    }
}

/// Registers HELP/UNIT metadata for the acquisition metric families this crate
/// emits.
///
/// Crate-public because the binary's metrics setup calls it once at startup, so
/// the scrape surface is self-describing even while a family is still empty.
pub fn describe_acquisition_metrics() {
    metrics::describe_counter!(
        GRAB_SUBMISSIONS_TOTAL,
        "Grab submissions attempted, labelled by trigger (rss, auto, season_pack, pending, manual), media facet, and result (grabbed, reused, conflict, deferred, failed)."
    );
    metrics::describe_counter!(
        GRABS_TOTAL,
        "Releases newly sent to a download client, labelled by the indexer that supplied the release, the media facet, and the trigger that asked for the grab."
    );

    metrics::describe_counter!(
        RSS_SYNC_TOTAL,
        "RSS sync cycles that finished, labelled by outcome (completed, or an early exit: no_titles, no_clients)."
    );
    metrics::describe_histogram!(
        "scryer_rss_sync_duration_seconds",
        metrics::Unit::Seconds,
        "Wall-clock duration of one RSS sync cycle, including early exits."
    );
    metrics::describe_counter!(
        "scryer_rss_releases_fetched_total",
        "Releases returned by indexer RSS feeds across all completed RSS sync cycles."
    );
    metrics::describe_counter!(
        "scryer_rss_releases_matched_total",
        "Fetched RSS releases that matched a monitored title or episode."
    );
    metrics::describe_counter!(
        "scryer_rss_releases_grabbed_total",
        "Matched RSS releases that were actually grabbed."
    );

    metrics::describe_counter!(
        "scryer_background_acquisition_title_work_total",
        "Background title-level acquisition units of work, labelled by outcome (completed or failed)."
    );
    metrics::describe_counter!(
        "scryer_background_acquisition_target_work_total",
        "Background target-level acquisition units of work, labelled by outcome (completed or failed)."
    );
    metrics::describe_counter!(
        "scryer_background_acquisition_scan_owned_yields_total",
        "Times background acquisition yielded because a library scan owned the facet it wanted to work on."
    );

    metrics::describe_counter!(
        "scryer_wanted_projection_cache_total",
        "Wanted-projection cache lookups, labelled by result (hit or miss)."
    );
    metrics::describe_histogram!(
        "scryer_wanted_projection_rebuild_duration_seconds",
        metrics::Unit::Seconds,
        "Time taken to rebuild a wanted projection, labelled by the projection kind."
    );
    metrics::describe_gauge!(
        "scryer_wanted_projection_items",
        "Number of rows in the most recently rebuilt wanted projection, labelled by the projection kind."
    );
}

#[cfg(test)]
mod grab_metrics_tests {
    use std::collections::BTreeMap;

    use metrics::with_local_recorder;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use super::*;

    /// One recorded counter series: name, sorted labels, value.
    type CounterSeries = (String, BTreeMap<String, String>, u64);

    fn recorded_counters(record: impl FnOnce()) -> Vec<CounterSeries> {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        with_local_recorder(&recorder, record);
        snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _, _, value)| match value {
                DebugValue::Counter(count) => Some((
                    key.key().name().to_string(),
                    key.key()
                        .labels()
                        .map(|label| (label.key().to_string(), label.value().to_string()))
                        .collect::<BTreeMap<String, String>>(),
                    count,
                )),
                _ => None,
            })
            .collect()
    }

    fn series<'a>(counters: &'a [CounterSeries], name: &str) -> Vec<&'a CounterSeries> {
        counters
            .iter()
            .filter(|(series_name, _, _)| series_name == name)
            .collect()
    }

    fn grab_result() -> DownloadGrabResult {
        DownloadGrabResult {
            job_id: "job-1".to_string(),
            client_id: Some("client-1".to_string()),
            client_type: "sabnzbd".to_string(),
            info_hash: None,
            download_id: None,
            seed_goals: None,
        }
    }

    fn accepted(newly_submitted: bool) -> AppResult<CanonicalDownloadSubmissionOutcome> {
        Ok(CanonicalDownloadSubmissionOutcome::Accepted(
            CanonicalDownloadSubmission {
                grab: grab_result(),
                newly_submitted,
            },
        ))
    }

    fn conflict() -> AppResult<CanonicalDownloadSubmissionOutcome> {
        Ok(CanonicalDownloadSubmissionOutcome::Conflict(
            SubmissionScopeConflict {
                title_id: "title-1".to_string(),
                title_name: "Example".to_string(),
                download_client_id: Some("client-1".to_string()),
                download_client_type: "sabnzbd".to_string(),
                download_client_item_id: "job-1".to_string(),
                source_title: None,
                source_kind: None,
                scope: SubmissionScope::Title,
                state: None,
                replaceable: false,
            },
        ))
    }

    fn failure(error: AppError) -> AppResult<CanonicalDownloadSubmissionOutcome> {
        Err(error)
    }

    fn result_label_for(result: &AppResult<CanonicalDownloadSubmissionOutcome>) -> String {
        let counters = recorded_counters(|| {
            record_grab_submission_outcome(
                GrabTrigger::Rss,
                &MediaFacet::Series,
                Some("nzb"),
                result,
            )
        });
        let submissions = series(&counters, GRAB_SUBMISSIONS_TOTAL);
        assert_eq!(
            submissions.len(),
            1,
            "exactly one submission series per call: {counters:?}"
        );
        submissions[0]
            .1
            .get("result")
            .expect("result label present")
            .clone()
    }

    #[test]
    fn every_submission_outcome_maps_to_its_result_label() {
        assert_eq!(result_label_for(&accepted(true)), RESULT_GRABBED);
        assert_eq!(result_label_for(&accepted(false)), RESULT_REUSED);
        assert_eq!(result_label_for(&conflict()), RESULT_CONFLICT);
        // The two deferrable failures: the client was unavailable, and the
        // request may have been accepted with the response lost. Both are
        // retried without burning the release, so neither may read as `failed`.
        assert_eq!(
            result_label_for(&failure(AppError::DownloadSubmitUnavailable(
                "client offline".to_string()
            ))),
            RESULT_DEFERRED
        );
        assert_eq!(
            result_label_for(&failure(AppError::DownloadSubmitFailoverExhausted(
                "every client failed".to_string()
            ))),
            RESULT_DEFERRED
        );
        assert_eq!(
            result_label_for(&failure(AppError::DownloadSubmitAmbiguous(
                "response lost".to_string()
            ))),
            RESULT_DEFERRED
        );
        assert_eq!(
            result_label_for(&failure(AppError::Validation("bad request".to_string()))),
            RESULT_FAILED
        );
        assert_eq!(
            result_label_for(&failure(AppError::DownloadSubmitRejected(
                "client said no".to_string()
            ))),
            RESULT_FAILED
        );
    }

    #[test]
    fn submission_counter_carries_trigger_and_facet() {
        let counters = recorded_counters(|| {
            record_grab_submission_outcome(
                GrabTrigger::SeasonPack,
                &MediaFacet::Anime,
                Some("nzbgeek"),
                &conflict(),
            );
        });

        let submissions = series(&counters, GRAB_SUBMISSIONS_TOTAL);
        assert_eq!(submissions.len(), 1);
        let (_, labels, value) = submissions[0];
        assert_eq!(*value, 1);
        assert_eq!(
            labels.get("trigger").map(String::as_str),
            Some("season_pack")
        );
        assert_eq!(labels.get("facet").map(String::as_str), Some("anime"));
        assert_eq!(labels.get("result").map(String::as_str), Some("conflict"));
    }

    #[test]
    fn grabs_total_is_emitted_only_for_a_new_submission() {
        for result in [
            accepted(false),
            conflict(),
            failure(AppError::DownloadSubmitUnavailable("offline".to_string())),
            failure(AppError::Validation("bad".to_string())),
        ] {
            let counters = recorded_counters(|| {
                record_grab_submission_outcome(
                    GrabTrigger::Auto,
                    &MediaFacet::Movie,
                    Some("nzbgeek"),
                    &result,
                );
            });
            assert!(
                series(&counters, GRABS_TOTAL).is_empty(),
                "non-grabbed outcome must not count as a grab: {counters:?}"
            );
        }

        let counters = recorded_counters(|| {
            record_grab_submission_outcome(
                GrabTrigger::Auto,
                &MediaFacet::Movie,
                Some("nzbgeek"),
                &accepted(true),
            );
        });
        let grabs = series(&counters, GRABS_TOTAL);
        assert_eq!(grabs.len(), 1);
        let (_, labels, value) = grabs[0];
        assert_eq!(*value, 1);
        assert_eq!(labels.get("indexer").map(String::as_str), Some("nzbgeek"));
        assert_eq!(labels.get("facet").map(String::as_str), Some("movie"));
        assert_eq!(labels.get("trigger").map(String::as_str), Some("auto"));
    }

    #[test]
    fn missing_or_blank_indexer_falls_back_to_unknown() {
        for indexer in [None, Some(""), Some("   ")] {
            let counters = recorded_counters(|| {
                record_grab_submission_outcome(
                    GrabTrigger::Manual,
                    &MediaFacet::Movie,
                    indexer,
                    &accepted(true),
                );
            });
            let grabs = series(&counters, GRABS_TOTAL);
            assert_eq!(grabs.len(), 1);
            assert_eq!(
                grabs[0].1.get("indexer").map(String::as_str),
                Some(UNKNOWN_INDEXER),
                "indexer {indexer:?} should fall back to unknown"
            );
        }
    }

    #[test]
    fn trigger_labels_are_unique_snake_case() {
        let triggers = [
            GrabTrigger::Rss,
            GrabTrigger::Auto,
            GrabTrigger::SeasonPack,
            GrabTrigger::Pending,
            GrabTrigger::Manual,
        ];
        let labels: std::collections::BTreeSet<&str> =
            triggers.iter().map(|trigger| trigger.as_str()).collect();
        assert_eq!(
            labels.len(),
            triggers.len(),
            "trigger labels must be unique"
        );
        for label in labels {
            assert!(
                !label.is_empty()
                    && label.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                    && !label.starts_with('_')
                    && !label.ends_with('_'),
                "trigger label {label:?} is not snake_case"
            );
        }
    }
}
