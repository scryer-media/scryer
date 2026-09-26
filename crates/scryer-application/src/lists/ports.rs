//! Repository ports for Lists.
//!
//! Five traits, one per aggregate, so a store can implement all of them on one
//! type while a test doubles only the one it exercises. None of these ports
//! filters by viewer: privacy is enforced one layer up, in the use cases, where
//! the actor is known. A repository that silently dropped rows would make an
//! owner check look like an empty list.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_domain::{
    ExternalId, ListCounts, ListExclusion, ListMembership, ListScope, ListSubscription,
    ListSyncRun, ListSyncStatus, MediaFacet, UserListAccount, UserListPolicy,
};

use crate::AppResult;

/// Which subscriptions to read. Every field is a conjunction; `None` means "any".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListSubscriptionQuery {
    pub scope: Option<ListScope>,
    pub owner_user_id: Option<String>,
    pub provider: Option<String>,
    pub enabled: Option<bool>,
}

impl ListSubscriptionQuery {
    pub fn public() -> Self {
        Self {
            scope: Some(ListScope::Public),
            ..Self::default()
        }
    }

    pub fn personal_for(owner_user_id: impl Into<String>) -> Self {
        Self {
            scope: Some(ListScope::Personal),
            owner_user_id: Some(owner_user_id.into()),
            ..Self::default()
        }
    }
}

#[async_trait]
pub trait ListSubscriptionRepository: Send + Sync {
    async fn create(&self, subscription: ListSubscription) -> AppResult<ListSubscription>;

    /// Rewrites the subscription's settings and routes. The sync bookkeeping
    /// (`sync`, `counts`) is written by [`Self::record_sync`], not here, so an
    /// edit racing a sync cannot roll the sync's result back.
    async fn update(&self, subscription: ListSubscription) -> AppResult<ListSubscription>;

    async fn get_by_id(&self, id: &str) -> AppResult<Option<ListSubscription>>;

    async fn list(&self, query: ListSubscriptionQuery) -> AppResult<Vec<ListSubscription>>;

    /// Enabled subscriptions whose next sync is at or before `now` and that
    /// are not paused past it, oldest due first. `limit` bounds one tick.
    async fn list_due(&self, now: DateTime<Utc>, limit: usize) -> AppResult<Vec<ListSubscription>>;

    /// Writes the outcome of one sync: state, timestamps, fingerprint, counts.
    async fn record_sync(
        &self,
        id: &str,
        sync: &ListSyncStatus,
        counts: &ListCounts,
    ) -> AppResult<()>;

    /// Removes the subscription and, through the schema, its routes,
    /// memberships, list-scoped exclusions, and sync runs. Titles and requests
    /// are untouched.
    async fn delete(&self, id: &str) -> AppResult<()>;

    async fn record_sync_run(&self, run: ListSyncRun) -> AppResult<ListSyncRun>;

    /// Newest first.
    async fn list_sync_runs(
        &self,
        subscription_id: &str,
        limit: usize,
    ) -> AppResult<Vec<ListSyncRun>>;
}

#[async_trait]
pub trait ListMembershipRepository: Send + Sync {
    /// Insert or refresh rows keyed by (subscription, item). A refreshed row
    /// keeps its `first_seen_at` and clears `left_at`/`left_handled`: an item
    /// that left and came back is present again, not still departed.
    async fn upsert_many(&self, memberships: &[ListMembership]) -> AppResult<u64>;

    async fn list_by_subscription(&self, subscription_id: &str) -> AppResult<Vec<ListMembership>>;

    /// Every subscription's row for one title, across both scopes. This is the
    /// cross-list guard's read; it carries subscription ids only, so it reveals
    /// nothing a caller could not already see.
    async fn list_by_title(&self, title_id: &str) -> AppResult<Vec<ListMembership>>;

    /// [`Self::list_by_title`] for many titles in one read, for callers that
    /// build facts for a batch of titles.
    async fn list_by_titles(&self, title_ids: &[String]) -> AppResult<Vec<ListMembership>>;

    async fn list_by_request(&self, request_id: &str) -> AppResult<Vec<ListMembership>>;

    /// Marks rows the current sync did not refresh as departed: every row of
    /// the subscription with `last_seen_at` before `seen_before` and no
    /// `left_at` yet gets `left_at = left_at_value`. Returns the rows marked,
    /// so the on-leave step acts on exactly this sync's departures.
    async fn mark_left(
        &self,
        subscription_id: &str,
        seen_before: DateTime<Utc>,
        left_at: DateTime<Utc>,
    ) -> AppResult<Vec<ListMembership>>;

    async fn set_left_handled(&self, subscription_id: &str, item_keys: &[String])
    -> AppResult<u64>;
}

#[async_trait]
pub trait ListExclusionRepository: Send + Sync {
    async fn create(&self, exclusion: ListExclusion) -> AppResult<ListExclusion>;

    async fn get_by_id(&self, id: &str) -> AppResult<Option<ListExclusion>>;

    /// Every exclusion, all-lists first, then list-scoped, newest first within
    /// each.
    async fn list(&self) -> AppResult<Vec<ListExclusion>>;

    /// Exclusions that cover an item of `kind` carrying any of `external_ids`:
    /// all-lists exclusions, plus those scoped to `subscription_id` when given.
    async fn find_matching(
        &self,
        kind: MediaFacet,
        external_ids: &[ExternalId],
        subscription_id: Option<&str>,
    ) -> AppResult<Vec<ListExclusion>>;

    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait UserListAccountRepository: Send + Sync {
    /// Stores the account with its credential encrypted. Fails on a duplicate
    /// (user, provider, provider user id).
    async fn create(&self, account: UserListAccount) -> AppResult<UserListAccount>;

    async fn update(&self, account: UserListAccount) -> AppResult<UserListAccount>;

    async fn get_by_id(&self, id: &str) -> AppResult<Option<UserListAccount>>;

    async fn list_by_user_id(&self, user_id: &str) -> AppResult<Vec<UserListAccount>>;

    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait UserListPolicyRepository: Send + Sync {
    /// `None` when the member has never been assigned a policy; callers apply
    /// the default.
    async fn get(&self, user_id: &str) -> AppResult<Option<UserListPolicy>>;

    async fn set(&self, policy: UserListPolicy) -> AppResult<UserListPolicy>;

    async fn list(&self) -> AppResult<Vec<UserListPolicy>>;
}
