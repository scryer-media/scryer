//! Which public lists a title is on, or was on, for the title's own page.
//!
//! Only public subscriptions are named. A personal list's row is dropped here
//! whoever asks, so a title page never reveals what a member follows.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use scryer_domain::{
    LibraryPermission, ListMembership, ListMembershipState, ListScope, ListSubscription, User,
};

use super::ListSubscriptionQuery;
use crate::{AppResult, AppUseCase};

/// One public list's hold on a title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleListMembership {
    pub subscription_id: String,
    pub name: String,
    pub state: ListMembershipState,
    /// The list created this title.
    pub added_by_list: bool,
    /// When the title left the list, or `None` while it is still on it.
    pub left_at: Option<DateTime<Utc>>,
}

/// Public memberships keyed by title id. Titles on no public list have no
/// entry. Within a title, lists it is still on come first, then by name.
pub(crate) fn public_memberships_by_title(
    rows: &[ListMembership],
    subscriptions: &[ListSubscription],
) -> HashMap<String, Vec<TitleListMembership>> {
    let public: HashMap<&str, &ListSubscription> = subscriptions
        .iter()
        .filter(|subscription| subscription.scope == ListScope::Public)
        .map(|subscription| (subscription.id.as_str(), subscription))
        .collect();
    let mut by_title: HashMap<String, Vec<TitleListMembership>> = HashMap::new();
    for row in rows {
        let Some(title_id) = row.title_id.as_deref() else {
            continue;
        };
        let Some(subscription) = public.get(row.subscription_id.as_str()) else {
            continue;
        };
        by_title
            .entry(title_id.to_string())
            .or_default()
            .push(TitleListMembership {
                subscription_id: subscription.id.clone(),
                name: subscription.name.clone(),
                state: row.state,
                added_by_list: row.added_by_list,
                left_at: row.left_at,
            });
    }
    for memberships in by_title.values_mut() {
        memberships.sort_by(|left, right| {
            left.left_at
                .is_some()
                .cmp(&right.left_at.is_some())
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.subscription_id.cmp(&right.subscription_id))
        });
    }
    by_title
}

impl AppUseCase {
    /// The public lists each title is on or has left, for titles the actor
    /// may view. One read of the memberships and one of the public
    /// subscriptions for the whole batch.
    pub async fn public_list_memberships_for_titles(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<TitleListMembership>>> {
        let title_ids = self
            .filter_title_ids_for_permission(actor, title_ids, LibraryPermission::View)
            .await?;
        if title_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let lists = &self.services.lists;
        let rows = lists.memberships.list_by_titles(&title_ids).await?;
        if rows.is_empty() {
            return Ok(HashMap::new());
        }
        let subscriptions = lists
            .subscriptions
            .list(ListSubscriptionQuery::public())
            .await?;
        Ok(public_memberships_by_title(&rows, &subscriptions))
    }
}

#[cfg(test)]
#[path = "title_provenance_tests.rs"]
mod tests;
