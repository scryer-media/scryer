//! Mutations that start, cancel, and resume location operations (US2, FR-030,
//! FR-033, FR-081, FR-083).
//!
//! Starting is a confirmation, not a submission: the client sends back the
//! fingerprint it previewed and the server rebuilds the plan from current state,
//! so a caller can never confirm one plan and execute another.

use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::AppError;
use scryer_application::location::classify::DestinationRequest;
use scryer_application::location::consolidation_execution::StartRootConsolidationRequest;
use scryer_application::location::model::LocationExecutionMode;
use scryer_application::location::operations::{LocationResumeDecision, StartRootMoveRequest};
use scryer_application::location::preview::{PlanConfirmationRequest, PlanFingerprint};
use scryer_application::location::root_change_execution::StartRootChangeRequest;

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{
    from_canceled_location_operation, from_resumed_location_operation,
    from_started_location_operation, location_destination_into_application,
    location_execution_mode_into_application, root_scoped_execution_mode_into_application,
};
use crate::types::{
    CancelLocationOperationPayload, LocationDestinationInput, LocationRootChangeTargetInput,
    LocationRootConsolidationTargetInput, ResumeLocationOperationPayload,
    StartLocationOperationInput, StartLocationOperationPayload,
};
use scryer_interface_core::to_location_root_gql_error;

/// The one destination form a start confirms.
///
/// `StartLocationOperationInput` offers three and the schema cannot say "pick
/// exactly one", so this is where that is said. Sending none, or more than one,
/// is a request that does not describe a plan, and it is refused before the
/// application is asked to rebuild anything.
enum StartLocationTarget {
    Selection {
        title_ids: Vec<String>,
        destination: DestinationRequest,
    },
    RootChange(LocationRootChangeTargetInput),
    RootConsolidation(LocationRootConsolidationTargetInput),
}

fn start_location_target(
    title_ids: Option<Vec<ID>>,
    destination: Option<LocationDestinationInput>,
    root_change: Option<LocationRootChangeTargetInput>,
    root_consolidation: Option<LocationRootConsolidationTargetInput>,
) -> GqlResult<StartLocationTarget> {
    let names_a_selection = title_ids.is_some() || destination.is_some();
    match (names_a_selection, root_change, root_consolidation) {
        (false, Some(target), None) => Ok(StartLocationTarget::RootChange(target)),
        (false, None, Some(target)) => Ok(StartLocationTarget::RootConsolidation(target)),
        (true, None, None) => Ok(StartLocationTarget::Selection {
            // A client that predates the root-scoped variants always sends
            // both, so it lands here with exactly the request it always sent.
            title_ids: title_ids
                .unwrap_or_default()
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
            destination: location_destination_into_application(destination.unwrap_or(
                LocationDestinationInput {
                    library_id: None,
                    root_id: None,
                },
            )),
        }),
        _ => Err(to_gql_error(AppError::Validation(
            "name exactly one of a title selection, a root change, or a root consolidation"
                .to_string(),
        ))),
    }
}

#[derive(Default)]
pub(crate) struct LocationMutations;

