//! Mutations that start, cancel, and resume location operations (US2, FR-030,
//! FR-033, FR-081, FR-083).
//!
//! Starting is a confirmation, not a submission: the client sends back the
//! fingerprint it previewed and the server rebuilds the plan from current state,
//! so a caller can never confirm one plan and execute another.

use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::location::operations::StartRootMoveRequest;
use scryer_application::location::preview::{PlanConfirmationRequest, PlanFingerprint};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{
    from_canceled_location_operation, from_resumed_location_operation,
    from_started_location_operation, location_destination_into_application,
};
use crate::types::{
    CancelLocationOperationPayload, ResumeLocationOperationPayload, StartLocationOperationInput,
    StartLocationOperationPayload,
};

#[derive(Default)]
pub(crate) struct LocationMutations;

#[Object]
impl LocationMutations {
    /// Confirm a previewed location operation and start it in the background.
    ///
    /// The plan is rebuilt server side and compared against the fingerprint the
    /// client previewed; a stale fingerprint, or a selection that still holds
    /// blocked or unresolved titles, is refused instead of started.
    async fn start_location_operation(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Previewed selection, destination, plan fingerprint, and typed confirmation when one is required."
        )]
        input: StartLocationOperationInput,
    ) -> GqlResult<StartLocationOperationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let request = StartRootMoveRequest {
            title_ids: input
                .title_ids
                .into_iter()
                .map(|id| id.to_string())
                .collect(),
            destination: location_destination_into_application(input.destination),
            confirmation: PlanConfirmationRequest {
                fingerprint: PlanFingerprint(input.plan_fingerprint),
                typed_confirmation: input.typed_confirmation,
            },
        };
        let accepted = app
            .start_root_move(&actor, request)
            .await
            .map_err(to_gql_error)?;
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

        let Some(plan) = app
            .resume_location_operation(&operation_id)
            .await
            .map_err(to_gql_error)?
        else {
            return Ok(from_resumed_location_operation(
                &operation_id,
                false,
                Some(
                    "this operation has finished or has no stored plan, so there is nothing to resume"
                        .to_string(),
                ),
            ));
        };
        app.spawn_location_operation(operation_id.clone(), plan);
        Ok(from_resumed_location_operation(&operation_id, true, None))
    }
}
