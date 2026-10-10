//! List reads with owner-only access to personal accounts and subscriptions.

use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_application::lists::public::{LIST_MEMBERSHIP_PAGE_MAX, LIST_SYNC_RUNS_MAX};
use scryer_interface_core::{actor_from_ctx, app_from_ctx, to_gql_error};
use scryer_interface_media::mappers::{
    from_list_exclusion, from_list_membership_page, from_list_preview, from_list_provider,
    from_list_provider_settings, from_list_subscription, from_list_sync_run,
    from_member_list_policy, list_source_draft_from_input,
};
use scryer_interface_media::types::{
    ListAccountPayload, ListExclusionPayload, ListMembershipPagePayload, ListPreviewPayload,
    ListProviderAppPayload, ListProviderPayload, ListProviderSettingsPayload, ListSourceInput,
    ListSubscriptionPayload, ListSyncRunPayload, MemberListPolicyPayload,
};

/// Cached canonical genre and theme vocabulary supplied by the metadata gateway.
#[derive(async_graphql::SimpleObject)]
struct CanonicalTagVocabularyPayload {
    /// BLAKE3 content version of the canonical registry snapshot.
    version: String,
    /// Complete canonical vocabulary in deterministic order.
    entries: Vec<CanonicalTagVocabularyEntryPayload>,
}

/// Registry category of a canonical tag.
#[derive(async_graphql::Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
enum CanonicalTagCategoryValue {
    /// A genre.
    Genre,
    /// A theme.
    Theme,
}

impl CanonicalTagCategoryValue {
    fn from_application(
        value: scryer_application::lists::vocabulary::CanonicalTagCategory,
    ) -> Self {
        use scryer_application::lists::vocabulary::CanonicalTagCategory;
        match value {
            CanonicalTagCategory::Genre => Self::Genre,
            CanonicalTagCategory::Theme => Self::Theme,
        }
    }
}

