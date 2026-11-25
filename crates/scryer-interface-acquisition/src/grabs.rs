//! Grabs that run in the background.
//!
//! A grab waits on the indexer to hand over the release and on the download
//! client to accept it, and either can take a long time. `startGrab` records a
//! job and returns at once; the grab itself is the same code the synchronous
//! mutations run, and its payload or refusal is read back with `grabJobResult`
//! once the job has finished.

use async_graphql::{Context, ID, InputObject, Object, OneofObject, Result as GqlResult};
use scryer_application::{AppUseCase, GrabJobOutcome, GrabJobReport};
use scryer_domain::User;

use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::grab_results::remember_grab_result;
use scryer_interface_media::types::*;

use crate::wanted::{StandbyConflictIdentityInput, StandbyQueueChoiceValue};

#[derive(Default)]
pub struct GrabMutations;

/// A signed candidate queued with an operator-selected client and category.
#[derive(InputObject)]
pub struct GrabIndexerSearchAssignmentInput {
    /// Signed candidate token and submission options; the token supplies the release identity.
    pub input: QueueDownloadInput,
    /// Operator-selected client and optional category override.
    pub routing: IndexerGrabSelectionInput,
    /// Whether this release replaces existing owned media.
    pub replacement: bool,
}

/// A saved standby candidate to queue.
#[derive(InputObject)]
pub struct GrabStandbyReleaseInput {
    /// The saved standby candidate to queue.
    pub id: ID,
    /// What to do about downloads of this title that are already running.
    #[graphql(default_with = "StandbyQueueChoiceValue::Check")]
    pub choice: StandbyQueueChoiceValue,
    /// The conflicts the operator saw and confirmed; any other running download is listed again instead.
    #[graphql(default)]
    pub confirmed: Vec<StandbyConflictIdentityInput>,
}

/// The grab to run. Each choice takes what the matching synchronous mutation takes.
#[derive(OneofObject)]
pub enum StartGrabInput {
    /// As `queueExistingTitleDownload`.
    Existing(QueueDownloadInput),
    /// As `queueReplacementRelease`.
    Replacement(QueueDownloadInput),
    /// As `queueIndexerSearchAssignment`.
    IndexerSearchAssignment(GrabIndexerSearchAssignmentInput),
    /// As `queueUnlinkedRelease`.
    Unlinked(QueueUnlinkedReleaseInput),
    /// As `queueStandbyRelease`.
    Standby(GrabStandbyReleaseInput),
    /// As `forceGrabPendingRelease`: the pending-release identity to grab.
    Pending(ID),
}

impl StartGrabInput {
    fn kind(&self) -> &'static str {
        match self {
            Self::Existing(_) => "existing",
            Self::Replacement(_) => "replacement",
            Self::IndexerSearchAssignment(_) => "indexer_search_assignment",
            Self::Unlinked(_) => "unlinked",
            Self::Standby(_) => "standby",
            Self::Pending(_) => "pending",
        }
    }
}

/// Queue a release of an interactive search that has no catalog title behind it.
pub async fn queue_unlinked_release(
    app: &AppUseCase,
    actor: &User,
    input: QueueUnlinkedReleaseInput,
) -> GqlResult<QueueUnlinkedReleasePayload> {
    let QueueUnlinkedReleaseInput {
        search_id,
        download_url,
        download_client_id,
        category,
    } = input;
    let outcome = app
        .queue_unlinked_release_with_category(
            actor,
            search_id.as_ref(),
            &download_url,
            download_client_id.as_ref(),
            category,
        )
        .await
        .map_err(to_gql_error)?;
    Ok(QueueUnlinkedReleasePayload {
        download_id: outcome.download_id,
        client_name: outcome.client_name,
        source_title: outcome.source_title,
    })
}

