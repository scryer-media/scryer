use super::*;

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
