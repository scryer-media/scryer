use async_graphql::{Context, Error, Object, Result as GqlResult};
use scryer_domain::Entitlement;

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::types::*;

#[derive(Default)]
pub(crate) struct WantedMutations;

#[Object]
impl WantedMutations {
    async fn trigger_title_wanted_search(
        &self,
        ctx: &Context<'_>,
        input: TitleIdInput,
    ) -> GqlResult<i32> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let queued = app
            .trigger_title_wanted_search(&input.title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(queued as i32)
    }

    async fn trigger_season_wanted_search(
        &self,
        ctx: &Context<'_>,
        input: SeasonSearchInput,
    ) -> GqlResult<i32> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let queued = app
            .trigger_season_wanted_search(&input.title_id, input.season_number as u32)
            .await
            .map_err(to_gql_error)?;
        Ok(queued as i32)
    }

    async fn trigger_wanted_search(
        &self,
        ctx: &Context<'_>,
        input: WantedItemIdInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.trigger_wanted_item_search(&input.wanted_item_id)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn pause_wanted_item(
        &self,
        ctx: &Context<'_>,
        input: WantedItemIdInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.pause_wanted_item(&input.wanted_item_id)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn resume_wanted_item(
        &self,
        ctx: &Context<'_>,
        input: WantedItemIdInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.resume_wanted_item(&input.wanted_item_id)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn reset_wanted_item(
        &self,
        ctx: &Context<'_>,
        input: WantedItemIdInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.reset_wanted_item(&input.wanted_item_id)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn force_grab_pending_release(
        &self,
        ctx: &Context<'_>,
        input: PendingReleaseActionInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.force_grab_pending_release(&input.id)
            .await
            .map_err(to_gql_error)
    }

    async fn dismiss_pending_release(
        &self,
        ctx: &Context<'_>,
        input: PendingReleaseActionInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.dismiss_pending_release(&input.id)
            .await
            .map_err(to_gql_error)
    }
}
