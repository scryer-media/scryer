//! The list store an assembly has when it has none.
//!
//! Reads answer "nothing"; writes refuse. That split keeps a test assembly
//! honest: a use case that lists subscriptions sees an empty instance, while a
//! use case that tries to subscribe learns the store is missing rather than
//! believing it wrote something.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_domain::{
    ExternalId, ListCounts, ListExclusion, ListMembership, ListSubscription, ListSyncRun,
    ListSyncStatus, MediaFacet, UserListAccount, UserListPolicy,
};

use super::ports::{
    ListExclusionRepository, ListMembershipRepository, ListSubscriptionQuery,
    ListSubscriptionRepository, UserListAccountRepository, UserListPolicyRepository,
};
use crate::{AppError, AppResult};

fn not_configured() -> AppError {
    AppError::Repository("list store is not configured".into())
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NullListStore;

#[async_trait]
impl ListSubscriptionRepository for NullListStore {
    async fn create(&self, _: ListSubscription) -> AppResult<ListSubscription> {
        Err(not_configured())
    }

    async fn update(&self, _: ListSubscription) -> AppResult<ListSubscription> {
        Err(not_configured())
    }

    async fn get_by_id(&self, _: &str) -> AppResult<Option<ListSubscription>> {
        Ok(None)
    }

    async fn list(&self, _: ListSubscriptionQuery) -> AppResult<Vec<ListSubscription>> {
        Ok(Vec::new())
    }

    async fn list_due(&self, _: DateTime<Utc>, _: usize) -> AppResult<Vec<ListSubscription>> {
        Ok(Vec::new())
    }

    async fn record_sync(&self, _: &str, _: &ListSyncStatus, _: &ListCounts) -> AppResult<()> {
        Err(not_configured())
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Err(not_configured())
    }

    async fn record_sync_run(&self, _: ListSyncRun) -> AppResult<ListSyncRun> {
        Err(not_configured())
    }

    async fn list_sync_runs(&self, _: &str, _: usize) -> AppResult<Vec<ListSyncRun>> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl ListMembershipRepository for NullListStore {
    async fn upsert_many(&self, _: &[ListMembership]) -> AppResult<u64> {
        Err(not_configured())
    }

    async fn list_by_subscription(&self, _: &str) -> AppResult<Vec<ListMembership>> {
        Ok(Vec::new())
    }

    async fn list_by_title(&self, _: &str) -> AppResult<Vec<ListMembership>> {
        Ok(Vec::new())
    }

    async fn list_by_titles(&self, _: &[String]) -> AppResult<Vec<ListMembership>> {
        Ok(Vec::new())
    }

    async fn list_by_request(&self, _: &str) -> AppResult<Vec<ListMembership>> {
        Ok(Vec::new())
    }

    async fn mark_left(
        &self,
        _: &str,
        _: DateTime<Utc>,
        _: DateTime<Utc>,
    ) -> AppResult<Vec<ListMembership>> {
        Err(not_configured())
    }

    async fn set_left_handled(&self, _: &str, _: &[String]) -> AppResult<u64> {
        Err(not_configured())
    }
}

#[async_trait]
impl ListExclusionRepository for NullListStore {
    async fn create(&self, _: ListExclusion) -> AppResult<ListExclusion> {
        Err(not_configured())
    }

    async fn get_by_id(&self, _: &str) -> AppResult<Option<ListExclusion>> {
        Ok(None)
    }

    async fn list(&self) -> AppResult<Vec<ListExclusion>> {
        Ok(Vec::new())
    }

    async fn find_matching(
        &self,
        _: MediaFacet,
        _: &[ExternalId],
        _: Option<&str>,
    ) -> AppResult<Vec<ListExclusion>> {
        Ok(Vec::new())
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Err(not_configured())
    }
}

#[async_trait]
impl UserListAccountRepository for NullListStore {
    async fn create(&self, _: UserListAccount) -> AppResult<UserListAccount> {
        Err(not_configured())
    }

    async fn update(&self, _: UserListAccount) -> AppResult<UserListAccount> {
        Err(not_configured())
    }

    async fn get_by_id(&self, _: &str) -> AppResult<Option<UserListAccount>> {
        Ok(None)
    }

    async fn list_by_user_id(&self, _: &str) -> AppResult<Vec<UserListAccount>> {
        Ok(Vec::new())
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Err(not_configured())
    }
}

#[async_trait]
impl UserListPolicyRepository for NullListStore {
    async fn get(&self, _: &str) -> AppResult<Option<UserListPolicy>> {
        Ok(None)
    }

    async fn set(&self, _: UserListPolicy) -> AppResult<UserListPolicy> {
        Err(not_configured())
    }

    async fn list(&self) -> AppResult<Vec<UserListPolicy>> {
        Ok(Vec::new())
    }
}