async fn run_grab(
    app: &AppUseCase,
    actor: &User,
    input: StartGrabInput,
) -> GqlResult<GrabJobResultPayload> {
    Ok(match input {
        StartGrabInput::Existing(input) => GrabJobResultPayload {
            queued: Some(crate::downloads::queue_existing_title_download(app, actor, input).await?),
            ..Default::default()
        },
        StartGrabInput::Replacement(input) => GrabJobResultPayload {
            queued: Some(crate::downloads::queue_replacement_release(app, actor, input).await?),
            ..Default::default()
        },
        StartGrabInput::IndexerSearchAssignment(grab) => GrabJobResultPayload {
            queued: Some(
                crate::downloads::queue_indexer_search_assignment(
                    app,
                    actor,
                    grab.input,
                    grab.routing,
                    grab.replacement,
                )
                .await?,
            ),
            ..Default::default()
        },
        StartGrabInput::Unlinked(input) => GrabJobResultPayload {
            unlinked: Some(queue_unlinked_release(app, actor, input).await?),
            ..Default::default()
        },
        StartGrabInput::Standby(grab) => GrabJobResultPayload {
            standby: Some(
                crate::wanted::queue_standby_release(
                    app,
                    actor,
                    grab.id,
                    grab.choice,
                    grab.confirmed,
                )
                .await?,
            ),
            ..Default::default()
        },
        StartGrabInput::Pending(id) => GrabJobResultPayload {
            pending: Some(crate::wanted::force_grab_pending_release(app, actor, id).await?),
            ..Default::default()
        },
    })
}

/// What the job run records for a finished grab.
fn grab_job_report(result: &GqlResult<GrabJobResultPayload>) -> GrabJobReport {
    let payload = match result {
        Ok(payload) => payload,
        Err(error) => {
            let reason = error
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.get("reason"))
                .and_then(|reason| match reason {
                    async_graphql::Value::String(reason) => Some(reason.clone()),
                    _ => None,
                });
            return GrabJobReport {
                outcome: GrabJobOutcome::Failed,
                reason,
                summary_text: None,
                error_text: Some(error.message.clone()),
            };
        }
    };
    let conflict = payload
        .queued
        .as_ref()
        .is_some_and(|queued| queued.status == QueueDownloadResultStatusValue::Conflict)
        || payload
            .standby
            .as_ref()
            .is_some_and(|standby| !standby.grabbed && !standby.conflicts.is_empty());
    if conflict {
        return GrabJobReport {
            outcome: GrabJobOutcome::Conflict,
            reason: None,
            summary_text: Some(
                "Not queued: other downloads of this title are running.".to_string(),
            ),
            error_text: None,
        };
    }
    let not_queued = payload
        .standby
        .as_ref()
        .is_some_and(|standby| !standby.grabbed)
        || payload
            .pending
            .as_ref()
            .is_some_and(|pending| !pending.grabbed);
    if not_queued {
        return GrabJobReport {
            outcome: GrabJobOutcome::NotQueued,
            reason: None,
            summary_text: Some("Nothing was sent to the download client.".to_string()),
            error_text: None,
        };
    }
    let source_title = payload
        .queued
        .as_ref()
        .and_then(|queued| queued.source_title.as_deref())
        .or(payload
            .unlinked
            .as_ref()
            .map(|unlinked| unlinked.source_title.as_str()));
    GrabJobReport {
        outcome: GrabJobOutcome::Queued,
        reason: None,
        summary_text: Some(match source_title {
            Some(source_title) => format!("Sent {source_title} to the download client."),
            None => "Sent the release to the download client.".to_string(),
        }),
        error_text: None,
    }
}

#[Object]
impl GrabMutations {
    /// Start a grab in the background and return its job at once.
    /// The grab runs exactly as the matching synchronous mutation does, and
    /// `grabJobResult` returns that mutation's payload or error once the job
    /// has finished.
    async fn start_grab(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The one grab to run.")] input: StartGrabInput,
    ) -> GqlResult<StartGrabPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let kind = input.kind();
        let job_app = app.clone();
        let job_actor = actor.clone();
        let run = app
            .start_grab_job(&actor, kind, move |run_id| async move {
                let result = run_grab(&job_app, &job_actor, input).await;
                let report = grab_job_report(&result);
                // Held before the job run is finished, so the outcome is there
                // to read as soon as the operator is told the grab is over.
                remember_grab_result(&run_id, &job_actor.id, result);
                report
            })
            .await
            .map_err(to_gql_error)?;
        Ok(StartGrabPayload {
            job_run: scryer_interface_media::mappers::from_job_run(run),
        })
    }
}