#[Object]
impl LocationMutations {
    /// Confirm a previewed location operation and start it in the background.
    ///
    /// The plan is rebuilt server side and compared against the fingerprint the
    /// client previewed; a stale fingerprint, or a selection that still holds
    /// blocked or unresolved titles, is refused instead of started.
    ///
    /// The mode is confirmed the same way everything else is: it rebuilds the
    /// plan, so confirming `FILES_ALREADY_THERE` against a managed-move
    /// fingerprint fails the comparison rather than running the other workflow.
    /// An adoption whose destination is missing or ambiguous media is refused
    /// here, because its rebuilt plan is blocked (FR-052).
    ///
    /// A root change (US4) and a root consolidation (US5) confirm through the
    /// same mutation and the same typed-confirmation field; only the destination
    /// form differs. Exactly one of the three destination forms may be named.
    async fn start_location_operation(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Previewed destination form (selection, root change, or root consolidation), mode, plan fingerprint, and typed confirmation when one is required."
        )]
        input: StartLocationOperationInput,
    ) -> GqlResult<StartLocationOperationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let mode = location_execution_mode_into_application(input.mode);
        let confirmation = PlanConfirmationRequest {
            fingerprint: PlanFingerprint(input.plan_fingerprint),
            typed_confirmation: input.typed_confirmation,
        };
        let target = start_location_target(
            input.title_ids,
            input.destination,
            input.root_change,
            input.root_consolidation,
        )?;
        let accepted = match target {
            StartLocationTarget::Selection {
                title_ids,
                destination,
            } => {
                let request = StartRootMoveRequest {
                    title_ids,
                    destination,
                    confirmation,
                };
                match mode {
                    LocationExecutionMode::FilesAlreadyThere => {
                        app.start_adoption(&actor, request).await
                    }
                    _ => app.start_root_move(&actor, request).await,
                }
                .map_err(to_gql_error)
            }
            StartLocationTarget::RootChange(target) => {
                // The same guard the preview applies: a root change's
                // destination must be empty or absent, so the files can never
                // already be there, and the planner would otherwise stamp a
                // mode the executor does not honour.
                let mode = root_scoped_execution_mode_into_application(input.mode)
                    .map_err(to_gql_error)?;
                app.start_root_change(
                    &actor,
                    StartRootChangeRequest {
                        library_id: target.library_id.to_string(),
                        root_id: target.root_id.to_string(),
                        destination_path: target.destination_path,
                        mode,
                        confirmation,
                    },
                )
                .await
                .map_err(to_location_root_gql_error)
            }
            StartLocationTarget::RootConsolidation(target) => app
                .start_root_consolidation(
                    &actor,
                    StartRootConsolidationRequest {
                        library_id: target.library_id.to_string(),
                        source_root_id: target.source_root_id.to_string(),
                        destination_root_id: target.destination_root_id.to_string(),
                        mode,
                        confirmation,
                    },
                )
                .await
                .map_err(to_location_root_gql_error),
        }?;
        // A just-accepted operation has no checkpoints yet; they are written as
        // each title enters the run.
        let checkpoints = app
            .location_operation_checkpoints(&accepted.operation.id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_started_location_operation(
            &accepted.operation,
            &checkpoints,
            &accepted.plan.fingerprint,
        ))
    }

    /// Request cancellation; the operation stops at its next title checkpoint.
    ///
    /// Titles already finished stay where the operation put them: cancelling is
    /// a safe stop, never a rollback.
    async fn cancel_location_operation(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Location-operation identity to stop at its next title checkpoint.")]
        id: ID,
    ) -> GqlResult<CancelLocationOperationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let operation_id = id.to_string();
        let requested = app
            .cancel_location_operation(&actor, &operation_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_canceled_location_operation(&operation_id, requested))
    }

    /// Pick an interrupted operation back up from its last verified checkpoint.
    ///
    /// Finished titles are never reprocessed, and an operation that is already
    /// terminal or was stored without its plan is reported as not resumed rather
    /// than restarted from the beginning.
    async fn resume_location_operation(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Location-operation identity to resume from its checkpoints.")] id: ID,
    ) -> GqlResult<ResumeLocationOperationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let operation_id = id.to_string();
        let Some(operation) = app
            .location_operation(&operation_id)
            .await
            .map_err(to_gql_error)?
        else {
            return Ok(from_resumed_location_operation(
                &operation_id,
                false,
                Some("this operation no longer exists".to_string()),
            ));
        };
        // FR-083 applies to a resume exactly as it applies to a start.
        app.require_location_operation_permission(&actor, &operation)
            .await
            .map_err(to_gql_error)?;

        // The reason comes from the application layer: it is the half that
        // knows whether the operation is finished, has no stored plan, or is
        // sitting on a volume that is not mounted right now.
        let plan = match app
            .resume_location_operation(&operation_id)
            .await
            .map_err(to_gql_error)?
        {
            LocationResumeDecision::Resume(plan) => *plan,
            LocationResumeDecision::NotResumable(reason) => {
                return Ok(from_resumed_location_operation(
                    &operation_id,
                    false,
                    Some(reason),
                ));
            }
        };
        app.spawn_location_operation(operation_id.clone(), plan);
        Ok(from_resumed_location_operation(&operation_id, true, None))
    }
}
