use async_graphql::{Context, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};

#[derive(Default)]
pub struct RecycleBinMutations;

#[Object]
impl RecycleBinMutations {
    /// Restore a recycled item back to its original path on disk.
    async fn restore_recycled_item(&self, ctx: &Context<'_>, id: String) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.restore_recycled_item(&actor, &id)
            .await
            .map_err(to_gql_error)
    }

    /// Permanently delete a single recycled item.
    async fn delete_recycled_item(&self, ctx: &Context<'_>, id: String) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_recycled_item(&actor, &id)
            .await
            .map_err(to_gql_error)
    }

    /// Empty all recycle bins. Returns the number of items purged.
    async fn empty_recycle_bin(&self, ctx: &Context<'_>) -> GqlResult<i32> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.empty_recycle_bin(&actor)
            .await
            .map(|n| n as i32)
            .map_err(to_gql_error)
    }
}
