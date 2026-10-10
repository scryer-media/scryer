use async_graphql::{Context, ID, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{
    from_list_exclusion, from_list_provider_settings, from_list_subscription,
    from_member_list_policy, list_exclusion_input_from_input,
    list_provider_setting_changes_from_input, public_list_input_from_input,
    public_list_patch_from_input,
};
use crate::types::{
    AddListExclusionInput, ListAccountLinkPayload, ListAccountPayload, ListAccountPollPayload,
    ListAccountPollStatusValue, ListExclusionPayload, ListPolicyValue, ListProviderAppPayload,
    ListProviderSettingChangeInput, ListProviderSettingsPayload, ListScopeValue,
    ListSubscriptionPayload, ListSyncEnqueuedPayload, MemberListPolicyPayload, SubscribeListInput,
    UpdateListSubscriptionInput,
};

fn enqueued(ids: Vec<String>) -> ListSyncEnqueuedPayload {
    ListSyncEnqueuedPayload {
        subscription_ids: ids.into_iter().map(ID::from).collect(),
    }
}

/// Public-list management and owner-only personal account and list changes.
/// Routes follow the caller's library permissions.
#[derive(Default)]
pub struct ListMutations;

#[Object]
impl ListMutations {
    /// Start linking a provider account to the current member from an approved instance origin.
    async fn start_list_account_link(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Provider whose account is being linked.")] provider: String,
        #[graphql(
            desc = "Exact browser origin approved by the instance public URL or local listening address."
        )]
        origin: String,
    ) -> GqlResult<ListAccountLinkPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let origin = crate::context::list_account_link_origin_from_ctx(ctx, &origin).await?;
        Ok(app
            .start_list_account_link(&actor, &provider, &origin)
            .await
            .map_err(to_gql_error)?
            .into())
    }
    /// Poll an account authorization session owned by the current member.
    async fn poll_list_account_link(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Opaque session returned when linking started.")] session_id: ID,
    ) -> GqlResult<ListAccountPollPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let poll = app
            .poll_list_account_link(&actor, session_id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(ListAccountPollPayload {
            status: ListAccountPollStatusValue::from_application(poll.status),
            account: poll.account.map(ListAccountPayload::from_view),
        })
    }
    /// Consume an owned authorization session and link the verified provider identity.
    async fn complete_list_account_link(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Opaque session returned when linking started.")] session_id: ID,
        #[graphql(desc = "State returned by authorization.")] state: String,
        #[graphql(desc = "Provider bound to this session.")] provider: String,
        #[graphql(desc = "One-use authorization code or sealed exchange receipt.")] code: String,
        #[graphql(desc = "Provider issuer, required for Simkl.")] issuer: Option<String>,
    ) -> GqlResult<ListAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        Ok(ListAccountPayload::from_view(
            app.complete_list_account_link(
                &actor,
                session_id.as_str(),
                &state,
                &provider,
                &code,
                issuer.as_deref(),
            )
            .await
            .map_err(to_gql_error)?,
        ))
    }
    /// Remove an owned account and its follows while preserving library media.
    async fn unlink_list_account(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of an account owned by the current member.")] id: ID,
    ) -> GqlResult<ID> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        Ok(app
            .unlink_list_account(&actor, id.as_str())
            .await
            .map_err(to_gql_error)?
            .into())
    }
    /// Save an optional OAuth application for Trakt, AniList or MyAnimeList.
    async fn update_list_provider_app(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Provider whose OAuth application is being configured.")] provider: String,
        #[graphql(desc = "Application client identifier; omitted retains the current value.")]
        client_id: Option<String>,
        #[graphql(desc = "Write-only application secret; omitted retains the current value.")]
        client_secret: Option<String>,
        #[graphql(desc = "Callback URL on this instance.")] redirect_uri: Option<String>,
        #[graphql(desc = "Whether to use this application instead of the default flow.")]
        enabled: bool,
    ) -> GqlResult<ListProviderAppPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        Ok(app
            .update_list_provider_app(
                &actor,
                &provider,
                client_id,
                client_secret,
                redirect_uri,
                enabled,
            )
            .await
            .map_err(to_gql_error)?
            .into())
    }
    /// Follow a public list or a personal list from an owned account. Its first sync is due at once.
    async fn subscribe_list(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "The public or personal list to follow and how its titles are handled.")]
        input: SubscribeListInput,
    ) -> GqlResult<ListSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let personal = input.scope == ListScopeValue::Personal;
        let account_id = input.credential_id.as_ref().map(|id| id.to_string());
        let mut public_shape = input;
        public_shape.scope = ListScopeValue::Public;
        let input = public_list_input_from_input(public_shape).map_err(to_gql_error)?;
        let subscription = if personal {
            app.subscribe_personal_list(
                &actor,
                account_id.as_deref().ok_or_else(|| {
                    to_gql_error(scryer_application::AppError::Validation(
                        "choose a linked account".into(),
                    ))
                })?,
                input,
            )
            .await
        } else {
            app.subscribe_public_list(&actor, input).await
        }
        .map_err(to_gql_error)?;
        Ok(from_list_subscription(subscription))
    }

    /// Change a followed public list or an owned personal list. Omitted fields stay as they
    /// are.
    async fn update_list_subscription(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of a public list or a personal list owned by the current member.")] id: ID,
        #[graphql(desc = "The settings to change.")] input: UpdateListSubscriptionInput,
    ) -> GqlResult<ListSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let patch = public_list_patch_from_input(input).map_err(to_gql_error)?;
        let subscription = app
            .update_visible_list(&actor, id.as_str(), patch)
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_subscription(subscription))
    }

    /// Turn a followed public list or an owned personal list off or back on. Turning it on makes it due
    /// at once.
    async fn set_list_subscription_enabled(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of a public list or a personal list owned by the current member.")] id: ID,
        #[graphql(desc = "Whether the list syncs.")] enabled: bool,
    ) -> GqlResult<ListSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let subscription = app
            .set_visible_list_enabled(&actor, id.as_str(), enabled)
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_subscription(subscription))
    }

    /// Make one followed public list or an owned personal list due now and start a sync.
    async fn sync_list_subscription(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of a public list or a personal list owned by the current member.")] id: ID,
    ) -> GqlResult<ListSyncEnqueuedPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let ids = app
            .sync_visible_list_now(&actor, id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(enqueued(ids))
    }

    /// Make every enabled list in the selected scope due now and start a sync.
    /// Personal scope includes only lists owned by the current member.
    async fn sync_all_lists(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "PUBLIC or omitted syncs instance public lists; PERSONAL syncs only the current member's personal lists."
        )]
        scope: Option<ListScopeValue>,
    ) -> GqlResult<ListSyncEnqueuedPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let ids = if scope == Some(ListScopeValue::Personal) {
            app.sync_my_lists(&actor).await
        } else {
            app.sync_all_public_lists(&actor).await
        }
        .map_err(to_gql_error)?;
        Ok(enqueued(ids))
    }

    /// Stop following a public list or an owned personal list. Every title and request it made stays.
    /// Returns the ID of the list.
    async fn unsubscribe_list(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of a public list or a personal list owned by the current member.")] id: ID,
    ) -> GqlResult<ID> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = app
            .unsubscribe_visible_list(&actor, id.as_str())
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