#[cfg(test)]
mod tests {
    use async_graphql::ErrorExtensions;

    use super::*;

    #[test]
    fn a_refusal_reports_its_reason_code_apart_from_its_sentence() {
        let refusal = async_graphql::Error::new("validation: the source is gone").extend_with(
            |_, extensions| {
                extensions.set("code", "VALIDATION_ERROR");
                extensions.set("reason", "EXAMPLE_SOURCE_GONE");
            },
        );

        let report = grab_job_report(&Err(refusal));

        assert_eq!(report.outcome, GrabJobOutcome::Failed);
        assert_eq!(report.reason.as_deref(), Some("EXAMPLE_SOURCE_GONE"));
        assert_eq!(
            report.error_text.as_deref(),
            Some("validation: the source is gone")
        );
    }

    fn example_conflict() -> QueueDownloadConflictPayload {
        QueueDownloadConflictPayload {
            title_id: ID::from("title-1"),
            title_name: "Example Title".to_string(),
            download_client_id: None,
            download_client_type: "example".to_string(),
            download_client_item_id: "item-1".to_string(),
            source_title: None,
            source_kind: None,
            scope: QueueDownloadScopePayload::episode(ID::from("episode-1")),
            state: None,
            replaceable: true,
        }
    }

    #[test]
    fn a_standby_grab_that_lists_conflicts_is_not_reported_as_queued() {
        let report = grab_job_report(&Ok(GrabJobResultPayload {
            standby: Some(QueueStandbyReleasePayload {
                grabbed: false,
                conflicts: vec![example_conflict()],
                cancelled: Vec::new(),
                cancel_failed: Vec::new(),
            }),
            ..Default::default()
        }));

        assert_eq!(report.outcome, GrabJobOutcome::Conflict);
        assert_eq!(report.error_text, None);
    }

    #[test]
    fn a_standby_grab_with_nothing_queued_and_no_conflicts_is_not_a_conflict() {
        let report = grab_job_report(&Ok(GrabJobResultPayload {
            standby: Some(QueueStandbyReleasePayload {
                grabbed: false,
                conflicts: Vec::new(),
                cancelled: Vec::new(),
                cancel_failed: Vec::new(),
            }),
            ..Default::default()
        }));

        assert_eq!(report.outcome, GrabJobOutcome::NotQueued);
        assert_eq!(report.error_text, None);
    }

    #[test]
    fn a_pending_release_that_was_not_grabbed_is_not_reported_as_queued() {
        let report = grab_job_report(&Ok(GrabJobResultPayload {
            pending: Some(ForceGrabPendingReleasePayload {
                id: ID::from("pending-1"),
                grabbed: false,
            }),
            ..Default::default()
        }));

        assert_eq!(report.outcome, GrabJobOutcome::NotQueued);
        assert_eq!(report.error_text, None);
    }

    #[test]
    fn a_pending_release_that_was_grabbed_is_reported_as_queued() {
        let report = grab_job_report(&Ok(GrabJobResultPayload {
            pending: Some(ForceGrabPendingReleasePayload {
                id: ID::from("pending-1"),
                grabbed: true,
            }),
            ..Default::default()
        }));

        assert_eq!(report.outcome, GrabJobOutcome::Queued);
    }

    #[test]
    fn an_accepted_grab_names_the_release() {
        let report = grab_job_report(&Ok(GrabJobResultPayload {
            unlinked: Some(QueueUnlinkedReleasePayload {
                download_id: "download-1".to_string(),
                client_name: "Example Client".to_string(),
                source_title: "Example.Release.1080p".to_string(),
            }),
            ..Default::default()
        }));

        assert_eq!(report.outcome, GrabJobOutcome::Queued);
        assert_eq!(
            report.summary_text.as_deref(),
            Some("Sent Example.Release.1080p to the download client.")
        );
    }
}
