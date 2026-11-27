use async_graphql::{Context, ID, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::from_job_run;
use crate::types::{
    DeleteRecycledItemPayload, DeleteRecycledItemsInput, DeleteRecycledItemsPayload,
    EmptyRecycleBinPayload, RecycleRestoreConflictPolicyValue, RestoreRecycledItemPayload,
    RestoreRecycledItemsInput, RestoreRecycledItemsPayload,
};

#[derive(Default)]
pub struct RecycleBinMutations;

#[Object]
impl RecycleBinMutations {
    /// Restore a recycled item back to its original path on disk.
    async fn restore_recycled_item(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Recycled-item identity to restore to its original path.")] id: ID,
        #[graphql(
            default,
            desc = "Confirms restoring a file marked bad for its title and clears that mark."
        )]
        clear_bad_file: bool,
    ) -> GqlResult<RestoreRecycledItemPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let accepted = app
            .start_restore_recycled_item_job(&actor, id.as_str(), clear_bad_file)
            .await
            .map_err(to_gql_error)?;
        Ok(RestoreRecycledItemPayload {
            id,
            job_run: from_job_run(accepted.job_run),
        })
    }

    /// Restore selected recycled items using one background job.
    async fn restore_recycled_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Recycled-item identities, conflict policy, and preview fingerprint for the restore job."
        )]
        input: RestoreRecycledItemsInput,
    ) -> GqlResult<RestoreRecycledItemsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let conflict_policy = match input.conflict_policy {
            RecycleRestoreConflictPolicyValue::KeepBoth => {
                scryer_application::RecycleRestoreConflictPolicy::KeepBoth
            }
            RecycleRestoreConflictPolicyValue::ReplaceExisting => {
                scryer_application::RecycleRestoreConflictPolicy::ReplaceExisting
            }
        };
        let accepted = app
            .start_restore_recycled_items_job(
                &actor,
                input.ids.iter().map(|id| id.as_str().to_string()).collect(),
                conflict_policy,
                &input.preview_fingerprint,
                input.clear_bad_files,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(RestoreRecycledItemsPayload {
            ids: accepted.entry_ids.into_iter().map(ID::from).collect(),
            job_run: from_job_run(accepted.job_run),
        })
    }

    /// Permanently delete a single recycled item and its retained file when applicable.
    async fn delete_recycled_item(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Recycled-item identity to permanently delete.")] id: ID,
    ) -> GqlResult<DeleteRecycledItemPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let deleted = app
            .delete_recycled_item(&actor, id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteRecycledItemPayload { id, deleted })
    }

    /// Permanently delete selected recycled items using one background job.
    async fn delete_recycled_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Recycled-item identities to permanently delete.")]
        input: DeleteRecycledItemsInput,
    ) -> GqlResult<DeleteRecycledItemsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let accepted = app
            .start_purge_recycled_items_job(
                &actor,
                input.ids.iter().map(|id| id.as_str().to_string()).collect(),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteRecycledItemsPayload {
            ids: accepted.entry_ids.into_iter().map(ID::from).collect(),
            job_run: from_job_run(accepted.job_run),
        })
    }

    /// Empty recycle bins for the selected libraries using a background job.
    async fn empty_recycle_bin(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Optional library identities to purge; null targets all accessible recycle bins."
        )]
        library_ids: Option<Vec<ID>>,
    ) -> GqlResult<EmptyRecycleBinPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let library_ids = library_ids.map(|ids| ids.into_iter().map(|id| id.to_string()).collect());
        let job_run = app
            .start_empty_recycle_bin_job(&actor, library_ids)
            .await
            .map_err(to_gql_error)?;
        Ok(EmptyRecycleBinPayload {
            job_run: from_job_run(job_run),
        })
    }
}
