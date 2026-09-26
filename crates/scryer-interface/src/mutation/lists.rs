use async_graphql::{Context, ID, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{
    from_list_exclusion, from_list_provider_settings, from_list_subscription,
    from_member_list_policy, list_exclusion_input_from_input,
    list_provider_setting_changes_from_input, public_list_input_from_input,
    public_list_patch_from_input,
};
use crate::types::{
    AddListExclusionInput, ListExclusionPayload, ListPolicyValue, ListProviderSettingChangeInput,
    ListProviderSettingsPayload, ListScopeValue, ListSubscriptionPayload, ListSyncEnqueuedPayload,
    MemberListPolicyPayload, SubscribeListInput, UpdateListSubscriptionInput,
};

fn enqueued(ids: Vec<String>) -> ListSyncEnqueuedPayload {
    ListSyncEnqueuedPayload {
        subscription_ids: ids.into_iter().map(ID::from).collect(),
    }
}

/// Public-list changes. Every one requires list management permission, and a
/// route may target only a library the caller may manage titles in.
#[derive(Default)]
pub struct ListMutations;

#[Object]
impl ListMutations {
    /// Follow a public list. Its first sync is due at once.
    async fn subscribe_list(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The list to follow and how its titles are handled.")]
        input: SubscribeListInput,
    ) -> GqlResult<ListSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let input = public_list_input_from_input(input).map_err(to_gql_error)?;
        let subscription = app
            .subscribe_public_list(&actor, input)
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_subscription(subscription))
    }

    /// Change a followed public list's settings. Omitted fields stay as they
    /// are.
    async fn update_list_subscription(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the followed public list.")] id: ID,
        #[graphql(desc = "The settings to change.")] input: UpdateListSubscriptionInput,
    ) -> GqlResult<ListSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let patch = public_list_patch_from_input(input).map_err(to_gql_error)?;
        let subscription = app
            .update_public_list(&actor, id.as_str(), patch)
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_subscription(subscription))
    }

    /// Turn a followed public list off or back on. Turning it on makes it due
    /// at once.
    async fn set_list_subscription_enabled(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the followed public list.")] id: ID,
        #[graphql(desc = "Whether the list syncs.")] enabled: bool,
    ) -> GqlResult<ListSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let subscription = app
            .set_public_list_enabled(&actor, id.as_str(), enabled)
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_subscription(subscription))
    }

    /// Make one followed public list due now and start a sync.
    async fn sync_list_subscription(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the followed public list.")] id: ID,
    ) -> GqlResult<ListSyncEnqueuedPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let ids = app
            .sync_public_list_now(&actor, id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(enqueued(ids))
    }

    /// Make every enabled public list due now and start a sync.
    async fn sync_all_lists(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Must be `PUBLIC` or omitted.")] scope: Option<ListScopeValue>,
    ) -> GqlResult<ListSyncEnqueuedPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if scope.is_some_and(|scope| scope != ListScopeValue::Public) {
            return Err(to_gql_error(scryer_application::AppError::Validation(
                "only public lists can be synced here".to_string(),
            )));
        }
        let ids = app
            .sync_all_public_lists(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(enqueued(ids))
    }

    /// Stop following a public list. Every title and request it made stays.
    /// Returns the ID of the list.
    async fn unsubscribe_list(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the followed public list.")] id: ID,
    ) -> GqlResult<ID> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = app
            .unsubscribe_public_list(&actor, id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(id.into())
    }

    /// Keep a title off every list, or off one public list.
    async fn add_list_exclusion(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The title to exclude and where.")] input: AddListExclusionInput,
    ) -> GqlResult<ListExclusionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let input = list_exclusion_input_from_input(input).map_err(to_gql_error)?;
        let view = app
            .add_list_exclusion(&actor, input)
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_exclusion(view))
    }

    /// Let lists add a title again. Returns the ID of the removed exclusion.
    async fn remove_list_exclusion(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the exclusion.")] id: ID,
    ) -> GqlResult<ID> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = app
            .remove_list_exclusion(&actor, id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(id.into())
    }

    /// Change a list provider's server-wide settings. A key left out keeps
    /// its value; a null or blank value clears it. Secret values are never
    /// returned.
    async fn update_list_provider_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Provider key, such as `mdblist`.")] provider: String,
        #[graphql(desc = "The settings to change.")] changes: Vec<ListProviderSettingChangeInput>,
    ) -> GqlResult<ListProviderSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .update_list_provider_settings(
                &actor,
                provider.trim(),
                list_provider_setting_changes_from_input(changes),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_provider_settings(settings))
    }

    /// Set how one member's personal-list requests are admitted.
    async fn set_member_list_policy(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the member.")] user_id: ID,
        #[graphql(desc = "The member's new list policy.")] policy: ListPolicyValue,
    ) -> GqlResult<MemberListPolicyPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let entry = app
            .set_member_list_policy(&actor, user_id.as_str(), policy.into_domain())
            .await
            .map_err(to_gql_error)?;
        Ok(from_member_list_policy(entry))
    }
}
