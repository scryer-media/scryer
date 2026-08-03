use async_graphql::{Context, ID, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::from_job_run;
use crate::types::{DeleteRecycledItemPayload, EmptyRecycleBinPayload, RestoreRecycledItemPayload};

#[derive(Default)]
pub struct RecycleBinMutations;

#[Object]
impl RecycleBinMutations {
    /// Restore a recycled item back to its original path on disk.
    async fn restore_recycled_item(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<RestoreRecycledItemPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let accepted = app
            .start_restore_recycled_item_job(&actor, id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(RestoreRecycledItemPayload {
            id,
            job_run: from_job_run(accepted.job_run),
        })
    }

    /// Permanently delete a single recycled item.
    async fn delete_recycled_item(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<DeleteRecycledItemPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let deleted = app
            .delete_recycled_item(&actor, id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteRecycledItemPayload { id, deleted })
    }

    /// Empty recycle bins for the selected libraries. Returns the number of items purged.
    async fn empty_recycle_bin(
        &self,
        ctx: &Context<'_>,
        library_ids: Option<Vec<ID>>,
    ) -> GqlResult<EmptyRecycleBinPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let library_ids = library_ids.map(|ids| ids.into_iter().map(|id| id.to_string()).collect());
        let purged_count = app
            .empty_recycle_bin(&actor, library_ids)
            .await
            .map(|n| n as i32)
            .map_err(to_gql_error)?;
        Ok(EmptyRecycleBinPayload { purged_count })
    }
}
