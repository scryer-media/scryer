//! Public-list reads: the provider catalog, followed lists, their titles and
//! sync history, previews, exclusions, and members' list policies. Personal
//! lists are never read here.

use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::lists::public::{LIST_MEMBERSHIP_PAGE_MAX, LIST_SYNC_RUNS_MAX};
use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::mappers::{
    from_list_exclusion, from_list_membership_page, from_list_preview, from_list_provider,
    from_list_provider_settings, from_list_subscription, from_list_sync_run,
    from_member_list_policy, list_source_draft_from_input,
};
use scryer_interface_media::types::{
    ListExclusionPayload, ListMembershipPagePayload, ListPreviewPayload, ListProviderPayload,
    ListProviderSettingsPayload, ListSourceInput, ListSubscriptionPayload, ListSyncRunPayload,
    MemberListPolicyPayload,
};

const DEFAULT_MEMBERSHIP_LIMIT: i32 = 100;
const DEFAULT_SYNC_RUN_LIMIT: i32 = 20;

fn clamp(value: Option<i32>, default: i32, max: usize) -> usize {
    usize::try_from(value.unwrap_or(default).max(1))
        .unwrap_or(1)
        .min(max)
}

#[derive(Default)]
pub(crate) struct ListQueries;

#[Object]
impl ListQueries {
    /// Every provider a public list can be followed from, with the lists each
    /// offers and the link shapes it is recognised by.
    async fn list_providers(&self, ctx: &Context<'_>) -> GqlResult<Vec<ListProviderPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let providers = app
            .list_provider_catalog(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(providers.into_iter().map(from_list_provider).collect())
    }

    /// The server-wide settings of every installed list provider that
    /// declares any. Secret values are never returned. Requires list
    /// management permission.
    async fn list_provider_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ListProviderSettingsPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .list_provider_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(settings
            .into_iter()
            .map(from_list_provider_settings)
            .collect())
    }

    /// The instance's followed public lists, by name. Failure text is shown
    /// only to callers who manage lists.
    async fn list_subscriptions(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ListSubscriptionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let subscriptions = app
            .public_list_subscriptions(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(subscriptions
            .into_iter()
            .map(from_list_subscription)
            .collect())
    }

    /// One followed public list, or null when there is none with that ID.
    async fn list_subscription(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the followed public list.")] id: ID,
    ) -> GqlResult<Option<ListSubscriptionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let subscription = app
            .public_list_subscription(&actor, id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(subscription.map(from_list_subscription))
    }

    /// A page of the titles currently on a followed public list, in list order.
    async fn list_subscription_memberships(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the followed public list.")] id: ID,
        #[graphql(desc = "Page size from 1 through 500; defaults to 100.")] limit: Option<i32>,
        #[graphql(desc = "Titles to skip; defaults to 0.")] offset: Option<i32>,
    ) -> GqlResult<ListMembershipPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = clamp(limit, DEFAULT_MEMBERSHIP_LIMIT, LIST_MEMBERSHIP_PAGE_MAX);
        let offset = usize::try_from(offset.unwrap_or(0).max(0)).unwrap_or(0);
        let page = app
            .public_list_memberships(&actor, id.as_str(), limit, offset)
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_membership_page(page))
    }

    /// A followed public list's recent syncs, newest first.
    async fn list_sync_runs(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the followed public list.")] subscription_id: ID,
        #[graphql(desc = "Most syncs to return, from 1 through 100; defaults to 20.")]
        limit: Option<i32>,
    ) -> GqlResult<Vec<ListSyncRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = clamp(limit, DEFAULT_SYNC_RUN_LIMIT, LIST_SYNC_RUNS_MAX);
        let runs = app
            .public_list_sync_runs(&actor, subscription_id.as_str(), limit)
            .await
            .map_err(to_gql_error)?;
        Ok(runs.into_iter().map(from_list_sync_run).collect())
    }

    /// What the next sync of a followed public list would do. Requires list
    /// management permission.
    async fn list_subscription_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the followed public list.")] id: ID,
    ) -> GqlResult<ListPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview = app
            .preview_public_list(&actor, id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_preview(preview))
    }

    /// What following the list at a link would do. An unknown link answers
    /// `recognized: false`. Requires list management permission.
    async fn list_url_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Link to a list page.")] url: String,
    ) -> GqlResult<ListPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview = app
            .preview_list_url(&actor, &url)
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_preview(preview))
    }

    /// What following a list source would do. Requires list management
    /// permission.
    async fn list_source_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The provider, source type and parameters, or a link.")]
        input: ListSourceInput,
    ) -> GqlResult<ListPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview = app
            .preview_list_source(&actor, list_source_draft_from_input(input))
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_preview(preview))
    }

    /// Titles no list may add.
    async fn list_exclusions(&self, ctx: &Context<'_>) -> GqlResult<Vec<ListExclusionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let exclusions = app.list_exclusions(&actor).await.map_err(to_gql_error)?;
        Ok(exclusions.into_iter().map(from_list_exclusion).collect())
    }

    /// Every member's list policy and recent list requests. Requires list
    /// management permission.
    async fn list_member_policies(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<MemberListPolicyPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let policies = app
            .member_list_policies(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(policies.into_iter().map(from_member_list_policy).collect())
    }
}