/// A registered canonical genre or theme available to list filters.
#[derive(async_graphql::SimpleObject)]
struct CanonicalTagVocabularyEntryPayload {
    /// Stable canonical key stored in saved filters.
    key: String,
    /// Registry category.
    category: CanonicalTagCategoryValue,
    /// Display name of the canonical entry.
    name: String,
    /// Registered aliases usable for search and unambiguous legacy-label conversion.
    aliases: Vec<String>,
}

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
    /// Read the canonical vocabulary, refreshing stale snapshots on demand.
    async fn canonical_tag_vocabulary(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            default = false,
            desc = "Explicitly refresh the snapshot, bypassing retry delay while sharing any in-flight request."
        )]
        retry: bool,
    ) -> GqlResult<CanonicalTagVocabularyPayload> {
        actor_from_ctx(ctx)?;
        let snapshot = app_from_ctx(ctx)?
            .canonical_tag_vocabulary(retry)
            .await
            .map_err(to_gql_error)?;
        Ok(CanonicalTagVocabularyPayload {
            version: snapshot.version,
            entries: snapshot
                .entries
                .into_iter()
                .map(|entry| CanonicalTagVocabularyEntryPayload {
                    key: entry.key,
                    category: CanonicalTagCategoryValue::from_application(entry.category),
                    name: entry.name,
                    aliases: entry.aliases,
                })
                .collect(),
        })
    }

    /// Linked accounts owned by the current member. Credentials are never returned.
    async fn my_list_accounts(&self, ctx: &Context<'_>) -> GqlResult<Vec<ListAccountPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        Ok(app
            .my_list_accounts(&actor)
            .await
            .map_err(to_gql_error)?
            .into_iter()
            .map(ListAccountPayload::from_view)
            .collect())
    }
    /// Refresh an owned account's provider identity and available personal lists.
    async fn list_account(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of an account owned by the current member.")] id: ID,
    ) -> GqlResult<ListAccountPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        Ok(ListAccountPayload::from_view(
            app.list_account(&actor, id.as_str())
                .await
                .map_err(to_gql_error)?,
        ))
    }
    /// Optional operator-owned OAuth applications. Secrets are write-only.
    async fn list_provider_apps(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ListProviderAppPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        Ok(app
            .list_provider_apps(&actor)
            .await
            .map_err(to_gql_error)?
            .into_iter()
            .map(Into::into)
            .collect())
    }
    /// Personal lists followed by the current member.
    async fn my_list_subscriptions(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ListSubscriptionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        Ok(app
            .my_list_subscriptions(&actor)
            .await
            .map_err(to_gql_error)?
            .into_iter()
            .map(from_list_subscription)
            .collect())
    }
    /// Every provider public or personal lists can be followed from, with the lists each
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

    /// One followed public list or a personal list owned by the current member.
    /// Returns null when no list has that ID.
    async fn list_subscription(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of a public list or a personal list owned by the current member.")] id: ID,
    ) -> GqlResult<Option<ListSubscriptionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let subscription = app
            .visible_list_subscription(&actor, id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(subscription.map(from_list_subscription))
    }

    /// A page of titles on a followed public list or an owned personal list, in list order.
    async fn list_subscription_memberships(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of a public list or a personal list owned by the current member.")] id: ID,
        #[graphql(desc = "Page size from 1 through 500; defaults to 100.")] limit: Option<i32>,
        #[graphql(desc = "Titles to skip; defaults to 0.")] offset: Option<i32>,
    ) -> GqlResult<ListMembershipPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = clamp(limit, DEFAULT_MEMBERSHIP_LIMIT, LIST_MEMBERSHIP_PAGE_MAX);
        let offset = usize::try_from(offset.unwrap_or(0).max(0)).unwrap_or(0);
        let page = app
            .visible_list_memberships(&actor, id.as_str(), limit, offset)
            .await
            .map_err(to_gql_error)?;
        Ok(from_list_membership_page(page))
    }

    /// Recent syncs of a followed public list or an owned personal list, newest first.
    async fn list_sync_runs(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of a public list or a personal list owned by the current member.")]
        subscription_id: ID,
        #[graphql(desc = "Most syncs to return, from 1 through 1000; defaults to 20.")]
        limit: Option<i32>,
    ) -> GqlResult<Vec<ListSyncRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = clamp(limit, DEFAULT_SYNC_RUN_LIMIT, LIST_SYNC_RUNS_MAX);
        let runs = app
            .visible_list_sync_runs(&actor, subscription_id.as_str(), limit)
            .await
            .map_err(to_gql_error)?;
        Ok(runs.into_iter().map(from_list_sync_run).collect())
    }

    /// What the next sync of a followed public list or an owned personal list would do.
    /// Public-list previews require list management permission.
    async fn list_subscription_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of a public list or a personal list owned by the current member.")] id: ID,
        #[graphql(
            desc = "Draft filter override for this preview; omitted uses the saved filters."
        )]
        filters: Option<Vec<scryer_interface_media::types::ListFilterInput>>,
        #[graphql(desc = "Draft facet override for this preview; omitted uses the saved facets.")]
        kinds: Option<Vec<scryer_interface_media::types::MediaFacetValue>>,
        #[graphql(desc = "Draft per-sync cap; omitted keeps the saved cap, null removes it.")]
        max_per_sync: async_graphql::MaybeUndefined<u32>,
    ) -> GqlResult<ListPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview = app
            .preview_visible_list_with_filters(
                &actor,
                id.as_str(),
                filters
                    .map(scryer_interface_media::mappers::filters_from_input)
                    .transpose()
                    .map_err(to_gql_error)?,
                kinds.map(|kinds| kinds.into_iter().map(|kind| kind.into_domain()).collect()),
                match max_per_sync {
                    async_graphql::MaybeUndefined::Undefined => None,
                    async_graphql::MaybeUndefined::Null => Some(None),
                    async_graphql::MaybeUndefined::Value(value) => Some(Some(value)),
                },
            )
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

    /// What following a public source or a source from an owned account would do.
    /// Public-source previews require list management permission.
    async fn list_source_preview(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "The provider, source type and parameters, or a link; personal sources also require an owned linked account."
        )]
        input: ListSourceInput,
        #[graphql(
            default,
            desc = "Draft filters to evaluate before following the list; defaults to no filters."
        )]
        filters: Vec<scryer_interface_media::types::ListFilterInput>,
        #[graphql(desc = "Draft facets to preview; omitted uses the source's available facets.")]
        kinds: Option<Vec<scryer_interface_media::types::MediaFacetValue>>,
        #[graphql(desc = "Draft per-sync cap; omitted or null means unlimited.")]
        max_per_sync: Option<u32>,
    ) -> GqlResult<ListPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let mut draft = list_source_draft_from_input(input);
        draft.preview_max_per_sync = max_per_sync;
        draft.preview_filters =
            scryer_interface_media::mappers::filters_from_input(filters).map_err(to_gql_error)?;
        draft.preview_kinds =
            kinds.map(|kinds| kinds.into_iter().map(|kind| kind.into_domain()).collect());
        let preview = if draft.credential_id.is_some() {
            app.preview_personal_source(&actor, draft).await
        } else {
            app.preview_list_source(&actor, draft).await
        }
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
