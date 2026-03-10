use async_graphql::{Context, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::from_user;
use crate::types::*;
use crate::utils::parse_entitlements;

#[derive(Default)]
pub(crate) struct UserMutations;

#[Object]
impl UserMutations {
    async fn create_user(
        &self,
        ctx: &Context<'_>,
        input: CreateUserInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let entitlements = parse_entitlements(&input.entitlements)?;
        let user = app
            .create_user(&actor, input.username, input.password, entitlements)
            .await
            .map_err(to_gql_error)?;
        Ok(from_user(user))
    }

    async fn set_user_password(
        &self,
        ctx: &Context<'_>,
        input: SetUserPasswordInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let user = app
            .set_user_password(
                &actor,
                &input.user_id,
                input.password,
                input.current_password,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_user(user))
    }

    async fn set_user_entitlements(
        &self,
        ctx: &Context<'_>,
        input: SetUserEntitlementsInput,
    ) -> GqlResult<UserPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let entitlements = parse_entitlements(&input.entitlements)?;
        let user = app
            .set_user_entitlements(&actor, &input.user_id, entitlements)
            .await
            .map_err(to_gql_error)?;
        Ok(from_user(user))
    }

    async fn delete_user(&self, ctx: &Context<'_>, input: DeleteUserInput) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_user(&actor, &input.user_id)
            .await
            .map(|_| true)
            .map_err(to_gql_error)
    }
}
