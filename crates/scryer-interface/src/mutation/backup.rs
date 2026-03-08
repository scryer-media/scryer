use async_graphql::{Context, Object, Result as GqlResult};
use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::from_backup_info;
use crate::types::BackupInfoPayload;

#[derive(Default)]
pub struct BackupMutations;

#[Object]
impl BackupMutations {
    async fn create_backup(&self, ctx: &Context<'_>) -> GqlResult<BackupInfoPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let info = app.create_backup(&actor).await.map_err(to_gql_error)?;
        Ok(from_backup_info(info))
    }

    async fn delete_backup(&self, ctx: &Context<'_>, filename: String) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_backup(&actor, &filename).await.map_err(to_gql_error)
    }
}
